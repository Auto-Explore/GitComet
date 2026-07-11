//! Platform-aware keyboard-shortcut labels for menus, tooltips, and hints.
//!
//! Keybindings registered with the `secondary` modifier resolve to Cmd on
//! macOS and Ctrl everywhere else; labels shown to the user must match.

/// Label for a shortcut bound with the `secondary` modifier, e.g.
/// `secondary_shortcut("O")` → "Cmd+O" on macOS, "Ctrl+O" elsewhere.
pub(crate) fn secondary_shortcut(suffix: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("Cmd+{suffix}")
    } else {
        format!("Ctrl+{suffix}")
    }
}

/// A displayable shortcut attached to a command-palette entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Shortcut {
    None,
    /// Bound with the `secondary` modifier: shown as Cmd+… on macOS,
    /// Ctrl+… elsewhere.
    Secondary(&'static str),
    /// Shown verbatim on every platform (e.g. "Ctrl+Tab", "F11").
    Fixed(&'static str),
}

impl Shortcut {
    pub(crate) fn label(&self) -> Option<String> {
        match self {
            Shortcut::None => None,
            Shortcut::Secondary(suffix) => Some(secondary_shortcut(suffix)),
            Shortcut::Fixed(label) => Some((*label).to_string()),
        }
    }
}
