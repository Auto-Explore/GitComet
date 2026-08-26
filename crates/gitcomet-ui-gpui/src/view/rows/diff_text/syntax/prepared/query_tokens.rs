use super::*;

pub(crate) struct DocumentTokenCollectionContext<'a> {
    pub(crate) line_starts: &'a [usize],
    pub(crate) start_line_ix: usize,
    pub(crate) end_line_ix: usize,
    /// Byte offset of this parsed input inside the root prepared document.
    /// Zero for the host tree; an outer injection's absolute start for a nested
    /// tree. Injection cache keys always use root-document coordinates.
    pub(crate) document_byte_start: usize,
    pub(crate) per_line: &'a mut [Vec<SyntaxToken>],
}

pub(crate) fn syntax_tokens_for_line_treesitter(
    text: &str,
    language: DiffSyntaxLanguage,
) -> Option<Vec<SyntaxToken>> {
    let highlight = tree_sitter_highlight_spec(language)?;
    let ts_language = &highlight.ts_language;

    let input_len = text.len();
    let tree = TS_INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.clear();
        input.push_str(text);
        input.push('\n');

        with_ts_parser_parse_result(ts_language, |parser| parser.parse(&*input, None))
    })?;

    let mut tokens: Vec<SyntaxToken> = Vec::new();
    let query_succeeded = catch_treesitter_query_panic(|| {
        TS_INPUT.with(|input| {
            let input = input.borrow();
            let query_pass = TreesitterQueryPass {
                byte_range: 0..input.len(),
                containing_byte_range: None,
            };
            TS_CURSOR.with(|cursor| {
                let mut cursor = cursor.borrow_mut();
                configure_query_cursor(&mut cursor, &query_pass, input.len());
                let mut captures =
                    cursor.captures(&highlight.query, tree.root_node(), input.as_bytes());
                tree_sitter::StreamingIterator::advance(&mut captures);
                while let Some((m, capture_ix)) = captures.get() {
                    let Some(capture) = m.captures.get(*capture_ix) else {
                        tree_sitter::StreamingIterator::advance(&mut captures);
                        continue;
                    };

                    let Some(kind) = highlight
                        .capture_kinds
                        .get(capture.index as usize)
                        .copied()
                        .flatten()
                    else {
                        tree_sitter::StreamingIterator::advance(&mut captures);
                        continue;
                    };

                    let mut range = capture.node.byte_range();
                    range.start = range.start.min(input_len);
                    range.end = range.end.min(input_len);
                    if range.start < range.end {
                        tokens.push(SyntaxToken { range, kind });
                    }

                    tree_sitter::StreamingIterator::advance(&mut captures);
                }
            });
        });
    })
    .is_some();
    if !query_succeeded {
        return None;
    }

    Some(normalize_non_overlapping_tokens(tokens))
}

pub(crate) fn treesitter_document_cache_key(
    language: DiffSyntaxLanguage,
    input: &str,
) -> PreparedSyntaxCacheKey {
    #[cfg(test)]
    TS_DOCUMENT_HASH_COUNT.with(|count| count.set(count.get().saturating_add(1)));

    PreparedSyntaxCacheKey {
        language,
        doc_hash: treesitter_document_hash(language, input),
    }
}

pub(crate) fn prepared_document_source_identity_for_shared_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
    line_count: usize,
) -> Option<PreparedSyntaxSourceIdentity> {
    if !should_prepare_treesitter_document(language, mode, text.len()) {
        return None;
    }

    Some(PreparedSyntaxSourceIdentity {
        language,
        text_ptr: text.as_ptr() as usize,
        text_len: text.len(),
        line_count,
    })
}

pub(crate) fn store_pending_parse_request(
    identity: PreparedSyntaxSourceIdentity,
    request: TreesitterDocumentParseRequest,
) {
    TS_PENDING_PARSE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        if let Some(existing) = requests
            .iter_mut()
            .find(|existing| existing.identity == identity)
        {
            existing.request = request;
            return;
        }
        if requests.len() >= TS_PENDING_PARSE_REQUEST_MAX_ENTRIES {
            requests.remove(0);
        }
        requests.push(PendingParseRequest { identity, request });
    });
}

