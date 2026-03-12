use crate::theme::AppTheme;
use gpui::prelude::*;
use gpui::{Div, div, px};

pub fn diff_stat(theme: AppTheme, added: usize, removed: usize) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py(px(1.0))
        .rounded(px(2.0))
        .bg(theme.colors.surface_bg)
        .border_1()
        .border_color(theme.colors.border)
        .text_xs()
        .child(
            div()
                .text_color(theme.colors.success)
                .child(format!("+{added}")),
        )
        .child(
            div()
                .text_color(theme.colors.danger)
                .child(format!("-{removed}")),
        )
}
