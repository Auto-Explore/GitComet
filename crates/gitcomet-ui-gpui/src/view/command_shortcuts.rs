//! Keyboard access to the same commands and checks as the command palette.

use super::{GitCometView, GitCometViewMode, command_palette};
use crate::kit::components::Shortcut;
use gpui::{Action, App, KeyBinding, Unbind, Window};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema, Action)]
#[action(namespace = command)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunBuiltinCommand {
    pub(crate) id: String,
}

pub(crate) fn bind_keys(cx: &mut App) {
    // The palette's keystroke labels are also the binding definitions, so the
    // shortcuts users discover there cannot drift from the registered keys.
    cx.bind_keys(
        command_palette::COMMANDS
            .iter()
            .filter_map(|entry| {
                let Shortcut::Keystrokes(keys) = entry.shortcut else {
                    return None;
                };
                Some(bindings(keys, entry.id))
            })
            .flatten(),
    );
    cx.bind_keys(bindings(
        "f5",
        super::commands::Command::ReloadRepository.id(),
    ));
    cx.bind_keys(bindings(
        "secondary-shift-b",
        super::commands::Command::ToggleDetails.id(),
    ));
    #[cfg(not(target_os = "macos"))]
    cx.bind_keys(bindings(
        "ctrl-shift-j",
        super::commands::Command::ToggleTerminal.id(),
    ));
    for keys in ["secondary-?", "secondary-shift-/", "secondary-shift-?"] {
        cx.bind_keys(bindings(
            keys,
            super::commands::Command::KeyboardShortcuts.id(),
        ));
    }
}

fn bindings(keys: &str, id: &str) -> [KeyBinding; 2] {
    let action = RunBuiltinCommand { id: id.to_owned() };
    let unbind = Unbind(action.name().into());
    let terminal_action = matches!(
        super::commands::Command::from_id(id),
        Some(
            super::commands::Command::ToggleTerminal | super::commands::Command::KeyboardShortcuts
        )
    ) && (cfg!(target_os = "macos")
        || keys.contains("shift")
        || keys.contains('?'));
    [
        // A negated context predicate does not match an empty context stack.
        // Bind globally, then disable just this action in protected contexts.
        KeyBinding::new(keys, action, None),
        // Match protected ancestors at the deepest context as well, so this
        // exclusion takes precedence over the global binding in nested inputs.
        KeyBinding::new(
            keys,
            unbind,
            Some(if terminal_action {
                "!(!ContextMenu && !PopoverPrompt)"
            } else {
                "!(!Terminal && !ContextMenu && !PopoverPrompt)"
            }),
        ),
    ]
}

pub(crate) fn is_terminal_toggle(keystroke: &gpui::Keystroke) -> bool {
    let shifted = !cfg!(target_os = "macos");
    keystroke.key == "j"
        && keystroke.modifiers.secondary()
        && keystroke.modifiers.shift == shifted
        && keystroke.modifiers.number_of_modifiers() == if shifted { 2 } else { 1 }
}

/// Off macOS a focused terminal keeps plain Ctrl+/; Windows and Linux report
/// Ctrl+Shift+/ as `ctrl-?`, dropping the Shift.
pub(crate) fn is_shortcut_help(keystroke: &gpui::Keystroke) -> bool {
    let mods = keystroke.modifiers;
    mods.secondary()
        && (cfg!(target_os = "macos") || mods.shift || keystroke.key == "?")
        && !mods.alt
        && !mods.function
        && mods.number_of_modifiers() == if mods.shift { 2 } else { 1 }
        && matches!(keystroke.key.as_str(), "?" | "/")
}

/// Windows reports AltGr as Ctrl+Alt, so a chord that typed a character
/// (AltGr+S is "ś" on a Polish layout) is text, not a shortcut.
pub(crate) fn types_character(keystroke: &gpui::Keystroke) -> bool {
    let mods = keystroke.modifiers;
    mods.control
        && mods.alt
        && !mods.platform
        && keystroke
            .key_char
            .as_ref()
            .is_some_and(|ch| *ch != keystroke.key)
}

impl GitCometView {
    pub(crate) fn run_builtin_shortcut(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        if id == super::commands::Command::KeyboardShortcuts.id() {
            self.toggle_shortcut_help(window, cx);
            return;
        }
        // Text inputs already take such characters; elsewhere they type nothing.
        if self.view_mode != GitCometViewMode::Normal
            || self.background_hovers_blocked(cx)
            || self.keystroke_types_character
        {
            return;
        }
        self.execute_command(id, Some(window), cx);
    }
}
