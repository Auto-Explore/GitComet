use super::super::perf::{self, ViewPerfSpan};
use super::*;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

mod syntax;

pub(in crate::view) use syntax::{
    DiffSyntaxBudget, DiffSyntaxEdit, DiffSyntaxLanguage, DiffSyntaxMode,
    diff_syntax_language_for_code_fence_info, diff_syntax_language_for_path,
};

/// Extracts the text content of a specific line from a document using precomputed
/// line starts. Returns an empty string if the line index is out of bounds.
/// Strips trailing newline.
pub(in crate::view) fn resolved_output_line_text<'a>(
    text: &'a str,
    line_starts: &[usize],
    line_ix: usize,
) -> &'a str {
    if text.is_empty() {
        return "";
    }
    let (start, end) = line_byte_bounds(text, line_starts, line_ix);
    if start >= text.len() {
        return "";
    }
    text.get(start..end).unwrap_or("")
}

/// Returns `Auto` when a prepared document exists (full-document syntax),
/// `HeuristicOnly` when it doesn't (per-line fallback).
pub(super) fn syntax_mode_for_prepared_document(
    document: Option<PreparedDiffSyntaxDocument>,
) -> DiffSyntaxMode {
    if document.is_some() {
        DiffSyntaxMode::Auto
    } else {
        DiffSyntaxMode::HeuristicOnly
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::view) struct PreparedDiffSyntaxDocument {
    inner: syntax::PreparedSyntaxDocument,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum PreparedDiffSyntaxParseMode {
    Full,
    Incremental,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct PreparedDiffSyntaxReparseSeed {
    inner: syntax::PreparedSyntaxReparseSeed,
}

#[derive(Clone, Debug)]
pub(in crate::view) struct BackgroundPreparedDiffSyntaxDocument {
    inner: syntax::PreparedSyntaxDocumentData,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum PrepareDiffSyntaxDocumentResult {
    Ready(PreparedDiffSyntaxDocument),
    TimedOut,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct DiffSyntaxConfig {
    pub language: Option<DiffSyntaxLanguage>,
    pub mode: DiffSyntaxMode,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedDiffSyntaxLine {
    pub document: Option<PreparedDiffSyntaxDocument>,
    pub line_ix: usize,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug)]
pub(super) struct DiffTextSourceIdentity(u64);

#[cfg(feature = "benchmarks")]
impl DiffTextSourceIdentity {
    pub(super) fn from_str(text: &str) -> Self {
        let mut hasher = FxHasher::default();
        text.hash(&mut hasher);
        Self(hasher.finish())
    }
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug)]
pub(super) struct PreparedDiffSyntaxTextSource {
    pub document: Option<PreparedDiffSyntaxDocument>,
}

#[cfg(feature = "benchmarks")]
#[derive(Clone, Copy, Debug)]
pub(super) struct InlineDiffSyntaxOnlyRow<'a> {
    pub text: &'a str,
    pub line: &'a AnnotatedDiffLine,
}

#[cfg(feature = "benchmarks")]
pub(super) type SyntaxHighlightPalette = AppTheme;

#[cfg(feature = "benchmarks")]
pub(super) fn syntax_highlight_palette(theme: AppTheme) -> SyntaxHighlightPalette {
    theme
}

#[cfg(feature = "benchmarks")]
pub(super) struct DiffTextBuildRequest<'a> {
    pub text: &'a str,
    pub word_ranges: &'a [Range<usize>],
    pub query: &'a str,
    pub syntax: DiffSyntaxConfig,
    pub word_color: Option<gpui::Rgba>,
}

#[cfg(feature = "benchmarks")]
pub(super) struct PreparedDiffTextBuildRequest<'a> {
    pub build: DiffTextBuildRequest<'a>,
    pub prepared_line: PreparedDiffSyntaxLine,
}

/// Projects an optional 1-based line number into a prepared syntax document.
///
/// Diff metadata stores line numbers using the natural 1-based file coordinates.
/// Rendering uses zero-based indices, so full-document syntax consumers should
/// route through this helper instead of assuming visual row indices line up with
/// real document lines.
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

fn maybe_expand_tabs(s: &str) -> SharedString {
    if !s.contains('\t') {
        return SharedString::new(s);
    }

    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
    }
    out.into()
}

#[inline]
fn segment_overlaps_sorted_ranges(
    segment_start: usize,
    segment_end: usize,
    ranges: &[Range<usize>],
    cursor: &mut usize,
) -> bool {
    while *cursor < ranges.len() && ranges[*cursor].end <= segment_start {
        *cursor += 1;
    }

    ranges
        .get(*cursor)
        .is_some_and(|range| segment_start < range.end && segment_end > range.start)
}

fn build_diff_text_segments(
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    syntax_tokens_override: Option<&[syntax::SyntaxToken]>,
) -> Vec<CachedDiffTextSegment> {
    if text.is_empty() {
        return Vec::new();
    }

    let query = query.trim();
    if word_ranges.is_empty()
        && query.is_empty()
        && language.is_none()
        && syntax_tokens_override.is_none()
    {
        return vec![CachedDiffTextSegment {
            text: maybe_expand_tabs(text),
            in_word: false,
            in_query: false,
            syntax: SyntaxTokenKind::None,
        }];
    }

    let syntax_tokens = if let Some(tokens) = syntax_tokens_override {
        tokens.to_vec()
    } else if let Some(language) = language {
        let _syntax_scope = perf::span(ViewPerfSpan::SyntaxHighlighting);
        syntax::syntax_tokens_for_line(text, language, syntax_mode)
    } else {
        Vec::new()
    };

    let _word_query_scope = perf::span(ViewPerfSpan::WordQueryHighlighting);
    let query_ranges = if !query.is_empty() {
        find_all_ascii_case_insensitive(text, query)
    } else {
        Default::default()
    };

    thread_local! {
        static BOUNDARY_BUF: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
    }

    BOUNDARY_BUF.with_borrow_mut(|boundaries| {
        boundaries.clear();
        boundaries.push(0);
        boundaries.push(text.len());
        for r in word_ranges {
            boundaries.push(r.start.min(text.len()));
            boundaries.push(r.end.min(text.len()));
        }
        for r in &query_ranges {
            boundaries.push(r.start);
            boundaries.push(r.end);
        }
        for t in &syntax_tokens {
            boundaries.push(t.range.start.min(text.len()));
            boundaries.push(t.range.end.min(text.len()));
        }
        boundaries.sort_unstable();
        boundaries.dedup();

        let mut token_ix = 0usize;
        let mut word_ix = 0usize;
        let mut query_ix = 0usize;
        let mut segments = Vec::with_capacity(boundaries.len().saturating_sub(1));
        for w in boundaries.windows(2) {
            let (a, b) = (w[0], w[1]);
            if a >= b || a >= text.len() {
                continue;
            }
            let b = b.min(text.len());
            let Some(seg) = text.get(a..b) else {
                return vec![CachedDiffTextSegment {
                    text: maybe_expand_tabs(text),
                    in_word: false,
                    in_query: false,
                    syntax: SyntaxTokenKind::None,
                }];
            };

            while token_ix < syntax_tokens.len() && syntax_tokens[token_ix].range.end <= a {
                token_ix += 1;
            }
            let syntax = syntax_tokens
                .get(token_ix)
                .filter(|t| t.range.start <= a && t.range.end >= b)
                .map(|t| t.kind)
                .unwrap_or(SyntaxTokenKind::None);

            let in_word = segment_overlaps_sorted_ranges(a, b, word_ranges, &mut word_ix);
            let in_query = segment_overlaps_sorted_ranges(a, b, &query_ranges, &mut query_ix);

            segments.push(CachedDiffTextSegment {
                text: maybe_expand_tabs(seg),
                in_word,
                in_query,
                syntax,
            });
        }

        segments
    })
}

