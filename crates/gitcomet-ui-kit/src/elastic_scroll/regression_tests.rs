//! Behavioral guards for rerendered bindings and the presented pointer surface.
use super::{ElasticScroll, ElasticScrollDriver, ElasticScrollState};
use crate::test_support::refresh_and_draw;
use gpui::{
    Context, IntoElement, Modifiers, MouseButton, Pixels, Point, Render, ScrollDelta, ScrollHandle,
    ScrollWheelEvent, TestAppContext, TouchPhase, VisualTestContext, Window, div, point,
    prelude::*, px,
};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

#[derive(Debug, PartialEq)]
enum Delivery {
    Driver(u32, f32),
    Wheel(u32),
    Logical(u32, f32),
}

type Deliveries = Rc<RefCell<Vec<Delivery>>>;

struct DriverBinding {
    revision: u32,
    scroll: ScrollHandle,
    deliveries: Deliveries,
    drops: Rc<Cell<usize>>,
}

impl Drop for DriverBinding {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct CountingDriver(Rc<DriverBinding>);

impl ElasticScrollDriver for CountingDriver {
    fn metrics(&self) -> (f64, f64) {
        (f64::from(f32::from(self.0.scroll.offset().y)), 580.0)
    }

    fn scroll_by(&self, delta: Pixels) -> Pixels {
        self.0
            .deliveries
            .borrow_mut()
            .push(Delivery::Driver(self.0.revision, f32::from(delta)));
        let old = self.0.scroll.offset();
        let next = (old.y + delta).clamp(px(-580.0), px(0.0));
        self.0.scroll.set_offset(point(old.x, next));
        next - old.y
    }
}

struct CallbackBinding {
    revision: u32,
    deliveries: Deliveries,
    drops: Rc<Cell<usize>>,
}

impl Drop for CallbackBinding {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

struct BindingFixture {
    driver: Rc<DriverBinding>,
    callbacks: Rc<CallbackBinding>,
    motion: ElasticScrollState,
    reset_in_callbacks: bool,
}

impl BindingFixture {
    fn new(position: Pixels, reset_in_callbacks: bool) -> Self {
        let deliveries = Deliveries::default();
        let scroll = ScrollHandle::new();
        scroll.set_offset(point(px(0.0), position));
        Self {
            driver: Rc::new(DriverBinding {
                revision: 1,
                scroll,
                deliveries: deliveries.clone(),
                drops: Default::default(),
            }),
            callbacks: Rc::new(CallbackBinding {
                revision: 1,
                deliveries,
                drops: Default::default(),
            }),
            motion: Default::default(),
            reset_in_callbacks,
        }
    }

    fn replace_bindings(&mut self) {
        let scroll = ScrollHandle::new();
        scroll.set_offset(self.driver.scroll.offset());
        self.driver = Rc::new(DriverBinding {
            revision: 2,
            scroll,
            deliveries: self.driver.deliveries.clone(),
            drops: Default::default(),
        });
        self.callbacks = Rc::new(CallbackBinding {
            revision: 2,
            deliveries: self.callbacks.deliveries.clone(),
            drops: Default::default(),
        });
    }
}

impl Render for BindingFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let wheel = self.callbacks.clone();
        let logical = self.callbacks.clone();
        let wheel_motion = self.motion.clone();
        let logical_motion = self.motion.clone();
        let reset = self.reset_in_callbacks;
        ElasticScroll::vertical(
            "rerendered_elastic_binding",
            CountingDriver(self.driver.clone()),
            div()
                .id("rerendered_scroll_surface")
                .debug_selector(|| "binding_viewport".into())
                .w(px(200.0))
                .h(px(120.0))
                .overflow_y_scroll()
                .track_scroll(&self.driver.scroll)
                .child(div().h(px(700.0))),
        )
        .motion_state(self.motion.clone())
        .on_scroll(move |_, _, _| {
            // Real callers may inspect or reset their motion in callbacks. The
            // router must release its mutable state borrow before invoking them.
            let _ = wheel_motion.displacement();
            if reset {
                wheel_motion.reset();
            }
            wheel
                .deliveries
                .borrow_mut()
                .push(Delivery::Wheel(wheel.revision));
        })
        .on_logical_scroll(move |delta, _, _, _| {
            let _ = logical_motion.displacement();
            if reset {
                logical_motion.reset();
            }
            logical
                .deliveries
                .borrow_mut()
                .push(Delivery::Logical(logical.revision, f32::from(delta)));
        })
    }
}

fn dispatch_wheel(cx: &mut VisualTestContext, position: Point<Pixels>, delta: ScrollDelta) {
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta,
                touch_phase: TouchPhase::Started,
                modifiers: Default::default(),
            }),
            app,
        );
    });
    refresh_and_draw(cx);
}

