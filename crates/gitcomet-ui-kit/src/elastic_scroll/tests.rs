use super::*;
use gpui::{Context, Render, ScrollDelta, TestAppContext, VisualTestContext, div, prelude::*};

struct Fixture {
    scroll: ScrollHandle,
    inner: ScrollHandle,
    nested: bool,
}

impl Render for Fixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let content = div()
            .id("viewport")
            .w(px(200.0))
            .h(px(120.0))
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .debug_selector(|| "elastic_content".into())
                    .h(px(700.0))
                    .when(self.nested, |d| {
                        d.child(
                            ElasticScroll::new(
                                "inner_elastic",
                                self.inner.clone(),
                                div()
                                    .id("inner_viewport")
                                    .w(px(150.0))
                                    .h(px(80.0))
                                    .overflow_y_scroll()
                                    .track_scroll(&self.inner)
                                    .child(div().h(px(400.0))),
                            )
                            .vertical_only(),
                        )
                    }),
            );
        div()
            .flex()
            .flex_col()
            .child(div().debug_selector(|| "fixed_header".into()).h(px(30.0)))
            .child(
                div()
                    .debug_selector(|| "elastic_viewport".into())
                    .w(px(200.0))
                    .h(px(120.0))
                    .child(
                        ElasticScroll::new("outer_elastic", self.scroll.clone(), content)
                            .vertical_only(),
                    ),
            )
    }
}

fn fixture(
    cx: &mut TestAppContext,
    nested: bool,
) -> (gpui::Entity<Fixture>, &mut VisualTestContext) {
    cx.add_window_view(|window, _| {
        window.activate();
        Fixture {
            scroll: ScrollHandle::new(),
            inner: ScrollHandle::new(),
            nested,
        }
    })
}

fn wheel(cx: &mut VisualTestContext, y: f32, phase: TouchPhase, precise: bool) {
    wheel_at(cx, y, phase, precise, point(px(40.0), px(30.0)));
}

fn wheel_at(
    cx: &mut VisualTestContext,
    y: f32,
    phase: TouchPhase,
    precise: bool,
    position: gpui::Point<Pixels>,
) {
    let bounds = cx.debug_bounds("elastic_viewport").unwrap();
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                position: bounds.origin + position,
                delta: if precise {
                    ScrollDelta::Pixels(point(px(0.0), px(y)))
                } else {
                    ScrollDelta::Lines(point(0.0, y))
                },
                touch_phase: phase,
                modifiers: Default::default(),
            }),
            app,
        );
    });
    crate::test_support::refresh_and_draw(cx);
}

#[gpui::test]
fn an_outer_stretch_releases_when_the_inner_area_owns_finger_lift(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (_, cx) = fixture(cx, true);
        crate::test_support::refresh_and_draw(cx);
        let original = cx.debug_bounds("elastic_content").unwrap().top();
        wheel_at(
            cx,
            100.0,
            TouchPhase::Started,
            true,
            point(px(180.0), px(30.0)),
        );
        assert!(cx.debug_bounds("elastic_content").unwrap().top() > original);
        wheel(cx, 0.0, TouchPhase::Ended, true);
        cx.update(|window, app| {
            app.background_executor()
                .advance_clock(std::time::Duration::from_secs(1));
            window.simulate_next_frame(app);
        });
        crate::test_support::refresh_and_draw(cx);
        assert_eq!(cx.debug_bounds("elastic_content").unwrap().top(), original);
    });
}

struct VirtualFixture {
    left: UniformListScrollHandle,
    right: UniformListScrollHandle,
    motion: ElasticScrollState,
    shared: bool,
    most_rows_built: usize,
}

