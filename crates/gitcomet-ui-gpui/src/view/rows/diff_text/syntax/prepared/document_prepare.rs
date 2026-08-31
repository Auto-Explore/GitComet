use super::*;

pub(crate) fn should_apply_chunk_build_result(
    result: &PreparedSyntaxChunkBuildResult,
    current_thread: std::thread::ThreadId,
    target_cache_key: Option<PreparedSyntaxCacheKey>,
) -> bool {
    result.thread_id == current_thread
        && match target_cache_key {
            Some(cache_key) => result.chunk_key.cache_key == cache_key,
            None => true,
        }
}

#[derive(Clone)]
pub(crate) struct TreesitterDocumentInput {
    pub(crate) text: SharedString,
    pub(crate) line_starts: Arc<[usize]>,
}

#[derive(Clone)]
pub(crate) struct TreesitterDocumentParseRequest {
    pub(crate) language: DiffSyntaxLanguage,
    pub(crate) ts_language: tree_sitter::Language,
    pub(crate) input: TreesitterDocumentInput,
    pub(crate) cache_key: PreparedSyntaxCacheKey,
}

pub(crate) struct PendingParseRequest {
    pub(crate) identity: PreparedSyntaxSourceIdentity,
    pub(crate) request: TreesitterDocumentParseRequest,
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn benchmark_reset_prepared_syntax_cache_metrics() {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().reset_metrics());
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn benchmark_prepared_syntax_cache_metrics() -> (u64, u64, u64, u64) {
    TS_DOCUMENT_CACHE.with(|cache| {
        let metrics = cache.borrow().metrics();
        (
            metrics.hit,
            metrics.miss,
            metrics.evict,
            metrics.chunk_build_ms,
        )
    })
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn benchmark_prepared_syntax_loaded_chunk_count(
    document: PreparedSyntaxDocument,
) -> Option<usize> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().loaded_chunk_count(document.cache_key))
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_prepared_syntax_cache_contains_document(
    document: PreparedSyntaxDocument,
) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().contains_key(document.cache_key))
}

#[cfg(test)]
pub(crate) fn prepared_syntax_cache_metrics() -> PreparedSyntaxCacheMetrics {
    let (hit, miss, evict, chunk_build_ms) = benchmark_prepared_syntax_cache_metrics();
    PreparedSyntaxCacheMetrics {
        hit,
        miss,
        evict,
        chunk_build_ms,
    }
}

#[cfg(test)]
pub(crate) fn reset_prepared_syntax_cache() {
    TS_DOCUMENT_CACHE.with(|cache| {
        *cache.borrow_mut() = TreesitterDocumentCache::new();
    });
    TS_PENDING_PARSE_REQUESTS.with(|requests| requests.borrow_mut().clear());
    let mut store = match shared_prepared_document_seed_store().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    store.clear();
}

#[cfg(test)]
pub(crate) fn prepared_syntax_loaded_chunk_count(document: PreparedSyntaxDocument) -> usize {
    benchmark_prepared_syntax_loaded_chunk_count(document).unwrap_or_default()
}

