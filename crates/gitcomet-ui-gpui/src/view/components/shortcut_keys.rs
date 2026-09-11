use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, div};

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
    let chip_bg = theme.hover_overlay();
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
                .px(scale.px(6.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded(scale.px(4.0))
                .bg(chip_bg)
                .font_family(crate::font_preferences::EDITOR_MONOSPACE_FONT_FAMILY)
                .text_size(scale.ui_text(12.0))
                .line_height(scale.px(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child(key.to_owned())
        }))
}
