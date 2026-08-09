use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, div};

/// Keycap chips for a shortcut label such as `Ctrl+Shift+W`. The label is split
/// on `+` so each key gets its own chip, keeping menu rows and command-palette
/// rows visually identical.
pub fn shortcut_keys(label: &str, theme: AppTheme, scale: impl Into<UiScale>) -> Div {
    let scale = scale.into();
    let chip_bg = theme.hover_overlay();
    div()
        .debug_selector(|| "shortcut_keycaps".to_string())
        .flex()
        .items_center()
        .flex_shrink_0()
        .gap(scale.px(4.0))
        .children(label.split('+').map(move |key| {
            div()
                .min_w(scale.px(22.0))
                .h(scale.px(22.0))
                .px(scale.px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(scale.px(4.0))
                .bg(chip_bg)
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_xs()
                .line_height(scale.px(14.0))
                .text_color(theme.colors.text_muted)
                .child(key.to_owned())
        }))
}