pub(crate) fn prepare_treesitter_document_with_budget_reuse_text(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    budget: DiffSyntaxBudget,
    old_document: Option<PreparedSyntaxDocument>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> PrepareTreesitterDocumentResult {
    // Before the source-identity fast path: the specs this language injects into
    // are needed later, by the render path, whether or not this document is
    // already cached.
    request_highlight_spec_warmup(language);
    let line_count = normalized_treesitter_line_starts(text.as_ref(), line_starts.as_ref()).len();
    if let Some(identity) =
        prepared_document_source_identity_for_shared_text(language, mode, text.as_ref(), line_count)
        && let Some(document) = TS_DOCUMENT_CACHE
            .with(|cache| cache.borrow_mut().document_for_source_identity(identity))
    {
        return PrepareTreesitterDocumentResult::Ready(document);
    }
    let source_identity = prepared_document_source_identity_for_shared_text(
        language,
        mode,
        text.as_ref(),
        line_count,
    );
    let old_tree_state = old_document.and_then(prepared_document_tree_state);
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    let reparse_plan = old_tree_state.as_ref().and_then(|state| {
        build_treesitter_reparse_plan(state, language, &input, edit_hint.as_ref())
    });
    if let (Some(document), Some(identity), Some(TreesitterReparsePlan::Unchanged)) =
        (old_document, source_identity, reparse_plan.as_ref())
    {
        TS_DOCUMENT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .alias_source_identity(document.cache_key, identity);
        });
        clear_pending_parse_request(identity);
        return PrepareTreesitterDocumentResult::Ready(document);
    }
    let Some(request) = treesitter_document_parse_request_from_input_with_reuse(
        language,
        mode,
        input,
        old_tree_state.as_ref(),
        reparse_plan.as_ref(),
    ) else {
        if let Some(identity) = source_identity {
            clear_pending_parse_request(identity);
        }
        return PrepareTreesitterDocumentResult::Unsupported;
    };
    let has_cache_hit = TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .contains_document(request.cache_key, line_count)
    });
    if has_cache_hit {
        if let Some(identity) = source_identity {
            clear_pending_parse_request(identity);
        }
        return PrepareTreesitterDocumentResult::Ready(PreparedSyntaxDocument {
            cache_key: request.cache_key,
        });
    }

    let result = prepare_treesitter_document_request_after_cache_lookup(
        request.clone(),
        Some(budget),
        old_document,
        edit_hint.is_some(),
        reparse_plan,
    );
    match (source_identity, result) {
        (Some(identity), PrepareTreesitterDocumentResult::TimedOut) => {
            store_pending_parse_request(identity, request);
        }
        (Some(identity), _) => {
            clear_pending_parse_request(identity);
        }
        (None, _) => {}
    }
    result
}

#[cfg(test)]
pub(crate) fn prepare_treesitter_document_in_background_text_with_reuse(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    old_document: Option<PreparedSyntaxDocument>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> Option<PreparedSyntaxDocumentData> {
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    let old_tree_state = old_document.and_then(prepared_document_tree_state);
    let reparse_plan = old_tree_state.as_ref().and_then(|state| {
        build_treesitter_reparse_plan(state, language, &input, edit_hint.as_ref())
    });
    let request = take_pending_parse_request_for_shared_text(
        language,
        mode,
        input.text.as_ref(),
        input.line_starts.as_ref(),
    )
    .or_else(|| {
        treesitter_document_parse_request_from_input_with_reuse(
            language,
            mode,
            input,
            old_tree_state.as_ref(),
            reparse_plan.as_ref(),
        )
    })?;
    prepare_treesitter_document_data_request_impl(request, old_document, reparse_plan)
}

pub(crate) fn prepare_treesitter_document_in_background_text_with_reparse_seed(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text: SharedString,
    line_starts: Arc<[usize]>,
    reparse_seed: Option<PreparedSyntaxReparseSeed>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> Option<PreparedSyntaxDocumentData> {
    request_highlight_spec_warmup(language);
    let input = treesitter_document_input_from_shared_text(text, line_starts);
    let (old_document, old_tree_state) = match reparse_seed {
        Some(seed) => (Some(seed.document), Some(seed.tree_state)),
        None => (None, None),
    };
    let reparse_plan = old_tree_state.as_ref().and_then(|state| {
        build_treesitter_reparse_plan(state, language, &input, edit_hint.as_ref())
    });
    let request = take_pending_parse_request_for_shared_text(
        language,
        mode,
        input.text.as_ref(),
        input.line_starts.as_ref(),
    )
    .or_else(|| {
        treesitter_document_parse_request_from_input_with_reuse(
            language,
            mode,
            input,
            old_tree_state.as_ref(),
            reparse_plan.as_ref(),
        )
    })?;
    prepare_treesitter_document_data_request_impl(request, old_document, reparse_plan)
}

pub(crate) fn inject_prepared_document_data(
    document: PreparedSyntaxDocumentData,
) -> PreparedSyntaxDocument {
    store_shared_prepared_document_seed(&document);
    TS_DOCUMENT_CACHE.with(|cache| {
        cache.borrow_mut().insert_document_with_mode(
            document.cache_key,
            TreesitterCachedDocument::from_chunked_line_tokens(
                document.line_count,
                document.line_token_chunks,
                document.tree_state,
            ),
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    });
    PreparedSyntaxDocument {
        cache_key: document.cache_key,
    }
}

#[cfg(test)]
pub(crate) fn syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<Vec<SyntaxToken>> {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().line_tokens(document.cache_key, line_ix))
}

pub(crate) fn request_syntax_tokens_for_prepared_document_line(
    document: PreparedSyntaxDocument,
    line_ix: usize,
) -> Option<PreparedSyntaxLineTokensRequest> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .request_line_tokens(document.cache_key, line_ix)
    })
}

