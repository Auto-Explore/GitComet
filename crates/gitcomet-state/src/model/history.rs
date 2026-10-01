//! Commit history state and the `RepoState` methods that maintain it.

use super::{CommitSignatureMap, Loadable, RepoState, Shared};
use gitcomet_core::domain::*;
use rustc_hash::FxHashSet;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct HistoryState {
    pub indexed: crate::indexed_history::IndexedHistoryState,
    pub authors: crate::history_authors::HistoryAuthorsState,
    pub find: crate::history_find::HistoryFindState,
    pub history_scope: LogScope,
    /// Case-insensitive author filter for the history, or `None` for all
    /// authors. Matches the author name shown in the UI.
    pub history_author_filter: Option<String>,
    pub log: Loadable<Shared<LogPage>>,
    pub retained_log_while_loading: Option<Shared<LogPage>>,
    pub log_loading_more: bool,
    /// Identity of the exact Git inputs behind the loaded page.
    pub log_snapshot: Option<gitcomet_core::services::HistorySnapshot>,
    /// Commits visited by the streaming initial walk, retained for diagnostics.
    /// `None` once the page is complete. Updating this does not rebuild rows.
    pub log_scan_progress: Option<u64>,
    pub log_rev: u64,
    pub file_history_path: Option<PathBuf>,
    pub file_history: Loadable<Shared<LogPage>>,
    pub selected_commit: Option<CommitId>,
    pub selected_commit_rev: u64,
    /// Last processed UI selection request, even when it did not change focus.
    /// Automatic store reconciliation leaves this acknowledgment unchanged.
    pub selection_ack: Option<u64>,
    /// The commit a "reveal in history" is currently walking toward.
    ///
    /// It is selected the moment the reveal starts, before the log has paged far
    /// enough to contain its row, so page reconciliation has to be told not to
    /// mistake "not loaded yet" for "no longer exists".
    pub reveal_target: Option<CommitId>,
    pub commit_details: Loadable<Shared<CommitDetails>>,
    pub commit_details_rev: u64,
    /// Signature verdicts by commit, shared by the details pane and the
    /// history rows. Only badge-worthy commits appear: absent means no badge.
    /// Behind `Arc` because `AppState` is deep-copied on every dispatch.
    pub commit_signatures: Shared<CommitSignatureMap>,
    pub commit_signatures_rev: u64,
    /// Invalidates batches started before a refresh or preference change.
    pub commit_signatures_epoch: u64,
    /// Bounded memo of started attempts, including completed no-badge results.
    pub(crate) commit_signatures_requested: Shared<FxHashSet<CommitId>>,
    pub(crate) commit_signatures_attempt_order: Shared<VecDeque<CommitId>>,
    pub(crate) commit_signatures_visible: Shared<[CommitId]>,
    pub(crate) commit_signatures_queue: VecDeque<Shared<[CommitId]>>,
    pub(crate) commit_signatures_in_flight: bool,
    pub(crate) commit_signatures_batch: u64,
    pub(crate) commit_signatures_cancellation: gitcomet_core::services::CancellationToken,
    pub multi_selection: CommitMultiSelection,
    selected_ids: Arc<FxHashSet<CommitId>>,
    squash_cache: Option<Arc<HistorySquashCache>>,
    /// Active "compare two points" selection: when two commits are selected (or
    /// a mark/compare pair is chosen), this holds the ordered `from`/`to` pair
    /// and the changed-file list between them. `None` when no comparison is
    /// active. The per-file and whole-range diffs render through the normal
    /// `DiffState` pipeline via a `DiffTarget::CommitRange`.
    pub range_selection: Option<RangeSelection>,
    /// Path of the linked worktree whose uncommitted changes the history row
    /// selection is on, if any. A third kind of selection alongside a commit and
    /// a range; the details pane branches on it.
    pub worktree_selection: Option<PathBuf>,
    pub worktree_selection_rev: u64,
    pub range_files: Loadable<Shared<Vec<CommitFileChange>>>,
    pub range_files_rev: u64,
    /// Monotonic id of the newest issued range-file load. A reply carrying an
    /// older id is dropped, so out-of-order completions cannot overwrite a
    /// newer list. The `(from, to)` pair alone cannot decide this: a
    /// commit↔working-tree comparison keeps the same pair across every
    /// refresh, so every reply would look current.
    pub range_files_request: u64,
    /// A range-file load is outstanding. Refreshes raised while it runs are
    /// folded into `range_files_refresh_queued` rather than each spawning
    /// their own pair of full-tree `git diff` calls.
    pub range_files_in_flight: bool,
    /// The worktree moved again while a load was in flight; re-run once it
    /// lands, so the list still ends up describing the final state.
    pub range_files_refresh_queued: bool,
    pub squash_preview: Loadable<SquashPreview>,
    pub squash_preview_rev: u64,
    /// The `(oldest, head)` range whose message preview is currently being
    /// loaded. Lets a returning preview result be accepted even if the squash
    /// plan is transiently invalid (e.g. HEAD momentarily unresolved during a
    /// concurrent reload), as long as the range still matches what was asked.
    pub squash_preview_pending: Option<(CommitId, CommitId)>,
    /// The Reveal Commit dialog's current reference lookup. Preview only: it
    /// never selects anything, so typing in the dialog cannot move the main
    /// view the way `reveal_target` does.
    ///
    /// Carries no `_rev` counterpart because no pane fingerprints it: the
    /// dialog is its own entity and repaints itself when this changes.
    pub commit_lookup: CommitLookup,
    /// Parents for the open cherry-pick/revert confirmation; see
    /// [`CommitLookupPurpose`].
    pub mainline_lookup: CommitLookup,
}

