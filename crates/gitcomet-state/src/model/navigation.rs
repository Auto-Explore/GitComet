//! File browsing and back/forward navigation history.

use super::{Loadable, RangeSelection, RepoState};
use gitcomet_core::domain::*;
use rustc_hash::FxHashSet;
use std::path::PathBuf;
use std::sync::Arc;

// ── File browser ────────────────────────────────────────────────

/// The file preview that was open when the browse point moved, to re-target
/// once the new listing says whether the file exists there.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFileBrowserReopen {
    pub path: PathBuf,
    /// `diff_target_rev` at capture time; a later change means the user moved
    /// on and the re-open is dropped.
    pub diff_target_rev: u64,
}

#[derive(Clone, Debug)]
pub struct FileBrowserState {
    pub selection: crate::explorer::Selection,
    pub show_hidden: bool,
    pub show_ignored: bool,
    pub revealed_paths: FxHashSet<PathBuf>,
    /// Entered by "Start file browsing" and left by "Exit file browsing".
    /// Selecting the working-tree row changes the source without exiting.
    pub active: bool,
    pub source: FileSource,
    pub entries: Loadable<Arc<Vec<FileEntry>>>,
    pub expanded_dirs: FxHashSet<Arc<PathBuf>>,
    /// Subtrees to expand after their ignored descendants have been loaded.
    pub pending_recursive_expansions: FxHashSet<PathBuf>,
    pub search_query: String,
    pub file_browser_rev: u64,
    /// The rows on screen are not the current truth: the worktree moved under
    /// a listing nobody is looking at, or the browse point moved and the new
    /// listing is still on its way. Either way the rows stay up rather than
    /// flashing back to "Loading files...".
    pub stale: bool,
    /// `selected_commit_rev` the browse point was last synced to. `None` forces
    /// a sync on the next chance.
    pub followed_selection_rev: Option<u64>,
    pub pending_reopen: Option<PendingFileBrowserReopen>,
}

impl Default for FileBrowserState {
    fn default() -> Self {
        Self {
            selection: crate::explorer::Selection::default(),
            show_hidden: true,
            show_ignored: false,
            revealed_paths: FxHashSet::default(),
            active: false,
            source: FileSource::default(),
            entries: Loadable::NotLoaded,
            expanded_dirs: FxHashSet::default(),
            pending_recursive_expansions: FxHashSet::default(),
            search_query: String::new(),
            file_browser_rev: 0,
            stale: false,
            followed_selection_rev: None,
            pending_reopen: None,
        }
    }
}

impl FileBrowserState {
    pub(crate) fn cancel_pending_expansions(&mut self, path: &std::path::Path) {
        self.pending_recursive_expansions
            .retain(|root| !root.starts_with(path) && !path.starts_with(root));
    }

    pub(crate) fn set_active(&mut self, active: bool) {
        if self.active != active {
            self.active = active;
            self.bump_rev();
        }
    }

    pub fn bump_rev(&mut self) {
        self.file_browser_rev = self.file_browser_rev.wrapping_add(1);
    }

    pub fn needs_load(&self) -> bool {
        self.stale || matches!(self.entries, Loadable::NotLoaded | Loadable::Error(_))
    }
}

// ── Navigation history ──────────────────────────────────────────

/// Maximum number of entries remembered per back/forward navigation stack.
pub const NAV_HISTORY_CAP: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ViewNavDir {
    Back,
    Forward,
}

/// One opened file-content view, enough to replay it: the source revision,
/// the path, and where the commit renamed or copied it from, so the replayed
/// diff pairs the two sides again. (Working-tree previews use
/// [`FileSource::WorkingDirectory`].)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewHistoryEntry {
    pub source: FileSource,
    pub path: PathBuf,
    pub old_path: Option<PathBuf>,
}

