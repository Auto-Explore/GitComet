use super::super::perf::{self, ViewPerfSpan};
use super::*;
use gitcomet_core::domain::DiffLineKind;
use memchr::memchr2_iter;
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, OnceLock};

mod build;
mod display;
mod prepared;

pub(in crate::view) use display::{
    DIFF_TEXT_DERIVED_CACHE_MAX_ENTRIES, display_diff_text, display_plain_text,
    display_styled_slice, line_display_text,
};

use gitcomet_syntax as syntax;

pub(super) use crate::kit::syntax_style::{
    SyntaxHighlightPalette, syntax_highlight_color, syntax_highlight_palette,
    syntax_highlight_style,
};

use build::*;

pub(in crate::view) use build::DiffSearchMatchEmphasis;
pub(in crate::view) use build::PreparedDocumentByteRangeHighlights;
#[cfg(feature = "benchmarks")]
pub(super) use build::build_cached_diff_styled_text_with_palette;
pub(in crate::view::rows) use build::hash_rgba_bits;
pub(in crate::view::rows) use build::hash_text_content;
pub(in crate::view) use build::query_highlight_style;
pub(in crate::view) use build::syntax_highlights_for_line;
pub(in crate::view::rows) use build::syntax_theme_signature;
pub(in crate::view::rows) use build::word_highlight_colors;
pub(super) use build::{
    build_cached_diff_query_overlay_styled_text, build_cached_diff_styled_text,
    build_cached_diff_styled_text_from_relative_highlights,
    build_cached_diff_styled_text_with_source_identity, diff_line_colors,
    selectable_cached_diff_text,
};
/// Neither is the "are chunks still building?" query: only the resolved output
/// ever polled it, and the live engine has no pending state to poll.
#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) use prepared::has_pending_prepared_diff_syntax_chunk_builds_for_document;
#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) use prepared::prepare_diff_syntax_document_in_background_text;
/// Line-range highlights are no longer a render path — the resolved output is
/// the only view that ever asked for them, and it now goes through the live
/// engine. Kept for the diff-scroll bench warmup and its own unit tests; the
/// production request API is the byte-range one above.
#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) use prepared::request_syntax_highlights_for_prepared_document_line_range;
pub(super) use prepared::{
    PreparedDocumentLineStyledText,
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking,
    build_cached_diff_styled_text_for_prepared_document_line_nonblocking_with_palette,
};
#[cfg(feature = "benchmarks")]
#[allow(unused_imports)]
pub(in crate::view) use prepared::{
    benchmark_diff_syntax_cache_drop_payload_timed_step,
    benchmark_diff_syntax_cache_replacement_drop_step,
    benchmark_diff_syntax_prepared_cache_contains_document,
    benchmark_diff_syntax_prepared_cache_metrics,
    benchmark_diff_syntax_prepared_loaded_chunk_count,
    benchmark_flush_diff_syntax_deferred_drop_queue,
    benchmark_reset_diff_syntax_prepared_cache_metrics,
};
pub(in crate::view) use prepared::{
    build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking,
    drain_completed_prepared_diff_syntax_chunk_builds,
    drain_completed_prepared_diff_syntax_chunk_builds_for_document,
    has_pending_prepared_diff_syntax_chunk_builds, inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_in_background_text_with_reuse,
    prepare_diff_syntax_document_with_budget_reuse_text,
    prepared_diff_syntax_document_is_available, prepared_diff_syntax_line_for_inline_diff_row,
    prepared_diff_syntax_line_for_one_based_line,
    prepared_diff_syntax_occurrences_at_display_offset,
    prepared_diff_syntax_pair_at_display_offset, prepared_diff_syntax_reparse_seed,
    request_syntax_highlights_for_prepared_document_byte_range,
};
#[cfg(test)]
pub(in crate::view) use prepared::{
    prepared_diff_syntax_parse_mode, prepared_diff_syntax_source_version,
    syntax_highlights_for_prepared_document_byte_range,
};
#[cfg(test)]
pub(in crate::view) use syntax::OCCURRENCE_MAX_TEXT_BYTES;
#[cfg(test)]
pub(in crate::view) use syntax::SyntaxPairKind;
pub(in crate::view) use syntax::{
    DiffSyntaxBudget, DiffSyntaxEdit, DiffSyntaxLanguage, DiffSyntaxMode, LiveSyntaxDocument,
    LiveSyntaxSyncOutcome, PREPARED_DIFF_SYNTAX_DOCUMENT_MAX_TEXT_BYTES, SyntaxPair,
    diff_syntax_language_for_path, shared_byte_affix_bounds,
};
pub(in crate::view) use syntax::{live_syntax_document_supported, live_syntax_reparse};

