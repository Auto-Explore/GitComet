use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, div, px};

/// Toggle-switch visual. It renders state only; the enclosing row owns the
/// click handling, so this stays a plain element rather than a control.
pub fn switch(theme: AppTheme, ui_scale: impl Into<UiScale>, on: bool) -> Div {
    let ui_scale = ui_scale.into();
    let scaled_px = |value: f32| ui_scale.px(value);
    let track_bg = if on {
        theme.colors.accent
    } else {
        with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.35 } else { 0.30 },
        )
    };

    div()
        .w(scaled_px(30.0))
        .h(scaled_px(16.0))
        .flex_none()
        .rounded(px(theme.radii.pill))
        .bg(track_bg)
        .flex()
        .items_center()
        .px(scaled_px(2.0))
        .when(on, |d| d.justify_end())
        .child(
            div()
                .size(scaled_px(12.0))
                .rounded(px(theme.radii.pill))
                .bg(theme.colors.accent_text),
        )
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}