pub(crate) fn request_syntax_tokens_for_prepared_document_line_range_into(
    document: PreparedSyntaxDocument,
    line_range: Range<usize>,
    requests: &mut Vec<PreparedSyntaxLineTokensRequest>,
) -> Option<PreparedSyntaxLineTokensRangeSummary> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .request_line_tokens_range_into(document.cache_key, line_range, requests)
    })
}

pub(crate) fn drain_completed_prepared_syntax_chunk_builds() -> usize {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow_mut().drain_completed_chunk_builds())
}

pub(crate) fn has_pending_prepared_syntax_chunk_builds() -> bool {
    TS_DOCUMENT_CACHE.with(|cache| cache.borrow().has_pending_chunk_requests())
}

pub(crate) fn drain_completed_prepared_syntax_chunk_builds_for_document(
    document: PreparedSyntaxDocument,
) -> usize {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .drain_completed_chunk_builds_for_cache_key(document.cache_key)
    })
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn has_pending_prepared_syntax_chunk_builds_for_document(
    document: PreparedSyntaxDocument,
) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow()
            .has_pending_chunk_requests_for_cache_key(document.cache_key)
    })
}

pub(crate) fn prepared_document_tree_state(
    document: PreparedSyntaxDocument,
) -> Option<PreparedSyntaxTreeState> {
    TS_DOCUMENT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(tree_state) = cache.tree_state(document.cache_key) {
            return Some(tree_state);
        }
        cache.merge_document_from_shared_seed(document.cache_key);
        cache.tree_state(document.cache_key)
    })
}

/// Makes an opaque handle usable on this thread, if its shared parse seed is
/// still retained.
///
/// View-level handle caches outlive the small thread-local tree cache. A caller
/// must use this check before treating a handle hit as a ready document; a
/// missing shared seed then falls through to the normal parse/worker path.
pub(crate) fn prepared_syntax_document_is_available(document: PreparedSyntaxDocument) -> bool {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state_is_available(document.cache_key)
    })
}

/// One end of a matched pair, in the coordinate space the row canvases speak:
/// a document line index plus a *display* byte range within that line.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedSyntaxPairSpan {
    pub(in crate::view) line_ix: usize,
    pub(in crate::view) display_range: Range<usize>,
}

/// A matched pair on a prepared document, already projected into line space.
///
/// Each end is a *list* of spans, one per line it covers: a start tag written
/// across several lines (`<div\n  class="card">`, ordinary in HTML and JSX) is
/// one delimiter but several rows, and reporting only its first line would wash
/// `<div` and leave the rest bare.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedSyntaxPairHit {
    /// Which construct the pair delimits. Test-only for the same reason as
    /// [`crate::view::DiffTextPairMatch::kind`]: every kind is painted alike, so
    /// this exists to let an assertion say which pair was found, not to steer
    /// anything.
    #[cfg(test)]
    pub(in crate::view) kind: SyntaxPairKind,
    pub(in crate::view) open: Vec<PreparedSyntaxPairSpan>,
    pub(in crate::view) close: Vec<PreparedSyntaxPairSpan>,
}