/// Which dialog a commit lookup answers. They resolve different references at
/// the same time, so each owns its slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitLookupPurpose {
    /// The Reveal Commit dialog's preview row.
    RevealDialog,
    /// The parent list the cherry-pick/revert confirmations pick a mainline from.
    MainlineParents,
}

/// A resolved-or-failed answer to "what commit does this reference name?".
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommitLookup {
    /// Monotonic id of the newest issued lookup. A reply carrying an older id
    /// is dropped, so an out-of-order completion cannot overwrite a newer
    /// answer — the same guard `range_files_request` uses.
    pub request: u64,
    /// The reference `result` answers, so a caller can tell whether the answer
    /// is about what the user has typed *now*.
    pub reference: Option<CommitId>,
    pub result: Loadable<Commit>,
}

impl Default for CommitLookup {
    fn default() -> Self {
        Self {
            request: 0,
            reference: None,
            result: Loadable::NotLoaded,
        }
    }
}

#[derive(Clone, Debug)]
struct HistorySquashCache {
    key: (usize, u64, u64, u64, Option<CommitId>, usize),
    // Pin identities used by the cache key across asynchronous snapshots.
    _selection: Arc<Vec<CommitId>>,
    _index: Option<gitcomet_core::history_index::HistoryIndexHandle>,
    plan: Option<gitcomet_core::squash::SquashPlan>,
}

impl HistoryState {
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn signature_targets_for_test(&self) -> &Shared<[CommitId]> {
        &self.commit_signatures_visible
    }

    pub fn selection_contains(&self, id: &CommitId) -> bool {
        if self.selected_ids.len() == self.multi_selection.commits.len() {
            self.selected_ids.contains(id)
        } else {
            self.multi_selection.contains(id)
        }
    }
}

impl Default for HistoryState {
    fn default() -> Self {
        Self {
            indexed: Default::default(),
            authors: Default::default(),
            find: Default::default(),
            history_scope: LogScope::default(),
            history_author_filter: None,
            log: Loadable::NotLoaded,
            retained_log_while_loading: None,
            log_loading_more: false,
            log_scan_progress: None,
            log_snapshot: None,
            log_rev: 0,
            file_history_path: None,
            file_history: Loadable::NotLoaded,
            selected_commit: None,
            selected_commit_rev: 0,
            selection_ack: None,
            reveal_target: None,
            commit_details: Loadable::NotLoaded,
            commit_details_rev: 0,
            commit_signatures: Shared::default(),
            commit_signatures_rev: 0,
            commit_signatures_epoch: 0,
            commit_signatures_requested: Shared::default(),
            commit_signatures_attempt_order: Shared::default(),
            commit_signatures_visible: Shared::default(),
            commit_signatures_queue: VecDeque::new(),
            commit_signatures_in_flight: false,
            commit_signatures_batch: 0,
            commit_signatures_cancellation: Default::default(),
            multi_selection: CommitMultiSelection::default(),
            selected_ids: Arc::new(FxHashSet::default()),
            squash_cache: None,
            range_selection: None,
            worktree_selection: None,
            worktree_selection_rev: 0,
            range_files: Loadable::NotLoaded,
            range_files_rev: 0,
            range_files_request: 0,
            range_files_in_flight: false,
            range_files_refresh_queued: false,
            squash_preview: Loadable::NotLoaded,
            squash_preview_rev: 0,
            squash_preview_pending: None,
            commit_lookup: CommitLookup::default(),
            mainline_lookup: CommitLookup::default(),
        }
    }
}

