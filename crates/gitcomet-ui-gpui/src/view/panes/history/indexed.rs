use super::indexed_graph::IndexedGraph;
use super::*;
use crate::view::caches::HistoryListRow;
use gitcomet_core::domain::WorktreeDirtySummary;
use gitcomet_core::history_index::{HISTORY_BLOCK_SIZE, HistoryIndexHandle};
use gitcomet_core::services::CancellationToken;
use gitcomet_state::indexed_history::IndexedHistoryMsg as Event;
use std::rc::Rc;

type PreparedDecorations = ((bool, u64), Arc<FxHashMap<usize, HistoryDecorationRowVm>>);

pub(in crate::view) struct Presentation {
    pub key: HistoryBaseCacheRequest,
    pub graph: Arc<IndexedGraph>,
    pub head: Option<String>,
    pub branches: Arc<Vec<Branch>>,
    pub remotes: Arc<Vec<RemoteBranch>>,
    pub stashes: Arc<Vec<StashEntry>>,
    pub head_branch: Option<String>,
    decorations: std::sync::Mutex<Option<PreparedDecorations>>,
    stash_rows: FxHashMap<usize, usize>,
}

fn empty_decoration() -> HistoryDecorationRowVm {
    static EMPTY: std::sync::OnceLock<HistoryDecorationRowVm> = std::sync::OnceLock::new();
    EMPTY
        .get_or_init(|| HistoryDecorationRowVm {
            branches_text: Default::default(),
            tag_names: Arc::from([]),
            branch_chips: Arc::from([]),
            ref_items: Arc::from([]),
            lane_branch: None,
        })
        .clone()
}

impl Presentation {
    fn decorations(
        &self,
        revision: (bool, u64),
        tags: &[Tag],
    ) -> Arc<FxHashMap<usize, HistoryDecorationRowVm>> {
        let mut cache = self.decorations.lock().unwrap();
        if let Some((key, rows)) = &*cache
            && *key == revision
        {
            return rows.clone();
        }
        gitcomet_core::history_perf::record(gitcomet_core::history_perf::Work::DecorationBuild);
        let head = self.head.as_deref();
        let branch = self.head_branch.as_deref();
        let (mut texts, head_text) =
            build_history_branch_text_by_target(&self.branches, &self.remotes, branch, head);
        let (mut chips, head_chips) =
            build_history_branch_chips_by_target(&self.branches, &self.remotes, branch, head);
        let (mut refs, head_refs) =
            build_history_branch_ref_items_by_target(&self.branches, &self.remotes, branch, head);
        if let Some(head) = head {
            if let Some(text) = head_text {
                texts.insert(head, text);
            }
            if let Some(value) = head_chips {
                chips.insert(head, value);
            }
            if let Some(refs_value) = head_refs {
                refs.insert(head, refs_value);
            }
        }
        let tags = build_history_tag_names_by_target(tags);
        let mut rows = FxHashMap::default();
        for (id, text) in texts {
            if let Some(row) = self.graph.projection.position(id) {
                rows.entry(row)
                    .or_insert_with(empty_decoration)
                    .branches_text = text;
            }
        }
        for (id, value) in chips {
            if let Some(row) = self.graph.projection.position(id) {
                rows.entry(row)
                    .or_insert_with(empty_decoration)
                    .branch_chips = value;
            }
        }
        for (id, value) in refs {
            if let Some(row) = self.graph.projection.position(id) {
                rows.entry(row).or_insert_with(empty_decoration).ref_items = value;
            }
        }
        for (id, names) in tags {
            if let Some(row) = self.graph.projection.position(id) {
                let row = rows.entry(row).or_insert_with(empty_decoration);
                row.ref_items =
                    history_ref_items_from_displayed_refs(&names, row.ref_items.clone());
                row.tag_names = names;
            }
        }
        let rows = Arc::new(rows);
        *cache = Some((revision, rows.clone()));
        rows
    }
}

#[derive(Clone)]
pub(in crate::view) struct WindowCache {
    pub start: usize,
    pub cache: HistoryCache,
    pub loaded: Vec<bool>,
    pub labels: Arc<[Option<SharedString>]>,
    pub selected_lane: Option<crate::view::rows::history_graph_paint::SelectedLane>,
    key: WindowKey,
    // Keep pointer-based dependency identities alive for the lifetime of the cache.
    _presentation: Arc<Presentation>,
    _blocks: Vec<Arc<gitcomet_core::history_index::HistoryRange>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WindowKey {
    presentation: usize,
    start: usize,
    end: usize,
    blocks: Vec<(usize, usize)>,
    selection: Option<(usize, Option<bool>)>,
    tags_rev: u64,
    tags_visible: bool,
}

struct PendingPresentation {
    presentation: Arc<Presentation>,
    old_presentation: Option<usize>,
    nearest_survivors: Option<Vec<u32>>,
    mapping: bool,
    cancellation: CancellationToken,
    window: Option<WindowCache>,
}

impl Drop for PendingPresentation {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

#[derive(Default)]
pub(in crate::view) struct IndexedViewState {
    pub presentation: Option<Arc<Presentation>>,
    pending: Option<PendingPresentation>,
    building: Option<(HistoryBaseCacheRequest, CancellationToken)>,
    pub window: Option<Rc<WindowCache>>,
    window_building: Option<(WindowKey, CancellationToken)>,
    requested_blocks: Option<(gitcomet_core::services::HistorySnapshot, Vec<usize>)>,
    pub plan: HistoryListPlan,
    pub worktrees: Arc<Vec<WorktreeDirtySummary>>,
    plan_key: Option<(usize, u64, bool)>,
}

impl Drop for IndexedViewState {
    fn drop(&mut self) {
        if let Some((_, cancel)) = &self.building {
            cancel.cancel();
        }
        if let Some((_, cancel)) = &self.window_building {
            cancel.cancel();
        }
    }
}

fn window_commit_range(
    shown: &Presentation,
    plan: &HistoryListPlan,
    logical: &super::scroll::LogicalViewport,
) -> (usize, usize) {
    let visible = logical.visible_range();
    let to_commit = |list| match plan.row_at(list) {
        Some(
            HistoryListRow::Commit { visible_ix }
            | HistoryListRow::WorktreeUncommitted { visible_ix, .. },
        ) => visible_ix,
        _ => 0,
    };
    let first = to_commit(visible.start).min(shown.graph.projection.len());
    let last = to_commit(visible.end.saturating_sub(1))
        .saturating_add(1)
        .min(shown.graph.projection.len());
    (first, last)
}

impl HistoryView {
    pub(super) fn indexed_is_building(&self) -> bool {
        self.indexed.building.is_some() || self.indexed.pending.is_some()
    }
    fn index_key(&self, repo: &RepoState, index: &HistoryIndexHandle) -> HistoryBaseCacheRequest {
        HistoryBaseCacheRequest {
            repo_id: repo.id,
            history_scope: repo.history_state.history_scope,
            log_source: Arc::as_ptr(index) as usize,
            history_author_filter: repo.history_state.history_author_filter.clone(),
            head_branch_rev: repo.head_branch_rev,
            detached_head_commit: repo.detached_head_commit.clone(),
            head_branch_target: Self::attached_head_target_for_repo(repo),
            branches_rev: repo.branches_rev,
            remote_branches_rev: repo.remote_branches_rev,
            stashes_rev: repo.stashes_rev,
        }
    }

