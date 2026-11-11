//! macOS boundary elasticity, independent of the document's logical offsets.
//!
//! Wrap the scrolling content, leaving headers and scrollbars outside. The
//! child keeps its layout and virtualization; only prepaint coordinates move.
//! Other platforms and deterministic hosts retain their usual scroll handling.

mod motion;
mod native;
mod routing;
mod surfaces;
mod tracked;
pub use tracked::TrackedElasticScroll;
#[cfg(all(test, target_os = "macos"))]
mod linked_work_tests;
#[cfg(all(test, target_os = "macos"))]
mod regression_tests;
#[cfg(all(test, target_os = "macos"))]
mod surface_tests;
#[cfg(all(test, target_os = "macos"))]
mod tests;
#[cfg(all(test, target_os = "macos"))]
mod work_tests;

use crate::{ScrollbarAxis, ScrollbarDriver};
use gpui::{
    AnyElement, App, AppContext, Bounds, ContentMask, DispatchPhase, Element, ElementId,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseDownEvent, OngoingScroll, Pixels, ScrollHandle, ScrollWheelEvent, TouchPhase,
    UniformListScrollHandle, Window, point, px,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// A scroll model for one axis. Positions use f64 so very large virtual documents
/// do not lose sub-row precision. `scroll_by` returns the distance consumed.
pub trait ElasticScrollDriver: 'static {
    /// Signed position (negative below the top), and nonnegative maximum.
    /// Driver methods run synchronously while motion is borrowed; use the
    /// wrapper's callbacks for work that reads or resets its motion state.
    fn metrics(&self) -> (f64, f64);
    /// Returns the signed logical distance actually consumed. A zero result
    /// alone requests no repaint. Notify from the owner, or install a scroll
    /// callback, if the operation changes other visible state at a clamped edge.
    fn scroll_by(&self, delta: Pixels) -> Pixels;
    /// A partially measured list may need its native handler to discover more
    /// rows before its scrollbar estimate represents a real boundary.
    fn should_handle(&self, _delta: Pixels) -> bool {
        true
    }
}

fn pixel_metrics(driver: &impl ScrollbarDriver) -> (f64, f64) {
    axis_metrics(driver, ScrollbarAxis::Vertical)
}

fn axis_metrics(driver: &impl ScrollbarDriver, axis: ScrollbarAxis) -> (f64, f64) {
    (
        f64::from(f32::from(driver.raw_offset(axis))),
        f64::from(f32::from(driver.max_offset(axis))),
    )
}

fn scroll_pixels(driver: &impl ScrollbarDriver, delta: Pixels) -> Pixels {
    scroll_axis_pixels(driver, delta, ScrollbarAxis::Vertical)
}

fn scroll_axis_pixels(driver: &impl ScrollbarDriver, delta: Pixels, axis: ScrollbarAxis) -> Pixels {
    let max = driver.max_offset(axis).max(px(0.0));
    let old = driver.raw_offset(axis).clamp(-max, px(0.0));
    let next = (old + delta).clamp(-max, px(0.0));
    driver.set_axis_offset(axis, next);
    next - old
}

struct HorizontalDriver(ScrollHandle);

impl ElasticScrollDriver for HorizontalDriver {
    fn metrics(&self) -> (f64, f64) {
        axis_metrics(&self.0, ScrollbarAxis::Horizontal)
    }

    fn scroll_by(&self, delta: Pixels) -> Pixels {
        scroll_axis_pixels(&self.0, delta, ScrollbarAxis::Horizontal)
    }
}

impl ElasticScrollDriver for ScrollHandle {
    fn metrics(&self) -> (f64, f64) {
        pixel_metrics(self)
    }
    fn scroll_by(&self, delta: Pixels) -> Pixels {
        scroll_pixels(self, delta)
    }
}

