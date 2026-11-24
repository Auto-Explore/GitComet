use crate::{BRACKETED_PASTE_END, BRACKETED_PASTE_START};
use gpui::{Pixels, px};

const TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS: usize = 24;

pub(crate) fn terminal_alt_screen_scroll_bytes(
    delta_y: Pixels,
    step_rows: usize,
    app_cursor: bool,
) -> Vec<u8> {
    let delta_gt_zero = delta_y > px(0.0);
    let sequence: &[u8] = if delta_gt_zero {
        if app_cursor { b"\x1bOA" } else { b"\x1b[A" }
    } else if app_cursor {
        b"\x1bOB"
    } else {
        b"\x1b[B"
    };
    let repeats = step_rows.clamp(1, TERMINAL_ALT_SCREEN_WHEEL_MAX_KEY_REPEATS);
    let mut bytes = Vec::with_capacity(sequence.len() * repeats);
    for _ in 0..repeats {
        bytes.extend_from_slice(sequence);
    }
    bytes
}

pub(crate) fn encode_alacritty_key_input(
    keystroke: &gpui::Keystroke,
    app_cursor: bool,
) -> Option<Vec<u8>> {
    let key = keystroke.key.as_str();
    let mods = keystroke.modifiers;

    if mods.platform || mods.function {
        return None;
    }

    // Alt-as-Meta is intentionally NOT handled up front: special keys (arrows,
    // home/end, function keys) must flow through the `modifier_code` block below
    // so `Alt+Left` becomes `ESC[1;3D`, and character keys are Meta-prefixed by
    // the `ESC`+key fallback at the end (`Alt+f` -> `ESC f`). Handling Alt here
    // first — and routing letters through `encode_control_key` — produced
    // `ESC Ctrl-F` for `Alt+f` and swallowed `Alt`+special-key combos entirely.

    if mods.shift && !mods.control && !mods.alt {
        match key {
            "tab" => return Some(b"\x1b[Z".to_vec()),
            "up" => return Some(b"\x1b[1;2A".to_vec()),
            "down" => return Some(b"\x1b[1;2B".to_vec()),
            "right" => return Some(b"\x1b[1;2C".to_vec()),
            "left" => return Some(b"\x1b[1;2D".to_vec()),
            _ => {}
        }
    }

    let modifier_code = terminal_modifier_code(keystroke);
    if modifier_code > 1
        && let Some(bytes) = modified_key_bytes(key, modifier_code)
    {
        return Some(bytes);
    }

    if mods.control
        && !mods.alt
        && let Some(control) = encode_control_key(key)
    {
        return Some(vec![control]);
    }

    // Meta (Alt) + editing keys that have no CSI-with-modifier form: prefix ESC.
    if mods.alt && !mods.control {
        match key {
            "backspace" => return Some(vec![0x1b, 0x7f]),
            "enter" => return Some(vec![0x1b, b'\r']),
            _ => {}
        }
    }

    match key {
        "enter" => Some(vec![b'\r']),
        "tab" => Some(vec![b'\t']),
        "space" => Some(vec![b' ']),
        "backspace" => Some(vec![0x7f]),
        "escape" => Some(vec![0x1b]),
        "insert" => Some(b"\x1b[2~".to_vec()),
        "delete" => Some(b"\x1b[3~".to_vec()),
        "home" => {
            if app_cursor {
                Some(b"\x1bOH".to_vec())
            } else {
                Some(b"\x1b[H".to_vec())
            }
        }
        "end" => {
            if app_cursor {
                Some(b"\x1bOF".to_vec())
            } else {
                Some(b"\x1b[F".to_vec())
            }
        }
        "pageup" => Some(b"\x1b[5~".to_vec()),
        "pagedown" => Some(b"\x1b[6~".to_vec()),
        "up" => {
            if app_cursor {
                Some(b"\x1bOA".to_vec())
            } else {
                Some(b"\x1b[A".to_vec())
            }
        }
        "down" => {
            if app_cursor {
                Some(b"\x1bOB".to_vec())
            } else {
                Some(b"\x1b[B".to_vec())
            }
        }
        "right" => {
            if app_cursor {
                Some(b"\x1bOC".to_vec())
            } else {
                Some(b"\x1b[C".to_vec())
            }
        }
        "left" => {
            if app_cursor {
                Some(b"\x1bOD".to_vec())
            } else {
                Some(b"\x1b[D".to_vec())
            }
        }
        "f1" => Some(b"\x1bOP".to_vec()),
        "f2" => Some(b"\x1bOQ".to_vec()),
        "f3" => Some(b"\x1bOR".to_vec()),
        "f4" => Some(b"\x1bOS".to_vec()),
        "f5" => Some(b"\x1b[15~".to_vec()),
        "f6" => Some(b"\x1b[17~".to_vec()),
        "f7" => Some(b"\x1b[18~".to_vec()),
        "f8" => Some(b"\x1b[19~".to_vec()),
        "f9" => Some(b"\x1b[20~".to_vec()),
        "f10" => Some(b"\x1b[21~".to_vec()),
        "f11" => Some(b"\x1b[23~".to_vec()),
        "f12" => Some(b"\x1b[24~".to_vec()),
        _ if mods.alt && !mods.control && key.len() == 1 => {
            let base = key.as_bytes();
            let mut bytes = Vec::with_capacity(1 + base.len());
            bytes.push(0x1b);
            bytes.extend_from_slice(base);
            Some(bytes)
        }
        _ if key.len() == 1 && !mods.control && !mods.alt => {
            let ch = keystroke.key_char.as_deref().unwrap_or(key);
            Some(ch.as_bytes().to_vec())
        }
        _ => None,
    }
}

