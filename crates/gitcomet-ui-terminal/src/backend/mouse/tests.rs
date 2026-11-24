use super::*;

#[test]
fn mouse_mode_composite_flag() {
    assert!(!TerminalModes::default().mouse_mode());
    for mode in [
        TerminalModes::MOUSE_REPORT_CLICK,
        TerminalModes::MOUSE_DRAG,
        TerminalModes::MOUSE_MOTION,
    ] {
        assert!(mode.mouse_mode());
    }
}

#[test]
fn sgr_mouse_button_press_report() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    let report = terminal_mouse_button_report(
        5,
        10,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
        true,
        mode,
    )
    .unwrap();
    let s = String::from_utf8(report).unwrap();
    assert_eq!(s, "\x1b[<0;11;6M");
}

#[test]
fn sgr_mouse_button_release_report() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    let report = terminal_mouse_button_report(
        3,
        7,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
        false,
        mode,
    )
    .unwrap();
    let s = String::from_utf8(report).unwrap();
    assert_eq!(s, "\x1b[<0;8;4m");
}

#[test]
fn sgr_mouse_report_with_modifiers() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    let mods = gpui::Modifiers {
        shift: true,
        control: true,
        ..Default::default()
    };
    let report =
        terminal_mouse_button_report(0, 0, gpui::MouseButton::Left, mods, true, mode).unwrap();
    let s = String::from_utf8(report).unwrap();
    assert_eq!(s, "\x1b[<20;1;1M", "shift=4 + control=16 + button=0 = 20");
}

#[test]
fn normal_mouse_button_press_report() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK;
    let report = terminal_mouse_button_report(
        0,
        0,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
        true,
        mode,
    )
    .unwrap();
    assert_eq!(report, vec![0x1b, b'[', b'M', 32, (32 + 1), (32 + 1)]);
}

#[test]
fn normal_mouse_button_release_report() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK;
    let report = terminal_mouse_button_report(
        0,
        0,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
        false,
        mode,
    )
    .unwrap();
    assert_eq!(report, vec![0x1b, b'[', b'M', 32 + 3, (32 + 1), (32 + 1)]);
}

#[test]
fn mouse_moved_report_with_motion_mode() {
    let mode = TerminalModes::MOUSE_MOTION | TerminalModes::SGR_MOUSE;
    let report = terminal_mouse_moved_report(
        2,
        5,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
        mode,
    )
    .unwrap();
    let s = String::from_utf8(report).unwrap();
    assert_eq!(s, "\x1b[<32;6;3M");
}

#[test]
fn mouse_moved_report_none_move_blocked_in_drag_only_mode() {
    let mode = TerminalModes::MOUSE_DRAG;
    let report = terminal_mouse_moved_report(0, 0, None, gpui::Modifiers::default(), mode);
    assert!(
        report.is_none(),
        "NoneMove blocked when only MOUSE_DRAG is set"
    );
}

#[test]
fn scroll_report_generates_reports() {
    let mode = TerminalModes::MOUSE_MODE | TerminalModes::SGR_MOUSE;
    let report =
        terminal_scroll_report(10, 20, gpui::Modifiers::default(), px(-1.0), 2, mode).unwrap();
    assert_eq!(report, b"\x1b[<65;21;11M\x1b[<65;21;11M");
}

#[test]
fn mouse_report_rejects_negative_grid_rows() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    let report = terminal_mouse_button_report(
        -1,
        0,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
        true,
        mode,
    );
    assert!(
        report.is_none(),
        "negative grid row must not produce report"
    );
}

#[test]
fn mouse_report_suppresses_navigate_buttons() {
    let mode = TerminalModes::MOUSE_REPORT_CLICK | TerminalModes::SGR_MOUSE;
    let report = terminal_mouse_button_report(
        0,
        0,
        gpui::MouseButton::Navigate(gpui::NavigationDirection::Back),
        gpui::Modifiers::default(),
        true,
        mode,
    );
    assert_eq!(report, None);
}
