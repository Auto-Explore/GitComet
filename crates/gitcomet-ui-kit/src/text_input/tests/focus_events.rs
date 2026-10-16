use super::*;

#[gpui::test]
fn window_blur_stops_text_input_until_deliberately_refocused(cx: &mut gpui::TestAppContext) {
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        for multiline in [false, true] {
            let (input, cx) = cx.add_window_view(|window, cx| {
                window.activate();
                cx.observe_window_activation(window, |_input, window, cx| {
                    crate::window_focus::reset_on_deactivation(window, cx);
                })
                .detach();
                let options = TextInputOptions {
                    multiline,
                    ..Default::default()
                };
                // Both constructors must acquire blur listeners when rendered.
                if multiline {
                    TextInput::new_inert(options, cx)
                } else {
                    TextInput::new(options, window, cx)
                }
            });
            cx.update(|window, app| {
                input.update(app, |input, cx| {
                    input.set_text("alpha beta", cx);
                    window.focus(&input.focus_handle, cx);
                    input.replace_text_in_range(Some(10..10), "!", window, cx);
                });
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            cx.executor().advance_clock(Duration::from_millis(800));
            cx.run_until_parked();
            cx.update(|window, app| {
                input.update(app, |input, cx| {
                    assert!(input.interaction.cursor_blink_task.is_some());
                    assert!(
                        !input.interaction.cursor_blink_visible,
                        "the live blink ran"
                    );
                    input.set_selected_range(1..4, false, window, cx);
                    input.interaction.is_selecting = true;
                    input.interaction.mouse_selection_anchor = Some(1);
                    input.interaction.pending_mouse_selection_anchor =
                        Some(point(px(4.0), px(4.0)));
                });
            });
            let undo_len = cx.update(|_, app| input.read(app).selection.undo_stack.len());

            cx.deactivate_window();
            cx.update(|window, app| {
                assert!(window.focused(app).is_none());
                input.update(app, |input, cx| {
                    assert!(!input.interaction.has_focus);
                    assert!(input.interaction.cursor_blink_task.is_none());
                    assert!(!input.interaction.is_selecting);
                    assert!(input.interaction.mouse_selection_anchor.is_none());
                    assert!(input.interaction.pending_mouse_selection_anchor.is_none());
                    assert!(!input.accepts_text_input(window, cx));
                    assert_eq!(input.text(), "alpha beta!");
                    assert_eq!(input.selection.range, 1..4);
                    assert_eq!(input.selection.undo_stack.len(), undo_len);
                });
                window.activate();
            });
            cx.run_until_parked();
            crate::test_support::refresh_and_draw(cx);
            cx.simulate_keystrokes("x");
            cx.executor().advance_clock(Duration::from_secs(2));
            cx.run_until_parked();
            cx.update(|window, app| {
                assert!(window.focused(app).is_none());
                let input = input.read(app);
                assert_eq!(input.text(), "alpha beta!");
                assert_eq!(input.selection.range, 1..4);
                assert!(input.interaction.cursor_blink_task.is_none());
            });

            // A delivered click reactivates and places the caret in one gesture.
            let position = cx.update(|_, app| input.read(app).layout.bounds.unwrap().center());
            cx.simulate_mouse_move(position, None, gpui::Modifiers::default());
            cx.simulate_click(position, gpui::Modifiers::default());
            crate::test_support::refresh_and_draw(cx);
            cx.update(|window, app| {
                let input = input.read(app);
                assert!(crate::window_focus::is_active(&input.focus_handle, window));
                assert!(input.interaction.cursor_blink_visible);
                assert!(input.interaction.cursor_blink_task.is_some());
                assert!(input.selection.range.is_empty());
            });
            cx.simulate_keystrokes("z");
            cx.update(|_, app| assert!(input.read(app).text().contains('z')));
            // Stop the live task before leaving this test window behind.
            cx.deactivate_window();
        }
    });
}

#[gpui::test]
fn idle_caret_stops_blinking_until_the_next_edit(cx: &mut gpui::TestAppContext) {
    use super::render::{CURSOR_BLINK_INTERVAL, CURSOR_BLINK_TIMEOUT};
    crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
        let (input, cx) = cx.add_window_view(|window, cx| {
            window.activate();
            TextInput::new(TextInputOptions::default(), window, cx)
        });
        cx.update(|window, app| {
            input.update(app, |input, cx| window.focus(&input.focus_handle, cx));
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        cx.executor()
            .advance_clock(CURSOR_BLINK_TIMEOUT - CURSOR_BLINK_INTERVAL);
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(input.read(app).interaction.cursor_blink_task.is_some());
        });

        // Each blink redraws the window, so an idle caret rests, shown.
        cx.executor().advance_clock(CURSOR_BLINK_INTERVAL * 2);
        cx.run_until_parked();
        crate::test_support::refresh_and_draw(cx);
        cx.update(|_, app| {
            let input = input.read(app);
            assert!(input.interaction.cursor_blink_task.is_none());
            assert!(input.interaction.cursor_blink_visible);
        });

        cx.simulate_keystrokes("a");
        crate::test_support::refresh_and_draw(cx);
        cx.executor().advance_clock(CURSOR_BLINK_INTERVAL);
        cx.run_until_parked();
        cx.update(|_, app| {
            let input = input.read(app);
            assert_eq!(input.text(), "a");
            assert!(input.interaction.cursor_blink_task.is_some());
            assert!(
                !input.interaction.cursor_blink_visible,
                "typing blinks again"
            );
        });
        // Stop the live task before leaving this test window behind.
        cx.deactivate_window();
    });
}