fn modified_key_bytes(key: &str, modifier_code: u32) -> Option<Vec<u8>> {
    match key {
        "up" => Some(format!("\x1b[1;{modifier_code}A")),
        "down" => Some(format!("\x1b[1;{modifier_code}B")),
        "right" => Some(format!("\x1b[1;{modifier_code}C")),
        "left" => Some(format!("\x1b[1;{modifier_code}D")),
        "home" => Some(format!("\x1b[1;{modifier_code}H")),
        "end" => Some(format!("\x1b[1;{modifier_code}F")),
        "insert" => Some(format!("\x1b[2;{modifier_code}~")),
        "delete" => Some(format!("\x1b[3;{modifier_code}~")),
        "pageup" => Some(format!("\x1b[5;{modifier_code}~")),
        "pagedown" => Some(format!("\x1b[6;{modifier_code}~")),
        "f1" => Some(format!("\x1b[1;{modifier_code}P")),
        "f2" => Some(format!("\x1b[1;{modifier_code}Q")),
        "f3" => Some(format!("\x1b[1;{modifier_code}R")),
        "f4" => Some(format!("\x1b[1;{modifier_code}S")),
        "f5" => Some(format!("\x1b[15;{modifier_code}~")),
        "f6" => Some(format!("\x1b[17;{modifier_code}~")),
        "f7" => Some(format!("\x1b[18;{modifier_code}~")),
        "f8" => Some(format!("\x1b[19;{modifier_code}~")),
        "f9" => Some(format!("\x1b[20;{modifier_code}~")),
        "f10" => Some(format!("\x1b[21;{modifier_code}~")),
        "f11" => Some(format!("\x1b[23;{modifier_code}~")),
        "f12" => Some(format!("\x1b[24;{modifier_code}~")),
        _ => None,
    }
    .map(String::into_bytes)
}

fn encode_control_key(key: &str) -> Option<u8> {
    if key.len() == 1 {
        let ch = key.as_bytes()[0];
        if ch.is_ascii_lowercase() {
            return Some(ch - b'a' + 1);
        }
    }
    match key {
        "space" => Some(0),
        "[" => Some(0x1b),
        "\\" => Some(0x1c),
        "]" => Some(0x1d),
        "^" => Some(0x1e),
        "_" | "/" => Some(0x1f),
        _ => None,
    }
}

fn terminal_modifier_code(keystroke: &gpui::Keystroke) -> u32 {
    let mut modifier_code = 0;
    if keystroke.modifiers.shift {
        modifier_code |= 1;
    }
    if keystroke.modifiers.alt {
        modifier_code |= 1 << 1;
    }
    if keystroke.modifiers.control {
        modifier_code |= 1 << 2;
    }
    modifier_code + 1
}

pub(crate) fn terminal_paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let framing = if bracketed {
        BRACKETED_PASTE_START.len() + BRACKETED_PASTE_END.len()
    } else {
        0
    };
    let mut bytes = Vec::with_capacity(text.len() + framing);
    if bracketed {
        bytes.extend_from_slice(BRACKETED_PASTE_START);
    }
    let mut chars = text.chars().peekable();
    let mut encoded = [0; 4];
    while let Some(ch) = chars.next() {
        let ch = if ch == '\r' {
            chars.next_if_eq(&'\n');
            '\n'
        } else {
            ch
        };
        if !bracketed || matches!(ch, '\n' | '\t') || !ch.is_control() {
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    if bracketed {
        bytes.extend_from_slice(BRACKETED_PASTE_END);
    }
    bytes
}

#[cfg(test)]
mod tests;
