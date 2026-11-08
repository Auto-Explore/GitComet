//! macOS boundary elasticity, independent of the document's logical offsets.
//!
//! Wrap the scrolling content, leaving headers and scrollbars outside. The
//! child keeps its layout and virtualization; only prepaint coordinates move.
//! Other platforms and deterministic hosts retain their usual scroll handling.

mod motion;
mod native;
#[cfg(all(test, target_os = "macos"))]
mod tests;

use crate::{ScrollbarAxis, ScrollbarDriver};
use gpui::{
    AnyElement, App, AppContext, Bounds, ContentMask, DispatchPhase, Element, ElementId,
    GlobalElementId, Hitbox, HitboxBehavior, InspectorElementId, IntoElement, LayoutId,
    MouseDownEvent, OngoingScroll, Pixels, ScrollHandle, ScrollWheelEvent, TouchPhase,
    UniformListScrollHandle, Window, WindowId, point, px,
};
use std::cell::RefCell;
use std::rc::Rc;

/// A vertical scroll model. Positions use f64 so very large virtual documents
/// do not lose sub-row precision. `scroll_by` returns the distance consumed.
pub trait ElasticScrollDriver: 'static {
    /// Signed position (negative below the top), and nonnegative maximum.
    fn metrics(&self) -> (f64, f64);
    fn scroll_by(&self, delta: Pixels) -> Pixels;
}

fn pixel_metrics(driver: &impl ScrollbarDriver) -> (f64, f64) {
    (
        f64::from(f32::from(driver.raw_offset(ScrollbarAxis::Vertical))),
        f64::from(f32::from(driver.max_offset(ScrollbarAxis::Vertical))),
    )
}

