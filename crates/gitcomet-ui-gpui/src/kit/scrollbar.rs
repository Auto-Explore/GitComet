use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{
    Bounds, CursorStyle, DispatchPhase, ElementId, Hitbox, HitboxBehavior, ListState, MouseButton,
    MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, ScrollHandle, UniformListScrollHandle,
    canvas, div, fill, point, px, size,
};
use std::sync::Arc;
use std::time::Duration;

pub const SCROLLBAR_GUTTER_PX: f32 = 16.0;
const SCROLLBAR_THUMB_THICKNESS_PX: f32 = 6.0;

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

/// Trait abstracting the scroll model a scrollbar drives.
///
/// Implementations exist for:
/// - GPUI `ScrollHandle`
/// - GPUI `UniformListScrollHandle`
/// - Alacritty terminal scroll model
pub trait ScrollbarDriver: 'static {
    /// Maximum scroll extent in pixels for the given axis.
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels;

    /// Current scroll position as a signed pixel offset.
    /// Sign convention: negative values represent "reversed" scroll direction
    /// (e.g. newer content at higher offsets).
    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels;

    /// Set the scroll position for the given axis.
    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels);

    /// Lifecycle: called when a scrollbar drag starts.
    fn drag_started(&self, _axis: ScrollbarAxis) {}

    /// Lifecycle: called when a scrollbar drag ends.
    fn drag_ended(&self, _axis: ScrollbarAxis) {}
}

// ---------------------------------------------------------------------------
// Scrollbar driver implementations for GPUI scroll handles
// ---------------------------------------------------------------------------

impl ScrollbarDriver for ScrollHandle {
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match axis {
            ScrollbarAxis::Vertical => self.max_offset().y.max(px(0.0)),
            ScrollbarAxis::Horizontal => self.max_offset().x.max(px(0.0)),
        }
    }

    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match axis {
            ScrollbarAxis::Vertical => self.offset().y,
            ScrollbarAxis::Horizontal => self.offset().x,
        }
    }

    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels) {
        let current = self.offset();
        match axis {
            ScrollbarAxis::Vertical => self.set_offset(point(current.x, offset)),
            ScrollbarAxis::Horizontal => self.set_offset(point(offset, current.y)),
        }
    }

    fn drag_started(&self, _axis: ScrollbarAxis) {}

    fn drag_ended(&self, _axis: ScrollbarAxis) {}
}

impl ScrollbarDriver for UniformListScrollHandle {
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match axis {
            ScrollbarAxis::Vertical => self
                .0
                .borrow()
                .last_item_size
                .map(|size| (size.contents.height - size.item.height).max(px(0.0)))
                .unwrap_or_else(|| self.0.borrow().base_handle.max_offset().y),
            ScrollbarAxis::Horizontal => self
                .0
                .borrow()
                .last_item_size
                .map(|size| (size.contents.width - size.item.width).max(px(0.0)))
                .unwrap_or_else(|| self.0.borrow().base_handle.max_offset().x),
        }
    }

    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        let base = self.0.borrow().base_handle.clone();
        match axis {
            ScrollbarAxis::Vertical => base.offset().y,
            ScrollbarAxis::Horizontal => base.offset().x,
        }
    }

    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels) {
        let base = self.0.borrow().base_handle.clone();
        let current = base.offset();
        match axis {
            ScrollbarAxis::Vertical => base.set_offset(point(current.x, offset)),
            ScrollbarAxis::Horizontal => base.set_offset(point(offset, current.y)),
        }
    }

    fn drag_started(&self, _axis: ScrollbarAxis) {}

    fn drag_ended(&self, _axis: ScrollbarAxis) {}
}

impl ScrollbarDriver for ListState {
    fn max_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match axis {
            // `list` virtualizes vertically only.
            ScrollbarAxis::Vertical => self.max_offset_for_scrollbar().y.max(px(0.0)),
            ScrollbarAxis::Horizontal => px(0.0),
        }
    }

    fn raw_offset(&self, axis: ScrollbarAxis) -> Pixels {
        match axis {
            // Negative y = scrolled down, matching the driver's sign convention.
            ScrollbarAxis::Vertical => self.scroll_px_offset_for_scrollbar().y,
            ScrollbarAxis::Horizontal => px(0.0),
        }
    }

    fn set_axis_offset(&self, axis: ScrollbarAxis, offset: Pixels) {
        if axis == ScrollbarAxis::Vertical {
            self.set_offset_from_scrollbar(point(px(0.0), offset));
        }
    }

    fn drag_started(&self, _axis: ScrollbarAxis) {
        self.scrollbar_drag_started();
    }

    fn drag_ended(&self, _axis: ScrollbarAxis) {
        self.scrollbar_drag_ended();
    }
}

