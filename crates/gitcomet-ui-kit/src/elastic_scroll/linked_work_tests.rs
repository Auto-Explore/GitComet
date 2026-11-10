//! Linked surfaces retain their geometry guard without doing post-child work at rest.
use super::{ElasticScroll, ElasticScrollDriver, ElasticScrollState};
use crate::test_support::refresh_and_draw;
use gpui::{
    Context, IntoElement, Pixels, Render, ScrollDelta, ScrollHandle, ScrollWheelEvent,
    TestAppContext, TouchPhase, VisualTestContext, Window, div, point, prelude::*, px,
};
use std::cell::Cell;
use std::rc::Rc;

struct CountingDriver {
    scroll: ScrollHandle,
    reads: Cell<usize>,
    writes: Cell<usize>,
    nonzero_writes: Cell<usize>,
    drops: Rc<Cell<usize>>,
}

impl CountingDriver {
    fn new(scroll: ScrollHandle) -> Self {
        Self {
            scroll,
            reads: Cell::new(0),
            writes: Cell::new(0),
            nonzero_writes: Cell::new(0),
            drops: Default::default(),
        }
    }
}

impl Drop for CountingDriver {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

impl ElasticScrollDriver for CountingDriver {
    fn metrics(&self) -> (f64, f64) {
        self.reads.set(self.reads.get() + 1);
        ElasticScrollDriver::metrics(&self.scroll)
    }

    fn scroll_by(&self, delta: Pixels) -> Pixels {
        self.writes.set(self.writes.get() + 1);
        if delta != px(0.0) {
            self.nonzero_writes.set(self.nonzero_writes.get() + 1);
        }
        ElasticScrollDriver::scroll_by(&self.scroll, delta)
    }
}

struct LinkedFixture {
    master: Rc<CountingDriver>,
    follower: Rc<CountingDriver>,
    motion: ElasticScrollState,
}

impl Render for LinkedFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let surface = |id: &'static str, driver: &Rc<CountingDriver>| {
            ElasticScroll::with_shared_driver(
                id,
                driver.clone(),
                crate::restrict_scroll_to_vertical_axis(
                    div()
                        .id(id)
                        .w(px(200.0))
                        .h(px(120.0))
                        .overflow_y_scroll()
                        .track_scroll(&driver.scroll)
                        .child(div().h(px(700.0))),
                ),
            )
            .vertical_only()
            .shared_motion(self.motion.clone())
        };
        div()
            .flex()
            .child(surface("linked_budget_master", &self.master))
            .child(surface("linked_budget_follower", &self.follower))
    }
}

fn fixture(cx: &mut TestAppContext) -> (gpui::Entity<LinkedFixture>, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|window, _| {
        window.activate();
        LinkedFixture {
            master: Rc::new(CountingDriver::new(ScrollHandle::new())),
            follower: Rc::new(CountingDriver::new(ScrollHandle::new())),
            motion: Default::default(),
        }
    });
    refresh_and_draw(cx);
    (view, cx)
}

fn wheel(cx: &mut VisualTestContext, position: gpui::Point<Pixels>, delta: f32, phase: TouchPhase) {
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
    refresh_and_draw(cx);
}

#[gpui::test]
fn linked_resting_frames_read_only_the_master_guard_and_each_surface_metrics(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let (master, follower, motion) = view.read_with(cx, |view, _| {
            (
                view.master.clone(),
                view.follower.clone(),
                view.motion.clone(),
            )
        });
        let position = master.scroll.bounds().center();
        wheel(cx, position, -20.25, TouchPhase::Started);
        wheel(cx, position, 0.0, TouchPhase::Ended);
        assert_eq!(master.scroll.offset().y, px(-20.25));
        assert_eq!(
            master.nonzero_writes.get(),
            1,
            "one logical packet is applied once"
        );
        assert_eq!(follower.scroll.offset().y, px(0.0));
        assert_eq!(
            follower.writes.get(),
            0,
            "shared motion does not install logical synchronization"
        );
        assert_eq!(motion.displacement(), px(0.0));
        let master_writes = master.writes.get();
        master.reads.set(0);
        follower.reads.set(0);
        for frame in 1..=32 {
            refresh_and_draw(cx);
            // Each sibling checks the retained master's metrics in request_layout;
            // prepaint reads its own model once. No offset means no post-child check.
            assert_eq!(
                master.reads.get(),
                frame * 3,
                "master work in resting frame {frame}"
            );
            assert_eq!(
                follower.reads.get(),
                frame,
                "follower work in resting frame {frame}"
            );
            cx.update(|window, app| {
                assert_eq!(
                    window.simulate_next_frame(app),
                    0,
                    "linked rest must not request another frame"
                );
            });
        }
        assert_eq!(master.reads.get() + follower.reads.get(), 32 * 4);
        assert_eq!(master.writes.get(), master_writes);
        assert_eq!(master.scroll.offset().y, px(-20.25));
        assert_eq!(follower.scroll.offset().y, px(0.0));
    });
}

#[gpui::test]
fn resetting_linked_motion_releases_its_old_master_driver_after_a_same_geometry_rerender(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx);
        let position = view.read_with(cx, |view, _| view.master.scroll.bounds().center());
        wheel(cx, position, -40.25, TouchPhase::Started);
        wheel(cx, position, 0.0, TouchPhase::Ended);
        let (old, drops, scroll, maximum) = view.read_with(cx, |view, _| {
            (
                Rc::downgrade(&view.master),
                view.master.drops.clone(),
                view.master.scroll.clone(),
                view.master.scroll.max_offset().y,
            )
        });
        assert!(old.upgrade().is_some());
        assert_eq!(scroll.offset().y, px(-40.25));
        view.update(cx, |view, cx| {
            view.motion.reset();
            // Retain the native handle, element ids, offset and extent. Only
            // driver identity changes; the previous input master must be released.
            view.master = Rc::new(CountingDriver::new(view.master.scroll.clone()));
            cx.notify();
        });
        // Release both GPUI frame buffers as well as the owner's previous binding.
        for _ in 0..3 {
            refresh_and_draw(cx);
        }
        assert!(
            old.upgrade().is_none(),
            "reset retained the obsolete linked master driver"
        );
        assert_eq!(drops.get(), 1);
        view.read_with(cx, |view, _| {
            assert_eq!(view.master.scroll.offset().y, px(-40.25));
            assert_eq!(view.master.scroll.max_offset().y, maximum);
            assert_eq!(view.follower.scroll.offset().y, px(0.0));
            assert_eq!(view.motion.displacement(), px(0.0));
        });
        wheel(cx, position, -7.5, TouchPhase::Started);
        wheel(cx, position, 0.0, TouchPhase::Ended);
        view.read_with(cx, |view, _| {
            assert_eq!(view.master.scroll.offset().y, px(-47.75));
            assert_eq!(view.master.nonzero_writes.get(), 1);
            assert_eq!(view.follower.writes.get(), 0);
        });
        cx.update(|window, app| assert_eq!(window.simulate_next_frame(app), 0));
    });
}
