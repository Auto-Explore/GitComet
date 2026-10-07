use super::{canvas_target, on_canvas_click, on_click};
use crate::test_support::{lock_visual_test, refresh_and_draw};
use gpui::prelude::*;
use gpui::{
    AnyElement, Context, ElementId, Entity, HitboxBehavior, Modifiers, MouseButton, Render,
    TestAppContext, VisualTestContext, Window, canvas, div, point, px,
};

#[test]
fn canvas_targets_are_integers_that_tell_their_keys_apart() {
    let row = canvas_target("diff-click", (7u64, 3usize, "row"));
    assert!(
        matches!(row, ElementId::NamedInteger(..)),
        "no string is built for a target"
    );
    assert_eq!(row, canvas_target("diff-click", (7u64, 3usize, "row")));
    assert_ne!(row, canvas_target("diff-click", (7u64, 3usize, "context")));
    assert_ne!(row, canvas_target("diff-click", (7u64, 4usize, "row")));
    assert_ne!(row, canvas_target("diff-click", (8u64, 3usize, "row")));
    assert_ne!(row, canvas_target("history-menu", (7u64, 3usize, "row")));
}

#[derive(Clone, Copy, Debug)]
enum TargetKind {
    Element,
    Canvas,
}

struct PointerFixture {
    targets: [TargetKind; 2],
    button: MouseButton,
    window_reset: bool,
    clicks: [usize; 2],
}

impl PointerFixture {
    fn target(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let control = div()
            .id(("click_target", index))
            .debug_selector(move || format!("click_target_{index}"))
            .w(px(100.0))
            .h(px(40.0));
        match self.targets[index] {
            TargetKind::Element => on_click(
                control,
                self.button,
                cx.listener(move |this, _, _, _| this.clicks[index] += 1),
            )
            .into_any_element(),
            TargetKind::Canvas => {
                let view = cx.entity();
                let button = self.button;
                control
                    .child(
                        canvas(
                            |bounds, window, _| {
                                window.insert_hitbox(bounds, HitboxBehavior::Normal)
                            },
                            move |_, hitbox, window, _| {
                                on_canvas_click(
                                    window,
                                    ("click_target", index).into(),
                                    &hitbox,
                                    button,
                                    true,
                                    move |_, _, cx| {
                                        view.update(cx, |this, _| this.clicks[index] += 1);
                                    },
                                );
                            },
                        )
                        .size_full(),
                    )
                    .into_any_element()
            }
        }
    }
}

impl Render for PointerFixture {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .relative()
            .size_full()
            .when(self.window_reset, |root| {
                // Register first, as the production window frame's root hook does.
                root.child(
                    canvas(
                        |_, _, _| (),
                        |_, _, window, _| crate::press_gesture::install_reset(window),
                    )
                    .absolute()
                    .size_full(),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(16.0))
                    .children([self.target(0, cx), self.target(1, cx)]),
            )
    }
}

fn fixture(
    cx: &mut TestAppContext,
    targets: [TargetKind; 2],
    button: MouseButton,
    window_reset: bool,
) -> (Entity<PointerFixture>, &mut VisualTestContext) {
    let (view, cx) = cx.add_window_view(|_, _| PointerFixture {
        targets,
        button,
        window_reset,
        clicks: [0; 2],
    });
    refresh_and_draw(cx);
    (view, cx)
}

#[gpui::test]
fn pointer_clicks_survive_movement_on_the_same_target(cx: &mut TestAppContext) {
    let _guard = lock_visual_test();
    for kind in [TargetKind::Element, TargetKind::Canvas] {
        for button in MouseButton::all() {
            for window_reset in [false, true] {
                let (view, cx) = fixture(cx, [kind; 2], button, window_reset);
                for redraw_between_events in [false, true] {
                    for index in 0..2 {
                        let selector = ["click_target_0", "click_target_1"][index];
                        let from = cx.debug_bounds(selector).unwrap().center();
                        let to = from + point(px(1.0), px(1.0));
                        let before = cx.update(|_, app| view.read(app).clicks);
                        cx.simulate_mouse_move(from, None, Modifiers::default());
                        cx.simulate_mouse_down(from, button, Modifiers::default());
                        if redraw_between_events {
                            refresh_and_draw(cx);
                        }
                        cx.simulate_mouse_move(to, Some(button), Modifiers::default());
                        if redraw_between_events {
                            refresh_and_draw(cx);
                        }
                        cx.update(|_, app| {
                            assert_eq!(view.read(app).clicks, before, "a press must not activate");
                        });
                        cx.simulate_mouse_up(to, button, Modifiers::default());
                        let mut expected = before;
                        expected[index] += 1;
                        cx.update(|_, app| {
                            assert_eq!(
                                view.read(app).clicks,
                                expected,
                                "{kind:?}, {button:?}, window_reset={window_reset}, redraw={redraw_between_events}"
                            );
                        });
                        cx.simulate_mouse_up(to, button, Modifiers::default());
                        cx.update(|_, app| {
                            assert_eq!(view.read(app).clicks, expected, "duplicate release");
                        });
                    }
                }
            }
        }
    }
}

#[gpui::test]
fn pointer_clicks_do_not_transfer_to_another_target(cx: &mut TestAppContext) {
    let _guard = lock_visual_test();
    for source in [TargetKind::Element, TargetKind::Canvas] {
        for destination in [TargetKind::Element, TargetKind::Canvas] {
            for button in MouseButton::all() {
                for window_reset in [false, true] {
                    let (view, cx) = fixture(cx, [source, destination], button, window_reset);
                    let a = cx.debug_bounds("click_target_0").unwrap().center();
                    let b = cx.debug_bounds("click_target_1").unwrap().center();
                    let outside = point(px(280.0), px(200.0));
                    for (from, to) in [(a, b), (b, a), (a, outside), (outside, a)] {
                        cx.simulate_mouse_down(from, button, Modifiers::default());
                        refresh_and_draw(cx);
                        cx.simulate_mouse_move(to, Some(button), Modifiers::default());
                        refresh_and_draw(cx);
                        cx.simulate_mouse_up(to, button, Modifiers::default());
                        cx.update(|_, app| {
                            assert_eq!(
                                view.read(app).clicks,
                                [0; 2],
                                "{source:?} -> {destination:?}, {button:?}, window_reset={window_reset}: neither target may activate"
                            );
                        });
                    }
                }
            }
        }
    }
}

#[gpui::test]
fn unheld_pointer_movement_cancels_a_missed_release(cx: &mut TestAppContext) {
    let _guard = lock_visual_test();
    for kind in [TargetKind::Element, TargetKind::Canvas] {
        for button in MouseButton::all() {
            for window_reset in [false, true] {
                let (view, cx) = fixture(cx, [kind; 2], button, window_reset);
                let from = cx.debug_bounds("click_target_0").unwrap().center();
                let to = from + point(px(1.0), px(1.0));
                cx.simulate_mouse_down(from, button, Modifiers::default());
                refresh_and_draw(cx);
                cx.simulate_mouse_move(to, None, Modifiers::default());
                cx.simulate_mouse_up(to, button, Modifiers::default());
                cx.update(|_, app| {
                    assert_eq!(
                        view.read(app).clicks,
                        [0; 2],
                        "a stranded press must cancel"
                    );
                });
                cx.simulate_mouse_down(from, button, Modifiers::default());
                cx.simulate_mouse_up(from, button, Modifiers::default());
                cx.update(|_, app| {
                    assert_eq!(view.read(app).clicks, [1, 0], "the next click must work");
                });
            }
        }
    }
}
