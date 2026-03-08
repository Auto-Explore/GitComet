use super::diff_canvas;
use super::diff_text::*;
use super::*;

impl MainPaneView {
    pub(in super::super) fn render_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width = this.diff_horizontal_min_width;
        static EMPTY_QUERY: SharedString = SharedString::new_static("");
        let query = if this.diff_search_active {
            this.diff_search_query.clone()
        } else {
            EMPTY_QUERY.clone()
        };

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let empty_ranges: &[Range<usize>] = &[];
            let syntax_mode =
                if this.file_diff_inline_cache.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
                    DiffSyntaxMode::Auto
                } else {
                    DiffSyntaxMode::HeuristicOnly
                };
            let language = this.file_diff_cache_language;

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(inline_ix) = this.diff_visible_indices.get(visible_ix).copied() else {
                        return div()
                            .id(("diff_missing", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };

                    let word_ranges: &[Range<usize>] = this
                        .file_diff_inline_word_highlights
                        .get(inline_ix)
                        .and_then(|r| r.as_ref().map(Vec::as_slice))
                        .unwrap_or(empty_ranges);

                    if this.diff_text_segments_cache_get(inline_ix).is_none() {
                        let Some(line) = this.file_diff_inline_cache.get(inline_ix) else {
                            return div()
                                .id(("diff_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };

                        let word_color = match line.kind {
                            gitcomet_core::domain::DiffLineKind::Add => Some(theme.colors.success),
                            gitcomet_core::domain::DiffLineKind::Remove => {
                                Some(theme.colors.danger)
                            }
                            _ => None,
                        };

                        let language = matches!(
                            line.kind,
                            gitcomet_core::domain::DiffLineKind::Add
                                | gitcomet_core::domain::DiffLineKind::Remove
                                | gitcomet_core::domain::DiffLineKind::Context
                        )
                        .then_some(language)
                        .flatten();

                        let computed = build_cached_diff_styled_text(
                            theme,
                            diff_content_text(line),
                            word_ranges,
                            &query,
                            language,
                            syntax_mode,
                            word_color,
                        );
                        this.diff_text_segments_cache_set(inline_ix, computed);
                    }

                    let Some(line) = this.file_diff_inline_cache.get(inline_ix) else {
                        return div()
                            .id(("diff_oob", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    let styled = this.diff_text_segments_cache_get(inline_ix);
                    debug_assert!(
                        styled.is_some(),
                        "diff text segment cache missing for inline row {inline_ix} after populate"
                    );

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
        let repo_id_for_context_menu = this.active_repo_id();
        let active_context_menu_invoker = this.active_context_menu_invoker.clone();
        let syntax_mode = if this.diff_cache.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            DiffSyntaxMode::Auto
        } else {
            DiffSyntaxMode::HeuristicOnly
        };
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                let Some(src_ix) = this.diff_visible_indices.get(visible_ix).copied() else {
                    return div()
                        .id(("diff_missing", visible_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };
                let click_kind = this
                    .diff_click_kinds
                    .get(src_ix)
                    .copied()
                    .unwrap_or(DiffClickKind::Line);

                let word_ranges: &[Range<usize>] = this
                    .diff_word_highlights
                    .get(src_ix)
                    .and_then(|r| r.as_ref().map(Vec::as_slice))
                    .unwrap_or(&[]);

                let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);

                let language = this.diff_language_for_src_ix.get(src_ix).copied().flatten();

                let should_style = matches!(click_kind, DiffClickKind::Line) || !query.is_empty();
                if should_style && this.diff_text_segments_cache_get(src_ix).is_none() {
                    let Some(line) = this.diff_cache.get(src_ix) else {
                        return div()
                            .id(("diff_oob", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };

                    let computed = if matches!(click_kind, DiffClickKind::Line) {
                        let word_color = match line.kind {
                            gitcomet_core::domain::DiffLineKind::Add => Some(theme.colors.success),
                            gitcomet_core::domain::DiffLineKind::Remove => {
                                Some(theme.colors.danger)
                            }
                            _ => None,
                        };

                        build_cached_diff_styled_text(
                            theme,
                            diff_content_text(line),
                            word_ranges,
                            &query,
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
                            &query,
                            None,
                            syntax_mode,
                            None,
                        )
                    };
                    this.diff_text_segments_cache_set(src_ix, computed);
                }

                let styled: Option<&CachedDiffStyledText> = should_style
                    .then(|| this.diff_text_segments_cache_get(src_ix))
                    .flatten();

                let Some(line) = this.diff_cache.get(src_ix) else {
                    return div()
                        .id(("diff_oob", visible_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
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
                    line,
                    file_stat,
                    header_display,
                    styled,
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
        let min_width = this.diff_horizontal_min_width;
        static EMPTY_QUERY: SharedString = SharedString::new_static("");
        let query = if this.diff_search_active {
            this.diff_search_query.clone()
        } else {
            EMPTY_QUERY.clone()
        };

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let empty_ranges: &[Range<usize>] = &[];
            let syntax_mode =
                if this.file_diff_cache_rows.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
                    DiffSyntaxMode::Auto
                } else {
                    DiffSyntaxMode::HeuristicOnly
                };
            let language = this.file_diff_cache_language;

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(row_ix) = this.diff_visible_indices.get(visible_ix).copied() else {
                        return div()
                            .id(("diff_split_left_missing", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    let key = row_ix * 2;
                    if this.diff_text_segments_cache_get(key).is_none() {
                        let Some(row) = this.file_diff_cache_rows.get(row_ix) else {
                            return div()
                                .id(("diff_split_left_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };

                        if let Some(text) = row.old.as_deref() {
                            let word_color = matches!(
                                row.kind,
                                gitcomet_core::file_diff::FileDiffRowKind::Remove
                                    | gitcomet_core::file_diff::FileDiffRowKind::Modify
                            )
                            .then_some(theme.colors.danger);

                            let word_ranges: &[Range<usize>] = this
                                .file_diff_split_word_highlights_old
                                .get(row_ix)
                                .and_then(|r| r.as_ref().map(Vec::as_slice))
                                .unwrap_or(empty_ranges);

                            let computed = build_cached_diff_styled_text(
                                theme,
                                text,
                                word_ranges,
                                &query,
                                language,
                                syntax_mode,
                                word_color,
                            );
                            this.diff_text_segments_cache_set(key, computed);
                        }
                    }

                    let Some(row) = this.file_diff_cache_rows.get(row_ix) else {
                        return div()
                            .id(("diff_split_left_oob", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    let styled: Option<&CachedDiffStyledText> = row
                        .old
                        .is_some()
                        .then(|| this.diff_text_segments_cache_get(key))
                        .flatten();

                    patch_split_column_row(
                        theme,
                        PatchSplitColumn::Left,
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
        let syntax_mode = if this.diff_cache.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            DiffSyntaxMode::Auto
        } else {
            DiffSyntaxMode::HeuristicOnly
        };
        let empty_ranges: &[Range<usize>] = &[];
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                let Some(row_ix) = this.diff_visible_indices.get(visible_ix).copied() else {
                    return div()
                        .id(("diff_split_left_missing", visible_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };
                let Some(row) = this.diff_split_cache.get(row_ix) else {
                    return div()
                        .id(("diff_split_left_oob", visible_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };

                match row {
                    PatchSplitRow::Aligned { old_src_ix, .. } => {
                        if let Some(src_ix) = *old_src_ix
                            && this.diff_text_segments_cache_get(src_ix).is_none()
                        {
                            let Some(PatchSplitRow::Aligned { row, .. }) =
                                this.diff_split_cache.get(row_ix)
                            else {
                                return div()
                                    .id(("diff_split_left_oob", visible_ix))
                                    .h(px(20.0))
                                    .px_2()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("")
                                    .into_any_element();
                            };

                            let text = row.old.as_deref().unwrap_or("");
                            let language =
                                this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                            let word_ranges: &[Range<usize>] = this
                                .diff_word_highlights
                                .get(src_ix)
                                .and_then(|r| r.as_ref().map(Vec::as_slice))
                                .unwrap_or(empty_ranges);
                            let word_color =
                                this.diff_cache
                                    .get(src_ix)
                                    .and_then(|line| match line.kind {
                                        gitcomet_core::domain::DiffLineKind::Add => {
                                            Some(theme.colors.success)
                                        }
                                        gitcomet_core::domain::DiffLineKind::Remove => {
                                            Some(theme.colors.danger)
                                        }
                                        _ => None,
                                    });

                            let computed = build_cached_diff_styled_text(
                                theme,
                                text,
                                word_ranges,
                                &query,
                                language,
                                syntax_mode,
                                word_color,
                            );
                            this.diff_text_segments_cache_set(src_ix, computed);
                        }

                        let Some(PatchSplitRow::Aligned {
                            row, old_src_ix, ..
                        }) = this.diff_split_cache.get(row_ix)
                        else {
                            return div()
                                .id(("diff_split_left_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };

                        let styled =
                            old_src_ix.and_then(|src_ix| this.diff_text_segments_cache_get(src_ix));

                        patch_split_column_row(
                            theme,
                            PatchSplitColumn::Left,
                            visible_ix,
                            selected,
                            min_width,
                            row,
                            styled,
                            cx,
                        )
                    }
                    PatchSplitRow::Raw { src_ix, click_kind } => {
                        let src_ix = *src_ix;
                        let click_kind = *click_kind;
                        if this.diff_cache.get(src_ix).is_none() {
                            return div()
                                .id(("diff_split_left_src_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };
                        let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);
                        let should_style = !query.is_empty();
                        if should_style && this.diff_text_segments_cache_get(src_ix).is_none() {
                            let display = this
                                .diff_text_line_for_region(visible_ix, DiffTextRegion::SplitLeft);
                            let computed = build_cached_diff_styled_text(
                                theme,
                                display.as_ref(),
                                &[],
                                &query,
                                None,
                                syntax_mode,
                                None,
                            );
                            this.diff_text_segments_cache_set(src_ix, computed);
                        }
                        let styled = should_style
                            .then(|| this.diff_text_segments_cache_get(src_ix))
                            .flatten();
                        let Some(line) = this.diff_cache.get(src_ix) else {
                            return div()
                                .id(("diff_split_left_src_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };
                        let context_menu_active = click_kind == DiffClickKind::HunkHeader
                            && this.active_repo_id().is_some_and(|repo_id| {
                                let invoker: SharedString =
                                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                                this.active_context_menu_invoker.as_ref() == Some(&invoker)
                            });
                        patch_split_header_row(
                            theme,
                            PatchSplitColumn::Left,
                            visible_ix,
                            click_kind,
                            selected,
                            min_width,
                            line,
                            file_stat,
                            this.diff_header_display_cache.get(&src_ix).cloned(),
                            styled,
                            context_menu_active,
                            cx,
                        )
                    }
                }
            })
            .collect()
    }

    pub(in super::super) fn render_diff_split_right_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let min_width = this.diff_horizontal_min_width;
        static EMPTY_QUERY: SharedString = SharedString::new_static("");
        let query = if this.diff_search_active {
            this.diff_search_query.clone()
        } else {
            EMPTY_QUERY.clone()
        };

        if this.is_file_diff_view_active() {
            let theme = this.theme;
            let empty_ranges: &[Range<usize>] = &[];
            let syntax_mode =
                if this.file_diff_cache_rows.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
                    DiffSyntaxMode::Auto
                } else {
                    DiffSyntaxMode::HeuristicOnly
                };
            let language = this.file_diff_cache_language;

            return range
                .map(|visible_ix| {
                    let selected = this
                        .diff_selection_range
                        .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                    let Some(row_ix) = this.diff_visible_indices.get(visible_ix).copied() else {
                        return div()
                            .id(("diff_split_right_missing", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    let key = row_ix * 2 + 1;
                    if this.diff_text_segments_cache_get(key).is_none() {
                        let Some(row) = this.file_diff_cache_rows.get(row_ix) else {
                            return div()
                                .id(("diff_split_right_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };

                        if let Some(text) = row.new.as_deref() {
                            let word_color = matches!(
                                row.kind,
                                gitcomet_core::file_diff::FileDiffRowKind::Add
                                    | gitcomet_core::file_diff::FileDiffRowKind::Modify
                            )
                            .then_some(theme.colors.success);

                            let word_ranges: &[Range<usize>] = this
                                .file_diff_split_word_highlights_new
                                .get(row_ix)
                                .and_then(|r| r.as_ref().map(Vec::as_slice))
                                .unwrap_or(empty_ranges);

                            let computed = build_cached_diff_styled_text(
                                theme,
                                text,
                                word_ranges,
                                &query,
                                language,
                                syntax_mode,
                                word_color,
                            );
                            this.diff_text_segments_cache_set(key, computed);
                        }
                    }

                    let Some(row) = this.file_diff_cache_rows.get(row_ix) else {
                        return div()
                            .id(("diff_split_right_oob", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    let styled: Option<&CachedDiffStyledText> = row
                        .new
                        .is_some()
                        .then(|| this.diff_text_segments_cache_get(key))
                        .flatten();

                    patch_split_column_row(
                        theme,
                        PatchSplitColumn::Right,
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
        let syntax_mode = if this.diff_cache.len() <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
            DiffSyntaxMode::Auto
        } else {
            DiffSyntaxMode::HeuristicOnly
        };
        let empty_ranges: &[Range<usize>] = &[];
        range
            .map(|visible_ix| {
                let selected = this
                    .diff_selection_range
                    .is_some_and(|(a, b)| visible_ix >= a.min(b) && visible_ix <= a.max(b));

                let Some(row_ix) = this.diff_visible_indices.get(visible_ix).copied() else {
                    return div()
                        .id(("diff_split_right_missing", visible_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };
                let Some(row) = this.diff_split_cache.get(row_ix) else {
                    return div()
                        .id(("diff_split_right_oob", visible_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };

                match row {
                    PatchSplitRow::Aligned { new_src_ix, .. } => {
                        if let Some(src_ix) = *new_src_ix
                            && this.diff_text_segments_cache_get(src_ix).is_none()
                        {
                            let Some(PatchSplitRow::Aligned { row, .. }) =
                                this.diff_split_cache.get(row_ix)
                            else {
                                return div()
                                    .id(("diff_split_right_oob", visible_ix))
                                    .h(px(20.0))
                                    .px_2()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child("")
                                    .into_any_element();
                            };

                            let text = row.new.as_deref().unwrap_or("");
                            let language =
                                this.diff_language_for_src_ix.get(src_ix).copied().flatten();
                            let word_ranges: &[Range<usize>] = this
                                .diff_word_highlights
                                .get(src_ix)
                                .and_then(|r| r.as_ref().map(Vec::as_slice))
                                .unwrap_or(empty_ranges);
                            let word_color =
                                this.diff_cache
                                    .get(src_ix)
                                    .and_then(|line| match line.kind {
                                        gitcomet_core::domain::DiffLineKind::Add => {
                                            Some(theme.colors.success)
                                        }
                                        gitcomet_core::domain::DiffLineKind::Remove => {
                                            Some(theme.colors.danger)
                                        }
                                        _ => None,
                                    });

                            let computed = build_cached_diff_styled_text(
                                theme,
                                text,
                                word_ranges,
                                &query,
                                language,
                                syntax_mode,
                                word_color,
                            );
                            this.diff_text_segments_cache_set(src_ix, computed);
                        }

                        let Some(PatchSplitRow::Aligned {
                            row, new_src_ix, ..
                        }) = this.diff_split_cache.get(row_ix)
                        else {
                            return div()
                                .id(("diff_split_right_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };

                        let styled =
                            new_src_ix.and_then(|src_ix| this.diff_text_segments_cache_get(src_ix));

                        patch_split_column_row(
                            theme,
                            PatchSplitColumn::Right,
                            visible_ix,
                            selected,
                            min_width,
                            row,
                            styled,
                            cx,
                        )
                    }
                    PatchSplitRow::Raw { src_ix, click_kind } => {
                        let src_ix = *src_ix;
                        let click_kind = *click_kind;
                        if this.diff_cache.get(src_ix).is_none() {
                            return div()
                                .id(("diff_split_right_src_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };
                        let file_stat = this.diff_file_stats.get(src_ix).and_then(|s| *s);
                        let should_style = !query.is_empty();
                        if should_style && this.diff_text_segments_cache_get(src_ix).is_none() {
                            let display = this
                                .diff_text_line_for_region(visible_ix, DiffTextRegion::SplitRight);
                            let computed = build_cached_diff_styled_text(
                                theme,
                                display.as_ref(),
                                &[],
                                &query,
                                None,
                                syntax_mode,
                                None,
                            );
                            this.diff_text_segments_cache_set(src_ix, computed);
                        }
                        let styled = should_style
                            .then(|| this.diff_text_segments_cache_get(src_ix))
                            .flatten();
                        let Some(line) = this.diff_cache.get(src_ix) else {
                            return div()
                                .id(("diff_split_right_src_oob", visible_ix))
                                .h(px(20.0))
                                .px_2()
                                .text_xs()
                                .text_color(theme.colors.text_muted)
                                .child("")
                                .into_any_element();
                        };
                        let context_menu_active = click_kind == DiffClickKind::HunkHeader
                            && this.active_repo_id().is_some_and(|repo_id| {
                                let invoker: SharedString =
                                    format!("diff_hunk_menu_{}_{}", repo_id.0, src_ix).into();
                                this.active_context_menu_invoker.as_ref() == Some(&invoker)
                            });
                        patch_split_header_row(
                            theme,
                            PatchSplitColumn::Right,
                            visible_ix,
                            click_kind,
                            selected,
                            min_width,
                            line,
                            file_stat,
                            this.diff_header_display_cache.get(&src_ix).cloned(),
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
            let Some(&src_ix) = this.diff_visible_indices.get(visible_ix) else {
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
            let left_kind = match line.kind {
                gitcomet_core::domain::DiffLineKind::Remove => {
                    gitcomet_core::domain::DiffLineKind::Remove
                }
                gitcomet_core::domain::DiffLineKind::Add => {
                    gitcomet_core::domain::DiffLineKind::Context
                }
                _ => gitcomet_core::domain::DiffLineKind::Context,
            };
            let right_kind = match line.kind {
                gitcomet_core::domain::DiffLineKind::Add => {
                    gitcomet_core::domain::DiffLineKind::Add
                }
                gitcomet_core::domain::DiffLineKind::Remove => {
                    gitcomet_core::domain::DiffLineKind::Context
                }
                _ => gitcomet_core::domain::DiffLineKind::Context,
            };

            let (left_bg, left_fg, left_gutter) = diff_line_colors(theme, left_kind);
            let (right_bg, right_fg, right_gutter) = diff_line_colors(theme, right_kind);

            let (left_text, right_text) = match line.kind {
                gitcomet_core::domain::DiffLineKind::Remove => (styled, None),
                gitcomet_core::domain::DiffLineKind::Add => (None, styled),
                gitcomet_core::domain::DiffLineKind::Context => (styled, styled),
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
    let (ctx_bg, ctx_fg, ctx_gutter) =
        diff_line_colors(theme, gitcomet_core::domain::DiffLineKind::Context);
    let (add_bg, add_fg, add_gutter) =
        diff_line_colors(theme, gitcomet_core::domain::DiffLineKind::Add);
    let (rem_bg, rem_fg, rem_gutter) =
        diff_line_colors(theme, gitcomet_core::domain::DiffLineKind::Remove);

    let (bg, fg, gutter_fg) = match (column, row.kind) {
        (
            PatchSplitColumn::Left,
            gitcomet_core::file_diff::FileDiffRowKind::Remove
            | gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ) => (rem_bg, rem_fg, rem_gutter),
        (
            PatchSplitColumn::Right,
            gitcomet_core::file_diff::FileDiffRowKind::Add
            | gitcomet_core::file_diff::FileDiffRowKind::Modify,
        ) => (add_bg, add_fg, add_gutter),
        _ => (ctx_bg, ctx_fg, ctx_gutter),
    };

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
                let Some(&row_ix) = this.diff_visible_indices.get(visible_ix) else {
                    return;
                };
                let Some(PatchSplitRow::Raw {
                    src_ix,
                    click_kind: DiffClickKind::HunkHeader,
                }) = this.diff_split_cache.get(row_ix)
                else {
                    return;
                };
                let src_ix = *src_ix;
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
