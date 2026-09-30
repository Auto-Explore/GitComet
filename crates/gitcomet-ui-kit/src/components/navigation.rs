//! Navigation tabs: a strip of mutually exclusive views (for example a
//! sidebar's Branches and Files), and read-only selectable fields.

use super::{Button, content_header_height, control_height};
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, Rgba, SharedString, div, px};

/// One navigation tab. `selected_bg` overrides the selected fill (for a strip
/// tinted by its content); the caller attaches the click handler.
pub fn navigation_tab(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    selected: bool,
    selected_bg: Option<Rgba>,
    theme: AppTheme,
) -> Button {
    Button::new(id, label)
        .borderless()
        .truncate_label()
        .selected(selected)
        .selected_bg(selected_bg.unwrap_or(theme.colors.interaction.selected_background))
        .text_color(if selected {
            theme.colors.interaction.selected_foreground
        } else {
            theme.colors.foreground.secondary
        })
}

/// Sizes a rendered [`navigation_tab`] on the density ramp.
pub fn navigation_tab_metrics<E: Styled>(tab: E, theme: AppTheme, ui_scale: UiScale) -> E {
    tab.px(ui_scale.px(theme.metrics.ramp(8.0, 12.0)))
        .h(control_height(ui_scale))
        .text_size(theme.ui_text(12.0))
}

/// The strip holding navigation tabs, as tall as a content header.
pub fn navigation_tab_strip(background: Rgba, ui_scale: UiScale) -> Div {
    div()
        .flex()
        .flex_row()
        .items_center()
        .gap(ui_scale.px(2.0))
        .w_full()
        .h(content_header_height(ui_scale))
        .px(ui_scale.px(4.0))
        .bg(background)
}

/// A labelled read-only value; pair it with a selectable text input
/// ([`crate::TextInputOptions::selectable`]) so the value can be copied.
pub fn selectable_field(theme: AppTheme, label: impl Into<SharedString>, value: AnyElement) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .child(
            div()
                .text_size(theme.ui_text(14.0))
                .text_color(theme.colors.foreground.secondary)
                .child(label.into()),
        )
        .child(
            div()
                .w_full()
                .min_w(px(0.0))
                .text_size(theme.ui_text(14.0))
                .child(value),
        )
}

/// A navigation tab with an optional icon and badge and a selected underline.
/// Attach activation with the interaction kit's `on_activate` after rendering.
pub struct NavTab {
    id: SharedString,
    label: SharedString,
    icon: Option<SharedString>,
    badge: Option<SharedString>,
    selected: bool,
}

impl NavTab {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            badge: None,
            selected: false,
        }
    }

    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn badge(mut self, badge: impl Into<SharedString>) -> Self {
        self.badge = Some(badge.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    pub fn render(self, theme: AppTheme, scale: UiScale) -> gpui::Stateful<Div> {
        let mut button = navigation_tab(self.id, self.label, self.selected, None, theme);
        if let Some(icon) = self.icon {
            button = button.start_slot(
                gpui::svg()
                    .path(icon)
                    .size(scale.px(14.0))
                    .text_color(theme.colors.foreground.secondary),
            );
        }
        if let Some(badge) = self.badge {
            button = button.end_slot(
                div()
                    .px(scale.px(4.0))
                    .rounded(scale.px(8.0))
                    .bg(theme.colors.surface.raised)
                    .child(badge),
            );
        }
        navigation_tab_metrics(button.render(theme, scale), theme, scale)
            .border_b_2()
            .border_color(if self.selected {
                theme.colors.accent.foreground
            } else {
                gpui::rgba(0x00000000)
            })
    }
}
