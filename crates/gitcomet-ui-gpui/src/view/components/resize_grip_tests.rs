//! The resize grip inside the application's window frame.

use super::*;
use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{DispatchPhase, Window, canvas, div, point, px};
use gpui::{Entity, MouseButton, ScrollHandle};

struct DragFeedbackView {
    theme: AppTheme,
    axis: ScrollbarAxis,
    scroll: ScrollHandle,
    resizing: Option<ResizeGripAxis>,
    input: Entity<TextInput>,
}

#[derive(Default)]
struct CachedSibling {
    renders: usize,
}

impl gpui::Render for CachedSibling {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        self.renders += 1;
        div().size_full()
    }
}

struct CachedFeedbackRoot {
    feedback: Entity<DragFeedbackView>,
    sibling: Entity<CachedSibling>,
}

impl gpui::Render for CachedFeedbackRoot {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div().size_full().child(self.feedback.clone()).child(
            gpui::AnyView::from(self.sibling.clone()).cached(gpui::StyleRefinement::default()),
        )
    }
}

#[gpui::test]
fn review_pointer_presses_do_not_redraw_unrelated_cached_views(cx: &mut gpui::TestAppContext) {
    let theme = AppTheme::from_key("gitcomet_light").unwrap();
    let (root, cx) = cx.add_window_view(|window, cx| CachedFeedbackRoot {
        feedback: cx.new(|cx| DragFeedbackView::new(theme, ScrollbarAxis::Vertical, window, cx)),
        sibling: cx.new(|_| CachedSibling::default()),
    });
    draw(cx);
    let sibling = cx.update(|_, app| root.read(app).sibling.clone());
    let renders = cx.update(|_, app| sibling.read(app).renders);
    let empty = point(px(400.0), px(400.0));
    cx.simulate_mouse_down(empty, MouseButton::Left, Default::default());
    draw(cx);
    cx.simulate_mouse_up(empty, MouseButton::Left, Default::default());
    draw(cx);
    assert_eq!(
        cx.update(|_, app| sibling.read(app).renders),
        renders,
        "an ordinary click invalidated unrelated cached views"
    );
}

impl DragFeedbackView {
    fn new(
        theme: AppTheme,
        axis: ScrollbarAxis,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextInput::new(TextInputOptions::default(), window, cx);
            input.set_text("Drag a text selection across the divider", cx);
            input
        });
        Self {
            theme,
            axis,
            scroll: ScrollHandle::new(),
            resizing: None,
            input,
        }
    }
}

impl gpui::Render for DragFeedbackView {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        let release_listener = canvas(
            |_, _, _| (),
            move |_, _, window, _| {
                window.on_mouse_event(move |event: &gpui::MouseUpEvent, phase, _, cx| {
                    if phase == DispatchPhase::Capture && event.button == MouseButton::Left {
                        view.update(cx, |view, cx| {
                            if view.resizing.take().is_some() {
                                cx.notify();
                            }
                        });
                    }
                });
            },
        )
        .absolute()
        .inset_0();
        let grip = |id: &'static str, axis, cx: &mut gpui::Context<Self>| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .group(id)
                .absolute()
                .when(axis == ResizeGripAxis::Vertical, |d| {
                    d.left(px(210.0)).top_0().w(px(8.0)).h(px(120.0))
                })
                .when(axis == ResizeGripAxis::Horizontal, |d| {
                    d.left_0().top(px(130.0)).w(px(200.0)).h(px(8.0))
                })
                .child(resize_grip(
                    self.theme,
                    100,
                    id,
                    axis,
                    self.resizing == Some(axis),
                    Some(self.theme.colors.stroke.subtle),
                ))
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(move |view, _, _, cx| {
                        view.resizing = Some(axis);
                        crate::press_gesture::claim_press(cx);
                        cx.notify();
                    }),
                )
        };
        let scrollbar = match self.axis {
            ScrollbarAxis::Vertical => Scrollbar::new("feedback_scrollbar", self.scroll.clone()),
            ScrollbarAxis::Horizontal => {
                Scrollbar::horizontal("feedback_scrollbar", self.scroll.clone())
            }
        };
        let content = div()
            .relative()
            .size_full()
            .child(release_listener)
            .child(
                div()
                    .relative()
                    .w(px(200.0))
                    .h(px(120.0))
                    .child(
                        div()
                            .id("feedback_scroll")
                            .size_full()
                            .overflow_scroll()
                            .track_scroll(&self.scroll)
                            .child(div().w(px(1200.0)).h(px(1200.0)).flex_none()),
                    )
                    .child(
                        scrollbar
                            .always_visible()
                            .debug_selector("feedback_scrollbar")
                            .render(self.theme),
                    ),
            )
            .child(grip("feedback_vertical_grip", ResizeGripAxis::Vertical, cx))
            .child(grip(
                "feedback_horizontal_grip",
                ResizeGripAxis::Horizontal,
                cx,
            ))
            .child(
                div()
                    .absolute()
                    .top(px(160.0))
                    .left_0()
                    .w(px(200.0))
                    .child(self.input.clone()),
            );
        crate::view::window_frame(
            self.theme,
            gpui::Decorations::Server,
            content.into_any_element(),
            None,
            100,
        )
    }
}

