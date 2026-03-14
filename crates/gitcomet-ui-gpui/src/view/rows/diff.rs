use super::diff_canvas;
use super::diff_text::*;
use super::*;
use crate::view::panes::main::{
    VersionedCachedDiffStyledText, versioned_cached_diff_styled_text_is_current,
};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::file_diff::FileDiffRowKind;

/// Returns the word-highlight color for a diff line kind: success for Add,
/// danger for Remove, None otherwise.
fn diff_line_word_color(kind: DiffLineKind, theme: AppTheme) -> Option<gpui::Rgba> {
    match kind {
        DiffLineKind::Add => Some(theme.colors.success),
        DiffLineKind::Remove => Some(theme.colors.danger),
        _ => None,
    }
}

/// Applies query overlay to pending styled text if a query is active, returning
/// the final styled text for rendering before it can be cached.
fn pending_styled_with_query_overlay(
    styled: CachedDiffStyledText,
    query: &str,
    theme: AppTheme,
) -> CachedDiffStyledText {
    if query.is_empty() {
        styled
    } else {
        build_cached_diff_query_overlay_styled_text(theme, &styled, query)
    }
}

/// Returns the word-highlight color for a file diff split column.
/// Left highlights Remove/Modify in danger; Right highlights Add/Modify in success.
fn file_diff_split_word_color(
    column: PatchSplitColumn,
    kind: FileDiffRowKind,
    theme: AppTheme,
) -> Option<gpui::Rgba> {
    match column {
        PatchSplitColumn::Left => matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify)
            .then_some(theme.colors.danger),
        PatchSplitColumn::Right => matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify)
            .then_some(theme.colors.success),
    }
}

fn diff_placeholder_row(id: impl Into<gpui::ElementId>, theme: AppTheme) -> AnyElement {
    div()
        .id(id)
        .h(px(20.0))
        .px_2()
        .text_xs()
        .text_color(theme.colors.text_muted)
        .child("")
        .into_any_element()
}

impl MainPaneView {
    fn diff_text_segments_cache_get_for_query(
        &mut self,
        key: usize,
        query: &str,
        syntax_epoch: u64,
    ) -> Option<CachedDiffStyledText> {
        let query = query.trim();
        if query.is_empty() {
            return self
                .diff_text_segments_cache_get(key, syntax_epoch)
                .cloned();
        }

        self.sync_diff_text_query_overlay_cache(query);
        if self.diff_text_query_segments_cache.len() <= key {
            self.diff_text_query_segments_cache
                .resize_with(key + 1, || None);
        }

        if versioned_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
        )
        .is_none()
        {
            let base = self
                .diff_text_segments_cache_get(key, syntax_epoch)?
                .clone();
            let overlaid = build_cached_diff_query_overlay_styled_text(self.theme, &base, query);
            self.diff_text_query_segments_cache[key] = Some(VersionedCachedDiffStyledText {
                syntax_epoch,
                styled: overlaid,
            });
        }

