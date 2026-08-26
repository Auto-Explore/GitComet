use super::*;

pub(crate) fn collect_treesitter_injection_matches_for_line_window_at(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
    document_byte_start: usize,
) -> TreesitterInjectionMatches {
    let empty = || TreesitterInjectionMatches {
        singles: Vec::new(),
        combined: Vec::new(),
        truncated: false,
    };
    let Some(injection_query) = highlight.injection_query.as_ref() else {
        return empty();
    };
    let Some(injection_content_capture_ix) =
        injection_query.capture_index_for_name("injection.content")
    else {
        return empty();
    };
    let injection_language_capture_ix =
        injection_query.capture_index_for_name("injection.language");
    let language_capture_ix = injection_query.capture_index_for_name("language");

    let query_passes = treesitter_document_query_passes_for_line_window(
        line_starts,
        input.len(),
        start_line_ix,
        end_line_ix,
    );
    if query_passes.is_empty() {
        return empty();
    }

    // The one gate that keeps this a no-op for every grammar that does not declare
    // `injection.combined`: no map allocated, no per-match pattern lookup, no
    // `did_exceed_match_limit` read, and `combined` stays empty below.
    let has_combined = highlight.has_combined_injections;

    let mut seen = FxHashSet::default();
    let mut injections = Vec::new();
    let mut combined_ranges: FxHashMap<(DiffSyntaxLanguage, usize), Vec<Range<usize>>> =
        FxHashMap::default();
    let mut truncated = false;
    for pass in &query_passes {
        catch_treesitter_query_panic(|| {
            TS_CURSOR.with(|cursor| {
                let mut cursor = cursor.borrow_mut();
                configure_query_cursor(&mut cursor, pass, input.len());
                let mut matches = cursor.matches(injection_query, tree.root_node(), input);
                tree_sitter::StreamingIterator::advance(&mut matches);
                while let Some(m) = matches.get() {
                    let Some(language) = injection_language_for_match(
                        injection_query,
                        m,
                        input,
                        injection_language_capture_ix,
                        language_capture_ix,
                    ) else {
                        tree_sitter::StreamingIterator::advance(&mut matches);
                        continue;
                    };
                    let pattern_ix = m.pattern_index;
                    let is_combined = highlight.is_combined_injection_pattern(pattern_ix);
                    for capture in m
                        .captures
                        .iter()
                        .filter(|capture| capture.index == injection_content_capture_ix)
                    {
                        let Some(byte_range) =
                            normalized_injection_content_byte_range(capture.node, input.len())
                        else {
                            continue;
                        };
                        if is_combined {
                            combined_ranges
                                .entry((language, pattern_ix))
                                .or_default()
                                .push(byte_range);
                            continue;
                        }
                        let injection = TreesitterInjectionMatch {
                            document_hash,
                            language,
                            byte_start: byte_range.start.saturating_add(document_byte_start),
                            byte_end: byte_range.end.saturating_add(document_byte_start),
                            content_hash: injection_content_hash(
                                &input[byte_range.start..byte_range.end],
                            ),
                        };
                        if seen.insert(injection) {
                            injections.push(injection);
                        }
                    }
                    tree_sitter::StreamingIterator::advance(&mut matches);
                }
                // Read inside the same borrow: the cursor is a thread-local and
                // the next pass reconfigures it.
                if has_combined {
                    truncated |= cursor.did_exceed_match_limit();
                }
            });
        });
    }

    injections.sort_by_key(|injection| (injection.byte_start, injection.byte_end));

    if !has_combined {
        return TreesitterInjectionMatches {
            singles: injections,
            combined: Vec::new(),
            truncated: false,
        };
    }

    let combined = combined_injection_groups_in_apply_order(combined_ranges)
        .into_iter()
        .map(|(language, _, ranges)| CombinedInjectionGroup { language, ranges })
        .collect::<Vec<_>>();

    TreesitterInjectionMatches {
        singles: injections,
        combined,
        truncated,
    }
}