    pub(super) fn ensure_indexed_history(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(repo) = self.active_repo() else {
            self.indexed = Default::default();
            self.scroll_interaction.borrow_mut().logical = None;
            return;
        };
        let repo_id = repo.id;
        if self.indexed.presentation.as_ref().is_some_and(|shown| {
            shown.key.repo_id != repo_id
                || shown.key.history_scope != repo.history_state.history_scope
                || shown.key.history_author_filter != repo.history_state.history_author_filter
        }) {
            self.indexed = Default::default();
            self.scroll_interaction.borrow_mut().logical = None;
        }
        let repo = self.active_repo().unwrap();
        if matches!(repo.log, Loadable::Ready(_))
            && repo.history_state.log_snapshot.is_some()
            && (repo.history_state.indexed.requested != repo.history_state.log_snapshot
                || repo.history_state.indexed.epoch != repo.load_epoch)
        {
            self.store
                .dispatch(Msg::IndexedHistory(Event::Ensure { repo_id }));
        }
        let Some(index) = repo
            .history_state
            .indexed
            .index
            .as_ref()
            .filter(|index| Some(&index.snapshot) == repo.history_state.log_snapshot.as_ref())
            .cloned()
        else {
            return;
        };
        let key = self.index_key(repo, &index);
        if self
            .indexed
            .presentation
            .as_ref()
            .is_some_and(|shown| shown.key == key)
            || self
                .indexed
                .pending
                .as_ref()
                .is_some_and(|pending| pending.presentation.key == key)
            || self
                .indexed
                .building
                .as_ref()
                .is_some_and(|(request, _)| request == &key)
        {
            return;
        }
        let branches = match &repo.branches {
            Loadable::Ready(rows) => rows.clone(),
            _ => Arc::new(Vec::new()),
        };
        let remotes = match &repo.remote_branches {
            Loadable::Ready(rows) => rows.clone(),
            _ => Arc::new(Vec::new()),
        };
        let stashes = match &repo.stashes {
            Loadable::Ready(rows) => rows.clone(),
            _ => Arc::new(Vec::new()),
        };
        let head_branch = match &repo.head_branch {
            Loadable::Ready(head) => Some(head.clone()),
            _ => None,
        };
        let head = match head_branch.as_deref() {
            Some("HEAD") => repo
                .detached_head_commit
                .as_ref()
                .map(|id| id.as_ref().to_owned())
                .or_else(|| {
                    index
                        .mode
                        .guarantees_head_visibility()
                        .then(|| index.commit_id(0))
                        .flatten()
                        .map(|id| id.as_ref().to_owned())
                }),
            Some(name) => branches
                .iter()
                .find(|branch| branch.name == name)
                .map(|branch| branch.target.as_ref().to_owned()),
            None => None,
        };
        if let Some((_, cancellation)) = self.indexed.building.take() {
            cancellation.cancel();
        }
        let previous = self.indexed.presentation.clone();
        let cancellation = CancellationToken::new();
        self.indexed.building = Some((key.clone(), cancellation.clone()));
        cx.spawn(async move |view, cx| {
            let task_key = key.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let reusable = previous.as_ref().filter(|old| {
                        Arc::ptr_eq(&old.graph.projection.index, &index)
                            && old.head == head
                            && old.head_branch == head_branch
                            && old.branches == branches
                            && old.remotes == remotes
                            && old.stashes == stashes
                    });
                    let graph = match reusable {
                        Some(old) => old.graph.clone(),
                        None => Arc::new(IndexedGraph::build_reusing(
                            index,
                            &branches,
                            &remotes,
                            &stashes,
                            head_branch.as_deref(),
                            head.as_deref(),
                            &cancellation,
                            previous.as_ref().map(|old| old.graph.as_ref()),
                        )?),
                    };
                    let presentation = Arc::new(Presentation {
                        key: task_key,
                        stash_rows: stashes
                            .iter()
                            .enumerate()
                            .filter_map(|(ix, stash)| {
                                graph
                                    .projection
                                    .position(stash.id.as_ref())
                                    .map(|row| (row, ix))
                            })
                            .collect(),
                        graph,
                        head,
                        branches,
                        remotes,
                        stashes,
                        head_branch,
                        decorations: Default::default(),
                    });
                    Ok::<_, gitcomet_core::error::Error>(PendingPresentation {
                        presentation,
                        old_presentation: previous.as_ref().map(|old| Arc::as_ptr(old) as usize),
                        nearest_survivors: None,
                        mapping: false,
                        cancellation,
                        window: None,
                    })
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this
                    .indexed
                    .building
                    .as_ref()
                    .is_none_or(|(request, _)| request != &key)
                {
                    return;
                }
                this.indexed.building = None;
                if let Ok(presentation) = result {
                    this.indexed.pending = Some(presentation);
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn apply_indexed_history(&mut self, cx: &mut gpui::Context<Self>) {
        if self.scroll_interaction.borrow().dragging {
            return;
        }
        let Some(mut pending) = self.indexed.pending.take() else {
            return;
        };
        let next = pending.presentation.clone();
        let Some(repo) = self.active_repo() else {
            return;
        };
        if repo.id != next.key.repo_id
            || repo
                .history_state
                .indexed
                .index
                .as_ref()
                .is_none_or(|index| self.index_key(repo, index) != next.key)
        {
            return;
        }
        let height = f64::from(f32::from(crate::view::rows::history_row_height(
            self.ui_scale(),
        )));
        let (old_top, within, viewport) =
            if let Some(logical) = &self.scroll_interaction.borrow().logical {
                (logical.top, logical.within, logical.viewport)
            } else {
                let scroll = self.history_scroll.0.borrow();
                let offset = f64::from(f32::from(-scroll.base_handle.offset().y)).max(0.0);
                (
                    (offset / height).floor() as usize,
                    offset % height,
                    scroll
                        .last_item_size
                        .map_or(600.0, |size| f64::from(f32::from(size.item.height))),
                )
            };
        let old_plan = if self.indexed.presentation.is_some() {
            self.indexed.plan.clone()
        } else {
            self.ensure_history_list_plan()
        };
        let anchor = match old_plan.row_at(old_top) {
            Some(
                HistoryListRow::Commit { visible_ix }
                | HistoryListRow::WorktreeUncommitted { visible_ix, .. },
            ) => self
                .indexed
                .presentation
                .as_ref()
                .and_then(|shown| shown.graph.projection.commit_id(visible_ix))
                .or_else(|| {
                    self.history_cache.as_ref().and_then(|cache| {
                        cache
                            .base
                            .visible_indices
                            .get(visible_ix)
                            .and_then(|raw| cache.page.commits.get(raw))
                            .map(|commit| commit.id.clone())
                    })
                }),
            _ => None,
        };
        let synthetic_path = match old_plan.row_at(old_top) {
            Some(HistoryListRow::WorktreeUncommitted { worktree_ix, .. }) => {
                let dirty = if self.indexed.presentation.is_some() {
                    Some(self.indexed.worktrees.as_slice())
                } else {
                    self.active_repo()
                        .and_then(|repo| match &repo.worktree_dirty {
                            Loadable::Ready(rows) => Some(rows.as_slice()),
                            _ => None,
                        })
                };
                dirty
                    .and_then(|rows| rows.get(worktree_ix))
                    .map(|summary| summary.path.clone())
            }
            _ => None,
        };
        if anchor
            .as_ref()
            .is_some_and(|id| next.graph.projection.position(id.as_ref()).is_none())
            && pending.nearest_survivors.is_none()
            && let Some(old) = self.indexed.presentation.clone()
            && pending.old_presentation == Some(Arc::as_ptr(&old) as usize)
        {
            if !pending.mapping {
                pending.mapping = true;
                let replacement = next.clone();
                let cancellation = pending.cancellation.clone();
                cx.spawn(async move |view, cx| {
                    let identity = Arc::as_ptr(&replacement) as usize;
                    let mapped = cx
                        .background_executor()
                        .spawn(async move {
                            old.graph
                                .projection
                                .nearest_survivors(&replacement.graph.projection, &cancellation)
                        })
                        .await;
                    let _ = view.update(cx, |this, cx| {
                        if let Some(pending) = &mut this.indexed.pending
                            && Arc::as_ptr(&pending.presentation) as usize == identity
                        {
                            pending.mapping = false;
                            if let Ok(mapped) = mapped {
                                pending.nearest_survivors = Some(mapped);
                            }
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            self.indexed.pending = Some(pending);
            return;
        }
        let survivor = anchor
            .as_ref()
            .and_then(|id| next.graph.projection.position(id.as_ref()))
            .or_else(|| {
                if pending.old_presentation
                    != self
                        .indexed
                        .presentation
                        .as_ref()
                        .map(|old| Arc::as_ptr(old) as usize)
                {
                    return None;
                }
                let Some(
                    HistoryListRow::Commit { visible_ix }
                    | HistoryListRow::WorktreeUncommitted { visible_ix, .. },
                ) = old_plan.row_at(old_top)
                else {
                    return None;
                };
                pending
                    .nearest_survivors
                    .as_ref()
                    .and_then(|rows| rows.get(visible_ix))
                    .copied()
                    .filter(|&row| row != gitcomet_core::history_index::MISSING_PARENT)
                    .map(|row| row as usize)
            });
        let show_summary = self.ensure_history_worktree_summary_cache().0;
        let repo = self.active_repo().unwrap();
        let dirty = match &repo.worktree_dirty {
            Loadable::Ready(rows) => rows.clone(),
            _ => Arc::new(Vec::new()),
        };
        let plan_key = (
            Arc::as_ptr(&next) as usize,
            repo.worktree_dirty_rev,
            show_summary,
        );
        let anchors = dirty
            .iter()
            .enumerate()
            .filter_map(|(worktree_ix, summary)| {
                Some(HistoryWorktreeRowAnchor {
                    visible_ix: next
                        .graph
                        .projection
                        .position(summary.head.as_ref()?.as_ref())?,
                    worktree_ix,
                })
            })
            .collect();
        let plan = HistoryListPlan::new(show_summary, anchors);
        let mut logical = super::scroll::LogicalViewport::new(
            plan.list_len(next.graph.projection.len()),
            height,
            viewport,
        );
        let top = synthetic_path
            .as_ref()
            .and_then(|path| dirty.iter().position(|summary| &summary.path == path))
            .and_then(|ix| plan.list_ix_for_worktree(ix))
            .or_else(|| survivor.map(|visible| plan.list_ix_for_visible(visible)))
            .unwrap_or(old_top);
        logical.set_position(top as f64 * height + within);
        let (first, last) = window_commit_range(&next, &plan, &logical);
        let ready = pending.window.as_ref().is_some_and(|window| {
            window.start <= first
                && window.start + window.loaded.len() >= last
                && (first..last).all(|row| {
                    window.loaded[row - window.start]
                        || next
                            .graph
                            .projection
                            .commit_id(row)
                            .is_none_or(|id| self.cached_history_commit(&id).is_none())
                })
        });
        if !ready {
            // Keep the existing rows interactive until the replacement graph and
            // viewport can be published together. Re-evaluate the latest anchor
            // on every frame; a user may scroll while this work is in flight.
            self.indexed.pending = Some(pending);
            self.prepare_window_for(next, plan, dirty, logical, true, cx);
            return;
        }
        self.indexed.presentation = Some(next.clone());
        self.store.dispatch(Msg::IndexedHistory(Event::Publish {
            repo_id: next.key.repo_id,
            index: next.graph.projection.index.clone(),
        }));
        self.indexed.window = pending.window.take().map(Rc::new);
        self.indexed
            .window_building
            .take()
            .inspect(|(_, token)| token.cancel());
        self.indexed.requested_blocks = None;
        self.indexed.plan = plan;
        self.indexed.worktrees = dirty;
        self.indexed.plan_key = Some(plan_key);
        self.scroll_interaction.borrow_mut().logical = Some(logical);
        // The bootstrap page is no longer a presentation or a selection index.
        self.history_cache_seq = self.history_cache_seq.wrapping_add(1);
        self.history_cache_inflight = None;
        self.history_cache = None;
        self.pending_history_cache = None;
        super::scroll::trace(&self.scroll_interaction.borrow(), "snapshot-published");
        self.presented_history = None;
        // The same row rank can name a different commit in the new snapshot.
        self.update_history_row_hover(None, None, cx);
    }

    pub(super) fn sync_indexed_plan(&mut self, cx: &mut gpui::Context<Self>) {
        if self.scroll_interaction.borrow().dragging {
            return;
        }
        let Some(shown) = self.indexed.presentation.clone() else {
            return;
        };
        let show_summary = self.ensure_history_worktree_summary_cache().0;
        let Some(repo) = self.active_repo() else {
            return;
        };
        let key = (
            Arc::as_ptr(&shown) as usize,
            repo.worktree_dirty_rev,
            show_summary,
        );
        if self.indexed.plan_key == Some(key) {
            return;
        }
        let dirty = match &repo.worktree_dirty {
            Loadable::Ready(rows) => rows.clone(),
            _ => Arc::new(Vec::new()),
        };
        let anchors = dirty
            .iter()
            .enumerate()
            .filter_map(|(worktree_ix, summary)| {
                let visible_ix = shown
                    .graph
                    .projection
                    .position(summary.head.as_ref()?.as_ref())?;
                Some(HistoryWorktreeRowAnchor {
                    visible_ix,
                    worktree_ix,
                })
            })
            .collect();
        let plan = HistoryListPlan::new(show_summary, anchors);
        if let Some(logical) = &mut self.scroll_interaction.borrow_mut().logical {
            let next_top = match self.indexed.plan.row_at(logical.top) {
                Some(HistoryListRow::Commit { visible_ix }) => {
                    Some(plan.list_ix_for_visible(visible_ix))
                }
                Some(HistoryListRow::WorktreeUncommitted { worktree_ix, .. }) => self
                    .indexed
                    .worktrees
                    .get(worktree_ix)
                    .and_then(|old| dirty.iter().position(|new| new.path == old.path))
                    .and_then(|ix| plan.list_ix_for_worktree(ix)),
                Some(HistoryListRow::WorkingTreeSummary)
                    if plan.show_working_tree_summary_row() =>
                {
                    Some(0)
                }
                _ => None,
            };
            logical.total = plan.list_len(shown.graph.projection.len());
            logical.set_position(
                next_top.unwrap_or(logical.top) as f64 * logical.height + logical.within,
            );
        }
        self.indexed.plan = plan;
        self.indexed.worktrees = dirty;
        self.indexed.plan_key = Some(key);
        self.update_history_row_hover(None, None, cx);
    }

    pub(super) fn prepare_indexed_window(&mut self, cx: &mut gpui::Context<Self>) {
        let Some(shown) = self.indexed.presentation.clone() else {
            return;
        };
        let Some(logical) = self.scroll_interaction.borrow().logical.clone() else {
            return;
        };
        if self.indexed.pending.is_some() && !self.scroll_interaction.borrow().dragging {
            return;
        }
        self.prepare_window_for(
            shown,
            self.indexed.plan.clone(),
            self.indexed.worktrees.clone(),
            logical,
            false,
            cx,
        );
    }

    fn cached_history_commit(&self, id: &CommitId) -> Option<&Commit> {
        let (cache, window) = if let Some(window) = &self.indexed.window {
            (&window.cache, Some(window))
        } else {
            (self.history_cache.as_ref()?, None)
        };
        let visible = *cache.base.visible_ix_by_commit.get(id)?;
        if window.is_some_and(|window| !window.loaded.get(visible).copied().unwrap_or(false)) {
            return None;
        }
        cache
            .page
            .commits
            .get(cache.base.visible_indices.get(visible)?)
    }

    fn prepare_window_for(
        &mut self,
        shown: Arc<Presentation>,
        plan: HistoryListPlan,
        worktrees: Arc<Vec<WorktreeDirtySummary>>,
        logical: super::scroll::LogicalViewport,
        handoff: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let visible = logical.visible_range();
        let (first, last) = window_commit_range(&shown, &plan, &logical);
        let screen = visible.len().max(1);
        let ahead = if logical.down { 2 } else { 1 };
        let behind = if logical.down { 1 } else { 2 };
        let mut blocks = Vec::new();
        let older = last..(last + ahead * screen).min(shown.graph.projection.len());
        let newer = first.saturating_sub(behind * screen)..first;
        let ranges = if logical.down {
            [first..last, older, newer]
        } else {
            [first..last, newer, older]
        };
        for range in ranges {
            for row in range {
                if let Some(raw) = shown.graph.projection.raw_position(row) {
                    let block = raw / HISTORY_BLOCK_SIZE * HISTORY_BLOCK_SIZE;
                    if !blocks.contains(&block) {
                        blocks.push(block);
                    }
                }
            }
        }
        let snapshot = shown.graph.projection.index.snapshot.clone();
        if self
            .indexed
            .requested_blocks
            .as_ref()
            .is_none_or(|(old_snapshot, old_blocks)| {
                old_snapshot != &snapshot || old_blocks != &blocks
            })
        {
            self.indexed.requested_blocks = Some((snapshot.clone(), blocks.clone()));
            self.store
                .dispatch(Msg::IndexedHistory(Event::RequestRanges {
                    repo_id: shown.key.repo_id,
                    snapshot: snapshot.clone(),
                    blocks,
                    retry: false,
                }));
        }
        let Some(repo) = self.active_repo() else {
            return;
        };
        let start = first / HISTORY_BLOCK_SIZE * HISTORY_BLOCK_SIZE;
        let end = (last.div_ceil(HISTORY_BLOCK_SIZE) * HISTORY_BLOCK_SIZE)
            .min(shown.graph.projection.len());
        let selection = if self.history_highlight_commit_chain
            && !repo.history_state.multi_selection.is_multi()
        {
            match history_primary_selection(repo, plan.show_working_tree_summary_row()) {
                Some(HistoryPrimarySelection::Commit(id)) => shown
                    .graph
                    .projection
                    .position(id.as_ref())
                    .map(|row| (row, None)),
                Some(HistoryPrimarySelection::WorkingTree) => shown
                    .head
                    .as_ref()
                    .and_then(|head| shown.graph.projection.position(head))
                    .map(|row| (row, None)),
                Some(HistoryPrimarySelection::Worktree(path)) => worktrees
                    .iter()
                    .find(|summary| summary.path == path)
                    .and_then(|summary| {
                        shown
                            .graph
                            .projection
                            .position(summary.head.as_ref()?.as_ref())
                            .map(|row| (row, Some(summary.branch.is_some() && !summary.detached)))
                    }),
                None => None,
            }
        } else {
            None
        };
        let key = WindowKey {
            presentation: Arc::as_ptr(&shown) as usize,
            start,
            end,
            blocks: {
                let indexed = &repo.history_state.indexed;
                let mut blocks = Vec::new();
                for row in start..end {
                    let raw = shown.graph.projection.raw_position(row).unwrap();
                    let block = raw / HISTORY_BLOCK_SIZE * HISTORY_BLOCK_SIZE;
                    if blocks.last().is_some_and(|(old, _)| *old == block) {
                        continue;
                    }
                    let identity = (indexed.range_index.as_ref().map(|index| &index.snapshot)
                        == Some(&snapshot))
                    .then(|| indexed.ranges.get(&block))
                    .flatten()
                    .map_or(0, |range| Arc::as_ptr(range) as usize);
                    blocks.push((block, identity));
                }
                blocks
            },
            selection,
            tags_visible: self.history_show_tags,
            tags_rev: if self.history_show_tags {
                repo.tags_rev
            } else {
                0
            },
        };
        if (!handoff
            && self
                .indexed
                .window
                .as_ref()
                .is_some_and(|window| window.key == key))
            || self
                .indexed
                .window_building
                .as_ref()
                .is_some_and(|(request, _)| request == &key)
        {
            return;
        }
        // Seed only this target window by immutable object ID. Missing objects
        // remain placeholders; an existing visible commit never regresses to one.
        let old_window = self
            .indexed
            .window
            .as_ref()
            .map(|window| window.as_ref().clone());
        let bootstrap = self.history_cache.as_ref().map(|cache| cache.page.clone());
        let ranges = repo.history_state.indexed.ranges.clone();
        let range_snapshot = repo
            .history_state
            .indexed
            .range_index
            .as_ref()
            .map(|index| index.snapshot.clone());
        let tags = if self.history_show_tags {
            match &repo.tags {
                Loadable::Ready(rows) => rows.clone(),
                _ => Arc::new(Vec::new()),
            }
        } else {
            Arc::new(Vec::new())
        };
        if let Some((_, token)) = self.indexed.window_building.take() {
            token.cancel();
        }
        let cancellation = CancellationToken::new();
        self.indexed.window_building = Some((key.clone(), cancellation.clone()));
        cx.spawn(async move |view, cx| {
            let task_key = key.clone();
            let result = cx
                .background_executor()
                .spawn(async move {
                    let mut old_window = old_window;
                    if old_window.as_ref().is_some_and(|old| {
                        old.key.presentation == task_key.presentation
                            && old.key.start == task_key.start
                            && old.key.end == task_key.end
                            && old.key.blocks == task_key.blocks
                    }) {
                        // Selection and tag visibility do not change commit text.
                        // Retain the page, formatted text and geometry verbatim.
                        let mut cached = old_window.take().unwrap();
                        cancellation.check_cancelled()?;
                        if cached.key.selection != selection {
                            cached.selected_lane = match selection {
                                Some((row, branch)) => {
                                    shown
                                        .graph
                                        .selected_lane(row, branch, start, &cancellation)?
                                }
                                None => None,
                            };
                        }
                        if (cached.key.tags_visible, cached.key.tags_rev)
                            != (task_key.tags_visible, task_key.tags_rev)
                        {
                            let sparse = shown
                                .decorations((task_key.tags_visible, task_key.tags_rev), &tags);
                            cached.cache.decorations.row_vms = (start..end)
                                .map(|row| {
                                    sparse.get(&row).cloned().unwrap_or_else(empty_decoration)
                                })
                                .collect();
                            cached.cache.decorations.request.tags_rev = task_key.tags_rev;
                        }
                        cached.key = task_key;
                        return Ok(cached);
                    }
                    let seed: FxHashMap<_, _> = old_window
                        .iter()
                        .flat_map(|window| window.cache.page.commits.iter().zip(&window.loaded))
                        .filter(|(_, loaded)| **loaded)
                        .map(|(commit, _)| commit)
                        .chain(bootstrap.iter().flat_map(|page| &page.commits))
                        .map(|commit| (commit.id.clone(), commit.clone()))
                        .collect();
                    let graph = shown.graph.window(start..end, &cancellation)?;
                    let selected_lane = match selection {
                        Some((row, branch)) => {
                            shown
                                .graph
                                .selected_lane(row, branch, start, &cancellation)?
                        }
                        None => None,
                    };
                    gitcomet_core::history_perf::record(
                        gitcomet_core::history_perf::Work::TextWindowBuild,
                    );
                    let mut loaded = Vec::with_capacity(end - start);
                    let mut commits = Vec::with_capacity(end - start);
                    for row in start..end {
                        cancellation.check_cancelled()?;
                        let raw = shown.graph.projection.raw_position(row).unwrap();
                        let block = raw / HISTORY_BLOCK_SIZE * HISTORY_BLOCK_SIZE;
                        let fallback_id = std::cell::OnceCell::new();
                        let id = || {
                            fallback_id.get_or_init(|| {
                                shown.graph.projection.index.commit_id(raw).unwrap()
                            })
                        };
                        let commit = (range_snapshot.as_ref() == Some(&snapshot))
                            .then(|| {
                                ranges
                                    .get(&block)
                                    .and_then(|range| range.commits.get(raw - block))
                            })
                            .flatten()
                            .or_else(|| seed.get(id()));
                        loaded.push(commit.is_some());
                        commits.push(commit.cloned().unwrap_or_else(|| Commit {
                            id: id().clone(),
                            parent_ids: Default::default(),
                            summary: Arc::from(""),
                            author: Arc::from(""),
                            time: std::time::UNIX_EPOCH,
                        }));
                    }
                    let page = Arc::new(LogPage {
                        commits,
                        next_cursor: None,
                    });
                    let mut request = shown.key.clone();
                    request.log_source = Arc::as_ptr(&page) as usize;
                    request.detached_head_commit = shown
                        .head
                        .as_ref()
                        .map(|head| CommitId(head.clone().into()));
                    let row_vms = page
                        .commits
                        .iter()
                        .enumerate()
                        .map(|(ix, commit)| {
                            if let Some(old) = &old_window
                                && old.cache.base.request.repo_id == shown.key.repo_id
                                && old.cache.base.request.branches_rev == shown.key.branches_rev
                                && old.cache.base.request.stashes_rev == shown.key.stashes_rev
                                && old.cache.base.request.head_branch_rev
                                    == shown.key.head_branch_rev
                                && old.cache.base.request.detached_head_commit
                                    == request.detached_head_commit
                                && let Some(&old_ix) =
                                    old.cache.base.visible_ix_by_commit.get(&commit.id)
                                && old.cache.page.commits.get(old_ix) == Some(commit)
                            {
                                return old.cache.base.row_vms[old_ix].clone();
                            }
                            let raw = shown.graph.projection.raw_position(start + ix).unwrap();
                            let stash = shown
                                .stash_rows
                                .get(&(start + ix))
                                .map(|&ix| &shown.stashes[ix]);
                            let is_stash = stash.is_some()
                                || shown.graph.projection.index.is_probable_stash(raw);
                            let summary = if is_stash {
                                stash
                                    .map(|stash| stash.message.as_ref())
                                    .filter(|message| !message.trim().is_empty())
                                    .or_else(|| stash_summary_from_log_summary(&commit.summary))
                                    .unwrap_or(&commit.summary)
                            } else {
                                &commit.summary
                            };
                            HistoryBaseRowVm {
                                author: HistoryTextVm::new(commit.author.clone().into()),
                                summary: HistoryTextVm::new(
                                    if summary == commit.summary.as_ref() {
                                        commit.summary.clone().into()
                                    } else {
                                        SharedString::new(summary)
                                    },
                                ),
                                when: HistoryWhenVm::deferred(commit.time),
                                short_sha: HistoryShortShaVm::new(commit.id.as_ref()),
                                is_head: shown.head.as_deref() == Some(commit.id.as_ref()),
                                is_stash,
                            }
                        })
                        .collect();
                    let base = HistoryBaseCache {
                        request: request.clone(),
                        visible_indices: HistoryVisibleIndices::all(page.commits.len()),
                        visible_ix_by_commit: Arc::new(
                            page.commits
                                .iter()
                                .enumerate()
                                .map(|(ix, commit)| (commit.id.clone(), ix))
                                .collect(),
                        ),
                        graph_rows: graph.rows,
                        row_vms,
                    };
                    let sparse =
                        shown.decorations((task_key.tags_visible, task_key.tags_rev), &tags);
                    let decorations = HistoryDecorationCache {
                        request: HistoryDecorationCacheRequest {
                            base_request: request,
                            head_branch_rev: shown.key.head_branch_rev,
                            detached_head_commit: shown
                                .head
                                .as_ref()
                                .map(|head| CommitId(head.clone().into())),
                            branches_rev: shown.key.branches_rev,
                            remote_branches_rev: shown.key.remote_branches_rev,
                            tags_rev: task_key.tags_rev,
                        },
                        row_vms: (start..end)
                            .map(|row| sparse.get(&row).cloned().unwrap_or_else(empty_decoration))
                            .collect(),
                        // Indexed attribution is already carried in graph.labels.
                        branch_names: Arc::from([]),
                    };
                    Ok::<_, gitcomet_core::error::Error>(WindowCache {
                        start,
                        cache: HistoryCache {
                            page,
                            base,
                            decorations,
                        },
                        loaded,
                        labels: graph.labels,
                        selected_lane,
                        _presentation: shown,
                        _blocks: task_key
                            .blocks
                            .iter()
                            .filter_map(|(start, identity)| {
                                ranges
                                    .get(start)
                                    .filter(|range| Arc::as_ptr(range) as usize == *identity)
                                    .cloned()
                            })
                            .collect(),
                        key: task_key,
                    })
                })
                .await;
            let _ = view.update(cx, |this, cx| {
                if this
                    .indexed
                    .window_building
                    .as_ref()
                    .is_none_or(|(request, _)| request != &key)
                {
                    return;
                }
                this.indexed.window_building = None;
                if let Ok(window) = result {
                    if handoff {
                        if let Some(pending) = &mut this.indexed.pending
                            && Arc::as_ptr(&pending.presentation) as usize == key.presentation
                        {
                            pending.window = Some(window);
                        }
                    } else {
                        this.indexed.window = Some(Rc::new(window));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

impl HistoryView {
    pub(super) fn indexed_history_body(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) -> impl IntoElement {
        let logical = self
            .scroll_interaction
            .borrow()
            .logical
            .clone()
            .expect("indexed viewport");
        let row_height = crate::view::rows::history_row_height(self.ui_scale());
        let range = logical.visible_range();
        // All physical coordinates remain local to the viewport. GPUI's f32
        // Pixels never represents millions of row heights.
        let rows = Self::render_history_rows(self, range.clone(), cx);
        let interaction = self.scroll_interaction.clone();
        let view = cx.entity().downgrade();
        let measure = gpui::canvas(
            move |bounds, window, _cx| {
                let mut state = interaction.borrow_mut();
                if let Some(logical) = &mut state.logical {
                    let height = f64::from(f32::from(bounds.size.height));
                    let row = f64::from(f32::from(row_height));
                    if logical.viewport != height || logical.height != row {
                        let position = logical.top as f64 * row + logical.within.min(row);
                        logical.viewport = height;
                        logical.height = row;
                        logical.set_position(position);
                        let view = view.clone();
                        window.on_next_frame(move |_, cx| {
                            let _ = view.update(cx, |_, cx| cx.notify());
                        });
                    }
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        div()
            .id("history_main_scroll_container")
            .debug_selector(|| "indexed_history_viewport".to_owned())
            .relative()
            .h_full()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(
                div()
                    .id("history_main")
                    .relative()
                    .h_full()
                    .mr(super::history_scrollbar_gutter())
                    .overflow_hidden()
                    .on_scroll_wheel(cx.listener(Self::history_wheel))
                    .child(measure)
                    .children(rows.into_iter().enumerate().map(|(ix, row)| {
                        div()
                            .absolute()
                            .left_0()
                            .w_full()
                            .top(px((ix as f64 * logical.height - logical.within) as f32))
                            .h(row_height)
                            .child(row)
                    })),
            )
            .child(
                components::Scrollbar::new(
                    "history_main_scrollbar",
                    super::scroll::HistoryScrollDriver {
                        view: cx.entity().downgrade(),
                        handle: self.history_scroll.clone(),
                        interaction: self.scroll_interaction.clone(),
                    },
                )
                .always_visible()
                .render(self.theme),
            )
    }

    pub(in crate::view) fn retry_indexed_window(
        &self,
        repo_id: RepoId,
        snapshot: gitcomet_core::services::HistorySnapshot,
        block: Option<usize>,
    ) {
        let blocks = self
            .indexed
            .requested_blocks
            .as_ref()
            .filter(|(current, _)| current == &snapshot)
            .map_or_else(|| block.into_iter().collect(), |(_, blocks)| blocks.clone());
        self.store
            .dispatch(Msg::IndexedHistory(Event::RequestRanges {
                repo_id,
                snapshot,
                blocks,
                retry: true,
            }));
    }

    pub(in crate::view) fn select_indexed_commit(
        &self,
        repo_id: RepoId,
        commit_id: CommitId,
        mode: gitcomet_state::msg::CommitSelectMode,
    ) -> bool {
        let Some(shown) = self
            .indexed
            .presentation
            .as_ref()
            .filter(|shown| shown.key.repo_id == repo_id)
        else {
            return false;
        };
        self.store.dispatch(Msg::IndexedHistory(Event::Select {
            repo_id,
            commit_id,
            mode,
            projection: shown.graph.projection.clone(),
        }));
        true
    }

    fn scroll_indexed_to(&mut self, row: usize, center: bool) {
        if let Some(logical) = &mut self.scroll_interaction.borrow_mut().logical {
            let top = row as f64 * logical.height;
            let position = if center {
                top - (logical.viewport - logical.height) / 2.0
            } else if top < logical.position() {
                top
            } else if top + logical.height > logical.position() + logical.viewport {
                top + logical.height - logical.viewport
            } else {
                logical.position()
            };
            logical.set_position(position);
        }
    }

    pub(super) fn select_adjacent_indexed(
        &mut self,
        direction: i8,
        cx: &mut gpui::Context<Self>,
    ) -> bool {
        let Some(shown) = self.indexed.presentation.clone() else {
            return false;
        };
        let Some(repo) = self.active_repo() else {
            return false;
        };
        let plan = &self.indexed.plan;
        let current = match history_primary_selection(repo, plan.show_working_tree_summary_row()) {
            Some(HistoryPrimarySelection::Commit(id)) => shown
                .graph
                .projection
                .position(id.as_ref())
                .map(|row| plan.list_ix_for_visible(row)),
            Some(HistoryPrimarySelection::WorkingTree) => Some(0),
            Some(HistoryPrimarySelection::Worktree(path)) => {
                let Some(row) = self
                    .indexed
                    .worktrees
                    .iter()
                    .position(|summary| summary.path == path)
                    .and_then(|ix| plan.list_ix_for_worktree(ix))
                else {
                    return false;
                };
                Some(row)
            }
            None => None,
        }
        .unwrap_or(0);
        let total = plan.list_len(shown.graph.projection.len());
        if total == 0 {
            return false;
        }
        let row = current
            .saturating_add_signed(direction as isize)
            .min(total - 1);
        match plan.row_at(row) {
            Some(HistoryListRow::Commit { visible_ix }) => {
                if let Some(id) = shown.graph.projection.commit_id(visible_ix) {
                    self.select_indexed_commit(
                        repo.id,
                        id,
                        gitcomet_state::msg::CommitSelectMode::Single,
                    );
                }
            }
            Some(HistoryListRow::WorkingTreeSummary) => {
                self.select_working_tree_summary_row(repo.id, cx)
            }
            Some(HistoryListRow::WorktreeUncommitted { worktree_ix, .. }) => {
                if let Some(summary) = self.indexed.worktrees.get(worktree_ix) {
                    self.store.dispatch(Msg::SelectWorktreeUncommitted {
                        repo_id: repo.id,
                        path: summary.path.clone(),
                    });
                }
            }
            _ => return false,
        }
        self.cancel_history_scroll_reveal();
        self.scroll_indexed_to(row, false);
        cx.notify();
        true
    }

    pub(super) fn reveal_indexed(&mut self, cx: &mut gpui::Context<Self>) -> bool {
        let Some(shown) = self.indexed.presentation.as_ref() else {
            return false;
        };
        let Some(pending) = self.pending_history_reveal.clone() else {
            return true;
        };
        if pending.repo_id != shown.key.repo_id {
            self.cancel_history_scroll_reveal();
            return true;
        }
        if let Some(visible) = shown
            .graph
            .projection
            .index
            .resolve_reference(pending.commit_id.as_ref())
            .and_then(|row| shown.graph.projection.visible_position(row))
        {
            let row = pending
                .worktree_path
                .as_ref()
                .and_then(|path| {
                    self.indexed
                        .worktrees
                        .iter()
                        .position(|summary| &summary.path == path)
                })
                .and_then(|ix| self.indexed.plan.list_ix_for_worktree(ix))
                .unwrap_or_else(|| self.indexed.plan.list_ix_for_visible(visible));
            let resolved_id = shown.graph.projection.commit_id(visible).unwrap();
            if let Some(path) = &pending.worktree_path {
                self.store.dispatch(Msg::SelectWorktreeUncommitted {
                    repo_id: pending.repo_id,
                    path: path.clone(),
                });
            } else {
                self.select_indexed_commit(
                    pending.repo_id,
                    resolved_id,
                    gitcomet_state::msg::CommitSelectMode::Single,
                );
            }
            self.scroll_indexed_to(row, true);
            self.cancel_history_scroll_reveal();
            cx.notify();
        } else if let Some(scope) = pending
            .fallback_scope
            .filter(|scope| *scope != shown.key.history_scope)
        {
            self.store.dispatch(Msg::SetHistoryScope {
                repo_id: pending.repo_id,
                scope,
            });
        } else if self.active_repo().is_some_and(|repo| {
            repo.history_state.log_snapshot.as_ref() == Some(&shown.graph.projection.index.snapshot)
        }) {
            self.cancel_history_scroll_reveal();
        }
        true
    }
}

#[cfg(test)]
mod cache_regressions {
    use super::*;
    use gitcomet_core::history_index::HistoryIndexBuilder;
    use gitcomet_core::history_perf::{self, Work};
    use gitcomet_core::services::HistorySnapshot;

    #[test]
    fn indexed_history_warm_decorations_do_not_rebuild_ref_maps() {
        let mut builder =
            HistoryIndexBuilder::new(HistorySnapshot("refs".into()), LogScope::AllBranches, 20)
                .unwrap();
        for row in 0..100u8 {
            builder.push(&[row; 20], std::iter::empty(), false).unwrap();
        }
        let index = builder.finish(&CancellationToken::new()).unwrap();
        let head = index.commit_id(0).unwrap();
        let branches = Arc::new(vec![Branch {
            name: "main".into(),
            target: head.clone(),
            upstream: None,
            divergence: None,
        }]);
        let graph = Arc::new(
            IndexedGraph::build(
                index,
                &branches,
                &[],
                &[],
                Some("main"),
                Some(head.as_ref()),
                &CancellationToken::new(),
            )
            .unwrap(),
        );
        let key = HistoryBaseCacheRequest {
            repo_id: RepoId(1),
            history_scope: LogScope::AllBranches,
            log_source: 0,
            history_author_filter: None,
            head_branch_rev: 1,
            detached_head_commit: None,
            head_branch_target: Some(head.clone()),
            branches_rev: 1,
            remote_branches_rev: 1,
            stashes_rev: 1,
        };
        let presentation = Presentation {
            key,
            graph,
            head: Some(head.as_ref().into()),
            branches,
            remotes: Arc::new(Vec::new()),
            stashes: Arc::new(Vec::new()),
            head_branch: Some("main".into()),
            decorations: Default::default(),
            stash_rows: Default::default(),
        };
        let tags: Vec<_> = (0..40_000)
            .map(|row| Tag {
                name: format!("v{row}"),
                target: presentation.graph.projection.commit_id(row % 100).unwrap(),
            })
            .collect();
        let _capture = history_perf::capture();
        let first = presentation.decorations((true, 0), &tags);
        assert_eq!(history_perf::count(Work::DecorationBuild), 1);
        for _ in 0..10 {
            assert!(Arc::ptr_eq(
                &first,
                &presentation.decorations((true, 0), &tags)
            ));
        }
        assert_eq!(history_perf::count(Work::DecorationBuild), 1);
        assert_eq!(first.len(), 100);
        assert_eq!(first[&0].tag_names.len(), 400);
        let before = presentation
            .graph
            .window(0..40, &CancellationToken::new())
            .unwrap();
        let hidden = presentation.decorations((false, 0), &[]);
        assert!(hidden.values().all(|row| row.tag_names.is_empty()));
        let after = presentation
            .graph
            .window(0..40, &CancellationToken::new())
            .unwrap();
        assert!(Arc::ptr_eq(&before.rows, &after.rows));
        assert_eq!(history_perf::count(Work::DecorationBuild), 2);
    }
}
