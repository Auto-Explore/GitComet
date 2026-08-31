use super::*;

#[derive(Clone, Debug)]
pub(crate) struct TreesitterIncrementalSeed {
    pub(crate) tree: tree_sitter::Tree,
    pub(crate) next_version: u64,
}

#[derive(Clone, Debug)]
pub(crate) enum TreesitterReparsePlan {
    Unchanged,
    Changed {
        edit_ranges: Vec<TreesitterByteEditRange>,
        incremental_seed: Option<TreesitterIncrementalSeed>,
        reusable_prefix_chunk_count: usize,
    },
}

pub(crate) fn build_treesitter_reparse_plan(
    previous: &PreparedSyntaxTreeState,
    language: DiffSyntaxLanguage,
    input: &TreesitterDocumentInput,
    edit_hint: Option<&DiffSyntaxEdit>,
) -> Option<TreesitterReparsePlan> {
    if previous.language != language {
        return None;
    }

    let old_input = previous.text.as_bytes();
    let new_input = input.text.as_bytes();
    let edit_ranges = edit_hint
        .and_then(|hint| {
            treesitter_byte_edit_range_from_hint(hint, old_input.len(), new_input.len())
        })
        .map(|range| vec![range])
        .unwrap_or_else(|| compute_incremental_edit_ranges(old_input, new_input));
    if edit_ranges.is_empty() {
        return Some(TreesitterReparsePlan::Unchanged);
    }
    let reusable_prefix_chunk_count =
        reusable_prefix_chunk_count(&previous.line_starts, old_input, &edit_ranges);

    let incremental_enabled = incremental_reparse_enabled();
    let should_attempt_incremental = incremental_enabled
        && (!incremental_reparse_should_fallback(&edit_ranges, old_input.len(), new_input.len())
            || incremental_reparse_should_try_large_late_edit(
                &edit_ranges,
                old_input.len(),
                new_input.len(),
            ));
    let incremental_seed = if should_attempt_incremental {
        let new_line_starts = input.line_starts.as_ref();
        let mut tree = previous.tree.clone();
        for edit_range in &edit_ranges {
            let input_edit = tree_sitter::InputEdit {
                start_byte: edit_range.start_byte,
                old_end_byte: edit_range.old_end_byte,
                new_end_byte: edit_range.new_end_byte,
                start_position: treesitter_point_for_byte(
                    &previous.line_starts,
                    old_input,
                    edit_range.start_byte,
                ),
                old_end_position: treesitter_point_for_byte(
                    &previous.line_starts,
                    old_input,
                    edit_range.old_end_byte,
                ),
                new_end_position: treesitter_point_for_byte(
                    new_line_starts,
                    new_input,
                    edit_range.new_end_byte,
                ),
            };
            tree.edit(&input_edit);
        }

        Some(TreesitterIncrementalSeed {
            tree,
            next_version: previous.source_version.saturating_add(1),
        })
    } else {
        None
    };

    Some(TreesitterReparsePlan::Changed {
        edit_ranges,
        incremental_seed,
        reusable_prefix_chunk_count,
    })
}

pub(crate) fn treesitter_document_cache_key_for_reparse_plan(
    language: DiffSyntaxLanguage,
    previous: &PreparedSyntaxTreeState,
    input: &TreesitterDocumentInput,
    edit_ranges: &[TreesitterByteEditRange],
) -> PreparedSyntaxCacheKey {
    use std::hash::{Hash, Hasher};

    let old_input = previous.text.as_bytes();
    let new_input = input.text.as_bytes();
    let mut hasher = FxHasher::default();
    previous.source_hash.hash(&mut hasher);
    input.text.len().hash(&mut hasher);
    input.line_starts.len().hash(&mut hasher);
    edit_ranges.len().hash(&mut hasher);
    for edit in edit_ranges {
        edit.start_byte.hash(&mut hasher);
        edit.old_end_byte.hash(&mut hasher);
        edit.new_end_byte.hash(&mut hasher);
        old_input[edit.start_byte..edit.old_end_byte].hash(&mut hasher);
        new_input[edit.start_byte..edit.new_end_byte].hash(&mut hasher);
    }
    PreparedSyntaxCacheKey {
        language,
        doc_hash: hasher.finish(),
    }
}