impl Render for VirtualFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let left = gpui::uniform_list(
            "virtual_left",
            1_000_000,
            cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                this.most_rows_built = this.most_rows_built.max(range.len());
                range
                    .map(|ix| {
                        div()
                            .h(px(24.0))
                            .debug_selector(move || format!("left_row_{ix}"))
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .w(px(200.0))
        .h(px(120.0))
        .track_scroll(&self.left);
        let right = gpui::uniform_list(
            "virtual_right",
            1_000_000,
            cx.processor(|this, range: std::ops::Range<usize>, _, _| {
                this.most_rows_built = this.most_rows_built.max(range.len());
                range
                    .map(|ix| {
                        div()
                            .h(px(24.0))
                            .debug_selector(move || format!("right_row_{ix}"))
                            .into_any_element()
                    })
                    .collect()
            }),
        )
        .w(px(200.0))
        .h(px(120.0))
        .track_scroll(&self.right);
        let left =
            ElasticScroll::new("virtual_left_elastic", self.left.clone(), left).vertical_only();
        let right =
            ElasticScroll::new("virtual_right_elastic", self.right.clone(), right).vertical_only();
        let left = if self.shared {
            left.shared_motion(self.motion.clone())
        } else {
            left
        };
        div()
            .flex()
            .child(
                div()
                    .debug_selector(|| "elastic_viewport".into())
                    .w(px(200.0))
                    .h(px(120.0))
                    .child(left),
            )
            .when(self.shared, |d| {
                d.child(right.shared_motion(self.motion.clone()))
            })
    }
}

fn virtual_fixture(
    cx: &mut TestAppContext,
    shared: bool,
) -> (gpui::Entity<VirtualFixture>, &mut VisualTestContext) {
    cx.add_window_view(|window, _| {
        window.activate();
        VirtualFixture {
            left: Default::default(),
            right: Default::default(),
            motion: Default::default(),
            shared,
            most_rows_built: 0,
        }
    })
}

#[gpui::test]
fn million_row_uniform_list_preserves_virtualization_and_fractional_scroll(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = virtual_fixture(cx, false);
        crate::test_support::refresh_and_draw(cx);
        wheel(cx, -40.25, TouchPhase::Started, true);
        let (position, rows) = view.read_with(cx, |v, _| {
            (v.left.0.borrow().base_handle.offset().y, v.most_rows_built)
        });
        assert_eq!(position, px(-40.25));
        assert!(rows < 12, "only visible rows are constructed, got {rows}");
    });
}

#[gpui::test]
fn linked_columns_share_stretch_without_putting_it_in_scroll_handles(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = virtual_fixture(cx, true);
        crate::test_support::refresh_and_draw(cx);
        let original = cx.debug_bounds("left_row_0").unwrap().top();
        wheel(cx, 100.0, TouchPhase::Started, true);
        let left = cx.debug_bounds("left_row_0").unwrap().top();
        let right = cx.debug_bounds("right_row_0").unwrap().top();
        assert!(left > original);
        assert_eq!(left, right);
        assert_eq!(
            view.read_with(cx, |v, _| v.left.0.borrow().base_handle.offset().y),
            px(0.0)
        );
        assert_eq!(
            view.read_with(cx, |v, _| v.right.0.borrow().base_handle.offset().y),
            px(0.0)
        );
        wheel(cx, 0.0, TouchPhase::Ended, true);
        cx.update(|window, app| {
            app.background_executor()
                .advance_clock(std::time::Duration::from_millis(30));
            assert_eq!(
                window.simulate_next_frame(app),
                1,
                "linked columns request one pane frame"
            );
        });
        crate::test_support::refresh_and_draw(cx);
        assert_eq!(
            cx.debug_bounds("left_row_0").unwrap().top(),
            cx.debug_bounds("right_row_0").unwrap().top()
        );
        cx.update(|window, app| {
            app.background_executor()
                .advance_clock(std::time::Duration::from_secs(1));
            window.simulate_next_frame(app);
        });
        crate::test_support::refresh_and_draw(cx);
        assert_eq!(cx.debug_bounds("left_row_0").unwrap().top(), original);
        cx.update(|window, app| {
            assert_eq!(
                window.simulate_next_frame(app),
                0,
                "settled elasticity is idle"
            )
        });
    });
}

#[gpui::test]
fn stretch_moves_content_and_hitboxes_but_not_the_logical_offset_or_header(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx, false);
        crate::test_support::refresh_and_draw(cx);
        let header = cx.debug_bounds("fixed_header").unwrap();
        let content = cx.debug_bounds("elastic_content").unwrap();
        wheel(cx, 100.0, TouchPhase::Started, true);
        let stretched = cx.debug_bounds("elastic_content").unwrap();
        assert!(stretched.top() > content.top() + px(5.0));
        assert!(stretched.top() < content.top() + px(72.0));
        assert_eq!(cx.debug_bounds("fixed_header").unwrap(), header);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(0.0));
        wheel(cx, 0.0, TouchPhase::Ended, true);
        cx.update(|window, app| {
            app.background_executor()
                .advance_clock(std::time::Duration::from_secs(1));
            window.simulate_next_frame(app);
        });
        crate::test_support::refresh_and_draw(cx);
        assert_eq!(
            cx.debug_bounds("elastic_content").unwrap().top(),
            content.top()
        );
    });
}