// ---------------------------------------------------------------------------
// Scrollbar component
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct Scrollbar {
    id: ElementId,
    driver: Arc<dyn ScrollbarDriver>,
    axis: ScrollbarAxis,
    markers: Vec<ScrollbarMarker>,
    always_visible: bool,
    #[cfg(test)]
    debug_selector: Option<&'static str>,
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

#[derive(Clone, Debug)]
struct ScrollbarPrepaintState {
    interaction_bounds: Bounds<Pixels>,
    track_bounds: Bounds<Pixels>,
    thumb_bounds: Bounds<Pixels>,
    thumb_hit_bounds: Bounds<Pixels>,
    cursor_hitbox: Hitbox,
    scroll: Pixels,
}

impl Scrollbar {
    pub fn new(id: impl Into<ElementId>, driver: impl ScrollbarDriver) -> Self {
        Self {
            id: id.into(),
            driver: Arc::new(driver),
            axis: ScrollbarAxis::Vertical,
            markers: Vec::new(),
            always_visible: true,
            #[cfg(test)]
            debug_selector: None,
        }
    }

    pub fn horizontal(id: impl Into<ElementId>, driver: impl ScrollbarDriver) -> Self {
        Self {
            id: id.into(),
            driver: Arc::new(driver),
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

    /// Show the thumb only while scrolling or hovering the track, fading it
    /// out afterwards.
    pub fn auto_hide(mut self) -> Self {
        self.always_visible = false;
        self
    }

    #[cfg(test)]
    pub fn debug_selector(mut self, selector: &'static str) -> Self {
        self.debug_selector = Some(selector);
        self
    }

    pub fn render(self, theme: AppTheme) -> impl IntoElement {
        let driver = self.driver.clone();
        let axis = self.axis;
        let markers = self.markers;
        let id = self.id.clone();
        let always_visible = self.always_visible;

        let prepaint_driver = driver.clone();
        let paint = canvas(
            move |bounds, window, _cx| {
                let margin = px(4.0);
                let (viewport_size, max_offset, raw_offset) = match axis {
                    ScrollbarAxis::Vertical => (
                        bounds.size.height,
                        prepaint_driver.max_offset(axis),
                        prepaint_driver.raw_offset(axis),
                    ),
                    ScrollbarAxis::Horizontal => (
                        bounds.size.width,
                        prepaint_driver.max_offset(axis),
                        prepaint_driver.raw_offset(axis),
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
                    scroll,
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

                let scroll = prepaint.scroll;
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
                    theme.colors.scrollbar.thumb_pressed
                } else if hovered {
                    theme.colors.scrollbar.thumb_hover
                } else {
                    theme.colors.scrollbar.thumb
                };

                if show {
                    window.paint_quad(
                        fill(prepaint.thumb_bounds, thumb_color)
                            .corner_radii(px(SCROLLBAR_THUMB_THICKNESS_PX / 2.0)),
                    );
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
                    let driver = driver.clone();
                    move |event: &MouseDownEvent, phase, window, cx| {
                        if phase != capture_phase || event.button != MouseButton::Left {
                            return;
                        }
                        if !interaction_bounds.contains(&event.position) {
                            return;
                        }

                        let max_offset = driver.max_offset(axis);
                        if max_offset <= px(0.0) {
                            return;
                        }

                        crate::press_gesture::claim_press(cx);

                        if thumb_hit_bounds.contains(&event.position) {
                            driver.drag_started(axis);
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
                            driver.set_axis_offset(axis, new_offset);
                        }

                        window.refresh();
                        cx.stop_propagation();
                    }
                });

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    let driver = driver.clone();
                    move |event: &MouseMoveEvent, phase, _window, cx| {
                        if phase != capture_phase || !event.dragging() {
                            return;
                        }

                        let Some(grab) = interaction.read(cx).drag_offset else {
                            return;
                        };

                        let max_offset = driver.max_offset(axis);
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
                        driver.set_axis_offset(axis, new_offset);
                        if !always_visible {
                            interaction.update(cx, |state, _cx| state.showing = true);
                        }
                        _window.refresh();
                        cx.stop_propagation();
                    }
                });

                window.on_mouse_event({
                    let interaction = interaction.clone();
                    let driver = driver.clone();
                    move |event: &MouseUpEvent, phase, window, cx| {
                        if phase != capture_phase || event.button != MouseButton::Left {
                            return;
                        }
                        if interaction.read(cx).drag_offset.is_none() {
                            return;
                        }
                        driver.drag_ended(axis);
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

    pub fn gutter(_axis: ScrollbarAxis) -> Pixels {
        px(SCROLLBAR_GUTTER_PX)
    }

    pub fn visible_gutter(driver: impl ScrollbarDriver, axis: ScrollbarAxis) -> Pixels {
        if driver.max_offset(axis) > px(0.0) {
            Self::gutter(axis)
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

// ---------------------------------------------------------------------------
// Thumb metrics
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct ThumbMetrics {
    pub(crate) offset: Pixels,
    pub(crate) length: Pixels,
    pub(crate) thickness: Pixels,
}

// ---------------------------------------------------------------------------
// Render helpers
// ---------------------------------------------------------------------------

fn marker_colors(
    theme: AppTheme,
    kind: ScrollbarMarkerKind,
) -> (Option<gpui::Rgba>, Option<gpui::Rgba>) {
    let mut add = theme.colors.diff.added.foreground;
    let mut rem = theme.colors.diff.removed.foreground;
    let alpha = if theme.is_dark { 0.70 } else { 0.55 };
    add.alpha = alpha;
    rem.alpha = alpha;

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
        thickness: px(SCROLLBAR_THUMB_THICKNESS_PX),
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
        thickness: px(SCROLLBAR_THUMB_THICKNESS_PX),
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
                theme.colors.scrollbar.thumb,
                theme.colors.scrollbar.thumb_hover,
                theme.colors.scrollbar.thumb_pressed,
            ] {
                assert!(c.alpha >= 0.0 && c.alpha <= 1.0);
            }
        }
    }

    #[test]
    fn vertical_thumb_hit_bounds_cover_full_gutter_width() {
        let gutter_bounds = Bounds::new(point(px(100.0), px(20.0)), size(px(16.0), px(120.0)));
        let thumb_bounds = Bounds::new(
            point(px(106.0), px(40.0)),
            size(px(SCROLLBAR_THUMB_THICKNESS_PX), px(24.0)),
        );

        assert_eq!(
            expanded_thumb_hit_bounds(gutter_bounds, thumb_bounds, ScrollbarAxis::Vertical),
            Bounds::new(point(px(100.0), px(40.0)), size(px(16.0), px(24.0)))
        );
    }
}