#[gpui::test]
fn rerender_replaces_driver_and_callbacks_without_retaining_the_old_bindings(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            BindingFixture::new(px(-40.0), false)
        });
        refresh_and_draw(cx);
        let position = cx.debug_bounds("binding_viewport").unwrap().center();
        dispatch_wheel(cx, position, ScrollDelta::Pixels(point(px(0.0), px(-10.0))));
        let (old_driver, old_callbacks, old_scroll, driver_drops, callback_drops, deliveries) =
            view.read_with(cx, |view, _| {
                (
                    Rc::downgrade(&view.driver),
                    Rc::downgrade(&view.callbacks),
                    view.driver.scroll.clone(),
                    view.driver.drops.clone(),
                    view.callbacks.drops.clone(),
                    view.driver.deliveries.clone(),
                )
            });
        assert_eq!(old_scroll.offset().y, px(-50.0));
        assert_eq!(
            *deliveries.borrow(),
            vec![
                Delivery::Driver(1, -10.0),
                Delivery::Wheel(1),
                Delivery::Logical(1, -10.0)
            ]
        );
        let old_metrics =
            view.read_with(cx, |view, _| CountingDriver(view.driver.clone()).metrics());
        view.update(cx, |view, cx| {
            view.replace_bindings();
            cx.notify();
        });
        // GPUI may retain the previous painted frame until the next draw. A
        // stable redraw must not keep obsolete driver or callback captures alive.
        for _ in 0..3 {
            refresh_and_draw(cx);
        }
        assert_eq!(
            view.read_with(cx, |view, _| CountingDriver(view.driver.clone()).metrics()),
            old_metrics,
            "the replacement changes identity, not document geometry or position"
        );
        assert!(
            old_driver.upgrade().is_none(),
            "old driver binding retained"
        );
        assert!(
            old_callbacks.upgrade().is_none(),
            "old callback binding retained"
        );
        assert_eq!(driver_drops.get(), 1);
        assert_eq!(callback_drops.get(), 1);
        deliveries.borrow_mut().clear();
        dispatch_wheel(cx, position, ScrollDelta::Pixels(point(px(0.0), px(-7.5))));
        assert_eq!(old_scroll.offset().y, px(-50.0));
        assert_eq!(
            view.read_with(cx, |view, _| view.driver.scroll.offset().y),
            px(-57.5)
        );
        assert_eq!(
            *deliveries.borrow(),
            vec![
                Delivery::Driver(2, -7.5),
                Delivery::Wheel(2),
                Delivery::Logical(2, -7.5)
            ]
        );
    });
}

#[gpui::test]
fn wheel_callbacks_can_read_and_reset_motion_after_the_driver_consumes_the_packet(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            BindingFixture::new(px(0.0), true)
        });
        refresh_and_draw(cx);
        let position = cx.debug_bounds("binding_viewport").unwrap().center();
        dispatch_wheel(cx, position, ScrollDelta::Pixels(point(px(0.0), px(100.0))));
        view.read_with(cx, |view, _| {
            assert_eq!(view.driver.scroll.offset().y, px(0.0));
            assert_eq!(view.motion.displacement(), px(0.0));
            assert_eq!(
                *view.driver.deliveries.borrow(),
                vec![
                    Delivery::Driver(1, 100.0),
                    Delivery::Wheel(1),
                    Delivery::Logical(1, 100.0)
                ]
            );
        });
    });
}

struct PointerFixture {
    scroll: ScrollHandle,
    body_down: Rc<Cell<usize>>,
    clipped_down: Rc<Cell<usize>>,
    header_down: Rc<Cell<usize>>,
}

impl Render for PointerFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let body = self.body_down.clone();
        let clipped = self.clipped_down.clone();
        let header = self.header_down.clone();
        div()
            .flex()
            .flex_col()
            .w(px(200.0))
            .h(px(230.0))
            .child(
                div()
                    .id("pointer_fixed_header")
                    .debug_selector(|| "pointer_fixed_header".into())
                    .flex_shrink_0()
                    .h(px(30.0))
                    .on_mouse_down(MouseButton::Left, move |_, _, _| {
                        header.set(header.get() + 1)
                    }),
            )
            .child(
                div()
                    .debug_selector(|| "pointer_fixed_viewport".into())
                    .flex_shrink_0()
                    .w(px(200.0))
                    .h(px(120.0))
                    .child(ElasticScroll::vertical(
                        "pointer_elastic",
                        self.scroll.clone(),
                        div()
                            .id("pointer_scroller")
                            .w(px(200.0))
                            .h(px(120.0))
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .h(px(700.0))
                                    .child(
                                        div()
                                            .id("pointer_body_target")
                                            .debug_selector(|| "pointer_body_target".into())
                                            .flex_shrink_0()
                                            .h(px(24.0))
                                            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                                body.set(body.get() + 1)
                                            }),
                                    )
                                    .child(div().flex_shrink_0().h(px(76.0)))
                                    .child(
                                        div()
                                            .id("pointer_clipped_target")
                                            .debug_selector(|| "pointer_clipped_target".into())
                                            .flex_shrink_0()
                                            .h(px(24.0))
                                            .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                                clipped.set(clipped.get() + 1)
                                            }),
                                    )
                                    .child(div().flex_shrink_0().h(px(576.0))),
                            ),
                    )),
            )
    }
}

