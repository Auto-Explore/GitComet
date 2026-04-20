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
