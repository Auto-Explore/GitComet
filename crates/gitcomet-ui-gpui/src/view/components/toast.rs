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
    let status = match kind {
        ToastKind::Success => theme.colors.status.success,
        ToastKind::Warning => theme.colors.status.warning,
        ToastKind::Error => theme.colors.status.danger,
    };
    let (accent, bg, border) = if theme.is_dark {
        (
            with_alpha(status.foreground, 0.85),
            with_alpha(theme.colors.surface.raised, 0.96),
            with_alpha(status.foreground, 0.55),
        )
    } else {
        (status.foreground, status.background, status.border)
    };

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
        .text_color(theme.colors.foreground.primary)
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
