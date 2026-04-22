use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, IntoElement, div, px};

use super::control_height;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitButtonStyle {
    Filled,
    Outlined,
}

pub struct SplitButton {
    left: AnyElement,
    right: AnyElement,
    style: SplitButtonStyle,
}

impl SplitButton {
    pub fn new(left: impl IntoElement, right: impl IntoElement) -> Self {
        Self {
            left: left.into_any_element(),
            right: right.into_any_element(),
            style: SplitButtonStyle::Filled,
        }
    }

    pub fn style(mut self, style: SplitButtonStyle) -> Self {
        self.style = style;
        self
    }

    pub fn render(self, theme: AppTheme, ui_scale: impl Into<UiScale>) -> Div {
        let ui_scale = ui_scale.into();
        let bg = match self.style {
            SplitButtonStyle::Filled => theme.colors.surface_bg_elevated,
            SplitButtonStyle::Outlined => gpui::rgba(0x00000000),
        };
        let border_color = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.34 } else { 0.26 },
        );
        let hover_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.55 } else { 0.40 },
        );
        let hover_bg = with_alpha(theme.colors.text, if theme.is_dark { 0.05 } else { 0.04 });

        let inner = div()
            .flex()
            .items_center()
            .h_full()
            .w_full()
            .rounded(px(theme.radii.row))
            .bg(bg)
            .overflow_hidden()
            .p(px(1.0))
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(self.left),
            )
            .child(div().h_full().w(px(1.0)).bg(with_alpha(border_color, 0.9)))
            .child(div().h_full().flex().items_center().child(self.right));

        div()
            .flex()
            .items_center()
            .h(control_height(ui_scale))
            .rounded(px(theme.radii.row))
            .bg(gpui::rgba(0x00000000))
            .border_1()
            .border_color(border_color)
            .when(self.style == SplitButtonStyle::Filled, |this| {
                this.shadow_sm()
            })
            .hover(move |s| s.bg(hover_bg).border_color(hover_border))
            .child(inner)
    }
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}
