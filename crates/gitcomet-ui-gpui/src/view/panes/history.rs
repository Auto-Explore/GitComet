use super::super::*;
use crate::view::caches::{
    HistoryListPlan, HistoryListPlanCache, HistoryShortShaVm, HistoryVisibleIndices, HistoryWhenVm,
    HistoryWorktreeRowAnchor, analyze_history_stashes, build_history_branch_chips_by_target,
    build_history_branch_containment_bits, build_history_branch_ref_items_by_target,
    build_history_branch_text_by_target, build_history_tag_names_by_target,
    build_history_visible_indices, history_ref_items_from_displayed_refs,
    next_history_stash_tip_for_commit_ix, related_commit_contains,
};
use rustc_hash::FxHasher;
use smallvec::SmallVec;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

pub(in crate::view) mod annotations;
mod cache_build;
mod columns;
pub(in crate::view) mod find;
mod history_panel;
mod indexed;
pub(in crate::view) mod indexed_graph;
mod loading;
mod scroll;
mod signatures;
mod viewport;

use cache_build::*;
pub(in crate::view) use columns::*;

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

/// Keyed on the base cache's whole request rather than its `log_source`:
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

struct PendingHistorySelection {
    request_id: u64,
    selection: HistoryPrimarySelection,
    multi_selection: PendingHistoryMembers,
}

/// Keep a pending indexed range compact: shift-clicking across a large
/// history must not materialize its commit IDs on the UI thread. Membership
/// is only needed if another modifier click arrives before the acknowledgment.
enum PendingHistoryMembers {
    Commits(gitcomet_state::model::CommitMultiSelection),
    IndexedRange {
        projection: gitcomet_core::history_index::HistoryProjection,
        rows: std::ops::RangeInclusive<usize>,
        anchor: Option<CommitId>,
        anchor_index: usize,
        log_rev: u64,
    },
}

impl PendingHistoryMembers {
    fn anchor(&self) -> Option<&CommitId> {
        match self {
            Self::Commits(selection) => selection.anchor.as_ref(),
            Self::IndexedRange { anchor, .. } => anchor.as_ref(),
        }
    }

    fn materialize(&self) -> gitcomet_state::model::CommitMultiSelection {
        match self {
            Self::Commits(selection) => selection.clone(),
            Self::IndexedRange {
                projection,
                rows,
                anchor,
                anchor_index,
                log_rev,
            } => gitcomet_state::model::CommitMultiSelection {
                commits: Arc::new(
                    rows.clone()
                        .filter_map(|row| projection.commit_id(row))
                        .collect(),
                ),
                anchor: anchor.clone(),
                anchor_index: Some(*anchor_index),
                anchor_log_rev: Some(*log_rev),
            },
        }
    }
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
    pub(in super::super) store: crate::view::pane_store::PaneStore,
    state: Arc<AppState>,
    pub(in super::super) theme: AppTheme,
    pub(in super::super) ui_scale_percent: u32,
    pub(in crate::view) appearance_metrics: crate::appearance::Appearance,
    pub(in super::super) date_time_format: DateTimeFormat,
    pub(in super::super) timezone: Timezone,
    pub(in super::super) show_timezone: bool,
    pub(in super::super) history_relative_dates: bool,
    pub(in super::super) history_highlight_commit_chain: bool,
    _ui_model_subscription: gpui::Subscription,
    root_view: WeakEntity<GitCometView>,
    tooltip_host: WeakEntity<TooltipHost>,
    /// Which row sub-area the pointer is over, so the shared tooltip is only
    /// rewritten when the hover actually moves rather than on every pixel.
    row_hover: Option<(usize, HistoryRowHoverArea)>,
    row_hover_scroll_position: f64,
    /// Exactly what we last handed the shared host, so we only ever retract
    /// our own tooltip and never one another surface has since set.
    row_hover_tooltip: Option<SharedString>,
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
    pub(in super::super) history_branch_names: HistoryBranchNamesMode,
    pub(in super::super) history_show_graph: bool,
    pub(in super::super) history_show_graph_gradient: bool,
    pub(in super::super) history_show_author: bool,
    pub(in super::super) history_show_date: bool,
    pub(in super::super) history_show_sha: bool,
    pub(in super::super) history_show_tags: bool,
    pub(in super::super) history_auto_fetch_tags_on_repo_activation: bool,
    pub(in super::super) history_col_resize: Option<HistoryColResizeState>,
    pub(in super::super) history_cache: Option<HistoryCache>,
    pending_history_cache: Option<HistoryCache>,
    history_selected_list_index_cache: Option<HistorySelectedListIndexCache>,
    selected_branch: Option<SelectedBranch>,
    pending_history_reveal: Option<PendingHistoryReveal>,
    /// Last browse-point commit we scrolled to, so a new one is revealed only when
    /// the historical browse point actually changes.
    last_browse_commit: Option<CommitId>,
    pub(in super::super) history_worktree_summary_cache: Option<HistoryWorktreeSummaryCache>,
    history_list_plan_cache: Option<HistoryListPlanCache>,
    presented_history: Option<viewport::PresentedHistory>,
    scroll_interaction: scroll::SharedScrollInteraction,
    pub(in crate::view) indexed: indexed::IndexedViewState,
    pub(in crate::view) loading: loading::HistoryLoading,
    history_selected_lane_color_cache: Option<HistorySelectedLaneColorCache>,
    pub(in super::super) history_stash_ids_cache: Option<HistoryStashIdsCache>,
    pub(in super::super) history_scroll: UniformListScrollHandle,
    pub(in super::super) history_panel_focus_handle: FocusHandle,
    pub(in crate::view) history_annotations: Option<annotations::HistoryAnnotations>,
    /// Minute tick that re-renders the table while the relative date format is
    /// active, so "2 mins ago" labels don't freeze. `None` for absolute formats.
    relative_time_tick: Option<gpui::Task<()>>,
    signature_viewport: Option<signatures::ViewportKey>,
    signature_debounce: Option<gpui::Task<()>>,
    /// The Cmd-F find bar, created the first time it opens.
    find: Option<find::HistoryFind>,
    // Inputs can arrive before the store publishes their selections. Keep the
    // outstanding destinations so navigation starts from the latest intent.
    pending_history_selections: std::collections::VecDeque<PendingHistorySelection>,
}