        versioned_cached_diff_styled_text_is_current(
            self.diff_text_query_segments_cache
                .get(key)
                .and_then(Option::as_ref),
            syntax_epoch,
        )
        .cloned()
    }

    pub(in super::super) fn render_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width = this.diff_horizontal_min_width;
        let query = this.diff_search_query_or_empty();

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let language = this.file_diff_cache_language;
            // Inline syntax is now projected from the real old/new (split)
            // documents instead of parsing a synthetic mixed inline stream.
            // syntax_mode is determined per-row based on projection availability.

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(inline_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row(("diff_missing", visible_ix), theme);
                    };

                    let word_ranges: &[Range<usize>] = this
                        .file_diff_inline_word_highlights
                        .get(inline_ix)
                        .and_then(|r| r.as_ref().map(Vec::as_slice))
                        .unwrap_or(&[]);

                    let mut pending_styled = None;
                    let cache_epoch = match this.file_diff_inline_cache.get(inline_ix) {
                        Some(line) => this.file_diff_inline_style_cache_epoch(line),
                        None => {
                            return diff_placeholder_row(("diff_oob", visible_ix), theme);
                        }
                    };
                    if this
                        .diff_text_segments_cache_get(inline_ix, cache_epoch)
                        .is_none()
                    {
                        let Some(line) = this.file_diff_inline_cache.get(inline_ix) else {
                            return diff_placeholder_row(("diff_oob", visible_ix), theme);
                        };

                        let word_color = diff_line_word_color(line.kind, theme);

                        let is_content_line = matches!(
                            line.kind,
                            DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
                        );
                        let line_language = is_content_line.then_some(language).flatten();

                        // Project syntax from the correct side's prepared document.
                        // Full-document views always use Auto fallback since prepared
                        // documents handle the heavy lifting.
                        let projected = this.file_diff_inline_projected_syntax(line);
                        let syntax_mode = syntax_mode_for_prepared_document(projected.document);

                        let (styled, is_pending) =
                            build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                                theme,
                                diff_content_text(line),
                                word_ranges,
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
                            pending_styled =
                                Some(pending_styled_with_query_overlay(styled, &query, theme));
                        } else {
                            this.diff_text_segments_cache_set(inline_ix, cache_epoch, styled);
                        }
                    }

                    let cached_styled = this.diff_text_segments_cache_get_for_query(
                        inline_ix,
                        query.as_ref(),
                        cache_epoch,
                    );
                    let styled = pending_styled.as_ref().or(cached_styled.as_ref());
                    debug_assert!(
                        pending_styled.is_some() || styled.is_some(),
                        "diff text segment cache missing for inline row {inline_ix} after populate"
                    );
                    let Some(line) = this.file_diff_inline_cache.get(inline_ix) else {
                        return diff_placeholder_row(("diff_oob", visible_ix), theme);
                    };

                    diff_row(
                        theme,
                        visible_ix,
                        DiffClickKind::Line,
                        selected,
                        DiffViewMode::Inline,
                        min_width,
                        line,
                        None,
                        None,
                        styled,
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
                    return diff_placeholder_row(("diff_missing", visible_ix), theme);
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

                let should_style = matches!(click_kind, DiffClickKind::Line) || !query.is_empty();
                if should_style
                    && this
                        .diff_text_segments_cache_get(src_ix, cache_epoch)
                        .is_none()
                {
                    let Some(line) = this.patch_diff_row(src_ix) else {
                        return diff_placeholder_row(("diff_oob", visible_ix), theme);
                    };

                    let computed = if matches!(click_kind, DiffClickKind::Line) {
                        let word_color = diff_line_word_color(line.kind, theme);

                        build_cached_diff_styled_text(
                            theme,
                            diff_content_text(&line),
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

                let styled = should_style
                    .then(|| {
                        this.diff_text_segments_cache_get_for_query(
                            src_ix,
                            query.as_ref(),
                            cache_epoch,
                        )
                    })
                    .flatten();

                let Some(line) = this.patch_diff_row(src_ix) else {
                    return diff_placeholder_row(("diff_oob", visible_ix), theme);
                };

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
                diff_row(
                    theme,
                    visible_ix,
                    click_kind,
                    selected,
                    DiffViewMode::Inline,
                    min_width,
                    &line,
                    file_stat,
                    header_display,
                    styled.as_ref(),
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

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(row_ix) = this.diff_mapped_ix_for_visible_ix(visible_ix) else {
                        return diff_placeholder_row(
                            (id_missing, visible_ix),
                            theme,
                        );
                    };
                    let key = this.file_diff_split_cache_key(row_ix, region);
                    let mut pending_styled = None;
                    if let Some(key) = key
                        && this.diff_text_segments_cache_get(key, cache_epoch).is_none()
                    {
                        let Some(row) = this.file_diff_cache_rows.get(row_ix) else {
                            return diff_placeholder_row(
                                (id_oob, visible_ix),
                                theme,
                            );
                        };

                        let text = if is_left {
                            row.old.as_deref()
                        } else {
                            row.new.as_deref()
                        };
                        if let Some(text) = text {
                            let word_color =
                                file_diff_split_word_color(column, row.kind, theme);

                            let word_highlights = if is_left {
                                &this.file_diff_split_word_highlights_old
                            } else {
                                &this.file_diff_split_word_highlights_new
                            };
                            let word_ranges: &[Range<usize>] = word_highlights
                                .get(row_ix)
                                .and_then(|r| r.as_ref().map(Vec::as_slice))
                                .unwrap_or(&[]);

                            let (styled, is_pending) = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                                theme,
                                text,
                                word_ranges,
                                "",
                                DiffSyntaxConfig {
                                    language,
                                    mode: syntax_mode,
                                },
                                word_color,
                                PreparedDiffSyntaxLine {
                                    document: syntax_document,
                                    line_ix: row_ix,
                                },
                            )
                            .into_parts();
                            if is_pending {
                                this.ensure_prepared_syntax_chunk_poll(cx);
                                pending_styled = Some(pending_styled_with_query_overlay(
                                    styled, &query, theme,
                                ));
                            } else {
                                this.diff_text_segments_cache_set(key, cache_epoch, styled);
                            }
                        }
                    }

                    let row_has_content = this
                        .file_diff_cache_rows
                        .get(row_ix)
                        .is_some_and(|row| if is_left { row.old.is_some() } else { row.new.is_some() });
                    let cached_styled = if row_has_content {
                        key.and_then(|k| {
                            this.diff_text_segments_cache_get_for_query(
                                k,
                                query.as_ref(),
                                cache_epoch,
                            )
                        })
                    } else {
                        None
                    };
                    let styled = pending_styled.as_ref().or(cached_styled.as_ref());
                    debug_assert!(
                        !row_has_content || key.is_none() || pending_styled.is_some() || styled.is_some(),
                        "diff text segment cache missing for split-{column:?} row {row_ix} after populate"
                    );

                    let Some(row) = this.file_diff_cache_rows.get(row_ix) else {
                        return diff_placeholder_row(
                            (id_oob, visible_ix),
                            theme,
                        );
                    };
                    patch_split_column_row(
                        theme,
                        column,
                        visible_ix,
                        selected,
                        min_width,
                        row,
                        styled,
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
                    return diff_placeholder_row((id_missing, visible_ix), theme);
                };
                let Some(row) = this.patch_diff_split_row(row_ix) else {
                    return diff_placeholder_row((id_oob, visible_ix), theme);
                };

                match row {
                    PatchSplitRow::Aligned {
                        row,
                        old_src_ix,
                        new_src_ix,
                    } => {
                        let src_ix = if is_left { old_src_ix } else { new_src_ix };
                        if let Some(src_ix) = src_ix
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
                            let language =
                                this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                            this.ensure_patch_diff_word_highlight_for_src_ix(src_ix);
                            let word_ranges: &[Range<usize>] = this
                                .diff_word_highlights
                                .get(src_ix)
                                .and_then(|r| r.as_ref().map(Vec::as_slice))
                                .unwrap_or(&[]);
                            let word_color = this
                                .patch_diff_row(src_ix)
                                .and_then(|line| diff_line_word_color(line.kind, theme));

                            let computed = build_cached_diff_styled_text(
                                theme,
                                text,
                                word_ranges,
                                "",
                                language,
                                syntax_mode,
                                word_color,
                            );
                            this.diff_text_segments_cache_set(src_ix, cache_epoch, computed);
                        }

                        let styled = src_ix.and_then(|src_ix| {
                            this.diff_text_segments_cache_get_for_query(
                                src_ix,
                                query.as_ref(),
                                cache_epoch,
                            )
                        });

                        patch_split_column_row(
                            theme,
                            column,
                            visible_ix,
                            selected,
                            min_width,
                            &row,
                            styled.as_ref(),
                            cx,
                        )
                    }
                    PatchSplitRow::Raw { src_ix, click_kind } => {
                        if this.patch_diff_row(src_ix).is_none() {
                            return diff_placeholder_row((id_src_oob, visible_ix), theme);
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
                        let styled = should_style
                            .then(|| {
                                this.diff_text_segments_cache_get_for_query(
                                    src_ix,
                                    query.as_ref(),
                                    cache_epoch,
                                )
                            })
                            .flatten();
                        let Some(line) = this.patch_diff_row(src_ix) else {
                            return diff_placeholder_row((id_src_oob, visible_ix), theme);
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
                        patch_split_header_row(
                            theme,
                            column,
                            visible_ix,
                            click_kind,
                            selected,
                            min_width,
                            &line,
                            file_stat,
                            this.diff_header_display_cache.get(&src_ix).cloned(),
                            styled.as_ref(),
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
    visible_ix: usize,
    click_kind: DiffClickKind,
    selected: bool,
    mode: DiffViewMode,
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

    if matches!(click_kind, DiffClickKind::FileHeader) {
        let file = header_display.unwrap_or_else(|| line.text.clone().into());
        let mut row = div()
            .id(("diff_file_hdr", visible_ix))
            .h(px(28.0))
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
        let display = header_display.unwrap_or_else(|| line.text.clone().into());

        let mut row = div()
            .id(("diff_hunk_hdr", visible_ix))
            .h(px(24.0))
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
            visible_ix,
            min_width,
            selected,
            old,
            new,
            bg,
            fg,
            gutter_fg,
            styled,
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

            diff_canvas::split_diff_line_row_canvas(
                theme,
                cx.entity(),
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
    column: PatchSplitColumn,
    visible_ix: usize,
    selected: bool,
    min_width: Pixels,
    row: &gitcomet_core::file_diff::FileDiffRow,
    styled: Option<&CachedDiffStyledText>,
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
        column,
        visible_ix,
        min_width,
        selected,
        bg,
        fg,
        gutter_fg,
        line_no,
        styled,
    )
}

#[allow(clippy::too_many_arguments)]
fn patch_split_header_row(
    theme: AppTheme,
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
            let display = header_display.unwrap_or_else(|| line.text.clone().into());
            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_file_hdr",
                        PatchSplitColumn::Right => "diff_split_right_file_hdr",
                    },
                    visible_ix,
                ))
                .h(px(28.0))
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
            let display = header_display.unwrap_or_else(|| line.text.clone().into());

            let mut row = div()
                .id((
                    match column {
                        PatchSplitColumn::Left => "diff_split_left_hunk_hdr",
                        PatchSplitColumn::Right => "diff_split_right_hunk_hdr",
                    },
                    visible_ix,
                ))
                .h(px(24.0))
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
        DiffClickKind::Line => patch_split_meta_row(theme, column, visible_ix, selected, line, cx),
    }
}

fn patch_split_meta_row(
    theme: AppTheme,
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
        .h(px(20.0))
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
            line.text.clone().into(),
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