impl ElasticScrollDriver for UniformListScrollHandle {
    fn metrics(&self) -> (f64, f64) {
        pixel_metrics(self)
    }
    fn scroll_by(&self, delta: Pixels) -> Pixels {
        self.0.borrow_mut().deferred_scroll_to_item = None;
        scroll_pixels(self, delta)
    }
}

impl ElasticScrollDriver for gpui::ListState {
    fn metrics(&self) -> (f64, f64) {
        pixel_metrics(self)
    }

    fn should_handle(&self, delta: Pixels) -> bool {
        if self.is_scrolled_to_end().is_some() {
            return true;
        }
        // The scrollbar describes measured rows, not the unknown document's
        // actual extent. Keep native list scrolling until a genuine rendered
        // boundary is known; do not turn its provisional maximum into a spring.
        let top = self.logical_scroll_top();
        if delta >= px(0.0) && top.item_ix == 0 && top.offset_in_item <= px(0.0) {
            return true;
        }
        let viewport = self.viewport_bounds();
        delta <= px(0.0)
            && viewport.size.height > px(0.0)
            && self.item_count().checked_sub(1).is_none_or(|last| {
                self.bounds_for_item(last)
                    .is_some_and(|bounds| bounds.bottom() <= viewport.bottom() + px(0.5))
            })
    }

    fn scroll_by(&self, delta: Pixels) -> Pixels {
        let known_extent = self.is_scrolled_to_end().is_some();
        let outward_at_boundary = self.should_handle(delta);
        let max = self.max_offset_for_scrollbar().y.max(px(0.0));
        let old = self.scroll_px_offset_for_scrollbar().y.clamp(-max, px(0.0));
        let next = (old + delta).clamp(-max, px(0.0));
        // ListState's public pixel scroll uses positive distances downward.
        // Apply the same measured-range clamp as its native wheel listener.
        gpui::ListState::scroll_by(self, old - next);
        if known_extent || outward_at_boundary {
            next - old
        } else {
            // Only an active spring reversal reaches this path: after taking
            // up its stretched distance, the remaining inward intent follows
            // the native clamp without treating unknown rows as a new edge.
            delta
        }
    }
}

type WheelListener = Box<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>;
type WheelPredicate = Rc<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App) -> bool>;
type LogicalScrollListener = Box<dyn Fn(Pixels, &ScrollWheelEvent, &mut Window, &mut App)>;

/// Adds rubber-banding to an existing scrollable element.
pub struct ElasticScroll {
    id: ElementId,
    child: Option<AnyElement>,
    driver: Rc<dyn ElasticScrollDriver>,
    on_scroll: Option<WheelListener>,
    wheel_predicate: Option<WheelPredicate>,
    on_logical_scroll: Option<LogicalScrollListener>,
    vertical_only: bool,
    axis: ScrollbarAxis,
    horizontal_only: bool,
    /// Linked diff columns share displacement while their logical handles
    /// remain independently clamped by the application's sync policy.
    shared_motion: Option<ElasticScrollState>,
    linked_motion: bool,
}

/// Optional shared visual state for synchronized columns. Own one per pane,
/// never per repository, and reset it when the displayed document changes.
#[derive(Clone, Default)]
pub struct ElasticScrollState(Rc<RefCell<State>>);

impl ElasticScrollState {
    pub fn reset(&self) {
        let mut state = self.0.borrow_mut();
        state.motion.reset();
        state.expected_driver = None;
        state.geometry_reset_scheduled = false;
    }

    /// The displacement used by the most recent prepaint. Hit testing and IME
    /// geometry can use it without treating it as a logical scroll offset.
    pub fn displacement(&self) -> Pixels {
        px(self.0.borrow().applied_offset)
    }
}

impl ElasticScroll {
    /// Whether this platform/runtime installs the precise-wheel router.
    /// Custom logical scrollers must leave even axis-rejected precise packets
    /// to that router. Reduce Motion disables stretching, not input routing.
    pub fn uses_precise_wheel_routing() -> bool {
        cfg!(target_os = "macos") && crate::ui_runtime::current().uses_pane_animations()
    }

