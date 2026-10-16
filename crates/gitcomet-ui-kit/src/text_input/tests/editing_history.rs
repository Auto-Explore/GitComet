use super::*;

#[test]
fn detected_line_endings_prefer_file_contents_over_platform_default() {
    for (content, expected) in [
        ("first\r\nsecond", "\r\n"),
        ("first\nsecond", "\n"),
        ("first\r\nsecond\nthird", "\r\n"),
        ("no newline", if cfg!(windows) { "\r\n" } else { "\n" }),
    ] {
        assert_eq!(TextInput::detect_line_ending(content), expected);
    }
}

#[gpui::test]
fn multiline_shift_enter_inserts_a_line_break(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let expected = format!("alpha{}", input.line_ending);

            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), expected);
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should insert a newline instead of flagging enter-pressed"
            );
        });
    });
}

#[gpui::test]
fn multiline_submit_on_enter_keeps_shift_enter_as_line_break(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_submit_on_enter(true);
            input.set_text("alpha", cx);

            input.enter(&Enter, window, cx);

            assert_eq!(input.text(), "alpha");
            assert!(
                input.take_enter_pressed(),
                "enter should submit multiline inputs when submit-on-enter is enabled"
            );

            let expected = format!("alpha{}", input.line_ending);
            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), expected);
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should still insert a newline"
            );
        });
    });
}

#[gpui::test]
fn single_line_shift_enter_is_a_noop(cx: &mut gpui::TestAppContext) {
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
            input.set_text("alpha", cx);

            input.shift_enter(&ShiftEnter, window, cx);

            assert_eq!(input.text(), "alpha");
            assert!(
                !input.take_enter_pressed(),
                "shift-enter should not submit or modify single-line inputs"
            );
        });
    });
}

#[gpui::test]
fn redo_restores_text_after_undo(cx: &mut gpui::TestAppContext) {
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
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);
            assert_eq!(input.text(), "beta");

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "beta");
            assert!(input.selection.redo_stack.is_empty());
        });
    });
}

#[gpui::test]
fn edit_delta_queue_retains_multiple_edits_and_undo_redo(cx: &mut gpui::TestAppContext) {
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
            input.set_text("abcd", cx);
            input.replace_utf8_range(1..2, "XX", cx);
            input.replace_utf8_range(3..4, "", cx);
            assert_eq!(input.text(), "aXXd");
            assert_eq!(
                input.drain_recent_utf8_edit_deltas(),
                vec![(1..2, 1..3), (3..4, 3..3)],
            );

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "aXXcd");
            assert_eq!(input.drain_recent_utf8_edit_deltas(), vec![(3..3, 3..4)],);

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "aXXd");
            assert_eq!(input.drain_recent_utf8_edit_deltas(), vec![(3..4, 3..3)],);
        });
    });
}

#[gpui::test]
fn redo_is_cleared_by_a_new_edit_after_undo(cx: &mut gpui::TestAppContext) {
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
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);

            let inserted = input.replace_utf8_range(0..5, "gamma", cx);
            assert_eq!(inserted, 0..5);
            assert_eq!(input.text(), "gamma");
            assert!(input.selection.redo_stack.is_empty());

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "gamma");
            assert!(input.selection.redo_stack.is_empty());
        });
    });
}

#[gpui::test]
fn redo_is_noop_when_input_is_read_only(cx: &mut gpui::TestAppContext) {
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
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);

            input.undo(&Undo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);

            input.set_read_only(true, cx);
            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "alpha");
            assert_eq!(input.selection.redo_stack.len(), 1);
        });
    });
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)] // One protected range.
fn rejected_composition_preserves_content_history_and_protected_ranges(
    cx: &mut gpui::TestAppContext,
) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("old\nlocked\n", cx);
            input.replace_text_in_range(Some(0..3), "new", window, cx);
            input.undo(&Undo, window, cx);
            input.drain_recent_utf8_edit_deltas();
            input.set_protected_ranges(Arc::from([4..11]));

            input.replace_and_mark_text_in_range(Some(4..10), "猫", Some(0..1), window, cx);
            assert_eq!(input.text(), "old\nlocked\n");
            assert_eq!(input.protected_ranges(), &[4..11]);
            assert_eq!(input.marked_text_range(window, cx), None);
            assert!(input.drain_recent_utf8_edit_deltas().is_empty());

            input.redo(&Redo, window, cx);
            assert_eq!(input.text(), "new\nlocked\n");
        });
    });
}

