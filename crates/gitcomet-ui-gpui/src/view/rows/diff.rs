use super::diff_canvas;
use super::diff_text::*;
use super::*;
use crate::view::panes::diff::diff_search::DiffSearchOptions;
use crate::view::panes::diff::helpers::state::CollapsedDiffVisibleRow;
use crate::view::panes::diff::helpers::state::DiffHorizontalScrollColumn;
use crate::view::panes::diff::helpers::state::VersionedCachedDiffStyledText;
use crate::view::panes::diff::helpers::state::versioned_query_cached_diff_styled_text_is_current;
use blame_presentation::BlamePrev;
use blame_presentation::build_row_blame_paint_tracked;
use file_rows::build_file_diff_cached_styled_text;
use file_rows::build_file_diff_cached_styled_text_for_prepared_line_nonblocking;
use file_rows::file_diff_split_side_line;
use file_rows::file_diff_split_side_text;
use file_rows::file_diff_split_side_text_owned;
use file_rows::file_diff_split_word_kind;
use file_rows::heuristic_streamed_diff_text_spec;
use file_rows::prepared_streamed_diff_text_spec;
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;
use patch_rows::DiffRowDecor;
use patch_rows::DiffRowStyle;
use patch_rows::collapsed_hunk_shell_width;
use patch_rows::collapsed_inline_header_row;
use patch_rows::collapsed_split_header_row;
use patch_rows::diff_line_word_kind;
use patch_rows::diff_placeholder_row;
use patch_rows::diff_row;
use patch_rows::patch_split_column_row;
use patch_rows::patch_split_header_row;

pub(super) mod blame_presentation;
mod file_rows;
mod patch_rows;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PatchSplitColumn {
    Left,
    Right,
}

impl DiffRenderer {
    fn diff_text_segments_cache_get_for_query(
        &mut self,
        key: usize,
        query: &str,
        options: DiffSearchOptions,
        syntax_epoch: u64,
    ) -> Option<&CachedDiffStyledText> {
        if query.is_empty() {
            return self.diff_text_segments_cache_get(key, syntax_epoch);
        }

        self.sync_diff_text_query_overlay_cache(query, options);
        let query_generation = self.diff_text_query_cache_generation;
        if self.diff_text_query_segments_cache.len() <= key {
            self.diff_text_query_segments_cache
                .resize_with(key + 1, || None);
        }

        if versioned_query_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
            query_generation,
        )
        .is_none()
        {
            let base = self
                .diff_text_segments_cache_get(key, syntax_epoch)?
                .clone();
            // The diff view marks its current match by selecting the row, so
            // every match here wears the same wash.
            let overlaid = build_cached_diff_query_overlay_styled_text(
                self.theme,
                &base,
                self.diff_text_query_cache_matcher_shared.as_ref()?,
                DiffSearchMatchEmphasis::Other,
            );
            self.diff_text_query_segments_cache[key] = Some(VersionedCachedDiffStyledText {
                syntax_epoch,
                query_generation,
                styled: overlaid,
            });
        }

