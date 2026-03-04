use super::super::conflict_resolver;
use super::conflict_canvas::{
    self, ConflictChunkContext, ThreeWayCanvasColumn, ThreeWayChunkContext,
};
use super::diff_text::*;
use super::*;

fn conflict_syntax_mode_for_total_rows(total_rows: usize) -> DiffSyntaxMode {
    if total_rows <= MAX_LINES_FOR_SYNTAX_HIGHLIGHTING {
        DiffSyntaxMode::Auto
    } else {
        DiffSyntaxMode::HeuristicOnly
    }
}

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
        let syntax_lang = this.conflict_resolver.conflict_syntax_language;
        let syntax_mode = conflict_syntax_mode_for_total_rows(this.conflict_resolver.three_way_len);

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
                    syntax_mode,
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
                    let mut collapsed = div()
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
                        .cursor(CursorStyle::PointingHand);
                    let has_base = this
                        .conflict_resolver
                        .conflict_has_base
                        .get(range_ix)
                        .copied()
                        .unwrap_or(false);
                    let selected_choices =
                        this.conflict_resolver_selected_choices_for_conflict_ix(range_ix);
                    let context_menu_invoker: SharedString =
                        format!("resolver_three_way_collapsed_chunk_menu_{}", range_ix).into();
                    collapsed = collapsed.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_conflict_resolver_chunk_context_menu(
                                context_menu_invoker.clone(),
                                range_ix,
                                has_base,
                                true,
                                selected_choices.clone(),
                                None,
                                e.position,
                                window,
                                cx,
                            );
                        }),
                    );
                    elements.push(collapsed.into_any_element());
                }
                conflict_resolver::ThreeWayVisibleItem::Line(ix) => {
                    let base_line = this.conflict_resolver.three_way_base_lines.get(ix);
                    let ours_line = this.conflict_resolver.three_way_ours_lines.get(ix);
                    let theirs_line = this.conflict_resolver.three_way_theirs_lines.get(ix);
                    let base_range_ix = this
                        .conflict_resolver
                        .three_way_base_line_conflict_map
                        .get(ix)
                        .copied()
                        .flatten()
                        .filter(|_| base_line.is_some());
                    let ours_range_ix = this
                        .conflict_resolver
                        .three_way_ours_line_conflict_map
                        .get(ix)
                        .copied()
                        .flatten()
                        .filter(|_| ours_line.is_some());
                    let theirs_range_ix = this
                        .conflict_resolver
                        .three_way_theirs_line_conflict_map
                        .get(ix)
                        .copied()
                        .flatten()
                        .filter(|_| theirs_line.is_some());
                    let is_in_conflict = base_range_ix.is_some()
                        || ours_range_ix.is_some()
                        || theirs_range_ix.is_some();

                    // Which column is chosen for this conflict?
                    let base_choice_for_row =
                        base_range_ix.and_then(|ri| conflict_choices.get(ri).copied());
                    let ours_choice_for_row =
                        ours_range_ix.and_then(|ri| conflict_choices.get(ri).copied());
                    let theirs_choice_for_row =
                        theirs_range_ix.and_then(|ri| conflict_choices.get(ri).copied());
                    let base_is_chosen =
                        base_choice_for_row == Some(conflict_resolver::ConflictChoice::Base);
                    let ours_is_chosen = matches!(
                        ours_choice_for_row,
                        Some(conflict_resolver::ConflictChoice::Ours)
                            | Some(conflict_resolver::ConflictChoice::Both)
                    );
                    let theirs_is_chosen = matches!(
                        theirs_choice_for_row,
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

                    let base_bg = if is_in_conflict && base_line.is_some() {
                        with_alpha(
                            theme.colors.warning,
                            if theme.is_dark { 0.10 } else { 0.08 },
                        )
                    } else {
                        with_alpha(theme.colors.surface_bg_elevated, 0.0)
                    };
                    let ours_bg = if is_in_conflict && ours_line.is_some() {
                        with_alpha(
                            theme.colors.success,
                            if theme.is_dark { 0.10 } else { 0.08 },
                        )
                    } else {
                        with_alpha(theme.colors.surface_bg_elevated, 0.0)
                    };
                    let theirs_bg = if is_in_conflict && theirs_line.is_some() {
                        with_alpha(theme.colors.accent, if theme.is_dark { 0.14 } else { 0.10 })
                    } else {
                        with_alpha(theme.colors.surface_bg_elevated, 0.0)
                    };
                    let base_fg = if base_line.is_some() {
                        theme.colors.text
                    } else {
                        theme.colors.text_muted
                    };
                    let ours_fg = if ours_line.is_some() {
                        theme.colors.text
                    } else {
                        theme.colors.text_muted
                    };
                    let theirs_fg = if theirs_line.is_some() {
                        theme.colors.text
                    } else {
                        theme.colors.text_muted
                    };

                    let base_line_no = line_number_string(
                        base_line
                            .is_some()
                            .then(|| u32::try_from(ix + 1).ok())
                            .flatten(),
                    );
                    let ours_line_no = line_number_string(
                        ours_line
                            .is_some()
                            .then(|| u32::try_from(ix + 1).ok())
                            .flatten(),
                    );
                    let theirs_line_no = line_number_string(
                        theirs_line
                            .is_some()
                            .then(|| u32::try_from(ix + 1).ok())
                            .flatten(),
                    );

                    if this.conflict_canvas_rows_enabled {
                        let base_chunk_context =
                            base_range_ix.map(|conflict_ix| ConflictChunkContext {
                                conflict_ix,
                                has_base: this
                                    .conflict_resolver
                                    .conflict_has_base
                                    .get(conflict_ix)
                                    .copied()
                                    .unwrap_or(false),
                                selected_choices: this
                                    .conflict_resolver_selected_choices_for_conflict_ix(
                                        conflict_ix,
                                    ),
                            });
                        let ours_chunk_context =
                            ours_range_ix.map(|conflict_ix| ConflictChunkContext {
                                conflict_ix,
                                has_base: this
                                    .conflict_resolver
                                    .conflict_has_base
                                    .get(conflict_ix)
                                    .copied()
                                    .unwrap_or(false),
                                selected_choices: this
                                    .conflict_resolver_selected_choices_for_conflict_ix(
                                        conflict_ix,
                                    ),
                            });
                        let theirs_chunk_context =
                            theirs_range_ix.map(|conflict_ix| ConflictChunkContext {
                                conflict_ix,
                                has_base: this
                                    .conflict_resolver
                                    .conflict_has_base
                                    .get(conflict_ix)
                                    .copied()
                                    .unwrap_or(false),
                                selected_choices: this
                                    .conflict_resolver_selected_choices_for_conflict_ix(
                                        conflict_ix,
                                    ),
                            });
                        let min_width =
                            col_a_w + col_b_w + col_c_w + px(PANE_RESIZE_HANDLE_PX) * 2.0;
                        elements.push(conflict_canvas::three_way_conflict_row_canvas(
                            theme,
                            cx.entity(),
                            vi,
                            ix,
                            min_width,
                            col_a_w,
                            col_b_w,
                            col_c_w,
                            ThreeWayCanvasColumn {
                                line_no: base_line_no,
                                bg: if base_is_chosen { chosen_bg } else { base_bg },
                                fg: base_fg,
                                text: base_line.cloned().unwrap_or_default(),
                            },
                            ThreeWayCanvasColumn {
                                line_no: ours_line_no,
                                bg: if ours_is_chosen { chosen_bg } else { ours_bg },
                                fg: ours_fg,
                                text: ours_line.cloned().unwrap_or_default(),
                            },
                            ThreeWayCanvasColumn {
                                line_no: theirs_line_no,
                                bg: if theirs_is_chosen {
                                    chosen_bg
                                } else {
                                    theirs_bg
                                },
                                fg: theirs_fg,
                                text: theirs_line.cloned().unwrap_or_default(),
                            },
                            base_styled,
                            ours_styled,
                            theirs_styled,
                            show_ws,
                            ThreeWayChunkContext {
                                base: base_chunk_context,
                                ours: ours_chunk_context,
                                theirs: theirs_chunk_context,
                            },
                        ));
                        continue;
                    }

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
                        .text_color(base_fg)
                        .whitespace_nowrap()
                        .bg(base_bg)
                        .when(base_is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div()
                                .w(px(38.0))
                                .text_color(theme.colors.text_muted)
                                .child(base_line_no),
                        )
                        .child(conflict_diff_text_cell(
                            base_line.cloned().unwrap_or_default(),
                            base_styled,
                            show_ws,
                        ));

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
                        .text_color(ours_fg)
                        .whitespace_nowrap()
                        .bg(ours_bg)
                        .when(ours_is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div()
                                .w(px(38.0))
                                .text_color(theme.colors.text_muted)
                                .child(ours_line_no),
                        )
                        .child(conflict_diff_text_cell(
                            ours_line.cloned().unwrap_or_default(),
                            ours_styled,
                            show_ws,
                        ));

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
                        .text_color(theirs_fg)
                        .whitespace_nowrap()
                        .bg(theirs_bg)
                        .when(theirs_is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div()
                                .w(px(38.0))
                                .text_color(theme.colors.text_muted)
                                .child(theirs_line_no),
                        )
                        .child(conflict_diff_text_cell(
                            theirs_line.cloned().unwrap_or_default(),
                            theirs_styled,
                            show_ws,
                        ));

                    if let Some(conflict_ix) = base_range_ix {
                        let has_base = this
                            .conflict_resolver
                            .conflict_has_base
                            .get(conflict_ix)
                            .copied()
                            .unwrap_or(false);
                        let selected_choices =
                            this.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
                        let context_menu_invoker: SharedString =
                            format!("resolver_three_way_base_chunk_menu_{}_{}", conflict_ix, ix)
                                .into();
                        base = base.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_conflict_resolver_chunk_context_menu(
                                    context_menu_invoker.clone(),
                                    conflict_ix,
                                    has_base,
                                    true,
                                    selected_choices.clone(),
                                    None,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        );
                    }
                    if let Some(conflict_ix) = ours_range_ix {
                        let has_base = this
                            .conflict_resolver
                            .conflict_has_base
                            .get(conflict_ix)
                            .copied()
                            .unwrap_or(false);
                        let selected_choices =
                            this.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
                        let context_menu_invoker: SharedString =
                            format!("resolver_three_way_ours_chunk_menu_{}_{}", conflict_ix, ix)
                                .into();
                        ours = ours.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_conflict_resolver_chunk_context_menu(
                                    context_menu_invoker.clone(),
                                    conflict_ix,
                                    has_base,
                                    true,
                                    selected_choices.clone(),
                                    None,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        );
                    }
                    if let Some(conflict_ix) = theirs_range_ix {
                        let has_base = this
                            .conflict_resolver
                            .conflict_has_base
                            .get(conflict_ix)
                            .copied()
                            .unwrap_or(false);
                        let selected_choices =
                            this.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
                        let context_menu_invoker: SharedString = format!(
                            "resolver_three_way_theirs_chunk_menu_{}_{}",
                            conflict_ix, ix
                        )
                        .into();
                        theirs = theirs.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_conflict_resolver_chunk_context_menu(
                                    context_menu_invoker.clone(),
                                    conflict_ix,
                                    has_base,
                                    true,
                                    selected_choices.clone(),
                                    None,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }),
                        );
                    }

                    let handle_w = px(PANE_RESIZE_HANDLE_PX);
                    let row = div()
                        .id(("conflict_three_way_row", ix))
                        .w_full()
                        .flex()
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
                        .child(theirs);

                    elements.push(row.into_any_element());
                }
            }
        }
        elements
    }

    pub(in super::super) fn render_conflict_resolved_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let theme = this.theme;

        range
            .map(|ix| {
                if this.conflict_resolved_preview_lines.get(ix).is_none() {
                    return div()
                        .id(("conflict_resolved_preview_oob", ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                }

                let source_meta = this.conflict_resolver.resolved_line_meta.get(ix);
                let source = source_meta
                    .map(|m| m.source)
                    .unwrap_or(conflict_resolver::ResolvedLineSource::Manual);
                let (_, badge_fg) = resolved_output_source_badge_colors(theme, source);
                let conflict_marker = this
                    .conflict_resolver
                    .resolved_output_conflict_markers
                    .get(ix)
                    .copied()
                    .flatten();
                let conflict_active = conflict_marker.is_some_and(|marker| {
                    marker.conflict_ix == this.conflict_resolver.active_conflict
                });
                let conflict_unresolved = conflict_marker.is_some_and(|marker| marker.unresolved);
                let marker_color = if conflict_unresolved {
                    with_alpha(theme.colors.danger, if theme.is_dark { 0.96 } else { 0.90 })
                } else if conflict_active {
                    with_alpha(theme.colors.accent, if theme.is_dark { 0.92 } else { 0.84 })
                } else {
                    with_alpha(
                        theme.colors.success,
                        if theme.is_dark { 0.82 } else { 0.72 },
                    )
                };
                let marker_lane = div()
                    .w(px(12.0))
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when_some(conflict_marker, |d, marker| {
                        d.child(
                            div()
                                .relative()
                                .w(px(2.0))
                                .h_full()
                                .bg(marker_color)
                                .when(marker.is_start, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(0.0))
                                            .left(px(-3.0))
                                            .w(px(8.0))
                                            .h(px(2.0))
                                            .bg(marker_color),
                                    )
                                })
                                .when(marker.is_end, |d| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .bottom(px(0.0))
                                            .left(px(-3.0))
                                            .w(px(8.0))
                                            .h(px(2.0))
                                            .bg(marker_color),
                                    )
                                }),
                        )
                    });

                let mut row = div()
                    .id(("conflict_resolved_preview_row", ix))
                    .relative()
                    .h(px(20.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .font_family("monospace")
                    .text_color(theme.colors.text)
                    .when(
                        source == conflict_resolver::ResolvedLineSource::Manual
                            && conflict_marker.is_none(),
                        |d| {
                            d.bg(with_alpha(
                                theme.colors.surface_bg_elevated,
                                if theme.is_dark { 0.18 } else { 0.12 },
                            ))
                        },
                    )
                    .child(marker_lane)
                    .child(
                        div()
                            .w(px(38.0))
                            .text_color(theme.colors.text_muted)
                            .child(line_number_string(u32::try_from(ix + 1).ok())),
                    )
                    .child(
                        div()
                            .w(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                div()
                                    .w(px(18.0))
                                    .h(px(14.0))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .font_weight(FontWeight::BOLD)
                                    .text_color(badge_fg)
                                    .child(source.badge_char().to_string()),
                            ),
                    );
                if let Some(marker) = conflict_marker {
                    let has_base = this
                        .conflict_resolver
                        .conflict_has_base
                        .get(marker.conflict_ix)
                        .copied()
                        .unwrap_or(false);
                    let is_three_way =
                        this.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay;
                    let selected_choices =
                        this.conflict_resolver_selected_choices_for_conflict_ix(marker.conflict_ix);
                    let context_menu_invoker: SharedString =
                        format!("resolver_output_chunk_menu_{}_{}", marker.conflict_ix, ix).into();
                    row = row.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_conflict_resolver_chunk_context_menu(
                                context_menu_invoker.clone(),
                                marker.conflict_ix,
                                has_base,
                                is_three_way,
                                selected_choices.clone(),
                                Some(ix),
                                e.position,
                                window,
                                cx,
                            );
                        }),
                    );
                }
                row.into_any_element()
            })
            .collect()
    }

    pub(in super::super) fn render_conflict_compare_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let query: SharedString = if this.diff_search_active {
            this.diff_search_query.clone()
        } else {
            SharedString::default()
        };
        let query = query.as_ref().trim().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str());
        let syntax_lang = this.conflict_resolver.conflict_syntax_language;
        match this.diff_view {
            DiffViewMode::Split => {
                let syntax_mode =
                    conflict_syntax_mode_for_total_rows(this.conflict_resolver.diff_rows.len());
                range
                    .map(|row_ix| {
                        this.render_conflict_compare_split_row(row_ix, syntax_lang, syntax_mode, cx)
                    })
                    .collect()
            }
            DiffViewMode::Inline => {
                let syntax_mode =
                    conflict_syntax_mode_for_total_rows(this.conflict_resolver.inline_rows.len());
                range
                    .map(|ix| {
                        this.render_conflict_compare_inline_row(ix, syntax_lang, syntax_mode, cx)
                    })
                    .collect()
            }
        }
    }

    pub(in super::super) fn render_conflict_resolver_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let query: SharedString = if this.diff_search_active {
            this.diff_search_query.clone()
        } else {
            SharedString::default()
        };
        let query = query.as_ref().trim().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str());
        let syntax_lang = this.conflict_resolver.conflict_syntax_language;
        match this.conflict_resolver.diff_mode {
            ConflictDiffMode::Split => {
                let syntax_mode =
                    conflict_syntax_mode_for_total_rows(this.conflict_resolver.diff_rows.len());
                range
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
                        let conflict_ix = this
                            .conflict_resolver
                            .diff_row_conflict_map
                            .get(row_ix)
                            .copied()
                            .flatten();
                        this.render_conflict_resolver_split_row(
                            visible_row_ix,
                            row_ix,
                            conflict_ix,
                            syntax_lang,
                            syntax_mode,
                            cx,
                        )
                    })
                    .collect()
            }
            ConflictDiffMode::Inline => {
                let syntax_mode =
                    conflict_syntax_mode_for_total_rows(this.conflict_resolver.inline_rows.len());
                range
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
                        let conflict_ix = this
                            .conflict_resolver
                            .inline_row_conflict_map
                            .get(ix)
                            .copied()
                            .flatten();
                        this.render_conflict_resolver_inline_row(
                            visible_ix,
                            ix,
                            conflict_ix,
                            syntax_lang,
                            syntax_mode,
                            cx,
                        )
                    })
                    .collect()
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict_split_row_styled(
        theme: AppTheme,
        stable_cache: &mut HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
        query_cache: &mut HashMap<(usize, ConflictPickSide), CachedDiffStyledText>,
        row_ix: usize,
        side: ConflictPickSide,
        text: Option<&str>,
        word_ranges: &[Range<usize>],
        query: &str,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
    ) -> Option<CachedDiffStyledText> {
        let text = text?;
        if text.is_empty() {
            return None;
        }

        let query = query.trim();
        let query_active = !query.is_empty();
        let base_has_style = !word_ranges.is_empty() || syntax_lang.is_some();
        let key = (row_ix, side);

        if base_has_style {
            stable_cache.entry(key).or_insert_with(|| {
                build_cached_diff_styled_text(
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_lang,
                    syntax_mode,
                    None,
                )
            });
        }

        if query_active {
            query_cache.entry(key).or_insert_with(|| {
                if let Some(base) = stable_cache.get(&key) {
                    build_cached_diff_query_overlay_styled_text(theme, base, query)
                } else {
                    build_cached_diff_styled_text(
                        theme,
                        text,
                        word_ranges,
                        query,
                        syntax_lang,
                        syntax_mode,
                        None,
                    )
                }
            });
            return query_cache.get(&key).cloned();
        }

        if base_has_style {
            stable_cache.get(&key).cloned()
        } else {
            None
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict_inline_row_styled(
        theme: AppTheme,
        stable_cache: &mut HashMap<usize, CachedDiffStyledText>,
        query_cache: &mut HashMap<usize, CachedDiffStyledText>,
        row_ix: usize,
        text: &str,
        query: &str,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
    ) -> Option<CachedDiffStyledText> {
        if text.is_empty() {
            return None;
        }

        let query = query.trim();
        let query_active = !query.is_empty();
        let base_has_style = syntax_lang.is_some();

        if base_has_style {
            stable_cache.entry(row_ix).or_insert_with(|| {
                build_cached_diff_styled_text(theme, text, &[], "", syntax_lang, syntax_mode, None)
            });
        }

        if query_active {
            query_cache.entry(row_ix).or_insert_with(|| {
                if let Some(base) = stable_cache.get(&row_ix) {
                    build_cached_diff_query_overlay_styled_text(theme, base, query)
                } else {
                    build_cached_diff_styled_text(
                        theme,
                        text,
                        &[],
                        query,
                        syntax_lang,
                        syntax_mode,
                        None,
                    )
                }
            });
            return query_cache.get(&row_ix).cloned();
        }

        if base_has_style {
            stable_cache.get(&row_ix).cloned()
        } else {
            None
        }
    }

    fn render_conflict_compare_split_row(
        &mut self,
        row_ix: usize,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
        cx: &mut gpui::Context<Self>,
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
        let query = self.conflict_diff_query_cache_query.as_ref();
        let left_styled = Self::conflict_split_row_styled(
            theme,
            &mut self.conflict_diff_segments_cache_split,
            &mut self.conflict_diff_query_segments_cache_split,
            row_ix,
            ConflictPickSide::Ours,
            row.old.as_deref(),
            old_word_ranges,
            query,
            syntax_lang,
            syntax_mode,
        );
        let right_styled = Self::conflict_split_row_styled(
            theme,
            &mut self.conflict_diff_segments_cache_split,
            &mut self.conflict_diff_query_segments_cache_split,
            row_ix,
            ConflictPickSide::Theirs,
            row.new.as_deref(),
            new_word_ranges,
            query,
            syntax_lang,
            syntax_mode,
        );

        let left_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Ours);
        let right_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Theirs);

        let [left_col_w, right_col_w] = self.conflict_diff_split_col_widths;
        let left_fg = if row.old.is_some() {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };
        let right_fg = if row.new.is_some() {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };

        if self.conflict_canvas_rows_enabled {
            let min_width = left_col_w + right_col_w + px(PANE_RESIZE_HANDLE_PX);
            return conflict_canvas::split_conflict_row_canvas(
                theme,
                cx.entity(),
                row_ix,
                row_ix,
                min_width,
                left_col_w,
                right_col_w,
                line_number_string(row.old_line),
                line_number_string(row.new_line),
                left_bg,
                right_bg,
                left_fg,
                right_fg,
                left_text,
                right_text,
                left_styled.as_ref(),
                right_styled.as_ref(),
                show_ws,
                None,
            );
        }

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
            .text_color(left_fg)
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.old_line)),
            )
            .child(conflict_diff_text_cell(
                left_text.clone(),
                left_styled.as_ref(),
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
            .text_color(right_fg)
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.new_line)),
            )
            .child(conflict_diff_text_cell(
                right_text.clone(),
                right_styled.as_ref(),
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
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
        cx: &mut gpui::Context<Self>,
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

        let query = self.conflict_diff_query_cache_query.as_ref();
        let styled = Self::conflict_inline_row_styled(
            theme,
            &mut self.conflict_diff_segments_cache_inline,
            &mut self.conflict_diff_query_segments_cache_inline,
            ix,
            row.content.as_str(),
            query,
            syntax_lang,
            syntax_mode,
        );

        let bg = inline_row_bg(theme, row.kind, row.side);
        let prefix: SharedString = match row.kind {
            gitgpui_core::domain::DiffLineKind::Add => "+",
            gitgpui_core::domain::DiffLineKind::Remove => "-",
            gitgpui_core::domain::DiffLineKind::Context => " ",
            gitgpui_core::domain::DiffLineKind::Header => " ",
            gitgpui_core::domain::DiffLineKind::Hunk => " ",
        }
        .into();

        if self.conflict_canvas_rows_enabled {
            return conflict_canvas::inline_conflict_row_canvas(
                theme,
                cx.entity(),
                ix,
                ix,
                px(0.0),
                line_number_string(row.old_line),
                line_number_string(row.new_line),
                prefix.clone(),
                bg,
                theme.colors.text,
                row.content.clone().into(),
                styled.as_ref(),
                show_ws,
                None,
            );
        }

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
                styled.as_ref(),
                show_ws,
            ))
            .into_any_element()
    }

    fn render_conflict_resolver_split_row(
        &mut self,
        visible_row_ix: usize,
        row_ix: usize,
        conflict_ix: Option<usize>,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
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
        let query = self.conflict_diff_query_cache_query.as_ref();
        let left_styled = Self::conflict_split_row_styled(
            theme,
            &mut self.conflict_diff_segments_cache_split,
            &mut self.conflict_diff_query_segments_cache_split,
            row_ix,
            ConflictPickSide::Ours,
            row.old.as_deref(),
            old_word_ranges,
            query,
            syntax_lang,
            syntax_mode,
        );
        let right_styled = Self::conflict_split_row_styled(
            theme,
            &mut self.conflict_diff_segments_cache_split,
            &mut self.conflict_diff_query_segments_cache_split,
            row_ix,
            ConflictPickSide::Theirs,
            row.new.as_deref(),
            new_word_ranges,
            query,
            syntax_lang,
            syntax_mode,
        );

        let left_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Ours);
        let right_bg = split_cell_bg(theme, row.kind, ConflictPickSide::Theirs);

        let [left_col_w, right_col_w] = self.conflict_diff_split_col_widths;
        let left_fg = if row.old.is_some() {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };
        let right_fg = if row.new.is_some() {
            theme.colors.text
        } else {
            theme.colors.text_muted
        };

        if self.conflict_canvas_rows_enabled {
            let chunk_context = conflict_ix.map(|conflict_ix| ConflictChunkContext {
                conflict_ix,
                has_base: self
                    .conflict_resolver
                    .conflict_has_base
                    .get(conflict_ix)
                    .copied()
                    .unwrap_or(false),
                selected_choices: self
                    .conflict_resolver_selected_choices_for_conflict_ix(conflict_ix),
            });
            let min_width = left_col_w + right_col_w + px(PANE_RESIZE_HANDLE_PX);
            return conflict_canvas::split_conflict_row_canvas(
                theme,
                cx.entity(),
                visible_row_ix,
                row_ix,
                min_width,
                left_col_w,
                right_col_w,
                line_number_string(row.old_line),
                line_number_string(row.new_line),
                left_bg,
                right_bg,
                left_fg,
                right_fg,
                left_text,
                right_text,
                left_styled.as_ref(),
                right_styled.as_ref(),
                show_ws,
                chunk_context,
            );
        }

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
            .text_color(left_fg)
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.old_line)),
            )
            .child(conflict_diff_text_cell(
                left_text.clone(),
                left_styled.as_ref(),
                show_ws,
            ));
        if let Some(conflict_ix) = conflict_ix {
            let has_base = self
                .conflict_resolver
                .conflict_has_base
                .get(conflict_ix)
                .copied()
                .unwrap_or(false);
            let selected_choices =
                self.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
            let context_menu_invoker: SharedString = format!(
                "resolver_two_way_split_ours_chunk_menu_{}_{}",
                conflict_ix, row_ix
            )
            .into();
            left = left.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_conflict_resolver_chunk_context_menu(
                        context_menu_invoker.clone(),
                        conflict_ix,
                        has_base,
                        false,
                        selected_choices.clone(),
                        None,
                        e.position,
                        window,
                        cx,
                    );
                }),
            );
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
            .text_color(right_fg)
            .whitespace_nowrap()
            .child(
                div()
                    .w(px(38.0))
                    .text_color(theme.colors.text_muted)
                    .child(line_number_string(row.new_line)),
            )
            .child(conflict_diff_text_cell(
                right_text.clone(),
                right_styled.as_ref(),
                show_ws,
            ));
        if let Some(conflict_ix) = conflict_ix {
            let has_base = self
                .conflict_resolver
                .conflict_has_base
                .get(conflict_ix)
                .copied()
                .unwrap_or(false);
            let selected_choices =
                self.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
            let context_menu_invoker: SharedString = format!(
                "resolver_two_way_split_theirs_chunk_menu_{}_{}",
                conflict_ix, row_ix
            )
            .into();
            right = right.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_conflict_resolver_chunk_context_menu(
                        context_menu_invoker.clone(),
                        conflict_ix,
                        has_base,
                        false,
                        selected_choices.clone(),
                        None,
                        e.position,
                        window,
                        cx,
                    );
                }),
            );
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
        visible_ix: usize,
        ix: usize,
        conflict_ix: Option<usize>,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
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

        let query = self.conflict_diff_query_cache_query.as_ref();
        let styled = Self::conflict_inline_row_styled(
            theme,
            &mut self.conflict_diff_segments_cache_inline,
            &mut self.conflict_diff_query_segments_cache_inline,
            ix,
            row.content.as_str(),
            query,
            syntax_lang,
            syntax_mode,
        );

        let bg = inline_row_bg(theme, row.kind, row.side);
        let prefix: SharedString = match row.kind {
            gitgpui_core::domain::DiffLineKind::Add => "+",
            gitgpui_core::domain::DiffLineKind::Remove => "-",
            gitgpui_core::domain::DiffLineKind::Context => " ",
            gitgpui_core::domain::DiffLineKind::Header => " ",
            gitgpui_core::domain::DiffLineKind::Hunk => " ",
        }
        .into();

        if self.conflict_canvas_rows_enabled {
            let chunk_context = conflict_ix.map(|conflict_ix| ConflictChunkContext {
                conflict_ix,
                has_base: self
                    .conflict_resolver
                    .conflict_has_base
                    .get(conflict_ix)
                    .copied()
                    .unwrap_or(false),
                selected_choices: self
                    .conflict_resolver_selected_choices_for_conflict_ix(conflict_ix),
            });
            return conflict_canvas::inline_conflict_row_canvas(
                theme,
                cx.entity(),
                visible_ix,
                ix,
                px(0.0),
                line_number_string(row.old_line),
                line_number_string(row.new_line),
                prefix.clone(),
                bg,
                theme.colors.text,
                row.content.clone().into(),
                styled.as_ref(),
                show_ws,
                chunk_context,
            );
        }

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
                styled.as_ref(),
                show_ws,
            ));
        if let Some(conflict_ix) = conflict_ix {
            let has_base = self
                .conflict_resolver
                .conflict_has_base
                .get(conflict_ix)
                .copied()
                .unwrap_or(false);
            let selected_choices =
                self.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
            let context_menu_invoker: SharedString =
                format!("resolver_two_way_inline_chunk_menu_{}_{}", conflict_ix, ix).into();
            base = base.on_mouse_down(
                MouseButton::Right,
                cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                    cx.stop_propagation();
                    this.open_conflict_resolver_chunk_context_menu(
                        context_menu_invoker.clone(),
                        conflict_ix,
                        has_base,
                        false,
                        selected_choices.clone(),
                        None,
                        e.position,
                        window,
                        cx,
                    );
                }),
            );
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