/// Multi-selected commits in the history view. `commits` always mirrors the
/// selection (a plain single-select stores one id here); only `len() > 1`
/// switches the UI into multi-selection presentation. The anchor is the
/// origin for shift-click ranges; `anchor_index`/`anchor_log_rev` are a
/// resolution hint trusted only while the log revision is unchanged.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CommitMultiSelection {
    pub commits: Arc<Vec<CommitId>>,
    pub anchor: Option<CommitId>,
    pub anchor_index: Option<usize>,
    pub anchor_log_rev: Option<u64>,
}

impl CommitMultiSelection {
    pub fn is_multi(&self) -> bool {
        self.commits.len() > 1
    }

    pub fn contains(&self, id: &CommitId) -> bool {
        self.commits.iter().any(|c| c == id)
    }

    /// Predicts focus and membership for a history input without scheduling loads.
    /// Shared by the reducer and the UI while selection replies are in flight.
    pub fn select(
        &self,
        commit_id: CommitId,
        mode: crate::msg::CommitSelectMode,
        clicked_index: Option<usize>,
        mut visible_order: Option<Vec<CommitId>>,
        log_rev: u64,
    ) -> (Self, Option<CommitId>) {
        let mut sel = self.clone();
        let focus = match mode {
            crate::msg::CommitSelectMode::Single => {
                collapse_multi_selection_to(&mut sel, commit_id.clone(), clicked_index, log_rev);
                commit_id
            }
            crate::msg::CommitSelectMode::Toggle => {
                if let Some(ix) = sel.commits.iter().position(|c| *c == commit_id) {
                    Arc::make_mut(&mut sel.commits).remove(ix);
                    let Some(focus) = sel.commits.last().cloned() else {
                        // Toggled the last commit away: clear the selection
                        // entirely (also dissolves the multi-selection).
                        return (Self::default(), None);
                    };
                    focus
                } else {
                    Arc::make_mut(&mut sel.commits).push(commit_id.clone());
                    sel.anchor = Some(commit_id.clone());
                    sel.anchor_index = clicked_index;
                    sel.anchor_log_rev = Some(log_rev);
                    commit_id
                }
            }
            crate::msg::CommitSelectMode::Range => {
                let entries = visible_order.as_deref().unwrap_or(&[]);
                let clicked_ix = commit_selection_entry_index(entries, &commit_id, clicked_index);
                match clicked_ix {
                    None => {
                        collapse_multi_selection_to(
                            &mut sel,
                            commit_id.clone(),
                            clicked_index,
                            log_rev,
                        );
                    }
                    Some(clicked_ix) => {
                        let anchor_ix = sel
                            .anchor
                            .as_ref()
                            .and_then(|anchor| {
                                let trusted_hint = sel
                                    .anchor_index
                                    .filter(|_| sel.anchor_log_rev == Some(log_rev));
                                commit_selection_entry_index(entries, anchor, trusted_hint)
                            })
                            .unwrap_or(clicked_ix);
                        let (a, b) = if anchor_ix <= clicked_ix {
                            (anchor_ix, clicked_ix)
                        } else {
                            (clicked_ix, anchor_ix)
                        };
                        sel.commits = Arc::new(if a == 0 && b + 1 == entries.len() {
                            visible_order.take().unwrap()
                        } else {
                            entries[a..=b].to_vec()
                        });
                        if sel.anchor.is_none() {
                            sel.anchor = Some(commit_id.clone());
                        }
                        sel.anchor_index = Some(anchor_ix);
                        sel.anchor_log_rev = Some(log_rev);
                    }
                }
                commit_id
            }
            crate::msg::CommitSelectMode::PreserveIfSelected => {
                // Keep an existing multi-selection intact when the clicked commit
                // is already part of it — only the focus moves. Otherwise collapse
                // to the clicked commit like a plain click.
                if !sel.commits.contains(&commit_id) {
                    collapse_multi_selection_to(
                        &mut sel,
                        commit_id.clone(),
                        clicked_index,
                        log_rev,
                    );
                }
                commit_id
            }
        };

        (sel, Some(focus))
    }
}

