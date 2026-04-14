use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{
    Bounds, CursorStyle, DispatchPhase, ElementId, Hitbox, HitboxBehavior, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle, UniformListScrollHandle,
    canvas, div, fill, point, px, size,
};
use std::time::Duration;

pub const SCROLLBAR_GUTTER_PX: f32 = 16.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollbarMarkerKind {
    Add,
    Remove,
    Modify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollbarAxis {
    Vertical,
    Horizontal,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollbarMarker {
    /// Start of the marker as a fraction of total content height in `[0, 1]`.
    pub start: f32,
    /// End of the marker as a fraction of total content height in `[0, 1]`.
    pub end: f32,
    pub kind: ScrollbarMarkerKind,
}

#[derive(Clone)]
pub struct Scrollbar {
    id: ElementId,
    handle: ScrollbarHandle,
    axis: ScrollbarAxis,
    markers: Vec<ScrollbarMarker>,
    always_visible: bool,
    #[cfg(test)]
    debug_selector: Option<&'static str>,
}

#[derive(Clone)]
#[doc(hidden)]
pub enum ScrollbarHandle {
    Scroll(ScrollHandle),
    UniformList(UniformListScrollHandle),
}

impl From<ScrollHandle> for ScrollbarHandle {
    fn from(handle: ScrollHandle) -> Self {
        Self::Scroll(handle)
    }
}

impl From<UniformListScrollHandle> for ScrollbarHandle {
    fn from(handle: UniformListScrollHandle) -> Self {
        Self::UniformList(handle)
    }
}

struct ScrollbarInteractionState {
    drag_offset: Option<Pixels>,
    showing: bool,
    hide_task: Option<gpui::Task<()>>,
    last_scroll: Pixels,
    thumb_visible: bool,
    /// Some GPUI scroll surfaces report positive offsets while others report negative offsets.
    /// Track the observed sign so the thumb moves/drag-scrolls in the correct direction.
    offset_sign: i8,
}

impl Default for ScrollbarInteractionState {
    fn default() -> Self {
        Self {
            drag_offset: None,
            showing: false,
            hide_task: None,
            last_scroll: px(0.0),
            thumb_visible: false,
            offset_sign: -1,
        }
    }
}

impl ScrollbarHandle {
    fn base_handle(&self) -> ScrollHandle {
        match self {
            Self::Scroll(handle) => handle.clone(),
            Self::UniformList(handle) => handle.0.borrow().base_handle.clone(),
        }
    }

    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match (axis, self) {
            (ScrollbarAxis::Vertical, Self::UniformList(handle)) => handle
                .0
                .borrow()
                .last_item_size
                .map(|size| (size.contents.height - size.item.height).max(px(0.0)))
                .unwrap_or_else(|| handle.0.borrow().base_handle.max_offset().y),
            (ScrollbarAxis::Horizontal, Self::UniformList(handle)) => handle
                .0
                .borrow()
                .last_item_size
                .map(|size| (size.contents.width - size.item.width).max(px(0.0)))
                .unwrap_or_else(|| handle.0.borrow().base_handle.max_offset().x),
            (ScrollbarAxis::Vertical, _) => self.base_handle().max_offset().y.max(px(0.0)),
            (ScrollbarAxis::Horizontal, _) => self.base_handle().max_offset().x.max(px(0.0)),
        }
    }

    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match (axis, self) {
            (ScrollbarAxis::Vertical, Self::UniformList(handle)) => {
                handle.0.borrow().base_handle.offset().y
            }
            (ScrollbarAxis::Vertical, _) => self.base_handle().offset().y,
            (ScrollbarAxis::Horizontal, _) => self.base_handle().offset().x,
        }
    }

    fn set_axis_offset(&self, axis: ScrollbarAxis, axis_offset: Pixels) {
        let base = self.base_handle();
        let current = base.offset();
        match axis {
            ScrollbarAxis::Vertical => base.set_offset(point(current.x, axis_offset)),
            ScrollbarAxis::Horizontal => base.set_offset(point(axis_offset, current.y)),
        }
    }

    fn scrollbar_drag_started(&self, _axis: ScrollbarAxis) {}

    fn scrollbar_drag_ended(&self, _axis: ScrollbarAxis) {}
}