/// Canvas sub-areas drive the shared [`TooltipHost`] without retained tooltips.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::view) enum HistoryRowHoverArea {
    Signature,
    Date,
    Author,
}

impl HistoryView {
    /// Point the shared tooltip host at the row sub-area now under the pointer.
    ///
    /// Deliberately does not `notify`: nothing in the row's paint depends on this
    /// hover, so repainting every row on pointer movement would be pure waste.
    pub(in crate::view) fn update_history_row_hover(
        &mut self,
        next: Option<(usize, HistoryRowHoverArea)>,
        tooltip: Option<SharedString>,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.row_hover == next && (next.is_none() || self.row_hover(cx) == next) {
            return;
        }
        self.row_hover = next;
        self.row_hover_scroll_position = self.history_scroll_position();
        let Some(host) = self.tooltip_host.upgrade() else {
            return;
        };
        let previous = self.row_hover_tooltip.take();
        self.row_hover_tooltip = tooltip.clone();
        host.update(cx, |host, cx| match tooltip {
            Some(text) => {
                host.set_tooltip_text_if_changed(Some(text), cx);
            }
            None => {
                // Retract only our own text. A blanket `clear_tooltip` would
                // also cancel a pending reveal another surface just armed.
                if let Some(previous) = previous {
                    host.clear_tooltip_if_matches(&previous, cx);
                }
            }
        });
    }

    /// Forget the hover without touching the host.
    ///
    /// Any mouse-down clears the host from the window root, which would leave
    /// this mirror claiming a tooltip that is no longer shown — and then the
    /// equality gate would suppress re-showing it until the pointer left the
    /// cell and came back.
    pub(in crate::view) fn reset_history_row_hover(&mut self) {
        self.row_hover = None;
        self.row_hover_tooltip = None;
    }

    pub(in crate::view) fn row_hover(&self, cx: &App) -> Option<(usize, HistoryRowHoverArea)> {
        let host = self.tooltip_host.upgrade()?;
        let tooltip = self.row_hover_tooltip.as_ref()?;
        host.read(cx)
            .tooltip_text_matches(tooltip)
            .then_some(self.row_hover)
            .flatten()
    }