/// A live document snapshot styled with the theme's syntax palette.
pub(in crate::view) type LiveSyntaxSnapshot = syntax::LiveSyntaxSnapshot<SyntaxHighlightPalette>;

/// The foreground parse budget for the installed UI runtime.
pub(in crate::view) fn default_diff_syntax_budget() -> DiffSyntaxBudget {
    DiffSyntaxBudget {
        foreground_parse: crate::ui_runtime::current().diff_syntax_foreground_parse_budget(),
    }
}

pub(super) fn syntax_highlights_for_streamed_line_slice_heuristic(
    theme: AppTheme,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    language: DiffSyntaxLanguage,
    requested_slice_range: Range<usize>,
    resolved_slice_range: Range<usize>,
) -> Option<Vec<(Range<usize>, gpui::HighlightStyle)>> {
    let tokens = syntax::syntax_tokens_for_streamed_line_slice_heuristic(
        raw_text,
        language,
        requested_slice_range,
        resolved_slice_range.clone(),
    )?;
    Some(prepared_document_line_highlights_from_tokens(
        theme,
        resolved_slice_range.len(),
        tokens.as_slice(),
    ))
}

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

#[cfg(test)]
pub(in crate::view) fn whitespace_visible_text(text: &str) -> SharedString {
    whitespace_visible_text_and_highlights(text, &[]).0
}

pub(in crate::view) fn whitespace_visible_line_text(text: &str) -> SharedString {
    whitespace_visible_line_text_and_highlights(text, &[]).0
}

#[cfg(test)]
pub(in crate::view) fn whitespace_visible_multiline_text(text: &str) -> SharedString {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            ' ' => out.push('\u{00B7}'),
            '\t' => out.push('\u{2192}'),
            '\r' => out.push('\u{240D}'),
            '\n' => {
                out.push('\u{21B5}');
                out.push('\n');
            }
            _ if ch.is_whitespace() => out.push('\u{2420}'),
            _ => out.push(ch),
        }
    }
    out.into()
}

#[cfg(test)]
pub(in crate::view) fn whitespace_visible_text_and_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    whitespace_visible_text_and_highlights_impl(text, highlights, false)
}

pub(in crate::view) fn whitespace_visible_line_text_and_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    whitespace_visible_text_and_highlights_impl(text, highlights, true)
}

pub(in crate::view) fn whitespace_visible_styled_text(
    styled: &CachedDiffStyledText,
    append_eol_marker: bool,
) -> CachedDiffStyledText {
    let (text, highlights) = whitespace_visible_text_and_highlights_impl(
        styled.text.as_ref(),
        styled.highlights.as_ref(),
        append_eol_marker,
    );
    let text_hash = hash_text_content(text.as_ref());
    let highlights_hash = hash_visible_highlights(&highlights);
    CachedDiffStyledText {
        text,
        highlights: Arc::from(highlights),
        highlights_hash,
        text_hash,
    }
}

pub(in crate::view) fn whitespace_visible_line_styled_text(
    styled: &CachedDiffStyledText,
) -> CachedDiffStyledText {
    whitespace_visible_styled_text(styled, true)
}

