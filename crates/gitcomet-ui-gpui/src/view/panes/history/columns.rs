//! History column widths: which columns fit, resizing and resets.

use super::{
    Branch, HISTORY_COL_AUTHOR_MAX_PX, HISTORY_COL_AUTHOR_MIN_PX, HISTORY_COL_AUTHOR_PX,
    HISTORY_COL_BRANCH_MAX_PX, HISTORY_COL_BRANCH_MIN_PX, HISTORY_COL_BRANCH_PX,
    HISTORY_COL_DATE_MAX_PX, HISTORY_COL_DATE_MIN_PX, HISTORY_COL_DATE_PX,
    HISTORY_COL_GRAPH_MAX_PX, HISTORY_COL_GRAPH_PX, HISTORY_COL_MESSAGE_MIN_PX,
    HISTORY_COL_SHA_MAX_PX, HISTORY_COL_SHA_MIN_PX, HISTORY_COL_SHA_PX,
    HISTORY_GRAPH_FULL_WIDTH_LANES, HistoryBranchNamesMode, HistoryColResizeHandle,
    HistoryColResizeState, LogScope, Pixels, RemoteBranch, px, ui_scale,
};

pub(in super::super::super) fn history_scrollbar_gutter() -> Pixels {
    crate::view::components::Scrollbar::gutter(crate::view::components::ScrollbarAxis::Vertical)
}

pub(super) fn history_columns_available_width(content_width: Pixels) -> Pixels {
    (content_width - history_scrollbar_gutter()).max(px(0.0))
}

pub(super) fn history_scale(ui_scale_percent: u32) -> ui_scale::UiScale {
    ui_scale::UiScale::from_percent(ui_scale_percent)
}

pub(super) fn history_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    history_scale(ui_scale_percent).px(value)
}

pub(super) fn history_message_min_width(ui_scale_percent: u32) -> Pixels {
    history_scaled_px(HISTORY_COL_MESSAGE_MIN_PX, ui_scale_percent)
}

pub(super) fn graph_branch_heads<'a>(
    history_scope: LogScope,
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
) -> impl Iterator<Item = &'a str> + 'a {
    let (branches, remote_branches): (&[Branch], &[RemoteBranch]) =
        if history_scope.is_current_branch_mode() {
            (&[], &[])
        } else {
            (branches, remote_branches)
        };
    branches
        .iter()
        .map(|b| b.target.as_ref())
        .chain(remote_branches.iter().map(|b| b.target.as_ref()))
}

pub(super) fn history_column_static_bounds(
    handle: HistoryColResizeHandle,
    scale: ui_scale::UiScale,
) -> (Pixels, Pixels) {
    let ui_scale_percent = scale.percent();
    match handle {
        HistoryColResizeHandle::Branch => (
            history_scaled_px(HISTORY_COL_BRANCH_MIN_PX, ui_scale_percent),
            history_scaled_px(HISTORY_COL_BRANCH_MAX_PX, ui_scale_percent),
        ),
        HistoryColResizeHandle::Graph => (
            crate::view::rows::history_node::graph_min_width(scale),
            scale.px(HISTORY_COL_GRAPH_MAX_PX)
                + crate::view::rows::history_node::graph_font_growth(scale)
                    * HISTORY_GRAPH_FULL_WIDTH_LANES as f32,
        ),
        HistoryColResizeHandle::Author => (
            history_scaled_px(HISTORY_COL_AUTHOR_MIN_PX, ui_scale_percent),
            history_scaled_px(HISTORY_COL_AUTHOR_MAX_PX, ui_scale_percent),
        ),
        HistoryColResizeHandle::Date => (
            history_scaled_px(HISTORY_COL_DATE_MIN_PX, ui_scale_percent),
            history_scaled_px(HISTORY_COL_DATE_MAX_PX, ui_scale_percent),
        ),
        HistoryColResizeHandle::Sha => (
            history_scaled_px(HISTORY_COL_SHA_MIN_PX, ui_scale_percent),
            history_scaled_px(HISTORY_COL_SHA_MAX_PX, ui_scale_percent),
        ),
    }
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) struct HistoryColumnWidths {
    pub(super) branch: Pixels,
    pub(super) graph: Pixels,
    pub(super) author: Pixels,
    pub(super) date: Pixels,
    pub(super) sha: Pixels,
}

