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

    /// The icon follows the label: the selected foreground when selected.
    fn icon_color(&self, theme: AppTheme) -> Rgba {
        if self.selected {
            theme.colors.interaction.selected_foreground
        } else {
            theme.colors.foreground.secondary
        }
    }

    /// Debug selectors `{id}_icon` and `{id}_badge` name the parts.
    pub fn render(self, theme: AppTheme, scale: UiScale) -> gpui::Stateful<Div> {
        let icon_color = self.icon_color(theme);
        let id = self.id.clone();
        let mut button = navigation_tab(self.id, self.label, self.selected, None, theme);
        if let Some(icon) = self.icon {
            let selector = format!("{id}_icon");
            button = button.start_slot(
                gpui::svg()
                    .path(icon)
                    .size(scale.px(14.0))
                    .flex_none()
                    .text_color(icon_color)
                    .debug_selector(move || selector),
            );
        }
        if let Some(badge) = self.badge {
            let selector = format!("{id}_badge");
            button = button.end_slot(
                div()
                    .debug_selector(move || selector)
                    .flex_none()
                    .px(scale.px(4.0))
                    .rounded(scale.px(8.0))
                    .bg(theme.colors.surface.raised)
                    .text_size(theme.ui_text(11.0))
                    .text_color(theme.colors.foreground.secondary)
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

#[cfg(test)]
mod nav_tab_tests {
    use super::*;

    struct Tabs {
        theme: AppTheme,
    }

    impl gpui::Render for Tabs {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let scale = UiScale::from_percent(100);
            navigation_tab_strip(self.theme.colors.surface.canvas, scale)
                .child(
                    NavTab::new("tab_on", "History")
                        .icon("icons/history.svg")
                        .selected(true)
                        .render(self.theme, scale),
                )
                .child(
                    NavTab::new("tab_off", "Review")
                        .badge("3")
                        .render(self.theme, scale),
                )
        }
    }

    /// Icons and badges are named parts; the selected tab alone is
    /// underlined with the accent.
    #[gpui::test]
    fn nav_tabs_name_their_parts_and_underline_the_selected_tab(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        let theme = AppTheme::gitcomet_dark();
        let (_view, cx) = cx.add_window_view(|_, _| Tabs { theme });
        crate::test_support::redraw(cx);
        assert!(cx.debug_bounds("tab_on_icon").is_some());
        assert!(cx.debug_bounds("tab_off_icon").is_none());
        assert!(cx.debug_bounds("tab_off_badge").is_some());
        assert!(cx.debug_bounds("tab_on_badge").is_none());
        let borders = |cx: &mut gpui::VisualTestContext, selector| {
            crate::test_support::painted_control_quads(cx, selector)
                .into_iter()
                .map(|(_, border)| border)
                .collect::<Vec<_>>()
        };
        let accent = gpui::Background::from(theme.colors.accent.foreground);
        assert!(borders(cx, "tab_on").contains(&accent));
        assert!(!borders(cx, "tab_off").contains(&accent));
    }

    #[test]
    fn a_selected_tabs_icon_takes_the_selected_foreground() {
        let theme = AppTheme::gitcomet_light();
        assert_ne!(
            theme.colors.foreground.secondary,
            theme.colors.interaction.selected_foreground
        );
        let tab = NavTab::new("tab", "History").icon("icons/history.svg");
        assert_eq!(tab.icon_color(theme), theme.colors.foreground.secondary);
        assert_eq!(
            tab.selected(true).icon_color(theme),
            theme.colors.interaction.selected_foreground
        );
    }
}