    /// Virtualization can remove the owning row and its mouse listener entirely.
    /// Check the active viewport when the list lays out, including scrollbar
    /// drags and programmatic reveals that do not send a wheel event.
    pub(in crate::view) fn clear_history_row_hover_if_scrolled(
        &mut self,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.row_hover.is_some()
            && self.history_scroll_position() != self.row_hover_scroll_position
        {
            self.update_history_row_hover(None, None, cx);
        }
    }

    /// Window-space bounds of the active repository's history list once it
    /// has been laid out, for scripted input that must land on it.
    pub(in crate::view) fn history_viewport_bounds(&self) -> Option<Bounds<Pixels>> {
        let active = self.active_repo_id()?;
        if self.indexed.presentation.is_some() {
            let (repo_id, bounds) = self.scroll_interaction.borrow().viewport_bounds?;
            return (repo_id == active).then_some(bounds);
        }
        // The list's bounds outlive a repository switch; its cache does not.
        let cache_repo = self.history_cache.as_ref()?.base.request.repo_id;
        let bounds = self.history_scroll.0.borrow().base_handle.bounds();
        (cache_repo == active && bounds.size.height > px(0.0)).then_some(bounds)
    }

    /// Geometry for native pointer dispatch in the opt-in performance driver.
    pub(in crate::view) fn scenario_drag_points(
        &self,
        fraction: f32,
    ) -> Option<(Point<Pixels>, Point<Pixels>)> {
        let bounds = self.history_viewport_bounds()?;
        let scroll = self.scroll_interaction.borrow();
        let start = if let Some(logical) = &scroll.logical {
            let position = if logical.max() > 0.0 {
                logical.position() / logical.max()
            } else {
                0.0
            };
            point(
                // The measured content canvas excludes the scrollbar gutter.
                bounds.right() + px(8.0),
                bounds.top() + px(16.0) + (bounds.size.height - px(32.0)) * position as f32,
            )
        } else {
            point(bounds.right() + px(8.0), bounds.top() + px(28.0))
        };
        Some((
            start,
            point(start.x, bounds.top() + bounds.size.height * fraction),
        ))
    }

    pub(in crate::view) fn history_scroll_position(&self) -> f64 {
        self.scroll_interaction
            .borrow()
            .logical
            .as_ref()
            .map_or_else(
                || {
                    f64::from(f32::from(
                        -self.history_scroll.0.borrow().base_handle.offset().y,
                    ))
                },
                |logical| logical.position(),
            )
    }

    /// The absolute, timezone-qualified rendering of a commit time, for the date
    /// column's tooltip. Built on demand from the pointer handler so a relative
    /// label like "4 days ago" costs nothing until someone actually hovers it.
    pub(in crate::view) fn full_commit_time_text(
        &self,
        time: std::time::SystemTime,
    ) -> SharedString {
        let mut text = String::with_capacity(32);
        crate::view::date_time::format_datetime_into(
            &mut text,
            time,
            self.date_time_format,
            self.timezone,
            true,
        );
        text.into()
    }