pub(in crate::view) fn whitespace_visible_line_styled_text_for_raw(
    tab_width: usize,
    styled: &CachedDiffStyledText,
    raw_text: &str,
) -> CachedDiffStyledText {
    if raw_text.contains('\t') && styled.text.as_ref() != raw_text {
        let raw_highlights =
            expanded_highlights_to_raw_text(tab_width, raw_text, styled.highlights.as_ref());
        let (text, highlights) =
            whitespace_visible_line_text_and_highlights(raw_text, &raw_highlights);
        let text_hash = hash_text_content(text.as_ref());
        let highlights_hash = hash_visible_highlights(&highlights);
        return CachedDiffStyledText {
            text,
            highlights: Arc::from(highlights),
            highlights_hash,
            text_hash,
        };
    }

    whitespace_visible_line_styled_text(styled)
}

#[cfg(test)]
pub(in crate::view) fn diff_wrap_row_count_for_text(
    tab_width: usize,
    text: &str,
    wrap_columns: usize,
) -> usize {
    if text.is_empty() {
        return 1;
    }

    let mut row_start = 0usize;
    let mut line_column = 0usize;
    let mut count = 0usize;
    let wrap_columns = wrap_columns.max(1);
    while row_start < text.len() {
        let Some((_, next_start)) = diff_wrap_next_range_for_text(
            tab_width,
            text,
            wrap_columns,
            row_start,
            &mut line_column,
        ) else {
            break;
        };
        count += 1;
        if next_start <= row_start {
            break;
        }
        row_start = next_start;
    }
    count.max(1)
}

#[cfg(test)]
pub(in crate::view) fn diff_wrap_range_for_text(
    tab_width: usize,
    text: &str,
    wrap_columns: usize,
    wrap_ix: usize,
) -> Option<Range<usize>> {
    if text.is_empty() {
        return (wrap_ix == 0).then_some(0..0);
    }

    let mut row_start = 0usize;
    let mut row_ix = 0usize;
    let mut line_column = 0usize;
    let wrap_columns = wrap_columns.max(1);
    while row_start < text.len() {
        let (range, next_start) = diff_wrap_next_range_for_text(
            tab_width,
            text,
            wrap_columns,
            row_start,
            &mut line_column,
        )?;
        if row_ix == wrap_ix {
            return Some(range);
        }
        if next_start <= row_start {
            break;
        }
        row_start = next_start;
        row_ix += 1;
    }
    None
}

pub(in crate::view) fn slice_cached_diff_styled_text(
    styled: &CachedDiffStyledText,
    range: Range<usize>,
) -> CachedDiffStyledText {
    let start = clamp_to_char_boundary(styled.text.as_ref(), range.start.min(styled.text.len()));
    let end = clamp_to_char_boundary(styled.text.as_ref(), range.end.min(styled.text.len()));
    if start >= end {
        return CachedDiffStyledText {
            text: SharedString::default(),
            highlights: Arc::from(Vec::new()),
            highlights_hash: 0,
            text_hash: hash_text_content(""),
        };
    }

    let text: SharedString = styled.text.as_ref()[start..end].to_string().into();
    let mut highlights = Vec::new();
    for (range, style) in styled.highlights.as_ref() {
        let clipped_start = range.start.max(start);
        let clipped_end = range.end.min(end);
        if clipped_start < clipped_end {
            highlights.push((clipped_start - start..clipped_end - start, *style));
        }
    }
    let text_hash = hash_text_content(text.as_ref());
    let highlights_hash = hash_visible_highlights(&highlights);
    CachedDiffStyledText {
        text,
        highlights: Arc::from(highlights),
        highlights_hash,
        text_hash,
    }
}

pub(in crate::view) fn diff_wrap_ranges_for_text(
    tab_width: usize,
    text: &str,
    wrap_columns: usize,
) -> Vec<Range<usize>> {
    if text.is_empty() {
        return std::iter::once(0..0).collect();
    }

    let wrap_columns = wrap_columns.max(1);
    let mut ranges = Vec::new();
    let mut row_start = 0usize;
    let mut line_column = 0usize;

    while row_start < text.len() {
        let Some((range, next_start)) = diff_wrap_next_range_for_text(
            tab_width,
            text,
            wrap_columns,
            row_start,
            &mut line_column,
        ) else {
            break;
        };
        ranges.push(range);
        if next_start <= row_start {
            break;
        }
        row_start = next_start;
    }

    if ranges.is_empty() {
        ranges.push(0..0);
    }
    ranges
}

