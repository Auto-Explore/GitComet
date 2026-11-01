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

/// History outlives many edits, so its entries must not pin the whole-document
/// caches each revision warmed: a line-start table and a flattened copy per
/// keystroke, up to the step limit.
#[gpui::test]
fn undo_history_keeps_no_whole_document_caches(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);

    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text("alpha\nbeta", cx);
            for _ in 0..3 {
                // What prepaint and caret motion read between keystrokes.
                input.content.line_starts();
                input.text();
                input.replace_utf8_range(0..0, "x", cx);
            }
            input.undo(&Undo, window, cx);

            let history = input.selection.undo_stack.iter();
            for entry in history.chain(&input.selection.redo_stack) {
                assert!(!entry.content.is_line_index_built());
                assert!(!entry.content.is_materialized());
            }
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

/// `cargo test -p gitcomet-ui-kit --lib timing_undo_large_document --
/// --ignored --nocapture`: undoing one typed character in a 4.8 MB document
/// shown with the content-width layout (the file editor without wrap).
#[gpui::test]
#[ignore = "timing probe"]
fn timing_undo_large_document(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let text = "let value = compute(alpha, beta);\n".repeat(140_000);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_content_width_layout(true);
            input.set_text(text.as_str(), cx);
            let mut samples = Vec::new();
            for key in 0..11 {
                let at = text.len() / 2 + key * 40;
                input.replace_utf8_range(at..at, "x", cx);
                // What a frame reads between the keystroke and the undo.
                let _ = input.text();
                let started = std::time::Instant::now();
                input.undo(&Undo, window, cx);
                samples.push(started.elapsed());
            }
            samples.sort();
            println!(
                "timing undo_large_document bytes {} p50 {:?} min {:?}",
                text.len(),
                samples[5],
                samples[0]
            );
        });
    });
}

/// Undo finds the restored span from the two ropes' chunks. Flattening both
/// texts to compare them, then re-measuring every line, cost 82 ms per step
/// at 4.8 MB.
#[gpui::test]
fn undo_does_not_flatten_the_document(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let text = "let value = compute(alpha, beta);\n".repeat(2_000);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_content_width_layout(true);
            input.set_text(text.as_str(), cx);
            let middle = text.len() / 2;
            input.replace_utf8_range(middle..middle, "x\ny", cx);
            input.undo(&Undo, window, cx);
            let content = input.content.snapshot();
            assert!(!content.is_materialized());
            assert_eq!(input.text(), text);
        });
    });
}

/// The undo's width bookkeeping matches a full re-measure of the lines.
#[gpui::test]
fn undo_keeps_the_content_width_cache_exact(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let text = "short\n\tindented line\nwidest line of them all\n".repeat(300);
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_content_width_layout(true);
            input.set_text(text.as_str(), cx);
            let at = text.len() / 3;
            input.replace_utf8_range(at..at + 40, "joined\n\n\nmuch wider replacement text\n", cx);
            input.replace_utf8_range(0..0, "\t\t", cx);
            for _ in 0..2 {
                input.undo(&Undo, window, cx);
                let incremental = input.content_width_cache.take().unwrap();
                input.rebuild_content_width_cache();
                let rebuilt = input.content_width_cache.as_ref().unwrap();
                assert_eq!(incremental.line_units, rebuilt.line_units);
                assert_eq!(incremental.unit_counts, rebuilt.unit_counts);
            }
            assert_eq!(input.text(), text);
        });
    });
}

/// Undo restores the minimal edit across rope chunks, including a change
/// inside a multi-byte character (é and è share their first byte).
#[gpui::test]
fn undo_reports_the_minimal_edit_across_chunks(cx: &mut gpui::TestAppContext) {
    let (input, cx) = multiline_input(cx);
    let base = "αβγ plain ascii text é line\n".repeat(400);
    let accent = base.rfind('é').unwrap();
    let middle = (base.len() / 2..)
        .find(|&ix| base.is_char_boundary(ix))
        .unwrap();
    cx.update(|window, app| {
        input.update(app, |input, cx| {
            input.set_text(base.as_str(), cx);
            for (range, with) in [
                (accent..accent + 'é'.len_utf8(), "è"),
                (middle..middle, "inserted\nlines\n"),
                (middle..middle + 30, ""),
                (0..0, "x"),
            ] {
                let inserted = input.replace_utf8_range(range.clone(), with, cx);
                let edited = input.text().to_string();
                input.undo(&Undo, window, cx);
                assert_eq!(input.text(), base);
                assert_eq!(
                    input.selection.pending_text_edit_deltas.last(),
                    utf8_edit_delta_between_texts(&edited, &base).as_ref(),
                    "undoing {range:?} -> {with:?}"
                );
                assert_eq!(
                    input.selection.pending_text_edit_deltas.last(),
                    Some(&(inserted, range.clone())),
                    "undoing {range:?} -> {with:?}"
                );
            }
        });
    });
}
