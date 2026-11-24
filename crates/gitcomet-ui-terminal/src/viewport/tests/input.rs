use super::*;
use alacritty_terminal::{event_loop::Msg, vte::ansi::Processor};
use gpui::{Keystroke, Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};

fn wheel(lines: f32) -> ScrollWheelEvent {
    ScrollWheelEvent {
        position: test_cell_pos(0, 0),
        delta: ScrollDelta::Lines(point(0.0, lines)),
        modifiers: Modifiers::default(),
        touch_phase: TouchPhase::Moved,
    }
}

fn with_recording_pty(
    view: &Entity<TerminalViewportView>,
    cx: &mut gpui::VisualTestContext,
    input: impl FnOnce(&mut TerminalViewportView, &mut Window, &mut gpui::Context<TerminalViewportView>),
) -> smol::channel::Receiver<Msg> {
    let (pty, messages) = PtySender::recording();
    with_viewport(view, cx, |this, window, cx| {
        this.pty_sender = Some(pty);
        input(this, window, cx);
        this.pty_sender = None;
    });
    messages
}

#[gpui::test]
fn application_cursor_wheel_reports_direction_with_bounded_repeats(cx: &mut gpui::TestAppContext) {
    let term = test_term_with_lines(30);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut *term.lock(), b"\x1b[?1049h\x1b[?1h");
    let (view, cx) = test_viewport(term.clone(), cx);
    let messages = with_recording_pty(&view, cx, |this, window, cx| {
        this.last_content = Some(make_terminal_content(&term.lock()));
        this.handle_scroll_wheel(&wheel(2.0), window, cx);
        this.handle_scroll_wheel(&wheel(-40.0), window, cx);
    });

    for expected in [b"\x1bOA".repeat(2), b"\x1bOB".repeat(24)] {
        let Msg::Input(bytes) = messages.try_recv().expect("wheel input") else {
            panic!("wheel must send key input");
        };
        assert_eq!(bytes.as_ref(), expected.as_slice());
    }
    assert!(messages.is_empty());
    assert_eq!(display_offset_of(&view, cx), 0);
}

#[gpui::test]
fn a_wheel_gesture_scrolls_the_buffer_once_without_sending_keys(cx: &mut gpui::TestAppContext) {
    let (term, events) = test_term_with_lines_and_events(30);
    let (view, cx) = test_viewport(term, cx);
    let messages = with_recording_pty(&view, cx, |this, window, cx| {
        this.handle_scroll_wheel(&wheel(12.0), window, cx);
    });
    assert_eq!(display_offset_of(&view, cx), 12);
    assert!(matches!(
        events.try_recv(),
        Ok(TerminalBackendEvent::Wakeup)
    ));
    assert!(
        events.is_empty(),
        "one wheel gesture wakes the painter once"
    );
    assert!(messages.is_empty(), "scrollback is local to the terminal");

    let messages = with_recording_pty(&view, cx, |this, window, cx| {
        this.handle_scroll_wheel(&wheel(-80.0), window, cx)
    });
    assert_eq!(display_offset_of(&view, cx), 0);
    assert!(messages.is_empty(), "scrollback is local to the terminal");
}

#[test]
fn a_scrollbar_jump_updates_the_offset_with_one_wakeup() {
    let (term, events) = test_term_with_lines_and_events(30);
    let driver = TerminalScrollbarDriver {
        term_lock: Some(term),
        line_height: px(TEST_LINE_H),
        fallback_history: 0,
        fallback_display: 0,
    };

    let target = px(-TEST_LINE_H * 5.0);
    driver.set_axis_offset(ScrollbarAxis::Vertical, target);
    assert_eq!(driver.raw_offset(ScrollbarAxis::Vertical), target);
    assert!(matches!(
        events.try_recv(),
        Ok(TerminalBackendEvent::Wakeup)
    ));
    assert!(events.is_empty());
}

#[gpui::test]
fn terminal_mouse_motion_reports_each_new_cell_once(cx: &mut gpui::TestAppContext) {
    let term = test_term_with_lines(1);
    let mut parser: Processor = Processor::new();
    parser.advance(&mut *term.lock(), b"\x1b[?1003h\x1b[?1006h");
    let (view, cx) = test_viewport(term.clone(), cx);
    let messages = with_recording_pty(&view, cx, |this, window, cx| {
        this.last_content = Some(make_terminal_content(&term.lock()));
        let first = test_cell_pos(0, 0);
        for position in [
            first,
            point(first.x + px(1.0), first.y),
            test_cell_pos(0, 1),
        ] {
            this.handle_mouse_move(
                &MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers: Modifiers::default(),
                },
                window,
                cx,
            );
        }
    });

    for expected in [b"\x1b[<35;1;1M", b"\x1b[<35;2;1M"] {
        let Msg::Input(bytes) = messages.try_recv().expect("mouse report") else {
            panic!("mouse movement must send input");
        };
        assert_eq!(bytes.as_ref(), expected);
    }
    assert!(messages.is_empty());
}

#[gpui::test]
fn keyboard_copy_without_a_selection_copies_only_the_visible_rows(cx: &mut gpui::TestAppContext) {
    let term = test_term_with_lines(30);
    term.lock().scroll_display(Scroll::Delta(3));
    let (view, cx) = test_viewport(term, cx);
    let shortcut = Keystroke::parse(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-shift-c"
    })
    .unwrap();
    with_viewport(&view, cx, |this, window, cx| {
        assert!(!this.has_selection());
        assert!(this.handle_key_down(&shortcut, window, cx));
    });
    let copied = cx.update(|_, app| app.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(
        copied.as_deref(),
        Some("line021\nline022\nline023\nline024\nline025\nline026")
    );
}
