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
    let scale = scale.into();
    keycaps(
        label,
        scale,
        scale.px(COMPACT_KEYCAP_HEIGHT_PX),
        Keycap {
            pad: 4.0,
            text: 11.0,
            line: 12.0,
            fg,
            bg: crate::theme::with_alpha(fg, 0.14),
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
        .gap(scale.px(4.0))
        .children(label.split('+').map(move |key| {
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
                .child(key.to_owned())
        }))
}

/// Format GPUI keystrokes for the current platform.
pub fn keystrokes_display(source: &str) -> String {
    keystrokes_label_for(source, cfg!(target_os = "macos"))
}

pub fn keystrokes_label_for(source: &str, is_macos: bool) -> String {
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