/// Sorts and coalesces injection ranges into the form `set_included_ranges`
/// requires: ascending, non-overlapping, non-empty.
///
/// Touching ranges (`a.end == b.start`) are merged too. Leaving them adjacent
/// would be accepted by tree-sitter but produces one more range than necessary,
/// and the gap list in [`combined_injection_gaps`] would carry an empty entry.
pub(crate) fn merge_sorted_injection_ranges(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.retain(|range| range.start < range.end);
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => {
                last.end = last.end.max(range.end);
            }
            _ => merged.push(range),
        }
    }
    merged
}

/// The complement of `ranges` inside `window`.
///
/// Combined injections need this because tree-sitter reports *document* offsets
/// for a tree parsed with disjoint included ranges: a node whose first token is
/// in one range and whose last token is in another spans every host byte in
/// between. `((element) @tag)` over
/// `{% if x %}<div>{% endif %}text</div>` would otherwise paint the `{% endif %}`
/// with the injected grammar's colour. Subtracting these gaps from the injected
/// tokens is what keeps each grammar inside its own bytes.
///
/// `ranges` must already be sorted and non-overlapping.
pub(crate) fn combined_injection_gaps(
    window: Range<usize>,
    ranges: &[Range<usize>],
) -> Vec<Range<usize>> {
    let mut gaps = Vec::with_capacity(ranges.len() + 1);
    let mut cursor = window.start;
    for range in ranges {
        let start = range.start.max(window.start);
        if start > cursor {
            gaps.push(cursor..start.min(window.end));
        }
        cursor = cursor.max(range.end.min(window.end));
        if cursor >= window.end {
            break;
        }
    }
    if cursor < window.end {
        gaps.push(cursor..window.end);
    }
    gaps.retain(|gap| gap.start < gap.end);
    gaps
}

pub(crate) fn bounded_node_byte_range(
    node: tree_sitter::Node,
    input_len: usize,
) -> Option<Range<usize>> {
    let mut byte_range = node.byte_range();
    byte_range.start = byte_range.start.min(input_len);
    byte_range.end = byte_range.end.min(input_len);
    (byte_range.start < byte_range.end).then_some(byte_range)
}

pub(crate) fn normalized_injection_content_byte_range(
    node: tree_sitter::Node,
    input_len: usize,
) -> Option<Range<usize>> {
    let byte_range = bounded_node_byte_range(node, input_len)?;
    if !matches!(node.kind(), "string" | "template_string") {
        return Some(byte_range);
    }

    let named_child_count = node.named_child_count();
    if named_child_count == 0 {
        return Some(byte_range);
    }

    let mut content_start = usize::MAX;
    let mut content_end = 0usize;
    for child_ix in 0..named_child_count {
        let Some(child) = node.named_child(child_ix as u32) else {
            continue;
        };
        match child.kind() {
            "string_fragment" | "string_content" | "escape_sequence" => {
                let child_range = bounded_node_byte_range(child, input_len)?;
                content_start = content_start.min(child_range.start);
                content_end = content_end.max(child_range.end);
            }
            _ => return Some(byte_range),
        }
    }

    if content_start < content_end {
        Some(content_start..content_end)
    } else {
        Some(byte_range)
    }
}

pub(crate) fn injection_language_for_match(
    query: &tree_sitter::Query,
    query_match: &tree_sitter::QueryMatch<'_, '_>,
    input: &[u8],
    injection_language_capture_ix: Option<u32>,
    language_capture_ix: Option<u32>,
) -> Option<DiffSyntaxLanguage> {
    let pattern_language = query
        .property_settings(query_match.pattern_index)
        .iter()
        .filter(|setting| matches!(setting.key.as_ref(), "injection.language" | "language"))
        .find_map(|setting| {
            setting
                .value
                .as_deref()
                .and_then(injection_language_from_name)
                .or_else(|| {
                    setting.capture_id.and_then(|capture_id| {
                        query_capture_text(query_match.captures, capture_id as u32, input)
                            .and_then(injection_language_from_name)
                    })
                })
        });
    pattern_language.or_else(|| {
        [injection_language_capture_ix, language_capture_ix]
            .into_iter()
            .flatten()
            .find_map(|capture_ix| {
                query_capture_text(query_match.captures, capture_ix, input)
                    .and_then(injection_language_from_name)
            })
    })
}

