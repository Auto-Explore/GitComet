use super::*;
use crate::components::{
    PickerPrompt, PickerPromptGeometry, PickerPromptItem, PickerPromptLayout, TabBar, TabBarScroll,
    picker_prompt_layout,
};
use crate::test_support::refresh_and_draw;
use crate::theme::AppTheme;
use gpui::{
    Context, Entity, Render, ScrollDelta, TestAppContext, VisualTestContext, div, prelude::*,
};
use std::rc::Rc;
use std::time::Duration;

fn precise_wheel(
    cx: &mut VisualTestContext,
    position: gpui::Point<Pixels>,
    x: f32,
    y: f32,
    phase: TouchPhase,
) {
    cx.update(|window, app| {
        window.dispatch_event(
            gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                position,
                delta: ScrollDelta::Pixels(point(px(x), px(y))),
                touch_phase: phase,
                modifiers: Default::default(),
            }),
            app,
        );
    });
    refresh_and_draw(cx);
}

fn settle(cx: &mut VisualTestContext, position: gpui::Point<Pixels>) {
    precise_wheel(cx, position, 0.0, 0.0, TouchPhase::Ended);
    cx.background_executor.advance_clock(Duration::from_secs(1));
    cx.update(|window, app| window.simulate_next_frame(app));
    refresh_and_draw(cx);
}

struct HorizontalSurface {
    scroll: ScrollHandle,
    gate: Rc<std::cell::Cell<bool>>,
    logical_delta: Rc<std::cell::Cell<f32>>,
}

impl Render for HorizontalSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        ElasticScroll::horizontal(
            "horizontal_surface_elastic",
            self.scroll.clone(),
            div()
                .id("horizontal_surface")
                .flex()
                .w(px(200.0))
                .h(px(80.0))
                .overflow_x_scroll()
                .track_scroll(&self.scroll)
                .child(
                    div()
                        .debug_selector(|| "horizontal_surface_content".into())
                        .flex_shrink_0()
                        .w(px(700.0))
                        .h(px(80.0)),
                ),
        )
        .when_wheel({
            let gate = self.gate.clone();
            move |_, _, _| gate.get()
        })
        .on_logical_scroll({
            let logical_delta = self.logical_delta.clone();
            move |delta, _, _, _| logical_delta.set(f32::from(delta))
        })
    }
}

#[gpui::test]
fn horizontal_surface_bounces_at_both_edges_without_changing_logical_extent(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            HorizontalSurface {
                scroll: ScrollHandle::new(),
                gate: Rc::new(std::cell::Cell::new(true)),
                logical_delta: Default::default(),
            }
        });
        refresh_and_draw(cx);
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        let viewport = scroll.bounds();
        let position = viewport.center();
        let first = cx.debug_bounds("horizontal_surface_content").unwrap();
        let max = scroll.max_offset().x;
        assert_eq!(max, px(500.0));

        precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
        assert!(
            cx.debug_bounds("horizontal_surface_content")
                .unwrap()
                .left()
                > first.left() + px(20.0)
        );
        assert_eq!(scroll.offset(), point(px(0.0), px(0.0)));
        assert_eq!(scroll.max_offset().x, max);
        settle(cx, position);
        assert_eq!(
            cx.debug_bounds("horizontal_surface_content").unwrap(),
            first
        );

        precise_wheel(cx, position, -40.25, 0.0, TouchPhase::Started);
        assert_eq!(scroll.offset().x, px(-40.25));
        settle(cx, position);
        scroll.set_offset(point(-max, px(0.0)));
        refresh_and_draw(cx);
        let last = cx.debug_bounds("horizontal_surface_content").unwrap();
        precise_wheel(cx, position, -100.0, 0.0, TouchPhase::Started);
        assert!(
            cx.debug_bounds("horizontal_surface_content")
                .unwrap()
                .left()
                < last.left() - px(20.0)
        );
        assert_eq!(scroll.offset().x, -max);
        assert_eq!(scroll.max_offset().x, max);
        settle(cx, position);
        assert_eq!(cx.debug_bounds("horizontal_surface_content").unwrap(), last);
        assert_eq!(scroll.offset().x, -max);
    });
}

#[gpui::test]
fn live_wheel_gate_preserves_native_input_and_logical_forwarding_excludes_reversal(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            HorizontalSurface {
                scroll: ScrollHandle::new(),
                gate: Rc::new(std::cell::Cell::new(false)),
                logical_delta: Rc::new(std::cell::Cell::new(f32::NAN)),
            }
        });
        refresh_and_draw(cx);
        let (scroll, gate, logical) = view.read_with(cx, |view, _| {
            (
                view.scroll.clone(),
                view.gate.clone(),
                view.logical_delta.clone(),
            )
        });
        let position = scroll.bounds().center();
        precise_wheel(cx, position, -10.0, 0.0, TouchPhase::Started);
        assert_eq!(
            scroll.offset().x,
            px(-10.0),
            "the native child still scrolls"
        );
        assert!(
            logical.get().is_nan(),
            "the rejected packet bypasses elastic callbacks"
        );

        scroll.set_offset(point(px(0.0), px(0.0)));
        refresh_and_draw(cx);
        gate.set(true);
        precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
        assert_eq!(logical.get(), 100.0);
        assert_eq!(scroll.offset().x, px(0.0));
        precise_wheel(cx, position, -30.0, 0.0, TouchPhase::Moved);
        assert_eq!(
            logical.get(),
            0.0,
            "reversal first takes up stretched finger travel"
        );
        assert_eq!(scroll.offset().x, px(0.0));
        precise_wheel(cx, position, -100.0, 0.0, TouchPhase::Moved);
        assert_eq!(logical.get(), -30.0);
        assert_eq!(scroll.offset().x, px(-30.0));
    });
}

