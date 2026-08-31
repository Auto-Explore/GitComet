use super::super::*;
use crate::view::caches::{
    HistoryListPlan, HistoryListPlanCache, HistoryShortShaVm, HistoryVisibleIndices, HistoryWhenVm,
    HistoryWorktreeRowAnchor, analyze_history_stashes, build_history_branch_containment_bits,
    build_history_branch_ref_items_by_target, build_history_branch_text_by_target,
    build_history_tag_names_by_target, build_history_visible_indices,
    history_ref_items_from_displayed_refs, next_history_stash_tip_for_commit_ix,
    related_commit_contains,
};
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

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
    /// Identity of the row interleaving the cached `list_ix` was computed
    /// against; a worktree row appearing or moving shifts every index below it.
    plan_fingerprint: u64,
    selected_commit: Option<CommitId>,
    list_ix: usize,
}

/// Memo for [`HistoryView::history_selected_lane_color_ix`].
/// What the lane highlight is anchored to.
#[derive(Clone, Debug, Eq, PartialEq)]
enum HistoryLaneAnchor {
    /// A commit, highlighting the lane it is drawn on.
    Commit(CommitId),
    /// A linked worktree. Its HEAD locates the row, but the lane is the
    /// *branch's* — which for a branch that has fallen behind is the fork lane
    /// beside that commit rather than the commit's own.
    Worktree { head: CommitId, on_branch: bool },
}

/// Keyed on the base cache's whole request rather than its `log_fingerprint`:
/// the answer is read out of `graph_rows`, which is recomputed for every field
/// of that request. Creating, deleting or checking out a branch changes which
/// rows `force_branch_head_lane` fires on and so which colour index each lane
/// draws, all without touching the fingerprint — a fingerprint-only key would
/// keep saturating the lane the selection used to be on.
#[derive(Clone, Debug, Eq, PartialEq)]
struct HistorySelectedLaneColorCache {
    base_request: HistoryBaseCacheRequest,
    anchor: HistoryLaneAnchor,
    /// `None` when the anchor is not on screen — then no lane is highlighted.
    lane: Option<crate::view::rows::history_graph_paint::SelectedLane>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PendingHistoryReveal {
    repo_id: RepoId,
    commit_id: CommitId,
    fallback_scope: Option<LogScope>,
    /// Set when the reveal is aimed at a linked worktree's row rather than the
    /// commit itself. The commit is still what gets located — the row sits
    /// directly above it — but the selection and the scroll land on the row.
    worktree_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PendingHistoryRevealDecision {
    set_scope: Option<LogScope>,
    select_commit: Option<CommitId>,
    scroll_to_list_ix: Option<usize>,
    load_more: bool,
    clear_pending: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum HistoryCommitReferenceMatch {
    Unique { list_ix: usize, commit_id: CommitId },
    Ambiguous,
    Missing,
}

fn commit_id_matches_reference(commit_id: &CommitId, reference: &CommitId) -> bool {
    let commit_id = commit_id.as_ref();
    let reference = reference.as_ref();
    commit_id.eq_ignore_ascii_case(reference)
        || (reference.len() >= 7
            && reference.len() < commit_id.len()
            && commit_id
                .get(..reference.len())
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case(reference)))
}

fn history_selected_list_index_cache_matches(
    cache: &HistorySelectedListIndexCache,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    plan: &HistoryListPlan,
    selected_commit: Option<&CommitId>,
) -> bool {
    cache.repo_id == repo_id
        && cache.log_rev == log_rev
        && cache.stashes_rev == stashes_rev
        && cache.history_scope == history_scope
        && cache.show_working_tree_summary_row == plan.show_working_tree_summary_row()
        && cache.plan_fingerprint == plan.fingerprint()
        && cache.selected_commit.as_ref() == selected_commit
}

fn set_history_selected_list_index_cache(
    cache: &mut Option<HistorySelectedListIndexCache>,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    plan: &HistoryListPlan,
    selected_commit: Option<CommitId>,
    list_ix: usize,
) {
    *cache = Some(HistorySelectedListIndexCache {
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        show_working_tree_summary_row: plan.show_working_tree_summary_row(),
        plan_fingerprint: plan.fingerprint(),
        selected_commit,
        list_ix,
    });
}

/// What the history selection currently rests on, for the list-index
/// bookkeeping. The three states are mutually exclusive: a commit, a worktree
/// row, or -- when neither -- the working-tree row.
#[derive(Clone, Copy)]
struct HistorySelectionRef<'a> {
    commit: Option<&'a CommitId>,
    worktree_selected: bool,
}

/// The row that should read as selected in the active workspace's history.
///
/// An explicit selection wins while the user remains in the tab. With no
/// explicit selection, the live workspace is the focus: its uncommitted row
/// when one exists, otherwise the commit at HEAD.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::view) enum HistoryPrimarySelection {
    WorkingTree,
    Commit(CommitId),
    Worktree(PathBuf),
}

pub(in crate::view) fn history_primary_selection(
    repo: &RepoState,
    show_working_tree_summary_row: bool,
) -> Option<HistoryPrimarySelection> {
    if let Some(path) = &repo.history_state.worktree_selection {
        return Some(HistoryPrimarySelection::Worktree(path.clone()));
    }
    if let Some(commit_id) = &repo.history_state.selected_commit {
        return Some(HistoryPrimarySelection::Commit(commit_id.clone()));
    }
    if show_working_tree_summary_row {
        return Some(HistoryPrimarySelection::WorkingTree);
    }
    repo.head_commit_id().map(HistoryPrimarySelection::Commit)
}