    /// Tracks an ordinary overflow area without adding state to its owning view.
    /// The handle lives with this element's scoped id across rendered frames.
    pub fn tracked_vertical(
        id: impl Into<ElementId>,
        surface: gpui::Stateful<gpui::Div>,
    ) -> TrackedElasticScroll {
        TrackedElasticScroll::new(id.into(), surface, ScrollbarAxis::Vertical)
    }

    /// Tracks a horizontal overflow area, retaining its native wheel mapping.
    pub fn tracked_horizontal(
        id: impl Into<ElementId>,
        surface: gpui::Stateful<gpui::Div>,
    ) -> TrackedElasticScroll {
        TrackedElasticScroll::new(id.into(), surface, ScrollbarAxis::Horizontal)
    }

    pub fn new(
        id: impl Into<ElementId>,
        driver: impl ElasticScrollDriver,
        child: impl IntoElement,
    ) -> Self {
        Self::with_shared_driver(id, Rc::new(driver), child)
    }

    /// Reuses an owner-retained driver without allocating an erased driver on
    /// each render. The current callbacks are still installed afresh each paint.
    pub fn with_shared_driver(
        id: impl Into<ElementId>,
        driver: Rc<dyn ElasticScrollDriver>,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            child: Some(child.into_any_element()),
            driver,
            on_scroll: None,
            wheel_predicate: None,
            on_logical_scroll: None,
            vertical_only: false,
            axis: ScrollbarAxis::Vertical,
            horizontal_only: false,
            shared_motion: None,
            linked_motion: false,
        }
    }

    pub fn vertical_only(mut self) -> Self {
        self.vertical_only = true;
        self
    }

    /// A horizontal strip. Like GPUI's unrestricted horizontal scrollers,
    /// vertical-only wheel packets are mapped onto the horizontal axis.
    pub fn horizontal(
        id: impl Into<ElementId>,
        scroll: ScrollHandle,
        child: impl IntoElement,
    ) -> Self {
        let mut element = Self::new(id, HorizontalDriver(scroll), child);
        element.axis = ScrollbarAxis::Horizontal;
        element
    }

    /// A nested horizontal region must leave vertical gestures to its parent.
    pub fn horizontal_only(mut self) -> Self {
        self.horizontal_only = true;
        self
    }

    /// Input bookkeeping normally installed on the child's wheel listener.
    /// Called once when this wrapper owns the precise vertical event.
    pub fn on_scroll(
        mut self,
        listener: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_scroll = Some(Box::new(listener));
        self
    }

    /// Leaves a packet to the child's native handler when its live input mode
    /// owns scrolling (for example terminal mouse reporting).
    pub fn when_wheel(
        mut self,
        predicate: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) -> bool + 'static,
    ) -> Self {
        self.wheel_predicate = Some(Rc::new(predicate));
        self
    }

    /// Logical intent remaining after a reversal takes up stretched distance,
    /// before this driver's boundary clamp. Linked models with unequal ranges
    /// can forward this delta without moving while a spring is unwinding.
    pub fn on_logical_scroll(
        mut self,
        listener: impl Fn(Pixels, &ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_logical_scroll = Some(Box::new(listener));
        self
    }

    pub fn shared_motion(mut self, state: ElasticScrollState) -> Self {
        self.shared_motion = Some(state);
        self.linked_motion = true;
        self
    }

    /// Exposes one surface's motion to its owner while retaining automatic
    /// cancellation when its logical position, extent or viewport changes.
    pub fn motion_state(mut self, state: ElasticScrollState) -> Self {
        self.shared_motion = Some(state);
        self.linked_motion = false;
        self
    }
}

#[derive(Default)]
struct State {
    motion: motion::Motion,
    frame_scheduled: bool,
    applied_offset: f32,
    expected_driver: Option<(Rc<dyn ElasticScrollDriver>, (f64, f64))>,
    geometry_reset_scheduled: bool,
}

