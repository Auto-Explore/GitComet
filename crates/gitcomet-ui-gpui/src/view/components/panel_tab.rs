//! Shared shape for the bottom panels' tab strips: the terminal's per-instance
//! tabs, the terminal/reflog switcher, and the reflog pane's header tab. Only
//! the styling is shared; each caller attaches its own handlers and children.
use crate::theme::{AppTheme, with_alpha};
use crate::ui_scale::UiScale;
use crate::view::icons::svg_icon;
use gpui::prelude::*;
use gpui::{CursorStyle, Div, ElementId, Stateful, div, px};

/// Close affordance on a panel tab: smaller than a control since it sits
/// inside the tab, but on the same density ramp.
const PANEL_TAB_CLOSE_SIZE_PX: f32 = 14.0;
const PANEL_TAB_CLOSE_COMFORTABLE_SIZE_PX: f32 = 20.0;
const PANEL_TAB_CLOSE_ICON_PX: f32 = 10.0;
/// Danger tint the plate picks up on hover, matching the repository tab's `x`.
const PANEL_TAB_CLOSE_HOVER_ALPHA: f32 = 0.18;

const PANEL_TAB_GAP_PX: f32 = 6.0;
const PANEL_TAB_PAD_X_PX: f32 = 8.0;
const PANEL_TAB_ICON_PX: f32 = 12.0;

/// The `x` on a panel tab. The caller adds the handler -- with
/// `stop_propagation`, so closing never also selects -- and the tooltip.
pub fn panel_tab_close(
    id: impl Into<ElementId>,
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    icon_color: gpui::Rgba,
) -> Stateful<Div> {
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    let id = id.into();
    let selector = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(ui_scale.row_height(PANEL_TAB_CLOSE_SIZE_PX, PANEL_TAB_CLOSE_COMFORTABLE_SIZE_PX))
        .rounded(px(theme.radii.row))
        .cursor(CursorStyle::PointingHand)
        .hover(move |s| {
            s.bg(with_alpha(
                theme.colors.status.danger.foreground,
                PANEL_TAB_CLOSE_HOVER_ALPHA,
            ))
        })
        .child(svg_icon(
            "icons/generic_close.svg",
            icon_color,
            ui_scale.px(PANEL_TAB_CLOSE_ICON_PX),
        ))
}

/// A panel tab's box with its leading icon and label. The caller adds the
/// close affordance, the click handler, and any selected-state styling.
pub fn panel_tab(
    id: impl Into<ElementId>,
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    icon: &'static str,
    label: impl Into<gpui::SharedString>,
    background: gpui::Rgba,
    text_color: gpui::Rgba,
) -> Stateful<Div> {
    let ui_scale = ui_scale.into().with_appearance(theme.metrics);
    let id = id.into();
    let selector = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .flex()
        .flex_none()
        .flex_row()
        .items_center()
        .gap(ui_scale.px(PANEL_TAB_GAP_PX))
        .px(ui_scale.px(PANEL_TAB_PAD_X_PX))
        .h(super::control_height(ui_scale))
        .rounded(px(theme.radii.row))
        .bg(background)
        .text_color(text_color)
        .text_size(theme.ui_text(12.0))
        .cursor(CursorStyle::PointingHand)
        .child(svg_icon(icon, text_color, ui_scale.px(PANEL_TAB_ICON_PX)))
        .child(label.into())
}