#[gpui::test]
fn control_blur_cancels_caret_and_drag_without_losing_selection(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, _| window.activate());
    cx.run_until_parked();
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha beta", cx);
            window.focus(&input.focus_handle, cx);
            input.set_selected_range(2..5, false, window, cx);
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        input.update(app, |input, _| {
            input.interaction.is_selecting = true;
            input.interaction.mouse_selection_anchor = Some(2);
        });
        window.blur(app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|_, app| {
        let input = input.read(app);
        assert!(!input.interaction.has_focus);
        assert!(!input.interaction.is_selecting);
        assert_eq!(input.selection.range, 2..5);
    });
}

#[gpui::test]
fn content_events_exclude_focus_selection_and_identical_replacements(
    cx: &mut gpui::TestAppContext,
) {
    use std::sync::Mutex;
    let (input, cx) = multiline_input(cx);
    let changes = Arc::new(Mutex::new(Vec::new()));
    let sink = changes.clone();
    let _subscription = cx.update(|_, app| {
        app.subscribe(&input, move |_, event: &TextInputChanged, _| {
            sink.lock().unwrap().push(*event);
        })
    });
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("hello", cx);
            input.set_selected_range(1..3, false, window, cx);
            window.focus(&input.focus_handle(), cx);
            cx.notify();
            input.replace_utf8_range(0..5, "hello", cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(changes.lock().unwrap().len(), 1);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.replace_text_in_range(Some(0..5), "world", window, cx);
            input.undo(&Undo, window, cx);
            input.redo(&Redo, window, cx);
            input.replace_and_mark_text_in_range(Some(0..5), "猫", Some(0..1), window, cx);
        })
    });
    cx.run_until_parked();
    let changes = changes.lock().unwrap();
    assert_eq!(
        changes.len(),
        5,
        "typing/paste, undo, redo and IME each emit one content change"
    );
    assert!(changes.windows(2).all(|events| events[0] != events[1]));
}

#[gpui::test]
fn escape_key_flags_escape_pressed(cx: &mut gpui::TestAppContext) {
    let (input, cx) =
        cx.add_window_view(|window, cx| TextInput::new(TextInputOptions::default(), window, cx));

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            assert!(
                !input.take_escape_pressed(),
                "escape should not be flagged before the key is pressed"
            );

            let keystroke = gpui::Keystroke::parse("escape")
                .expect("valid keystroke")
                .with_simulated_ime();
            let event = gpui::KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            };
            input.on_key_down(&event, _window, cx);

            assert!(
                input.take_escape_pressed(),
                "escape key should flag escape_pressed"
            );
            assert!(
                !input.take_escape_pressed(),
                "take_escape_pressed should consume the flag"
            );
        });
    });
}

#[gpui::test]
fn modified_escape_keystroke_does_not_flag_escape_pressed(cx: &mut gpui::TestAppContext) {
    let (input, cx) =
        cx.add_window_view(|window, cx| TextInput::new(TextInputOptions::default(), window, cx));

    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            let mut keystroke = gpui::Keystroke::parse("ctrl-escape")
                .expect("valid keystroke")
                .with_simulated_ime();
            keystroke.modifiers.control = true;
            let event = gpui::KeyDownEvent {
                keystroke,
                is_held: false,
                prefer_character_input: false,
            };
            input.on_key_down(&event, _window, cx);

            assert!(
                !input.take_escape_pressed(),
                "modified escape should not flag escape_pressed"
            );
        });
    });
}

#[gpui::test]
fn single_line_enter_flags_enter_pressed(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        TextInput::new(
            TextInputOptions {
                multiline: false,
                ..Default::default()
            },
            window,
            cx,
        )
    });

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            assert!(
                !input.take_enter_pressed(),
                "enter should not be flagged before the key is pressed"
            );

            input.enter(&Enter, window, cx);

            assert!(
                input.take_enter_pressed(),
                "enter key should flag enter_pressed in single-line inputs"
            );
            assert!(
                !input.take_enter_pressed(),
                "take_enter_pressed should consume the flag"
            );
        });
    });
}
