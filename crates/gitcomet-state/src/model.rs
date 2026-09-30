use crate::msg::RepoCommandKind;
use crate::msg::RepoPath;
use crate::session;
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictSession, ConflictStageParts, canonicalize_stage_parts,
};
use gitcomet_core::domain::*;
use gitcomet_core::git_operation::{GitOperationId, GitOutputStream, HookExecutionId};
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{
    BlameLine, ForcePushLease, InteractiveRebaseEntry, SafePushAfterCommitContext, SequencerState,
    SubmoduleTrustTarget,
};
use gitcomet_core::signing_tools::SigningToolsState;
use gitcomet_core::text_format::{TextAttributes, TextEncoding, TextOverride};
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

mod signature_map;
pub use signature_map::CommitSignatureMap;

pub type Shared<T> = Arc<T>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SidebarDataRequest {
    pub worktrees: bool,
    pub submodules: bool,
    pub stashes: bool,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SidebarMode {
    #[default]
    Branches,
    Files,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum GitLogTagFetchMode {
    #[default]
    OnRepositoryActivation,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitLogSettings {
    pub show_history_tags: bool,
    pub tag_fetch_mode: GitLogTagFetchMode,
    /// Escape hatch: a misconfigured `gpg.program` or a wedged `gpg-agent`
    /// would otherwise slow every history page with no way to turn it off.
    pub verify_commit_signatures: bool,
}

impl Default for GitLogSettings {
    fn default() -> Self {
        Self {
            show_history_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::OnRepositoryActivation,
            verify_commit_signatures: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum DefaultTagType {
    #[default]
    Lightweight,
    Annotated,
}

impl GitLogSettings {
    pub fn auto_fetch_tags_on_repo_activation(self) -> bool {
        matches!(
            self.tag_fetch_mode,
            GitLogTagFetchMode::OnRepositoryActivation
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RemoteSettings {
    pub prune_deleted_remote_branches_on_fetch: bool,
}

impl Default for RemoteSettings {
    fn default() -> Self {
        Self {
            prune_deleted_remote_branches_on_fetch: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileBrowserSettings {
    /// Active file browsing follows the selected history row.
    pub follow_selected_commit: bool,
}

impl Default for FileBrowserSettings {
    fn default() -> Self {
        Self {
            follow_selected_commit: true,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoLoadsInFlight {
    in_flight: u32,
    pending: u32,
    pending_log: Option<PendingLogLoad>,
    /// The log walk that is actually running, so replies from one a newer
    /// request superseded can be told apart from the current one.
    active_log: Option<(LogLoadSeq, PendingLogLoad)>,
    last_log_seq: LogLoadSeq,
    line_stats_generation: LineStatsGeneration,
    active_line_stats: Option<LineStatsGeneration>,
    line_stats_requested: bool,
}

/// Identifies one dispatched log walk. Handed out by
/// [`RepoLoadsInFlight::request_log`] and carried by the effect and its replies,
/// so a reply is matched to the request that started it and nothing else.
///
/// A walk cannot be identified by what it asks for: switching the filter away
/// and back leaves the second request looking exactly like the first, and the
/// first walk's reply would then be taken for the second's — clearing the
/// bookkeeping while the walk it belongs to is still running.
pub type LogLoadSeq = u64;

/// Advances on invalidation, even when the set of changed paths is unchanged.
pub type LineStatsGeneration = u64;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLogLoad {
    pub scope: LogScope,
    pub author: Option<String>,
    pub limit: usize,
    pub cursor: Option<LogCursor>,
}

impl RepoLoadsInFlight {
    pub const HEAD_BRANCH: u32 = 1 << 0;
    pub const UPSTREAM_DIVERGENCE: u32 = 1 << 1;
    pub const BRANCHES: u32 = 1 << 2;
    pub const TAGS: u32 = 1 << 3;
    pub const REMOTES: u32 = 1 << 4;
    pub const REMOTE_BRANCHES: u32 = 1 << 5;
    pub const WORKTREE_STATUS: u32 = 1 << 6;
    pub const STAGED_STATUS: u32 = 1 << 7;
    pub const STASHES: u32 = 1 << 8;
    pub const REFLOG: u32 = 1 << 9;
    pub const REBASE_STATE: u32 = 1 << 10;
    pub const LOG: u32 = 1 << 11;
    pub const MERGE_COMMIT_MESSAGE: u32 = 1 << 12;
    pub const REMOTE_TAGS: u32 = 1 << 13;
    pub const WORKTREES: u32 = 1 << 14;
    pub const SUBMODULES: u32 = 1 << 15;
    pub const REF_METADATA: u32 = 1 << 16;
    pub const WORKTREE_DIRTY: u32 = 1 << 17;
    /// Deliberately outside `PRIMARY_REFRESH_FLAGS`: the live listing is a
    /// worktree walk, far costlier than the other loads.
    pub const FILE_BROWSER: u32 = 1 << 18;
    /// Also outside `PRIMARY_REFRESH_FLAGS`: counting reads both sides of every
    /// changed file, which a stat-only status walk avoids. Kept separate so status
    /// latency is unchanged and the numbers arrive after the list.
    /// Managed by `invalidate_line_stats`/`start_line_stats`/`finish_line_stats`,
    /// not generic `request`/`finish`: replays need a fresh status snapshot.
    pub const UNCOMMITTED_LINE_STATS: u32 = 1 << 19;
    const PRIMARY_REFRESH_FLAGS: u32 = Self::HEAD_BRANCH
        | Self::UPSTREAM_DIVERGENCE
        | Self::REBASE_STATE
        | Self::MERGE_COMMIT_MESSAGE
        | Self::WORKTREE_STATUS
        | Self::STAGED_STATUS
        | Self::LOG;

    pub fn is_in_flight(&self, flag: u32) -> bool {
        (self.in_flight & flag) != 0
    }

    pub fn any_in_flight(&self) -> bool {
        self.in_flight != 0
    }

    pub fn clear(&mut self) {
        self.in_flight = 0;
        self.pending = 0;
        self.pending_log = None;
        self.active_log = None;
        self.line_stats_generation = self.line_stats_generation.wrapping_add(1);
        self.active_line_stats = None;
        self.line_stats_requested = false;
    }

    pub(crate) fn invalidate_line_stats(&mut self) {
        self.line_stats_generation = self.line_stats_generation.wrapping_add(1);
        self.line_stats_requested = true;
    }

    /// Called only after both status lanes, including their replays, settle.
    pub(crate) fn start_line_stats(&mut self, status_ready: bool) -> Option<LineStatsGeneration> {
        if self.is_in_flight(Self::WORKTREE_STATUS | Self::STAGED_STATUS)
            || self.active_line_stats.is_some()
            || !self.line_stats_requested
        {
            return None;
        }
        self.line_stats_requested = false;
        if !status_ready {
            return None;
        }
        self.in_flight |= Self::UNCOMMITTED_LINE_STATS;
        self.active_line_stats = Some(self.line_stats_generation);
        Some(self.line_stats_generation)
    }

    /// Only the matching job may release the lane; invalidated results are discarded.
    pub(crate) fn finish_line_stats(&mut self, generation: LineStatsGeneration) -> bool {
        if self.active_line_stats != Some(generation) {
            return false;
        }
        self.active_line_stats = None;
        self.in_flight &= !Self::UNCOMMITTED_LINE_STATS;
        generation == self.line_stats_generation
    }

    /// Starts the common primary-refresh batch immediately when no work is already queued or
    /// running. Callers fall back to per-load request coalescing when this returns `None`.
    ///
    /// The batch includes a log load, so it takes that request and returns its
    /// sequence number: replies are matched against it by
    /// [`Self::is_active_log_reply`], and a batch that failed to declare one
    /// would have its log page silently discarded.
    pub fn request_primary_refresh_batch(&mut self, log: PendingLogLoad) -> Option<LogLoadSeq> {
        if self.in_flight == 0 && self.pending == 0 && self.pending_log.is_none() {
            self.in_flight |= Self::PRIMARY_REFRESH_FLAGS;
            Some(self.start_log(log))
        } else {
            None
        }
    }

    /// Marks `load` as the walk now in flight and hands out its sequence number.
    fn start_log(&mut self, load: PendingLogLoad) -> LogLoadSeq {
        self.last_log_seq = self.last_log_seq.wrapping_add(1);
        self.active_log = Some((self.last_log_seq, load));
        self.last_log_seq
    }

    /// For non-log loads: starts immediately if not in flight, otherwise coalesces by remembering
    /// one pending refresh for the same kind.
    pub fn request(&mut self, flag: u32) -> bool {
        if self.is_in_flight(flag) {
            self.pending |= flag;
            false
        } else {
            self.in_flight |= flag;
            true
        }
    }

    /// For non-log loads: finishes and indicates whether a pending request should be scheduled now.
    pub fn finish(&mut self, flag: u32) -> bool {
        self.in_flight &= !flag;
        if (self.pending & flag) != 0 {
            self.pending &= !flag;
            self.in_flight |= flag;
            true
        } else {
            false
        }
    }

    /// For log loads: coalesce by keeping only the latest requested
    /// `(scope, author, cursor)` while a log load is already in flight. Returns
    /// the new walk's sequence number when it starts now, `None` when it was
    /// queued behind the walk in flight.
    ///
    /// A request that changes the scope or the author filter is dispatched
    /// straight away instead of being queued: on a large repository a walk runs
    /// for tens of seconds, and the repo-load pool has one or two threads, so
    /// waiting the old one out would stall the new filter for that whole time.
    /// The effects layer cancels the superseded walk, and its reply is dropped
    /// by [`Self::is_active_log_reply`].
    pub fn request_log(&mut self, next: PendingLogLoad) -> Option<LogLoadSeq> {
        if !self.is_in_flight(Self::LOG) {
            self.in_flight |= Self::LOG;
            return Some(self.start_log(next));
        }

        let supersedes_active = self
            .active_log
            .as_ref()
            .is_none_or(|(_, active)| active.scope != next.scope || active.author != next.author);
        if supersedes_active {
            self.pending_log = None;
            return Some(self.start_log(next));
        }
        match &self.pending_log {
            // Scope or author changes invalidate older pending requests
            // (including pagination).
            Some(existing) if existing.scope != next.scope || existing.author != next.author => {
                self.pending_log = Some(next);
            }
            // A fresh walk invalidates pagination cursors. Never let a later
            // pagination request replace a pending refresh.
            Some(existing) if existing.cursor.is_none() && next.cursor.is_some() => {}
            _ => {
                self.pending_log = Some(next);
            }
        }
        None
    }

    /// Whether a log reply belongs to the walk that is currently in flight,
    /// rather than one that a newer request superseded (and that the effects
    /// layer cancelled). Superseded replies must be dropped without touching
    /// the in-flight bookkeeping — the walk that replaced them is still going.
    pub fn is_active_log_reply(&self, seq: LogLoadSeq) -> bool {
        self.active_log
            .as_ref()
            .is_some_and(|(active, _)| *active == seq)
    }

    /// The sequence number of the walk in flight, if any. Tests that answer a
    /// dispatched load by hand need it to send a reply the reducer will accept.
    pub fn active_log_seq(&self) -> Option<LogLoadSeq> {
        self.active_log.as_ref().map(|(seq, _)| *seq)
    }

    /// Whether the walk in flight is paginating rather than rebuilding the page.
    pub fn active_log_is_load_more(&self) -> bool {
        self.active_log
            .as_ref()
            .is_some_and(|(_, active)| active.cursor.is_some())
    }

    /// Finishes the walk in flight and starts whichever request queued behind
    /// it, returning that request and its sequence number. `prepare` adjusts
    /// the request before it is recorded, so the effect and bookkeeping agree.
    pub fn finish_log(
        &mut self,
        prepare: impl FnOnce(&mut PendingLogLoad),
    ) -> Option<(LogLoadSeq, PendingLogLoad)> {
        self.in_flight &= !Self::LOG;
        self.active_log = None;
        let mut next = self.pending_log.take()?;
        prepare(&mut next);
        self.in_flight |= Self::LOG;
        let seq = self.start_log(next.clone());
        Some((seq, next))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictFile {
    pub path: RepoPath,
    pub base_bytes: Option<Arc<[u8]>>,
    pub ours_bytes: Option<Arc<[u8]>>,
    pub theirs_bytes: Option<Arc<[u8]>>,
    pub current_bytes: Option<Arc<[u8]>>,
    pub base: Option<Arc<str>>,
    pub ours: Option<Arc<str>>,
    pub theirs: Option<Arc<str>>,
    pub current: Option<Arc<str>>,
}

impl ConflictFile {
    /// Build a conflict file from stage/current parts, canonicalizing UTF-8
    /// payloads down to text-only storage.
    pub fn from_loaded_stage_parts(
        path: impl Into<RepoPath>,
        base: ConflictStageParts,
        ours: ConflictStageParts,
        theirs: ConflictStageParts,
        current: ConflictStageParts,
    ) -> Self {
        let (base_bytes, base) = canonicalize_stage_parts(base.0, base.1);
        let (ours_bytes, ours) = canonicalize_stage_parts(ours.0, ours.1);
        let (theirs_bytes, theirs) = canonicalize_stage_parts(theirs.0, theirs.1);
        let (current_bytes, current) = canonicalize_stage_parts(current.0, current.1);

        Self {
            path: path.into(),
            base_bytes,
            ours_bytes,
            theirs_bytes,
            current_bytes,
            base,
            ours,
            theirs,
            current,
        }
    }

    /// Build a conflict file directly from an existing session without
    /// round-tripping through staged parts first.
    pub fn from_shared_conflict_session(
        path: impl Into<RepoPath>,
        session: &ConflictSession,
    ) -> Self {
        let (base_bytes, base) = conflict_file_side_from_payload(&session.base);
        let (ours_bytes, ours) = conflict_file_side_from_payload(&session.ours);
        let (theirs_bytes, theirs) = conflict_file_side_from_payload(&session.theirs);
        let (current_bytes, current) = session
            .current
            .as_ref()
            .map(conflict_file_side_from_payload)
            .unwrap_or((None, None));

        Self {
            path: path.into(),
            base_bytes,
            ours_bytes,
            theirs_bytes,
            current_bytes,
            base,
            ours,
            theirs,
            current,
        }
    }
}

fn conflict_file_side_from_payload(
    payload: &ConflictPayload,
) -> (Option<Arc<[u8]>>, Option<Arc<str>>) {
    payload.clone().into_stage_parts()
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConflictFileLoadMode {
    #[default]
    CurrentOnly,
    Full,
}

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
    /// Entered by "Start file browsing" and left by "Exit file browsing".
    /// Selecting the working-tree row changes the source without exiting.
    pub active: bool,
    pub source: FileSource,
    pub entries: Loadable<Arc<Vec<FileEntry>>>,
    pub expanded_dirs: FxHashSet<Arc<PathBuf>>,
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
            active: false,
            source: FileSource::default(),
            entries: Loadable::NotLoaded,
            expanded_dirs: FxHashSet::default(),
            search_query: String::new(),
            file_browser_rev: 0,
            stale: false,
            followed_selection_rev: None,
            pending_reopen: None,
        }
    }
}

impl FileBrowserState {
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

/// One opened file-content view, enough to replay it: the source revision and
/// the path. (Working-tree previews use [`FileSource::WorkingDirectory`].)
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewHistoryEntry {
    pub source: FileSource,
    pub path: PathBuf,
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

// ── App state ───────────────────────────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub repos: Vec<RepoState>,
    pub active_repo: Option<RepoId>,
    /// Unacknowledged open failures, shared by snapshots until they change.
    /// Window routing releases its reservations before acknowledging these.
    pub repo_open_failures: Arc<FxHashMap<PathBuf, u64>>,
    /// Store-wide sequence; acknowledgements must not reset retry baselines.
    pub repo_open_failure_revision: u64,
    pub clone: Option<CloneOpState>,
    pub notifications: Vec<AppNotification>,
    pub auth_prompt: Option<AuthPromptState>,
    pub branch_exists_prompt: Option<BranchExistsPromptState>,
    pub submodule_trust_prompt: Option<SubmoduleTrustPromptState>,
    /// A submodule trust check is running in the background. Set the moment the
    /// add/update/load is triggered and cleared when the check resolves, so the
    /// UI can show a pending/spinner state instead of a dead gap before the
    /// trust dialog (or a silent proceed) appears.
    pub submodule_trust_check_pending: Option<SubmoduleTrustCheckState>,
    pub git_runtime: GitRuntimeState,
    /// The signature verifiers Git can run. Formats without one are not verified.
    pub signing_tools: SigningToolsState,
    pub remote_url_policy: RemoteUrlPolicy,
    pub git_log_settings: GitLogSettings,
    pub remote_settings: RemoteSettings,
    pub file_browser_settings: FileBrowserSettings,
    pub sidebar_mode: SidebarMode,
    pub default_tag_type: DefaultTagType,
    /// Outstanding [`WatchLease`](crate::store::WatchLease)s per open
    /// repository. A leased repository keeps its file watcher running while
    /// it is not the active one (a hosted view of a linked worktree, say).
    pub watch_leases: Arc<FxHashMap<RepoId, u32>>,
}

impl AppState {
    /// Deterministic fixture: tests opt into an available runtime without spawning Git.
    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    pub fn test_default() -> Self {
        Self {
            git_runtime: GitRuntimeState {
                preference: gitcomet_core::process::GitExecutablePreference::SystemPath,
                availability: gitcomet_core::process::GitExecutableAvailability::Available {
                    version_output: "git version 2.55.0 (test)".into(),
                },
            },
            ..Self::default()
        }
    }

    /// The signature formats to verify: none when the preference is off,
    /// otherwise those whose verifier was not found missing.
    pub fn signature_verification_formats(&self) -> SignatureFormats {
        if self.git_log_settings.verify_commit_signatures {
            self.signing_tools.usable_formats()
        } else {
            SignatureFormats::NONE
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BranchExistsPromptOperation {
    CreateBranch,
    CheckoutRemoteBranch { remote: String, branch: String },
    RenameBranch { old_name: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BranchExistsPromptState {
    pub repo_id: RepoId,
    pub name: String,
    pub target: String,
    pub operation: BranchExistsPromptOperation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthPromptKind {
    UsernamePassword,
    Passphrase,
    HostVerification,
}

impl AuthPromptKind {
    pub fn requires_username(self) -> bool {
        matches!(self, Self::UsernamePassword)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthRetryOperation {
    RepoCommand {
        repo_id: RepoId,
        command: RepoCommandKind,
    },
    SafePushAfterCommit {
        repo_id: RepoId,
        context: SafePushAfterCommitContext,
    },
    Commit {
        repo_id: RepoId,
        message: String,
        amend: bool,
        push_after_commit: bool,
    },
    Clone {
        url: String,
        dest: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthPromptState {
    pub kind: AuthPromptKind,
    pub reason: String,
    pub operation: AuthRetryOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustPromptOperation {
    Add {
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
    },
    Update,
    Load {
        path: PathBuf,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustPromptState {
    pub repo_id: RepoId,
    pub operation: SubmoduleTrustPromptOperation,
    pub sources: Vec<SubmoduleTrustTarget>,
}

/// Which pending action a background trust check belongs to. Mirrors the
/// operation so the spinner's title matches the trust dialog that may follow.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleTrustCheckOperation {
    Add,
    Update,
    Load,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SubmoduleTrustCheckState {
    pub repo_id: RepoId,
    pub operation: SubmoduleTrustCheckOperation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppNotification {
    pub time: SystemTime,
    pub kind: AppNotificationKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppNotificationKind {
    Info,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CloneOpState {
    pub url: Arc<str>,
    pub dest: Arc<PathBuf>,
    pub status: CloneOpStatus,
    pub progress: CloneProgressMeter,
    pub seq: u64,
    pub output_tail: VecDeque<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmoduleAddProgressState {
    pub url: String,
    pub path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloneProgressStage {
    Loading,
    RemoteObjects,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CloneProgressMeter {
    pub stage: CloneProgressStage,
    pub percent: u8,
}

impl Default for CloneProgressMeter {
    fn default() -> Self {
        Self {
            stage: CloneProgressStage::Loading,
            percent: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloneOpStatus {
    Running,
    Cancelling,
    FinishedOk,
    Cancelled,
    FinishedErr(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandLogEntry {
    pub time: SystemTime,
    pub ok: bool,
    pub command: String,
    pub summary: String,
    /// Shared and capped: the log is deep-copied on every store dispatch
    /// (copy-on-write state), so owned per-entry output turned every message
    /// into a memcpy of up to 200 command transcripts.
    pub stdout: Arc<str>,
    pub stderr: Arc<str>,
    /// Whether finishing this command is worth telling the user about. Routine,
    /// user-initiated edits announce themselves through the change they make —
    /// a toast per staged line is noise — but they still belong in the log.
    /// Failures are always surfaced, whatever this says.
    pub announce_success: bool,
    /// The hook-activity entry that owns user-facing reporting for this
    /// command. When present, the UI does not also show the generic command
    /// completion toast/banner.
    pub hook_operation_id: Option<GitOperationId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHookOperationStatus {
    Running,
    Cancelling,
    Succeeded,
    SucceededWithHookFailure,
    Failed,
    Cancelled,
    TimedOut,
}

impl GitHookOperationStatus {
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Cancelling)
    }

    pub fn is_warning(self) -> bool {
        matches!(self, Self::SucceededWithHookFailure | Self::Cancelled)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitHookRunStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHookRun {
    pub id: HookExecutionId,
    pub name: String,
    pub status: GitHookRunStatus,
    pub exit_code: Option<i32>,
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHookOutputChunk {
    pub stream: GitOutputStream,
    pub text: Arc<str>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitHookOperation {
    pub id: GitOperationId,
    pub label: String,
    /// Single-line, user-facing context for the operation that invoked these
    /// hooks, such as a commit subject or branch/remote direction.
    pub context: Option<String>,
    pub time: SystemTime,
    pub duration: Option<Duration>,
    pub status: GitHookOperationStatus,
    pub hooks: Vec<GitHookRun>,
    pub output: Arc<VecDeque<GitHookOutputChunk>>,
    pub output_bytes: usize,
    pub output_truncated: bool,
    pub latest_line: String,
}

impl GitHookOperation {
    pub fn has_hooks(&self) -> bool {
        !self.hooks.is_empty()
    }

    pub fn active_hook_name(&self) -> Option<&str> {
        self.hooks
            .iter()
            .rev()
            .find(|hook| hook.status == GitHookRunStatus::Running)
            .map(|hook| hook.name.as_str())
    }

    pub fn combined_output(&self) -> String {
        let mut output = String::new();
        if self.output_truncated {
            output.push_str("[Earlier hook output was truncated]\n");
        }
        for chunk in self.output.iter() {
            output.push_str(&chunk.text);
        }
        output
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitOperationOuterOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCommitRetry {
    pub message: String,
    pub amend: bool,
    pub push_after_commit: bool,
}

#[derive(Clone, Debug)]
pub struct HistoryState {
    pub indexed: crate::indexed_history::IndexedHistoryState,
    pub authors: crate::history_authors::HistoryAuthorsState,
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
    pub blame_path: Option<PathBuf>,
    pub blame_source: Option<BlameSource>,
    pub blame: Loadable<Shared<Vec<BlameLine>>>,
    /// Annotations to keep painting while blame reloads for the same target, so
    /// the annotation column does not blank out on every refresh.
    pub retained_blame_while_loading: Option<Shared<Vec<BlameLine>>>,
    pub selected_commit: Option<CommitId>,
    pub selected_commit_rev: u64,
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
            blame_path: None,
            blame_source: None,
            blame: Loadable::NotLoaded,
            retained_blame_while_loading: None,
            selected_commit: None,
            selected_commit_rev: 0,
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

#[derive(Clone, Debug)]
pub struct DiffState {
    pub diff_target: Option<DiffTarget>,
    /// When true, the selected `diff_target` is rendered as a full-content file
    /// preview (the same renderer used for added/removed files — syntax
    /// highlighted, no green/red) rather than a diff. Set by `OpenFileContent`.
    pub content_preview: bool,
    /// When true, the file-content view is the editable buffer rather than the
    /// read-only preview. Only ever set together with `content_preview`, and
    /// only for a `WorkingTree` target — editing is always of the file on disk.
    /// Set by `OpenFileEditor`, cleared by `ExitDiffEditMode`.
    pub edit_mode: bool,
    /// The view that opened the editor. Editing always retargets the working
    /// tree, so both the original target and whether it was a diff or a
    /// full-content preview have to be retained explicitly for Save/Discard to
    /// return to the right place.
    pub edit_return_view: Option<FileEditReturnView>,
    pub diff_target_rev: u64,
    pub diff_state_rev: u64,
    /// A reload of the *same* target is in flight and the content still on
    /// screen is the generation from before it. Set when a reload keeps that
    /// content rather than blanking it, and cleared when the reload lands.
    ///
    /// Anything that builds a patch out of the rendered rows — staging a line or
    /// a hunk out of the diff — has to sit out this window: those rows describe
    /// the index as it was before the last command, so a patch cut from them no
    /// longer applies.
    pub diff_reload_in_flight: bool,
    pub diff_rev: u64,
    pub diff: Loadable<Shared<Diff>>,
    pub diff_file_rev: u64,
    pub diff_file: Loadable<Option<Shared<FileDiffText>>>,
    pub diff_preview_text_file_rev: u64,
    pub diff_preview_text_file: Loadable<Option<Shared<DiffPreviewTextFile>>>,
    pub submodule_summary_rev: u64,
    pub submodule_summary: Loadable<Shared<SubmoduleDiffSummary>>,
    pub inline_submodule_diff_rev: u64,
    pub inline_submodule_diff: Option<InlineSubmoduleDiffState>,
    pub diff_file_image: Loadable<Option<Shared<FileDiffImage>>>,
    pub text_attributes_rev: u64,
    /// `.gitattributes` and config for the selected file.
    pub text_attributes: Loadable<Arc<TextAttributes>>,
    pub text_override_rev: u64,
    /// The user's encoding / line-ending / tab-size choice for the open file.
    /// Dropped when another path is selected.
    pub text_override: Option<OpenFileTextOverride>,
}

/// A [`TextOverride`] and the file it belongs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenFileTextOverride {
    pub path: PathBuf,
    pub value: TextOverride,
}

impl DiffState {
    /// The override for `path` when it is the open file.
    pub fn text_override_for(&self, path: &std::path::Path) -> Option<TextOverride> {
        self.text_override
            .as_ref()
            .filter(|open| open.path == path)
            .map(|open| open.value)
    }

    /// The user's encoding choice for the selected file.
    pub fn selected_encoding_override(&self) -> Option<TextEncoding> {
        let path = self.diff_target.as_ref()?.file_path()?;
        self.text_override_for(path)?.encoding
    }
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            diff_target: None,
            content_preview: false,
            edit_mode: false,
            edit_return_view: None,
            diff_target_rev: 0,
            diff_state_rev: 0,
            diff_reload_in_flight: false,
            diff_rev: 0,
            diff: Loadable::NotLoaded,
            diff_file_rev: 0,
            diff_file: Loadable::NotLoaded,
            diff_preview_text_file_rev: 0,
            diff_preview_text_file: Loadable::NotLoaded,
            submodule_summary_rev: 0,
            submodule_summary: Loadable::NotLoaded,
            inline_submodule_diff_rev: 0,
            inline_submodule_diff: None,
            diff_file_image: Loadable::NotLoaded,
            text_attributes_rev: 0,
            text_attributes: Loadable::NotLoaded,
            text_override_rev: 0,
            text_override: None,
        }
    }
}

/// Main-pane destination restored when an editable working-tree buffer closes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEditReturnView {
    pub target: DiffTarget,
    pub content_preview: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InlineSubmoduleDiffSection {
    Range(SubmoduleDiffRangeKind),
    LiveStaged,
    LiveUnstaged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InlineSubmoduleDiffEntry {
    pub path: PathBuf,
    pub kind: FileStatusKind,
    pub target: DiffTarget,
    pub section: InlineSubmoduleDiffSection,
}

/// Which half of a submodule summary a changed file sits in, and therefore
/// which target the inline diff opens it with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SubmoduleChangeSection {
    /// One of the summary's pointer ranges, by slot.
    Range(usize),
    LiveStaged,
    LiveUnstaged,
}

/// The changed file at `section`/`index` and the target it opens under, or
/// `None` when there is nothing to open.
///
/// The one place a summary's coordinates become a target, so a row and the entry
/// a click resolves cannot describe different files.
fn submodule_change_at(
    summary: &SubmoduleDiffSummary,
    section: SubmoduleChangeSection,
    index: usize,
) -> Option<(
    &SubmoduleInnerChange,
    DiffTarget,
    InlineSubmoduleDiffSection,
)> {
    match section {
        SubmoduleChangeSection::Range(slot) => {
            let range = summary.ranges.get(slot)?;
            let change = range.changes.get(index)?;
            let (from_commit_id, to_commit_id) = (range.from.as_ref()?, range.to.as_ref()?);
            Some((
                change,
                DiffTarget::commit_range(
                    from_commit_id.clone(),
                    Some(to_commit_id.clone()),
                    Some(change.path.clone()),
                )
                .with_old_path(change.old_path.clone()),
                InlineSubmoduleDiffSection::Range(range.kind),
            ))
        }
        // Only a worktree summary has live halves, and only it gets live rows.
        SubmoduleChangeSection::LiveStaged | SubmoduleChangeSection::LiveUnstaged
            if summary.mode != SubmoduleDiffSummaryMode::Worktree =>
        {
            None
        }
        SubmoduleChangeSection::LiveStaged => {
            let change = summary.live_staged.get(index)?;
            Some((
                change,
                DiffTarget::working_tree(change.path.clone(), DiffArea::Staged),
                InlineSubmoduleDiffSection::LiveStaged,
            ))
        }
        SubmoduleChangeSection::LiveUnstaged => {
            let change = summary.live_unstaged.get(index)?;
            Some((
                change,
                DiffTarget::working_tree(change.path.clone(), DiffArea::Unstaged),
                InlineSubmoduleDiffSection::LiveUnstaged,
            ))
        }
    }
}

/// The target alone, without the entry around it: what a row needs per frame.
pub fn submodule_inline_diff_target(
    summary: &SubmoduleDiffSummary,
    section: SubmoduleChangeSection,
    index: usize,
) -> Option<DiffTarget> {
    submodule_change_at(summary, section, index).map(|(_, target, _)| target)
}

/// The inline-diff entry for one changed file; `submodule_inline_diff_entries`
/// is this in a loop.
pub fn submodule_inline_diff_entry(
    summary: &SubmoduleDiffSummary,
    section: SubmoduleChangeSection,
    index: usize,
) -> Option<InlineSubmoduleDiffEntry> {
    submodule_change_at(summary, section, index).map(|(change, target, section)| {
        InlineSubmoduleDiffEntry {
            path: change.path.clone(),
            kind: change.kind,
            target,
            section,
        }
    })
}

pub fn submodule_inline_diff_entries(
    summary: &SubmoduleDiffSummary,
) -> Vec<InlineSubmoduleDiffEntry> {
    let capacity = summary
        .ranges
        .iter()
        .map(|range| range.changes.len())
        .sum::<usize>()
        + summary.live_staged.len()
        + summary.live_unstaged.len();
    let mut entries = Vec::with_capacity(capacity);
    let sections = (0..summary.ranges.len())
        .map(SubmoduleChangeSection::Range)
        .chain([
            SubmoduleChangeSection::LiveStaged,
            SubmoduleChangeSection::LiveUnstaged,
        ]);
    for section in sections {
        let count = match section {
            SubmoduleChangeSection::Range(slot) => summary.ranges[slot].changes.len(),
            SubmoduleChangeSection::LiveStaged => summary.live_staged.len(),
            SubmoduleChangeSection::LiveUnstaged => summary.live_unstaged.len(),
        };
        entries.extend(
            (0..count).filter_map(|index| submodule_inline_diff_entry(summary, section, index)),
        );
    }
    entries
}

/// The inline-diff entries for a linked worktree's changed files, in the order
/// the rows are rendered: staged first, then unstaged, the same order the
/// working-tree pane uses.
///
/// One builder rather than two, because the indices have to agree. The rows are
/// rebuilt from every scan while the open diff carries the list it was opened
/// with, so the reducer re-resolves that list against each new scan
/// (`refresh_worktree_inline_diff_entries`) -- and a second, separately written
/// ordering in the view would silently desynchronize the two.
pub fn worktree_inline_diff_entries(
    summary: &WorktreeDirtySummary,
) -> Vec<InlineSubmoduleDiffEntry> {
    let staged = summary.staged.iter().map(|f| (f, DiffArea::Staged));
    let unstaged = summary.unstaged.iter().map(|f| (f, DiffArea::Unstaged));
    staged
        .chain(unstaged)
        .map(|(file, area)| InlineSubmoduleDiffEntry {
            path: file.path.clone(),
            kind: file.kind,
            target: DiffTarget::working_tree(file.path.clone(), area),
            section: match area {
                DiffArea::Staged => InlineSubmoduleDiffSection::LiveStaged,
                _ => InlineSubmoduleDiffSection::LiveUnstaged,
            },
        })
        .collect()
}

/// Which foreign repository the inline diff is showing, and therefore how the
/// UI labels it. The machinery is the same either way: a throwaway handle opened
/// at `submodule_repo_path`, with its files and diffs parked on the active repo.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ForeignDiffOrigin {
    Submodule,
    Worktree {
        branch: Option<String>,
        detached: bool,
    },
}

#[derive(Clone, Debug)]
pub struct InlineSubmoduleDiffState {
    pub origin: ForeignDiffOrigin,
    pub submodule_repo_path: PathBuf,
    pub parent_submodule_path: PathBuf,
    pub entries: Arc<[InlineSubmoduleDiffEntry]>,
    pub selected_ix: usize,
    pub target: DiffTarget,
    pub rev: u64,
    pub diff_rev: u64,
    pub diff: Loadable<Shared<Diff>>,
    pub diff_file_rev: u64,
    pub diff_file: Loadable<Option<Shared<FileDiffText>>>,
    pub diff_file_image: Loadable<Option<Shared<FileDiffImage>>>,
}

#[derive(Clone, Debug)]
pub struct ConflictState {
    pub conflict_file_path: Option<PathBuf>,
    pub conflict_file_load_mode: ConflictFileLoadMode,
    pub conflict_file: Loadable<Option<ConflictFile>>,
    pub conflict_session: Option<ConflictSession>,
    /// Session stashed across a same-path conflict reload so
    /// `conflict_file_loaded` can restore resolutions (and skip the on-open
    /// autosolve). Cleared on path switch and consumed on load completion.
    pub session_pending_restore: Option<ConflictSession>,
    pub conflict_hide_resolved: bool,
    pub conflict_rev: u64,
}

impl Default for ConflictState {
    fn default() -> Self {
        Self {
            conflict_file_path: None,
            conflict_file_load_mode: ConflictFileLoadMode::CurrentOnly,
            conflict_file: Loadable::NotLoaded,
            conflict_session: None,
            session_pending_restore: None,
            conflict_hide_resolved: false,
            conflict_rev: 0,
        }
    }
}

const BRANCH_SIDEBAR_REV_MIX: u64 = 0x9e37_79b9_7f4a_7c15;
const STATUS_CACHE_REV_MIX: u64 = 0x517c_c1b7_2722_0a95;

#[inline]
fn mix_branch_sidebar_revs(values: [u64; 7]) -> u64 {
    let mut acc = BRANCH_SIDEBAR_REV_MIX;
    for value in values {
        acc ^= value.wrapping_mul(BRANCH_SIDEBAR_REV_MIX);
        acc = acc.rotate_left(11).wrapping_add(BRANCH_SIDEBAR_REV_MIX);
    }
    acc
}

#[inline]
pub fn mix_status_cache_revs(values: [u64; 2]) -> u64 {
    let mut acc = STATUS_CACHE_REV_MIX;
    for value in values {
        acc ^= value.wrapping_mul(STATUS_CACHE_REV_MIX);
        acc = acc.rotate_left(9).wrapping_add(STATUS_CACHE_REV_MIX);
    }
    acc
}

#[derive(Clone, Debug)]
pub struct InteractiveRebaseSetup {
    pub base: String,
    pub entries: Loadable<Vec<InteractiveRebaseEntry>>,
}

#[derive(Clone, Debug)]
pub struct InteractiveCherryPickSetup {
    pub entries: Vec<InteractiveRebaseEntry>,
    pub source_colors: Vec<(String, u8)>,
    /// Full commit messages are loaded separately from the subject-only log
    /// entries. The editor must not expose rewording or start the operation
    /// until this is `Ready`, otherwise saving a reword can truncate a body.
    pub full_messages: Loadable<()>,
}

/// User-visible repository feedback and bounded diagnostic/activity history.
///
/// Keeping this together makes the lifecycle explicit: a successful reopen can
/// reset repository feedback without touching loaded Git data or navigation.
#[derive(Clone, Debug, Default)]
pub struct RepoFeedbackState {
    pub missing_on_disk: bool,
    pub last_error: Option<String>,
    pub diagnostics: Vec<DiagnosticEntry>,
    /// Number appended, including entries evicted from the bounded history.
    pub diagnostics_seq: u64,
    pub command_log: Vec<CommandLogEntry>,
    pub hook_activity: Vec<GitHookOperation>,
    pub hook_activity_rev: u64,
    /// Set only while reducing the existing command completion nested inside a
    /// `GitOperationFinished` message.
    pub(crate) command_log_operation_id: Option<GitOperationId>,
}

/// Deferred operation context retained between an initial failure and the
/// user's follow-up action (authentication retry or confirmed force-push).
#[derive(Clone, Debug, Default)]
pub struct RepoPendingState {
    pub commit_retry: Option<PendingCommitRetry>,
    pub force_push_lease: Option<ForcePushLease>,
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

#[derive(Clone, Debug)]
pub struct TagPushPreviewState {
    pub request: gitcomet_core::tag_push::TagPushRequest,
    pub generation: u64,
    pub cancellation: gitcomet_core::services::CancellationToken,
    pub result: Loadable<Arc<gitcomet_core::tag_push::TagPushPreview>>,
}

#[derive(Clone, Debug)]
pub struct RepoState {
    pub id: RepoId,
    pub spec: RepoSpec,
    /// Unique per repository opened in this process. `RepoId`s are per store
    /// and may be reused; the pair (window, id, lifetime) never is.
    lifetime: u64,
    session_workdir_key: Arc<str>,
    /// A loading tab created from an external folder drop. It remains visible
    /// while the backend validates it, but session persistence must ignore it
    /// until [`Self::commit_external_drop_open`] is called.
    provisional_external_drop_open: bool,
    /// Repository that was active immediately before this external-drop tab
    /// was created. A failed validation restores this exact tab when it still
    /// exists instead of selecting the dropped tab's neighbour.
    external_drop_previous_active_repo: Option<RepoId>,
    pub loads_in_flight: RepoLoadsInFlight,
    /// Fetches and prunes as well as pulls.
    pub pull_in_flight: u32,
    /// The pulls among `pull_in_flight`: those also merge into the checkout.
    pub worktree_pull_in_flight: u32,
    pub push_in_flight: u32,
    pub worktrees_in_flight: u32,
    pub local_actions_in_flight: u32,
    /// Commands that write sequencer state or move HEAD. Continue and Abort
    /// wait for these alone, so a merge tool cannot lock them out.
    pub sequencer_actions_in_flight: u32,
    pub commit_in_flight: u32,

    pub open: Loadable<()>,
    pub history_state: HistoryState,
    pub head_branch: Loadable<String>,
    pub detached_head_commit: Option<CommitId>,
    pub head_branch_rev: u64,
    pub upstream_divergence: Loadable<Option<UpstreamDivergence>>,
    pub upstream_divergence_rev: u64,
    pub branches: Loadable<Arc<Vec<Branch>>>,
    pub branches_rev: u64,
    pub tags: Loadable<Arc<Vec<Tag>>>,
    pub tags_rev: u64,
    pub remote_tags: Loadable<Arc<Vec<RemoteTag>>>,
    pub remote_tags_rev: u64,
    pub remotes: Loadable<Arc<Vec<Remote>>>,
    pub remotes_rev: u64,
    pub remote_branches: Loadable<Arc<Vec<RemoteBranch>>>,
    pub remote_branches_rev: u64,
    pub worktree_status: Loadable<Arc<Vec<FileStatus>>>,
    pub worktree_status_rev: u64,
    /// Per-file `+/-` for both lanes, cached until the next index or worktree
    /// change.
    pub uncommitted_line_stats: Loadable<Arc<UncommittedLineStats>>,
    /// Per lane, so churn in one does not invalidate the other's rows.
    pub staged_line_stats_rev: u64,
    pub unstaged_line_stats_rev: u64,
    pub staged_status: Loadable<Arc<Vec<FileStatus>>>,
    pub staged_status_rev: u64,
    pub status: Loadable<Shared<RepoStatus>>,
    pub status_rev: u64,
    /// Paths confirmed as gitlinks in the current HEAD tree. This small cache
    /// preserves the classification of staged submodule deletions across view
    /// navigation without leaking backend-specific tree objects into state.
    pub head_gitlink_paths: FxHashSet<PathBuf>,
    /// Cached flag: true when the current unstaged/worktree lane contains at
    /// least one `FileStatusKind::Conflicted` entry. Recomputed in
    /// `set_worktree_status` and `set_status`.
    pub has_unstaged_conflicts: bool,
    pub log: Loadable<Shared<LogPage>>,
    pub log_loading_more: bool,
    pub log_rev: u64,
    pub stashes: Loadable<Arc<Vec<StashEntry>>>,
    pub stashes_rev: u64,
    pub reflog: Loadable<Arc<Vec<ReflogEntry>>>,
    pub reflog_rev: u64,
    pub recent_commit_messages: Loadable<Arc<Vec<RecentCommitMessage>>>,
    pub recent_commit_messages_rev: u64,
    pub rebase_in_progress: Loadable<bool>,
    pub sequencer_state: Loadable<SequencerState>,
    pub merge_commit_message: Loadable<Option<String>>,
    /// Commit whose full message the history hover card is showing, and the
    /// message once it arrives. A single slot: only one card is ever open, and
    /// the view keeps its own small cache of recently fetched messages.
    pub tag_push_previews: [Option<TagPushPreviewState>; 2],
    pub hover_commit_message: Option<(CommitId, Loadable<Arc<str>>)>,
    pub interactive_rebase_setup: Option<InteractiveRebaseSetup>,
    pub interactive_cherry_pick_setup: Option<InteractiveCherryPickSetup>,
    pub merge_message_rev: u64,
    /// Commit message git prepared for the next commit (a staged revert), and
    /// a rev so the commit box can apply it exactly once.
    pub suggested_commit_message: Option<String>,
    pub suggested_commit_message_rev: u64,
    pub worktrees: Loadable<Arc<Vec<Worktree>>>,
    pub worktrees_rev: u64,
    /// Uncommitted-change counts for the *other* linked worktrees, so the
    /// history pane can show work left behind in a worktree that is not the one
    /// being viewed. Only worktrees with changes are kept.
    pub worktree_dirty: Loadable<Arc<Vec<WorktreeDirtySummary>>>,
    pub worktree_dirty_rev: u64,
    /// Tip-commit author/date/summary per short refname, loaded on demand by
    /// pickers that display it. Invalidated whenever the branch or
    /// remote-branch lists change, so it never outlives the refs it describes.
    pub ref_metadata: Loadable<Arc<FxHashMap<String, RefMetadata>>>,
    pub ref_metadata_rev: u64,
    pub submodules: Loadable<Arc<Vec<Submodule>>>,
    pub submodules_rev: u64,
    pub submodule_add_in_flight: Option<SubmoduleAddProgressState>,
    pub sidebar_data_request: SidebarDataRequest,
    /// Invalidates cached branch-sidebar rows when any sidebar-relevant source changes.
    pub branch_sidebar_rev: u64,
    pub file_browser: FileBrowserState,
    pub navigation: RepoNavigationState,

    pub diff_state: DiffState,
    pub conflict_state: ConflictState,

    pub open_rev: u64,
    pub ops_rev: u64,
    /// Bumped when the watcher (or the window-focus full refresh) reports a
    /// working-tree write. The view stats the open file when this moves.
    pub worktree_change_rev: u64,
    /// Hosted diff panes' sessions, apart from History's selected diff.
    pub diff_sessions:
        Arc<FxHashMap<crate::diff_session::DiffViewId, crate::diff_session::DiffSession>>,
    /// The worktree paths of the change that set `worktree_change_rev`; see
    /// [`RepoState::worktree_paths_changed_since`].
    pub worktree_changed_paths: crate::msg::ChangedPaths,
    /// Bumped when a GitComet-run git command that may have rewritten
    /// worktree files completes. Not `ops_rev`: that one also moves when a
    /// command *starts*, which would spend the signal before the disk changed.
    pub local_worktree_write_rev: u64,
    pub last_active_at: Option<SystemTime>,

    pub feedback: RepoFeedbackState,
    pub pending: RepoPendingState,
    pub load_epoch: u64,
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
    /// See the `lifetime` field.
    pub fn lifetime(&self) -> u64 {
        self.lifetime
    }

    pub fn new_opening(id: RepoId, spec: RepoSpec) -> Self {
        static NEXT_LIFETIME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let session_workdir_key = session::path_storage_key_shared(&spec.workdir);
        Self {
            id,
            spec,
            lifetime: NEXT_LIFETIME.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            session_workdir_key,
            provisional_external_drop_open: false,
            external_drop_previous_active_repo: None,
            loads_in_flight: RepoLoadsInFlight::default(),
            pull_in_flight: 0,
            worktree_pull_in_flight: 0,
            push_in_flight: 0,
            worktrees_in_flight: 0,
            local_actions_in_flight: 0,
            sequencer_actions_in_flight: 0,
            commit_in_flight: 0,
            open: Loadable::Loading,
            history_state: HistoryState::default(),
            head_branch: Loadable::NotLoaded,
            detached_head_commit: None,
            head_branch_rev: 0,
            upstream_divergence: Loadable::NotLoaded,
            upstream_divergence_rev: 0,
            branches: Loadable::NotLoaded,
            branches_rev: 0,
            tags: Loadable::NotLoaded,
            tags_rev: 0,
            remote_tags: Loadable::NotLoaded,
            remote_tags_rev: 0,
            remotes: Loadable::NotLoaded,
            remotes_rev: 0,
            remote_branches: Loadable::NotLoaded,
            remote_branches_rev: 0,
            worktree_status: Loadable::NotLoaded,
            uncommitted_line_stats: Loadable::NotLoaded,
            staged_line_stats_rev: 0,
            unstaged_line_stats_rev: 0,
            worktree_status_rev: 0,
            staged_status: Loadable::NotLoaded,
            staged_status_rev: 0,
            status: Loadable::NotLoaded,
            status_rev: 0,
            head_gitlink_paths: FxHashSet::default(),
            has_unstaged_conflicts: false,
            log: Loadable::NotLoaded,
            log_loading_more: false,
            log_rev: 0,
            stashes: Loadable::NotLoaded,
            stashes_rev: 0,
            reflog: Loadable::NotLoaded,
            reflog_rev: 0,
            recent_commit_messages: Loadable::NotLoaded,
            recent_commit_messages_rev: 0,
            rebase_in_progress: Loadable::NotLoaded,
            sequencer_state: Loadable::NotLoaded,
            merge_commit_message: Loadable::NotLoaded,
            tag_push_previews: [None, None],
            hover_commit_message: None,
            interactive_rebase_setup: None,
            interactive_cherry_pick_setup: None,
            merge_message_rev: 0,
            suggested_commit_message: None,
            suggested_commit_message_rev: 0,
            worktrees: Loadable::NotLoaded,
            worktrees_rev: 0,
            worktree_dirty: Loadable::NotLoaded,
            worktree_dirty_rev: 0,
            ref_metadata: Loadable::NotLoaded,
            ref_metadata_rev: 0,
            submodules: Loadable::NotLoaded,
            submodules_rev: 0,
            submodule_add_in_flight: None,
            sidebar_data_request: SidebarDataRequest::default(),
            branch_sidebar_rev: 0,
            file_browser: FileBrowserState::default(),
            navigation: RepoNavigationState::default(),
            diff_state: DiffState::default(),
            conflict_state: ConflictState::default(),
            open_rev: 0,
            ops_rev: 0,
            worktree_change_rev: 0,
            worktree_changed_paths: crate::msg::ChangedPaths::Unknown,
            diff_sessions: Arc::default(),
            local_worktree_write_rev: 0,
            last_active_at: None,
            feedback: RepoFeedbackState::default(),
            pending: RepoPendingState::default(),
            load_epoch: 0,
        }
    }

    pub(crate) fn new_external_drop_opening(
        id: RepoId,
        spec: RepoSpec,
        previous_active_repo: Option<RepoId>,
    ) -> Self {
        let mut repo = Self::new_opening(id, spec);
        repo.provisional_external_drop_open = true;
        repo.external_drop_previous_active_repo = previous_active_repo;
        repo
    }

    pub fn is_provisional_external_drop_open(&self) -> bool {
        self.provisional_external_drop_open
    }

    pub(crate) fn external_drop_previous_active_repo(&self) -> Option<RepoId> {
        self.external_drop_previous_active_repo
    }

    pub(crate) fn set_external_drop_previous_active_repo(&mut self, repo_id: Option<RepoId>) {
        if self.provisional_external_drop_open {
            self.external_drop_previous_active_repo = repo_id;
        }
    }

    /// Commits a successfully opened external-drop candidate. Returns whether
    /// the repository was provisional so the reducer can emit its deferred
    /// session and recent-repository persistence exactly once.
    pub(crate) fn commit_external_drop_open(&mut self) -> bool {
        let was_provisional = std::mem::take(&mut self.provisional_external_drop_open);
        if was_provisional {
            self.external_drop_previous_active_repo = None;
        }
        was_provisional
    }

    pub(crate) fn set_spec(&mut self, spec: RepoSpec) {
        self.session_workdir_key = session::path_storage_key_shared(&spec.workdir);
        self.spec = spec;
    }

    pub(crate) fn session_workdir_key(&self) -> &Arc<str> {
        &self.session_workdir_key
    }

    pub(crate) fn set_head_branch(&mut self, head_branch: Loadable<String>) {
        if self.head_branch == head_branch {
            return;
        }
        self.head_branch = head_branch;
        self.head_branch_rev = self.head_branch_rev.wrapping_add(1);
        self.bump_branch_sidebar_rev();
    }

    pub(crate) fn set_detached_head_commit(&mut self, detached_head_commit: Option<CommitId>) {
        if self.detached_head_commit == detached_head_commit {
            return;
        }
        self.detached_head_commit = detached_head_commit;
    }

    pub(crate) fn set_branches(&mut self, branches: Loadable<Vec<Branch>>) {
        let branches = loadable_into_arc(branches);
        if self.branches == branches {
            return;
        }
        self.branches = branches;
        self.branches_rev = self.branches_rev.wrapping_add(1);
        self.invalidate_ref_metadata();
        self.bump_branch_sidebar_rev();
    }

    pub(crate) fn set_tags(&mut self, tags: Loadable<Vec<Tag>>) {
        let tags = loadable_into_arc(tags);
        if self.tags == tags {
            return;
        }
        self.tags = tags;
        self.tags_rev = self.tags_rev.wrapping_add(1);
    }

    pub(crate) fn set_remote_tags(&mut self, remote_tags: Loadable<Vec<RemoteTag>>) {
        let remote_tags = loadable_into_arc(remote_tags);
        if self.remote_tags == remote_tags {
            return;
        }
        self.remote_tags = remote_tags;
        self.remote_tags_rev = self.remote_tags_rev.wrapping_add(1);
    }

    pub(crate) fn set_remotes(&mut self, remotes: Loadable<Vec<Remote>>) {
        let remotes = loadable_into_arc(remotes);
        if self.remotes == remotes {
            return;
        }
        self.remotes = remotes;
        self.remotes_rev = self.remotes_rev.wrapping_add(1);
        self.bump_branch_sidebar_rev();
    }

    pub(crate) fn set_remote_branches(&mut self, remote_branches: Loadable<Vec<RemoteBranch>>) {
        let remote_branches = loadable_into_arc(remote_branches);
        if self.remote_branches == remote_branches {
            return;
        }
        self.remote_branches = remote_branches;
        self.remote_branches_rev = self.remote_branches_rev.wrapping_add(1);
        self.invalidate_ref_metadata();
        self.bump_branch_sidebar_rev();
    }

    pub(crate) fn set_stashes(&mut self, stashes: Loadable<Vec<StashEntry>>) {
        let stashes = loadable_into_arc(stashes);
        if self.stashes == stashes {
            return;
        }
        self.stashes = stashes;
        self.stashes_rev = self.stashes_rev.wrapping_add(1);
        self.bump_branch_sidebar_rev();
    }

    /// Reflog entries for the HEAD reflog, behind an `Arc` for the same reason
    /// `stashes` is: the reflog panel reads the whole list every render, and a
    /// deep clone of up to 200 entries per frame is exactly the cost that made
    /// that panel feel slow. `reflog_rev` is what the panel keys its filtered
    /// row cache on, so it must bump on every real change and never otherwise.
    pub(crate) fn set_reflog(&mut self, reflog: Loadable<Vec<ReflogEntry>>) {
        let reflog = loadable_into_arc(reflog);
        if self.reflog == reflog {
            return;
        }
        self.reflog = reflog;
        self.reflog_rev = self.reflog_rev.wrapping_add(1);
    }

    pub(crate) fn set_recent_commit_messages(
        &mut self,
        messages: Loadable<Vec<RecentCommitMessage>>,
    ) {
        let messages = loadable_into_arc(messages);
        if self.recent_commit_messages == messages {
            return;
        }
        self.recent_commit_messages = messages;
        self.recent_commit_messages_rev = self.recent_commit_messages_rev.wrapping_add(1);
    }

    pub(crate) fn clear_head_dependent_cached_state(&mut self) {
        self.pending.force_push_lease = None;
        self.head_gitlink_paths.clear();
        self.set_recent_commit_messages(Loadable::NotLoaded);
    }

    pub(crate) fn set_worktrees(&mut self, worktrees: Loadable<Vec<Worktree>>) {
        let worktrees = loadable_into_arc(worktrees);
        if self.worktrees == worktrees {
            return;
        }
        self.worktrees = worktrees;
        self.worktrees_rev = self.worktrees_rev.wrapping_add(1);
        self.bump_branch_sidebar_rev();
    }

    pub(crate) fn set_worktree_dirty(
        &mut self,
        worktree_dirty: Loadable<Vec<WorktreeDirtySummary>>,
    ) {
        let worktree_dirty = loadable_into_arc(worktree_dirty);
        if self.worktree_dirty == worktree_dirty {
            return;
        }
        self.worktree_dirty = worktree_dirty;
        self.worktree_dirty_rev = self.worktree_dirty_rev.wrapping_add(1);
    }

    pub(crate) fn set_ref_metadata(
        &mut self,
        ref_metadata: Loadable<Arc<FxHashMap<String, RefMetadata>>>,
    ) {
        if self.ref_metadata == ref_metadata {
            return;
        }
        self.ref_metadata = ref_metadata;
        self.ref_metadata_rev = self.ref_metadata_rev.wrapping_add(1);
    }

    /// Drops cached ref metadata so the next picker open re-fetches it. Called
    /// from the branch setters, which already early-return when unchanged, so
    /// background refreshes that find no ref changes will not thrash this.
    fn invalidate_ref_metadata(&mut self) {
        // A load that read the *old* refs may already be in flight. Mark it
        // pending so its result schedules a refetch; otherwise that stale map
        // lands as `Ready` and, since callers only refetch on
        // `NotLoaded | Error`, it would never be corrected.
        if self
            .loads_in_flight
            .is_in_flight(RepoLoadsInFlight::REF_METADATA)
        {
            self.loads_in_flight
                .request(RepoLoadsInFlight::REF_METADATA);
        }
        if matches!(self.ref_metadata, Loadable::NotLoaded) {
            return;
        }
        self.ref_metadata = Loadable::NotLoaded;
        self.ref_metadata_rev = self.ref_metadata_rev.wrapping_add(1);
    }

    pub(crate) fn set_submodules(&mut self, submodules: Loadable<Vec<Submodule>>) {
        let submodules = loadable_into_arc(submodules);
        if self.submodules == submodules {
            return;
        }
        self.submodules = submodules;
        self.submodules_rev = self.submodules_rev.wrapping_add(1);
        self.bump_branch_sidebar_rev();
    }

    #[inline]
    fn bump_branch_sidebar_rev(&mut self) {
        self.branch_sidebar_rev = self.branch_sidebar_rev.wrapping_add(1);
    }

    #[inline]
    pub fn branch_sidebar_cache_rev(&self) -> u64 {
        let rev = self.branch_sidebar_rev;
        if rev != 0 {
            rev
        } else {
            mix_branch_sidebar_revs([
                self.head_branch_rev,
                self.branches_rev,
                self.remotes_rev,
                self.remote_branches_rev,
                self.worktrees_rev,
                self.submodules_rev,
                self.stashes_rev,
            ])
        }
    }

    pub(crate) fn set_sidebar_data_request(&mut self, request: SidebarDataRequest) {
        self.sidebar_data_request = request;
    }

    /// Bumps only the lanes that changed. Never sets `Loading`, so the numbers
    /// stay on screen across a rescan the way `worktree_dirty` does.
    pub(crate) fn set_uncommitted_line_stats(
        &mut self,
        stats: Loadable<Arc<UncommittedLineStats>>,
    ) {
        let (staged_changed, unstaged_changed) = match (&self.uncommitted_line_stats, &stats) {
            (Loadable::Ready(previous), Loadable::Ready(next)) => (
                previous.staged != next.staged,
                previous.unstaged != next.unstaged,
            ),
            _ => (true, true),
        };
        self.uncommitted_line_stats = stats;
        if staged_changed {
            self.staged_line_stats_rev = self.staged_line_stats_rev.wrapping_add(1);
        }
        if unstaged_changed {
            self.unstaged_line_stats_rev = self.unstaged_line_stats_rev.wrapping_add(1);
        }
    }

    pub fn line_stats_rev(&self, area: DiffArea) -> u64 {
        match area {
            DiffArea::Staged => self.staged_line_stats_rev,
            DiffArea::Unstaged => self.unstaged_line_stats_rev,
        }
    }

    pub fn line_stats_for_area(
        &self,
        area: DiffArea,
    ) -> Option<&rustc_hash::FxHashMap<PathBuf, LineStats>> {
        match &self.uncommitted_line_stats {
            Loadable::Ready(stats) => Some(stats.for_area(area)),
            _ => None,
        }
    }

    pub(crate) fn set_worktree_status(&mut self, status: Loadable<Vec<FileStatus>>) {
        let status = loadable_into_arc(status);
        if self.worktree_status == status {
            return;
        }
        self.has_unstaged_conflicts = matches!(
            &status,
            Loadable::Ready(entries)
                if entries.iter().any(|entry| entry.kind == FileStatusKind::Conflicted)
        );
        self.worktree_status = status;
        self.worktree_status_rev = self.worktree_status_rev.wrapping_add(1);
    }

    pub(crate) fn set_staged_status(&mut self, status: Loadable<Vec<FileStatus>>) {
        let status = loadable_into_arc(status);
        if self.staged_status == status {
            return;
        }
        self.staged_status = status;
        self.staged_status_rev = self.staged_status_rev.wrapping_add(1);
    }

    pub(crate) fn set_status(&mut self, status: Loadable<Shared<RepoStatus>>) {
        let next_worktree = match &status {
            Loadable::NotLoaded => Loadable::NotLoaded,
            Loadable::Loading => Loadable::Loading,
            Loadable::Error(err) => Loadable::Error(err.clone()),
            Loadable::Ready(status) => Loadable::Ready(Arc::clone(&status.unstaged)),
        };
        let next_staged = match &status {
            Loadable::NotLoaded => Loadable::NotLoaded,
            Loadable::Loading => Loadable::Loading,
            Loadable::Error(err) => Loadable::Error(err.clone()),
            Loadable::Ready(status) => Loadable::Ready(Arc::clone(&status.staged)),
        };
        if self.worktree_status != next_worktree {
            self.worktree_status = next_worktree;
            self.worktree_status_rev = self.worktree_status_rev.wrapping_add(1);
        }
        self.has_unstaged_conflicts = matches!(
            &status,
            Loadable::Ready(s) if s.unstaged.iter().any(|e| e.kind == FileStatusKind::Conflicted)
        );
        if self.staged_status != next_staged {
            self.staged_status = next_staged;
            self.staged_status_rev = self.staged_status_rev.wrapping_add(1);
        }
        if self.status == status {
            return;
        }
        self.status = status;
        self.status_rev = self.status_rev.wrapping_add(1);
    }

    pub fn worktree_status_entries(&self) -> Option<&[FileStatus]> {
        match &self.worktree_status {
            Loadable::Ready(entries) => Some(entries.as_slice()),
            _ => match &self.status {
                Loadable::Ready(status) => Some(status.unstaged.as_slice()),
                _ => None,
            },
        }
    }

    pub fn staged_status_entries(&self) -> Option<&[FileStatus]> {
        match &self.staged_status {
            Loadable::Ready(entries) => Some(entries.as_slice()),
            _ => match &self.status {
                Loadable::Ready(status) => Some(status.staged.as_slice()),
                _ => None,
            },
        }
    }

    /// Whether an ordinary `git commit -m` would have no staged snapshot to
    /// record. Unstaged and untracked changes do not make that command
    /// committable; a merge waiting to be concluded is the exception because
    /// Git still needs its merge commit even when the resulting tree is clean.
    pub fn nothing_to_commit(&self) -> bool {
        self.staged_status_entries()
            .is_some_and(|entries| entries.is_empty())
            && !matches!(self.merge_commit_message, Loadable::Ready(Some(_)))
    }

    /// Whether starting a history-rewriting operation (rebase, cherry-pick,
    /// revert, squash) must be blocked. Git runs a single sequencer and
    /// refuses to start any of these while a rebase, cherry-pick, or revert
    /// is in progress or a merge awaits its commit — launch surfaces gate on
    /// the same rule rather than surfacing git's refusal as a raw error.
    pub fn history_rewrite_busy(&self) -> bool {
        self.local_actions_in_flight > 0
            || matches!(
                self.sequencer_state,
                Loadable::Ready(state) if state != SequencerState::None
            )
            || matches!(self.rebase_in_progress, Loadable::Ready(true))
            || matches!(&self.merge_commit_message, Loadable::Ready(Some(_)))
    }

    pub fn status_entries_for_area(&self, area: DiffArea) -> Option<&[FileStatus]> {
        match area {
            DiffArea::Unstaged => self.worktree_status_entries(),
            DiffArea::Staged => self.staged_status_entries(),
        }
    }

    /// The commit the user is browsing when the file directory is pinned to a
    /// historical point (`file_browser.source == Commit`); `None` on live state.
    pub fn browsing_commit(&self) -> Option<&CommitId> {
        match &self.file_browser.source {
            FileSource::Commit(id) => Some(id),
            _ => None,
        }
    }

    /// The repo-relative path of the file the main pane is showing, whatever
    /// form it is showing it in — a diff, the read-only content view, or the
    /// editor.
    ///
    /// Used by the file explorer to mark the open file and by the locate action
    /// to decide what to reveal. Deliberately not gated on `content_preview`:
    /// a diff of a file still means that file is the one open.
    pub fn open_file_path(&self) -> Option<&std::path::Path> {
        match self.diff_state.diff_target.as_ref()? {
            DiffTarget::WorkingTree { path, .. } => Some(path.as_path()),
            DiffTarget::Commit { path, .. } | DiffTarget::CommitRange { path, .. } => {
                path.as_deref()
            }
        }
    }

    pub fn status_entry_for_path(
        &self,
        area: DiffArea,
        path: &std::path::Path,
    ) -> Option<&FileStatus> {
        self.status_entries_for_area(area)?
            .iter()
            .find(|entry| entry.path == path)
    }

    pub fn worktree_status_cache_rev(&self) -> u64 {
        if self.worktree_status_rev != 0 || !matches!(self.worktree_status, Loadable::NotLoaded) {
            self.worktree_status_rev
        } else {
            self.status_rev
        }
    }

    pub fn staged_status_cache_rev(&self) -> u64 {
        if self.staged_status_rev != 0 || !matches!(self.staged_status, Loadable::NotLoaded) {
            self.staged_status_rev
        } else {
            self.status_rev
        }
    }

    pub fn status_cache_rev(&self) -> u64 {
        let worktree = self.worktree_status_cache_rev();
        let staged = self.staged_status_cache_rev();
        if worktree == 0 && staged == 0 {
            0
        } else {
            mix_status_cache_revs([worktree, staged])
        }
    }

    pub fn worktree_status_is_loading(&self) -> bool {
        matches!(self.worktree_status, Loadable::Loading)
            || (matches!(self.worktree_status, Loadable::NotLoaded)
                && matches!(self.status, Loadable::Loading))
    }

    pub fn staged_status_is_loading(&self) -> bool {
        matches!(self.staged_status, Loadable::Loading)
            || (matches!(self.staged_status, Loadable::NotLoaded)
                && matches!(self.status, Loadable::Loading))
    }

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
        if self.history_state.retained_blame_while_loading.is_some() {
            return;
        }

        if let Loadable::Ready(lines) = &self.history_state.blame {
            self.history_state.retained_blame_while_loading = Some(Arc::clone(lines));
        }
    }

    pub(crate) fn clear_retained_blame(&mut self) {
        self.history_state.retained_blame_while_loading = None;
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

    /// Resolves the commit HEAD points at: the current branch's target when
    /// attached, else the detached HEAD commit.
    pub fn head_commit_id(&self) -> Option<CommitId> {
        if let Loadable::Ready(head_branch) = &self.head_branch
            && head_branch != "HEAD"
            && let Loadable::Ready(branches) = &self.branches
            && let Some(branch) = branches.iter().find(|b| b.name == *head_branch)
        {
            return Some(branch.target.clone());
        }
        self.detached_head_commit.clone()
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

    pub(crate) fn set_hover_commit_message(
        &mut self,
        commit_id: CommitId,
        message: Loadable<Arc<str>>,
    ) {
        self.hover_commit_message = Some((commit_id, message));
    }

    pub(crate) fn set_suggested_commit_message(&mut self, message: Option<String>) {
        self.suggested_commit_message = message;
        self.suggested_commit_message_rev = self.suggested_commit_message_rev.wrapping_add(1);
    }

    pub(crate) fn set_merge_commit_message(&mut self, v: Loadable<Option<String>>) {
        self.merge_commit_message = v;
        self.merge_message_rev = self.merge_message_rev.wrapping_add(1);
    }

    pub(crate) fn set_rebase_in_progress(&mut self, v: Loadable<bool>) {
        self.rebase_in_progress = v;
        self.merge_message_rev = self.merge_message_rev.wrapping_add(1);
    }

    pub(crate) fn set_sequencer_state(&mut self, v: Loadable<SequencerState>) {
        self.sequencer_state = v;
        self.merge_message_rev = self.merge_message_rev.wrapping_add(1);
    }

    pub(crate) fn set_upstream_divergence(&mut self, v: Loadable<Option<UpstreamDivergence>>) {
        self.upstream_divergence = v;
        self.upstream_divergence_rev = self.upstream_divergence_rev.wrapping_add(1);
    }

    pub(crate) fn set_open(&mut self, v: Loadable<()>) {
        self.open = v;
        self.open_rev = self.open_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_file_path(&mut self, v: Option<PathBuf>) {
        self.conflict_state.conflict_file_path = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_file_load_mode(&mut self, v: ConflictFileLoadMode) {
        if self.conflict_state.conflict_file_load_mode == v {
            return;
        }
        self.conflict_state.conflict_file_load_mode = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_file(&mut self, v: Loadable<Option<ConflictFile>>) {
        self.conflict_state.conflict_file = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_session(&mut self, v: Option<ConflictSession>) {
        self.conflict_state.conflict_session = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn set_conflict_hide_resolved(&mut self, v: bool) {
        if self.conflict_state.conflict_hide_resolved == v {
            return;
        }
        self.conflict_state.conflict_hide_resolved = v;
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

    pub(crate) fn bump_conflict_rev(&mut self) {
        self.conflict_state.conflict_rev = self.conflict_state.conflict_rev.wrapping_add(1);
    }

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

    pub(crate) fn set_diff_target(&mut self, target: Option<DiffTarget>) {
        if self.diff_state.diff_target != target {
            self.diff_state.diff_target_rev = self.diff_state.diff_target_rev.wrapping_add(1);
        }
        // The override lasts while its file stays open, across views of it.
        let keeps_override = self.diff_state.text_override.as_ref().is_none_or(|open| {
            target.as_ref().and_then(DiffTarget::file_path) == Some(open.path.as_path())
        });
        if !keeps_override {
            self.diff_state.text_override = None;
            self.bump_text_override_rev();
        }
        // Attributes belong to one path; another file must not be read under
        // them while its own are loading.
        let old_path = self
            .diff_state
            .diff_target
            .as_ref()
            .and_then(DiffTarget::file_path);
        if old_path != target.as_ref().and_then(DiffTarget::file_path)
            && !matches!(self.diff_state.text_attributes, Loadable::NotLoaded)
        {
            self.diff_state.text_attributes = Loadable::NotLoaded;
            self.diff_state.text_attributes_rev =
                self.diff_state.text_attributes_rev.wrapping_add(1);
        }
        self.diff_state.diff_target = target;
    }

    pub(crate) fn bump_text_override_rev(&mut self) {
        self.diff_state.text_override_rev = self.diff_state.text_override_rev.wrapping_add(1);
    }

    pub(crate) fn bump_diff_state_rev(&mut self) {
        self.diff_state.diff_state_rev = self.diff_state.diff_state_rev.wrapping_add(1);
    }

    pub(crate) fn bump_ops_rev(&mut self) {
        self.ops_rev = self.ops_rev.wrapping_add(1);
    }

    pub(crate) fn record_worktree_change(&mut self, paths: crate::msg::ChangedPaths) {
        self.worktree_change_rev = self.worktree_change_rev.wrapping_add(1);
        self.worktree_changed_paths = paths;
    }

    /// Which worktree paths changed after `seen_rev`, for a consumer that
    /// last looked at `worktree_change_rev == seen_rev`. Exact only one
    /// change back; a consumer that missed more than that gets
    /// [`ChangedPaths::Unknown`](crate::msg::ChangedPaths::Unknown) and must
    /// refresh broadly.
    pub fn worktree_paths_changed_since(&self, seen_rev: u64) -> crate::msg::ChangedPaths {
        if seen_rev == self.worktree_change_rev {
            crate::msg::ChangedPaths::none()
        } else if seen_rev.wrapping_add(1) == self.worktree_change_rev {
            self.worktree_changed_paths.clone()
        } else {
            crate::msg::ChangedPaths::Unknown
        }
    }

    pub(crate) fn bump_local_worktree_write_rev(&mut self) {
        self.local_worktree_write_rev = self.local_worktree_write_rev.wrapping_add(1);
    }

    /// A long-running GitComet git command that writes the checkout is still
    /// going: merge/rebase/reset family, a pull, or a commit whose hooks may
    /// rewrite files. Its watcher flush can arrive before it finishes. Not
    /// fetch, push, staging or our own editor save — counting those would pass
    /// off another program's edit as ours. Short commands (checkout, discard,
    /// stash) finish before the debounced flush and need no entry here.
    pub fn git_operation_in_flight(&self) -> bool {
        self.sequencer_actions_in_flight > 0
            || self.worktree_pull_in_flight > 0
            || self.commit_in_flight > 0
    }

    pub(crate) fn bump_load_epoch(&mut self) -> u64 {
        let previous = self.load_epoch;
        self.load_epoch = self.load_epoch.wrapping_add(1);
        self.history_state.indexed.cancel();
        self.history_state.authors.cancellation.cancel();
        previous
    }
}

fn loadable_into_arc<T>(loadable: Loadable<T>) -> Loadable<Arc<T>> {
    match loadable {
        Loadable::Ready(v) => Loadable::Ready(Arc::new(v)),
        Loadable::Loading => Loadable::Loading,
        Loadable::NotLoaded => Loadable::NotLoaded,
        Loadable::Error(e) => Loadable::Error(e),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticEntry {
    pub time: SystemTime,
    pub kind: DiagnosticKind,
    pub message: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticKind {
    Info,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RepoId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Loadable<T> {
    NotLoaded,
    Loading,
    Ready(T),
    Error(String),
}

impl<T> Loadable<T> {
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::Loading)
    }

    /// The loaded value, if there is one.
    ///
    /// Exists so the ~160 sites that only care about the `Ready` arm can say so
    /// in one line instead of spelling out a `match` with a `_ => ..` fallback,
    /// which is how the same five-line block ended up copied across the pickers.
    pub fn ready(&self) -> Option<&T> {
        match self {
            Self::Ready(value) => Some(value),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests;
