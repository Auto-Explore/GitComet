use super::super::conflict_resolver;
use super::super::perf::{self, ViewPerfRenderLane, ViewPerfSpan};
use super::conflict_canvas::{self, ConflictChunkContext};
use super::diff_text::*;
use super::*;

const CONFLICT_ROW_FONT_SCALE: f32 = 0.80;
const CONFLICT_ROW_TEXT_TRAILING_PADDING_PX: f32 = 16.0;

fn build_conflict_cached_diff_styled_text(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_color: Option<gpui::Rgba>,
) -> CachedDiffStyledText {
    let _perf_scope = perf::span(ViewPerfSpan::StyledTextBuild);
    build_cached_diff_styled_text(
        theme,
        text,
        word_ranges,
        query,
        language,
        syntax_mode,
        word_color,
    )
}

#[derive(Default)]
struct ConflictRowStyledText {
    styled: Option<CachedDiffStyledText>,
    pending: bool,
}

fn build_conflict_row_base_styled(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    syntax_lang: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    prepared_line: PreparedDiffSyntaxLine,
) -> PreparedDocumentLineStyledText {
    if prepared_line.document.is_some() {
        return build_cached_diff_styled_text_for_prepared_document_line_nonblocking(
            theme,
            text,
            word_ranges,
            "",
            DiffSyntaxConfig {
                language: syntax_lang,
                mode: syntax_mode,
            },
            None,
            prepared_line,
        );
    }

    PreparedDocumentLineStyledText::Cacheable(build_conflict_cached_diff_styled_text(
        theme,
        text,
        word_ranges,
        "",
        syntax_lang,
        syntax_mode,
        None,
    ))
}

fn conflict_display_text(
    text: &SharedString,
    styled: Option<&CachedDiffStyledText>,
    show_whitespace: bool,
) -> SharedString {
    match styled {
        Some(styled) if show_whitespace => whitespace_visible_text(styled.text.as_ref()),
        Some(styled) => styled.text.clone(),
        None if show_whitespace => whitespace_visible_text(text.as_ref()),
        None => text.clone(),
    }
}

fn conflict_row_text_width(
    window: &mut Window,
    text: &SharedString,
    font_family: Option<&'static str>,
) -> Pixels {
    if text.is_empty() {
        return px(0.0);
    }

    let mut style = window.text_style();
    style.font_weight = FontWeight::NORMAL;
    if let Some(font_family) = font_family {
        style.font_family = font_family.into();
    }

    let font_size = style.font_size.to_pixels(window.rem_size()) * CONFLICT_ROW_FONT_SCALE;
    if !text.as_ref().contains(['\n', '\r']) {
        return window
            .text_system()
            .shape_line(text.clone(), font_size, &[style.to_run(text.len())], None)
            .width;
    }

    text.as_ref()
        .split(['\n', '\r'])
        .filter(|line| !line.is_empty())
        .map(|line| {
            window
                .text_system()
                .shape_line(
                    line.to_string().into(),
                    font_size,
                    &[style.to_run(line.len())],
                    None,
                )
                .width
        })
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal))
        .unwrap_or(px(0.0))
}

fn conflict_input_row_min_width(window: &mut Window, text: &SharedString) -> Pixels {
    let pad = window.rem_size() * 0.5;
    let line_no_width = px(38.0);
    let gap = pad;
    let row_extra = pad * 2.0 + line_no_width + gap;
    (row_extra
        + conflict_row_text_width(window, text, None)
        + px(CONFLICT_ROW_TEXT_TRAILING_PADDING_PX))
    .round()
}

fn conflict_resolved_output_row_min_width(window: &mut Window, text: &SharedString) -> Pixels {
    let pad = window.rem_size() * 0.5;
    let row_extra = pad * 2.0;
    (row_extra
        + conflict_row_text_width(window, text, Some(crate::view::UI_MONOSPACE_FONT_FAMILY))
        + px(CONFLICT_ROW_TEXT_TRAILING_PADDING_PX))
    .round()
}

