//! Rows of the three-way conflict columns (base, ours, theirs).

use super::{
    AnyElement, CONFLICT_ROW_ACCENT_BAR_WIDTH_PX, ConflictChunkContext, CursorStyle, DiffRenderer,
    DiffSearchMatchEmphasis, DiffSyntaxConfig, DiffSyntaxMode, MouseButton, MouseDownEvent,
    MouseMoveEvent, PreparedDiffSyntaxLine, Range, SharedString, ThreeWayColumn, ViewPerfSpan,
    Window, build_cached_diff_styled_text_for_prepared_document_line_nonblocking,
    build_conflict_cached_diff_styled_text, conflict_canvas, conflict_diff_line_number_cell,
    conflict_diff_text_cell, conflict_display_text, conflict_input_row_min_width,
    conflict_resolver, conflict_scaled_px, div, line_number_string, perf,
    three_way_input_row_menu_targets, with_alpha,
};
use crate::kit::click::PointerClickExt;
use gpui::prelude::*;

impl DiffRenderer {
    pub(super) fn render_conflict_three_way_column_rows(
        this: &mut Self,
        range: Range<usize>,
        column: ThreeWayColumn,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let tab_width = this.display_tab_width;

        let _perf_scope = perf::span(ViewPerfSpan::RenderThreeWayRows);
        let theme = this.theme;
        let ui_scale_percent = crate::ui_scale::current(cx).percent;
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let show_ws = this.reveal_whitespace_chars;
        let query = this.diff_search_query_or_empty().as_ref().to_string();
        let query_options = this.diff_search_options_or_default();
        this.sync_conflict_diff_query_overlay_caches(query.as_str(), query_options);
        let query_matcher = this.diff_search_query_matcher_shared();
        // The current match wears a different wash, and it moves as the search
        // cursor steps, so its row is rebuilt every frame instead of cached.
        let current_match_row = this.diff_search_current_match_row();
        // A three-way conflict column marks changed words, so it takes the
        // "modified" diff palette -- the same amber every bundled theme also
        // uses for status.warning, but themeable as the diff token it is.
        let word_hl_kind = Some(crate::theme::DiffColorKind::Modified);
        let syntax_lang = this.conflict_row_syntax_language();
        let prepared_docs = &this.conflict_three_way_prepared_syntax_documents;

        let prepared_doc = match column {
            ThreeWayColumn::Base => prepared_docs.base,
            ThreeWayColumn::Ours => prepared_docs.ours,
            ThreeWayColumn::Theirs => prepared_docs.theirs,
        };
        let highlights = match column {
            ThreeWayColumn::Base => &this.conflict_resolver.three_way_word_highlights.base,
            ThreeWayColumn::Ours => &this.conflict_resolver.three_way_word_highlights.ours,
            ThreeWayColumn::Theirs => &this.conflict_resolver.three_way_word_highlights.theirs,
        };

        // Pre-build styled text cache entries for visible lines in this column.
        let mut needs_chunk_poll = false;
        for vi in range.clone() {
            let Some(conflict_resolver::ThreeWayVisibleItem::Line(row)) =
                this.conflict_resolver.three_way_visible_item(vi)
            else {
                continue;
            };
            // section 30 aligned row space: syntax documents, word highlights, and
            // the styled-text cache are all keyed by the side's own line.
            let Some(ix) = this
                .conflict_resolver
                .three_way_side_line_for_row(column, row)
            else {
                continue;
            };
            if this
                .conflict_three_way_segments_cache
                .contains_key(&(ix, column))
            {
                continue;
            }
            let word_ranges = highlights.get(&ix).map(|v| v.as_slice()).unwrap_or(&[]);
            let text = this
                .conflict_resolver
                .three_way_line_text(column, ix)
                .unwrap_or("");
            if text.is_empty() {
                continue;
            }
            if word_ranges.is_empty() && syntax_lang.is_none() {
                continue;
            }

            if let Some(document) = prepared_doc {
                let prepared_line = PreparedDiffSyntaxLine {
                    document: Some(document),
                    line_ix: ix,
                };
                let syntax_config = DiffSyntaxConfig {
                    language: syntax_lang,
                    mode: DiffSyntaxMode::Auto,
                };
                let result = build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
                    tab_width,
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_config,
                    word_hl_kind,
                    prepared_line,
                );
                let (styled, is_pending) = result.into_parts();
                if is_pending {
                    needs_chunk_poll = true;
                    // Don't cache — will re-render when chunk completes.
                } else {
                    this.conflict_three_way_segments_cache
                        .insert((ix, column), styled);
                }
            } else {
                let styled = build_conflict_cached_diff_styled_text(
                    tab_width,
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_lang,
                    DiffSyntaxMode::Auto,
                    word_hl_kind,
                );
                this.conflict_three_way_segments_cache
                    .insert((ix, column), styled);
            }
        }
        if needs_chunk_poll {
            this.ensure_prepared_syntax_chunk_poll(cx);
        }

