use super::super::diff_canvas;
use super::super::diff_text::*;
use super::super::*;
use crate::view::panes::diff::diff_search::DiffSearchMatcher;
use crate::view::panes::diff::diff_search::DiffSearchOptions;
use crate::view::panes::diff::helpers::state::CollapsedDiffVisibleRow;
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;

use super::DiffRowsRenderPass;
use super::PatchSplitColumn;
use super::SplitDiffRowsRenderPass;
use super::blame_presentation::BlamePrev;
use super::blame_presentation::build_row_blame_paint_tracked;
use super::patch_rows::DiffRowDecor;
use super::patch_rows::DiffRowStyle;
use super::patch_rows::collapsed_hunk_shell_width;
use super::patch_rows::collapsed_inline_header_row;
use super::patch_rows::collapsed_split_header_row;
use super::patch_rows::diff_line_word_kind;
use super::patch_rows::diff_placeholder_row;
use super::patch_rows::diff_row;
use super::patch_rows::patch_split_column_row;

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

pub(super) fn render_collapsed_diff_rows(
    this: &mut DiffRenderer,
    range: Range<usize>,
    pass: DiffRowsRenderPass,
    cx: &mut gpui::Context<DiffRenderer>,
) -> Vec<AnyElement> {
    let mark_lines = !this.structural_diff_active();
    let DiffRowsRenderPass {
        tab_width,
        annot_hover,
        stage_area,
        stage_hover,
        min_width,
        query,
        query_options,
        query_matcher,
        reveal_whitespace_chars,
        ui_scale_percent,
        annotation_width,
        blame_ctx,
    } = pass;

    let theme = this.theme;
    let language = this.file_diff_cache_language;
    let old_document_text: Arc<str> = this
        .file_diff_side_document_text(DiffTextRegion::SplitLeft)
        .into();
    let old_line_starts = Arc::clone(&this.file_diff_old_line_starts);
    let new_document_text: Arc<str> = this
        .file_diff_side_document_text(DiffTextRegion::SplitRight)
        .into();
    let new_line_starts = Arc::clone(&this.file_diff_new_line_starts);
    let pinned_hunk_shell_width = collapsed_hunk_shell_width(&this.diff_scroll, min_width);
    let pinned_hunk_shell_scroll = this.diff_scroll.clone();

    let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
    range
        .map(|visible_ix| {
            let selected = this
                .diff_selection_range
                .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
            let show_line_numbers = this.diff_show_line_numbers;
            let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
            let Some(source_visible_ix) = this.diff_source_visible_ix_for_visible_ix(visible_ix)
            else {
                return diff_placeholder_row(
                    ("collapsed_diff_missing", visible_ix),
                    theme,
                    ui_scale_percent,
                );
            };
            let Some(row) = this.collapsed_visible_row(source_visible_ix) else {
                return diff_placeholder_row(
                    ("collapsed_diff_missing", visible_ix),
                    theme,
                    ui_scale_percent,
                );
            };

            match row {
                CollapsedDiffVisibleRow::HunkHeader {
                    src_ix,
                    expansion_kind,
                    hidden_rows,
                    ..
                } => {
                    let display_src_ix = row.header_display_src_ix();
                    let display = display_src_ix
                        .and_then(|display_src_ix| {
                            this.collapsed_diff_hunk_header_display(display_src_ix)
                        })
                        .unwrap_or_default();
                    let context_menu_active = display_src_ix.is_some()
                        && this.active_repo_id().is_some_and(|repo_id| {
                            let invoker: SharedString =
                                format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                            this.active_context_menu_invoker.as_ref() == Some(&invoker)
                        });
                    let collapsed_hunk = this.collapsed_diff_hunk_for_src_ix(src_ix);

                    collapsed_inline_header_row(
                        theme,
                        ui_scale_percent,
                        visible_ix,
                        DiffClickKind::HunkHeader,
                        selected,
                        min_width,
                        pinned_hunk_shell_width,
                        pinned_hunk_shell_scroll.clone(),
                        collapsed_hunk,
                        None,
                        display,
                        None,
                        context_menu_active,
                        src_ix,
                        expansion_kind,
                        hidden_rows,
                        cx,
                    )
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } => {
                    let row_word_ranges = this.file_diff_inline_word_ranges(row_ix);
                    let Some(row) = this.file_diff_inline_render_data(row_ix) else {
                        return diff_placeholder_row(
                            ("collapsed_diff_oob", visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };
                    let visual_kind = this.file_diff_inline_visual_kind(row_ix);
                    let line = AnnotatedDiffLine {
                        kind: row.kind,
                        text: "".into(),
                        old_line: row.old_line,
                        new_line: row.new_line,
                    };
                    let streamed_spec = {
                        let line_language = matches!(
                            row.kind,
                            DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                        )
                        .then_some(language)
                        .flatten();
                        let word_kind = diff_line_word_kind(visual_kind);
                        let prepared_line = match row.kind {
                            DiffLineKind::Remove => {
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    this.file_diff_split_prepared_syntax_document(
                                        DiffTextRegion::SplitLeft,
                                    ),
                                    row.old_line,
                                )
                            }
                            DiffLineKind::Add | DiffLineKind::Context => {
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    this.file_diff_split_prepared_syntax_document(
                                        DiffTextRegion::SplitRight,
                                    ),
                                    row.new_line,
                                )
                            }
                            DiffLineKind::Header | DiffLineKind::Hunk => {
                                rows::prepared_diff_syntax_line_for_one_based_line(None, None)
                            }
                        };
                        let (document_text, line_starts) = match row.kind {
                            DiffLineKind::Remove => {
                                (Arc::clone(&old_document_text), Arc::clone(&old_line_starts))
                            }
                            DiffLineKind::Add | DiffLineKind::Context => {
                                (Arc::clone(&new_document_text), Arc::clone(&new_line_starts))
                            }
                            DiffLineKind::Header | DiffLineKind::Hunk => {
                                (Arc::clone(&new_document_text), Arc::clone(&new_line_starts))
                            }
                        };
                        let syntax_mode = DiffSyntaxMode::Auto;
                        prepared_streamed_diff_text_spec(
                            row.text.clone(),
                            &query,
                            query_options,
                            query_matcher.clone(),
                            row_word_ranges.clone(),
                            word_kind,
                            line_language,
                            syntax_mode,
                            document_text,
                            line_starts,
                            prepared_line,
                        )
                    };

                    let row_tint = this.hosted_row_tint(visible_ix, None);
                    let styled = if streamed_spec.is_some() {
                        None
                    } else {
                        let cache_epoch = this.file_diff_style_cache_epochs.inline_epoch(row.kind);
                        if this
                            .diff_text_segments_cache_get(row_ix, cache_epoch)
                            .is_none()
                        {
                            let word_kind = diff_line_word_kind(visual_kind);
                            let is_content_line = matches!(
                                line.kind,
                                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                            );
                            let line_language = is_content_line.then_some(language).flatten();
                            let projected = this.file_diff_inline_projected_syntax(&line);
                            let syntax_mode = DiffSyntaxMode::Auto;
                            let (styled, is_pending) =
                                build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                                    tab_width,
                                    theme,
                                    &row.text,
                                    &row_word_ranges,
                                    "",
                                    DiffSyntaxConfig {
                                        language: line_language,
                                        mode: syntax_mode,
                                    },
                                    word_kind,
                                    projected,
                                );
                            if is_pending {
                                this.ensure_prepared_syntax_chunk_poll(cx);
                            }
                            this.diff_text_segments_cache_set(row_ix, cache_epoch, styled);
                        }
                        this.diff_text_segments_cache_get_for_query(
                            row_ix,
                            query.as_ref(),
                            query_options,
                            cache_epoch,
                        )
                    };

                    diff_row(
                        DiffRowStyle {
                            tab_width,
                            theme,
                            ui_scale_percent,
                            mode: DiffViewMode::Inline,
                            min_width,
                            reveal_whitespace_chars,
                            show_line_numbers,
                            annotation_width,
                        },
                        visible_ix,
                        DiffClickKind::Line,
                        &line,
                        DiffRowDecor {
                            tint: row_tint,
                            selected,
                            visual_kind,
                            mark_lines,
                            file_stat: None,
                            header_display: None,
                            styled,
                            streamed_spec,
                            raw_text: Some(diff_canvas::DiffRowText::Line(&row.text)),
                            context_menu_active: false,
                            wrap,
                            row_blame: blame_ctx.as_ref().and_then(|ctx| {
                                build_row_blame_paint_tracked(
                                    ctx,
                                    matches!(visual_kind, DiffLineKind::Context),
                                    line.old_line,
                                    line.new_line,
                                    &blame_prev_nl,
                                    wrap,
                                    theme,
                                )
                            }),
                            annot_hover,
                            stage_area,
                            stage_hover,
                        },
                        cx,
                    )
                }
            }
        })
        .collect()
}