fn draw(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn painted_in(cx: &mut gpui::VisualTestContext, selector: &'static str, color: gpui::Rgba) -> bool {
    let bounds = cx.debug_bounds(selector).unwrap();
    cx.update(|window, _| {
        let scale = window.scale_factor();
        window.painted_quads().iter().any(|quad| {
            let max_radius = quad.bounds.size.width.0.min(quad.bounds.size.height.0) / 2.0;
            quad.background == color.into()
                && quad.bounds.size.width.0 > 0.0
                && quad.bounds.size.height.0 > 0.0
                // An oversized radius still produces a scene quad but the GPU
                // can render the entire grip transparent.
                && [
                    quad.corner_radii.top_left,
                    quad.corner_radii.top_right,
                    quad.corner_radii.bottom_left,
                    quad.corner_radii.bottom_right,
                ].into_iter().all(|radius| radius.0 <= max_radius + 0.5)
                && quad.bounds.left().0 >= f32::from(bounds.left()) * scale - 0.5
                && quad.bounds.right().0 <= f32::from(bounds.right()) * scale + 0.5
                && quad.bounds.top().0 >= f32::from(bounds.top()) * scale - 0.5
                && quad.bounds.bottom().0 <= f32::from(bounds.bottom()) * scale + 0.5
        })
    })
}

#[gpui::test]
fn scrollbar_drag_uses_the_accent_and_suppresses_other_resize_grips(cx: &mut gpui::TestAppContext) {
    for key in ["gitcomet_light", "gitcomet_dark", "amber_dark"] {
        for axis in [ScrollbarAxis::Vertical, ScrollbarAxis::Horizontal] {
            let theme = AppTheme::from_key(key).unwrap();
            let (view, cx) =
                cx.add_window_view(|window, cx| DragFeedbackView::new(theme, axis, window, cx));
            draw(cx);
            let vertical = cx.debug_bounds("feedback_vertical_grip").unwrap().center();
            let horizontal = cx
                .debug_bounds("feedback_horizontal_grip")
                .unwrap()
                .center();
            cx.simulate_mouse_move(vertical, None, Default::default());
            draw(cx);
            assert!(painted_in(
                cx,
                "feedback_vertical_grip",
                resize_grip_hover_tint(theme)
            ));

            let bar = cx.debug_bounds("feedback_scrollbar").unwrap();
            let start = match axis {
                ScrollbarAxis::Vertical => point(bar.right() - px(2.0), bar.top() + px(6.0)),
                ScrollbarAxis::Horizontal => point(bar.left() + px(6.0), bar.bottom() - px(2.0)),
            };
            cx.simulate_mouse_move(start, None, Default::default());
            draw(cx);
            assert!(painted_in(
                cx,
                "feedback_scrollbar",
                theme.colors.scrollbar.thumb_hover
            ));
            cx.simulate_mouse_down(start, MouseButton::Left, Default::default());
            draw(cx);
            assert!(painted_in(
                cx,
                "feedback_scrollbar",
                theme.colors.accent.foreground
            ));
            for (selector, target) in [
                ("feedback_vertical_grip", vertical),
                ("feedback_horizontal_grip", horizontal),
            ] {
                cx.simulate_mouse_move(target, Some(MouseButton::Left), Default::default());
                draw(cx);
                assert!(
                    painted_in(cx, "feedback_scrollbar", theme.colors.accent.foreground),
                    "{key}: {axis:?} scrollbar must keep its accent while dragged over {selector}",
                );
                assert!(!painted_in(cx, selector, resize_grip_hover_tint(theme)));
                assert!(
                    painted_in(cx, selector, theme.colors.stroke.subtle),
                    "permanent dividers remain visible"
                );
            }
            cx.simulate_mouse_up(horizontal, MouseButton::Left, Default::default());
            draw(cx);
            assert!(painted_in(
                cx,
                "feedback_scrollbar",
                theme.colors.scrollbar.thumb
            ));
            assert!(painted_in(
                cx,
                "feedback_horizontal_grip",
                resize_grip_hover_tint(theme)
            ));
            cx.update(|window, app| {
                assert!(!crate::press_gesture::pointer_is_down(window, app));
                assert!(
                    crate::press_gesture::is_press_claimed(app),
                    "release ownership must outlive pointer feedback"
                );
                let offset = view.read(app).scroll.offset();
                assert!(
                    if axis == ScrollbarAxis::Vertical {
                        offset.y
                    } else {
                        offset.x
                    } < px(0.0)
                );
            });

            // The grip being resized retains its own feedback outside its bounds.
            cx.simulate_mouse_down(horizontal, MouseButton::Left, Default::default());
            cx.simulate_mouse_move(vertical, Some(MouseButton::Left), Default::default());
            draw(cx);
            assert!(painted_in(
                cx,
                "feedback_horizontal_grip",
                theme.colors.accent.foreground
            ));
            assert!(!painted_in(
                cx,
                "feedback_vertical_grip",
                resize_grip_hover_tint(theme)
            ));
            cx.simulate_mouse_up(vertical, MouseButton::Left, Default::default());
            draw(cx);
            assert!(painted_in(
                cx,
                "feedback_vertical_grip",
                resize_grip_hover_tint(theme)
            ));
        }
    }
}

#[gpui::test]
fn resize_grips_suppress_text_and_gpui_drags_and_recover_after_a_missed_release(
    cx: &mut gpui::TestAppContext,
) {
    let theme = AppTheme::gitcomet_dark();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        DragFeedbackView::new(theme, ScrollbarAxis::Vertical, window, cx)
    });
    draw(cx);
    let target = cx.debug_bounds("feedback_vertical_grip").unwrap().center();
    let input_point = point(px(20.0), px(175.0));
    cx.simulate_mouse_down(input_point, MouseButton::Left, Default::default());
    cx.simulate_mouse_move(target, Some(MouseButton::Left), Default::default());
    draw(cx);
    assert!(!painted_in(
        cx,
        "feedback_vertical_grip",
        resize_grip_hover_tint(theme)
    ));
    // The OS can deliver a no-button move after a release outside the window.
    cx.simulate_mouse_move(target, None, Default::default());
    draw(cx);
    assert!(painted_in(
        cx,
        "feedback_vertical_grip",
        resize_grip_hover_tint(theme)
    ));

    // GPUI drag-and-drop can enter a window without a local mouse-down event.
    cx.update(|window, app| {
        let ghost = app.new(|_| gpui::Empty);
        app.start_drag(gpui::AnyDrag::new((), ghost));
        window.refresh();
    });
    draw(cx);
    assert!(!painted_in(
        cx,
        "feedback_vertical_grip",
        resize_grip_hover_tint(theme)
    ));
    cx.update(|window, app| {
        app.stop_active_drag(window);
    });
    draw(cx);
    assert!(painted_in(
        cx,
        "feedback_vertical_grip",
        resize_grip_hover_tint(theme)
    ));
}