fn stretch_pointer_surface(cx: &mut VisualTestContext) {
    let position = cx.debug_bounds("pointer_fixed_viewport").unwrap().center();
    dispatch_wheel(cx, position, ScrollDelta::Pixels(point(px(0.0), px(100.0))));
}

#[gpui::test]
fn presented_pointer_targets_move_with_stretch_but_stay_inside_the_fixed_clip(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            PointerFixture {
                scroll: ScrollHandle::new(),
                body_down: Default::default(),
                clipped_down: Default::default(),
                header_down: Default::default(),
            }
        });
        refresh_and_draw(cx);
        let viewport = cx.debug_bounds("pointer_fixed_viewport").unwrap();
        let header = cx.debug_bounds("pointer_fixed_header").unwrap();
        let original_body = cx.debug_bounds("pointer_body_target").unwrap();
        let (body, clipped, header_count) = view.read_with(cx, |view, _| {
            (
                view.body_down.clone(),
                view.clipped_down.clone(),
                view.header_down.clone(),
            )
        });

        stretch_pointer_surface(cx);
        let translated = cx.debug_bounds("pointer_body_target").unwrap();
        assert!(translated.top() > original_body.top() + px(20.0));
        // Mouse-down uses the currently presented hitboxes. It also cancels
        // the spring for the following frame, so restretch before each probe.
        cx.simulate_click(translated.center(), Modifiers::default());
        assert_eq!(
            body.get(),
            1,
            "the translated target receives a real pointer action"
        );

        refresh_and_draw(cx);
        stretch_pointer_surface(cx);
        assert!(original_body.center().y < cx.debug_bounds("pointer_body_target").unwrap().top());
        cx.simulate_click(original_body.center(), Modifiers::default());
        assert_eq!(
            body.get(),
            1,
            "the exposed gap is not the old target hitbox"
        );

        refresh_and_draw(cx);
        stretch_pointer_surface(cx);
        let outside = cx.debug_bounds("pointer_clipped_target").unwrap().center();
        assert!(outside.y > viewport.bottom());
        let translated_native_viewport = view.read_with(cx, |view, _| view.scroll.bounds());
        assert!(outside.y < translated_native_viewport.bottom());
        cx.simulate_click(outside, Modifiers::default());
        assert_eq!(
            clipped.get(),
            0,
            "translated children remain clipped by the fixed viewport"
        );

        refresh_and_draw(cx);
        stretch_pointer_surface(cx);
        assert_eq!(cx.debug_bounds("pointer_fixed_header").unwrap(), header);
        cx.simulate_click(header.center(), Modifiers::default());
        assert_eq!(header_count.get(), 1);
        assert_eq!(body.get(), 1);
        assert_eq!(clipped.get(), 0);
        assert_eq!(
            view.read_with(cx, |view, _| view.scroll.offset().y),
            px(0.0)
        );
    });
}

struct VerticalHelperFixture {
    scroll: ScrollHandle,
}

impl Render for VerticalHelperFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        ElasticScroll::vertical(
            "vertical_helper_elastic",
            self.scroll.clone(),
            div()
                .id("vertical_helper_surface")
                .debug_selector(|| "vertical_helper_surface".into())
                .w(px(200.0))
                .h(px(120.0))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(div().w(px(700.0)).h(px(700.0))),
        )
    }
}

#[gpui::test]
fn vertical_helper_routes_precise_input_once_and_does_not_remap_sideways_native_wheels(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            VerticalHelperFixture {
                scroll: ScrollHandle::new(),
            }
        });
        refresh_and_draw(cx);
        let position = cx.debug_bounds("vertical_helper_surface").unwrap().center();
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        dispatch_wheel(
            cx,
            position,
            ScrollDelta::Pixels(point(px(-2.0), px(-20.0))),
        );
        assert_eq!(scroll.offset(), point(px(0.0), px(-20.0)));
        let line_height = cx.update(|window, _| window.line_height());
        dispatch_wheel(cx, position, ScrollDelta::Lines(point(-2.0, -1.0)));
        assert_eq!(scroll.offset(), point(px(0.0), px(-20.0) - line_height));
        let before = scroll.offset();
        dispatch_wheel(cx, position, ScrollDelta::Lines(point(-1.0, 0.0)));
        assert_eq!(
            scroll.offset(),
            before,
            "horizontal native input is not remapped onto the vertical document"
        );
    });
}
