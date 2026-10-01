//! A full-surface interstitial: a card centred over a backdrop, shown while
//! something loads or when the app cannot continue (no Git, no repository),
//! and the call-to-action buttons drawn on it.

use crate::icons::svg_icon;
use crate::interaction::{ControlInteractionExt, InteractionState, InteractionStyle};
use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{
    AnyElement, Background, CursorStyle, Div, FontWeight, SharedString, Stateful, StyleRefinement,
    div, point, px,
};

/// The card's widest extent, before scaling.
pub const INTERSTITIAL_CARD_MAX_WIDTH_PX: f32 = 560.0;
const CTA_HEIGHT_PX: f32 = 36.0;
const CTA_COMFORTABLE_HEIGHT_PX: f32 = 44.0;

/// A call-to-action button for an interstitial: taller than a toolbar
/// control, coloured from `theme.colors.interstitial` to sit on the
/// backdrop. The caller attaches activation and any tooltip.
pub fn interstitial_cta_button(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    icon: impl Into<SharedString>,
    primary: bool,
    theme: AppTheme,
    ui_scale: UiScale,
) -> Stateful<Div> {
    let colors = if primary {
        theme.colors.interstitial.primary
    } else {
        theme.colors.interstitial.secondary
    };
    let id = id.into();
    let debug = id.to_string();
    div()
        .id(id)
        .debug_selector(move || debug.clone())
        .tab_index(0)
        .h(ui_scale.px(theme
            .metrics
            .row_height(CTA_HEIGHT_PX, CTA_COMFORTABLE_HEIGHT_PX)))
        .px(ui_scale.px(16.0))
        .flex()
        .items_center()
        .justify_center()
        .gap(ui_scale.px(6.0))
        .rounded(ui_scale.px(2.0))
        .border_1()
        .border_color(colors.border)
        .bg(colors.background)
        .text_size(theme.ui_text(13.0))
        .font_weight(FontWeight::BOLD)
        .text_color(colors.text)
        .cursor(CursorStyle::PointingHand)
        .whitespace_nowrap()
        .child(svg_icon(icon, colors.text, ui_scale.px(14.0)))
        .child(label.into())
        .control_interaction(
            InteractionStyle::new(theme)
                .hover(
                    StyleRefinement::default()
                        .bg(colors.background_hover)
                        .border_color(colors.border_hover),
                )
                .pressed(
                    StyleRefinement::default()
                        .bg(colors.background_active)
                        .border_color(colors.border_active),
                ),
            InteractionState::default(),
        )
}

/// `backdrop` fills the surface behind the card (the product's artwork);
/// `base` paints under it.
pub fn interstitial(
    id: impl Into<SharedString>,
    base: Background,
    backdrop: AnyElement,
    content: impl IntoElement,
    theme: AppTheme,
    ui_scale: UiScale,
) -> AnyElement {
    let id = id.into();
    let debug = id.to_string();
    let border_glow = with_alpha(
        theme.colors.stroke.default,
        if theme.is_dark { 0.86 } else { 0.74 },
    );

    div()
        .id(id)
        .debug_selector(move || debug.clone())
        .relative()
        .flex()
        .flex_1()
        .min_h(px(0.0))
        .items_center()
        .justify_center()
        .overflow_hidden()
        .px_3()
        .py_4()
        .bg(base)
        .child(backdrop)
        .child(
            div()
                .relative()
                .w_full()
                .max_w(ui_scale.px(INTERSTITIAL_CARD_MAX_WIDTH_PX))
                .bg(with_alpha(
                    theme.colors.surface.panel,
                    if theme.is_dark { 0.96 } else { 0.98 },
                ))
                .border_1()
                .border_color(border_glow)
                .rounded(px(theme.radii.panel))
                .shadow(vec![gpui::BoxShadow {
                    color: gpui::rgba(if theme.is_dark {
                        0x00000052
                    } else {
                        0x171a3b14
                    })
                    .into(),
                    offset: point(px(0.0), px(22.0)),
                    blur_radius: px(52.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
                .p_4()
                .child(content),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Ctas {
        theme: AppTheme,
    }

    impl gpui::Render for Ctas {
        fn render(
            &mut self,
            _: &mut gpui::Window,
            _: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let scale = UiScale::from_percent(100);
            div()
                .flex()
                .gap_2()
                .child(interstitial_cta_button(
                    "cta_primary",
                    "Open",
                    "icons/folder.svg",
                    true,
                    self.theme,
                    scale,
                ))
                .child(interstitial_cta_button(
                    "cta_secondary",
                    "Clone",
                    "icons/cloud.svg",
                    false,
                    self.theme,
                    scale,
                ))
        }
    }

    /// A product's own interstitial matches the host's: both buttons paint
    /// the theme's interstitial colours, light and dark.
    #[gpui::test]
    fn call_to_action_buttons_paint_the_themes_interstitial_colors(cx: &mut gpui::TestAppContext) {
        let _guard = crate::test_support::lock_visual_test();
        for theme in [AppTheme::gitcomet_dark(), AppTheme::gitcomet_light()] {
            let (_view, cx) = cx.add_window_view(|_, _| Ctas { theme });
            // The pointer starts at the origin, over the first button.
            cx.simulate_mouse_move(
                gpui::point(px(600.0), px(500.0)),
                None,
                gpui::Modifiers::default(),
            );
            crate::test_support::redraw(cx);
            for (selector, colors) in [
                ("cta_primary", theme.colors.interstitial.primary),
                ("cta_secondary", theme.colors.interstitial.secondary),
            ] {
                // Fill and border paint as separate quads.
                let quads = crate::test_support::painted_control_quads(cx, selector);
                assert!(
                    quads
                        .iter()
                        .any(|(fill, _)| *fill == colors.background.into()),
                    "{selector} fill: {quads:?}"
                );
                assert!(
                    quads
                        .iter()
                        .any(|(_, border)| *border == colors.border.into()),
                    "{selector} border: {quads:?}"
                );
            }
        }
    }
}
