//! Tabs inside a bar (the repository views in the action bar) and read-only
//! selectable fields. A side panel's tabs are a
//! [`PanelHeader`](super::PanelHeader).

use super::{ControlInteractionExt, InteractionState, InteractionStyle};
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Div, FontWeight, Rgba, SharedString, Stateful, StyleRefinement, div, px};

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

/// A tab in a bar of mutually exclusive views, such as the repository views
/// in the action bar: an icon and a label on the bar itself, without a fill,
/// the selected tab underlined with the accent along the bar's bottom edge.
/// As tall as the bar holding it. Attach activation with the interaction
/// kit's `on_activate` after rendering.
pub struct BarTab {
    id: SharedString,
    label: Option<SharedString>,
    icon: Option<SharedString>,
    end_icon: Option<SharedString>,
    selected: bool,
    open: bool,
}

impl BarTab {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            end_icon: None,
            selected: false,
            open: false,
        }
    }

    /// Left out where the bar shows icons only; the caller names the tab in
    /// a tooltip then.
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// A trailing icon, such as the chevron of a tab that opens a menu.
    pub fn end_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.end_icon = Some(icon.into());
        self
    }

    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// The tab's menu is showing.
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    /// The selected tab's label is primary and its icon takes the accent;
    /// the others are secondary until hovered.
    fn colors(&self, theme: AppTheme) -> (Rgba, Rgba) {
        if self.selected {
            (
                theme.colors.foreground.primary,
                theme.colors.accent.foreground,
            )
        } else {
            (
                theme.colors.foreground.secondary,
                theme.colors.foreground.secondary,
            )
        }
    }

    /// Debug selectors `{id}_icon`, `{id}_label` and `{id}_underline` name
    /// the parts.
    pub fn render(self, theme: AppTheme, scale: UiScale) -> Stateful<Div> {
        let (text, icon_color) = self.colors(theme);
        let primary = theme.colors.foreground.primary;
        let id = self.id.clone();
        let selector = id.to_string();
        let icon = |path: SharedString, part: &str| {
            let selector = format!("{id}_{part}");
            gpui::svg()
                .path(path)
                .size(scale.px(14.0))
                .flex_none()
                .text_color(icon_color)
                .debug_selector(move || selector)
        };
        let mut tab = div()
            .id(self.id.clone())
            .debug_selector(move || selector)
            .relative()
            .flex()
            .flex_none()
            .items_center()
            .h_full()
            .gap(scale.px(6.0))
            .px(scale.px(theme.metrics.ramp(9.0, 13.0)))
            .text_size(theme.ui_text(12.0))
            .text_color(text)
            .when(self.selected, |tab| tab.font_weight(FontWeight::MEDIUM))
            .control_interaction(
                InteractionStyle::new(theme)
                    .selection_outline(false)
                    .hover(StyleRefinement::default().text_color(primary))
                    .pressed(StyleRefinement::default().text_color(primary)),
                InteractionState::default().open(self.open),
            );
        if let Some(path) = self.icon {
            tab = tab.child(icon(path, "icon"));
        }
        if let Some(label) = self.label {
            let selector = format!("{id}_label");
            tab = tab.child(
                div()
                    .debug_selector(move || selector)
                    .whitespace_nowrap()
                    .child(label),
            );
        }
        if let Some(path) = self.end_icon {
            tab = tab.child(icon(path, "end_icon"));
        }
        if self.selected {
            let selector = format!("{id}_underline");
            tab = tab.child(
                div()
                    .debug_selector(move || selector)
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(scale.px(2.0))
                    .bg(theme.colors.accent.foreground),
            );
        }
        tab
    }
}

#[cfg(test)]
mod bar_tab_tests {
    use super::*;

    struct Bar {
        theme: AppTheme,
    }

    impl gpui::Render for Bar {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let scale = UiScale::from_percent(100);
            div()
                .flex()
                .items_center()
                .h(px(40.0))
                .child(
                    BarTab::new("bar_on")
                        .icon("icons/history.svg")
                        .label("History")
                        .selected(true)
                        .render(self.theme, scale),
                )
                .child(
                    BarTab::new("bar_off")
                        .icon("icons/history.svg")
                        .render(self.theme, scale),
                )
                .child(
                    BarTab::new("bar_menu")
                        .label("More")
                        .end_icon("icons/chevron_down.svg")
                        .render(self.theme, scale),
                )
        }
    }

    /// Bar tabs fill their bar's height; the selected one alone carries the
    /// underline, along the bottom edge, and a tab without a label shows its
    /// icon only.
    #[gpui::test]
    fn bar_tabs_fill_the_bar_and_underline_the_selected_tab(cx: &mut gpui::TestAppContext) {
        let theme = AppTheme::gitcomet_dark();
        let (_view, cx) = cx.add_window_view(|_, _| Bar { theme });
        crate::test_support::redraw(cx);
        let on = cx.debug_bounds("bar_on").expect("drawn");
        assert_eq!(on.size.height, px(40.0), "as tall as the bar");
        let underline = cx.debug_bounds("bar_on_underline").expect("underlined");
        assert_eq!(underline.bottom(), on.bottom());
        assert_eq!(underline.size.width, on.size.width);
        assert!(cx.debug_bounds("bar_off_underline").is_none());
        assert!(cx.debug_bounds("bar_on_label").is_some());
        assert!(cx.debug_bounds("bar_off_icon").is_some());
        assert!(cx.debug_bounds("bar_off_label").is_none());
        assert!(cx.debug_bounds("bar_menu_end_icon").is_some());
        assert!(cx.debug_bounds("bar_menu_icon").is_none());
    }

    #[test]
    fn a_selected_bar_tab_is_primary_with_an_accent_icon() {
        let theme = AppTheme::gitcomet_light();
        let tab = BarTab::new("tab").label("Review");
        assert_eq!(
            tab.colors(theme),
            (
                theme.colors.foreground.secondary,
                theme.colors.foreground.secondary
            )
        );
        assert_eq!(
            tab.selected(true).colors(theme),
            (
                theme.colors.foreground.primary,
                theme.colors.accent.foreground
            )
        );
    }
}