/// The next wrapped row from `row_start`. `line_column` is the line's column
/// at `row_start` — a tab's width depends on it — and is advanced to the
/// column where the next row starts.
fn diff_wrap_next_range_for_text(
    tab_width: usize,
    text: &str,
    wrap_columns: usize,
    row_start: usize,
    line_column: &mut usize,
) -> Option<(Range<usize>, usize)> {
    if row_start >= text.len() || !text.is_char_boundary(row_start) {
        return None;
    }

    let mut end = row_start;
    let mut column = 0usize;
    let mut last_break = None;
    let mut forced_newline = false;
    let mut saw_char = false;
    // Line columns at `end` and at `last_break`, so the next row knows its own.
    let mut end_line_column = *line_column;
    let mut break_line_column = *line_column;

    for (rel_start, ch) in text[row_start..].char_indices() {
        let start = row_start + rel_start;
        let char_end = start + ch.len_utf8();
        let width = crate::view::tab_width::char_columns(tab_width, ch, end_line_column);
        if column > 0 && column + width > wrap_columns {
            break;
        }

        saw_char = true;
        column += width;
        end = char_end;
        end_line_column += width;
        if ch.is_whitespace() {
            last_break = Some(char_end);
            break_line_column = end_line_column;
        }
        if ch == '\n' {
            forced_newline = true;
            break;
        }
        if column >= wrap_columns {
            break;
        }
    }

    if !saw_char {
        return None;
    }

    let next_start = if end >= text.len() || forced_newline {
        end
    } else if text[end..].chars().next().is_some_and(char::is_whitespace) {
        // The word ended exactly on the column boundary, so the row is not
        // splitting one — keep it full and let the following whitespace open
        // the next row. Backing up to `last_break` here would drop a whole
        // word that fit, wrapping the line short of the width it had.
        end
    } else {
        last_break
            .filter(|break_ix| *break_ix > row_start)
            .unwrap_or(end)
    };
    let next_start = if next_start > row_start {
        next_start
    } else {
        row_start + text[row_start..].chars().next()?.len_utf8()
    };
    *line_column = if next_start == end {
        end_line_column
    } else if Some(next_start) == last_break {
        break_line_column
    } else {
        // A single character wider than the row: count it from the start.
        let ch = text[row_start..].chars().next()?;
        *line_column + crate::view::tab_width::char_columns(tab_width, ch, *line_column)
    };

    Some((row_start..next_start, next_start))
}

fn clamp_to_char_boundary(text: &str, mut ix: usize) -> usize {
    while ix > 0 && !text.is_char_boundary(ix) {
        ix -= 1;
    }
    ix
}

fn expanded_highlights_to_raw_text(
    tab_width: usize,
    raw_text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let mut expanded_to_raw = Vec::with_capacity(raw_text.len() + 1);
    expanded_to_raw.push(0);
    let mut column = 0usize;
    for (raw_start, ch) in raw_text.char_indices() {
        let raw_end = raw_start + ch.len_utf8();
        let expanded_len = crate::view::tab_width::char_expanded_len(tab_width, ch, column);
        column += crate::view::tab_width::char_columns(tab_width, ch, column);
        for _ in 0..expanded_len {
            expanded_to_raw.push(raw_end);
        }
    }

    let mut remapped = Vec::with_capacity(highlights.len());
    for (range, style) in highlights {
        let Some(&start) = expanded_to_raw.get(range.start) else {
            continue;
        };
        let Some(&end) = expanded_to_raw.get(range.end) else {
            continue;
        };
        if start < end {
            remapped.push((start..end, *style));
        }
    }
    remapped
}

