use super::diff_canvas;
use super::diff_text::*;
use super::*;
use crate::view::panes::main::{
    VersionedCachedDiffStyledText, versioned_query_cached_diff_styled_text_is_current,
};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;

const DIFF_ROW_HEIGHT_PX: f32 = 20.0;
const DIFF_FILE_HEADER_HEIGHT_PX: f32 = 28.0;
const DIFF_HUNK_HEADER_HEIGHT_PX: f32 = 24.0;

fn diff_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    crate::ui_scale::design_px_from_percent(value, ui_scale_percent)
}

fn diff_row_height(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_ROW_HEIGHT_PX, ui_scale_percent)
}

fn diff_file_header_height(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_FILE_HEADER_HEIGHT_PX, ui_scale_percent)
}

fn diff_hunk_header_height(ui_scale_percent: u32) -> Pixels {
    diff_scaled_px(DIFF_HUNK_HEADER_HEIGHT_PX, ui_scale_percent)
}

/// Returns the word-highlight color for a diff line kind.
fn diff_line_word_color(kind: DiffLineKind, theme: AppTheme) -> Option<gpui::Rgba> {
    match kind {
        DiffLineKind::Add => Some(theme.colors.diff_add_text),
        DiffLineKind::Remove => Some(theme.colors.diff_remove_text),
        _ => None,
    }
}

/// Returns the word-highlight color for a file diff split column.
/// Left highlights Remove/Modify; Right highlights Add/Modify.
fn file_diff_split_word_color(
    column: PatchSplitColumn,
    kind: FileDiffRowKind,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    match column {
        PatchSplitColumn::Left => matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify)
            .then_some(theme.colors.diff_remove_text),
        PatchSplitColumn::Right => matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify)
            .then_some(theme.colors.diff_add_text),
    }
}

fn diff_placeholder_row(
    id: impl Into<gpui::ElementId>,
    theme: AppTheme,
    ui_scale_percent: u32,
) -> AnyElement {
    div()
        .id(id)
        .h(diff_row_height(ui_scale_percent))
        .px_2()
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child("")
        .into_any_element()
}

fn streamed_diff_text_spec_with_syntax(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    word_ranges: Vec<Range<usize>>,
    word_color: Option<gpui::Rgba>,
    syntax: diff_canvas::StreamedDiffTextSyntaxSource,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    diff_canvas::is_streamable_diff_text(&raw_text).then(|| {
        diff_canvas::StreamedDiffTextPaintSpec {
            raw_text,
            query: query.clone(),
            word_ranges: Arc::from(word_ranges),
            word_color,
            syntax,
        }
    })
}

fn heuristic_streamed_diff_text_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    word_ranges: Vec<Range<usize>>,
    word_color: Option<gpui::Rgba>,
    language: Option<rows::DiffSyntaxLanguage>,
    mode: rows::DiffSyntaxMode,
) -> Option<diff_canvas::StreamedDiffTextPaintSpec> {
    let syntax = match language {
        Some(language) => diff_canvas::StreamedDiffTextSyntaxSource::Heuristic { language, mode },
        None => diff_canvas::StreamedDiffTextSyntaxSource::None,
    };
    streamed_diff_text_spec_with_syntax(raw_text, query, word_ranges, word_color, syntax)
}

#[allow(clippy::too_many_arguments)]
fn prepared_streamed_diff_text_spec(
    raw_text: gitcomet_core::file_diff::FileDiffLineText,
    query: &SharedString,
    word_ranges: Vec<Range<usize>>,
    word_color: Option<gpui::Rgba>,
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
    streamed_diff_text_spec_with_syntax(raw_text, query, word_ranges, word_color, syntax)
}