/// One line's byte range, without its line terminator.
///
/// The newline belongs to the line's bytes but never to its display columns, so
/// it is trimmed before any offset is measured against the line.
///
/// Every bound goes through `text.get`, so a `line_starts` that does not
/// describe `text` answers `None` instead of panicking. That is not
/// hypothetical: the click path can index a side whose file was re-read after
/// the diff was built, and a file that shrank in between leaves starts past the
/// end of the text.
pub(crate) fn prepared_line_span(
    text: &str,
    line_starts: &[usize],
    ix: usize,
) -> Option<Range<usize>> {
    let start = *line_starts.get(ix)?;
    let end = line_starts
        .get(ix + 1)
        .copied()
        .unwrap_or(text.len())
        .min(text.len());
    let line = text.get(start..end)?;
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    Some(start..start + line.len())
}

/// The matching pair at a click, taking and returning the canvases' own
/// coordinates.
///
/// The whole display/raw tab conversion lives here rather than at the call site
/// because this is the only place that holds both halves of it: the tree indexes
/// raw bytes, the canvases count tab-expanded columns, and
/// [`PreparedSyntaxTreeState`] carries the `text` and `line_starts` that relate
/// them. The view layer therefore never handles a document byte offset.
///
/// Returns `None` when the document has no retained tree (it was evicted, or the
/// parse never finished), when `line_ix` is past the end, when the click landed
/// at a caret boundary beyond the line, or when nothing pairs. Geometric clicks
/// in trailing blank space are rejected by the view before reaching this API.
///
/// Injections *are* consulted, and first -- see [`injected_syntax_pair_at`]. The
/// injected region's own tree is kept for exactly this, because to the host
/// grammar an injected body is one opaque leaf: a delimiter inside it matches
/// nothing, and the walk falls out to the enclosing element. That was the whole
/// bug -- clicking the `<` of a `<html>` tag in a PHP file did nothing.
/// Combined injections stay out of it, and the host tree remains the fallback.
pub(in crate::view) fn prepared_document_syntax_pair_at_display_offset(
    document: PreparedSyntaxDocument,
    line_ix: usize,
    display_offset: usize,
) -> Option<PreparedSyntaxPairHit> {
    let state = prepared_document_tree_state(document)?;
    let text = state.text.as_ref();
    let line_starts = state.line_starts.as_ref();

    let line_span = |ix: usize| prepared_line_span(text, line_starts, ix);

    let clicked = line_span(line_ix)?;
    let clicked_line = text.get(clicked.clone())?;
    let offset =
        clicked.start + clicked_raw_offset_for_display_offset(clicked_line, display_offset)?;

    let source_ranges_equal = |left: Range<usize>, right: Range<usize>| {
        let bytes = text.as_bytes();
        match (bytes.get(left), bytes.get(right)) {
            (Some(left), Some(right)) => left == right,
            _ => false,
        }
    };
    ensure_injection_chain_cached_for_pair_lookup(&state, offset);
    let pair = injected_syntax_pair_at(text, state.source_hash, offset)
        .or_else(|| syntax_pair_in_tree(&state.tree, offset, &source_ranges_equal))?;

    let project = |range: &Range<usize>| -> Vec<PreparedSyntaxPairSpan> {
        // `partition_point` gives the count of starts at or before `range.start`,
        // so subtracting one lands on the line containing it.
        let Some(first) = line_starts
            .partition_point(|start| *start <= range.start)
            .checked_sub(1)
        else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for ix in first..line_starts.len() {
            let Some(span) = line_span(ix) else { break };
            if span.start >= range.end && ix > first {
                break;
            }
            let Some(line) = text.get(span.clone()) else {
                break;
            };
            let start = range.start.clamp(span.start, span.end) - span.start;
            let end = range.end.clamp(span.start, span.end) - span.start;
            if start < end {
                out.push(PreparedSyntaxPairSpan {
                    line_ix: ix,
                    display_range: display_offset_for_raw_offset(line, start)
                        ..display_offset_for_raw_offset(line, end),
                });
            }
            if span.end >= range.end {
                break;
            }
        }
        out
    };

    let (open, close) = (project(&pair.open), project(&pair.close));
    (!open.is_empty() && !close.is_empty()).then_some(PreparedSyntaxPairHit {
        #[cfg(test)]
        kind: pair.kind,
        open,
        close,
    })
}

