use crate::theme::{AppTheme, StatusColorSet};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, div, px};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToastKind {
    Success,
    Warning,
    Error,
}

/// Every toast is this wide, so a stack of them lines up.
pub const TOAST_WIDTH_PX: f32 = 420.0;
/// The status badge; also the height of a toast's first line.
pub const TOAST_BADGE_PX: f32 = 28.0;

fn status(theme: AppTheme, kind: ToastKind) -> (StatusColorSet, &'static str) {
    match kind {
        ToastKind::Success => (theme.colors.status.success, "icons/circle_check.svg"),
        ToastKind::Warning => (theme.colors.status.warning, "icons/warning.svg"),
        ToastKind::Error => (theme.colors.status.danger, "icons/circle_alert.svg"),
    }
}

/// The card and edge colours of a toast of `kind`.
pub(crate) fn toast_surface(theme: AppTheme, kind: ToastKind) -> (gpui::Rgba, gpui::Rgba) {
    let raised = theme.colors.surface.raised;
    match kind {
        // Errors stay until closed, so they also warm the card and its edge
        // to read as errors at a glance.
        ToastKind::Error => {
            let danger = theme.colors.status.danger.foreground;
            (
                composite_over(
                    raised,
                    with_alpha(danger, if theme.is_dark { 0.07 } else { 0.035 }),
                ),
                with_alpha(danger, if theme.is_dark { 0.45 } else { 0.38 }),
            )
        }
        ToastKind::Success | ToastKind::Warning => (raised, theme.colors.stroke.default),
    }
}

/// A notification card: a status badge beside the content, with room on the
/// right for the close button the host places there. The status is the badge,
/// not an edge stripe, which the rounded card would clip.
pub fn toast(
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    kind: ToastKind,
    message: impl IntoElement,
) -> Div {
    let ui_scale = ui_scale.into();
    let (status, icon) = status(theme, kind);
    let (bg, border) = toast_surface(theme, kind);
    let badge = div()
        .flex_none()
        .size(ui_scale.px(TOAST_BADGE_PX))
        .rounded(px(999.0))
        .flex()
        .items_center()
        .justify_center()
        .bg(with_alpha(
            status.foreground,
            if theme.is_dark { 0.16 } else { 0.12 },
        ))
        .child(crate::view::icons::svg_icon(
            icon,
            status.foreground,
            ui_scale.px(16.0),
        ));

    div()
        .w(ui_scale.px(TOAST_WIDTH_PX))
        .flex()
        .items_start()
        .gap(ui_scale.px(12.0))
        .pl(ui_scale.px(14.0))
        .pr(ui_scale.px(40.0))
        .py(ui_scale.px(12.0))
        .bg(bg)
        .border_1()
        .border_color(border)
        .rounded(px(theme.radii.popover))
        .shadow(crate::theme::shadow_popover(theme))
        .text_size(theme.ui_text(14.0))
        .text_color(theme.colors.foreground.primary)
        .child(badge)
        .child(div().flex_1().min_w(px(0.0)).child(message))
}

use crate::theme::{composite_over, with_alpha};