fn collapse_multi_selection_to(
    sel: &mut CommitMultiSelection,
    commit_id: CommitId,
    clicked_index: Option<usize>,
    log_rev: u64,
) {
    sel.commits = Arc::new(vec![commit_id.clone()]);
    sel.anchor = Some(commit_id);
    sel.anchor_index = clicked_index;
    sel.anchor_log_rev = Some(log_rev);
}

/// Resolves `target`'s index in `entries`, preferring the index hint when it
/// still points at the target.
fn commit_selection_entry_index(
    entries: &[CommitId],
    target: &CommitId,
    index_hint: Option<usize>,
) -> Option<usize> {
    index_hint
        .filter(|&ix| entries.get(ix) == Some(target))
        .or_else(|| entries.iter().position(|id| id == target))
}

/// A "compare two points" selection. `from` is the base/older side and `to`
/// the newer side, so `git diff from to` reads as "what `to` adds". A `to` of
/// `None` compares `from` against the live working tree. The labels are what the
/// UI shows (short shas for commits, ref names for branches/tags, "Working
/// tree" for the worktree tip).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RangeSelection {
    pub from: CommitId,
    pub to: Option<CommitId>,
    pub from_label: String,
    pub to_label: String,
    /// How the comparison measures; direct unless asked otherwise.
    pub options: gitcomet_core::services::ComparisonOptions,
    /// The commit the loaded list was measured from: the merge base of a
    /// merge-base comparison. `None` until the list loads.
    pub base: Option<CommitId>,
}

impl RangeSelection {
    /// The requested comparison, independent of its asynchronously loaded base.
    pub fn same_comparison(&self, other: &Self) -> bool {
        self.from == other.from
            && self.to == other.to
            && self.from_label == other.from_label
            && self.to_label == other.to_label
            && self.options == other.options
    }

    pub fn new(from: CommitId, to: Option<CommitId>, from_label: String, to_label: String) -> Self {
        Self {
            from,
            to,
            from_label,
            to_label,
            options: Default::default(),
            base: None,
        }
    }

    /// Where a file's diff in this comparison starts: the resolved base once
    /// known, else `from`.
    pub fn diff_from(&self) -> &CommitId {
        self.base.as_ref().unwrap_or(&self.from)
    }
}

/// Backend-built default message for the squash confirmation prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SquashPreview {
    pub oldest: CommitId,
    pub head: CommitId,
    /// Single-line subject, split from the combined message by core.
    pub subject: String,
    /// Message body (everything after the subject line), possibly empty.
    pub body: String,
}

impl RepoState {
    #[inline]
    pub(crate) fn bump_log_revs(&mut self) {
        self.log_rev = self.log_rev.wrapping_add(1);
        self.history_state.log_rev = self.history_state.log_rev.wrapping_add(1);
    }

    pub(crate) fn set_log(&mut self, log: Loadable<Shared<LogPage>>) {
        if self.history_state.log == log && self.log == log {
            return;
        }
        if !matches!(log, Loadable::Loading) {
            self.history_state.retained_log_while_loading = None;
        }
        // A caller accepting a backend result installs its snapshot after this
        // write. Reload and filter transitions must never reuse the old token.
        self.history_state.log_snapshot = None;
        self.history_state.log = log.clone();
        self.log = log;
        self.bump_log_revs();
    }

    pub(crate) fn retain_log_while_loading(&mut self) {
        if self.history_state.retained_log_while_loading.is_some() {
            return;
        }

        if let Loadable::Ready(page) = &self.log {
            self.history_state.retained_log_while_loading = Some(Arc::clone(page));
        }
    }

    /// Shows a partially built page while the walk building it keeps running,
    /// in place of whatever [`Self::retain_log_while_loading`] was holding —
    /// which, when a filter has just changed, is the rows the user is trying to
    /// get away from.
    ///
    /// Deliberately not `set_log(Ready)`: the page is not finished, and a
    /// `Ready` page whose `next_cursor` is `None` is indistinguishable from a
    /// complete history with nothing more to load. Only meaningful while the log
    /// is `Loading`; `set_log` drops the retained page once the walk finishes.
    pub(crate) fn set_partial_log_while_loading(&mut self, page: Shared<LogPage>) {
        if !matches!(self.log, Loadable::Loading) {
            return;
        }
        self.history_state.retained_log_while_loading = Some(page);
        self.bump_log_revs();
    }

