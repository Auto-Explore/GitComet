use super::super::conflict_resolver;
use super::super::perf::{self, ViewPerfRenderLane, ViewPerfSpan};
use super::conflict_canvas::{self, ConflictChunkContext};
use super::diff_text::*;
use super::*;
use crate::view::panes::main::diff_search::{DiffSearchMatcher, DiffSearchOptions};

const CONFLICT_ROW_FONT_SCALE: f32 = 0.80;
const CONFLICT_ROW_TEXT_TRAILING_PADDING_PX: f32 = 16.0;

fn build_conflict_cached_diff_styled_text(
    theme: AppTheme,
    text: &str,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_kind: Option<crate::theme::DiffColorKind>,
) -> CachedDiffStyledText {
    build_conflict_cached_diff_styled_text_with_source_identity(
        theme,
        text,
        None,
        word_ranges,
        query,
        language,
        syntax_mode,
        word_kind,
    )
}

fn build_conflict_cached_diff_styled_text_with_source_identity(
    theme: AppTheme,
    text: &str,
    source_identity: Option<DiffTextSourceIdentity>,
    word_ranges: &[Range<usize>],
    query: &str,
    language: Option<DiffSyntaxLanguage>,
    syntax_mode: DiffSyntaxMode,
    word_kind: Option<crate::theme::DiffColorKind>,
) -> CachedDiffStyledText {
    let _perf_scope = perf::span(ViewPerfSpan::StyledTextBuild);
    build_cached_diff_styled_text_with_source_identity(
        theme,
        text,
        source_identity,
        word_ranges,
        query,
        language,
        syntax_mode,
        word_kind,
    )
}

enum ConflictRowStyledTextValue {
    StableCached,
    QueryCached,
    Owned(CachedDiffStyledText),
}

#[derive(Default)]
struct ConflictRowStyledText {
    styled: Option<ConflictRowStyledTextValue>,
    pending: bool,
}

impl ConflictRowStyledText {
    fn resolve<'a>(
        &'a self,
        stable_cache: &'a conflict_resolver::ConflictSplitStyledTextCache,
        query_cache: &'a conflict_resolver::ConflictSplitStyledTextCache,
        key: (usize, ConflictPickSide),
    ) -> Option<&'a CachedDiffStyledText> {
        match self.styled.as_ref()? {
            ConflictRowStyledTextValue::StableCached => stable_cache.get(&key),
            ConflictRowStyledTextValue::QueryCached => query_cache.get(&key),
            ConflictRowStyledTextValue::Owned(styled) => Some(styled),
        }
    }
}

fn conflict_diff_query_matcher(
    query: &str,
    query_options: DiffSearchOptions,
) -> Option<DiffSearchMatcher> {
    (!query.is_empty()).then(|| DiffSearchMatcher::new(query, query_options))
}

fn build_conflict_row_base_styled(
    theme: AppTheme,
    text: &str,
    source_identity: Option<DiffTextSourceIdentity>,
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

    PreparedDocumentLineStyledText::Cacheable(
        build_conflict_cached_diff_styled_text_with_source_identity(
            theme,
            text,
            source_identity,
            word_ranges,
            "",
            syntax_lang,
            syntax_mode,
            None,
        ),
    )
}

fn conflict_display_text(
    text: &SharedString,
    styled: Option<&CachedDiffStyledText>,
    reveal_whitespace_chars: bool,
) -> SharedString {
    match styled {
        Some(_styled) if reveal_whitespace_chars => whitespace_visible_line_text(text.as_ref()),
        Some(styled) => styled.text.clone(),
        None if reveal_whitespace_chars => whitespace_visible_line_text(text.as_ref()),
        None => text.clone(),
    }
}