pub(super) fn selectable_cached_diff_text(
    visible_ix: usize,
    region: DiffTextRegion,
    double_click_kind: DiffClickKind,
    base_fg: gpui::Rgba,
    styled: Option<&CachedDiffStyledText>,
    fallback_text: SharedString,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let view = cx.entity();
    let (text, highlights) = if let Some(styled) = styled {
        (styled.text.clone(), Arc::clone(&styled.highlights))
    } else {
        (fallback_text, empty_highlights())
    };

    let overlay_text = text.clone();
    let overlay = div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .child(DiffTextSelectionOverlay {
            view: view.clone(),
            visible_ix,
            region,
            text: overlay_text,
        });

    let content = if text.is_empty() {
        div().into_any_element()
    } else if highlights.is_empty() {
        div()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(text.clone())
            .into_any_element()
    } else {
        div()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(gpui::StyledText::new(text.clone()).with_highlights(highlights.iter().cloned()))
            .into_any_element()
    };

    div()
        .relative()
        .min_w(px(0.0))
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(base_fg)
        .cursor(CursorStyle::IBeam)
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                window.focus(&this.diff_panel_focus_handle);
                if e.click_count >= 2 {
                    cx.stop_propagation();
                    this.double_click_select_diff_text(visible_ix, region, double_click_kind);
                    cx.notify();
                    return;
                }
                this.begin_diff_text_selection(visible_ix, region, e.position);
                this.begin_diff_text_scroll_tracking(e.position, cx);
                cx.notify();
            }),
        )
        .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _w, cx| {
            if !this.diff_text_selecting {
                return;
            }
            let before = this.diff_text_head;
            this.update_diff_text_selection_from_mouse(e.position);
            if this.diff_text_head != before {
                cx.notify();
            }
        }))
        .on_mouse_up(
            MouseButton::Left,
            cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                this.end_diff_text_selection();
                cx.notify();
            }),
        )
        .on_mouse_up_out(
            MouseButton::Left,
            cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                this.end_diff_text_selection();
                cx.notify();
            }),
        )
        .on_mouse_down(
            MouseButton::Right,
            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                if double_click_kind == DiffClickKind::HunkHeader {
                    return;
                }
                cx.stop_propagation();
                this.open_diff_editor_context_menu(visible_ix, region, e.position, window, cx);
            }),
        )
        .child(overlay)
        .child(content)
        .into_any_element()
}

fn empty_highlights() -> Arc<Vec<(Range<usize>, gpui::HighlightStyle)>> {
    type Highlights = Vec<(Range<usize>, gpui::HighlightStyle)>;
    type HighlightsRef = Arc<Highlights>;

    static EMPTY: OnceLock<HighlightsRef> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(Vec::new())))
}

fn styled_text_to_cached(
    text: SharedString,
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
) -> CachedDiffStyledText {
    let mut hasher = FxHasher::default();
    text.as_ref().hash(&mut hasher);
    let text_hash = hasher.finish();

    if highlights.is_empty() {
        return CachedDiffStyledText {
            text,
            highlights: empty_highlights(),
            highlights_hash: 0,
            text_hash,
        };
    }

    let highlights_hash = hash_highlights(&highlights);
    CachedDiffStyledText {
        text,
        highlights: Arc::new(highlights),
        highlights_hash,
        text_hash,
    }
}

fn segments_to_cached_styled_text(
    theme: AppTheme,
    segments: &[CachedDiffTextSegment],
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    let (expanded_text, highlights) = styled_text_for_diff_segments(theme, segments, word_color);
    styled_text_to_cached(expanded_text, highlights)
}

pub(super) fn build_cached_diff_styled_text_from_relative_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> CachedDiffStyledText {
    if text.is_empty() {
        return empty_styled_text();
    }

    let (expanded_text, remapped_highlights) =
        expanded_text_and_remapped_relative_highlights(text, highlights);
    styled_text_to_cached(expanded_text, remapped_highlights)
}

fn empty_styled_text() -> CachedDiffStyledText {
    styled_text_to_cached("".into(), Vec::new())
}

pub(super) fn build_cached_diff_styled_text(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    if text.is_empty() {
        return empty_styled_text();
    }

    let segments = build_diff_text_segments(text, word_ranges, query, language, syntax_mode, None);
    segments_to_cached_styled_text(theme, &segments, word_color)
}

#[cfg(feature = "benchmarks")]
pub(super) fn build_cached_diff_styled_text_with_source_identity(
    theme: AppTheme,
    text: &str,
    source_identity: Option<DiffTextSourceIdentity>,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    if let Some(identity) = source_identity {
        let _ = identity.0;
    }
    build_cached_diff_styled_text(
        theme,
        text,
        word_ranges,
        query,
        language,
        syntax_mode,
        word_color,
    )
}

#[cfg(feature = "benchmarks")]
pub(super) fn build_cached_diff_styled_text_with_palette(
    theme: AppTheme,
    _palette: &SyntaxHighlightPalette,
    request: DiffTextBuildRequest<'_>,
) -> CachedDiffStyledText {
    build_cached_diff_styled_text(
        theme,
        request.text,
        request.word_ranges,
        request.query,
        request.syntax.language,
        request.syntax.mode,
        request.word_color,
    )
}

pub(super) enum PreparedDocumentLineStyledText {
    Cacheable(CachedDiffStyledText),
    Pending(CachedDiffStyledText),
}

impl PreparedDocumentLineStyledText {
    /// Extracts the inner styled text regardless of variant.
    #[cfg(feature = "benchmarks")]
    pub(super) fn into_inner(self) -> CachedDiffStyledText {
        match self {
            Self::Cacheable(s) | Self::Pending(s) => s,
        }
    }

    /// Returns `(styled_text, is_pending)`. Use this to avoid the match block
    /// when the caller just needs to branch on pending vs cacheable.
    pub(super) fn into_parts(self) -> (CachedDiffStyledText, bool) {
        match self {
            Self::Cacheable(s) => (s, false),
            Self::Pending(s) => (s, true),
        }
    }
}

pub(super) fn build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    syntax: DiffSyntaxConfig,
    word_color: Option<gpui::Rgba>,
    prepared_line: PreparedDiffSyntaxLine,
) -> PreparedDocumentLineStyledText {
    let DiffSyntaxConfig {
        language,
        mode: syntax_mode,
    } = syntax;
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
            let segments = build_diff_text_segments(
                text,
                word_ranges,
                query,
                None,
                DiffSyntaxMode::HeuristicOnly,
                Some(tokens.as_slice()),
            );
            PreparedDocumentLineStyledText::Cacheable(segments_to_cached_styled_text(
                theme, &segments, word_color,
            ))
        }
        Some(syntax::PreparedSyntaxLineTokensRequest::Pending) => {
            PreparedDocumentLineStyledText::Pending(fallback(DiffSyntaxMode::HeuristicOnly))
        }
        None => PreparedDocumentLineStyledText::Cacheable(fallback(syntax_mode)),
    }
}