#[gpui::test]
fn precise_deltas_move_once_and_stay_clamped_at_both_edges(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx, false);
        crate::test_support::refresh_and_draw(cx);
        wheel(cx, -40.0, TouchPhase::Started, true);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-40.0));
        wheel(cx, -10_000.0, TouchPhase::Moved, true);
        let (offset, max) =
            view.read_with(cx, |v, _| (v.scroll.offset().y, v.scroll.max_offset().y));
        assert_eq!(offset, -max);
        let viewport = cx.debug_bounds("elastic_viewport").unwrap();
        let content = cx.debug_bounds("elastic_content").unwrap();
        assert!(content.bottom() < viewport.bottom());
        assert!(viewport.bottom() - content.bottom() <= px(72.0));
    });
}

#[gpui::test]
fn nested_scroll_has_one_owner_including_at_the_edge(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = fixture(cx, true);
        crate::test_support::refresh_and_draw(cx);
        wheel(cx, -40.0, TouchPhase::Started, true);
        let offsets = view.read_with(cx, |v, _| (v.scroll.offset().y, v.inner.offset().y));
        assert_eq!(offsets, (px(0.0), px(-40.0)));
        wheel(cx, -10_000.0, TouchPhase::Moved, true);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(0.0));
    });
}

#[gpui::test]
fn reduced_motion_and_phaseless_wheels_keep_content_inside_bounds(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (_, cx) = fixture(cx, false);
        crate::test_support::refresh_and_draw(cx);
        let original = cx.debug_bounds("elastic_content").unwrap();
        wheel(cx, 100.0, TouchPhase::Moved, true);
        assert_eq!(cx.debug_bounds("elastic_content").unwrap(), original);
        cx.update(|_, app| app.set_reduce_motion(true));
        wheel(cx, 100.0, TouchPhase::Started, true);
        assert_eq!(cx.debug_bounds("elastic_content").unwrap(), original);
        cx.update(|_, app| app.set_reduce_motion(false));
        wheel(cx, 100.0, TouchPhase::Started, true);
        assert!(cx.debug_bounds("elastic_content").unwrap().top() > original.top());
        wheel(cx, 1.0, TouchPhase::Moved, false);
        assert_eq!(cx.debug_bounds("elastic_content").unwrap(), original);
    });
}

struct MomentumFixture {
    scroll: ScrollHandle,
    state: ElasticScrollState,
}
impl Render for MomentumFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        ElasticScroll::new(
            "momentum_elastic",
            self.scroll.clone(),
            div()
                .id("momentum_viewport")
                .w(px(200.0))
                .h(px(120.0))
                .overflow_y_scroll()
                .track_scroll(&self.scroll)
                .child(
                    div()
                        .h(px(700.0))
                        .debug_selector(|| "momentum_content".into()),
                ),
        )
        .motion_state(self.state.clone())
        .resample_native_momentum()
        .vertical_only()
    }
}
fn momentum_fixture(
    cx: &mut TestAppContext,
) -> (gpui::Entity<MomentumFixture>, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|window, _| {
        window.activate();
        MomentumFixture {
            scroll: ScrollHandle::new(),
            state: ElasticScrollState::default(),
        }
    });
    crate::test_support::refresh_and_draw(cx);
    view.update(cx, |view, _| {
        view.scroll.set_offset(point(px(0.0), px(-200.0)))
    });
    crate::test_support::refresh_and_draw(cx);
    (view, cx)
}
fn momentum_packet(
    cx: &mut VisualTestContext,
    delta: f32,
    source: ReplaySource,
    phase: TouchPhase,
    ended: bool,
) {
    momentum_packet_with_age(cx, delta, source, phase, ended, std::time::Duration::ZERO);
}
fn momentum_packet_with_age(
    cx: &mut VisualTestContext,
    delta: f32,
    source: ReplaySource,
    phase: TouchPhase,
    ended: bool,
    age: std::time::Duration,
) {
    cx.update(|window, app| {
        let at = app.background_executor().now() - age;
        with_replay_input(
            ReplayInput {
                source,
                phase,
                at,
                momentum_ended: ended,
            },
            || {
                window.dispatch_event(
                    gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                        position: point(px(40.0), px(40.0)),
                        delta: ScrollDelta::Pixels(point(px(0.0), px(delta))),
                        touch_phase: phase,
                        modifiers: Default::default(),
                    }),
                    app,
                );
            },
        );
    });
}
fn advance_momentum(cx: &mut VisualTestContext, millis: u64) -> usize {
    cx.update(|window, app| {
        app.background_executor()
            .advance_clock(std::time::Duration::from_millis(millis));
        window.simulate_next_frame(app)
    })
}