pub(crate) fn query_capture_text<'capture, 'input>(
    captures: &[tree_sitter::QueryCapture<'capture>],
    capture_ix: u32,
    input: &'input [u8],
) -> Option<&'input str> {
    let capture = captures
        .iter()
        .rev()
        .find(|capture| capture.index == capture_ix)?;
    let mut byte_range = capture.node.byte_range();
    byte_range.start = byte_range.start.min(input.len());
    byte_range.end = byte_range.end.min(input.len());
    if byte_range.start >= byte_range.end {
        return None;
    }
    std::str::from_utf8(&input[byte_range.start..byte_range.end]).ok()
}

pub(crate) fn injection_language_from_name(name: &str) -> Option<DiffSyntaxLanguage> {
    let name =
        name.trim_matches(|ch: char| ch.is_ascii_whitespace() || matches!(ch, '"' | '\'' | '`'));
    if name.is_empty() {
        return None;
    }
    diff_syntax_language_for_code_fence_info(name)
}

pub(crate) fn next_injection_access() -> u64 {
    TS_INJECTION_ACCESS_COUNTER.with(|c| {
        let val = c.get().wrapping_add(1);
        c.set(val);
        val
    })
}

pub(crate) fn evict_injection_cache_if_full(
    cache: &mut FxHashMap<TreesitterInjectionMatch, CachedInjection>,
) {
    if cache.len() < TS_INJECTION_CACHE_MAX_ENTRIES {
        return;
    }

    // Evict the least-recently-used half instead of clearing everything.
    let mut entries: Vec<_> = cache
        .iter()
        .map(|(key, value)| (*key, value.last_access))
        .collect();
    entries.sort_unstable_by_key(|(_, access)| *access);
    let evict_count = entries.len() / 2;
    for (key, _) in entries.into_iter().take(evict_count) {
        cache.remove(&key);
    }
}

pub(crate) fn ensure_injection_cached(
    input: &[u8],
    line_starts: &[usize],
    injection: TreesitterInjectionMatch,
    parent_document_byte_start: usize,
) -> bool {
    TS_INJECTION_CACHE.with(|cache| {
        if let Some(entry) = cache.borrow_mut().get_mut(&injection) {
            entry.last_access = next_injection_access();
            return true;
        }

        let Some(local_byte_start) = injection.byte_start.checked_sub(parent_document_byte_start)
        else {
            return false;
        };
        let Some(local_byte_end) = injection.byte_end.checked_sub(parent_document_byte_start)
        else {
            return false;
        };
        let injection_byte_range =
            local_byte_start.min(input.len())..local_byte_end.min(input.len());
        if injection_byte_range.is_empty() {
            return false;
        }
        let Ok(injection_text) = std::str::from_utf8(&input[injection_byte_range.clone()]) else {
            return false;
        };
        if injection_text.is_empty() {
            return false;
        }
        let injection_input = treesitter_document_input_from_text(injection_text);
        if injection_input.line_starts.is_empty() {
            return false;
        }
        let Some(highlight) = tree_sitter_highlight_spec(injection.language) else {
            return false;
        };
        let Some(tree) = with_ts_parser_parse_result(&highlight.ts_language, |parser| {
            parse_treesitter_tree(parser, injection_input.text.as_bytes(), None, None)
        }) else {
            return false;
        };

        let injection_line_count = injection_input.line_starts.len();
        let all_line_tokens = collect_treesitter_document_line_tokens_for_line_window_at(
            &tree,
            highlight,
            injection_input.text.as_bytes(),
            injection_input.line_starts.as_ref(),
            0,
            injection_line_count,
            injection.document_hash,
            injection.byte_start,
        );

        let injection_start_line_ix = line_ix_for_byte(line_starts, local_byte_start);
        let access = next_injection_access();

        let mut cache = cache.borrow_mut();
        evict_injection_cache_if_full(&mut cache);
        cache.insert(
            injection,
            CachedInjection {
                all_line_tokens,
                injection_line_starts: injection_input.line_starts.as_ref().to_vec(),
                injection_start_line_ix,
                tree,
                last_access: access,
            },
        );
        true
    })
}

