//! Work budgets exercise real wheel dispatch and prepaint, not row fingerprints.
use super::*;
use crate::test_support::refresh_and_draw;
use gpui::{Context, Render, ScrollDelta, TestAppContext, VisualTestContext, div, prelude::*};
use std::time::Duration;

struct CountingDriver {
    scroll: ScrollHandle,
    reads: Cell<usize>,
    writes: Cell<usize>,
}

impl ElasticScrollDriver for CountingDriver {
    fn metrics(&self) -> (f64, f64) {
        self.reads.set(self.reads.get() + 1);
        ElasticScrollDriver::metrics(&self.scroll)
    }

    fn scroll_by(&self, delta: Pixels) -> Pixels {
        self.writes.set(self.writes.get() + 1);
        ElasticScrollDriver::scroll_by(&self.scroll, delta)
    }
}

struct WorkSurface {
    driver: Rc<CountingDriver>,
    motion: ElasticScrollState,
    callbacks: bool,
    callback_count: Rc<Cell<usize>>,
    nested: bool,
    inner: ScrollHandle,
}

impl Render for WorkSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let surface = ElasticScroll::with_shared_driver(
            "work_elastic",
            self.driver.clone(),
            div()
                .id("work_viewport")
                .w(px(200.0))
                .h(px(120.0))
                .overflow_y_scroll()
                .track_scroll(&self.driver.scroll)
                .child(div().h(px(700.0)).when(self.nested, |body| {
                    body.child(ElasticScroll::vertical(
                        "new_inner_elastic",
                        self.inner.clone(),
                        div()
                            .id("new_inner_viewport")
                            .w(px(150.0))
                            .h(px(80.0))
                            .overflow_y_scroll()
                            .track_scroll(&self.inner)
                            .child(div().h(px(400.0))),
                    ))
                })),
        )
        .vertical_only()
        .motion_state(self.motion.clone());
        if self.callbacks {
            let calls = self.callback_count.clone();
            surface.on_logical_scroll(move |_, _, _, _| calls.set(calls.get() + 1))
        } else {
            surface
        }
    }
}

fn fixture(cx: &mut TestAppContext) -> (gpui::Entity<WorkSurface>, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|window, _| {
        window.activate();
        WorkSurface {
            driver: Rc::new(CountingDriver {
                scroll: ScrollHandle::new(),
                reads: Cell::new(0),
                writes: Cell::new(0),
            }),
            motion: Default::default(),
            callbacks: false,
            callback_count: Default::default(),
            nested: false,
            inner: ScrollHandle::new(),
        }
    });
    refresh_and_draw(cx);
    (view, cx)
}

fn dispatch(
    cx: &mut VisualTestContext,
    position: gpui::Point<Pixels>,
    delta: f32,
    phase: TouchPhase,
) {
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(0.0), px(delta))),
                touch_phase: phase,
                modifiers: Default::default(),
            }),
            app,
        );
    });
}

#[gpui::test]
fn idle_prepaint_reads_metrics_once_and_never_queues_frames(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let driver = view.read_with(cx, |view, _| view.driver.clone());
        driver.reads.set(0);
        for _ in 0..32 {
            refresh_and_draw(cx);
            cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));
        }
        assert_eq!(
            driver.reads.get(),
            32,
            "no post-child geometry read without stretch"
        );
        assert_eq!(driver.writes.get(), 0);
    });
}

#[gpui::test]
fn clamped_packets_keep_one_claim_without_global_effects_or_redundant_notifies(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let driver = view.read_with(cx, |view, _| view.driver.clone());
        let position = driver.scroll.bounds().center();
        let globals = Rc::new(Cell::new(0));
        let notifies = Rc::new(Cell::new(0));
        cx.update(|_, app| {
            app.set_reduce_motion(true);
            let count = globals.clone();
            app.observe_global::<WheelClaim>(move |_| count.set(count.get() + 1))
                .detach();
            let count = notifies.clone();
            app.observe(&view, move |_, _| count.set(count.get() + 1))
                .detach();
        });
        for _ in 0..100 {
            dispatch(cx, position, 10.0, TouchPhase::Moved);
        }
        assert_eq!(
            driver.writes.get(),
            100,
            "clamping must still claim every packet"
        );
        assert_eq!(driver.scroll.offset().y, px(0.0));
        assert_eq!(
            globals.get(),
            0,
            "wheel claims do not replace or notify the global"
        );
        assert_eq!(
            notifies.get(),
            0,
            "unchanged clamped content needs no redraw"
        );
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));

        view.update(cx, |view, cx| {
            view.callbacks = true;
            cx.notify();
        });
        refresh_and_draw(cx);
        notifies.set(0);
        for _ in 0..5 {
            dispatch(cx, position, 10.0, TouchPhase::Moved);
        }
        assert_eq!(view.read_with(cx, |view, _| view.callback_count.get()), 5);
        assert_eq!(
            notifies.get(),
            5,
            "callbacks may move linked followers at an edge"
        );
    });
}

