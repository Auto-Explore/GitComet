use super::super::*;
use crate::view::caches::{
    HistoryShortShaVm, HistoryVisibleIndices, HistoryWhenVm, analyze_history_stashes,
    build_history_branch_text_by_target, build_history_tag_names_by_target,
    build_history_visible_indices, next_history_stash_tip_for_commit_ix,
};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

mod history_panel;

fn history_columns_available_width(content_width: Pixels) -> Pixels {
    content_width.max(px(0.0))
}

fn graph_branch_heads<'a>(
    history_scope: LogScope,
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
) -> impl Iterator<Item = &'a str> + 'a {
    let (branches, remote_branches): (&[Branch], &[RemoteBranch]) =
        if history_scope == LogScope::CurrentBranch {
            (&[], &[])
        } else {
            (branches, remote_branches)
        };
    branches
        .iter()
        .map(|b| b.target.as_ref())
        .chain(remote_branches.iter().map(|b| b.target.as_ref()))
}

fn history_column_static_bounds(handle: HistoryColResizeHandle) -> (Pixels, Pixels) {
    match handle {
        HistoryColResizeHandle::Branch => {
            (px(HISTORY_COL_BRANCH_MIN_PX), px(HISTORY_COL_BRANCH_MAX_PX))
        }
        HistoryColResizeHandle::Graph => {
            (px(HISTORY_COL_GRAPH_MIN_PX), px(HISTORY_COL_GRAPH_MAX_PX))
        }
        HistoryColResizeHandle::Author => {
            (px(HISTORY_COL_AUTHOR_MIN_PX), px(HISTORY_COL_AUTHOR_MAX_PX))
        }
        HistoryColResizeHandle::Date => (px(HISTORY_COL_DATE_MIN_PX), px(HISTORY_COL_DATE_MAX_PX)),
        HistoryColResizeHandle::Sha => (px(HISTORY_COL_SHA_MIN_PX), px(HISTORY_COL_SHA_MAX_PX)),
    }
}

#[derive(Copy, Clone)]
struct HistoryColumnWidths {
    branch: Pixels,
    graph: Pixels,
    author: Pixels,
    date: Pixels,
    sha: Pixels,
}

fn default_history_column_widths() -> HistoryColumnWidths {
    HistoryColumnWidths {
        branch: px(HISTORY_COL_BRANCH_PX),
        graph: px(HISTORY_COL_GRAPH_PX),
        author: px(HISTORY_COL_AUTHOR_PX),
        date: px(HISTORY_COL_DATE_PX),
        sha: px(HISTORY_COL_SHA_PX),
    }
}

#[derive(Copy, Clone)]
pub(in crate::view) struct HistoryColumnDragLayout {
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
    preferred: (bool, bool, bool),
    widths: HistoryColumnWidths,
) -> (bool, bool, bool) {
    if available_width <= px(0.0) {
        return (false, false, false);
    }

    let min_message = px(HISTORY_COL_MESSAGE_MIN_PX);

    let (mut show_author, mut show_date, mut show_sha) = preferred;

    // Always show Branch + Graph; Message is flex.
    let fixed_base = widths.branch + widths.graph;
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
    preferred: (bool, bool, bool),
    widths: HistoryColumnWidths,
) -> Pixels {
    let (show_author, show_date, show_sha) =
        history_visible_columns_for_width(available_width, preferred, widths);
    history_column_drag_clamped_width(
        handle,
        candidate,
        available_width,
        HistoryColumnDragLayout {
            show_author,
            show_date,
            show_sha,
            branch_w: widths.branch,
            graph_w: widths.graph,
            author_w: widths.author,
            date_w: widths.date,
            sha_w: widths.sha,
        },
    )
}

