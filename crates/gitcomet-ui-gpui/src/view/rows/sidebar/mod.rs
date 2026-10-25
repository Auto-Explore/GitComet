use super::*;
use crate::kit::click::PointerClickExt as _;
use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
use crate::ui_scale;
use crate::view::components::InteractiveRowExt as _;
use crate::view::sidebar_presentation::SidebarPresentation;
use crate::view::sidebar_sticky::SidebarRowSurface;
use gitcomet_core::domain::LogScope;
use gitcomet_core::domain::SubmoduleStatus;
use palette::IntoColor;
use std::num::NonZeroU32;

mod badges;
mod branch_lookup;
mod branch_rows;
mod file_rows;
mod search_labels;

pub(in crate::view) use badges::*;
pub(in crate::view) use branch_lookup::*;
use search_labels::*;

pub(in crate::view) fn sidebar_row_background(
    theme: AppTheme,
    surface: SidebarRowSurface,
    row: &BranchSidebarRow,
    stuck: bool,
) -> gpui::Rgba {
    match surface {
        SidebarRowSurface::Rail => theme.colors.surface.raised,
        SidebarRowSurface::Pins => theme.colors.surface.panel,
        _ if !stuck
            && matches!(
                row,
                BranchSidebarRow::GroupHeader { .. } | BranchSidebarRow::RemoteHeader { .. }
            ) =>
        {
            theme.colors.surface.chrome
        }
        _ if crate::view::sidebar_sticky::header_key(row).is_some() => theme.colors.surface.panel,
        _ => theme.colors.surface.chrome,
    }
}

/// Row height of every continuous list in the sidebar. The tabs swap lists in
/// place, so a differing rhythm would make the rows jump.
const SIDEBAR_TREE_ROW_HEIGHT_PX: f32 = 24.0;
const SIDEBAR_TREE_COMFORTABLE_ROW_HEIGHT_PX: f32 = 32.0;

/// Unscaled height shared by both sidebar modes. Round the density ramp before
/// scaling so natural rows and sticky overlays use the same slot geometry.
pub(in crate::view) fn sidebar_list_row_height_px(theme: AppTheme) -> f32 {
    theme
        .metrics
        .row_height(
            SIDEBAR_TREE_ROW_HEIGHT_PX,
            SIDEBAR_TREE_COMFORTABLE_ROW_HEIGHT_PX,
        )
        .round()
}

pub(in crate::view) fn sidebar_list_row_height(
    theme: AppTheme,
    ui_scale_percent: u32,
) -> gpui::Pixels {
    ui_scale::design_px_from_percent(sidebar_list_row_height_px(theme), ui_scale_percent)
}

#[cfg(test)]
mod tests;
