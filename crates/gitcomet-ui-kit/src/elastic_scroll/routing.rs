//! One capture/bubble router for every elastic scroll surface.
use super::*;

#[derive(Clone, Copy)]
struct AxisPolicy {
    axis: ScrollbarAxis,
    restrict: bool,
    horizontal_only: bool,
}

impl AxisPolicy {
    fn delta(
        self,
        event: &ScrollWheelEvent,
        ongoing: &mut OngoingScroll,
        line_height: Pixels,
    ) -> Option<Pixels> {
        let mut delta = event.delta.pixel_delta(line_height);
        if self.restrict && event.delta.precise() {
            // Track rejected packets too: diagonal jitter must not switch a
            // nested region away from the axis that started the gesture.
            ongoing.filter(&mut delta, event.touch_phase);
        }
        match self.axis {
            ScrollbarAxis::Vertical if delta.y.abs() >= delta.x.abs() => Some(delta.y),
            ScrollbarAxis::Vertical => None,
            ScrollbarAxis::Horizontal if self.horizontal_only => {
                (delta.x.abs() >= delta.y.abs()).then_some(delta.x)
            }
            ScrollbarAxis::Horizontal => Some(if delta.x != px(0.0) { delta.x } else { delta.y }),
        }
    }
}

/// Frame-local bindings are deliberately not retained in element state. A
/// rerender may replace a driver, input predicate or callback under the same id.
struct Bindings {
    driver: Rc<dyn ElasticScrollDriver>,
    on_scroll: Option<WheelListener>,
    predicate: Option<WheelPredicate>,
    on_logical_scroll: Option<LogicalScrollListener>,
    linked: bool,
}

#[derive(Clone)]
struct Surface {
    token: u64,
    hitbox: Hitbox,
    state: ElasticScrollState,
    local: Rc<RefCell<LocalState>>,
    view: gpui::EntityId,
    policy: AxisPolicy,
}

impl Surface {
    fn capture(
        &self,
        event: &ScrollWheelEvent,
        predicate: Option<&WheelPredicate>,
        window: &mut Window,
        cx: &mut App,
    ) {
        // A claim from an earlier packet must never authorize its old delta,
        // including when a live terminal input mode changes between phases.
        self.local.borrow_mut().captured_delta = None;
        if self.state.0.borrow().motion.touching()
            && matches!(
                native::read(event).phase,
                TouchPhase::Ended | TouchPhase::Cancelled
            )
        {
            let mut state = self.state.0.borrow_mut();
            state.motion.release(cx.background_executor().now());
            let redraw = state.motion.animating() || state.applied_offset != 0.0;
            drop(state);
            if redraw {
                cx.notify(self.view);
            }
        }
        // Release precedes hit testing so finger lift outside this surface
        // still releases it. Predicates remain live in both dispatch phases.
        if !self.hitbox.should_handle_scroll(window)
            || predicate.is_some_and(|predicate| !predicate(event, window, cx))
        {
            return;
        }
        let mut local = self.local.borrow_mut();
        if let Some(delta) = self
            .policy
            .delta(event, &mut local.axis, window.line_height())
        {
            local.captured_delta = Some(delta);
            // The private global is allocated once. Interior mutation avoids
            // both a Box allocation and a global-observer effect per claim.
            if cx.try_global::<WheelClaim>().is_none() {
                cx.set_global(WheelClaim::default());
            }
            cx.global::<WheelClaim>().owner.set(Some(self.token));
        }
    }

    fn bubble(
        &self,
        event: &ScrollWheelEvent,
        bindings: &Bindings,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !self.hitbox.should_handle_scroll(window)
            || !cx
                .try_global::<WheelClaim>()
                .is_some_and(|claim| claim.owner.get() == Some(self.token))
        {
            return;
        }
        let Some(delta) = self.local.borrow().captured_delta else {
            return;
        };
        if bindings
            .predicate
            .as_ref()
            .is_some_and(|predicate| !predicate(event, window, cx))
        {
            return;
        }
        let mut state = self.state.0.borrow_mut();
        if !event.delta.precise() {
            let redraw = state.motion.offset() != 0.0;
            state.motion.reset();
            drop(state);
            if redraw {
                cx.notify(self.view);
            }
            // Ordinary wheels keep the child's native scrolling policy.
            return;
        }
        if !f32::from(delta).is_finite()
            || (state.motion.offset() == 0.0 && !bindings.driver.should_handle(delta))
        {
            return;
        }
        let now = cx.background_executor().now();
        let before = state.motion.offset();
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
        let delta = f32::from(delta);
        let remaining = state.motion.take_reversal(delta, now);
        let consumed = bindings.driver.scroll_by(px(remaining));
        if animate {
            state.motion.record_delta(delta, now);
            state
                .motion
                .overscroll(remaining - f32::from(consumed), now);
        }
        let metrics = bindings.driver.metrics();
        self.local.borrow_mut().expected_metrics = Some(metrics);
        if bindings.linked {
            state.expected_driver = Some((bindings.driver.clone(), metrics));
        }
        // Callbacks can move a linked follower even when this model is clamped.
        // Keep their repaint contract, and invoke them outside the state borrow.
        let redraw = consumed != px(0.0)
            || state.motion.offset() != before
            || (state.motion.animating() && !state.frame_scheduled)
            || bindings.on_scroll.is_some()
            || bindings.on_logical_scroll.is_some();
        drop(state);
        if let Some(on_scroll) = &bindings.on_scroll {
            on_scroll(event, window, cx);
        }
        if let Some(on_logical_scroll) = &bindings.on_logical_scroll {
            on_logical_scroll(px(remaining), event, window, cx);
        }
        if redraw {
            cx.notify(self.view);
        }
        // A clamped no-op still belongs to this surface, not its parent.
        cx.stop_propagation();
    }
}