    /// Hold on to the currently loaded annotations so the blame column keeps
    /// painting them while the same target reloads, instead of blanking out.
    /// Only valid while `blame_path`/`blame_source` still describe them —
    /// callers that re-target blame must call [`Self::clear_retained_blame`].
    pub(crate) fn retain_blame_while_loading(&mut self) {
        if self.diff_state.retained_blame_while_loading.is_some() {
            return;
        }

        if let Loadable::Ready(lines) = &self.diff_state.blame {
            self.diff_state.retained_blame_while_loading = Some(Arc::clone(lines));
        }
    }

    pub(crate) fn clear_retained_blame(&mut self) {
        self.diff_state.retained_blame_while_loading = None;
    }

    pub(crate) fn set_log_loading_more(&mut self, v: bool) {
        if self.history_state.log_loading_more == v && self.log_loading_more == v {
            return;
        }
        self.history_state.log_loading_more = v;
        self.log_loading_more = v;
        self.bump_log_revs();
    }

    /// Records how far a running walk has scanned, or clears it when the page
    /// is complete. Deliberately does not bump `log_rev`: the log itself has not
    /// changed, and the rows must not be rebuilt just to move a counter.
    pub(crate) fn set_log_scan_progress(&mut self, scanned: Option<u64>) {
        self.history_state.log_scan_progress = scanned;
    }

    pub(crate) fn set_log_scope(&mut self, scope: LogScope) {
        if self.history_state.history_scope == scope {
            return;
        }
        self.history_state.indexed.reset_query();
        self.history_state.history_scope = scope;
        self.history_state.authors.cancellation.cancel();
        self.bump_log_revs();
    }

    pub(crate) fn set_history_author_filter(&mut self, author: Option<String>) {
        if self.history_state.history_author_filter == author {
            return;
        }
        self.history_state.indexed.reset_query();
        self.history_state.history_author_filter = author;
        self.bump_log_revs();
    }

    pub(crate) fn set_reveal_target(&mut self, v: Option<CommitId>) {
        self.history_state.reveal_target = v;
    }

    /// Start a new Reveal Commit lookup, returning the request id the reply has
    /// to carry to be accepted.
    pub(crate) fn commit_lookup_mut(&mut self, purpose: CommitLookupPurpose) -> &mut CommitLookup {
        match purpose {
            CommitLookupPurpose::RevealDialog => &mut self.history_state.commit_lookup,
            CommitLookupPurpose::MainlineParents => &mut self.history_state.mainline_lookup,
        }
    }

    pub(crate) fn begin_commit_lookup(
        &mut self,
        purpose: CommitLookupPurpose,
        reference: CommitId,
    ) -> u64 {
        let lookup = self.commit_lookup_mut(purpose);
        lookup.request = lookup.request.wrapping_add(1);
        lookup.reference = Some(reference);
        lookup.result = Loadable::Loading;
        lookup.request
    }

    /// Record a lookup reply, ignoring one that a newer lookup has overtaken.
    pub(crate) fn finish_commit_lookup(
        &mut self,
        purpose: CommitLookupPurpose,
        request: u64,
        result: Loadable<Commit>,
    ) {
        let lookup = self.commit_lookup_mut(purpose);
        if lookup.request != request {
            return;
        }
        lookup.result = result;
    }

    /// Selecting a worktree row takes the details pane over, so the commit
    /// selection lets go first. Passing `None` simply clears it, which is what
    /// selecting a commit or the working-tree row ends up doing.
    pub(crate) fn set_worktree_selection(&mut self, path: Option<PathBuf>) {
        if self.history_state.worktree_selection == path {
            return;
        }
        if path.is_some() {
            // Clears `worktree_selection` as a side effect, hence the assignment
            // afterwards rather than before.
            self.set_selected_commit(None);
        }
        self.history_state.worktree_selection = path;
        self.history_state.worktree_selection_rev =
            self.history_state.worktree_selection_rev.wrapping_add(1);
    }