#[derive(Copy, Clone, Debug, PartialEq)]
pub(super) struct HistoryColumnDesignWidths {
    pub(super) branch: f32,
    pub(super) graph: f32,
    pub(super) author: f32,
    pub(super) date: f32,
    pub(super) sha: f32,
}

pub(super) fn default_history_column_design_widths() -> HistoryColumnDesignWidths {
    HistoryColumnDesignWidths {
        branch: HISTORY_COL_BRANCH_PX,
        graph: HISTORY_COL_GRAPH_PX,
        author: HISTORY_COL_AUTHOR_PX,
        date: HISTORY_COL_DATE_PX,
        sha: HISTORY_COL_SHA_PX,
    }
}

pub(super) fn scaled_history_column_widths(
    widths: HistoryColumnDesignWidths,
    scale: ui_scale::UiScale,
) -> HistoryColumnWidths {
    HistoryColumnWidths {
        branch: scale.px(widths.branch),
        graph: scale.px(widths.graph),
        author: scale.px(widths.author),
        date: scale.px(widths.date),
        sha: scale.px(widths.sha),
    }
}

pub(super) fn default_history_column_widths(ui_scale_percent: u32) -> HistoryColumnWidths {
    scaled_history_column_widths(
        default_history_column_design_widths(),
        history_scale(ui_scale_percent),
    )
}

#[derive(Copy, Clone)]
pub(in crate::view) struct HistoryColumnDragLayout {
    pub(in crate::view) show_graph: bool,
    pub(in crate::view) show_author: bool,
    pub(in crate::view) show_date: bool,
    pub(in crate::view) show_sha: bool,
    pub(in crate::view) branch_w: Pixels,
    pub(in crate::view) graph_w: Pixels,
    pub(in crate::view) author_w: Pixels,
    pub(in crate::view) date_w: Pixels,
    pub(in crate::view) sha_w: Pixels,
}

pub(super) fn history_visible_columns_for_width(
    available_width: Pixels,
    show_graph: bool,
    preferred: (bool, bool, bool),
    widths: HistoryColumnWidths,
    ui_scale_percent: u32,
) -> (bool, bool, bool) {
    if available_width <= px(0.0) {
        return (false, false, false);
    }

    let min_message = history_message_min_width(ui_scale_percent);

    let (mut show_author, mut show_date, mut show_sha) = preferred;

    let fixed_base = widths.branch + if show_graph { widths.graph } else { px(0.0) };
    let mut fixed = fixed_base
        + if show_author { widths.author } else { px(0.0) }
        + if show_date { widths.date } else { px(0.0) }
        + if show_sha { widths.sha } else { px(0.0) };

    if available_width - fixed < min_message && show_sha {
        show_sha = false;
        fixed -= widths.sha;
    }
    if available_width - fixed < min_message {
        if show_date {
            show_date = false;
            fixed -= widths.date;
        }
        show_sha = false;
    }
    if available_width - fixed < min_message && show_author {
        show_author = false;
        fixed -= widths.author;
    }

    if available_width - fixed < min_message {
        show_author = false;
        show_date = false;
        show_sha = false;
    }

    (show_author, show_date, show_sha)
}

