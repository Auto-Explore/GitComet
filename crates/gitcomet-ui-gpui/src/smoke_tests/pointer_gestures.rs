use super::*;

#[gpui::test]
fn release_outside_a_text_input_does_not_click_where_it_lands(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(PressGestureHostView::new);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let input = cx
        .debug_bounds("pg_input")
        .expect("expected the input bounds");
    let click_target = cx
        .debug_bounds("pg_click")
        .expect("expected the on_click target bounds");
    let release_target = cx
        .debug_bounds("pg_release")
        .expect("expected the mouse-up target bounds");

    // Positive control: both targets do fire for an ordinary click, so the
    // assertions below cannot pass just because the harness never reaches them.
    cx.simulate_mouse_move(click_target.center(), None, Modifiers::default());
    cx.simulate_click(click_target.center(), Modifiers::default());
    cx.simulate_mouse_move(release_target.center(), None, Modifiers::default());
    cx.simulate_click(release_target.center(), Modifiers::default());
    assert_eq!(
        press_gesture_hits(cx, &view),
        (1, 1),
        "expected a plain click on each target to register"
    );

    // Press in the input, drag across both targets, release on the far one.
    cx.simulate_mouse_move(input.center(), None, Modifiers::default());
    cx.simulate_mouse_down(input.center(), MouseButton::Left, Modifiers::default());
    cx.update(|_window, app| {
        assert!(
            crate::press_gesture::is_press_claimed(app),
            "the input should own the press while the button is held"
        );
    });
    cx.simulate_mouse_move(
        click_target.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_move(
        release_target.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        release_target.center(),
        MouseButton::Left,
        Modifiers::default(),
    );

    assert_eq!(
        press_gesture_hits(cx, &view),
        (1, 1),
        "a release that only drifted onto a target must not click it"
    );

    // The claim lasts exactly one press: the next one still works normally.
    cx.simulate_mouse_move(release_target.center(), None, Modifiers::default());
    cx.simulate_click(release_target.center(), Modifiers::default());
    assert_eq!(
        press_gesture_hits(cx, &view),
        (1, 2),
        "expected the next press to clear the claim"
    );
}

#[gpui::test]
fn a_press_under_an_occluding_overlay_still_clears_the_claim(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = cx.add_window_view(PressGestureHostView::new);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let input = cx
        .debug_bounds("pg_input")
        .expect("expected the input bounds");
    let inert = cx
        .debug_bounds("pg_inert")
        .expect("expected the inert overlay area bounds");

    cx.simulate_mouse_move(input.center(), None, Modifiers::default());
    cx.simulate_click(input.center(), Modifiers::default());
    cx.update(|_window, app| {
        assert!(
            crate::press_gesture::is_press_claimed(app),
            "the claim outlives the release it belongs to"
        );
    });

    // Both points sit inside an `occlude()`d overlay, so the window root is not
    // hovered. A hitbox-gated reset would never run here and the claim would
    // stick for the rest of the session.
    cx.simulate_mouse_move(inert.center(), None, Modifiers::default());
    cx.simulate_mouse_down(inert.center(), MouseButton::Left, Modifiers::default());
    cx.update(|_window, app| {
        assert!(
            !crate::press_gesture::is_press_claimed(app),
            "a press that claims nothing must leave the claim clear"
        );
    });
}

#[gpui::test]
fn a_button_less_pointer_move_clears_a_stranded_claim(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = cx.add_window_view(PressGestureHostView::new);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let input = cx
        .debug_bounds("pg_input")
        .expect("expected the input bounds");

    cx.simulate_mouse_move(input.center(), None, Modifiers::default());
    cx.simulate_mouse_down(input.center(), MouseButton::Left, Modifiers::default());
    cx.update(|_window, app| {
        assert!(crate::press_gesture::is_press_claimed(app));
    });

    // No release ever arrives — the pointer just moves with nothing held, which
    // only happens once the gesture is over.
    cx.simulate_mouse_move(input.center(), None, Modifiers::default());
    cx.update(|_window, app| {
        assert!(
            !crate::press_gesture::is_press_claimed(app),
            "a move with no button held must strand no claim"
        );
    });
}

fn press_gesture_hits(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<PressGestureHostView>,
) -> (usize, usize) {
    cx.update(|_window, app| {
        let this = view.read(app);
        (this.click_hits, this.release_hits)
    })
}

/// Hosts a text input beside two click targets — one on gpui's `on_click`, one
/// on a raw `on_mouse_up` guarded by [`crate::press_gesture`] — so a drag that
/// starts in the input and ends on a target can be replayed. The input sits
/// inside an `occlude()`d overlay, standing in for the centered prompts that
/// host inputs above a blocked hit test.
struct PressGestureHostView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
    click_hits: usize,
    release_hits: usize,
}

impl PressGestureHostView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        let input = cx.new(|cx| {
            components::TextInput::new(
                components::TextInputOptions {
                    placeholder: "Enter".into(),
                    ..Default::default()
                },
                window,
                cx,
            )
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
            click_hits: 0,
            release_hits: 0,
        }
    }
}

impl gpui::Render for PressGestureHostView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let content = div()
            .relative()
            .size_full()
            .child(
                div()
                    .id("pg_click")
                    .debug_selector(|| "pg_click".to_string())
                    .absolute()
                    .top(px(200.0))
                    .left(px(0.0))
                    .w(px(240.0))
                    .h(px(40.0))
                    .on_click(cx.listener(|this, _e: &gpui::ClickEvent, _w, cx| {
                        this.click_hits += 1;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .id("pg_release")
                    .debug_selector(|| "pg_release".to_string())
                    .absolute()
                    .top(px(260.0))
                    .left(px(0.0))
                    .w(px(240.0))
                    .h(px(40.0))
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, _e: &MouseUpEvent, _w, cx| {
                            if crate::press_gesture::is_press_claimed(cx) {
                                return;
                            }
                            this.release_hits += 1;
                            cx.notify();
                        }),
                    ),
            )
            .child(
                div()
                    .id("pg_overlay")
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .w(px(240.0))
                    .h(px(160.0))
                    .occlude()
                    .child(
                        div()
                            .id("pg_input")
                            .debug_selector(|| "pg_input".to_string())
                            .w(px(240.0))
                            .child(self.input.clone()),
                    )
                    .child(
                        div()
                            .id("pg_inert")
                            .debug_selector(|| "pg_inert".to_string())
                            .absolute()
                            .top(px(100.0))
                            .left(px(0.0))
                            .w(px(240.0))
                            .h(px(40.0)),
                    ),
            )
            .into_any_element();

        view::window_frame(
            self.theme,
            window.window_decorations(),
            content,
            None,
            ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
    }
}
