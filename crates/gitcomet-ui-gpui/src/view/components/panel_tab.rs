//! Shared shape for the bottom panels' tab strips: the terminal's per-instance
//! tabs, the terminal/reflog switcher, and the reflog pane's header tab. Callers
//! attach actions through the common activation policy and supply their content.
use super::{ControlActivation, ControlInteractionExt, InteractionState, InteractionStyle};
use crate::theme::AppTheme;
use crate::ui_scale::UiScale;
use crate::view::icons::svg_icon;
use gpui::prelude::*;
use gpui::{Div, ElementId, Stateful, div, px};

/// Close affordance on a panel tab: smaller than a control since it sits
/// inside the tab, but on the same density ramp.
const PANEL_TAB_CLOSE_SIZE_PX: f32 = 14.0;
const PANEL_TAB_CLOSE_COMFORTABLE_SIZE_PX: f32 = 20.0;
const PANEL_TAB_CLOSE_ICON_PX: f32 = 10.0;

const PANEL_TAB_GAP_PX: f32 = 6.0;
const PANEL_TAB_PAD_X_PX: f32 = 8.0;
const PANEL_TAB_ICON_PX: f32 = 12.0;

/// The `x` on a panel tab. The caller adds the tooltip and the handler, which
/// uses `on_nested_control_click` so closing never also selects.
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
        .control_interaction(
            InteractionStyle::destructive(theme),
            InteractionState::default(),
        )
        .child(svg_icon(
            "icons/generic_close.svg",
            icon_color,
            ui_scale.px(PANEL_TAB_CLOSE_ICON_PX),
        ))
}

/// A panel tab's box with its leading icon and label. The caller adds the
/// close affordance and the click handler. Selection is resolved here.
pub fn panel_tab(
    id: impl Into<ElementId>,
    theme: AppTheme,
    ui_scale: impl Into<UiScale>,
    icon: &'static str,
    label: impl Into<gpui::SharedString>,
    selected: bool,
) -> Stateful<Div> {
    let text_color = panel_tab_text_color(theme, selected);
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
        .control_interaction(
            InteractionStyle::new(theme)
                .resting_background(theme.colors.surface.panel)
                .selection_outline(false),
            InteractionState::default()
                .selected(selected, theme.colors.interaction.selected_background),
        )
        .text_color(text_color)
        .text_size(theme.ui_text(12.0))
        .child(svg_icon(icon, text_color, ui_scale.px(PANEL_TAB_ICON_PX)))
        .child(label.into())
}

/// Foreground shared by the tab label, leading icon and nested close control.
pub fn panel_tab_text_color(theme: AppTheme, selected: bool) -> gpui::Rgba {
    if selected {
        theme.colors.interaction.selected_foreground
    } else {
        theme.colors.foreground.secondary
    }
}

/// Nested actions consume their press and activate only on a completed click.
pub fn on_nested_control_click<V: 'static>(
    control: Stateful<Div>,
    cx: &mut gpui::Context<V>,
    handler: impl Fn(&mut V, &gpui::ClickEvent, &mut gpui::Window, &mut gpui::Context<V>) + 'static,
) -> Stateful<Div> {
    control.on_activate(false, ControlActivation::Nested, cx.listener(handler))
}
