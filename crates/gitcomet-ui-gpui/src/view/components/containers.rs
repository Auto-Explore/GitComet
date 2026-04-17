use crate::theme::AppTheme;
use crate::ui_scale;
use gpui::prelude::*;
use gpui::{Div, FontWeight, SharedString, div, px};

#[cfg(test)]
use super::CONTROL_HEIGHT_MD_PX;
use super::CONTROL_HEIGHT_PX;
#[cfg(test)]
use gpui::IntoElement;

#[cfg(test)]
pub fn panel(
    theme: AppTheme,
    title: impl Into<SharedString>,
    subtitle: Option<SharedString>,
    content: impl IntoElement,
) -> Div {
    let title: SharedString = title.into();
    let show_header = !title.as_ref().is_empty() || subtitle.is_some();
    let mut header = div()
        .flex()
        .items_center()
        .justify_between()
        .h(px(CONTROL_HEIGHT_MD_PX))
        .px_2()
        .border_b_1()
        .border_color(theme.colors.border)
        .bg(theme.colors.surface_bg_elevated)
        .child(div().text_sm().font_weight(FontWeight::BOLD).child(title));

    if let Some(subtitle) = subtitle {
        header = header.child(
            div()
                .text_xs()
                .text_color(theme.colors.text_muted)
                .child(subtitle),
        );
    }

    div()
        .flex()
        .flex_col()
        .bg(theme.colors.surface_bg)
        .border_1()
        .border_color(theme.colors.border)
        .rounded(px(theme.radii.panel))
        .overflow_hidden()
        .when(show_header, |this| this.child(header))
        .child(
            div().flex().flex_col().flex_1().min_h(px(0.0)).p_2().child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h(px(0.0))
                    .child(content),
            ),
        )
}

#[cfg(test)]
pub fn pill(theme: AppTheme, label: impl Into<SharedString>, bg: gpui::Rgba) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded(px(theme.radii.pill))
        .bg(bg)
        .text_xs()
        .text_color(theme.colors.text)
        .child(label.into())
}

pub fn empty_state(
    theme: AppTheme,
    title: impl Into<SharedString>,
    message: impl Into<SharedString>,
) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .px_2()
        .py_4()
        .child(
            div()
                .text_lg()
                .font_weight(FontWeight::BOLD)
                .text_color(theme.colors.text)
                .child(title.into()),
        )
        .child(
            div()
                .text_sm()
                .text_color(theme.colors.text_muted)
                .child(message.into()),
        )
}

#[allow(dead_code)]
pub fn split_columns_header(
    theme: AppTheme,
    left: impl Into<SharedString>,
    right: impl Into<SharedString>,
) -> Div {
    split_columns_header_scaled(theme, ui_scale::DEFAULT_UI_SCALE_PERCENT, left, right)
}

pub fn split_columns_header_scaled(
    theme: AppTheme,
    ui_scale_percent: u32,
    left: impl Into<SharedString>,
    right: impl Into<SharedString>,
) -> Div {
    div()
        .h(ui_scale::design_px_from_percent(
            CONTROL_HEIGHT_PX,
            ui_scale_percent,
        ))
        .flex()
        .items_center()
        .text_xs()
        .text_color(theme.colors.text_muted)
        .bg(theme.colors.surface_bg_elevated)
        .border_b_1()
        .border_color(theme.colors.border)
        .child(div().flex_1().min_w(px(0.0)).px_2().child(left.into()))
        .child(div().w(px(1.0)).h_full().bg(theme.colors.border))
        .child(div().flex_1().min_w(px(0.0)).px_2().child(right.into()))
}
