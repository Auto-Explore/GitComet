use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, IntoElement, div, px};

use super::{control_height, split_button_divider_height};

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
    /// Construct both segments through Button's shared interaction and callback
    /// handling. Only the menu segment needs anchor bounds.
    pub fn action_menu<V: 'static>(
        main: super::Button,
        menu: super::Button,
        theme: AppTheme,
        cx: &mut gpui::Context<V>,
        action: impl Fn(&mut V, &gpui::ClickEvent, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
        open_menu: impl Fn(
            &mut V,
            &gpui::ClickEvent,
            gpui::Bounds<gpui::Pixels>,
            &mut gpui::Window,
            &mut gpui::Context<V>,
        ) + 'static,
    ) -> Self {
        Self::from_buttons(
            main,
            menu,
            cx,
            |button, cx| button.on_click(theme, cx, action),
            |button, cx| button.on_click_with_bounds(theme, cx, open_menu),
        )
    }

    /// Keep segment geometry shared when callers add per-segment tooltips or
    /// other content around their Button handlers.
    pub fn from_buttons<V: 'static, A: IntoElement, M: IntoElement>(
        main: super::Button,
        menu: super::Button,
        cx: &mut gpui::Context<V>,
        render_main: impl FnOnce(super::Button, &mut gpui::Context<V>) -> A,
        render_menu: impl FnOnce(super::Button, &mut gpui::Context<V>) -> M,
    ) -> Self {
        Self::new(
            render_main(main.rounded_left(), cx),
            render_menu(menu.rounded_right(), cx),
        )
    }

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
        let ui_scale = ui_scale.into().with_appearance(theme.metrics);
        let borderless = self.style == SplitButtonStyle::Borderless;
        let bg = match self.style {
            SplitButtonStyle::Filled => theme.colors.surface.raised,
            SplitButtonStyle::Borderless => gpui::rgba(0x00000000),
        };
        let border_color = with_alpha(
            theme.colors.foreground.secondary,
            if theme.is_dark { 0.34 } else { 0.26 },
        );
        // Without a frame around it the divider is the only thing left holding
        // the pair together, so it stays — just quieter than a real border.
        let divider_color = if borderless {
            with_alpha(
                theme.colors.foreground.secondary,
                if theme.is_dark { 0.24 } else { 0.18 },
            )
        } else {
            border_color
        };

        let inner = div()
            .flex()
            .items_center()
            .h_full()
            .w_full()
            .rounded(px(theme.radii.control))
            .bg(bg)
            .overflow_hidden()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .flex()
                    .items_center()
                    .child(self.left),
            )
            // A short centered tick, not a rule: the halves draw their own hover
            // borders now, and a full-height divider would collide with them.
            .child(
                div()
                    .h(split_button_divider_height(ui_scale))
                    .w(px(1.0))
                    .bg(divider_color),
            )
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
            // Frame at rest only, for the same reason: the halves own hover, and
            // the hovered one's border sits directly inside this one.
            outer.border_1().border_color(border_color).child(inner)
        }
    }
}

use crate::theme::with_alpha;
