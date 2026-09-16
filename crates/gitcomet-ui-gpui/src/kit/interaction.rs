//! Interaction policy for discrete controls. Geometry and content belong to
//! their component; pointer feedback, persistent state and focus cues live here.
use crate::theme::{AppTheme, composite_over, with_alpha};
use gpui::prelude::*;
use gpui::{BoxShadow, CursorStyle, Div, Refineable, Rgba, Stateful, StyleRefinement, point, px};
use palette::IntoColor;

pub const REMOVE_BUTTON_HOVER_ALPHA: f32 = 0.18;
pub const REMOVE_BUTTON_PRESSED_ALPHA: f32 = 0.26;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum InteractionFeedback {
    #[default]
    Resting,
    Hovered,
    Pressed,
}

/// Ordinary actions activate on a completed click (including Enter/Space).
/// Composite rows leave focus management to their owner; suggestions preserve
/// input focus until activation. Nested actions also consume the press so that
/// the enclosing row cannot act on it. Tab can focus them for keyboard use.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControlActivation {
    Action,
    Nested,
    /// A leaf control whose renderer already configured its focus handle.
    ManagedFocus,
    Composite,
    PreserveFocus,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct InteractionState {
    selected: Option<Rgba>,
    open: bool,
    busy: bool,
    disabled: bool,
}

impl InteractionState {
    pub fn selected(mut self, selected: bool, background: Rgba) -> Self {
        self.selected = selected.then_some(background);
        self
    }

    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }

    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// The same ring is used for keyboard focus and light-theme selection, with
/// different theme colors. An inset shadow preserves geometry and split edges.
pub fn interaction_outline(color: Rgba) -> BoxShadow {
    BoxShadow {
        color: color.into(),
        offset: point(px(0.0), px(0.0)),
        blur_radius: px(0.0),
        spread_radius: px(1.0),
        inset: true,
    }
}

pub fn control_open_background(theme: AppTheme) -> Rgba {
    with_alpha(
        theme.colors.accent.foreground,
        if theme.is_dark { 0.26 } else { 0.20 },
    )
}

#[derive(Clone)]
pub struct InteractionStyle {
    theme: AppTheme,
    resting: Option<Rgba>,
    persistent: Rgba,
    hover: StyleRefinement,
    pressed: StyleRefinement,
    pointer_feedback: bool,
    selection_outline: bool,
    disabled_opacity: f32,
}

impl InteractionStyle {
    pub fn new(theme: AppTheme) -> Self {
        Self {
            theme,
            resting: None,
            persistent: theme.active_overlay(),
            hover: StyleRefinement::default().bg(theme.hover_overlay()),
            pressed: StyleRefinement::default().bg(theme.active_overlay()),
            pointer_feedback: true,
            selection_outline: true,
            disabled_opacity: 1.0,
        }
    }

    pub fn accent(theme: AppTheme) -> Self {
        Self::new(theme).persistent_background(control_open_background(theme))
    }

    /// Compact selectors in table and diff headers share a quieter hover.
    pub fn header(theme: AppTheme) -> Self {
        Self::new(theme)
            .persistent_background(theme.colors.interaction.pressed_background)
            .hover(
                StyleRefinement::default()
                    .bg(with_alpha(theme.colors.interaction.hover_background, 0.55)),
            )
            .pressed(StyleRefinement::default().bg(theme.colors.interaction.pressed_background))
    }

    pub fn link(theme: AppTheme) -> Self {
        Self::new(theme)
            .hover(StyleRefinement::default().text_color(theme.colors.accent.foreground))
            .pressed(StyleRefinement::default().text_color(theme.colors.accent.foreground))
    }

    pub fn destructive(theme: AppTheme) -> Self {
        Self::tinted(theme, theme.colors.status.danger.foreground)
    }

    /// Semantic actions (remove, stage, unstage) differ in hue, while their
    /// pointer feedback follows the same intensity and precedence.
    pub fn tinted(theme: AppTheme, color: Rgba) -> Self {
        Self::new(theme)
            .hover(StyleRefinement::default().bg(with_alpha(color, REMOVE_BUTTON_HOVER_ALPHA)))
            .pressed(StyleRefinement::default().bg(with_alpha(color, REMOVE_BUTTON_PRESSED_ALPHA)))
    }