        // Search wash, layered over whatever the pass above produced. It needs
        // its own pass because that one skips lines with no syntax and no word
        // highlights, and those lines still have to show their matches.
        if let Some(matcher) = query_matcher.as_deref() {
            // Built into a batch first: the builder reads all of `this`, so the
            // cache insert cannot happen while that borrow is live.
            let built: Vec<_> = range
                .clone()
                .filter(|vi| current_match_row != Some(*vi))
                .filter_map(|vi| {
                    let conflict_resolver::ThreeWayVisibleItem::Line(row) =
                        this.conflict_resolver.three_way_visible_item(vi)?
                    else {
                        return None;
                    };
                    let ix = this
                        .conflict_resolver
                        .three_way_side_line_for_row(column, row)?;
                    if this
                        .conflict_three_way_query_segments_cache
                        .contains_key(&(ix, column))
                    {
                        return None;
                    }
                    let (styled, cacheable) = Self::conflict_three_way_query_styled(
                        theme,
                        this,
                        column,
                        ix,
                        matcher,
                        DiffSearchMatchEmphasis::Other,
                        word_hl_kind,
                        syntax_lang,
                    )?;
                    cacheable.then_some(((ix, column), styled))
                })
                .collect();
            this.conflict_three_way_query_segments_cache.extend(built);
        }

        let chosen_bg = with_alpha(
            theme.colors.accent.foreground,
            if theme.is_dark { 0.16 } else { 0.12 },
        );
        let conflict_choices = this.conflict_resolver.conflict_choices.as_slice();
        // section 30 R11 (kdiff3 change colours): with a real base alignment, the
        // side columns tint only rows whose own line differs from base; the
        // base column keeps the marker-region tint as the pickable-conflict
        // locator. Without alignment all columns fall back to region tints.
        let per_side_change_rows = this.conflict_resolver.three_way_per_side_change_rows();

        let (canvas_id_prefix, div_id_prefix, chunk_menu_prefix, input_menu_prefix) = match column {
            ThreeWayColumn::Base => (
                "conflict_canvas_base",
                "conflict_three_way_col_base",
                "resolver_three_way_base_chunk_menu",
                "resolver_three_way_base_input_menu",
            ),
            ThreeWayColumn::Ours => (
                "conflict_canvas_ours",
                "conflict_three_way_col_ours",
                "resolver_three_way_ours_chunk_menu",
                "resolver_three_way_ours_input_menu",
            ),
            ThreeWayColumn::Theirs => (
                "conflict_canvas_theirs",
                "conflict_three_way_col_theirs",
                "resolver_three_way_theirs_chunk_menu",
                "resolver_three_way_theirs_input_menu",
            ),
        };
        let choice_enum = match column {
            ThreeWayColumn::Base => conflict_resolver::ConflictChoice::Base,
            ThreeWayColumn::Ours => conflict_resolver::ConflictChoice::Ours,
            ThreeWayColumn::Theirs => conflict_resolver::ConflictChoice::Theirs,
        };