#[cfg(feature = "benchmarks")]
pub(super) fn build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette(
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
pub(super) fn build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
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

pub(super) fn build_cached_diff_query_overlay_styled_text(
    theme: AppTheme,
    base: &CachedDiffStyledText,
    query: &str,
) -> CachedDiffStyledText {
    let query = query.trim();
    if query.is_empty() || base.text.is_empty() {
        return base.clone();
    }

    let query_ranges = find_all_ascii_case_insensitive(base.text.as_ref(), query);
    if query_ranges.is_empty() {
        return base.clone();
    }

    let base_highlights = base.highlights.as_ref();
    let mut boundaries: Vec<usize> =
        Vec::with_capacity(2 + base_highlights.len() * 2 + query_ranges.len() * 2);
    boundaries.push(0);
    boundaries.push(base.text.len());
    for (range, _) in base_highlights {
        boundaries.push(range.start.min(base.text.len()));
        boundaries.push(range.end.min(base.text.len()));
    }
    for range in &query_ranges {
        boundaries.push(range.start.min(base.text.len()));
        boundaries.push(range.end.min(base.text.len()));
    }
    boundaries.sort_unstable();
    boundaries.dedup();

    let query_bg = with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 }).into();
    let mut merged: Vec<(Range<usize>, gpui::HighlightStyle)> =
        Vec::with_capacity(boundaries.len().saturating_sub(1));
    let mut base_ix = 0usize;
    let mut query_ix = 0usize;
    let default_style = gpui::HighlightStyle::default();

    for window in boundaries.windows(2) {
        let (a, b) = (window[0], window[1]);
        if a >= b || a >= base.text.len() {
            continue;
        }
        let b = b.min(base.text.len());
        if a >= b {
            continue;
        }

        while base_ix < base_highlights.len() && base_highlights[base_ix].0.end <= a {
            base_ix += 1;
        }
        let base_style = base_highlights
            .get(base_ix)
            .filter(|(range, _)| range.start <= a && range.end >= b)
            .map(|(_, style)| *style);

        while query_ix < query_ranges.len() && query_ranges[query_ix].end <= a {
            query_ix += 1;
        }
        let in_query = query_ranges
            .get(query_ix)
            .is_some_and(|range| range.start <= a && range.end >= b);

        let mut style = base_style.unwrap_or_default();
        if in_query {
            style.background_color = Some(query_bg);
        }

        if style != default_style {
            merged.push((a..b, style));
        }
    }

    if merged.is_empty() {
        return CachedDiffStyledText {
            text: base.text.clone(),
            highlights: empty_highlights(),
            highlights_hash: 0,
            text_hash: base.text_hash,
        };
    }

    let highlights_hash = hash_highlights(&merged);
    CachedDiffStyledText {
        text: base.text.clone(),
        highlights: Arc::new(merged),
        highlights_hash,
        text_hash: base.text_hash,
    }
}

fn hash_highlights(highlights: &[(Range<usize>, gpui::HighlightStyle)]) -> u64 {
    let mut hasher = FxHasher::default();
    for (range, style) in highlights {
        range.hash(&mut hasher);
        style.hash(&mut hasher);
    }
    hasher.finish()
}

fn syntax_highlight_color(theme: AppTheme, kind: SyntaxTokenKind) -> Option<gpui::Rgba> {
    match kind {
        SyntaxTokenKind::None => None,
        SyntaxTokenKind::Comment => Some(theme.syntax.comment),
        SyntaxTokenKind::CommentDoc => Some(theme.syntax.comment_doc),
        SyntaxTokenKind::String => Some(theme.syntax.string),
        SyntaxTokenKind::StringEscape => Some(theme.syntax.string_escape),
        SyntaxTokenKind::Keyword => Some(theme.syntax.keyword),
        SyntaxTokenKind::KeywordControl => Some(theme.syntax.keyword_control),
        SyntaxTokenKind::Number => Some(theme.syntax.number),
        SyntaxTokenKind::Boolean => Some(theme.syntax.boolean),
        SyntaxTokenKind::Function => Some(theme.syntax.function),
        SyntaxTokenKind::FunctionMethod => Some(theme.syntax.function_method),
        SyntaxTokenKind::FunctionSpecial => Some(theme.syntax.function_special),
        SyntaxTokenKind::Type => Some(theme.syntax.type_name),
        SyntaxTokenKind::TypeBuiltin => Some(theme.syntax.type_builtin),
        SyntaxTokenKind::TypeInterface => Some(theme.syntax.type_interface),
        SyntaxTokenKind::Variable => theme.syntax.variable,
        SyntaxTokenKind::VariableParameter => Some(theme.syntax.variable_parameter),
        SyntaxTokenKind::VariableSpecial => Some(theme.syntax.variable_special),
        SyntaxTokenKind::Property => Some(theme.syntax.property),
        SyntaxTokenKind::Constant => Some(theme.syntax.constant),
        SyntaxTokenKind::Operator => Some(theme.syntax.operator),
        SyntaxTokenKind::Punctuation => Some(theme.syntax.punctuation),
        SyntaxTokenKind::PunctuationBracket => Some(theme.syntax.punctuation_bracket),
        SyntaxTokenKind::PunctuationDelimiter => Some(theme.syntax.punctuation_delimiter),
        SyntaxTokenKind::Tag => Some(theme.syntax.tag),
        SyntaxTokenKind::Attribute => Some(theme.syntax.attribute),
        SyntaxTokenKind::Lifetime => Some(theme.syntax.lifetime),
    }
}

fn syntax_highlight_style(theme: AppTheme, kind: SyntaxTokenKind) -> Option<gpui::HighlightStyle> {
    let fg = syntax_highlight_color(theme, kind)?;
    let mut style = gpui::HighlightStyle {
        color: Some(fg.into()),
        ..gpui::HighlightStyle::default()
    };
    match kind {
        // Doc comments render italic to distinguish from regular comments.
        SyntaxTokenKind::CommentDoc => {
            style.font_style = Some(gpui::FontStyle::Italic);
        }
        // Control-flow keywords (if/else/for/while/return/match) render semibold.
        SyntaxTokenKind::KeywordControl => {
            style.font_weight = Some(gpui::FontWeight::SEMIBOLD);
        }
        _ => {}
    }
    Some(style)
}

fn expanded_text_and_remapped_relative_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    if !text.contains('\t') {
        return (SharedString::new(text), highlights.to_vec());
    }

    let mut out = String::with_capacity(text.len());
    let mut byte_map = vec![0usize; text.len() + 1];

    for (start, ch) in text.char_indices() {
        byte_map[start] = out.len();
        match ch {
            '\t' => out.push_str("    "),
            _ => out.push(ch),
        }
        let end = start + ch.len_utf8();
        let mapped_end = out.len();
        for mapped in byte_map.iter_mut().take(end + 1).skip(start + 1) {
            *mapped = mapped_end;
        }
    }

    let mut remapped = Vec::with_capacity(highlights.len());
    for (range, style) in highlights {
        let start = *byte_map
            .get(range.start.min(text.len()))
            .unwrap_or(&out.len());
        let end = *byte_map
            .get(range.end.min(text.len()))
            .unwrap_or(&out.len());
        if start < end {
            remapped.push((start..end, *style));
        }
    }

    (out.into(), remapped)
}