fn conflict_row_text_width(
    window: &mut Window,
    text: &SharedString,
    font_family: Option<&str>,
) -> Pixels {
    if text.is_empty() {
        return px(0.0);
    }

    let mut style = window.text_style();
    style.font_weight = FontWeight::NORMAL;
    if let Some(font_family) = font_family {
        style.font_family = font_family.to_string().into();
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

fn conflict_input_row_min_width(
    window: &mut Window,
    text: &SharedString,
    editor_font_family: &str,
    show_line_numbers: bool,
) -> Pixels {
    let pad = window.rem_size() * 0.5;
    let gap = pad;
    let line_no_width = if show_line_numbers {
        px(super::CONFLICT_DIFF_LINE_NO_WIDTH_PX) + gap
    } else {
        px(0.0)
    };
    let row_extra = pad * 2.0 + line_no_width;
    (row_extra
        + conflict_row_text_width(window, text, Some(editor_font_family))
        + px(CONFLICT_ROW_TEXT_TRAILING_PADDING_PX))
    .round()
}

fn conflict_resolved_output_row_min_width(
    window: &mut Window,
    text: &SharedString,
    editor_font_family: &str,
) -> Pixels {
    let pad = window.rem_size() * 0.5;
    let row_extra = pad * 2.0;
    (row_extra
        + conflict_row_text_width(window, text, Some(editor_font_family))
        + px(CONFLICT_ROW_TEXT_TRAILING_PADDING_PX))
    .round()
}

/// Width of the resolved-output line-number cell, sized to the file's digit
/// count so a short number sits snug against the marker lane instead of floating
/// across a cell wide enough for the largest line. The gutter container tracks
/// this width, so the marker stays pinned at the far-left edge and only where the
/// code column begins shifts a few px between files of very different line counts
/// (the same way any editor's line-number gutter widens with the line total).
pub(in crate::view) fn resolved_output_line_no_width(line_count: usize) -> Pixels {
    let digits = line_count.max(1).to_string().len().max(2);
    px(digits as f32 * 8.0)
}

/// Total width of the resolved-output gutter (marker lane + optional line-number
/// cell + origin badge, plus the row's horizontal padding), so the container
/// hugs its content and the badge/border sit right against the code.
pub(in crate::view) fn resolved_output_gutter_width(
    line_count: usize,
    show_line_numbers: bool,
) -> Pixels {
    /// Marker lane: 12px marker + 4px `mr_1` gap.
    const MARKER_LANE_PX: f32 = 12.0 + 4.0;
    /// Origin badge width.
    const BADGE_PX: f32 = 24.0;
    /// Row horizontal padding: `px_2` on each side (8 + 8).
    const ROW_PADDING_X_PX: f32 = 8.0 + 8.0;
    /// `mr_1` gap after the line-number cell.
    const LINE_NO_GAP_PX: f32 = 4.0;

    let marker_and_badge = px(MARKER_LANE_PX + BADGE_PX + ROW_PADDING_X_PX);
    if show_line_numbers {
        marker_and_badge + resolved_output_line_no_width(line_count) + px(LINE_NO_GAP_PX)
    } else {
        marker_and_badge
    }
}

fn render_conflict_markdown_preview_rows(
    this: &mut MainPaneView,
    range: Range<usize>,
    side: ThreeWayColumn,
    window: &mut Window,
    cx: &mut gpui::Context<MainPaneView>,
) -> Vec<AnyElement> {
    let theme = this.theme;
    let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
    let Loadable::Ready(document) = this.conflict_resolver.markdown_preview.document(side) else {
        return Vec::new();
    };
    let document = Arc::clone(document);
    let viewport_width = match side {
        ThreeWayColumn::Base => {
            this.conflict_resolver_diff_scroll
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .width
        }
        ThreeWayColumn::Ours => {
            this.conflict_preview_ours_scroll
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .width
        }
        ThreeWayColumn::Theirs => {
            this.conflict_preview_theirs_scroll
                .0
                .borrow()
                .base_handle
                .bounds()
                .size
                .width
        }
    }
    .max(px(0.0));
    this.update_markdown_preview_horizontal_min_width(
        document.as_ref(),
        range.clone(),
        editor_font_family.as_str(),
        window,
        cx,
    );
    super::history::render_markdown_preview_document_rows(
        document.as_ref(),
        range,
        &super::history::MarkdownPreviewRenderContext {
            theme,
            min_width: this.diff_horizontal_content_width().max(viewport_width),
            editor_font_family: editor_font_family.into(),
            ui_scale_percent: crate::ui_scale::current(cx).percent,
            view: None,
            text_region: DiffTextRegion::Inline,
            wrap_plan: None,
            image_base_dir: None,
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
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let show_ws = this.reveal_whitespace_chars;
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
                        .h(px(20.0))
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
                        .h(px(20.0))
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
                                        .w(px(3.0))
                                        .bg(theme.colors.accent.foreground),
                                )
                            },
                        )
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.foreground.secondary)
                        .child(label)
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                // section 30: clicking a conflict block body selects it.
                                this.conflict_resolver_select_conflict(range_ix, cx);
                            }),
                        )
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

                    let styled = side_line
                        .and_then(|l| this.conflict_three_way_segments_cache.get(&(l, column)));

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
                        window,
                        &display_text,
                        editor_font_family.as_str(),
                        show_line_numbers,
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
                        ));
                        continue;
                    }

                    let mut cell = div()
                        .id((div_id_prefix, ix))
                        .relative()
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
                        .when(is_active_conflict, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(3.0))
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
                            d.child(conflict_diff_line_number_cell(theme, line_no))
                        })
                        .child(conflict_diff_text_cell(line_text.clone(), styled, show_ws));

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
                            cell = cell.on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                    // section 30: clicking a conflict block body selects it.
                                    this.conflict_resolver_select_conflict(conflict_ix, cx);
                                }),
                            );
                        }
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
                    } else if let Some(target_index) = semantic_nav_target {
                        cell = cell.cursor(CursorStyle::PointingHand).on_mouse_down(
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
        // section 30 aligned row space: two-way full mode shares the three-way
        // projection. The block-local path below remains for giant files and
        // partially loaded sides.
        if this.conflict_resolver.two_way_uses_aligned_rows() {
            return Self::render_conflict_diff_aligned_column_rows(this, range, side, window, cx);
        }
        let _perf_scope = perf::span(ViewPerfSpan::RenderResolverDiffRows);
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query = query.as_ref().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str(), query_options);
        let query_matcher = conflict_diff_query_matcher(query.as_str(), query_options);
        let syntax_lang = this.conflict_row_syntax_language();
        let syntax_mode = DiffSyntaxMode::Auto;
        let theme = this.theme;
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let show_ws = this.reveal_whitespace_chars;
        let query_text = this.conflict_diff_query_cache_query.clone();
        let query = query_text.as_ref();

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
                        .text_color(theme.colors.foreground.secondary)
                        .child("")
                        .into_any_element();
                };
                let conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix: row_ix,
                    row,
                    conflict_ix,
                } = visible_row;
                let visual_kind = this.conflict_resolver.two_way_split_visual_kind_at(
                    row_ix,
                    &row,
                    this.diff_whitespace_mode,
                );

                let (text_opt, line_no, document) = match side {
                    ConflictPickSide::Ours => (
                        row.old.as_ref(),
                        row.old_line,
                        this.conflict_three_way_prepared_syntax_documents.ours,
                    ),
                    ConflictPickSide::Theirs => (
                        row.new.as_ref(),
                        row.new_line,
                        this.conflict_three_way_prepared_syntax_documents.theirs,
                    ),
                };

                let text = SharedString::new(text_opt.map(AsRef::as_ref).unwrap_or_default());
                let styling_enabled = this.conflict_row_styling_enabled();
                let word_hl = if styling_enabled
                    && !matches!(
                        visual_kind,
                        gitcomet_core::file_diff::FileDiffRowKind::Context
                    ) {
                    this.conflict_resolver
                        .two_way_split_word_highlight_for_row(row_ix, &row)
                } else {
                    None
                };
                let word_ranges = match side {
                    ConflictPickSide::Ours => word_hl
                        .as_ref()
                        .map(|pair| pair.0.as_slice())
                        .unwrap_or(&[]),
                    ConflictPickSide::Theirs => word_hl
                        .as_ref()
                        .map(|pair| pair.1.as_slice())
                        .unwrap_or(&[]),
                };
                let styled_result = Self::conflict_split_row_styled(
                    theme,
                    &mut this.conflict_diff_segments_cache_split,
                    &mut this.conflict_diff_query_segments_cache_split,
                    row_ix,
                    side,
                    text_opt.map(AsRef::as_ref),
                    word_ranges,
                    query,
                    query_options,
                    query_matcher.as_ref(),
                    syntax_lang,
                    syntax_mode,
                    prepared_diff_syntax_line_for_one_based_line(document, line_no),
                );
                if styled_result.pending {
                    this.ensure_prepared_syntax_chunk_poll(cx);
                }
                let styled = styled_result.resolve(
                    &this.conflict_diff_segments_cache_split,
                    &this.conflict_diff_query_segments_cache_split,
                    (row_ix, side),
                );

                let bg = split_cell_bg(theme, visual_kind, side);
                let fg = if text_opt.is_some() {
                    theme.colors.foreground.primary
                } else {
                    theme.colors.foreground.secondary
                };
                let display_text = conflict_display_text(&text, styled, show_ws);
                let show_line_numbers = this.mergetool_show_line_numbers;
                let min_width = conflict_input_row_min_width(
                    window,
                    &display_text,
                    editor_font_family.as_str(),
                    show_line_numbers,
                );

                let is_active_conflict = this.conflict_resolver.conflict_is_active(conflict_ix);
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
                        show_line_numbers,
                        line_number_string(line_no),
                        bg,
                        fg,
                        text,
                        styled,
                        show_ws,
                        chunk_context_data,
                        chunk_menu_prefix,
                        false,
                        None,
                        is_active_conflict,
                        // Block-local two-way rows are not in the shared aligned
                        // space, so split selection and manual alignment are
                        // both unavailable here.
                        None,
                        None,
                    );
                }

                let mut cell = div()
                    .id((div_id_prefix, row_ix))
                    .relative()
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
                    .when(is_active_conflict, |d| {
                        d.child(
                            div()
                                .absolute()
                                .left_0()
                                .top_0()
                                .bottom_0()
                                .w(px(3.0))
                                .bg(theme.colors.accent.foreground),
                        )
                    })
                    .when(show_line_numbers, |d| {
                        d.child(conflict_diff_line_number_cell(
                            theme,
                            line_number_string(line_no),
                        ))
                    })
                    .child(conflict_diff_text_cell(text.clone(), styled, show_ws));

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
                        MouseButton::Left,
                        cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                            // section 30: clicking a conflict block body selects it.
                            this.conflict_resolver_select_conflict(conflict_ix, cx);
                        }),
                    );
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

    /// section 30 aligned row space: two-way full mode. Renders one column of the
    /// ours↔theirs diff over the shared three-way visible projection —
    /// whole-file rows, context folds, and collapsed resolved blocks — while
    /// keeping the two-way diff styling (add/remove/modify backgrounds and
    /// ours↔theirs word highlights). Row keys for the styled-text caches are
    /// aligned rows, which are stable for the session.
    fn render_conflict_diff_aligned_column_rows(
        this: &mut Self,
        range: Range<usize>,
        side: ConflictPickSide,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        use gitcomet_core::file_diff::FileDiffRowKind as RK;

        let _perf_scope = perf::span(ViewPerfSpan::RenderResolverDiffRows);
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query = query.as_ref().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str(), query_options);
        let query_matcher = conflict_diff_query_matcher(query.as_str(), query_options);
        let syntax_lang = this.conflict_row_syntax_language();
        let syntax_mode = DiffSyntaxMode::Auto;
        let theme = this.theme;
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let show_ws = this.reveal_whitespace_chars;
        let query_text = this.conflict_diff_query_cache_query.clone();
        let query = query_text.as_ref();
        let whitespace_mode = this.diff_whitespace_mode;
        let styling_enabled = this.conflict_row_styling_enabled();

        let column = match side {
            ConflictPickSide::Ours => ThreeWayColumn::Ours,
            ConflictPickSide::Theirs => ThreeWayColumn::Theirs,
        };
        let document = match side {
            ConflictPickSide::Ours => this.conflict_three_way_prepared_syntax_documents.ours,
            ConflictPickSide::Theirs => this.conflict_three_way_prepared_syntax_documents.theirs,
        };
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
        let conflict_choices = this.conflict_resolver.conflict_choices.clone();

        let mut needs_chunk_poll = false;
        let mut elements = Vec::with_capacity(range.len());
        for vi in range {
            let Some(visible_item) = this.conflict_resolver.three_way_visible_item(vi) else {
                // Past-the-end rows exist only as bottom overscroll space.
                elements.push(
                    div()
                        .id((div_id_prefix, vi))
                        .w_full()
                        .h(px(20.0))
                        .into_any_element(),
                );
                continue;
            };

            match visible_item {
                conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(range_ix) => {
                    let label: SharedString = if matches!(side, ConflictPickSide::Ours) {
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
                        .h(px(20.0))
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
                                        .w(px(3.0))
                                        .bg(theme.colors.accent.foreground),
                                )
                            },
                        )
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.foreground.secondary)
                        .child(label)
                        .cursor(CursorStyle::PointingHand)
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                // section 30: clicking a conflict block body selects it.
                                this.conflict_resolver_select_conflict(range_ix, cx);
                            }),
                        )
                        .on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                let invoker: SharedString = format!(
                                    "resolver_two_way_collapsed_chunk_menu_{}_{}",
                                    range_ix, vi
                                )
                                .into();
                                this.open_conflict_resolver_chunk_context_menu(
                                    invoker,
                                    range_ix,
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
                conflict_resolver::ThreeWayVisibleItem::Line(row) => {
                    let ours_line = this
                        .conflict_resolver
                        .three_way_side_line_for_row(ThreeWayColumn::Ours, row);
                    let theirs_line = this
                        .conflict_resolver
                        .three_way_side_line_for_row(ThreeWayColumn::Theirs, row);
                    let ours_text = ours_line.and_then(|l| {
                        this.conflict_resolver
                            .three_way_line_text(ThreeWayColumn::Ours, l)
                    });
                    let theirs_text = theirs_line.and_then(|l| {
                        this.conflict_resolver
                            .three_way_line_text(ThreeWayColumn::Theirs, l)
                    });

                    // Per-row diff kind from the aligned pair. Unlike the
                    // block-local path there is no run-level whitespace
                    // arbitration; a row is whitespace-equal on its own.
                    let visual_kind = match (ours_text, theirs_text) {
                        (Some(o), Some(t)) if o == t => RK::Context,
                        (Some(o), Some(t)) => {
                            if whitespace_mode != DiffWhitespaceMode::Show
                                && texts_equal_ignoring_whitespace(o, t)
                            {
                                RK::Context
                            } else {
                                RK::Modify
                            }
                        }
                        (Some(_), None) => RK::Remove,
                        (None, Some(_)) => RK::Add,
                        // Padding-only row (e.g. a base-only run): blank.
                        (None, None) => RK::Context,
                    };

                    // Word highlights are precomputed once per rebuild (shared by
                    // both columns); look up this aligned row and take the current
                    // side's ranges. Cloned into an owned Vec so it doesn't hold a
                    // borrow of `this` across the `&mut this` cache use below.
                    let word_ranges_owned: Vec<Range<usize>> =
                        if styling_enabled && matches!(visual_kind, RK::Modify) {
                            this.conflict_resolver
                                .two_way_aligned_word_highlights
                                .get(&row)
                                .map(|(o, n)| match side {
                                    ConflictPickSide::Ours => o.as_slice().to_vec(),
                                    ConflictPickSide::Theirs => n.as_slice().to_vec(),
                                })
                                .unwrap_or_default()
                        } else {
                            Vec::new()
                        };
                    let word_ranges: &[Range<usize>] = &word_ranges_owned;

                    let (side_line, side_text) = match side {
                        ConflictPickSide::Ours => (ours_line, ours_text),
                        ConflictPickSide::Theirs => (theirs_line, theirs_text),
                    };
                    let has_text = side_text.is_some();
                    // kdiff3 behavior: per-column line numbers from the
                    // side's own file; padding rows have none.
                    let line_no_opt = side_line
                        .filter(|_| has_text)
                        .and_then(|l| u32::try_from(l + 1).ok());

                    let styled_result = Self::conflict_split_row_styled(
                        theme,
                        &mut this.conflict_diff_segments_cache_split,
                        &mut this.conflict_diff_query_segments_cache_split,
                        row,
                        side,
                        side_text,
                        word_ranges,
                        query,
                        query_options,
                        query_matcher.as_ref(),
                        syntax_lang,
                        syntax_mode,
                        prepared_diff_syntax_line_for_one_based_line(document, line_no_opt),
                    );
                    if styled_result.pending {
                        needs_chunk_poll = true;
                    }
                    let styled = styled_result.resolve(
                        &this.conflict_diff_segments_cache_split,
                        &this.conflict_diff_query_segments_cache_split,
                        (row, side),
                    );

                    let text = SharedString::new(side_text.unwrap_or_default());
                    let bg = split_cell_bg(theme, visual_kind, side);
                    let fg = if has_text {
                        theme.colors.foreground.primary
                    } else {
                        theme.colors.foreground.secondary
                    };
                    let display_text = conflict_display_text(&text, styled, show_ws);
                    let show_line_numbers = this.mergetool_show_line_numbers;
                    let min_width = conflict_input_row_min_width(
                        window,
                        &display_text,
                        editor_font_family.as_str(),
                        show_line_numbers,
                    );

                    let conflict_ix = this
                        .conflict_resolver
                        .conflict_index_for_side_line(column, row);
                    let semantic_nav_target =
                        this.conflict_resolver.nav_target_index_for_aligned_row(row);
                    let is_active_conflict = this.conflict_resolver.conflict_is_active(conflict_ix)
                        || this
                            .conflict_resolver
                            .selected_nav_target_contains_aligned_row(row);
                    let row_selected = this.conflict_resolver.conflict_row_is_selected(row);
                    let row_selection_enabled =
                        this.conflict_resolver.conflict_row_selection_enabled();

                    if this.conflict_canvas_rows_enabled {
                        let chunk_context = conflict_ix.map(|conflict_ix| ConflictChunkContext {
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
                            row,
                            min_width,
                            show_line_numbers,
                            line_number_string(line_no_opt),
                            bg,
                            fg,
                            text,
                            styled,
                            show_ws,
                            chunk_context,
                            chunk_menu_prefix,
                            false,
                            semantic_nav_target,
                            is_active_conflict,
                            row_selection_enabled.then_some(row_selected),
                            // The two-way split shows ours/theirs only; a pin
                            // needs all three source columns to place it.
                            None,
                        ));
                        continue;
                    }

                    let mut cell = div()
                        .id((div_id_prefix, vi))
                        .relative()
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
                        .when(is_active_conflict, |d| {
                            d.child(
                                div()
                                    .absolute()
                                    .left_0()
                                    .top_0()
                                    .bottom_0()
                                    .w(px(3.0))
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
                                line_number_string(line_no_opt),
                            ))
                        })
                        .child(conflict_diff_text_cell(text.clone(), styled, show_ws));

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
                            two_way_aligned_input_row_menu_targets(row, conflict_ix, side);
                        if row_selection_enabled {
                            cell = cell
                                .on_mouse_down(
                                    MouseButton::Left,
                                    cx.listener(move |this, e: &MouseDownEvent, _window, cx| {
                                        if e.modifiers.shift || e.modifiers.control {
                                            this.conflict_resolver_click_row_selection(
                                                conflict_ix,
                                                row,
                                                e.modifiers,
                                                cx,
                                            );
                                        } else {
                                            this.conflict_resolver_begin_row_selection(
                                                conflict_ix,
                                                row,
                                                cx,
                                            );
                                        }
                                    }),
                                )
                                .on_mouse_move(cx.listener(
                                    move |this, _e: &MouseMoveEvent, _window, cx| {
                                        this.conflict_resolver_extend_row_selection(
                                            conflict_ix,
                                            row,
                                            cx,
                                        );
                                    },
                                ));
                        } else {
                            cell = cell.on_mouse_down(
                                MouseButton::Left,
                                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                                    // section 30: clicking a conflict block body selects it.
                                    this.conflict_resolver_select_conflict(conflict_ix, cx);
                                }),
                            );
                        }
                        cell = cell.on_mouse_down(
                            MouseButton::Right,
                            cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                                cx.stop_propagation();
                                if e.modifiers.shift {
                                    let invoker: SharedString =
                                        format!("{}_{}_{}", input_menu_prefix, conflict_ix, row)
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
                                        format!("{}_{}_{}", chunk_menu_prefix, conflict_ix, row)
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
                    } else if let Some(target_index) = semantic_nav_target {
                        cell = cell.cursor(CursorStyle::PointingHand).on_mouse_down(
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
        if needs_chunk_poll {
            this.ensure_prepared_syntax_chunk_poll(cx);
        }
        elements
    }

    /// Diff-view-style collapsed context fold row (section 30, R6): muted band,
    /// reveal arrows clustered where the line-number gutter sits, and a
    /// left-aligned hidden-range label; clicking elsewhere expands the whole
    /// fold. `output_pane` selects which fold-reveal state the controls
    /// mutate (source columns vs resolved output).
    #[allow(clippy::too_many_arguments)]
    fn conflict_context_fold_row(
        theme: AppTheme,
        id_prefix: &'static str,
        vi: usize,
        source_line_start: usize,
        len: usize,
        fold_id: usize,
        output_pane: bool,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let first_line = source_line_start + 1;
        let last_line = source_line_start + len;
        let label: SharedString =
            format!("⋯ {len} unchanged lines ({first_line}–{last_line})").into();
        let fold_bg = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.14 } else { 0.10 },
        );
        let reveal_btn = |id_suffix: &'static str,
                          icon: &'static str,
                          tooltip: &'static str,
                          from_top: bool,
                          cx: &mut gpui::Context<Self>| {
            div()
                .id((id_suffix, vi))
                .w(px(18.0))
                .h(px(18.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(theme.radii.row))
                .cursor(CursorStyle::PointingHand)
                .hover(move |style| {
                    style.bg(with_alpha(theme.colors.interaction.hover_background, 0.55))
                })
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                        cx.stop_propagation();
                        if output_pane {
                            this.conflict_resolver_reveal_output_context_fold(
                                fold_id, from_top, cx,
                            );
                        } else {
                            this.conflict_resolver_reveal_context_fold(fold_id, from_top, cx);
                        }
                    }),
                )
                .child(svg_icon(icon, theme.colors.foreground.secondary, px(10.0)))
                .gitcomet_tooltip(theme, tooltip.into())
        };
        div()
            .id((id_prefix, vi))
            .w_full()
            .h(px(20.0))
            .px_2()
            .flex()
            .items_center()
            .gap_2()
            .bg(fold_bg)
            .text_xs()
            .text_color(theme.colors.foreground.secondary)
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .child(reveal_btn(
                        "conflict_fold_reveal_top",
                        "icons/arrow_down.svg",
                        "Reveal 20 more lines from the top of this fold",
                        true,
                        cx,
                    ))
                    .child(reveal_btn(
                        "conflict_fold_reveal_bottom",
                        "icons/arrow_up.svg",
                        "Reveal 20 more lines from the bottom of this fold",
                        false,
                        cx,
                    )),
            )
            .child(label)
            .cursor(CursorStyle::PointingHand)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _e: &MouseDownEvent, _window, cx| {
                    cx.stop_propagation();
                    if output_pane {
                        this.conflict_resolver_expand_output_context_fold(fold_id, cx);
                    } else {
                        this.conflict_resolver_expand_context_fold(fold_id, cx);
                    }
                }),
            )
            .gitcomet_tooltip(theme, "Expand all hidden lines".into())
            .into_any_element()
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
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let line_count = this.conflict_resolved_preview_line_count;

        if this.conflict_resolver.resolved_outline_gutter_rows.len() != line_count {
            let meta = &this.conflict_resolver.resolved_outline.meta;
            let markers = &this.conflict_resolver.resolved_outline.markers;
            let line_starts = &this.conflict_resolved_preview_line_starts;
            // A placeholder row is unresolved by definition, so read that off
            // the row's own text rather than trusting the marker array, which
            // is rebuilt incrementally and can lag a resolve/unresolve.
            let placeholder_rows: Vec<bool> =
                this.conflict_resolver_input.read_with(cx, |input, _| {
                    let text = input.text();
                    (0..line_count)
                        .map(|ix| {
                            conflict_resolver::line_is_unresolved_conflict_placeholder(
                                resolved_output_line_text(text, line_starts, ix),
                            )
                        })
                        .collect()
                });
            let mut gutter_rows = Vec::with_capacity(line_count);
            for ix in 0..line_count {
                let source = meta
                    .get(ix)
                    .map(|entry| entry.source)
                    .unwrap_or(conflict_resolver::ResolvedLineSource::Manual);
                let marker = markers.get(ix).copied().flatten();
                let row = conflict_resolver::ResolvedOutputGutterRow::new(
                    source,
                    marker.map(|entry| entry.conflict_ix),
                    marker.is_some_and(|entry| entry.is_start),
                    marker.is_some_and(|entry| entry.is_end),
                    marker.is_some_and(|entry| entry.unresolved),
                );
                let is_placeholder = placeholder_rows.get(ix).copied().unwrap_or(false);
                gutter_rows.push(if is_placeholder {
                    row.with_unresolved_placeholder()
                } else {
                    row
                });
            }
            this.conflict_resolver.resolved_outline_gutter_rows = gutter_rows;
        }

        let fold_bg = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.14 } else { 0.10 },
        );
        // Line-number cell sized to this file's digit count so short numbers sit
        // snug against the marker lane; the gutter container width tracks it.
        let line_no_w = resolved_output_line_no_width(line_count);
        let elements: Vec<AnyElement> = range
            .map(|vi| {
                // Collapsed context mode projects the output row space; map
                // each visible row to its line (folds render a matching band).
                let ix = match this.resolved_output_item_for_visible(vi) {
                    Some(conflict_resolver::ThreeWayVisibleItem::Line(line)) => line,
                    Some(conflict_resolver::ThreeWayVisibleItem::CollapsedContext { .. }) => {
                        return div()
                            .id(("conflict_resolved_preview_fold", vi))
                            .h(px(20.0))
                            .w_full()
                            .bg(fold_bg)
                            .into_any_element();
                    }
                    Some(conflict_resolver::ThreeWayVisibleItem::CollapsedBlock(_)) | None => {
                        return div()
                            .id(("conflict_resolved_preview_oob", vi))
                            .h(px(20.0))
                            .px_2()
                            .text_xs()
                            .text_color(theme.colors.foreground.secondary)
                            .child("")
                            .into_any_element();
                    }
                };

                let gutter_row = this
                    .conflict_resolver
                    .resolved_outline_gutter_rows
                    .get(ix)
                    .copied()
                    .unwrap_or_default();
                let source = gutter_row.source();
                let (_, badge_fg) = resolved_output_source_badge_colors(theme, source);
                // section 30 R11: outside marker regions the badge is provenance of
                // a line git itself pre-merged (or plain context), not a
                // resolver pick — mute it so only real picks read as
                // decisions.
                let badge_fg = if gutter_row.has_marker() && gutter_row.unresolved() {
                    theme.colors.status.danger.foreground
                } else if gutter_row.has_marker() {
                    badge_fg
                } else {
                    with_alpha(badge_fg, if theme.is_dark { 0.45 } else { 0.55 })
                };
                let conflict_ix = gutter_row.marker_conflict_ix();
                let conflict_active = this.conflict_resolver.conflict_is_active(conflict_ix);
                let conflict_unresolved = gutter_row.unresolved();
                let marker_color = if conflict_unresolved {
                    with_alpha(
                        theme.colors.status.danger.foreground,
                        if theme.is_dark { 0.96 } else { 0.90 },
                    )
                } else if conflict_active {
                    with_alpha(
                        theme.colors.accent.foreground,
                        if theme.is_dark { 0.92 } else { 0.84 },
                    )
                } else {
                    with_alpha(
                        theme.colors.status.success.foreground,
                        if theme.is_dark { 0.82 } else { 0.72 },
                    )
                };
                let marker_lane = div()
                    .w(px(12.0))
                    .mr_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(gutter_row.has_marker(), |d| {
                        d.child(
                            div()
                                .relative()
                                .w(px(2.0))
                                .h_full()
                                .bg(marker_color)
                                .when(gutter_row.is_start(), |d| {
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
                                .when(gutter_row.is_end(), |d| {
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
                    .h(px(crate::view::panes::main::RESOLVED_OUTPUT_ROW_HEIGHT_PX))
                    .px_2()
                    .flex()
                    .items_center()
                    .text_xs()
                    .font_family(editor_font_family.clone())
                    .text_color(theme.colors.foreground.primary)
                    // The active conflict's open row wears the same yellow wash
                    // the editor paints behind its `<Merge Conflict>` text, so
                    // the gutter and the code read as one highlighted row.
                    .when(conflict_active && conflict_unresolved, |d| {
                        d.bg(
                            crate::view::panes::main::resolved_output_active_conflict_background(
                                theme,
                            ),
                        )
                    })
                    .when(gutter_row.manual_without_marker(), |d| {
                        d.bg(with_alpha(
                            theme.colors.surface.raised,
                            if theme.is_dark { 0.18 } else { 0.12 },
                        ))
                    })
                    .child(marker_lane)
                    .when(this.mergetool_show_line_numbers, |d| {
                        // half the marker gap between the number and the badge;
                        // right-align so short numbers hug the badge instead of
                        // leaving a wide empty stretch inside the cell.
                        d.child(
                            div()
                                .w(line_no_w)
                                .mr_1()
                                .flex()
                                .justify_end()
                                .text_color(theme.colors.foreground.secondary)
                                .child(line_number_string(u32::try_from(ix + 1).ok())),
                        )
                    })
                    .child({
                        // section 30: confidence dot on the first row of an
                        // auto-resolved conflict (accent/warning/danger for
                        // high/medium/low). Rule detail for the active
                        // conflict shows in the resolver header trace label.
                        let confidence = conflict_ix
                            .filter(|_| gutter_row.is_start() && !conflict_unresolved)
                            .and_then(|cix| this.conflict_autosolve_confidence_for_ix(cix));
                        div()
                            .w(px(24.0))
                            .relative()
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
                                    .child(gutter_row.badge_char().to_string()),
                            )
                            .when_some(confidence, |d, confidence| {
                                use gitcomet_core::conflict_session::AutosolveConfidence;
                                let dot_color = match confidence {
                                    AutosolveConfidence::High => theme.colors.accent.foreground,
                                    AutosolveConfidence::Medium => {
                                        theme.colors.status.warning.foreground
                                    }
                                    AutosolveConfidence::Low => {
                                        theme.colors.status.danger.foreground
                                    }
                                };
                                d.child(
                                    div()
                                        .absolute()
                                        .top(px(1.0))
                                        .right(px(0.0))
                                        .w(px(5.0))
                                        .h(px(5.0))
                                        .rounded(px(2.5))
                                        .bg(dot_color),
                                )
                            })
                    });
                if let Some(conflict_ix) = conflict_ix {
                    let has_base = this
                        .conflict_resolver
                        .conflict_has_base
                        .get(conflict_ix)
                        .copied()
                        .unwrap_or(false);
                    let is_three_way =
                        this.conflict_resolver.view_mode == ConflictResolverViewMode::ThreeWay;
                    let selected_choices =
                        this.conflict_resolver_selected_choices_for_conflict_ix(conflict_ix);
                    let context_menu_invoker: SharedString =
                        format!("resolver_output_chunk_menu_{}_{}", conflict_ix, ix).into();
                    row = row.on_mouse_down(
                        MouseButton::Right,
                        cx.listener(move |this, e: &MouseDownEvent, window, cx| {
                            cx.stop_propagation();
                            this.open_conflict_resolver_chunk_context_menu(
                                context_menu_invoker.clone(),
                                conflict_ix,
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
        let editor_font_family = crate::font_preferences::current_editor_font_family(cx);
        let show_ws = this.reveal_whitespace_chars;
        if let Some(projection) = this.conflict_resolved_output_projection.as_ref() {
            let unresolved_row_bg = with_alpha(
                theme.colors.status.danger.foreground,
                if theme.is_dark { 0.18 } else { 0.10 },
            );
            let active_unresolved_row_bg =
                crate::view::panes::main::resolved_output_active_conflict_background(theme);
            let resolved_row_bg = with_alpha(
                theme.colors.status.success.foreground,
                if theme.is_dark { 0.12 } else { 0.08 },
            );
            let line_count = this.conflict_resolved_preview_line_count;
            let mut elements = Vec::with_capacity(requested_rows);

            let push_row = |ix: usize, line: &str| {
                let line_text = if show_ws {
                    whitespace_visible_line_text(line)
                } else {
                    SharedString::new(line)
                };
                let min_width = conflict_resolved_output_row_min_width(
                    window,
                    &line_text,
                    editor_font_family.as_str(),
                );

                let conflict_marker = this
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .get(ix)
                    .copied()
                    .flatten();
                let row_bg = conflict_marker.map(|marker| {
                    if !marker.unresolved {
                        resolved_row_bg
                    } else if this
                        .conflict_resolver
                        .conflict_is_active(Some(marker.conflict_ix))
                    {
                        // Same yellow the editable output washes its active row
                        // with: which open conflict the picks apply to.
                        active_unresolved_row_bg
                    } else {
                        unresolved_row_bg
                    }
                });
                let text_color = if conflict_marker.is_some_and(|marker| marker.unresolved) {
                    theme.colors.status.danger.foreground
                } else {
                    theme.colors.foreground.primary
                };

                elements.push(
                    div()
                        .id(("conflict_resolved_output_row", ix))
                        .w_full()
                        .min_w(min_width)
                        .h(px(20.0))
                        .px_2()
                        .flex()
                        .items_center()
                        .text_xs()
                        .font_family(editor_font_family.clone())
                        .text_color(text_color)
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
                        .into_any_element(),
                );
            };

            let visible_end = range.end.min(line_count);
            if range.start < visible_end {
                projection.for_each_line_text_in_range(
                    &this.conflict_resolver.marker_segments,
                    range.start..visible_end,
                    push_row,
                );
            }

            for ix in range.start.max(visible_end)..range.end {
                elements.push(
                    div()
                        .id(("conflict_resolved_output_oob", ix))
                        .h(px(20.0))
                        .px_2()
                        .text_xs()
                        .text_color(theme.colors.foreground.secondary)
                        .child("")
                        .into_any_element(),
                );
            }
            perf::record_row_batch(
                ViewPerfRenderLane::ResolvedPreview,
                requested_rows,
                elements.len(),
            );
            return elements;
        }

        // Unreachable: this list is only mounted when the output is streamed
        // (`conflict_resolver_view.rs`, inside `if streamed`), and `streamed` is
        // exactly `conflict_resolved_output_projection.is_some()` — the branch
        // above. The editable output is drawn by the `TextInput` instead, with
        // `render_conflict_resolved_preview_rows` supplying only its gutter.
        perf::record_row_batch(ViewPerfRenderLane::ResolvedPreview, requested_rows, 0);
        Vec::new()
    }

    pub(in super::super) fn render_conflict_compare_diff_rows(
        this: &mut Self,
        range: Range<usize>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Vec<AnyElement> {
        let query = this.diff_search_query_or_empty();
        let query_options = this.diff_search_options_or_default();
        let query = query.as_ref().to_string();
        this.sync_conflict_diff_query_overlay_caches(query.as_str(), query_options);
        let query_matcher = conflict_diff_query_matcher(query.as_str(), query_options);
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
                        .text_color(this.theme.colors.foreground.secondary)
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
                    query_matcher.as_ref(),
                    cx,
                )
            })
            .collect()
    }

    #[allow(clippy::too_many_arguments)]
    fn conflict_split_row_styled(
        theme: AppTheme,
        stable_cache: &mut conflict_resolver::ConflictSplitStyledTextCache,
        query_cache: &mut conflict_resolver::ConflictSplitStyledTextCache,
        row_ix: usize,
        side: ConflictPickSide,
        text: Option<&str>,
        word_ranges: &[Range<usize>],
        query: &str,
        _query_options: DiffSearchOptions,
        query_matcher: Option<&DiffSearchMatcher>,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
        prepared_line: PreparedDiffSyntaxLine,
    ) -> ConflictRowStyledText {
        let Some(text) = text else {
            return ConflictRowStyledText::default();
        };
        let source_identity = Some(DiffTextSourceIdentity::from_str(text));
        let key = (row_ix, side);
        let mut result = ConflictRowStyledText::default();
        if text.is_empty() {
            return result;
        }

        let query_active = !query.is_empty();
        let base_has_style = !word_ranges.is_empty() || syntax_lang.is_some();

        if base_has_style {
            if let Some(cached) = stable_cache.get(&key) {
                let _ = cached;
                result.styled = Some(ConflictRowStyledTextValue::StableCached);
            } else {
                let (styled, pending) = build_conflict_row_base_styled(
                    theme,
                    text,
                    source_identity,
                    word_ranges,
                    syntax_lang,
                    syntax_mode,
                    prepared_line,
                )
                .into_parts();
                if !pending {
                    stable_cache.insert(key, styled);
                    result.styled = Some(ConflictRowStyledTextValue::StableCached);
                } else {
                    result.styled = Some(ConflictRowStyledTextValue::Owned(styled));
                }
                result.pending = pending;
            }
        }

        if query_active {
            let Some(query_matcher) = query_matcher else {
                return result;
            };
            if !result.pending
                && let Some(cached) = query_cache.get(&key)
            {
                let _ = cached;
                result.styled = Some(ConflictRowStyledTextValue::QueryCached);
                return result;
            }

            let styled = if let Some(base) = match result.styled.as_ref() {
                Some(ConflictRowStyledTextValue::Owned(styled)) => Some(styled),
                _ => stable_cache.get(&key),
            } {
                build_cached_diff_query_overlay_styled_text(theme, base, query_matcher)
            } else {
                let base = build_conflict_cached_diff_styled_text_with_source_identity(
                    theme,
                    text,
                    source_identity,
                    word_ranges,
                    "",
                    syntax_lang,
                    syntax_mode,
                    None,
                );
                build_cached_diff_query_overlay_styled_text(theme, &base, query_matcher)
            };
            if !result.pending {
                query_cache.insert(key, styled);
                result.styled = Some(ConflictRowStyledTextValue::QueryCached);
            } else {
                result.styled = Some(ConflictRowStyledTextValue::Owned(styled));
            }
        }

        result
    }

    fn render_conflict_compare_split_row(
        &mut self,
        visible_row_ix: usize,
        row_ix: usize,
        row: gitcomet_core::file_diff::FileDiffRow,
        syntax_lang: Option<DiffSyntaxLanguage>,
        syntax_mode: DiffSyntaxMode,
        query_matcher: Option<&DiffSearchMatcher>,
        cx: &mut gpui::Context<Self>,
    ) -> AnyElement {
        let theme = self.theme;
        let show_ws = self.reveal_whitespace_chars;

        let left_text = SharedString::new(row.old.as_deref().unwrap_or_default());
        let right_text = SharedString::new(row.new.as_deref().unwrap_or_default());
        let ours_document = self.conflict_three_way_prepared_syntax_documents.ours;
        let theirs_document = self.conflict_three_way_prepared_syntax_documents.theirs;
        let visual_kind = self.conflict_resolver.two_way_split_visual_kind_at(
            row_ix,
            &row,
            self.diff_whitespace_mode,
        );

        // Large streamed compare views should avoid retaining per-row styled
        // caches as users scroll through the whole-file projection.
        let styling_enabled = self.conflict_row_styling_enabled()
            && self.conflict_resolver.three_way_len
                <= conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES;
        let word_hl = if styling_enabled
            && !matches!(
                visual_kind,
                gitcomet_core::file_diff::FileDiffRowKind::Context
            ) {
            self.conflict_resolver
                .two_way_split_word_highlight_for_row(row_ix, &row)
        } else {
            None
        };
        let old_word_ranges = word_hl
            .as_ref()
            .map(|pair| pair.0.as_slice())
            .unwrap_or(&[]);
        let new_word_ranges = word_hl
            .as_ref()
            .map(|pair| pair.1.as_slice())
            .unwrap_or(&[]);
        let query_text = self.conflict_diff_query_cache_query.clone();
        let query_options = self.conflict_diff_query_cache_options;
        let query = query_text.as_ref();
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
                    query_options,
                    query_matcher,
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
                    query_options,
                    query_matcher,
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
        let left_styled = left_styled.resolve(
            &self.conflict_diff_segments_cache_split,
            &self.conflict_diff_query_segments_cache_split,
            (row_ix, ConflictPickSide::Ours),
        );
        let right_styled = right_styled.resolve(
            &self.conflict_diff_segments_cache_split,
            &self.conflict_diff_query_segments_cache_split,
            (row_ix, ConflictPickSide::Theirs),
        );

        let left_bg = split_cell_bg(theme, visual_kind, ConflictPickSide::Ours);
        let right_bg = split_cell_bg(theme, visual_kind, ConflictPickSide::Theirs);

        let [left_col_w, right_col_w] = self.conflict_diff_split_col_widths;
        let left_fg = if row.old.is_some() {
            theme.colors.foreground.primary
        } else {
            theme.colors.foreground.secondary
        };
        let right_fg = if row.new.is_some() {
            theme.colors.foreground.primary
        } else {
            theme.colors.foreground.secondary
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
                self.mergetool_show_line_numbers,
                line_number_string(row.old_line),
                line_number_string(row.new_line),
                left_bg,
                right_bg,
                left_fg,
                right_fg,
                left_text,
                right_text,
                left_styled,
                right_styled,
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
            .when(self.mergetool_show_line_numbers, |d| {
                d.child(conflict_diff_line_number_cell(
                    theme,
                    line_number_string(row.old_line),
                ))
            })
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
            .text_color(right_fg)
            .whitespace_nowrap()
            .overflow_hidden()
            .when(self.mergetool_show_line_numbers, |d| {
                d.child(conflict_diff_line_number_cell(
                    theme,
                    line_number_string(row.new_line),
                ))
            })
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
                    .child(div().w(px(1.0)).h_full().bg(theme.colors.stroke.default)),
            )
            .child(right)
            .into_any_element()
    }
}

/// A diff-column line-number cell: fixed width, vertically centered, with a
/// full-height right divider separating the number gutter from the code —
/// matching the resolved-output gutter's separator.
fn conflict_diff_line_number_cell(theme: AppTheme, line_no: SharedString) -> gpui::Div {
    div()
        .w(px(super::CONFLICT_DIFF_LINE_NO_WIDTH_PX))
        .h_full()
        .flex()
        .items_center()
        .border_r_1()
        .border_color(theme.colors.stroke.default)
        .text_color(theme.colors.foreground.secondary)
        .child(line_no)
}

fn conflict_diff_text_cell(
    text: SharedString,
    styled: Option<&CachedDiffStyledText>,
    reveal_whitespace_chars: bool,
) -> AnyElement {
    let Some(styled) = styled else {
        let display = if reveal_whitespace_chars {
            whitespace_visible_line_text(text.as_ref())
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
        let display = if reveal_whitespace_chars {
            whitespace_visible_line_text(text.as_ref())
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

    if reveal_whitespace_chars {
        let visible = whitespace_visible_line_styled_text_for_raw(styled, text.as_ref());
        if visible.highlights.is_empty() {
            return div()
                .flex_1()
                .min_w(px(0.0))
                .overflow_hidden()
                .child(visible.text)
                .into_any_element();
        }
        let visible_text = visible.text;
        let visible_highlights = visible.highlights;
        return div()
            .flex_1()
            .min_w(px(0.0))
            .overflow_hidden()
            .child(
                gpui::StyledText::new(visible_text)
                    .with_highlights(visible_highlights.iter().cloned()),
            )
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

#[cfg(test)]
fn whitespace_visible_text(text: &str) -> SharedString {
    whitespace_visible_text_and_highlights(text, &[]).0
}

#[cfg(test)]
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
            with_alpha(
                theme.colors.accent.foreground,
                if theme.is_dark { 0.68 } else { 0.56 },
            ),
            theme.colors.accent.foreground,
        ),
        conflict_resolver::ResolvedLineSource::B => (
            with_alpha(
                theme.colors.status.success.foreground,
                if theme.is_dark { 0.68 } else { 0.56 },
            ),
            theme.colors.status.success.foreground,
        ),
        conflict_resolver::ResolvedLineSource::C => (
            with_alpha(
                theme.colors.status.warning.foreground,
                if theme.is_dark { 0.68 } else { 0.56 },
            ),
            theme.colors.status.warning.foreground,
        ),
        conflict_resolver::ResolvedLineSource::Manual => (
            with_alpha(
                theme.colors.foreground.secondary,
                if theme.is_dark { 0.48 } else { 0.42 },
            ),
            theme.colors.foreground.secondary,
        ),
    }
}

fn three_way_choice_short_label(choice: conflict_resolver::ConflictChoice) -> &'static str {
    match choice {
        conflict_resolver::ConflictChoice::Base => "A",
        conflict_resolver::ConflictChoice::Ours => "B",
        conflict_resolver::ConflictChoice::Theirs => "C",
        conflict_resolver::ConflictChoice::Both => "B+C",
        _ => "ordered",
    }
}

fn two_way_side_label(side: ConflictPickSide) -> &'static str {
    match side {
        ConflictPickSide::Ours => "A",
        ConflictPickSide::Theirs => "B",
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

/// Input-row menu targets for the section 30 aligned two-way view. `row_ix` is an
/// aligned visual row (shared by both columns), so the line pick reuses the
/// aligned-row-space `ThreeWayLine` target with this side's choice.
fn two_way_aligned_input_row_menu_targets(
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
        ResolverPickTarget::ThreeWayLine {
            line_ix: row_ix,
            choice,
        },
        format!("Pick this chunk ({side_label})").into(),
        ResolverPickTarget::Chunk {
            conflict_ix,
            choice,
            output_line_ix: None,
        },
    )
}

/// Whether two lines are equal once all whitespace is removed. Matches the
/// block-local `append_conflict_row_without_whitespace` semantics used to
/// downgrade whitespace-only differences to context rows.
fn texts_equal_ignoring_whitespace(a: &str, b: &str) -> bool {
    a.chars()
        .filter(|ch| !ch.is_whitespace())
        .eq(b.chars().filter(|ch| !ch.is_whitespace()))
}

fn split_cell_bg(
    theme: AppTheme,
    kind: gitcomet_core::file_diff::FileDiffRowKind,
    side: ConflictPickSide,
) -> gpui::Rgba {
    // Side-identity colours, matching the three-way view: Ours = success
    // (green), Theirs = accent (blue). A cell is tinted only when that side
    // actually has changed content on the row (Ours: Remove/Modify, Theirs:
    // Add/Modify), so unchanged padding stays transparent.
    match (kind, side) {
        (gitcomet_core::file_diff::FileDiffRowKind::Add, ConflictPickSide::Theirs)
        | (gitcomet_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Theirs) => {
            with_alpha(
                theme.colors.accent.foreground,
                if theme.is_dark { 0.14 } else { 0.10 },
            )
        }
        (gitcomet_core::file_diff::FileDiffRowKind::Remove, ConflictPickSide::Ours)
        | (gitcomet_core::file_diff::FileDiffRowKind::Modify, ConflictPickSide::Ours) => {
            with_alpha(
                theme.colors.status.success.foreground,
                if theme.is_dark { 0.10 } else { 0.08 },
            )
        }
        _ => with_alpha(theme.colors.surface.raised, 0.0),
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
    fn conflict_display_text_reveals_implicit_line_break_marker() {
        let text: SharedString = "a b\t".into();
        let display = conflict_display_text(&text, None, true);

        assert_eq!(display.as_ref(), "a·b→↵");
    }

    #[test]
    fn conflict_diff_query_matcher_preserves_significant_whitespace() {
        let space_matcher =
            conflict_diff_query_matcher(" ", DiffSearchOptions::default()).expect("space query");
        assert_eq!(space_matcher.query(), " ");
        assert!(space_matcher.is_match("a b"));

        let padded_matcher = conflict_diff_query_matcher(" foo ", DiffSearchOptions::default())
            .expect("padded query");
        assert_eq!(padded_matcher.query(), " foo ");
        assert!(padded_matcher.is_match("x foo y"));
        assert!(!padded_matcher.is_match("foo"));

        assert!(conflict_diff_query_matcher("", DiffSearchOptions::default()).is_none());
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

        assert_eq!(line_label.as_ref(), "Pick this line (A)");
        assert_eq!(chunk_label.as_ref(), "Pick this chunk (A)");
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
