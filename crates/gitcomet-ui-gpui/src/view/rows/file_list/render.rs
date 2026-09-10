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
const ROW_GAP_PX: f32 = 4.0;
/// `diff_stat` reserves two 30px columns with a `gap_1` between and before.
const STAT_WIDTH_PX: f32 = 68.0;
/// Below this the label is not worth showing, so the stat gives way first.
const LABEL_MIN_PX: f32 = 48.0;

/// How much of a folder row's trailing detail fits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum DirectoryRowDetail {
    WithStat,
    LabelOnly,
}

/// Whether the edit size still fits beside the label.
///
/// Design px against a device-px width: UI scale grows the row but not the pane
/// holding it, which is why this fires at 200% where 100% is comfortable.
/// `Pixels::MAX` (an unmeasured list) reads as plenty of room.
pub(in crate::view) fn directory_row_detail_for_width(
    available_width: gpui::Pixels,
    depth: usize,
    has_stat: bool,
    ui_scale_percent: u32,
) -> DirectoryRowDetail {
    if !has_stat {
        return DirectoryRowDetail::LabelOnly;
    }
    if available_width == gpui::Pixels::MAX || available_width <= gpui::px(0.0) {
        return DirectoryRowDetail::WithStat;
    }

    let needed = BASE_PAD_X_PX
        + INDENT_STEP_PX * depth as f32
        + CHEVRON_SLOT_PX
        + ROW_GAP_PX
        + ICON_SLOT_PX
        + ROW_GAP_PX
        + BASE_PAD_X_PX
        + LABEL_MIN_PX
        + STAT_WIDTH_PX;

    if crate::ui_scale::design_px_from_percent(needed, ui_scale_percent) <= available_width {
        DirectoryRowDetail::WithStat
    } else {
        DirectoryRowDetail::LabelOnly
    }
}

pub(in crate::view) struct DirectoryRowProps<'a> {
    pub(in crate::view) theme: AppTheme,
    pub(in crate::view) ui_scale_percent: u32,
    pub(in crate::view) id: ElementId,
    pub(in crate::view) label: &'a SharedString,
    pub(in crate::view) depth: usize,
    pub(in crate::view) collapsed: bool,
    pub(in crate::view) additions: Option<u64>,
    pub(in crate::view) deletions: Option<u64>,
    /// Must equal the list's file-row height: `uniform_list` measures row 0 and
    /// applies that height to every row.
    pub(in crate::view) row_height_px: f32,
    /// Hover group for the trailing overlay; `None` on lists that cannot stage.
    pub(in crate::view) row_group: Option<SharedString>,
    /// From [`directory_row_detail_for_width`].
    pub(in crate::view) detail: DirectoryRowDetail,
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
        additions,
        deletions,
        row_height_px,
        row_group,
        detail,
    } = props;
    let scaled = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
    let secondary = theme.colors.foreground.secondary;

    gpui::div()
        .id(id)
        // So a caller can position a trailing action at the row's right edge.
        .relative()
        .when_some(row_group, |row, group| row.group(group))
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
                .min_w(px(0.0))
                .text_sm()
                .line_height(scaled(18.0))
                .line_clamp(1)
                .whitespace_nowrap()
                .text_color(theme.colors.foreground.primary)
                .text_ellipsis()
                .child(label.clone()),
        )
        .child(gpui::div().flex_1().min_w(px(0.0)))
        .when(
            detail == DirectoryRowDetail::WithStat
                && (additions.is_some() || deletions.is_some()),
            |row| {
                row.child(gpui::div().flex_none().child(components::diff_stat(
                    theme,
                    ui_scale_percent,
                    additions.unwrap_or(0) as usize,
                    deletions.unwrap_or(0) as usize,
                )))
            },
        )
}