fn line_range_for_absolute_byte_window(
    line_starts: &[usize],
    text_len: usize,
    byte_range: &Range<usize>,
) -> Range<usize> {
    if line_starts.is_empty() || text_len == 0 {
        return 0..0;
    }

    let start = byte_range.start.min(text_len);
    let end = byte_range.end.min(text_len);
    if start >= end {
        return 0..0;
    }

    let start_line = line_starts
        .partition_point(|&line_start| line_start <= start)
        .saturating_sub(1);
    let end_line = line_starts
        .partition_point(|&line_start| line_start <= end.saturating_sub(1))
        .saturating_sub(1);
    start_line..end_line.saturating_add(1)
}

/// Returns `(line_start, line_end)` byte offsets for a zero-based line index,
/// stripping any trailing newline. Both values are clamped to `text.len()`.
fn line_byte_bounds(text: &str, line_starts: &[usize], line_ix: usize) -> (usize, usize) {
    let text_len = text.len();
    let start = line_starts
        .get(line_ix)
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    let mut end = line_starts
        .get(line_ix.saturating_add(1))
        .copied()
        .unwrap_or(text_len)
        .min(text_len);
    if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
        end = end.saturating_sub(1);
    }
    (start, end)
}

/// Clip a line-relative range to an absolute clamped window and push if non-empty.
fn clip_and_push_line_highlight(
    highlights: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
    line_start: usize,
    line_end: usize,
    clamped_range: &Range<usize>,
    relative_range: Range<usize>,
    style: gpui::HighlightStyle,
) {
    if relative_range.start >= relative_range.end {
        return;
    }
    let absolute_start = line_start
        .saturating_add(relative_range.start)
        .min(line_end);
    let absolute_end = line_start.saturating_add(relative_range.end).min(line_end);
    let clipped_start = absolute_start.max(clamped_range.start);
    let clipped_end = absolute_end.min(clamped_range.end);
    if clipped_start < clipped_end {
        highlights.push((clipped_start..clipped_end, style));
    }
}

fn prepared_document_line_highlights_from_tokens(
    theme: AppTheme,
    line_len: usize,
    tokens: &[syntax::SyntaxToken],
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    tokens
        .iter()
        .filter_map(|token| {
            let style = syntax_highlight_style(theme, token.kind)?;
            if token.range.start >= token.range.end || token.range.start >= line_len {
                return None;
            }
            let end = token.range.end.min(line_len);
            (token.range.start < end).then_some((token.range.start..end, style))
        })
        .collect()
}

fn push_clipped_absolute_line_highlights(
    highlights: &mut Vec<(Range<usize>, gpui::HighlightStyle)>,
    line_start: usize,
    line_end: usize,
    clamped_range: &Range<usize>,
    line_highlights: &[(Range<usize>, gpui::HighlightStyle)],
) {
    for (range, style) in line_highlights {
        clip_and_push_line_highlight(
            highlights,
            line_start,
            line_end,
            clamped_range,
            range.clone(),
            *style,
        );
    }
}

#[derive(Clone, Default)]
pub(in crate::view) struct PreparedDocumentByteRangeHighlights {
    pub highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub pending: bool,
}

#[derive(Clone, Default)]
pub(in crate::view) struct PreparedDocumentLineHighlights {
    pub line_ix: usize,
    pub highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    pub pending: bool,
}