    pub(crate) fn set_selected_commit(&mut self, v: Option<CommitId>) {
        // Moving the commit selection at all -- including clearing it for the
        // working-tree row -- means the worktree row is no longer what is shown.
        if self.history_state.worktree_selection.take().is_some() {
            self.history_state.worktree_selection_rev =
                self.history_state.worktree_selection_rev.wrapping_add(1);
        }
        // Selecting anything other than the commit a reveal is walking toward
        // means the user moved on, and the reveal's exemption from page
        // reconciliation retires with it.
        if self.history_state.reveal_target != v {
            self.history_state.reveal_target = None;
        }
        if v.is_none() {
            // Clearing the selection always dissolves any multi-selection too;
            // every clear site (scope change, repo switch, diff selection)
            // relies on this. A range comparison is likewise a form of
            // selection, so it must dissolve here as well.
            self.history_state.multi_selection = CommitMultiSelection::default();
            self.history_state.selected_ids = Arc::new(FxHashSet::default());
            self.clear_range_comparison();
        }
        self.history_state.selected_commit = v;
        self.history_state.selected_commit_rev =
            self.history_state.selected_commit_rev.wrapping_add(1);
    }

    /// Leave comparison mode: drop the endpoints and the file list, and retire
    /// any load still in flight so its reply cannot repopulate the list of a
    /// comparison the user has already left. Returns whether there was anything
    /// to leave — a plain commit click runs through here on every selection, and
    /// bumping the revs for a comparison that was never active would invalidate
    /// the range-file row cache for nothing.
    pub(crate) fn clear_range_comparison(&mut self) -> bool {
        if self.history_state.range_selection.is_none() && !self.history_state.range_files_in_flight
        {
            return false;
        }
        self.set_range_selection(None);
        self.set_range_files(Loadable::NotLoaded);
        self.history_state.range_files_request =
            self.history_state.range_files_request.wrapping_add(1);
        self.history_state.range_files_in_flight = false;
        self.history_state.range_files_refresh_queued = false;
        true
    }

    /// Claim the next range-file load. Returns the request id to carry through
    /// the effect and back on the reply; anything older is stale by definition.
    pub(crate) fn begin_range_files_load(&mut self) -> u64 {
        self.history_state.range_files_request =
            self.history_state.range_files_request.wrapping_add(1);
        self.history_state.range_files_in_flight = true;
        self.history_state.range_files_refresh_queued = false;
        self.history_state.range_files_request
    }

    /// Raise a refresh of the current comparison's file list. `Some(request)`
    /// claims the load and must be issued; `None` means one is already running
    /// and this was folded into it, to be re-issued when that reply lands.
    ///
    /// Claiming and issuing are one call on purpose: a caller that decided to
    /// refresh but forgot to claim would leave `range_files_in_flight` false
    /// forever, and every later change would start its own pair of full-tree
    /// `git diff` calls.
    pub(crate) fn request_range_files_refresh(&mut self) -> Option<u64> {
        if self.history_state.range_files_in_flight {
            self.history_state.range_files_refresh_queued = true;
            return None;
        }
        Some(self.begin_range_files_load())
    }

    pub(crate) fn set_range_selection(&mut self, v: Option<RangeSelection>) {
        if self.history_state.range_selection == v {
            return;
        }
        self.history_state.range_selection = v;
        // The details pane keys its comparison-vs-single/multi decision off the
        // commit-selection revision, so bump it when the comparison changes.
        self.history_state.selected_commit_rev =
            self.history_state.selected_commit_rev.wrapping_add(1);
    }

    pub(crate) fn set_range_files(&mut self, v: Loadable<Shared<Vec<CommitFileChange>>>) {
        self.history_state.range_files = v;
        self.history_state.range_files_rev = self.history_state.range_files_rev.wrapping_add(1);
    }

    fn history_squash_key(&self) -> (usize, u64, u64, u64, Option<CommitId>, usize) {
        (
            Arc::as_ptr(&self.history_state.multi_selection.commits) as usize,
            self.log_rev,
            self.head_branch_rev,
            self.branches_rev,
            self.detached_head_commit.clone(),
            self.history_state
                .indexed
                .index
                .as_ref()
                .filter(|index| Some(&index.snapshot) == self.history_state.log_snapshot.as_ref())
                .map_or(0, |index| Arc::as_ptr(index) as usize),
        )
    }