pub(crate) fn clear_pending_parse_request(identity: PreparedSyntaxSourceIdentity) {
    TS_PENDING_PARSE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        if let Some(pos) = requests
            .iter()
            .position(|existing| existing.identity == identity)
        {
            requests.remove(pos);
        }
    });
}

pub(crate) fn take_pending_parse_request_for_shared_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: &str,
    line_starts: &[usize],
) -> Option<TreesitterDocumentParseRequest> {
    let normalized_line_starts = normalized_treesitter_line_starts(text, line_starts);
    let identity = prepared_document_source_identity_for_shared_text(
        language,
        mode,
        text,
        normalized_line_starts.len(),
    )?;

    TS_PENDING_PARSE_REQUESTS.with(|requests| {
        let mut requests = requests.borrow_mut();
        let pos = requests
            .iter()
            .position(|existing| existing.identity == identity)?;
        let request = requests.remove(pos).request;
        let text_matches =
            request.input.text.as_ptr() == text.as_ptr() && request.input.text.len() == text.len();
        if text_matches && request.input.line_starts.as_ref() == normalized_line_starts {
            return Some(request);
        }
        None
    })
}

pub(crate) fn treesitter_text_hash(input: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    input.hash(&mut hasher);
    hasher.finish()
}

/// Identity used by prepared trees and every injection derived from them.
///
/// `PreparedSyntaxCacheKey` already stores the host language separately for
/// cache lookup, but injection entries retain only this hash. Folding the host
/// language in here prevents identical bytes parsed as, for example, HTML and
/// Vue from sharing an injection identity.
pub(crate) fn treesitter_document_hash(language: DiffSyntaxLanguage, input: &str) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = FxHasher::default();
    language.hash(&mut hasher);
    treesitter_text_hash(input).hash(&mut hasher);
    hasher.finish()
}

pub(crate) fn collect_treesitter_document_line_tokens_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
) -> Vec<Vec<SyntaxToken>> {
    collect_treesitter_document_line_tokens_for_line_window_at(
        tree,
        highlight,
        input,
        line_starts,
        start_line_ix,
        end_line_ix,
        document_hash,
        0,
    )
}

pub(crate) fn collect_treesitter_document_line_tokens_for_line_window_at(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
    document_byte_start: usize,
) -> Vec<Vec<SyntaxToken>> {
    if line_starts.is_empty() {
        return Vec::new();
    }
    let end_line_ix = end_line_ix.min(line_starts.len());
    if start_line_ix >= end_line_ix {
        return Vec::new();
    }

    let mut per_line: Vec<Vec<SyntaxToken>> = vec![Vec::new(); end_line_ix - start_line_ix];
    let query_passes = treesitter_document_query_passes_for_line_window(
        line_starts,
        input.len(),
        start_line_ix,
        end_line_ix,
    );
    {
        let mut context = DocumentTokenCollectionContext {
            line_starts,
            start_line_ix,
            end_line_ix,
            document_byte_start,
            per_line: &mut per_line,
        };
        for pass in &query_passes {
            collect_query_pass_tokens_for_document(tree, highlight, input, pass, &mut context);
        }
        apply_injection_query_tokens_for_document(
            tree,
            highlight,
            input,
            document_hash,
            &mut context,
        );
    }

    for line_tokens in &mut per_line {
        let normalized = normalize_non_overlapping_tokens(std::mem::take(line_tokens));
        *line_tokens = normalized;
    }
    per_line
}

pub(crate) fn line_ix_for_byte(line_starts: &[usize], byte: usize) -> usize {
    match line_starts.binary_search(&byte) {
        Ok(ix) => ix,
        Err(0) => 0,
        Err(ix) => ix - 1,
    }
}

