#[cfg(test)]
use crate::interaction::interaction_outline;
use crate::interaction::{InteractionFeedback, InteractionState, InteractionStyle};
use crate::theme::{AppTheme, composite_over, layer_over};
use gpui::prelude::*;
use gpui::{Div, Rgba, Stateful, StyleRefinement, px};

pub type InteractiveRowState = InteractionState;

/// Row geometry and compositing over a known surface. All interaction state
/// and feedback are applied by the same routine as buttons and menu controls.
#[derive(Clone, Copy)]
pub struct InteractiveRowStyle {
    surface: Rgba,
    theme: AppTheme,
    hover_enabled: bool,
    radius: f32,
    tint: Option<Rgba>,
}

impl InteractiveRowStyle {
    pub fn new(theme: AppTheme, surface: Rgba) -> Self {
        Self {
            surface,
            theme,
            hover_enabled: true,
            radius: theme.radii.row,
            tint: None,
        }
    }

    pub fn flat(mut self) -> Self {
        self.radius = 0.0;
        self
    }

    /// Avoid hover invalidations when another interaction supplies the feedback,
    /// such as a drag whose destination is highlighted explicitly.
    pub fn without_hover(mut self) -> Self {
        self.hover_enabled = false;
        self
    }

    /// A translucent wash the row keeps in every state: it lies under hover
    /// and press feedback, and over the fill from [`Self::selection_fill`].
    pub fn tinted(mut self, tint: Option<Rgba>) -> Self {
        self.tint = tint;
        self
    }

    /// The selection colour to pass for this row, so a selected row keeps
    /// its wash even under an opaque selection fill. Half strength: a full
    /// wash greys out a light theme's selection.
    pub fn selection_fill(&self, selected: Rgba) -> Rgba {
        self.tint.map_or(selected, |tint| {
            layer_over(selected, crate::theme::with_alpha(tint, tint.alpha * 0.5))
        })
    }

    fn interaction(&self) -> InteractionStyle {
        let style = InteractionStyle::new(self.theme);
        let Some(tint) = self.tint else {
            return style;
        };
        let over_tint = |overlay| StyleRefinement::default().bg(layer_over(tint, overlay));
        style
            .resting_background(tint)
            .persistent_background(layer_over(self.theme.active_overlay(), tint))
            .hover(over_tint(self.theme.hover_overlay()))
            .pressed(over_tint(self.theme.active_overlay()))
    }

    fn resting_fill(&self, state: InteractiveRowState) -> Option<Rgba> {
        self.interaction().resting_fill(state)
    }

    fn hover_fill(&self, state: InteractiveRowState) -> Rgba {
        self.interaction()
            .fill(state, InteractionFeedback::Hovered)
            .unwrap_or(gpui::rgba(0x00000000))
    }

    #[cfg(test)]
    fn active_fill(&self, state: InteractiveRowState) -> Rgba {
        self.interaction()
            .fill(state, InteractionFeedback::Pressed)
            .unwrap_or(gpui::rgba(0x00000000))
    }

    #[cfg(test)]
    fn focus_outline(&self) -> gpui::BoxShadow {
        interaction_outline(self.theme.colors.interaction.focus_ring)
    }

    #[cfg(test)]
    fn selection_outline(&self) -> gpui::BoxShadow {
        interaction_outline(self.theme.colors.interaction.selected_indicator)
    }

    pub fn resolved_background(&self, state: InteractiveRowState) -> Rgba {
        self.resting_fill(state)
            .map_or(self.surface, |fill| composite_over(self.surface, fill))
    }

    pub fn resolved_hover_background(&self, state: InteractiveRowState) -> Rgba {
        composite_over(self.surface, self.hover_fill(state))
    }

    fn apply(self, row: Stateful<Div>, state: InteractiveRowState) -> Stateful<Div> {
        self.interaction()
            .hover_feedback(self.hover_enabled)
            .apply(row.rounded(px(self.radius)), state)
    }
}

pub trait InteractiveRowExt {
    fn interactive_row(self, style: InteractiveRowStyle, state: InteractiveRowState) -> Self;
    /// Paint a leading accent without changing the row's content geometry or hitboxes.
    fn row_accent(self, color: Rgba) -> Self;
}

