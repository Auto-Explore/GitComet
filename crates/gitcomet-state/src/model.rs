use crate::msg::RepoCommandKind;
use crate::msg::RepoPath;
use crate::session;
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictSession, ConflictStageParts, canonicalize_stage_parts,
};
use gitcomet_core::domain::*;
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::services::{
    BlameLine, ForcePushLease, SafePushAfterCommitContext, SubmoduleTrustTarget,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

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
}

impl Default for GitLogSettings {
    fn default() -> Self {
        Self {
            show_history_tags: true,
            tag_fetch_mode: GitLogTagFetchMode::OnRepositoryActivation,
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

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RepoLoadsInFlight {
    in_flight: u32,
    pending: u32,
    pending_log: Option<PendingLogLoad>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingLogLoad {
    pub scope: LogScope,
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
    }

    /// Starts the common primary-refresh batch immediately when no work is already queued or
    /// running. Callers fall back to per-load request coalescing when this returns `false`.
    pub fn request_primary_refresh_batch(&mut self) -> bool {
        if self.in_flight == 0 && self.pending == 0 && self.pending_log.is_none() {
            self.in_flight |= Self::PRIMARY_REFRESH_FLAGS;
            true
        } else {
            false
        }
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

    /// For log loads: coalesce by keeping only the latest requested `(scope, cursor)` while a log
    /// load is already in flight.
    pub fn request_log(
        &mut self,
        scope: LogScope,
        limit: usize,
        cursor: Option<LogCursor>,
    ) -> bool {
        if self.is_in_flight(Self::LOG) {
            let next = PendingLogLoad {
                scope,
                limit,
                cursor,
            };
            match &self.pending_log {
                // Scope changes invalidate older pending requests (including pagination).
                Some(existing) if existing.scope != next.scope => {
                    self.pending_log = Some(next);
                }
                // Don't let a refresh request (cursor=None) clobber a pending pagination request
                // for the same scope.
                Some(existing) if existing.cursor.is_some() && next.cursor.is_none() => {}
                _ => {
                    self.pending_log = Some(next);
                }
            }
            false
        } else {
            self.in_flight |= Self::LOG;
            true
        }
    }

    pub fn finish_log(&mut self) -> Option<PendingLogLoad> {
        self.in_flight &= !Self::LOG;
        if let Some(next) = self.pending_log.take() {
            self.in_flight |= Self::LOG;
            Some(next)
        } else {
            None
        }
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
    match payload {
        ConflictPayload::Text(text) => (None, Some(text.clone())),
        ConflictPayload::Binary(bytes) => (Some(bytes.clone()), None),
        ConflictPayload::Absent => (None, None),
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConflictFileLoadMode {
    #[default]
    CurrentOnly,
    Full,
}

// ── File browser ────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub struct FileBrowserState {
    pub source: FileSource,
    pub entries: Loadable<Arc<Vec<FileEntry>>>,
    pub expanded_dirs: HashSet<Arc<PathBuf>>,
    pub search_query: String,
    pub file_browser_rev: u64,
}

impl Default for FileBrowserState {
    fn default() -> Self {
        Self {
            source: FileSource::default(),
            entries: Loadable::NotLoaded,
            expanded_dirs: HashSet::new(),
            search_query: String::new(),
            file_browser_rev: 0,
        }
    }
}

impl FileBrowserState {
    pub fn bump_rev(&mut self) {
        self.file_browser_rev = self.file_browser_rev.wrapping_add(1);
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
/// is a full-content preview, and the selected commit. Replaying a snapshot only
/// restores view/selection state; it never re-runs operations like a checkout.
#[derive(Clone, Debug, PartialEq)]
pub struct MainViewSnapshot {
    pub diff_target: Option<DiffTarget>,
    pub content_preview: bool,
    pub selected_commit: Option<CommitId>,
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
        if self.cursor > 0 && self.entries.get(self.cursor - 1) == Some(&cur) {
            // Folding in-place made this entry match the previous one;
            // collapse to avoid a consecutive duplicate that would require
            // two back clicks to step past a closed/cleared view.
            self.entries.truncate(self.cursor);
            self.cursor -= 1;
        } else {
            self.entries[self.cursor] = cur;
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
    pub clone: Option<CloneOpState>,
    pub notifications: Vec<AppNotification>,
    pub banner_error: Option<BannerErrorState>,
    pub auth_prompt: Option<AuthPromptState>,
    pub submodule_trust_prompt: Option<SubmoduleTrustPromptState>,
    pub git_runtime: GitRuntimeState,
    pub git_log_settings: GitLogSettings,
    pub sidebar_mode: SidebarMode,
    pub default_tag_type: DefaultTagType,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BannerErrorState {
    pub repo_id: Option<RepoId>,
    pub message: String,
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
    pub stdout: String,
    pub stderr: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingCommitRetry {
    pub message: String,
    pub amend: bool,
    pub push_after_commit: bool,
}

#[derive(Clone, Debug)]
pub struct HistoryState {
    pub history_scope: LogScope,
    pub log: Loadable<Shared<LogPage>>,
    pub retained_log_while_loading: Option<Shared<LogPage>>,
    pub log_loading_more: bool,
    pub log_rev: u64,
    pub file_history_path: Option<PathBuf>,
    pub file_history: Loadable<Shared<LogPage>>,
    pub blame_path: Option<PathBuf>,
    pub blame_source: Option<BlameSource>,
    pub blame: Loadable<Shared<Vec<BlameLine>>>,
    pub selected_commit: Option<CommitId>,
    pub selected_commit_rev: u64,
    pub commit_details: Loadable<Shared<CommitDetails>>,
    pub commit_details_rev: u64,
}

impl Default for HistoryState {
    fn default() -> Self {
        Self {
            history_scope: LogScope::default(),
            log: Loadable::NotLoaded,
            retained_log_while_loading: None,
            log_loading_more: false,
            log_rev: 0,
            file_history_path: None,
            file_history: Loadable::NotLoaded,
            blame_path: None,
            blame_source: None,
            blame: Loadable::NotLoaded,
            selected_commit: None,
            selected_commit_rev: 0,
            commit_details: Loadable::NotLoaded,
            commit_details_rev: 0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DiffState {
    pub diff_target: Option<DiffTarget>,
    /// When true, the selected `diff_target` is rendered as a full-content file
    /// preview (the same renderer used for added/removed files — syntax
    /// highlighted, no green/red) rather than a diff. Set by `OpenFileContent`.
    pub content_preview: bool,
    pub diff_target_rev: u64,
    pub diff_state_rev: u64,
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
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            diff_target: None,
            content_preview: false,
            diff_target_rev: 0,
            diff_state_rev: 0,
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
        }
    }
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

#[derive(Clone, Debug)]
pub struct InlineSubmoduleDiffState {
    pub submodule_repo_path: PathBuf,
    pub parent_submodule_path: PathBuf,
    pub entries: Vec<InlineSubmoduleDiffEntry>,
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
fn mix_status_cache_revs(values: [u64; 2]) -> u64 {
    let mut acc = STATUS_CACHE_REV_MIX;
    for value in values {
        acc ^= value.wrapping_mul(STATUS_CACHE_REV_MIX);
        acc = acc.rotate_left(9).wrapping_add(STATUS_CACHE_REV_MIX);
    }
    acc
}

#[derive(Clone, Debug)]
pub struct RepoState {
    pub id: RepoId,
    pub spec: RepoSpec,
    session_workdir_key: Arc<str>,
    pub loads_in_flight: RepoLoadsInFlight,
    pub pull_in_flight: u32,
    pub push_in_flight: u32,
    pub worktrees_in_flight: u32,
    pub local_actions_in_flight: u32,
    pub commit_in_flight: u32,

    pub open: Loadable<()>,
    pub history_state: HistoryState,
    pub fetch_prune_deleted_remote_tracking_branches: bool,
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
    pub staged_status: Loadable<Arc<Vec<FileStatus>>>,
    pub staged_status_rev: u64,
    pub status: Loadable<Shared<RepoStatus>>,
    pub status_rev: u64,
    /// Cached flag: true when the current unstaged/worktree lane contains at
    /// least one `FileStatusKind::Conflicted` entry. Recomputed in
    /// `set_worktree_status` and `set_status`.
    pub has_unstaged_conflicts: bool,
    pub log: Loadable<Shared<LogPage>>,
    pub log_loading_more: bool,
    pub log_rev: u64,
    pub stashes: Loadable<Arc<Vec<StashEntry>>>,
    pub stashes_rev: u64,
    pub reflog: Loadable<Vec<ReflogEntry>>,
    pub recent_commit_messages: Loadable<Arc<Vec<RecentCommitMessage>>>,
    pub recent_commit_messages_rev: u64,
    pub rebase_in_progress: Loadable<bool>,
    pub merge_commit_message: Loadable<Option<String>>,
    pub merge_message_rev: u64,
    pub worktrees: Loadable<Arc<Vec<Worktree>>>,
    pub worktrees_rev: u64,
    pub submodules: Loadable<Arc<Vec<Submodule>>>,
    pub submodules_rev: u64,
    pub submodule_add_in_flight: Option<SubmoduleAddProgressState>,
    pub sidebar_data_request: SidebarDataRequest,
    /// Invalidates cached branch-sidebar rows when any sidebar-relevant source changes.
    pub branch_sidebar_rev: u64,
    pub file_browser: FileBrowserState,
    /// Commits the user has browsed this session (file directory pinned to a
    /// historical point). The current point is `file_browser.source`; this is the
    /// stack the badge dropdown lists. Cleared by "Go live".
    pub browse_history: Vec<CommitId>,
    /// Browser-style back/forward stack of opened file-content views, shared
    /// across files. Drives the viewer header's back/forward controls.
    pub view_history: NavStack<ViewHistoryEntry>,
    /// Broader, global back/forward stack of main-content-view snapshots
    /// (diffs, file content, the history log, commit selections). Drives the
    /// mouse side buttons.
    pub nav_history: NavStack<MainViewSnapshot>,

    pub diff_state: DiffState,
    pub conflict_state: ConflictState,

    pub open_rev: u64,
    pub ops_rev: u64,
    pub last_active_at: Option<SystemTime>,

    pub missing_on_disk: bool,
    pub last_error: Option<String>,
    pub diagnostics: Vec<DiagnosticEntry>,

    pub command_log: Vec<CommandLogEntry>,
    pub pending_commit_retry: Option<PendingCommitRetry>,
    pub load_epoch: u64,
    pub pending_force_push_lease: Option<ForcePushLease>,
}

impl RepoState {
    pub fn new_opening(id: RepoId, spec: RepoSpec) -> Self {
        let session_workdir_key = session::path_storage_key_shared(&spec.workdir);
        Self {
            id,
            spec,
            session_workdir_key,
            loads_in_flight: RepoLoadsInFlight::default(),
            pull_in_flight: 0,
            push_in_flight: 0,
            worktrees_in_flight: 0,
            local_actions_in_flight: 0,
            commit_in_flight: 0,
            open: Loadable::Loading,
            history_state: HistoryState::default(),
            fetch_prune_deleted_remote_tracking_branches: true,
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
            worktree_status_rev: 0,
            staged_status: Loadable::NotLoaded,
            staged_status_rev: 0,
            status: Loadable::NotLoaded,
            status_rev: 0,
            has_unstaged_conflicts: false,
            log: Loadable::NotLoaded,
            log_loading_more: false,
            log_rev: 0,
            stashes: Loadable::NotLoaded,
            stashes_rev: 0,
            reflog: Loadable::NotLoaded,
            recent_commit_messages: Loadable::NotLoaded,
            recent_commit_messages_rev: 0,
            rebase_in_progress: Loadable::NotLoaded,
            merge_commit_message: Loadable::NotLoaded,
            merge_message_rev: 0,
            worktrees: Loadable::NotLoaded,
            worktrees_rev: 0,
            submodules: Loadable::NotLoaded,
            submodules_rev: 0,
            submodule_add_in_flight: None,
            sidebar_data_request: SidebarDataRequest::default(),
            branch_sidebar_rev: 0,
            file_browser: FileBrowserState::default(),
            browse_history: Vec::new(),
            view_history: NavStack::default(),
            nav_history: NavStack::default(),
            diff_state: DiffState::default(),
            conflict_state: ConflictState::default(),
            open_rev: 0,
            ops_rev: 0,
            last_active_at: None,
            missing_on_disk: false,
            last_error: None,
            diagnostics: Vec::new(),
            command_log: Vec::new(),
            pending_commit_retry: None,
            load_epoch: 0,
            pending_force_push_lease: None,
        }
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
        self.pending_force_push_lease = None;
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
            Loadable::Ready(status) => Loadable::Ready(Arc::new(status.unstaged.clone())),
        };
        let next_staged = match &status {
            Loadable::NotLoaded => Loadable::NotLoaded,
            Loadable::Loading => Loadable::Loading,
            Loadable::Error(err) => Loadable::Error(err.clone()),
            Loadable::Ready(status) => Loadable::Ready(Arc::new(status.staged.clone())),
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

    pub(crate) fn set_log_loading_more(&mut self, v: bool) {
        if self.history_state.log_loading_more == v && self.log_loading_more == v {
            return;
        }
        self.history_state.log_loading_more = v;
        self.log_loading_more = v;
        self.bump_log_revs();
    }

    pub(crate) fn set_log_scope(&mut self, scope: LogScope) {
        if self.history_state.history_scope == scope {
            return;
        }
        self.history_state.history_scope = scope;
        self.bump_log_revs();
    }

    pub(crate) fn set_selected_commit(&mut self, v: Option<CommitId>) {
        self.history_state.selected_commit = v;
        self.history_state.selected_commit_rev =
            self.history_state.selected_commit_rev.wrapping_add(1);
    }

    pub(crate) fn set_commit_details(&mut self, v: Loadable<Shared<CommitDetails>>) {
        self.history_state.commit_details = v;
        self.history_state.commit_details_rev =
            self.history_state.commit_details_rev.wrapping_add(1);
    }

    pub(crate) fn set_merge_commit_message(&mut self, v: Loadable<Option<String>>) {
        self.merge_commit_message = v;
        self.merge_message_rev = self.merge_message_rev.wrapping_add(1);
    }

    pub(crate) fn set_rebase_in_progress(&mut self, v: Loadable<bool>) {
        self.rebase_in_progress = v;
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
            selected_commit: self.history_state.selected_commit.clone(),
        }
    }

    /// Whether the current main view equals `other`, compared by borrow so the
    /// nav-history reconcile can skip cloning a `MainViewSnapshot` (which owns a
    /// `PathBuf`) on the common path where the view did not move.
    pub(crate) fn main_view_snapshot_matches(&self, other: &MainViewSnapshot) -> bool {
        self.diff_state.diff_target == other.diff_target
            && self.diff_state.content_preview == other.content_preview
            && self.history_state.selected_commit == other.selected_commit
    }

    pub(crate) fn set_diff_target(&mut self, target: Option<DiffTarget>) {
        if self.diff_state.diff_target != target {
            self.diff_state.diff_target_rev = self.diff_state.diff_target_rev.wrapping_add(1);
        }
        self.diff_state.diff_target = target;
    }

    pub(crate) fn bump_diff_state_rev(&mut self) {
        self.diff_state.diff_state_rev = self.diff_state.diff_state_rev.wrapping_add(1);
    }

    pub(crate) fn bump_ops_rev(&mut self) {
        self.ops_rev = self.ops_rev.wrapping_add(1);
    }

    pub(crate) fn bump_load_epoch(&mut self) -> u64 {
        let previous = self.load_epoch;
        self.load_epoch = self.load_epoch.wrapping_add(1);
        previous
    }
}

fn loadable_into_arc<T>(loadable: Loadable<Vec<T>>) -> Loadable<Arc<Vec<T>>> {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn entry(name: &str) -> ViewHistoryEntry {
        ViewHistoryEntry {
            source: FileSource::Commit(crate::model::CommitId(name.into())),
            path: PathBuf::from("src/lib.rs"),
        }
    }

    #[test]
    fn nav_stack_reconcile_seeds_origin_pushes_and_updates_in_place() {
        let history_view = MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            selected_commit: None,
        };
        let commit_view = MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            selected_commit: Some(CommitId("aaa".into())),
        };
        let file_view = MainViewSnapshot {
            diff_target: Some(DiffTarget::Commit {
                commit_id: CommitId("aaa".into()),
                path: Some(PathBuf::from("src/lib.rs")),
            }),
            content_preview: false,
            selected_commit: Some(CommitId("aaa".into())),
        };

        let mut h: NavStack<MainViewSnapshot> = NavStack::default();
        // Pre-change sync on the first navigation seeds the origin (history log).
        h.reconcile(history_view.clone(), false);
        // Selecting a commit then opening a file each push a distinct step.
        h.reconcile(commit_view.clone(), true);
        h.reconcile(file_view.clone(), true);
        assert_eq!(
            h.entries,
            vec![history_view.clone(), commit_view.clone(), file_view.clone()]
        );
        assert_eq!(h.cursor, 2);

        // Back steps one-by-one: file diff → commit details → history log.
        assert_eq!(h.step(ViewNavDir::Back), Some(commit_view.clone()));
        assert_eq!(h.step(ViewNavDir::Back), Some(history_view.clone()));
        assert!(!h.can_back());
        // Forward reopens them one-by-one.
        assert_eq!(h.step(ViewNavDir::Forward), Some(commit_view.clone()));
        assert_eq!(h.step(ViewNavDir::Forward), Some(file_view.clone()));

        // A non-push (background) change folds into the current entry without
        // adding a step or dropping forward history.
        let reloaded_file_view = MainViewSnapshot {
            content_preview: true,
            ..file_view.clone()
        };
        h.reconcile(reloaded_file_view.clone(), false);
        assert_eq!(h.entries.len(), 3, "in-place update must not add a step");
        assert_eq!(h.entries[2], reloaded_file_view);
        assert_eq!(h.cursor, 2);
    }

    #[test]
    fn view_history_records_and_navigates() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        assert!(!h.can_back());
        assert!(!h.can_forward());

        h.record(entry("a"));
        h.record(entry("b"));
        h.record(entry("c"));
        assert_eq!(h.cursor, 2);
        assert!(h.can_back());
        assert!(!h.can_forward());

        // Step back to b, then a.
        assert_eq!(h.step(ViewNavDir::Back), Some(entry("b")));
        assert_eq!(h.step(ViewNavDir::Back), Some(entry("a")));
        assert!(!h.can_back());
        // Clamped at the start.
        assert_eq!(h.step(ViewNavDir::Back), None);

        // Forward again to b.
        assert_eq!(h.step(ViewNavDir::Forward), Some(entry("b")));
        assert!(h.can_forward());
    }

    #[test]
    fn view_history_new_open_truncates_forward() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("b"));
        h.record(entry("c"));
        // Go back to a, then open a new view: forward (b, c) is dropped.
        h.step(ViewNavDir::Back);
        h.step(ViewNavDir::Back);
        h.record(entry("d"));
        assert_eq!(
            h.entries,
            vec![entry("a"), entry("d")],
            "forward history past the cursor is truncated on a new open"
        );
        assert_eq!(h.cursor, 1);
        assert!(!h.can_forward());
    }

    #[test]
    fn seek_or_record_moves_cursor_to_existing_entry_without_mutating() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("b"));
        h.record(entry("c"));
        // Realign onto an entry already present: only the cursor moves.
        h.seek_or_record(entry("a"));
        assert_eq!(h.cursor, 0);
        assert_eq!(h.entries, vec![entry("a"), entry("b"), entry("c")]);
        assert!(h.can_forward());
    }

    #[test]
    fn seek_or_record_appends_when_entry_absent() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("b"));
        // An entry not in the stack is recorded as a fresh destination.
        h.seek_or_record(entry("z"));
        assert_eq!(h.entries, vec![entry("a"), entry("b"), entry("z")]);
        assert_eq!(h.cursor, 2);
    }

    #[test]
    fn view_history_dedupes_repeat_of_current() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("a"));
        assert_eq!(h.entries, vec![entry("a")]);
        assert_eq!(h.cursor, 0);
    }

    #[test]
    fn view_history_caps_length() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        for i in 0..(NAV_HISTORY_CAP + 5) {
            h.record(entry(&format!("c{i}")));
        }
        assert_eq!(h.entries.len(), NAV_HISTORY_CAP);
        assert_eq!(h.cursor, NAV_HISTORY_CAP - 1);
        // Oldest entries were evicted; newest is current.
        assert_eq!(
            h.entries.last(),
            Some(&entry(&format!("c{}", NAV_HISTORY_CAP + 4)))
        );
    }

    #[test]
    fn reconcile_leaves_history_intact_when_parked_mid_stack() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("b"));
        h.record(entry("c"));
        // Navigate back to the middle entry "b".
        assert_eq!(h.step(ViewNavDir::Back), Some(entry("b")));
        assert_eq!(h.cursor, 1);

        // A background (non-push) change to a different snapshot must not rewrite
        // the historical entry nor drop the forward entry "c".
        h.reconcile(entry("x"), false);
        assert_eq!(
            h.entries,
            vec![entry("a"), entry("b"), entry("c")],
            "background change while parked mid-stack must not mutate saved history"
        );
        assert_eq!(h.cursor, 1);
        assert!(h.can_forward());
        assert_eq!(h.step(ViewNavDir::Forward), Some(entry("c")));
    }

