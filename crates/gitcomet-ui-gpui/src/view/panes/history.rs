use super::super::*;
use crate::view::caches::{
    HistoryShortShaVm, HistoryVisibleIndices, HistoryWhenVm, analyze_history_stashes,
    build_history_branch_text_by_target, build_history_tag_names_by_target,
    build_history_visible_indices, next_history_stash_tip_for_commit_ix,
};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

mod history_panel;

pub(in super::super) fn history_scrollbar_gutter() -> Pixels {
    crate::view::components::Scrollbar::gutter(crate::view::components::ScrollbarAxis::Vertical)
}

fn history_columns_available_width(content_width: Pixels) -> Pixels {
    (content_width - history_scrollbar_gutter()).max(px(0.0))
}

fn history_scale(ui_scale_percent: u32) -> ui_scale::UiScale {
    ui_scale::UiScale::from_percent(ui_scale_percent)
}

fn history_scaled_px(value: f32, ui_scale_percent: u32) -> Pixels {
    history_scale(ui_scale_percent).px(value)
}

fn history_message_min_width(ui_scale_percent: u32) -> Pixels {
    history_scaled_px(HISTORY_COL_MESSAGE_MIN_PX, ui_scale_percent)
}

fn graph_branch_heads<'a>(
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

fn history_column_static_bounds(
    handle: HistoryColResizeHandle,
    ui_scale_percent: u32,
) -> (Pixels, Pixels) {
    match handle {
        HistoryColResizeHandle::Branch => (
            history_scaled_px(HISTORY_COL_BRANCH_MIN_PX, ui_scale_percent),
            history_scaled_px(HISTORY_COL_BRANCH_MAX_PX, ui_scale_percent),
        ),
        HistoryColResizeHandle::Graph => (
            history_scaled_px(HISTORY_COL_GRAPH_MIN_PX, ui_scale_percent),
            history_scaled_px(HISTORY_COL_GRAPH_MAX_PX, ui_scale_percent),
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
struct HistoryColumnWidths {
    branch: Pixels,
    graph: Pixels,
    author: Pixels,
    date: Pixels,
    sha: Pixels,
}

#[derive(Copy, Clone, Debug, PartialEq)]
struct HistoryColumnDesignWidths {
    branch: f32,
    graph: f32,
    author: f32,
    date: f32,
    sha: f32,
}

fn default_history_column_design_widths() -> HistoryColumnDesignWidths {
    HistoryColumnDesignWidths {
        branch: HISTORY_COL_BRANCH_PX,
        graph: HISTORY_COL_GRAPH_PX,
        author: HISTORY_COL_AUTHOR_PX,
        date: HISTORY_COL_DATE_PX,
        sha: HISTORY_COL_SHA_PX,
    }
}

fn scaled_history_column_widths(
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

fn default_history_column_widths(ui_scale_percent: u32) -> HistoryColumnWidths {
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

fn history_visible_columns_for_width(
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

fn history_column_drag_next_width(
    handle: HistoryColResizeHandle,
    candidate: Pixels,
    available_width: Pixels,
    show_graph: bool,
    preferred: (bool, bool, bool),
    widths: HistoryColumnWidths,
    ui_scale_percent: u32,
) -> Pixels {
    let (show_author, show_date, show_sha) = history_visible_columns_for_width(
        available_width,
        show_graph,
        preferred,
        widths,
        ui_scale_percent,
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
        ui_scale_percent,
    )
}

fn history_reset_widths_for_available_width(
    available_width: Pixels,
    show_graph: bool,
    preferred: (bool, bool, bool),
    ui_scale_percent: u32,
) -> HistoryColumnWidths {
    let mut widths = default_history_column_widths(ui_scale_percent);
    widths.graph = history_column_drag_next_width(
        HistoryColResizeHandle::Graph,
        widths.graph,
        available_width,
        show_graph,
        preferred,
        widths,
        ui_scale_percent,
    );
    widths.branch = history_column_drag_next_width(
        HistoryColResizeHandle::Branch,
        widths.branch,
        available_width,
        show_graph,
        preferred,
        widths,
        ui_scale_percent,
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
    ui_scale_percent: u32,
) -> HistoryColumnResizeDragParams {
    let (start_width, drag_delta_sign) = match handle {
        HistoryColResizeHandle::Branch => (layout.branch_w, 1.0),
        HistoryColResizeHandle::Graph => (layout.graph_w, 1.0),
        HistoryColResizeHandle::Author => (layout.author_w, -1.0),
        HistoryColResizeHandle::Date => (layout.date_w, -1.0),
        HistoryColResizeHandle::Sha => (layout.sha_w, -1.0),
    };
    let (min_width, static_max_width) = history_column_static_bounds(handle, ui_scale_percent);
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
    ui_scale_percent: u32,
) -> HistoryColResizeState {
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
        ui_scale_percent,
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

fn history_column_drag_clamped_width(
    handle: HistoryColResizeHandle,
    candidate: Pixels,
    available_width: Pixels,
    layout: HistoryColumnDragLayout,
    ui_scale_percent: u32,
) -> Pixels {
    let params = history_column_resize_drag_params(handle, layout, ui_scale_percent);
    candidate
        .max(params.min_width)
        .min(history_column_resize_max_width(
            params,
            available_width,
            ui_scale_percent,
        ))
}

fn history_column_width_for_handle(
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct HistorySelectedListIndexCache {
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    show_working_tree_summary_row: bool,
    selected_commit: Option<CommitId>,
    list_ix: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingHistoryReveal {
    repo_id: RepoId,
    commit_id: CommitId,
    fallback_scope: Option<LogScope>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PendingHistoryRevealDecision {
    set_scope: Option<LogScope>,
    select_commit: bool,
    scroll_to_list_ix: Option<usize>,
    load_more: bool,
    clear_pending: bool,
}

fn history_selected_list_index_cache_matches(
    cache: &HistorySelectedListIndexCache,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    show_working_tree_summary_row: bool,
    selected_commit: Option<&CommitId>,
) -> bool {
    cache.repo_id == repo_id
        && cache.log_rev == log_rev
        && cache.stashes_rev == stashes_rev
        && cache.history_scope == history_scope
        && cache.show_working_tree_summary_row == show_working_tree_summary_row
        && cache.selected_commit.as_ref() == selected_commit
}

fn set_history_selected_list_index_cache(
    cache: &mut Option<HistorySelectedListIndexCache>,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    show_working_tree_summary_row: bool,
    selected_commit: Option<CommitId>,
    list_ix: usize,
) {
    *cache = Some(HistorySelectedListIndexCache {
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        show_working_tree_summary_row,
        selected_commit,
        list_ix,
    });
}

fn peek_history_selected_list_index(
    cache: Option<&HistorySelectedListIndexCache>,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    show_working_tree_summary_row: bool,
    selected_commit: Option<&CommitId>,
    visible_indices: &HistoryVisibleIndices,
    commits: &[Commit],
) -> Option<usize> {
    if show_working_tree_summary_row && selected_commit.is_none() {
        return Some(0);
    }

    if let Some(list_ix) = cache
        .filter(|entry| {
            history_selected_list_index_cache_matches(
                entry,
                repo_id,
                log_rev,
                stashes_rev,
                history_scope,
                show_working_tree_summary_row,
                selected_commit,
            )
        })
        .map(|entry| entry.list_ix)
    {
        return Some(list_ix);
    }

    let selected_commit = selected_commit?;
    let offset = usize::from(show_working_tree_summary_row);
    let visible_ix = visible_indices.iter().position(|commit_ix| {
        commits
            .get(commit_ix)
            .is_some_and(|commit| &commit.id == selected_commit)
    })?;
    Some(visible_ix + offset)
}

fn resolve_history_selected_list_index(
    cache: &mut Option<HistorySelectedListIndexCache>,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    show_working_tree_summary_row: bool,
    selected_commit: Option<&CommitId>,
    visible_indices: &HistoryVisibleIndices,
    commits: &[Commit],
) -> Option<usize> {
    let list_ix = peek_history_selected_list_index(
        cache.as_ref(),
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        show_working_tree_summary_row,
        selected_commit,
        visible_indices,
        commits,
    )?;
    set_history_selected_list_index_cache(
        cache,
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        show_working_tree_summary_row,
        selected_commit.cloned(),
        list_ix,
    );
    Some(list_ix)
}

#[allow(clippy::too_many_arguments)]
fn decide_pending_history_reveal(
    pending: &PendingHistoryReveal,
    active_repo_id: Option<RepoId>,
    current_scope: Option<LogScope>,
    selected_commit: Option<&CommitId>,
    log_rev: u64,
    stashes_rev: u64,
    log_loading_more: bool,
    display_page: Option<&LogPage>,
    live_page_has_more: Option<bool>,
    cache_request_matches: bool,
    visible_indices: Option<&HistoryVisibleIndices>,
    show_working_tree_summary_row: bool,
    selected_list_index_cache: Option<&HistorySelectedListIndexCache>,
) -> PendingHistoryRevealDecision {
    let mut decision = PendingHistoryRevealDecision::default();

    if active_repo_id != Some(pending.repo_id) {
        decision.clear_pending = true;
        return decision;
    }

    let Some(current_scope) = current_scope else {
        decision.clear_pending = true;
        return decision;
    };

    decision.select_commit = selected_commit != Some(&pending.commit_id);

    let Some(display_page) = display_page else {
        return decision;
    };
    if !cache_request_matches {
        return decision;
    }
    let Some(visible_indices) = visible_indices else {
        return decision;
    };

    if let Some(list_ix) = peek_history_selected_list_index(
        selected_list_index_cache,
        pending.repo_id,
        log_rev,
        stashes_rev,
        current_scope,
        show_working_tree_summary_row,
        Some(&pending.commit_id),
        visible_indices,
        &display_page.commits,
    ) {
        decision.scroll_to_list_ix = Some(list_ix);
        decision.clear_pending = true;
        return decision;
    }

    match live_page_has_more {
        Some(true) => {
            decision.load_more = !log_loading_more;
            return decision;
        }
        Some(false) => {}
        None => return decision,
    }

    if let Some(fallback_scope) = pending.fallback_scope
        && current_scope != fallback_scope
    {
        decision.set_scope = Some(fallback_scope);
        return decision;
    }

    decision.clear_pending = true;
    decision
}

pub(in super::super) struct HistoryView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) ui_scale_percent: u32,
    pub(in super::super) date_time_format: DateTimeFormat,
    pub(in super::super) timezone: Timezone,
    pub(in super::super) show_timezone: bool,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    tooltip_host: WeakEntity<TooltipHost>,
    notify_fingerprint: u64,
    pub(in super::super) active_context_menu_invoker: Option<SharedString>,
    pub(in super::super) last_window_size: Size<Pixels>,
    pub(in super::super) history_content_width: Pixels,

    pub(in super::super) history_cache_seq: u64,
    pub(in super::super) history_cache_inflight: Option<HistoryCacheBuildRequest>,
    history_col_branch_design: f32,
    history_col_graph_design: f32,
    history_col_author_design: f32,
    history_col_date_design: f32,
    history_col_sha_design: f32,
    pub(in super::super) history_col_branch: Pixels,
    pub(in super::super) history_col_graph: Pixels,
    pub(in super::super) history_col_author: Pixels,
    pub(in super::super) history_col_date: Pixels,
    pub(in super::super) history_col_sha: Pixels,
    pub(in super::super) history_show_graph: bool,
    pub(in super::super) history_show_author: bool,
    pub(in super::super) history_show_date: bool,
    pub(in super::super) history_show_sha: bool,
    pub(in super::super) history_show_tags: bool,
    pub(in super::super) history_auto_fetch_tags_on_repo_activation: bool,
    pub(in super::super) history_col_graph_auto: bool,
    pub(in super::super) history_col_resize: Option<HistoryColResizeState>,
    pub(in super::super) history_cache: Option<HistoryCache>,
    history_selected_list_index_cache: Option<HistorySelectedListIndexCache>,
    selected_branch: Option<SelectedBranch>,
    pending_history_reveal: Option<PendingHistoryReveal>,
    pub(in super::super) history_worktree_summary_cache: Option<HistoryWorktreeSummaryCache>,
    pub(in super::super) history_stash_ids_cache: Option<HistoryStashIdsCache>,
    pub(in super::super) history_scroll: UniformListScrollHandle,
    pub(in super::super) history_panel_focus_handle: FocusHandle,
}

impl HistoryView {
    fn notify_fingerprint_for(state: &AppState, show_history_tags: bool) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.log_rev.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.detached_head_commit.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.remote_branches_rev.hash(&mut hasher);
            if show_history_tags {
                repo.tags_rev.hash(&mut hasher);
            }
            repo.stashes_rev.hash(&mut hasher);
            repo.history_state.selected_commit_rev.hash(&mut hasher);
            repo.worktree_status_cache_rev().hash(&mut hasher);
            repo.staged_status_cache_rev().hash(&mut hasher);
        }

        hasher.finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        ui_scale_percent: u32,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        history_show_graph: bool,
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        history_show_tags: bool,
        history_auto_fetch_tags_on_repo_activation: bool,
        root_view: WeakEntity<GitCometView>,
        tooltip_host: WeakEntity<TooltipHost>,
        last_window_size: Size<Pixels>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state, history_show_tags);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next, this.history_show_tags);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.state = next;
            cx.notify();
        });

        let history_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let default_design_widths = default_history_column_design_widths();
        let scale = ui_scale::UiScale::from_percent(ui_scale_percent);
        let default_widths = scaled_history_column_widths(default_design_widths, scale);

        Self {
            store,
            state,
            theme,
            ui_scale_percent,
            date_time_format,
            timezone,
            show_timezone,
            _ui_model_subscription: subscription,
            root_view,
            tooltip_host,
            notify_fingerprint: initial_fingerprint,
            active_context_menu_invoker: None,
            last_window_size,
            history_content_width: history_columns_available_width(last_window_size.width),
            history_cache_seq: 0,
            history_cache_inflight: None,
            history_col_branch_design: default_design_widths.branch,
            history_col_graph_design: default_design_widths.graph,
            history_col_author_design: default_design_widths.author,
            history_col_date_design: default_design_widths.date,
            history_col_sha_design: default_design_widths.sha,
            history_col_branch: default_widths.branch,
            history_col_graph: default_widths.graph,
            history_col_author: default_widths.author,
            history_col_date: default_widths.date,
            history_col_sha: default_widths.sha,
            history_show_graph,
            history_show_author,
            history_show_date,
            history_show_sha,
            history_show_tags,
            history_auto_fetch_tags_on_repo_activation,
            history_col_graph_auto: true,
            history_col_resize: None,
            history_cache: None,
            history_selected_list_index_cache: None,
            selected_branch: None,
            pending_history_reveal: None,
            history_worktree_summary_cache: None,
            history_stash_ids_cache: None,
            history_scroll: UniformListScrollHandle::default(),
            history_panel_focus_handle,
        }
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    pub(in crate::view) fn display_log_page_for_repo(repo: &RepoState) -> Option<Arc<LogPage>> {
        match &repo.log {
            Loadable::Ready(page) => Some(Arc::clone(page)),
            Loadable::Loading => repo
                .history_state
                .retained_log_while_loading
                .as_ref()
                .map(Arc::clone),
            Loadable::NotLoaded | Loadable::Error(_) => None,
        }
    }

    fn live_log_page_has_more_for_repo(repo: &RepoState) -> Option<bool> {
        match &repo.log {
            Loadable::Ready(page) => Some(page.next_cursor.is_some()),
            Loadable::Loading | Loadable::NotLoaded | Loadable::Error(_) => None,
        }
    }

    fn history_base_cache_request_for_repo(
        &self,
        repo: &RepoState,
        page: &LogPage,
    ) -> HistoryBaseCacheRequest {
        HistoryBaseCacheRequest {
            repo_id: repo.id,
            history_scope: repo.history_state.history_scope,
            log_fingerprint: Self::log_fingerprint(&page.commits),
            head_branch_rev: repo.head_branch_rev,
            detached_head_commit: repo.detached_head_commit.clone(),
            branches_rev: repo.branches_rev,
            remote_branches_rev: if repo.history_state.history_scope.is_current_branch_mode() {
                0
            } else {
                repo.remote_branches_rev
            },
            stashes_rev: repo.stashes_rev,
        }
    }

    pub(in crate::view) fn ui_scale(&self) -> ui_scale::UiScale {
        history_scale(self.ui_scale_percent)
    }

    fn sync_history_column_widths_from_design(&mut self) {
        let scale = self.ui_scale();
        self.history_col_branch = scale.px(self.history_col_branch_design);
        self.history_col_graph = scale.px(self.history_col_graph_design);
        self.history_col_author = scale.px(self.history_col_author_design);
        self.history_col_date = scale.px(self.history_col_date_design);
        self.history_col_sha = scale.px(self.history_col_sha_design);
    }

    fn sync_history_column_design_widths_from_pixels(&mut self) {
        let scale = self.ui_scale();
        self.history_col_branch_design = scale.design_units_from_pixels(self.history_col_branch);
        self.history_col_graph_design = scale.design_units_from_pixels(self.history_col_graph);
        self.history_col_author_design = scale.design_units_from_pixels(self.history_col_author);
        self.history_col_date_design = scale.design_units_from_pixels(self.history_col_date);
        self.history_col_sha_design = scale.design_units_from_pixels(self.history_col_sha);
    }

    fn history_decoration_cache_request_for_repo(
        &self,
        repo: &RepoState,
        page: &LogPage,
    ) -> HistoryDecorationCacheRequest {
        HistoryDecorationCacheRequest {
            base_request: self.history_base_cache_request_for_repo(repo, page),
            head_branch_rev: repo.head_branch_rev,
            detached_head_commit: repo.detached_head_commit.clone(),
            branches_rev: repo.branches_rev,
            remote_branches_rev: repo.remote_branches_rev,
            tags_rev: if self.history_show_tags {
                repo.tags_rev
            } else {
                0
            },
        }
    }

    pub(in crate::view) fn request_reveal_commit(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = PendingHistoryReveal {
            repo_id,
            commit_id,
            fallback_scope,
        };
        if self.pending_history_reveal.as_ref() != Some(&next) {
            self.pending_history_reveal = Some(next);
        }
        self.drive_pending_history_reveal(cx);
        cx.notify();
    }

    pub(in crate::view) fn set_selected_branch(
        &mut self,
        repo_id: RepoId,
        section: BranchSection,
        name: &str,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some(SelectedBranch {
            repo_id,
            section,
            name: name.to_string(),
        });
        if self.selected_branch.as_ref() == next.as_ref() {
            return;
        }
        self.selected_branch = next;
        cx.notify();
    }

    pub(in super::super) fn selected_branch_entry_text_for_history_row(
        &self,
        repo_id: RepoId,
        is_head: bool,
        selected: bool,
    ) -> Option<SharedString> {
        selected_branch_history_entry_text(
            self.selected_branch.as_ref(),
            repo_id,
            is_head,
            selected,
        )
    }

    pub(in super::super) fn history_visible_column_preferences(&self) -> (bool, bool, bool, bool) {
        (
            self.history_show_graph,
            self.history_show_author,
            self.history_show_date,
            self.history_show_sha,
        )
    }

    pub(in super::super) fn history_visible_columns(&self) -> (bool, bool, bool, bool) {
        let available = self.history_content_width;
        let layout = HistoryColumnDragLayout {
            show_graph: self.history_show_graph,
            show_author: self.history_show_author,
            show_date: self.history_show_date,
            show_sha: self.history_show_sha,
            branch_w: self.history_col_branch,
            graph_w: self.history_col_graph,
            author_w: self.history_col_author,
            date_w: self.history_col_date,
            sha_w: self.history_col_sha,
        };
        let (show_author, show_date, show_sha) =
            history_visible_columns_for_layout_with_resize_state(
                available,
                layout,
                self.history_col_resize.as_ref(),
                self.ui_scale_percent,
            );
        (self.history_show_graph, show_author, show_date, show_sha)
    }

    pub(in super::super) fn reset_history_column_widths(&mut self) {
        let widths = history_reset_widths_for_available_width(
            self.history_content_width,
            self.history_show_graph,
            (
                self.history_show_author,
                self.history_show_date,
                self.history_show_sha,
            ),
            self.ui_scale_percent,
        );
        self.history_col_branch = widths.branch;
        self.history_col_graph = widths.graph;
        self.history_col_author = widths.author;
        self.history_col_date = widths.date;
        self.history_col_sha = widths.sha;
        self.sync_history_column_design_widths_from_pixels();
        self.history_col_graph_auto = true;
        self.history_col_resize = None;
    }

    pub(in super::super) fn history_column_width_mut(
        &mut self,
        handle: HistoryColResizeHandle,
    ) -> &mut Pixels {
        match handle {
            HistoryColResizeHandle::Branch => &mut self.history_col_branch,
            HistoryColResizeHandle::Graph => &mut self.history_col_graph,
            HistoryColResizeHandle::Author => &mut self.history_col_author,
            HistoryColResizeHandle::Date => &mut self.history_col_date,
            HistoryColResizeHandle::Sha => &mut self.history_col_sha,
        }
    }

    pub(in super::super) fn set_theme(&mut self, theme: AppTheme, cx: &mut gpui::Context<Self>) {
        self.theme = theme;
        cx.notify();
    }

    pub(in super::super) fn set_active_context_menu_invoker(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.active_context_menu_invoker == next {
            return;
        }
        self.active_context_menu_invoker = next;
        cx.notify();
    }

    pub(in super::super) fn apply_ui_scale_percent(
        &mut self,
        previous_percent: u32,
        next_percent: u32,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.ui_scale_percent == next_percent {
            return;
        }

        debug_assert_eq!(self.ui_scale_percent, previous_percent);
        self.sync_history_column_design_widths_from_pixels();
        self.ui_scale_percent = next_percent;
        self.history_col_resize = None;
        self.sync_history_column_widths_from_design();
        cx.notify();
    }

    pub(in super::super) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        cx.notify();
    }

    pub(in super::super) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
        cx.notify();
    }

    pub(in super::super) fn set_show_timezone(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.show_timezone == enabled {
            return;
        }
        self.show_timezone = enabled;
        cx.notify();
    }

    pub(in super::super) fn history_tag_preferences(&self) -> (bool, bool) {
        (
            self.history_show_tags,
            self.history_auto_fetch_tags_on_repo_activation,
        )
    }

    pub(in super::super) fn set_history_column_preferences(
        &mut self,
        show_graph: bool,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_show_graph == show_graph
            && self.history_show_author == show_author
            && self.history_show_date == show_date
            && self.history_show_sha == show_sha
        {
            return;
        }

        self.history_show_graph = show_graph;
        self.history_show_author = show_author;
        self.history_show_date = show_date;
        self.history_show_sha = show_sha;
        self.history_col_resize = None;
        cx.notify();
    }

    pub(in super::super) fn set_history_tag_preferences(
        &mut self,
        show_tags: bool,
        auto_fetch_tags_on_repo_activation: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_show_tags == show_tags
            && self.history_auto_fetch_tags_on_repo_activation == auto_fetch_tags_on_repo_activation
        {
            return;
        }

        let show_tags_changed = self.history_show_tags != show_tags;
        self.history_show_tags = show_tags;
        self.history_auto_fetch_tags_on_repo_activation = auto_fetch_tags_on_repo_activation;
        if show_tags_changed {
            self.notify_fingerprint = Self::notify_fingerprint_for(&self.state, show_tags);
            self.history_cache_inflight = None;
        }
        cx.notify();
    }

    pub(in super::super) fn set_last_window_size(&mut self, size: Size<Pixels>) {
        self.last_window_size = size;
    }

    pub(in super::super) fn set_history_content_width(&mut self, width: Pixels) {
        self.history_content_width = history_columns_available_width(width);
    }

    pub(in super::super) fn open_popover_at(
        &mut self,
        kind: PopoverKind,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_at(kind, anchor, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn open_popover_for_bounds(
        &mut self,
        kind: PopoverKind,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.open_popover_for_bounds(kind, anchor_bounds, window, cx);
                });
            });
        });
    }

    pub(in super::super) fn activate_context_menu_invoker(
        &mut self,
        invoker: SharedString,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, move |root, cx| {
            root.set_active_context_menu_invoker(Some(invoker), cx);
        });
    }

    pub(in super::super) fn set_tooltip_text_if_changed(
        &mut self,
        next: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.set_tooltip_text_if_changed(next, cx));
        false
    }

    pub(in super::super) fn clear_tooltip_if_matches(
        &mut self,
        tooltip: &SharedString,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let tooltip = tooltip.clone();
        let _ = self
            .tooltip_host
            .update(cx, |host, cx| host.clear_tooltip_if_matches(&tooltip, cx));
        false
    }

    pub(in crate::view) fn drive_pending_history_reveal(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(pending) = self.pending_history_reveal.clone() else {
            return;
        };

        let (show_working_tree_summary_row, _) = self.ensure_history_worktree_summary_cache();
        let (
            active_repo_id,
            current_scope,
            log_rev,
            stashes_rev,
            page,
            cache_request_matches,
            decision,
        ) = {
            let active_repo_id = self.active_repo_id();
            let Some(repo) = self.active_repo() else {
                let decision = decide_pending_history_reveal(
                    &pending,
                    active_repo_id,
                    None,
                    None,
                    0,
                    0,
                    false,
                    None,
                    None,
                    false,
                    None,
                    show_working_tree_summary_row,
                    self.history_selected_list_index_cache.as_ref(),
                );
                return self.finish_pending_history_reveal(decision, pending, None, cx);
            };

            let current_scope = repo.history_state.history_scope;
            let log_rev = repo.log_rev;
            let stashes_rev = repo.stashes_rev;
            let log_loading_more = repo.history_state.log_loading_more;
            let display_page = Self::display_log_page_for_repo(repo);
            let live_page_has_more = Self::live_log_page_has_more_for_repo(repo);
            let cache_request_matches = display_page.as_ref().is_some_and(|page| {
                let request = self.history_base_cache_request_for_repo(repo, page.as_ref());
                self.history_cache
                    .as_ref()
                    .is_some_and(|cache| cache.base.request == request)
            });
            let visible_indices = if cache_request_matches {
                self.history_cache
                    .as_ref()
                    .map(|cache| &cache.base.visible_indices)
            } else {
                None
            };
            let decision = decide_pending_history_reveal(
                &pending,
                active_repo_id,
                Some(current_scope),
                repo.history_state.selected_commit.as_ref(),
                log_rev,
                stashes_rev,
                log_loading_more,
                display_page.as_deref(),
                live_page_has_more,
                cache_request_matches,
                visible_indices,
                show_working_tree_summary_row,
                self.history_selected_list_index_cache.as_ref(),
            );

            (
                active_repo_id,
                current_scope,
                log_rev,
                stashes_rev,
                display_page,
                cache_request_matches,
                decision,
            )
        };

        let cache_meta =
            (active_repo_id == Some(pending.repo_id) && page.is_some() && cache_request_matches)
                .then_some((
                    log_rev,
                    stashes_rev,
                    current_scope,
                    show_working_tree_summary_row,
                ));

        self.finish_pending_history_reveal(decision, pending, cache_meta, cx);
    }

    fn finish_pending_history_reveal(
        &mut self,
        decision: PendingHistoryRevealDecision,
        pending: PendingHistoryReveal,
        cache_meta: Option<(u64, u64, LogScope, bool)>,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(scope) = decision.set_scope {
            self.store.dispatch(Msg::SetHistoryScope {
                repo_id: pending.repo_id,
                scope,
            });
            return;
        }

        if decision.select_commit {
            self.store.dispatch(Msg::SelectCommit {
                repo_id: pending.repo_id,
                commit_id: pending.commit_id.clone(),
            });
        }

        if let Some(list_ix) = decision.scroll_to_list_ix {
            if let Some((log_rev, stashes_rev, history_scope, show_working_tree_summary_row)) =
                cache_meta
            {
                set_history_selected_list_index_cache(
                    &mut self.history_selected_list_index_cache,
                    pending.repo_id,
                    log_rev,
                    stashes_rev,
                    history_scope,
                    show_working_tree_summary_row,
                    Some(pending.commit_id.clone()),
                    list_ix,
                );
            }
            self.history_scroll
                .scroll_to_item_strict(list_ix, gpui::ScrollStrategy::Center);
        } else if decision.load_more {
            self.store.dispatch(Msg::LoadMoreHistory {
                repo_id: pending.repo_id,
            });
        }

        if decision.clear_pending {
            self.pending_history_reveal = None;
            cx.notify();
        }
    }
}