/// The hash an injection's bytes are keyed by.
///
/// One function so the two callers cannot drift: the cache is keyed on this at
/// insertion, and [`injected_syntax_pair_at`] re-derives it to prove an entry
/// belongs to the document it is about to answer for. Hashing a `&[u8]` and
/// hashing the `&str` over the same bytes give different values, which is a
/// silent miss rather than a wrong answer -- the pair lookup just falls through
/// to the host grammar, exactly the bug it was added to fix.
pub(crate) fn injection_content_hash(content: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = FxHasher::default();
    content.hash(&mut hasher);
    hasher.finish()
}

/// The pair at `offset` as the *injected* grammar sees it.
///
/// Tried before the host tree by [`prepared_document_syntax_pair_at_display_offset`],
/// for the same reason `LiveSyntaxSnapshot::syntax_pair_at` tries its layers
/// first: a bracket inside an injected region belongs to the grammar that region
/// is written in. To PHP, the whole of a file's inline HTML is one `text` node,
/// so without this a click on a tag there found no delimiter and fell through to
/// whatever PHP block enclosed it -- which is why clicking a bracket appeared to
/// do nothing while clicking a column away appeared to work.
///
/// `document_hash` scopes the search to the caller's own document and
/// `content_hash` is re-checked against its text rather than trusted: the cache
/// is a thread-local shared by every document, and two of them can easily have
/// an injection at the same byte range -- with the same bytes in it, under
/// different grammars. Neither check subsumes the other, so both run.
///
/// Combined injections are deliberately not consulted. Their ranges are stitched
/// from several disjoint spans, so a single `byte_start` shift cannot map their
/// offsets back, and the host grammar remains the right answer in the gaps
/// between them.
pub(crate) fn injected_syntax_pair_at(
    text: &str,
    document_hash: u64,
    offset: usize,
) -> Option<SyntaxPair> {
    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        // The innermost injection wins, the same rule the tree walk uses: an
        // injection nested inside another is the more specific answer.
        let mut best: Option<(&TreesitterInjectionMatch, &CachedInjection)> = None;
        for (key, entry) in cache.iter() {
            if key.document_hash != document_hash {
                continue;
            }
            if offset < key.byte_start || offset >= key.byte_end {
                continue;
            }
            let Some(content) = text.as_bytes().get(key.byte_start..key.byte_end) else {
                continue;
            };
            if injection_content_hash(content) != key.content_hash {
                continue;
            }
            let width = key.byte_end.saturating_sub(key.byte_start);
            if best.is_none_or(|(best_key, _)| {
                width < best_key.byte_end.saturating_sub(best_key.byte_start)
            }) {
                best = Some((key, entry));
            }
        }
        let (key, entry) = best?;
        let local = offset.checked_sub(key.byte_start)?;
        let content = text.as_bytes().get(key.byte_start..key.byte_end)?;
        let source_ranges_equal =
            |left: Range<usize>, right: Range<usize>| match (content.get(left), content.get(right))
            {
                (Some(left), Some(right)) => left == right,
                _ => false,
            };
        let pair = syntax_pair_in_tree(&entry.tree, local, &source_ranges_equal)?;
        let shift = |range: Range<usize>| {
            range.start.saturating_add(key.byte_start)..range.end.saturating_add(key.byte_start)
        };
        Some(SyntaxPair {
            open: shift(pair.open),
            close: shift(pair.close),
            kind: pair.kind,
        })
    })
}

