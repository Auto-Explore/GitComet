use super::*;
use alacritty_terminal::event_loop::Msg;
use gpui::InputHandler;

#[gpui::test]
fn ime_reports_utf16_preedit_and_sends_only_committed_text(cx: &mut gpui::TestAppContext) {
    let cx = cx.add_empty_window();
    cx.update(|window, _| window.activate());
    cx.run_until_parked();
    cx.update(|window, app| {
        let focus = app.focus_handle();
        window.focus(&focus, app);
        let (pty, messages) = PtySender::recording();
        let mut handler = TerminalTextInputHandler::new(focus, Some(pty));

        assert!(handler.accepts_text_input(window, app));
        assert_eq!(handler.marked_text_range(window, app), None);
        handler.replace_and_mark_text_in_range(None, "猫🦊", None, window, app);
        assert_eq!(handler.marked_text_range(window, app), Some(0..3));
        assert!(messages.is_empty());

        handler.replace_text_in_range(None, "猫🦊", window, app);
        let Msg::Input(bytes) = messages.try_recv().expect("committed text") else {
            panic!("the commit must be sent to the terminal");
        };
        assert_eq!(bytes.as_ref(), "猫🦊".as_bytes());
        assert_eq!(handler.marked_text_range(window, app), None);

        handler.replace_and_mark_text_in_range(None, "仮", None, window, app);
        assert_eq!(handler.marked_text_range(window, app), Some(0..1));
        handler.unmark_text(window, app);
        assert_eq!(handler.marked_text_range(window, app), None);
        assert!(messages.is_empty());
    });
}

#[gpui::test]
fn ime_handler_apple_press_and_hold_disabled(cx: &mut gpui::TestAppContext) {
    let mut handler = TerminalTextInputHandler::new(cx.update(|app| app.focus_handle()), None);
    assert!(!handler.apple_press_and_hold_enabled());
}