pub(crate) fn reusable_prefix_chunk_count(
    old_line_starts: &[usize],
    old_input: &[u8],
    edit_ranges: &[TreesitterByteEditRange],
) -> usize {
    let Some(first_changed_byte) = edit_ranges.iter().map(|edit| edit.start_byte).min() else {
        return 0;
    };
    let first_changed_line =
        treesitter_point_for_byte(old_line_starts, old_input, first_changed_byte).row;
    first_changed_line / TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS
}

#[derive(Default)]
pub(crate) struct ReusedPrefixLineTokenChunks {
    pub(crate) line_token_chunks: FxHashMap<usize, Vec<Arc<[SyntaxToken]>>>,
    pub(crate) injection_source: Option<ReusedPrefixInjectionSource>,
}

#[derive(Clone, Copy)]
pub(crate) struct ReusedPrefixInjectionSource {
    pub(crate) document_hash: u64,
    pub(crate) byte_end: usize,
}

impl TreesitterDocumentCache {
    pub(crate) fn clone_prefix_line_token_chunks(
        &mut self,
        cache_key: PreparedSyntaxCacheKey,
        chunk_limit: usize,
    ) -> ReusedPrefixLineTokenChunks {
        if chunk_limit == 0 {
            return ReusedPrefixLineTokenChunks::default();
        }
        let reused = self
            .by_cache_key
            .get(&cache_key)
            .map(|document| {
                let line_token_chunks = document
                    .line_token_chunks
                    .iter()
                    .filter(|&(&chunk_ix, _)| chunk_ix < chunk_limit)
                    .map(|(&chunk_ix, chunk)| (chunk_ix, chunk.clone()))
                    .collect();
                let injection_source = document.tree_state.as_ref().map(|state| {
                    let prefix_line_ix = chunk_limit
                        .saturating_mul(TS_DOCUMENT_LINE_TOKEN_CHUNK_ROWS)
                        .min(state.line_starts.len());
                    ReusedPrefixInjectionSource {
                        document_hash: state.source_hash,
                        byte_end: state
                            .line_starts
                            .get(prefix_line_ix)
                            .copied()
                            .unwrap_or(state.text.len()),
                    }
                });
                ReusedPrefixLineTokenChunks {
                    line_token_chunks,
                    injection_source,
                }
            })
            .unwrap_or_default();
        if !reused.line_token_chunks.is_empty() {
            self.touch_key(cache_key);
        }
        reused
    }
}

/// Carry injection trees alongside prefix token chunks reused by an incremental
/// reparse. Only injections wholly contained in the reusable prefix are copied:
/// a tree spanning the edit boundary may depend on changed suffix bytes.
pub(crate) fn clone_prefix_injection_cache_entries(
    old_document_hash: u64,
    new_document_hash: u64,
    prefix_byte_end: usize,
) {
    if old_document_hash == new_document_hash || prefix_byte_end == 0 {
        return;
    }

    TS_INJECTION_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        // Moved under the new hash rather than copied under it. The old hash
        // names a document that has just been replaced, so a duplicate is dead
        // weight -- and each one holds the injection's whole `all_line_tokens`
        // and its tree. Copying them grew the cache by the reused prefix on
        // every incremental reparse, so a template-heavy document reached
        // `TS_INJECTION_CACHE_MAX_ENTRIES` in a handful of edits and
        // `evict_injection_cache_if_full` then threw away half of it, including
        // entries the very next chunk build wanted. Nothing else reclaims a
        // stale document hash.
        let stale: Vec<TreesitterInjectionMatch> = cache
            .keys()
            .filter(|key| key.document_hash == old_document_hash && key.byte_end <= prefix_byte_end)
            .copied()
            .collect();

        for old_key in stale {
            let mut new_key = old_key;
            new_key.document_hash = new_document_hash;
            if cache.contains_key(&new_key) {
                continue;
            }
            let Some(mut entry) = cache.remove(&old_key) else {
                continue;
            };
            evict_injection_cache_if_full(&mut cache);
            entry.last_access = next_injection_access();
            cache.insert(new_key, entry);
        }

        // Whatever is left under the old hash lies past the reusable prefix, so
        // it describes bytes the edit may have changed and the new document can
        // never look it up -- `document_hash` is part of the key. Nothing else
        // in this module reclaims a superseded hash, so without this the cache
        // fills with dead entries and the LRU starts evicting live ones. A
        // document still painting from the old hash simply re-parses that
        // injection on demand, which is the same thing eviction already does.
        cache.retain(|key, _| key.document_hash != old_document_hash);
    });
}