/// Recreates the injection chain containing `offset` after one of its LRU
/// entries was evicted while the prepared document and token chunks remained
/// cached.
///
/// Pair lookup normally reads the already-tokenized injection tree. A document
/// can outlive the 32-entry injection cache, though, and requesting an old token
/// chunk does not run its injection query again. Re-run just the clicked line's
/// query at each permitted layer, then feed the matching regions through the
/// same cache construction path used by token painting. Walking through cached
/// parents matters too: a parent can survive the LRU while its narrower child
/// was evicted.
pub(crate) fn ensure_injection_chain_cached_for_pair_lookup(
    state: &PreparedSyntaxTreeState,
    offset: usize,
) {
    let Some(highlight) = tree_sitter_highlight_spec(state.language) else {
        return;
    };
    let line_ix = line_ix_for_byte(state.line_starts.as_ref(), offset);
    let matches = collect_treesitter_injection_matches_for_line_window_at(
        &state.tree,
        highlight,
        state.text.as_bytes(),
        state.line_starts.as_ref(),
        line_ix,
        line_ix.saturating_add(1),
        state.source_hash,
        0,
    );
    let Some(injection) = matches
        .singles
        .into_iter()
        .filter(|injection| offset >= injection.byte_start && offset < injection.byte_end)
        .min_by_key(|injection| injection.byte_end.saturating_sub(injection.byte_start))
    else {
        return;
    };
    // `ensure_injection_cached` is normally called inside the host token
    // collector's depth guard. Pair lookup enters the equivalent guards itself
    // so rebuilding a nested entry cannot parse deeper than the configured
    // root-to-injection limit.
    let Some(root_depth_guard) = InjectionDepthGuard::enter() else {
        return;
    };
    let mut depth_guards = vec![root_depth_guard];
    if !ensure_injection_cached(
        state.text.as_bytes(),
        state.line_starts.as_ref(),
        injection,
        0,
    ) {
        return;
    }

    let mut parent = injection;
    for _ in 1..TS_MAX_INJECTION_DEPTH {
        let Some((tree, line_starts)) = TS_INJECTION_CACHE.with(|cache| {
            cache
                .borrow()
                .get(&parent)
                .map(|entry| (entry.tree.clone(), entry.injection_line_starts.clone()))
        }) else {
            break;
        };
        let Some(input) = state
            .text
            .as_bytes()
            .get(parent.byte_start..parent.byte_end)
        else {
            break;
        };
        let Some(highlight) = tree_sitter_highlight_spec(parent.language) else {
            break;
        };
        let local_offset = offset.saturating_sub(parent.byte_start);
        let line_ix = line_ix_for_byte(&line_starts, local_offset);
        let matches = collect_treesitter_injection_matches_for_line_window_at(
            &tree,
            highlight,
            input,
            &line_starts,
            line_ix,
            line_ix.saturating_add(1),
            state.source_hash,
            parent.byte_start,
        );
        let Some(child) = matches
            .singles
            .into_iter()
            .filter(|child| offset >= child.byte_start && offset < child.byte_end)
            .min_by_key(|child| child.byte_end.saturating_sub(child.byte_start))
        else {
            break;
        };
        let Some(child_depth_guard) = InjectionDepthGuard::enter() else {
            break;
        };
        if !ensure_injection_cached(input, &line_starts, child, parent.byte_start) {
            break;
        }
        depth_guards.push(child_depth_guard);
        parent = child;
    }
}