pub(crate) fn clamp_query_range(range: Range<usize>, input_len: usize) -> Range<usize> {
    let start = range.start.min(input_len);
    let end = range.end.min(input_len).max(start);
    start..end
}

pub(crate) fn configure_query_cursor(
    cursor: &mut tree_sitter::QueryCursor,
    pass: &TreesitterQueryPass,
    input_len: usize,
) {
    cursor.set_match_limit(TS_QUERY_MATCH_LIMIT);
    cursor.set_byte_range(clamp_query_range(pass.byte_range.clone(), input_len));
    match &pass.containing_byte_range {
        Some(range) => {
            cursor.set_containing_byte_range(clamp_query_range(range.clone(), input_len));
        }
        None => {
            cursor.set_containing_byte_range(0..usize::MAX);
        }
    }
}

/// Byte offset where the region for line `line_ix` ends (start of next line, or `input_len`).
/// Replaces the old `line_query_end_byte(line_starts[i], line_lengths[i], input_len)`.
pub(crate) fn line_region_end_byte(
    line_starts: &[usize],
    input_len: usize,
    line_ix: usize,
) -> usize {
    line_starts
        .get(line_ix + 1)
        .copied()
        .unwrap_or(input_len)
        .min(input_len)
}

/// Content-end byte offset for line `line_ix` (excludes a trailing `\n` when present).
pub(crate) fn line_content_end_byte(line_starts: &[usize], input: &[u8], line_ix: usize) -> usize {
    let region_end = line_region_end_byte(line_starts, input.len(), line_ix);
    if input.get(region_end.saturating_sub(1)) == Some(&b'\n') {
        region_end.saturating_sub(1)
    } else {
        region_end
    }
}

pub(crate) fn treesitter_document_query_passes_for_line_window(
    line_starts: &[usize],
    input_len: usize,
    start_line_ix: usize,
    end_line_ix: usize,
) -> Vec<TreesitterQueryPass> {
    if input_len == 0 || line_starts.is_empty() {
        return Vec::new();
    }
    let end_line_ix = end_line_ix.min(line_starts.len());
    if start_line_ix >= end_line_ix {
        return Vec::new();
    }

    let window_start_byte = line_starts[start_line_ix].min(input_len);
    let window_end_byte = line_region_end_byte(line_starts, input_len, end_line_ix - 1);
    if window_start_byte >= window_end_byte {
        return Vec::new();
    }

    let window_bytes = window_end_byte.saturating_sub(window_start_byte);

    if window_bytes <= TS_MAX_BYTES_TO_QUERY {
        return vec![TreesitterQueryPass {
            byte_range: window_start_byte..window_end_byte,
            containing_byte_range: None,
        }];
    }

    let mut passes = Vec::new();
    let mut line_ix = start_line_ix;
    while line_ix < end_line_ix {
        let line_start = line_starts[line_ix].min(input_len);
        let line_end = line_region_end_byte(line_starts, input_len, line_ix);
        let line_bytes = line_end.saturating_sub(line_start);

        if line_bytes > TS_MAX_BYTES_TO_QUERY {
            let mut chunk_start = line_start;
            while chunk_start < line_end {
                let chunk_end = chunk_start
                    .saturating_add(TS_MAX_BYTES_TO_QUERY)
                    .min(line_end);
                passes.push(TreesitterQueryPass {
                    byte_range: chunk_start..chunk_end,
                    containing_byte_range: Some(chunk_start..chunk_end),
                });
                chunk_start = chunk_end;
            }
            line_ix = line_ix.saturating_add(1);
            continue;
        }

        let window_start_line = line_ix;
        let window_start = line_start;
        let mut window_end_line = line_ix;
        let mut window_end = line_end;

        while window_end_line + 1 < end_line_ix
            && (window_end_line - window_start_line + 1) < TS_QUERY_MAX_LINES_PER_PASS
        {
            let next_line_ix = window_end_line + 1;
            let next_line_end = line_region_end_byte(line_starts, input_len, next_line_ix);
            let candidate_end = window_end.max(next_line_end);
            let candidate_bytes = candidate_end.saturating_sub(window_start);
            if candidate_bytes > TS_MAX_BYTES_TO_QUERY {
                break;
            }
            window_end = candidate_end;
            window_end_line = next_line_ix;
        }

        passes.push(TreesitterQueryPass {
            byte_range: window_start..window_end,
            containing_byte_range: None,
        });
        line_ix = window_end_line.saturating_add(1);
    }

    if passes.is_empty() {
        return vec![TreesitterQueryPass {
            byte_range: window_start_byte..window_end_byte,
            containing_byte_range: None,
        }];
    }

    passes
}