struct OwnedMotionSurface {
    scroll: ScrollHandle,
    motion: ElasticScrollState,
    width: Pixels,
    content_width: Pixels,
}

impl Render for OwnedMotionSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        ElasticScroll::horizontal(
            "owned_motion_surface_elastic",
            self.scroll.clone(),
            div()
                .id("owned_motion_surface")
                .flex()
                .w(self.width)
                .h(px(80.0))
                .overflow_x_scroll()
                .track_scroll(&self.scroll)
                .child(
                    div()
                        .debug_selector(|| "owned_motion_content".into())
                        .flex_shrink_0()
                        .w(self.content_width)
                        .h(px(80.0)),
                ),
        )
        .motion_state(self.motion.clone())
    }
}

#[gpui::test]
fn owned_motion_cancels_stretch_on_programmatic_scroll_and_viewport_resize(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            OwnedMotionSurface {
                scroll: ScrollHandle::new(),
                motion: Default::default(),
                width: px(200.0),
                content_width: px(700.0),
            }
        });
        refresh_and_draw(cx);
        let (scroll, motion) =
            view.read_with(cx, |view, _| (view.scroll.clone(), view.motion.clone()));
        let position = scroll.bounds().center();
        let first = cx.debug_bounds("owned_motion_content").unwrap();
        precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
        assert!(cx.debug_bounds("owned_motion_content").unwrap().left() > first.left() + px(20.0));

        scroll.set_offset(point(px(-40.25), px(0.0)));
        refresh_and_draw(cx);
        assert_eq!(scroll.offset().x, px(-40.25));
        assert!(
            (cx.debug_bounds("owned_motion_content").unwrap().left() - (first.left() - px(40.25)))
                .abs()
                <= px(0.5)
        );
        assert_eq!(motion.displacement(), px(0.0));
        assert!(!motion.0.borrow().motion.animating());

        scroll.set_offset(point(px(0.0), px(0.0)));
        refresh_and_draw(cx);
        precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
        assert!(cx.debug_bounds("owned_motion_content").unwrap().left() > first.left() + px(20.0));
        view.update(cx, |view, cx| {
            view.width = px(250.0);
            cx.notify();
        });
        refresh_and_draw(cx);
        assert_eq!(cx.debug_bounds("owned_motion_content").unwrap(), first);
        assert_eq!(scroll.max_offset().x, px(450.0));
        assert_eq!(motion.displacement(), px(0.0));
        assert!(!motion.0.borrow().motion.animating());
    });
}

#[gpui::test]
fn appended_content_clears_a_held_old_edge_using_its_own_cleanup_frame(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            OwnedMotionSurface {
                scroll: ScrollHandle::new(),
                motion: Default::default(),
                width: px(200.0),
                content_width: px(700.0),
            }
        });
        refresh_and_draw(cx);
        let (scroll, motion) =
            view.read_with(cx, |view, _| (view.scroll.clone(), view.motion.clone()));
        scroll.set_offset(point(px(-500.0), px(0.0)));
        refresh_and_draw(cx);
        precise_wheel(
            cx,
            scroll.bounds().center(),
            -100.0,
            0.0,
            TouchPhase::Started,
        );
        assert!(motion.displacement() < px(-20.0));
        assert!(
            !motion.0.borrow().motion.animating(),
            "holding a finger schedules no spring"
        );
        view.update(cx, |view, cx| {
            view.content_width = px(900.0);
            cx.notify();
        });
        refresh_and_draw(cx);
        assert_eq!(scroll.offset().x, px(-500.0));
        assert_eq!(scroll.max_offset().x, px(700.0));
        // No release or incidental refresh: the measured growth schedules
        // exactly the frame needed to discard the obsolete edge translation.
        cx.update(|window, app| {
            window.simulate_next_frame(app);
            let _ = window.draw(app);
        });
        assert_eq!(motion.displacement(), px(0.0));
        assert_eq!(
            cx.debug_bounds("owned_motion_content").unwrap().left(),
            px(-500.0)
        );
        assert!(!motion.0.borrow().motion.animating());
        assert!(!motion.0.borrow().geometry_reset_scheduled);
    });
}

struct UnequalLinkedSurfaces {
    narrow: ScrollHandle,
    wide: ScrollHandle,
    motion: ElasticScrollState,
}

impl Render for UnequalLinkedSurfaces {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let surface = |name: &'static str, width: f32, scroll: &ScrollHandle| {
            div().w(px(width)).h(px(80.0)).child(
                ElasticScroll::horizontal(
                    (ElementId::from(name), "elastic"),
                    scroll.clone(),
                    div()
                        .id((ElementId::from(name), "viewport"))
                        .flex()
                        .w_full()
                        .h_full()
                        .overflow_x_scroll()
                        .track_scroll(scroll)
                        .child(
                            div()
                                .debug_selector(move || name.into())
                                .flex_shrink_0()
                                .w(px(700.0))
                                .h(px(80.0)),
                        ),
                )
                .shared_motion(self.motion.clone()),
            )
        };
        div()
            .flex()
            .flex_col()
            .gap(px(10.0))
            .child(surface("linked_narrow_content", 200.0, &self.narrow))
            .child(surface("linked_wide_content", 300.0, &self.wide))
    }
}