impl State {
    fn schedule_geometry_reset(
        &mut self,
        state_weak: std::rc::Weak<RefCell<Self>>,
        view: gpui::EntityId,
        window: &mut Window,
    ) {
        if self.geometry_reset_scheduled {
            return;
        }
        self.geometry_reset_scheduled = true;
        window.on_next_frame(move |_, cx| {
            if let Some(state) = state_weak.upgrade() {
                let reset = {
                    let mut state = state.borrow_mut();
                    if state.geometry_reset_scheduled {
                        state.motion.reset();
                        state.expected_driver = None;
                        state.geometry_reset_scheduled = false;
                        true
                    } else {
                        false
                    }
                };
                if reset {
                    cx.notify(view);
                }
            }
        });
    }
}

#[derive(Default)]
struct LocalState {
    claim_token: u64,
    axis: OngoingScroll,
    expected_metrics: Option<(f64, f64)>,
    size: Option<gpui::Size<Pixels>>,
    captured_delta: Option<Pixels>,
}

/// Only the innermost wrapper under the pointer handles a scroll. Captures
/// claim from outer to inner; bubble listeners check the final claim before
/// the child's native scroll listener can apply the same delta a second time.
#[derive(Default)]
struct WheelClaim {
    owner: Cell<Option<u64>>,
}
impl gpui::Global for WheelClaim {}

impl WheelClaim {
    fn register_surface() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        // Test hosts may clear App globals while retaining painted elements.
        // Never recycle their tokens when a new claim global is initialized.
        static NEXT: AtomicU64 = AtomicU64::new(1);
        NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |token| {
            token.checked_add(1)
        })
        .expect("scroll surface ids exhausted")
    }
}

pub struct ElasticScrollPrepaintState {
    hitbox: Hitbox,
    state: ElasticScrollState,
    local: Rc<RefCell<LocalState>>,
}

fn motion_enabled(cx: &App) -> bool {
    if cx.reduce_motion() {
        return false;
    }
    #[cfg(all(target_os = "macos", not(test)))]
    {
        // Public AppKit API; already linked by GPUI. Query on the UI thread so
        // changes in Accessibility settings take effect without a restart.
        if objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion() {
            return false;
        }
    }
    true
}