// Render impl is in history_panel.rs

// --- History cache methods ---

use gitcomet_core::domain::{LogPage, LogScope, RemoteBranch, StashEntry};

impl HistoryView {
    pub(in super::super) fn ensure_history_worktree_summary_cache(
        &mut self,
    ) -> (bool, (usize, usize, usize)) {
        enum Action {
            Clear,
            CacheOk {
                show_row: bool,
                counts: (usize, usize, usize),
            },
            Rebuild {
                repo_id: RepoId,
                worktree_status_rev: u64,
                staged_status_rev: u64,
                show_row: bool,
                counts: (usize, usize, usize),
            },
        }

        let action = (|| {
            let Some(repo) = self.active_repo() else {
                return Action::Clear;
            };
            let worktree = repo.worktree_status_entries();
            let staged = repo.staged_status_entries();
            if worktree.is_none() && staged.is_none() {
                return Action::Clear;
            }

            let worktree_status_rev = repo.worktree_status_cache_rev();
            let staged_status_rev = repo.staged_status_cache_rev();

            if let Some(cache) = &self.history_worktree_summary_cache
                && cache.repo_id == repo.id
                && cache.worktree_status_rev == worktree_status_rev
                && cache.staged_status_rev == staged_status_rev
            {
                return Action::CacheOk {
                    show_row: cache.show_row,
                    counts: cache.counts,
                };
            }

            let count_for = |entries: &[FileStatus]| {
                let mut added = 0usize;
                let mut modified = 0usize;
                let mut deleted = 0usize;
                for entry in entries {
                    match entry.kind {
                        FileStatusKind::Untracked | FileStatusKind::Added => added += 1,
                        FileStatusKind::Deleted => deleted += 1,
                        FileStatusKind::Modified
                        | FileStatusKind::Renamed
                        | FileStatusKind::Conflicted => modified += 1,
                    }
                }
                (added, modified, deleted)
            };

            let unstaged_counts = worktree.map_or((0, 0, 0), count_for);
            let staged_counts = staged.map_or((0, 0, 0), count_for);
            let show_row = worktree.is_some_and(|entries| !entries.is_empty())
                || staged.is_some_and(|entries| !entries.is_empty());
            let counts = (
                unstaged_counts.0 + staged_counts.0,
                unstaged_counts.1 + staged_counts.1,
                unstaged_counts.2 + staged_counts.2,
            );

            Action::Rebuild {
                repo_id: repo.id,
                worktree_status_rev,
                staged_status_rev,
                show_row,
                counts,
            }
        })();

        match action {
            Action::Clear => {
                self.history_worktree_summary_cache = None;
                (false, (0, 0, 0))
            }
            Action::CacheOk { show_row, counts } => (show_row, counts),
            Action::Rebuild {
                repo_id,
                worktree_status_rev,
                staged_status_rev,
                show_row,
                counts,
            } => {
                self.history_worktree_summary_cache = Some(HistoryWorktreeSummaryCache {
                    repo_id,
                    worktree_status_rev,
                    staged_status_rev,
                    show_row,
                    counts,
                });
                (show_row, counts)
            }
        }
    }