#[gpui::test]
fn linked_horizontal_surfaces_with_unequal_widths_share_stretch_and_return(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            UnequalLinkedSurfaces {
                narrow: ScrollHandle::new(),
                wide: ScrollHandle::new(),
                motion: Default::default(),
            }
        });
        refresh_and_draw(cx);
        let (narrow, wide) = view.read_with(cx, |view, _| (view.narrow.clone(), view.wide.clone()));
        let first_narrow = cx.debug_bounds("linked_narrow_content").unwrap();
        let first_wide = cx.debug_bounds("linked_wide_content").unwrap();
        assert_ne!(narrow.bounds().size.width, wide.bounds().size.width);
        assert_ne!(narrow.max_offset().x, wide.max_offset().x);

        for position in [narrow.bounds().center(), wide.bounds().center()] {
            precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
            let narrow_travel =
                cx.debug_bounds("linked_narrow_content").unwrap().left() - first_narrow.left();
            let wide_travel =
                cx.debug_bounds("linked_wide_content").unwrap().left() - first_wide.left();
            assert!(narrow_travel > px(20.0));
            assert_eq!(wide_travel, narrow_travel);
            assert_eq!(narrow.offset().x, px(0.0));
            assert_eq!(wide.offset().x, px(0.0));

            precise_wheel(cx, position, 0.0, 0.0, TouchPhase::Ended);
            cx.background_executor
                .advance_clock(Duration::from_millis(40));
            cx.update(|window, app| window.simulate_next_frame(app));
            refresh_and_draw(cx);
            let narrow_return =
                cx.debug_bounds("linked_narrow_content").unwrap().left() - first_narrow.left();
            let wide_return =
                cx.debug_bounds("linked_wide_content").unwrap().left() - first_wide.left();
            assert!(narrow_return > px(0.0) && narrow_return < narrow_travel);
            assert_eq!(wide_return, narrow_return);

            cx.background_executor.advance_clock(Duration::from_secs(1));
            cx.update(|window, app| window.simulate_next_frame(app));
            refresh_and_draw(cx);
            assert_eq!(
                cx.debug_bounds("linked_narrow_content").unwrap(),
                first_narrow
            );
            assert_eq!(cx.debug_bounds("linked_wide_content").unwrap(), first_wide);
        }
    });
}

#[gpui::test]
fn linked_motion_cancels_all_columns_before_prepaint_when_the_master_moves_programmatically(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            UnequalLinkedSurfaces {
                narrow: ScrollHandle::new(),
                wide: ScrollHandle::new(),
                motion: Default::default(),
            }
        });
        refresh_and_draw(cx);
        let (narrow, wide, motion) = view.read_with(cx, |view, _| {
            (view.narrow.clone(), view.wide.clone(), view.motion.clone())
        });
        let first_narrow = cx.debug_bounds("linked_narrow_content").unwrap();
        let first_wide = cx.debug_bounds("linked_wide_content").unwrap();
        for master in [&narrow, &wide] {
            narrow.set_offset(point(px(0.0), px(0.0)));
            wide.set_offset(point(px(0.0), px(0.0)));
            refresh_and_draw(cx);
            precise_wheel(
                cx,
                master.bounds().center(),
                100.0,
                0.0,
                TouchPhase::Started,
            );
            assert!(motion.displacement() > px(20.0));
            master.set_offset(point(px(-40.0), px(0.0)));
            refresh_and_draw(cx);
            assert_eq!(motion.displacement(), px(0.0));
            assert_eq!(
                cx.debug_bounds("linked_narrow_content").unwrap().left(),
                first_narrow.left() + narrow.offset().x
            );
            assert_eq!(
                cx.debug_bounds("linked_wide_content").unwrap().left(),
                first_wide.left() + wide.offset().x
            );
        }
    });
}

struct ShortLinkedSurface {
    width: Pixels,
    scrolls: [ScrollHandle; 2],
    motion: ElasticScrollState,
}
impl Render for ShortLinkedSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .children(self.scrolls.iter().enumerate().map(|(ix, scroll)| {
                ElasticScroll::horizontal(
                    ("short_linked_elastic", ix),
                    scroll.clone(),
                    div()
                        .id(("short_linked_viewport", ix))
                        .w(self.width)
                        .h(px(80.0))
                        .overflow_x_scroll()
                        .track_scroll(scroll)
                        .child(
                            div()
                                .debug_selector(move || format!("short_linked_content_{ix}"))
                                .w(px(80.0))
                                .h(px(80.0)),
                        ),
                )
                .shared_motion(self.motion.clone())
            }))
    }
}

#[gpui::test]
fn linked_short_documents_cancel_resize_together_even_with_unchanged_zero_extent(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            ShortLinkedSurface {
                width: px(200.0),
                scrolls: Default::default(),
                motion: Default::default(),
            }
        });
        refresh_and_draw(cx);
        let (scroll, motion) =
            view.read_with(cx, |view, _| (view.scrolls[0].clone(), view.motion.clone()));
        let first = cx.debug_bounds("short_linked_content_0").unwrap();
        let second = cx.debug_bounds("short_linked_content_1").unwrap();
        precise_wheel(
            cx,
            scroll.bounds().center(),
            100.0,
            0.0,
            TouchPhase::Started,
        );
        assert!(motion.displacement() > px(20.0));
        view.update(cx, |view, cx| {
            view.width = px(250.0);
            cx.notify();
        });
        refresh_and_draw(cx);
        let a = cx.debug_bounds("short_linked_content_0").unwrap().left() - first.left();
        let b = cx.debug_bounds("short_linked_content_1").unwrap().left() - second.left();
        assert_eq!(a, b, "resize cannot split the group for one frame");
        cx.update(|window, app| window.simulate_next_frame(app));
        refresh_and_draw(cx);
        assert_eq!(scroll.max_offset().x, px(0.0));
        assert_eq!(motion.displacement(), px(0.0));
        assert_eq!(cx.debug_bounds("short_linked_content_0").unwrap(), first);
        assert_eq!(cx.debug_bounds("short_linked_content_1").unwrap(), second);
        assert!(!motion.0.borrow().motion.animating());
    });
}