fn resolved_output_source_badge_colors(
    theme: AppTheme,
    source: conflict_resolver::ResolvedLineSource,
) -> (gpui::Rgba, gpui::Rgba) {
    match source {
        conflict_resolver::ResolvedLineSource::A => (
            with_alpha(theme.colors.accent, if theme.is_dark { 0.68 } else { 0.56 }),
            theme.colors.accent,
        ),
        conflict_resolver::ResolvedLineSource::B => (
            with_alpha(
                theme.colors.success,
                if theme.is_dark { 0.68 } else { 0.56 },
            ),
            theme.colors.success,
        ),
        conflict_resolver::ResolvedLineSource::C => (
            with_alpha(
                theme.colors.warning,
                if theme.is_dark { 0.68 } else { 0.56 },
            ),
            theme.colors.warning,
        ),
        conflict_resolver::ResolvedLineSource::Manual => (
            with_alpha(
                theme.colors.text_muted,
                if theme.is_dark { 0.48 } else { 0.42 },
            ),
            theme.colors.text_muted,
        ),
    }
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
        | (gitgpui_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Ours) => with_alpha(
            theme.colors.warning,
            if theme.is_dark { 0.10 } else { 0.08 },
        ),
        _ => with_alpha(theme.colors.surface_bg_elevated, 0.0),
    }
}