    pub(in super::super) fn ensure_history_stash_ids_cache(
        &mut self,
    ) -> Option<Arc<HashSet<CommitId>>> {
        enum Action {
            Clear,
            CacheOk(Arc<HashSet<CommitId>>),
            Rebuild {
                repo_id: RepoId,
                stashes_rev: u64,
                ids: Arc<HashSet<CommitId>>,
            },
        }

        let action = (|| {
            let Some(repo) = self.active_repo() else {
                return Action::Clear;
            };
            let Loadable::Ready(stashes) = &repo.stashes else {
                return Action::Clear;
            };
            if stashes.is_empty() {
                return Action::Clear;
            }

            let stashes_rev = repo.stashes_rev;
            if let Some(cache) = &self.history_stash_ids_cache
                && cache.repo_id == repo.id
                && cache.stashes_rev == stashes_rev
            {
                return Action::CacheOk(Arc::clone(&cache.ids));
            }

            let ids: HashSet<_> = stashes.iter().map(|s| s.id.clone()).collect();
            let ids = Arc::new(ids);
            Action::Rebuild {
                repo_id: repo.id,
                stashes_rev,
                ids: Arc::clone(&ids),
            }
        })();

        match action {
            Action::Clear => {
                self.history_stash_ids_cache = None;
                None
            }
            Action::CacheOk(ids) => Some(ids),
            Action::Rebuild {
                repo_id,
                stashes_rev,
                ids,
            } => {
                self.history_stash_ids_cache = Some(HistoryStashIdsCache {
                    repo_id,
                    stashes_rev,
                    ids: Arc::clone(&ids),
                });
                Some(ids)
            }
        }
    }