#[gpui::test]
fn zero_delta_finger_lift_in_the_interior_needs_no_extra_redraw(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let driver = view.read_with(cx, |view, _| view.driver.clone());
        let position = driver.scroll.bounds().center();
        dispatch(cx, position, -20.0, TouchPhase::Started);
        refresh_and_draw(cx);
        let notifies = Rc::new(Cell::new(0));
        cx.update(|_, app| {
            let count = notifies.clone();
            app.observe(&view, move |_, _| count.set(count.get() + 1))
                .detach();
        });
        dispatch(cx, position, 0.0, TouchPhase::Ended);
        assert_eq!(notifies.get(), 0);
        assert_eq!(driver.scroll.offset().y, px(-20.0));
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));
    });
}

#[gpui::test]
fn held_and_reset_motion_has_no_recurring_animation_frames(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let position = view.read_with(cx, |view, _| view.driver.scroll.bounds().center());
        dispatch(cx, position, 100.0, TouchPhase::Started);
        refresh_and_draw(cx);
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));
        dispatch(cx, position, 0.0, TouchPhase::Ended);
        refresh_and_draw(cx);
        view.update(cx, |view, cx| {
            view.motion.reset();
            cx.notify();
        });
        refresh_and_draw(cx);
        cx.background_executor.advance_clock(Duration::from_secs(1));
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 1));
        refresh_and_draw(cx);
        assert_eq!(
            view.read_with(cx, |view, _| view.motion.displacement()),
            px(0.0)
        );
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));
    });
}

#[gpui::test]
fn a_click_cancels_stretch_created_after_the_last_paint(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let position = view.read_with(cx, |view, _| view.driver.scroll.bounds().center());
        dispatch(cx, position, 100.0, TouchPhase::Started);
        assert!(view.read_with(cx, |view, _| view.motion.0.borrow().motion.offset()) > 0.0);
        // No redraw between these inputs: a reset listener installed only when
        // the last painted offset was nonzero would miss this click.
        cx.update(|window, app| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position,
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                app,
            );
        });
        assert_eq!(
            view.read_with(cx, |view, _| view.motion.0.borrow().motion.offset()),
            0.0
        );
        refresh_and_draw(cx);
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));
    });
}

#[gpui::test]
fn clearing_app_globals_keeps_painted_input_safe_and_surface_tokens_unique(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let position = view.read_with(cx, |view, _| view.driver.scroll.bounds().center());
        let earlier_token = WheelClaim::register_surface();
        cx.update(|_, app| app.clear_globals());
        dispatch(cx, position, -10.0, TouchPhase::Started);
        assert_eq!(
            view.read_with(cx, |view, _| view.driver.scroll.offset().y),
            px(-10.0)
        );
        assert!(WheelClaim::register_surface() > earlier_token);
        refresh_and_draw(cx);
        dispatch(cx, position, -20.0, TouchPhase::Moved);
        assert_eq!(
            view.read_with(cx, |view, _| view.driver.scroll.offset().y),
            px(-30.0)
        );
    });
}

#[gpui::test]
fn a_nested_surface_mounted_after_global_reset_has_a_distinct_gesture_owner(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let position = view.read_with(cx, |view, _| view.driver.scroll.bounds().center());
        dispatch(cx, position, -10.0, TouchPhase::Started);
        let outer = cx.update(|_, app| app.global::<WheelClaim>().owner.get().unwrap());
        cx.update(|_, app| app.clear_globals());
        view.update(cx, |view, cx| {
            view.driver.scroll.set_offset(point(px(0.0), px(0.0)));
            view.nested = true;
            cx.notify();
        });
        refresh_and_draw(cx);
        dispatch(cx, position, -20.0, TouchPhase::Started);
        view.read_with(cx, |view, _| {
            assert_eq!(view.driver.scroll.offset().y, px(0.0));
            assert_eq!(
                view.driver.writes.get(),
                1,
                "the outer model did not handle the inner packet"
            );
            assert_eq!(view.inner.offset().y, px(-20.0));
        });
        let inner = cx.update(|_, app| app.global::<WheelClaim>().owner.get().unwrap());
        assert_ne!(
            outer, inner,
            "retained and newly mounted surfaces cannot share a token"
        );
    });
}