fn inline_row_bg(
    theme: AppTheme,
    kind: gitgpui_core::domain::DiffLineKind,
    side: ConflictPickSide,
) -> gpui::Rgba {
    match (kind, side) {
        (gitgpui_core::domain::DiffLineKind::Add, ConflictPickSide::Ours)
        | (gitgpui_core::domain::DiffLineKind::Remove, ConflictPickSide::Ours) => with_alpha(
            theme.colors.warning,
            if theme.is_dark { 0.10 } else { 0.08 },
        ),
        (gitgpui_core::domain::DiffLineKind::Add, ConflictPickSide::Theirs)
        | (gitgpui_core::domain::DiffLineKind::Remove, ConflictPickSide::Theirs) => with_alpha(
            theme.colors.success,
            if theme.is_dark { 0.10 } else { 0.08 },
        ),
        _ => with_alpha(theme.colors.surface_bg_elevated, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflict_syntax_mode_uses_auto_below_threshold() {
        assert_eq!(
            conflict_syntax_mode_for_total_rows(MAX_LINES_FOR_SYNTAX_HIGHLIGHTING),
            DiffSyntaxMode::Auto
        );
    }

    #[test]
    fn conflict_syntax_mode_downgrades_above_threshold() {
        assert_eq!(
            conflict_syntax_mode_for_total_rows(MAX_LINES_FOR_SYNTAX_HIGHLIGHTING + 1),
            DiffSyntaxMode::HeuristicOnly
        );
    }
}