fn scroll_pixels(driver: &impl ScrollbarDriver, delta: Pixels) -> Pixels {
    let max = driver.max_offset(ScrollbarAxis::Vertical).max(px(0.0));
    let old = driver
        .raw_offset(ScrollbarAxis::Vertical)
        .clamp(-max, px(0.0));
    let next = (old + delta).clamp(-max, px(0.0));
    driver.set_axis_offset(ScrollbarAxis::Vertical, next);
    next - old
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

type WheelListener = Rc<dyn Fn(&ScrollWheelEvent, &mut Window, &mut App)>;

/// Adds vertical rubber-banding to an existing scrollable element.
pub struct ElasticScroll {
    id: ElementId,
    child: Option<AnyElement>,
    driver: Rc<dyn ElasticScrollDriver>,
    on_scroll: Option<WheelListener>,
    vertical_only: bool,
    /// Linked diff columns share displacement while their logical handles
    /// remain independently clamped by the application's sync policy.
    shared_motion: Option<ElasticScrollState>,
}

/// Optional shared visual state for synchronized columns. Own one per pane,
/// never per repository, and reset it when the displayed document changes.
#[derive(Clone, Default)]
pub struct ElasticScrollState(Rc<RefCell<State>>);

impl ElasticScrollState {
    pub fn reset(&self) {
        self.0.borrow_mut().motion.reset();
    }
}

impl ElasticScroll {
    pub fn new(
        id: impl Into<ElementId>,
        driver: impl ElasticScrollDriver,
        child: impl IntoElement,
    ) -> Self {
        Self {
            id: id.into(),
            child: Some(child.into_any_element()),
            driver: Rc::new(driver),
            on_scroll: None,
            vertical_only: false,
            shared_motion: None,
        }
    }

    pub fn vertical_only(mut self) -> Self {
        self.vertical_only = true;
        self
    }

    /// Input bookkeeping normally installed on the child's wheel listener.
    /// Called once when this wrapper owns the precise vertical event.
    pub fn on_scroll(
        mut self,
        listener: impl Fn(&ScrollWheelEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_scroll = Some(Rc::new(listener));
        self
    }

    pub fn shared_motion(mut self, state: ElasticScrollState) -> Self {
        self.shared_motion = Some(state);
        self
    }
}

#[derive(Default)]
struct State {
    motion: motion::Motion,
    axis: OngoingScroll,
    expected_metrics: Option<(f64, f64)>,
    size: Option<gpui::Size<Pixels>>,
    frame_scheduled: bool,
}

/// Only the innermost wrapper under the pointer handles a scroll. Captures
/// claim from outer to inner; bubble listeners check the final claim before
/// the child's native scroll listener can apply the same delta a second time.
#[derive(Default)]
struct WheelClaim(Option<(WindowId, GlobalElementId)>);
impl gpui::Global for WheelClaim {}

pub struct ElasticScrollPrepaintState {
    hitbox: Hitbox,
    state: ElasticScrollState,
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
        if !cfg!(target_os = "macos") || !crate::ui_runtime::current().uses_pane_animations() {
            child.prepaint(window, cx);
            return None;
        }
        let state = self.shared_motion.clone().unwrap_or_else(|| {
            window.with_element_state(id.expect("elastic scroll id"), |state, _| {
                let state: ElasticScrollState = state.unwrap_or_default();
                (state.clone(), state)
            })
        });
        let state_weak = Rc::downgrade(&state.0);
        let view = window.current_view();
        let offset = {
            let mut state = state.0.borrow_mut();
            let metrics = self.driver.metrics();
            let offset = state.motion.offset();
            if (self.shared_motion.is_none()
                && state
                    .expected_metrics
                    .is_some_and(|expected| expected != metrics))
                || state
                    .size
                    .is_some_and(|size| size.height != bounds.size.height)
                || (offset > 0.0 && metrics.0 < -0.5)
                || (offset < 0.0 && metrics.0 > -metrics.1 + 0.5)
                || !motion_enabled(cx)
                || !window.is_window_active()
            {
                state.motion.reset();
            }
            state.expected_metrics = Some(metrics);
            state.size = Some(bounds.size);
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
            px(state.motion.offset())
        };
        let hitbox = window.insert_hitbox(bounds, HitboxBehavior::Normal);
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| {
                window.with_element_offset(point(px(0.0), offset), |window| {
                    child.prepaint(window, cx)
                });
            },
        );
        Some(ElasticScrollPrepaintState { hitbox, state })
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
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
        let id = id.expect("elastic scroll id").clone();
        let window_id = window.window_handle().window_id();
        let view = window.current_view();
        window.on_mouse_event({
            let id = id.clone();
            let hitbox = prepaint.hitbox.clone();
            let state = prepaint.state.clone();
            move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Capture {
                    return;
                }
                if matches!(
                    native::read(event).phase,
                    TouchPhase::Ended | TouchPhase::Cancelled
                ) && state.0.borrow().motion.touching()
                {
                    state
                        .0
                        .borrow_mut()
                        .motion
                        .release(cx.background_executor().now());
                    cx.notify(view);
                }
                if hitbox.should_handle_scroll(window) {
                    cx.set_global(WheelClaim(Some((window_id, id.clone()))));
                }
            }
        });
        window.on_mouse_event({
            let state = prepaint.state.clone();
            move |_: &MouseDownEvent, phase, _, cx| {
                if phase == DispatchPhase::Capture && state.0.borrow().motion.offset() != 0.0 {
                    state.0.borrow_mut().motion.reset();
                    cx.notify(view);
                }
            }
        });
        window.with_content_mask(
            Some(ContentMask {
                bounds,
                ..Default::default()
            }),
            |window| child.paint(window, cx),
        );
        window.on_mouse_event({
            let hitbox = prepaint.hitbox.clone();
            let state = prepaint.state.clone();
            let driver = self.driver.clone();
            let on_scroll = self.on_scroll.clone();
            let vertical_only = self.vertical_only;
            move |event: &ScrollWheelEvent, phase, window, cx| {
                if phase != DispatchPhase::Bubble
                    || !hitbox.should_handle_scroll(window)
                    || !cx
                        .try_global::<WheelClaim>()
                        .is_some_and(|claim| claim.0.as_ref() == Some(&(window_id, id.clone())))
                {
                    return;
                }
                let mut state = state.0.borrow_mut();
                if !event.delta.precise() {
                    if state.motion.offset() != 0.0 {
                        cx.notify(view);
                    }
                    state.motion.reset();
                    return;
                }
                let now = cx.background_executor().now();
                let mut delta = event.delta.pixel_delta(window.line_height());
                if vertical_only {
                    state.axis.filter(&mut delta, event.touch_phase);
                }
                let animate = motion_enabled(cx);
                if animate {
                    let input = native::read(event);
                    if input.source == Some(native::Source::Wheel) {
                        state.motion.reset();
                    } else {
                        let phase = if input.source == Some(native::Source::Touch)
                            && input.phase == TouchPhase::Moved
                            && !state.motion.touching()
                        {
                            TouchPhase::Started
                        } else {
                            input.phase
                        };
                        state.motion.begin(phase, now);
                        if input.source == Some(native::Source::Momentum) {
                            state.motion.allow_native_momentum(now);
                        }
                    }
                } else {
                    state.motion.reset();
                }
                if delta.x.abs() > delta.y.abs() {
                    return;
                }
                let delta = f32::from(delta.y);
                if !delta.is_finite() {
                    return;
                }
                let remaining = state.motion.take_reversal(delta, now);
                let consumed = driver.scroll_by(px(remaining));
                if animate {
                    state.motion.record_delta(delta, now);
                    state
                        .motion
                        .overscroll(remaining - f32::from(consumed), now);
                }
                state.expected_metrics = Some(driver.metrics());
                drop(state);
                if let Some(on_scroll) = &on_scroll {
                    on_scroll(event, window, cx);
                }
                cx.notify(view);
                cx.stop_propagation();
            }
        });
    }
}