pub(super) fn history_column_drag_next_width(
    handle: HistoryColResizeHandle,
    candidate: Pixels,
    available_width: Pixels,
    show_graph: bool,
    preferred: (bool, bool, bool),
    widths: HistoryColumnWidths,
    scale: impl Into<ui_scale::UiScale>,
) -> Pixels {
    let scale = scale.into();
    let (show_author, show_date, show_sha) = history_visible_columns_for_width(
        available_width,
        show_graph,
        preferred,
        widths,
        scale.percent(),
    );
    history_column_drag_clamped_width(
        handle,
        candidate,
        available_width,
        HistoryColumnDragLayout {
            show_graph,
            show_author,
            show_date,
            show_sha,
            branch_w: widths.branch,
            graph_w: widths.graph,
            author_w: widths.author,
            date_w: widths.date,
            sha_w: widths.sha,
        },
        scale,
    )
}

pub(super) fn history_reset_widths_for_available_width(
    available_width: Pixels,
    branch_names: HistoryBranchNamesMode,
    show_graph: bool,
    preferred: (bool, bool, bool),
    scale: impl Into<ui_scale::UiScale>,
) -> HistoryColumnWidths {
    let scale = scale.into();
    let mut widths = default_history_column_widths(scale.percent());
    // The hidden ref column must not constrain the graph's reset width.
    let branch_width = widths.branch;
    if branch_names == HistoryBranchNamesMode::Inline {
        widths.branch = px(0.0);
    }
    widths.graph = history_column_drag_next_width(
        HistoryColResizeHandle::Graph,
        widths.graph,
        available_width,
        show_graph,
        preferred,
        widths,
        scale,
    );
    if branch_names == HistoryBranchNamesMode::Inline {
        widths.branch = branch_width;
        return widths;
    }
    widths.branch = history_column_drag_next_width(
        HistoryColResizeHandle::Branch,
        widths.branch,
        available_width,
        show_graph,
        preferred,
        widths,
        scale,
    );
    widths
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::view) struct HistoryColumnResizeDragParams {
    pub(in crate::view) start_width: Pixels,
    pub(in crate::view) drag_delta_sign: f32,
    pub(in crate::view) min_width: Pixels,
    pub(in crate::view) static_max_width: Pixels,
    pub(in crate::view) other_fixed_width: Pixels,
}

pub(in crate::view) fn history_column_resize_drag_params(
    handle: HistoryColResizeHandle,
    layout: HistoryColumnDragLayout,
    scale: impl Into<ui_scale::UiScale>,
) -> HistoryColumnResizeDragParams {
    let (start_width, drag_delta_sign) = match handle {
        HistoryColResizeHandle::Branch => (layout.branch_w, 1.0),
        HistoryColResizeHandle::Graph => (layout.graph_w, 1.0),
        HistoryColResizeHandle::Author => (layout.author_w, -1.0),
        HistoryColResizeHandle::Date => (layout.date_w, -1.0),
        HistoryColResizeHandle::Sha => (layout.sha_w, -1.0),
    };
    let (min_width, static_max_width) = history_column_static_bounds(handle, scale.into());
    let other_fixed_width = match handle {
        HistoryColResizeHandle::Branch => {
            (if layout.show_graph {
                layout.graph_w
            } else {
                px(0.0)
            }) + if layout.show_author {
                layout.author_w
            } else {
                px(0.0)
            } + if layout.show_date {
                layout.date_w
            } else {
                px(0.0)
            } + if layout.show_sha {
                layout.sha_w
            } else {
                px(0.0)
            }
        }
        HistoryColResizeHandle::Graph => {
            layout.branch_w
                + if layout.show_author {
                    layout.author_w
                } else {
                    px(0.0)
                }
                + if layout.show_date {
                    layout.date_w
                } else {
                    px(0.0)
                }
                + if layout.show_sha {
                    layout.sha_w
                } else {
                    px(0.0)
                }
        }
        HistoryColResizeHandle::Author => {
            layout.branch_w
                + if layout.show_graph {
                    layout.graph_w
                } else {
                    px(0.0)
                }
                + if layout.show_date {
                    layout.date_w
                } else {
                    px(0.0)
                }
                + if layout.show_sha {
                    layout.sha_w
                } else {
                    px(0.0)
                }
        }
        HistoryColResizeHandle::Date => {
            layout.branch_w
                + if layout.show_graph {
                    layout.graph_w
                } else {
                    px(0.0)
                }
                + if layout.show_author {
                    layout.author_w
                } else {
                    px(0.0)
                }
                + if layout.show_sha {
                    layout.sha_w
                } else {
                    px(0.0)
                }
        }
        HistoryColResizeHandle::Sha => {
            layout.branch_w
                + if layout.show_graph {
                    layout.graph_w
                } else {
                    px(0.0)
                }
                + if layout.show_author {
                    layout.author_w
                } else {
                    px(0.0)
                }
                + if layout.show_date {
                    layout.date_w
                } else {
                    px(0.0)
                }
        }
    };

    HistoryColumnResizeDragParams {
        start_width,
        drag_delta_sign,
        min_width,
        static_max_width,
        other_fixed_width,
    }
}