    /// Called on the store worker before publication, once per selection/topology.
    pub(crate) fn prepare_history_squash_plan(&mut self) {
        if !self.history_state.multi_selection.is_multi() {
            self.history_state.squash_cache = None;
            return;
        }
        let key = self.history_squash_key();
        if self
            .history_state
            .squash_cache
            .as_ref()
            .is_some_and(|cache| cache.key == key)
        {
            return;
        }
        let plan = self.compute_history_squash_plan();
        self.history_state.squash_cache = Some(Arc::new(HistorySquashCache {
            key,
            _selection: self.history_state.multi_selection.commits.clone(),
            _index: self.history_state.indexed.index.clone(),
            plan,
        }));
    }

    pub fn history_squash_plan(&self) -> Option<gitcomet_core::squash::SquashPlan> {
        if let Some(cache) = &self.history_state.squash_cache
            && cache.key == self.history_squash_key()
        {
            return cache.plan.clone();
        }
        self.compute_history_squash_plan()
    }

    fn compute_history_squash_plan(&self) -> Option<gitcomet_core::squash::SquashPlan> {
        let head = self.head_commit_id()?;
        if let Some(index) = self
            .history_state
            .indexed
            .index
            .as_ref()
            .filter(|index| Some(&index.snapshot) == self.history_state.log_snapshot.as_ref())
        {
            return gitcomet_core::squash::squash_eligibility_indexed(
                index,
                &self.history_state.multi_selection.commits,
                &head,
            );
        }
        let Loadable::Ready(page) = &self.log else {
            return None;
        };
        gitcomet_core::squash::squash_eligibility(
            &page.commits,
            &self.history_state.multi_selection.commits,
            &head,
        )
    }

    pub(crate) fn set_commit_multi_selection(&mut self, v: CommitMultiSelection) {
        if self.history_state.multi_selection == v {
            return;
        }
        if !Arc::ptr_eq(&self.history_state.multi_selection.commits, &v.commits) {
            self.history_state.selected_ids = Arc::new(v.commits.iter().cloned().collect());
        }
        self.history_state.multi_selection = v;
        self.history_state.selected_commit_rev =
            self.history_state.selected_commit_rev.wrapping_add(1);
    }

    pub(crate) fn set_squash_preview(&mut self, v: Loadable<SquashPreview>) {
        self.history_state.squash_preview = v;
        self.history_state.squash_preview_rev =
            self.history_state.squash_preview_rev.wrapping_add(1);
    }

    pub(crate) fn set_commit_details(&mut self, v: Loadable<Shared<CommitDetails>>) {
        self.history_state.commit_details = v;
        self.history_state.commit_details_rev =
            self.history_state.commit_details_rev.wrapping_add(1);
    }

    /// Invalidates both verdicts and in-flight batches. Trust inputs can change
    /// independently of commit objects, so refreshes must recheck signed commits.
    pub(crate) fn clear_commit_signatures(&mut self) {
        self.history_state.commit_signatures_cancellation.cancel();
        self.history_state.commit_signatures_cancellation = Default::default();
        self.history_state.commit_signatures_requested = Shared::default();
        self.history_state.commit_signatures_attempt_order = Shared::default();
        self.history_state.commit_signatures_visible = Shared::default();
        self.history_state.commit_signatures_queue.clear();
        self.history_state.commit_signatures_in_flight = false;
        self.history_state.commit_signatures_epoch =
            self.history_state.commit_signatures_epoch.wrapping_add(1);
        self.history_state.commit_signatures = Shared::default();
        self.history_state.commit_signatures_rev =
            self.history_state.commit_signatures_rev.wrapping_add(1);
    }

    /// Merges batches from the current verification epoch without dropping
    /// verdicts for other pages or selected commits.
    pub(crate) fn merge_commit_signatures(&mut self, verified: Vec<(CommitId, CommitSignature)>) {
        let updates: Vec<_> = verified
            .into_iter()
            .filter(|(id, signature)| {
                self.history_state.commit_signatures_requested.contains(id)
                    && self.history_state.commit_signatures.get(id) != Some(signature)
            })
            .collect();
        if updates.is_empty() {
            return;
        }
        let map = Arc::make_mut(&mut self.history_state.commit_signatures);
        for (id, signature) in updates {
            map.insert(id, signature);
        }
        self.history_state.commit_signatures_rev =
            self.history_state.commit_signatures_rev.wrapping_add(1);
    }
}
