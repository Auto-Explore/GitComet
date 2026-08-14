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
    // The status colour lives in the accent stripe and the border; the panel
    // itself stays a neutral elevated surface. A status-tinted background reads
    // as a coloured card rather than as a notification, and on the light theme
    // `status.warning.background` is a distinctly orange one. Matches
    // `render_progress_shell`, which is the same shell with a bar in it.
    let bg = with_alpha(
        theme.colors.surface.raised,
        if theme.is_dark { 0.96 } else { 0.98 },
    );
    let (accent, border) = if theme.is_dark {
        (
            with_alpha(status.foreground, 0.85),
            with_alpha(status.foreground, 0.55),
        )
    } else {
        (status.foreground, status.border)
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
