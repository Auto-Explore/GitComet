use crate::ui_scale::UiScale;

pub const CONTROL_HEIGHT_PX: f32 = 22.0;
/// Medium control height
pub const CONTROL_HEIGHT_MD_PX: f32 = 28.0;

/// Default horizontal padding for text buttons.
pub const CONTROL_PAD_X_PX: f32 = 10.0;
/// Default vertical padding for text buttons.
pub const CONTROL_PAD_Y_PX: f32 = 3.0;

/// Horizontal padding for icon-only buttons.
pub const ICON_PAD_X_PX: f32 = 6.0;

/// Horizontal inset applied to a list row's selection/hover highlight so the
/// rounded background reads as an inset pill/card rather than a full-bleed band.
pub const ROW_HIGHLIGHT_INSET_PX: f32 = 6.0;

/// Height of the divider between a split button's two halves. Deliberately far
/// short of the control height: each half now draws its own hover border, so a
/// full-height rule would read as a third frame rather than a seam.
pub const SPLIT_BUTTON_DIVIDER_HEIGHT_PX: f32 = 11.0;

/// Trailing close/remove affordance shared by repository tabs and the picker
/// rows that can drop an entry: a small hit box holding a danger-tinted X,
/// whose plate is the danger colour at these alphas. Both live off the same
/// tokens so the two buttons stay visually identical.
pub const REMOVE_BUTTON_ICON: &str = "icons/repo_tab_close.svg";
pub const REMOVE_BUTTON_SIZE_PX: f32 = 18.0;
pub const REMOVE_BUTTON_ICON_SIZE_PX: f32 = 12.0;
pub const REMOVE_BUTTON_HOVER_ALPHA: f32 = 0.18;
pub const REMOVE_BUTTON_PRESSED_ALPHA: f32 = 0.26;

pub fn control_height(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(CONTROL_HEIGHT_PX)
}

pub fn control_height_md(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(CONTROL_HEIGHT_MD_PX)
}

pub fn control_pad_x(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(CONTROL_PAD_X_PX)
}

pub fn control_pad_y(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(CONTROL_PAD_Y_PX)
}

pub fn icon_pad_x(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(ICON_PAD_X_PX)
}

pub fn split_button_divider_height(scale: impl Into<UiScale>) -> gpui::Pixels {
    scale.into().px(SPLIT_BUTTON_DIVIDER_HEIGHT_PX)
}
