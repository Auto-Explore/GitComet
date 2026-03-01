use super::super::conflict_resolver;
use super::diff_text::*;
use super::*;

impl MainPaneView {
    pub(in super::super) fn render_conflict_resolver_three_way_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;
        let show_ws = this.show_whitespace;
        let [col_a_w, col_b_w, col_c_w] = this.conflict_three_way_col_widths;
        let active_range = this
            .conflict_resolver
            .three_way_conflict_ranges
            .get(this.conflict_resolver.active_conflict)
            .cloned();

        // Build a lookup: for each line index, which conflict range_ix does it belong to?
        let conflict_ranges = &this.conflict_resolver.three_way_conflict_ranges;
        let conflict_range_for_ix =
            |ix: usize| -> Option<usize> { conflict_ranges.iter().position(|r| r.contains(&ix)) };

        // Build per-conflict choice lookup so we can highlight the selected column.
        let conflict_choices: Vec<conflict_resolver::ConflictChoice> = this
            .conflict_resolver
            .marker_segments
            .iter()
            .filter_map(|seg| match seg {
                conflict_resolver::ConflictSegment::Block(b) => Some(b.choice),
                _ => None,
            })
            .collect();

        // Collect the real line indices we need to render (from visible map).
        let real_line_indices: Vec<usize> = range
            .clone()
            .filter_map(
                |vi| match this.conflict_resolver.three_way_visible_map.get(vi) {
                    Some(conflict_resolver::ThreeWayVisibleItem::Line(ix)) => Some(*ix),
                    _ => None,
                },
            )
            .collect();

        let word_hl_color = Some(theme.colors.warning);

        // Resolve syntax language from the conflict file path for tree-sitter highlighting.
        let syntax_lang = this
            .conflict_resolver
            .path
            .as_ref()
            .and_then(|p| diff_syntax_language_for_path(&p.to_string_lossy()));

        // Pre-build styled text cache entries for all visible lines.
        for &ix in &real_line_indices {
            for (col, highlights_vec) in [
                (
                    ThreeWayColumn::Base,
                    &this.conflict_resolver.three_way_word_highlights_base,
                ),
                (
                    ThreeWayColumn::Ours,
                    &this.conflict_resolver.three_way_word_highlights_ours,
                ),
                (
                    ThreeWayColumn::Theirs,
                    &this.conflict_resolver.three_way_word_highlights_theirs,
                ),
            ] {
                if this
                    .conflict_three_way_segments_cache
                    .contains_key(&(ix, col))
                {
                    continue;
                }
                let word_ranges = highlights_vec
                    .get(ix)
                    .and_then(|o| o.as_ref())
                    .map(|v| v.as_slice())
                    .unwrap_or(&[]);
                let text = match col {
                    ThreeWayColumn::Base => this
                        .conflict_resolver
                        .three_way_base_lines
                        .get(ix)
                        .map(|s| s.as_ref())
                        .unwrap_or(""),
                    ThreeWayColumn::Ours => this
                        .conflict_resolver
                        .three_way_ours_lines
                        .get(ix)
                        .map(|s| s.as_ref())
                        .unwrap_or(""),
                    ThreeWayColumn::Theirs => this
                        .conflict_resolver
                        .three_way_theirs_lines
                        .get(ix)
                        .map(|s| s.as_ref())
                        .unwrap_or(""),
                };
                if text.is_empty() {
                    continue;
                }
                if word_ranges.is_empty() && syntax_lang.is_none() {
                    continue;
                }
                let styled = build_cached_diff_styled_text(
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_lang,
                    DiffSyntaxMode::Auto,
                    word_hl_color,
                );
                this.conflict_three_way_segments_cache
                    .insert((ix, col), styled);
            }
        }

        // Background for the selected (chosen) column in a conflict range.
        let chosen_bg = with_alpha(theme.colors.accent, if theme.is_dark { 0.16 } else { 0.12 });