/// Every place the document names the token at a click, in the canvases' own
/// coordinates.
///
/// Shares the display/raw conversion and the line projection with
/// [`prepared_document_syntax_pair_at_display_offset`], for the same reason:
/// this is the only place holding both the tree's byte offsets and the text the
/// rows were painted from.
pub(in crate::view) fn prepared_document_occurrences_at_display_offset(
    document: PreparedSyntaxDocument,
    line_ix: usize,
    display_offset: usize,
) -> Vec<PreparedSyntaxPairSpan> {
    let Some(state) = prepared_document_tree_state(document) else {
        return Vec::new();
    };
    let text = state.text.as_ref();
    if text.len() > OCCURRENCE_MAX_TEXT_BYTES {
        return Vec::new();
    }
    let line_starts = state.line_starts.as_ref();

    let Some(clicked) = prepared_line_span(text, line_starts, line_ix) else {
        return Vec::new();
    };
    let Some(clicked_line) = text.get(clicked.clone()) else {
        return Vec::new();
    };
    // A caret boundary beyond the line names nothing. The view separately
    // rejects pixel clicks in trailing blank space, whose clamped boundary can
    // equal the valid end boundary produced by the final glyph's right half.
    let Some(raw_offset) = clicked_raw_offset_for_display_offset(clicked_line, display_offset)
    else {
        return Vec::new();
    };
    let offset = clicked.start + raw_offset;

    let Some(found) = syntax_occurrences_in_tree(&state.tree, text, offset) else {
        return Vec::new();
    };
    found
        .ranges
        .iter()
        .filter_map(|range| {
            // A name never spans a line, so one span per occurrence is exact.
            let ix = line_starts
                .partition_point(|start| *start <= range.start)
                .checked_sub(1)?;
            let span = prepared_line_span(text, line_starts, ix)?;
            let line = text.get(span.clone())?;
            let start = range.start.clamp(span.start, span.end) - span.start;
            let end = range.end.clamp(span.start, span.end) - span.start;
            (start < end).then(|| PreparedSyntaxPairSpan {
                line_ix: ix,
                display_range: display_offset_for_raw_offset(line, start)
                    ..display_offset_for_raw_offset(line, end),
            })
        })
        .collect()
}

pub(crate) fn prepared_document_reparse_seed(
    document: PreparedSyntaxDocument,
) -> Option<PreparedSyntaxReparseSeed> {
    prepared_document_tree_state(document).map(|tree_state| PreparedSyntaxReparseSeed {
        document,
        tree_state,
    })
}

#[cfg(test)]
pub(crate) fn prepared_document_parse_mode(
    document: PreparedSyntaxDocument,
) -> Option<TreesitterParseReuseMode> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state(document.cache_key)
            .map(|state| state.parse_mode)
    })
}