struct LinkedTrailingSurfaces {
    edge: ScrollHandle,
    follower: ScrollHandle,
    motion: ElasticScrollState,
    edge_first: bool,
}

impl Render for LinkedTrailingSurfaces {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let surface =
            |name: &'static str, viewport_width: f32, content_width: f32, scroll: &ScrollHandle| {
                div().w(px(viewport_width)).h(px(80.0)).child(
                    ElasticScroll::horizontal(
                        (ElementId::from(name), "elastic"),
                        scroll.clone(),
                        div()
                            .id((ElementId::from(name), "viewport"))
                            .flex()
                            .w_full()
                            .h_full()
                            .overflow_x_scroll()
                            .track_scroll(scroll)
                            .child(
                                div()
                                    .debug_selector(move || name.into())
                                    .flex_shrink_0()
                                    .w(px(content_width))
                                    .h(px(80.0)),
                            ),
                    )
                    .shared_motion(self.motion.clone()),
                )
            };
        let edge = surface("linked_trailing_edge_content", 200.0, 300.0, &self.edge);
        let follower = surface(
            "linked_trailing_follower_content",
            300.0,
            800.0,
            &self.follower,
        );
        let column = div().flex().flex_col().gap(px(10.0));
        if self.edge_first {
            column.child(edge).child(follower)
        } else {
            column.child(follower).child(edge)
        }
    }
}

#[gpui::test]
fn linked_trailing_edge_stretches_an_interior_follower_in_either_prepaint_order(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        for edge_first in [true, false] {
            let (view, cx) = cx.add_window_view(|window, _| {
                window.activate();
                LinkedTrailingSurfaces {
                    edge: ScrollHandle::new(),
                    follower: ScrollHandle::new(),
                    motion: Default::default(),
                    edge_first,
                }
            });
            refresh_and_draw(cx);
            let (edge, follower, motion) = view.read_with(cx, |view, _| {
                (
                    view.edge.clone(),
                    view.follower.clone(),
                    view.motion.clone(),
                )
            });
            assert_eq!(edge.max_offset().x, px(100.0));
            assert_eq!(follower.max_offset().x, px(500.0));
            edge.set_offset(point(px(-100.0), px(0.0)));
            follower.set_offset(point(px(-100.0), px(0.0)));
            refresh_and_draw(cx);
            let edge_before = cx.debug_bounds("linked_trailing_edge_content").unwrap();
            let follower_before = cx.debug_bounds("linked_trailing_follower_content").unwrap();
            let position = edge.bounds().center();

            precise_wheel(cx, position, -100.0, 0.0, TouchPhase::Started);
            let edge_travel = cx
                .debug_bounds("linked_trailing_edge_content")
                .unwrap()
                .left()
                - edge_before.left();
            let follower_travel = cx
                .debug_bounds("linked_trailing_follower_content")
                .unwrap()
                .left()
                - follower_before.left();
            assert!(edge_travel < px(-20.0), "edge_first={edge_first}");
            assert_eq!(follower_travel, edge_travel, "edge_first={edge_first}");
            assert_eq!(edge.offset().x, px(-100.0));
            assert_eq!(follower.offset().x, px(-100.0));

            precise_wheel(cx, position, 0.0, 0.0, TouchPhase::Ended);
            cx.background_executor
                .advance_clock(Duration::from_millis(40));
            cx.update(|window, app| window.simulate_next_frame(app));
            refresh_and_draw(cx);
            let edge_return = cx
                .debug_bounds("linked_trailing_edge_content")
                .unwrap()
                .left()
                - edge_before.left();
            let follower_return = cx
                .debug_bounds("linked_trailing_follower_content")
                .unwrap()
                .left()
                - follower_before.left();
            assert!(edge_return < px(0.0) && edge_return > edge_travel);
            assert_eq!(follower_return, edge_return, "edge_first={edge_first}");

            cx.background_executor.advance_clock(Duration::from_secs(1));
            cx.update(|window, app| window.simulate_next_frame(app));
            refresh_and_draw(cx);
            assert_eq!(
                cx.debug_bounds("linked_trailing_edge_content").unwrap(),
                edge_before
            );
            assert_eq!(
                cx.debug_bounds("linked_trailing_follower_content").unwrap(),
                follower_before
            );
            assert!(!motion.0.borrow().motion.animating());
            assert!(!motion.0.borrow().frame_scheduled);
        }
    });
}

struct NestedAxes {
    vertical: ScrollHandle,
    horizontal: ScrollHandle,
}

