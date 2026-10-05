//! The pieces of a diff's bottom bar: the row itself, the arrow buttons that
//! step through files (and, in the header, through changes), and labelled
//! buttons that show their shortcut as keycaps. Shared so every bar, the
//! host's and an extension's own, looks the same.

use super::{Button, ButtonStyle, shortcut_keys_compact};
use crate::icons::svg_icon;
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{Div, SharedString, div};

/// The bar's row: the header's band colour, a rule above, controls centred.
pub fn bottom_bar(theme: AppTheme, scale: UiScale) -> Div {
    div()
        .flex()
        .items_center()
        .gap(scale.px(6.0))
        .px(scale.px(8.0))
        .py(scale.px(3.0))
        .bg(crate::theme::content_header_bg(theme))
        .border_t_1()
        .border_color(theme.colors.stroke.default)
}

/// A borderless icon button for stepping: previous/next file in the bar,
/// previous/next change in the header. Disabled at either end. The icon's
/// debug selector is `{id}_icon`.
pub fn nav_arrow_button(
    id: impl Into<SharedString>,
    icon: &'static str,
    theme: AppTheme,
    scale: UiScale,
    enabled: bool,
) -> Button {
    let color = if enabled {
        theme.colors.foreground.primary
    } else {
        theme.colors.foreground.disabled
    };
    let id: SharedString = id.into();
    let icon_selector = format!("{id}_icon");
    Button::new(id, "")
        .borderless()
        .style(ButtonStyle::Subtle)
        .start_slot(svg_icon(icon, color, scale.px(14.0)).debug_selector(move || icon_selector))
        .disabled(!enabled)
}

/// A labelled bar button: an optional icon, the label, and the shortcut
/// that runs it as compact keycaps. `primary` is the bar's main action.
pub struct BarButton {
    id: SharedString,
    label: SharedString,
    icon: Option<SharedString>,
    shortcut: Option<SharedString>,
    primary: bool,
    enabled: bool,
}

impl BarButton {
    pub fn new(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            primary: false,
            enabled: true,
        }
    }

    pub fn icon(mut self, path: impl Into<SharedString>) -> Self {
        self.icon = Some(path.into());
        self
    }

    /// As displayed, keys joined by `+` (e.g. "Space", "Ctrl+S").
    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn primary(mut self, primary: bool) -> Self {
        self.primary = primary;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// The button, ready for `on_click` and a tooltip.
    pub fn into_button(self, theme: AppTheme, scale: UiScale) -> Button {
        let text = if !self.enabled {
            theme.colors.foreground.disabled
        } else if self.primary {
            theme.colors.accent.foreground
        } else {
            theme.colors.foreground.secondary
        };
        let mut button = Button::new(self.id, self.label)
            .style(if self.primary {
                ButtonStyle::Filled
            } else {
                ButtonStyle::Subtle
            })
            .disabled(!self.enabled);
        if !self.primary {
            button = button.borderless();
        }
        if let Some(icon) = self.icon {
            button = button.start_slot(svg_icon(icon, text, scale.px(14.0)));
        }
        if let Some(shortcut) = self.shortcut {
            button = button.end_slot(shortcut_keys_compact(&shortcut, text, scale));
        }
        button
    }
}