        let mut elements = Vec::with_capacity(range.len());
        for vi in range {
            let Some(visible_item) = this.conflict_resolver.three_way_visible_map.get(vi) else {
                continue;
            };

            match *visible_item {
                conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(range_ix) => {
                    // Render a collapsed summary row for a resolved conflict.
                    let choice_label = conflict_choices
                        .get(range_ix)
                        .map(|c| match c {
                            conflict_resolver::ConflictChoice::Base => "Base (A)",
                            conflict_resolver::ConflictChoice::Ours => "Local (B)",
                            conflict_resolver::ConflictChoice::Theirs => "Remote (C)",
                            conflict_resolver::ConflictChoice::Both => "Local+Remote (B+C)",
                        })
                        .unwrap_or("?");
                    let label: SharedString = format!("  Resolved: picked {choice_label}").into();
                    let handle_w = px(PANE_RESIZE_HANDLE_PX);
                    elements.push(
                        div()
                            .id(("conflict_three_way_collapsed", vi))
                            .w_full()
                            .h(px(20.0))
                            .flex()
                            .items_center()
                            .bg(with_alpha(
                                theme.colors.success,
                                if theme.is_dark { 0.08 } else { 0.06 },
                            ))
                            .child(
                                div()
                                    .w(col_a_w)
                                    .min_w(px(0.0))
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .px_2()
                                    .text_xs()
                                    .text_color(theme.colors.text_muted)
                                    .child(label),
                            )
                            .child(
                                div()
                                    .w(handle_w)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border)),
                            )
                            .child(div().w(col_b_w).min_w(px(0.0)).h_full())
                            .child(
                                div()
                                    .w(handle_w)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border)),
                            )
                            .child(div().w(col_c_w).flex_grow().min_w(px(0.0)).h_full())
                            .into_any_element(),
                    );
                }
                conflict_resolver::ThreeWayVisibleItem::Line(ix) => {
                    let base_line = this.conflict_resolver.three_way_base_lines.get(ix);
                    let ours_line = this.conflict_resolver.three_way_ours_lines.get(ix);
                    let theirs_line = this.conflict_resolver.three_way_theirs_lines.get(ix);
                    let is_in_active_conflict =
                        active_range.as_ref().is_some_and(|r| r.contains(&ix));
                    let range_ix = conflict_range_for_ix(ix);
                    let is_in_conflict = range_ix.is_some();

                    // Which column is chosen for this conflict?
                    let choice_for_row = range_ix.and_then(|ri| conflict_choices.get(ri).copied());
                    let base_is_chosen =
                        choice_for_row == Some(conflict_resolver::ConflictChoice::Base);
                    let ours_is_chosen = matches!(
                        choice_for_row,
                        Some(conflict_resolver::ConflictChoice::Ours)
                            | Some(conflict_resolver::ConflictChoice::Both)
                    );
                    let theirs_is_chosen = matches!(
                        choice_for_row,
                        Some(conflict_resolver::ConflictChoice::Theirs)
                            | Some(conflict_resolver::ConflictChoice::Both)
                    );

                    let base_styled = this
                        .conflict_three_way_segments_cache
                        .get(&(ix, ThreeWayColumn::Base));
                    let ours_styled = this
                        .conflict_three_way_segments_cache
                        .get(&(ix, ThreeWayColumn::Ours));
                    let theirs_styled = this
                        .conflict_three_way_segments_cache
                        .get(&(ix, ThreeWayColumn::Theirs));

                    let mut base = div()
                        .id(("conflict_three_way_base", ix))
                        .w(col_a_w)
                        .min_w(px(0.0))
                        .h(px(20.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .border_1()
                        .border_color(with_alpha(theme.colors.success, 0.0))
                        .text_color(if base_line.is_some() {
                            theme.colors.text
                        } else {
                            theme.colors.text_muted
                        })
                        .whitespace_nowrap()
                        .when(base_is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div().w(px(38.0)).text_color(theme.colors.text_muted).child(
                                line_number_string(
                                    base_line
                                        .is_some()
                                        .then(|| u32::try_from(ix + 1).ok())
                                        .flatten(),
                                ),
                            ),
                        )
                        .child(conflict_diff_text_cell(
                            base_line.cloned().unwrap_or_default(),
                            base_styled,
                            show_ws,
                        ));
                    if let Some(ri) = range_ix.filter(|_| base_line.is_some()) {
                        base = base
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| {
                                s.bg(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.14 } else { 0.10 },
                                ))
                                .border_color(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.78 } else { 0.64 },
                                ))
                            })
                            .active(move |s| {
                                s.bg(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.20 } else { 0.16 },
                                ))
                                .border_color(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.94 } else { 0.82 },
                                ))
                            })
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                this.conflict_resolver_pick_at(
                                    ri,
                                    conflict_resolver::ConflictChoice::Base,
                                    cx,
                                );
                            }));
                    }

                    let mut ours = div()
                        .id(("conflict_three_way_ours", ix))
                        .w(col_b_w)
                        .min_w(px(0.0))
                        .h(px(20.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .border_1()
                        .border_color(with_alpha(theme.colors.success, 0.0))
                        .text_color(if ours_line.is_some() {
                            theme.colors.text
                        } else {
                            theme.colors.text_muted
                        })
                        .whitespace_nowrap()
                        .when(ours_is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div().w(px(38.0)).text_color(theme.colors.text_muted).child(
                                line_number_string(
                                    ours_line
                                        .is_some()
                                        .then(|| u32::try_from(ix + 1).ok())
                                        .flatten(),
                                ),
                            ),
                        )
                        .child(conflict_diff_text_cell(
                            ours_line.cloned().unwrap_or_default(),
                            ours_styled,
                            show_ws,
                        ));
                    if let Some(ri) = range_ix {
                        ours = ours
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| {
                                s.bg(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.14 } else { 0.10 },
                                ))
                                .border_color(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.78 } else { 0.64 },
                                ))
                            })
                            .active(move |s| {
                                s.bg(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.20 } else { 0.16 },
                                ))
                                .border_color(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.94 } else { 0.82 },
                                ))
                            })
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                this.conflict_resolver_pick_at(
                                    ri,
                                    conflict_resolver::ConflictChoice::Ours,
                                    cx,
                                );
                            }));
                    }

                    let mut theirs = div()
                        .id(("conflict_three_way_theirs", ix))
                        .w(col_c_w)
                        .flex_grow()
                        .min_w(px(0.0))
                        .h(px(20.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .border_1()
                        .border_color(with_alpha(theme.colors.success, 0.0))
                        .text_color(if theirs_line.is_some() {
                            theme.colors.text
                        } else {
                            theme.colors.text_muted
                        })
                        .whitespace_nowrap()
                        .when(theirs_is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div().w(px(38.0)).text_color(theme.colors.text_muted).child(
                                line_number_string(
                                    theirs_line
                                        .is_some()
                                        .then(|| u32::try_from(ix + 1).ok())
                                        .flatten(),
                                ),
                            ),
                        )
                        .child(conflict_diff_text_cell(
                            theirs_line.cloned().unwrap_or_default(),
                            theirs_styled,
                            show_ws,
                        ));
                    if let Some(ri) = range_ix {
                        theirs = theirs
                            .cursor(CursorStyle::PointingHand)
                            .hover(move |s| {
                                s.bg(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.14 } else { 0.10 },
                                ))
                                .border_color(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.78 } else { 0.64 },
                                ))
                            })
                            .active(move |s| {
                                s.bg(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.20 } else { 0.16 },
                                ))
                                .border_color(with_alpha(
                                    theme.colors.success,
                                    if theme.is_dark { 0.94 } else { 0.82 },
                                ))
                            })
                            .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                                this.conflict_resolver_pick_at(
                                    ri,
                                    conflict_resolver::ConflictChoice::Theirs,
                                    cx,
                                );
                            }));
                    }

                    let handle_w = px(PANE_RESIZE_HANDLE_PX);
                    elements.push(
                        div()
                            .id(("conflict_three_way_row", ix))
                            .w_full()
                            .flex()
                            .when(is_in_active_conflict, |d| {
                                d.bg(with_alpha(
                                    theme.colors.accent,
                                    if theme.is_dark { 0.08 } else { 0.06 },
                                ))
                            })
                            .when(is_in_conflict && !is_in_active_conflict, |d| {
                                d.bg(with_alpha(
                                    theme.colors.accent,
                                    if theme.is_dark { 0.03 } else { 0.02 },
                                ))
                            })
                            .child(base)
                            .child(
                                div()
                                    .w(handle_w)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border)),
                            )
                            .child(ours)
                            .child(
                                div()
                                    .w(handle_w)
                                    .h_full()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border)),
                            )
                            .child(theirs)
                            .into_any_element(),
                    );
                }
            }
        }
        elements
    }

    pub(in super::super) fn render_conflict_compare_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        match this.diff_view {
            DiffViewMode::Split => range
                .map(|row_ix| this.render_conflict_compare_split_row(row_ix, cx))
                .collect(),
            DiffViewMode::Inline => range
                .map(|ix| this.render_conflict_compare_inline_row(ix, cx))
                .collect(),
        }
    }

    pub(in super::super) fn render_conflict_resolver_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        match this.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => range
                .map(|visible_row_ix| {
                    let Some(&row_ix) = this
                        .conflict_resolver
                        .diff_visible_row_indices
                        .get(visible_row_ix)
                    else {
                        return div()
                            .id(("conflict_diff_split_visible_oob", visible_row_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(this.theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    this.render_conflict_resolver_split_row(visible_row_ix, row_ix, cx)
                })
                .collect(),
            ConflictDiffMode::Inline => range
                .map(|visible_ix| {
                    let Some(&ix) = this
                        .conflict_resolver
                        .inline_visible_row_indices
                        .get(visible_ix)
                    else {
                        return div()
                            .id(("conflict_diff_inline_visible_oob", visible_ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(this.theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    };
                    this.render_conflict_resolver_inline_row(visible_ix, ix, cx)
                })
                .collect(),
        }
    }

    fn render_conflict_compare_split_row(
        &mut self,
        row_ix: usize,
        _cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let show_ws = self.show_whitespace;
        let Some(row) = self.conflict_resolver.diff_rows.get(row_ix) else {
            return div()
                .id(("conflict_compare_split_oob", row_ix))
                .h(px(20.0))
                .px_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("")
                .into_any_element();
        };

        let left_text: SharedString = row.old.clone().unwrap_or_default().into();
        let right_text: SharedString = row.new.clone().unwrap_or_default().into();

        let word_hl = self
            .conflict_resolver
            .diff_word_highlights_split
            .get(row_ix)
            .and_then(|o| o.as_ref());
        let old_word_ranges = word_hl.map(|(o, _)| o.as_slice()).unwrap_or(&[]);
        let new_word_ranges = word_hl.map(|(_, n)| n.as_slice()).unwrap_or(&[]);

        let syntax_lang = self
            .conflict_resolver
            .path
            .as_ref()
            .and_then(|p| diff_syntax_language_for_path(&p.to_string_lossy()));

        let query = if self.diff_search_active {
            self.diff_search_query.clone()
        } else {
            SharedString::default()
        };
        let query = query.as_ref().trim();
        let should_style = !query.is_empty()
            || !old_word_ranges.is_empty()
            || !new_word_ranges.is_empty()
            || syntax_lang.is_some();
        if should_style {
            if let Some(text) = row.old.as_deref() {
                self.conflict_diff_segments_cache_split
                    .entry((row_ix, ConflictPickSide::Ours))
                    .or_insert_with(|| {
                        build_cached_diff_styled_text(
                            theme,
                            text,
                            old_word_ranges,
                            query,
                            syntax_lang,
                            DiffSyntaxMode::Auto,
                            None,
                        )
                    });
            }
            if let Some(text) = row.new.as_deref() {
                self.conflict_diff_segments_cache_split
                    .entry((row_ix, ConflictPickSide::Theirs))
                    .or_insert_with(|| {
                        build_cached_diff_styled_text(
                            theme,
                            text,
                            new_word_ranges,
                            query,
                            syntax_lang,
                            DiffSyntaxMode::Auto,
                            None,
                        )
                    });
            }
        }
        let left_styled = should_style
            .then(|| {
                self.conflict_diff_segments_cache_split
                    .get(&(row_ix, ConflictPickSide::Ours))
            })
            .flatten();
        let right_styled = should_style
            .then(|| {
                self.conflict_diff_segments_cache_split
                    .get(&(row_ix, ConflictPickSide::Theirs))
            })
            .flatten();

        let left_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Ours);
        let right_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Theirs);

        let [left_col_w, right_col_w] = self.conflict_diff_split_col_widths;

        let left = div()
            .id(("conflict_compare_split_ours", row_ix))
            .w(left_col_w)
            .min_w(px(0.0))
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .bg(left_bg)
            .text_color(if row.old.is_some() {
                theme.colors.text
            } else {
                theme.colors.text_muted
            })
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.old_line)),
            )
            .child(conflict_diff_text_cell(
                left_text.clone(),
                left_styled,
                show_ws,
            ));

        let right = div()
            .id(("conflict_compare_split_theirs", row_ix))
            .w(right_col_w)
            .flex_grow()
            .min_w(px(0.0))
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .bg(right_bg)
            .text_color(if row.new.is_some() {
                theme.colors.text
            } else {
                theme.colors.text_muted
            })
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.new_line)),
            )
            .child(conflict_diff_text_cell(
                right_text.clone(),
                right_styled,
                show_ws,
            ));

        let handle_w = px(PANE_RESIZE_HANDLE_PX);
        div()
            .id(("conflict_compare_split_row", row_ix))
            .w_full()
            .flex()
            .child(left)
            .child(
                div()
                    .w(handle_w)
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border)),
            )
            .child(right)
            .into_any_element()
    }

    fn render_conflict_compare_inline_row(
        &mut self,
        ix: usize,
        _cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let show_ws = self.show_whitespace;
        let Some(row) = self.conflict_resolver.inline_rows.get(ix) else {
            return div()
                .id(("conflict_compare_inline_oob", ix))
                .h(px(20.0))
                .px_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("")
                .into_any_element();
        };

        let syntax_lang = self
            .conflict_resolver
            .path
            .as_ref()
            .and_then(|p| diff_syntax_language_for_path(&p.to_string_lossy()));

        let query = if self.diff_search_active {
            self.diff_search_query.clone()
        } else {
            SharedString::default()
        };
        let query = query.as_ref().trim();
        let should_style = !query.is_empty() || syntax_lang.is_some();
        if should_style && !row.content.is_empty() {
            self.conflict_diff_segments_cache_inline
                .entry(ix)
                .or_insert_with(|| {
                    build_cached_diff_styled_text(
                        theme,
                        row.content.as_str(),
                        &[],
                        query,
                        syntax_lang,
                        DiffSyntaxMode::Auto,
                        None,
                    )
                });
        }
        let styled = should_style
            .then(|| self.conflict_diff_segments_cache_inline.get(&ix))
            .flatten();

        let bg = inline_row_bg(theme, row.kind, row.side);
        let prefix = match row.kind {
            gitgpui_core::domain::DiffLineKind::Add => "+",
            gitgpui_core::domain::DiffLineKind::Remove => "-",
            gitgpui_core::domain::DiffLineKind::Context => " ",
            gitgpui_core::domain::DiffLineKind::Header => " ",
            gitgpui_core::domain::DiffLineKind::Hunk => " ",
        };

        div()
            .id(("conflict_compare_inline", ix))
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .bg(bg)
            .text_color(theme.colors.text)
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.old_line)),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.new_line)),
            )
            .child(
                div()
                    .w(px(12.0))
                    .text_color(theme.colors.text_muted)
                    .child(prefix),
            )
            .child(conflict_diff_text_cell(
                row.content.clone().into(),
                styled,
                show_ws,
            ))
            .into_any_element()
    }

    fn render_conflict_resolver_split_row(
        &mut self,
        _visible_row_ix: usize,
        row_ix: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let show_ws = self.show_whitespace;
        let Some(row) = self.conflict_resolver.diff_rows.get(row_ix) else {
            return div()
                .id(("conflict_diff_split_oob", row_ix))
                .h(px(20.0))
                .px_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("")
                .into_any_element();
        };

        let left_text: SharedString = row.old.clone().unwrap_or_default().into();
        let right_text: SharedString = row.new.clone().unwrap_or_default().into();

        let word_hl = self
            .conflict_resolver
            .diff_word_highlights_split
            .get(row_ix)
            .and_then(|o| o.as_ref());
        let old_word_ranges = word_hl.map(|(o, _)| o.as_slice()).unwrap_or(&[]);
        let new_word_ranges = word_hl.map(|(_, n)| n.as_slice()).unwrap_or(&[]);

        let syntax_lang = self
            .conflict_resolver
            .path
            .as_ref()
            .and_then(|p| diff_syntax_language_for_path(&p.to_string_lossy()));

        let query = if self.diff_search_active {
            self.diff_search_query.clone()
        } else {
            SharedString::default()
        };
        let query = query.as_ref().trim();
        let should_style = !query.is_empty()
            || !old_word_ranges.is_empty()
            || !new_word_ranges.is_empty()
            || syntax_lang.is_some();
        if should_style {
            if let Some(text) = row.old.as_deref() {
                self.conflict_diff_segments_cache_split
                    .entry((row_ix, ConflictPickSide::Ours))
                    .or_insert_with(|| {
                        build_cached_diff_styled_text(
                            theme,
                            text,
                            old_word_ranges,
                            query,
                            syntax_lang,
                            DiffSyntaxMode::Auto,
                            None,
                        )
                    });
            }
            if let Some(text) = row.new.as_deref() {
                self.conflict_diff_segments_cache_split
                    .entry((row_ix, ConflictPickSide::Theirs))
                    .or_insert_with(|| {
                        build_cached_diff_styled_text(
                            theme,
                            text,
                            new_word_ranges,
                            query,
                            syntax_lang,
                            DiffSyntaxMode::Auto,
                            None,
                        )
                    });
            }
        }
        let left_styled = should_style
            .then(|| {
                self.conflict_diff_segments_cache_split
                    .get(&(row_ix, ConflictPickSide::Ours))
            })
            .flatten();
        let right_styled = should_style
            .then(|| {
                self.conflict_diff_segments_cache_split
                    .get(&(row_ix, ConflictPickSide::Theirs))
            })
            .flatten();

        let left_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Ours);
        let right_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Theirs);

        // Check dedupe: whether each side's line is already in the resolved output.
        let left_in_output = row.old.as_ref().map_or(false, |text| {
            conflict_resolver::is_source_line_in_output(
                &self.conflict_resolver.resolved_output_line_sources_index,
                ConflictResolverViewMode::TwoWayDiff,
                conflict_resolver::ResolvedLineSource::A,
                row.old_line.unwrap_or(0),
                text,
            )
        });
        let right_in_output = row.new.as_ref().map_or(false, |text| {
            conflict_resolver::is_source_line_in_output(
                &self.conflict_resolver.resolved_output_line_sources_index,
                ConflictResolverViewMode::TwoWayDiff,
                conflict_resolver::ResolvedLineSource::B,
                row.new_line.unwrap_or(0),
                text,
            )
        });

        let left_click = cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.conflict_resolver_append_split_line_to_output(row_ix, ConflictPickSide::Ours, cx);
        });
        let right_click = cx.listener(move |this, _e: &ClickEvent, _w, cx| {
            this.conflict_resolver_append_split_line_to_output(
                row_ix,
                ConflictPickSide::Theirs,
                cx,
            );
        });

        let [left_col_w, right_col_w] = self.conflict_diff_split_col_widths;

        // Plus icon: shown on hover when line is pickable and not already in output.
        let left_plus = row.old.is_some() && !left_in_output;
        let right_plus = row.new.is_some() && !right_in_output;

        let mut left = div()
            .id(("conflict_diff_split_ours", row_ix))
            .w(left_col_w)
            .min_w(px(0.0))
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .bg(left_bg)
            .text_color(theme.colors.text)
            .whitespace_nowrap()
            .child(
                // Plus icon gutter (visible on group hover when pickable).
                div()
                    .w(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(left_plus, |d| {
                        d.child(
                            div()
                                .text_color(theme.colors.success)
                                .text_xs()
                                .invisible()
                                .group_hover("split_left_row", |s| s.visible())
                                .child("+"),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.old_line)),
            )
            .child(conflict_diff_text_cell(
                left_text.clone(),
                left_styled,
                show_ws,
            ));
        if row.old.is_some() {
            left = left
                .group("split_left_row")
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| {
                    s.bg(with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.14 } else { 0.10 },
                    ))
                })
                .active(move |s| {
                    s.bg(with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.20 } else { 0.16 },
                    ))
                })
                .on_click(left_click);
        } else {
            left = left.text_color(theme.colors.text_muted);
        }

        let mut right = div()
            .id(("conflict_diff_split_theirs", row_ix))
            .w(right_col_w)
            .flex_grow()
            .min_w(px(0.0))
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .bg(right_bg)
            .text_color(theme.colors.text)
            .whitespace_nowrap()
            .child(
                // Plus icon gutter (visible on group hover when pickable).
                div()
                    .w(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(right_plus, |d| {
                        d.child(
                            div()
                                .text_color(theme.colors.success)
                                .text_xs()
                                .invisible()
                                .group_hover("split_right_row", |s| s.visible())
                                .child("+"),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.new_line)),
            )
            .child(conflict_diff_text_cell(
                right_text.clone(),
                right_styled,
                show_ws,
            ));
        if row.new.is_some() {
            right = right
                .group("split_right_row")
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| {
                    s.bg(with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.14 } else { 0.10 },
                    ))
                })
                .active(move |s| {
                    s.bg(with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.20 } else { 0.16 },
                    ))
                })
                .on_click(right_click);
        } else {
            right = right.text_color(theme.colors.text_muted);
        }

        let handle_w = px(PANE_RESIZE_HANDLE_PX);
        div()
            .id(("conflict_diff_split_row", row_ix))
            .w_full()
            .flex()
            .child(left)
            .child(
                div()
                    .w(handle_w)
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(div().w(px(1.0)).h_full().bg(theme.colors.border)),
            )
            .child(right)
            .into_any_element()
    }

    fn render_conflict_resolver_inline_row(
        &mut self,
        _visible_ix: usize,
        ix: usize,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let show_ws = self.show_whitespace;
        let Some(row) = self.conflict_resolver.inline_rows.get(ix) else {
            return div()
                .id(("conflict_diff_inline_oob", ix))
                .h(px(20.0))
                .px_2()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child("")
                .into_any_element();
        };

        let syntax_lang = self
            .conflict_resolver
            .path
            .as_ref()
            .and_then(|p| diff_syntax_language_for_path(&p.to_string_lossy()));

        let query = if self.diff_search_active {
            self.diff_search_query.clone()
        } else {
            SharedString::default()
        };
        let query = query.as_ref().trim();
        let should_style = !query.is_empty() || syntax_lang.is_some();
        if should_style && !row.content.is_empty() {
            self.conflict_diff_segments_cache_inline
                .entry(ix)
                .or_insert_with(|| {
                    build_cached_diff_styled_text(
                        theme,
                        row.content.as_str(),
                        &[],
                        query,
                        syntax_lang,
                        DiffSyntaxMode::Auto,
                        None,
                    )
                });
        }
        let styled = should_style
            .then(|| self.conflict_diff_segments_cache_inline.get(&ix))
            .flatten();

        let bg = inline_row_bg(theme, row.kind, row.side);
        let prefix = match row.kind {
            gitgpui_core::domain::DiffLineKind::Add => "+",
            gitgpui_core::domain::DiffLineKind::Remove => "-",
            gitgpui_core::domain::DiffLineKind::Context => " ",
            gitgpui_core::domain::DiffLineKind::Header => " ",
            gitgpui_core::domain::DiffLineKind::Hunk => " ",
        };

        // Dedupe check for plus icon visibility.
        let inline_source = match row.side {
            ConflictPickSide::Ours => conflict_resolver::ResolvedLineSource::A,
            ConflictPickSide::Theirs => conflict_resolver::ResolvedLineSource::B,
        };
        let line_no = row.new_line.or(row.old_line).unwrap_or(0);
        let in_output = !row.content.is_empty()
            && conflict_resolver::is_source_line_in_output(
                &self.conflict_resolver.resolved_output_line_sources_index,
                ConflictResolverViewMode::TwoWayDiff,
                inline_source,
                line_no,
                &row.content,
            );
        let show_plus = !row.content.is_empty() && !in_output;

        let mut base = div()
            .id(("conflict_diff_inline", ix))
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .text_xs()
            .bg(bg)
            .text_color(theme.colors.text)
            .whitespace_nowrap()
            .child(
                // Plus icon gutter (visible on group hover when pickable).
                div()
                    .w(px(16.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(show_plus, |d| {
                        d.child(
                            div()
                                .text_color(theme.colors.success)
                                .text_xs()
                                .invisible()
                                .group_hover("inline_row", |s| s.visible())
                                .child("+"),
                        )
                    }),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.old_line)),
            )
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.new_line)),
            )
            .child(
                div()
                    .w(px(12.0))
                    .text_color(theme.colors.text_muted)
                    .child(prefix),
            )
            .child(conflict_diff_text_cell(
                row.content.clone().into(),
                styled,
                show_ws,
            ));

        if !row.content.is_empty() {
            base = base
                .group("inline_row")
                .cursor(CursorStyle::PointingHand)
                .hover(move |s| {
                    s.bg(with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.14 } else { 0.10 },
                    ))
                })
                .active(move |s| {
                    s.bg(with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.20 } else { 0.16 },
                    ))
                })
                .on_click(cx.listener(move |this, _e: &ClickEvent, _w, cx| {
                    this.conflict_resolver_append_inline_line_to_output(ix, cx);
                }));
        }

        base.into_any_element()
    }
}