impl Render for NestedAxes {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut inner = div()
            .id("nested_horizontal_surface")
            .flex()
            .w(px(180.0))
            .h(px(80.0))
            .overflow_x_scroll()
            .track_scroll(&self.horizontal);
        inner.style().restrict_scroll_to_axis = Some(true);
        let inner = inner.child(
            div()
                .debug_selector(|| "nested_horizontal_content".into())
                .flex_shrink_0()
                .w(px(700.0))
                .h(px(80.0)),
        );
        ElasticScroll::new(
            "nested_vertical_elastic",
            self.vertical.clone(),
            div()
                .id("nested_vertical_surface")
                .w(px(200.0))
                .h(px(120.0))
                .overflow_y_scroll()
                .track_scroll(&self.vertical)
                .child(
                    div()
                        .debug_selector(|| "nested_vertical_content".into())
                        .h(px(700.0))
                        .child(
                            ElasticScroll::horizontal(
                                "nested_horizontal_elastic",
                                self.horizontal.clone(),
                                inner,
                            )
                            .horizontal_only(),
                        ),
                ),
        )
        .vertical_only()
    }
}

#[gpui::test]
fn nested_horizontal_surface_leaves_vertical_gestures_to_its_parent(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            NestedAxes {
                vertical: ScrollHandle::new(),
                horizontal: ScrollHandle::new(),
            }
        });
        refresh_and_draw(cx);
        let (vertical, horizontal) = view.read_with(cx, |view, _| {
            (view.vertical.clone(), view.horizontal.clone())
        });
        let position = horizontal.bounds().center();
        let outer = cx.debug_bounds("nested_vertical_content").unwrap();
        let inner = cx.debug_bounds("nested_horizontal_content").unwrap();

        precise_wheel(cx, position, 0.0, 100.0, TouchPhase::Started);
        assert!(cx.debug_bounds("nested_vertical_content").unwrap().top() > outer.top() + px(20.0));
        assert_eq!(horizontal.offset().x, px(0.0));
        assert_eq!(vertical.offset().y, px(0.0));
        settle(cx, position);
        assert_eq!(cx.debug_bounds("nested_vertical_content").unwrap(), outer);

        precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
        assert!(
            cx.debug_bounds("nested_horizontal_content").unwrap().left() > inner.left() + px(20.0)
        );
        assert_eq!(cx.debug_bounds("nested_vertical_content").unwrap(), outer);
        assert_eq!(vertical.offset().y, px(0.0));
        settle(cx, position);
        assert_eq!(cx.debug_bounds("nested_horizontal_content").unwrap(), inner);

        precise_wheel(cx, position, 0.0, -24.25, TouchPhase::Started);
        assert_eq!(vertical.offset().y, px(-24.25));
        assert_eq!(horizontal.offset().x, px(0.0));
    });
}

#[gpui::test]
fn nested_surfaces_keep_the_started_axis_when_small_jitter_changes_packet_dominance(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            NestedAxes {
                vertical: ScrollHandle::new(),
                horizontal: ScrollHandle::new(),
            }
        });
        refresh_and_draw(cx);
        let (vertical, horizontal) = view.read_with(cx, |view, _| {
            (view.vertical.clone(), view.horizontal.clone())
        });
        let position = horizontal.bounds().center();

        // Dispatch each pair together: the second packet belongs to the same
        // gesture and arrives before GPUI's 28 ms fallback separation. Neither
        // packet reaches its 6 px axis-unlock threshold.
        for (packets, expected_y, expected_x) in [
            ([(0.0, -5.0), (-5.0, -4.0)], -9.0, 0.0),
            ([(-5.0, 0.0), (-4.0, -5.0)], 0.0, -9.0),
        ] {
            vertical.set_offset(point(px(0.0), px(0.0)));
            horizontal.set_offset(point(px(0.0), px(0.0)));
            refresh_and_draw(cx);
            cx.update(|window, app| {
                for (ix, (x, y)) in packets.into_iter().enumerate() {
                    window.dispatch_event(
                        gpui::PlatformInput::ScrollWheel(ScrollWheelEvent {
                            position,
                            delta: ScrollDelta::Pixels(point(px(x), px(y))),
                            touch_phase: if ix == 0 {
                                TouchPhase::Started
                            } else {
                                TouchPhase::Moved
                            },
                            modifiers: Default::default(),
                        }),
                        app,
                    );
                }
            });
            refresh_and_draw(cx);
            assert_eq!(vertical.offset().y, px(expected_y));
            assert_eq!(horizontal.offset().x, px(expected_x));
            settle(cx, position);
        }
    });
}

struct TrackedSurface {
    revision: usize,
}

impl Render for TrackedSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .debug_selector(|| "tracked_fixed_viewport".into())
            .w(px(200.0))
            .h(px(120.0))
            .child(ElasticScroll::tracked_vertical(
                "tracked_surface_elastic",
                div()
                    .id("tracked_surface")
                    .w_full()
                    .h_full()
                    .overflow_y_scroll()
                    .child(
                        div()
                            .id(("tracked_content_revision", self.revision))
                            .debug_selector(|| "tracked_surface_content".into())
                            .h(px(700.0)),
                    ),
            ))
    }
}

