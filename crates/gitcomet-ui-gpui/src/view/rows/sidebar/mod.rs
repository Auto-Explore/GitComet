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

const SIDEBAR_TREE_ROW_HEIGHT_PX: f32 = 24.0;
const SIDEBAR_TREE_COMFORTABLE_ROW_HEIGHT_PX: f32 = 32.0;
/// A name line (23px) over a details line as tall as the worktree chip, plus
/// one pixel of air: 42 / 48 / 52 by density.
const BRANCH_SIDEBAR_TALL_ROW_HEIGHT_PX: f32 = 42.0;
const BRANCH_SIDEBAR_COMFORTABLE_TALL_ROW_HEIGHT_PX: f32 = 48.0;

/// Height of two-line branch sidebar rows (local branches and worktrees).
/// Every other sidebar row uses `sidebar_list_row_height`.
pub(in crate::view) fn branch_sidebar_tall_row_height(
    theme: AppTheme,
    ui_scale_percent: u32,
) -> gpui::Pixels {
    let metrics = theme.metrics;
    // `row_height` grows by the font bonus once; both lines grow with it.
    let second_line_bonus = (metrics.ui_text(20.0) - 20.0).max(0.0);
    ui_scale::design_px_from_percent(
        (metrics.row_height(
            BRANCH_SIDEBAR_TALL_ROW_HEIGHT_PX,
            BRANCH_SIDEBAR_COMFORTABLE_TALL_ROW_HEIGHT_PX,
        ) + second_line_bonus)
            .round(),
        ui_scale_percent,
    )
}

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