#[derive(Clone, Debug)]
struct ScrollbarPrepaintState {
    interaction_bounds: Bounds<Pixels>,
    track_bounds: Bounds<Pixels>,
    thumb_bounds: Bounds<Pixels>,
    thumb_hit_bounds: Bounds<Pixels>,
    cursor_hitbox: Hitbox,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, handle: impl Into<ScrollbarHandle>) -> Self {
        Self {
            id: id.into(),
            handle: handle.into(),
            axis: ScrollbarAxis::Vertical,
            markers: Vec::new(),
            always_visible: true,
            #[cfg(test)]
            debug_selector: None,
        }
    }

    pub fn horizontal(id: impl Into<ElementId>, handle: impl Into<ScrollbarHandle>) -> Self {
        Self {
            id: id.into(),
            handle: handle.into(),
            axis: ScrollbarAxis::Horizontal,
            markers: Vec::new(),
            always_visible: true,
            #[cfg(test)]
            debug_selector: None,
        }
    }

    pub fn markers(mut self, markers: Vec<ScrollbarMarker>) -> Self {
        self.markers = markers;
        self
    }

    pub fn always_visible(mut self) -> Self {
        self.always_visible = true;
        self
    }

    #[cfg(test)]
    pub fn debug_selector(mut self, selector: &'static str) -> Self {
        self.debug_selector = Some(selector);
        self
    }

    pub fn render(self, theme: AppTheme) -> impl IntoElement {
        let handle = self.handle.clone();
        let axis = self.axis;
        let markers = self.markers;
        let id = self.id.clone();
        let always_visible = self.always_visible;

        let prepaint_handle = handle.clone();
        let paint = canvas(
            move |bounds, window, _cx| {
                let margin = px(4.0);
                let (viewport_size, max_offset, raw_offset) = match axis {
                    ScrollbarAxis::Vertical => (
                        bounds.size.height,
                        prepaint_handle.max_offset(axis),
                        prepaint_handle.raw_offset(axis),
                    ),
                    ScrollbarAxis::Horizontal => (
                        bounds.size.width,
                        prepaint_handle.max_offset(axis),
                        prepaint_handle.raw_offset(axis),
                    ),
                };
                let scroll = if raw_offset < px(0.0) {
                    (-raw_offset).max(px(0.0)).min(max_offset)
                } else {
                    raw_offset.max(px(0.0)).min(max_offset)
                };

                let metrics = match axis {
                    ScrollbarAxis::Vertical => {
                        vertical_thumb_metrics(viewport_size, max_offset, scroll)?
                    }
                    ScrollbarAxis::Horizontal => {
                        horizontal_thumb_metrics(viewport_size, max_offset, scroll)?
                    }
                };

                let (track_bounds, thumb_bounds) = match axis {
                    ScrollbarAxis::Vertical => {
                        let track_h = (viewport_size - margin * 2.0).max(px(0.0));
                        let track_bounds = Bounds::new(
                            point(bounds.left(), bounds.top() + margin),
                            size(bounds.size.width, track_h),
                        );

                        let thumb_x = bounds.right() - margin - metrics.thickness;
                        let thumb_bounds = Bounds::new(
                            point(thumb_x, bounds.top() + metrics.offset),
                            size(metrics.thickness, metrics.length),
                        );
                        (track_bounds, thumb_bounds)
                    }
                    ScrollbarAxis::Horizontal => {
                        let track_w = (viewport_size - margin * 2.0).max(px(0.0));
                        let track_bounds = Bounds::new(
                            point(bounds.left() + margin, bounds.top()),
                            size(track_w, bounds.size.height),
                        );

                        let thumb_y = bounds.bottom() - margin - metrics.thickness;
                        let thumb_bounds = Bounds::new(
                            point(bounds.left() + metrics.offset, thumb_y),
                            size(metrics.length, metrics.thickness),
                        );
                        (track_bounds, thumb_bounds)
                    }
                };

                let interaction_bounds = bounds;
                let thumb_hit_bounds = expanded_thumb_hit_bounds(bounds, thumb_bounds, axis);
                let cursor_hitbox = window
                    .insert_hitbox(interaction_bounds, HitboxBehavior::BlockMouseExceptScroll);

                Some(ScrollbarPrepaintState {
                    interaction_bounds,
                    track_bounds,
                    thumb_bounds,
                    thumb_hit_bounds,
                    cursor_hitbox,
                })
            },
            move |bounds, prepaint, window, cx| {
                let interaction = window.use_keyed_state(
                    (id.clone(), "scrollbar_interaction"),
                    cx,
                    |_window, _cx| ScrollbarInteractionState::default(),
                );
                let thumb_visible = prepaint.is_some();
                let visibility_changed = interaction.read(cx).thumb_visible != thumb_visible;
                if visibility_changed {
                    interaction.update(cx, |interaction, cx| {
                        interaction.thumb_visible = thumb_visible;
                        cx.notify();
                    });
                }

                let Some(prepaint) = prepaint else {
                    return;
                };
                let capture_phase = if interaction.read(cx).drag_offset.is_some() {
                    DispatchPhase::Capture
                } else {
                    DispatchPhase::Bubble
                };

                let margin = px(4.0);
                if axis == ScrollbarAxis::Vertical {
                    let track_h = prepaint.track_bounds.size.height.max(px(0.0));

                    let thumb_x = prepaint.thumb_bounds.origin.x;
                    let marker_w = px(4.0);
                    let marker_x = (thumb_x - margin - marker_w).max(bounds.left());

                    for marker in &markers {
                        let start = marker.start.clamp(0.0, 1.0);
                        let end = marker.end.clamp(0.0, 1.0);
                        if end <= start {
                            continue;
                        }

                        let y0 = prepaint.track_bounds.top() + track_h * start;
                        let y1 = prepaint.track_bounds.top() + track_h * end;
                        let min_h = px(2.0);
                        let h = (y1 - y0).max(min_h);

                        let (left, right) = marker_colors(theme, marker.kind);
                        if let Some(left) = left {
                            window.paint_quad(fill(
                                gpui::Bounds::new(point(marker_x, y0), size(marker_w / 2.0, h)),
                                left,
                            ));
                        }
                        if let Some(right) = right {
                            window.paint_quad(fill(
                                gpui::Bounds::new(
                                    point(marker_x + marker_w / 2.0, y0),
                                    size(marker_w / 2.0, h),
                                ),
                                right,
                            ));
                        }
                    }
                }

                let hovered = prepaint.cursor_hitbox.is_hovered(window);
                let is_dragging = interaction.read(cx).drag_offset.is_some();

                let max_offset = handle.max_offset(axis);
                let raw_offset = handle.raw_offset(axis);
                let observed_sign = if raw_offset < px(0.0) {
                    -1
                } else if raw_offset > px(0.0) {
                    1
                } else {
                    interaction.read(cx).offset_sign
                };
                if observed_sign != interaction.read(cx).offset_sign && raw_offset != px(0.0) {
                    interaction.update(cx, |state, _cx| state.offset_sign = observed_sign);
                }
                let scroll = if observed_sign < 0 {
                    (-raw_offset).max(px(0.0)).min(max_offset)
                } else {
                    raw_offset.max(px(0.0)).min(max_offset)
                };
                let show = if always_visible {
                    true
                } else {
                    let scrolled = interaction.read(cx).last_scroll != scroll;
                    if scrolled {
                        interaction.update(cx, |state, _cx| {
                            state.last_scroll = scroll;
                            state.showing = true;
                            state.hide_task.take();
                        });
                    }

                    // Auto-hide: show on hover/drag, then hide after a delay.
                    let state = interaction.read(cx);
                    let show = hovered || is_dragging || state.showing;
                    let should_schedule_hide =
                        !hovered && !is_dragging && state.showing && state.hide_task.is_none();
                    let _ = state;

                    if hovered || is_dragging {
                        interaction.update(cx, |state, _cx| {
                            state.showing = true;
                            state.hide_task.take();
                        });
                    } else if should_schedule_hide {
                        interaction.update(cx, |state, cx| {
                            state.hide_task.take();
                            let task = cx.spawn(
                                async move |state: gpui::WeakEntity<ScrollbarInteractionState>,
                                            cx: &mut gpui::AsyncApp| {
                                    smol::Timer::after(Duration::from_millis(1000)).await;
                                    let _ = state.update(cx, |s, cx| {
                                        if s.drag_offset.is_none() {
                                            s.showing = false;
                                            cx.notify();
                                        }
                                        s.hide_task = None;
                                    });
                                },
                            );
                            state.hide_task = Some(task);
                        });
                    }

                    show
                };
                let thumb_color = if is_dragging {
                    theme.colors.scrollbar_thumb_active
                } else if hovered {
                    theme.colors.scrollbar_thumb_hover
                } else {
                    theme.colors.scrollbar_thumb
                };

                if show {
                    window.paint_quad(fill(prepaint.thumb_bounds, thumb_color));
                }

                if interaction.read(cx).drag_offset.is_some() {
                    window.set_window_cursor_style(CursorStyle::Arrow);
                } else {
                    window.set_cursor_style(CursorStyle::Arrow, &prepaint.cursor_hitbox);
                }

                let interaction_bounds = prepaint.interaction_bounds;
                let track_bounds = prepaint.track_bounds;
                let thumb_bounds = prepaint.thumb_bounds;
                let thumb_hit_bounds = prepaint.thumb_hit_bounds;
                let thumb_size = match axis {
                    ScrollbarAxis::Vertical => thumb_bounds.size.height,
                    ScrollbarAxis::Horizontal => thumb_bounds.size.width,
                };

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    let handle = handle.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != capture_phase || event.button != MouseButton::Left {
                            return;
                        }
                        if !interaction_bounds.contains(&event.position) {
                            return;
                        }

                        let max_offset = handle.max_offset(axis);
                        if max_offset <= px(0.0) {
                            return;
                        }

                        if thumb_hit_bounds.contains(&event.position) {
                            handle.scrollbar_drag_started(axis);
                            let grab = match axis {
                                ScrollbarAxis::Vertical => event.position.y - thumb_bounds.origin.y,
                                ScrollbarAxis::Horizontal => {
                                    event.position.x - thumb_bounds.origin.x
                                }
                            };
                            interaction.update(cx, |state, _cx| {
                                state.drag_offset = Some(grab);
                                if !always_visible {
                                    state.showing = true;
                                    state.hide_task.take();
                                }
                            });
                        } else {
                            interaction.update(cx, |state, _cx| {
                                state.drag_offset = None;
                                if !always_visible {
                                    state.showing = true;
                                    state.hide_task.take();
                                }
                            });
                            let sign = interaction.read(cx).offset_sign;
                            let new_offset = match axis {
                                ScrollbarAxis::Vertical => compute_vertical_click_offset(
                                    clamped_track_axis_position(event.position, track_bounds, axis),
                                    track_bounds,
                                    thumb_size,
                                    thumb_size / 2.0,
                                    max_offset,
                                    sign,
                                ),
                                ScrollbarAxis::Horizontal => compute_horizontal_click_offset(
                                    clamped_track_axis_position(event.position, track_bounds, axis),
                                    track_bounds,
                                    thumb_size,
                                    thumb_size / 2.0,
                                    max_offset,
                                    sign,
                                ),
                            };
                            handle.set_axis_offset(axis, new_offset);
                        }

                        window.refresh();
                        cx.stop_propagation();
                    }
                });

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    let handle = handle.clone();
                    move |event: &MouseMoveEvent, phase, _window, cx| {
                        if phase != capture_phase || !event.dragging() {
                            return;
                        }

                        let Some(grab) = interaction.read(cx).drag_offset else {
                            return;
                        };

                        let max_offset = handle.max_offset(axis);
                        if max_offset <= px(0.0) {
                            return;
                        }

                        let sign = interaction.read(cx).offset_sign;
                        let new_offset = match axis {
                            ScrollbarAxis::Vertical => compute_vertical_click_offset(
                                event.position.y,
                                track_bounds,
                                thumb_size,
                                grab,
                                max_offset,
                                sign,
                            ),
                            ScrollbarAxis::Horizontal => compute_horizontal_click_offset(
                                event.position.x,
                                track_bounds,
                                thumb_size,
                                grab,
                                max_offset,
                                sign,
                            ),
                        };
                        handle.set_axis_offset(axis, new_offset);
                        if !always_visible {
                            interaction.update(cx, |state, _cx| state.showing = true);
                        }
                        _window.refresh();
                        cx.stop_propagation();
                    }
                });

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    move |event: &MouseUpEvent, phase, window, cx| {
                        if phase != capture_phase || event.button != MouseButton::Left {
                            return;
                        }
                        if interaction.read(cx).drag_offset.is_none() {
                            return;
                        }
                        handle.scrollbar_drag_ended(axis);
                        interaction.update(cx, |state, _cx| state.drag_offset = None);
                        window.refresh();
                        cx.stop_propagation();
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full();

        let base = match axis {
            ScrollbarAxis::Vertical => div()
                .id(self.id)
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(px(SCROLLBAR_GUTTER_PX))
                .child(paint),
            ScrollbarAxis::Horizontal => div()
                .id(self.id)
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(SCROLLBAR_GUTTER_PX))
                .child(paint),
        };

        #[cfg(test)]
        let base = match self.debug_selector {
            Some(selector) => base.debug_selector(|| selector.to_string()),
            None => base,
        };

        base
    }

    pub fn visible_gutter(handle: impl Into<ScrollbarHandle>, axis: ScrollbarAxis) -> Pixels {
        let handle: ScrollbarHandle = handle.into();
        if handle.max_offset(axis) > px(0.0) {
            px(SCROLLBAR_GUTTER_PX)
        } else {
            px(0.0)
        }
    }
}