#[gpui::test]
fn tracked_vertical_retains_implicit_scroll_across_renders_and_bounces_at_the_end(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            TrackedSurface { revision: 0 }
        });
        refresh_and_draw(cx);
        let viewport = cx.debug_bounds("tracked_fixed_viewport").unwrap();
        let position = viewport.center();
        let first = cx.debug_bounds("tracked_surface_content").unwrap();
        precise_wheel(cx, position, 0.0, -40.25, TouchPhase::Started);
        settle(cx, position);
        let interior = cx.debug_bounds("tracked_surface_content").unwrap();
        // The logical model retains subpixels; GPUI snaps painted row bounds.
        assert!((interior.top() - (first.top() - px(40.25))).abs() <= px(0.5));

        view.update(cx, |view, cx| {
            view.revision += 1;
            cx.notify();
        });
        refresh_and_draw(cx);
        refresh_and_draw(cx);
        assert_eq!(
            cx.debug_bounds("tracked_surface_content").unwrap(),
            interior
        );

        precise_wheel(cx, position, 0.0, -1_000.0, TouchPhase::Started);
        settle(cx, position);
        let last = cx.debug_bounds("tracked_surface_content").unwrap();
        assert_eq!(
            last.bottom(),
            viewport.bottom(),
            "the logical end remains clamped"
        );
        precise_wheel(cx, position, 0.0, -100.0, TouchPhase::Started);
        assert!(cx.debug_bounds("tracked_surface_content").unwrap().top() < last.top() - px(20.0));
        assert_eq!(cx.debug_bounds("tracked_fixed_viewport").unwrap(), viewport);
        settle(cx, position);
        assert_eq!(cx.debug_bounds("tracked_surface_content").unwrap(), last);
        view.update(cx, |view, cx| {
            view.revision += 1;
            cx.notify();
        });
        refresh_and_draw(cx);
        assert_eq!(cx.debug_bounds("tracked_surface_content").unwrap(), last);
    });
}

struct VariableListSurface {
    scroll: gpui::ListState,
    motion: ElasticScrollState,
    rows_built: usize,
}

impl Render for VariableListSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.rows_built = 0;
        div()
            .debug_selector(|| "variable_list_fixed_viewport".into())
            .w(px(200.0))
            .h(px(120.0))
            .child(
                ElasticScroll::new(
                    "variable_list_elastic",
                    self.scroll.clone(),
                    gpui::list(
                        self.scroll.clone(),
                        cx.processor(|this, ix: usize, _, _| {
                            this.rows_built += 1;
                            div()
                                .debug_selector(move || format!("variable_list_row_{ix}"))
                                .h(px(24.0))
                                .into_any_element()
                        }),
                    )
                    .size_full(),
                )
                .vertical_only()
                .motion_state(self.motion.clone()),
            )
    }
}

#[gpui::test]
fn list_state_preserves_fractional_rows_virtualization_and_both_elastic_boundaries(
    cx: &mut TestAppContext,
) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            let scroll = gpui::ListState::new(2_000, gpui::ListAlignment::Top, px(48.0));
            scroll.reset_with_uniform_height(2_000, px(24.0));
            VariableListSurface {
                scroll,
                motion: Default::default(),
                rows_built: 0,
            }
        });
        refresh_and_draw(cx);
        let (scroll, motion) =
            view.read_with(cx, |view, _| (view.scroll.clone(), view.motion.clone()));
        let viewport = cx.debug_bounds("variable_list_fixed_viewport").unwrap();
        let position = viewport.center();
        let first = cx.debug_bounds("variable_list_row_0").unwrap();
        assert!(view.read_with(cx, |view, _| view.rows_built) < 40);
        // GPUI clears uniform hints on first layout. Only the visible rows are
        // measured: its scrollbar maximum is not the document's actual end.
        assert_eq!(scroll.is_scrolled_to_end(), None);
        assert!(scroll.max_offset_for_scrollbar().y < px(1_000.0));

        precise_wheel(cx, position, 0.0, -40.25, TouchPhase::Started);
        let logical = scroll.logical_scroll_top();
        assert_eq!(logical.item_ix, 1);
        assert_eq!(logical.offset_in_item, px(16.25));
        assert!(
            (cx.debug_bounds("variable_list_row_1").unwrap().top() - (viewport.top() - px(16.25)))
                .abs()
                <= px(0.5)
        );
        assert!(view.read_with(cx, |view, _| view.rows_built) < 40);
        assert_eq!(motion.displacement(), px(0.0));
        for packet in 0..12 {
            // The native listener clamps against currently measured rows,
            // then layout extends the known range by its 48px overdraw. A
            // 100px packet therefore need not advance a full 100px yet.
            let before = -scroll.scroll_px_offset_for_scrollbar().y;
            let native_max = scroll.max_offset_for_scrollbar().y;
            let expected = (before + px(100.0)).min(native_max);
            assert!(
                expected > before,
                "the native viewport can advance packet {packet}"
            );
            precise_wheel(cx, position, 0.0, -100.0, TouchPhase::Moved);
            assert_eq!(
                -scroll.scroll_px_offset_for_scrollbar().y,
                expected,
                "packet {packet} retains native measured-range scrolling"
            );
            assert_eq!(
                motion.displacement(),
                px(0.0),
                "unknown rows are not a bottom boundary"
            );
            assert!(view.read_with(cx, |view, _| view.rows_built) < 40);
        }
        assert_eq!(scroll.is_scrolled_to_end(), None);
        assert!(scroll.max_offset_for_scrollbar().y < px(4_000.0));
        settle(cx, position);

        scroll.scroll_to(gpui::ListOffset {
            item_ix: 0,
            offset_in_item: px(0.0),
        });
        refresh_and_draw(cx);
        precise_wheel(cx, position, 0.0, 100.0, TouchPhase::Started);
        assert!(cx.debug_bounds("variable_list_row_0").unwrap().top() > first.top() + px(20.0));
        assert_eq!(scroll.logical_scroll_top().item_ix, 0);
        assert_eq!(scroll.logical_scroll_top().offset_in_item, px(0.0));
        settle(cx, position);
        assert_eq!(cx.debug_bounds("variable_list_row_0").unwrap(), first);
        precise_wheel(cx, position, 0.0, 100.0, TouchPhase::Started);
        let stretched = motion.displacement();
        precise_wheel(cx, position, 0.0, -5.0, TouchPhase::Moved);
        assert_eq!(scroll.logical_scroll_top().item_ix, 0);
        assert_eq!(scroll.logical_scroll_top().offset_in_item, px(0.0));
        assert!(motion.displacement() > px(0.0) && motion.displacement() < stretched);
        precise_wheel(cx, position, 0.0, -100.0, TouchPhase::Moved);
        assert_eq!(motion.displacement(), px(0.0));
        // The pull admitted 100px of finger travel. Returning 5px then 100px
        // takes up that travel before the final 5px enters the first row.
        assert_eq!(scroll.logical_scroll_top().item_ix, 0);
        assert_eq!(scroll.logical_scroll_top().offset_in_item, px(5.0));
        assert_eq!(scroll.scroll_px_offset_for_scrollbar().y, px(-5.0));
        settle(cx, position);

        // Jump by item anchor, not the provisional scrollbar estimate. The
        // final rendered row establishes a real bottom even while earlier
        // unvisited rows still have unknown heights.
        scroll.scroll_to_end();
        refresh_and_draw(cx);
        let last = cx.debug_bounds("variable_list_row_1999").unwrap();
        assert_eq!(last.bottom(), viewport.bottom());
        assert_eq!(scroll.is_scrolled_to_end(), None);
        assert_eq!(
            scroll.scroll_px_offset_for_scrollbar().y,
            -scroll.max_offset_for_scrollbar().y
        );
        precise_wheel(cx, position, 0.0, -100.0, TouchPhase::Started);
        assert!(cx.debug_bounds("variable_list_row_1999").unwrap().top() < last.top() - px(20.0));
        assert_eq!(scroll.logical_scroll_top().item_ix, 1_995);
        assert_eq!(scroll.logical_scroll_top().offset_in_item, px(0.0));
        assert!(view.read_with(cx, |view, _| view.rows_built) < 40);
        settle(cx, position);
        assert_eq!(cx.debug_bounds("variable_list_row_1999").unwrap(), last);
    });
}