    /// Menus in both the input kit and application views use this profile.
    /// Their host owns arrow-key navigation and selection.
    pub fn menu(theme: AppTheme) -> Self {
        Self::new(theme).selection_outline(false)
    }

    /// Resolve solid fills for custom-painted controls and their opaque
    /// decorations using the very same precedence as `apply`.
    pub fn fill(&self, state: InteractionState, feedback: InteractionFeedback) -> Option<Rgba> {
        if let Some(persistent) = self.persistent_fill(state) {
            return Some(persistent);
        }
        let feedback = if state.disabled || !self.pointer_feedback {
            InteractionFeedback::Resting
        } else {
            feedback
        };
        let refinement = match feedback {
            InteractionFeedback::Resting => return self.resting,
            InteractionFeedback::Hovered => &self.hover,
            InteractionFeedback::Pressed => &self.pressed,
        };
        refinement
            .background
            .as_ref()
            .and_then(|fill| fill.color())
            .and_then(|background| background.as_solid())
            .map(|color| color.into_color())
            .or(self.resting)
    }

    pub fn resolved_background(
        &self,
        surface: Rgba,
        state: InteractionState,
        feedback: InteractionFeedback,
    ) -> Rgba {
        self.fill(state, feedback)
            .map_or(surface, |fill| composite_over(surface, fill))
    }

    /// Flatten fills onto an opaque backing, for label masks and tinted rows.
    /// Geometry and text/border refinements are retained.
    pub fn on_surface(mut self, surface: Rgba) -> Self {
        assert!(
            surface.alpha >= 1.0,
            "on_surface requires an opaque backing; use unflattened overlays for transparent controls"
        );
        self.resting = Some(
            self.resting
                .map_or(surface, |fill| composite_over(surface, fill)),
        );
        self.persistent = composite_over(surface, self.persistent);
        for style in [&mut self.hover, &mut self.pressed] {
            if let Some(fill) = style
                .background
                .as_ref()
                .and_then(|fill| fill.color())
                .and_then(|background| background.as_solid())
            {
                style.background = Some(composite_over(surface, fill.into_color()).into());
            }
        }
        self
    }

    pub fn resting_background(mut self, background: Rgba) -> Self {
        self.resting = Some(background);
        self
    }

    pub fn persistent_background(mut self, background: Rgba) -> Self {
        self.persistent = background;
        self
    }

    pub fn hover(mut self, style: StyleRefinement) -> Self {
        self.hover = style;
        self
    }

    pub fn pressed(mut self, style: StyleRefinement) -> Self {
        self.pressed = style;
        self
    }

    pub fn selection_outline(mut self, show: bool) -> Self {
        self.selection_outline = show;
        self
    }

    /// A parent menu may hold a highlight while other items remain actionable.
    /// Suppressing transient feedback must not disable activation or focus.
    pub fn pointer_feedback(mut self, enabled: bool) -> Self {
        self.pointer_feedback = enabled;
        self
    }

    pub fn disabled_opacity(mut self, opacity: f32) -> Self {
        self.disabled_opacity = opacity;
        self
    }

    pub fn persistent_fill(&self, state: InteractionState) -> Option<Rgba> {
        if state.open || state.busy {
            Some(self.persistent)
        } else {
            state.selected
        }
    }

    pub fn resting_fill(&self, state: InteractionState) -> Option<Rgba> {
        self.persistent_fill(state).or(self.resting)
    }