fn whitespace_visible_text_and_highlights_impl(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
    append_eol_marker: bool,
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    let mut out = String::with_capacity(text.len() + usize::from(append_eol_marker));
    let mut byte_map = vec![0usize; text.len() + 1];

    for (start, ch) in text.char_indices() {
        byte_map[start] = out.len();
        match ch {
            ' ' => out.push('\u{00B7}'),
            '\t' => out.push('\u{2192}'),
            '\r' => out.push('\u{240D}'),
            '\n' => out.push('\u{21B5}'),
            _ if ch.is_whitespace() => out.push('\u{2420}'),
            _ => out.push(ch),
        }
        let end = start + ch.len_utf8();
        let mapped_end = out.len();
        for mapped in byte_map.iter_mut().take(end + 1).skip(start + 1) {
            *mapped = mapped_end;
        }
    }

    if append_eol_marker && !text.ends_with('\n') {
        out.push('\u{21B5}');
    }

    let mut remapped = Vec::with_capacity(highlights.len());
    for (range, style) in highlights {
        let start = *byte_map.get(range.start).unwrap_or(&out.len());
        let end = *byte_map.get(range.end).unwrap_or(&out.len());
        if start < end {
            remapped.push((start..end, *style));
        }
    }

    (out.into(), remapped)
}

fn hash_visible_highlights(highlights: &[(Range<usize>, gpui::HighlightStyle)]) -> u64 {
    if highlights.is_empty() {
        return 0;
    }

    let mut hasher = FxHasher::default();
    for (range, style) in highlights {
        range.hash(&mut hasher);
        crate::text_runs::hash_highlight_style(style, &mut hasher);
    }
    hasher.finish()
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

const SINGLE_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES: usize = 4_096;
const PREPARED_READY_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES: usize = 32_768;
const SINGLE_LINE_STYLED_TEXT_CACHE_MAX_SOURCE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) struct DiffTextSourceIdentity {
    source_ptr: usize,
    source_len: usize,
}