struct TabSurface {
    scroll: TabBarScroll,
    count: usize,
}

impl Render for TabSurface {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut tabs = TabBar::new("elastic_test_tabs").scroll(self.scroll.clone());
        for ix in 0..self.count {
            tabs = tabs.tab(
                div()
                    .debug_selector(move || format!("elastic_test_tab_{ix}"))
                    .flex_none()
                    .w(px(100.0))
                    .h(px(34.0)),
            );
        }
        let tabs = tabs.tab_end(
            div()
                .debug_selector(|| "elastic_test_tab_add".into())
                .flex_none()
                .w(px(32.0))
                .h(px(34.0)),
        );
        div().flex().w(px(300.0)).h(px(40.0)).child(tabs.render())
    }
}

#[gpui::test]
fn tab_bar_keeps_add_control_and_viewport_fixed_and_reveal_clears_bounce(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            TabSurface {
                scroll: TabBarScroll::new(),
                count: 8,
            }
        });
        refresh_and_draw(cx);
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        let viewport = scroll.viewport();
        let position = viewport.center();
        let add = cx.debug_bounds("elastic_test_tab_add").unwrap();
        let first = cx.debug_bounds("elastic_test_tab_0").unwrap();
        assert!(scroll.max_scroll() > px(400.0));
        assert!(scroll.tab_is_visible(0));

        precise_wheel(cx, position, 100.0, 0.0, TouchPhase::Started);
        assert!(cx.debug_bounds("elastic_test_tab_0").unwrap().left() > first.left() + px(20.0));
        assert_eq!(scroll.scrolled(), px(0.0));
        assert_eq!(scroll.viewport(), viewport);
        assert_eq!(cx.debug_bounds("elastic_test_tab_add").unwrap(), add);
        assert!(
            scroll.tab_is_visible(0),
            "elastic paint must not change logical visibility"
        );

        // This is the same reveal API used by repository-tab keyboard navigation.
        scroll.scroll_to_tab(7);
        refresh_and_draw(cx);
        refresh_and_draw(cx);
        assert!(scroll.tab_is_visible(7));
        assert_eq!(scroll.scrolled(), scroll.max_scroll());
        assert_eq!(scroll.viewport(), viewport);
        assert_eq!(cx.debug_bounds("elastic_test_tab_add").unwrap(), add);
        let last = cx.debug_bounds("elastic_test_tab_7").unwrap();
        assert_eq!(
            last.right(),
            viewport.right(),
            "reveal must clear the old displacement"
        );

        precise_wheel(cx, position, -100.0, 0.0, TouchPhase::Started);
        assert!(cx.debug_bounds("elastic_test_tab_7").unwrap().left() < last.left() - px(20.0));
        assert_eq!(scroll.scrolled(), scroll.max_scroll());
        assert_eq!(scroll.viewport(), viewport);
        assert_eq!(cx.debug_bounds("elastic_test_tab_add").unwrap(), add);
        settle(cx, position);
        assert_eq!(cx.debug_bounds("elastic_test_tab_7").unwrap(), last);

        precise_wheel(cx, position, -100.0, 0.0, TouchPhase::Started);
        scroll.scroll_to_tab(0);
        refresh_and_draw(cx);
        refresh_and_draw(cx);
        assert_eq!(scroll.scrolled(), px(0.0));
        assert_eq!(cx.debug_bounds("elastic_test_tab_0").unwrap(), first);
        assert_eq!(cx.debug_bounds("elastic_test_tab_add").unwrap(), add);
    });
}