impl InteractiveRowExt for Stateful<Div> {
    fn row_accent(self, color: Rgba) -> Self {
        self.relative().child(
            gpui::canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    window.paint_quad(gpui::fill(bounds, color));
                },
            )
            .absolute()
            .left_0()
            .top_0()
            .w(px(1.0))
            .h_full(),
        )
    }

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

        assert_eq!(selected_then_open, open_then_selected);
        let style = InteractiveRowStyle::new(dark_theme(), dark_theme().colors.surface.chrome);
        assert_eq!(
            style.resting_fill(selected_then_open),
            Some(dark_theme().active_overlay())
        );
        assert_eq!(
            style.resting_fill(selected_then_open.open(false)),
            Some(selected)
        );
        assert_eq!(
            style.resting_fill(selected_then_open.open(false).selected(false, selected)),
            None
        );
    }

    #[test]
    fn selected_background_persists_through_hover_and_press() {
        let theme = dark_theme();
        let selected = gpui::rgba(0x22446680);
        let style = InteractiveRowStyle::new(theme, theme.colors.surface.chrome);
        let state = InteractiveRowState::default().selected(true, selected);

        assert_eq!(style.resting_fill(state), Some(selected));
        assert_eq!(style.hover_fill(state), selected);
        assert_eq!(style.active_fill(state), selected);
    }

    #[test]
    fn flat_rows_have_square_interaction_fills() {
        let theme = dark_theme();
        let style = InteractiveRowStyle::new(theme, theme.colors.surface.chrome).flat();

        assert_eq!(style.radius, 0.0);
    }

    #[test]
    fn focus_outline_is_a_one_pixel_inset_in_both_appearances() {
        for theme in [dark_theme(), AppTheme::gitcomet_light()] {
            let outline =
                InteractiveRowStyle::new(theme, theme.colors.surface.chrome).focus_outline();

            assert!(outline.inset);
            assert_eq!(outline.offset, gpui::point(px(0.0), px(0.0)));
            assert_eq!(outline.blur_radius, px(0.0));
            assert_eq!(outline.spread_radius, px(1.0));
        }
    }

    #[test]
    fn selection_outline_adds_a_non_fill_selection_cue() {
        let theme = AppTheme::gitcomet_light();
        let outline =
            InteractiveRowStyle::new(theme, theme.colors.surface.chrome).selection_outline();

        assert!(outline.inset);
        assert_eq!(outline.spread_radius, px(1.0));
        assert_eq!(
            outline.color,
            theme.colors.interaction.selected_indicator.into()
        );
    }

    #[test]
    fn tinted_rows_keep_their_wash_in_every_state() {
        let theme = dark_theme();
        let surface = theme.colors.surface.chrome;
        let tint = gpui::rgba(0xe0a02020);
        let plain = InteractiveRowStyle::new(theme, surface);
        let tinted = plain.tinted(Some(tint));
        let idle = InteractiveRowState::default();
        // Hover and press fills round-trip through `Hsla` in `StyleRefinement::bg`.
        let assert_close = |a: Rgba, b: Rgba| {
            for (x, y) in [
                (a.red, b.red),
                (a.green, b.green),
                (a.blue, b.blue),
                (a.alpha, b.alpha),
            ] {
                assert!((x - y).abs() < 1e-5, "{a:?} vs {b:?}");
            }
        };

        assert_eq!(plain.resting_fill(idle), None);
        assert_eq!(tinted.resting_fill(idle), Some(tint));
        assert_eq!(
            tinted.resolved_background(idle),
            composite_over(surface, tint)
        );
        assert_close(
            tinted.resolved_hover_background(idle),
            composite_over(surface, layer_over(tint, theme.hover_overlay())),
        );
        assert_ne!(
            tinted.resolved_hover_background(idle),
            plain.resolved_hover_background(idle)
        );
        assert_close(
            tinted.active_fill(idle),
            layer_over(tint, theme.active_overlay()),
        );

        // An opaque selection would hide a wash painted under it.
        let selected = theme.colors.interaction.selected_background;
        assert_eq!(plain.selection_fill(selected), selected);
        let fill = tinted.selection_fill(selected);
        assert_eq!(
            fill,
            layer_over(selected, crate::theme::with_alpha(tint, tint.alpha * 0.5))
        );
        let state = idle.selected(true, fill);
        assert_eq!(tinted.resting_fill(state), Some(fill));
        assert_eq!(tinted.hover_fill(state), fill);
        assert_eq!(
            tinted.resting_fill(idle.open(true)),
            Some(layer_over(theme.active_overlay(), tint))
        );
    }

    #[test]
    fn idle_rows_use_canonical_overlays_on_every_surface() {
        let theme = dark_theme();
        let sidebar = InteractiveRowStyle::new(theme, theme.colors.surface.chrome);
        let popover = InteractiveRowStyle::new(theme, theme.colors.surface.raised);

        assert_eq!(
            sidebar.hover_fill(InteractiveRowState::default()),
            theme.hover_overlay()
        );
        assert_eq!(
            popover.hover_fill(InteractiveRowState::default()),
            theme.hover_overlay()
        );
        assert_eq!(
            sidebar.active_fill(InteractiveRowState::default()),
            theme.active_overlay()
        );
        assert_eq!(
            popover.active_fill(InteractiveRowState::default()),
            theme.active_overlay()
        );
        assert_ne!(
            sidebar.resolved_hover_background(InteractiveRowState::default()),
            popover.resolved_hover_background(InteractiveRowState::default()),
            "resolved colors should retain each surface while sharing intensity",
        );
    }
}
