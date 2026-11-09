use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, Rgba, div};

/// Keycap chips for a shortcut label such as `Ctrl+Shift+W`. The label is split
/// on `+` so each key gets its own chip, keeping menu rows and command-palette
/// rows visually identical.
/// Keycap plate. Sized to fit inside a context-menu row's content box at every
/// density, so a shortcut never deepens the row.
const KEYCAP_HEIGHT_PX: f32 = 20.0;
const KEYCAP_COMFORTABLE_HEIGHT_PX: f32 = 24.0;

pub fn shortcut_keys(label: &str, theme: AppTheme, scale: impl Into<UiScale>) -> Div {
    let scale = scale.into();
    let chip_height = scale.row_height(KEYCAP_HEIGHT_PX, KEYCAP_COMFORTABLE_HEIGHT_PX);
    keycaps(
        label,
        scale,
        chip_height,
        Keycap {
            pad: 6.0,
            text: 12.0,
            line: 14.0,
            fg: theme.colors.foreground.secondary,
            bg: theme.hover_overlay(),
        },
    )
}

/// Compact keycap plate, for a shortcut beside a button's label or in a
/// tooltip, where a menu-sized keycap would crowd the line.
const COMPACT_KEYCAP_HEIGHT_PX: f32 = 16.0;

/// [`shortcut_keys`] at a compact size, in `fg` on a faint plate of `fg`:
/// legible on a toolbar and inside a tooltip bubble alike.
pub fn shortcut_keys_compact(label: &str, fg: Rgba, scale: impl Into<UiScale>) -> Div {
    compact_keycaps(label, fg, compact_keycap_plate(fg), scale.into())
}

pub(crate) fn compact_keycap_plate(fg: Rgba) -> Rgba {
    crate::theme::with_alpha(fg, 0.14)
}

pub(crate) fn compact_keycaps(label: &str, fg: Rgba, bg: Rgba, scale: UiScale) -> Div {
    keycaps(
        label,
        scale,
        scale.px(COMPACT_KEYCAP_HEIGHT_PX),
        Keycap {
            pad: 4.0,
            text: 11.0,
            line: 12.0,
            fg,
            bg,
        },
    )
}

struct Keycap {
    pad: f32,
    text: f32,
    line: f32,
    fg: Rgba,
    bg: Rgba,
}

fn keycaps(label: &str, scale: UiScale, chip_height: gpui::Pixels, style: Keycap) -> Div {
    let Keycap {
        pad,
        text,
        line,
        fg,
        bg,
    } = style;
    div()
        .debug_selector(|| "shortcut_keycaps".to_string())
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(scale.px(10.0))
        .children(keycap_chords(label).into_iter().map(move |chord| {
            div()
                .flex()
                .items_center()
                .gap(scale.px(4.0))
                .children(chord.into_iter().map(move |key| {
                    div()
                        .min_w(chip_height)
                        .h(chip_height)
                        .px(scale.px(pad))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(scale.px(4.0))
                        .bg(bg)
                        .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                        .text_size(scale.ui_text(text))
                        .line_height(scale.px(line))
                        .text_color(fg)
                        .child(key)
                }))
        }))
}

