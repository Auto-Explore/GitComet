//! Focus policy inside the application's window frame.

use crate::kit::{TextInput, TextInputOptions};
use crate::test_support::refresh_and_draw;
use gitcomet_ui_kit::window_focus::*;
use gpui::prelude::*;
use gpui::{Context, FocusHandle, Subscription, Window};
use gpui::{Entity, div, px};

struct FocusFixture {
    inputs: [Entity<TextInput>; 2],
    between_inputs: FocusHandle,
    _activation: Subscription,
}

impl FocusFixture {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        window.activate();
        Self {
            inputs: std::array::from_fn(|_| {
                cx.new(|cx| TextInput::new(TextInputOptions::default(), window, cx))
            }),
            between_inputs: cx.focus_handle().tab_index(0).tab_stop(true),
            _activation: cx.observe_window_activation(window, |_, window, cx| {
                reset_on_deactivation(window, cx);
            }),
        }
    }
}

impl Render for FocusFixture {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        crate::view::window_frame(
            crate::theme::AppTheme::gitcomet_dark(),
            window.window_decorations(),
            div()
                .flex()
                .flex_col()
                .gap(px(8.0))
                .child(self.inputs[0].clone())
                .child(
                    div()
                        .track_focus(&self.between_inputs)
                        .child("Focusable control"),
                )
                .child(self.inputs[1].clone())
                .into_any_element(),
            None,
            crate::ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
    }
}

#[gpui::test]
fn tab_can_start_focus_after_window_blur(cx: &mut gpui::TestAppContext) {
    let _tab_navigation = cx.update(observe_tab_navigation);
    let (view, cx) = cx.add_window_view(FocusFixture::new);
    refresh_and_draw(cx);
    for (key, expected) in [("tab", 0), ("shift-tab", 1)] {
        cx.deactivate_window();
        cx.update(|window, _| window.activate());
        cx.run_until_parked();
        refresh_and_draw(cx);
        cx.simulate_keystrokes(key);
        cx.update(|window, app| {
            assert!(is_active(
                &view.read(app).inputs[expected].read(app).focus_handle(),
                window
            ));
        });
        // A Tab that already has an owner must not be stolen by the fallback.
        cx.simulate_keystrokes(key);
        cx.update(|window, app| {
            assert!(is_active(
                &view.read(app).inputs[expected].read(app).focus_handle(),
                window
            ));
        });
    }
    // Navigation must also pass through non-text controls on the way back
    // to an input; the first tab stop in a real window may be a button.
    for (key, expected) in [("tab", 1), ("shift-tab", 0)] {
        cx.update(|window, app| {
            let focus = view.read(app).between_inputs.clone();
            window.focus(&focus, app);
        });
        refresh_and_draw(cx);
        cx.simulate_keystrokes(key);
        cx.update(|window, app| {
            assert!(is_active(
                &view.read(app).inputs[expected].read(app).focus_handle(),
                window
            ));
        });
    }
}

#[gpui::test]
fn switching_windows_resets_only_the_window_losing_focus(cx: &mut gpui::TestAppContext) {
    let (first, first_cx) = cx.add_window_view(FocusFixture::new);
    let first_window = first_cx.update(|window, app| {
        window.focus(&first.read(app).inputs[0].read(app).focus_handle(), app);
        let _ = window.draw(app);
        window.window_handle()
    });
    let (second, second_cx) = first_cx.add_window_view(FocusFixture::new);
    second_cx.update(|window, app| {
        window.activate();
        window.focus(&second.read(app).inputs[0].read(app).focus_handle(), app);
        let _ = window.draw(app);
    });
    second_cx.run_until_parked();
    first_window
        .update(second_cx, |_, window, app| {
            assert!(!window.is_window_active());
            assert!(window.focused(app).is_none());
            window.activate();
        })
        .unwrap();
    second_cx.run_until_parked();
    second_cx.update(|window, app| assert!(window.focused(app).is_none()));
    first_window
        .update(second_cx, |_, window, app| {
            assert!(window.is_window_active());
            assert!(
                window.focused(app).is_none(),
                "activation alone never restores focus"
            );
        })
        .unwrap();
}