struct PickerSurface {
    query: Entity<crate::TextInput>,
    scroll: ScrollHandle,
    items: Rc<[PickerPromptItem]>,
    layout: Rc<PickerPromptLayout>,
    geometry: Rc<PickerPromptGeometry>,
}

impl Render for PickerSurface {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(320.0)).child(
            PickerPrompt::new(self.query.clone(), self.scroll.clone())
                .prebuilt_items(self.items.clone(), self.layout.clone())
                .prebuilt_geometry(self.geometry.clone())
                .max_height(px(120.0))
                .padded_query_row()
                .query_row_trailing(
                    div()
                        .debug_selector(|| "elastic_picker_query_control".into())
                        .w(px(20.0))
                        .h(px(20.0)),
                )
                .render(AppTheme::gitcomet_dark(), 100u32, cx, |_, _, _, _, _| {}),
        )
    }
}

#[gpui::test]
fn picker_prompt_keeps_query_and_scrollbar_fixed_and_preserves_windowing(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, cx| {
            window.activate();
            let query = cx.new(|cx| crate::TextInput::new(Default::default(), window, cx));
            let items: Rc<[PickerPromptItem]> = (0..500)
                .map(|ix| PickerPromptItem::plain(format!("Repository {ix:04}")))
                .collect::<Vec<_>>()
                .into();
            let layout = Rc::new(picker_prompt_layout(&items, ""));
            let geometry = Rc::new(PickerPromptGeometry::new(&items, &layout, 100u32));
            PickerSurface {
                query,
                scroll: ScrollHandle::new(),
                items,
                layout,
                geometry,
            }
        });
        refresh_and_draw(cx);
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        let position = scroll.bounds().center();
        let query = cx.debug_bounds("elastic_picker_query_control").unwrap();
        let scrollbar = cx.debug_bounds("picker_prompt_scrollbar").unwrap();
        let first = cx.debug_bounds("picker_prompt_item_0").unwrap();
        let max = scroll.max_offset().y;
        assert!(max > px(1_000.0));
        assert!(cx.debug_bounds("picker_prompt_item_100").is_none());
        assert!(cx.debug_bounds("picker_prompt_item_499").is_none());

        precise_wheel(cx, position, 0.0, 100.0, TouchPhase::Started);
        assert!(cx.debug_bounds("picker_prompt_item_0").unwrap().top() > first.top() + px(20.0));
        assert_eq!(scroll.offset().y, px(0.0));
        assert_eq!(scroll.max_offset().y, max);
        assert_eq!(
            cx.debug_bounds("elastic_picker_query_control").unwrap(),
            query
        );
        assert_eq!(
            cx.debug_bounds("picker_prompt_scrollbar").unwrap(),
            scrollbar
        );
        assert!(cx.debug_bounds("picker_prompt_item_100").is_none());
        settle(cx, position);
        assert_eq!(cx.debug_bounds("picker_prompt_item_0").unwrap(), first);

        scroll.set_offset(point(px(0.0), -max));
        refresh_and_draw(cx);
        refresh_and_draw(cx);
        let last = cx.debug_bounds("picker_prompt_item_499").unwrap();
        assert!(cx.debug_bounds("picker_prompt_item_0").is_none());
        precise_wheel(cx, position, 0.0, -100.0, TouchPhase::Started);
        assert!(cx.debug_bounds("picker_prompt_item_499").unwrap().top() < last.top() - px(20.0));
        assert_eq!(scroll.offset().y, -max);
        assert_eq!(scroll.max_offset().y, max);
        assert_eq!(
            cx.debug_bounds("elastic_picker_query_control").unwrap(),
            query
        );
        assert_eq!(
            cx.debug_bounds("picker_prompt_scrollbar").unwrap(),
            scrollbar
        );
        assert!(cx.debug_bounds("picker_prompt_item_0").is_none());
        settle(cx, position);
        assert_eq!(cx.debug_bounds("picker_prompt_item_499").unwrap(), last);
    });
}

#[gpui::test]
fn the_latest_tab_reveal_supersedes_an_unmeasured_new_tab_before_paint(cx: &mut TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (view, cx) = cx.add_window_view(|window, _| {
            window.activate();
            TabSurface {
                scroll: TabBarScroll::new(),
                count: 8,
            }
        });
        refresh_and_draw(cx);
        let scroll = view.read_with(cx, |view, _| view.scroll.clone());
        view.update(cx, |view, cx| {
            view.count = 9;
            view.scroll.scroll_to_tab(8);
            view.scroll.scroll_to_tab(0);
            cx.notify();
        });
        refresh_and_draw(cx);
        refresh_and_draw(cx);
        assert_eq!(scroll.scrolled(), px(0.0));
        assert!(scroll.tab_is_visible(0));
        assert!(!scroll.tab_is_visible(8));
        scroll.scroll_to_tab(8);
        refresh_and_draw(cx);
        refresh_and_draw(cx);
        assert!(scroll.tab_is_visible(8));

        view.update(cx, |view, cx| {
            view.count = 10;
            view.scroll.scroll_to_tab(9);
            cx.notify();
        });
        refresh_and_draw(cx);
        // The deferred reveal must request its own redraw. Merely advancing
        // the frame runs that request; this test does not force a refresh.
        cx.update(|window, app| {
            window.simulate_next_frame(app);
            let _ = window.draw(app);
        });
        assert_eq!(
            cx.debug_bounds("elastic_test_tab_9").unwrap().right(),
            scroll.viewport().right()
        );
    });
}