    pub(in super::super) fn ensure_history_cache(&mut self, cx: &mut gpui::Context<Self>) {
        enum Next {
            Clear,
            CacheOk,
            Inflight,
            Build {
                request: HistoryCacheBuildRequest,
                page: Arc<LogPage>,
                head_branch: Option<String>,
                branches: Arc<Vec<Branch>>,
                remote_branches: Arc<Vec<RemoteBranch>>,
                tags: Arc<Vec<Tag>>,
                stashes: Arc<Vec<StashEntry>>,
                base_reuse: Option<HistoryBaseCache>,
            },
        }

        let next = if let Some(repo) = self.active_repo() {
            if let Some(page) = Self::display_log_page_for_repo(repo) {
                let base_request = self.history_base_cache_request_for_repo(repo, page.as_ref());
                let decoration_request =
                    self.history_decoration_cache_request_for_repo(repo, page.as_ref());
                let request = HistoryCacheBuildRequest {
                    base_request: base_request.clone(),
                    decoration_request,
                };

                let cache_ok = self.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request == base_request
                        && cache.decorations.request == request.decoration_request
                });
                if cache_ok {
                    Next::CacheOk
                } else if self.history_cache_inflight.as_ref() == Some(&request) {
                    Next::Inflight
                } else {
                    let base_reuse = self
                        .history_cache
                        .as_ref()
                        .filter(|cache| cache.base.request == base_request)
                        .map(|cache| cache.base.clone());
                    Next::Build {
                        request,
                        page,
                        head_branch: match &repo.head_branch {
                            Loadable::Ready(h) => Some(h.clone()),
                            _ => None,
                        },
                        branches: match &repo.branches {
                            Loadable::Ready(b) => Arc::clone(b),
                            _ => Arc::new(Vec::new()),
                        },
                        remote_branches: match &repo.remote_branches {
                            Loadable::Ready(b) => Arc::clone(b),
                            _ => Arc::new(Vec::new()),
                        },
                        tags: if self.history_show_tags {
                            match &repo.tags {
                                Loadable::Ready(t) => Arc::clone(t),
                                _ => Arc::new(Vec::new()),
                            }
                        } else {
                            Arc::new(Vec::new())
                        },
                        stashes: match &repo.stashes {
                            Loadable::Ready(s) => Arc::clone(s),
                            _ => Arc::new(Vec::new()),
                        },
                        base_reuse,
                    }
                }
            } else {
                Next::Clear
            }
        } else {
            Next::Clear
        };

        let (
            request_for_task,
            page,
            head_branch,
            branches,
            remote_branches,
            tags,
            stashes,
            base_reuse,
        ) = match next {
            Next::Clear => {
                self.history_cache_inflight = None;
                self.history_cache = None;
                return;
            }
            Next::CacheOk => {
                self.history_cache_inflight = None;
                return;
            }
            Next::Inflight => {
                return;
            }
            Next::Build {
                request,
                page,
                head_branch,
                branches,
                remote_branches,
                tags,
                stashes,
                base_reuse,
            } => (
                request,
                page,
                head_branch,
                branches,
                remote_branches,
                tags,
                stashes,
                base_reuse,
            ),
        };

        self.history_cache_seq = self.history_cache_seq.wrapping_add(1);
        let seq = self.history_cache_seq;
        self.history_cache_inflight = Some(request_for_task.clone());

        let theme = self.theme;

        cx.spawn(
            async move |view: WeakEntity<HistoryView>, cx: &mut gpui::AsyncApp| {
                let request_for_update = request_for_task.clone();
                let base_request_for_build = request_for_task.base_request.clone();
                let decoration_request_for_build = request_for_task.decoration_request.clone();

                let build_rebuild = move || {
                    let base = base_reuse.unwrap_or_else(|| {
                        build_history_base_cache(
                            base_request_for_build,
                            page.as_ref(),
                            theme,
                            head_branch.as_deref(),
                            branches.as_ref(),
                            remote_branches.as_ref(),
                            stashes.as_ref(),
                        )
                    });
                    let decorations = build_history_decoration_cache(
                        decoration_request_for_build,
                        page.as_ref(),
                        &base,
                        head_branch.as_deref(),
                        branches.as_ref(),
                        remote_branches.as_ref(),
                        tags.as_ref(),
                    );

                    HistoryCache { base, decorations }
                };

                let rebuild: HistoryCache =
                    if crate::ui_runtime::current().uses_background_compute() {
                        smol::unblock(build_rebuild).await
                    } else {
                        build_rebuild()
                    };

                let _ = view.update(cx, |this, cx| {
                    if this.history_cache_seq != seq {
                        return;
                    }
                    if this.history_cache_inflight.as_ref() != Some(&request_for_update) {
                        return;
                    }
                    if this.active_repo_id() != Some(request_for_update.base_request.repo_id) {
                        return;
                    }

                    if this.history_col_graph_auto && this.history_col_resize.is_none() {
                        let required = history_scaled_px(
                            HISTORY_GRAPH_MARGIN_X_PX * 2.0
                                + HISTORY_GRAPH_COL_GAP_PX * (rebuild.base.max_lanes as f32),
                            this.ui_scale_percent,
                        );
                        if this.history_show_graph {
                            this.history_col_graph = history_column_drag_next_width(
                                HistoryColResizeHandle::Graph,
                                required.min(history_scaled_px(
                                    HISTORY_COL_GRAPH_MAX_PX,
                                    this.ui_scale_percent,
                                )),
                                this.history_content_width,
                                this.history_show_graph,
                                (
                                    this.history_show_author,
                                    this.history_show_date,
                                    this.history_show_sha,
                                ),
                                HistoryColumnWidths {
                                    branch: this.history_col_branch,
                                    graph: this.history_col_graph,
                                    author: this.history_col_author,
                                    date: this.history_col_date,
                                    sha: this.history_col_sha,
                                },
                                this.ui_scale_percent,
                            );
                            this.history_col_graph_design = this
                                .ui_scale()
                                .design_units_from_pixels(this.history_col_graph);
                        }
                    }

                    this.history_cache_inflight = None;
                    this.history_cache = Some(rebuild);
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn log_fingerprint(commits: &[Commit]) -> u64 {
        let mut hasher = FxHasher::default();
        commits.len().hash(&mut hasher);
        for id in commits.iter().take(3).map(|c| c.id.as_ref()) {
            id.hash(&mut hasher);
        }
        for id in commits.iter().rev().take(3).map(|c| c.id.as_ref()) {
            id.hash(&mut hasher);
        }
        hasher.finish()
    }
}

#[cfg(test)]
fn is_probable_stash_tip(commit: &Commit) -> bool {
    crate::view::caches::history_commit_is_probable_stash_tip(commit)
}

fn stash_summary_from_log_summary(summary: &str) -> Option<&str> {
    let (_, tail) = summary.split_once(": ")?;
    let trimmed = tail.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn resolve_history_head_target<'a>(
    history_scope: LogScope,
    detached_head_commit: Option<&'a CommitId>,
    head_branch: Option<&'a str>,
    branches: &'a [Branch],
    visible_indices: &HistoryVisibleIndices,
    commits: &'a [Commit],
) -> Option<&'a str> {
    match head_branch {
        Some("HEAD") => detached_head_commit.map(AsRef::as_ref).or_else(|| {
            history_scope
                .guarantees_head_visibility()
                .then(|| {
                    visible_indices
                        .first()
                        .and_then(|ix| commits.get(ix))
                        .map(|commit| commit.id.as_ref())
                })
                .flatten()
        }),
        Some(head) => branches
            .iter()
            .find(|branch| branch.name == head)
            .map(|branch| branch.target.as_ref()),
        None => None,
    }
}

fn build_history_base_cache(
    request: HistoryBaseCacheRequest,
    page: &LogPage,
    theme: AppTheme,
    head_branch: Option<&str>,
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
    stashes: &[StashEntry],
) -> HistoryBaseCache {
    let stash_analysis = analyze_history_stashes(&page.commits, stashes);
    let stash_tips = stash_analysis.stash_tips;
    let stash_helper_ids = stash_analysis.stash_helper_ids;

    let visible_indices = build_history_visible_indices(&page.commits, &stash_helper_ids);
    let head_target = resolve_history_head_target(
        request.history_scope,
        request.detached_head_commit.as_ref(),
        head_branch,
        branches,
        &visible_indices,
        &page.commits,
    );

    let branch_heads = graph_branch_heads(request.history_scope, branches, remote_branches);
    let graph_rows: Arc<[history_graph::GraphRow]> = if stash_helper_ids.is_empty() {
        history_graph::compute_graph(&page.commits, theme, branch_heads, head_target).into()
    } else {
        let visible_commit_refs = visible_indices
            .iter()
            .map(|ix| &page.commits[ix])
            .collect::<Vec<_>>();
        history_graph::compute_graph_refs(&visible_commit_refs, theme, branch_heads, head_target)
            .into()
    };
    let max_lanes = graph_rows
        .iter()
        .map(|row| row.lanes_now.len().max(row.lanes_next.len()))
        .max()
        .unwrap_or(1);

    let has_stash_tips = !stash_tips.is_empty();
    let mut author_cache: HashMap<&str, HistoryTextVm> =
        HashMap::with_capacity_and_hasher(64, Default::default());
    let mut row_vms = Vec::with_capacity(visible_indices.len());
    if has_stash_tips {
        let mut next_stash_tip_ix = 0usize;
        for ix in visible_indices.iter() {
            let Some(commit) = page.commits.get(ix) else {
                continue;
            };
            let commit_id = commit.id.as_ref();
            let author = author_cache
                .entry(commit.author.as_ref())
                .or_insert_with(|| HistoryTextVm::new(commit.author.clone().into()))
                .clone();
            let (is_stash, summary) =
                match next_history_stash_tip_for_commit_ix(&stash_tips, &mut next_stash_tip_ix, ix)
                {
                    Some(stash_tip) => (
                        true,
                        stash_tip
                            .message
                            .map(|message| Arc::clone(message).into())
                            .or_else(|| {
                                stash_summary_from_log_summary(&commit.summary)
                                    .map(SharedString::new)
                            })
                            .unwrap_or_else(|| commit.summary.clone().into()),
                    ),
                    None => (false, commit.summary.clone().into()),
                };

            row_vms.push(HistoryBaseRowVm {
                author,
                summary: HistoryTextVm::new(summary),
                when: HistoryWhenVm::deferred(commit.time),
                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                is_head: head_target == Some(commit_id),
                is_stash,
            });
        }
    } else {
        for ix in visible_indices.iter() {
            let Some(commit) = page.commits.get(ix) else {
                continue;
            };
            let author = author_cache
                .entry(commit.author.as_ref())
                .or_insert_with(|| HistoryTextVm::new(commit.author.clone().into()))
                .clone();
            row_vms.push(HistoryBaseRowVm {
                author,
                summary: HistoryTextVm::new(commit.summary.clone().into()),
                when: HistoryWhenVm::deferred(commit.time),
                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                is_head: head_target == Some(commit.id.as_ref()),
                is_stash: false,
            });
        }
    }

    HistoryBaseCache {
        request,
        visible_indices,
        graph_rows,
        max_lanes,
        row_vms,
    }
}

fn build_history_decoration_cache(
    request: HistoryDecorationCacheRequest,
    page: &LogPage,
    base: &HistoryBaseCache,
    head_branch: Option<&str>,
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
    tags: &[Tag],
) -> HistoryDecorationCache {
    let head_target = resolve_history_head_target(
        request.base_request.history_scope,
        request.detached_head_commit.as_ref(),
        head_branch,
        branches,
        &base.visible_indices,
        &page.commits,
    );
    let (mut branch_text_by_target, head_branches_text) =
        build_history_branch_text_by_target(branches, remote_branches, head_branch, head_target);
    let mut tag_names_by_target = build_history_tag_names_by_target(tags);
    let mut row_vms = Vec::with_capacity(base.visible_indices.len());
    for (commit_ix, base_row) in base.visible_indices.iter().zip(base.row_vms.iter()) {
        let Some(commit) = page.commits.get(commit_ix) else {
            continue;
        };
        let commit_id = commit.id.as_ref();
        let branches_text = if base_row.is_head {
            head_branches_text.clone().unwrap_or_default()
        } else {
            branch_text_by_target
                .remove(commit_id)
                .unwrap_or_else(HistoryTextVm::default)
        };
        row_vms.push(HistoryDecorationRowVm {
            branches_text,
            tag_names: tag_names_by_target.remove(commit_id).unwrap_or_default(),
        });
    }

    HistoryDecorationCache {
        request,
        row_vms: row_vms.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{CommitId, LogCursor, LogPage, RepoSpec};
    use gitcomet_core::services::{GitBackend, GitRepository, Result};
    use gitcomet_state::model::AppState;
    use gitcomet_state::store::AppStore;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;
    use std::time::{Duration, Instant, SystemTime};

    struct BlockingBackend;

    impl GitBackend for BlockingBackend {
        fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
            loop {
                std::thread::park();
            }
        }
    }

    fn wait_until(
        cx: &mut gpui::VisualTestContext,
        description: &str,
        ready: impl Fn(&mut gpui::VisualTestContext) -> bool,
    ) {
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
            if ready(cx) {
                return;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for {description}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn set_history_view_state_for_tests(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<GitCometView>,
        state: Arc<AppState>,
    ) {
        cx.update(|window, app| {
            let history_view = view.read(app).main_pane.read(app).history_view.clone();
            history_view.update(app, |history, cx| {
                history.notify_fingerprint =
                    HistoryView::notify_fingerprint_for(&state, history.history_show_tags);
                history.state = Arc::clone(&state);
                cx.notify();
            });
            window.refresh();
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    fn ensure_history_cache_for_tests(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<GitCometView>,
        state: Arc<AppState>,
    ) {
        set_history_view_state_for_tests(cx, view, state);
        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            history_view.update(app, |history, cx| history.ensure_history_cache(cx));
            window.refresh();
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    fn commit(id: &str, parents: &[&str], summary: &str) -> Commit {
        Commit {
            id: CommitId(id.into()),
            parent_ids: parents.iter().map(|p| CommitId((*p).into())).collect(),
            summary: summary.into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }
    }

    fn all_columns_visible_drag_layout() -> HistoryColumnDragLayout {
        HistoryColumnDragLayout {
            show_graph: true,
            show_author: true,
            show_date: true,
            show_sha: true,
            branch_w: px(HISTORY_COL_BRANCH_PX),
            graph_w: px(HISTORY_COL_GRAPH_PX),
            author_w: px(HISTORY_COL_AUTHOR_PX),
            date_w: px(HISTORY_COL_DATE_PX),
            sha_w: px(HISTORY_COL_SHA_PX),
        }
    }

    fn branch(name: &str, target: &str) -> Branch {
        Branch {
            name: name.into(),
            target: CommitId(target.into()),
            upstream: None,
            divergence: None,
        }
    }

    fn remote_branch(remote: &str, name: &str, target: &str) -> RemoteBranch {
        RemoteBranch {
            remote: remote.into(),
            name: name.into(),
            target: CommitId(target.into()),
        }
    }

    fn log_page(commits: Vec<Commit>, next_cursor: Option<&str>) -> LogPage {
        LogPage {
            commits,
            next_cursor: next_cursor.map(|last_seen| LogCursor {
                last_seen: CommitId(last_seen.into()),
                resume_from: None,
            }),
        }
    }

    #[test]
    fn stash_tip_detection_requires_stash_like_message_and_multiple_parents() {
        assert!(is_probable_stash_tip(&commit(
            "s",
            &["p0", "p1"],
            "On main: quick stash"
        )));
        assert!(is_probable_stash_tip(&commit(
            "s",
            &["p0", "p1"],
            "WIP on main: quick stash"
        )));
        assert!(!is_probable_stash_tip(&commit(
            "c",
            &["p0"],
            "On main: normal commit"
        )));
        assert!(!is_probable_stash_tip(&commit(
            "c",
            &["p0", "p1"],
            "Regular summary"
        )));
    }

    #[test]
    fn stash_summary_parser_extracts_tail_after_prefix() {
        assert_eq!(
            stash_summary_from_log_summary("On feature/x: savepoint"),
            Some("savepoint")
        );
        assert_eq!(
            stash_summary_from_log_summary("WIP on main: keep this"),
            Some("keep this")
        );
        assert_eq!(stash_summary_from_log_summary("no delimiter"), None);
    }

    #[test]
    fn graph_branch_heads_are_hidden_for_current_branch_scope() {
        let branches = vec![branch("main", "local-head")];
        let remote_branches = vec![remote_branch("origin", "feature/x", "remote-head")];

        let mut current_branch_heads =
            graph_branch_heads(LogScope::CurrentBranch, &branches, &remote_branches);
        assert!(current_branch_heads.next().is_none());

        let all_branch_heads =
            graph_branch_heads(LogScope::AllBranches, &branches, &remote_branches)
                .collect::<Vec<_>>();
        assert_eq!(all_branch_heads.len(), 2);
        assert!(all_branch_heads.contains(&"local-head"));
        assert!(all_branch_heads.contains(&"remote-head"));
    }

    #[test]
    fn selected_branch_history_entry_text_formats_head_local_branch() {
        let selected_branch = SelectedBranch {
            repo_id: RepoId(7),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert_eq!(
            selected_branch_history_entry_text(Some(&selected_branch), RepoId(7), true, true),
            Some(SharedString::from("HEAD → main"))
        );
    }

    #[test]
    fn selected_branch_history_entry_text_formats_remote_branch_without_head_prefix() {
        let selected_branch = SelectedBranch {
            repo_id: RepoId(7),
            section: BranchSection::Remote,
            name: "origin/feature/topic".into(),
        };

        assert_eq!(
            selected_branch_history_entry_text(Some(&selected_branch), RepoId(7), true, true),
            Some(SharedString::from("origin/feature/topic"))
        );
    }

    #[test]
    fn selected_branch_history_entry_text_requires_selected_row_and_matching_repo() {
        let selected_branch = SelectedBranch {
            repo_id: RepoId(7),
            section: BranchSection::Local,
            name: "main".into(),
        };

        assert_eq!(
            selected_branch_history_entry_text(Some(&selected_branch), RepoId(8), true, true),
            None
        );
        assert_eq!(
            selected_branch_history_entry_text(Some(&selected_branch), RepoId(7), true, false),
            None
        );
    }

    #[test]
    fn history_columns_available_width_reserves_scrollbar_gutter() {
        let gutter = history_scrollbar_gutter();
        assert_eq!(
            history_columns_available_width(px(200.0)),
            px(200.0) - gutter
        );
        assert_eq!(history_columns_available_width(gutter), px(0.0));
    }

    #[test]
    fn history_column_drag_clamp_respects_static_maximums() {
        let available = history_columns_available_width(px(1436.0));
        let layout = all_columns_visible_drag_layout();
        let next = history_column_drag_clamped_width(
            HistoryColResizeHandle::Branch,
            px(900.0),
            available,
            layout,
            100,
        );
        assert_eq!(next, px(HISTORY_COL_BRANCH_MAX_PX));
    }

    #[test]
    fn history_column_drag_clamp_preserves_message_space() {
        let available = history_columns_available_width(px(836.0));
        let layout = all_columns_visible_drag_layout();
        let next = history_column_drag_clamped_width(
            HistoryColResizeHandle::Branch,
            px(500.0),
            available,
            layout,
            100,
        );

        let next_f: f32 = next.into();
        assert!((next_f - 132.0).abs() < 1e-3);
    }

    #[test]
    fn history_column_drag_clamp_never_goes_below_minimum() {
        let available = history_columns_available_width(px(1436.0));
        let layout = all_columns_visible_drag_layout();
        let next = history_column_drag_clamped_width(
            HistoryColResizeHandle::Sha,
            px(0.0),
            available,
            layout,
            100,
        );
        assert_eq!(next, px(HISTORY_COL_SHA_MIN_PX));
    }

    #[test]
    fn history_column_widths_recompute_from_design_units_with_ui_scale_percent() {
        let widths = scaled_history_column_widths(
            default_history_column_design_widths(),
            ui_scale::UiScale::from_percent(200),
        );
        assert_eq!(
            widths,
            HistoryColumnWidths {
                branch: px(HISTORY_COL_BRANCH_PX * 2.0),
                graph: px(HISTORY_COL_GRAPH_PX * 2.0),
                author: px(HISTORY_COL_AUTHOR_PX * 2.0),
                date: px(HISTORY_COL_DATE_PX * 2.0),
                sha: px(HISTORY_COL_SHA_PX * 2.0),
            }
        );
    }

    #[test]
    fn graph_drag_ignores_auto_hidden_optional_columns() {
        let available = history_columns_available_width(px(500.0));
        let widths = default_history_column_widths(100);
        let preferred = (true, true, true);

        assert_eq!(
            history_visible_columns_for_width(available, true, preferred, widths, 100),
            (false, false, false)
        );

        let next = history_column_drag_next_width(
            HistoryColResizeHandle::Graph,
            px(90.0),
            available,
            true,
            preferred,
            widths,
            100,
        );

        assert_eq!(next, px(90.0));
    }

    #[test]
    fn reset_widths_clamp_default_graph_in_narrow_windows() {
        let widths =
            history_reset_widths_for_available_width(px(396.0), true, (true, true, true), 100);

        assert_eq!(widths.branch, px(116.0));
        assert_eq!(widths.graph, px(HISTORY_COL_GRAPH_MIN_PX));
    }

    #[test]
    fn reset_widths_clamp_branch_after_graph_reaches_minimum() {
        let widths =
            history_reset_widths_for_available_width(px(360.0), true, (true, true, true), 100);

        assert_eq!(widths.graph, px(HISTORY_COL_GRAPH_MIN_PX));
        assert_eq!(widths.branch, px(80.0));
    }

    #[test]
    fn history_resize_state_uses_actual_visible_columns_in_narrow_windows() {
        let available = history_columns_available_width(px(500.0));
        let layout = all_columns_visible_drag_layout();
        let state = history_column_resize_state(
            HistoryColResizeHandle::Graph,
            px(0.0),
            available,
            layout,
            100,
        );

        assert_eq!(
            history_resize_state_visible_columns(available, Some(&state)),
            Some((false, false, false))
        );
    }

    #[test]
    fn history_resize_state_preserves_visible_columns_within_drag_bounds() {
        let available = history_columns_available_width(px(836.0));
        let layout = all_columns_visible_drag_layout();
        let state = history_column_resize_state(
            HistoryColResizeHandle::Graph,
            px(0.0),
            available,
            layout,
            100,
        );

        assert!(history_resize_state_preserves_visible_columns(
            available,
            layout,
            Some(&state)
        ));
        assert_eq!(
            history_visible_columns_for_layout_with_resize_state(
                available,
                layout,
                Some(&state),
                100,
            ),
            (true, true, true)
        );
    }

    #[test]
    fn history_resize_state_visibility_fast_path_falls_back_for_out_of_bounds_layout() {
        let available = history_columns_available_width(px(836.0));
        let state = history_column_resize_state(
            HistoryColResizeHandle::Graph,
            px(0.0),
            available,
            all_columns_visible_drag_layout(),
            100,
        );
        let layout = HistoryColumnDragLayout {
            graph_w: px(140.0),
            ..all_columns_visible_drag_layout()
        };

        assert!(!history_resize_state_preserves_visible_columns(
            available,
            layout,
            Some(&state)
        ));
        assert_eq!(
            history_visible_columns_for_layout_with_resize_state(
                available,
                layout,
                Some(&state),
                100,
            ),
            history_visible_columns_for_layout(available, layout, 100)
        );
    }

    #[test]
    fn history_resize_state_visible_columns_fast_path_rejects_stale_current_width() {
        let available = history_columns_available_width(px(836.0));
        let layout = all_columns_visible_drag_layout();
        let state = history_column_resize_state(
            HistoryColResizeHandle::Date,
            px(0.0),
            available,
            layout,
            100,
        );

        assert_eq!(
            history_resize_state_visible_columns_for_current_width(
                available,
                px(HISTORY_COL_DATE_PX),
                Some(&state),
            ),
            Some((true, true, true))
        );
        assert_eq!(
            history_resize_state_visible_columns_for_current_width(
                available,
                px(HISTORY_COL_DATE_PX + 1.0),
                Some(&state),
            ),
            None
        );
    }

    #[test]
    fn resolve_history_selected_list_index_populates_cache_for_commit_selection() {
        let commits = vec![
            commit("a", &["p0"], "a"),
            commit("b", &["a"], "b"),
            commit("c", &["b"], "c"),
        ];
        let selected = CommitId("c".into());
        let mut cache = None;

        let list_ix = resolve_history_selected_list_index(
            &mut cache,
            RepoId(7),
            11,
            13,
            LogScope::AllBranches,
            true,
            Some(&selected),
            &HistoryVisibleIndices::Filtered(vec![0, 2].into()),
            &commits,
        );

        assert_eq!(list_ix, Some(2));
        assert_eq!(
            cache,
            Some(HistorySelectedListIndexCache {
                repo_id: RepoId(7),
                log_rev: 11,
                stashes_rev: 13,
                history_scope: LogScope::AllBranches,
                show_working_tree_summary_row: true,
                selected_commit: Some(selected),
                list_ix: 2,
            })
        );
    }

    #[test]
    fn resolve_history_selected_list_index_reuses_matching_cache() {
        let selected = CommitId("cached".into());
        let mut cache = Some(HistorySelectedListIndexCache {
            repo_id: RepoId(3),
            log_rev: 21,
            stashes_rev: 34,
            history_scope: LogScope::CurrentBranch,
            show_working_tree_summary_row: false,
            selected_commit: Some(selected.clone()),
            list_ix: 5,
        });

        let list_ix = resolve_history_selected_list_index(
            &mut cache,
            RepoId(3),
            21,
            34,
            LogScope::CurrentBranch,
            false,
            Some(&selected),
            &HistoryVisibleIndices::all(0),
            &[],
        );

        assert_eq!(list_ix, Some(5));
    }

    #[test]
    fn pending_history_reveal_visible_target_scrolls_and_clears() {
        let commits = vec![
            commit("a", &["p0"], "a"),
            commit("b", &["a"], "b"),
            commit("c", &["b"], "c"),
        ];
        let pending = PendingHistoryReveal {
            repo_id: RepoId(7),
            commit_id: CommitId("c".into()),
            fallback_scope: Some(LogScope::AllBranches),
        };

        let decision = decide_pending_history_reveal(
            &pending,
            Some(RepoId(7)),
            Some(LogScope::CurrentBranch),
            None,
            11,
            13,
            false,
            Some(&log_page(commits, None)),
            Some(false),
            true,
            Some(&HistoryVisibleIndices::Filtered(vec![0, 2].into())),
            true,
            None,
        );

        assert_eq!(
            decision,
            PendingHistoryRevealDecision {
                set_scope: None,
                select_commit: true,
                scroll_to_list_ix: Some(2),
                load_more: false,
                clear_pending: true,
            }
        );
    }

    #[test]
    fn pending_history_reveal_missing_target_requests_load_more() {
        let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
        let pending = PendingHistoryReveal {
            repo_id: RepoId(7),
            commit_id: CommitId("c".into()),
            fallback_scope: Some(LogScope::AllBranches),
        };

        let decision = decide_pending_history_reveal(
            &pending,
            Some(RepoId(7)),
            Some(LogScope::CurrentBranch),
            None,
            11,
            13,
            false,
            Some(&log_page(commits, Some("b"))),
            Some(true),
            true,
            Some(&HistoryVisibleIndices::all(2)),
            false,
            None,
        );

        assert_eq!(
            decision,
            PendingHistoryRevealDecision {
                set_scope: None,
                select_commit: true,
                scroll_to_list_ix: None,
                load_more: true,
                clear_pending: false,
            }
        );
    }

    #[test]
    fn pending_history_reveal_switches_to_fallback_scope_after_exhausting_current_mode() {
        let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
        let pending = PendingHistoryReveal {
            repo_id: RepoId(7),
            commit_id: CommitId("c".into()),
            fallback_scope: Some(LogScope::AllBranches),
        };

        let decision = decide_pending_history_reveal(
            &pending,
            Some(RepoId(7)),
            Some(LogScope::CurrentBranch),
            None,
            11,
            13,
            false,
            Some(&log_page(commits, None)),
            Some(false),
            true,
            Some(&HistoryVisibleIndices::all(2)),
            false,
            None,
        );

        assert_eq!(
            decision,
            PendingHistoryRevealDecision {
                set_scope: Some(LogScope::AllBranches),
                select_commit: true,
                scroll_to_list_ix: None,
                load_more: false,
                clear_pending: false,
            }
        );
    }

    #[test]
    fn pending_history_reveal_missing_target_with_exhausted_history_and_no_fallback_clears() {
        let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
        let pending = PendingHistoryReveal {
            repo_id: RepoId(7),
            commit_id: CommitId("c".into()),
            fallback_scope: None,
        };

        let decision = decide_pending_history_reveal(
            &pending,
            Some(RepoId(7)),
            Some(LogScope::CurrentBranch),
            None,
            11,
            13,
            false,
            Some(&log_page(commits, None)),
            Some(false),
            true,
            Some(&HistoryVisibleIndices::all(2)),
            false,
            None,
        );

        assert_eq!(
            decision,
            PendingHistoryRevealDecision {
                set_scope: None,
                select_commit: true,
                scroll_to_list_ix: None,
                load_more: false,
                clear_pending: true,
            }
        );
    }

    #[test]
    fn pending_history_reveal_already_selected_commit_still_scrolls() {
        let commits = vec![commit("a", &["p0"], "a"), commit("b", &["a"], "b")];
        let selected = CommitId("b".into());
        let pending = PendingHistoryReveal {
            repo_id: RepoId(7),
            commit_id: selected.clone(),
            fallback_scope: None,
        };

        let decision = decide_pending_history_reveal(
            &pending,
            Some(RepoId(7)),
            Some(LogScope::CurrentBranch),
            Some(&selected),
            21,
            34,
            false,
            Some(&log_page(commits, None)),
            Some(false),
            true,
            Some(&HistoryVisibleIndices::all(2)),
            false,
            None,
        );

        assert_eq!(
            decision,
            PendingHistoryRevealDecision {
                set_scope: None,
                select_commit: false,
                scroll_to_list_ix: Some(1),
                load_more: false,
                clear_pending: true,
            }
        );
    }

    #[test]
    fn display_log_page_uses_retained_page_while_loading() {
        let mut repo = RepoState::new_opening(
            RepoId(9),
            RepoSpec {
                workdir: "/tmp/repo".into(),
            },
        );
        let page = Arc::new(log_page(vec![commit("a", &[], "a")], None));
        repo.log = Loadable::Loading;
        repo.history_state.log = Loadable::Loading;
        repo.history_state.retained_log_while_loading = Some(Arc::clone(&page));

        let display = HistoryView::display_log_page_for_repo(&repo)
            .expect("retained log should remain available while loading");
        assert!(Arc::ptr_eq(&display, &page));
    }

    #[gpui::test]
    fn date_time_changes_reuse_history_cache_and_rows_still_render(cx: &mut gpui::TestAppContext) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let page = Arc::new(log_page(vec![commit("tip", &[], "tip")], None));
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/history-date-time-reuse"),
            },
        );
        repo.history_state.history_scope = LogScope::AllBranches;
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.head_branch_rev = 1;
        repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
        repo.branches_rev = 1;
        repo.log = Loadable::Ready(Arc::clone(&page));
        repo.log_rev = 1;
        repo.history_state.log = Loadable::Ready(page);
        repo.history_state.log_rev = 1;

        let state = Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        ensure_history_cache_for_tests(cx, &view, state);

        wait_until(cx, "initial history cache for date-time reuse", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.row_vms.len() == 1
                        && cache.base.row_vms[0].summary.as_ref() == "tip"
                        && cache.decorations.row_vms.len() == 1
                })
            })
        });

        let (before_graph_rows, before_base_request, before_decoration_request, before_when_text) =
            cx.update(|window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let rows_len = history_view.update(app, |history, cx| {
                    HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
                });
                assert_eq!(rows_len, 1, "initial history row should render");

                let history = history_view.read(app);
                let cache = history
                    .history_cache
                    .as_ref()
                    .expect("history cache should be available");
                (
                    Arc::clone(&cache.base.graph_rows),
                    cache.base.request.clone(),
                    cache.decorations.request.clone(),
                    cache.base.row_vms[0]
                        .when
                        .resolve(HistoryDisplayKey::new(
                            DateTimeFormat::YmdHm,
                            Timezone::Utc,
                            true,
                        ))
                        .as_ref()
                        .to_owned(),
                )
            });

        assert_eq!(
            before_when_text,
            format_datetime(
                SystemTime::UNIX_EPOCH,
                DateTimeFormat::YmdHm,
                Timezone::Utc,
                true,
            )
        );

        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            history_view.update(app, |history, cx| {
                history.set_date_time_format(DateTimeFormat::MdyHm, cx);
                history.ensure_history_cache(cx);
                let rows = HistoryView::render_history_table_rows(history, 0..1, window, cx);
                assert_eq!(
                    rows.len(),
                    1,
                    "history row should still render after date change"
                );
            });
            window.refresh();
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let (after_graph_rows, after_base_request, after_decoration_request, after_when_text) = cx
            .update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                assert!(
                    history.history_cache_inflight.is_none(),
                    "display-only changes should not enqueue a cache rebuild"
                );
                let cache = history
                    .history_cache
                    .as_ref()
                    .expect("history cache should still be available");
                (
                    Arc::clone(&cache.base.graph_rows),
                    cache.base.request.clone(),
                    cache.decorations.request.clone(),
                    cache.base.row_vms[0]
                        .when
                        .resolve(HistoryDisplayKey::new(
                            DateTimeFormat::MdyHm,
                            Timezone::Utc,
                            true,
                        ))
                        .as_ref()
                        .to_owned(),
                )
            });

        assert!(
            Arc::ptr_eq(&before_graph_rows, &after_graph_rows),
            "date/time changes should keep the heavy graph cache"
        );
        assert_eq!(after_base_request, before_base_request);
        assert_eq!(after_decoration_request, before_decoration_request);
        assert_eq!(
            after_when_text,
            format_datetime(
                SystemTime::UNIX_EPOCH,
                DateTimeFormat::MdyHm,
                Timezone::Utc,
                true,
            )
        );
        assert_ne!(after_when_text, before_when_text);
    }

    #[gpui::test]
    fn current_branch_remote_branch_changes_reuse_base_cache_and_refresh_decorations(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let page = Arc::new(log_page(vec![commit("tip", &[], "tip")], None));
        let repo_path = PathBuf::from("/tmp/history-current-branch-remote-reuse");

        let mut initial_repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
        initial_repo.history_state.history_scope = LogScope::CurrentBranch;
        initial_repo.head_branch = Loadable::Ready("main".to_string());
        initial_repo.head_branch_rev = 1;
        initial_repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "tip")]));
        initial_repo.branches_rev = 1;
        initial_repo.remote_branches =
            Loadable::Ready(Arc::new(vec![remote_branch("origin", "main", "tip")]));
        initial_repo.remote_branches_rev = 1;
        initial_repo.log = Loadable::Ready(Arc::clone(&page));
        initial_repo.log_rev = 1;
        initial_repo.history_state.log = Loadable::Ready(Arc::clone(&page));
        initial_repo.history_state.log_rev = 1;

        let initial_state = Arc::new(AppState {
            repos: vec![initial_repo.clone()],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        let mut updated_repo = initial_repo;
        updated_repo.remote_branches = Loadable::Ready(Arc::new(vec![
            remote_branch("origin", "main", "tip"),
            remote_branch("upstream", "main", "tip"),
        ]));
        updated_repo.remote_branches_rev = 2;

        let updated_state = Arc::new(AppState {
            repos: vec![updated_repo],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        ensure_history_cache_for_tests(cx, &view, initial_state);

        wait_until(cx, "initial current-branch history cache", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request.history_scope == LogScope::CurrentBranch
                        && cache.base.request.remote_branches_rev == 0
                        && cache.decorations.row_vms.len() == 1
                        && cache.decorations.row_vms[0]
                            .branches_text
                            .as_ref()
                            .contains("origin/main")
                })
            })
        });

        let (before_graph_rows, before_base_request, before_branches_text) =
            cx.update(|window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let rows_len = history_view.update(app, |history, cx| {
                    HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
                });
                assert_eq!(rows_len, 1, "initial current-branch row should render");

                let history = history_view.read(app);
                let cache = history
                    .history_cache
                    .as_ref()
                    .expect("history cache should be available");
                (
                    Arc::clone(&cache.base.graph_rows),
                    cache.base.request.clone(),
                    cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .to_owned(),
                )
            });

        assert!(before_branches_text.contains("origin/main"));
        assert!(!before_branches_text.contains("upstream/main"));

        ensure_history_cache_for_tests(cx, &view, updated_state);

        wait_until(cx, "updated current-branch decorations", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request.history_scope == LogScope::CurrentBranch
                        && cache.base.request.remote_branches_rev == 0
                        && cache.decorations.request.remote_branches_rev == 2
                        && cache.decorations.row_vms.len() == 1
                        && cache.decorations.row_vms[0]
                            .branches_text
                            .as_ref()
                            .contains("upstream/main")
                })
            })
        });

        let (after_graph_rows, after_base_request, after_branches_text) =
            cx.update(|window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let rows_len = history_view.update(app, |history, cx| {
                    HistoryView::render_history_table_rows(history, 0..1, window, cx).len()
                });
                assert_eq!(
                    rows_len, 1,
                    "updated current-branch row should still render"
                );

                let history = history_view.read(app);
                let cache = history
                    .history_cache
                    .as_ref()
                    .expect("history cache should be available");
                (
                    Arc::clone(&cache.base.graph_rows),
                    cache.base.request.clone(),
                    cache.decorations.row_vms[0]
                        .branches_text
                        .as_ref()
                        .to_owned(),
                )
            });

        assert!(
            Arc::ptr_eq(&before_graph_rows, &after_graph_rows),
            "remote branch changes in current-branch mode should reuse the heavy base cache"
        );
        assert_eq!(after_base_request, before_base_request);
        assert!(after_branches_text.contains("origin/main"));
        assert!(after_branches_text.contains("upstream/main"));
    }

    #[gpui::test]
    fn history_scope_switch_keeps_rows_visible_and_refreshes_automatically(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let initial_scope = LogScope::FullReachable;
        let switched_scope = LogScope::AllBranches;
        let repo_path = PathBuf::from("/tmp/history-scope-switch-test");
        let initial_page = Arc::new(log_page(vec![commit("main-tip", &[], "main tip")], None));
        let switched_page = Arc::new(log_page(
            vec![
                commit("all-tip", &[], "all branches tip"),
                commit("main-tip", &[], "main tip"),
            ],
            None,
        ));

        let mut initial_repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
        initial_repo.history_state.history_scope = initial_scope;
        initial_repo.log = Loadable::Ready(Arc::clone(&initial_page));
        initial_repo.log_rev = 1;
        initial_repo.history_state.log = Loadable::Ready(Arc::clone(&initial_page));
        initial_repo.history_state.log_rev = 1;

        let initial_state = Arc::new(AppState {
            repos: vec![initial_repo.clone()],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        let mut loading_repo = initial_repo.clone();
        loading_repo.history_state.history_scope = switched_scope;
        loading_repo.log = Loadable::Loading;
        loading_repo.log_rev = 2;
        loading_repo.history_state.log = Loadable::Loading;
        loading_repo.history_state.log_rev = 2;
        loading_repo.history_state.retained_log_while_loading = Some(Arc::clone(&initial_page));

        let loading_state = Arc::new(AppState {
            repos: vec![loading_repo.clone()],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        let mut loaded_repo = loading_repo;
        loaded_repo.log = Loadable::Ready(Arc::clone(&switched_page));
        loaded_repo.log_rev = 3;
        loaded_repo.history_state.log = Loadable::Ready(Arc::clone(&switched_page));
        loaded_repo.history_state.log_rev = 3;
        loaded_repo.history_state.retained_log_while_loading = None;

        let loaded_state = Arc::new(AppState {
            repos: vec![loaded_repo],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        ensure_history_cache_for_tests(cx, &view, Arc::clone(&initial_state));

        wait_until(cx, "initial history rows", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request.history_scope == initial_scope
                        && cache.base.visible_indices.len() == 1
                        && cache.base.row_vms.len() == 1
                        && cache.base.row_vms[0].summary.as_ref() == "main tip"
                })
            })
        });

        ensure_history_cache_for_tests(cx, &view, Arc::clone(&loading_state));

        wait_until(cx, "retained history rows during loading", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.active_repo().is_some_and(|repo| {
                    repo.history_state.history_scope == switched_scope
                        && matches!(repo.log, Loadable::Loading)
                        && repo
                            .history_state
                            .retained_log_while_loading
                            .as_ref()
                            .is_some_and(|page| Arc::ptr_eq(page, &initial_page))
                }) && history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.visible_indices.len() == 1
                        && cache.base.row_vms.len() == 1
                        && cache.base.row_vms[0].summary.as_ref() == "main tip"
                })
            })
        });

        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            history_view.update(app, |history, cx| {
                let rows = HistoryView::render_history_table_rows(history, 0..1, window, cx);
                assert_eq!(rows.len(), 1, "retained history row should still render");
            });
        });

        ensure_history_cache_for_tests(cx, &view, Arc::clone(&loaded_state));

        wait_until(cx, "history rows refresh after scope load", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request.history_scope == switched_scope
                        && cache.base.visible_indices.len() == 2
                        && cache.base.row_vms.len() == 2
                        && cache.base.row_vms[0].summary.as_ref() == "all branches tip"
                        && cache.base.row_vms[1].summary.as_ref() == "main tip"
                })
            })
        });
    }

    #[gpui::test]
    fn filtered_modes_do_not_infer_detached_head_target_from_first_visible_row(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        for (scope, commits, expected_summary) in [
            (
                LogScope::NoMerges,
                vec![commit("visible", &["hidden"], "visible non-merge")],
                "visible non-merge",
            ),
            (
                LogScope::MergesOnly,
                vec![commit("visible-merge", &["p0", "p1"], "visible merge")],
                "visible merge",
            ),
        ] {
            let page = Arc::new(log_page(commits, None));
            let mut repo = RepoState::new_opening(
                RepoId(1),
                RepoSpec {
                    workdir: PathBuf::from("/tmp/history-detached-head-filtered"),
                },
            );
            repo.history_state.history_scope = scope;
            repo.head_branch = Loadable::Ready("HEAD".to_string());
            repo.head_branch_rev = 1;
            repo.log = Loadable::Ready(Arc::clone(&page));
            repo.log_rev = 1;
            repo.history_state.log = Loadable::Ready(page);
            repo.history_state.log_rev = 1;

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                ..Default::default()
            });

            ensure_history_cache_for_tests(cx, &view, state);

            let description = format!("filtered {scope:?} history cache");
            wait_until(cx, &description, |cx| {
                cx.update(|_window, app| {
                    let main_pane = view.read(app).main_pane.clone();
                    let history_view = main_pane.read(app).history_view.clone();
                    let history = history_view.read(app);
                    history.history_cache.as_ref().is_some_and(|cache| {
                        cache.base.request.history_scope == scope
                            && cache.base.row_vms.len() == 1
                            && !cache.base.row_vms[0].is_head
                            && cache.base.row_vms[0].summary.as_ref() == expected_summary
                    })
                })
            });
        }
    }

    #[gpui::test]
    fn retained_history_rows_support_keyboard_navigation_while_loading(
        cx: &mut gpui::TestAppContext,
    ) {
        let _visual_guard = crate::test_support::lock_visual_test();
        let (store, events) = AppStore::new(Arc::new(BlockingBackend));
        let store_for_assert = store.clone();
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

        let repo_id = RepoId(1);
        let first = CommitId("tip".into());
        let second = CommitId("base".into());
        let repo_path = PathBuf::from(format!(
            "/tmp/history-retained-nav-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));

        cx.update(|window, app| {
            let _ = window.draw(app);
        });

        store_for_assert.dispatch(Msg::OpenRepo(repo_path.clone()));
        wait_until(cx, "opened repo placeholder", |_cx| {
            let snapshot = store_for_assert.snapshot();
            snapshot.active_repo == Some(repo_id)
                && snapshot.repos.iter().any(|repo| repo.id == repo_id)
        });

        let page = Arc::new(log_page(
            vec![commit("tip", &["base"], "tip"), commit("base", &[], "base")],
            None,
        ));
        let mut repo = RepoState::new_opening(repo_id, RepoSpec { workdir: repo_path });
        repo.history_state.history_scope = LogScope::AllBranches;
        repo.history_state.selected_commit = Some(first.clone());
        repo.history_state.retained_log_while_loading = Some(Arc::clone(&page));
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.head_branch_rev = 1;
        repo.log = Loadable::Loading;
        repo.log_rev = 1;
        repo.history_state.log = Loadable::Loading;
        repo.history_state.log_rev = 1;

        let state = Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..Default::default()
        });

        ensure_history_cache_for_tests(cx, &view, state);

        wait_until(cx, "retained rows available during loading", |cx| {
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                let history_view = main_pane.read(app).history_view.clone();
                let history = history_view.read(app);
                history.active_repo().is_some_and(|repo| {
                    repo.history_state.history_scope == LogScope::AllBranches
                        && matches!(repo.log, Loadable::Loading)
                        && repo.history_state.retained_log_while_loading.is_some()
                        && repo.history_state.selected_commit.as_ref() == Some(&first)
                }) && history.history_cache.as_ref().is_some_and(|cache| {
                    cache.base.request.history_scope == LogScope::AllBranches
                        && cache.base.row_vms.len() == 2
                        && cache.base.row_vms[0].summary.as_ref() == "tip"
                        && cache.base.row_vms[1].summary.as_ref() == "base"
                })
            })
        });

        cx.update(|window, app| {
            let main_pane = view.read(app).main_pane.clone();
            let history_view = main_pane.read(app).history_view.clone();
            history_view.update(app, |history, cx| {
                assert!(history.history_select_adjacent_commit(1, cx));
            });
            window.refresh();
            let _ = window.draw(app);
        });

        wait_until(cx, "selected second retained commit", |_cx| {
            let snapshot = store_for_assert.snapshot();
            let Some(repo) = snapshot.repos.iter().find(|repo| repo.id == repo_id) else {
                return false;
            };
            repo.history_state.selected_commit.as_ref() == Some(&second)
        });
    }
}
