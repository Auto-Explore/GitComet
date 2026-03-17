use super::canvas::keyed_canvas;
use super::*;
use gpui::{
    App, Bounds, CursorStyle, DispatchPhase, HighlightStyle, Hitbox, HitboxBehavior, Pixels,
    Styled, TextRun, TextStyle, Window, fill, point, px, size,
};
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;
use std::sync::OnceLock;

const DIFF_FONT_SCALE: f32 = 0.80;

const GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES: usize = 16_384;

type HighlightSpans = Arc<Vec<(Range<usize>, HighlightStyle)>>;

thread_local! {
    static GUTTER_TEXT_LAYOUT_CACHE: RefCell<FxLruCache<u64, gpui::ShapedLine>> =
        RefCell::new(new_fx_lru_cache(GUTTER_TEXT_LAYOUT_CACHE_MAX_ENTRIES));
}

#[allow(clippy::too_many_arguments)]
pub(super) fn inline_diff_line_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    old: SharedString,
    new: SharedString,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    styled: Option<&CachedDiffStyledText>,
) -> AnyElement {
    let text = styled.map(|s| s.text.clone()).unwrap_or_default();
    let highlights = styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
    let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);

    keyed_canvas(
        ("diff_row_canvas_inline", visible_ix),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(window, pad);
            let text_bounds = inline_text_bounds(bounds, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);

            InlineRowPrepaintState {
                bounds,
                pad,
                gutter_total,
                text_bounds,
                text_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            paint_gutter_text(
                &old,
                prepaint.bounds.left() + prepaint.pad,
                y,
                gutter_fg,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &new,
                prepaint.bounds.left() + prepaint.gutter_total + prepaint.pad,
                y,
                gutter_fg,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    DiffTextRegion::Inline,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    highlights_hash,
                    text_hash,
                    y,
                    fg,
                    line_metrics,
                    window,
                    cx,
                );
            });

            let row_bounds = prepaint.bounds;
            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_row_bounds = row_bounds.intersect(&clip_bounds);
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_bounds: visible_row_bounds,
                    regions: DiffRowTextRegions::single(
                        DiffTextRegion::Inline,
                        visible_text_bounds,
                    ),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                },
            );

            if selected {
                window.paint_quad(gpui::outline(
                    bounds,
                    with_alpha(theme.colors.accent, 0.55),
                    gpui::BorderStyle::default(),
                ));
            }
        },
    )
    .h(px(20.0))
    .min_w(min_width)
    .w_full()
    .bg(bg)
    .text_xs()
    .whitespace_nowrap()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub(super) fn split_diff_line_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    old: SharedString,
    new: SharedString,
    left_bg: gpui::Rgba,
    left_fg: gpui::Rgba,
    left_gutter: gpui::Rgba,
    right_bg: gpui::Rgba,
    right_fg: gpui::Rgba,
    right_gutter: gpui::Rgba,
    left_styled: Option<&CachedDiffStyledText>,
    right_styled: Option<&CachedDiffStyledText>,
) -> AnyElement {
    let left_text = left_styled.map(|s| s.text.clone()).unwrap_or_default();
    let left_highlights = left_styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let left_highlights_hash = left_styled.map(|s| s.highlights_hash).unwrap_or(0);
    let left_text_hash = left_styled.map(|s| s.text_hash).unwrap_or(0);
    let right_text = right_styled.map(|s| s.text.clone()).unwrap_or_default();
    let right_highlights = right_styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let right_highlights_hash = right_styled.map(|s| s.highlights_hash).unwrap_or(0);
    let right_text_hash = right_styled.map(|s| s.text_hash).unwrap_or(0);

    keyed_canvas(
        ("diff_row_canvas_split", visible_ix),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(window, pad);
            let (left_col, sep_bounds, right_col) = split_columns(bounds);
            let left_text_bounds = column_text_bounds(left_col, gutter_total, pad);
            let right_text_bounds = column_text_bounds(right_col, gutter_total, pad);

            let left_hitbox = window.insert_hitbox(left_text_bounds, HitboxBehavior::Normal);
            let right_hitbox = window.insert_hitbox(right_text_bounds, HitboxBehavior::Normal);

            SplitRowPrepaintState {
                bounds,
                pad,
                left_col,
                sep_bounds,
                right_col,
                left_text_bounds,
                right_text_bounds,
                left_hitbox,
                right_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.left_hitbox);
            window.set_cursor_style(CursorStyle::IBeam, &prepaint.right_hitbox);

            window.paint_quad(fill(prepaint.left_col, left_bg));
            window.paint_quad(fill(prepaint.sep_bounds, theme.colors.border));
            window.paint_quad(fill(prepaint.right_col, right_bg));

            paint_gutter_text(
                &old,
                prepaint.left_col.left() + prepaint.pad,
                y,
                left_gutter,
                line_metrics,
                window,
                cx,
            );
            paint_gutter_text(
                &new,
                prepaint.right_col.left() + prepaint.pad,
                y,
                right_gutter,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.left_text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    DiffTextRegion::SplitLeft,
                    prepaint.left_text_bounds,
                    &left_text,
                    &left_highlights,
                    left_highlights_hash,
                    left_text_hash,
                    y,
                    left_fg,
                    line_metrics,
                    window,
                    cx,
                );
            });

            window.paint_layer(prepaint.right_text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    DiffTextRegion::SplitRight,
                    prepaint.right_text_bounds,
                    &right_text,
                    &right_highlights,
                    right_highlights_hash,
                    right_text_hash,
                    y,
                    right_fg,
                    line_metrics,
                    window,
                    cx,
                );
            });

            let row_bounds = prepaint.bounds;
            let left_text_bounds = prepaint.left_text_bounds;
            let right_text_bounds = prepaint.right_text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_row_bounds = row_bounds.intersect(&clip_bounds);
            let visible_left_text_bounds = left_text_bounds.intersect(&clip_bounds);
            let visible_right_text_bounds = right_text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_bounds: visible_row_bounds,
                    regions: DiffRowTextRegions::split(
                        visible_left_text_bounds,
                        visible_right_text_bounds,
                    ),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                },
            );

            if selected {
                window.paint_quad(gpui::outline(
                    bounds,
                    with_alpha(theme.colors.accent, 0.55),
                    gpui::BorderStyle::default(),
                ));
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
pub(super) fn patch_split_column_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    column: super::diff::PatchSplitColumn,
    visible_ix: usize,
    min_width: Pixels,
    selected: bool,
    bg: gpui::Rgba,
    fg: gpui::Rgba,
    gutter_fg: gpui::Rgba,
    line_no: SharedString,
    styled: Option<&CachedDiffStyledText>,
) -> AnyElement {
    let region = match column {
        super::diff::PatchSplitColumn::Left => DiffTextRegion::SplitLeft,
        super::diff::PatchSplitColumn::Right => DiffTextRegion::SplitRight,
    };
    let text = styled.map(|s| s.text.clone()).unwrap_or_default();
    let highlights = styled
        .map(|s| Arc::clone(&s.highlights))
        .unwrap_or_else(empty_highlights);
    let highlights_hash = styled.map(|s| s.highlights_hash).unwrap_or(0);
    let text_hash = styled.map(|s| s.text_hash).unwrap_or(0);

    keyed_canvas(
        (
            match column {
                super::diff::PatchSplitColumn::Left => "diff_row_canvas_file_split_left",
                super::diff::PatchSplitColumn::Right => "diff_row_canvas_file_split_right",
            },
            visible_ix,
        ),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(window, pad);
            let text_bounds = single_column_text_bounds(bounds, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            SingleColumnRowPrepaintState {
                bounds,
                pad,
                text_bounds,
                text_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            window.paint_quad(fill(prepaint.bounds, bg));

            paint_gutter_text(
                &line_no,
                prepaint.bounds.left() + prepaint.pad,
                y,
                gutter_fg,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    visible_ix,
                    region,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    highlights_hash,
                    text_hash,
                    y,
                    fg,
                    line_metrics,
                    window,
                    cx,
                );
            });

            let row_bounds = prepaint.bounds;
            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_row_bounds = row_bounds.intersect(&clip_bounds);
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            install_diff_row_mouse_handlers(
                window,
                &view,
                visible_ix,
                DiffRowMouseHandlers {
                    row_bounds: visible_row_bounds,
                    regions: DiffRowTextRegions::single(region, visible_text_bounds),
                    right_click: DiffRowRightClickBehavior::OpenContextMenu,
                    mouse_up: DiffRowMouseUpBehavior::HandlePatchRowClick,
                },
            );

            if selected {
                window.paint_quad(gpui::outline(
                    bounds,
                    with_alpha(theme.colors.accent, 0.55),
                    gpui::BorderStyle::default(),
                ));
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

pub(super) fn worktree_preview_row_canvas(
    theme: AppTheme,
    view: Entity<MainPaneView>,
    ix: usize,
    min_width: Pixels,
    bar_color: Option<gpui::Rgba>,
    line_no: SharedString,
    styled: &CachedDiffStyledText,
) -> AnyElement {
    let text = styled.text.clone();
    let highlights = Arc::clone(&styled.highlights);
    let highlights_hash = styled.highlights_hash;
    let text_hash = styled.text_hash;

    keyed_canvas(
        ("worktree_preview_row_canvas", ix),
        move |bounds, window, _cx| {
            let pad = px_2(window);
            let gutter_total = gutter_cell_total_width(window, pad);
            let bar_w = if bar_color.is_some() {
                px(3.0)
            } else {
                px(0.0)
            };
            let inner = Bounds::new(
                point(bounds.left() + bar_w, bounds.top()),
                size((bounds.size.width - bar_w).max(px(0.0)), bounds.size.height),
            );
            let text_bounds = single_column_text_bounds(inner, gutter_total, pad);
            let text_hitbox = window.insert_hitbox(text_bounds, HitboxBehavior::Normal);
            WorktreePreviewRowPrepaintState {
                inner,
                pad,
                bar_w,
                text_bounds,
                text_hitbox,
            }
        },
        move |bounds, prepaint, window, cx| {
            let line_metrics = line_metrics(window);
            let y = center_text_y(bounds, line_metrics.line_height);

            window.paint_quad(fill(bounds, theme.colors.window_bg));
            if let Some(color) = bar_color
                && prepaint.bar_w > px(0.0)
            {
                window.paint_quad(fill(
                    Bounds::new(
                        point(bounds.left(), bounds.top()),
                        size(prepaint.bar_w, bounds.size.height),
                    ),
                    color,
                ));
            }

            window.set_cursor_style(CursorStyle::IBeam, &prepaint.text_hitbox);

            paint_gutter_text(
                &line_no,
                prepaint.inner.left() + prepaint.pad,
                y,
                theme.colors.text_muted,
                line_metrics,
                window,
                cx,
            );

            window.paint_layer(prepaint.text_bounds, |window| {
                paint_selectable_diff_text(
                    &view,
                    ix,
                    DiffTextRegion::Inline,
                    prepaint.text_bounds,
                    &text,
                    &highlights,
                    highlights_hash,
                    text_hash,
                    y,
                    theme.colors.text,
                    line_metrics,
                    window,
                    cx,
                );
            });

            let text_bounds = prepaint.text_bounds;
            let clip_bounds = window.content_mask().bounds;
            let visible_text_bounds = text_bounds.intersect(&clip_bounds);
            window.on_mouse_event({
                let view = view.clone();
                move |event: &gpui::MouseDownEvent, phase, window, cx| {
                    if phase != DispatchPhase::Bubble
                        || !visible_text_bounds.contains(&event.position)
                    {
                        return;
                    }

                    if event.button == gpui::MouseButton::Left {
                        window.focus(&view.read(cx).diff_panel_focus_handle);
                        let click_count = event.click_count;
                        let position = event.position;
                        view.update(cx, |this, cx| {
                            if click_count >= 2 {
                                this.double_click_select_diff_text(
                                    ix,
                                    DiffTextRegion::Inline,
                                    DiffClickKind::Line,
                                );
                            } else {
                                this.begin_diff_text_selection(
                                    ix,
                                    DiffTextRegion::Inline,
                                    position,
                                );
                                this.begin_diff_text_scroll_tracking(position, cx);
                            }
                            cx.notify();
                        });
                    } else if event.button == gpui::MouseButton::Right {
                        view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                ix,
                                DiffTextRegion::Inline,
                                event.position,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            });
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
struct InlineRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    gutter_total: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
struct SplitRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    left_col: Bounds<Pixels>,
    sep_bounds: Bounds<Pixels>,
    right_col: Bounds<Pixels>,
    left_text_bounds: Bounds<Pixels>,
    right_text_bounds: Bounds<Pixels>,
    left_hitbox: Hitbox,
    right_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
struct SingleColumnRowPrepaintState {
    bounds: Bounds<Pixels>,
    pad: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
struct WorktreePreviewRowPrepaintState {
    inner: Bounds<Pixels>,
    pad: Pixels,
    bar_w: Pixels,
    text_bounds: Bounds<Pixels>,
    text_hitbox: Hitbox,
}

#[derive(Clone, Debug)]
enum DiffRowTextRegions {
    Single {
        region: DiffTextRegion,
        bounds: Bounds<Pixels>,
    },
    Split {
        left_bounds: Bounds<Pixels>,
        right_bounds: Bounds<Pixels>,
    },
}

impl DiffRowTextRegions {
    fn single(region: DiffTextRegion, bounds: Bounds<Pixels>) -> Self {
        Self::Single { region, bounds }
    }

    fn split(left_bounds: Bounds<Pixels>, right_bounds: Bounds<Pixels>) -> Self {
        Self::Split {
            left_bounds,
            right_bounds,
        }
    }

    fn region_at(&self, position: gpui::Point<Pixels>) -> Option<DiffTextRegion> {
        match self {
            Self::Single { region, bounds } => bounds.contains(&position).then_some(*region),
            Self::Split {
                left_bounds,
                right_bounds,
            } => {
                if left_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitLeft)
                } else if right_bounds.contains(&position) {
                    Some(DiffTextRegion::SplitRight)
                } else {
                    None
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffRowRightClickBehavior {
    OpenContextMenu,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DiffRowMouseUpBehavior {
    None,
    HandlePatchRowClick,
}

#[derive(Clone, Debug)]
struct DiffRowMouseHandlers {
    row_bounds: Bounds<Pixels>,
    regions: DiffRowTextRegions,
    right_click: DiffRowRightClickBehavior,
    mouse_up: DiffRowMouseUpBehavior,
}

fn should_handle_row_mouse_event(
    phase: DispatchPhase,
    row_bounds: &Bounds<Pixels>,
    position: gpui::Point<Pixels>,
) -> bool {
    phase == DispatchPhase::Bubble && row_bounds.contains(&position)
}

fn install_diff_row_mouse_handlers(
    window: &mut Window,
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    handlers: DiffRowMouseHandlers,
) {
    let DiffRowMouseHandlers {
        row_bounds,
        regions,
        right_click,
        mouse_up,
    } = handlers;
    let row_bounds_for_down = row_bounds;
    let regions = regions.clone();
    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseDownEvent, phase, window, cx| {
            if !should_handle_row_mouse_event(phase, &row_bounds_for_down, event.position) {
                return;
            }

            let region = regions.region_at(event.position);

            if event.button == gpui::MouseButton::Left {
                window.focus(&view.read(cx).diff_panel_focus_handle);
                if let Some(region) = region {
                    let click_count = event.click_count;
                    let position = event.position;
                    view.update(cx, |this, cx| {
                        if click_count >= 2 {
                            this.double_click_select_diff_text(
                                visible_ix,
                                region,
                                DiffClickKind::Line,
                            );
                        } else {
                            this.begin_diff_text_selection(visible_ix, region, position);
                            this.begin_diff_text_scroll_tracking(position, cx);
                        }
                        cx.notify();
                    });
                }
            } else if event.button == gpui::MouseButton::Right
                && let Some(region) = region
            {
                match right_click {
                    DiffRowRightClickBehavior::OpenContextMenu => {
                        view.update(cx, |this, cx| {
                            this.open_diff_editor_context_menu(
                                visible_ix,
                                region,
                                event.position,
                                window,
                                cx,
                            );
                            cx.notify();
                        });
                    }
                }
            }
        }
    });

    if mouse_up == DiffRowMouseUpBehavior::None {
        return;
    }

    let row_bounds_for_up = row_bounds;
    window.on_mouse_event({
        let view = view.clone();
        move |event: &gpui::MouseUpEvent, phase, _window, cx| {
            if event.button != gpui::MouseButton::Left
                || !should_handle_row_mouse_event(phase, &row_bounds_for_up, event.position)
            {
                return;
            }

            let shift = event.modifiers.shift;
            view.update(cx, |this, cx| {
                if this.consume_suppress_click_after_drag() {
                    cx.notify();
                    return;
                }
                this.handle_patch_row_click(visible_ix, DiffClickKind::Line, shift);
                cx.notify();
            });
        }
    });
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

fn gutter_cell_total_width(window: &Window, pad: Pixels) -> Pixels {
    let _ = window;
    px(44.0) + pad * 2.0
}

fn inline_text_bounds(bounds: Bounds<Pixels>, gutter_total: Pixels, pad: Pixels) -> Bounds<Pixels> {
    let left = bounds.left() + gutter_total * 2.0 + pad;
    let width = (bounds.size.width - gutter_total * 2.0 - pad * 2.0).max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

fn single_column_text_bounds(
    bounds: Bounds<Pixels>,
    gutter_total: Pixels,
    pad: Pixels,
) -> Bounds<Pixels> {
    let left = bounds.left() + gutter_total + pad;
    let width = (bounds.size.width - gutter_total - pad * 2.0).max(px(0.0));
    Bounds::new(point(left, bounds.top()), size(width, bounds.size.height))
}

fn split_columns(bounds: Bounds<Pixels>) -> (Bounds<Pixels>, Bounds<Pixels>, Bounds<Pixels>) {
    let sep = px(1.0);
    let total_w = bounds.size.width.max(px(0.0));
    let inner_w = (total_w - sep).max(px(0.0));
    let left_w = (inner_w * 0.5).floor();
    let right_w = (inner_w - left_w).max(px(0.0));
    let left = Bounds::new(bounds.origin, size(left_w, bounds.size.height));
    let sep_bounds = Bounds::new(
        point(bounds.left() + left_w, bounds.top()),
        size(sep, bounds.size.height),
    );
    let right = Bounds::new(
        point(bounds.left() + left_w + sep, bounds.top()),
        size(right_w, bounds.size.height),
    );
    (left, sep_bounds, right)
}

fn column_text_bounds(col: Bounds<Pixels>, gutter_total: Pixels, pad: Pixels) -> Bounds<Pixels> {
    single_column_text_bounds(col, gutter_total, pad)
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
        let mut hasher = FxHasher::default();
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

    let shaped = GUTTER_TEXT_LAYOUT_CACHE.with(|cache| cache.borrow_mut().get(&key).cloned());
    let shaped = shaped.unwrap_or_else(|| {
        let run = style.to_run(text.len());
        let shaped = window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None);

        GUTTER_TEXT_LAYOUT_CACHE.with(|cache| {
            cache.borrow_mut().put(key, shaped.clone());
        });

        shaped
    });
    let _ = shaped.paint(point(x, y), metrics.line_height, window, cx);
}

#[allow(clippy::too_many_arguments)]
fn paint_selectable_diff_text(
    view: &Entity<MainPaneView>,
    visible_ix: usize,
    region: DiffTextRegion,
    bounds: Bounds<Pixels>,
    text: &SharedString,
    highlights: &Arc<Vec<(Range<usize>, HighlightStyle)>>,
    highlights_hash: u64,
    text_hash: u64,
    y: Pixels,
    base_fg: gpui::Rgba,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) {
    let mut base_style = diff_text_style(window);
    base_style.color = base_fg.into();
    base_style.white_space = gpui::WhiteSpace::Nowrap;
    base_style.text_overflow = None;

    let selection = view
        .read(cx)
        .diff_text_local_selection_range(visible_ix, region, text.len());

    let (layout_key, layout, shaped_new) = ensure_layout_cached(
        view,
        text_hash,
        text,
        &base_style,
        base_fg,
        highlights.as_ref(),
        highlights_hash,
        metrics,
        window,
        cx,
    );

    let pad = px_2(window);
    let gutter_total = gutter_cell_total_width(window, pad);
    let row_extra = match region {
        DiffTextRegion::Inline => gutter_total * 2.0 + pad * 2.0,
        DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => gutter_total + pad * 2.0,
    };
    let required_row_w = (row_extra + layout.width + px(16.0)).round();

    if let Some(r) = selection {
        let x0 = layout.x_for_index(r.start.min(text.len()));
        let x1 = layout.x_for_index(r.end.min(text.len()));
        if x1 > x0 {
            let color = view.read(cx).diff_text_selection_color();
            window.paint_quad(fill(
                Bounds::from_corners(
                    point(bounds.left() + x0, bounds.top()),
                    point(bounds.left() + x1, bounds.bottom()),
                ),
                color,
            ));
        }
    }

    let hitbox = DiffTextHitbox {
        bounds,
        layout_key,
        text_len: text.len(),
    };

    view.update(cx, |this, cx| {
        this.set_diff_text_hitbox(visible_ix, region, hitbox);
        this.touch_diff_text_layout_cache(layout_key, shaped_new);
        if required_row_w > this.diff_horizontal_min_width {
            this.diff_horizontal_min_width = required_row_w;
            cx.notify();
        }
    });

    if text.is_empty() {
        return;
    }

    if highlights.is_empty() {
        let _ = layout.paint(point(bounds.left(), y), metrics.line_height, window, cx);
        return;
    }

    let _ = layout.paint_background(point(bounds.left(), y), metrics.line_height, window, cx);
    let _ = layout.paint(point(bounds.left(), y), metrics.line_height, window, cx);
}

fn diff_layout_base_key(
    text_hash: u64,
    base_style: &TextStyle,
    base_fg: gpui::Rgba,
    metrics: LineMetrics,
) -> u64 {
    let mut hasher = FxHasher::default();
    text_hash.hash(&mut hasher);
    metrics.font_size.hash(&mut hasher);
    base_style.font_family.hash(&mut hasher);
    base_style.font_weight.hash(&mut hasher);
    base_fg.r.to_bits().hash(&mut hasher);
    base_fg.g.to_bits().hash(&mut hasher);
    base_fg.b.to_bits().hash(&mut hasher);
    base_fg.a.to_bits().hash(&mut hasher);
    hasher.finish()
}

#[allow(clippy::too_many_arguments)]
fn ensure_layout_cached(
    view: &Entity<MainPaneView>,
    text_hash: u64,
    text: &SharedString,
    base_style: &TextStyle,
    base_fg: gpui::Rgba,
    highlights: &[(Range<usize>, HighlightStyle)],
    highlights_hash: u64,
    metrics: LineMetrics,
    window: &mut Window,
    cx: &mut App,
) -> (u64, gpui::ShapedLine, Option<gpui::ShapedLine>) {
    let base_key = diff_layout_base_key(text_hash, base_style, base_fg, metrics);

    let layout_key = if highlights.is_empty() {
        base_key
    } else {
        let mut hasher = FxHasher::default();
        base_key.hash(&mut hasher);
        highlights_hash.hash(&mut hasher);
        highlights.len().hash(&mut hasher);
        hasher.finish()
    };

    if let Some(entry) = view.read(cx).diff_text_layout_cache.get(&layout_key) {
        return (layout_key, entry.layout.clone(), None);
    }

    let shaped = if highlights.is_empty() {
        let run = base_style.to_run(text.len());
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &[run], None)
    } else {
        let runs = compute_runs(text.as_ref(), base_style, highlights);
        window
            .text_system()
            .shape_line(text.clone(), metrics.font_size, &runs, None)
    };
    (layout_key, shaped.clone(), Some(shaped))
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

    fn test_bounds(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        Bounds::new(point(px(x), px(y)), size(px(width), px(height)))
    }

    #[test]
    fn should_handle_row_mouse_event_requires_bubble_phase_and_in_bounds() {
        let row_bounds = test_bounds(10.0, 20.0, 50.0, 10.0);
        let inside = point(px(20.0), px(25.0));
        let outside = point(px(200.0), px(25.0));

        assert!(should_handle_row_mouse_event(
            DispatchPhase::Bubble,
            &row_bounds,
            inside,
        ));
        assert!(!should_handle_row_mouse_event(
            DispatchPhase::Capture,
            &row_bounds,
            inside,
        ));
        assert!(!should_handle_row_mouse_event(
            DispatchPhase::Bubble,
            &row_bounds,
            outside,
        ));
    }

    #[test]
    fn diff_row_text_regions_single_only_hits_inside_text() {
        let regions =
            DiffRowTextRegions::single(DiffTextRegion::Inline, test_bounds(5.0, 5.0, 20.0, 10.0));

        assert_eq!(
            regions.region_at(point(px(10.0), px(10.0))),
            Some(DiffTextRegion::Inline)
        );
        assert_eq!(regions.region_at(point(px(1.0), px(10.0))), None);
    }

    #[test]
    fn diff_row_text_regions_split_maps_left_and_right_regions() {
        let regions = DiffRowTextRegions::split(
            test_bounds(0.0, 0.0, 40.0, 20.0),
            test_bounds(41.0, 0.0, 40.0, 20.0),
        );

        assert_eq!(
            regions.region_at(point(px(10.0), px(10.0))),
            Some(DiffTextRegion::SplitLeft)
        );
        assert_eq!(
            regions.region_at(point(px(60.0), px(10.0))),
            Some(DiffTextRegion::SplitRight)
        );
        assert_eq!(regions.region_at(point(px(40.5), px(10.0))), None);
    }
}