#[cfg(test)]
pub(crate) fn prepared_document_source_version(document: PreparedSyntaxDocument) -> Option<u64> {
    TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .tree_state(document.cache_key)
            .map(|state| state.source_version)
    })
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_cache_replacement_drop_step(
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
    defer_drop: bool,
) -> u64 {
    use std::hash::{Hash, Hasher};

    let payloads = benchmark_line_tokens_payload_batch(lines, tokens_per_line, replacements, 0);
    let drop_mode = if defer_drop {
        SyntaxCacheDropMode::DeferredWhenLarge
    } else {
        SyntaxCacheDropMode::InlineWhenLarge
    };
    let mut cache = TreesitterDocumentCache::new();
    let mut h = FxHasher::default();
    for (nonce, line_tokens) in payloads.into_iter().enumerate() {
        cache.insert_document_with_mode(
            PreparedSyntaxCacheKey {
                language: DiffSyntaxLanguage::Rust,
                doc_hash: 0,
            },
            TreesitterCachedDocument::from_line_tokens(line_tokens, None),
            drop_mode,
        );
        cache.by_cache_key.len().hash(&mut h);
        nonce.hash(&mut h);
    }
    h.finish()
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_drop_payload_timed_step(
    lines: usize,
    tokens_per_line: usize,
    seed: usize,
    defer_drop: bool,
) -> Duration {
    let payload = benchmark_line_tokens_payload(lines.max(1), tokens_per_line.max(1), seed);
    let arc_payload = share_recent_line_token_arcs(payload);
    let estimated_bytes = estimated_line_tokens_allocation_bytes(&arc_payload);
    let drop_mode = if defer_drop {
        SyntaxCacheDropMode::DeferredWhenLarge
    } else {
        SyntaxCacheDropMode::InlineWhenLarge
    };
    let start = std::time::Instant::now();
    drop_line_tokens_with_mode(
        SyntaxCacheDropPayload::new(arc_payload, estimated_bytes),
        drop_mode,
    );
    start.elapsed()
}

#[cfg(feature = "benchmarks")]
pub(crate) fn benchmark_line_tokens_payload_batch(
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
    seed: usize,
) -> Vec<Vec<Vec<SyntaxToken>>> {
    let lines = lines.max(1);
    let tokens_per_line = tokens_per_line.max(1);
    let replacements = replacements.max(1);
    let mut payloads = Vec::with_capacity(replacements);
    for nonce in 0..replacements {
        payloads.push(benchmark_line_tokens_payload(
            lines,
            tokens_per_line,
            seed.wrapping_add(nonce),
        ));
    }
    payloads
}

#[cfg(any(test, feature = "benchmarks"))]
pub(crate) fn benchmark_line_tokens_payload(
    lines: usize,
    tokens_per_line: usize,
    nonce: usize,
) -> Vec<Vec<SyntaxToken>> {
    let mut payload = Vec::with_capacity(lines);
    for line_ix in 0..lines {
        let mut line = Vec::with_capacity(tokens_per_line);
        for token_ix in 0..tokens_per_line {
            let start = token_ix.saturating_mul(2);
            let kind = if (line_ix.wrapping_add(nonce).wrapping_add(token_ix) & 1) == 0 {
                SyntaxTokenKind::Keyword
            } else {
                SyntaxTokenKind::String
            };
            line.push(SyntaxToken {
                range: start..start.saturating_add(1),
                kind,
            });
        }
        payload.push(line);
    }
    payload
}

/// Core parsing logic shared by both foreground (cache-inserting) and background (data-returning)
/// document preparation paths.
pub(crate) fn parse_treesitter_document_core(
    request: &TreesitterDocumentParseRequest,
    foreground_timeout: Option<Duration>,
    old_document: Option<PreparedSyntaxDocument>,
    reparse_plan: Option<&TreesitterReparsePlan>,
) -> Option<PreparedSyntaxDocumentData> {
    let incremental_seed = match reparse_plan {
        Some(TreesitterReparsePlan::Changed {
            incremental_seed, ..
        }) => incremental_seed.as_ref(),
        _ => None,
    };

    #[cfg(test)]
    {
        let used_old_document_without_incremental = incremental_reparse_enabled()
            && matches!(
                reparse_plan,
                Some(TreesitterReparsePlan::Changed {
                    incremental_seed: None,
                    ..
                })
            );
        if incremental_seed.is_some() {
            TS_INCREMENTAL_PARSE_COUNT.with(|count| count.set(count.get().saturating_add(1)));
        } else if used_old_document_without_incremental {
            TS_INCREMENTAL_FALLBACK_COUNT.with(|count| count.set(count.get().saturating_add(1)));
        }
    }

    let old_tree_for_parse = incremental_seed.as_ref().map(|seed| &seed.tree);
    let tree = with_ts_parser_parse_result(&request.ts_language, |parser| {
        parse_treesitter_tree(
            parser,
            request.input.text.as_bytes(),
            old_tree_for_parse,
            foreground_timeout,
        )
    })?;

    #[cfg(test)]
    let parse_mode = if incremental_seed.is_some() {
        TreesitterParseReuseMode::Incremental
    } else {
        TreesitterParseReuseMode::Full
    };
    let source_version = incremental_seed.map(|seed| seed.next_version).unwrap_or(1);
    let reused_prefix = match (old_document, reparse_plan) {
        (
            Some(document),
            Some(TreesitterReparsePlan::Changed {
                reusable_prefix_chunk_count,
                ..
            }),
        ) if *reusable_prefix_chunk_count > 0 => TS_DOCUMENT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .clone_prefix_line_token_chunks(document.cache_key, *reusable_prefix_chunk_count)
        }),
        _ => ReusedPrefixLineTokenChunks::default(),
    };

    if let Some(source) = reused_prefix.injection_source {
        clone_prefix_injection_cache_entries(
            source.document_hash,
            request.cache_key.doc_hash,
            source.byte_end,
        );
    }

    Some(PreparedSyntaxDocumentData {
        cache_key: request.cache_key,
        line_count: request.input.line_starts.len(),
        line_token_chunks: reused_prefix.line_token_chunks,
        tree_state: Some(PreparedSyntaxTreeState {
            language: request.language,
            text: request.input.text.clone(),
            line_starts: request.input.line_starts.clone(),
            source_hash: request.cache_key.doc_hash,
            source_version,
            tree,
            #[cfg(test)]
            parse_mode,
        }),
    })
}

