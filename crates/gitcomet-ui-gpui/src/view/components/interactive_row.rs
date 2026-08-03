use crate::theme::{AppTheme, composite_over};
use gpui::prelude::*;
use gpui::{CursorStyle, Div, Rgba, Stateful, px};

/// Semantic state for an interactive list/tree row.
///
/// Transient hover and press feedback comes from [`InteractiveRowStyle`]. A
/// selected row keeps its caller-provided persistent background through those
/// transient states, while an open menu takes precedence over selection.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum InteractiveRowState {
    #[default]
    Idle,
    Selected(Rgba),
    Open,
}

impl InteractiveRowState {
    pub fn selected(self, selected: bool, background: Rgba) -> Self {
        match (self, selected) {
            (Self::Open, _) | (_, false) => self,
            (_, true) => Self::Selected(background),
        }
    }

    pub fn open(self, open: bool) -> Self {
        if open { Self::Open } else { self }
    }
}

/// Shared interaction treatment for rows painted over a known surface.
///
/// Row elements receive translucent theme overlays directly. Consumers that
/// paint their own opaque pixels on top of a row (for example text fades) can
/// ask this style for the resolved background for the same semantic state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteractiveRowStyle {
    surface: Rgba,
    hover: Rgba,
    active: Rgba,
    focus: Rgba,
    focus_ring: Rgba,
    radius: f32,
}

impl InteractiveRowStyle {
    pub fn new(theme: AppTheme, surface: Rgba) -> Self {
        Self {
            surface,
            hover: theme.hover_overlay(),
            active: theme.active_overlay(),
            focus: theme.colors.focus_ring_bg,
            focus_ring: theme.colors.focus_ring,
            radius: theme.radii.row,
        }
    }

    fn resting_fill(self, state: InteractiveRowState) -> Option<Rgba> {
        match state {
            InteractiveRowState::Idle => None,
            InteractiveRowState::Selected(background) => Some(background),
            InteractiveRowState::Open => Some(self.active),
        }
    }

    fn hover_fill(self, state: InteractiveRowState) -> Rgba {
        match state {
            InteractiveRowState::Idle => self.hover,
            InteractiveRowState::Selected(background) => background,
            InteractiveRowState::Open => self.active,
        }
    }

    fn active_fill(self, state: InteractiveRowState) -> Rgba {
        match state {
            InteractiveRowState::Idle => self.active,
            InteractiveRowState::Selected(background) => background,
            InteractiveRowState::Open => self.active,
        }
    }

    fn focus_fill(self, state: InteractiveRowState) -> Rgba {
        match state {
            InteractiveRowState::Idle => self.focus,
            InteractiveRowState::Selected(background) => background,
            InteractiveRowState::Open => self.active,
        }
    }

    pub fn resolved_background(self, state: InteractiveRowState) -> Rgba {
        self.resting_fill(state)
            .map_or(self.surface, |fill| composite_over(self.surface, fill))
    }

    pub fn resolved_hover_background(self, state: InteractiveRowState) -> Rgba {
        composite_over(self.surface, self.hover_fill(state))
    }

    fn apply(self, row: Stateful<Div>, state: InteractiveRowState) -> Stateful<Div> {
        let resting = self.resting_fill(state);
        let hover = self.hover_fill(state);
        let active = self.active_fill(state);
        let focus = self.focus_fill(state);
        let focus_ring = self.focus_ring;

        row.rounded(px(self.radius))
            .cursor(CursorStyle::PointingHand)
            .when_some(resting, |row, background| row.bg(background))
            .hover(move |row| row.bg(hover))
            .active(move |row| row.bg(active))
            .focus(move |row| row.bg(focus).border_1().border_color(focus_ring))
    }
}

pub trait InteractiveRowExt {
    fn interactive_row(self, style: InteractiveRowStyle, state: InteractiveRowState) -> Self;
}

impl InteractiveRowExt for Stateful<Div> {
    fn interactive_row(self, style: InteractiveRowStyle, state: InteractiveRowState) -> Self {
        style.apply(self, state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dark_theme() -> AppTheme {
        AppTheme::from_key(crate::theme::DEFAULT_DARK_THEME_KEY)
            .expect("embedded dark theme exists")
    }

    #[test]
    fn open_state_takes_priority_over_selection() {
        let selected = gpui::rgba(0x22446680);
        let selected_then_open = InteractiveRowState::default()
            .selected(true, selected)
            .open(true);
        let open_then_selected = InteractiveRowState::default()
            .open(true)
            .selected(true, selected);

        assert_eq!(selected_then_open, InteractiveRowState::Open);
        assert_eq!(open_then_selected, InteractiveRowState::Open);
    }

    #[test]
    fn selected_background_persists_through_hover_and_press() {
        let theme = dark_theme();
        let selected = gpui::rgba(0x22446680);
        let style = InteractiveRowStyle::new(theme, theme.colors.sidebar_bg);
        let state = InteractiveRowState::Selected(selected);

        assert_eq!(style.resting_fill(state), Some(selected));
        assert_eq!(style.hover_fill(state), selected);
        assert_eq!(style.active_fill(state), selected);
    }

    #[test]
    fn idle_rows_use_canonical_overlays_on_every_surface() {
        let theme = dark_theme();
        let sidebar = InteractiveRowStyle::new(theme, theme.colors.sidebar_bg);
        let popover = InteractiveRowStyle::new(theme, theme.colors.surface_bg_elevated);

        assert_eq!(
            sidebar.hover_fill(InteractiveRowState::Idle),
            theme.hover_overlay()
        );
        assert_eq!(
            popover.hover_fill(InteractiveRowState::Idle),
            theme.hover_overlay()
        );
        assert_eq!(
            sidebar.active_fill(InteractiveRowState::Idle),
            theme.active_overlay()
        );
        assert_eq!(
            popover.active_fill(InteractiveRowState::Idle),
            theme.active_overlay()
        );
        assert_ne!(
            sidebar.resolved_hover_background(InteractiveRowState::Idle),
            popover.resolved_hover_background(InteractiveRowState::Idle),
            "resolved colors should retain each surface while sharing intensity",
        );
    }
}
