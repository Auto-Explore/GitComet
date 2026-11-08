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