pub(crate) fn collect_query_pass_tokens_for_document(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    pass: &TreesitterQueryPass,
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    catch_treesitter_query_panic(|| {
        TS_CURSOR.with(|cursor| {
            let mut cursor = cursor.borrow_mut();
            configure_query_cursor(&mut cursor, pass, input.len());
            let pass_range = clamp_query_range(pass.byte_range.clone(), input.len());
            let mut captures = cursor.captures(&highlight.query, tree.root_node(), input);
            tree_sitter::StreamingIterator::advance(&mut captures);
            while let Some((m, capture_ix)) = captures.get() {
                let Some(capture) = m.captures.get(*capture_ix) else {
                    tree_sitter::StreamingIterator::advance(&mut captures);
                    continue;
                };
                let Some(kind) = highlight
                    .capture_kinds
                    .get(capture.index as usize)
                    .copied()
                    .flatten()
                else {
                    tree_sitter::StreamingIterator::advance(&mut captures);
                    continue;
                };

                let mut byte_range = capture.node.byte_range();
                byte_range.start = byte_range.start.min(input.len());
                byte_range.end = byte_range.end.min(input.len());
                byte_range.start = byte_range.start.max(pass_range.start);
                byte_range.end = byte_range.end.min(pass_range.end);
                if byte_range.start >= byte_range.end {
                    tree_sitter::StreamingIterator::advance(&mut captures);
                    continue;
                }

                let mut line_ix = line_ix_for_byte(context.line_starts, byte_range.start);
                if line_ix < context.start_line_ix {
                    line_ix = context.start_line_ix;
                }
                while line_ix < context.end_line_ix && line_ix < context.line_starts.len() {
                    let line_start = context.line_starts[line_ix];
                    let line_end = line_content_end_byte(context.line_starts, input, line_ix);
                    let token_start = byte_range.start.max(line_start);
                    let token_end = byte_range.end.min(line_end);
                    if token_start < token_end {
                        context.per_line[line_ix - context.start_line_ix].push(SyntaxToken {
                            range: (token_start - line_start)..(token_end - line_start),
                            kind,
                        });
                    }
                    if byte_range.end <= line_end {
                        break;
                    }
                    line_ix = line_ix.saturating_add(1);
                }

                tree_sitter::StreamingIterator::advance(&mut captures);
            }
        });
    });
}

pub(crate) struct InjectionDepthGuard(usize);

impl InjectionDepthGuard {
    pub(crate) fn enter() -> Option<Self> {
        let depth = TS_INJECTION_DEPTH.with(|d| d.get());
        if depth >= TS_MAX_INJECTION_DEPTH {
            return None;
        }
        TS_INJECTION_DEPTH.with(|d| d.set(depth + 1));
        Some(Self(depth))
    }
}

impl Drop for InjectionDepthGuard {
    fn drop(&mut self) {
        TS_INJECTION_DEPTH.with(|d| d.set(self.0));
    }
}