pub(super) fn render_file_diff_rows(
    this: &mut DiffRenderer,
    range: Range<usize>,
    pass: DiffRowsRenderPass,
    cx: &mut gpui::Context<DiffRenderer>,
) -> Vec<AnyElement> {
    let mark_lines = !this.structural_diff_active();
    let DiffRowsRenderPass {
        tab_width,
        annot_hover,
        stage_area,
        stage_hover,
        min_width,
        query,
        query_options,
        query_matcher,
        reveal_whitespace_chars,
        ui_scale_percent,
        annotation_width,
        blame_ctx,
    } = pass;

    let theme = this.theme;
    let language = this.file_diff_cache_language;
    let old_document_text: Arc<str> = this
        .file_diff_side_document_text(DiffTextRegion::SplitLeft)
        .into();
    let old_line_starts = Arc::clone(&this.file_diff_old_line_starts);
    let new_document_text: Arc<str> = this
        .file_diff_side_document_text(DiffTextRegion::SplitRight)
        .into();
    let new_line_starts = Arc::clone(&this.file_diff_new_line_starts);
    // Inline syntax is now projected from the real old/new (split)
    // documents instead of parsing a synthetic mixed inline stream.
    // syntax_mode is determined per-row based on projection availability.
    if let Some(language) = language {
        struct SyntaxOnlyBatchRow {
            inline_ix: usize,
            cache_epoch: u64,
            line: AnnotatedDiffLine,
            text: gitcomet_core::file_diff::FileDiffLineText,
        }

        let mut syntax_only_rows = Vec::new();
        for visible_ix in range.clone() {
            let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                continue;
            };
            let Some(row) = this.file_diff_inline_render_data(inline_ix) else {
                continue;
            };
            if diff_canvas::is_streamable_diff_text(&row.text) {
                continue;
            }
            if should_truncate_file_diff_display(&row.text) {
                continue;
            }
            let line = AnnotatedDiffLine {
                kind: row.kind,
                text: "".into(),
                old_line: row.old_line,
                new_line: row.new_line,
            };
            let cache_epoch = this.file_diff_style_cache_epochs.inline_epoch(row.kind);
            if this
                .diff_text_segments_cache_get(inline_ix, cache_epoch)
                .is_some()
            {
                continue;
            }
            if !matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                continue;
            }
            if !this.file_diff_inline_word_ranges(inline_ix).is_empty() {
                continue;
            }
            syntax_only_rows.push(SyntaxOnlyBatchRow {
                inline_ix,
                cache_epoch,
                line,
                text: row.text,
            });
        }

        if !syntax_only_rows.is_empty() {
            let batch_rows = syntax_only_rows
                .iter()
                .map(|row| InlineDiffSyntaxOnlyRow {
                    text: row.text.as_ref(),
                    line: &row.line,
                })
                .collect::<Vec<_>>();
            let batched_styles =
                build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
                    tab_width,
                    theme,
                    Some(language),
                    PreparedDiffSyntaxTextSource {
                        document: this
                            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                    },
                    PreparedDiffSyntaxTextSource {
                        document: this
                            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                    },
                    batch_rows.as_slice(),
                    DiffSyntaxMode::Auto,
                );
            let mut pending_batch = false;
            for (row, prepared) in syntax_only_rows.iter().zip(batched_styles) {
                let (styled, is_pending) = prepared.into_parts();
                pending_batch |= is_pending;
                this.diff_text_segments_cache_set(row.inline_ix, row.cache_epoch, styled);
            }
            if pending_batch {
                this.ensure_prepared_syntax_chunk_poll(cx);
            }
        }
    }

    let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
    range
        .map(|visible_ix| {
            let selected = this
                .diff_selection_range
                .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
            let show_line_numbers = this.diff_show_line_numbers;
            let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

            let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return diff_placeholder_row(("diff_missing", visible_ix), theme, ui_scale_percent);
            };
            let row_word_ranges = this.file_diff_inline_word_ranges(inline_ix);
            let visual_kind = this.file_diff_inline_visual_kind(inline_ix);
            let render_data = this.file_diff_inline_render_data(inline_ix);
            let streamed_spec = render_data.as_ref().and_then(|row| {
                let line_language = matches!(
                    row.kind,
                    DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                )
                .then_some(language)
                .flatten();
                let word_kind = diff_line_word_kind(visual_kind);
                let prepared_line = match row.kind {
                    DiffLineKind::Remove => rows::prepared_diff_syntax_line_for_one_based_line(
                        this.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                        row.old_line,
                    ),
                    DiffLineKind::Add | DiffLineKind::Context => {
                        rows::prepared_diff_syntax_line_for_one_based_line(
                            this.file_diff_split_prepared_syntax_document(
                                DiffTextRegion::SplitRight,
                            ),
                            row.new_line,
                        )
                    }
                    DiffLineKind::Header | DiffLineKind::Hunk => {
                        rows::prepared_diff_syntax_line_for_one_based_line(None, None)
                    }
                };
                let (document_text, line_starts) = match row.kind {
                    DiffLineKind::Remove => {
                        (Arc::clone(&old_document_text), Arc::clone(&old_line_starts))
                    }
                    DiffLineKind::Add | DiffLineKind::Context => {
                        (Arc::clone(&new_document_text), Arc::clone(&new_line_starts))
                    }
                    DiffLineKind::Header | DiffLineKind::Hunk => {
                        (Arc::clone(&new_document_text), Arc::clone(&new_line_starts))
                    }
                };
                let syntax_mode = DiffSyntaxMode::Auto;
                prepared_streamed_diff_text_spec(
                    row.text.clone(),
                    &query,
                    query_options,
                    query_matcher.clone(),
                    row_word_ranges.clone(),
                    word_kind,
                    line_language,
                    syntax_mode,
                    document_text,
                    line_starts,
                    prepared_line,
                )
            });

            let row_tint = this.hosted_row_tint(visible_ix, None);
            let (line, cache_epoch, styled) = if let Some(row) = render_data.as_ref() {
                let line = AnnotatedDiffLine {
                    kind: row.kind,
                    text: "".into(),
                    old_line: row.old_line,
                    new_line: row.new_line,
                };
                let cache_epoch = this.file_diff_style_cache_epochs.inline_epoch(row.kind);
                if streamed_spec.is_none()
                    && this
                        .diff_text_segments_cache_get(inline_ix, cache_epoch)
                        .is_none()
                {
                    let word_kind = diff_line_word_kind(visual_kind);
                    let is_content_line = matches!(
                        line.kind,
                        DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                    );
                    let line_language = is_content_line.then_some(language).flatten();
                    let projected = this.file_diff_inline_projected_syntax(&line);
                    let syntax_mode = DiffSyntaxMode::Auto;
                    let (styled, is_pending) =
                        build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                            tab_width,
                            theme,
                            &row.text,
                            &row_word_ranges,
                            "",
                            DiffSyntaxConfig {
                                language: line_language,
                                mode: syntax_mode,
                            },
                            word_kind,
                            projected,
                        );
                    if is_pending {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                    }
                    this.diff_text_segments_cache_set(inline_ix, cache_epoch, styled);
                }
                let styled = if streamed_spec.is_none() {
                    this.diff_text_segments_cache_get_for_query(
                        inline_ix,
                        query.as_ref(),
                        query_options,
                        cache_epoch,
                    )
                } else {
                    None
                };
                debug_assert!(
                    streamed_spec.is_some() || styled.is_some(),
                    "diff text segment cache missing for inline row {inline_ix} after populate"
                );
                (line, cache_epoch, styled)
            } else {
                let Some(line) = this.file_diff_inline_row(inline_ix) else {
                    return diff_placeholder_row(("diff_oob", visible_ix), theme, ui_scale_percent);
                };
                let cache_epoch = this.file_diff_inline_style_cache_epoch(&line);
                if this
                    .diff_text_segments_cache_get(inline_ix, cache_epoch)
                    .is_none()
                {
                    let word_kind = diff_line_word_kind(visual_kind);
                    let is_content_line = matches!(
                        line.kind,
                        DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                    );
                    let line_language = is_content_line.then_some(language).flatten();
                    let projected = this.file_diff_inline_projected_syntax(&line);
                    let syntax_mode = DiffSyntaxMode::Auto;
                    let (styled, is_pending) =
                        build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                            tab_width,
                            theme,
                            diff_content_text(&line),
                            &row_word_ranges,
                            "",
                            DiffSyntaxConfig {
                                language: line_language,
                                mode: syntax_mode,
                            },
                            word_kind,
                            projected,
                        )
                        .into_parts();
                    if is_pending {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                    }
                    this.diff_text_segments_cache_set(inline_ix, cache_epoch, styled);
                }

                let styled = this.diff_text_segments_cache_get_for_query(
                    inline_ix,
                    query.as_ref(),
                    query_options,
                    cache_epoch,
                );
                debug_assert!(
                    styled.is_some(),
                    "diff text segment cache missing for inline row {inline_ix} after populate"
                );
                (line, cache_epoch, styled)
            };
            let _ = cache_epoch;

            diff_row(
                DiffRowStyle {
                    tab_width,
                    theme,
                    ui_scale_percent,
                    mode: DiffViewMode::Inline,
                    min_width,
                    reveal_whitespace_chars,
                    show_line_numbers,
                    annotation_width,
                },
                visible_ix,
                DiffClickKind::Line,
                &line,
                DiffRowDecor {
                    tint: row_tint,
                    selected,
                    visual_kind,
                    mark_lines,
                    file_stat: None,
                    header_display: None,
                    styled,
                    streamed_spec,
                    raw_text: render_data
                        .as_ref()
                        .map(|row| diff_canvas::DiffRowText::Line(&row.text))
                        .or_else(|| Some(diff_canvas::DiffRowText::Str(diff_content_text(&line)))),
                    context_menu_active: false,
                    wrap,
                    row_blame: blame_ctx.as_ref().and_then(|ctx| {
                        build_row_blame_paint_tracked(
                            ctx,
                            matches!(visual_kind, DiffLineKind::Context),
                            line.old_line,
                            line.new_line,
                            &blame_prev_nl,
                            wrap,
                            theme,
                        )
                    }),
                    annot_hover,
                    stage_area,
                    stage_hover,
                },
                cx,
            )
        })
        .collect()
}