pub(crate) fn prepare_treesitter_document_request_after_cache_lookup(
    request: TreesitterDocumentParseRequest,
    foreground_budget: Option<DiffSyntaxBudget>,
    old_document: Option<PreparedSyntaxDocument>,
    has_edit_hint: bool,
    reparse_plan: Option<TreesitterReparsePlan>,
) -> PrepareTreesitterDocumentResult {
    if foreground_budget.is_some_and(|budget| budget.foreground_parse.is_zero()) {
        return PrepareTreesitterDocumentResult::TimedOut;
    }
    if foreground_budget.is_some_and(|budget| {
        should_skip_budgeted_foreground_parse(
            &request,
            budget,
            old_document.is_some(),
            has_edit_hint,
        )
    }) {
        return PrepareTreesitterDocumentResult::TimedOut;
    }

    let Some(data) = parse_treesitter_document_core(
        &request,
        foreground_budget.map(|b| b.foreground_parse),
        old_document,
        reparse_plan.as_ref(),
    ) else {
        return if foreground_budget.is_some() {
            PrepareTreesitterDocumentResult::TimedOut
        } else {
            PrepareTreesitterDocumentResult::Unsupported
        };
    };

    store_shared_prepared_document_seed(&data);
    TS_DOCUMENT_CACHE.with(|cache| {
        cache.borrow_mut().insert_document_with_mode(
            data.cache_key,
            TreesitterCachedDocument::from_chunked_line_tokens(
                data.line_count,
                data.line_token_chunks,
                data.tree_state,
            ),
            SyntaxCacheDropMode::DeferredWhenLarge,
        );
    });

    PrepareTreesitterDocumentResult::Ready(PreparedSyntaxDocument {
        cache_key: request.cache_key,
    })
}

pub(crate) fn prepare_treesitter_document_data_request_impl(
    request: TreesitterDocumentParseRequest,
    old_document: Option<PreparedSyntaxDocument>,
    reparse_plan: Option<TreesitterReparsePlan>,
) -> Option<PreparedSyntaxDocumentData> {
    if matches!(reparse_plan, Some(TreesitterReparsePlan::Unchanged))
        && let Some(document) = old_document
    {
        let line_count = request.input.line_starts.len();
        if let Some(cached) = TS_DOCUMENT_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .prepared_document_data(document.cache_key, line_count)
        }) {
            return Some(cached);
        }
    }
    if let Some(cached) = TS_DOCUMENT_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .prepared_document_data(request.cache_key, request.input.line_starts.len())
    }) {
        return Some(cached);
    }

    parse_treesitter_document_core(&request, None, old_document, reparse_plan.as_ref())
}