fn peek_history_selected_list_index(
    cache: Option<&HistorySelectedListIndexCache>,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    plan: &HistoryListPlan,
    selection: HistorySelectionRef<'_>,
    visible_indices: &HistoryVisibleIndices,
    commits: &[Commit],
) -> Option<usize> {
    // A selected worktree row also leaves the commit selection empty, but it is
    // not the working-tree row -- claiming index 0 here would leave two rows
    // looking selected and send the scroll bookkeeping to the wrong one. The
    // worktree row's own index comes from `worktree_row_list_ix`.
    if selection.worktree_selected {
        return None;
    }
    let selected_commit = selection.commit;
    if plan.show_working_tree_summary_row() && selected_commit.is_none() {
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
                plan,
                selected_commit,
            )
        })
        .map(|entry| entry.list_ix)
    {
        return Some(list_ix);
    }

    let selected_commit = selected_commit?;
    match visible_commit_match_for_reference(selected_commit, visible_indices, commits, plan) {
        HistoryCommitReferenceMatch::Unique { list_ix, .. } => Some(list_ix),
        HistoryCommitReferenceMatch::Ambiguous | HistoryCommitReferenceMatch::Missing => None,
    }
}

fn visible_commit_match_for_reference(
    reference: &CommitId,
    visible_indices: &HistoryVisibleIndices,
    commits: &[Commit],
    plan: &HistoryListPlan,
) -> HistoryCommitReferenceMatch {
    let mut found = None;

    for (visible_ix, commit_ix) in visible_indices.iter().enumerate() {
        let Some(commit) = commits.get(commit_ix) else {
            continue;
        };
        if !commit_id_matches_reference(&commit.id, reference) {
            continue;
        }

        let next = (plan.list_ix_for_visible(visible_ix), commit.id.clone());
        if found.is_some() {
            return HistoryCommitReferenceMatch::Ambiguous;
        }
        found = Some(next);
    }

    if let Some((list_ix, commit_id)) = found {
        HistoryCommitReferenceMatch::Unique { list_ix, commit_id }
    } else {
        HistoryCommitReferenceMatch::Missing
    }
}

/// What clicking a worktree in the sidebar should focus in the log.
#[derive(Clone, Debug, Eq, PartialEq)]
enum WorktreeRevealTarget {
    /// The pinned row at the top -- only this tab's own changes live there.
    WorkingTreeSummaryRow,
    /// A linked worktree's own uncommitted-changes row.
    WorktreeRow {
        head: CommitId,
        fallback_scope: Option<LogScope>,
    },
    Commit {
        head: CommitId,
        fallback_scope: Option<LogScope>,
    },
    /// A clean worktree whose HEAD we could not resolve; nothing to aim at.
    Nothing,
}

/// One rule for every worktree row: its changes if it has any, otherwise the
/// commit it sits on. Where "its changes" live differs -- this tab's are pinned
/// at the top of the log, every other worktree's are a row of their own.
///
/// `worktree_is_dirty` is `None` while the scan has not answered for this
/// worktree yet, which is not the same as answering that it is clean: aiming at
/// the commit on an unknown commits to a row set that is about to grow, and the
/// first scan reply then shifts the log under the user. Aiming at the row
/// instead costs nothing when the guess is wrong -- the reveal keeps the commit
/// as its scroll target until the row exists, and a worktree that turns out
/// clean has its selection dropped by the reducer.
fn worktree_reveal_target(
    is_current: bool,
    current_has_changes: bool,
    worktree_is_dirty: Option<bool>,
    head: Option<CommitId>,
) -> WorktreeRevealTarget {
    if is_current && current_has_changes {
        return WorktreeRevealTarget::WorkingTreeSummaryRow;
    }
    let Some(head) = head else {
        return WorktreeRevealTarget::Nothing;
    };
    // A linked worktree's branch need not be in the current scope -- the same
    // reason a non-HEAD branch row falls back to all branches. It applies to the
    // row as much as to the commit: the row is anchored to the same commit, and
    // without the fallback a dirty worktree on an out-of-scope branch had
    // nothing to scroll to.
    let fallback_scope = (!is_current).then_some(LogScope::AllBranches);
    if !is_current && worktree_is_dirty != Some(false) {
        return WorktreeRevealTarget::WorktreeRow {
            head,
            fallback_scope,
        };
    }
    WorktreeRevealTarget::Commit {
        head,
        fallback_scope,
    }
}

/// Where the worktree row for `path` currently sits, if anywhere.
fn worktree_row_list_ix(
    plan: &HistoryListPlan,
    repo: Option<&RepoState>,
    path: &std::path::Path,
) -> Option<usize> {
    let Loadable::Ready(dirty) = &repo?.worktree_dirty else {
        return None;
    };
    let worktree_ix = dirty.iter().position(|summary| summary.path == path)?;
    plan.list_ix_for_worktree(worktree_ix)
}