pub(crate) fn apply_injection_query_tokens_for_document(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    document_hash: u64,
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    let Some(_guard) = InjectionDepthGuard::enter() else {
        return;
    };
    let injections = collect_treesitter_injection_matches_for_line_window_at(
        tree,
        highlight,
        input,
        context.line_starts,
        context.start_line_ix,
        context.end_line_ix,
        document_hash,
        context.document_byte_start,
    );
    for injection in &injections.singles {
        let injection = *injection;
        let Some(injected_tokens) = collect_injected_tokens_for_parent_line_window(
            input,
            context.line_starts,
            context.start_line_ix,
            context.end_line_ix,
            injection,
            context.document_byte_start,
        ) else {
            continue;
        };

        // Subtract only what the injection actually paints, not its whole span.
        //
        // Blanking the span outright loses any host capture the injection has no
        // opinion about: a markdown heading is `@text.title` in the block
        // grammar, and the inline grammar that owns those bytes captures nothing
        // for plain prose, so the heading came out uncoloured. Cutting per
        // painted token keeps last-wins where the two overlap and leaves the
        // host's answer standing in the gaps.
        // Reused across lines so a document full of fenced blocks does not
        // allocate a cut list per line.
        let mut cuts: Vec<Range<usize>> = Vec::new();
        for (parent_line_ix, tokens) in &injected_tokens {
            // Same window guard as the append loop below: `saturating_sub` on a
            // line below the window would clamp to index 0 and subtract these
            // ranges out of the first visible row's host tokens instead.
            if *parent_line_ix < context.start_line_ix {
                continue;
            }
            // The tokens are already grouped by line and their ranges are
            // already line-relative, so going back through the absolute form
            // would re-derive by binary search what is known here.
            let Some(line_tokens) = context
                .per_line
                .get_mut(parent_line_ix.saturating_sub(context.start_line_ix))
            else {
                continue;
            };
            // One merged sweep rather than one rebuild of `line_tokens` per
            // injected token -- see
            // [`subtract_relative_ranges_from_line_tokens`].
            cuts.clear();
            cuts.extend(tokens.iter().map(|token| token.range.clone()));
            subtract_relative_ranges_from_line_tokens(line_tokens, &mut cuts);
        }

        for (parent_line_ix, tokens) in injected_tokens {
            if tokens.is_empty() || parent_line_ix < context.start_line_ix {
                continue;
            }
            let Some(line_tokens) = context
                .per_line
                .get_mut(parent_line_ix.saturating_sub(context.start_line_ix))
            else {
                continue;
            };
            line_tokens.extend(tokens);
        }
    }

    // Combined last, and the order matters once a grammar declares both kinds over
    // the same bytes. Each layer clears what it is about to paint out of
    // `per_line` first -- per painted token for a single, over the whole span for
    // a combined group -- so the last one wins the overlap; running combined
    // first let a single delete combined tokens and then repaint only the bytes
    // its own captures cover, leaving the rest bare. No in-tree grammar mixes
    // them yet.
    if !injections.truncated {
        for group in &injections.combined {
            apply_combined_injection_tokens(group, input, document_hash, context);
        }
    }
}

/// Parses `ranges` as one document with `spec`'s grammar, leaving node offsets in
/// *document* coordinates.
///
/// `set_included_ranges` is what buys that: the parser reads the whole input but
/// only builds nodes inside the ranges, so no coordinate remapping is needed on
/// the way out -- unlike the per-injection path, which slices the text and maps
/// injection-local offsets back in `collect_injected_tokens_for_parent_line_window`.
///
/// The ranges are cleared again on every exit path. `TS_PARSER` is pooled, its
/// included ranges are sticky, and `with_ts_parser`'s fast path can skip
/// `set_language` entirely -- leaving them set would silently truncate the next
/// root parse on this thread, for any language. `IncludedRangesGuard` makes that
/// unconditional rather than relying on each `return` remembering.
pub(crate) fn parse_combined_injection_tree(
    spec: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    ranges: &[Range<usize>],
) -> Option<tree_sitter::Tree> {
    // Never call `set_included_ranges` with an empty slice: that is tree-sitter's
    // reset to "the whole document", so a window whose group collapsed to nothing
    // would highlight the entire file with the injected grammar.
    if ranges.is_empty() {
        return None;
    }
    let ts_ranges = ranges
        .iter()
        .map(|range| tree_sitter::Range {
            start_byte: range.start,
            end_byte: range.end,
            start_point: treesitter_point_for_byte(line_starts, input, range.start),
            end_point: treesitter_point_for_byte(line_starts, input, range.end),
        })
        .collect::<Vec<_>>();

    with_ts_parser_parse_result(&spec.ts_language, |parser| {
        let mut guard = IncludedRangesGuard::set(parser, &ts_ranges)?;
        parse_treesitter_tree(guard.parser(), input, None, None)
    })
}