pub(in crate::view) fn history_column_resize_max_width(
    params: HistoryColumnResizeDragParams,
    available_width: Pixels,
    ui_scale_percent: u32,
) -> Pixels {
    let dynamic_max =
        (available_width - params.other_fixed_width - history_message_min_width(ui_scale_percent))
            .max(params.min_width);
    params
        .static_max_width
        .min(dynamic_max)
        .max(params.min_width)
}

pub(in crate::view) fn history_column_resize_state(
    handle: HistoryColResizeHandle,
    start_x: Pixels,
    available_width: Pixels,
    layout: HistoryColumnDragLayout,
    scale: impl Into<ui_scale::UiScale>,
) -> HistoryColResizeState {
    let scale = scale.into();
    let ui_scale_percent = scale.percent();
    let visible_columns =
        history_visible_columns_for_layout(available_width, layout, ui_scale_percent);
    let params = history_column_resize_drag_params(
        handle,
        HistoryColumnDragLayout {
            show_author: visible_columns.0,
            show_date: visible_columns.1,
            show_sha: visible_columns.2,
            ..layout
        },
        scale,
    );
    HistoryColResizeState {
        handle,
        start_x,
        start_width: params.start_width,
        current_width: params.start_width,
        drag_delta_sign: params.drag_delta_sign,
        min_width: params.min_width,
        static_max_width: params.static_max_width,
        other_fixed_width: params.other_fixed_width,
        bounds_available_width: available_width,
        max_width: history_column_resize_max_width(params, available_width, ui_scale_percent),
        visible_columns,
    }
}

#[inline]
pub(in crate::view) fn history_resize_state_visible_columns(
    available: Pixels,
    resize_state: Option<&HistoryColResizeState>,
) -> Option<(bool, bool, bool)> {
    let state = resize_state?;
    if available <= px(0.0)
        || state.bounds_available_width != available
        || state.current_width < state.min_width
        || state.current_width > state.max_width
    {
        return None;
    }

    Some(state.visible_columns)
}

#[cfg(test)]
#[inline]
pub(in crate::view) fn history_resize_state_visible_columns_for_current_width(
    available: Pixels,
    current_width: Pixels,
    resize_state: Option<&HistoryColResizeState>,
) -> Option<(bool, bool, bool)> {
    let state = resize_state?;
    if current_width != state.current_width {
        return None;
    }

    history_resize_state_visible_columns(available, Some(state))
}

pub(in crate::view) fn history_column_drag_clamped_width_for_state(
    state: &mut HistoryColResizeState,
    current_x: Pixels,
    available_width: Pixels,
    ui_scale_percent: u32,
) -> Pixels {
    if state.bounds_available_width != available_width {
        let params = HistoryColumnResizeDragParams {
            start_width: state.start_width,
            drag_delta_sign: state.drag_delta_sign,
            min_width: state.min_width,
            static_max_width: state.static_max_width,
            other_fixed_width: state.other_fixed_width,
        };
        state.max_width =
            history_column_resize_max_width(params, available_width, ui_scale_percent);
        state.bounds_available_width = available_width;
    }

    let dx = current_x - state.start_x;
    let next = (state.start_width + (dx * state.drag_delta_sign))
        .max(state.min_width)
        .min(state.max_width);
    state.current_width = next;
    next
}