fn render_conflict_markdown_preview_rows(
    this: &mut MainPaneView,
    range: Range<usize>,
    side: ThreeWayColumn,
    window: &mut Window,
    cx: &mut gpui::Context<MainPaneView>,
) -> Vec<AnyElement> {
    let theme = this.theme;
    let Loadable::Ready(document) = this.conflict_resolver.markdown_preview.document(side) else {
        return Vec::new();
    };
    let document = Arc::clone(document);
    let (row_id_prefix, horizontal_scroll_handle) = match side {
        ThreeWayColumn::Base => (
            "conflict_markdown_preview_base",
            this.conflict_resolver_diff_scroll
                .0
                .borrow()
                .base_handle
                .clone(),
        ),
        ThreeWayColumn::Ours => (
            "conflict_markdown_preview_ours",
            this.conflict_preview_ours_scroll
                .0
                .borrow()
                .base_handle
                .clone(),
        ),
        ThreeWayColumn::Theirs => (
            "conflict_markdown_preview_theirs",
            this.conflict_preview_theirs_scroll
                .0
                .borrow()
                .base_handle
                .clone(),
        ),
    };
    this.update_markdown_preview_horizontal_min_width(
        document.as_ref(),
        range.clone(),
        None,
        window,
        cx,
    );
    super::history::render_markdown_preview_document_rows(
        document.as_ref(),
        range,
        &super::history::MarkdownPreviewRenderContext {
            theme,
            bar_color: None,
            min_width: this.diff_horizontal_min_width,
            row_id_prefix,
            horizontal_scroll_handle: Some(horizontal_scroll_handle),
            view: None,
            text_region: DiffTextRegion::Inline,
        },
    )
}

