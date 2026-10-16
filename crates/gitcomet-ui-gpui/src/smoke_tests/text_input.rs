use super::*;

#[gpui::test]
fn text_input_constructs_without_panicking(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        cx.open_window(Default::default(), |window, cx| {
            cx.new(|cx| {
                components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Commit message".into(),
                        ..Default::default()
                    },
                    window,
                    cx,
                )
            })
        })
        .unwrap();
    });
}

#[gpui::test]
fn text_input_focus_after_initial_draw_accepts_typed_input(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(TextInputHostView::new);

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    cx.simulate_input("x");

    let text = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app).input.read(app).text().to_string()
    });
    assert_eq!(text, "x");
}

#[gpui::test]
fn text_input_supports_basic_clipboard_and_word_shortcuts(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("ctrl-a", crate::kit::SelectAll, Some("TextInput")),
            KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
            KeyBinding::new("ctrl-x", crate::kit::Cut, Some("TextInput")),
            KeyBinding::new("ctrl-v", crate::kit::Paste, Some("TextInput")),
            KeyBinding::new("ctrl-left", crate::kit::WordLeft, Some("TextInput")),
            KeyBinding::new(
                "ctrl-backspace",
                crate::kit::DeleteWordLeft,
                Some("TextInput"),
            ),
            KeyBinding::new(
                "ctrl-delete",
                crate::kit::DeleteWordRight,
                Some("TextInput"),
            ),
            KeyBinding::new(
                "ctrl-shift-left",
                crate::kit::SelectWordLeft,
                Some("TextInput"),
            ),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });
    });

    cx.simulate_keystrokes("ctrl-a ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("hello world".into())
    );

    cx.simulate_keystrokes("ctrl-x");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "");

    cx.write_to_clipboard(ClipboardItem::new_string("abc".to_string()));
    cx.simulate_keystrokes("ctrl-v");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "abc");

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });
    });
    cx.simulate_keystrokes("ctrl-shift-left ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("world".into())
    );

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello brave world", cx));
        });
    });

    cx.simulate_keystrokes("ctrl-backspace");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello brave ");

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello brave world", cx));
        });
    });

    cx.simulate_keystrokes("ctrl-left ctrl-delete");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello brave ");
}

#[gpui::test]
fn text_input_cmd_backspace_and_cmd_delete_delete_to_the_line_edges(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);
    let set_text = |cx: &mut gpui::VisualTestContext, text: &'static str| {
        cx.update(|window, app| {
            let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
            window.focus(&focus, app);
            view.update(app, |this, cx| {
                this.input.update(cx, |input, cx| input.set_text(text, cx));
            });
        });
    };
    let text = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).input.read(app).text().to_string())
    };

    cx.update(|_window, app| {
        app.bind_keys([
            KeyBinding::new("alt-left", crate::kit::WordLeft, Some("TextInput")),
            KeyBinding::new("cmd-left", crate::kit::Home, Some("TextInput")),
            KeyBinding::new(
                "cmd-backspace",
                crate::kit::DeleteToLineStart,
                Some("TextInput"),
            ),
            KeyBinding::new("cmd-delete", crate::kit::DeleteToLineEnd, Some("TextInput")),
            KeyBinding::new(
                "ctrl-shift-backspace",
                crate::kit::DeleteToLineStart,
                Some("TextInput"),
            ),
            KeyBinding::new(
                "ctrl-shift-delete",
                crate::kit::DeleteToLineEnd,
                Some("TextInput"),
            ),
        ]);
    });

    // With the caret at the end, the usual case in a search box, the whole
    // query goes.
    set_text(cx, "hello brave world");
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(text(cx), "");

    // Mid-line, each deletes only its own side of the caret.
    set_text(cx, "hello brave world");
    cx.simulate_keystrokes("alt-left cmd-backspace");
    assert_eq!(text(cx), "world");
    set_text(cx, "hello brave world");
    cx.simulate_keystrokes("alt-left cmd-delete");
    assert_eq!(text(cx), "hello brave ");

    // The Windows/Linux chords do the same.
    set_text(cx, "hello brave world");
    cx.simulate_keystrokes("alt-left ctrl-shift-backspace");
    assert_eq!(text(cx), "world");
    cx.simulate_keystrokes("ctrl-shift-delete");
    assert_eq!(text(cx), "");

    // With nothing on that side there is nothing to delete.
    set_text(cx, "hello");
    cx.simulate_keystrokes("cmd-left cmd-backspace");
    assert_eq!(text(cx), "hello");
    cx.simulate_keystrokes("cmd-delete");
    assert_eq!(text(cx), "");
}