pub(crate) fn collect_injected_tokens_for_parent_line_window(
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    injection: TreesitterInjectionMatch,
    parent_document_byte_start: usize,
) -> Option<Vec<(usize, Vec<SyntaxToken>)>> {
    if !ensure_injection_cached(input, line_starts, injection, parent_document_byte_start) {
        return Some(Vec::new());
    }

    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        let cached = cache.get(&injection)?;

        let injection_end_line_ix = cached
            .injection_start_line_ix
            .saturating_add(cached.all_line_tokens.len());
        let parent_start_line_ix = start_line_ix.max(cached.injection_start_line_ix);
        let parent_end_line_ix = end_line_ix.min(injection_end_line_ix);
        if parent_start_line_ix >= parent_end_line_ix {
            return Some(Vec::new());
        }

        let local_start_line_ix =
            parent_start_line_ix.saturating_sub(cached.injection_start_line_ix);
        let local_end_line_ix = parent_end_line_ix.saturating_sub(cached.injection_start_line_ix);

        let mut mapped_tokens = Vec::with_capacity(local_end_line_ix - local_start_line_ix);
        for local_line_ix in local_start_line_ix..local_end_line_ix {
            let parent_line_ix = cached.injection_start_line_ix.saturating_add(local_line_ix);
            let Some(parent_line_start) = line_starts.get(parent_line_ix).copied() else {
                continue;
            };
            let parent_content_end = line_content_end_byte(line_starts, input, parent_line_ix);
            let parent_line_len = parent_content_end.saturating_sub(parent_line_start);
            let Some(local_line_start) = cached.injection_line_starts.get(local_line_ix).copied()
            else {
                continue;
            };
            let injection_start_in_parent = injection
                .byte_start
                .checked_sub(parent_document_byte_start)?;
            let absolute_line_start = injection_start_in_parent.saturating_add(local_line_start);
            let offset_within_parent = absolute_line_start.saturating_sub(parent_line_start);
            let tokens = cached
                .all_line_tokens
                .get(local_line_ix)
                .cloned()
                .unwrap_or_default();
            let mut remapped = Vec::with_capacity(tokens.len());
            for token in tokens {
                let start = offset_within_parent.saturating_add(token.range.start);
                let end = offset_within_parent
                    .saturating_add(token.range.end)
                    .min(parent_line_len);
                if start >= end || start >= parent_line_len {
                    continue;
                }
                remapped.push(SyntaxToken {
                    range: start..end,
                    kind: token.kind,
                });
            }
            mapped_tokens.push((parent_line_ix, remapped));
        }

        Some(mapped_tokens)
    })
}

pub(crate) fn subtract_absolute_range_from_document_tokens(
    line_starts: &[usize],
    input: &[u8],
    start_line_ix: usize,
    per_line: &mut [Vec<SyntaxToken>],
    absolute_range: Range<usize>,
) {
    if absolute_range.start >= absolute_range.end || per_line.is_empty() {
        return;
    }

    let first_line_ix = line_ix_for_byte(line_starts, absolute_range.start);
    let last_line_ix = line_ix_for_byte(line_starts, absolute_range.end.saturating_sub(1));
    let visible_end_line_ix = start_line_ix.saturating_add(per_line.len());
    let clipped_start_line_ix = first_line_ix.max(start_line_ix);
    let clipped_end_line_ix = last_line_ix.saturating_add(1).min(visible_end_line_ix);
    if clipped_start_line_ix >= clipped_end_line_ix {
        return;
    }

    for line_ix in clipped_start_line_ix..clipped_end_line_ix {
        let Some(line_start) = line_starts.get(line_ix).copied() else {
            continue;
        };
        let content_end = line_content_end_byte(line_starts, input, line_ix);
        let cut_start = absolute_range
            .start
            .max(line_start)
            .saturating_sub(line_start);
        let cut_end = absolute_range
            .end
            .min(content_end)
            .saturating_sub(line_start);
        if cut_start >= cut_end {
            continue;
        }
        let Some(line_tokens) = per_line.get_mut(line_ix.saturating_sub(start_line_ix)) else {
            continue;
        };
        subtract_relative_range_from_line_tokens(line_tokens, cut_start..cut_end);
    }
}

