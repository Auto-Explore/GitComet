use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, div};

/// Plain `+N` / `-N` counters. Each part right-aligns inside a fixed minimum
/// width so consecutive rows line up into two tidy columns instead of a
/// ragged ("waving") edge — the colored text carries the meaning without a
/// chip background.
pub fn diff_stat(theme: AppTheme, scale: impl Into<UiScale>, added: usize, removed: usize) -> Div {
    let scale = scale.into();
    // Fits "+9999"; wider stats simply grow and stay right-aligned.
    let part_min_w = scale.px(30.0);
    let part = |text: String, color: gpui::Rgba| {
        div()
            .min_w(part_min_w)
            .flex()
            .justify_end()
            .text_xs()
            .text_color(color)
            .child(text)
    };
    div()
        .flex()
        .items_center()
        .flex_none()
        .gap_1()
        .child(part(
            format!("+{added}"),
            theme.colors.diff.added.foreground,
        ))
        .child(part(
            format!("-{removed}"),
            theme.colors.diff.removed.foreground,
        ))
}

/// [`diff_stat`] for lists where some rows have no counts. Always reserves the
/// two columns: a row that rendered nothing would let its label run wider and
/// drag the shared truncation anchor.
pub fn diff_stat_optional(
    theme: AppTheme,
    scale: impl Into<UiScale>,
    added: Option<u32>,
    removed: Option<u32>,
) -> Div {
    let scale = scale.into();
    let part_min_w = scale.px(30.0);
    let part = |text: String, color: gpui::Rgba| {
        div()
            .min_w(part_min_w)
            .flex()
            .justify_end()
            .text_xs()
            .text_color(color)
            .child(text)
    };
    div()
        .flex()
        .items_center()
        .flex_none()
        .gap_1()
        .child(part(
            added.map(|n| format!("+{n}")).unwrap_or_default(),
            theme.colors.diff.added.foreground,
        ))
        .child(part(
            removed.map(|n| format!("-{n}")).unwrap_or_default(),
            theme.colors.diff.removed.foreground,
        ))
}