#[gpui::test]
fn text_input_cmd_backspace_joins_lines_at_the_start_of_a_row(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(TextInputCursorScrollView::new);
    cx.update(|window, app| {
        app.bind_keys([KeyBinding::new(
            "cmd-backspace",
            crate::kit::DeleteToLineStart,
            Some("TextInput"),
        )]);
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("first\nsecond", cx));
        });
        let _ = window.draw(app);
    });
    let text = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).input.read(app).text().to_string())
    };

    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(
        text(cx),
        "first\n",
        "deletes back to the start of its own row"
    );
    cx.simulate_keystrokes("cmd-backspace");
    assert_eq!(
        text(cx),
        "first",
        "at a row start, joins with the row above"
    );
}

#[gpui::test]
fn text_input_shift_backspace_deletes_like_backspace(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("backspace", crate::kit::Backspace, Some("TextInput")),
            KeyBinding::new("shift-backspace", crate::kit::Backspace, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello", cx));
        });
    });

    cx.simulate_keystrokes("backspace");
    let plain_backspace =
        cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(plain_backspace, "hell");

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello", cx));
        });
    });

    cx.simulate_keystrokes("shift-backspace");
    let shift_backspace =
        cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(shift_backspace, plain_backspace);
}

#[gpui::test]
fn multiline_text_input_cursor_navigation_keeps_scroll_in_view(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(TextInputCursorScrollView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("enter", crate::kit::Enter, Some("TextInput")),
            KeyBinding::new("up", crate::kit::Up, Some("TextInput")),
            KeyBinding::new("down", crate::kit::Down, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("line".to_string(), cx));
        });

        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("enter enter enter enter enter enter enter enter enter enter");
    cx.run_until_parked();
    let (after_enter, max_after_enter) = cx.update(|window, app| {
        let _ = window.draw(app);
        let v = view.read(app);
        (
            abs_scroll_y(v.scroll_handle.offset().y),
            v.scroll_handle.max_offset().y,
        )
    });
    assert!(
        after_enter > px(0.0),
        "expected Enter to move cursor down and auto-scroll to keep it visible"
    );
    assert!(
        max_after_enter <= px(0.0) || after_enter >= max_after_enter - px(1.0),
        "expected Enter at EOF to keep scroll pinned to bottom"
    );

    cx.simulate_keystrokes("up up up up up up up up up up");
    cx.run_until_parked();
    let after_up = cx.update(|window, app| {
        let _ = window.draw(app);
        abs_scroll_y(view.read(app).scroll_handle.offset().y)
    });
    assert!(
        after_up < after_enter,
        "expected Up navigation to scroll back upward with cursor"
    );

    cx.simulate_keystrokes("down down down down down down down down down down");
    cx.run_until_parked();
    let after_down = cx.update(|window, app| {
        let _ = window.draw(app);
        abs_scroll_y(view.read(app).scroll_handle.offset().y)
    });
    assert!(
        after_down > after_up,
        "expected Down navigation to scroll downward with cursor"
    );
}