#[cfg(test)]
impl Scrollbar {
    pub fn thumb_visible_for_test(handle: &ScrollHandle, viewport_h_fallback: Pixels) -> bool {
        let viewport_h = viewport_h_fallback;
        let max_offset = handle.max_offset().y.max(px(0.0));
        let raw_offset_y = handle.offset().y;
        let scroll_y = if raw_offset_y < px(0.0) {
            (-raw_offset_y).max(px(0.0)).min(max_offset)
        } else {
            raw_offset_y.max(px(0.0)).min(max_offset)
        };
        vertical_thumb_metrics(viewport_h, max_offset, scroll_y).is_some()
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ThumbMetrics {
    pub(crate) offset: Pixels,
    pub(crate) length: Pixels,
    pub(crate) thickness: Pixels,
}

fn marker_colors(
    theme: AppTheme,
    kind: ScrollbarMarkerKind,
) -> (Option<gpui::Rgba>, Option<gpui::Rgba>) {
    let mut add = theme.colors.diff_add_text;
    let mut rem = theme.colors.diff_remove_text;
    let alpha = if theme.is_dark { 0.70 } else { 0.55 };
    add.a = alpha;
    rem.a = alpha;

    match kind {
        ScrollbarMarkerKind::Add => (Some(add), Some(add)),
        ScrollbarMarkerKind::Remove => (Some(rem), Some(rem)),
        ScrollbarMarkerKind::Modify => (Some(rem), Some(add)),
    }
}

fn expanded_thumb_hit_bounds(
    gutter_bounds: Bounds<Pixels>,
    thumb_bounds: Bounds<Pixels>,
    axis: ScrollbarAxis,
) -> Bounds<Pixels> {
    match axis {
        ScrollbarAxis::Vertical => Bounds::new(
            point(gutter_bounds.left(), thumb_bounds.top()),
            size(gutter_bounds.size.width, thumb_bounds.size.height),
        ),
        ScrollbarAxis::Horizontal => Bounds::new(
            point(thumb_bounds.left(), gutter_bounds.top()),
            size(thumb_bounds.size.width, gutter_bounds.size.height),
        ),
    }
}

fn clamped_track_axis_position(
    position: gpui::Point<Pixels>,
    track_bounds: Bounds<Pixels>,
    axis: ScrollbarAxis,
) -> Pixels {
    match axis {
        ScrollbarAxis::Vertical => position
            .y
            .max(track_bounds.top())
            .min(track_bounds.bottom()),
        ScrollbarAxis::Horizontal => position
            .x
            .max(track_bounds.left())
            .min(track_bounds.right()),
    }
}

pub(crate) fn compute_vertical_click_offset(
    event_y: Pixels,
    track_bounds: Bounds<Pixels>,
    thumb_size: Pixels,
    thumb_offset: Pixels,
    max_offset: Pixels,
    sign_y: i8,
) -> Pixels {
    let viewport_size = track_bounds.size.height.max(px(0.0));
    if viewport_size <= px(0.0) || max_offset <= px(0.0) {
        return px(0.0);
    }

    let max_thumb_start = (viewport_size - thumb_size).max(px(0.0));
    let thumb_start = (event_y - track_bounds.origin.y - thumb_offset)
        .max(px(0.0))
        .min(max_thumb_start);

    let pct = if max_thumb_start > px(0.0) {
        thumb_start / max_thumb_start
    } else {
        0.0
    };

    let scroll_y = (max_offset * pct).max(px(0.0)).min(max_offset);
    let sign = if sign_y < 0 { -1.0 } else { 1.0 };
    scroll_y * sign
}

fn compute_horizontal_click_offset(
    event_x: Pixels,
    track_bounds: Bounds<Pixels>,
    thumb_size: Pixels,
    thumb_offset: Pixels,
    max_offset: Pixels,
    sign_x: i8,
) -> Pixels {
    let viewport_size = track_bounds.size.width.max(px(0.0));
    if viewport_size <= px(0.0) || max_offset <= px(0.0) {
        return px(0.0);
    }

    let max_thumb_start = (viewport_size - thumb_size).max(px(0.0));
    let thumb_start = (event_x - track_bounds.origin.x - thumb_offset)
        .max(px(0.0))
        .min(max_thumb_start);

    let pct = if max_thumb_start > px(0.0) {
        thumb_start / max_thumb_start
    } else {
        0.0
    };

    let scroll_x = (max_offset * pct).max(px(0.0)).min(max_offset);
    let sign = if sign_x < 0 { -1.0 } else { 1.0 };
    scroll_x * sign
}

pub(crate) fn vertical_thumb_metrics(
    viewport_h: Pixels,
    max_offset: Pixels,
    scroll_y: Pixels,
) -> Option<ThumbMetrics> {
    if viewport_h <= px(0.0) || max_offset <= px(0.0) {
        return None;
    }
    let content_h = viewport_h + max_offset;
    let margin = px(4.0);
    let track_h = (viewport_h - margin * 2.0).max(px(0.0));

    let thumb_h = ((viewport_h * (viewport_h / content_h)).max(px(24.0))).min(track_h);
    let available = (track_h - thumb_h).max(px(0.0));

    let pct = if max_offset <= px(0.0) {
        0.0
    } else {
        scroll_y / max_offset
    };

    let top = margin + available * pct;

    Some(ThumbMetrics {
        offset: top,
        length: thumb_h,
        thickness: px(8.0),
    })
}

fn horizontal_thumb_metrics(
    viewport_w: Pixels,
    max_offset: Pixels,
    scroll_x: Pixels,
) -> Option<ThumbMetrics> {
    if viewport_w <= px(0.0) || max_offset <= px(0.0) {
        return None;
    }
    let content_w = viewport_w + max_offset;
    let margin = px(4.0);
    let track_w = (viewport_w - margin * 2.0).max(px(0.0));

    let thumb_w = ((viewport_w * (viewport_w / content_w)).max(px(24.0))).min(track_w);
    let available = (track_w - thumb_w).max(px(0.0));

    let pct = if max_offset <= px(0.0) {
        0.0
    } else {
        scroll_x / max_offset
    };

    let left = margin + available * pct;

    Some(ThumbMetrics {
        offset: left,
        length: thumb_w,
        thickness: px(8.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_metrics_none_without_overflow() {
        assert!(vertical_thumb_metrics(px(100.0), px(0.0), px(0.0)).is_none());
    }

    #[test]
    fn scrollbar_thumb_alpha_in_range() {
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            for c in [
                theme.colors.scrollbar_thumb,
                theme.colors.scrollbar_thumb_hover,
                theme.colors.scrollbar_thumb_active,
            ] {
                assert!(c.a >= 0.0 && c.a <= 1.0);
            }
        }
    }

    #[test]
    fn vertical_thumb_hit_bounds_cover_full_gutter_width() {
        let gutter_bounds = Bounds::new(point(px(100.0), px(20.0)), size(px(16.0), px(120.0)));
        let thumb_bounds = Bounds::new(point(px(104.0), px(40.0)), size(px(8.0), px(24.0)));

        assert_eq!(
            expanded_thumb_hit_bounds(gutter_bounds, thumb_bounds, ScrollbarAxis::Vertical),
            Bounds::new(point(px(100.0), px(40.0)), size(px(16.0), px(24.0)))
        );
    }
}