pub(crate) fn subtract_relative_range_from_line_tokens(
    line_tokens: &mut Vec<SyntaxToken>,
    cut_range: Range<usize>,
) {
    if cut_range.start >= cut_range.end || line_tokens.is_empty() {
        return;
    }

    // Usually a no-op: the caller subtracts once per gap and once per range, so a
    // dense template reaches this ~2R times per chunk while any one line intersects
    // only a couple of those cuts. Rebuilding regardless cost a malloc, a memcpy and
    // a free per miss.
    if line_tokens
        .iter()
        .all(|token| token.range.end <= cut_range.start || token.range.start >= cut_range.end)
    {
        return;
    }

    let mut out = Vec::with_capacity(line_tokens.len().saturating_add(1));
    for token in line_tokens.drain(..) {
        if token.range.end <= cut_range.start || token.range.start >= cut_range.end {
            out.push(token);
            continue;
        }
        if token.range.start < cut_range.start {
            out.push(SyntaxToken {
                range: token.range.start..cut_range.start,
                kind: token.kind,
            });
        }
        if token.range.end > cut_range.end {
            out.push(SyntaxToken {
                range: cut_range.end..token.range.end,
                kind: token.kind,
            });
        }
    }
    *line_tokens = out;
}

/// Subtract every one of `cuts` from `line_tokens` in a single sweep.
///
/// The same result as calling [`subtract_relative_range_from_line_tokens`] once
/// per cut -- a token minus a sequence of ranges is the token minus their union
/// -- at a fraction of the work. Cutting one at a time rebuilds the whole line's
/// vector per intersecting cut *and* leaves more fragments behind for the next
/// cut to re-scan, so an injection painting `k` tokens over one host run cost
/// O(k^2). That is the ordinary shape of a minified inline `<script>`: measured
/// on an 11 KB one-line script, 800 injected tokens took 27.8 ms against 5.4 ms
/// for the same bytes spread over lines.
pub(crate) fn subtract_relative_ranges_from_line_tokens(
    line_tokens: &mut Vec<SyntaxToken>,
    cuts: &mut Vec<Range<usize>>,
) {
    cuts.retain(|cut| cut.start < cut.end);
    if cuts.is_empty() || line_tokens.is_empty() {
        return;
    }
    cuts.sort_unstable_by_key(|cut| cut.start);
    // Merged so the sweep below can advance monotonically through them, and so
    // overlapping captures cannot each split the same token again.
    let mut merged: Vec<Range<usize>> = Vec::with_capacity(cuts.len());
    for cut in cuts.iter() {
        match merged.last_mut() {
            Some(last) if cut.start <= last.end => last.end = last.end.max(cut.end),
            _ => merged.push(cut.clone()),
        }
    }

    // The same cheap miss the single-range form takes: most lines of a template
    // are host text that no injected token touches.
    let hull = merged[0].start..merged[merged.len() - 1].end;
    if line_tokens
        .iter()
        .all(|token| token.range.end <= hull.start || token.range.start >= hull.end)
    {
        return;
    }

    let mut out = Vec::with_capacity(line_tokens.len().saturating_add(merged.len()));
    for token in line_tokens.drain(..) {
        // First cut that could reach this token. Tokens are not required to be
        // sorted, so this is found per token rather than carried along.
        let first = merged.partition_point(|cut| cut.end <= token.range.start);
        let mut cursor = token.range.start;
        for cut in &merged[first..] {
            if cut.start >= token.range.end {
                break;
            }
            if cut.start > cursor {
                out.push(SyntaxToken {
                    range: cursor..cut.start,
                    kind: token.kind,
                });
            }
            cursor = cursor.max(cut.end);
        }
        if cursor < token.range.end {
            out.push(SyntaxToken {
                range: cursor..token.range.end,
                kind: token.kind,
            });
        }
    }
    *line_tokens = out;
}