impl DiffTextSourceIdentity {
    pub(super) fn from_str(text: &str) -> Self {
        Self {
            source_ptr: text.as_ptr() as usize,
            source_len: text.len(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SingleLineTextSourceCacheKey {
    Hashed(u64),
    Identity(DiffTextSourceIdentity),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SingleLineStyledTextCacheKey {
    tab_width: usize,
    language: DiffSyntaxLanguage,
    mode: DiffSyntaxMode,
    theme_signature: u64,
    source: SingleLineTextSourceCacheKey,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct PreparedReadyLineStyledTextCacheKey {
    tab_width: usize,
    theme_signature: u64,
    source_ptr: usize,
    source_len: usize,
    tokens_ptr: usize,
    tokens_len: usize,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SingleLineWordHighlightedTextCacheKey {
    tab_width: usize,
    language: Option<DiffSyntaxLanguage>,
    mode: DiffSyntaxMode,
    theme_signature: u64,
    source: SingleLineTextSourceCacheKey,
    word_ranges_hash: u64,
}

#[derive(Clone)]
struct CachedSingleLineStyledText {
    source_text: Option<Arc<str>>,
    styled: CachedDiffStyledText,
}

#[derive(Clone)]
struct CachedSingleLineWordHighlightedText {
    source_text: Option<Arc<str>>,
    word_ranges: Arc<[Range<usize>]>,
    styled: CachedDiffStyledText,
}

#[derive(Clone)]
struct CachedPreparedReadyLineStyledText {
    source_text: Arc<str>,
    tokens: Arc<[syntax::SyntaxToken]>,
    styled: CachedDiffStyledText,
}

#[derive(Clone, Copy)]
struct CachedSingleLineThemeSignature {
    theme: AppTheme,
    signature: u64,
}

struct SingleLineStyledTextCache {
    by_key: FxLruCache<SingleLineStyledTextCacheKey, CachedSingleLineStyledText>,
    word_highlighted_by_key:
        FxLruCache<SingleLineWordHighlightedTextCacheKey, CachedSingleLineWordHighlightedText>,
    prepared_by_key:
        FxLruCache<PreparedReadyLineStyledTextCacheKey, CachedPreparedReadyLineStyledText>,
    cached_theme_signature: Option<CachedSingleLineThemeSignature>,
}

impl SingleLineStyledTextCache {
    fn new() -> Self {
        Self {
            by_key: new_fx_lru_cache(SINGLE_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES),
            word_highlighted_by_key: new_fx_lru_cache(SINGLE_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES),
            // Prepared ready-line scroll workloads can revisit tens of thousands
            // of line/side pairs before cycling back to the starting window.
            // Keep a larger LRU here so large prepared documents retain their
            // warmed row styles instead of thrashing after a single sweep.
            prepared_by_key: new_fx_lru_cache(PREPARED_READY_LINE_STYLED_TEXT_CACHE_MAX_ENTRIES),
            cached_theme_signature: None,
        }
    }

    fn theme_signature(&mut self, theme: AppTheme) -> u64 {
        if let Some(cached) = self.cached_theme_signature
            && cached.theme == theme
        {
            return cached.signature;
        }

        let signature = syntax_theme_signature(theme);
        self.cached_theme_signature = Some(CachedSingleLineThemeSignature { theme, signature });
        signature
    }

    fn key_for(
        &mut self,
        tab_width: usize,
        theme: AppTheme,
        language: DiffSyntaxLanguage,
        mode: DiffSyntaxMode,
        text: &str,
        source_identity: Option<DiffTextSourceIdentity>,
    ) -> SingleLineStyledTextCacheKey {
        SingleLineStyledTextCacheKey {
            tab_width,
            language,
            mode,
            theme_signature: self.theme_signature(theme),
            source: Self::source_key(text, source_identity),
        }
    }

    fn prepared_key_for(
        &mut self,
        tab_width: usize,
        theme: AppTheme,
        text: &str,
        tokens: &Arc<[syntax::SyntaxToken]>,
    ) -> PreparedReadyLineStyledTextCacheKey {
        PreparedReadyLineStyledTextCacheKey {
            tab_width,
            theme_signature: self.theme_signature(theme),
            source_ptr: text.as_ptr() as usize,
            source_len: text.len(),
            tokens_ptr: tokens.as_ptr() as usize,
            tokens_len: tokens.len(),
        }
    }

    fn word_highlighted_key_for(
        &mut self,
        tab_width: usize,
        theme: AppTheme,
        language: Option<DiffSyntaxLanguage>,
        mode: DiffSyntaxMode,
        text: &str,
        source_identity: Option<DiffTextSourceIdentity>,
        word_ranges: &[Range<usize>],
    ) -> SingleLineWordHighlightedTextCacheKey {
        SingleLineWordHighlightedTextCacheKey {
            tab_width,
            language,
            mode,
            theme_signature: self.theme_signature(theme),
            source: Self::source_key(text, source_identity),
            word_ranges_hash: hash_word_ranges(word_ranges),
        }
    }

    fn source_key(
        text: &str,
        source_identity: Option<DiffTextSourceIdentity>,
    ) -> SingleLineTextSourceCacheKey {
        source_identity
            .filter(|identity| identity.source_len == text.len())
            .map_or_else(
                || SingleLineTextSourceCacheKey::Hashed(hash_text_content(text)),
                SingleLineTextSourceCacheKey::Identity,
            )
    }

    fn get(
        &mut self,
        key: SingleLineStyledTextCacheKey,
        text: &str,
    ) -> Option<CachedDiffStyledText> {
        self.by_key
            .get(&key)
            .filter(|entry| {
                entry
                    .source_text
                    .as_deref()
                    .unwrap_or(entry.styled.text.as_ref())
                    == text
            })
            .map(|entry| entry.styled.clone())
    }

    fn get_word_highlighted(
        &mut self,
        key: SingleLineWordHighlightedTextCacheKey,
        text: &str,
        word_ranges: &[Range<usize>],
    ) -> Option<CachedDiffStyledText> {
        self.word_highlighted_by_key
            .get(&key)
            .filter(|entry| {
                entry
                    .source_text
                    .as_deref()
                    .unwrap_or(entry.styled.text.as_ref())
                    == text
                    && entry.word_ranges.as_ref() == word_ranges
            })
            .map(|entry| entry.styled.clone())
    }

    fn get_prepared(
        &mut self,
        key: PreparedReadyLineStyledTextCacheKey,
        text: &str,
        tokens: &Arc<[syntax::SyntaxToken]>,
    ) -> Option<CachedDiffStyledText> {
        self.prepared_by_key
            .get(&key)
            .filter(|entry| {
                entry.source_text.as_ref() == text && Arc::ptr_eq(&entry.tokens, tokens)
            })
            .map(|entry| entry.styled.clone())
    }

    fn insert(
        &mut self,
        key: SingleLineStyledTextCacheKey,
        text: &str,
        styled: CachedDiffStyledText,
    ) {
        self.by_key.put(
            key,
            CachedSingleLineStyledText {
                source_text: text.contains('\t').then(|| Arc::<str>::from(text)),
                styled,
            },
        );
    }

    fn insert_word_highlighted(
        &mut self,
        key: SingleLineWordHighlightedTextCacheKey,
        text: &str,
        word_ranges: &[Range<usize>],
        styled: CachedDiffStyledText,
    ) {
        self.word_highlighted_by_key.put(
            key,
            CachedSingleLineWordHighlightedText {
                source_text: text.contains('\t').then(|| Arc::<str>::from(text)),
                word_ranges: Arc::<[Range<usize>]>::from(word_ranges),
                styled,
            },
        );
    }

    fn insert_prepared(
        &mut self,
        key: PreparedReadyLineStyledTextCacheKey,
        text: &str,
        tokens: Arc<[syntax::SyntaxToken]>,
        styled: CachedDiffStyledText,
    ) {
        let source_text = if text.contains('\t') {
            Arc::<str>::from(text)
        } else {
            Arc::<str>::from(styled.text.clone())
        };
        self.prepared_by_key.put(
            key,
            CachedPreparedReadyLineStyledText {
                source_text,
                tokens,
                styled,
            },
        );
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

type DiffTextHighlight = (Range<usize>, gpui::HighlightStyle);
type DiffTextHighlights = [DiffTextHighlight];
type SharedDiffTextHighlights = Arc<DiffTextHighlights>;

#[derive(Clone, Copy)]
pub(super) struct DiffTextBuildRequest<'a> {
    pub(super) text: &'a str,
    pub(super) word_ranges: &'a [Range<usize>],
    pub(super) query: &'a str,
    pub(super) syntax: DiffSyntaxConfig,
    pub(super) word_kind: Option<crate::theme::DiffColorKind>,
}

#[derive(Clone, Copy)]
pub(super) struct PreparedDiffTextBuildRequest<'a> {
    pub(super) build: DiffTextBuildRequest<'a>,
    pub(super) prepared_line: PreparedDiffSyntaxLine,
}

#[derive(Clone, Copy)]
pub(in crate::view) struct PreparedDiffSyntaxTextSource {
    pub document: Option<PreparedDiffSyntaxDocument>,
}

#[derive(Clone, Copy)]
pub(in crate::view) struct InlineDiffSyntaxOnlyRow<'a> {
    pub line: &'a AnnotatedDiffLine,
    pub text: &'a str,
}

#[derive(Clone, Copy)]
struct FusedDiffTextBuildRequest<'a> {
    build: DiffTextBuildRequest<'a>,
    syntax_tokens_override: Option<&'a [syntax::SyntaxToken]>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) struct PreparedDiffSyntaxLine {
    pub document: Option<PreparedDiffSyntaxDocument>,
    pub line_ix: usize,
}

#[cfg(test)]
mod tests;