    #[test]
    fn reconcile_folds_background_change_into_live_tail() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("b"));
        // At the live tail a background change still folds in place (no new step).
        h.reconcile(entry("b2"), false);
        assert_eq!(h.entries, vec![entry("a"), entry("b2")]);
        assert_eq!(h.cursor, 1);
    }

    #[test]
    fn clear_resets_stack() {
        let mut h: NavStack<ViewHistoryEntry> = NavStack::default();
        h.record(entry("a"));
        h.record(entry("b"));
        h.clear();
        assert!(h.entries.is_empty());
        assert_eq!(h.cursor, 0);
        assert!(!h.can_back());
        assert!(!h.can_forward());
    }

    #[test]
    fn reconcile_fold_collapses_consecutive_duplicate() {
        let history_log = MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            selected_commit: None,
        };
        let commit_view = MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            selected_commit: Some(CommitId("aaa".into())),
        };
        let file_diff = MainViewSnapshot {
            diff_target: Some(DiffTarget::Commit {
                commit_id: CommitId("aaa".into()),
                path: Some(PathBuf::from("src/lib.rs")),
            }),
            content_preview: false,
            selected_commit: Some(CommitId("aaa".into())),
        };

        let mut h: NavStack<MainViewSnapshot> = NavStack::default();
        h.reconcile(history_log.clone(), false);
        h.reconcile(commit_view.clone(), true);
        h.reconcile(file_diff.clone(), true);
        assert_eq!(h.cursor, 2);
        assert_eq!(h.entries.len(), 3);

        // Folding the cleared-diff state in-place makes it match the
        // previous entry (commit details), so the stack collapses back
        // to that entry instead of creating a consecutive duplicate.
        h.reconcile(commit_view.clone(), false);
        assert_eq!(
            h.entries.len(),
            2,
            "duplicate entries collapsed into original"
        );
        assert_eq!(h.cursor, 1);
        assert_eq!(h.entries[1], commit_view);
    }

    #[test]
    fn reconcile_fold_no_collapse_when_not_adjacent_duplicate() {
        let view_a = MainViewSnapshot {
            diff_target: Some(DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Unstaged,
            }),
            content_preview: false,
            selected_commit: None,
        };
        let view_b = MainViewSnapshot {
            diff_target: Some(DiffTarget::WorkingTree {
                path: PathBuf::from("b.txt"),
                area: DiffArea::Unstaged,
            }),
            content_preview: false,
            selected_commit: None,
        };

        let mut h: NavStack<MainViewSnapshot> = NavStack::default();
        let empty = MainViewSnapshot {
            diff_target: None,
            content_preview: false,
            selected_commit: None,
        };
        h.reconcile(empty.clone(), false);
        h.reconcile(view_a.clone(), true);
        h.reconcile(view_b.clone(), true);
        assert_eq!(h.entries.len(), 3);

        // Folding into an adjacent entry that is *different* does not
        // collapse — it overwrites in place.
        let changed = MainViewSnapshot {
            content_preview: true,
            ..view_b.clone()
        };
        h.reconcile(changed.clone(), false);
        assert_eq!(h.entries.len(), 3, "no collapse when adjacent differs");
        assert_eq!(h.entries[2], changed);
        assert_eq!(h.cursor, 2);
    }

    #[test]
    fn browsing_commit_reflects_file_browser_source() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        );
        assert_eq!(repo.browsing_commit(), None);

        repo.file_browser.source = FileSource::Commit(CommitId("abc123".into()));
        assert_eq!(repo.browsing_commit(), Some(&CommitId("abc123".into())));

        repo.file_browser.source = FileSource::WorkingDirectory;
        assert_eq!(repo.browsing_commit(), None);
    }

    #[test]
    fn app_state_clone_shares_heavy_repo_fields_via_arc() {
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));

        let repo = &mut state.repos[0];
        repo.status = Loadable::Ready(Arc::new(RepoStatus::default()));
        repo.history_state.log = Loadable::Ready(Arc::new(LogPage {
            commits: vec![Commit {
                id: CommitId("c1".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "s1".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        }));
        repo.history_state.file_history = Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }));
        repo.history_state.blame = Loadable::Ready(Arc::new(vec![BlameLine {
            commit_id: "c1".into(),
            author: "a".into(),
            author_time_unix: None,
            summary: "s1".into(),
            body: None,
            line: "line".to_string(),
            prior_exists: true,
            source_path: None,
            prior_commit: None,
        }]));
        repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
            id: CommitId("c1".into()),
            message: "m".to_string(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "t".to_string(),
            parent_ids: Vec::new(),
            files: Vec::new(),
        }));
        repo.diff_state.diff = Loadable::Ready(Arc::new(Diff {
            target: DiffTarget::Commit {
                commit_id: CommitId("c1".into()),
                path: None,
            },
            lines: Vec::new(),
        }));

        let cloned = state.clone();

        let repo1 = &state.repos[0];
        let repo2 = &cloned.repos[0];

        let Loadable::Ready(status1) = &repo1.status else {
            panic!("expected status ready");
        };
        let Loadable::Ready(status2) = &repo2.status else {
            panic!("expected status ready");
        };
        assert!(Arc::ptr_eq(status1, status2));
        assert_eq!(Arc::strong_count(status1), 2);

        let Loadable::Ready(log1) = &repo1.history_state.log else {
            panic!("expected log ready");
        };
        let Loadable::Ready(log2) = &repo2.history_state.log else {
            panic!("expected log ready");
        };
        assert!(Arc::ptr_eq(log1, log2));
        assert_eq!(Arc::strong_count(log1), 2);

        let Loadable::Ready(diff1) = &repo1.diff_state.diff else {
            panic!("expected diff ready");
        };
        let Loadable::Ready(diff2) = &repo2.diff_state.diff else {
            panic!("expected diff ready");
        };
        assert!(Arc::ptr_eq(diff1, diff2));
        assert_eq!(Arc::strong_count(diff1), 2);
    }

    fn new_repo() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    fn file_status(path: &str, kind: FileStatusKind) -> FileStatus {
        FileStatus {
            path: PathBuf::from(path),
            kind,
            conflict: None,
        }
    }

    #[test]
    fn request_primary_refresh_batch_marks_all_primary_loads_when_idle() {
        let mut loads = RepoLoadsInFlight::default();

        assert!(loads.request_primary_refresh_batch());
        assert!(loads.is_in_flight(RepoLoadsInFlight::HEAD_BRANCH));
        assert!(loads.is_in_flight(RepoLoadsInFlight::UPSTREAM_DIVERGENCE));
        assert!(loads.is_in_flight(RepoLoadsInFlight::REBASE_STATE));
        assert!(loads.is_in_flight(RepoLoadsInFlight::MERGE_COMMIT_MESSAGE));
        assert!(loads.is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS));
        assert!(loads.is_in_flight(RepoLoadsInFlight::STAGED_STATUS));
        assert!(loads.is_in_flight(RepoLoadsInFlight::LOG));
    }

    #[test]
    fn request_primary_refresh_batch_skips_when_any_load_is_already_in_flight() {
        let mut loads = RepoLoadsInFlight::default();
        assert!(loads.request(RepoLoadsInFlight::WORKTREE_STATUS));

        assert!(!loads.request_primary_refresh_batch());
        assert!(!loads.is_in_flight(RepoLoadsInFlight::HEAD_BRANCH));
        assert!(loads.is_in_flight(RepoLoadsInFlight::WORKTREE_STATUS));
        assert!(!loads.is_in_flight(RepoLoadsInFlight::LOG));
    }

    #[test]
    fn request_log_scope_change_replaces_pending_log_request() {
        let mut loads = RepoLoadsInFlight::default();
        assert!(loads.request_log(LogScope::FullReachable, 20, None));

        assert!(!loads.request_log(
            LogScope::AllBranches,
            20,
            Some(LogCursor {
                last_seen: CommitId("older".into()),
                resume_from: None,
                resume_token: None,
            }),
        ));
        assert!(!loads.request_log(LogScope::NoMerges, 20, None));

        assert_eq!(
            loads.finish_log(),
            Some(PendingLogLoad {
                scope: LogScope::NoMerges,
                limit: 20,
                cursor: None,
            })
        );
    }

    #[test]
    fn request_log_same_scope_refresh_does_not_clobber_pending_pagination() {
        let mut loads = RepoLoadsInFlight::default();
        let cursor = LogCursor {
            last_seen: CommitId("page-1".into()),
            resume_from: None,
            resume_token: None,
        };

        assert!(loads.request_log(LogScope::MergesOnly, 20, None));
        assert!(!loads.request_log(LogScope::MergesOnly, 20, Some(cursor.clone())));
        assert!(!loads.request_log(LogScope::MergesOnly, 20, None));

        assert_eq!(
            loads.finish_log(),
            Some(PendingLogLoad {
                scope: LogScope::MergesOnly,
                limit: 20,
                cursor: Some(cursor),
            })
        );
    }

    #[test]
    fn set_spec_refreshes_session_workdir_key() {
        let mut repo = RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo-a"),
            },
        );
        assert_eq!(repo.session_workdir_key().as_ref(), "/tmp/repo-a");

        repo.set_spec(RepoSpec {
            workdir: PathBuf::from("/tmp/repo-b"),
        });

        assert_eq!(repo.spec.workdir, PathBuf::from("/tmp/repo-b"));
        assert_eq!(repo.session_workdir_key().as_ref(), "/tmp/repo-b");
    }

    // --- Setter rev-bump tests ---

    #[test]
    fn set_status_bumps_status_rev() {
        let mut repo = new_repo();
        let before = repo.status_rev;
        repo.set_status(Loadable::Loading);
        assert_eq!(repo.status_rev, before + 1);
        repo.set_status(Loadable::Ready(Arc::new(RepoStatus::default())));
        assert_eq!(repo.status_rev, before + 2);
    }

    #[test]
    fn split_status_setters_do_not_bump_legacy_status_rev() {
        let mut repo = new_repo();
        repo.set_status(Loadable::Loading);
        let status_rev = repo.status_rev;

        repo.set_worktree_status(Loadable::Ready(vec![file_status(
            "src/lib.rs",
            FileStatusKind::Modified,
        )]));
        assert_eq!(repo.status_rev, status_rev);

        repo.set_staged_status(Loadable::Ready(vec![file_status(
            "src/lib.rs",
            FileStatusKind::Added,
        )]));
        assert_eq!(repo.status_rev, status_rev);
    }

    #[test]
    fn status_entry_for_path_prefers_split_lane_entries() {
        let mut repo = new_repo();
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged: vec![file_status("legacy.rs", FileStatusKind::Modified)],
            staged: vec![file_status("legacy-stage.rs", FileStatusKind::Added)],
        }));
        repo.status_rev = 1;
        repo.set_worktree_status(Loadable::Ready(vec![file_status(
            "split.rs",
            FileStatusKind::Deleted,
        )]));

        let entry = repo
            .status_entry_for_path(DiffArea::Unstaged, std::path::Path::new("split.rs"))
            .expect("split lane entry");
        assert_eq!(entry.kind, FileStatusKind::Deleted);
        assert!(
            repo.status_entry_for_path(DiffArea::Unstaged, std::path::Path::new("legacy.rs"))
                .is_none()
        );
    }

    #[test]
    fn status_cache_rev_changes_with_split_lane_revisions() {
        let mut repo = new_repo();
        let initial = repo.status_cache_rev();
        repo.set_worktree_status(Loadable::Loading);
        let after_worktree = repo.status_cache_rev();
        assert_ne!(after_worktree, initial);

        repo.set_staged_status(Loadable::Loading);
        assert_ne!(repo.status_cache_rev(), after_worktree);
    }

    #[test]
    fn set_log_bumps_log_rev() {
        let mut repo = new_repo();
        let before = (repo.log_rev, repo.history_state.log_rev);
        repo.set_log(Loadable::Loading);
        assert_eq!(repo.log_rev, before.0 + 1);
        assert_eq!(repo.history_state.log_rev, before.1 + 1);
    }

    #[test]
    fn retain_log_while_loading_keeps_ready_log_alive_until_next_log_state() {
        let mut repo = new_repo();
        let page = Arc::new(LogPage {
            commits: vec![Commit {
                id: CommitId("c1".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "s1".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        });
        repo.set_log(Loadable::Ready(Arc::clone(&page)));

        repo.retain_log_while_loading();
        repo.set_log(Loadable::Loading);

        let retained = repo
            .history_state
            .retained_log_while_loading
            .as_ref()
            .expect("ready log should stay retained while loading");
        assert!(Arc::ptr_eq(retained, &page));

        repo.set_log(Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        })));

        assert!(repo.history_state.retained_log_while_loading.is_none());
    }

    #[test]
    fn set_log_loading_more_bumps_log_rev() {
        let mut repo = new_repo();
        let before = (repo.log_rev, repo.history_state.log_rev);
        repo.set_log_loading_more(true);
        assert_eq!(repo.log_rev, before.0 + 1);
        assert_eq!(repo.history_state.log_rev, before.1 + 1);
        repo.set_log_loading_more(false);
        assert_eq!(repo.log_rev, before.0 + 2);
        assert_eq!(repo.history_state.log_rev, before.1 + 2);
    }

    #[test]
    fn set_log_scope_bumps_log_rev() {
        let mut repo = new_repo();
        let before = (repo.log_rev, repo.history_state.log_rev);
        repo.set_log_scope(LogScope::AllBranches);
        assert_eq!(repo.log_rev, before.0 + 1);
        assert_eq!(repo.history_state.log_rev, before.1 + 1);
    }

    #[test]
    fn set_selected_commit_bumps_selected_commit_rev() {
        let mut repo = new_repo();
        let before = repo.history_state.selected_commit_rev;
        repo.set_selected_commit(Some(CommitId("abc".into())));
        assert_eq!(repo.history_state.selected_commit_rev, before + 1);
        repo.set_selected_commit(None);
        assert_eq!(repo.history_state.selected_commit_rev, before + 2);
    }

    #[test]
    fn set_commit_details_bumps_commit_details_rev() {
        let mut repo = new_repo();
        let before = repo.history_state.commit_details_rev;
        repo.set_commit_details(Loadable::Loading);
        assert_eq!(repo.history_state.commit_details_rev, before + 1);
    }

    #[test]
    fn set_merge_commit_message_bumps_merge_message_rev() {
        let mut repo = new_repo();
        let before = repo.merge_message_rev;
        repo.set_merge_commit_message(Loadable::Ready(Some("merge".to_string())));
        assert_eq!(repo.merge_message_rev, before + 1);
    }

    #[test]
    fn set_rebase_in_progress_bumps_merge_message_rev() {
        let mut repo = new_repo();
        let before = repo.merge_message_rev;
        repo.set_rebase_in_progress(Loadable::Ready(true));
        assert_eq!(repo.merge_message_rev, before + 1);
    }

    #[test]
    fn merge_message_and_rebase_share_same_rev_counter() {
        let mut repo = new_repo();
        let before = repo.merge_message_rev;
        repo.set_merge_commit_message(Loadable::Ready(None));
        repo.set_rebase_in_progress(Loadable::Ready(false));
        assert_eq!(repo.merge_message_rev, before + 2);
    }

    #[test]
    fn set_upstream_divergence_bumps_upstream_divergence_rev() {
        let mut repo = new_repo();
        let before = repo.upstream_divergence_rev;
        repo.set_upstream_divergence(Loadable::Loading);
        assert_eq!(repo.upstream_divergence_rev, before + 1);
    }

    #[test]
    fn set_open_bumps_open_rev() {
        let mut repo = new_repo();
        let before = repo.open_rev;
        repo.set_open(Loadable::Ready(()));
        assert_eq!(repo.open_rev, before + 1);
    }

    #[test]
    fn set_conflict_file_path_bumps_conflict_rev() {
        let mut repo = new_repo();
        let before = repo.conflict_state.conflict_rev;
        repo.set_conflict_file_path(Some(PathBuf::from("file.rs")));
        assert_eq!(repo.conflict_state.conflict_rev, before + 1);
    }

    #[test]
    fn set_conflict_file_bumps_conflict_rev() {
        let mut repo = new_repo();
        let before = repo.conflict_state.conflict_rev;
        repo.set_conflict_file(Loadable::Loading);
        assert_eq!(repo.conflict_state.conflict_rev, before + 1);
    }

    #[test]
    fn conflict_file_path_and_file_share_same_rev_counter() {
        let mut repo = new_repo();
        let before = repo.conflict_state.conflict_rev;
        repo.set_conflict_file_path(Some(PathBuf::from("a.rs")));
        repo.set_conflict_file(Loadable::Loading);
        assert_eq!(repo.conflict_state.conflict_rev, before + 2);
    }

    #[test]
    fn set_conflict_file_load_mode_bumps_conflict_rev_only_on_change() {
        let mut repo = new_repo();
        let before = repo.conflict_state.conflict_rev;
        repo.set_conflict_file_load_mode(ConflictFileLoadMode::Full);
        assert_eq!(
            repo.conflict_state.conflict_file_load_mode,
            ConflictFileLoadMode::Full
        );
        assert_eq!(repo.conflict_state.conflict_rev, before + 1);

        repo.set_conflict_file_load_mode(ConflictFileLoadMode::Full);
        assert_eq!(repo.conflict_state.conflict_rev, before + 1);
    }

    #[test]
    fn set_conflict_hide_resolved_bumps_conflict_rev_only_on_change() {
        let mut repo = new_repo();
        let before = repo.conflict_state.conflict_rev;
        repo.set_conflict_hide_resolved(true);
        assert!(repo.conflict_state.conflict_hide_resolved);
        assert_eq!(repo.conflict_state.conflict_rev, before + 1);
        repo.set_conflict_hide_resolved(true);
        assert_eq!(repo.conflict_state.conflict_rev, before + 1);
        repo.set_conflict_hide_resolved(false);
        assert!(!repo.conflict_state.conflict_hide_resolved);
        assert_eq!(repo.conflict_state.conflict_rev, before + 2);
    }

    #[test]
    fn bump_diff_state_rev_increments() {
        let mut repo = new_repo();
        let before = repo.diff_state.diff_state_rev;
        repo.bump_diff_state_rev();
        assert_eq!(repo.diff_state.diff_state_rev, before + 1);
        repo.bump_diff_state_rev();
        assert_eq!(repo.diff_state.diff_state_rev, before + 2);
    }

    #[test]
    fn set_diff_target_bumps_target_rev_only_on_change() {
        let mut repo = new_repo();
        let target = DiffTarget::WorkingTree {
            path: PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };

        repo.set_diff_target(Some(target.clone()));
        assert_eq!(repo.diff_state.diff_target, Some(target.clone()));
        assert_eq!(repo.diff_state.diff_target_rev, 1);

        repo.set_diff_target(Some(target));
        assert_eq!(repo.diff_state.diff_target_rev, 1);

        repo.set_diff_target(None);
        assert!(repo.diff_state.diff_target.is_none());
        assert_eq!(repo.diff_state.diff_target_rev, 2);
    }

    #[test]
    fn bump_ops_rev_increments() {
        let mut repo = new_repo();
        let before = repo.ops_rev;
        repo.bump_ops_rev();
        assert_eq!(repo.ops_rev, before + 1);
        repo.bump_ops_rev();
        assert_eq!(repo.ops_rev, before + 2);
    }

    // --- Equality-guard tests: setters that skip rev bump on no-change ---

    #[test]
    fn set_head_branch_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_head_branch(Loadable::Ready("main".to_string()));
        let rev_after_first = repo.head_branch_rev;
        repo.set_head_branch(Loadable::Ready("main".to_string()));
        assert_eq!(
            repo.head_branch_rev, rev_after_first,
            "rev should not bump for same value"
        );
    }

    #[test]
    fn set_head_branch_bumps_rev_when_changed() {
        let mut repo = new_repo();
        repo.set_head_branch(Loadable::Ready("main".to_string()));
        let rev_after_first = repo.head_branch_rev;
        repo.set_head_branch(Loadable::Ready("develop".to_string()));
        assert_eq!(repo.head_branch_rev, rev_after_first + 1);
    }

    #[test]
    fn branch_sidebar_cache_rev_falls_back_to_component_revisions() {
        let mut repo = new_repo();
        let initial = repo.branch_sidebar_cache_rev();
        repo.branches_rev = 1;
        assert_ne!(repo.branch_sidebar_cache_rev(), initial);
    }

    #[test]
    fn branch_sidebar_cache_rev_bumps_only_for_relevant_changes() {
        let mut repo = new_repo();
        let initial = repo.branch_sidebar_cache_rev();

        repo.set_head_branch(Loadable::Ready("main".to_string()));
        let after_head = repo.branch_sidebar_cache_rev();
        assert_ne!(after_head, initial);

        repo.set_head_branch(Loadable::Ready("main".to_string()));
        assert_eq!(repo.branch_sidebar_cache_rev(), after_head);

        repo.set_worktrees(Loadable::Loading);
        assert_ne!(repo.branch_sidebar_cache_rev(), after_head);
    }

    #[test]
    fn set_detached_head_commit_updates_only_on_change() {
        let mut repo = new_repo();
        let head = CommitId("abc123".into());
        repo.set_detached_head_commit(Some(head.clone()));
        assert_eq!(repo.detached_head_commit, Some(head.clone()));

        repo.set_detached_head_commit(Some(head.clone()));
        assert_eq!(repo.detached_head_commit, Some(head));

        repo.set_detached_head_commit(None);
        assert!(repo.detached_head_commit.is_none());
    }

    #[test]
    fn set_branches_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_branches(Loadable::NotLoaded);
        let rev = repo.branches_rev;
        repo.set_branches(Loadable::NotLoaded);
        assert_eq!(
            repo.branches_rev, rev,
            "rev should not bump for same Loadable variant"
        );
    }

    #[test]
    fn set_tags_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_tags(Loadable::NotLoaded);
        let rev = repo.tags_rev;
        repo.set_tags(Loadable::NotLoaded);
        assert_eq!(repo.tags_rev, rev);
    }

    #[test]
    fn set_remotes_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_remotes(Loadable::Loading);
        let rev = repo.remotes_rev;
        repo.set_remotes(Loadable::Loading);
        assert_eq!(repo.remotes_rev, rev);
    }

    #[test]
    fn set_stashes_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_stashes(Loadable::Loading);
        let rev = repo.stashes_rev;
        repo.set_stashes(Loadable::Loading);
        assert_eq!(repo.stashes_rev, rev);
    }

    #[test]
    fn set_worktrees_bumps_rev_when_changed() {
        let mut repo = new_repo();
        let before = repo.worktrees_rev;
        repo.set_worktrees(Loadable::Loading);
        assert_eq!(repo.worktrees_rev, before + 1);
        repo.set_worktrees(Loadable::Ready(vec![]));
        assert_eq!(repo.worktrees_rev, before + 2);
    }

    #[test]
    fn set_submodules_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_submodules(Loadable::Loading);
        let rev = repo.submodules_rev;
        repo.set_submodules(Loadable::Loading);
        assert_eq!(repo.submodules_rev, rev);
    }

    #[test]
    fn set_status_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_status(Loadable::Loading);
        let rev = repo.status_rev;
        repo.set_status(Loadable::Loading);
        assert_eq!(repo.status_rev, rev);
    }

    #[test]
    fn set_log_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_log(Loadable::Loading);
        let rev = (repo.log_rev, repo.history_state.log_rev);
        repo.set_log(Loadable::Loading);
        assert_eq!(repo.log_rev, rev.0);
        assert_eq!(repo.history_state.log_rev, rev.1);
    }

    #[test]
    fn set_log_loading_more_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_log_loading_more(true);
        let rev = (repo.log_rev, repo.history_state.log_rev);
        repo.set_log_loading_more(true);
        assert_eq!(repo.log_rev, rev.0);
        assert_eq!(repo.history_state.log_rev, rev.1);
    }

    #[test]
    fn set_log_scope_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_log_scope(LogScope::AllBranches);
        let rev = (repo.log_rev, repo.history_state.log_rev);
        repo.set_log_scope(LogScope::AllBranches);
        assert_eq!(repo.log_rev, rev.0);
        assert_eq!(repo.history_state.log_rev, rev.1);
    }

    // --- Isolation tests: one setter does not bump another's rev ---

    #[test]
    fn setters_only_bump_their_own_rev_counter() {
        let mut repo = new_repo();
        let snap = (
            repo.status_rev,
            repo.log_rev,
            repo.history_state.log_rev,
            repo.history_state.selected_commit_rev,
            repo.history_state.commit_details_rev,
            repo.merge_message_rev,
            repo.upstream_divergence_rev,
            repo.open_rev,
            repo.conflict_state.conflict_rev,
            repo.diff_state.diff_target_rev,
            repo.diff_state.diff_state_rev,
            repo.ops_rev,
        );

        repo.set_status(Loadable::Loading);
        assert_eq!(repo.status_rev, snap.0 + 1);
        assert_eq!(repo.log_rev, snap.1);
        assert_eq!(repo.history_state.log_rev, snap.2);
        assert_eq!(repo.history_state.selected_commit_rev, snap.3);
        assert_eq!(repo.history_state.commit_details_rev, snap.4);
        assert_eq!(repo.merge_message_rev, snap.5);
        assert_eq!(repo.upstream_divergence_rev, snap.6);
        assert_eq!(repo.open_rev, snap.7);
        assert_eq!(repo.conflict_state.conflict_rev, snap.8);
        assert_eq!(repo.diff_state.diff_target_rev, snap.9);
        assert_eq!(repo.diff_state.diff_state_rev, snap.10);
        assert_eq!(repo.ops_rev, snap.11);
    }

    #[test]
    fn all_rev_counters_start_at_zero() {
        let repo = new_repo();
        assert_eq!(repo.status_rev, 0);
        assert_eq!(repo.log_rev, 0);
        assert_eq!(repo.history_state.log_rev, 0);
        assert_eq!(repo.history_state.selected_commit_rev, 0);
        assert_eq!(repo.history_state.commit_details_rev, 0);
        assert_eq!(repo.merge_message_rev, 0);
        assert_eq!(repo.upstream_divergence_rev, 0);
        assert_eq!(repo.open_rev, 0);
        assert_eq!(repo.conflict_state.conflict_rev, 0);
        assert_eq!(repo.diff_state.diff_target_rev, 0);
        assert_eq!(repo.diff_state.diff_state_rev, 0);
        assert_eq!(repo.ops_rev, 0);
        assert_eq!(repo.head_branch_rev, 0);
        assert_eq!(repo.branches_rev, 0);
        assert_eq!(repo.tags_rev, 0);
        assert_eq!(repo.remotes_rev, 0);
        assert_eq!(repo.remote_branches_rev, 0);
        assert_eq!(repo.stashes_rev, 0);
        assert_eq!(repo.worktrees_rev, 0);
        assert_eq!(repo.submodules_rev, 0);
        assert_eq!(repo.branch_sidebar_rev, 0);
    }

    #[test]
    fn grouped_state_defaults_are_initialized() {
        let repo = new_repo();
        assert_eq!(repo.history_state.history_scope, LogScope::FullReachable);
        assert!(matches!(repo.history_state.log, Loadable::NotLoaded));
        assert!(matches!(
            repo.history_state.file_history,
            Loadable::NotLoaded
        ));
        assert!(matches!(repo.history_state.blame, Loadable::NotLoaded));

        assert!(repo.diff_state.diff_target.is_none());
        assert!(matches!(repo.diff_state.diff, Loadable::NotLoaded));
        assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
        assert!(matches!(
            repo.diff_state.diff_file_image,
            Loadable::NotLoaded
        ));

        assert!(repo.conflict_state.conflict_file_path.is_none());
        assert!(matches!(
            repo.conflict_state.conflict_file,
            Loadable::NotLoaded
        ));
        assert!(repo.conflict_state.conflict_session.is_none());
        assert!(!repo.conflict_state.conflict_hide_resolved);
        assert!(repo.detached_head_commit.is_none());
        assert_eq!(repo.sidebar_data_request, SidebarDataRequest::default());
    }
}