#[gpui::test]
fn multiline_text_input_mousewheel_does_not_trigger_cursor_autoscroll(
    cx: &mut gpui::TestAppContext,
) {
    let (view, cx) = cx.add_window_view(TextInputCursorScrollView::new);

    cx.update(|window, app| {
        app.bind_keys([KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        let long_text = (0..40)
            .map(|ix| format!("line {ix}"))
            .collect::<Vec<_>>()
            .join("\n");
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text(long_text, cx));
        });

        let _ = window.draw(app);
    });

    // Set a deterministic non-bottom scroll position before wheel input.
    let (before_wheel, max_offset) = cx.update(|window, app| {
        let _ = window.draw(app);
        let (scroll_handle, max_offset) = {
            let v = view.read(app);
            (
                v.scroll_handle.clone(),
                v.scroll_handle.max_offset().y.max(px(0.0)),
            )
        };
        let baseline = (max_offset * 0.5).max(px(1.0));
        scroll_handle.set_offset(gpui::point(px(0.0), -baseline.min(max_offset)));
        let _ = window.draw(app);
        (abs_scroll_y(scroll_handle.offset().y), max_offset)
    });
    assert!(
        max_offset > px(0.0),
        "expected multiline content to overflow"
    );
    assert!(
        before_wheel > px(0.0) && before_wheel < max_offset,
        "expected baseline scroll offset to be between top and bottom"
    );

    let surface_bounds = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app).scroll_handle.bounds()
    });
    cx.simulate_event(ScrollWheelEvent {
        position: surface_bounds.center(),
        delta: ScrollDelta::Pixels(gpui::point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    cx.run_until_parked();

    let after_wheel = cx.update(|window, app| {
        let _ = window.draw(app);
        let v = view.read(app);
        abs_scroll_y(v.scroll_handle.offset().y)
    });

    let wheel_delta = if after_wheel >= before_wheel {
        after_wheel - before_wheel
    } else {
        before_wheel - after_wheel
    };
    assert!(
        wheel_delta > px(0.5),
        "expected mousewheel to move scroll (before={before_wheel:?}, after={after_wheel:?})"
    );
    assert!(
        after_wheel < max_offset - px(1.0),
        "expected mousewheel not to snap back to bottom (after={after_wheel:?}, max={max_offset:?})"
    );
}

#[gpui::test]
fn text_input_right_click_context_menu_supports_copy(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });

        let _ = window.draw(app);
    });

    cx.write_to_clipboard(ClipboardItem::new_string("initial".to_string()));

    let bounds = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds");
    let click = bounds.center();

    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("initial".into())
    );

    let select_all_bounds = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input select-all context menu row");
    let select_all_click = select_all_bounds.center();

    cx.simulate_mouse_move(select_all_click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: select_all_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: select_all_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    // Open menu again while full selection is active, then copy from the menu.
    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });

    let copy_bounds = cx
        .debug_bounds("text_input_context_copy")
        .expect("expected text-input copy context menu row");
    let copy_click = copy_bounds.center();

    cx.simulate_mouse_move(copy_click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: copy_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: copy_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("hello world".into())
    );
}

#[gpui::test]
fn text_input_context_menu_does_not_resize_input_container(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = cx.add_window_view(TextInputHostView::new);

    let before = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds before opening context menu");
    let click = before.center();

    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });

    crate::test_support::refresh_and_draw(cx);

    let _ = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input context menu to be open");

    let after = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds after opening context menu");
    let width_delta = (f32::from(after.size.width) - f32::from(before.size.width)).abs();
    let height_delta = (f32::from(after.size.height) - f32::from(before.size.height)).abs();
    assert!(
        width_delta <= 0.1 && height_delta <= 0.1,
        "expected input bounds to stay stable when context menu opens; before=({}, {}) after=({}, {})",
        f32::from(before.size.width),
        f32::from(before.size.height),
        f32::from(after.size.width),
        f32::from(after.size.height)
    );
}

#[gpui::test]
fn text_input_context_menu_grows_wider_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });

        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds");
    let click = bounds.center();

    open_text_input_context_menu(cx, click);

    let default_row_width: f32 = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input context menu row before zooming")
        .size
        .width
        .into();

    cx.update(|window, app| {
        view.update(app, |_this, cx| {
            crate::ui_scale::set_default(cx, 200);
        });
        crate::ui_scale::apply_to_window(window, 200);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let zoomed_row_width: f32 = cx
        .debug_bounds("text_input_context_select_all")
        .expect("expected text-input context menu row after zooming")
        .size
        .width
        .into();

    assert!(
        zoomed_row_width > default_row_width * 1.6,
        "expected the text-input context menu to grow substantially with zoom (default={default_row_width}, zoomed={zoomed_row_width})"
    );
}

#[gpui::test]
fn text_input_supports_ctrl_z_undo(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("ctrl-v", crate::kit::Paste, Some("TextInput")),
            KeyBinding::new("ctrl-z", crate::kit::Undo, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello", cx));
        });

        let _ = window.draw(app);
    });

    cx.write_to_clipboard(ClipboardItem::new_string(" world".to_string()));
    cx.simulate_keystrokes("ctrl-v");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello world");

    cx.simulate_keystrokes("ctrl-z");
    let text = cx.update(|_window, app| view.read(app).input.read(app).text().to_string());
    assert_eq!(text, "hello");
}

