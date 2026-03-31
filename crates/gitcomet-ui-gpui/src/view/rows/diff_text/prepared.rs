use super::*;

pub(in crate::view) fn prepared_diff_syntax_line_for_one_based_line(
    document: Option<PreparedDiffSyntaxDocument>,
    line_number: Option<u32>,
) -> PreparedDiffSyntaxLine {
    let no_syntax = PreparedDiffSyntaxLine {
        document: None,
        line_ix: 0,
    };
    let Some(document) = document else {
        return no_syntax;
    };
    let Some(line_ix) = line_number
        .and_then(|n| usize::try_from(n).ok())
        .and_then(|n| n.checked_sub(1))
    else {
        return no_syntax;
    };
    PreparedDiffSyntaxLine {
        document: Some(document),
        line_ix,
    }
}

/// Projects an inline diff row into the correct real old/new prepared document.
///
/// Inline file diffs interleave rows from two document versions, so syntax must
/// come from the corresponding source side instead of the synthetic inline order.
pub(in crate::view) fn prepared_diff_syntax_line_for_inline_diff_row(
    old_document: Option<PreparedDiffSyntaxDocument>,
    new_document: Option<PreparedDiffSyntaxDocument>,
    line: &AnnotatedDiffLine,
) -> PreparedDiffSyntaxLine {
    use gitcomet_core::domain::DiffLineKind;

    match line.kind {
        DiffLineKind::Remove => {
            prepared_diff_syntax_line_for_one_based_line(old_document, line.old_line)
        }
        DiffLineKind::Add | DiffLineKind::Context => {
            prepared_diff_syntax_line_for_one_based_line(new_document, line.new_line)
        }
        DiffLineKind::Header | DiffLineKind::Hunk => {
            prepared_diff_syntax_line_for_one_based_line(None, None)
        }
    }
}

fn map_prepare_result(
    result: syntax::PrepareTreesitterDocumentResult,
) -> PrepareDiffSyntaxDocumentResult {
    match result {
        syntax::PrepareTreesitterDocumentResult::Ready(inner) => {
            PrepareDiffSyntaxDocumentResult::Ready(PreparedDiffSyntaxDocument { inner })
        }
        syntax::PrepareTreesitterDocumentResult::TimedOut => {
            PrepareDiffSyntaxDocumentResult::TimedOut
        }
        syntax::PrepareTreesitterDocumentResult::Unsupported => {
            PrepareDiffSyntaxDocumentResult::Unsupported
        }
    }
}

