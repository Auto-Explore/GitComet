use super::super::conflict_resolver;
use super::canvas::keyed_canvas;
use super::*;
use gpui::{
    App, Bounds, DispatchPhase, HighlightStyle, Pixels, Styled, TextRun, TextStyle, Window, fill,
    point, px, size,
};
use rustc_hash::FxHashMap as HashMap;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;

const DIFF_FONT_SCALE: f32 = 0.80;
const GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 16_384;
const CONFLICT_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 32_768;

type HighlightSpans = Arc<Vec<(Range<usize>, HighlightStyle)>>;
type ThreeWayColumnBounds = (
    Bounds<Pixels>,
    Bounds<Pixels>,
    Bounds<Pixels>,
    Bounds<Pixels>,
    Bounds<Pixels>,
);

thread_local! {
    static GUTTER_TEXT_LAYOUT_CACHE: RefCell<HashMap<u64, gpui::ShapedLine>> =
        RefCell::new(HashMap::default());
    static CONFLICT_TEXT_LAYOUT_CACHE: RefCell<HashMap<u64, gpui::ShapedLine>> =
        RefCell::new(HashMap::default());
}

#[derive(Clone, Debug)]
pub(super) struct ConflictChunkContext {
    pub(super) conflict_ix: usize,
    pub(super) has_base: bool,
    pub(super) selected_choices: Vec<conflict_resolver::ConflictChoice>,
}

#[derive(Clone, Debug)]
pub(super) struct ThreeWayCanvasColumn {
    pub(super) line_no: SharedString,
    pub(super) bg: gpui::Rgba,
    pub(super) fg: gpui::Rgba,
    pub(super) text: SharedString,
}