impl IntoElement for ElasticScroll {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for ElasticScroll {
    type RequestLayoutState = AnyElement;
    type PrepaintState = Option<ElasticScrollPrepaintState>;

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, AnyElement) {
        if self.linked_motion
            && let Some(state) = &self.shared_motion
        {
            let mut state = state.0.borrow_mut();
            if state
                .expected_driver
                .as_ref()
                .is_some_and(|(driver, expected)| driver.metrics() != *expected)
            {
                // All request-layout calls precede prepaint. Check the input
                // master's model here so every linked column sees a reset in
                // the same frame, regardless of sibling/prepaint order.
                state.motion.reset();
                state.expected_driver = None;
                state.geometry_reset_scheduled = false;
            }
        }
        let mut child = self.child.take().expect("elastic scroll child");
        (child.request_layout(window, cx), child)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        if !Self::uses_precise_wheel_routing() {
            if let Some(state) = &self.shared_motion {
                let mut state = state.0.borrow_mut();
                state.motion.reset();
                state.applied_offset = 0.0;
            }
            child.prepaint(window, cx);
            return None;
        }
        let state = self.shared_motion.clone().unwrap_or_else(|| {
            window.with_element_state(id.expect("elastic scroll id"), |state, _| {
                let state: ElasticScrollState = state.unwrap_or_default();
                (state.clone(), state)
            })
        });
        let local = window.with_element_state(id.expect("elastic scroll id"), |local, _| {
            let local: Rc<RefCell<LocalState>> = local.unwrap_or_else(|| {
                if cx.try_global::<WheelClaim>().is_none() {
                    cx.set_global(WheelClaim::default());
                }
                Rc::new(RefCell::new(LocalState {
                    claim_token: WheelClaim::register_surface(),
                    ..Default::default()
                }))
            });
            (local.clone(), local)
        });
        let state_weak = Rc::downgrade(&state.0);
        let view = window.current_view();
        let offset = {
            let mut state = state.0.borrow_mut();
            let mut local = local.borrow_mut();
            let metrics = self.driver.metrics();
            let offset = state.motion.offset();
            let resized = local.size.is_some_and(|size| match self.axis {
                ScrollbarAxis::Vertical => size.height != bounds.size.height,
                ScrollbarAxis::Horizontal => size.width != bounds.size.width,
            });
            if self.linked_motion && resized && offset != 0.0 {
                // Keep this frame aligned, then clear the group before the
                // next frame, even if a short document still has zero extent.
                state.schedule_geometry_reset(state_weak.clone(), view, window);
            }
            // Linked columns can have different extents and independently
            // clamped offsets. Only their input master admits unused travel;
            // a follower must never clear shared motion during prepaint.
            // Owners reset shared state on document/presentation changes.
            let local_geometry_changed = !self.linked_motion
                && (local
                    .expected_metrics
                    .is_some_and(|expected| expected != metrics)
                    || resized
                    || (offset > 0.0 && metrics.0 < -0.5)
                    || (offset < 0.0 && metrics.0 > -metrics.1 + 0.5));
            if local_geometry_changed
                || (offset != 0.0 && !motion_enabled(cx))
                || !window.is_window_active()
            {
                state.motion.reset();
            }
            local.expected_metrics = Some(metrics);
            local.size = Some(bounds.size);
            if !state.frame_scheduled {
                state.motion.advance(cx.background_executor().now());
            }
            if state.motion.animating() && !state.frame_scheduled {
                state.frame_scheduled = true;
                // Shared columns evaluate one spring position per frame and
                // schedule one pane invalidation, keeping their rows aligned.
                window.on_next_frame(move |_, cx| {
                    if let Some(state) = state_weak.upgrade() {
                        let animate = {
                            let mut state = state.borrow_mut();
                            state.frame_scheduled = false;
                            state.motion.animating()
                        };
                        if animate {
                            cx.notify(view);
                        }
                    }
                });
            }
            // GPUI snaps element bounds to device pixels. Record the actual
            // transform so fixed chrome can subtract it without fractional
            // drift from the snapped child coordinates.
            let scale = window.scale_factor();
            state.applied_offset = (state.motion.offset() * scale).round() / scale;
            px(state.applied_offset)
        };
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                let offset = match self.axis {
                    ScrollbarAxis::Vertical => point(px(0.0), offset),
                    ScrollbarAxis::Horizontal => point(offset, px(0.0)),
                };
                window.with_element_offset(offset, |window| child.prepaint(window, cx));
            },
        );
        {
            let mut motion = state.0.borrow_mut();
            // Only a displaced surface needs a post-measurement cleanup.
            // ListState and terminal metrics can otherwise repeat expensive work
            // after every unrelated redraw, even while resting at zero.
            if motion.motion.offset() != 0.0 {
                let changed = if self.linked_motion {
                    motion
                        .expected_driver
                        .as_ref()
                        .is_some_and(|(driver, expected)| driver.metrics() != *expected)
                } else {
                    local
                        .borrow()
                        .expected_metrics
                        .is_some_and(|expected| expected != self.driver.metrics())
                };
                if changed {
                    motion.schedule_geometry_reset(Rc::downgrade(&state.0), view, window);
                }
            }
        }
        Some(ElasticScrollPrepaintState {
            hitbox,
            state,
            local,
        })
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        child: &mut AnyElement,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let Some(prepaint) = prepaint else {
            child.paint(window, cx);
            return;
        };
        routing::paint(self, bounds, child, prepaint, window, cx);
    }
}
