use crate::theme::AppTheme;
use crate::view::components::{self, InteractiveRowExt, InteractiveRowState, InteractiveRowStyle};
use crate::view::file_icons;
use crate::view::icons::svg_icon;
use gpui::prelude::*;
use gpui::{CursorStyle, Div, ElementId, SharedString, Stateful, px};

/// Design indent per tree level, matched to the file explorer's.
pub(in crate::view) const INDENT_STEP_PX: f32 = 12.0;
const CHEVRON_SLOT_PX: f32 = 12.0;
const ICON_SLOT_PX: f32 = 16.0;
const BASE_PAD_X_PX: f32 = 8.0;

pub(in crate::view) struct DirectoryRowProps<'a> {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) id: ElementId,
    pub(in crate::view) label: &'a SharedString,
    pub(in crate::view) depth: usize,
    pub(in crate::view) collapsed: bool,
    pub(in crate::view) file_count: usize,
    pub(in crate::view) additions: Option<u64>,
    pub(in crate::view) deletions: Option<u64>,
    /// Must equal the list's file-row height: `uniform_list` measures row 0 and
    /// applies that height to every row.
    pub(in crate::view) row_height_px: f32,
}

/// Indent a file row sitting at `depth` in a tree, so its label lines up under
/// its folder's label rather than under the folder's chevron.
pub(in crate::view) fn file_row_indent_px(depth: usize, ui_scale_percent: u32) -> gpui::Pixels {
    crate::ui_scale::design_px_from_percent(
        BASE_PAD_X_PX + INDENT_STEP_PX * depth as f32,
        ui_scale_percent,
    )
}

pub(in crate::view) fn directory_row(props: DirectoryRowProps<'_>) -> Stateful<Div> {
    let DirectoryRowProps {
        theme,
        ui_scale_percent,
        id,
        label,
        depth,
        collapsed,
        file_count,
        additions,
        deletions,
        row_height_px,
    } = props;
    let scaled = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
    let secondary = theme.colors.foreground.secondary;

    gpui::div()
        .id(id)
        .h(scaled(row_height_px))
        .flex()
        .items_center()
        .gap(scaled(4.0))
        .pl(file_row_indent_px(depth, ui_scale_percent))
        .pr(scaled(BASE_PAD_X_PX))
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .interactive_row(
            InteractiveRowStyle::new(theme, theme.colors.surface.panel).flat(),
            InteractiveRowState::default(),
        )
        .child(
            gpui::div()
                .w(scaled(CHEVRON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(
                    file_icons::chevron_icon(!collapsed),
                    secondary,
                    scaled(10.0),
                )),
        )
        .child(
            gpui::div()
                .w(scaled(ICON_SLOT_PX))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .child(svg_icon(
                    file_icons::folder_icon(!collapsed),
                    secondary,
                    scaled(14.0),
                )),
        )
        .child(
            gpui::div()
                .flex_1()
                .min_w(px(0.0))
                .text_sm()
                .line_height(scaled(18.0))
                .line_clamp(1)
                .whitespace_nowrap()
                .text_color(theme.colors.foreground.primary)
                .text_ellipsis()
                .child(label.clone()),
        )
        .child(
            gpui::div()
                .flex_none()
                .text_xs()
                .text_color(secondary)
                .child(SharedString::from(file_count.to_string())),
        )
        .when(additions.is_some() || deletions.is_some(), |row| {
            row.child(gpui::div().flex_none().child(components::diff_stat(
                theme,
                ui_scale_percent,
                additions.unwrap_or(0) as usize,
                deletions.unwrap_or(0) as usize,
            )))
        })
}