fn conflict_diff_text_cell(
    text: SharedString,
    styled: Option<&CachedDiffStyledText>,
    show_whitespace: bool,
) -> AnyElement {
    let Some(styled) = styled else {
        let display = if show_whitespace {
            whitespace_visible_text(text.as_ref())
        } else {
            text
        };
        return div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(display)
            .into_any_element();
    };

    if styled.highlights.is_empty() {
        let display = if show_whitespace {
            whitespace_visible_text(styled.text.as_ref())
        } else {
            styled.text.clone()
        };
        return div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(display)
            .into_any_element();
    }

    // When highlights exist, don't transform (would break byte ranges).
    div()
        .flex_1()
        .min_w(px(0.0))
        .overflow_hidden()
        .child(
            gpui::StyledText::new(styled.text.clone())
                .with_highlights(styled.highlights.iter().cloned()),
        )
        .into_any_element()
}

fn whitespace_visible_text(text: &str) -> SharedString {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            ' ' => out.push('\u{00B7}'),  // middle dot
            '\t' => out.push('\u{2192}'), // rightwards arrow
            _ => out.push(ch),
        }
    }
    out.into()
}

fn split_cell_bg(
    theme: AppTheme,
    kind: gitgpui_core::file_diff::FileDiffRowKind,
    side: ConflictPickSide,
) -> gpui::Rgba {
    match (kind, side) {
        (gitgpui_core::file_diff::FileDiffRowKind::Add, ConflictPickSide::Theirs)
        | (gitgpui_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Theirs) => {
            with_alpha(
                theme.colors.success,
                if theme.is_dark { 0.10 } else { 0.08 },
            )
        }
        (gitgpui_core::file_diff::FileDiffRowKind::Remove, ConflictPickSide::Ours)
        | (gitgpui_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Ours) => {
            with_alpha(theme.colors.danger, if theme.is_dark { 0.10 } else { 0.08 })
        }
        _ => with_alpha(theme.colors.surface_bg_elevated, 0.0),
    }
}

fn inline_row_bg(
    theme: AppTheme,
    kind: gitgpui_core::domain::DiffLineKind,
    side: ConflictPickSide,
) -> gpui::Rgba {
    match (kind, side) {
        (gitgpui_core::domain::DiffLineKind::Add, _) => with_alpha(
            theme.colors.success,
            if theme.is_dark { 0.10 } else { 0.08 },
        ),
        (gitgpui_core::domain::DiffLineKind::Remove, _) => {
            with_alpha(theme.colors.danger, if theme.is_dark { 0.10 } else { 0.08 })
        }
        _ => with_alpha(theme.colors.surface_bg_elevated, 0.0),
    }
}
