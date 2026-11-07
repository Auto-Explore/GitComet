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
    [
        // A negated context predicate does not match an empty context stack.
        // Bind globally, then disable just this action in protected contexts.
        KeyBinding::new(keys, action, None),
        // Match protected ancestors at the deepest context as well, so this
        // exclusion takes precedence over the global binding in nested inputs.
        KeyBinding::new(
            keys,
            unbind,
            Some(
                if matches!(
                    super::commands::Command::from_id(id),
                    Some(
                        super::commands::Command::ToggleTerminal
                            | super::commands::Command::KeyboardShortcuts
                    )
                ) {
                    "!(!ContextMenu && !PopoverPrompt)"
                } else {
                    "!(!Terminal && !ContextMenu && !PopoverPrompt)"
                },
            ),
        ),
    ]
}

pub(crate) fn is_terminal_toggle(keystroke: &gpui::Keystroke) -> bool {
    keystroke.key == "j"
        && keystroke.modifiers.secondary()
        && keystroke.modifiers.number_of_modifiers() == 1
}

pub(crate) fn is_shortcut_help(keystroke: &gpui::Keystroke) -> bool {
    let mods = keystroke.modifiers;
    mods.secondary()
        && !mods.alt
        && !mods.function
        && mods.number_of_modifiers() == if mods.shift { 2 } else { 1 }
        && matches!(keystroke.key.as_str(), "?" | "/")
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
        if self.view_mode != GitCometViewMode::Normal
            || self.is_overlay_open(cx)
            || self.command_palette_open
            || self.reveal_commit_open
        {
            return;
        }
        self.execute_command(id, Some(window), cx);
    }
}
