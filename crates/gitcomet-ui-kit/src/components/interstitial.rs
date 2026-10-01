//! A full-surface interstitial: a card centred over a backdrop, shown while
//! something loads or when the app cannot continue (no Git, no repository).

use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use gpui::prelude::*;
use gpui::{AnyElement, Background, SharedString, div, point, px};

/// The card's widest extent, before scaling.
pub const INTERSTITIAL_CARD_MAX_WIDTH_PX: f32 = 560.0;

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