#[gpui::test]
fn native_momentum_is_coalesced_into_real_scroll_before_layout_and_finishes_exactly(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        let original = cx.debug_bounds("momentum_content").unwrap().top();
        momentum_packet(cx, -12.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        cx.update(|_, app| {
            app.background_executor()
                .advance_clock(std::time::Duration::from_millis(8))
        });
        momentum_packet(cx, -12.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-200.0),
            "native packets wait for the coalesced frame"
        );
        assert_eq!(
            advance_momentum(cx, 8),
            1,
            "only one callback for both packets"
        );
        let position = view.read_with(cx, |v, _| v.scroll.offset().y);
        assert!(
            position < px(-200.0) && position > px(-224.0),
            "interpolated logical pose: {position:?}"
        );
        crate::test_support::refresh_and_draw(cx);
        let rendered = cx.debug_bounds("momentum_content").unwrap().top();
        assert!(
            (f32::from(rendered - original) - f32::from(position + px(200.0))).abs() < 1.0,
            "content uses the same logical pose as hit testing/virtualization"
        );
        momentum_packet(cx, 0.0, ReplaySource::Momentum, TouchPhase::Moved, true);
        advance_momentum(cx, 30);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-224.0));
        crate::test_support::refresh_and_draw(cx);
        assert_eq!(
            advance_momentum(cx, 30),
            0,
            "ended momentum leaves no idle frame demand"
        );
    });
}

#[gpui::test]
fn reset_and_new_touch_discard_pending_momentum_without_a_flush_jump(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        momentum_packet(cx, -80.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        view.read_with(cx, |v, _| v.state.reset());
        momentum_packet(cx, -4.0, ReplaySource::Momentum, TouchPhase::Moved, true);
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-204.0),
            "stale epoch neither applies nor cancels the new callback"
        );
        crate::test_support::refresh_and_draw(cx);
        momentum_packet(cx, -80.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        momentum_packet(cx, -3.0, ReplaySource::Touch, TouchPhase::Started, false);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-207.0),
            "finger is immediate and old momentum is discarded"
        );
        advance_momentum(cx, 30);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-207.0));
    });
}

#[gpui::test]
fn hidden_panes_external_scroll_and_clicks_cancel_interior_momentum(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        momentum_packet(cx, -80.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        view.read_with(cx, |v, _| v.state.set_active(false));
        advance_momentum(cx, 30);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-200.0));
        view.read_with(cx, |v, _| v.state.set_active(true));
        momentum_packet(cx, -80.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        view.read_with(cx, |v, _| v.scroll.set_offset(point(px(0.0), px(-250.0))));
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-250.0),
            "an external logical scroll wins"
        );
        crate::test_support::refresh_and_draw(cx);
        momentum_packet(cx, -80.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        cx.update(|window, app| {
            window.dispatch_event(
                gpui::PlatformInput::MouseDown(MouseDownEvent {
                    button: gpui::MouseButton::Left,
                    position: point(px(40.0), px(40.0)),
                    modifiers: Default::default(),
                    click_count: 1,
                    first_mouse: false,
                }),
                app,
            );
        });
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-250.0),
            "zero spring displacement must not hide pending momentum from clicks"
        );
    });
}

#[gpui::test]
fn approaching_a_boundary_flushes_pending_native_travel_into_the_existing_spring(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        momentum_packet(cx, 100.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        momentum_packet(cx, 150.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(0.0));
        crate::test_support::refresh_and_draw(cx);
        assert!(
            view.read_with(cx, |v, _| v.state.displacement()) > px(0.0),
            "the ordinary boundary spring owns leftover native travel"
        );
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(0.0),
            "stale interpolation cannot run after boundary fallback"
        );
    });
}

#[gpui::test]
fn invalid_native_time_flushes_accepted_momentum_without_losing_its_endpoint(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        momentum_packet(cx, -40.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        momentum_packet_with_age(
            cx,
            -2.0,
            ReplaySource::Momentum,
            TouchPhase::Moved,
            false,
            std::time::Duration::from_millis(200),
        );
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-242.0));
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-242.0),
            "fallback invalidates the queued callback"
        );
    });
}

#[gpui::test]
fn native_momentum_reversal_preserves_the_accepted_endpoint_before_normal_routing(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        momentum_packet(cx, -40.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        momentum_packet(cx, 5.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-235.0));
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-235.0),
            "reversal fallback invalidates its old frame generation"
        );
    });
}