fn mouse_down(state: &ElasticScrollState, view: gpui::EntityId, cx: &mut App) {
    let mut state = state.0.borrow_mut();
    if state.motion.offset() != 0.0 {
        state.motion.reset();
        drop(state);
        cx.notify(view);
    }
}

pub(super) fn paint(
    element: &mut ElasticScroll,
    bounds: Bounds<Pixels>,
    child: &mut AnyElement,
    prepaint: &ElasticScrollPrepaintState,
    window: &mut Window,
    cx: &mut App,
) {
    let surface = Surface {
        token: prepaint.local.borrow().claim_token,
        hitbox: prepaint.hitbox.clone(),
        state: prepaint.state.clone(),
        local: prepaint.local.clone(),
        view: window.current_view(),
        policy: AxisPolicy {
            axis: element.axis,
            restrict: element.vertical_only || element.horizontal_only,
            horizontal_only: element.horizontal_only,
        },
    };
    window.on_mouse_event({
        let surface = surface.clone();
        let predicate = element.wheel_predicate.clone();
        move |event: &ScrollWheelEvent, phase, window, cx| {
            if phase == DispatchPhase::Capture {
                surface.capture(event, predicate.as_ref(), window, cx);
            }
        }
    });
    // Keep this listener even at rest: scrolling followed by a click before
    // the next draw must cancel newly created displacement too.
    window.on_mouse_event({
        let state = prepaint.state.clone();
        let view = surface.view;
        move |_: &MouseDownEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture {
                mouse_down(&state, view, cx);
            }
        }
    });
    window.with_content_mask(
        Some(ContentMask {
            bounds,
            ..Default::default()
        }),
        |window| {
            child.paint(window, cx);
        },
    );
    let bindings = Bindings {
        driver: element.driver.clone(),
        on_scroll: element.on_scroll.take(),
        predicate: element.wheel_predicate.take(),
        on_logical_scroll: element.on_logical_scroll.take(),
        linked: element.linked_motion,
    };
    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
        if phase == DispatchPhase::Bubble {
            surface.bubble(event, &bindings, window, cx);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::ScrollDelta;

    #[test]
    fn axis_admission_preserves_nonfinite_and_diagonal_packet_rules() {
        let horizontal = AxisPolicy {
            axis: ScrollbarAxis::Horizontal,
            restrict: true,
            horizontal_only: true,
        };
        let vertical = AxisPolicy {
            axis: ScrollbarAxis::Vertical,
            ..horizontal
        };
        for phase in [TouchPhase::Ended, TouchPhase::Cancelled] {
            let event = |x, y| ScrollWheelEvent {
                position: point(px(0.0), px(0.0)),
                delta: ScrollDelta::Pixels(point(px(x), px(y))),
                touch_phase: phase,
                modifiers: Default::default(),
            };
            for nonfinite in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
                assert!(
                    horizontal
                        .delta(
                            &event(10.0, nonfinite),
                            &mut OngoingScroll::default(),
                            px(16.0)
                        )
                        .is_none()
                );
                assert!(
                    vertical
                        .delta(
                            &event(nonfinite, 10.0),
                            &mut OngoingScroll::default(),
                            px(16.0)
                        )
                        .is_none()
                );
            }
            assert_eq!(
                horizontal.delta(&event(-10.0, 1.0), &mut OngoingScroll::default(), px(16.0)),
                Some(px(-10.0))
            );
            assert_eq!(
                vertical.delta(&event(1.0, -10.0), &mut OngoingScroll::default(), px(16.0)),
                Some(px(-10.0))
            );
        }
    }
}
