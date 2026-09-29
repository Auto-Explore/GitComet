//! Menu row geometry shared by text inputs and application popovers. The host
//! supplies content and navigation; pointer activation follows the menu gesture.
use super::interaction::{ControlInteractionExt, InteractionState, InteractionStyle};
use crate::{theme::AppTheme, ui_scale::UiScale};
use gpui::{Div, ElementId, Rgba, Stateful, div, prelude::*, px};

pub(crate) fn menu_item(
    id: impl Into<ElementId>,
    theme: AppTheme,
    scale: impl Into<UiScale>,
    selected: bool,
    disabled: bool,
) -> Stateful<Div> {
    menu_item_with_background(id, theme, scale, selected, disabled, None)
}

pub(crate) fn menu_item_with_background(
    id: impl Into<ElementId>,
    theme: AppTheme,
    scale: impl Into<UiScale>,
    selected: bool,
    disabled: bool,
    background: Option<Rgba>,
) -> Stateful<Div> {
    let scale = scale.into().with_appearance(theme.metrics);
    let mut interaction = InteractionStyle::menu(theme);
    if let Some(background) = background {
        interaction = interaction.resting_background(background);
    }
    div()
        .id(id)
        .min_h(scale.row_height(28.0, 32.0))
        .py(scale.px(4.0))
        .px(scale.px(8.0))
        .flex()
        .items_center()
        .justify_between()
        .gap(scale.px(20.0))
        .rounded(px(theme.radii.row))
        .text_size(theme.ui_text(14.0))
        .text_color(if disabled {
            theme.colors.foreground.secondary
        } else {
            theme.colors.foreground.primary
        })
        .control_interaction(
            interaction,
            InteractionState::default()
                .selected(selected && !disabled, theme.hover_overlay())
                .disabled(disabled),
        )
}