    pub fn apply(self, mut control: Stateful<Div>, state: InteractionState) -> Stateful<Div> {
        let persistent = self.persistent_fill(state);
        if let Some(background) = self.resting_fill(state) {
            control = control.bg(background);
        }
        if persistent.is_some() && self.selection_outline && !self.theme.is_dark {
            control = control.shadow(vec![interaction_outline(
                self.theme.colors.interaction.selected_indicator,
            )]);
        }
        if state.disabled {
            return control
                .tab_stop(false)
                .opacity(self.disabled_opacity)
                .cursor(CursorStyle::Arrow)
                .on_mouse_down(gpui::MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .on_mouse_up_all(|_, phase, hitbox, window, cx| {
                    if phase == gpui::DispatchPhase::Bubble && hitbox.is_hovered(window) {
                        cx.stop_propagation();
                    }
                });
        }

        let outline = interaction_outline(self.theme.colors.interaction.focus_ring);
        control = control.focus_visible(move |style| style.shadow(vec![outline]));
        if !self.pointer_feedback {
            return control.cursor(CursorStyle::Arrow);
        }

        let hover = persistent.map_or(self.hover, |bg| StyleRefinement::default().bg(bg));
        let pressed = persistent.map_or(self.pressed, |bg| StyleRefinement::default().bg(bg));
        control
            .cursor(CursorStyle::PointingHand)
            .hover(move |mut style| {
                style.refine(&hover);
                style
            })
            .active(move |mut style| {
                style.refine(&pressed);
                style
            })
    }
}

pub trait ControlInteractionExt: Sized {
    fn control_interaction(self, style: InteractionStyle, state: InteractionState) -> Self;
    fn on_activate(
        self,
        disabled: bool,
        activation: ControlActivation,
        handler: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self;
    /// Menu entries require a completed click on the same entry.
    /// Focus and text selection stay on the surface the menu acts upon.
    fn on_menu_activate(
        self,
        disabled: bool,
        handler: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self;
}

impl ControlInteractionExt for Stateful<Div> {
    fn control_interaction(self, style: InteractionStyle, state: InteractionState) -> Self {
        style.apply(self, state)
    }

    fn on_menu_activate(
        self,
        disabled: bool,
        handler: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        if disabled {
            return self;
        }
        let handler = std::rc::Rc::new(handler);
        let primary = handler.clone();
        let control = self.on_any_mouse_down(|_, window, cx| {
            crate::text_selection_owner::preserve(cx);
            window.prevent_default();
            cx.stop_propagation();
        });
        let control = super::click::on_click(
            control,
            gpui::MouseButton::Left,
            move |event, window, cx| primary(event, window, cx),
        );
        super::click::on_click(
            control,
            gpui::MouseButton::Right,
            move |event, window, cx| handler(event, window, cx),
        )
    }

    fn on_activate(
        mut self,
        disabled: bool,
        activation: ControlActivation,
        handler: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
    ) -> Self {
        if disabled {
            return self;
        }
        if matches!(
            activation,
            ControlActivation::Action | ControlActivation::Nested
        ) {
            self = self.tab_index(0);
        }
        if activation == ControlActivation::PreserveFocus {
            self = self.on_mouse_down(gpui::MouseButton::Left, |_, window, _| {
                window.prevent_default();
            });
        }
        if activation == ControlActivation::Nested {
            self = self.on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation());
        }
        let owns_keyboard = matches!(
            activation,
            ControlActivation::Action | ControlActivation::Nested | ControlActivation::ManagedFocus
        );
        if owns_keyboard {
            // GPUI records its pending keyboard click before these bubble
            // listeners. Consume the key without preventing that default.
            self = self
                .on_key_down(|event, _, cx| {
                    if is_activation_key(&event.keystroke) {
                        cx.stop_propagation();
                    }
                })
                .on_key_up(|event, _, cx| {
                    if is_activation_key(&event.keystroke) {
                        cx.stop_propagation();
                    }
                });
        }
        super::click::on_click(self, gpui::MouseButton::Left, move |event, window, cx| {
            if matches!(event, gpui::ClickEvent::Keyboard(_)) && !owns_keyboard {
                return;
            }
            if activation == ControlActivation::Nested {
                cx.stop_propagation();
            }
            handler(event, window, cx);
        })
    }
}

fn is_activation_key(stroke: &gpui::Keystroke) -> bool {
    !stroke.modifiers.modified() && matches!(stroke.key.as_str(), "enter" | "space")
}

#[cfg(test)]
#[path = "interaction_tests.rs"]
mod tests;