/// A snapshot of the main content view for the broad, global navigation history
/// (the mouse back/forward stack). Captures everything that decides what the
/// main pane shows: the diff/file target (`None` = history log view), whether it
/// is a full-content preview, the selected commit, and any active two-point
/// comparison. Replaying a snapshot only restores view/selection state; it never
/// re-runs operations like a checkout.
#[derive(Clone, Debug, PartialEq)]
pub struct MainViewSnapshot {
    pub diff_target: Option<DiffTarget>,
    pub content_preview: bool,
    /// Whether the file was open in the editor rather than the read-only
    /// content view. Recorded so back/forward can step *into* and *out of* edit
    /// mode: without it, opening the editor on the file already on screen
    /// produced a snapshot identical to the read-only one and deduped away, so
    /// neither direction could cross that boundary.
    pub edit_mode: bool,
    pub selected_commit: Option<CommitId>,
    /// The comparison the details pane is showing, if any. Without this a
    /// back/forward step could neither reproduce a comparison nor leave one:
    /// the comparison view takes precedence over the commit-detail views, so a
    /// snapshot that omitted it would restore a target and selection that the
    /// pane never gets around to showing.
    pub range_selection: Option<RangeSelection>,
    /// The linked-worktree row whose uncommitted changes the details pane is
    /// showing, if any. A third kind of history selection alongside a commit and
    /// a comparison, and mutually exclusive with both -- each setter clears the
    /// others. Without it, selecting a worktree row reads as "selection cleared"
    /// and back/forward can neither leave the row nor return to it.
    pub worktree_selection: Option<PathBuf>,
}

/// Browser-style back/forward stack. `cursor` indexes the currently shown entry
/// within `entries`.
#[derive(Clone, Debug)]
pub struct NavStack<T> {
    pub entries: Vec<T>,
    pub cursor: usize,
}

impl<T> Default for NavStack<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
        }
    }
}

impl<T: Clone + PartialEq> NavStack<T> {
    /// Record a freshly visited entry. Drops any forward history, dedupes a
    /// repeat of the current entry, and caps the total length.
    pub fn record(&mut self, entry: T) {
        if self.entries.get(self.cursor) == Some(&entry) {
            return;
        }
        self.entries.truncate(self.cursor.saturating_add(1));
        self.entries.push(entry);
        if self.entries.len() > NAV_HISTORY_CAP {
            let overflow = self.entries.len() - NAV_HISTORY_CAP;
            self.entries.drain(0..overflow);
        }
        self.cursor = self.entries.len() - 1;
    }

    /// Keep the stack in sync with the currently displayed `cur` view.
    ///
    /// This is called after every reduce so the cursor never goes stale: when
    /// the view changed since the last entry, a `push` navigation appends it as
    /// a new destination (truncating any forward history), while a non-`push`
    /// (background) change rewrites the *live tail* entry in place — keeping
    /// back/forward consistent without recording a spurious step.
    ///
    /// When the cursor is parked on a historical entry (the user has navigated
    /// Back/Forward and is sitting mid-stack), a non-`push` change must not
    /// touch the saved stack at all: rewriting or truncating it there would
    /// silently drop forward history or corrupt a snapshot the user navigated
    /// to. The next user navigation branches cleanly from the current cursor.
    pub fn reconcile(&mut self, cur: T, push: bool) {
        if self.entries.get(self.cursor) == Some(&cur) {
            return;
        }
        if self.entries.is_empty() {
            self.entries.push(cur);
            self.cursor = 0;
            return;
        }
        if push {
            self.entries.truncate(self.cursor.saturating_add(1));
            self.entries.push(cur);
            if self.entries.len() > NAV_HISTORY_CAP {
                let overflow = self.entries.len() - NAV_HISTORY_CAP;
                self.entries.drain(0..overflow);
            }
            self.cursor = self.entries.len() - 1;
            return;
        }
        // Non-`push` (background) change. Only fold it into the live tail; when
        // parked mid-stack leave saved history untouched.
        if self.cursor + 1 < self.entries.len() {
            return;
        }
        self.replace_current(cur);
    }

