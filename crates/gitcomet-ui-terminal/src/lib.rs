//! Embedded terminal widget: an alacritty backend on a PTY and the GPUI
//! viewport that paints, selects and forwards input to it. Tabs, sessions and
//! shutdown policy belong to the host.

mod backend;
mod painting;
mod viewport;

pub use backend::*;
pub use viewport::*;

pub(crate) const TERMINAL_LINE_HEIGHT_SCALE: f32 = 1.15;
pub(crate) const TERMINAL_MIN_GRID_ROWS: u16 = 2;
pub(crate) const TERMINAL_MIN_GRID_COLS: u16 = 8;
pub(crate) const TERMINAL_CARET_WIDTH_RATIO: f32 = 0.12;
pub(crate) const TERMINAL_CARET_MIN_WIDTH_PX: f32 = 2.0;
pub(crate) const TERMINAL_CARET_MAX_WIDTH_PX: f32 = 3.0;
pub(crate) const TERMINAL_CARET_VERTICAL_INSET_PX: f32 = 1.0;
pub(crate) const TERMINAL_CARET_RADIUS_PX: f32 = 0.0;
pub const TERMINAL_CARET_BLINK_INTERVAL_MS: u64 = 530;
pub(crate) const TERMINAL_CARET_RESUME_DELAY_MS: u64 = 700;
/// The caret blinks while the terminal is in use and then rests, shown, as
/// text inputs do: every blink redraws the whole window.
pub const TERMINAL_CARET_BLINK_TIMEOUT_MS: u64 = 10_000;
pub(crate) const TERMINAL_SELECTION_ALPHA: f32 = 0.32;
pub(crate) const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
pub(crate) const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

/// Clipboard and screen commands a host forwards to a focused terminal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalCommand {
    Copy,
    Paste,
    SelectAll,
    ClearScreenAndScrollback,
}

/// The copy, paste and select-all chord: Cmd on macOS, Ctrl+Shift elsewhere.
pub(crate) fn terminal_clipboard_shortcut_action(
    keystroke: &gpui::Keystroke,
) -> Option<TerminalCommand> {
    let action = match keystroke.key.as_str() {
        "c" | "C" => TerminalCommand::Copy,
        "v" | "V" => TerminalCommand::Paste,
        "a" | "A" => TerminalCommand::SelectAll,
        _ => return None,
    };
    let mods = keystroke.modifiers;
    if cfg!(target_os = "macos") {
        if mods.platform && !mods.control && !mods.alt && !mods.function && !mods.shift {
            Some(action)
        } else {
            None
        }
    } else if mods.control && mods.shift && !mods.platform && !mods.alt && !mods.function {
        Some(action)
    } else {
        None
    }
}

#[cfg(test)]
mod source_guards;
