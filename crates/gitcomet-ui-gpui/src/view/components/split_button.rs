use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, IntoElement, div, px};

use super::control_height;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SplitButtonStyle {
    Filled,
    /// No fill and no outline at rest — the pair reads as two plain toolbar
    /// buttons, held together only by the divider between them, and lights up
    /// with the standard hover overlay.
    Borderless,
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
        let borderless = self.style == SplitButtonStyle::Borderless;
        let bg = match self.style {
            SplitButtonStyle::Filled => theme.colors.surface_bg_elevated,
            SplitButtonStyle::Borderless => gpui::rgba(0x00000000),
        };
        let border_color = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.34 } else { 0.26 },
        );
        let hover_border = with_alpha(
            theme.colors.text_muted,
            if theme.is_dark { 0.55 } else { 0.40 },
        );
        // Without a frame around it the divider is the only thing left holding
        // the pair together, so it stays — just quieter than a real border.
        let divider_color = if borderless {
            with_alpha(
                theme.colors.text_muted,
                if theme.is_dark { 0.24 } else { 0.18 },
            )
        } else {
            with_alpha(border_color, 0.9)
        };
        let hover_bg = theme.hover_overlay();

        let inner = div()
            .flex()
            .items_center()
            .h_full()
            .w_full()
            .rounded(px(theme.radii.control))
            .bg(bg)
            .overflow_hidden()
            // Inset for the frame the filled style draws; without a frame the
            // halves should fill their full height so their hover fills do too.
            .p(if borderless { px(0.0) } else { px(1.0) })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(self.left),
            )
            .child(div().h_full().w(px(1.0)).bg(divider_color))
            .child(div().h_full().flex().items_center().child(self.right));

        let outer = div()
            .flex()
            .items_center()
            .h(control_height(ui_scale))
            .rounded(px(theme.radii.control))
            .bg(gpui::rgba(0x00000000));
        if borderless {
            // The inner buttons carry their own hover states; a hover fill out
            // here would light the whole pair up when only one half is under
            // the cursor.
            outer.child(inner)
        } else {
            outer
                .border_1()
                .border_color(border_color)
                .hover(move |s| s.bg(hover_bg).border_color(hover_border))
                .child(inner)
        }
    }
}

fn with_alpha(mut color: gpui::Rgba, alpha: f32) -> gpui::Rgba {
    color.a = alpha;
    color
}
