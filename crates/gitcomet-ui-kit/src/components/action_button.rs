//! Data-driven actions shared by built-in and contributed toolbars.

use super::split_button::filled_action_frame_border;
use super::{BarButton, ButtonStyle, filled_action_frame, keystrokes_display};
use crate::{theme::AppTheme, tooltip::GitCometTooltipExt, ui_scale::UiScale};
use gpui::prelude::*;
use gpui::{AnyElement, App, SharedString, div, px};
use std::rc::Rc;

pub type ActionRun<C> = Rc<dyn Fn(C, &mut App)>;
pub type ActionAvailability<C> = Rc<dyn Fn(&C) -> bool>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[non_exhaustive]
pub enum ActionButtonStyle {
    #[default]
    Default,
    Primary,
    /// A neutral frame and an accent icon, like the Commit button.
    Framed,
}

/// An action and its presentation. The owner supplies the invocation context
/// when rendering: a selected item, a line range, or any other plain value.
/// Availability uses the same context as invocation. Callbacks run after the
/// current update so they can safely update the view that owns the toolbar.
#[derive(Clone)]
#[non_exhaustive]
pub struct ActionButton<C> {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: Option<SharedString>,
    /// GPUI keystroke syntax, for display. The owner registers the binding.
    pub shortcut: Option<SharedString>,
    pub tooltip: Option<SharedString>,
    pub style: ActionButtonStyle,
    pub enabled: bool,
    pub available: Option<ActionAvailability<C>>,
    pub run: ActionRun<C>,
}

impl<C> ActionButton<C> {
    pub fn new(
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        run: impl Fn(C, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            icon: None,
            shortcut: None,
            tooltip: None,
            style: ActionButtonStyle::Default,
            enabled: true,
            available: None,
            run: Rc::new(run),
        }
    }

    pub fn with_icon(mut self, icon: impl Into<SharedString>) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn with_shortcut(mut self, keystrokes: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keystrokes.into());
        self
    }

    pub fn with_tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some(tooltip.into());
        self
    }

    pub fn primary(mut self) -> Self {
        self.style = ActionButtonStyle::Primary;
        self
    }

    pub fn framed(mut self) -> Self {
        self.style = ActionButtonStyle::Framed;
        self
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn enabled_when(mut self, available: impl Fn(&C) -> bool + 'static) -> Self {
        self.available = Some(Rc::new(available));
        self
    }

    pub fn is_enabled(&self, context: &C) -> bool {
        self.enabled
            && self
                .available
                .as_ref()
                .is_none_or(|available| available(context))
    }
}

impl<C: Clone + 'static> ActionButton<C> {
    /// `id` identifies this mounting of the action (including its debug
    /// selector), so the same action can appear in several toolbars.
    pub fn render(
        &self,
        id: impl Into<SharedString>,
        context: C,
        theme: AppTheme,
        scale: UiScale,
    ) -> AnyElement {
        let id = id.into();
        let enabled = self.is_enabled(&context);
        let shortcut = self.shortcut.as_deref().map(keystrokes_display);
        let mut button = BarButton::new(id.clone(), self.label.clone())
            .primary(self.style != ActionButtonStyle::Default)
            .enabled(enabled);
        if let Some(icon) = &self.icon {
            button = button.icon(icon.clone());
        }
        if let Some(shortcut) = &shortcut {
            button = button.shortcut(shortcut.clone());
        }
        let run = Rc::clone(&self.run);
        let frame_selector = format!("{id}_frame");
        let button = button.into_button(theme, scale);
        let button = if self.style == ActionButtonStyle::Framed {
            button.style(ButtonStyle::Subtle)
        } else {
            button
        };
        let button = button
            .on_click_handler(theme, scale, move |_, _, cx| {
                if enabled {
                    let run = Rc::clone(&run);
                    let context = context.clone();
                    cx.defer(move |cx| run(context, cx));
                }
            })
            .debug_selector(move || id.to_string())
            .gitcomet_tooltip_keyed(
                theme,
                self.tooltip.clone().unwrap_or_else(|| self.label.clone()),
                shortcut.into_iter().map(SharedString::from).collect(),
            )
            .into_any_element();
        if self.style == ActionButtonStyle::Framed {
            // The frame dims with the button, or a disabled action still reads as live.
            let disabled = !enabled;
            filled_action_frame(theme, scale)
                .border_color(theme.disabled_color_if(disabled, filled_action_frame_border(theme)))
                .debug_selector(move || frame_selector)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .h_full()
                        .rounded(px(theme.radii.control))
                        .bg(theme.disabled_color_if(disabled, theme.colors.surface.raised))
                        .overflow_hidden()
                        .child(button),
                )
                .into_any_element()
        } else {
            button
        }
    }
}

impl<C> std::fmt::Debug for ActionButton<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ActionButton")
            .field("id", &self.id)
            .field("label", &self.label)
            .field("icon", &self.icon)
            .field("shortcut", &self.shortcut)
            .field("tooltip", &self.tooltip)
            .field("style", &self.style)
            .field("enabled", &self.enabled)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{
        assert_color_close, color_close, painted_control_quads, painted_fills_inside, redraw,
        solid_color,
    };
    use gpui::{Context, Render, TestAppContext, Window};

    struct FramedPair(AppTheme);

    impl Render for FramedPair {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let action = |enabled| {
                ActionButton::new("save", "Save", |(), _| {})
                    .framed()
                    .enabled(enabled)
            };
            let scale = UiScale::from_percent(100);
            div()
                .flex()
                .child(action(true).render("on", (), self.0, scale))
                .child(action(false).render("off", (), self.0, scale))
        }
    }

    #[gpui::test]
    fn a_disabled_framed_action_dims_its_frame_and_fill(cx: &mut TestAppContext) {
        for theme in [
            AppTheme::from_key(crate::theme::DEFAULT_DARK_THEME_KEY).unwrap(),
            AppTheme::gitcomet_light(),
        ] {
            let (_view, cx) = cx.add_window_view(move |_, _| FramedPair(theme));
            redraw(cx);
            let border = |cx: &mut gpui::VisualTestContext, selector| {
                painted_control_quads(cx, selector)
                    .first()
                    .and_then(|(_, border)| solid_color(border))
            };
            let enabled = border(cx, "on_frame");
            assert!(enabled.is_some());
            assert_color_close(
                border(cx, "off_frame"),
                enabled.map(|color| theme.disabled_color(color)),
            );

            let raised = theme.colors.surface.raised;
            let has =
                |fills: &[gpui::Rgba], color| fills.iter().any(|fill| color_close(*fill, color));
            let enabled = painted_fills_inside(cx, "on_frame");
            let disabled = painted_fills_inside(cx, "off_frame");
            assert!(has(&enabled, raised), "{enabled:?}");
            assert!(has(&disabled, theme.disabled_color(raised)), "{disabled:?}");
            assert!(!has(&disabled, raised), "{disabled:?}");
        }
    }
}
