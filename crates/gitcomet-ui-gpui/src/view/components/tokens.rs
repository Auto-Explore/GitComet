pub const CONTROL_HEIGHT_PX: f32 = 22.0;
/// Medium control height
pub const CONTROL_HEIGHT_MD_PX: f32 = 28.0;

/// Default horizontal padding for text buttons.
pub const CONTROL_PAD_X_PX: f32 = 10.0;
/// Default vertical padding for text buttons.
pub const CONTROL_PAD_Y_PX: f32 = 3.0;

/// Horizontal padding for icon-only buttons.
pub const ICON_PAD_X_PX: f32 = 6.0;

pub fn control_height(ui_scale_percent: u32) -> gpui::Pixels {
    crate::ui_scale::design_px_from_percent(CONTROL_HEIGHT_PX, ui_scale_percent)
}

pub fn control_height_md(ui_scale_percent: u32) -> gpui::Pixels {
    crate::ui_scale::design_px_from_percent(CONTROL_HEIGHT_MD_PX, ui_scale_percent)
}

pub fn control_pad_x(ui_scale_percent: u32) -> gpui::Pixels {
    crate::ui_scale::design_px_from_percent(CONTROL_PAD_X_PX, ui_scale_percent)
}

pub fn control_pad_y(ui_scale_percent: u32) -> gpui::Pixels {
    crate::ui_scale::design_px_from_percent(CONTROL_PAD_Y_PX, ui_scale_percent)
}

pub fn icon_pad_x(ui_scale_percent: u32) -> gpui::Pixels {
    crate::ui_scale::design_px_from_percent(ICON_PAD_X_PX, ui_scale_percent)
}