pub(super) fn render_collapsed_diff_split_rows(
    this: &mut DiffRenderer,
    range: Range<usize>,
    pass: SplitDiffRowsRenderPass,
    cx: &mut gpui::Context<DiffRenderer>,
) -> Vec<AnyElement> {
    let mark_lines = !this.structural_diff_active();
    let SplitDiffRowsRenderPass {
        common,
        column,
        is_left,
        region,
        id_missing,
        id_oob,
        ..
    } = pass;
    let DiffRowsRenderPass {
        tab_width,
        annot_hover,
        stage_area,
        stage_hover,
        min_width,
        query,
        query_options,
        query_matcher,
        reveal_whitespace_chars,
        ui_scale_percent,
        annotation_width,
        blame_ctx,
    } = common;

    let theme = this.theme;
    let language = this.file_diff_cache_language;
    let cache_epoch = this.file_diff_split_style_cache_epoch(region);
    let syntax_document = this.file_diff_split_prepared_syntax_document(region);
    let syntax_mode = DiffSyntaxMode::Auto;
    let document_text: Arc<str> = if is_left {
        this.file_diff_side_document_text(DiffTextRegion::SplitLeft)
            .into()
    } else {
        this.file_diff_side_document_text(DiffTextRegion::SplitRight)
            .into()
    };
    let line_starts = if is_left {
        Arc::clone(&this.file_diff_old_line_starts)
    } else {
        Arc::clone(&this.file_diff_new_line_starts)
    };
    let pinned_hunk_shell_width = if is_left {
        collapsed_hunk_shell_width(&this.diff_scroll, min_width)
    } else {
        collapsed_hunk_shell_width(&this.diff_split_right_scroll, min_width)
    };
    let pinned_hunk_shell_scroll = if is_left {
        this.diff_scroll.clone()
    } else {
        this.diff_split_right_scroll.clone()
    };

    let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
    range
        .map(|visible_ix| {
            let selected = this
                .diff_selection_range
                .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
            let show_line_numbers = this.diff_show_line_numbers;
            let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
            let Some(source_visible_ix) = this.diff_source_visible_ix_for_visible_ix(visible_ix)
            else {
                return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
            };
            let Some(visible_row) = this.collapsed_visible_row(source_visible_ix) else {
                return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
            };

            match visible_row {
                CollapsedDiffVisibleRow::HunkHeader {
                    src_ix,
                    expansion_kind,
                    hidden_rows,
                    ..
                } => {
                    let display_src_ix = visible_row.header_display_src_ix();
                    let display = display_src_ix
                        .and_then(|display_src_ix| {
                            this.collapsed_diff_hunk_header_display(display_src_ix)
                        })
                        .unwrap_or_default();
                    let context_menu_active = display_src_ix.is_some()
                        && this.active_repo_id().is_some_and(|repo_id| {
                            let invoker: SharedString =
                                format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                            this.active_context_menu_invoker.as_ref() == Some(&invoker)
                        });
                    let collapsed_hunk = this.collapsed_diff_hunk_for_src_ix(src_ix);

                    collapsed_split_header_row(
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        DiffClickKind::HunkHeader,
                        selected,
                        min_width,
                        pinned_hunk_shell_width,
                        pinned_hunk_shell_scroll.clone(),
                        collapsed_hunk,
                        None,
                        display,
                        None,
                        context_menu_active,
                        src_ix,
                        expansion_kind,
                        hidden_rows,
                        cx,
                    )
                }
                CollapsedDiffVisibleRow::FileRow { row_ix } => {
                    let Some(row) = this.file_diff_split_render_data(row_ix) else {
                        return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
                    };
                    let visual_kind = this.file_diff_split_visual_kind(row_ix);
                    let row_word_ranges = this.file_diff_split_word_ranges(row_ix, region);
                    let row_word_kind = file_diff_split_word_kind(column, visual_kind);
                    let streamed_spec =
                        file_diff_split_side_text_owned(&row, is_left).and_then(|raw_text| {
                            prepared_streamed_diff_text_spec(
                                raw_text,
                                &query,
                                query_options,
                                query_matcher.clone(),
                                row_word_ranges.clone(),
                                row_word_kind,
                                language,
                                syntax_mode,
                                Arc::clone(&document_text),
                                Arc::clone(&line_starts),
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    syntax_document,
                                    file_diff_split_side_line(&row, is_left),
                                ),
                            )
                        });
                    let key = this.file_diff_split_cache_key(row_ix, region);
                    if let Some(key) = key
                        && streamed_spec.is_none()
                        && this
                            .diff_text_segments_cache_get(key, cache_epoch)
                            .is_none()
                    {
                        let raw_text = file_diff_split_side_text(&row, is_left);
                        if let Some(raw_text) = raw_text {
                            let (styled, is_pending) =
                                build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                                    tab_width,
                                    theme,
                                    raw_text,
                                    &row_word_ranges,
                                    "",
                                    DiffSyntaxConfig {
                                        language,
                                        mode: syntax_mode,
                                    },
                                    row_word_kind,
                                    rows::prepared_diff_syntax_line_for_one_based_line(
                                        syntax_document,
                                        file_diff_split_side_line(&row, is_left),
                                    ),
                                );
                            if is_pending {
                                this.ensure_prepared_syntax_chunk_poll(cx);
                            }
                            this.diff_text_segments_cache_set(key, cache_epoch, styled);
                        }
                    }

                    let row_has_content = file_diff_split_side_text(&row, is_left).is_some();
                    let row_tint = this.hosted_row_tint(
                        visible_ix,
                        Some(if is_left {
                            gitcomet_extension_api::DiffLineSide::Old
                        } else {
                            gitcomet_extension_api::DiffLineSide::New
                        }),
                    );
                    let styled = if row_has_content && streamed_spec.is_none() {
                        if let Some(key) = key {
                            this.diff_text_segments_cache_get_for_query(
                                key,
                                query.as_ref(),
                                query_options,
                                cache_epoch,
                            )
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    patch_split_column_row(
                        tab_width,
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        selected,
                        min_width,
                        &row,
                        visual_kind,
                        mark_lines,
                        styled,
                        streamed_spec,
                        reveal_whitespace_chars,
                        show_line_numbers,
                        wrap,
                        annotation_width,
                        if is_left {
                            blame_ctx.as_ref().and_then(|ctx| {
                                build_row_blame_paint_tracked(
                                    ctx,
                                    matches!(visual_kind, FileDiffRowKind::Context),
                                    row.old_line,
                                    row.new_line,
                                    &blame_prev_nl,
                                    wrap,
                                    theme,
                                )
                            })
                        } else {
                            None
                        },
                        annot_hover,
                        stage_area,
                        stage_hover,
                        row_tint,
                        cx,
                    )
                }
            }
        })
        .collect()
}