        let mut elements = Vec::with_capacity(range.len());
        for vi in range {
            let Some(visible_item) = this.conflict_resolver.three_way_visible_item(vi) else {
                // Past-the-end rows exist only as bottom overscroll space.
                elements.push(
                    div()
                        .id((div_id_prefix, vi))
                        .w_full()
                        .h(theme.editor_row_height(ui_scale_percent))
                        .into_any_element(),
                );
                continue;
            };

            match visible_item {
                conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(range_ix) => {
                    let label: SharedString = if matches!(column, ThreeWayColumn::Base) {
                        let choice_label = conflict_choices
                            .get(range_ix)
                            .map(|c| match *c {
                                conflict_resolver::ConflictChoice::Base => "Base (A)",
                                conflict_resolver::ConflictChoice::Ours => "Local (B)",
                                conflict_resolver::ConflictChoice::Theirs => "Remote (C)",
                                conflict_resolver::ConflictChoice::Both => "Local+Remote (B+C)",
                                _ => "Ordered source selection",
                            })
                            .unwrap_or("?");
                        format!("  Resolved: picked {choice_label}").into()
                    } else {
                        "".into()
                    };
                    let has_base = this
                        .conflict_resolver
                        .conflict_has_base
                        .get(range_ix)
                        .copied()
                        .unwrap_or(false);
                    let selected_choices =
                        this.conflict_resolver_selected_choices_for_conflict_ix(range_ix);
                    let collapsed = div()
                        .id((div_id_prefix, vi))
                        .relative()
                        .w_full()
                        .h(theme.editor_row_height(ui_scale_percent))
                        .flex()
                        .items_center()
                        .bg(with_alpha(
                            theme.colors.status.success.foreground,
                            if theme.is_dark { 0.08 } else { 0.06 },
                        ))
                        .when(
                            Some(range_ix) == this.conflict_resolver.active_conflict,
                            |d| {
                                d.child(
                                    div()
                                        .absolute()
                                        .left_0()
                                        .top_0()
                                        .bottom_0()
                                        .w(conflict_scaled_px(
                                            CONFLICT_ROW_ACCENT_BAR_WIDTH_PX,
                                            ui_scale_percent,
                                        ))
                                        .bg(theme.colors.accent.foreground),
                                )
                            },
                        )
                        .px_2()
                        .text_size(theme.editor_font_size(ui_scale_percent))
                        .text_color(theme.colors.foreground.secondary)
                        .child(label)
                        .cursor(CursorStyle::PointingHand)
                        .on_pointer_click(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                // section 30: clicking a conflict block body selects it.
                                this.conflict_resolver_select_conflict(range_ix, cx);
                            }),
                        )
                        .on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                let invoker: SharedString = format!(
                                    "resolver_three_way_collapsed_chunk_menu_{}_{}",
                                    range_ix, vi
                                )
                                .into();
                                this.open_conflict_resolver_chunk_context_menu(
                                    invoker,
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
                conflict_resolver::ThreeWayVisibleItem::CollapsedContext {
                    source_line_start,
                    len,
                    fold_id,
                } => {
                    elements.push(Self::conflict_context_fold_row(
                        theme,
                        div_id_prefix,
                        vi,
                        source_line_start,
                        len,
                        fold_id,
                        false,
                        cx,
                    ));
                }
                conflict_resolver::ThreeWayVisibleItem::Line(ix) => {
                    // section 30 aligned row space: `ix` is the shared visual row;
                    // each column renders its own line (or padding) there.
                    let side_line = this
                        .conflict_resolver
                        .three_way_side_line_for_row(column, ix);
                    let line_text = side_line
                        .and_then(|l| this.conflict_resolver.three_way_line_text(column, l));
                    // Conflict ranges are aligned-row ranges shared by all
                    // columns; padding rows inside a conflict still highlight.
                    let range_ix = this
                        .conflict_resolver
                        .conflict_index_for_side_line(column, ix);
                    let is_in_conflict = range_ix.is_some();

                    let choice_for_row = range_ix.and_then(|ri| conflict_choices.get(ri).copied());
                    let is_chosen = match column {
                        ThreeWayColumn::Base => {
                            choice_for_row == Some(conflict_resolver::ConflictChoice::Base)
                        }
                        ThreeWayColumn::Ours => matches!(
                            choice_for_row,
                            Some(conflict_resolver::ConflictChoice::Ours)
                                | Some(conflict_resolver::ConflictChoice::Both)
                        ),
                        ThreeWayColumn::Theirs => matches!(
                            choice_for_row,
                            Some(conflict_resolver::ConflictChoice::Theirs)
                                | Some(conflict_resolver::ConflictChoice::Both)
                        ),
                    };

                    // Built here rather than read from the cache in two cases:
                    // the current match, whose wash follows the search cursor,
                    // and a line still waiting on a background parse, which the
                    // pass above refuses to cache so the degraded colouring does
                    // not outlive the parse.
                    let inline_styled = match (query_matcher.as_deref(), side_line) {
                        (Some(matcher), Some(l))
                            if current_match_row == Some(vi)
                                || !this
                                    .conflict_three_way_query_segments_cache
                                    .contains_key(&(l, column)) =>
                        {
                            let emphasis = if current_match_row == Some(vi) {
                                DiffSearchMatchEmphasis::Current
                            } else {
                                DiffSearchMatchEmphasis::Other
                            };
                            Self::conflict_three_way_query_styled(
                                theme,
                                this,
                                column,
                                l,
                                matcher,
                                emphasis,
                                word_hl_kind,
                                syntax_lang,
                            )
                            .map(|(styled, _cacheable)| styled)
                        }
                        _ => None,
                    };
                    let styled = match inline_styled.as_ref() {
                        Some(styled) => Some(styled),
                        None => side_line.and_then(|l| {
                            this.conflict_three_way_query_segments_cache
                                .get(&(l, column))
                                .or_else(|| {
                                    this.conflict_three_way_segments_cache.get(&(l, column))
                                })
                        }),
                    };

                    let bg = if per_side_change_rows && !matches!(column, ThreeWayColumn::Base) {
                        if this
                            .conflict_resolver
                            .three_way_row_differs_from_base(column, ix)
                        {
                            match column {
                                ThreeWayColumn::Ours => with_alpha(
                                    theme.colors.status.success.foreground,
                                    if theme.is_dark { 0.10 } else { 0.08 },
                                ),
                                _ => with_alpha(
                                    theme.colors.accent.foreground,
                                    if theme.is_dark { 0.14 } else { 0.10 },
                                ),
                            }
                        } else {
                            with_alpha(theme.colors.surface.raised, 0.0)
                        }
                    } else if is_in_conflict {
                        match column {
                            ThreeWayColumn::Base => with_alpha(
                                theme.colors.status.warning.foreground,
                                if theme.is_dark { 0.10 } else { 0.08 },
                            ),
                            ThreeWayColumn::Ours => with_alpha(
                                theme.colors.status.success.foreground,
                                if theme.is_dark { 0.10 } else { 0.08 },
                            ),
                            ThreeWayColumn::Theirs => with_alpha(
                                theme.colors.accent.foreground,
                                if theme.is_dark { 0.14 } else { 0.10 },
                            ),
                        }
                    } else {
                        with_alpha(theme.colors.surface.raised, 0.0)
                    };
                    let fg = if line_text.is_some() {
                        theme.colors.foreground.primary
                    } else {
                        theme.colors.foreground.secondary
                    };
                    // kdiff3 behavior: per-column line numbers from the
                    // side's own file; padding rows have none.
                    let line_no = line_number_string(
                        side_line
                            .filter(|_| line_text.is_some())
                            .and_then(|l| u32::try_from(l + 1).ok()),
                    );
                    let line_text = line_text.map(SharedString::new).unwrap_or_default();
                    let display_text = conflict_display_text(&line_text, styled, show_ws);
                    let show_line_numbers = this.mergetool_show_line_numbers;
                    let min_width = conflict_input_row_min_width(
                        theme,
                        window,
                        &display_text,
                        editor_font_family.as_str(),
                        show_line_numbers,
                        ui_scale_percent,
                    );

                    let semantic_nav_target =
                        this.conflict_resolver.nav_target_index_for_aligned_row(ix);
                    let is_active_conflict = this.conflict_resolver.conflict_is_active(range_ix)
                        || this
                            .conflict_resolver
                            .selected_nav_target_contains_aligned_row(ix);
                    // section 30 split: highlight rows in the drag selection; the
                    // begin/extend handlers only fire when split is available.
                    let row_selected = this.conflict_resolver.conflict_row_is_selected(ix);
                    let row_selection_enabled =
                        this.conflict_resolver.conflict_row_selection_enabled();
                    // kdiff3 manual diff help: only the three-way source columns
                    // sit in the shared aligned space that a pin is expressed in.
                    let alignment_mark =
                        this.conflict_resolver.manual_alignment_enabled().then(|| {
                            conflict_canvas::AlignmentMarkContext {
                                column,
                                side_line,
                                marked: side_line.is_some_and(|line| {
                                    this.conflict_resolver
                                        .alignment_line_is_selected(column, line)
                                }),
                            }
                        });
                    if this.conflict_canvas_rows_enabled {
                        let chunk_context = range_ix.map(|conflict_ix| ConflictChunkContext {
                            conflict_ix,
                            has_base: this
                                .conflict_resolver
                                .conflict_has_base
                                .get(conflict_ix)
                                .copied()
                                .unwrap_or(false),
                            selected_choices: this
                                .conflict_resolver_selected_choices_for_conflict_ix(conflict_ix),
                        });
                        elements.push(conflict_canvas::single_column_conflict_canvas(
                            tab_width,
                            theme,
                            cx.entity(),
                            canvas_id_prefix,
                            vi,
                            ix,
                            min_width,
                            show_line_numbers,
                            line_no,
                            if is_chosen { chosen_bg } else { bg },
                            fg,
                            line_text.clone(),
                            styled,
                            show_ws,
                            chunk_context,
                            chunk_menu_prefix,
                            true,
                            semantic_nav_target,
                            is_active_conflict,
                            row_selection_enabled.then_some(row_selected),
                            alignment_mark,
                            Some(column),
                            ui_scale_percent,
                        ));
                        continue;
                    }

                    let mut cell = div()
                        .id((div_id_prefix, ix))
                        .relative()
                        .w_full()
                        .min_w(min_width)
                        .h(theme.editor_row_height(ui_scale_percent))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_size(theme.editor_font_size(ui_scale_percent))
                        .text_color(fg)
                        .whitespace_nowrap()
                        .bg(bg)
                        .when(is_chosen, |d| d.bg(chosen_bg))
                        .when(is_active_conflict, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(conflict_scaled_px(
                                        CONFLICT_ROW_ACCENT_BAR_WIDTH_PX,
                                        ui_scale_percent,
                                    ))
                                    .bg(theme.colors.accent.foreground),
                            )
                        })
                        .when(row_selected, |d| {
                            d.child(div().absolute().inset_0().bg(with_alpha(
                                theme.colors.accent.foreground,
                                if theme.is_dark { 0.20 } else { 0.14 },
                            )))
                        })
                        .when(show_line_numbers, |d| {
                            d.child(conflict_diff_line_number_cell(
                                theme,
                                line_no,
                                ui_scale_percent,
                            ))
                        })
                        .child(conflict_diff_text_cell(
                            tab_width,
                            line_text.clone(),
                            styled,
                            show_ws,
                        ));

                    if let Some(conflict_ix) = range_ix {
                        if row_selection_enabled {
                            cell = cell
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, e: &MouseDownEvent, _window, cx| {
                                        if e.modifiers.shift || e.modifiers.control {
                                            this.conflict_resolver_click_row_selection(
                                                conflict_ix,
                                                ix,
                                                e.modifiers,
                                                cx,
                                            );
                                        } else {
                                            this.conflict_resolver_begin_row_selection(
                                                conflict_ix,
                                                ix,
                                                cx,
                                            );
                                        }
                                    }),
                                )
                                .on_mouse_move(cx.listener(
                                    move |this, _e: &MouseMoveEvent, _window, cx| {
                                        this.conflict_resolver_extend_row_selection(
                                            conflict_ix,
                                            ix,
                                            cx,
                                        );
                                    },
                                ));
                        }
                        let has_base = this
                            .conflict_resolver
                            .conflict_has_base
                            .get(conflict_ix)
                            .copied()
                            .unwrap_or(false);
                        let selected_choices =
                            this.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
                        let (line_label, line_target, chunk_label, chunk_target) =
                            three_way_input_row_menu_targets(ix, conflict_ix, choice_enum);
                        if !row_selection_enabled {
                            // When split-selection is available, the begin
                            // handler above already selects the block.
                            cell = cell.on_pointer_click(
                                MouseButton::Left,
                                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                    // section 30: clicking a conflict block body selects it.
                                    this.conflict_resolver_select_conflict(conflict_ix, cx);
                                }),
                            );
                        }
                        cell = cell.on_pointer_click(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                if e.modifiers.shift {
                                    let invoker: SharedString =
                                        format!("{}_{}_{}", input_menu_prefix, conflict_ix, ix)
                                            .into();
                                    this.open_conflict_resolver_input_row_context_menu(
                                        invoker,
                                        line_label.clone(),
                                        line_target.clone(),
                                        chunk_label.clone(),
                                        chunk_target.clone(),
                                        e.position,
                                        window,
                                        cx,
                                    );
                                } else {
                                    let invoker: SharedString =
                                        format!("{}_{}_{}", chunk_menu_prefix, conflict_ix, ix)
                                            .into();
                                    this.open_conflict_resolver_chunk_context_menu(
                                        invoker,
                                        conflict_ix,
                                        has_base,
                                        true,
                                        selected_choices.clone(),
                                        None,
                                        e.position,
                                        window,
                                        cx,
                                    );
                                }
                            }),
                        );
                    } else if let Some(target_index) = semantic_nav_target {
                        cell = cell.cursor(CursorStyle::PointingHand).on_pointer_click(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                this.conflict_jump_to_nav_target(target_index, cx);
                            }),
                        );
                    }

                    elements.push(cell.into_any_element());
                }
            }
        }
        elements
    }
}