pub(crate) fn should_skip_budgeted_foreground_parse(
    request: &TreesitterDocumentParseRequest,
    budget: DiffSyntaxBudget,
    has_old_document: bool,
    has_edit_hint: bool,
) -> bool {
    if budget.foreground_parse > DIFF_SYNTAX_FOREGROUND_PARSE_BUDGET_NON_TEST {
        return false;
    }
    if has_old_document || has_edit_hint {
        return false;
    }

    request.input.text.len() >= DIFF_SYNTAX_FOREGROUND_SKIP_TEXT_BYTES
        || request.input.line_starts.len() >= DIFF_SYNTAX_FOREGROUND_SKIP_LINE_COUNT
}

#[cfg(test)]
pub(crate) fn treesitter_document_parse_request_from_input(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    input: TreesitterDocumentInput,
) -> Option<TreesitterDocumentParseRequest> {
    treesitter_document_parse_request_from_input_with_reuse(language, mode, input, None, None)
}

pub(crate) fn treesitter_document_parse_request_from_input_with_reuse(
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    input: TreesitterDocumentInput,
    old_tree_state: Option<&PreparedSyntaxTreeState>,
    reparse_plan: Option<&TreesitterReparsePlan>,
) -> Option<TreesitterDocumentParseRequest> {
    if !should_prepare_treesitter_document(language, mode, input.text.len()) {
        return None;
    }

    let spec = tree_sitter_highlight_spec(language)?;
    let cache_key = match (old_tree_state, reparse_plan) {
        (Some(previous), Some(TreesitterReparsePlan::Changed { edit_ranges, .. })) => {
            treesitter_document_cache_key_for_reparse_plan(language, previous, &input, edit_ranges)
        }
        _ => treesitter_document_cache_key(language, input.text.as_ref()),
    };

    Some(TreesitterDocumentParseRequest {
        language,
        ts_language: spec.ts_language.clone(),
        input,
        cache_key,
    })
}

pub(crate) fn should_prepare_treesitter_document(
    _language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    text_len: usize,
) -> bool {
    mode == DiffSyntaxMode::Auto && text_len <= TS_PREPARED_DOCUMENT_MAX_TEXT_BYTES
}

pub(crate) fn treesitter_document_input_from_shared_text(
    text: SharedString,
    line_starts: Arc<[usize]>,
) -> TreesitterDocumentInput {
    if text.is_empty() {
        return TreesitterDocumentInput {
            text,
            line_starts: Arc::default(),
        };
    }

    let normalized_line_starts =
        normalized_treesitter_line_starts(text.as_ref(), line_starts.as_ref());

    if normalized_line_starts.first().copied() != Some(0)
        || normalized_line_starts
            .windows(2)
            .any(|window| window[0] >= window[1])
        || normalized_line_starts.last().copied().unwrap_or(0) > text.len()
    {
        return treesitter_document_input_from_text(text.as_ref());
    }

    TreesitterDocumentInput {
        text,
        line_starts: if normalized_line_starts.len() == line_starts.len() {
            line_starts
        } else {
            Arc::<[usize]>::from(normalized_line_starts)
        },
    }
}

pub(crate) fn normalized_treesitter_line_starts<'a>(
    text: &str,
    line_starts: &'a [usize],
) -> &'a [usize] {
    if text.as_bytes().ends_with(b"\n") && line_starts.last().copied() == Some(text.len()) {
        return &line_starts[..line_starts.len().saturating_sub(1)];
    }
    line_starts
}

pub(crate) fn treesitter_document_input_from_text(text: &str) -> TreesitterDocumentInput {
    if text.is_empty() {
        return TreesitterDocumentInput {
            text: SharedString::new(""),
            line_starts: Arc::default(),
        };
    }

    let mut line_starts = vec![0usize];
    for (byte_ix, byte) in text.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(byte_ix.saturating_add(1));
        }
    }
    // If text ends with '\n', remove the phantom line start after the trailing newline.
    if text.as_bytes().ends_with(b"\n") {
        line_starts.pop();
    }

    TreesitterDocumentInput {
        text: SharedString::from(text.to_owned()),
        line_starts: Arc::<[usize]>::from(line_starts),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TreesitterByteEditRange {
    pub(crate) start_byte: usize,
    pub(crate) old_end_byte: usize,
    pub(crate) new_end_byte: usize,
}