        versioned_query_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
            query_generation,
        )
    }

    pub(in super::super) fn render_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let tab_width = this.display_tab_width;

        let annot_hover = this.blame_annot_hover;
        let stage_area = this.diff_stage_gutter_area();
        let stage_hover = this.diff_stage_gutter_hover;
        let min_width = this.diff_horizontal_layout_min_width(DiffHorizontalScrollColumn::Primary);
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = this.diff_search_query_matcher_shared();
        let reveal_whitespace_chars = this.reveal_whitespace_chars;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();
        let annotation_width = if this.annotation_active() {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        let blame_ctx = this.blame_render_ctx();

        if this.is_collapsed_diff_projection_active() {
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
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
                    let Some(source_visible_ix) =
                        this.diff_source_visible_ix_for_visible_ix(visible_ix)
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
                                        rows::prepared_diff_syntax_line_for_one_based_line(
                                            None, None,
                                        )
                                    }
                                };
                                let (document_text, line_starts) = match row.kind {
                                    DiffLineKind::Remove => (
                                        Arc::clone(&old_document_text),
                                        Arc::clone(&old_line_starts),
                                    ),
                                    DiffLineKind::Add | DiffLineKind::Context => (
                                        Arc::clone(&new_document_text),
                                        Arc::clone(&new_line_starts),
                                    ),
                                    DiffLineKind::Header | DiffLineKind::Hunk => (
                                        Arc::clone(&new_document_text),
                                        Arc::clone(&new_line_starts),
                                    ),
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
                                let cache_epoch =
                                    this.file_diff_style_cache_epochs.inline_epoch(row.kind);
                                if this
                                    .diff_text_segments_cache_get(row_ix, cache_epoch)
                                    .is_none()
                                {
                                    let word_kind = diff_line_word_kind(visual_kind);
                                    let is_content_line = matches!(
                                        line.kind,
                                        DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                                    );
                                    let line_language =
                                        is_content_line.then_some(language).flatten();
                                    let projected = this.file_diff_inline_projected_syntax(&line);
                                    let syntax_mode = DiffSyntaxMode::Auto;
                                    let (styled, is_pending) =
                                        build_file_diff_cached_styled_text_for_prepared_line_nonblocking(tab_width,
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

                            diff_row(DiffRowStyle {
tab_width,
theme,
ui_scale_percent,
mode: DiffViewMode::Inline,
min_width,
reveal_whitespace_chars,
show_line_numbers,
annotation_width
}, visible_ix, DiffClickKind::Line, &line, DiffRowDecor {
tint: row_tint,
selected,
visual_kind,
file_stat: None,
header_display: None,
styled,
streamed_spec,
raw_text: Some(row.text.as_ref()),
context_menu_active: false,
wrap,
row_blame: blame_ctx
                                    .as_ref()
                                    .and_then(|ctx| build_row_blame_paint_tracked(ctx, matches!(visual_kind, DiffLineKind::Context), line.old_line, line.new_line, &blame_prev_nl, wrap, theme)),
annot_hover,
stage_area,
stage_hover
}, cx)
                        }
                    }
                })
                .collect();
        }

        if this.is_file_diff_view_active() {
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
                    if this.file_diff_inline_modify_pair_texts(inline_ix).is_some() {
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
                                document: this.file_diff_split_prepared_syntax_document(
                                    DiffTextRegion::SplitLeft,
                                ),
                            },
                            PreparedDiffSyntaxTextSource {
                                document: this.file_diff_split_prepared_syntax_document(
                                    DiffTextRegion::SplitRight,
                                ),
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
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

                    let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row(
                            ("diff_missing", visible_ix),
                            theme,
                            ui_scale_percent,
                        );
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
                                this.file_diff_split_prepared_syntax_document(
                                    DiffTextRegion::SplitLeft,
                                ),
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
                            DiffLineKind::Remove => (
                                Arc::clone(&old_document_text),
                                Arc::clone(&old_line_starts),
                            ),
                            DiffLineKind::Add | DiffLineKind::Context => (
                                Arc::clone(&new_document_text),
                                Arc::clone(&new_line_starts),
                            ),
                            DiffLineKind::Header | DiffLineKind::Hunk => (
                                Arc::clone(&new_document_text),
                                Arc::clone(&new_line_starts),
                            ),
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
                                    build_file_diff_cached_styled_text_for_prepared_line_nonblocking(tab_width,
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
                            return diff_placeholder_row(
                                ("diff_oob", visible_ix),
                                theme,
                                ui_scale_percent,
                            );
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
                                build_cached_diff_styled_text_for_prepared_document_line_nonblocking(tab_width,
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

                    diff_row(DiffRowStyle {
tab_width,
theme,
ui_scale_percent,
mode: DiffViewMode::Inline,
min_width,
reveal_whitespace_chars,
show_line_numbers,
annotation_width
}, visible_ix, DiffClickKind::Line, &line, DiffRowDecor {
tint: row_tint,
selected,
visual_kind,
file_stat: None,
header_display: None,
styled,
streamed_spec,
raw_text: render_data
                            .as_ref()
                            .map(|row| row.text.as_ref())
                            .or_else(|| Some(diff_content_text(&line))),
context_menu_active: false,
wrap,
row_blame: blame_ctx
                            .as_ref()
                            .and_then(|ctx| build_row_blame_paint_tracked(ctx, matches!(visual_kind, DiffLineKind::Context), line.old_line, line.new_line, &blame_prev_nl, wrap, theme)),
annot_hover,
stage_area,
stage_hover
}, cx)
                })
                .collect();
        }

        let theme = this.theme;
        let cache_epoch = 0u64;
        let repo_id_for_context_menu = this.active_repo_id();
        let active_context_menu_invoker = this.active_context_menu_invoker.clone();
        let syntax_mode = this.patch_diff_syntax_mode();
        let blame_prev_nl = std::cell::Cell::new(BlamePrev::default());
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                let show_line_numbers = this.diff_show_line_numbers;
                let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);

                let Some(src_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return diff_placeholder_row(
                        ("diff_missing", visible_ix),
                        theme,
                        ui_scale_percent,
                    );
                };
                let click_kind = this
                    .diff_click_kinds
                    .get(src_ix)
                    .copied()
                    .unwrap_or(DiffClickKind::Line);

                this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                let word_ranges: &[Range<usize>] = this
                    .diff_word_highlights
                    .get(src_ix)
                    .and_then(|r| r.as_ref().map(Vec::as_slice))
                    .unwrap_or(&[]);

                let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);

                let language = this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                let Some(line) = this.patch_diff_row(src_ix) else {
                    return diff_placeholder_row(("diff_oob", visible_ix), theme, ui_scale_percent);
                };
                let visual_kind = this.patch_visual_line_kind(src_ix);
                let streamed_spec = matches!(click_kind, DiffClickKind::Line)
                    .then(|| {
                        heuristic_streamed_diff_text_spec(
                            crate::view::diff_utils::diff_content_line_text(&line),
                            &query,
                            query_options,
                            query_matcher.clone(),
                            Arc::from(word_ranges.to_vec()),
                            diff_line_word_kind(visual_kind),
                            language,
                            syntax_mode,
                        )
                    })
                    .flatten();

                let should_style = matches!(click_kind, DiffClickKind::Line) || !query.is_empty();
                if should_style
                    && streamed_spec.is_none()
                    && this
                        .diff_text_segments_cache_get(src_ix, cache_epoch)
                        .is_none()
                {
                    let computed = if matches!(click_kind, DiffClickKind::Line) {
                        let word_kind = diff_line_word_kind(visual_kind);
                        let content_text = diff_content_text(&line);

                        build_cached_diff_styled_text_with_source_identity(
                            tab_width,
                            theme,
                            content_text,
                            Some(DiffTextSourceIdentity::from_str(content_text)),
                            word_ranges,
                            "",
                            language,
                            syntax_mode,
                            word_kind,
                        )
                    } else {
                        let display =
                            this.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        build_cached_diff_styled_text(
                            tab_width,
                            theme,
                            display.as_ref(),
                            &[] as &[Range<usize>],
                            "",
                            None,
                            syntax_mode,
                            None,
                        )
                    };
                    this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                }

                let header_display = matches!(
                    click_kind,
                    DiffClickKind::FileHeader | DiffClickKind::HunkHeader
                )
                .then(|| this.diff_header_display_cache.get(&src_ix).cloned())
                .flatten();
                let context_menu_active = click_kind == DiffClickKind::HunkHeader
                    && repo_id_for_context_menu.is_some_and(|repo_id| {
                        let invoker: SharedString =
                            format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                        active_context_menu_invoker.as_ref() == Some(&invoker)
                    });
                let row_tint = this.hosted_row_tint(visible_ix, None);
                let styled = if should_style && streamed_spec.is_none() {
                    this.diff_text_segments_cache_get_for_query(
                        src_ix,
                        query.as_ref(),
                        query_options,
                        cache_epoch,
                    )
                } else {
                    None
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
                    click_kind,
                    &line,
                    DiffRowDecor {
                        tint: row_tint,
                        selected,
                        visual_kind,
                        file_stat,
                        header_display,
                        styled,
                        streamed_spec,
                        raw_text: Some(if matches!(click_kind, DiffClickKind::Line) {
                            diff_content_text(&line)
                        } else {
                            line.text.as_ref()
                        }),
                        context_menu_active,
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

    pub(in super::super) fn render_diff_split_left_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        Self::render_diff_split_rows(this, PatchSplitColumn::Left, range, annot_hover, cx)
    }

    pub(in super::super) fn render_diff_split_right_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let annot_hover = this.blame_annot_hover;
        Self::render_diff_split_rows(this, PatchSplitColumn::Right, range, annot_hover, cx)
    }

    fn render_diff_split_rows(
        this: &mut Self,
        column: PatchSplitColumn,
        range: Range<usize>,
        annot_hover: Option<(usize, AnnotArea)>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let tab_width = this.display_tab_width;

        let stage_area = this.diff_stage_gutter_area();
        let stage_hover = this.diff_stage_gutter_hover;
        let min_width =
            this.diff_horizontal_layout_min_width(if matches!(column, PatchSplitColumn::Right) {
                DiffHorizontalScrollColumn::SplitRight
            } else {
                DiffHorizontalScrollColumn::Primary
            });
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query_matcher = this.diff_search_query_matcher_shared();
        let reveal_whitespace_chars = this.reveal_whitespace_chars;
        let ui_scale_percent = crate::ui_scale::UiScale::current(cx).percent();

        let is_left = matches!(column, PatchSplitColumn::Left);
        // The annotation column is only drawn in the left split column. Reserve
        // its width whenever annotate mode is on — even before blame data is
        // ready — so the column space is stable and content does not shift when
        // blame finishes loading. Only the left column needs the blame context;
        // building it for the right column would clone the blame data and rescan
        // the time range for nothing.
        let blame_ctx = if is_left {
            this.blame_render_ctx()
        } else {
            None
        };
        let annotation_width = if this.annotation_active() && is_left {
            this.annotate_column_width_px(ui_scale_percent)
        } else {
            px(0.0)
        };
        let region = if is_left {
            DiffTextRegion::SplitLeft
        } else {
            DiffTextRegion::SplitRight
        };
        // Static ID tags to avoid format!/String allocation in element IDs.
        let (id_missing, id_oob, id_src_oob, id_hidden) = if is_left {
            (
                "diff_split_left_missing",
                "diff_split_left_oob",
                "diff_split_left_src_oob",
                "diff_split_left_hidden_header",
            )
        } else {
            (
                "diff_split_right_missing",
                "diff_split_right_oob",
                "diff_split_right_src_oob",
                "diff_split_right_hidden_header",
            )
        };

        if this.is_collapsed_diff_projection_active() {
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
            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));
                    let show_line_numbers = this.diff_show_line_numbers;
                    let wrap = this.diff_text_wrap_for_visible_ix(visible_ix);
                    let Some(source_visible_ix) =
                        this.diff_source_visible_ix_for_visible_ix(visible_ix)
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
                            let row_word_ranges =
                                this.file_diff_split_word_ranges(row_ix, region);
                            let row_word_kind =
                                file_diff_split_word_kind(column, visual_kind);
                            let streamed_spec =
                                file_diff_split_side_text_owned(&row, is_left).and_then(
                                    |raw_text| {
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
                                    },
                                );
                            let key = this.file_diff_split_cache_key(row_ix, region);
                            if let Some(key) = key
                                && streamed_spec.is_none()
                                && this.diff_text_segments_cache_get(key, cache_epoch).is_none()
                            {
                                let raw_text = file_diff_split_side_text(&row, is_left);
                                if let Some(raw_text) = raw_text {
                                    let (styled, is_pending) =
                                        build_file_diff_cached_styled_text_for_prepared_line_nonblocking(tab_width,
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
                            let row_tint = this.hosted_row_tint(visible_ix, Some(if is_left { gitcomet_extension_api::DiffLineSide::Old } else { gitcomet_extension_api::DiffLineSide::New }));
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

                            patch_split_column_row(tab_width,
                                theme,
                                ui_scale_percent,
                                column,
                                visible_ix,
                                selected,
                                min_width,
                                &row,
                                visual_kind,
                                styled,
                                streamed_spec,
                                reveal_whitespace_chars,
                                show_line_numbers,
                                wrap,
                                annotation_width,
                                if is_left {
                                    blame_ctx.as_ref().and_then(|ctx| {
                                        build_row_blame_paint_tracked(ctx, matches!(visual_kind, FileDiffRowKind::Context), row.old_line, row.new_line, &blame_prev_nl, wrap, theme)
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
                .collect();
        }

        if this.is_file_diff_view_active() {
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
            return range
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
                    let streamed_spec = file_diff_split_side_text_owned(&row, is_left).and_then(
                        |raw_text| {
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
                        },
                    );
                    let key = this.file_diff_split_cache_key(row_ix, region);
                    if let Some(key) = key
                        && streamed_spec.is_none()
                        && this.diff_text_segments_cache_get(key, cache_epoch).is_none()
                    {
                        let raw_text = file_diff_split_side_text(&row, is_left);
                        if let Some(raw_text) = raw_text {
                            let (styled, is_pending) = build_file_diff_cached_styled_text_for_prepared_line_nonblocking(tab_width,
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
                    let row_tint = this.hosted_row_tint(visible_ix, Some(if is_left { gitcomet_extension_api::DiffLineSide::Old } else { gitcomet_extension_api::DiffLineSide::New }));
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
                        !row_has_content
                            || key.is_none()
                            || streamed_spec.is_some()
                            || styled.is_some(),
                        "diff text segment cache missing for split-{column:?} row {row_ix} after populate"
                    );

                    patch_split_column_row(tab_width,
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        selected,
                        min_width,
                        &row,
                        visual_kind,
                        styled,
                        streamed_spec,
                        reveal_whitespace_chars,
                        show_line_numbers,
                        wrap,
                        annotation_width,
                        if is_left {
                            blame_ctx
                                .as_ref()
                                .and_then(|ctx| build_row_blame_paint_tracked(ctx, matches!(visual_kind, FileDiffRowKind::Context), row.old_line, row.new_line, &blame_prev_nl, wrap, theme))
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
                .collect();
        }

        let theme = this.theme;
        let cache_epoch = 0u64;
        let syntax_mode = this.patch_diff_syntax_mode();
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
                let Some(row) = this.patch_diff_split_row(row_ix) else {
                    return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
                };

                match row {
                    PatchSplitRow::Aligned {
                        row,
                        old_src_ix,
                        new_src_ix,
                    } => {
                        let src_ix = if is_left { old_src_ix } else { new_src_ix };
                        let old_changed = old_src_ix.is_some_and(|src_ix| {
                            matches!(this.patch_visual_line_kind(src_ix), DiffLineKind::Remove)
                        });
                        let new_changed = new_src_ix.is_some_and(|src_ix| {
                            matches!(this.patch_visual_line_kind(src_ix), DiffLineKind::Add)
                        });
                        let visual_kind = match (old_changed, new_changed) {
                            (true, true) => FileDiffRowKind::Modify,
                            (true, false) => FileDiffRowKind::Remove,
                            (false, true) => FileDiffRowKind::Add,
                            (false, false) => FileDiffRowKind::Context,
                        };
                        let row_tint = this.hosted_row_tint(
                            visible_ix,
                            Some(if is_left {
                                gitcomet_extension_api::DiffLineSide::Old
                            } else {
                                gitcomet_extension_api::DiffLineSide::New
                            }),
                        );
                        let (streamed_spec, styled) = if let Some(src_ix) = src_ix {
                            let language =
                                this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                            this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                            let word_ranges = this
                                .diff_word_highlights
                                .get(src_ix)
                                .and_then(|r| r.as_ref().cloned())
                                .unwrap_or_default();
                            let word_kind =
                                diff_line_word_kind(this.patch_visual_line_kind(src_ix));
                            let streamed_spec = file_diff_split_side_text_owned(&row, is_left)
                                .and_then(|raw_text| {
                                    heuristic_streamed_diff_text_spec(
                                        raw_text,
                                        &query,
                                        query_options,
                                        query_matcher.clone(),
                                        Arc::from(word_ranges.clone()),
                                        word_kind,
                                        language,
                                        syntax_mode,
                                    )
                                });
                            if streamed_spec.is_none()
                                && this
                                    .diff_text_segments_cache_get(src_ix, cache_epoch)
                                    .is_none()
                            {
                                let computed = if let Some(raw_text) =
                                    file_diff_split_side_text(&row, is_left)
                                {
                                    build_file_diff_cached_styled_text(
                                        tab_width,
                                        theme,
                                        raw_text,
                                        word_ranges.as_slice(),
                                        "",
                                        language,
                                        syntax_mode,
                                        word_kind,
                                    )
                                } else {
                                    build_cached_diff_styled_text(
                                        tab_width,
                                        theme,
                                        "",
                                        word_ranges.as_slice(),
                                        "",
                                        language,
                                        syntax_mode,
                                        word_kind,
                                    )
                                };
                                this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                            }

                            let styled = if streamed_spec.is_none() {
                                this.diff_text_segments_cache_get_for_query(
                                    src_ix,
                                    query.as_ref(),
                                    query_options,
                                    cache_epoch,
                                )
                            } else {
                                None
                            };
                            (streamed_spec, styled)
                        } else {
                            (None, None)
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
                    PatchSplitRow::Raw { src_ix, click_kind } => {
                        if this.patch_diff_row(src_ix).is_none() {
                            return diff_placeholder_row(
                                (id_src_oob, visible_ix),
                                theme,
                                ui_scale_percent,
                            );
                        };
                        let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);
                        let should_style = !query.is_empty();
                        if should_style
                            && this
                                .diff_text_segments_cache_get(src_ix, cache_epoch)
                                .is_none()
                        {
                            let display = this.diff_text_line_for_region(visible_ix, region);
                            let computed = build_cached_diff_styled_text(
                                tab_width,
                                theme,
                                display.as_ref(),
                                &[],
                                "",
                                None,
                                syntax_mode,
                                None,
                            );
                            this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                        }
                        let Some(line) = this.patch_diff_row(src_ix) else {
                            return diff_placeholder_row(
                                (id_src_oob, visible_ix),
                                theme,
                                ui_scale_percent,
                            );
                        };
                        if should_hide_unified_diff_header_line(&line) {
                            return div()
                                .id((id_hidden, visible_ix))
                                .h(px(0.0))
                                .into_any_element();
                        }
                        let context_menu_active = click_kind == DiffClickKind::HunkHeader
                            && this.active_repo_id().is_some_and(|repo_id| {
                                let invoker: SharedString =
                                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                                this.active_context_menu_invoker.as_ref() == Some(&invoker)
                            });
                        let header_display = this.diff_header_display_cache.get(&src_ix).cloned();
                        let styled = if should_style {
                            this.diff_text_segments_cache_get_for_query(
                                src_ix,
                                query.as_ref(),
                                query_options,
                                cache_epoch,
                            )
                        } else {
                            None
                        };
                        patch_split_header_row(
                            theme,
                            ui_scale_percent,
                            column,
                            visible_ix,
                            click_kind,
                            selected,
                            min_width,
                            &line,
                            file_stat,
                            header_display,
                            styled,
                            context_menu_active,
                            cx,
                        )
                    }
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests;