pub(in crate::view) fn syntax_highlights_for_line(
    theme: AppTheme,
    text: &str,
    language: DiffSyntaxLanguage,
    syntax_mode: DiffSyntaxMode,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    if text.is_empty() {
        return Vec::new();
    }

    let _syntax_scope = perf::span(ViewPerfSpan::SyntaxHighlighting);
    let tokens = syntax::syntax_tokens_for_line(text, language, syntax_mode);
    prepared_document_line_highlights_from_tokens(theme, text.len(), &tokens)
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

    let mut highlights = Vec::new();
    for line_ix in line_range {
        let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(document.inner, line_ix)?;
        let line_hl = prepared_document_line_highlights_from_tokens(
            theme,
            line_end.saturating_sub(line_start),
            tokens.as_slice(),
        );
        push_clipped_absolute_line_highlights(
            &mut highlights,
            line_start,
            line_end,
            &clamped_range,
            &line_hl,
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

    let mut line_highlights = Vec::with_capacity(clamped_range.len());
    for line_ix in clamped_range {
        let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
        match syntax::request_syntax_tokens_for_prepared_document_line(document.inner, line_ix)? {
            syntax::PreparedSyntaxLineTokensRequest::Ready(tokens) => {
                line_highlights.push(PreparedDocumentLineHighlights {
                    line_ix,
                    highlights: prepared_document_line_highlights_from_tokens(
                        theme,
                        line_end.saturating_sub(line_start),
                        tokens.as_slice(),
                    ),
                    pending: false,
                });
            }
            syntax::PreparedSyntaxLineTokensRequest::Pending => {
                let line_text = &text[line_start..line_end];
                line_highlights.push(PreparedDocumentLineHighlights {
                    line_ix,
                    highlights: syntax_highlights_for_line(
                        theme,
                        line_text,
                        language,
                        DiffSyntaxMode::HeuristicOnly,
                    ),
                    pending: true,
                });
            }
        }
    }

    Some(line_highlights)
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

    let mut highlights = Vec::new();
    let mut pending = false;
    for line_ix in line_range {
        let (line_start, line_end) = line_byte_bounds(text, line_starts, line_ix);
        match syntax::request_syntax_tokens_for_prepared_document_line(document.inner, line_ix)? {
            syntax::PreparedSyntaxLineTokensRequest::Ready(tokens) => {
                let line_hl = prepared_document_line_highlights_from_tokens(
                    theme,
                    line_end.saturating_sub(line_start),
                    tokens.as_slice(),
                );
                push_clipped_absolute_line_highlights(
                    &mut highlights,
                    line_start,
                    line_end,
                    &clamped_range,
                    &line_hl,
                );
            }
            syntax::PreparedSyntaxLineTokensRequest::Pending => {
                pending = true;
                let line_text = &text[line_start..line_end];
                let line_hl = syntax_highlights_for_line(
                    theme,
                    line_text,
                    language,
                    DiffSyntaxMode::HeuristicOnly,
                );
                push_clipped_absolute_line_highlights(
                    &mut highlights,
                    line_start,
                    line_end,
                    &clamped_range,
                    &line_hl,
                );
            }
        }
    }

    Some(PreparedDocumentByteRangeHighlights {
        highlights,
        pending,
    })
}

fn styled_text_for_diff_segments(
    theme: AppTheme,
    segments: &[CachedDiffTextSegment],
    word_color: Option<gpui::Rgba>,
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    let combined_len: usize = segments.iter().map(|s| s.text.len()).sum();
    let mut combined = String::with_capacity(combined_len);
    let mut highlights: Vec<(Range<usize>, gpui::HighlightStyle)> =
        Vec::with_capacity(segments.len());

    let mut offset = 0usize;
    for seg in segments {
        combined.push_str(seg.text.as_ref());
        let next_offset = offset + seg.text.len();

        let mut style = gpui::HighlightStyle::default();

        if seg.in_word
            && let Some(mut c) = word_color
        {
            c.a = if theme.is_dark { 0.22 } else { 0.16 };
            style.background_color = Some(c.into());
        }

        if seg.in_query {
            style.background_color = Some(
                with_alpha(theme.colors.accent, if theme.is_dark { 0.22 } else { 0.16 }).into(),
            );
        }

        let syntax_fg = syntax_highlight_color(theme, seg.syntax);
        if let Some(fg) = syntax_fg {
            style.color = Some(fg.into());
        }

        if style != gpui::HighlightStyle::default() && offset < next_offset {
            highlights.push((offset..next_offset, style));
        }

        offset = next_offset;
    }

    (combined.into(), highlights)
}

fn find_all_ascii_case_insensitive(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    const MAX_MATCHES: usize = 64;

    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() {
        return Vec::new();
    }

    let haystack_bytes = haystack.as_bytes();
    if needle_bytes.len() > haystack_bytes.len() {
        return Vec::new();
    }

    let max_possible = haystack_bytes.len() / needle_bytes.len().max(1);
    let mut out = Vec::with_capacity(MAX_MATCHES.min(max_possible));
    let mut start = 0usize;
    while start + needle_bytes.len() <= haystack_bytes.len() && out.len() < MAX_MATCHES {
        let mut matched = true;
        for (offset, needle_byte) in needle_bytes.iter().copied().enumerate() {
            let haystack_byte = haystack_bytes[start + offset];
            if !haystack_byte.eq_ignore_ascii_case(&needle_byte) {
                matched = false;
                break;
            }
        }

        if matched {
            out.push(start..(start + needle_bytes.len()));
            start = start.saturating_add(needle_bytes.len().max(1));
        } else {
            start = start.saturating_add(1);
        }
    }

    out
}

pub(super) fn diff_line_colors(
    theme: AppTheme,
    kind: gitcomet_core::domain::DiffLineKind,
) -> (gpui::Rgba, gpui::Rgba, gpui::Rgba) {
    use gitcomet_core::domain::DiffLineKind::*;

    match (theme.is_dark, kind) {
        (_, Header) => (
            theme.colors.window_bg,
            theme.colors.text_muted,
            theme.colors.text_muted,
        ),
        (_, Hunk) => (
            theme.colors.window_bg,
            theme.colors.accent,
            theme.colors.text_muted,
        ),
        (_, Add) => (
            theme.colors.diff_add_bg,
            theme.colors.diff_add_text,
            theme.colors.diff_add_text,
        ),
        (_, Remove) => (
            theme.colors.diff_remove_bg,
            theme.colors.diff_remove_text,
            theme.colors.diff_remove_text,
        ),
        (_, Context) => (
            theme.colors.window_bg,
            theme.colors.text,
            theme.colors.text_muted,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_line_starts(text: &str) -> Arc<[usize]> {
        let mut line_starts = Vec::with_capacity(
            text.as_bytes()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1,
        );
        line_starts.push(0);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix + 1);
            }
        }
        Arc::from(line_starts)
    }

    fn prepare_test_document(
        language: DiffSyntaxLanguage,
        text: &str,
    ) -> PreparedDiffSyntaxDocument {
        let text: SharedString = text.to_owned().into();
        let line_starts = test_line_starts(text.as_ref());
        match prepare_diff_syntax_document_with_budget_reuse_text(
            language,
            DiffSyntaxMode::Auto,
            text.clone(),
            Arc::clone(&line_starts),
            DiffSyntaxBudget {
                foreground_parse: std::time::Duration::from_millis(50),
            },
            None,
            None,
        ) {
            PrepareDiffSyntaxDocumentResult::Ready(document) => document,
            PrepareDiffSyntaxDocumentResult::TimedOut => {
                inject_background_prepared_diff_syntax_document(
                    prepare_diff_syntax_document_in_background_text(
                        language,
                        DiffSyntaxMode::Auto,
                        text,
                        line_starts,
                    )
                    .expect("background parse should be available for supported test documents"),
                )
            }
            PrepareDiffSyntaxDocumentResult::Unsupported => {
                panic!("test document should support prepared syntax parsing")
            }
        }
    }

    #[test]
    fn sorted_range_cursor_advances_without_rescanning() {
        let ranges = [2..4, 6..8];
        let mut cursor = 0usize;

        assert!(!segment_overlaps_sorted_ranges(0, 2, &ranges, &mut cursor));
        assert_eq!(cursor, 0);

        assert!(segment_overlaps_sorted_ranges(2, 3, &ranges, &mut cursor));
        assert_eq!(cursor, 0);

        assert!(!segment_overlaps_sorted_ranges(4, 6, &ranges, &mut cursor));
        assert_eq!(cursor, 1);

        assert!(segment_overlaps_sorted_ranges(6, 7, &ranges, &mut cursor));
        assert_eq!(cursor, 1);

        assert!(!segment_overlaps_sorted_ranges(8, 9, &ranges, &mut cursor));
        assert_eq!(cursor, 2);
    }

    #[test]
    fn build_segments_fast_path_skips_syntax_work() {
        let segments = build_diff_text_segments("a\tb", &[], "", None, DiffSyntaxMode::Auto, None);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text.as_ref(), "a    b");
        assert!(!segments[0].in_word);
        assert!(!segments[0].in_query);
        assert_eq!(segments[0].syntax, SyntaxTokenKind::None);
    }

    #[test]
    fn build_cached_styled_text_plain_has_no_highlights() {
        let theme = AppTheme::gitcomet_dark();
        let styled =
            build_cached_diff_styled_text(theme, "a\tb", &[], "", None, DiffSyntaxMode::Auto, None);
        assert_eq!(styled.text.as_ref(), "a    b");
        assert!(styled.highlights.is_empty());
        assert_eq!(styled.highlights_hash, 0);
    }

    #[test]
    fn build_segments_does_not_panic_on_non_char_boundary_ranges() {
        // This can happen if token ranges are computed in bytes that don't align to UTF-8
        // boundaries. We should never panic during diff rendering.
        let text = "aé"; // 'é' is 2 bytes in UTF-8
        let ranges = vec![Range { start: 1, end: 2 }];
        let segments =
            build_diff_text_segments(text, &ranges, "", None, DiffSyntaxMode::Auto, None);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].text.as_ref(), text);
    }

    #[test]
    fn styled_text_highlights_cover_combined_ranges() {
        let theme = AppTheme::gitcomet_dark();
        let segments = vec![
            CachedDiffTextSegment {
                text: "abc".into(),
                in_word: false,
                in_query: false,
                syntax: SyntaxTokenKind::None,
            },
            CachedDiffTextSegment {
                text: "def".into(),
                in_word: false,
                in_query: true,
                syntax: SyntaxTokenKind::Keyword,
            },
        ];

        let (text, highlights) = styled_text_for_diff_segments(theme, &segments, None);
        assert_eq!(text.as_ref(), "abcdef");
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].0, 3..6);
        assert_eq!(highlights[0].1.font_weight, None);
        assert!(highlights[0].1.background_color.is_some());

        // Hashing highlights is used for caching shaped layouts; it should be stable for identical
        // highlight sequences within a process.
        let styled = build_cached_diff_styled_text(
            theme,
            "abcdef",
            &[],
            "def",
            None,
            DiffSyntaxMode::Auto,
            None,
        );
        assert_eq!(styled.highlights.len(), 1);
        assert_eq!(styled.highlights[0].0, 3..6);
    }

    #[test]
    fn cached_styled_text_highlights_all_query_occurrences() {
        let theme = AppTheme::gitcomet_dark();
        let styled = build_cached_diff_styled_text(
            theme,
            "abxxab",
            &[],
            "ab",
            None,
            DiffSyntaxMode::Auto,
            None,
        );
        assert_eq!(styled.highlights.len(), 2);
        assert_eq!(styled.highlights[0].0, 0..2);
        assert_eq!(styled.highlights[1].0, 4..6);
    }

    #[test]
    fn styled_text_word_highlight_sets_background() {
        let theme = AppTheme::gitcomet_dark();
        let segments = vec![CachedDiffTextSegment {
            text: "x".into(),
            in_word: true,
            in_query: false,
            syntax: SyntaxTokenKind::None,
        }];
        let (text, highlights) =
            styled_text_for_diff_segments(theme, &segments, Some(theme.colors.diff_remove_text));
        assert_eq!(text.as_ref(), "x");
        assert_eq!(highlights.len(), 1);
        assert!(highlights[0].1.background_color.is_some());
    }

    #[test]
    fn syntax_colors_are_softened_for_keywords() {
        let theme = AppTheme::gitcomet_light();
        let segments = vec![CachedDiffTextSegment {
            text: "fn".into(),
            in_word: false,
            in_query: false,
            syntax: SyntaxTokenKind::Keyword,
        }];

        let (_text, highlights) = styled_text_for_diff_segments(theme, &segments, None);
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].0, 0..2);
        assert_ne!(highlights[0].1.color, Some(theme.colors.accent.into()));
    }

    #[test]
    fn doc_comment_renders_italic() {
        let theme = AppTheme::gitcomet_dark();
        let style = syntax_highlight_style(theme, SyntaxTokenKind::CommentDoc);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.font_style, Some(gpui::FontStyle::Italic));
        // Regular comments should not be italic.
        let plain = syntax_highlight_style(theme, SyntaxTokenKind::Comment).unwrap();
        assert_eq!(plain.font_style, None);
    }

    #[test]
    fn keyword_control_renders_semibold() {
        let theme = AppTheme::gitcomet_dark();
        let style = syntax_highlight_style(theme, SyntaxTokenKind::KeywordControl);
        assert!(style.is_some());
        let style = style.unwrap();
        assert_eq!(style.font_weight, Some(gpui::FontWeight::SEMIBOLD));
        // Regular keywords should not have font weight.
        let plain = syntax_highlight_style(theme, SyntaxTokenKind::Keyword).unwrap();
        assert_eq!(plain.font_weight, None);
    }

    #[test]
    fn syntax_highlight_style_uses_theme_syntax_overrides() {
        let theme = AppTheme::from_json_str(
            r##"{
                "name": "Fixture",
                "themes": [
                    {
                        "key": "fixture",
                        "name": "Fixture",
                        "appearance": "dark",
                        "colors": {
                            "window_bg": "#0d1016ff",
                            "surface_bg": "#1f2127ff",
                            "surface_bg_elevated": "#1f2127ff",
                            "active_section": "#2d2f34ff",
                            "border": "#2d2f34ff",
                            "text": "#bfbdb6ff",
                            "text_muted": "#8a8986ff",
                            "accent": "#5ac1feff",
                            "hover": "#2d2f34ff",
                            "active": { "hex": "#2d2f34ff", "alpha": 0.78 },
                            "focus_ring": { "hex": "#5ac1feff", "alpha": 0.60 },
                            "focus_ring_bg": { "hex": "#5ac1feff", "alpha": 0.16 },
                            "scrollbar_thumb": { "hex": "#8a8986ff", "alpha": 0.30 },
                            "scrollbar_thumb_hover": { "hex": "#8a8986ff", "alpha": 0.42 },
                            "scrollbar_thumb_active": { "hex": "#8a8986ff", "alpha": 0.52 },
                            "danger": "#ef7177ff",
                            "warning": "#feb454ff",
                            "success": "#aad84cff"
                        },
                        "syntax": {
                            "keyword": "#112233ff",
                            "variable": "#445566ff"
                        },
                        "radii": {
                            "panel": 2.0,
                            "pill": 2.0,
                            "row": 2.0
                        }
                    }
                ]
            }"##,
        )
        .expect("theme JSON should parse");

        let keyword = syntax_highlight_style(theme, SyntaxTokenKind::Keyword)
            .expect("keyword style should be present");
        assert_eq!(keyword.color, Some(gpui::rgba(0x112233ff).into()));

        let variable = syntax_highlight_style(theme, SyntaxTokenKind::Variable)
            .expect("variable style should be present when overridden");
        assert_eq!(variable.color, Some(gpui::rgba(0x445566ff).into()));
    }

    #[test]
    fn cached_styled_text_from_relative_highlights_expands_tabs_and_remaps_ranges() {
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.33, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let styled = build_cached_diff_styled_text_from_relative_highlights(
            "\tlet value",
            &[(0..1, style), (1..4, style)],
        );

        assert_eq!(styled.text.as_ref(), "    let value");
        assert_eq!(styled.highlights.len(), 2);
        assert_eq!(styled.highlights[0].0, 0..4);
        assert_eq!(styled.highlights[1].0, 4..7);
    }

    #[test]
    fn cached_styled_text_from_relative_highlights_handles_multibyte_utf8_with_tabs() {
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.5, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        // "→" is 3 bytes (U+2192), tab is 1 byte.
        // Input: "\t→x" — 5 bytes: tab(0..1), arrow(1..4), x(4..5)
        let styled = build_cached_diff_styled_text_from_relative_highlights(
            "\t\u{2192}x",
            &[(0..1, style), (1..4, style), (4..5, style)],
        );

        // Tab expands to 4 spaces, arrow stays 3 bytes, x stays 1 byte.
        assert_eq!(styled.text.as_ref(), "    \u{2192}x");
        assert_eq!(styled.highlights.len(), 3);
        // Tab (0..1) → expanded to 4-space span (0..4).
        assert_eq!(styled.highlights[0].0, 0..4);
        // Arrow (1..4) → starts at 4 (after tab expansion), length 3 bytes.
        assert_eq!(styled.highlights[1].0, 4..7);
        // x (4..5) → starts at 7.
        assert_eq!(styled.highlights[2].0, 7..8);
    }

    #[test]
    fn cached_styled_text_from_relative_highlights_no_tabs_passes_through() {
        let style = gpui::HighlightStyle {
            color: Some(gpui::hsla(0.5, 1.0, 0.5, 1.0)),
            ..gpui::HighlightStyle::default()
        };
        let styled = build_cached_diff_styled_text_from_relative_highlights(
            "let x = 1;",
            &[(0..3, style), (8..9, style)],
        );

        assert_eq!(styled.text.as_ref(), "let x = 1;");
        assert_eq!(styled.highlights.len(), 2);
        assert_eq!(styled.highlights[0].0, 0..3);
        assert_eq!(styled.highlights[1].0, 8..9);
    }

    #[test]
    fn prepared_document_byte_range_highlights_multiline_comment_continuation() {
        let theme = AppTheme::gitcomet_dark();
        let text = "/* open comment\nstill comment */ let x = 1;";
        let line_starts = vec![0, "/* open comment\n".len()];
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        let second_line_start = line_starts[1];
        let highlights = syntax_highlights_for_prepared_document_byte_range(
            theme,
            text,
            &line_starts,
            document,
            second_line_start..text.len(),
        )
        .expect("prepared document should still be available");

        assert!(
            highlights
                .iter()
                .all(|(range, _)| range.start >= second_line_start),
            "returned highlights should be clipped to the requested byte range"
        );
        assert!(
            highlights.iter().any(|(range, style)| {
                range.start <= second_line_start
                    && range.end > second_line_start
                    && style.color == Some(theme.colors.text_muted.into())
            }),
            "second line should retain comment highlighting from multiline document context"
        );
    }

    #[test]
    fn nonblocking_prepared_document_byte_range_upgrades_after_chunk_build() {
        let theme = AppTheme::gitcomet_dark();
        let text = "/* open comment\nstill comment */ let x = 1;";
        let line_starts = vec![0, "/* open comment\n".len()];
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        let second_line_start = line_starts[1];
        let first = request_syntax_highlights_for_prepared_document_byte_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            second_line_start..text.len(),
        )
        .expect("prepared document should be requestable");
        assert!(first.pending);
        assert!(
            !first.highlights.iter().any(|(range, style)| {
                range.start <= second_line_start
                    && range.end > second_line_start
                    && style.color == Some(theme.colors.text_muted.into())
            }),
            "heuristic fallback should not invent multiline comment state before the chunk is ready"
        );

        let started = std::time::Instant::now();
        while drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = request_syntax_highlights_for_prepared_document_byte_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            second_line_start..text.len(),
        )
        .expect("prepared document should still be available after chunk completion");
        assert!(!second.pending);
        assert!(
            second.highlights.iter().any(|(range, style)| {
                range.start <= second_line_start
                    && range.end > second_line_start
                    && style.color == Some(theme.colors.text_muted.into())
            }),
            "resolved output should upgrade to full document-aware comment highlighting"
        );
    }

    #[test]
    fn prepared_document_line_range_reports_ready_and_pending_rows_per_chunk() {
        let theme = AppTheme::gitcomet_dark();
        let lines: Vec<String> = (0..70)
            .map(|ix| format!("let chunk_boundary_value_{ix} = {ix};"))
            .collect();
        let text = lines.join("\n");
        let mut line_starts = Vec::with_capacity(lines.len());
        let mut offset = 0usize;
        for line in &lines {
            line_starts.push(offset);
            offset = offset.saturating_add(line.len()).saturating_add(1);
        }

        let document = prepare_test_document(DiffSyntaxLanguage::Rust, &text);
        assert!(
            syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some(),
            "first chunk should be loadable synchronously"
        );

        let first = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            &text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            63..66,
        )
        .expect("row-range request should succeed");
        assert_eq!(first.len(), 3);
        assert_eq!(first[0].line_ix, 63);
        assert!(!first[0].pending, "loaded chunk row should be ready");
        assert!(
            !first[0].highlights.is_empty(),
            "ready row should include syntax highlights"
        );
        assert_eq!(first[1].line_ix, 64);
        assert!(first[1].pending, "next chunk row should be pending");
        assert_eq!(first[2].line_ix, 65);
        assert!(first[2].pending, "same pending chunk should remain pending");

        let started = std::time::Instant::now();
        while drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            &text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            63..66,
        )
        .expect("row-range request should still succeed after chunk drain");
        assert!(second.iter().all(|line| !line.pending));
    }

    #[test]
    fn prepared_document_line_range_clamps_beyond_document_bounds() {
        let theme = AppTheme::gitcomet_dark();
        let text = "let a = 1;\nlet b = 2;";
        let line_starts = vec![0, "let a = 1;\n".len()];
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);
        // Load chunk 0 synchronously so the request API returns Ready.
        assert!(syntax::syntax_tokens_for_prepared_document_line(document.inner, 0).is_some());

        // Request range extends beyond the 2-line document.
        let result = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            1..5,
        )
        .expect("line-range request should succeed");
        // Should only return 1 line (line_ix 1), not 4.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].line_ix, 1);
        assert!(!result[0].pending);

        // Fully out-of-bounds range returns empty.
        let result = request_syntax_highlights_for_prepared_document_line_range(
            theme,
            text,
            &line_starts,
            document,
            DiffSyntaxLanguage::Rust,
            10..15,
        )
        .expect("out-of-bounds range should still succeed");
        assert!(result.is_empty());
    }

    #[test]
    fn nonblocking_prepared_line_helper_transitions_from_pending_to_cacheable() {
        let theme = AppTheme::gitcomet_dark();
        let text = "let value = 1;";
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, text);

        let first = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
            theme,
            text,
            &[],
            "",
            DiffSyntaxConfig {
                language: Some(DiffSyntaxLanguage::Rust),
                mode: DiffSyntaxMode::Auto,
            },
            None,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 0,
            },
        );
        match first {
            PreparedDocumentLineStyledText::Pending(styled) => {
                assert_eq!(styled.text.as_ref(), text);
            }
            PreparedDocumentLineStyledText::Cacheable(_) => {
                panic!("first nonblocking prepared-line request should be pending")
            }
        }

        let started = std::time::Instant::now();
        while drain_completed_prepared_diff_syntax_chunk_builds() == 0
            && started.elapsed() < std::time::Duration::from_secs(2)
        {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }

        let second = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
            theme,
            text,
            &[],
            "",
            DiffSyntaxConfig {
                language: Some(DiffSyntaxLanguage::Rust),
                mode: DiffSyntaxMode::Auto,
            },
            None,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 0,
            },
        );
        match second {
            PreparedDocumentLineStyledText::Cacheable(styled) => {
                assert!(
                    !styled.highlights.is_empty(),
                    "cacheable prepared-line styling should contain syntax highlights"
                );
            }
            PreparedDocumentLineStyledText::Pending(_) => {
                panic!("prepared-line helper should become cacheable after chunk drain")
            }
        }
    }

    #[test]
    fn prepared_diff_syntax_line_for_one_based_line_converts_to_zero_based_index() {
        let document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["let first = 1;", "let second = 2;"].join("\n"),
        );

        let prepared = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(2));
        assert_eq!(
            prepared,
            PreparedDiffSyntaxLine {
                document: Some(document),
                line_ix: 1,
            }
        );
    }

    #[test]
    fn prepared_diff_syntax_line_for_one_based_line_rejects_missing_or_zero_lines() {
        let document = prepare_test_document(DiffSyntaxLanguage::Rust, "let value = 1;");

        assert_eq!(
            prepared_diff_syntax_line_for_one_based_line(Some(document), None),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );
        assert_eq!(
            prepared_diff_syntax_line_for_one_based_line(Some(document), Some(0)),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );
    }

    #[test]
    fn prepared_diff_syntax_line_for_inline_diff_row_projects_remove_add_and_context_lines() {
        use gitcomet_core::domain::DiffLineKind;

        let old_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["let old_one = 1;", "let old_two = 2;"].join("\n"),
        );
        let new_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["let new_one = 1;", "let new_two = 2;", "let new_three = 3;"].join("\n"),
        );

        let remove_line = AnnotatedDiffLine {
            kind: DiffLineKind::Remove,
            text: Arc::from("-let old_two = 2;"),
            old_line: Some(2),
            new_line: None,
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                &remove_line,
            ),
            PreparedDiffSyntaxLine {
                document: Some(old_document),
                line_ix: 1,
            }
        );

        let add_line = AnnotatedDiffLine {
            kind: DiffLineKind::Add,
            text: Arc::from("+let new_three = 3;"),
            old_line: None,
            new_line: Some(3),
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                &add_line,
            ),
            PreparedDiffSyntaxLine {
                document: Some(new_document),
                line_ix: 2,
            }
        );

        let context_line = AnnotatedDiffLine {
            kind: DiffLineKind::Context,
            text: Arc::from(" let new_one = 1;"),
            old_line: Some(1),
            new_line: Some(1),
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(old_document),
                Some(new_document),
                &context_line,
            ),
            PreparedDiffSyntaxLine {
                document: Some(new_document),
                line_ix: 0,
            }
        );
    }

    #[test]
    fn prepared_diff_syntax_line_for_inline_diff_row_rejects_meta_rows_and_missing_lines() {
        use gitcomet_core::domain::DiffLineKind;

        let document = prepare_test_document(DiffSyntaxLanguage::Rust, "let value = 1;");

        let header_line = AnnotatedDiffLine {
            kind: DiffLineKind::Header,
            text: Arc::from("diff --git a/file b/file"),
            old_line: None,
            new_line: None,
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(document),
                Some(document),
                &header_line
            ),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );

        let missing_add_line = AnnotatedDiffLine {
            kind: DiffLineKind::Add,
            text: Arc::from("+let value = 1;"),
            old_line: None,
            new_line: None,
        };
        assert_eq!(
            prepared_diff_syntax_line_for_inline_diff_row(
                Some(document),
                Some(document),
                &missing_add_line,
            ),
            PreparedDiffSyntaxLine {
                document: None,
                line_ix: 0,
            }
        );
    }

    #[test]
    fn inline_projection_tokens_come_from_correct_document_side() {
        use gitcomet_core::domain::DiffLineKind;

        // Old document contains a struct definition; new document contains a function.
        // We verify that projected syntax tokens actually carry the expected token kinds
        // from the correct side, not just that the document/line_ix are set correctly.
        let old_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["struct Foo {", "    x: u32,", "}"].join("\n"),
        );
        let new_document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &["fn bar() {", "    let y = 42;", "}"].join("\n"),
        );

        // Remove line: old_line=1 should project from old document line 0 ("struct Foo {")
        let remove_line = AnnotatedDiffLine {
            kind: DiffLineKind::Remove,
            text: Arc::from("-struct Foo {"),
            old_line: Some(1),
            new_line: None,
        };
        let projected = prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &remove_line,
        );
        assert_eq!(projected.document, Some(old_document));
        let old_tokens = syntax::syntax_tokens_for_prepared_document_line(
            projected.document.unwrap().inner,
            projected.line_ix,
        );
        assert!(
            old_tokens
                .as_ref()
                .is_some_and(|tokens| tokens.iter().any(|t| t.kind == SyntaxTokenKind::Keyword)),
            "remove line should get tokens from old doc containing 'struct' keyword: {old_tokens:?}"
        );

        // Add line: new_line=2 should project from new document line 1 ("    let y = 42;")
        let add_line = AnnotatedDiffLine {
            kind: DiffLineKind::Add,
            text: Arc::from("+    let y = 42;"),
            old_line: None,
            new_line: Some(2),
        };
        let projected = prepared_diff_syntax_line_for_inline_diff_row(
            Some(old_document),
            Some(new_document),
            &add_line,
        );
        assert_eq!(projected.document, Some(new_document));
        let new_tokens = syntax::syntax_tokens_for_prepared_document_line(
            projected.document.unwrap().inner,
            projected.line_ix,
        );
        assert!(
            new_tokens
                .as_ref()
                .is_some_and(|tokens| tokens.iter().any(|t| t.kind == SyntaxTokenKind::Number)),
            "add line should get tokens from new doc containing number literal: {new_tokens:?}"
        );
    }

    #[test]
    fn split_view_projection_indexes_real_document_lines() {
        // Verify that prepared_diff_syntax_line_for_one_based_line gives correct
        // syntax tokens when the document is built from real file text rather than
        // the old aligned-row approach (which padded empty lines).
        let document = prepare_test_document(
            DiffSyntaxLanguage::Rust,
            &[
                "fn greet() {",          // line 1
                "    println!(\"hi\");", // line 2
                "}",                     // line 3
            ]
            .join("\n"),
        );

        // Line 1 should have a keyword ("fn")
        let line1 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(1));
        assert_eq!(line1.line_ix, 0);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(
            line1.document.unwrap().inner,
            line1.line_ix,
        );
        assert!(
            tokens
                .as_ref()
                .is_some_and(|t| t.iter().any(|tok| tok.kind == SyntaxTokenKind::Keyword)),
            "line 1 should contain 'fn' keyword: {tokens:?}"
        );

        // Line 2 should have a string
        let line2 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(2));
        assert_eq!(line2.line_ix, 1);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(
            line2.document.unwrap().inner,
            line2.line_ix,
        );
        assert!(
            tokens
                .as_ref()
                .is_some_and(|t| t.iter().any(|tok| tok.kind == SyntaxTokenKind::String)),
            "line 2 should contain a string literal: {tokens:?}"
        );

        // Line 3 should be just punctuation (closing brace)
        let line3 = prepared_diff_syntax_line_for_one_based_line(Some(document), Some(3));
        assert_eq!(line3.line_ix, 2);
        let tokens = syntax::syntax_tokens_for_prepared_document_line(
            line3.document.unwrap().inner,
            line3.line_ix,
        );
        assert!(
            tokens.as_ref().is_some_and(|t| t
                .iter()
                .any(|tok| tok.kind == SyntaxTokenKind::PunctuationBracket)),
            "line 3 should contain punctuation bracket: {tokens:?}"
        );
    }

    #[test]
    fn query_overlay_reuses_base_when_query_is_empty_or_missing() {
        let theme = AppTheme::gitcomet_dark();
        let text: SharedString = "abcdef".into();
        let mut text_hasher = FxHasher::default();
        text.as_ref().hash(&mut text_hasher);
        let text_hash = text_hasher.finish();
        let style = gpui::HighlightStyle {
            color: Some(theme.colors.text.into()),
            ..Default::default()
        };
        let base = CachedDiffStyledText {
            text,
            highlights: Arc::new(vec![(0..6, style)]),
            highlights_hash: 42,
            text_hash,
        };

        let empty_query = build_cached_diff_query_overlay_styled_text(theme, &base, "");
        assert!(Arc::ptr_eq(&empty_query.highlights, &base.highlights));
        assert_eq!(empty_query.highlights_hash, base.highlights_hash);

        let missing_query = build_cached_diff_query_overlay_styled_text(theme, &base, "xyz");
        assert!(Arc::ptr_eq(&missing_query.highlights, &base.highlights));
        assert_eq!(missing_query.highlights_hash, base.highlights_hash);
    }

    #[test]
    fn query_overlay_adds_background_without_losing_existing_color() {
        let theme = AppTheme::gitcomet_dark();
        let text: SharedString = "abcdef".into();
        let mut text_hasher = FxHasher::default();
        text.as_ref().hash(&mut text_hasher);
        let text_hash = text_hasher.finish();
        let style = gpui::HighlightStyle {
            color: Some(theme.colors.warning.into()),
            ..Default::default()
        };
        let base = CachedDiffStyledText {
            text,
            highlights: Arc::new(vec![(0..6, style)]),
            highlights_hash: 7,
            text_hash,
        };

        let overlaid = build_cached_diff_query_overlay_styled_text(theme, &base, "cd");
        assert_eq!(overlaid.highlights.len(), 3);
        assert_eq!(overlaid.highlights[1].0, 2..4);
        assert_eq!(
            overlaid.highlights[1].1.color,
            Some(theme.colors.warning.into())
        );
        assert!(overlaid.highlights[1].1.background_color.is_some());
        assert_ne!(overlaid.highlights_hash, base.highlights_hash);
    }
}