#[derive(Clone, Debug, Default)]
pub(super) struct ThreeWayChunkContext {
    pub(super) base: Option<ConflictChunkContext>,
    pub(super) ours: Option<ConflictChunkContext>,
    pub(super) theirs: Option<ConflictChunkContext>,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn split_conflict_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    visible_row_ix: usize,
    row_ix: usize,
    min_width: Pixels,
    left_target_width: Pixels,
    right_target_width: Pixels,
    left_line_no: SharedString,
    right_line_no: SharedString,
    left_bg: gpui::Rgba,
    right_bg: gpui::Rgba,
    left_fg: gpui::Rgba,
    right_fg: gpui::Rgba,
    left_text: SharedString,
    right_text: SharedString,
    left_styled: Option<&CachedDiffStyledText>,
    right_styled: Option<&CachedDiffStyledText>,
    show_whitespace: bool,
    chunk_context: Option<ConflictChunkContext>,
) -> AnyElement {
    let left_prepared = prepare_conflict_text_for_canvas(left_text, left_styled, show_whitespace);
    let right_prepared =
        prepare_conflict_text_for_canvas(right_text, right_styled, show_whitespace);

    keyed_canvas(
        ("conflict_resolver_split_row_canvas", visible_row_ix),
        move |bounds, _window, _cx| {
            let handle_width = px(PANE_RESIZE_HANDLE_PX);
            let (left_col, handle_bounds, right_col) = split_columns_with_widths(
                bounds,
                left_target_width,
                right_target_width,
                handle_width,
            );
            SplitRowPrepaintState {
                left_col,
                handle_bounds,
                right_col,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);
            let pad = px_2(window);
            let gap = pad;

            window.paint_quad(fill(prepaint.left_col, left_bg));
            window.paint_quad(fill(prepaint.right_col, right_bg));

            let divider_x = prepaint.handle_bounds.left()
                + ((prepaint.handle_bounds.size.width - px(1.0)).max(px(0.0)) * 0.5).floor();
            window.paint_quad(fill(
                Bounds::new(
                    point(divider_x, prepaint.handle_bounds.top()),
                    size(px(1.0), prepaint.handle_bounds.size.height),
                ),
                theme.colors.border,
            ));

            paint_gutter_text(
                &left_line_no,
                prepaint.left_col.left() + pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &right_line_no,
                prepaint.right_col.left() + pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );

            let left_text_bounds = split_column_text_bounds(prepaint.left_col, pad, gap);
            let right_text_bounds = split_column_text_bounds(prepaint.right_col, pad, gap);

            window.paint_layer(left_text_bounds, |window| {
                paint_conflict_text(
                    left_text_bounds,
                    left_fg,
                    y,
                    line_metrics,
                    &left_prepared,
                    window,
                    cx,
                );
            });
            window.paint_layer(right_text_bounds, |window| {
                paint_conflict_text(
                    right_text_bounds,
                    right_fg,
                    y,
                    line_metrics,
                    &right_prepared,
                    window,
                    cx,
                );
            });

            if let Some(chunk_context) = chunk_context.clone() {
                let clip_bounds = window.content_mask().bounds;
                let visible_left = prepaint.left_col.intersect(&clip_bounds);
                let visible_right = prepaint.right_col.intersect(&clip_bounds);
                window.on_mouse_event({
                    let view = view.clone();
                    move |event: &gpui::MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != gpui::MouseButton::Right
                        {
                            return;
                        }

                        let invoker = if visible_left.contains(&event.position) {
                            Some::<SharedString>(
                                format!(
                                    "resolver_two_way_split_ours_chunk_menu_{}_{}",
                                    chunk_context.conflict_ix, row_ix
                                )
                                .into(),
                            )
                        } else if visible_right.contains(&event.position) {
                            Some::<SharedString>(
                                format!(
                                    "resolver_two_way_split_theirs_chunk_menu_{}_{}",
                                    chunk_context.conflict_ix, row_ix
                                )
                                .into(),
                            )
                        } else {
                            None
                        };

                        let Some(invoker) = invoker else {
                            return;
                        };

                        let conflict_ix = chunk_context.conflict_ix;
                        let has_base = chunk_context.has_base;
                        let selected_choices = chunk_context.selected_choices.clone();
                        let anchor = event.position;
                        view.update(cx, |this, cx| {
                            this.open_conflict_resolver_chunk_context_menu(
                                invoker,
                                conflict_ix,
                                has_base,
                                false,
                                selected_choices,
                                None,
                                anchor,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                });
            }
        },
    )
    .h(px(20.0))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_conflict_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    visible_row_ix: usize,
    row_ix: usize,
    min_width: Pixels,
    old_line_no: SharedString,
    new_line_no: SharedString,
    prefix: SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    text: SharedString,
    styled: Option<&CachedDiffStyledText>,
    show_whitespace: bool,
    chunk_context: Option<ConflictChunkContext>,
) -> AnyElement {
    let prepared = prepare_conflict_text_for_canvas(text, styled, show_whitespace);

    keyed_canvas(
        ("conflict_resolver_inline_row_canvas", visible_row_ix),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gap = pad;
            let text_bounds = inline_text_bounds(bounds, pad, gap);
            InlineRowPrepaintState {
                bounds,
                text_bounds,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);
            let pad = px_2(window);
            let gap = pad;
            let line_no_width = conflict_line_no_width();

            window.paint_quad(fill(bounds, bg));

            let old_line_x = bounds.left() + pad;
            let new_line_x = old_line_x + line_no_width + gap;
            let prefix_x = new_line_x + line_no_width + gap;

            paint_gutter_text(
                &old_line_no,
                old_line_x,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &new_line_no,
                new_line_x,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &prefix,
                prefix_x,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_conflict_text(
                    prepaint.text_bounds,
                    fg,
                    y,
                    line_metrics,
                    &prepared,
                    window,
                    cx,
                );
            });

            if let Some(chunk_context) = chunk_context.clone() {
                let clip_bounds = window.content_mask().bounds;
                let visible_row = prepaint.bounds.intersect(&clip_bounds);
                window.on_mouse_event({
                    let view = view.clone();
                    move |event: &gpui::MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != gpui::MouseButton::Right
                            || !visible_row.contains(&event.position)
                        {
                            return;
                        }

                        let invoker: SharedString = format!(
                            "resolver_two_way_inline_chunk_menu_{}_{}",
                            chunk_context.conflict_ix, row_ix
                        )
                        .into();

                        let conflict_ix = chunk_context.conflict_ix;
                        let has_base = chunk_context.has_base;
                        let selected_choices = chunk_context.selected_choices.clone();
                        let anchor = event.position;
                        view.update(cx, |this, cx| {
                            this.open_conflict_resolver_chunk_context_menu(
                                invoker,
                                conflict_ix,
                                has_base,
                                false,
                                selected_choices,
                                None,
                                anchor,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                });
            }
        },
    )
    .h(px(20.0))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn three_way_conflict_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    visible_row_ix: usize,
    row_ix: usize,
    min_width: Pixels,
    base_target_width: Pixels,
    ours_target_width: Pixels,
    theirs_target_width: Pixels,
    base_column: ThreeWayCanvasColumn,
    ours_column: ThreeWayCanvasColumn,
    theirs_column: ThreeWayCanvasColumn,
    base_styled: Option<&CachedDiffStyledText>,
    ours_styled: Option<&CachedDiffStyledText>,
    theirs_styled: Option<&CachedDiffStyledText>,
    show_whitespace: bool,
    chunk_context: ThreeWayChunkContext,
) -> AnyElement {
    let base_prepared =
        prepare_conflict_text_for_canvas(base_column.text, base_styled, show_whitespace);
    let ours_prepared =
        prepare_conflict_text_for_canvas(ours_column.text, ours_styled, show_whitespace);
    let theirs_prepared =
        prepare_conflict_text_for_canvas(theirs_column.text, theirs_styled, show_whitespace);

    keyed_canvas(
        ("conflict_resolver_three_way_row_canvas", visible_row_ix),
        move |bounds, _window, _cx| {
            let handle_width = px(PANE_RESIZE_HANDLE_PX);
            let (base_col, first_handle, ours_col, second_handle, theirs_col) =
                three_way_columns_with_widths(
                    bounds,
                    base_target_width,
                    ours_target_width,
                    theirs_target_width,
                    handle_width,
                );

            ThreeWayRowPrepaintState {
                base_col,
                first_handle,
                ours_col,
                second_handle,
                theirs_col,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);
            let pad = px_2(window);
            let gap = pad;

            window.paint_quad(fill(prepaint.base_col, base_column.bg));
            window.paint_quad(fill(prepaint.ours_col, ours_column.bg));
            window.paint_quad(fill(prepaint.theirs_col, theirs_column.bg));

            let first_divider_x = prepaint.first_handle.left()
                + ((prepaint.first_handle.size.width - px(1.0)).max(px(0.0)) * 0.5).floor();
            window.paint_quad(fill(
                Bounds::new(
                    point(first_divider_x, prepaint.first_handle.top()),
                    size(px(1.0), prepaint.first_handle.size.height),
                ),
                theme.colors.border,
            ));
            let second_divider_x = prepaint.second_handle.left()
                + ((prepaint.second_handle.size.width - px(1.0)).max(px(0.0)) * 0.5).floor();
            window.paint_quad(fill(
                Bounds::new(
                    point(second_divider_x, prepaint.second_handle.top()),
                    size(px(1.0), prepaint.second_handle.size.height),
                ),
                theme.colors.border,
            ));

            paint_gutter_text(
                &base_column.line_no,
                prepaint.base_col.left() + pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &ours_column.line_no,
                prepaint.ours_col.left() + pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &theirs_column.line_no,
                prepaint.theirs_col.left() + pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );

            let base_text_bounds = split_column_text_bounds(prepaint.base_col, pad, gap);
            let ours_text_bounds = split_column_text_bounds(prepaint.ours_col, pad, gap);
            let theirs_text_bounds = split_column_text_bounds(prepaint.theirs_col, pad, gap);

            window.paint_layer(base_text_bounds, |window| {
                paint_conflict_text(
                    base_text_bounds,
                    base_column.fg,
                    y,
                    line_metrics,
                    &base_prepared,
                    window,
                    cx,
                );
            });
            window.paint_layer(ours_text_bounds, |window| {
                paint_conflict_text(
                    ours_text_bounds,
                    ours_column.fg,
                    y,
                    line_metrics,
                    &ours_prepared,
                    window,
                    cx,
                );
            });
            window.paint_layer(theirs_text_bounds, |window| {
                paint_conflict_text(
                    theirs_text_bounds,
                    theirs_column.fg,
                    y,
                    line_metrics,
                    &theirs_prepared,
                    window,
                    cx,
                );
            });

            if chunk_context.base.is_some()
                || chunk_context.ours.is_some()
                || chunk_context.theirs.is_some()
            {
                let clip_bounds = window.content_mask().bounds;
                let visible_base = prepaint.base_col.intersect(&clip_bounds);
                let visible_ours = prepaint.ours_col.intersect(&clip_bounds);
                let visible_theirs = prepaint.theirs_col.intersect(&clip_bounds);
                window.on_mouse_event({
                    let view = view.clone();
                    move |event: &gpui::MouseDownEvent, phase, window, cx| {
                        if phase != DispatchPhase::Bubble
                            || event.button != gpui::MouseButton::Right
                        {
                            return;
                        }

                        let (invoker, chunk_context) = if visible_base.contains(&event.position) {
                            match chunk_context.base.as_ref() {
                                Some(context) => (
                                    Some::<SharedString>(
                                        format!(
                                            "resolver_three_way_base_chunk_menu_{}_{}",
                                            context.conflict_ix, row_ix
                                        )
                                        .into(),
                                    ),
                                    Some(context.clone()),
                                ),
                                None => (None, None),
                            }
                        } else if visible_ours.contains(&event.position) {
                            match chunk_context.ours.as_ref() {
                                Some(context) => (
                                    Some::<SharedString>(
                                        format!(
                                            "resolver_three_way_ours_chunk_menu_{}_{}",
                                            context.conflict_ix, row_ix
                                        )
                                        .into(),
                                    ),
                                    Some(context.clone()),
                                ),
                                None => (None, None),
                            }
                        } else if visible_theirs.contains(&event.position) {
                            match chunk_context.theirs.as_ref() {
                                Some(context) => (
                                    Some::<SharedString>(
                                        format!(
                                            "resolver_three_way_theirs_chunk_menu_{}_{}",
                                            context.conflict_ix, row_ix
                                        )
                                        .into(),
                                    ),
                                    Some(context.clone()),
                                ),
                                None => (None, None),
                            }
                        } else {
                            (None, None)
                        };

                        let (Some(invoker), Some(chunk_context)) = (invoker, chunk_context) else {
                            return;
                        };

                        let anchor = event.position;
                        view.update(cx, |this, cx| {
                            this.open_conflict_resolver_chunk_context_menu(
                                invoker,
                                chunk_context.conflict_ix,
                                chunk_context.has_base,
                                true,
                                chunk_context.selected_choices,
                                None,
                                anchor,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                });
            }
        },
    )
    .h(px(20.0))
    .min_w(min_width)
    .w_full()
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[derive(Clone, Debug)]
struct SplitRowPrepaintState {
    left_col: Bounds<Pixels>,
    handle_bounds: Bounds<Pixels>,
    right_col: Bounds<Pixels>,
}

#[derive(Clone, Debug)]
struct ThreeWayRowPrepaintState {
    base_col: Bounds<Pixels>,
    first_handle: Bounds<Pixels>,
    ours_col: Bounds<Pixels>,
    second_handle: Bounds<Pixels>,
    theirs_col: Bounds<Pixels>,
}

#[derive(Clone, Debug)]
struct InlineRowPrepaintState {
    bounds: Bounds<Pixels>,
    text_bounds: Bounds<Pixels>,
}

#[derive(Clone, Debug)]
struct PreparedConflictText {
    text: SharedString,
    highlights: HighlightSpans,
    text_hash: u64,
    highlights_hash: u64,
}

fn prepare_conflict_text_for_canvas(
    text: SharedString,
    styled: Option<&CachedDiffStyledText>,
    show_whitespace: bool,
) -> PreparedConflictText {
    let Some(styled) = styled else {
        let display = if show_whitespace {
            whitespace_visible_text(text.as_ref())
        } else {
            text
        };
        return PreparedConflictText {
            text_hash: hash_text(display.as_ref()),
            text: display,
            highlights: empty_highlights(),
            highlights_hash: 0,
        };
    };

    if styled.highlights.is_empty() {
        let display = if show_whitespace {
            whitespace_visible_text(styled.text.as_ref())
        } else {
            styled.text.clone()
        };
        return PreparedConflictText {
            text_hash: hash_text(display.as_ref()),
            text: display,
            highlights: empty_highlights(),
            highlights_hash: 0,
        };
    }

    if show_whitespace {
        let (display, remapped) = whitespace_visible_text_and_highlights(
            styled.text.as_ref(),
            styled.highlights.as_ref(),
        );
        let highlights = if remapped.is_empty() {
            empty_highlights()
        } else {
            Arc::new(remapped)
        };
        return PreparedConflictText {
            text_hash: hash_text(display.as_ref()),
            text: display,
            highlights,
            highlights_hash: styled.highlights_hash,
        };
    }

    PreparedConflictText {
        text: styled.text.clone(),
        highlights: Arc::clone(&styled.highlights),
        text_hash: styled.text_hash,
        highlights_hash: styled.highlights_hash,
    }
}

fn hash_text(text: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn whitespace_visible_text(text: &str) -> SharedString {
    whitespace_visible_text_and_highlights(text, &[]).0
}

fn whitespace_visible_text_and_highlights(
    text: &str,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> (SharedString, Vec<(Range<usize>, HighlightStyle)>) {
    let mut out = String::with_capacity(text.len());
    let mut byte_map = vec![0usize; text.len() + 1];

    for (start, ch) in text.char_indices() {
        byte_map[start] = out.len();
        match ch {
            ' ' => out.push('\u{00B7}'),
            '\t' => out.push('\u{2192}'),
            '\r' => out.push('\u{240D}'),
            '\n' => out.push('\u{21B5}'),
            _ if ch.is_whitespace() => out.push('\u{2420}'),
            _ => out.push(ch),
        }
        let end = start + ch.len_utf8();
        let mapped_end = out.len();
        for ix in (start + 1)..=end {
            byte_map[ix] = mapped_end;
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

#[derive(Clone, Copy, Debug)]
struct LineMetrics {
    font_size: Pixels,
    line_height: Pixels,
}

fn diff_text_style(window: &Window) -> TextStyle {
    let mut style = window.text_style();
    style.font_weight = FontWeight::NORMAL;
    style
}

fn line_metrics(window: &Window) -> LineMetrics {
    let style = diff_text_style(window);
    let font_size = style.font_size.to_pixels(window.rem_size()) * DIFF_FONT_SCALE;
    let line_height = style
        .line_height
        .to_pixels(font_size.into(), window.rem_size());
    LineMetrics {
        font_size,
        line_height,
    }
}

fn center_text_y(bounds: Bounds<Pixels>, line_height: Pixels) -> Pixels {
    let extra = (bounds.size.height - line_height).max(px(0.0));
    bounds.top() + extra * 0.5
}

fn px_2(window: &Window) -> Pixels {
    window.rem_size() * 0.5
}

fn split_columns_with_widths(
    bounds: Bounds<Pixels>,
    left_target_width: Pixels,
    right_target_width: Pixels,
    handle_width: Pixels,
) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    let width = bounds.size.width.max(px(0.0));
    let mut left_w = left_target_width
        .min((width - handle_width).max(px(0.0)))
        .max(px(0.0));

    let mut right_w = (width - left_w - handle_width).max(px(0.0));
    if right_w < right_target_width {
        let deficit = right_target_width - right_w;
        let left_shrink = left_w.min(deficit);
        left_w -= left_shrink;
        right_w = (right_w + left_shrink).max(px(0.0));
    }

    let left = Bounds::new(bounds.origin, size(left_w, bounds.size.height));
    let handle = Bounds::new(
        point(bounds.left() + left_w, bounds.top()),
        size(handle_width, bounds.size.height),
    );
    let right = Bounds::new(
        point(handle.right(), bounds.top()),
        size(right_w, bounds.size.height),
    );
    (left, handle, right)
}

fn three_way_columns_with_widths(
    bounds: Bounds<Pixels>,
    base_target_width: Pixels,
    ours_target_width: Pixels,
    theirs_target_width: Pixels,
    handle_width: Pixels,
) -> ThreeWayColumnBounds {
    let width = bounds.size.width.max(px(0.0));
    let handles_total = handle_width * 2.0;
    let available = (width - handles_total).max(px(0.0));

    let min_total = base_target_width + ours_target_width + theirs_target_width;
    let (base_w, ours_w, theirs_w) = if available >= min_total {
        (
            base_target_width.max(px(0.0)),
            ours_target_width.max(px(0.0)),
            (available - base_target_width - ours_target_width).max(px(0.0)),
        )
    } else if available <= px(0.0) {
        (px(0.0), px(0.0), px(0.0))
    } else {
        let scale = available / min_total.max(px(1.0));
        let mut base = (base_target_width * scale).max(px(0.0));
        let mut ours = (ours_target_width * scale).max(px(0.0));
        let mut theirs = (available - base - ours).max(px(0.0));

        let used = base + ours + theirs;
        let slack = (available - used).max(px(0.0));
        theirs += slack;

        if theirs < px(0.0) {
            theirs = px(0.0);
        }

        base = base.max(px(0.0));
        ours = ours.max(px(0.0));
        (base, ours, theirs)
    };

    let base_col = Bounds::new(bounds.origin, size(base_w, bounds.size.height));
    let first_handle = Bounds::new(
        point(bounds.left() + base_w, bounds.top()),
        size(handle_width, bounds.size.height),
    );
    let ours_col = Bounds::new(
        point(first_handle.right(), bounds.top()),
        size(ours_w, bounds.size.height),
    );
    let second_handle = Bounds::new(
        point(ours_col.right(), bounds.top()),
        size(handle_width, bounds.size.height),
    );
    let theirs_col = Bounds::new(
        point(second_handle.right(), bounds.top()),
        size(theirs_w, bounds.size.height),
    );

    (base_col, first_handle, ours_col, second_handle, theirs_col)
}

fn split_column_text_bounds(col: Bounds<Pixels>, pad: Pixels, gap: Pixels) -> Bounds<Pixels> {
    let line_no_width = conflict_line_no_width();
    let left = col.left() + pad + line_no_width + gap;
    let width = (col.size.width - pad * 2.0 - line_no_width - gap).max(px(0.0));
    Bounds::new(point(left, col.top()), size(width, col.size.height))
}

fn inline_text_bounds(bounds: Bounds<Pixels>, pad: Pixels, gap: Pixels) -> Bounds<Pixels> {
    let line_no_width = conflict_line_no_width();
    let prefix_width = conflict_inline_prefix_width();
    let left = bounds.left() + pad + line_no_width + gap + line_no_width + gap + prefix_width + gap;
    let width =
        (bounds.size.width - pad * 2.0 - line_no_width - line_no_width - prefix_width - gap * 3.0)
            .max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

fn conflict_line_no_width() -> Pixels {
    px(38.0)
}

fn conflict_inline_prefix_width() -> Pixels {
    px(12.0)
}

fn paint_gutter_text(
    text: &SharedString,
    x: Pixels,
    y: Pixels,
    color: gpui::Rgba,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) {
    if text.is_empty() {
        return;
    }
    let mut style = diff_text_style(window);
    style.color = color.into();
    let key = {
        use std::collections::hash_map::DefaultHasher;
        let mut hasher = DefaultHasher::new();
        text.as_ref().hash(&mut hasher);
        metrics.font_size.hash(&mut hasher);
        style.font_family.hash(&mut hasher);
        style.font_weight.hash(&mut hasher);
        color.r.to_bits().hash(&mut hasher);
        color.g.to_bits().hash(&mut hasher);
        color.b.to_bits().hash(&mut hasher);
        color.a.to_bits().hash(&mut hasher);
        hasher.finish()
    };

    let shaped = GUTTER_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow().get(&key).cloned());
    let shaped = shaped.unwrap_or_else(|| {
        let run = style.to_run(text.len());
        let shaped = window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None);

        GUTTER_TEXT_LAYOUT_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() > GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES {
                cache.clear();
            }
            cache.insert(key, shaped.clone());
        });

        shaped
    });
    let _ = shaped.paint(point(x, y), metrics.line_height, window, cx);
}

fn paint_conflict_text(
    bounds: Bounds<Pixels>,
    fg: gpui::Rgba,
    y: Pixels,
    metrics: LineMetrics,
    prepared: &PreparedConflictText,
    window: &mut Window,
    cx: &mut App,
) {
    if prepared.text.is_empty() {
        return;
    }

    let mut base_style = diff_text_style(window);
    base_style.color = fg.into();
    base_style.white_space = gpui::WhiteSpace::Nowrap;
    base_style.text_overflow = None;

    let layout = ensure_layout_cached(prepared, &base_style, fg, metrics, window);

    if prepared.highlights.is_empty() {
        let _ = layout.paint(point(bounds.left(), y), metrics.line_height, window, cx);
        return;
    }

    let _ = layout.paint_background(point(bounds.left(), y), metrics.line_height, window, cx);
    let _ = layout.paint(point(bounds.left(), y), metrics.line_height, window, cx);
}

fn ensure_layout_cached(
    prepared: &PreparedConflictText,
    base_style: &TextStyle,
    fg: gpui::Rgba,
    metrics: LineMetrics,
    window: &mut Window,
) -> gpui::ShapedLine {
    let key = conflict_layout_key(prepared, base_style, fg, metrics);
    if let Some(layout) = CONFLICT_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow().get(&key).cloned())
    {
        return layout;
    }

    let shaped = if prepared.highlights.is_empty() {
        let run = base_style.to_run(prepared.text.len());
        window
            .text_system()
            .shape_line(prepared.text.clone(), metrics.font_size, &[run], None)
    } else {
        let runs = compute_runs(
            prepared.text.as_ref(),
            base_style,
            prepared.highlights.as_ref(),
        );
        window
            .text_system()
            .shape_line(prepared.text.clone(), metrics.font_size, &runs, None)
    };

    CONFLICT_TEXT_LAYOUT_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() > CONFLICT_TEXT_LAYOUT_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(key, shaped.clone());
    });

    shaped
}

fn conflict_layout_key(
    prepared: &PreparedConflictText,
    base_style: &TextStyle,
    fg: gpui::Rgba,
    metrics: LineMetrics,
) -> u64 {
    use std::collections::hash_map::DefaultHasher;

    let mut hasher = DefaultHasher::new();
    prepared.text_hash.hash(&mut hasher);
    prepared.highlights_hash.hash(&mut hasher);
    metrics.font_size.hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_weight.hash(&mut hasher);
    fg.r.to_bits().hash(&mut hasher);
    fg.g.to_bits().hash(&mut hasher);
    fg.b.to_bits().hash(&mut hasher);
    fg.a.to_bits().hash(&mut hasher);
    hasher.finish()
}

fn compute_runs(
    text: &str,
    default_style: &TextStyle,
    highlights: &[(Range<usize>, HighlightStyle)],
) -> Vec<TextRun> {
    let mut runs = Vec::with_capacity(highlights.len() * 2 + 1);
    let mut ix = 0usize;
    for (range, highlight) in highlights {
        if ix < range.start {
            runs.push(default_style.clone().to_run(range.start - ix));
        }
        runs.push(
            default_style
                .clone()
                .highlight(*highlight)
                .to_run(range.len()),
        );
        ix = range.end;
    }
    if ix < text.len() {
        runs.push(default_style.clone().to_run(text.len() - ix));
    }
    runs
}

fn empty_highlights() -> HighlightSpans {
    static EMPTY: OnceLock<HighlightSpans> = OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::new(Vec::new())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prepare_text_cell_applies_whitespace_when_no_styled_text() {
        let prepared = prepare_conflict_text_for_canvas("a b\t".into(), None, true);
        assert_eq!(prepared.text.as_ref(), "a·b→");
        assert!(prepared.highlights.is_empty());
    }

    #[test]
    fn three_way_layout_grows_last_column_when_space_allows() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(300.0), px(20.0)));
        let (base, _, ours, _, theirs) =
            three_way_columns_with_widths(bounds, px(70.0), px(70.0), px(70.0), px(10.0));

        assert_eq!(base.size.width, px(70.0));
        assert_eq!(ours.size.width, px(70.0));
        assert_eq!(theirs.size.width, px(140.0));
    }

    #[test]
    fn three_way_layout_scales_columns_when_space_is_tight() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(120.0), px(20.0)));
        let (base, _, ours, _, theirs) =
            three_way_columns_with_widths(bounds, px(70.0), px(70.0), px(70.0), px(10.0));
        let available = px(100.0);

        assert!(
            (base.size.width + ours.size.width + theirs.size.width - available).abs() < px(0.01)
        );
        assert!(base.size.width > px(0.0));
        assert!(ours.size.width > px(0.0));
        assert!(theirs.size.width > px(0.0));
    }

    #[test]
    fn split_layout_preserves_right_target_by_shrinking_left() {
        let bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(20.0)));
        let handle_width = px(10.0);
        let (left, handle, right) =
            split_columns_with_widths(bounds, px(120.0), px(120.0), handle_width);

        assert_eq!(left.size.width, px(70.0));
        assert_eq!(handle.size.width, handle_width);
        assert_eq!(right.size.width, px(120.0));
        assert_eq!(
            left.size.width + handle.size.width + right.size.width,
            bounds.size.width
        );
    }

    #[test]
    fn prepare_text_cell_remaps_highlighted_styled_text_for_whitespace() {
        let style = gpui::HighlightStyle::default();
        let styled = CachedDiffStyledText {
            text: "a b".into(),
            highlights: Arc::new(vec![(1..3, style)]),
            highlights_hash: 11,
            text_hash: 7,
        };

        let prepared = prepare_conflict_text_for_canvas("ignored".into(), Some(&styled), true);
        assert_eq!(prepared.text.as_ref(), "a·b");
        assert_eq!(prepared.highlights.len(), 1);
        assert_eq!(prepared.highlights[0].0, 1..4);
        assert_eq!(prepared.text_hash, hash_text("a·b"));
        assert_eq!(prepared.highlights_hash, 11);
    }

    #[test]
    fn prepare_text_cell_applies_whitespace_for_unhighlighted_styled_text() {
        let styled = CachedDiffStyledText {
            text: "a b\t".into(),
            highlights: empty_highlights(),
            highlights_hash: 0,
            text_hash: 1,
        };

        let prepared = prepare_conflict_text_for_canvas("ignored".into(), Some(&styled), true);
        assert_eq!(prepared.text.as_ref(), "a·b→");
        assert!(prepared.highlights.is_empty());
        assert_eq!(prepared.highlights_hash, 0);
    }

    #[test]
    fn whitespace_visible_text_marks_all_whitespace_kinds() {
        let display = whitespace_visible_text(" \t\r\n");
        assert_eq!(display.as_ref(), "·→␍↵");
    }
}