impl MainPaneView {
    fn diff_text_segments_cache_get_for_query(
        &mut self,
        key: usize,
        query: &str,
        syntax_epoch: u64,
    ) -> Option<&CachedDiffStyledText> {
        let query = query.trim();
        if query.is_empty() {
            return self.diff_text_segments_cache_get(key, syntax_epoch);
        }

        self.sync_diff_text_query_overlay_cache(query);
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
            let overlaid = build_cached_diff_query_overlay_styled_text(self.theme, &base, query);
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
        let min_width = this.diff_horizontal_min_width;
        let query = this.diff_search_query_or_empty();
        let ui_scale_percent = crate::ui_scale::current(cx).percent;

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            let old_document_text: Arc<str> = this.file_diff_old_text.clone().into();
            let old_line_starts = Arc::clone(&this.file_diff_old_line_starts);
            let new_document_text: Arc<str> = this.file_diff_new_text.clone().into();
            let new_line_starts = Arc::clone(&this.file_diff_new_line_starts);
            // Inline syntax is now projected from the real old/new (split)
            // documents instead of parsing a synthetic mixed inline stream.
            // syntax_mode is determined per-row based on projection availability.
            if let Some(language) = language {
                let mut syntax_only_rows = Vec::new();
                for visible_ix in range.clone() {
                    let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        continue;
                    };
                    if this
                        .file_diff_inline_render_data(inline_ix)
                        .is_some_and(|row| diff_canvas::is_streamable_diff_text(&row.text))
                    {
                        continue;
                    }
                    let Some(line) = this.file_diff_inline_row(inline_ix) else {
                        continue;
                    };
                    let cache_epoch = this.file_diff_inline_style_cache_epoch(&line);
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
                    syntax_only_rows.push((inline_ix, cache_epoch, line));
                }

                if !syntax_only_rows.is_empty() {
                    let batch_rows = syntax_only_rows
                        .iter()
                        .map(|(_, _, line)| InlineDiffSyntaxOnlyRow {
                            text: diff_content_text(line),
                            line,
                        })
                        .collect::<Vec<_>>();
                    let batched_styles =
                        build_cached_diff_styled_text_for_inline_syntax_only_rows_nonblocking(
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
                        );
                    let mut pending_batch = false;
                    for ((inline_ix, cache_epoch, _), prepared) in
                        syntax_only_rows.iter().zip(batched_styles.into_iter())
                    {
                        let (styled, is_pending) = prepared.into_parts();
                        pending_batch |= is_pending;
                        this.diff_text_segments_cache_set(*inline_ix, *cache_epoch, styled);
                    }
                    if pending_batch {
                        this.ensure_prepared_syntax_chunk_poll(cx);
                    }
                }
            }

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row(
                            ("diff_missing", visible_ix),
                            theme,
                            ui_scale_percent,
                        );
                    };
                    let row_word_ranges = this
                        .file_diff_inline_modify_pair_texts(inline_ix)
                        .map(|(old, new, kind)| {
                            let (old_ranges, new_ranges) = capped_word_diff_ranges(old, new);
                            match kind {
                                DiffLineKind::Remove => old_ranges,
                                DiffLineKind::Add => new_ranges,
                                DiffLineKind::Context
                                | DiffLineKind::Header
                                | DiffLineKind::Hunk => Vec::new(),
                            }
                        })
                        .unwrap_or_default();
                    let render_data = this.file_diff_inline_render_data(inline_ix);
                    let streamed_spec = render_data.as_ref().and_then(|row| {
                        let line_language = matches!(
                            row.kind,
                            DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                        )
                        .then_some(language)
                        .flatten();
                        let word_color = diff_line_word_color(row.kind, theme);
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
                        let syntax_mode = syntax_mode_for_prepared_document(prepared_line.document);
                        prepared_streamed_diff_text_spec(
                            row.text.clone(),
                            &query,
                            row_word_ranges.clone(),
                            word_color,
                            line_language,
                            syntax_mode,
                            document_text,
                            line_starts,
                            prepared_line,
                        )
                    });

                    let (line, cache_epoch, styled) = if let Some(row) = render_data.as_ref() {
                        if streamed_spec.is_some() {
                            (
                                AnnotatedDiffLine {
                                    kind: row.kind,
                                    text: "".into(),
                                    old_line: row.old_line,
                                    new_line: row.new_line,
                                },
                                this.file_diff_style_cache_epochs.inline_epoch(row.kind),
                                None,
                            )
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
                                let word_color = diff_line_word_color(line.kind, theme);
                                let is_content_line = matches!(
                                    line.kind,
                                    DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                                );
                                let line_language = is_content_line.then_some(language).flatten();
                                let projected = this.file_diff_inline_projected_syntax(&line);
                                let syntax_mode =
                                    syntax_mode_for_prepared_document(projected.document);
                                let (styled, is_pending) =
                                    build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                                        theme,
                                        diff_content_text(&line),
                                        row_word_ranges.as_slice(),
                                        "",
                                        DiffSyntaxConfig {
                                            language: line_language,
                                            mode: syntax_mode,
                                        },
                                        word_color,
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
                                cache_epoch,
                            );
                            debug_assert!(
                                styled.is_some(),
                                "diff text segment cache missing for inline row {inline_ix} after populate"
                            );
                            (line, cache_epoch, styled)
                        }
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
                            let word_color = diff_line_word_color(line.kind, theme);
                            let is_content_line = matches!(
                                line.kind,
                                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                            );
                            let line_language = is_content_line.then_some(language).flatten();
                            let projected = this.file_diff_inline_projected_syntax(&line);
                            let syntax_mode =
                                syntax_mode_for_prepared_document(projected.document);
                            let (styled, is_pending) =
                                build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                                    theme,
                                    diff_content_text(&line),
                                    row_word_ranges.as_slice(),
                                    "",
                                    DiffSyntaxConfig {
                                        language: line_language,
                                        mode: syntax_mode,
                                    },
                                    word_color,
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
                        theme,
                        ui_scale_percent,
                        visible_ix,
                        DiffClickKind::Line,
                        selected,
                        DiffViewMode::Inline,
                        min_width,
                        &line,
                        None,
                        None,
                        styled,
                        streamed_spec,
                        false,
                        cx,
                    )
                })
                .collect();
        }

        let theme = this.theme;
        let cache_epoch = 0u64;
        let repo_id_for_context_menu = this.active_repo_id();
        let active_context_menu_invoker = this.active_context_menu_invoker.clone();
        let syntax_mode = this.patch_diff_syntax_mode();
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

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
                let streamed_spec = matches!(click_kind, DiffClickKind::Line)
                    .then(|| {
                        heuristic_streamed_diff_text_spec(
                            crate::view::diff_utils::diff_content_line_text(&line),
                            &query,
                            word_ranges.to_vec(),
                            diff_line_word_color(line.kind, theme),
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
                        let word_color = diff_line_word_color(line.kind, theme);
                        let content_text = diff_content_text(&line);

                        build_cached_diff_styled_text_with_source_identity(
                            theme,
                            content_text,
                            Some(DiffTextSourceIdentity::from_str(content_text)),
                            word_ranges,
                            "",
                            language,
                            syntax_mode,
                            word_color,
                        )
                    } else {
                        let display =
                            this.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                        build_cached_diff_styled_text(
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
                let styled = if should_style && streamed_spec.is_none() {
                    this.diff_text_segments_cache_get_for_query(src_ix, query.as_ref(), cache_epoch)
                } else {
                    None
                };
                diff_row(
                    theme,
                    ui_scale_percent,
                    visible_ix,
                    click_kind,
                    selected,
                    DiffViewMode::Inline,
                    min_width,
                    &line,
                    file_stat,
                    header_display,
                    styled,
                    streamed_spec,
                    context_menu_active,
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
        Self::render_diff_split_rows(this, PatchSplitColumn::Left, range, cx)
    }

    pub(in super::super) fn render_diff_split_right_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_diff_split_rows(this, PatchSplitColumn::Right, range, cx)
    }

    fn render_diff_split_rows(
        this: &mut Self,
        column: PatchSplitColumn,
        range: Range<usize>,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width = this.diff_horizontal_min_width;
        let query = this.diff_search_query_or_empty();
        let ui_scale_percent = crate::ui_scale::current(cx).percent;

        let is_left = matches!(column, PatchSplitColumn::Left);
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

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            let cache_epoch = this.file_diff_split_style_cache_epoch(region);
            let syntax_document = this.file_diff_split_prepared_syntax_document(region);
            let syntax_mode = syntax_mode_for_prepared_document(syntax_document);
            let document_text: Arc<str> = if is_left {
                this.file_diff_old_text.clone().into()
            } else {
                this.file_diff_new_text.clone().into()
            };
            let line_starts = if is_left {
                Arc::clone(&this.file_diff_old_line_starts)
            } else {
                Arc::clone(&this.file_diff_new_line_starts)
            };

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row((id_missing, visible_ix), theme, ui_scale_percent);
                    };
                    let Some(row) = this.file_diff_split_row(row_ix) else {
                        return diff_placeholder_row((id_oob, visible_ix), theme, ui_scale_percent);
                    };
                    let row_word_ranges = this
                        .file_diff_split_modify_pair_texts(row_ix)
                        .map(|(old, new)| {
                            let (old_ranges, new_ranges) = capped_word_diff_ranges(old, new);
                            if is_left {
                                old_ranges
                            } else {
                                new_ranges
                            }
                        })
                        .unwrap_or_default();
                    let row_word_color = file_diff_split_word_color(column, row.kind, theme);
                    let streamed_spec = if is_left {
                        row.old.clone()
                    } else {
                        row.new.clone()
                    }
                    .and_then(|raw_text| {
                        prepared_streamed_diff_text_spec(
                            raw_text,
                            &query,
                            row_word_ranges.clone(),
                            row_word_color,
                            language,
                            syntax_mode,
                            Arc::clone(&document_text),
                            Arc::clone(&line_starts),
                            rows::prepared_diff_syntax_line_for_one_based_line(
                                syntax_document,
                                if is_left { row.old_line } else { row.new_line },
                            ),
                        )
                    });
                    let key = this.file_diff_split_cache_key(row_ix, region);
                    if let Some(key) = key
                        && streamed_spec.is_none()
                        && this.diff_text_segments_cache_get(key, cache_epoch).is_none()
                    {
                        let text = if is_left {
                            row.old.as_deref()
                        } else {
                            row.new.as_deref()
                        };
                        if let Some(text) = text {
                            let (styled, is_pending) = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                                theme,
                                text,
                                row_word_ranges.as_slice(),
                                "",
                                DiffSyntaxConfig {
                                    language,
                                    mode: syntax_mode,
                                },
                                row_word_color,
                                rows::prepared_diff_syntax_line_for_one_based_line(
                                    syntax_document,
                                    if is_left { row.old_line } else { row.new_line },
                                ),
                            )
                            .into_parts();
                            if is_pending {
                                this.ensure_prepared_syntax_chunk_poll(cx);
                            }
                            this.diff_text_segments_cache_set(key, cache_epoch, styled);
                        }
                    }

                    let row_has_content = if is_left {
                        row.old.is_some()
                    } else {
                        row.new.is_some()
                    };
                    let styled = if row_has_content && streamed_spec.is_none() {
                        if let Some(key) = key {
                            this.diff_text_segments_cache_get_for_query(
                                key,
                                query.as_ref(),
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

                    patch_split_column_row(
                        theme,
                        ui_scale_percent,
                        column,
                        visible_ix,
                        selected,
                        min_width,
                        &row,
                        styled,
                        streamed_spec,
                        cx,
                    )
                })
                .collect();
        }

        let theme = this.theme;
        let cache_epoch = 0u64;
        let syntax_mode = this.patch_diff_syntax_mode();
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

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
                        let (streamed_spec, styled) = if let Some(src_ix) = src_ix {
                            let language =
                                this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                            this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                            let word_ranges = this
                                .diff_word_highlights
                                .get(src_ix)
                                .and_then(|r| r.as_ref().cloned())
                                .unwrap_or_default();
                            let word_color = this
                                .patch_diff_row(src_ix)
                                .and_then(|line| diff_line_word_color(line.kind, theme));
                            let streamed_spec = if is_left {
                                row.old.clone()
                            } else {
                                row.new.clone()
                            }
                            .and_then(|raw_text| {
                                heuristic_streamed_diff_text_spec(
                                    raw_text,
                                    &query,
                                    word_ranges.clone(),
                                    word_color,
                                    language,
                                    syntax_mode,
                                )
                            });
                            if streamed_spec.is_none()
                                && this
                                    .diff_text_segments_cache_get(src_ix, cache_epoch)
                                    .is_none()
                            {
                                let text = if is_left {
                                    row.old.as_deref()
                                } else {
                                    row.new.as_deref()
                                }
                                .unwrap_or("");
                                let computed = build_cached_diff_styled_text(
                                    theme,
                                    text,
                                    word_ranges.as_slice(),
                                    "",
                                    language,
                                    syntax_mode,
                                    word_color,
                                );
                                this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                            }

                            let styled = if streamed_spec.is_none() {
                                this.diff_text_segments_cache_get_for_query(
                                    src_ix,
                                    query.as_ref(),
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
                            theme,
                            ui_scale_percent,
                            column,
                            visible_ix,
                            selected,
                            min_width,
                            &row,
                            styled,
                            streamed_spec,
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

#[allow(clippy::too_many_arguments)]
fn diff_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    mode: DiffViewMode,
    min_width: Pixels,
    line: &AnnotatedDiffLine,
    file_stat: Option<(usize, usize)>,
    header_display: Option<SharedString>,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<diff_canvas::StreamedDiffTextPaintSpec>,
    context_menu_active: bool,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, click_kind, e.modifiers().shift);
        cx.notify();
    });

    if matches!(click_kind, DiffClickKind::FileHeader) {
        let file =
            header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));
        let mut row = div()
            .id(("diff_file_hdr", visible_ix))
            .h(diff_file_header_height(ui_scale_percent))
            .w_full()
            .min_w(min_width)
            .flex()
            .items_center()
            .justify_between()
            .px_2()
            .bg(theme.colors.surface_bg_elevated)
            .border_b_1()
            .border_color(theme.colors.border)
            .text_sm()
            .font_weight(FontWeight::BOLD)
            .child(selectable_cached_diff_text(
                visible_ix,
                DiffTextRegion::Inline,
                DiffClickKind::FileHeader,
                theme.colors.text,
                None,
                file,
                cx,
            ))
            .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                let (a, r) = file_stat.unwrap_or_default();
                this.child(components::diff_stat(theme, a, r))
            })
            .on_click(on_click);

        if selected {
            row = row.bg(with_alpha(
                theme.colors.accent,
                if theme.is_dark { 0.10 } else { 0.07 },
            ));
        }

        return row.into_any_element();
    }

    if matches!(click_kind, DiffClickKind::HunkHeader) {
        let display =
            header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));

        let mut row = div()
            .id(("diff_hunk_hdr", visible_ix))
            .h(diff_hunk_header_height(ui_scale_percent))
            .w_full()
            .min_w(min_width)
            .flex()
            .items_center()
            .px_2()
            .bg(with_alpha(
                theme.colors.accent,
                if theme.is_dark { 0.10 } else { 0.07 },
            ))
            .border_b_1()
            .border_color(with_alpha(
                theme.colors.accent,
                if theme.is_dark { 0.28 } else { 0.22 },
            ))
            .text_xs()
            .text_color(theme.colors.text_muted)
            .child(selectable_cached_diff_text(
                visible_ix,
                DiffTextRegion::Inline,
                DiffClickKind::HunkHeader,
                theme.colors.text_muted,
                None,
                display,
                cx,
            ))
            .on_click(on_click);
        let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
            cx.stop_propagation();
            let Some(repo_id) = this.active_repo_id() else {
                return;
            };
            let Some(src_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return;
            };
            let context_menu_invoker: SharedString =
                format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
            this.activate_context_menu_invoker(context_menu_invoker, cx);
            this.open_popover_at(
                PopoverKind::DiffHunkMenu { repo_id, src_ix },
                e.position,
                window,
                cx,
            );
        });
        row = row.on_mouse_down(MouseButton::Right, on_right_click);

        if selected {
            row = row.bg(with_alpha(
                theme.colors.accent,
                if theme.is_dark { 0.14 } else { 0.10 },
            ));
        }
        if context_menu_active {
            row = row.bg(theme.colors.active);
        }

        return row.into_any_element();
    }

    let (bg, fg, gutter_fg) = diff_line_colors(theme, line.kind);

    let old = line_number_string(line.old_line);
    let new = line_number_string(line.new_line);

    match mode {
        DiffViewMode::Inline => diff_canvas::inline_diff_line_row_canvas(
            theme,
            cx.entity(),
            ui_scale_percent,
            visible_ix,
            min_width,
            selected,
            old,
            new,
            bg,
            fg,
            gutter_fg,
            styled,
            streamed_spec,
        ),
        DiffViewMode::Split => {
            let left_kind = if line.kind == DiffLineKind::Remove {
                DiffLineKind::Remove
            } else {
                DiffLineKind::Context
            };
            let right_kind = if line.kind == DiffLineKind::Add {
                DiffLineKind::Add
            } else {
                DiffLineKind::Context
            };

            let (left_bg, left_fg, left_gutter) = diff_line_colors(theme, left_kind);
            let (right_bg, right_fg, right_gutter) = diff_line_colors(theme, right_kind);

            let (left_text, right_text) = match line.kind {
                DiffLineKind::Remove => (styled, None),
                DiffLineKind::Add => (None, styled),
                DiffLineKind::Context => (styled, styled),
                _ => (styled, None),
            };
            let left_streamed_spec = match line.kind {
                DiffLineKind::Remove | DiffLineKind::Context => streamed_spec.clone(),
                _ => None,
            };
            let right_streamed_spec = match line.kind {
                DiffLineKind::Add | DiffLineKind::Context => streamed_spec,
                _ => None,
            };

            diff_canvas::split_diff_line_row_canvas(
                theme,
                cx.entity(),
                ui_scale_percent,
                visible_ix,
                min_width,
                selected,
                old,
                new,
                left_bg,
                left_fg,
                left_gutter,
                right_bg,
                right_fg,
                right_gutter,
                left_text,
                right_text,
                left_streamed_spec,
                right_streamed_spec,
            )
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum PatchSplitColumn {
    Left,
    Right,
}

#[allow(clippy::too_many_arguments)]
fn patch_split_column_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    min_width: Pixels,
    row: &gitcomet_core::file_diff::FileDiffRow,
    styled: Option<&CachedDiffStyledText>,
    streamed_spec: Option<diff_canvas::StreamedDiffTextPaintSpec>,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let line_kind = match (column, row.kind) {
        (PatchSplitColumn::Left, FileDiffRowKind::Remove | FileDiffRowKind::Modify) => {
            DiffLineKind::Remove
        }
        (PatchSplitColumn::Right, FileDiffRowKind::Add | FileDiffRowKind::Modify) => {
            DiffLineKind::Add
        }
        _ => DiffLineKind::Context,
    };
    let (bg, fg, gutter_fg) = diff_line_colors(theme, line_kind);

    let line_no = match column {
        PatchSplitColumn::Left => line_number_string(row.old_line),
        PatchSplitColumn::Right => line_number_string(row.new_line),
    };

    diff_canvas::patch_split_column_row_canvas(
        theme,
        cx.entity(),
        ui_scale_percent,
        column,
        visible_ix,
        min_width,
        selected,
        bg,
        fg,
        gutter_fg,
        line_no,
        styled,
        streamed_spec,
    )
}

#[allow(clippy::too_many_arguments)]
fn patch_split_header_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    min_width: Pixels,
    line: &AnnotatedDiffLine,
    file_stat: Option<(usize, usize)>,
    header_display: Option<SharedString>,
    styled: Option<&CachedDiffStyledText>,
    context_menu_active: bool,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, click_kind, e.modifiers().shift);
        cx.notify();
    });
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    match click_kind {
        DiffClickKind::FileHeader => {
            let display =
                header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));
            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_file_hdr",
                        PatchSplitColumn::Right => "diff_split_right_file_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_file_header_height(ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .justify_between()
                .px_2()
                .bg(theme.colors.surface_bg_elevated)
                .border_b_1()
                .border_color(theme.colors.border)
                .text_sm()
                .font_weight(FontWeight::BOLD)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::FileHeader,
                    theme.colors.text,
                    styled,
                    display,
                    cx,
                ))
                .when(file_stat.is_some_and(|(a, r)| a > 0 || r > 0), |this| {
                    let (a, r) = file_stat.unwrap_or_default();
                    this.child(components::diff_stat(theme, a, r))
                })
                .on_click(on_click);

            if selected {
                row = row.bg(with_alpha(
                    theme.colors.accent,
                    if theme.is_dark { 0.10 } else { 0.07 },
                ));
            }

            row.into_any_element()
        }
        DiffClickKind::HunkHeader => {
            let display =
                header_display.unwrap_or_else(|| SharedString::from(line.text.as_ref().to_owned()));

            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_hunk_hdr",
                        PatchSplitColumn::Right => "diff_split_right_hunk_hdr",
                    },
                    visible_ix,
                ))
                .h(diff_hunk_header_height(ui_scale_percent))
                .w_full()
                .min_w(min_width)
                .flex()
                .items_center()
                .px_2()
                .bg(with_alpha(
                    theme.colors.accent,
                    if theme.is_dark { 0.10 } else { 0.07 },
                ))
                .border_b_1()
                .border_color(with_alpha(
                    theme.colors.accent,
                    if theme.is_dark { 0.28 } else { 0.22 },
                ))
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(selectable_cached_diff_text(
                    visible_ix,
                    region,
                    DiffClickKind::HunkHeader,
                    theme.colors.text_muted,
                    styled,
                    display,
                    cx,
                ))
                .on_click(on_click);
            let on_right_click = cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                let Some(repo_id) = this.active_repo_id() else {
                    return;
                };
                let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return;
                };
                let Some(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::HunkHeader,
                }) = this.patch_diff_split_row(row_ix)
                else {
                    return;
                };
                let context_menu_invoker: SharedString =
                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                this.activate_context_menu_invoker(context_menu_invoker, cx);
                this.open_popover_at(
                    PopoverKind::DiffHunkMenu { repo_id, src_ix },
                    e.position,
                    window,
                    cx,
                );
            });
            row = row.on_mouse_down(MouseButton::Right, on_right_click);

            if selected {
                row = row.bg(with_alpha(
                    theme.colors.accent,
                    if theme.is_dark { 0.14 } else { 0.10 },
                ));
            }
            if context_menu_active {
                row = row.bg(theme.colors.active);
            }

            row.into_any_element()
        }
        DiffClickKind::Line => patch_split_meta_row(
            theme,
            ui_scale_percent,
            column,
            visible_ix,
            selected,
            line,
            cx,
        ),
    }
}