/// Clears a parser's included ranges on drop.
pub(crate) struct IncludedRangesGuard<'parser>(&'parser mut tree_sitter::Parser);

impl<'parser> IncludedRangesGuard<'parser> {
    pub(crate) fn set(
        parser: &'parser mut tree_sitter::Parser,
        ranges: &[tree_sitter::Range],
    ) -> Option<Self> {
        if parser.set_included_ranges(ranges).is_err() {
            let _ = parser.set_included_ranges(&[]);
            return None;
        }
        Some(Self(parser))
    }

    pub(crate) fn parser(&mut self) -> &mut tree_sitter::Parser {
        self.0
    }
}

impl Drop for IncludedRangesGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.set_included_ranges(&[]);
    }
}

/// The byte span a combined layer is parsed over for a given window.
///
/// The rendered window, widened by [`TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES`]
/// on each side so a construct straddling the window edge is still parsed whole.
/// See that constant for why the margin is not optional.
pub(crate) fn combined_injection_clip_region(
    line_starts: &[usize],
    input_len: usize,
    start_line_ix: usize,
    end_line_ix: usize,
) -> Range<usize> {
    let window_start = line_starts.get(start_line_ix).copied().unwrap_or(0);
    let window_end = line_region_end_byte(line_starts, input_len, end_line_ix.saturating_sub(1));
    let clip_start = window_start.saturating_sub(TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES);
    let clip_end = window_end
        .saturating_add(TS_COMBINED_INJECTION_CONTEXT_MARGIN_BYTES)
        .min(input_len);
    clip_start..clip_end.max(clip_start)
}

/// `ranges` is already sorted and non-overlapping, and clipping preserves both, so
/// the result needs no re-merge.
pub(crate) fn clip_injection_ranges_to_region(
    ranges: &[Range<usize>],
    region: &Range<usize>,
) -> Vec<Range<usize>> {
    ranges
        .iter()
        .filter_map(|range| {
            let start = range.start.max(region.start);
            let end = range.end.min(region.end);
            (start < end).then_some(start..end)
        })
        .collect()
}