#[gpui::test]
fn text_input_double_click_selects_word(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("alpha beta", cx));
        });

        let _ = window.draw(app);
    });

    let bounds = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds");
    let click = cx.update(|_window, app| {
        let input = view.read(app).input.clone();
        (0..200usize)
            .find_map(|step| {
                let pos = gpui::point(bounds.left() + px(8.0 + step as f32), bounds.center().y);
                let offset = input.read(app).offset_for_position(pos);
                (2..=4).contains(&offset).then_some(pos)
            })
            .unwrap_or_else(|| bounds.center())
    });

    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 2,
    });

    let selection = cx.update(|_window, app| view.read(app).input.read(app).selected_text());
    assert_eq!(selection, Some("alpha".into()));
}

#[gpui::test]
fn text_input_supports_shift_home_end_row_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
            KeyBinding::new("shift-home", crate::kit::SelectHome, Some("TextInput")),
            KeyBinding::new("shift-end", crate::kit::SelectEnd, Some("TextInput")),
            KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("abcde\n12345", cx));
        });

        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("left left shift-home ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("123".into())
    );

    cx.simulate_keystrokes("right shift-end ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("45".into())
    );
}

#[gpui::test]
fn text_input_supports_shift_pageup_pagedown_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("home", crate::kit::Home, Some("TextInput")),
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
            KeyBinding::new("shift-pageup", crate::kit::SelectPageUp, Some("TextInput")),
            KeyBinding::new(
                "shift-pagedown",
                crate::kit::SelectPageDown,
                Some("TextInput"),
            ),
            KeyBinding::new("ctrl-c", crate::kit::Copy, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("abcde\n12345\nxyz", cx));
        });

        let _ = window.draw(app);
    });

    // Move the cursor to the start of the second line.
    cx.simulate_keystrokes("home left home");

    cx.simulate_keystrokes("shift-pageup ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("abcde\n".into())
    );

    // Collapse selection back to the start of the second line.
    cx.simulate_keystrokes("right");

    cx.simulate_keystrokes("shift-pagedown ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("12345\n".into())
    );
}

#[gpui::test]
fn text_input_supports_up_down_with_sticky_column(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("up", crate::kit::Up, Some("TextInput")),
            KeyBinding::new("down", crate::kit::Down, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("aaaaa\nbb\nccccc", cx));
        });

        let _ = window.draw(app);
    });

    // Cursor starts at EOF (offset 14). Move to column 4 on the third line.
    cx.simulate_keystrokes("left");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 13);

    // Move up onto shorter middle line, then keep sticky column when moving again.
    cx.simulate_keystrokes("up");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 8);

    cx.simulate_keystrokes("up");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 4);

    cx.simulate_keystrokes("down down");
    let offset = cx.update(|_window, app| view.read(app).input.read(app).cursor_offset());
    assert_eq!(offset, 13);
}

#[gpui::test]
fn text_input_supports_shift_up_down_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        app.bind_keys([
            KeyBinding::new("home", crate::kit::Home, Some("TextInput")),
            KeyBinding::new("left", crate::kit::Left, Some("TextInput")),
            KeyBinding::new("right", crate::kit::Right, Some("TextInput")),
        ]);

        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);

        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("abcde\n12345\nxyz", cx));
        });

        let _ = window.draw(app);
    });

    // Move the cursor to the start of the second line.
    cx.simulate_keystrokes("home left home");
    cx.dispatch_action(crate::kit::SelectUp);
    let selection = cx.update(|_window, app| view.read(app).input.read(app).selected_text());
    assert_eq!(selection, Some("abcde\n".into()));

    // Collapse selection to the start of the second line.
    cx.simulate_keystrokes("right");
    cx.dispatch_action(crate::kit::SelectDown);
    let selection = cx.update(|_window, app| view.read(app).input.read(app).selected_text());
    assert_eq!(selection, Some("12345\n".into()));
}

