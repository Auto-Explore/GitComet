//! Platform-aware keyboard-shortcut labels for menus, tooltips, and hints.
//!
//! Keybindings registered with the `secondary` modifier resolve to Cmd on
//! macOS and Ctrl everywhere else; labels shown to the user must match.

/// Label for a shortcut bound with the `secondary` modifier, e.g.
/// `secondary_shortcut("O")` → "Cmd+O" on macOS, "Ctrl+O" elsewhere.
pub(crate) fn secondary_shortcut(suffix: &str) -> String {
    secondary_shortcut_for(suffix, cfg!(target_os = "macos"))
}

fn secondary_shortcut_for(suffix: &str, is_macos: bool) -> String {
    if is_macos {
        format!("Cmd+{suffix}")
    } else {
        format!("Ctrl+{suffix}")
    }
}

/// Label for a shortcut bound with the `alt` modifier. macOS calls this key
/// Option, while the other supported desktop platforms call it Alt.
pub(crate) fn alt_shortcut(suffix: &str) -> String {
    alt_shortcut_for(suffix, cfg!(target_os = "macos"))
}

fn alt_shortcut_for(suffix: &str, is_macos: bool) -> String {
    if is_macos {
        format!("Option+{suffix}")
    } else {
        format!("Alt+{suffix}")
    }
}

/// Tooltip for every diff surface's previous-change button.
pub(crate) fn previous_change_tooltip() -> String {
    format!("Previous change (F2 / Shift+F7 / {})", alt_shortcut("Up"))
}

/// Tooltip for every diff surface's next-change button.
pub(crate) fn next_change_tooltip() -> String {
    format!("Next change (F3 / F7 / {})", alt_shortcut("Down"))
}

/// A displayable shortcut attached to a command-palette entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shortcut {
    None,
    /// Bound with the `secondary` modifier: shown as Cmd+… on macOS,
    /// Ctrl+… elsewhere.
    Secondary(&'static str),
    /// Bound with the `alt` modifier: shown as Option+… on macOS,
    /// Alt+… elsewhere.
    Alt(&'static str),
    /// A binding whose preferred shortcut differs by platform.
    Platform {
        macos: &'static str,
        other: &'static str,
    },
    /// A binding that is only registered on macOS.
    MacOs(&'static str),
    /// An extension's binding in GPUI keystroke syntax, like `secondary-shift-k`.
    Keystrokes(&'static str),
}

impl Shortcut {
    pub(crate) fn label(&self) -> Option<String> {
        self.label_for(cfg!(target_os = "macos"))
    }

    fn label_for(&self, is_macos: bool) -> Option<String> {
        match self {
            Shortcut::None => None,
            Shortcut::Secondary(suffix) => Some(secondary_shortcut_for(suffix, is_macos)),
            Shortcut::Alt(suffix) => Some(alt_shortcut_for(suffix, is_macos)),
            Shortcut::Platform { macos, other } => {
                Some(if is_macos { *macos } else { *other }.to_string())
            }
            Shortcut::MacOs(label) => is_macos.then(|| (*label).to_string()),
            Shortcut::Keystrokes(source) => Some(keystrokes_label(source, is_macos)),
        }
    }
}

/// `secondary-shift-k g` → "Ctrl+Shift+K G" (Cmd on macOS). The modifiers
/// come from GPUI's parse, which resolves `secondary` for this platform.
fn keystrokes_label(source: &str, is_macos: bool) -> String {
    let chords: Vec<String> = source
        .split_whitespace()
        .map(|chord| {
            let Ok(keystroke) = gpui::Keystroke::parse(chord) else {
                return chord.to_string();
            };
            let modifiers = keystroke.modifiers;
            let mut parts: Vec<String> = Vec::new();
            if modifiers.control {
                parts.push("Ctrl".into());
            }
            if modifiers.alt {
                parts.push(if is_macos { "Option" } else { "Alt" }.into());
            }
            if modifiers.shift {
                parts.push("Shift".into());
            }
            if modifiers.platform {
                parts.push(if is_macos { "Cmd" } else { "Super" }.into());
            }
            let mut key = keystroke.key.chars();
            parts.push(match key.next() {
                Some(first) => first.to_uppercase().chain(key).collect(),
                None => String::new(),
            });
            parts.join("+")
        })
        .collect();
    chords.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_labels_use_macos_names() {
        assert_eq!(secondary_shortcut_for("O", true), "Cmd+O");
        assert_eq!(alt_shortcut_for("Left", true), "Option+Left");
        assert_eq!(secondary_shortcut_for("O", false), "Ctrl+O");
        assert_eq!(alt_shortcut_for("Left", false), "Alt+Left");
    }

    #[test]
    fn platform_shortcuts_choose_only_registered_labels() {
        let platform = Shortcut::Platform {
            macos: "Ctrl+Cmd+F",
            other: "F11",
        };
        assert_eq!(platform.label_for(true).as_deref(), Some("Ctrl+Cmd+F"));
        assert_eq!(platform.label_for(false).as_deref(), Some("F11"));

        let macos_only = Shortcut::MacOs("Cmd+M");
        assert_eq!(macos_only.label_for(true).as_deref(), Some("Cmd+M"));
        assert_eq!(macos_only.label_for(false), None);
    }

    #[test]
    fn extension_keystrokes_read_like_built_in_labels() {
        let label = Shortcut::Keystrokes("ctrl-shift-k alt-f5").label_for(false);
        assert_eq!(label.as_deref(), Some("Ctrl+Shift+K Alt+F5"));
        let label = Shortcut::Keystrokes("alt-f5").label_for(true);
        assert_eq!(label.as_deref(), Some("Option+F5"));
    }
}
