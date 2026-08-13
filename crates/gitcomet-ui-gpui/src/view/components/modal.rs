use crate::theme::{AppTheme, with_alpha};
use gpui::prelude::*;
use gpui::{Div, Stateful, div, px};

/// Full-window backdrop shared by modal overlays.
///
/// Matching the application window radius keeps the backdrop inside the
/// rounded window silhouette on platforms with client-side decorations.
pub fn modal_scrim(theme: AppTheme) -> Stateful<Div> {
    div()
        .id("modal_scrim")
        .debug_selector(|| "modal_scrim".to_string())
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .rounded(px(theme.radii.window))
        .bg(with_alpha(
            theme.colors.shadow,
            if theme.is_dark { 0.35 } else { 0.22 },
        ))
        .occlude()
}

/// Shared visual shell for scrim-backed modal content.
pub fn modal_surface(theme: AppTheme) -> Div {
    div()
        .bg(theme.colors.surface_bg_elevated)
        .border_1()
        .border_color(theme.colors.border)
        .rounded(px(theme.radii.popover))
        .shadow(crate::theme::shadow_modal(theme))
        .overflow_hidden()
}

/// Shared visual shell for anchored surfaces — popovers and the menus that
/// float over them. The lighter lift of the two: it hovers just above the
/// content rather than sitting on a scrim like [`modal_surface`].
///
/// A floating menu that skips this is invisible in the worst way — it still
/// lays out and still takes clicks, but paints straight onto whatever is
/// underneath.
pub fn popover_surface(theme: AppTheme) -> Div {
    div()
        .bg(theme.colors.surface_bg_elevated)
        .border_1()
        .border_color(theme.colors.border)
        .rounded(px(theme.radii.popover))
        .shadow(crate::theme::shadow_popover(theme))
        .overflow_hidden()
}