pub(crate) fn normalize_non_overlapping_tokens(tokens: Vec<SyntaxToken>) -> Vec<SyntaxToken> {
    let tokens = tokens
        .into_iter()
        .filter(|token| token.range.start < token.range.end)
        .collect::<Vec<_>>();
    if tokens.len() <= 1 {
        return tokens;
    }

    let mut boundaries = Vec::with_capacity(tokens.len().saturating_mul(2));
    for token in &tokens {
        boundaries.push(token.range.start);
        boundaries.push(token.range.end);
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    // Convert overlapping captures into non-overlapping segments while preserving
    // tree-sitter's "later capture wins" semantics within each overlapped slice.
    //
    // Every boundary is some token's endpoint, so a segment between two adjacent
    // boundaries is either wholly inside a token or wholly outside it -- there is
    // no partial overlap to resolve. The winner is therefore just the last token
    // in input order that covers the segment.
    //
    // Found by painting the segments *backwards* through the token list and
    // never repainting one that is already owned, with a skip list so a token
    // lying under later ones costs one step rather than its whole width. Asking
    // each segment which token wins instead meant scanning every token per
    // segment -- O(tokens^2), and tokens pile up on one line exactly where
    // injections do: a minified inline `<script>` put 4006 of them on a single
    // line, which took 27.8 ms per chunk build.
    let segment_count = boundaries.len().saturating_sub(1);
    let mut owner: Vec<Option<SyntaxTokenKind>> = vec![None; segment_count];
    // `next_unowned[i]` is the first segment at or after `i` with no owner yet,
    // found through path-compressed jumps. One extra slot so the last segment
    // has somewhere to point.
    let mut next_unowned: Vec<usize> = (0..=segment_count).collect();
    pub(crate) fn find_unowned(next_unowned: &mut [usize], mut ix: usize) -> usize {
        let mut root = ix;
        while next_unowned[root] != root {
            root = next_unowned[root];
        }
        while next_unowned[ix] != root {
            let parent = next_unowned[ix];
            next_unowned[ix] = root;
            ix = parent;
        }
        root
    }

    for token in tokens.iter().rev() {
        let start_ix = boundaries.partition_point(|boundary| *boundary < token.range.start);
        let end_ix = boundaries.partition_point(|boundary| *boundary < token.range.end);
        let mut ix = find_unowned(&mut next_unowned, start_ix.min(segment_count));
        while ix < end_ix {
            owner[ix] = Some(token.kind);
            next_unowned[ix] = ix + 1;
            ix = find_unowned(&mut next_unowned, ix + 1);
        }
    }

    let mut normalized: Vec<SyntaxToken> = Vec::with_capacity(tokens.len());
    for (ix, kind) in owner.into_iter().enumerate() {
        let Some(kind) = kind else {
            continue;
        };
        let (start, end) = (boundaries[ix], boundaries[ix + 1]);
        if start >= end {
            continue;
        }
        if let Some(last) = normalized.last_mut()
            && last.kind == kind
            && last.range.end == start
        {
            last.range.end = end;
            continue;
        }
        normalized.push(SyntaxToken {
            range: start..end,
            kind,
        });
    }

    normalized
}

/// How many injection cache entries there are, grouped by the document they
/// belong to. Used to pin that an incremental reparse leaves the cache one
/// document deep rather than accumulating superseded ones.
#[cfg(test)]
pub(crate) fn injection_cache_occupancy_by_document_hash_for_tests()
-> (usize, std::collections::BTreeMap<u64, usize>) {
    TS_INJECTION_CACHE.with(|cache| {
        let cache = cache.borrow();
        let mut by_hash: std::collections::BTreeMap<u64, usize> = Default::default();
        for key in cache.keys() {
            *by_hash.entry(key.document_hash).or_default() += 1;
        }
        (cache.len(), by_hash)
    })
}

#[cfg(test)]
pub(crate) fn clear_injection_cache_for_tests() {
    TS_INJECTION_CACHE.with(|cache| cache.borrow_mut().clear());
}