/// The menu's rows clear `interaction.context_menu` and stop propagation in the
/// bubble phase, so a guard that re-reads it after the press sees `None` and
/// blurs the input it was meant to protect.
#[gpui::test]
fn text_input_context_menu_copy_keeps_focus_and_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SmokeView::new);

    cx.update(|window, app| {
        let focus = view.update(app, |this, cx| this.input.read(cx).focus_handle());
        window.focus(&focus, app);
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.set_text("hello world", cx));
        });
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.run_until_parked();

    let click = cx
        .debug_bounds("smoke_input")
        .expect("expected smoke input bounds")
        .center();
    cx.simulate_mouse_move(click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: click,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });
    cx.run_until_parked();

    let copy_click = cx
        .debug_bounds("text_input_context_copy")
        .expect("expected the copy row")
        .center();
    cx.simulate_mouse_move(copy_click, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: copy_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: copy_click,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("hello world".into()),
        "precondition: the menu's Copy row ran"
    );
    cx.update(|window, app| {
        let input = view.read(app).input.clone();
        assert!(
            input.read(app).focus_handle().is_focused(window),
            "using the input's own menu must not blur it"
        );
        assert_eq!(
            input.read(app).selected_text(),
            Some("hello world".to_string()),
            "the selection the menu just copied must survive"
        );
    });
}

fn abs_scroll_y(raw: Pixels) -> Pixels {
    if raw < px(0.0) { -raw } else { raw }
}

fn open_text_input_context_menu(cx: &mut gpui::VisualTestContext, position: gpui::Point<Pixels>) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position,
        modifiers: Modifiers::default(),
        button: MouseButton::Right,
        click_count: 1,
    });
}

struct TextInputHostView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
}

struct TextInputCursorScrollView {
    theme: AppTheme,
    input: gpui::Entity<components::TextInput>,
    scroll_handle: ScrollHandle,
}

impl TextInputCursorScrollView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        window.activate();
        let scroll_handle = ScrollHandle::new();
        let input = cx.new({
            let scroll_handle = scroll_handle.clone();
            move |cx| {
                let mut input = components::TextInput::new(
                    components::TextInputOptions {
                        placeholder: "Enter".into(),
                        multiline: true,
                        soft_wrap: true,
                        ..Default::default()
                    },
                    window,
                    cx,
                );
                input.set_vertical_scroll_handle(Some(scroll_handle.clone()));
                input
            }
        });

        Self {
            theme: AppTheme::gitcomet_dark(),
            input,
            scroll_handle,
        }
    }
}

impl gpui::Render for TextInputCursorScrollView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let theme = self.theme;
        let content = div()
            .flex()
            .flex_col()
            .p_2()
            .child(
                div()
                    .id("cursor_scroll_surface")
                    .relative()
                    .w(px(280.0))
                    .h(px(100.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll_handle)
                    .child(self.input.clone())
                    .child(
                        components::Scrollbar::new("cursor_scrollbar", self.scroll_handle.clone())
                            .render(theme),
                    ),
            )
            .into_any_element();

        view::window_frame(
            theme,
            window.window_decorations(),
            content,
            None,
            ui_scale::DEFAULT_UI_SCALE_PERCENT,
        )
    }
}

impl TextInputHostView {
    fn new(window: &mut gpui::Window, cx: &mut gpui::Context<Self>) -> Self {
        window.activate();
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
        }
    }
}

impl gpui::Render for TextInputHostView {
    fn render(
        &mut self,
        window: &mut gpui::Window,
        _cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let content = div()
            .flex()
            .flex_col()
            .p_2()
            .child(
                div()
                    .id("smoke_input")
                    .debug_selector(|| "smoke_input".to_string())
                    .child(self.input.clone()),
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