#[gpui::test]
#[allow(clippy::single_range_in_vec_init)] // One protected range.
fn programmatic_replacement_can_update_protected_content_without_moving_the_view(
    cx: &mut gpui::TestAppContext,
) {
    let (input, cx) = multiline_input(cx);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("old\nlocked\ntail", cx);
            input.set_protected_ranges(Arc::from([4..11]));
            input.set_selected_range(11..15, false, window, cx);
            input.layout.scroll_x = px(24.0);
            input.interaction.pending_cursor_autoscroll = false;
            let inserted = input.replace_utf8_range_preserving_view(4..10, "resolved", cx);
            assert_eq!(inserted, 4..12);
            assert_eq!(input.text(), "old\nresolved\ntail");
            assert_eq!(input.selected_text(), Some("tail".to_string()));
            assert!(input.protected_ranges().is_empty());
            assert_eq!(input.layout.scroll_x, px(24.0));
            assert!(!input.interaction.pending_cursor_autoscroll);
        });
    });
}

const PROTECTED_SAMPLE_TEXT: &str = "head\n<Merge Conflict>\ntail\n";

fn protected_sample_span() -> Range<usize> {
    let start = PROTECTED_SAMPLE_TEXT
        .find("<Merge Conflict>")
        .expect("placeholder line");
    start..start + "<Merge Conflict>\n".len()
}

fn protected_sample_input(
    cx: &mut gpui::TestAppContext,
) -> (Entity<TextInput>, &mut gpui::VisualTestContext) {
    let (input, cx) = multiline_input(cx);
    cx.update(|_window, app| {
        input.update(app, |input, cx| {
            input.set_text(PROTECTED_SAMPLE_TEXT, cx);
            input.set_protected_ranges(Arc::from([protected_sample_span()]));
        });
    });
    (input, cx)
}

#[gpui::test]
fn protected_ranges_reject_typed_edits_that_would_alter_them(cx: &mut gpui::TestAppContext) {
    let span = protected_sample_span();
    let (input, cx) = protected_sample_input(cx);

    for (range, inserted) in [
        // Typing inside the line, over it, and at its first offset.
        (span.start + 2..span.start + 2, "x"),
        (span.start..span.end, "picked\n"),
        (span.start..span.start, "x"),
        // Backspace at the line start, which would join the line above into it.
        (span.start - 1..span.start, ""),
        // Replacing the line above with text that no longer ends the line.
        (0..span.start, "head"),
    ] {
        cx.update(|window, app| {
            input.update(app, |input, cx| {
                input.replace_text_in_range(Some(range.clone()), inserted, window, cx);
                assert_eq!(
                    input.text(),
                    PROTECTED_SAMPLE_TEXT,
                    "replacing {range:?} with {inserted:?} must be refused"
                );
                assert_eq!(input.protected_ranges(), std::slice::from_ref(&span));
            });
        });
    }
}

#[gpui::test]
fn protected_ranges_ride_along_with_edits_around_them(cx: &mut gpui::TestAppContext) {
    let span = protected_sample_span();
    let (input, cx) = protected_sample_input(cx);

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            // Editing the line after the span leaves it where it was.
            input.replace_text_in_range(Some(span.end..span.end), "new\n", window, cx);
            assert_eq!(input.text(), "head\n<Merge Conflict>\nnew\ntail\n");
            assert_eq!(input.protected_ranges(), std::slice::from_ref(&span));

            // Editing before it moves it by the length the edit added.
            input.replace_text_in_range(Some(0..0), "top\n", window, cx);
            assert_eq!(input.text(), "top\nhead\n<Merge Conflict>\nnew\ntail\n");
            let moved = span.start + "top\n".len()..span.end + "top\n".len();
            assert_eq!(input.protected_ranges(), std::slice::from_ref(&moved));

            // Deleting the whole line above keeps the placeholder a line of its
            // own, so that edit goes through.
            let span = input.protected_ranges()[0].clone();
            input.replace_text_in_range(
                Some(span.start - "head\n".len()..span.start),
                "",
                window,
                cx,
            );
            assert_eq!(input.text(), "top\n<Merge Conflict>\nnew\ntail\n");
            let moved = span.start - "head\n".len()..span.end - "head\n".len();
            assert_eq!(input.protected_ranges(), std::slice::from_ref(&moved));
        });
    });
}