#[gpui::test]
fn cancelled_momentum_and_wheels_outside_the_pane_stop_its_pending_travel(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        for (source, phase) in [
            (ReplaySource::Momentum, TouchPhase::Cancelled),
            (ReplaySource::Wheel, TouchPhase::Moved),
        ] {
            momentum_packet(cx, -40.0, ReplaySource::Momentum, TouchPhase::Moved, false);
            cx.update(|window, app| {
                let at = app.background_executor().now();
                with_replay_input(
                    ReplayInput {
                        source,
                        phase,
                        at,
                        momentum_ended: false,
                    },
                    || {
                        window.dispatch_event(
                            gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                                position: point(px(400.0), px(400.0)),
                                delta: ScrollDelta::Pixels(point(px(0.0), px(0.0))),
                                touch_phase: phase,
                                modifiers: Default::default(),
                            }),
                            app,
                        );
                    },
                );
            });
            advance_momentum(cx, 30);
            assert_eq!(
                view.read_with(cx, |v, _| v.scroll.offset().y),
                px(-200.0),
                "{source:?} outside the hitbox cancels prior pane momentum"
            );
        }
    });
}

#[derive(Clone)]
struct MeasuredMomentumDriver {
    position: Rc<std::cell::Cell<f64>>,
    maximum: Rc<std::cell::Cell<f64>>,
}
impl ElasticScrollDriver for MeasuredMomentumDriver {
    fn metrics(&self) -> (f64, f64) {
        (self.position.get(), self.maximum.get())
    }
    fn scroll_by(&self, delta: Pixels) -> Pixels {
        let old = self.position.get();
        let next = (old + f64::from(f32::from(delta))).clamp(-self.maximum.get(), 0.0);
        self.position.set(next);
        px((next - old) as f32)
    }
}
struct MeasuredMomentumFixture {
    driver: MeasuredMomentumDriver,
    measured_maximum: Rc<std::cell::Cell<f64>>,
    state: ElasticScrollState,
}
impl Render for MeasuredMomentumFixture {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let maximum = self.driver.maximum.clone();
        let measured = self.measured_maximum.clone();
        ElasticScroll::new(
            "measured_momentum",
            self.driver.clone(),
            div().w(px(200.0)).h(px(120.0)).child(
                gpui::canvas(move |_, _, _| maximum.set(measured.get()), |_, _, _, _| {})
                    .size_full(),
            ),
        )
        .motion_state(self.state.clone())
        .resample_native_momentum()
        .vertical_only()
    }
}
#[gpui::test]
fn newly_measured_extent_is_adopted_on_input_but_later_geometry_still_cancels_momentum(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            MeasuredMomentumFixture {
                driver: MeasuredMomentumDriver {
                    position: Rc::new(std::cell::Cell::new(-200.0)),
                    maximum: Rc::new(std::cell::Cell::new(0.0)),
                },
                measured_maximum: Rc::new(std::cell::Cell::new(580.0)),
                state: ElasticScrollState::default(),
            }
        });
        crate::test_support::refresh_and_draw(cx);
        view.read_with(cx, |v, _| v.measured_maximum.set(700.0));
        crate::test_support::refresh_and_draw(cx);
        momentum_packet(cx, -40.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        assert!(view.read_with(cx, |v, _| v.state.pending_native_momentum()));
        crate::test_support::refresh_and_draw(cx);
        assert!(
            view.read_with(cx, |v, _| v.state.pending_native_momentum()),
            "input adopted the extent measured after the previous wrapper prepaint"
        );
        advance_momentum(cx, 30);
        assert_eq!(view.read_with(cx, |v, _| v.driver.position.get()), -240.0);
        view.read_with(cx, |v, _| v.state.reset());
        momentum_packet(cx, -40.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        view.read_with(cx, |v, _| v.measured_maximum.set(800.0));
        crate::test_support::refresh_and_draw(cx);
        advance_momentum(cx, 30);
        assert_eq!(
            view.read_with(cx, |v, _| v.driver.position.get()),
            -240.0,
            "geometry changed after the input, so queued movement is cancelled"
        );
        assert!(!view.read_with(cx, |v, _| v.state.pending_native_momentum()));
    });
}

#[gpui::test]
fn native_momentum_in_a_background_window_uses_the_ordinary_logical_route(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = momentum_fixture(cx);
        cx.deactivate_window();
        cx.update(|window, _| assert!(!window.is_window_active()));
        momentum_packet(cx, -40.0, ReplaySource::Momentum, TouchPhase::Moved, false);
        assert_eq!(
            view.read_with(cx, |v, _| v.scroll.offset().y),
            px(-240.0),
            "background-window native scrolling must not be queued then discarded"
        );
        assert!(!view.read_with(cx, |v, _| v.state.pending_native_momentum()));
        advance_momentum(cx, 30);
        assert_eq!(view.read_with(cx, |v, _| v.scroll.offset().y), px(-240.0));
    });
}