pub(in crate::view) fn prepare_diff_syntax_document_with_budget_reuse_text(
    language: DiffSyntaxLanguage,
    syntax_mode: DiffSyntaxMode,
    text: gpui::SharedString,
    line_starts: Arc<[usize]>,
    budget: DiffSyntaxBudget,
    old_document: Option<PreparedDiffSyntaxDocument>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> PrepareDiffSyntaxDocumentResult {
    map_prepare_result(syntax::prepare_treesitter_document_with_budget_reuse_text(
        language,
        syntax_mode,
        text,
        line_starts,
        budget,
        old_document.map(|document| document.inner),
        edit_hint,
    ))
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn prepare_diff_syntax_document_in_background_text(
    language: DiffSyntaxLanguage,
    syntax_mode: DiffSyntaxMode,
    text: gpui::SharedString,
    line_starts: Arc<[usize]>,
) -> Option<BackgroundPreparedDiffSyntaxDocument> {
    prepare_diff_syntax_document_in_background_text_with_reuse(
        language,
        syntax_mode,
        text,
        line_starts,
        None,
        None,
    )
}

pub(in crate::view) fn prepared_diff_syntax_reparse_seed(
    document: PreparedDiffSyntaxDocument,
) -> Option<PreparedDiffSyntaxReparseSeed> {
    syntax::prepared_document_reparse_seed(document.inner)
        .map(|inner| PreparedDiffSyntaxReparseSeed { inner })
}

pub(in crate::view) fn prepare_diff_syntax_document_in_background_text_with_reuse(
    language: DiffSyntaxLanguage,
    syntax_mode: DiffSyntaxMode,
    text: gpui::SharedString,
    line_starts: Arc<[usize]>,
    old_reparse_seed: Option<PreparedDiffSyntaxReparseSeed>,
    edit_hint: Option<DiffSyntaxEdit>,
) -> Option<BackgroundPreparedDiffSyntaxDocument> {
    syntax::prepare_treesitter_document_in_background_text_with_reparse_seed(
        language,
        syntax_mode,
        text,
        line_starts,
        old_reparse_seed.map(|seed| seed.inner),
        edit_hint,
    )
    .map(|inner| BackgroundPreparedDiffSyntaxDocument { inner })
}

pub(in crate::view) fn inject_background_prepared_diff_syntax_document(
    document: BackgroundPreparedDiffSyntaxDocument,
) -> PreparedDiffSyntaxDocument {
    PreparedDiffSyntaxDocument {
        inner: syntax::inject_prepared_document_data(document.inner),
    }
}

#[cfg(test)]
pub(in crate::view) fn prepared_diff_syntax_parse_mode(
    document: PreparedDiffSyntaxDocument,
) -> Option<PreparedDiffSyntaxParseMode> {
    syntax::prepared_document_parse_mode(document.inner).map(|mode| match mode {
        syntax::TreesitterParseReuseMode::Full => PreparedDiffSyntaxParseMode::Full,
        syntax::TreesitterParseReuseMode::Incremental => PreparedDiffSyntaxParseMode::Incremental,
    })
}

#[cfg(test)]
pub(in crate::view) fn prepared_diff_syntax_source_version(
    document: PreparedDiffSyntaxDocument,
) -> Option<u64> {
    syntax::prepared_document_source_version(document.inner)
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_diff_syntax_cache_replacement_drop_step(
    lines: usize,
    tokens_per_line: usize,
    replacements: usize,
    defer_drop: bool,
) -> u64 {
    syntax::benchmark_cache_replacement_drop_step(lines, tokens_per_line, replacements, defer_drop)
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_diff_syntax_cache_drop_payload_timed_step(
    lines: usize,
    tokens_per_line: usize,
    seed: usize,
    defer_drop: bool,
) -> std::time::Duration {
    syntax::benchmark_drop_payload_timed_step(lines, tokens_per_line, seed, defer_drop)
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_flush_diff_syntax_deferred_drop_queue() -> bool {
    syntax::benchmark_flush_deferred_drop_queue()
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(in crate::view) struct PreparedDiffSyntaxCacheMetrics {
    pub hit: u64,
    pub miss: u64,
    pub evict: u64,
    pub chunk_build_ms: u64,
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_reset_diff_syntax_prepared_cache_metrics() {
    syntax::benchmark_reset_prepared_syntax_cache_metrics();
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_diff_syntax_prepared_cache_metrics()
-> PreparedDiffSyntaxCacheMetrics {
    let (hit, miss, evict, chunk_build_ms) = syntax::benchmark_prepared_syntax_cache_metrics();
    PreparedDiffSyntaxCacheMetrics {
        hit,
        miss,
        evict,
        chunk_build_ms,
    }
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_diff_syntax_prepared_loaded_chunk_count(
    document: PreparedDiffSyntaxDocument,
) -> Option<usize> {
    syntax::benchmark_prepared_syntax_loaded_chunk_count(document.inner)
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn benchmark_diff_syntax_prepared_cache_contains_document(
    document: PreparedDiffSyntaxDocument,
) -> bool {
    syntax::benchmark_prepared_syntax_cache_contains_document(document.inner)
}

pub(in crate::view) fn drain_completed_prepared_diff_syntax_chunk_builds() -> usize {
    syntax::drain_completed_prepared_syntax_chunk_builds()
}

pub(in crate::view) fn has_pending_prepared_diff_syntax_chunk_builds() -> bool {
    syntax::has_pending_prepared_syntax_chunk_builds()
}

pub(in crate::view) fn drain_completed_prepared_diff_syntax_chunk_builds_for_document(
    document: PreparedDiffSyntaxDocument,
) -> usize {
    syntax::drain_completed_prepared_syntax_chunk_builds_for_document(document.inner)
}

pub(in crate::view) fn has_pending_prepared_diff_syntax_chunk_builds_for_document(
    document: PreparedDiffSyntaxDocument,
) -> bool {
    syntax::has_pending_prepared_syntax_chunk_builds_for_document(document.inner)
}

pub(in super::super) enum PreparedDocumentLineStyledText {
    Cacheable(CachedDiffStyledText),
    Pending(CachedDiffStyledText),
}

impl PreparedDocumentLineStyledText {
    /// Extracts the inner styled text regardless of variant.
    #[cfg(feature = "benchmarks")]
    pub(in super::super) fn into_inner(self) -> CachedDiffStyledText {
        match self {
            Self::Cacheable(s) | Self::Pending(s) => s,
        }
    }

    /// Returns `(styled_text, is_pending)`. Use this to avoid the match block
    /// when the caller just needs to branch on pending vs cacheable.
    pub(in super::super) fn into_parts(self) -> (CachedDiffStyledText, bool) {
        match self {
            Self::Cacheable(s) => (s, false),
            Self::Pending(s) => (s, true),
        }
    }
}

pub(in super::super) fn build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    syntax: DiffSyntaxConfig,
    word_color: Option<gpui::Rgba>,
    prepared_line: PreparedDiffSyntaxLine,
) -> PreparedDocumentLineStyledText {
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_optional_palette(
        theme,
        None,
        PreparedDiffTextBuildRequest {
            build: DiffTextBuildRequest {
                text,
                word_ranges,
                query,
                syntax,
                word_color,
            },
            prepared_line,
        },
    )
}

pub(in super::super) fn build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
    theme: AppTheme,
    highlight_palette: &SyntaxHighlightPalette,
    request: PreparedDiffTextBuildRequest<'_>,
) -> PreparedDocumentLineStyledText {
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_optional_palette(
        theme,
        Some(highlight_palette),
        request,
    )
}

fn build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_optional_palette(
    theme: AppTheme,
    highlight_palette: Option<&SyntaxHighlightPalette>,
    request: PreparedDiffTextBuildRequest<'_>,
) -> PreparedDocumentLineStyledText {
    let text = request.build.text;
    let word_ranges = request.build.word_ranges;
    let query = request.build.query;
    let word_color = request.build.word_color;
    let prepared_line = request.prepared_line;
    let DiffSyntaxConfig {
        language,
        mode: syntax_mode,
    } = request.build.syntax;
    let fallback = |mode| {
        build_cached_diff_styled_text(theme, text, word_ranges, query, language, mode, word_color)
    };

    if language.is_none() {
        return PreparedDocumentLineStyledText::Cacheable(fallback(syntax_mode));
    }

    let Some(document) = prepared_line.document else {
        return PreparedDocumentLineStyledText::Cacheable(fallback(syntax_mode));
    };

    match syntax::request_syntax_tokens_for_prepared_document_line(
        document.inner,
        prepared_line.line_ix,
    ) {
        Some(syntax::PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
            let query_trimmed = query.trim();
            if word_ranges.is_empty() && query_trimmed.is_empty() {
                let build_syntax_only = || {
                    SYNTAX_HIGHLIGHTS_BUF.with_borrow_mut(|buf| {
                        match highlight_palette {
                            Some(highlight_palette) => {
                                prepared_document_line_highlights_from_tokens_into_with_palette(
                                    highlight_palette,
                                    text.len(),
                                    &tokens,
                                    buf,
                                );
                            }
                            None => {
                                prepared_document_line_highlights_from_tokens_into(
                                    theme,
                                    text.len(),
                                    &tokens,
                                    buf,
                                );
                            }
                        }
                        styled_text_to_cached_from_buf(text, buf)
                    })
                };

                if should_cache_single_line_styled_text(text) {
                    let (key, cached) = SINGLE_LINE_STYLED_TEXT_CACHE.with(|cache| {
                        let mut cache = cache.borrow_mut();
                        let key = cache.prepared_key_for(theme, text, &tokens);
                        let styled = cache.get_prepared(key, text, &tokens);
                        (key, styled)
                    });
                    if let Some(styled) = cached {
                        return PreparedDocumentLineStyledText::Cacheable(styled);
                    }

                    let styled = build_syntax_only();
                    SINGLE_LINE_STYLED_TEXT_CACHE.with(|cache| {
                        cache.borrow_mut().insert_prepared(
                            key,
                            text,
                            tokens.clone(),
                            styled.clone(),
                        );
                    });
                    return PreparedDocumentLineStyledText::Cacheable(styled);
                }

                PreparedDocumentLineStyledText::Cacheable(build_syntax_only())
            } else {
                PreparedDocumentLineStyledText::Cacheable(build_styled_text_fused(
                    theme,
                    FusedDiffTextBuildRequest {
                        build: DiffTextBuildRequest {
                            text,
                            word_ranges,
                            query,
                            syntax: DiffSyntaxConfig {
                                language: None,
                                mode: DiffSyntaxMode::HeuristicOnly,
                            },
                            word_color,
                        },
                        syntax_tokens_override: Some(&tokens),
                    },
                ))
            }
        }
        Some(syntax::PreparedSyntaxLineTokensRequest::Pending) | None => {
            PreparedDocumentLineStyledText::Pending(fallback(DiffSyntaxMode::HeuristicOnly))
        }
    }
}

#[cfg(feature = "benchmarks")]
pub(in super::super) fn build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
    theme: AppTheme,
    _palette: &SyntaxHighlightPalette,
    request: PreparedDiffTextBuildRequest<'_>,
) -> PreparedDocumentLineStyledText {
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
        theme,
        request.build.text,
        request.build.word_ranges,
        request.build.query,
        request.build.syntax,
        request.build.word_color,
        request.prepared_line,
    )
}

#[cfg(feature = "benchmarks")]
pub(in crate::view) fn build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
    theme: AppTheme,
    language: Option<DiffSyntaxLanguage>,
    old_source: PreparedDiffSyntaxTextSource,
    new_source: PreparedDiffSyntaxTextSource,
    rows: &[InlineDiffSyntaxOnlyRow<'_>],
) -> Vec<PreparedDocumentLineStyledText> {
    rows.iter()
        .map(|row| {
            build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                theme,
                row.text,
                &[],
                "",
                DiffSyntaxConfig {
                    language,
                    mode: DiffSyntaxMode::Auto,
                },
                None,
                prepared_diff_syntax_line_for_inline_diff_row(
                    old_source.document,
                    new_source.document,
                    row.line,
                ),
            )
        })
        .collect()
}

#[cfg(test)]
pub(in crate::view) fn syntax_highlights_for_prepared_document_byte_range(
    theme: AppTheme,
    text: &str,
    line_starts: &[usize],
    document: PreparedDiffSyntaxDocument,
    byte_range: Range<usize>,
) -> Option<Vec<(Range<usize>, gpui::HighlightStyle)>> {
    let text_len = text.len();
    let clamped_range = byte_range.start.min(text_len)..byte_range.end.min(text_len);
    if text.is_empty() || clamped_range.is_empty() {
        return Some(Vec::new());
    }

    let line_range = line_range_for_absolute_byte_window(line_starts, text_len, &clamped_range);
    if line_range.is_empty() {
        return Some(Vec::new());
    }

    let highlight_palette = syntax_highlight_palette(theme);
    let mut highlights = Vec::new();
    for line_ix in line_range {
        let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(document.inner, line_ix)?;
        push_clipped_absolute_prepared_document_token_highlights(
            &mut highlights,
            &highlight_palette,
            line_start,
            line_end,
            &clamped_range,
            &tokens,
        );
    }

    Some(highlights)
}

pub(in crate::view) fn request_syntax_highlights_for_prepared_document_line_range(
    theme: AppTheme,
    text: &str,
    line_starts: &[usize],
    document: PreparedDiffSyntaxDocument,
    language: DiffSyntaxLanguage,
    line_range: Range<usize>,
) -> Option<Vec<PreparedDocumentLineHighlights>> {
    if text.is_empty() || line_range.is_empty() {
        return Some(Vec::new());
    }

    let line_count = line_starts.len().max(1);
    let clamped_range = line_range.start.min(line_count)..line_range.end.min(line_count);
    if clamped_range.is_empty() {
        return Some(Vec::new());
    }

    PREPARED_TOKEN_REQUEST_BUF.with(|buf| {
        let token_requests = &mut *buf.borrow_mut();
        syntax::request_syntax_tokens_for_prepared_document_line_range_into(
            document.inner,
            clamped_range.clone(),
            token_requests,
        )?;

        let mut line_highlights = Vec::with_capacity(clamped_range.len());
        for (line_ix, token_request) in clamped_range.zip(token_requests.iter()) {
            let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
            match token_request {
                syntax::PreparedSyntaxLineTokensRequest::Ready(tokens) => {
                    line_highlights.push(PreparedDocumentLineHighlights {
                        line_ix,
                        highlights: prepared_document_line_highlights_from_tokens(
                            theme,
                            line_end.saturating_sub(line_start),
                            tokens.as_ref(),
                        ),
                        pending: false,
                    });
                }
                syntax::PreparedSyntaxLineTokensRequest::Pending => {
                    let line_text = &text[line_start..line_end];
                    let highlights = HEURISTIC_TOKEN_BUF.with(|buf| {
                        let tokens = &mut *buf.borrow_mut();
                        syntax::syntax_tokens_for_line_heuristic_into(line_text, language, tokens);
                        prepared_document_line_highlights_from_tokens(
                            theme,
                            line_end.saturating_sub(line_start),
                            tokens,
                        )
                    });
                    line_highlights.push(PreparedDocumentLineHighlights {
                        line_ix,
                        highlights,
                        pending: true,
                    });
                }
            }
        }

        Some(line_highlights)
    })
}

pub(in crate::view) fn build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
    theme: AppTheme,
    language: DiffSyntaxLanguage,
    old_source: PreparedDiffSyntaxTextSource,
    new_source: PreparedDiffSyntaxTextSource,
    rows: &[InlineDiffSyntaxOnlyRow<'_>],
) -> Vec<PreparedDocumentLineStyledText> {
    #[derive(Clone, Copy)]
    struct SideRow<'a> {
        result_ix: usize,
        line_ix: usize,
        text: &'a str,
    }

    fn styled_text_from_prepared_line_token_request(
        theme: AppTheme,
        language: DiffSyntaxLanguage,
        text: &str,
        token_request: &syntax::PreparedSyntaxLineTokensRequest,
    ) -> PreparedDocumentLineStyledText {
        match token_request {
            syntax::PreparedSyntaxLineTokensRequest::Ready(tokens) => {
                PreparedDocumentLineStyledText::Cacheable(SYNTAX_HIGHLIGHTS_BUF.with_borrow_mut(
                    |buf| {
                        prepared_document_line_highlights_from_tokens_into(
                            theme,
                            text.len(),
                            tokens.as_ref(),
                            buf,
                        );
                        styled_text_to_cached_from_buf(text, buf)
                    },
                ))
            }
            syntax::PreparedSyntaxLineTokensRequest::Pending => {
                let styled = HEURISTIC_TOKEN_BUF.with(|buf| {
                    let tokens = &mut *buf.borrow_mut();
                    syntax::syntax_tokens_for_line_heuristic_into(text, language, tokens);
                    SYNTAX_HIGHLIGHTS_BUF.with_borrow_mut(|highlights| {
                        prepared_document_line_highlights_from_tokens_into(
                            theme,
                            text.len(),
                            tokens,
                            highlights,
                        );
                        styled_text_to_cached_from_buf(text, highlights)
                    })
                });
                PreparedDocumentLineStyledText::Pending(styled)
            }
        }
    }

    fn fallback_syntax_only_row(
        theme: AppTheme,
        language: DiffSyntaxLanguage,
        text: &str,
        document: Option<PreparedDiffSyntaxDocument>,
    ) -> PreparedDocumentLineStyledText {
        PreparedDocumentLineStyledText::Cacheable(build_cached_diff_styled_text(
            theme,
            text,
            &[],
            "",
            Some(language),
            syntax_mode_for_prepared_document(document),
            None,
        ))
    }

    fn fill_side_results(
        theme: AppTheme,
        language: DiffSyntaxLanguage,
        source: PreparedDiffSyntaxTextSource,
        rows: &[SideRow<'_>],
        results: &mut [Option<PreparedDocumentLineStyledText>],
    ) {
        if rows.is_empty() {
            return;
        }

        let Some(document) = source.document else {
            for row in rows {
                results[row.result_ix] = Some(fallback_syntax_only_row(
                    theme,
                    language,
                    row.text,
                    source.document,
                ));
            }
            return;
        };

        let mut group_start = 0usize;
        while group_start < rows.len() {
            let mut group_end = group_start + 1;
            while group_end < rows.len()
                && rows[group_end].line_ix == rows[group_end - 1].line_ix.saturating_add(1)
            {
                group_end += 1;
            }

            let group = &rows[group_start..group_end];
            let line_range = group[0].line_ix..group[group.len() - 1].line_ix.saturating_add(1);
            PREPARED_TOKEN_REQUEST_BUF.with(|buf| {
                let token_requests = &mut *buf.borrow_mut();
                if syntax::request_syntax_tokens_for_prepared_document_line_range_into(
                    document.inner,
                    line_range,
                    token_requests,
                )
                .is_some()
                {
                    for (row, token_request) in group.iter().zip(token_requests.iter()) {
                        results[row.result_ix] =
                            Some(styled_text_from_prepared_line_token_request(
                                theme,
                                language,
                                row.text,
                                token_request,
                            ));
                    }
                } else {
                    for row in group {
                        results[row.result_ix] = Some(fallback_syntax_only_row(
                            theme,
                            language,
                            row.text,
                            Some(document),
                        ));
                    }
                }
            });

            group_start = group_end;
        }
    }

    let mut old_rows = Vec::new();
    let mut new_rows = Vec::new();
    let mut results = std::iter::repeat_with(|| None)
        .take(rows.len())
        .collect::<Vec<_>>();

    for (result_ix, row) in rows.iter().enumerate() {
        match row.line.kind {
            DiffLineKind::Remove => {
                if let Some(line_ix) = row
                    .line
                    .old_line
                    .and_then(|line| usize::try_from(line).ok())
                    .and_then(|line| line.checked_sub(1))
                {
                    old_rows.push(SideRow {
                        result_ix,
                        line_ix,
                        text: row.text,
                    });
                } else {
                    results[result_ix] = Some(fallback_syntax_only_row(
                        theme,
                        language,
                        row.text,
                        old_source.document,
                    ));
                }
            }
            DiffLineKind::Add | DiffLineKind::Context => {
                if let Some(line_ix) = row
                    .line
                    .new_line
                    .and_then(|line| usize::try_from(line).ok())
                    .and_then(|line| line.checked_sub(1))
                {
                    new_rows.push(SideRow {
                        result_ix,
                        line_ix,
                        text: row.text,
                    });
                } else {
                    results[result_ix] = Some(fallback_syntax_only_row(
                        theme,
                        language,
                        row.text,
                        new_source.document,
                    ));
                }
            }
            DiffLineKind::Header | DiffLineKind::Hunk => {
                results[result_ix] = Some(PreparedDocumentLineStyledText::Cacheable(
                    build_cached_diff_styled_text(
                        theme,
                        row.text,
                        &[],
                        "",
                        None,
                        DiffSyntaxMode::HeuristicOnly,
                        None,
                    ),
                ));
            }
        }
    }

    fill_side_results(
        theme,
        language,
        old_source,
        old_rows.as_slice(),
        results.as_mut_slice(),
    );
    fill_side_results(
        theme,
        language,
        new_source,
        new_rows.as_slice(),
        results.as_mut_slice(),
    );

    results
        .into_iter()
        .enumerate()
        .map(|(ix, styled)| {
            styled.unwrap_or_else(|| {
                fallback_syntax_only_row(
                    theme,
                    language,
                    rows[ix].text,
                    prepared_diff_syntax_line_for_inline_diff_row(
                        old_source.document,
                        new_source.document,
                        rows[ix].line,
                    )
                    .document,
                )
            })
        })
        .collect()
}

thread_local! {
    static HEURISTIC_TOKEN_BUF: RefCell<Vec<syntax::SyntaxToken>> = const { RefCell::new(Vec::new()) };
    static PREPARED_TOKEN_REQUEST_BUF: RefCell<Vec<syntax::PreparedSyntaxLineTokensRequest>> =
        const { RefCell::new(Vec::new()) };
}

pub(in crate::view) fn request_syntax_highlights_for_prepared_document_byte_range(
    theme: AppTheme,
    text: &str,
    line_starts: &[usize],
    document: PreparedDiffSyntaxDocument,
    language: DiffSyntaxLanguage,
    byte_range: Range<usize>,
) -> Option<PreparedDocumentByteRangeHighlights> {
    let text_len = text.len();
    let clamped_range = byte_range.start.min(text_len)..byte_range.end.min(text_len);
    if text.is_empty() || clamped_range.is_empty() {
        return Some(PreparedDocumentByteRangeHighlights::default());
    }

    let line_range = line_range_for_absolute_byte_window(line_starts, text_len, &clamped_range);
    if line_range.is_empty() {
        return Some(PreparedDocumentByteRangeHighlights::default());
    }

    PREPARED_TOKEN_REQUEST_BUF.with(|buf| {
        let token_requests = &mut *buf.borrow_mut();
        let summary = syntax::request_syntax_tokens_for_prepared_document_line_range_into(
            document.inner,
            line_range.clone(),
            token_requests,
        );
        let ready_line_count = summary
            .map(|summary| summary.ready_lines)
            .unwrap_or_default();
        let ready_highlight_count = summary
            .map(|summary| summary.ready_tokens)
            .unwrap_or_default();
        let pending_line_count = if summary.is_some() {
            line_range.len().saturating_sub(ready_line_count)
        } else {
            line_range.len()
        };
        let estimated_highlight_capacity = {
            let estimated_pending_line_highlights = if ready_line_count == 0 {
                pending_line_count.saturating_mul(8)
            } else {
                let avg_ready_highlights =
                    (ready_highlight_count + ready_line_count.saturating_sub(1)) / ready_line_count;
                pending_line_count.saturating_mul(avg_ready_highlights.max(1))
            };
            ready_highlight_count.saturating_add(estimated_pending_line_highlights)
        };
        let mut highlights = Vec::with_capacity(estimated_highlight_capacity);
        let highlight_palette = syntax_highlight_palette(theme);

        if pending_line_count == 0 {
            for (line_ix, token_request) in line_range.zip(token_requests.iter()) {
                let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
                if let syntax::PreparedSyntaxLineTokensRequest::Ready(tokens) = token_request {
                    push_clipped_absolute_prepared_document_token_highlights(
                        &mut highlights,
                        &highlight_palette,
                        line_start,
                        line_end,
                        &clamped_range,
                        tokens.as_ref(),
                    );
                }
            }
            return Some(PreparedDocumentByteRangeHighlights {
                highlights,
                pending: false,
            });
        }

        let mut pending = summary.is_none();
        let mut token_requests = token_requests.iter();
        HEURISTIC_TOKEN_BUF.with(|buf| {
            let heuristic_tokens = &mut *buf.borrow_mut();
            for line_ix in line_range {
                let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
                match token_requests.next() {
                    Some(syntax::PreparedSyntaxLineTokensRequest::Ready(tokens)) => {
                        push_clipped_absolute_prepared_document_token_highlights(
                            &mut highlights,
                            &highlight_palette,
                            line_start,
                            line_end,
                            &clamped_range,
                            tokens.as_ref(),
                        );
                    }
                    Some(syntax::PreparedSyntaxLineTokensRequest::Pending) | None => {
                        pending = true;
                        let line_text = &text[line_start..line_end];
                        syntax::syntax_tokens_for_line_heuristic_into(
                            line_text,
                            language,
                            heuristic_tokens,
                        );
                        push_clipped_absolute_prepared_document_token_highlights(
                            &mut highlights,
                            &highlight_palette,
                            line_start,
                            line_end,
                            &clamped_range,
                            heuristic_tokens,
                        );
                    }
                }
            }
        });

        Some(PreparedDocumentByteRangeHighlights {
            highlights,
            pending,
        })
    })
}