fn resolve_history_selected_list_index(
    cache: &mut Option<HistorySelectedListIndexCache>,
    repo_id: RepoId,
    log_rev: u64,
    stashes_rev: u64,
    history_scope: LogScope,
    plan: &HistoryListPlan,
    selection: HistorySelectionRef<'_>,
    visible_indices: &HistoryVisibleIndices,
    commits: &[Commit],
) -> Option<usize> {
    let list_ix = peek_history_selected_list_index(
        cache.as_ref(),
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        plan,
        selection,
        visible_indices,
        commits,
    )?;
    set_history_selected_list_index_cache(
        cache,
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        plan,
        selection.commit.cloned(),
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
    _log_rev: u64,
    _stashes_rev: u64,
    log_loading_more: bool,
    display_page: Option<&LogPage>,
    live_page_has_more: Option<bool>,
    cache_request_matches: bool,
    visible_indices: Option<&HistoryVisibleIndices>,
    plan: &HistoryListPlan,
    _selected_list_index_cache: Option<&HistorySelectedListIndexCache>,
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

    // Selecting a target that is *not* loaded yet is `Msg::RevealCommit`'s job:
    // it resolves the reference against the object database and shows the commit
    // straight away, without this deciding anything about a row it cannot see.
    //
    // A full id already sitting in the loaded page is the exception. Selecting it
    // needs no round-trip, and cannot flicker either: page reconciliation only
    // clears a selection the page does not contain.
    let Some(display_page) = display_page else {
        return decision;
    };
    if selected_commit != Some(&pending.commit_id)
        && display_page
            .commits
            .iter()
            .any(|commit| commit.id == pending.commit_id)
    {
        decision.select_commit = Some(pending.commit_id.clone());
    }

    if !cache_request_matches {
        return decision;
    }
    let Some(visible_indices) = visible_indices else {
        return decision;
    };

    match visible_commit_match_for_reference(
        &pending.commit_id,
        visible_indices,
        &display_page.commits,
        plan,
    ) {
        HistoryCommitReferenceMatch::Unique { list_ix, commit_id } => {
            // The row carries the full id; an abbreviated reference upgrades to
            // it here even if the resolve reply has not landed yet.
            if selected_commit != Some(&commit_id) {
                decision.select_commit = Some(commit_id);
            }
            decision.scroll_to_list_ix = Some(list_ix);
            decision.clear_pending = true;
            return decision;
        }
        HistoryCommitReferenceMatch::Ambiguous => {
            decision.select_commit = None;
            decision.clear_pending = true;
            return decision;
        }
        HistoryCommitReferenceMatch::Missing => {}
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
    pub(in super::super) history_relative_dates: bool,
    pub(in super::super) history_highlight_commit_chain: bool,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
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
    /// Last browse-point commit we scrolled to, so a new one is revealed only when
    /// the historical browse point actually changes.
    last_browse_commit: Option<CommitId>,
    pub(in super::super) history_worktree_summary_cache: Option<HistoryWorktreeSummaryCache>,
    history_list_plan_cache: Option<HistoryListPlanCache>,
    history_selected_lane_color_cache: Option<HistorySelectedLaneColorCache>,
    pub(in super::super) history_stash_ids_cache: Option<HistoryStashIdsCache>,
    pub(in super::super) history_scroll: UniformListScrollHandle,
    pub(in super::super) history_panel_focus_handle: FocusHandle,
    /// Minute tick that re-renders the table while the relative date format is
    /// active, so "2 mins ago" labels don't freeze. `None` for absolute formats.
    relative_time_tick: Option<gpui::Task<()>>,
}

impl HistoryView {
    fn notify_fingerprint_for(state: &AppState, show_history_tags: bool) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.log_rev.hash(&mut hasher);
            repo.history_state.log_rev.hash(&mut hasher);
            repo.history_state.history_scope.hash(&mut hasher);
            // A running walk reports progress without changing the log, and the
            // header prints that count — so it has to repaint on its own.
            repo.history_state.log_scan_progress.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.detached_head_commit.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.remote_branches_rev.hash(&mut hasher);
            if show_history_tags {
                repo.tags_rev.hash(&mut hasher);
            }
            repo.stashes_rev.hash(&mut hasher);
            repo.history_state.selected_commit_rev.hash(&mut hasher);
            repo.file_browser.file_browser_rev.hash(&mut hasher);
            // The linked-worktree rows live in this table: their badge counts come
            // from the dirty scan and the selected row from the worktree selection,
            // so both revs have to move the fingerprint or the rows never repaint.
            repo.worktree_dirty_rev.hash(&mut hasher);
            repo.history_state.worktree_selection_rev.hash(&mut hasher);
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
        history_relative_dates: bool,
        history_highlight_commit_chain: bool,
        history_show_graph: bool,
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        history_show_tags: bool,
        history_auto_fetch_tags_on_repo_activation: bool,
        root_view: WeakEntity<GitCometView>,
        last_window_size: Size<Pixels>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state, history_show_tags);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next, this.history_show_tags);
            let changed = next_fingerprint != this.notify_fingerprint;
            let switched_repo = this.state.active_repo != next.active_repo;
            this.state = next;

            if switched_repo {
                // These memos describe the selection in the tab we just left.
                // Their keys also include repository identity, but dropping
                // them here makes the transition atomic even when sibling
                // workspaces expose an identical commit page.
                this.history_selected_list_index_cache = None;
                this.history_selected_lane_color_cache = None;
            }

            // When the historical browse point changes, scroll the history to that
            // commit (its row is highlighted purple by the canvas).
            let browse_commit = this
                .active_repo()
                .and_then(|repo| repo.browsing_commit().cloned());
            if browse_commit != this.last_browse_commit {
                this.last_browse_commit = browse_commit.clone();
                if let (Some(repo_id), Some(commit_id)) = (this.active_repo_id(), browse_commit) {
                    this.request_reveal_commit(repo_id, commit_id, Some(LogScope::AllBranches), cx);
                }
            }

            if changed {
                this.notify_fingerprint = next_fingerprint;
                this.dismiss_history_refs_hover(cx);
                cx.notify();
            }
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
            history_relative_dates,
            history_highlight_commit_chain,
            _ui_model_subscription: subscription,
            root_view,
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
            last_browse_commit: None,
            history_worktree_summary_cache: None,
            history_list_plan_cache: None,
            history_selected_lane_color_cache: None,
            history_stash_ids_cache: None,
            history_scroll: UniformListScrollHandle::default(),
            history_panel_focus_handle,
            relative_time_tick: None,
        }
    }

    /// Keeps a minute-interval re-render task alive while relative history
    /// dates are enabled; drops it (cancelling the task) otherwise.
    pub(in super::super) fn ensure_relative_time_tick(&mut self, cx: &mut gpui::Context<Self>) {
        if !self.history_relative_dates {
            self.relative_time_tick = None;
            return;
        }
        if self.relative_time_tick.is_some() {
            return;
        }
        // The test scheduler would treat a sleeping loop as forever-pending work.
        if !crate::ui_runtime::current().uses_live_store_poller() {
            return;
        }
        self.relative_time_tick = Some(cx.spawn(
            async move |view: WeakEntity<HistoryView>, cx: &mut gpui::AsyncApp| {
                loop {
                    smol::Timer::after(std::time::Duration::from_secs(60)).await;
                    if view.update(cx, |_, cx| cx.notify()).is_err() {
                        break;
                    }
                }
            },
        ));
    }

    pub(in super::super) fn active_repo_id(&self) -> Option<RepoId> {
        self.state.active_repo
    }

    pub(in super::super) fn active_repo(&self) -> Option<&RepoState> {
        let repo_id = self.active_repo_id()?;
        self.state.repos.iter().find(|r| r.id == repo_id)
    }

    /// Visible commit ids in log order for shift-click range selection.
    /// Hidden rows (stash helper commits) are excluded, matching what the
    /// user sees.
    pub(in super::super) fn visible_commit_ids_for_repo(
        &self,
        repo_id: RepoId,
    ) -> Option<Vec<CommitId>> {
        let repo = self.state.repos.iter().find(|r| r.id == repo_id)?;
        let page = Self::display_log_page_for_repo(repo)?;
        let cache = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo_id)?;
        Some(
            cache
                .base
                .visible_indices
                .iter()
                .filter_map(|ix| page.commits.get(ix).map(|c| c.id.clone()))
                .collect(),
        )
    }

    pub(in crate::view) fn show_commit_message_hover(
        &mut self,
        next: crate::view::commit_message_hover::CommitMessageHoverState,
        pointer: Point<Pixels>,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.show_commit_message_hover(next, pointer, cx)
        });
    }

    pub(in crate::view) fn show_history_refs_hover(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        source_bounds: Bounds<Pixels>,
        items: Arc<[HistoryRefListItem]>,
        pointer: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.show_history_refs_hover(
                repo_id,
                commit_id,
                source_bounds,
                items,
                pointer,
                window,
                cx,
            );
        });
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

    fn attached_head_target_for_repo(repo: &RepoState) -> Option<CommitId> {
        let Loadable::Ready(head_branch) = &repo.head_branch else {
            return None;
        };
        if head_branch == "HEAD" {
            return None;
        }
        let Loadable::Ready(branches) = &repo.branches else {
            return None;
        };
        branches
            .iter()
            .find(|branch| branch.name == *head_branch)
            .map(|branch| branch.target.clone())
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
            head_branch_target: Self::attached_head_target_for_repo(repo),
            branches_rev: if repo.history_state.history_scope.is_current_branch_mode() {
                0
            } else {
                repo.branches_rev
            },
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
        self.request_reveal_commit_inner(repo_id, commit_id, fallback_scope, None, cx);
    }

    /// Focus whatever best represents a worktree in the log.
    ///
    /// The rule is the same for every worktree row in the sidebar, including the
    /// one this tab is checked out on: land on its uncommitted-changes row when
    /// it has changes, and on the commit its HEAD points at when it does not.
    /// Only the *current* worktree's changes live in the pinned row at the top;
    /// every other worktree's live in a row of their own.
    pub(in crate::view) fn reveal_worktree(
        &mut self,
        repo_id: RepoId,
        path: PathBuf,
        is_current: bool,
        head: Option<CommitId>,
        cx: &mut gpui::Context<Self>,
    ) {
        let current_has_changes = self.ensure_history_worktree_summary_cache().0;
        // `None` while the scan has not answered -- see `worktree_reveal_target`.
        let worktree_is_dirty = self
            .active_repo()
            .and_then(|repo| match &repo.worktree_dirty {
                Loadable::Ready(dirty) => Some(dirty.iter().any(|summary| summary.path == path)),
                _ => None,
            });

        match worktree_reveal_target(is_current, current_has_changes, worktree_is_dirty, head) {
            WorktreeRevealTarget::WorkingTreeSummaryRow => {
                self.select_working_tree_summary_row(repo_id, cx)
            }
            WorktreeRevealTarget::WorktreeRow {
                head,
                fallback_scope,
            } => self.request_reveal_worktree(repo_id, head, fallback_scope, path, cx),
            WorktreeRevealTarget::Commit {
                head,
                fallback_scope,
            } => self.request_reveal_commit(repo_id, head, fallback_scope, cx),
            WorktreeRevealTarget::Nothing => {}
        }
    }

    /// Select the pinned uncommitted-changes row at the top of the log.
    pub(in crate::view) fn select_working_tree_summary_row(
        &mut self,
        repo_id: RepoId,
        cx: &mut gpui::Context<Self>,
    ) {
        self.store.dispatch(Msg::ClearCommitSelection { repo_id });
        self.store.dispatch(Msg::ClearDiffSelection { repo_id });
        self.dismiss_history_refs_hover(cx);
        self.history_scroll
            .scroll_to_item_strict(0, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    /// Reveal the row for a linked worktree's uncommitted changes, locating it by
    /// the commit that worktree has checked out.
    pub(in crate::view) fn request_reveal_worktree(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        worktree_path: PathBuf,
        cx: &mut gpui::Context<Self>,
    ) {
        self.store.dispatch(Msg::SelectWorktreeUncommitted {
            repo_id,
            path: worktree_path.clone(),
        });
        self.request_reveal_commit_inner(
            repo_id,
            commit_id,
            fallback_scope,
            Some(worktree_path),
            cx,
        );
    }

    fn request_reveal_commit_inner(
        &mut self,
        repo_id: RepoId,
        commit_id: CommitId,
        fallback_scope: Option<LogScope>,
        worktree_path: Option<PathBuf>,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = PendingHistoryReveal {
            repo_id,
            commit_id,
            fallback_scope,
            worktree_path,
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

    pub(in super::super) fn selected_branch_for_history_row(
        &self,
        repo_id: RepoId,
        selected: bool,
    ) -> Option<SelectedHistoryBranch> {
        selected_branch_for_history_row(self.selected_branch.as_ref(), repo_id, selected)
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

    pub(in super::super) fn set_history_highlight_commit_chain(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_highlight_commit_chain == enabled {
            return;
        }
        self.history_highlight_commit_chain = enabled;
        cx.notify();
    }

    pub(in super::super) fn set_history_relative_dates(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_relative_dates == enabled {
            return;
        }
        self.history_relative_dates = enabled;
        self.ensure_relative_time_tick(cx);
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

    pub(in crate::view) fn drive_pending_history_reveal(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(pending) = self.pending_history_reveal.clone() else {
            return;
        };

        let plan = self.ensure_history_list_plan();
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
                    &plan,
                    self.history_selected_list_index_cache.as_ref(),
                );
                return self.finish_pending_history_reveal(decision, pending, None, &plan, cx);
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
                &plan,
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
                .then_some((log_rev, stashes_rev, current_scope));

        self.finish_pending_history_reveal(decision, pending, cache_meta, &plan, cx);
    }

    fn finish_pending_history_reveal(
        &mut self,
        decision: PendingHistoryRevealDecision,
        pending: PendingHistoryReveal,
        cache_meta: Option<(u64, u64, LogScope)>,
        plan: &HistoryListPlan,
        cx: &mut gpui::Context<Self>,
    ) {
        if let Some(scope) = decision.set_scope {
            self.store.dispatch(Msg::SetHistoryScope {
                repo_id: pending.repo_id,
                scope,
            });
            return;
        }

        match (&pending.worktree_path, decision.select_commit) {
            // A reveal aimed at a worktree row selects the row, not the commit
            // that located it -- and only when the row is not already selected.
            // This runs on every render of the history panel and the reveal
            // stays pending for as long as pagination takes, so dispatching
            // unconditionally would ask for the same selection every frame.
            (Some(path), _) => {
                let already_selected = self.active_repo().is_some_and(|repo| {
                    repo.history_state.worktree_selection.as_deref() == Some(path.as_path())
                });
                if !already_selected {
                    self.store.dispatch(Msg::SelectWorktreeUncommitted {
                        repo_id: pending.repo_id,
                        path: path.clone(),
                    });
                }
            }
            (None, Some(commit_id)) => self.store.dispatch(Msg::SelectCommit {
                repo_id: pending.repo_id,
                commit_id,
            }),
            (None, None) => {}
        }

        // The worktree row sits one line above the commit that located it, so
        // scroll to the row itself once the plan knows where it landed.
        // Two indices, bound together: the commit's own row, and the row to scroll
        // to -- the worktree's, when the reveal was aimed at one, which sits one
        // line above it.
        let reveal_rows = decision.scroll_to_list_ix.map(|commit_list_ix| {
            let scroll_to = pending
                .worktree_path
                .as_deref()
                .and_then(|path| worktree_row_list_ix(plan, self.active_repo(), path))
                .unwrap_or(commit_list_ix);
            (commit_list_ix, scroll_to)
        });

        if let Some((commit_list_ix, list_ix)) = reveal_rows {
            if let Some((log_rev, stashes_rev, history_scope)) = cache_meta {
                // The cache is keyed on the commit and read back as *its* row,
                // so it takes the commit's own index -- not the worktree row we
                // scrolled to, which sits one line above it.
                set_history_selected_list_index_cache(
                    &mut self.history_selected_list_index_cache,
                    pending.repo_id,
                    log_rev,
                    stashes_rev,
                    history_scope,
                    plan,
                    Some(pending.commit_id.clone()),
                    commit_list_ix,
                );
            }
            self.dismiss_history_refs_hover(cx);
            self.history_scroll
                .scroll_to_item_strict(list_ix, gpui::ScrollStrategy::Center);
        } else if decision.load_more {
            self.store.dispatch(Msg::LoadMoreHistory {
                repo_id: pending.repo_id,
            });
        }

        if decision.clear_pending {
            self.pending_history_reveal = None;
            // The target no longer needs shielding from page reconciliation.
            self.store.dispatch(Msg::FinishCommitReveal {
                repo_id: pending.repo_id,
            });
            cx.notify();
        }
    }
}

// Render impl is in history_panel.rs

// --- History cache methods ---

use gitcomet_core::domain::{LogPage, LogScope, RemoteBranch, StashEntry};

impl HistoryView {
    /// The lane the selection sits on. Every other lane — and everything else
    /// coloured from a lane, the nodes, the message borders and the graph fade —
    /// washes out against it.
    ///
    /// The anchor is the selected commit, or HEAD while the workspace's default
    /// row holds the selection. Dirty changes sit on HEAD, and a clean workspace
    /// defaults to HEAD itself, so either state lights the active branch lane.
    /// A multi-selection has no single lane to pick, so nothing washes.
    ///
    /// Memoised because resolving it is a scan of the page — the colour is one
    /// lookup, but pinning it to a row span walks the lane's whole run — and this
    /// is asked once per render rather than once per row.
    pub(in super::super) fn history_selected_lane(
        &mut self,
        show_worktree_summary_row: bool,
    ) -> Option<crate::view::rows::history_graph_paint::SelectedLane> {
        if !self.history_highlight_commit_chain {
            return None;
        }

        let (repo_id, anchor) = {
            let repo = self.active_repo()?;
            if repo.history_state.multi_selection.is_multi() {
                return None;
            }
            let anchor = match history_primary_selection(repo, show_worktree_summary_row)? {
                // A selected worktree row highlights that worktree's branch,
                // not the commit underneath it -- the two differ whenever the
                // branch is behind and has been given a lane of its own.
                HistoryPrimarySelection::Worktree(path) => match &repo.worktree_dirty {
                    Loadable::Ready(dirty) => dirty
                        .iter()
                        .find(|summary| summary.path == path)
                        .and_then(|summary| {
                            Some(HistoryLaneAnchor::Worktree {
                                head: summary.head.clone()?,
                                on_branch: summary.branch.is_some() && !summary.detached,
                            })
                        })?,
                    _ => return None,
                },
                HistoryPrimarySelection::WorkingTree => {
                    HistoryLaneAnchor::Commit(repo.head_commit_id()?)
                }
                HistoryPrimarySelection::Commit(commit_id) => HistoryLaneAnchor::Commit(commit_id),
            };
            (repo.id, anchor)
        };

        let cache = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo_id)?;
        let base_request = &cache.base.request;

        if let Some(memo) = &self.history_selected_lane_color_cache
            && memo.base_request == *base_request
            && memo.anchor == anchor
        {
            return memo.lane;
        }

        let (head, on_branch) = match &anchor {
            HistoryLaneAnchor::Commit(head) => (head, None),
            HistoryLaneAnchor::Worktree { head, on_branch } => (head, Some(*on_branch)),
        };
        let lane = cache
            .base
            .visible_ix_by_commit
            .get(head)
            .copied()
            .and_then(|anchor_row| {
                let row = cache.base.graph_rows.get(anchor_row)?;
                let color_ix = match on_branch {
                    Some(on_branch) => {
                        crate::view::rows::history_graph_paint::band_node_for(row, on_branch)
                            .color_ix
                    }
                    None => row.node_color_ix,
                };
                // The colour alone would also match unrelated lanes elsewhere on
                // the page that recycled the index; this resolves it to the one
                // lane's row span.
                crate::view::rows::history_graph_paint::selected_lane_at(
                    &cache.base.graph_rows,
                    anchor_row,
                    color_ix,
                )
            });

        let base_request = base_request.clone();
        self.history_selected_lane_color_cache = Some(HistorySelectedLaneColorCache {
            base_request,
            anchor,
            lane,
        });
        lane
    }

    /// Builds (or reuses) the mapping from list indices to rows.
    ///
    /// A dirty worktree only earns a row when its HEAD is one of the commits
    /// currently on screen — anchoring it anywhere else would misstate which
    /// commit the changes sit on top of. Worktrees whose HEAD has scrolled out
    /// of the loaded page, or that are on a branch outside the current scope,
    /// simply do not appear.
    pub(in super::super) fn ensure_history_list_plan(&mut self) -> HistoryListPlan {
        let (show_working_tree_summary_row, _) = self.ensure_history_worktree_summary_cache();

        let Some(repo) = self.active_repo() else {
            self.history_list_plan_cache = None;
            return HistoryListPlan::new(show_working_tree_summary_row, Vec::new());
        };
        let repo_id = repo.id;
        let worktrees_rev = repo.worktrees_rev;
        let worktree_dirty_rev = repo.worktree_dirty_rev;

        let Some(cache) = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo_id)
        else {
            self.history_list_plan_cache = None;
            return HistoryListPlan::new(show_working_tree_summary_row, Vec::new());
        };
        let base_request = &cache.base.request;

        if let Some(cached) = &self.history_list_plan_cache
            && cached.base_request == *base_request
            && cached.worktrees_rev == worktrees_rev
            && cached.worktree_dirty_rev == worktree_dirty_rev
            && cached.show_working_tree_summary_row == show_working_tree_summary_row
        {
            return cached.plan.clone();
        }

        let anchors = (|| {
            let Loadable::Ready(dirty) = &repo.worktree_dirty else {
                return Vec::new();
            };
            if dirty.is_empty() {
                return Vec::new();
            }

            // The base cache already indexed the page by commit id, off the render
            // path. Rebuilding that map here would walk every visible commit on
            // every scan revision to answer one lookup per dirty worktree.
            dirty
                .iter()
                .enumerate()
                .filter_map(|(worktree_ix, summary)| {
                    let head = summary.head.as_ref()?;
                    let visible_ix = cache.base.visible_ix_by_commit.get(head).copied()?;
                    Some(HistoryWorktreeRowAnchor {
                        visible_ix,
                        worktree_ix,
                    })
                })
                .collect()
        })();

        let plan = HistoryListPlan::new(show_working_tree_summary_row, anchors);
        // Cloned here rather than up front so a cache hit -- the common case, once
        // per render -- costs a comparison and nothing else.
        let base_request = base_request.clone();
        self.history_list_plan_cache = Some(HistoryListPlanCache {
            base_request,
            worktrees_rev,
            worktree_dirty_rev,
            show_working_tree_summary_row,
            plan: plan.clone(),
        });
        plan
    }

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

            // Shared with the per-worktree scan so the two rows can never
            // report the same tree differently.
            let count_for = gitcomet_core::domain::count_file_statuses;

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
    ) -> Option<Arc<FxHashSet<CommitId>>> {
        enum Action {
            Clear,
            CacheOk(Arc<FxHashSet<CommitId>>),
            Rebuild {
                repo_id: RepoId,
                stashes_rev: u64,
                ids: Arc<FxHashSet<CommitId>>,
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

            let ids: FxHashSet<_> = stashes.iter().map(|s| s.id.clone()).collect();
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
        let Some(repo) = self.active_repo() else {
            self.history_cache_inflight = None;
            self.history_cache = None;
            return;
        };
        let Some(page) = Self::display_log_page_for_repo(repo) else {
            self.history_cache_inflight = None;
            self.history_cache = None;
            return;
        };

        let base_request = self.history_base_cache_request_for_repo(repo, page.as_ref());
        let decoration_request =
            self.history_decoration_cache_request_for_repo(repo, page.as_ref());
        let request_for_task = HistoryCacheBuildRequest {
            base_request: base_request.clone(),
            decoration_request: decoration_request.clone(),
        };

        let cache_ok = self.history_cache.as_ref().is_some_and(|cache| {
            cache.base.request == base_request && cache.decorations.request == decoration_request
        });
        if cache_ok {
            self.history_cache_inflight = None;
            return;
        }
        if self.history_cache_inflight.as_ref() == Some(&request_for_task) {
            return;
        }

        let base_reuse = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request == base_request)
            .map(|cache| cache.base.clone());
        let head_branch = match &repo.head_branch {
            Loadable::Ready(h) => Some(h.clone()),
            _ => None,
        };
        let branches = match &repo.branches {
            Loadable::Ready(b) => Arc::clone(b),
            _ => Arc::new(Vec::new()),
        };
        let remote_branches = match &repo.remote_branches {
            Loadable::Ready(b) => Arc::clone(b),
            _ => Arc::new(Vec::new()),
        };
        let tags = if self.history_show_tags {
            match &repo.tags {
                Loadable::Ready(t) => Arc::clone(t),
                _ => Arc::new(Vec::new()),
            }
        } else {
            Arc::new(Vec::new())
        };
        let stashes = match &repo.stashes {
            Loadable::Ready(s) => Arc::clone(s),
            _ => Arc::new(Vec::new()),
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
    let mut author_cache: FxHashMap<&str, HistoryTextVm> =
        FxHashMap::with_capacity_and_hasher(64, Default::default());
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

    // One entry per visible commit, built here so its readers can look up an id
    // during layout without walking the page.
    let mut visible_ix_by_commit: FxHashMap<CommitId, usize> =
        FxHashMap::with_capacity_and_hasher(visible_indices.len(), Default::default());
    for (visible_ix, commit_ix) in visible_indices.iter().enumerate() {
        if let Some(commit) = page.commits.get(commit_ix) {
            visible_ix_by_commit
                .entry(commit.id.clone())
                .or_insert(visible_ix);
        }
    }

    HistoryBaseCache {
        request,
        visible_indices,
        visible_ix_by_commit: Arc::new(visible_ix_by_commit),
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
    let (mut branch_ref_items_by_target, head_branch_ref_items) =
        build_history_branch_ref_items_by_target(
            branches,
            remote_branches,
            head_branch,
            head_target,
        );
    let mut tag_names_by_target = build_history_tag_names_by_target(tags);
    let mut row_vms = Vec::with_capacity(base.visible_indices.len());

    // Branch attribution per lane column, carried downwards: a lane is started
    // by a branch head, and every commit below inherits it until the lane ends.
    //
    // Correct only because lane columns are stable for a lane's whole life (see
    // `history_graph::Lanes`) -- against shifting columns the carried name would
    // follow whichever lane slid into the column.
    let mut branch_names: Vec<SharedString> = Vec::new();
    // Owned keys: the names come from per-row `ref_items` that do not outlive
    // the iteration. Only ever written on a *miss*, so the allocations are
    // bounded by the number of distinct branch names rather than by rows.
    let mut branch_name_ix: FxHashMap<String, u16> = FxHashMap::default();
    // Local branches with an upstream, so attribution can prefer shared history
    // over a branch that only exists on this machine.
    let tracked_local_branches: FxHashSet<&str> = branches
        .iter()
        .filter(|branch| branch.upstream.is_some())
        .map(|branch| branch.name.as_str())
        .collect();
    // Index into `branch_names`, plus the row its branch head was seen on. The
    // row is what breaks ties where several branches contain the same commit.
    let mut lane_branch_by_col: SmallVec<[Option<(u16, usize)>; 8]> = SmallVec::new();

    // Integration branches present in this repo, each with the set of commits it
    // contains. A commit that is *in* `dev` is dev's, however the graph happens
    // to draw the lane it sits on -- carrying names down lanes alone gets this
    // wrong the moment a feature branch diverges, because the shared history
    // below the fork keeps whichever lane won the node.
    //
    // The names are interned up front, so the per-row lookup below yields an
    // index straight away rather than cloning a `String` on every row.
    let integration_containment: Vec<(u16, Arc<[u64]>)> = {
        let tips = integration_branch_tips(branches, remote_branches);
        let containment =
            build_history_branch_containment_bits(&page.commits, tips.iter().map(|(_, tip)| tip));
        tips.iter()
            .zip(containment)
            .filter_map(|((name, _), bits)| {
                let ix = intern_branch_name(&mut branch_names, &mut branch_name_ix, name)?;
                Some((ix, bits))
            })
            .collect()
    };

    for (visible_ix, (commit_ix, base_row)) in base
        .visible_indices
        .iter()
        .zip(base.row_vms.iter())
        .enumerate()
    {
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
        let branch_items = if base_row.is_head {
            head_branch_ref_items.clone().unwrap_or_default()
        } else {
            branch_ref_items_by_target
                .remove(commit_id)
                .unwrap_or_default()
        };
        let tag_names = tag_names_by_target.remove(commit_id).unwrap_or_default();
        let ref_items = history_ref_items_from_displayed_refs(&tag_names, branch_items);

        let graph_row = base.graph_rows.get(visible_ix);
        let node_col = graph_row.map_or(0, |row| usize::from(row.node_col));

        // Where lanes converge -- a fork point, where a feature branch rejoins
        // the branch it was cut from -- the commit is contained by every
        // converging branch, and taking whichever lane happens to own the node
        // is arbitrary. Prefer the branch head seen *nearest above* this commit,
        // which for the usual "feature cut from dev" shape is the base branch:
        // the feature's head sits further up the log, dev's nearer the shared
        // history. Both answers are true -- git would list both -- but this is
        // the one that matches how people read the graph.
        let mut resolved = lane_branch_by_col.get(node_col).copied().flatten();
        if let Some(graph_row) = graph_row {
            for edge in graph_row.joins_in.iter() {
                let candidate = lane_branch_by_col
                    .get(usize::from(edge.from_col))
                    .copied()
                    .flatten();
                if let Some(candidate) = candidate
                    && resolved.is_none_or(|(_, seeded_at)| candidate.1 > seeded_at)
                {
                    resolved = Some(candidate);
                }
            }
        }

        // Containment in an integration branch outranks everything: the commit
        // genuinely belongs to that branch, whatever lane it is drawn on. The
        // name is already interned, so the common case allocates nothing.
        let contained_in = integration_containment
            .iter()
            .find(|(_, bits)| related_commit_contains(bits, commit_ix))
            .map(|(ix, _)| *ix);

        // Otherwise a branch ref on this row beats anything inherited: the row
        // *is* that branch's head.
        let attributed = contained_in.or_else(|| {
            let name = history_row_attribution_branch(&ref_items, &tracked_local_branches)?;
            intern_branch_name(&mut branch_names, &mut branch_name_ix, name)
        });
        if let Some(ix) = attributed {
            resolved = Some((ix, visible_ix));
        }

        // The surviving lane carries whatever the convergence resolved to, so
        // the rest of the shared history follows the same branch.
        if lane_branch_by_col.len() <= node_col {
            lane_branch_by_col.resize(node_col + 1, None);
        }
        lane_branch_by_col[node_col] = resolved;

        let lane_branch = resolved.map(|(ix, _)| ix);

        // Carry the attribution into the next row: a lane born at this node
        // inherits the node's branch, and a column left empty forgets its own.
        if let Some(graph_row) = graph_row {
            if lane_branch_by_col.len() < graph_row.lanes_next.len() {
                lane_branch_by_col.resize(graph_row.lanes_next.len(), None);
            }
            for (col, lane) in graph_row.lanes_next.iter().enumerate() {
                if !lane.is_active() {
                    lane_branch_by_col[col] = None;
                } else if lane.starts_at_node() {
                    lane_branch_by_col[col] = resolved;
                }
            }
        }

        row_vms.push(HistoryDecorationRowVm {
            branches_text,
            tag_names,
            ref_items,
            lane_branch,
        });
    }

    HistoryDecorationCache {
        request,
        row_vms: row_vms.into(),
        branch_names: branch_names.into(),
    }
}

/// Records `name` in the decoration cache's shared name table and returns its
/// index, reusing the index when the name is already there.
///
/// `None` once the table is full. The index is a `u16`, and saturating at
/// `u16::MAX` instead would hand the same slot to every name past the cap while
/// the table kept growing, so rows would be labelled with someone else's branch.
fn intern_branch_name(
    names: &mut Vec<SharedString>,
    ix_by_name: &mut FxHashMap<String, u16>,
    name: &str,
) -> Option<u16> {
    // Probed by `&str` first: on the hit path -- which is nearly every row in a
    // repo with an integration branch -- this must not allocate a key.
    if let Some(ix) = ix_by_name.get(name) {
        return Some(*ix);
    }
    let ix = u16::try_from(names.len()).ok()?;
    let owned = name.to_owned();
    names.push(SharedString::from(owned.clone()));
    ix_by_name.insert(owned, ix);
    Some(ix)
}

/// Branch name a rendered ref stands for, or `None` for tags and detached HEAD.
fn history_ref_branch_name(item: &HistoryRefListItem) -> Option<&str> {
    match &item.kind {
        HistoryRefListItemKind::AttachedHead { branch } => Some(branch.as_str()),
        HistoryRefListItemKind::LocalBranch { name } => Some(name.as_str()),
        HistoryRefListItemKind::RemoteBranch { name } => Some(name.as_str()),
        HistoryRefListItemKind::Tag { .. } | HistoryRefListItemKind::DetachedHead => None,
    }
}

/// Integration branches present in the repo, highest priority first, as
/// `(display name, tip)`. A local branch is preferred over the remote of the
/// same name so the label matches what the ref column shows.
fn integration_branch_tips(
    branches: &[Branch],
    remote_branches: &[RemoteBranch],
) -> Vec<(String, CommitId)> {
    let mut found: Vec<(String, CommitId)> = Vec::new();
    for wanted in INTEGRATION_BRANCH_NAMES {
        if let Some(branch) = branches.iter().find(|branch| branch.name == wanted) {
            found.push((branch.name.clone(), branch.target.clone()));
            continue;
        }
        if let Some(remote) = remote_branches
            .iter()
            .find(|remote| remote.name == wanted && remote.remote == "origin")
        {
            found.push((
                format!("{}/{}", remote.remote, remote.name),
                remote.target.clone(),
            ));
        }
    }
    found
}

/// Branch names that conventionally carry shared history. A commit sitting on
/// one of these belongs to it, not to whatever short-lived branch happens to be
/// parked on the same commit.
const INTEGRATION_BRANCH_NAMES: [&str; 5] = ["main", "master", "dev", "develop", "trunk"];

/// Which of several branch refs on one commit names the history *below* it.
///
/// Several branches pointing at the same commit are structurally identical --
/// there is no graph signal to separate them -- so this ranks them on what the
/// refs themselves say. Lower is better:
///
/// 0. a conventional integration branch (`main`, `dev`, ...);
/// 1. a branch that is tracked on a remote, so its history is shared;
/// 2. anything else, i.e. a purely local branch.
///
/// The case this exists for: cutting a feature branch and not committing yet
/// leaves `HEAD -> feature` and `dev` on the same commit, and the entire history
/// beneath would otherwise be labelled with the brand-new feature branch.
fn branch_attribution_rank(name: &str, tracked: bool) -> u8 {
    // `origin/dev` ranks as `dev`.
    let leaf = name.rsplit('/').next().unwrap_or(name);
    if INTEGRATION_BRANCH_NAMES.contains(&leaf) {
        0
    } else if tracked {
        1
    } else {
        2
    }
}

/// Best branch ref on a row to attribute the history below it to, or `None`
/// when the row carries no branch ref. Ties keep the rendered ref order.
fn history_row_attribution_branch<'a>(
    ref_items: &'a [HistoryRefListItem],
    tracked_local_branches: &FxHashSet<&str>,
) -> Option<&'a str> {
    ref_items
        .iter()
        .enumerate()
        .filter_map(|(order, item)| {
            let name = history_ref_branch_name(item)?;
            // A remote branch is shared by definition; a local one only if it
            // has an upstream.
            let tracked = match &item.kind {
                HistoryRefListItemKind::RemoteBranch { .. } => true,
                _ => tracked_local_branches.contains(name),
            };
            Some((branch_attribution_rank(name, tracked), order, name))
        })
        .min_by_key(|(rank, order, _)| (*rank, *order))
        .map(|(_, _, name)| name)
}

#[cfg(test)]
mod tests;
