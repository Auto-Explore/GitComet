use super::FileListId;
use crate::theme::AppTheme;
use crate::view::components::{self, InteractiveRowExt, InteractiveRowState, InteractiveRowStyle};
use crate::view::file_icons;
use crate::view::icons::svg_icon;
use gpui::prelude::*;
use gpui::{CursorStyle, Div, ElementId, SharedString, Stateful, px};
use std::path::Path;

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
    /// Already resolved, and it must equal the list's file-row height:
    /// `uniform_list` measures row 0 and applies that height to every row.
    /// Resolved rather than a design number because row height is density- and
    /// font-dependent now, and a folder row that recomputed it from a bare
    /// design px would drift from the file rows it sits among.
    pub(in crate::view) row_height: gpui::Pixels,
    /// Hover group for the trailing overlay; `None` on lists that cannot stage.
    pub(in crate::view) row_group: Option<SharedString>,
    /// From [`directory_row_detail_for_width`].
    pub(in crate::view) detail: DirectoryRowDetail,
    /// The row's context menu is open, so it stays lit like a file row's.
    pub(in crate::view) menu_open: bool,
}

/// Names the folder row whose menu is open, so the row can show it.
pub(in crate::view) fn file_list_folder_menu_invoker(
    repo_id: u64,
    list: FileListId,
    key: &Path,
) -> SharedString {
    format!("file_list_folder_menu_{repo_id}_{list:?}_{}", key.display()).into()
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
        row_height,
        row_group,
        detail,
        menu_open,
    } = props;
    let scaled = |value: f32| crate::ui_scale::design_px_from_percent(value, ui_scale_percent);
    let secondary = theme.colors.foreground.secondary;

    gpui::div()
        .id(id)
        // So a caller can position a trailing action at the row's right edge.
        .relative()
        .when_some(row_group, |row, group| row.group(group))
        .h(row_height)
        .flex()
        .items_center()
        .gap(scaled(4.0))
        .pl(file_row_indent_px(depth, ui_scale_percent))
        .pr(scaled(BASE_PAD_X_PX))
        .w_full()
        .cursor(CursorStyle::PointingHand)
        .interactive_row(
            InteractiveRowStyle::new(theme, theme.colors.surface.panel).flat(),
            InteractiveRowState::default().open(menu_open),
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
            detail == DirectoryRowDetail::WithStat && (additions.is_some() || deletions.is_some()),
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

/// A group header row's contents.
pub(in crate::view) struct GroupHeaderProps {
    /// Element id, with the group's index.
    pub(in crate::view) id: (&'static str, usize),
    pub(in crate::view) selector: String,
    pub(in crate::view) label: SharedString,
    pub(in crate::view) count: usize,
    pub(in crate::view) collapsed: bool,
}

/// A group's header row, `row_height` like every file row of its list (the
/// list is uniform); activating it runs `toggle`, which collapses or expands
/// the group.
pub(in crate::view) fn group_header_row(
    props: GroupHeaderProps,
    theme: AppTheme,
    ui_scale: crate::ui_scale::UiScale,
    row_height: gpui::Pixels,
    toggle: impl Fn(&mut gpui::App) + 'static,
) -> gpui::AnyElement {
    use crate::kit::interaction::{self as controls, ControlInteractionExt as _};
    let GroupHeaderProps {
        id,
        selector,
        label,
        count,
        collapsed,
    } = props;
    gpui::div()
        .id(id)
        .debug_selector(move || selector)
        .h(row_height)
        .w_full()
        .flex()
        .items_center()
        .gap(ui_scale.px(4.0))
        .px(ui_scale.px(8.0))
        .bg(theme.colors.surface.panel)
        .control_interaction(
            controls::InteractionStyle::new(theme),
            controls::InteractionState::default(),
        )
        .text_size(theme.ui_text(12.0))
        .text_color(theme.colors.foreground.secondary)
        .child(if collapsed { "▸" } else { "▾" })
        .child(format!("{label} ({count})"))
        .on_activate(
            false,
            controls::ControlActivation::Action,
            move |_, _, cx| {
                cx.stop_propagation();
                toggle(cx);
            },
        )
        .into_any_element()
}

/// Draws a group header at its natural position or pinned over the list.
/// The boolean distinguishes the pinned copy for its identity and selectors.
pub(in crate::view) type PinnedHeader =
    std::rc::Rc<dyn Fn(usize, gpui::Pixels, bool, &mut gpui::App) -> Option<gpui::AnyElement>>;

/// Pins the current group at the top; the next header pushes it up. Header
/// chrome uses logical scrolling only, outside the body's elastic transform.
#[derive(Clone)]
pub(in crate::view) struct StickyGroupHeaders {
    /// The row of each header, ascending.
    pub(in crate::view) headers: std::sync::Arc<[usize]>,
    pub(in crate::view) header: PinnedHeader,
}

#[derive(Clone, Copy)]
struct GroupHeaderMetrics {
    scroll_offset: gpui::Point<gpui::Pixels>,
    item_height: gpui::Pixels,
}

struct GroupHeaderMeasurement(std::rc::Rc<std::cell::Cell<Option<GroupHeaderMetrics>>>);

impl gpui::UniformListDecoration for GroupHeaderMeasurement {
    fn compute(
        &self,
        _visible_range: std::ops::Range<usize>,
        _bounds: gpui::Bounds<gpui::Pixels>,
        scroll_offset: gpui::Point<gpui::Pixels>,
        item_height: gpui::Pixels,
        _item_count: usize,
        _window: &mut gpui::Window,
        _cx: &mut gpui::App,
    ) -> gpui::AnyElement {
        self.0.set(Some(GroupHeaderMetrics {
            scroll_offset,
            item_height,
        }));
        gpui::div().into_any_element()
    }
}

impl StickyGroupHeaders {
    /// The header of the group holding `row`.
    fn header_for(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        after.checked_sub(1).map(|ix| self.headers[ix])
    }

    /// The first header below `row`.
    fn next_header(&self, row: usize) -> Option<usize> {
        let after = self.headers.partition_point(|&header| header <= row);
        self.headers.get(after).copied()
    }

    /// Measure in the list, then paint in a fixed sibling after the elastic
    /// body. Keeping the two in the same render also avoids stale metrics.
    pub(in crate::view) fn with_overlay(
        self,
    ) -> (impl gpui::UniformListDecoration, gpui::AnyElement) {
        let metrics = std::rc::Rc::new(std::cell::Cell::new(None::<GroupHeaderMetrics>));
        let measured = metrics.clone();
        let overlay = gpui::canvas(
            move |bounds, window, cx| -> Option<gpui::AnyElement> {
                let metrics = measured.take()?;
                let mut element = self.render(metrics, bounds.size.height, cx);
                element.layout_as_root(
                    gpui::size(
                        gpui::AvailableSpace::Definite(bounds.size.width),
                        gpui::AvailableSpace::Definite(bounds.size.height),
                    ),
                    window,
                    cx,
                );
                window.with_content_mask(
                    Some(gpui::ContentMask {
                        bounds,
                        ..Default::default()
                    }),
                    |window| element.prepaint_at(bounds.origin, window, cx),
                );
                Some(element)
            },
            |bounds, mut element, window, cx| {
                if let Some(element) = &mut element {
                    window.with_content_mask(
                        Some(gpui::ContentMask {
                            bounds,
                            ..Default::default()
                        }),
                        |window| element.paint(window, cx),
                    );
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .into_any_element();
        (GroupHeaderMeasurement(metrics), overlay)
    }

    fn render(
        &self,
        metrics: GroupHeaderMetrics,
        height: gpui::Pixels,
        cx: &mut gpui::App,
    ) -> gpui::AnyElement {
        let GroupHeaderMetrics {
            scroll_offset,
            item_height,
        } = metrics;
        if item_height <= px(0.0) {
            return gpui::div().into_any_element();
        }
        let scrolled = -scroll_offset.y;
        let first = (scrolled / item_height).floor().max(0.0) as usize;
        let start = self.headers.partition_point(|&row| row < first);
        let current = self.header_for(first);
        let pinned =
            current.filter(|&row| scrolled > px(0.0) && item_height * row as f32 <= scrolled);
        let mut children = Vec::new();
        for row in pinned.into_iter().chain(
            self.headers[start..]
                .iter()
                .copied()
                .filter(|row| Some(*row) != pinned),
        ) {
            let is_pinned = pinned == Some(row);
            let mut top = item_height * row as f32 - scrolled;
            if is_pinned {
                top = px(0.0);
                if let Some(next) = self.next_header(row) {
                    top = top.min(item_height * next as f32 - scrolled - item_height);
                }
            }
            if top >= height {
                break;
            }
            if top + item_height <= px(0.0) {
                continue;
            }
            if let Some(header) = (self.header)(row, item_height, is_pinned, cx) {
                children.push(
                    gpui::div()
                        .absolute()
                        .top(top)
                        .left_0()
                        .right_0()
                        .h(item_height)
                        .block_mouse_except_scroll()
                        .child(header)
                        .into_any_element(),
                );
            }
        }
        gpui::div()
            .relative()
            .size_full()
            .children(children)
            .into_any_element()
    }
}