    fn notify_fingerprint_for(state: &AppState, show_history_tags: bool) -> u64 {
        let mut hasher = FxHasher::default();
        state.active_repo.hash(&mut hasher);
        state
            .git_log_settings
            .verify_commit_signatures
            .hash(&mut hasher);

        if let Some(repo_id) = state.active_repo
            && let Some(repo) = state.repos.iter().find(|r| r.id == repo_id)
        {
            repo.log_rev.hash(&mut hasher);
            repo.history_state.log_rev.hash(&mut hasher);
            repo.history_state.history_scope.hash(&mut hasher);
            repo.head_branch_rev.hash(&mut hasher);
            repo.detached_head_commit.hash(&mut hasher);
            repo.branches_rev.hash(&mut hasher);
            repo.remote_branches_rev.hash(&mut hasher);
            if show_history_tags {
                repo.tags_rev.hash(&mut hasher);
            }
            repo.stashes_rev.hash(&mut hasher);
            repo.history_state.selected_commit_rev.hash(&mut hasher);
            repo.history_state.indexed.rev.hash(&mut hasher);
            // The browsed commit's mark. Not `file_browser_rev`: that moves on
            // every sidebar search keystroke.
            repo.file_browser.source.hash(&mut hasher);
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
        history_show_graph_gradient: bool,
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
        let history_branch_names = ui_model.read(cx).preferences.history.branch_names;
        let initial_fingerprint = Self::notify_fingerprint_for(&state, history_show_tags);
        let subscription = cx.observe(&ui_model, |this, model, cx| {
            let next = Arc::clone(&model.read(cx).state);
            let next_fingerprint = Self::notify_fingerprint_for(&next, this.history_show_tags);
            let changed = next_fingerprint != this.notify_fingerprint;
            let switched_repo = this.state.active_repo != next.active_repo;
            let previous_repo = this.active_repo();
            let next_repo = next
                .repos
                .iter()
                .find(|repo| Some(repo.id) == next.active_repo);
            // Badges, find results, dropped selections and the index build's
            // status line repaint without invalidating refs menus.
            let signatures_changed = previous_repo
                .map(|repo| repo.history_state.commit_signatures_rev)
                != next_repo.map(|repo| repo.history_state.commit_signatures_rev);
            let index_status_changed = previous_repo
                .map(|repo| repo.history_state.indexed.status_rev)
                != next_repo.map(|repo| repo.history_state.indexed.status_rev);
            let find_changed = this.history_find_is_open()
                && previous_repo.map(|repo| repo.history_state.find.rev)
                    != next_repo.map(|repo| repo.history_state.find.rev);
            let selection_ack = next_repo.and_then(|repo| repo.history_state.selection_ack);
            let selection_ack_changed =
                previous_repo.and_then(|repo| repo.history_state.selection_ack) != selection_ack;
            if switched_repo {
                this.history_find_repo_changed();
                this.pending_history_selections.clear();
            } else if selection_ack_changed
                && let Some(ix) = this
                    .pending_history_selections
                    .iter()
                    .position(|pending| Some(pending.request_id) == selection_ack)
            {
                // Snapshots can coalesce replies. Retire through the acknowledged
                // request, even when its outcome repeated a row or was a no-op.
                this.pending_history_selections.drain(..=ix);
            }
            this.state = next;
            if selected_remote_branch_is_missing(&this.state, this.selected_branch.as_ref()) {
                this.selected_branch = None;
            }

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
                // A browse point that follows the selection is already the
                // visible row; revealing it would only scroll, page, or widen
                // the scope on every arrow key.
                let is_selected_row = this
                    .active_repo()
                    .is_some_and(|repo| repo.history_state.selected_commit == browse_commit);
                if !is_selected_row
                    && let (Some(repo_id), Some(commit_id)) = (this.active_repo_id(), browse_commit)
                {
                    this.request_reveal_commit(repo_id, commit_id, Some(LogScope::AllBranches), cx);
                }
            }

            if signatures_changed
                && matches!(this.row_hover, Some((_, HistoryRowHoverArea::Signature)))
            {
                // Verification can remove or replace a badge under a resting
                // pointer. Its old verdict must not survive in the tooltip.
                this.update_history_row_hover(None, None, cx);
            }
            if changed {
                this.notify_fingerprint = next_fingerprint;
                this.dismiss_history_refs_hover(cx);
                cx.notify();
            } else if signatures_changed
                || find_changed
                || selection_ack_changed
                || index_status_changed
            {
                cx.notify();
            }
        });

        let history_panel_focus_handle = cx.focus_handle().tab_index(0).tab_stop(false);
        let default_design_widths = default_history_column_design_widths();
        let scale = ui_scale::UiScale::from_percent(ui_scale_percent);
        let default_widths = scaled_history_column_widths(default_design_widths, scale);

        let history_annotations = super::super::extension_host::registry(cx)
            .filter(|registry| !registry.history_annotators().is_empty())
            .map(|registry| annotations::HistoryAnnotations::new(registry.history_annotators()));
        Self {
            history_annotations,
            store: store.into(),
            state,
            theme,
            ui_scale_percent,
            appearance_metrics: crate::appearance::current(cx),
            date_time_format,
            timezone,
            show_timezone,
            history_relative_dates,
            history_highlight_commit_chain,
            _ui_model_subscription: subscription,
            root_view,
            tooltip_host,
            row_hover: None,
            row_hover_scroll_position: 0.0,
            row_hover_tooltip: None,
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
            history_branch_names,
            history_show_graph,
            history_show_graph_gradient,
            history_show_author,
            history_show_date,
            history_show_sha,
            history_show_tags,
            history_auto_fetch_tags_on_repo_activation,
            history_col_resize: None,
            history_cache: None,
            pending_history_cache: None,
            history_selected_list_index_cache: None,
            selected_branch: None,
            pending_history_reveal: None,
            last_browse_commit: None,
            history_worktree_summary_cache: None,
            history_list_plan_cache: None,
            presented_history: None,
            scroll_interaction: Default::default(),
            indexed: Default::default(),
            loading: Default::default(),
            history_selected_lane_color_cache: None,
            history_stash_ids_cache: None,
            history_scroll: UniformListScrollHandle::default(),
            history_panel_focus_handle,
            relative_time_tick: None,
            signature_viewport: None,
            signature_debounce: None,
            find: None,
            pending_history_selections: Default::default(),
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
        self.state.repos.iter().find(|r| r.id == repo_id)?;
        let cache = self
            .history_cache
            .as_ref()
            .filter(|cache| cache.base.request.repo_id == repo_id)?;
        Some(
            cache
                .base
                .visible_indices
                .iter()
                .filter_map(|ix| cache.page.commits.get(ix).map(|c| c.id.clone()))
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
        cx: &mut gpui::Context<Self>,
    ) {
        let _ = self.root_view.update(cx, |root, cx| {
            root.show_history_refs_hover(repo_id, commit_id, source_bounds, items, pointer, cx);
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
            // The cache owns the Arc, so the source's address cannot be reused
            // while this key is live. A mutation must copy a shared page first.
            log_source: std::ptr::from_ref(page) as usize,
            history_author_filter: repo.history_state.history_author_filter.clone(),
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
        history_scale(self.ui_scale_percent).with_appearance(self.appearance_metrics)
    }

    fn sync_history_column_widths_from_design(&mut self) {
        let scale = self.ui_scale();
        self.history_col_branch = scale.px(self.history_col_branch_design);
        self.history_col_graph = scale
            .px(self.history_col_graph_design)
            .max(crate::view::rows::history_node::graph_min_width(scale));
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
        self.cancel_history_find_navigation();
        let request_id = self.note_history_selection(repo_id, HistoryPrimarySelection::WorkingTree);
        self.store.dispatch(Msg::ClearCommitSelection {
            request_id: Some(request_id),
            repo_id,
        });
        let keep_file_view = self.state.file_browser_settings.follow_selected_commit
            && self.state.sidebar_mode == gitcomet_state::model::SidebarMode::Files
            && self
                .active_repo()
                .is_some_and(|repo| repo.file_browser.active && repo.diff_state.content_preview);
        if !keep_file_view {
            self.store.dispatch(Msg::ClearDiffSelection { repo_id });
        }
        self.dismiss_history_refs_hover(cx);
        if let Some(logical) = &mut self.scroll_interaction.borrow_mut().logical {
            logical.set_position(0.0);
        } else {
            self.history_scroll
                .scroll_to_item_strict(0, gpui::ScrollStrategy::Center);
        }
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
            request_id: None,
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
        target: BranchMenuTarget,
        cx: &mut gpui::Context<Self>,
    ) {
        let next = Some(SelectedBranch { repo_id, target });
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
            branch_w: self.history_ref_column_width(),
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
            self.history_branch_names,
            self.history_show_graph,
            (
                self.history_show_author,
                self.history_show_date,
                self.history_show_sha,
            ),
            self.ui_scale(),
        );
        self.history_col_branch = widths.branch;
        self.history_col_graph = widths.graph;
        self.history_col_author = widths.author;
        self.history_col_date = widths.date;
        self.history_col_sha = widths.sha;
        self.sync_history_column_design_widths_from_pixels();
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

    pub(in super::super) fn set_appearance_metrics(
        &mut self,
        metrics: crate::appearance::Appearance,
        cx: &mut gpui::Context<Self>,
    ) {
        self.appearance_metrics = metrics;
        self.history_col_resize = None;
        self.sync_history_column_widths_from_design();
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

    pub(in super::super) fn set_history_show_graph_gradient(
        &mut self,
        enabled: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_show_graph_gradient == enabled {
            return;
        }
        self.history_show_graph_gradient = enabled;
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

    pub(in super::super) fn history_ref_column_width(&self) -> Pixels {
        match self.history_branch_names {
            HistoryBranchNamesMode::SeparateColumn => self.history_col_branch,
            HistoryBranchNamesMode::Inline => px(0.0),
        }
    }

    pub(in super::super) fn set_history_branch_names(
        &mut self,
        next: HistoryBranchNamesMode,
        cx: &mut gpui::Context<Self>,
    ) {
        if self.history_branch_names == next {
            return;
        }
        self.history_branch_names = next;
        self.history_col_resize = None;
        self.update_history_row_hover(None, None, cx);
        cx.notify();
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
        self.update_history_row_hover(None, None, cx);
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
        kind: impl Into<PopoverRequest>,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
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

    /// Opens an extension's `items` as a menu at `anchor`, the control
    /// `invoker` names showing its menu open, as the host's own menus do.
    pub(in crate::view) fn open_hosted_menu_at(
        &mut self,
        items: Vec<gitcomet_extension_api::HostedMenuItem>,
        invoker: SharedString,
        anchor: Point<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let id = super::super::extension_host::next_dialog_id();
        let root_view = self.root_view.clone();
        let window_handle = window.window_handle();
        cx.defer(move |cx| {
            let _ = window_handle.update(cx, |_, window, cx| {
                let _ = root_view.update(cx, |root, cx| {
                    root.popover_host
                        .update(cx, |host, _| host.set_hosted_menu(id, items));
                    root.open_popover_at(
                        PopoverKind::Hosted { id, menu: true }.invoked_by(invoker),
                        anchor,
                        window,
                        cx,
                    );
                });
            });
        });
    }

    pub(in super::super) fn open_popover_for_bounds(
        &mut self,
        kind: impl Into<PopoverRequest>,
        anchor_bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut gpui::Context<Self>,
    ) {
        let kind: PopoverRequest = kind.into();
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

    pub(in crate::view) fn drive_pending_history_reveal(&mut self, cx: &mut gpui::Context<Self>) {
        if self.reveal_indexed(cx) {
            return;
        }
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
                        request_id: None,
                        repo_id: pending.repo_id,
                        path: path.clone(),
                    });
                }
            }
            (None, Some(commit_id)) => self.store.dispatch(Msg::SelectCommit {
                request_id: None,
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
        } else if decision.load_more
            && self.active_repo().is_none_or(|repo| {
                !repo.history_state.indexed.loading && repo.history_state.indexed.index.is_none()
            })
        {
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
        if self.indexed.presentation.is_some() {
            return self
                .indexed
                .window
                .as_ref()
                .and_then(|window| window.selected_lane);
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

        let cache = if self.indexed.presentation.is_some() {
            self.indexed.window.as_ref().map(|window| &window.cache)
        } else {
            self.history_cache.as_ref()
        }
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
        if self.indexed.presentation.is_some() {
            return self.indexed.plan.clone();
        }
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

        if self.pending_history_cache.as_ref().is_some_and(|cache| {
            cache.base.request == base_request && cache.decorations.request == decoration_request
        }) {
            return;
        }
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

                    HistoryCache {
                        page,
                        base,
                        decorations,
                    }
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

                    let current_matches = this
                        .active_repo()
                        .and_then(|repo| {
                            let page = Self::display_log_page_for_repo(repo)?;
                            Some(
                                this.history_base_cache_request_for_repo(repo, &page)
                                    == request_for_update.base_request
                                    && this.history_decoration_cache_request_for_repo(repo, &page)
                                        == request_for_update.decoration_request,
                            )
                        })
                        .unwrap_or(false);
                    if !current_matches {
                        this.history_cache_inflight = None;
                        cx.notify();
                        return;
                    }

                    this.history_cache_inflight = None;
                    this.replace_history_cache(rebuild);
                    cx.notify();
                });
            },
        )
        .detach();
    }

    fn replace_history_cache(&mut self, rebuild: HistoryCache) {
        // Keep input handlers on the source that was actually displayed until
        // render can install the page, its row plan, and its viewport together.
        self.pending_history_cache = Some(rebuild);
    }

    fn apply_pending_history_cache(&mut self) {
        if self.scroll_interaction.borrow().dragging {
            return;
        }
        let Some(rebuild) = self.pending_history_cache.take() else {
            return;
        };
        let current_matches = self
            .active_repo()
            .and_then(|repo| {
                let page = Self::display_log_page_for_repo(repo)?;
                Some(
                    self.history_base_cache_request_for_repo(repo, &page) == rebuild.base.request
                        && self.history_decoration_cache_request_for_repo(repo, &page)
                            == rebuild.decorations.request,
                )
            })
            .unwrap_or(false);
        if current_matches {
            self.history_cache = Some(rebuild);
        }
    }
}

#[cfg(test)]
mod tests;