fn patch_split_meta_row(
    theme: AppTheme,
    ui_scale_percent: u32,
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    line: &AnnotatedDiffLine,
    cx: &mut gpui::Context<MainPaneView>,
) -> AnyElement {
    let on_click = cx.listener(move |this, e: &ClickEvent, _w, cx| {
        if this.consume_suppress_click_after_drag() {
            cx.notify();
            return;
        }
        this.handle_patch_row_click(visible_ix, DiffClickKind::Line, e.modifiers().shift);
        cx.notify();
    });
    let region = match column {
        PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };

    let (bg, fg, _) = diff_line_colors(theme, line.kind);
    let mut row = div()
        .id((
            match column {
                PatchSplitColumn::Left => "diff_split_left_meta",
                PatchSplitColumn::Right => "diff_split_right_meta",
            },
            visible_ix,
        ))
        .h(diff_row_height(ui_scale_percent))
        .flex()
        .items_center()
        .px_2()
        .text_xs()
        .bg(bg)
        .text_color(fg)
        .whitespace_nowrap()
        .child(selectable_cached_diff_text(
            visible_ix,
            region,
            DiffClickKind::Line,
            fg,
            None,
            SharedString::from(line.text.as_ref().to_owned()),
            cx,
        ))
        .on_click(on_click);

    if selected {
        row = row.bg(with_alpha(
            theme.colors.accent,
            if theme.is_dark { 0.10 } else { 0.07 },
        ));
    }

    row.into_any_element()
}
