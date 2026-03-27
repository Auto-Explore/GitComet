use super::super::*;
use crate::view::caches::{
    HistoryShortShaVm, HistoryWhenVm, build_history_branch_text_by_target,
    build_history_tag_names_by_target,
};
use rustc_hash::FxHasher;
use std::hash::{Hash, Hasher};

mod history_panel;

fn history_columns_available_width(window_width: Pixels) -> Pixels {
    let mut available = window_width;
    available -= px(280.0);
    available -= px(420.0);
    available -= px(64.0);
    available.max(px(0.0))
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
struct HistoryColumnDragLayout {
    show_author: bool,
    show_date: bool,
    show_sha: bool,
    branch_w: Pixels,
    graph_w: Pixels,
    author_w: Pixels,
    date_w: Pixels,
    sha_w: Pixels,
}

fn history_column_drag_clamped_width(
    handle: HistoryColResizeHandle,
    candidate: Pixels,
    available_width: Pixels,
    layout: HistoryColumnDragLayout,
) -> Pixels {
    let (min_w, static_max_w) = history_column_static_bounds(handle);
    let right_fixed_w = match handle {
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

    let dynamic_max = (available_width - right_fixed_w - px(HISTORY_COL_MESSAGE_MIN_PX)).max(min_w);
    let max_w = static_max_w.min(dynamic_max).max(min_w);
    candidate.max(min_w).min(max_w)
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
            history_cache_seq: 0,
            history_cache_inflight: None,
            history_col_branch: px(HISTORY_COL_BRANCH_PX),
            history_col_graph: px(HISTORY_COL_GRAPH_PX),
            history_col_author: px(HISTORY_COL_AUTHOR_PX),
            history_col_date: px(HISTORY_COL_DATE_PX),
            history_col_sha: px(HISTORY_COL_SHA_PX),
            history_show_author,
            history_show_date,
            history_show_sha,
            history_col_graph_auto: true,
            history_col_resize: None,
            history_cache: None,
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
        // Prefer keeping commit message visible. Hide SHA first, then date, then author.
        let available = history_columns_available_width(self.last_window_size.width);
        if available <= px(0.0) {
            return (false, false, false);
        }

        let min_message = px(HISTORY_COL_MESSAGE_MIN_PX);

        let mut show_author = self.history_show_author;
        let mut show_date = self.history_show_date;
        let mut show_sha = self.history_show_sha;

        // Always show Branch + Graph; Message is flex.
        let fixed_base = self.history_col_branch + self.history_col_graph;
        let mut fixed = fixed_base
            + if show_author {
                self.history_col_author
            } else {
                px(0.0)
            }
            + if show_date {
                self.history_col_date
            } else {
                px(0.0)
            }
            + if show_sha {
                self.history_col_sha
            } else {
                px(0.0)
            };

        if available - fixed < min_message && show_sha {
            show_sha = false;
            fixed -= self.history_col_sha;
        }
        if available - fixed < min_message {
            if show_date {
                show_date = false;
                fixed -= self.history_col_date;
            }
            show_sha = false;
        }
        if available - fixed < min_message && show_author {
            show_author = false;
            fixed -= self.history_col_author;
        }

        if available - fixed < min_message {
            show_author = false;
            show_date = false;
            show_sha = false;
        }

        (show_author, show_date, show_sha)
    }

    pub(in super::super) fn reset_history_column_widths(&mut self) {
        self.history_col_branch = px(HISTORY_COL_BRANCH_PX);
        self.history_col_graph = px(HISTORY_COL_GRAPH_PX);
        self.history_col_author = px(HISTORY_COL_AUTHOR_PX);
        self.history_col_date = px(HISTORY_COL_DATE_PX);
        self.history_col_sha = px(HISTORY_COL_SHA_PX);
        self.history_col_graph_auto = true;
        self.history_col_resize = None;
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
                    visible_indices: Vec<usize>,
                    graph_rows: Arc<[history_graph::GraphRow]>,
                    max_lanes: usize,
                    commit_row_vms: Vec<HistoryCommitRowVm>,
                }

                let request_for_update = request_for_task.clone();
                let request_for_build = request_for_task.clone();

                let rebuild = smol::unblock(move || {
                    let mut stash_messages_by_id: HashMap<&str, Arc<str>> =
                        HashMap::with_capacity_and_hasher(stashes.len(), Default::default());
                    for stash in stashes.iter() {
                        stash_messages_by_id.insert(stash.id.as_ref(), stash.message.clone());
                    }

                    let stash_tip_ids_from_list: HashSet<&str> =
                        stash_messages_by_id.keys().copied().collect();
                    let mut stash_tip_ids = stash_tip_ids_from_list.clone();
                    let derive_helper_ids_from_detected_tips = stash_tip_ids_from_list.is_empty();
                    let mut stash_helper_ids: HashSet<&str> = HashSet::with_capacity_and_hasher(
                        stash_tip_ids_from_list.len().max(4),
                        Default::default(),
                    );
                    for commit in page.commits.iter() {
                        let commit_id = commit.id.as_ref();
                        let is_probable_stash = is_probable_stash_tip(commit);
                        if is_probable_stash {
                            stash_tip_ids.insert(commit_id);
                        }

                        let should_collect_helpers = if derive_helper_ids_from_detected_tips {
                            is_probable_stash
                        } else {
                            stash_tip_ids_from_list.contains(commit_id)
                        };
                        if !should_collect_helpers {
                            continue;
                        }

                        for parent_id in commit.parent_ids.iter().skip(1).map(|p| p.as_ref()) {
                            stash_helper_ids.insert(parent_id);
                        }
                    }

                    let visible_indices: Vec<usize> = if stash_helper_ids.is_empty() {
                        (0..page.commits.len()).collect()
                    } else {
                        let mut visible_indices =
                            Vec::with_capacity(page.commits.len() - stash_helper_ids.len());
                        for (ix, commit) in page.commits.iter().enumerate() {
                            if !stash_helper_ids.contains(commit.id.as_ref()) {
                                visible_indices.push(ix);
                            }
                        }
                        visible_indices
                    };

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
                                            .and_then(|&ix| page.commits.get(ix))
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
                            .map(|&ix| &page.commits[ix])
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
                    let (branch_text_by_target, head_branches_text) =
                        build_history_branch_text_by_target(
                            branches.as_ref(),
                            remote_branches.as_ref(),
                            head_branch.as_deref(),
                            head_target,
                        );
                    let tag_names_by_target = build_history_tag_names_by_target(tags.as_ref());

                    let empty_tags: Arc<[SharedString]> = Vec::new().into();
                    let mut author_cache: HashMap<&str, SharedString> =
                        HashMap::with_capacity_and_hasher(64, Default::default());
                    let mut commit_row_vms = Vec::with_capacity(visible_indices.len());
                    for &ix in &visible_indices {
                        let Some(commit) = page.commits.get(ix) else {
                            continue;
                        };
                        let commit_id = commit.id.as_ref();

                        let is_head = head_target == Some(commit_id);

                        let branches_text = if is_head {
                            head_branches_text.clone().unwrap_or_default()
                        } else {
                            branch_text_by_target
                                .get(commit_id)
                                .cloned()
                                .unwrap_or_default()
                        };

                        let tag_names = tag_names_by_target
                            .get(commit_id)
                            .cloned()
                            .unwrap_or_else(|| Arc::clone(&empty_tags));

                        let author: SharedString = author_cache
                            .entry(commit.author.as_ref())
                            .or_insert_with(|| commit.author.clone().into())
                            .clone();
                        let is_stash = stash_tip_ids.contains(commit_id);
                        let summary: SharedString = if is_stash {
                            stash_messages_by_id
                                .get(commit_id)
                                .filter(|message| !message.trim().is_empty())
                                .cloned()
                                .map(Into::into)
                                .or_else(|| {
                                    stash_summary_from_log_summary(&commit.summary)
                                        .map(SharedString::new)
                                })
                                .unwrap_or_else(|| commit.summary.clone().into())
                        } else {
                            commit.summary.clone().into()
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

                    Rebuild {
                        visible_indices,
                        graph_rows,
                        max_lanes,
                        commit_row_vms,
                    }
                })
                .await;

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
                        this.history_col_graph = required
                            .min(px(HISTORY_COL_GRAPH_MAX_PX))
                            .max(px(HISTORY_COL_GRAPH_MIN_PX));
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

fn is_probable_stash_tip(commit: &Commit) -> bool {
    if !(2..=3).contains(&commit.parent_ids.len()) {
        return false;
    }
    let summary: &str = &commit.summary;
    (summary.starts_with("WIP on ") || summary.starts_with("On ")) && summary.contains(": ")
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
        let available = history_columns_available_width(px(2200.0));
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
        let available = history_columns_available_width(px(1600.0));
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
        let available = history_columns_available_width(px(2200.0));
        let layout = all_columns_visible_drag_layout();
        let next = history_column_drag_clamped_width(
            HistoryColResizeHandle::Sha,
            px(0.0),
            available,
            layout,
        );
        assert_eq!(next, px(HISTORY_COL_SHA_MIN_PX));
    }
}
