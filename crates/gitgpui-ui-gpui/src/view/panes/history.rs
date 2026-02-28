use super::super::*;
use std::hash::{Hash, Hasher};

mod history_panel;

pub(in super::super) struct HistoryView {
    pub(in super::super) store: Arc<AppStore>,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) date_time_format: DateTimeFormat,
    pub(in super::super) timezone: Timezone,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitGpuiView>,
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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        state.active_repo.hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.log_rev.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.remote_branches_rev.hash(&mut hasher);
            repo.tags_rev.hash(&mut hasher);
            repo.stashes_rev.hash(&mut hasher);
            repo.selected_commit_rev.hash(&mut hasher);
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
        history_show_author: bool,
        history_show_date: bool,
        history_show_sha: bool,
        root_view: WeakEntity<GitGpuiView>,
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
        let mut available = self.last_window_size.width;
        available -= px(280.0);
        available -= px(420.0);
        available -= px(64.0);
        if available <= px(0.0) {
            return (false, false, false);
        }

        let min_message = px(220.0);

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

    pub(in super::super) fn set_timezone(
        &mut self,
        next: Timezone,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.timezone == next {
            return;
        }
        self.timezone = next;
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

use gitgpui_core::domain::{LogPage, RemoteBranch};

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
            },
        }

        let next = if let Some(repo) = self.active_repo() {
            if let Loadable::Ready(page) = &repo.log {
                let request = HistoryCacheRequest {
                    repo_id: repo.id,
                    log_fingerprint: Self::log_fingerprint(&page.commits),
                    head_branch_rev: repo.head_branch_rev,
                    branches_rev: repo.branches_rev,
                    remote_branches_rev: repo.remote_branches_rev,
                    tags_rev: repo.tags_rev,
                    date_time_format: self.date_time_format,
                    timezone: self.timezone,
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
                    }
                }
            } else {
                Next::Clear
            }
        } else {
            Next::Clear
        };

        let (request_for_task, page, head_branch, branches, remote_branches, tags) = match next {
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
            } => (request, page, head_branch, branches, remote_branches, tags),
        };

        self.history_cache_seq = self.history_cache_seq.wrapping_add(1);
        let seq = self.history_cache_seq;
        self.history_cache_inflight = Some(request_for_task.clone());

        let theme = self.theme;

        cx.spawn(
            async move |view: WeakEntity<HistoryView>, cx: &mut gpui::AsyncApp| {
                struct Rebuild {
                    visible_indices: Vec<usize>,
                    graph_rows: Vec<Arc<history_graph::GraphRow>>,
                    max_lanes: usize,
                    commit_row_vms: Vec<HistoryCommitRowVm>,
                }

                let request_for_update = request_for_task.clone();
                let request_for_build = request_for_task.clone();

                let rebuild = smol::unblock(move || {
                    let visible_indices = (0..page.commits.len()).collect::<Vec<_>>();

                    let branch_heads: HashSet<&str> = branches
                        .iter()
                        .map(|b| b.target.as_ref())
                        .chain(remote_branches.iter().map(|b| b.target.as_ref()))
                        .collect();
                    let graph_rows: Vec<Arc<history_graph::GraphRow>> =
                        history_graph::compute_graph(&page.commits, theme, &branch_heads)
                            .into_iter()
                            .map(Arc::new)
                            .collect();
                    let max_lanes = graph_rows
                        .iter()
                        .map(|r| r.lanes_now.len().max(r.lanes_next.len()))
                        .max()
                        .unwrap_or(1);

                    let head_target = head_branch
                        .as_deref()
                        .and_then(|head| branches.iter().find(|b| b.name == head))
                        .map(|b| b.target.as_ref());

                    let mut branch_names_by_target: HashMap<&str, Vec<String>> =
                        HashMap::with_capacity_and_hasher(
                            branches.len() + remote_branches.len(),
                            Default::default(),
                        );
                    for branch in branches.iter() {
                        let should_skip = head_branch
                            .as_ref()
                            .is_some_and(|head| branch.name == *head)
                            && head_target == Some(branch.target.as_ref());
                        if should_skip {
                            continue;
                        }
                        branch_names_by_target
                            .entry(branch.target.as_ref())
                            .or_default()
                            .push(branch.name.clone());
                    }
                    for branch in remote_branches.iter() {
                        branch_names_by_target
                            .entry(branch.target.as_ref())
                            .or_default()
                            .push(format!("{}/{}", branch.remote, branch.name));
                    }
                    for names in branch_names_by_target.values_mut() {
                        names.sort();
                        names.dedup();
                    }

                    let mut tag_names_by_target: HashMap<&str, Vec<&str>> =
                        HashMap::with_capacity_and_hasher(tags.len(), Default::default());
                    for tag in tags.iter() {
                        tag_names_by_target
                            .entry(tag.target.as_ref())
                            .or_default()
                            .push(tag.name.as_str());
                    }
                    for names in tag_names_by_target.values_mut() {
                        names.sort_unstable();
                        names.dedup();
                    }

                    let empty_tags: Arc<[SharedString]> = Vec::new().into();
                    let commit_row_vms = visible_indices
                        .iter()
                        .filter_map(|ix| page.commits.get(*ix))
                        .map(|commit| {
                            let commit_id = commit.id.as_ref();

                            let is_head = head_target == Some(commit_id) && head_branch.is_some();

                            let branches_text = {
                                let branch_count =
                                    branch_names_by_target.get(commit_id).map_or(0, |b| b.len());
                                let mut names: Vec<String> =
                                    Vec::with_capacity(branch_count + usize::from(is_head));
                                if head_target == Some(commit_id)
                                    && let Some(head) = head_branch.as_ref()
                                {
                                    names.push(format!("HEAD → {head}"));
                                }
                                if let Some(branches) = branch_names_by_target.get(commit_id) {
                                    names.extend(branches.iter().cloned());
                                }
                                names.sort();
                                names.dedup();
                                if names.is_empty() {
                                    SharedString::from("")
                                } else {
                                    SharedString::from(names.join(", "))
                                }
                            };

                            let tag_names = tag_names_by_target.get(commit_id).map_or_else(
                                || Arc::clone(&empty_tags),
                                |names| {
                                    let tag_names: Vec<SharedString> = names
                                        .iter()
                                        .copied()
                                        .map(|n| n.to_string().into())
                                        .collect();
                                    tag_names.into()
                                },
                            );

                            let author: SharedString = commit.author.clone().into();
                            let summary: SharedString = commit.summary.clone().into();

                            let when: SharedString = format_datetime(
                                commit.time,
                                request_for_build.date_time_format,
                                request_for_build.timezone,
                            )
                            .into();

                            let id: &str = commit.id.as_ref();
                            let short = id.get(0..8).unwrap_or(id);
                            let short_sha: SharedString = short.to_string().into();

                            HistoryCommitRowVm {
                                branches_text,
                                tag_names,
                                author,
                                summary,
                                when,
                                short_sha,
                                is_head,
                            }
                        })
                        .collect::<Vec<_>>();

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
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
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