pub(super) fn render_file_diff_split_rows(
    this: &mut DiffRenderer,
    range: Range<usize>,
    pass: SplitDiffRowsRenderPass,
    cx: &mut gpui::Context<DiffRenderer>,
) -> Vec<AnyElement> {
    let mark_lines = !this.structural_diff_active();
    let SplitDiffRowsRenderPass {
        common,
        column,
        is_left,
        region,
        id_missing,
        id_oob,
        ..
    } = pass;
    let DiffRowsRenderPass {
        tab_width,
        annot_hover,
        stage_area,
        stage_hover,
        min_width,
        query,
        query_options,
        query_matcher,
        reveal_whitespace_chars,
        ui_scale_percent,
        annotation_width,
        blame_ctx,
    } = common;

    let theme = this.theme;
    let language = this.file_diff_cache_language;
    let cache_epoch = this.file_diff_split_style_cache_epoch(region);
    let syntax_document = this.file_diff_split_prepared_syntax_document(region);
    let syntax_mode = DiffSyntaxMode::Auto;
    let document_text: Arc<str> = if is_left {
        this.file_diff_side_document_text(DiffTextRegion::SplitLeft)
            .into()
    } else {
        this.file_diff_side_document_text(DiffTextRegion::SplitRight)
            .into()
    };
    let line_starts = if is_left {
        Arc::clone(&this.file_diff_old_line_starts)
    } else {
        Arc::clone(&this.file_diff_new_line_starts)
    };

    let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
    range
        .map(|visible_ix| {
            let selected = this
                .diff_selection_range
                .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
            let show_line_numbers = this.diff_show_line_numbers;
            let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

            let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
            };
            let Some(row) = this.file_diff_split_render_data(row_ix) else {
                return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
            };
            let visual_kind = this.file_diff_split_visual_kind(row_ix);
            let row_word_ranges = this.file_diff_split_word_ranges(row_ix, region);
            let row_word_kind = file_diff_split_word_kind(column, visual_kind);
            let streamed_spec =
                file_diff_split_side_text_owned(&row, is_left).and_then(|raw_text| {
                    prepared_streamed_diff_text_spec(
                        raw_text,
                        &query,
                        query_options,
                        query_matcher.clone(),
                        row_word_ranges.clone(),
                        row_word_kind,
                        language,
                        syntax_mode,
                        Arc::clone(&document_text),
                        Arc::clone(&line_starts),
                        rows::prepared_diff_syntax_line_for_one_based_line(
                            syntax_document,
                            file_diff_split_side_line(&row, is_left),
                        ),
                    )
                });
            let key = this.file_diff_split_cache_key(row_ix, region);
            if let Some(key) = key
                && streamed_spec.is_none()
                && this
                    .diff_text_segments_cache_get(key, cache_epoch)
                    .is_none()
            {
                let raw_text = file_diff_split_side_text(&row, is_left);
                if let Some(raw_text) = raw_text {
                    let (styled, is_pending) =
                        build_file_diff_cached_styled_text_for_prepared_line_nonblocking(
                            tab_width,
                            theme,
                            raw_text,
                            &row_word_ranges,
                            "",
                            DiffSyntaxConfig {
                                language,
                                mode: syntax_mode,
                            },
                            row_word_kind,
                            rows::prepared_diff_syntax_line_for_one_based_line(
                                syntax_document,
                                file_diff_split_side_line(&row, is_left),
                            ),
                        );
                    if is_pending {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                    }
                    this.diff_text_segments_cache_set(key, cache_epoch, styled);
                }
            }

            let row_has_content = file_diff_split_side_text(&row, is_left).is_some();
            let row_tint = this.hosted_row_tint(
                visible_ix,
                Some(if is_left {
                    gitcomet_extension_api::DiffLineSide::Old
                } else {
                    gitcomet_extension_api::DiffLineSide::New
                }),
            );
            let styled = if row_has_content && streamed_spec.is_none() {
                if let Some(key) = key {
                    this.diff_text_segments_cache_get_for_query(
                        key,
                        query.as_ref(),
                        query_options,
                        cache_epoch,
                    )
                } else {
                    None
                }
            } else {
                None
            };
            debug_assert!(
                !row_has_content || key.is_none() || streamed_spec.is_some() || styled.is_some(),
                "diff text segment cache missing for split-{column:?} row {row_ix} after populate"
            );

            patch_split_column_row(
                tab_width,
                theme,
                ui_scale_percent,
                column,
                visible_ix,
                selected,
                min_width,
                &row,
                visual_kind,
                mark_lines,
                styled,
                streamed_spec,
                reveal_whitespace_chars,
                show_line_numbers,
                wrap,
                annotation_width,
                if is_left {
                    blame_ctx.as_ref().and_then(|ctx| {
                        build_row_blame_paint_tracked(
                            ctx,
                            matches!(visual_kind, FileDiffRowKind::Context),
                            row.old_line,
                            row.new_line,
                            &blame_prev_nl,
                            wrap,
                            theme,
                        )
                    })
                } else {
                    None
                },
                annot_hover,
                stage_area,
                stage_hover,
                row_tint,
                cx,
            )
        })
        .collect()
}
