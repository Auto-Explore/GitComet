use super::*;

#[test]
fn paste_normalizes_line_endings_and_sanitizes_only_bracketed_input() {
    let text = "λ\r\n\t🦊\r\u{7}\u{85}done";
    assert_eq!(
        terminal_paste_bytes(text, false),
        "λ\n\t🦊\n\u{7}\u{85}done".as_bytes()
    );
    assert_eq!(
        terminal_paste_bytes(text, true),
        "\u{1b}[200~λ\n\t🦊\ndone\u{1b}[201~".as_bytes()
    );
}

fn alt_keystroke(key: &str) -> gpui::Keystroke {
    gpui::Keystroke {
        key: key.into(),
        modifiers: gpui::Modifiers {
            alt: true,
            ..Default::default()
        },
        key_char: None,
    }
}

#[test]
fn encode_alt_letter_is_meta_prefixed_not_control() {
    // Alt+f must be ESC 'f' (readline forward-word), not ESC Ctrl-F (0x06).
    let result = encode_alacritty_key_input(&alt_keystroke("f"), false);
    assert_eq!(result, Some(vec![0x1b, b'f']));
}

#[test]
fn encode_alt_arrow_uses_modifier_code() {
    // Alt+Left -> CSI 1 ; 3 D
    let result = encode_alacritty_key_input(&alt_keystroke("left"), false);
    assert_eq!(result, Some(b"\x1b[1;3D".to_vec()));
}

#[test]
fn encode_alt_backspace_is_meta_delete() {
    let result = encode_alacritty_key_input(&alt_keystroke("backspace"), false);
    assert_eq!(result, Some(vec![0x1b, 0x7f]));
}

#[test]
fn encode_shift_a_produces_uppercase_a() {
    let ks = gpui::Keystroke {
        key: "a".into(),
        modifiers: gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        key_char: Some("A".into()),
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(
        result,
        Some(b"A".to_vec()),
        "Shift+a must produce uppercase 'A'"
    );
}

#[test]
fn encode_no_shift_a_produces_lowercase_a() {
    let ks = gpui::Keystroke {
        key: "a".into(),
        modifiers: gpui::Modifiers::default(),
        key_char: Some("a".into()),
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b"a".to_vec()));
}

#[test]
fn encode_shift_digit_produces_symbol() {
    let ks = gpui::Keystroke {
        key: "1".into(),
        modifiers: gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        key_char: Some("!".into()),
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b"!".to_vec()), "Shift+1 must produce '!'");
}

#[test]
fn encode_shift_space_produces_space() {
    let ks = gpui::Keystroke {
        key: "space".into(),
        modifiers: gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        key_char: None,
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b" ".to_vec()));
}

#[test]
fn encode_tab_produces_tab_byte() {
    let ks = gpui::Keystroke {
        key: "tab".into(),
        modifiers: gpui::Modifiers::default(),
        key_char: None,
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b"\t".to_vec()));
}

#[test]
fn encode_shift_tab_produces_backtab_escape_sequence() {
    let ks = gpui::Keystroke {
        key: "tab".into(),
        modifiers: gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
        key_char: None,
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b"\x1b[Z".to_vec()));
}

#[test]
fn encode_ctrl_left_produces_modified_escape_sequence() {
    let ks = gpui::Keystroke {
        key: "left".into(),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        key_char: None,
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b"\x1b[1;5D".to_vec()));
}

#[test]
fn encode_ctrl_right_produces_modified_escape_sequence() {
    let ks = gpui::Keystroke {
        key: "right".into(),
        modifiers: gpui::Modifiers {
            control: true,
            ..Default::default()
        },
        key_char: None,
    };
    let result = encode_alacritty_key_input(&ks, false);
    assert_eq!(result, Some(b"\x1b[1;5C".to_vec()));
}