pub(super) fn history_column_drag_clamped_width(
    handle: HistoryColResizeHandle,
    candidate: Pixels,
    available_width: Pixels,
    layout: HistoryColumnDragLayout,
    scale: impl Into<ui_scale::UiScale>,
) -> Pixels {
    let scale = scale.into();
    let params = history_column_resize_drag_params(handle, layout, scale);
    candidate
        .max(params.min_width)
        .min(history_column_resize_max_width(
            params,
            available_width,
            scale.percent(),
        ))
}

pub(super) fn history_column_width_for_handle(
    layout: HistoryColumnDragLayout,
    handle: HistoryColResizeHandle,
) -> Pixels {
    match handle {
        HistoryColResizeHandle::Branch => layout.branch_w,
        HistoryColResizeHandle::Graph => layout.graph_w,
        HistoryColResizeHandle::Author => layout.author_w,
        HistoryColResizeHandle::Date => layout.date_w,
        HistoryColResizeHandle::Sha => layout.sha_w,
    }
}

#[cfg(test)]
pub(in crate::view) fn history_resize_state_preserves_visible_columns(
    available: Pixels,
    layout: HistoryColumnDragLayout,
    resize_state: Option<&HistoryColResizeState>,
) -> bool {
    let current_width =
        resize_state.map(|state| history_column_width_for_handle(layout, state.handle));
    history_resize_state_visible_columns_for_current_width(
        available,
        current_width.unwrap_or(px(0.0)),
        resize_state,
    )
    .is_some()
}

pub(in crate::view) fn history_visible_columns_for_layout_with_resize_state(
    available: Pixels,
    layout: HistoryColumnDragLayout,
    resize_state: Option<&HistoryColResizeState>,
    ui_scale_percent: u32,
) -> (bool, bool, bool) {
    if let Some(state) = resize_state {
        let current_width = history_column_width_for_handle(layout, state.handle);
        if current_width == state.current_width
            && let Some(columns) = history_resize_state_visible_columns(available, Some(state))
        {
            return columns;
        }
    }

    history_visible_columns_for_layout(available, layout, ui_scale_percent)
}

pub(in crate::view) fn history_visible_columns_for_layout(
    available: Pixels,
    layout: HistoryColumnDragLayout,
    ui_scale_percent: u32,
) -> (bool, bool, bool) {
    if available <= px(0.0) {
        return (false, false, false);
    }

    let min_message = history_message_min_width(ui_scale_percent);

    let mut show_author = layout.show_author;
    let mut show_date = layout.show_date;
    let mut show_sha = layout.show_sha;

    let fixed_base = layout.branch_w
        + if layout.show_graph {
            layout.graph_w
        } else {
            px(0.0)
        };
    let mut fixed = fixed_base
        + if show_author {
            layout.author_w
        } else {
            px(0.0)
        }
        + if show_date { layout.date_w } else { px(0.0) }
        + if show_sha { layout.sha_w } else { px(0.0) };

    if available - fixed < min_message && show_sha {
        show_sha = false;
        fixed -= layout.sha_w;
    }
    if available - fixed < min_message {
        if show_date {
            show_date = false;
            fixed -= layout.date_w;
        }
        show_sha = false;
    }
    if available - fixed < min_message && show_author {
        show_author = false;
        fixed -= layout.author_w;
    }

    if available - fixed < min_message {
        show_author = false;
        show_date = false;
        show_sha = false;
    }

    (show_author, show_date, show_sha)
}