impl MainPaneView {
    pub(in super::super) fn render_conflict_markdown_base_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_conflict_markdown_preview_rows(this, range, ThreeWayColumn::Base, window, cx)
    }

    pub(in super::super) fn render_conflict_markdown_ours_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_conflict_markdown_preview_rows(this, range, ThreeWayColumn::Ours, window, cx)
    }

    pub(in super::super) fn render_conflict_markdown_theirs_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        render_conflict_markdown_preview_rows(this, range, ThreeWayColumn::Theirs, window, cx)
    }

    // ── Per-column three-way render functions ──────────────────────────

    pub(in super::super) fn render_conflict_three_way_base_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_conflict_three_way_column_rows(this, range, ThreeWayColumn::Base, window, cx)
    }

    pub(in super::super) fn render_conflict_three_way_ours_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_conflict_three_way_column_rows(this, range, ThreeWayColumn::Ours, window, cx)
    }

    pub(in super::super) fn render_conflict_three_way_theirs_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_conflict_three_way_column_rows(this, range, ThreeWayColumn::Theirs, window, cx)
    }

    fn render_conflict_three_way_column_rows(
        this: &mut Self,
        range: Range<usize>,
        column: ThreeWayColumn,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let _perf_scope = perf::span(ViewPerfSpan::RenderThreeWayRows);
        let theme = this.theme;
        let show_ws = this.show_whitespace;
        let word_hl_color = Some(theme.colors.warning);
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
            let Some(conflict_resolver::ThreeWayVisibleItem::Line(ix)) =
                this.conflict_resolver.three_way_visible_item(vi)
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
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_config,
                    word_hl_color,
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
                    theme,
                    text,
                    word_ranges,
                    "",
                    syntax_lang,
                    DiffSyntaxMode::Auto,
                    word_hl_color,
                );
                this.conflict_three_way_segments_cache
                    .insert((ix, column), styled);
            }
        }
        if needs_chunk_poll {
            this.ensure_prepared_syntax_chunk_poll(cx);
        }

        let chosen_bg = with_alpha(theme.colors.accent, if theme.is_dark { 0.16 } else { 0.12 });
        let conflict_choices = this.conflict_resolver.conflict_choices.as_slice();

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
                continue;
            };

            match visible_item {
                conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(range_ix) => {
                    let label: SharedString = if matches!(column, ThreeWayColumn::Base) {
                        let choice_label = conflict_choices
                            .get(range_ix)
                            .map(|c| match c {
                                conflict_resolver::ConflictChoice::Base => "Base (A)",
                                conflict_resolver::ConflictChoice::Ours => "Local (B)",
                                conflict_resolver::ConflictChoice::Theirs => "Remote (C)",
                                conflict_resolver::ConflictChoice::Both => "Local+Remote (B+C)",
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
                        .w_full()
                        .h(px(20.0))
                        .flex()
                        .items_center()
                        .bg(with_alpha(
                            theme.colors.success,
                            if theme.is_dark { 0.08 } else { 0.06 },
                        ))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child(label)
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_down(
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
                conflict_resolver::ThreeWayVisibleItem::Line(ix) => {
                    let line_text = this.conflict_resolver.three_way_line_text(column, ix);
                    let range_ix = this
                        .conflict_resolver
                        .conflict_index_for_side_line(column, ix)
                        .filter(|_| line_text.is_some());
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

                    let styled = this.conflict_three_way_segments_cache.get(&(ix, column));

                    let bg = if is_in_conflict && line_text.is_some() {
                        match column {
                            ThreeWayColumn::Base => with_alpha(
                                theme.colors.warning,
                                if theme.is_dark { 0.10 } else { 0.08 },
                            ),
                            ThreeWayColumn::Ours => with_alpha(
                                theme.colors.success,
                                if theme.is_dark { 0.10 } else { 0.08 },
                            ),
                            ThreeWayColumn::Theirs => with_alpha(
                                theme.colors.accent,
                                if theme.is_dark { 0.14 } else { 0.10 },
                            ),
                        }
                    } else {
                        with_alpha(theme.colors.surface_bg_elevated, 0.0)
                    };
                    let fg = if line_text.is_some() {
                        theme.colors.text
                    } else {
                        theme.colors.text_muted
                    };
                    let line_no = line_number_string(
                        line_text
                            .is_some()
                            .then(|| u32::try_from(ix + 1).ok())
                            .flatten(),
                    );
                    let line_text = line_text.map(SharedString::new).unwrap_or_default();
                    let display_text = conflict_display_text(&line_text, styled, show_ws);
                    let min_width = conflict_input_row_min_width(window, &display_text);

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
                            theme,
                            cx.entity(),
                            canvas_id_prefix,
                            vi,
                            ix,
                            min_width,
                            line_no,
                            if is_chosen { chosen_bg } else { bg },
                            fg,
                            line_text.clone(),
                            styled,
                            show_ws,
                            chunk_context,
                            chunk_menu_prefix,
                            true,
                        ));
                        continue;
                    }

                    let mut cell = div()
                        .id((div_id_prefix, ix))
                        .w_full()
                        .min_w(min_width)
                        .h(px(20.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .gap_2()
                        .text_xs()
                        .text_color(fg)
                        .whitespace_nowrap()
                        .bg(bg)
                        .when(is_chosen, |d| d.bg(chosen_bg))
                        .child(
                            div()
                                .w(px(38.0))
                                .text_color(theme.colors.text_muted)
                                .child(line_no),
                        )
                        .child(conflict_diff_text_cell(line_text.clone(), styled, show_ws));

                    if let Some(conflict_ix) = range_ix {
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
                        cell = cell.on_mouse_down(
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
                    }

                    elements.push(cell.into_any_element());
                }
            }
        }
        elements
    }

    // ── Per-column two-way diff render functions ────────────────────────

    pub(in super::super) fn render_conflict_diff_left_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_conflict_diff_column_rows(this, range, ConflictPickSide::Ours, window, cx)
    }

    pub(in super::super) fn render_conflict_diff_right_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        Self::render_conflict_diff_column_rows(this, range, ConflictPickSide::Theirs, window, cx)
    }

    fn render_conflict_diff_column_rows(
        this: &mut Self,
        range: Range<usize>,
        side: ConflictPickSide,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let _perf_scope = perf::span(ViewPerfSpan::RenderResolverDiffRows);
        let query = this.diff_search_query_or_empty();
        let query = query.as_ref().trim().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str());
        let syntax_lang = this.conflict_row_syntax_language();
        let syntax_mode = DiffSyntaxMode::Auto;
        let theme = this.theme;
        let show_ws = this.show_whitespace;

        let (div_id_prefix, canvas_id_prefix, chunk_menu_prefix, input_menu_prefix) = match side {
            ConflictPickSide::Ours => (
                "conflict_diff_col_ours",
                "conflict_diff_canvas_ours",
                "resolver_two_way_split_ours_chunk_menu",
                "resolver_two_way_split_ours_input_menu",
            ),
            ConflictPickSide::Theirs => (
                "conflict_diff_col_theirs",
                "conflict_diff_canvas_theirs",
                "resolver_two_way_split_theirs_chunk_menu",
                "resolver_two_way_split_theirs_input_menu",
            ),
        };

        range
            .map(|visible_row_ix| {
                let Some(visible_row) = this
                    .conflict_resolver
                    .two_way_split_visible_row(visible_row_ix)
                else {
                    return div()
                        .id((div_id_prefix, visible_row_ix))
                        .h(px(20.0))
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };
                let conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix: row_ix,
                    row,
                    conflict_ix,
                } = visible_row;

                let (text_opt, line_no, document) = match side {
                    ConflictPickSide::Ours => (
                        row.old.as_deref(),
                        row.old_line,
                        this.conflict_three_way_prepared_syntax_documents.ours,
                    ),
                    ConflictPickSide::Theirs => (
                        row.new.as_deref(),
                        row.new_line,
                        this.conflict_three_way_prepared_syntax_documents.theirs,
                    ),
                };

                let text = SharedString::new(text_opt.unwrap_or_default());
                let styling_enabled = this.conflict_row_styling_enabled();
                let word_hl_computed = if styling_enabled {
                    conflict_resolver::compute_word_highlights_for_row(&row)
                } else {
                    None
                };
                let word_hl_precomputed = if styling_enabled {
                    this.conflict_resolver.two_way_split_word_highlight(row_ix)
                } else {
                    None
                };
                let word_hl = word_hl_computed.as_ref().or(word_hl_precomputed);
                let word_ranges = match side {
                    ConflictPickSide::Ours => word_hl.map(|(o, _)| o.as_slice()).unwrap_or(&[]),
                    ConflictPickSide::Theirs => word_hl.map(|(_, n)| n.as_slice()).unwrap_or(&[]),
                };
                let q = this.conflict_diff_query_cache_query.as_ref();
                let styled_result = Self::conflict_split_row_styled(
                    theme,
                    &mut this.conflict_diff_segments_cache_split,
                    &mut this.conflict_diff_query_segments_cache_split,
                    row_ix,
                    side,
                    text_opt,
                    word_ranges,
                    q,
                    syntax_lang,
                    syntax_mode,
                    prepared_diff_syntax_line_for_one_based_line(document, line_no),
                );
                if styled_result.pending {
                    this.ensure_prepared_syntax_chunk_poll(cx);
                }
                let styled = styled_result.styled;

                let bg = split_cell_bg(theme, row.kind, side);
                let fg = if text_opt.is_some() {
                    theme.colors.text
                } else {
                    theme.colors.text_muted
                };
                let display_text = conflict_display_text(&text, styled.as_ref(), show_ws);
                let min_width = conflict_input_row_min_width(window, &display_text);

                if this.conflict_canvas_rows_enabled {
                    let chunk_context_data = conflict_ix.map(|conflict_ix| ConflictChunkContext {
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
                    return conflict_canvas::single_column_conflict_canvas(
                        theme,
                        cx.entity(),
                        canvas_id_prefix,
                        visible_row_ix,
                        row_ix,
                        min_width,
                        line_number_string(line_no),
                        bg,
                        fg,
                        text,
                        styled.as_ref(),
                        show_ws,
                        chunk_context_data,
                        chunk_menu_prefix,
                        false,
                    );
                }

                let mut cell = div()
                    .id((div_id_prefix, row_ix))
                    .w_full()
                    .min_w(min_width)
                    .h(px(20.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .text_xs()
                    .bg(bg)
                    .text_color(fg)
                    .whitespace_nowrap()
                    .child(
                        div()
                            .w(px(38.0))
                            .text_color(theme.colors.text_muted)
                            .child(line_number_string(line_no)),
                    )
                    .child(conflict_diff_text_cell(
                        text.clone(),
                        styled.as_ref(),
                        show_ws,
                    ));

                if let Some(conflict_ix) = conflict_ix {
                    let has_base = this
                        .conflict_resolver
                        .conflict_has_base
                        .get(conflict_ix)
                        .copied()
                        .unwrap_or(false);
                    let selected_choices =
                        this.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
                    let (line_label, line_target, chunk_label, chunk_target) =
                        two_way_split_input_row_menu_targets(row_ix, conflict_ix, side);
                    cell = cell.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            if e.modifiers.shift {
                                let invoker: SharedString =
                                    format!("{}_{}_{}", input_menu_prefix, conflict_ix, row_ix)
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
                                    format!("{}_{}_{}", chunk_menu_prefix, conflict_ix, row_ix)
                                        .into();
                                this.open_conflict_resolver_chunk_context_menu(
                                    invoker,
                                    conflict_ix,
                                    has_base,
                                    false,
                                    selected_choices.clone(),
                                    None,
                                    e.position,
                                    window,
                                    cx,
                                );
                            }
                        }),
                    );
                }

                cell.into_any_element()
            })
            .collect()
    }

    pub(in super::super) fn render_conflict_resolved_preview_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let _perf_scope = perf::span(ViewPerfSpan::RenderResolvedPreviewRows);
        let requested_rows = range.len();
        let theme = this.theme;

        let elements: Vec<AnyElement> = range
            .map(|ix| {
                if ix >= this.conflict_resolved_preview_line_count {
                    return div()
                        .id(("conflict_resolved_preview_oob", ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                }

                let source_meta = this.conflict_resolver.resolved_outline.meta.get(ix);
                let source = source_meta
                    .map(|m| m.source)
                    .unwrap_or(conflict_resolver::ResolvedLineSource::Manual);
                let (_, badge_fg) = resolved_output_source_badge_colors(theme, source);
                let conflict_marker = this
                    .conflict_resolver
                    .resolved_outline
                    .markers
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
            .collect();
        perf::record_row_batch(
            ViewPerfRenderLane::ResolvedPreview,
            requested_rows,
            elements.len(),
        );
        elements
    }

    pub(in super::super) fn render_conflict_resolved_output_rows(
        this: &mut Self,
        range: Range<usize>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let _perf_scope = perf::span(ViewPerfSpan::RenderResolvedPreviewRows);
        let requested_rows = range.len();
        let theme = this.theme;
        if let Some(projection) = this.conflict_resolved_output_projection.as_ref() {
            let unresolved_row_bg =
                with_alpha(theme.colors.danger, if theme.is_dark { 0.18 } else { 0.10 });
            let resolved_row_bg = with_alpha(
                theme.colors.success,
                if theme.is_dark { 0.12 } else { 0.08 },
            );

            let elements: Vec<AnyElement> = range
                .map(|ix| {
                    if ix >= this.conflict_resolved_preview_line_count {
                        return div()
                            .id(("conflict_resolved_output_oob", ix))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.text_muted)
                            .child("")
                            .into_any_element();
                    }

                    let line_text: SharedString = projection
                        .line_text(&this.conflict_resolver.marker_segments, ix)
                        .unwrap_or(std::borrow::Cow::Borrowed(""))
                        .to_string()
                        .into();
                    let min_width = conflict_resolved_output_row_min_width(window, &line_text);

                    let conflict_marker = this
                        .conflict_resolver
                        .resolved_outline
                        .markers
                        .get(ix)
                        .copied()
                        .flatten();
                    let row_bg = conflict_marker.map(|marker| {
                        if marker.unresolved {
                            unresolved_row_bg
                        } else {
                            resolved_row_bg
                        }
                    });

                    div()
                        .id(("conflict_resolved_output_row", ix))
                        .w_full()
                        .min_w(min_width)
                        .h(px(20.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .text_xs()
                        .font_family("monospace")
                        .text_color(theme.colors.text)
                        .whitespace_nowrap()
                        .when_some(row_bg, |d, bg| d.bg(bg))
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                this.open_conflict_resolver_output_context_menu_for_line(
                                    ix, e.position, window, cx,
                                );
                            }),
                        )
                        .child(
                            div()
                                .w_full()
                                .min_w(px(0.0))
                                .overflow_hidden()
                                .child(line_text),
                        )
                        .into_any_element()
                })
                .collect();
            perf::record_row_batch(
                ViewPerfRenderLane::ResolvedPreview,
                requested_rows,
                elements.len(),
            );
            return elements;
        }

        let syntax_language = this.conflict_resolved_preview_render_syntax_language();
        let syntax_document = this.conflict_resolved_preview_prepared_syntax_document;
        let syntax_mode = syntax_mode_for_prepared_document(syntax_document);
        let line_starts = &this.conflict_resolved_preview_line_starts;
        let (line_texts, prepared_line_highlights) =
            this.conflict_resolver_input.read_with(cx, |input, _| {
                let text = input.text();
                let line_texts: Vec<SharedString> = range
                    .clone()
                    .map(|ix| {
                        resolved_output_line_text(text, line_starts, ix)
                            .to_string()
                            .into()
                    })
                    .collect();
                let prepared_line_highlights = syntax_document
                    .zip(syntax_language)
                    .and_then(|(document, language)| {
                        request_syntax_highlights_for_prepared_document_line_range(
                            theme,
                            text,
                            line_starts,
                            document,
                            language,
                            range.clone(),
                        )
                    })
                    .unwrap_or_default();
                (line_texts, prepared_line_highlights)
            });
        if prepared_line_highlights.iter().any(|line| line.pending) {
            this.ensure_prepared_syntax_chunk_poll(cx);
        }

        let unresolved_row_bg =
            with_alpha(theme.colors.danger, if theme.is_dark { 0.18 } else { 0.10 });
        let resolved_row_bg = with_alpha(
            theme.colors.success,
            if theme.is_dark { 0.12 } else { 0.08 },
        );

        let elements: Vec<AnyElement> = range
            .zip(line_texts)
            .enumerate()
            .map(|(local_ix, (ix, line_text))| {
                if ix >= this.conflict_resolved_preview_line_count {
                    return div()
                        .id(("conflict_resolved_output_oob", ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                }
                let min_width = conflict_resolved_output_row_min_width(window, &line_text);

                let row_content = if syntax_language.is_some() && !line_text.is_empty() {
                    let prepared_line_highlight = prepared_line_highlights
                        .get(local_ix)
                        .filter(|line| line.line_ix == ix);
                    let needs_refresh = this
                        .conflict_resolved_preview_segments_cache_get(ix)
                        .is_none_or(|styled| styled.text.as_ref() != line_text.as_ref());
                    let mut pending_styled = None;
                    if needs_refresh {
                        if let Some(line_highlights) = prepared_line_highlight {
                            let styled = build_cached_diff_styled_text_from_relative_highlights(
                                line_text.as_ref(),
                                line_highlights.highlights.as_slice(),
                            );
                            if line_highlights.pending {
                                pending_styled = Some(styled);
                            } else {
                                this.conflict_resolved_preview_segments_cache_set(ix, styled);
                            }
                        } else {
                            let styled = build_conflict_cached_diff_styled_text(
                                theme,
                                line_text.as_ref(),
                                &[],
                                "",
                                syntax_language,
                                syntax_mode,
                                None,
                            );
                            this.conflict_resolved_preview_segments_cache_set(ix, styled);
                        }
                    }
                    let cached_styled = this.conflict_resolved_preview_segments_cache_get(ix);
                    let styled = pending_styled
                        .as_ref()
                        .or(cached_styled)
                        .expect("resolved preview row style should exist after populate");
                    if styled.highlights.is_empty() {
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(styled.text.clone())
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .min_w(px(0.0))
                            .overflow_hidden()
                            .child(
                                gpui::StyledText::new(styled.text.clone())
                                    .with_highlights(styled.highlights.iter().cloned()),
                            )
                            .into_any_element()
                    }
                } else {
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .overflow_hidden()
                        .child(line_text)
                        .into_any_element()
                };

                let conflict_marker = this
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .get(ix)
                    .copied()
                    .flatten();
                let row_bg = conflict_marker.map(|marker| {
                    if marker.unresolved {
                        unresolved_row_bg
                    } else {
                        resolved_row_bg
                    }
                });

                div()
                    .id(("conflict_resolved_output_row", ix))
                    .w_full()
                    .min_w(min_width)
                    .h(px(20.0))
                    .px_2()
                    .flex()
                    .items_center()
                    .text_xs()
                    .font_family("monospace")
                    .text_color(theme.colors.text)
                    .whitespace_nowrap()
                    .when_some(row_bg, |d, bg| d.bg(bg))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_conflict_resolver_output_context_menu_for_line(
                                ix, e.position, window, cx,
                            );
                        }),
                    )
                    .child(row_content)
                    .into_any_element()
            })
            .collect();
        perf::record_row_batch(
            ViewPerfRenderLane::ResolvedPreview,
            requested_rows,
            elements.len(),
        );
        elements
    }

    pub(in super::super) fn render_conflict_compare_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let query = this.diff_search_query_or_empty();
        let query = query.as_ref().trim().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str());
        let syntax_lang = this.conflict_row_syntax_language();
        // Streamed conflicts may or may not have prepared side documents; Auto
        // remains the safe fallback when a row is not backed by one.
        let syntax_mode = DiffSyntaxMode::Auto;
        range
            .map(|visible_row_ix| {
                let Some(visible_row) = this
                    .conflict_resolver
                    .two_way_split_visible_row(visible_row_ix)
                else {
                    return div()
                        .id(("conflict_compare_split_visible_oob", visible_row_ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(this.theme.colors.text_muted)
                        .child("")
                        .into_any_element();
                };
                let row_ix = visible_row.source_row_ix;
                let row = visible_row.row;
                this.render_conflict_compare_split_row(
                    visible_row_ix,
                    row_ix,
                    row,
                    syntax_lang,
                    syntax_mode,
                    cx,
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict_row_styled<K>(
        theme: AppTheme,
        stable_cache: &mut HashMap<K, CachedDiffStyledText>,
        query_cache: &mut HashMap<K, CachedDiffStyledText>,
        key: K,
        text: &str,
        word_ranges: &[Range<usize>],
        query: &str,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
        prepared_line: PreparedDiffSyntaxLine,
    ) -> ConflictRowStyledText
    where
        K: Copy + Eq + std::hash::Hash,
    {
        let mut result = ConflictRowStyledText::default();
        if text.is_empty() {
            return result;
        }

        let query = query.trim();
        let query_active = !query.is_empty();
        let base_has_style = !word_ranges.is_empty() || syntax_lang.is_some();

        if base_has_style {
            if let Some(cached) = stable_cache.get(&key) {
                result.styled = Some(cached.clone());
            } else {
                let (styled, pending) = build_conflict_row_base_styled(
                    theme,
                    text,
                    word_ranges,
                    syntax_lang,
                    syntax_mode,
                    prepared_line,
                )
                .into_parts();
                if !pending {
                    stable_cache.insert(key, styled.clone());
                }
                result.styled = Some(styled);
                result.pending = pending;
            }
        }

        if query_active {
            if !result.pending
                && let Some(cached) = query_cache.get(&key)
            {
                result.styled = Some(cached.clone());
                return result;
            }

            let styled =
                if let Some(base) = result.styled.as_ref().or_else(|| stable_cache.get(&key)) {
                    build_cached_diff_query_overlay_styled_text(theme, base, query)
                } else {
                    build_conflict_cached_diff_styled_text(
                        theme,
                        text,
                        word_ranges,
                        query,
                        syntax_lang,
                        syntax_mode,
                        None,
                    )
                };
            if !result.pending {
                query_cache.insert(key, styled.clone());
            }
            result.styled = Some(styled);
        }

        result
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
        prepared_line: PreparedDiffSyntaxLine,
    ) -> ConflictRowStyledText {
        let Some(text) = text else {
            return ConflictRowStyledText::default();
        };
        Self::conflict_row_styled(
            theme,
            stable_cache,
            query_cache,
            (row_ix, side),
            text,
            word_ranges,
            query,
            syntax_lang,
            syntax_mode,
            prepared_line,
        )
    }

    fn render_conflict_compare_split_row(
        &mut self,
        visible_row_ix: usize,
        row_ix: usize,
        row: gitcomet_core::file_diff::FileDiffRow,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let show_ws = self.show_whitespace;

        let left_text = SharedString::new(row.old.as_deref().unwrap_or_default());
        let right_text = SharedString::new(row.new.as_deref().unwrap_or_default());
        let ours_document = self.conflict_three_way_prepared_syntax_documents.ours;
        let theirs_document = self.conflict_three_way_prepared_syntax_documents.theirs;

        // Large streamed compare views should avoid retaining per-row styled
        // caches as users scroll through the whole-file projection.
        let styling_enabled = self.conflict_row_styling_enabled()
            && self.conflict_resolver.three_way_len
                <= conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES;
        let word_hl_computed = if styling_enabled {
            conflict_resolver::compute_word_highlights_for_row(&row)
        } else {
            None
        };
        let word_hl_precomputed = if styling_enabled {
            self.conflict_resolver.two_way_split_word_highlight(row_ix)
        } else {
            None
        };
        let word_hl = word_hl_computed.as_ref().or(word_hl_precomputed);
        let old_word_ranges = word_hl.map(|(o, _)| o.as_slice()).unwrap_or(&[]);
        let new_word_ranges = word_hl.map(|(_, n)| n.as_slice()).unwrap_or(&[]);
        let query = self.conflict_diff_query_cache_query.as_ref();
        let (left_styled, right_styled) = if styling_enabled {
            (
                Self::conflict_split_row_styled(
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
                    prepared_diff_syntax_line_for_one_based_line(ours_document, row.old_line),
                ),
                Self::conflict_split_row_styled(
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
                    prepared_diff_syntax_line_for_one_based_line(theirs_document, row.new_line),
                ),
            )
        } else {
            (
                ConflictRowStyledText::default(),
                ConflictRowStyledText::default(),
            )
        };
        if left_styled.pending || right_styled.pending {
            self.ensure_prepared_syntax_chunk_poll(cx);
        }
        let left_styled = left_styled.styled;
        let right_styled = right_styled.styled;

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
            .overflow_hidden()
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
            .overflow_hidden()
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

    if show_whitespace {
        let (display, highlights) = whitespace_visible_text_and_highlights(
            styled.text.as_ref(),
            styled.highlights.as_ref(),
        );
        if highlights.is_empty() {
            return div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(display)
                .into_any_element();
        }
        return div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(gpui::StyledText::new(display).with_highlights(highlights))
            .into_any_element();
    }

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
    whitespace_visible_text_and_highlights(text, &[]).0
}

fn whitespace_visible_text_and_highlights(
    text: &str,
    highlights: &[(Range<usize>, gpui::HighlightStyle)],
) -> (SharedString, Vec<(Range<usize>, gpui::HighlightStyle)>) {
    let mut out = String::with_capacity(text.len());
    let mut byte_map = vec![0usize; text.len() + 1];

    for (start, ch) in text.char_indices() {
        byte_map[start] = out.len();
        match ch {
            ' ' => out.push('\u{00B7}'),                     // middle dot
            '\t' => out.push('\u{2192}'),                    // rightwards arrow
            '\r' => out.push('\u{240D}'),                    // carriage return symbol
            '\n' => out.push('\u{21B5}'),                    // carriage return arrow
            _ if ch.is_whitespace() => out.push('\u{2420}'), // symbol for space
            _ => out.push(ch),
        }
        let end = start + ch.len_utf8();
        let mapped_end = out.len();
        for mapped in byte_map.iter_mut().take(end + 1).skip(start + 1) {
            *mapped = mapped_end;
        }
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

fn three_way_choice_short_label(choice: conflict_resolver::ConflictChoice) -> &'static str {
    match choice {
        conflict_resolver::ConflictChoice::Base => "A",
        conflict_resolver::ConflictChoice::Ours => "B",
        conflict_resolver::ConflictChoice::Theirs => "C",
        conflict_resolver::ConflictChoice::Both => "B+C",
    }
}

fn two_way_side_label(side: ConflictPickSide) -> &'static str {
    match side {
        ConflictPickSide::Ours => "local",
        ConflictPickSide::Theirs => "remote",
    }
}

fn two_way_choice_for_side(side: ConflictPickSide) -> conflict_resolver::ConflictChoice {
    match side {
        ConflictPickSide::Ours => conflict_resolver::ConflictChoice::Ours,
        ConflictPickSide::Theirs => conflict_resolver::ConflictChoice::Theirs,
    }
}

fn three_way_input_row_menu_targets(
    line_ix: usize,
    conflict_ix: usize,
    choice: conflict_resolver::ConflictChoice,
) -> (
    SharedString,
    ResolverPickTarget,
    SharedString,
    ResolverPickTarget,
) {
    let label = three_way_choice_short_label(choice);
    (
        format!("Pick this line ({label})").into(),
        ResolverPickTarget::ThreeWayLine { line_ix, choice },
        format!("Pick this chunk ({label})").into(),
        ResolverPickTarget::Chunk {
            conflict_ix,
            choice,
            output_line_ix: None,
        },
    )
}

fn two_way_split_input_row_menu_targets(
    row_ix: usize,
    conflict_ix: usize,
    side: ConflictPickSide,
) -> (
    SharedString,
    ResolverPickTarget,
    SharedString,
    ResolverPickTarget,
) {
    let side_label = two_way_side_label(side);
    let choice = two_way_choice_for_side(side);
    (
        format!("Pick this line ({side_label})").into(),
        ResolverPickTarget::TwoWaySplitLine { row_ix, side },
        format!("Pick this chunk ({side_label})").into(),
        ResolverPickTarget::Chunk {
            conflict_ix,
            choice,
            output_line_ix: None,
        },
    )
}

fn split_cell_bg(
    theme: AppTheme,
    kind: gitcomet_core::file_diff::FileDiffRowKind,
    side: ConflictPickSide,
) -> gpui::Rgba {
    match (kind, side) {
        (gitcomet_core::file_diff::FileDiffRowKind::Add, ConflictPickSide::Theirs)
        | (gitcomet_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Theirs) => {
            with_alpha(
                theme.colors.success,
                if theme.is_dark { 0.10 } else { 0.08 },
            )
        }
        (gitcomet_core::file_diff::FileDiffRowKind::Remove, ConflictPickSide::Ours)
        | (gitcomet_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Ours) => {
            with_alpha(
                theme.colors.warning,
                if theme.is_dark { 0.10 } else { 0.08 },
            )
        }
        _ => with_alpha(theme.colors.surface_bg_elevated, 0.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whitespace_visible_text_and_highlights_remaps_highlight_ranges() {
        let style = gpui::HighlightStyle::default();
        let (display, highlights) =
            whitespace_visible_text_and_highlights("a b\t", &[(1..4, style)]);

        assert_eq!(display.as_ref(), "a·b→");
        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].0, 1..7);
    }

    #[test]
    fn whitespace_visible_text_marks_all_whitespace_kinds() {
        let display = whitespace_visible_text(" \t\r\n");
        assert_eq!(display.as_ref(), "·→␍↵");
    }

    #[test]
    fn three_way_input_row_targets_include_line_and_chunk_picks() {
        let (line_label, line_target, chunk_label, chunk_target) =
            three_way_input_row_menu_targets(4, 2, conflict_resolver::ConflictChoice::Theirs);

        assert_eq!(line_label.as_ref(), "Pick this line (C)");
        assert_eq!(chunk_label.as_ref(), "Pick this chunk (C)");
        assert_eq!(
            line_target,
            ResolverPickTarget::ThreeWayLine {
                line_ix: 4,
                choice: conflict_resolver::ConflictChoice::Theirs,
            }
        );
        assert_eq!(
            chunk_target,
            ResolverPickTarget::Chunk {
                conflict_ix: 2,
                choice: conflict_resolver::ConflictChoice::Theirs,
                output_line_ix: None,
            }
        );
    }

    #[test]
    fn two_way_split_input_row_targets_map_side_to_split_line_and_chunk_choice() {
        let (line_label, line_target, chunk_label, chunk_target) =
            two_way_split_input_row_menu_targets(9, 5, ConflictPickSide::Ours);

        assert_eq!(line_label.as_ref(), "Pick this line (local)");
        assert_eq!(chunk_label.as_ref(), "Pick this chunk (local)");
        assert_eq!(
            line_target,
            ResolverPickTarget::TwoWaySplitLine {
                row_ix: 9,
                side: ConflictPickSide::Ours,
            }
        );
        assert_eq!(
            chunk_target,
            ResolverPickTarget::Chunk {
                conflict_ix: 5,
                choice: conflict_resolver::ConflictChoice::Ours,
                output_line_ix: None,
            }
        );
    }
}