/// Keep the visual modifier order consistent in palettes, menus and tooltips.
/// This changes labels only, never the keys an action is bound to.
pub fn ordered_shortcut_label(label: &str) -> String {
    fn priority(key: &str) -> Option<u8> {
        match key.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "win" | "meta" | "⌘" => Some(0),
            "ctrl" | "control" | "⌃" => Some(1),
            "alt" | "option" | "opt" | "⌥" => Some(2),
            "shift" | "⇧" => Some(3),
            "fn" => Some(4),
            _ => None,
        }
    }
    label
        .split_whitespace()
        .map(|chord| {
            let mut parts = chord.split('+').collect::<Vec<_>>();
            let modifiers = parts
                .iter()
                .take_while(|part| priority(part).is_some())
                .count();
            parts[..modifiers].sort_by_key(|part| priority(part));
            parts.join("+")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn keycap_chords(label: &str) -> Vec<Vec<String>> {
    ordered_shortcut_label(label)
        .split_whitespace()
        .map(|chord| {
            if chord == "+" {
                return vec!["+".to_owned()];
            }
            if let Some(prefix) = chord.strip_suffix("++") {
                let mut parts = prefix.split('+').map(str::to_owned).collect::<Vec<_>>();
                parts.push("+".to_owned());
                parts
            } else {
                chord.split('+').map(str::to_owned).collect()
            }
        })
        .collect()
}

/// Format GPUI keystrokes for the current platform.
pub fn keystrokes_display(source: &str) -> String {
    keystrokes_label_for(source, cfg!(target_os = "macos"))
}

pub fn keystrokes_label_for(source: &str, is_macos: bool) -> String {
    let chords: Vec<String> = source
        .split_whitespace()
        .map(|chord| {
            // Resolve `secondary` for the requested platform, including when
            // rendering a cross-platform reference on a different host OS.
            let source = chord.replace("secondary-", if is_macos { "cmd-" } else { "ctrl-" });
            let Ok(keystroke) = gpui::Keystroke::parse(&source) else {
                return chord.to_string();
            };
            let modifiers = keystroke.modifiers;
            let mut parts: Vec<String> = Vec::new();
            if modifiers.platform {
                parts.push(if is_macos { "Cmd" } else { "Super" }.into());
            }
            if modifiers.control {
                parts.push("Ctrl".into());
            }
            if modifiers.alt {
                parts.push(if is_macos { "Option" } else { "Alt" }.into());
            }
            if modifiers.shift {
                parts.push("Shift".into());
            }
            if modifiers.function {
                parts.push("Fn".into());
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

/// Label for a shortcut bound with the `secondary` modifier, e.g.
/// `secondary_shortcut("O")` → "Cmd+O" on macOS, "Ctrl+O" elsewhere.
pub fn secondary_shortcut(suffix: &str) -> String {
    secondary_shortcut_for(suffix, cfg!(target_os = "macos"))
}

fn secondary_shortcut_for(suffix: &str, is_macos: bool) -> String {
    ordered_shortcut_label(&if is_macos {
        format!("Cmd+{suffix}")
    } else {
        format!("Ctrl+{suffix}")
    })
}

/// Label for a shortcut bound with the `alt` modifier. macOS calls this key
/// Option, while the other supported desktop platforms call it Alt.
pub fn alt_shortcut(suffix: &str) -> String {
    alt_shortcut_for(suffix, cfg!(target_os = "macos"))
}

fn alt_shortcut_for(suffix: &str, is_macos: bool) -> String {
    ordered_shortcut_label(&if is_macos {
        format!("Option+{suffix}")
    } else {
        format!("Alt+{suffix}")
    })
}

/// A displayable shortcut attached to a command-palette entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Shortcut {
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
    /// A binding in GPUI keystroke syntax, like `secondary-shift-k`.
    Keystrokes(&'static str),
}

impl Shortcut {
    pub fn label(&self) -> Option<String> {
        self.label_for(cfg!(target_os = "macos"))
    }

    fn label_for(&self, is_macos: bool) -> Option<String> {
        let label = match self {
            Shortcut::None => None,
            Shortcut::Secondary(suffix) => Some(secondary_shortcut_for(suffix, is_macos)),
            Shortcut::Alt(suffix) => Some(alt_shortcut_for(suffix, is_macos)),
            Shortcut::Platform { macos, other } => {
                Some(if is_macos { *macos } else { *other }.to_string())
            }
            Shortcut::MacOs(label) => is_macos.then(|| (*label).to_string()),
            Shortcut::Keystrokes(source) => Some(keystrokes_label_for(source, is_macos)),
        };
        label.map(|label| ordered_shortcut_label(&label))
    }
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
        assert_eq!(platform.label_for(true).as_deref(), Some("Cmd+Ctrl+F"));
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

    #[test]
    fn modifiers_follow_priority_for_every_label_source_and_platform() {
        assert_eq!(
            ordered_shortcut_label("Shift+Option+Ctrl+Cmd+K"),
            "Cmd+Ctrl+Option+Shift+K"
        );
        assert_eq!(
            ordered_shortcut_label("Alt+Shift+Ctrl+K"),
            "Ctrl+Alt+Shift+K"
        );
        assert_eq!(
            keystrokes_label_for("secondary-alt-shift-b", true),
            "Cmd+Option+Shift+B"
        );
        assert_eq!(
            keystrokes_label_for("secondary-alt-shift-b", false),
            "Ctrl+Alt+Shift+B"
        );
        assert_eq!(
            Shortcut::MacOs("Option+Cmd+H").label_for(true).as_deref(),
            Some("Cmd+Option+H")
        );
        assert_eq!(
            ordered_shortcut_label("Shift+Ctrl+K Alt+Cmd+S"),
            "Ctrl+Shift+K Cmd+Alt+S"
        );
    }

    #[test]
    fn keycaps_preserve_plus_keys_and_separate_chords() {
        assert_eq!(keycap_chords("Cmd++"), vec![vec!["Cmd", "+"]]);
        assert_eq!(keycap_chords("+"), vec![vec!["+"]]);
        assert_eq!(
            keycap_chords("Ctrl+K Ctrl+C"),
            vec![vec!["Ctrl", "K"], vec!["Ctrl", "C"]]
        );
    }
}
