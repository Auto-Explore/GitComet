use super::super::diff_canvas;
use super::super::diff_text::*;
use super::super::*;
use super::PatchSplitColumn;
use crate::view::panes::diff::diff_search::DiffSearchMatcher;
use crate::view::panes::diff::diff_search::DiffSearchOptions;
use gitcomet_core::file_diff::FileDiffRowKind;

/// Same, for a file diff split column.
/// Left highlights Remove/Modify; Right highlights Add/Modify.
pub(super) fn file_diff_split_word_kind(
    column: PatchSplitColumn,
    kind: FileDiffRowKind,
) -> Option<crate::theme::DiffColorKind> {
    match column {
        PatchSplitColumn::Left => matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify)
            .then_some(crate::theme::DiffColorKind::Removed),
        PatchSplitColumn::Right => matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify)
            .then_some(crate::theme::DiffColorKind::Added),
    }
}

fn streamed_diff_text_spec_with_syntax(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    word_ranges: Arc<[Range<usize>]>,
    word_kind: Option<crate::theme::DiffColorKind>,
    syntax: diff_canvas::StreamedDiffTextSyntaxSource,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    diff_canvas::is_streamable_diff_text(&raw_text).then(|| {
        diff_canvas::StreamedDiffTextPaintSpec {
            raw_text,
            query: query.clone(),
            query_options,
            query_matcher,
            query_emphasis: DiffSearchMatchEmphasis::Other,
            word_ranges,
            word_kind,
            syntax,
        }
    })
}

pub(super) fn heuristic_streamed_diff_text_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    word_ranges: Arc<[Range<usize>]>,
    word_kind: Option<crate::theme::DiffColorKind>,
    language: Option<rows::DiffSyntaxLanguage>,
    mode: rows::DiffSyntaxMode,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    let syntax = match language {
        Some(language) => diff_canvas::StreamedDiffTextSyntaxSource::Heuristic { language, mode },
        None => diff_canvas::StreamedDiffTextSyntaxSource::None,
    };
    streamed_diff_text_spec_with_syntax(
        raw_text,
        query,
        query_options,
        query_matcher,
        word_ranges,
        word_kind,
        syntax,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn prepared_streamed_diff_text_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    query_options: DiffSearchOptions,
    query_matcher: Option<Arc<DiffSearchMatcher>>,
    word_ranges: Arc<[Range<usize>]>,
    word_kind: Option<crate::theme::DiffColorKind>,
    language: Option<rows::DiffSyntaxLanguage>,
    fallback_mode: rows::DiffSyntaxMode,
    document_text: Arc<str>,
    line_starts: Arc<[usize]>,
    prepared_line: rows::PreparedDiffSyntaxLine,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    let syntax = match (language, prepared_line.document) {
        (Some(language), Some(document)) => diff_canvas::StreamedDiffTextSyntaxSource::Prepared {
            document_text,
            line_starts,
            document,
            language,
            line_ix: prepared_line.line_ix,
        },
        (Some(language), None) => diff_canvas::StreamedDiffTextSyntaxSource::Heuristic {
            language,
            mode: fallback_mode,
        },
        (None, _) => diff_canvas::StreamedDiffTextSyntaxSource::None,
    };
    streamed_diff_text_spec_with_syntax(
        raw_text,
        query,
        query_options,
        query_matcher,
        word_ranges,
        word_kind,
        syntax,
    )
}

pub(super) fn build_file_diff_cached_styled_text(
    tab_width: usize,
    theme: AppTheme,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    word_ranges: &[Range<usize>],
    context_prefix: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_kind: Option<crate::theme::DiffColorKind>,
) -> CachedDiffStyledText {
    if should_truncate_file_diff_display(raw_text) {
        let display = file_diff_display_text(tab_width, raw_text);
        return build_cached_diff_styled_text(
            tab_width,
            theme,
            display.as_ref(),
            &[],
            context_prefix,
            None,
            DiffSyntaxMode::HeuristicOnly,
            None,
        );
    }

    build_cached_diff_styled_text(
        tab_width,
        theme,
        raw_text.as_ref(),
        word_ranges,
        context_prefix,
        language,
        syntax_mode,
        word_kind,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
    tab_width: usize,
    theme: AppTheme,
    raw_text: &gitcomet_core::file_diff::FileDiffLineText,
    word_ranges: &[Range<usize>],
    context_prefix: &str,
    syntax: DiffSyntaxConfig,
    word_kind: Option<crate::theme::DiffColorKind>,
    projected: rows::PreparedDiffSyntaxLine,
) -> (CachedDiffStyledText, bool) {
    if should_truncate_file_diff_display(raw_text) {
        let display = file_diff_display_text(tab_width, raw_text);
        return (
            build_cached_diff_styled_text(
                tab_width,
                theme,
                display.as_ref(),
                &[],
                context_prefix,
                None,
                DiffSyntaxMode::HeuristicOnly,
                None,
            ),
            false,
        );
    }

    build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
        tab_width,
        theme,
        raw_text.as_ref(),
        word_ranges,
        context_prefix,
        syntax,
        word_kind,
        projected,
    )
    .into_parts()
}

pub(super) fn file_diff_split_side_text(
    row: &FileDiffRow,
    is_left: bool,
) -> Option<&gitcomet_core::file_diff::FileDiffLineText> {
    if is_left {
        row.old.as_ref()
    } else {
        row.new.as_ref()
    }
}

pub(super) fn file_diff_split_side_text_owned(
    row: &FileDiffRow,
    is_left: bool,
) -> Option<gitcomet_core::file_diff::FileDiffLineText> {
    file_diff_split_side_text(row, is_left).cloned()
}

pub(super) fn file_diff_split_side_line(row: &FileDiffRow, is_left: bool) -> Option<u32> {
    if is_left { row.old_line } else { row.new_line }
}
