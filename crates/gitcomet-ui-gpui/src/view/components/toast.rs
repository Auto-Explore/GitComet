use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{Div, div, px};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastKind {
    Success,
    Warning,
    Error,
}

pub fn toast(theme: AppTheme, kind: ToastKind, message: impl IntoElement) -> Div {
    let (accent, bg, border) = match kind {
        ToastKind::Success => (
            theme.colors.success,
            with_alpha(
                theme.colors.surface_bg_elevated,
                if theme.is_dark { 0.96 } else { 0.98 },
            ),
            with_alpha(
                theme.colors.success,
                if theme.is_dark { 0.55 } else { 0.45 },
            ),
        ),
        ToastKind::Warning => (
            theme.colors.warning,
            with_alpha(
                theme.colors.surface_bg_elevated,
                if theme.is_dark { 0.96 } else { 0.98 },
            ),
            with_alpha(
                theme.colors.warning,
                if theme.is_dark { 0.55 } else { 0.45 },
            ),
        ),
        ToastKind::Error => (
            theme.colors.danger,
            with_alpha(
                theme.colors.surface_bg_elevated,
                if theme.is_dark { 0.96 } else { 0.98 },
            ),
            with_alpha(theme.colors.danger, if theme.is_dark { 0.55 } else { 0.45 }),
        ),
    };

    let accent = with_alpha(accent, if theme.is_dark { 0.85 } else { 0.75 });

    div()
        .min_w(px(360.0))
        .max_w(px(900.0))
        .flex()
        .gap(px(12.0))
        .bg(bg)
        .border_1()
        .border_color(border)
        .rounded(px(theme.radii.popover))
        .overflow_hidden()
        .shadow(crate::theme::shadow_popover(theme))
        .text_lg()
        .text_color(theme.colors.text)
        .child(div().w(px(5.0)).bg(accent).flex_shrink_0())
        .child(
            div()
                .flex_1()
                .pl(px(16.0))
                .pr(px(48.0))
                .py(px(12.0))
                .child(message),
        )
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}