fn history_reset_widths_for_available_width(
    available_width: Pixels,
    preferred: (bool, bool, bool),
) -> HistoryColumnWidths {
    let mut widths = default_history_column_widths();
    widths.graph = history_column_drag_next_width(
        HistoryColResizeHandle::Graph,
        widths.graph,
        available_width,
        preferred,
        widths,
    );
    widths.branch = history_column_drag_next_width(
        HistoryColResizeHandle::Branch,
        widths.branch,
        available_width,
        preferred,
        widths,
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
) -> HistoryColumnResizeDragParams {
    let (start_width, drag_delta_sign) = match handle {
        HistoryColResizeHandle::Branch => (layout.branch_w, 1.0),
        HistoryColResizeHandle::Graph => (layout.graph_w, 1.0),
        HistoryColResizeHandle::Author => (layout.author_w, -1.0),
        HistoryColResizeHandle::Date => (layout.date_w, -1.0),
        HistoryColResizeHandle::Sha => (layout.sha_w, -1.0),
    };
    let (min_width, static_max_width) = history_column_static_bounds(handle);
    let other_fixed_width = match handle {
        HistoryColResizeHandle::Branch => {
            layout.graph_w
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
                + layout.graph_w
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
                + layout.graph_w
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
                + layout.graph_w
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
) -> Pixels {
    let dynamic_max = (available_width - params.other_fixed_width - px(HISTORY_COL_MESSAGE_MIN_PX))
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
) -> HistoryColResizeState {
    let visible_columns = history_visible_columns_for_layout(available_width, layout);
    let params = history_column_resize_drag_params(
        handle,
        HistoryColumnDragLayout {
            show_author: visible_columns.0,
            show_date: visible_columns.1,
            show_sha: visible_columns.2,
            ..layout
        },
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
        max_width: history_column_resize_max_width(params, available_width),
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
) -> Pixels {
    if state.bounds_available_width != available_width {
        let params = HistoryColumnResizeDragParams {
            start_width: state.start_width,
            drag_delta_sign: state.drag_delta_sign,
            min_width: state.min_width,
            static_max_width: state.static_max_width,
            other_fixed_width: state.other_fixed_width,
        };
        state.max_width = history_column_resize_max_width(params, available_width);
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
) -> Pixels {
    let params = history_column_resize_drag_params(handle, layout);
    candidate
        .max(params.min_width)
        .min(history_column_resize_max_width(params, available_width))
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
) -> (bool, bool, bool) {
    if let Some(state) = resize_state {
        let current_width = history_column_width_for_handle(layout, state.handle);
        if current_width == state.current_width
            && let Some(columns) = history_resize_state_visible_columns(available, Some(state))
        {
            return columns;
        }
    }

    history_visible_columns_for_layout(available, layout)
}

pub(in crate::view) fn history_visible_columns_for_layout(
    available: Pixels,
    layout: HistoryColumnDragLayout,
) -> (bool, bool, bool) {
    if available <= px(0.0) {
        return (false, false, false);
    }

    let min_message = px(HISTORY_COL_MESSAGE_MIN_PX);

    let mut show_author = layout.show_author;
    let mut show_date = layout.show_date;
    let mut show_sha = layout.show_sha;

    let fixed_base = layout.branch_w + layout.graph_w;
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
    if show_working_tree_summary_row && selected_commit.is_none() {
        set_history_selected_list_index_cache(
            cache,
            repo_id,
            log_rev,
            stashes_rev,
            history_scope,
            show_working_tree_summary_row,
            None,
            0,
        );
        return Some(0);
    }

    if let Some(list_ix) = cache
        .as_ref()
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
    let list_ix = visible_ix + offset;
    set_history_selected_list_index_cache(
        cache,
        repo_id,
        log_rev,
        stashes_rev,
        history_scope,
        show_working_tree_summary_row,
        Some(selected_commit.clone()),
        list_ix,
    );
    Some(list_ix)
}

pub(in super::super) struct HistoryView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
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
    pub(in super::super) history_cache_inflight: Option<HistoryCacheRequest>,
    pub(in super::super) history_col_branch: Pixels,
    pub(in super::super) history_col_graph: Pixels,
    pub(in super::super) history_col_author: Pixels,
    pub(in super::super) history_col_date: Pixels,
    pub(in super::super) history_col_sha: Pixels,
    pub(in super::super) history_show_author: bool,
    pub(in super::super) history_show_date: bool,
    pub(in super::super) history_show_sha: bool,
    pub(in super::super) history_col_graph_auto: bool,
    pub(in super::super) history_col_resize: Option<HistoryColResizeState>,
    pub(in super::super) history_cache: Option<HistoryCache>,
    history_selected_list_index_cache: Option<HistorySelectedListIndexCache>,
    pub(in super::super) history_worktree_summary_cache: Option<HistoryWorktreeSummaryCache>,
    pub(in super::super) history_stash_ids_cache: Option<HistoryStashIdsCache>,
    pub(in super::super) history_scroll: UniformListScrollHandle,
    pub(in super::super) history_panel_focus_handle: FocusHandle,
}

impl HistoryView {
    fn notify_fingerprint_for(state: &AppState) -> u64 {
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
            repo.tags_rev.hash(&mut hasher);
            repo.stashes_rev.hash(&mut hasher);
            repo.history_state.selected_commit_rev.hash(&mut hasher);
            repo.status_rev.hash(&mut hasher);
        }

        hasher.finish()
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn new(
        store: Arc<AppStore>,
        ui_model: Entity<AppUiModel>,
        theme: AppTheme,
        date_time_format: DateTimeFormat,
        timezone: Timezone,
        show_timezone: bool,
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        root_view: WeakEntity<GitCometView>,
        tooltip_host: WeakEntity<TooltipHost>,
        last_window_size: Size<Pixels>,
        _window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) -> Self {
        let state = Arc::clone(&ui_model.read(cx).state);
        let initial_fingerprint = Self::notify_fingerprint_for(&state);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next);
            if next_fingerprint == this.notify_fingerprint {
                this.state = next;
                return;
            }

            this.notify_fingerprint = next_fingerprint;
            this.state = next;
            cx.notify();
        });

        let history_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let default_widths = default_history_column_widths();

        Self {
            store,
            state,
            theme,
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
            history_col_branch: default_widths.branch,
            history_col_graph: default_widths.graph,
            history_col_author: default_widths.author,
            history_col_date: default_widths.date,
            history_col_sha: default_widths.sha,
            history_show_author,
            history_show_date,
            history_show_sha,
            history_col_graph_auto: true,
            history_col_resize: None,
            history_cache: None,
            history_selected_list_index_cache: None,
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

    pub(in super::super) fn history_visible_column_preferences(&self) -> (bool, bool, bool) {
        (
            self.history_show_author,
            self.history_show_date,
            self.history_show_sha,
        )
    }

    pub(in super::super) fn history_visible_columns(&self) -> (bool, bool, bool) {
        let available = self.history_content_width;
        let layout = HistoryColumnDragLayout {
            show_author: self.history_show_author,
            show_date: self.history_show_date,
            show_sha: self.history_show_sha,
            branch_w: self.history_col_branch,
            graph_w: self.history_col_graph,
            author_w: self.history_col_author,
            date_w: self.history_col_date,
            sha_w: self.history_col_sha,
        };
        history_visible_columns_for_layout_with_resize_state(
            available,
            layout,
            self.history_col_resize.as_ref(),
        )
    }

    pub(in super::super) fn reset_history_column_widths(&mut self) {
        let widths = history_reset_widths_for_available_width(
            self.history_content_width,
            self.history_visible_column_preferences(),
        );
        self.history_col_branch = widths.branch;
        self.history_col_graph = widths.graph;
        self.history_col_author = widths.author;
        self.history_col_date = widths.date;
        self.history_col_sha = widths.sha;
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

    pub(in super::super) fn set_date_time_format(
        &mut self,
        next: DateTimeFormat,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.date_time_format == next {
            return;
        }
        self.date_time_format = next;
        self.history_cache = None;
        self.history_cache_inflight = None;
        cx.notify();
    }

    pub(in super::super) fn set_timezone(&mut self, next: Timezone, cx: &mut gpui::Context<Self>) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
        self.history_cache = None;
        self.history_cache_inflight = None;
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
        self.history_cache = None;
        self.history_cache_inflight = None;
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
                status: Arc<RepoStatus>,
                show_row: bool,
                counts: (usize, usize, usize),
            },
        }

        let action = (|| {
            let Some(repo) = self.active_repo() else {
                return Action::Clear;
            };
            let Loadable::Ready(status) = &repo.status else {
                return Action::Clear;
            };

            if let Some(cache) = &self.history_worktree_summary_cache
                && cache.repo_id == repo.id
                && Arc::ptr_eq(&cache.status, status)
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

            let unstaged_counts = count_for(&status.unstaged);
            let staged_counts = count_for(&status.staged);
            let show_row = !status.unstaged.is_empty() || !status.staged.is_empty();
            let counts = (
                unstaged_counts.0 + staged_counts.0,
                unstaged_counts.1 + staged_counts.1,
                unstaged_counts.2 + staged_counts.2,
            );

            Action::Rebuild {
                repo_id: repo.id,
                status: Arc::clone(status),
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
                status,
                show_row,
                counts,
            } => {
                self.history_worktree_summary_cache = Some(HistoryWorktreeSummaryCache {
                    repo_id,
                    status,
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
                request: HistoryCacheRequest,
                page: Arc<LogPage>,
                head_branch: Option<String>,
                branches: Arc<Vec<Branch>>,
                remote_branches: Arc<Vec<RemoteBranch>>,
                tags: Arc<Vec<Tag>>,
                stashes: Arc<Vec<StashEntry>>,
            },
        }

        let next = if let Some(repo) = self.active_repo() {
            if let Loadable::Ready(page) = &repo.log {
                let request = HistoryCacheRequest {
                    repo_id: repo.id,
                    history_scope: repo.history_state.history_scope,
                    log_fingerprint: Self::log_fingerprint(&page.commits),
                    head_branch_rev: repo.head_branch_rev,
                    detached_head_commit: repo.detached_head_commit.clone(),
                    branches_rev: repo.branches_rev,
                    remote_branches_rev: repo.remote_branches_rev,
                    tags_rev: repo.tags_rev,
                    stashes_rev: repo.stashes_rev,
                    date_time_format: self.date_time_format,
                    timezone: self.timezone,
                    show_timezone: self.show_timezone,
                };

                let cache_ok = self
                    .history_cache
                    .as_ref()
                    .is_some_and(|c| c.request == request);
                if cache_ok {
                    Next::CacheOk
                } else if self.history_cache_inflight.as_ref() == Some(&request) {
                    Next::Inflight
                } else {
                    Next::Build {
                        request,
                        page: Arc::clone(page),
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
                        tags: match &repo.tags {
                            Loadable::Ready(t) => Arc::clone(t),
                            _ => Arc::new(Vec::new()),
                        },
                        stashes: match &repo.stashes {
                            Loadable::Ready(s) => Arc::clone(s),
                            _ => Arc::new(Vec::new()),
                        },
                    }
                }
            } else {
                Next::Clear
            }
        } else {
            Next::Clear
        };

        let (request_for_task, page, head_branch, branches, remote_branches, tags, stashes) =
            match next {
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
                } => (
                    request,
                    page,
                    head_branch,
                    branches,
                    remote_branches,
                    tags,
                    stashes,
                ),
            };

        self.history_cache_seq = self.history_cache_seq.wrapping_add(1);
        let seq = self.history_cache_seq;
        self.history_cache_inflight = Some(request_for_task.clone());

        let theme = self.theme;

        cx.spawn(
            async move |view: WeakEntity<HistoryView>, cx: &mut gpui::AsyncApp| {
                struct Rebuild {
                    visible_indices: HistoryVisibleIndices,
                    graph_rows: Arc<[history_graph::GraphRow]>,
                    max_lanes: usize,
                    commit_row_vms: Vec<HistoryCommitRowVm>,
                }

                let request_for_update = request_for_task.clone();
                let request_for_build = request_for_task.clone();

                let build_rebuild = move || {
                    let stash_analysis = analyze_history_stashes(&page.commits, stashes.as_ref());
                    let stash_tips = stash_analysis.stash_tips;
                    let stash_helper_ids = stash_analysis.stash_helper_ids;

                    let visible_indices =
                        build_history_visible_indices(&page.commits, &stash_helper_ids);

                    let head_target = match head_branch.as_deref() {
                        Some("HEAD") => request_for_build
                            .detached_head_commit
                            .as_ref()
                            .map(|id| id.as_ref())
                            .or_else(|| {
                                (request_for_build.history_scope == LogScope::CurrentBranch)
                                    .then(|| {
                                        visible_indices
                                            .first()
                                            .and_then(|ix| page.commits.get(ix))
                                            .map(|c| c.id.as_ref())
                                    })
                                    .flatten()
                            }),
                        Some(head) => branches
                            .iter()
                            .find(|b| b.name == head)
                            .map(|b| b.target.as_ref()),
                        None => None,
                    };

                    let branch_heads = graph_branch_heads(
                        request_for_build.history_scope,
                        branches.as_ref(),
                        remote_branches.as_ref(),
                    );
                    let graph_rows: Arc<[history_graph::GraphRow]> = if stash_helper_ids.is_empty()
                    {
                        history_graph::compute_graph(
                            &page.commits,
                            theme,
                            branch_heads,
                            head_target,
                        )
                        .into()
                    } else {
                        // Reuse the existing visible commits instead of cloning
                        // each filtered row's parent-id vector just for graph
                        // construction.
                        let visible_commit_refs = visible_indices
                            .iter()
                            .map(|ix| &page.commits[ix])
                            .collect::<Vec<_>>();
                        history_graph::compute_graph_refs(
                            &visible_commit_refs,
                            theme,
                            branch_heads,
                            head_target,
                        )
                        .into()
                    };
                    let max_lanes = graph_rows
                        .iter()
                        .map(|r| r.lanes_now.len().max(r.lanes_next.len()))
                        .max()
                        .unwrap_or(1);
                    let (mut branch_text_by_target, head_branches_text) =
                        build_history_branch_text_by_target(
                            branches.as_ref(),
                            remote_branches.as_ref(),
                            head_branch.as_deref(),
                            head_target,
                        );
                    let mut tag_names_by_target = build_history_tag_names_by_target(tags.as_ref());

                    let has_stash_tips = !stash_tips.is_empty();
                    let mut author_cache: HashMap<&str, SharedString> =
                        HashMap::with_capacity_and_hasher(64, Default::default());
                    let mut commit_row_vms = Vec::with_capacity(visible_indices.len());
                    if has_stash_tips {
                        let mut next_stash_tip_ix = 0usize;
                        for ix in visible_indices.iter() {
                            let Some(commit) = page.commits.get(ix) else {
                                continue;
                            };
                            let commit_id = commit.id.as_ref();

                            let is_head = head_target == Some(commit_id);

                            let branches_text = if is_head {
                                head_branches_text.clone().unwrap_or_default()
                            } else {
                                branch_text_by_target.remove(commit_id).unwrap_or_default()
                            };

                            let tag_names =
                                tag_names_by_target.remove(commit_id).unwrap_or_default();

                            let author: SharedString = author_cache
                                .entry(commit.author.as_ref())
                                .or_insert_with(|| commit.author.clone().into())
                                .clone();
                            let (is_stash, summary): (bool, SharedString) =
                                match next_history_stash_tip_for_commit_ix(
                                    &stash_tips,
                                    &mut next_stash_tip_ix,
                                    ix,
                                ) {
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

                            commit_row_vms.push(HistoryCommitRowVm {
                                branches_text,
                                tag_names,
                                author,
                                summary,
                                when: HistoryWhenVm::deferred(commit.time),
                                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                                is_head,
                                is_stash,
                            });
                        }
                    } else {
                        for ix in visible_indices.iter() {
                            let Some(commit) = page.commits.get(ix) else {
                                continue;
                            };
                            let commit_id = commit.id.as_ref();

                            let is_head = head_target == Some(commit_id);

                            let branches_text = if is_head {
                                head_branches_text.clone().unwrap_or_default()
                            } else {
                                branch_text_by_target.remove(commit_id).unwrap_or_default()
                            };

                            let tag_names =
                                tag_names_by_target.remove(commit_id).unwrap_or_default();

                            let author: SharedString = author_cache
                                .entry(commit.author.as_ref())
                                .or_insert_with(|| commit.author.clone().into())
                                .clone();

                            commit_row_vms.push(HistoryCommitRowVm {
                                branches_text,
                                tag_names,
                                author,
                                summary: commit.summary.clone().into(),
                                when: HistoryWhenVm::deferred(commit.time),
                                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                                is_head,
                                is_stash: false,
                            });
                        }
                    }

                    Rebuild {
                        visible_indices,
                        graph_rows,
                        max_lanes,
                        commit_row_vms,
                    }
                };

                let rebuild = if cfg!(test) {
                    build_rebuild()
                } else {
                    smol::unblock(build_rebuild).await
                };

                let _ = view.update(cx, |this, cx| {
                    if this.history_cache_seq != seq {
                        return;
                    }
                    if this.history_cache_inflight.as_ref() != Some(&request_for_update) {
                        return;
                    }
                    if this.active_repo_id() != Some(request_for_update.repo_id) {
                        return;
                    }

                    if this.history_col_graph_auto && this.history_col_resize.is_none() {
                        let required = px(HISTORY_GRAPH_MARGIN_X_PX * 2.0
                            + HISTORY_GRAPH_COL_GAP_PX * (rebuild.max_lanes as f32));
                        this.history_col_graph = history_column_drag_next_width(
                            HistoryColResizeHandle::Graph,
                            required.min(px(HISTORY_COL_GRAPH_MAX_PX)),
                            this.history_content_width,
                            this.history_visible_column_preferences(),
                            HistoryColumnWidths {
                                branch: this.history_col_branch,
                                graph: this.history_col_graph,
                                author: this.history_col_author,
                                date: this.history_col_date,
                                sha: this.history_col_sha,
                            },
                        );
                    }

                    this.history_cache_inflight = None;
                    this.history_cache = Some(HistoryCache {
                        request: request_for_update.clone(),
                        visible_indices: rebuild.visible_indices,
                        graph_rows: rebuild.graph_rows,
                        commit_row_vms: rebuild.commit_row_vms,
                    });
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

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::CommitId;
    use std::time::SystemTime;

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
    fn history_column_drag_clamp_respects_static_maximums() {
        let available = history_columns_available_width(px(1436.0));
        let layout = all_columns_visible_drag_layout();
        let next = history_column_drag_clamped_width(
            HistoryColResizeHandle::Branch,
            px(900.0),
            available,
            layout,
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
        );

        let next_f: f32 = next.into();
        assert!((next_f - 148.0).abs() < 1e-3);
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
        );
        assert_eq!(next, px(HISTORY_COL_SHA_MIN_PX));
    }

    #[test]
    fn graph_drag_ignores_auto_hidden_optional_columns() {
        let available = history_columns_available_width(px(500.0));
        let widths = default_history_column_widths();
        let preferred = (true, true, true);

        assert_eq!(
            history_visible_columns_for_width(available, preferred, widths),
            (false, false, false)
        );

        let next = history_column_drag_next_width(
            HistoryColResizeHandle::Graph,
            px(90.0),
            available,
            preferred,
            widths,
        );

        assert_eq!(next, px(90.0));
    }

    #[test]
    fn reset_widths_clamp_default_graph_in_narrow_windows() {
        let widths = history_reset_widths_for_available_width(px(396.0), (true, true, true));

        assert_eq!(widths.branch, px(HISTORY_COL_BRANCH_PX));
        assert_eq!(widths.graph, px(46.0));
    }

    #[test]
    fn reset_widths_clamp_branch_after_graph_reaches_minimum() {
        let widths = history_reset_widths_for_available_width(px(360.0), (true, true, true));

        assert_eq!(widths.graph, px(HISTORY_COL_GRAPH_MIN_PX));
        assert_eq!(widths.branch, px(96.0));
    }

    #[test]
    fn history_resize_state_uses_actual_visible_columns_in_narrow_windows() {
        let available = history_columns_available_width(px(500.0));
        let layout = all_columns_visible_drag_layout();
        let state =
            history_column_resize_state(HistoryColResizeHandle::Graph, px(0.0), available, layout);

        assert_eq!(
            history_resize_state_visible_columns(available, Some(&state)),
            Some((false, false, false))
        );
    }

    #[test]
    fn history_resize_state_preserves_visible_columns_within_drag_bounds() {
        let available = history_columns_available_width(px(836.0));
        let layout = all_columns_visible_drag_layout();
        let state =
            history_column_resize_state(HistoryColResizeHandle::Graph, px(0.0), available, layout);

        assert!(history_resize_state_preserves_visible_columns(
            available,
            layout,
            Some(&state)
        ));
        assert_eq!(
            history_visible_columns_for_layout_with_resize_state(available, layout, Some(&state)),
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
            history_visible_columns_for_layout_with_resize_state(available, layout, Some(&state)),
            history_visible_columns_for_layout(available, layout)
        );
    }

    #[test]
    fn history_resize_state_visible_columns_fast_path_rejects_stale_current_width() {
        let available = history_columns_available_width(px(836.0));
        let layout = all_columns_visible_drag_layout();
        let state =
            history_column_resize_state(HistoryColResizeHandle::Date, px(0.0), available, layout);

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
            &HistoryVisibleIndices::Filtered(vec![0, 2]),
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
}