    /// Replace the snapshot at the cursor without discarding forward history.
    ///
    /// Unlike a background [`Self::reconcile`] this is allowed while parked
    /// mid-stack. It is used when an external context change deliberately
    /// resets the view represented by the current entry, such as activating a
    /// repository tab at its live history tip.
    pub fn replace_current(&mut self, entry: T) {
        if self.entries.get(self.cursor) == Some(&entry) {
            return;
        }
        if self.entries.is_empty() {
            self.entries.push(entry);
            self.cursor = 0;
            return;
        }
        if self.cursor > 0 && self.entries.get(self.cursor - 1) == Some(&entry) {
            // Replacing this entry made it match the previous one. Remove only
            // the duplicate current entry so any forward history survives.
            self.entries.remove(self.cursor);
            self.cursor -= 1;
        } else {
            self.entries[self.cursor] = entry;
        }
    }

    /// Reset to an empty stack. Used when the repo's history becomes invalid
    /// (full reload / reopen): saved snapshots may reference commits or file
    /// revisions that no longer resolve, so back/forward must start fresh.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.cursor = 0;
    }

    /// Move the cursor one step in `dir` and return the entry to replay, or
    /// `None` if already at the corresponding end.
    pub fn step(&mut self, dir: ViewNavDir) -> Option<T> {
        match dir {
            ViewNavDir::Back if self.can_back() => self.cursor -= 1,
            ViewNavDir::Forward if self.can_forward() => self.cursor += 1,
            _ => return None,
        }
        self.entries.get(self.cursor).cloned()
    }

    /// Align the cursor with an entry restored by a *different* navigation
    /// stack. If `entry` is already present, move the cursor onto it without
    /// mutating the stack; otherwise record it as a fresh entry. Used so the
    /// in-viewer file-version history follows along when the global (mouse)
    /// back/forward navigation lands on a file-content view.
    pub fn seek_or_record(&mut self, entry: T) {
        match self.entries.iter().position(|e| *e == entry) {
            Some(idx) => self.cursor = idx,
            None => self.record(entry),
        }
    }

    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }
}

/// The three related navigation mechanisms owned by one repository.
#[derive(Clone, Debug, Default)]
pub struct RepoNavigationState {
    /// Commits browsed through the file browser during this session.
    pub browse_history: Vec<CommitId>,
    /// Back/forward history within the file viewer.
    pub view_history: NavStack<ViewHistoryEntry>,
    /// Back/forward history across the entire main content view.
    pub main_history: NavStack<MainViewSnapshot>,
    /// A commit, branch, or tag waiting to be compared with another target.
    pub comparison_mark: Option<ComparisonMark>,
}

/// A point marked for comparison via the "Mark for comparison" context-menu
/// action. `commit_id` is the resolved commit (branch/tag tips resolve to their
/// target); `label` is what the menu shows (short sha, branch, or tag name).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonMark {
    pub commit_id: CommitId,
    pub label: String,
}

impl RepoState {
    /// Snapshot the state that decides what the main content pane shows, for the
    /// global back/forward history.
    pub(crate) fn main_view_snapshot(&self) -> MainViewSnapshot {
        MainViewSnapshot {
            diff_target: self.diff_state.diff_target.clone(),
            content_preview: self.diff_state.content_preview,
            edit_mode: self.diff_state.edit_mode,
            selected_commit: self.history_state.selected_commit.clone(),
            range_selection: self.history_state.range_selection.clone(),
            worktree_selection: self.history_state.worktree_selection.clone(),
        }
    }

    /// Whether the current main view equals `other`, compared by borrow so the
    /// nav-history reconcile can skip cloning a `MainViewSnapshot` (which owns a
    /// `PathBuf`) on the common path where the view did not move.
    pub(crate) fn main_view_snapshot_matches(&self, other: &MainViewSnapshot) -> bool {
        self.diff_state.diff_target == other.diff_target
            && self.diff_state.content_preview == other.content_preview
            && self.diff_state.edit_mode == other.edit_mode
            && self.history_state.selected_commit == other.selected_commit
            && self.history_state.range_selection == other.range_selection
            && self.history_state.worktree_selection == other.worktree_selection
    }
}