/// Parses one combined group and splices its tokens into `context.per_line`.
pub(crate) fn apply_combined_injection_tokens(
    group: &CombinedInjectionGroup,
    input: &[u8],
    document_hash: u64,
    context: &mut DocumentTokenCollectionContext<'_>,
) {
    let Some(spec) = tree_sitter_highlight_spec(group.language) else {
        return;
    };

    // Clip before the ceilings are applied, not after; see their doc comment.
    let clip_region = combined_injection_clip_region(
        context.line_starts,
        input.len(),
        context.start_line_ix,
        context.end_line_ix,
    );
    let ranges = clip_injection_ranges_to_region(&group.ranges, &clip_region);
    if ranges.is_empty() {
        return;
    }
    if ranges.len() > TS_COMBINED_INJECTION_MAX_RANGES {
        return;
    }
    let total_bytes: usize = ranges
        .iter()
        .map(|range| range.end.saturating_sub(range.start))
        .sum();
    if total_bytes > TS_COMBINED_INJECTION_MAX_BYTES {
        return;
    }
    let Some(tree) = parse_combined_injection_tree(spec, input, context.line_starts, &ranges)
    else {
        return;
    };

    let mut injected = collect_treesitter_document_line_tokens_for_line_window_at(
        &tree,
        spec,
        input,
        context.line_starts,
        context.start_line_ix,
        context.end_line_ix,
        document_hash,
        context.document_byte_start,
    );
    if injected.len() != context.per_line.len() {
        return;
    }

    // Clip the injected tokens back to the ranges the injected grammar actually
    // owns; see `combined_injection_gaps`.
    let window_start = context
        .line_starts
        .get(context.start_line_ix)
        .copied()
        .unwrap_or(0);
    let window_end = line_region_end_byte(
        context.line_starts,
        input.len(),
        context.end_line_ix.saturating_sub(1),
    );
    for gap in combined_injection_gaps(window_start..window_end, &ranges) {
        subtract_absolute_range_from_document_tokens(
            context.line_starts,
            input,
            context.start_line_ix,
            &mut injected,
            gap,
        );
    }

    // ... and drop the host grammar's tokens from the bytes the injection took over.
    for range in &ranges {
        subtract_absolute_range_from_document_tokens(
            context.line_starts,
            input,
            context.start_line_ix,
            context.per_line,
            range.clone(),
        );
    }

    for (line_tokens, tokens) in context.per_line.iter_mut().zip(injected) {
        line_tokens.extend(tokens);
    }
}

/// Every match of one `(#set! injection.combined)` pattern, gathered into the
/// single layer the directive asks for.
///
/// `ranges` is sorted, non-overlapping and non-empty by the time this leaves
/// [`collect_treesitter_injection_matches_for_line_window`] -- all three are hard
/// requirements of `Parser::set_included_ranges`, and an *empty* slice is
/// tree-sitter's "reset to the whole document" rather than "parse nothing".
///
/// The key is `(language, pattern_index)` with no host-node identity, matching
/// upstream tree-sitter's semantics. The cost is that unrelated matches of one
/// pattern share a document: `buildPhase` and `installPhase` in the same chunk parse
/// as one bash script, so an unterminated `if` in the first recolours the second.
///
/// Do not "fix" that by sub-grouping on the parent node. Nix `(string_fragment)`s do
/// sit under their own `indented_string_expression`, but Jinja `(text)` nodes are
/// children of whatever encloses them (`source_file`, `for_statement`,
/// `if_statement`), so a parent key would shatter one template's HTML into a
/// document per block. The Nix case needs a per-pattern key.
pub(crate) struct CombinedInjectionGroup {
    pub(crate) language: DiffSyntaxLanguage,
    pub(crate) ranges: Vec<Range<usize>>,
}

pub(crate) struct TreesitterInjectionMatches {
    /// One layer per match, the pre-existing behaviour. Unchanged for every
    /// grammar that does not declare `injection.combined`.
    pub(crate) singles: Vec<TreesitterInjectionMatch>,
    pub(crate) combined: Vec<CombinedInjectionGroup>,
    /// The query cursor overflowed `TS_QUERY_MATCH_LIMIT` somewhere in this
    /// window, so tree-sitter silently dropped matches.
    ///
    /// Only the combined groups act on this. Losing one *single* injection costs
    /// that node its highlighting and nothing else, which is the status quo;
    /// losing one range out of a combined set changes the document the injected
    /// grammar sees, so an unbalanced `<div>` can wreck the whole window. Better
    /// to leave the host grammar painting.
    pub(crate) truncated: bool,
}

#[cfg(test)]
pub(crate) fn collect_treesitter_injection_matches_for_line_window(
    tree: &tree_sitter::Tree,
    highlight: &TreesitterHighlightSpec,
    input: &[u8],
    line_starts: &[usize],
    start_line_ix: usize,
    end_line_ix: usize,
    document_hash: u64,
) -> TreesitterInjectionMatches {
    collect_treesitter_injection_matches_for_line_window_at(
        tree,
        highlight,
        input,
        line_starts,
        start_line_ix,
        end_line_ix,
        document_hash,
        0,
    )
}