pub(crate) fn treesitter_byte_edit_range_from_hint(
    edit_hint: &DiffSyntaxEdit,
    old_len: usize,
    new_len: usize,
) -> Option<TreesitterByteEditRange> {
    if edit_hint.old_range.start != edit_hint.new_range.start
        || edit_hint.old_range.start > edit_hint.old_range.end
        || edit_hint.new_range.start > edit_hint.new_range.end
        || edit_hint.old_range.end > old_len
        || edit_hint.new_range.end > new_len
    {
        return None;
    }

    Some(TreesitterByteEditRange {
        start_byte: edit_hint.old_range.start,
        old_end_byte: edit_hint.old_range.end,
        new_end_byte: edit_hint.new_range.end,
    })
}

pub(crate) fn compute_incremental_edit_ranges(
    old: &[u8],
    new: &[u8],
) -> Vec<TreesitterByteEditRange> {
    if old == new {
        return Vec::new();
    }

    let (prefix, old_suffix_start, new_suffix_start) =
        super::super::shared_byte_affix_bounds(old, new);

    vec![TreesitterByteEditRange {
        start_byte: prefix,
        old_end_byte: old_suffix_start,
        new_end_byte: new_suffix_start,
    }]
}

pub(crate) fn incremental_reparse_should_fallback(
    edits: &[TreesitterByteEditRange],
    old_len: usize,
    new_len: usize,
) -> bool {
    let changed_bytes = incremental_reparse_changed_bytes(edits);
    if changed_bytes == 0 {
        return false;
    }
    if changed_bytes > TS_INCREMENTAL_REPARSE_MAX_CHANGED_BYTES {
        return true;
    }

    let baseline = old_len.max(new_len).max(1);
    changed_bytes.saturating_mul(100)
        > baseline.saturating_mul(TS_INCREMENTAL_REPARSE_MAX_CHANGED_PERCENT)
}

pub(crate) fn incremental_reparse_changed_bytes(edits: &[TreesitterByteEditRange]) -> usize {
    edits.iter().fold(0usize, |acc, edit| {
        let old_delta = edit.old_end_byte.saturating_sub(edit.start_byte);
        let new_delta = edit.new_end_byte.saturating_sub(edit.start_byte);
        acc.saturating_add(old_delta.max(new_delta))
    })
}

pub(crate) fn incremental_reparse_should_try_large_late_edit(
    edits: &[TreesitterByteEditRange],
    old_len: usize,
    new_len: usize,
) -> bool {
    let [edit] = edits else {
        return false;
    };
    if edit.start_byte < TS_INCREMENTAL_REPARSE_LATE_EDIT_MIN_PREFIX_BYTES {
        return false;
    }

    let changed_bytes = incremental_reparse_changed_bytes(edits);
    if changed_bytes == 0 || changed_bytes > TS_INCREMENTAL_REPARSE_LATE_EDIT_MAX_CHANGED_BYTES {
        return false;
    }

    let baseline = old_len.max(new_len).max(1);
    changed_bytes.saturating_mul(100)
        <= baseline.saturating_mul(TS_INCREMENTAL_REPARSE_LATE_EDIT_MAX_CHANGED_PERCENT)
}

pub(crate) fn treesitter_point_for_byte(
    line_starts: &[usize],
    input: &[u8],
    byte_offset: usize,
) -> tree_sitter::Point {
    let input_len = input.len();
    let byte_offset = byte_offset.min(input_len);
    if line_starts.is_empty() {
        return tree_sitter::Point::new(0, byte_offset);
    }
    if byte_offset == input_len && input.last().copied() == Some(b'\n') {
        // For newline-terminated inputs, EOF is the start of a trailing empty row.
        return tree_sitter::Point::new(line_starts.len(), 0);
    }

    let line_ix = line_ix_for_byte(line_starts, byte_offset);
    let line_start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or_default()
        .min(byte_offset);
    tree_sitter::Point::new(line_ix, byte_offset.saturating_sub(line_start))
}

pub(crate) fn parse_treesitter_tree(
    parser: &mut tree_sitter::Parser,
    input: &[u8],
    old_tree: Option<&tree_sitter::Tree>,
    foreground_parse_budget: Option<Duration>,
) -> Option<tree_sitter::Tree> {
    let Some(foreground_parse_budget) = foreground_parse_budget else {
        return parser.parse(input, old_tree);
    };

    let start = std::time::Instant::now();
    let mut read_input = |byte_offset: usize, _position: tree_sitter::Point| -> &[u8] {
        if byte_offset < input.len() {
            &input[byte_offset..]
        } else {
            &[]
        }
    };
    let mut progress = |_state: &tree_sitter::ParseState| {
        if start.elapsed() >= foreground_parse_budget {
            std::ops::ControlFlow::Break(())
        } else {
            std::ops::ControlFlow::Continue(())
        }
    };
    let options = tree_sitter::ParseOptions::new().progress_callback(&mut progress);
    parser.parse_with_options(&mut read_input, old_tree, Some(options))
}

pub(crate) const MAX_TREESITTER_LINE_BYTES: usize = 512;

pub(crate) fn should_use_treesitter_for_line(text: &str) -> bool {
    text.len() <= MAX_TREESITTER_LINE_BYTES
}

/// Returns `true` when the heuristic tokenizer is guaranteed to produce
/// results identical to tree-sitter for this line, making the expensive
/// per-line tree-sitter parse unnecessary. Currently covers:
///
/// - Whitespace-only lines (no tokens from either)
/// - Lines whose first non-whitespace content is a line comment prefix
///   (both tree-sitter and the heuristic produce a single Comment token
///   spanning the rest of the line)
pub(crate) fn is_heuristic_sufficient_for_line(text: &str, language: DiffSyntaxLanguage) -> bool {
    let trimmed = text.trim_start();
    if trimmed.is_empty() {
        return true;
    }
    let config = heuristic_comment_config(language);
    if let Some(prefix) = config.line_comment
        && trimmed.starts_with(prefix)
    {
        return true;
    }
    if config.hash_comment && trimmed.starts_with('#') {
        return true;
    }
    if config.visual_basic_line_comment
        && (trimmed.starts_with('\'')
            || trimmed
                .get(..4)
                .is_some_and(|p| p.eq_ignore_ascii_case("rem ")))
    {
        return true;
    }
    false
}

pub(crate) struct TreesitterHighlightSpec {
    pub(crate) ts_language: tree_sitter::Language,
    pub(crate) query: tree_sitter::Query,
    pub(crate) capture_kinds: Vec<Option<SyntaxTokenKind>>,
    pub(crate) injection_query: Option<tree_sitter::Query>,
    /// One flag per injection-query pattern: `true` when the pattern carries
    /// `(#set! injection.combined)`, meaning every match of it belongs to one
    /// shared layer rather than getting a layer each.
    ///
    /// Computed once in `init_highlight_spec` so the hot match loop never has to
    /// walk `property_settings`.
    pub(crate) injection_combined_patterns: Vec<bool>,
    /// `injection_combined_patterns.iter().any(|&c| c)`, hoisted so the whole
    /// combined path can be skipped with one branch. This is the gate that keeps
    /// the feature a no-op for every grammar that does not declare it -- today
    /// that is all of them except F#, whose `xml_doc` rule is combined upstream.
    pub(crate) has_combined_injections: bool,
}
