use crate::msg::RepoCommandKind;
use crate::msg::RepoPath;
use crate::session;
use gitcomet_core::conflict_session::{
    ConflictPayload, ConflictSession, ConflictStageParts, canonicalize_stage_parts,
};
use gitcomet_core::domain::*;
use gitcomet_core::services::{BlameLine, SubmoduleTrustTarget};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

pub type Shared<T> = Arc<T>;

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
    pub const STATUS: u32 = 1 << 6;
    pub const STASHES: u32 = 1 << 7;
    pub const REFLOG: u32 = 1 << 8;
    pub const REBASE_STATE: u32 = 1 << 9;
    pub const LOG: u32 = 1 << 10;
    pub const MERGE_COMMIT_MESSAGE: u32 = 1 << 11;
    pub const REMOTE_TAGS: u32 = 1 << 12;
    const PRIMARY_REFRESH_FLAGS: u32 = Self::HEAD_BRANCH
        | Self::UPSTREAM_DIVERGENCE
        | Self::REBASE_STATE
        | Self::MERGE_COMMIT_MESSAGE
        | Self::STATUS
        | Self::LOG;

    pub fn is_in_flight(&self, flag: u32) -> bool {
        (self.in_flight & flag) != 0
    }

    pub fn any_in_flight(&self) -> bool {
        self.in_flight != 0
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

#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub repos: Vec<RepoState>,
    pub active_repo: Option<RepoId>,
    pub clone: Option<CloneOpState>,
    pub notifications: Vec<AppNotification>,
    pub banner_error: Option<BannerErrorState>,
    pub auth_prompt: Option<AuthPromptState>,
    pub submodule_trust_prompt: Option<SubmoduleTrustPromptState>,
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
    Commit {
        repo_id: RepoId,
        message: String,
        amend: bool,
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
    pub seq: u64,
    pub output_tail: VecDeque<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CloneOpStatus {
    Running,
    FinishedOk,
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
    pub blame_rev: Option<String>,
    pub blame: Loadable<Shared<Vec<BlameLine>>>,
    pub selected_commit: Option<CommitId>,
    pub selected_commit_rev: u64,
    pub commit_details: Loadable<Shared<CommitDetails>>,
    pub commit_details_rev: u64,
}

impl Default for HistoryState {
    fn default() -> Self {
        Self {
            history_scope: LogScope::CurrentBranch,
            log: Loadable::NotLoaded,
            retained_log_while_loading: None,
            log_loading_more: false,
            log_rev: 0,
            file_history_path: None,
            file_history: Loadable::NotLoaded,
            blame_path: None,
            blame_rev: None,
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
    pub diff_state_rev: u64,
    pub diff_rev: u64,
    pub diff: Loadable<Shared<Diff>>,
    pub diff_file_rev: u64,
    pub diff_file: Loadable<Option<Shared<FileDiffText>>>,
    pub diff_file_image: Loadable<Option<Shared<FileDiffImage>>>,
}

impl Default for DiffState {
    fn default() -> Self {
        Self {
            diff_target: None,
            diff_state_rev: 0,
            diff_rev: 0,
            diff: Loadable::NotLoaded,
            diff_file_rev: 0,
            diff_file: Loadable::NotLoaded,
            diff_file_image: Loadable::NotLoaded,
        }
    }
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

#[inline]
fn mix_branch_sidebar_revs(values: [u64; 7]) -> u64 {
    let mut acc = BRANCH_SIDEBAR_REV_MIX;
    for value in values {
        acc ^= value.wrapping_mul(BRANCH_SIDEBAR_REV_MIX);
        acc = acc.rotate_left(11).wrapping_add(BRANCH_SIDEBAR_REV_MIX);
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
    pub status: Loadable<Shared<RepoStatus>>,
    pub status_rev: u64,
    /// Cached flag: true when `status` is `Ready` and at least one unstaged
    /// entry has `FileStatusKind::Conflicted`. Recomputed in `set_status`.
    pub has_unstaged_conflicts: bool,
    pub log: Loadable<Shared<LogPage>>,
    pub log_loading_more: bool,
    pub log_rev: u64,
    pub stashes: Loadable<Arc<Vec<StashEntry>>>,
    pub stashes_rev: u64,
    pub reflog: Loadable<Vec<ReflogEntry>>,
    pub rebase_in_progress: Loadable<bool>,
    pub merge_commit_message: Loadable<Option<String>>,
    pub merge_message_rev: u64,
    pub worktrees: Loadable<Arc<Vec<Worktree>>>,
    pub worktrees_rev: u64,
    pub submodules: Loadable<Arc<Vec<Submodule>>>,
    pub submodules_rev: u64,
    /// Invalidates cached branch-sidebar rows when any sidebar-relevant source changes.
    pub branch_sidebar_rev: u64,

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
            status: Loadable::NotLoaded,
            status_rev: 0,
            has_unstaged_conflicts: false,
            log: Loadable::NotLoaded,
            log_loading_more: false,
            log_rev: 0,
            stashes: Loadable::NotLoaded,
            stashes_rev: 0,
            reflog: Loadable::NotLoaded,
            rebase_in_progress: Loadable::NotLoaded,
            merge_commit_message: Loadable::NotLoaded,
            merge_message_rev: 0,
            worktrees: Loadable::NotLoaded,
            worktrees_rev: 0,
            submodules: Loadable::NotLoaded,
            submodules_rev: 0,
            branch_sidebar_rev: 0,
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

    pub(crate) fn set_status(&mut self, status: Loadable<Shared<RepoStatus>>) {
        if self.status == status {
            return;
        }
        self.has_unstaged_conflicts = matches!(
            &status,
            Loadable::Ready(s) if s.unstaged.iter().any(|e| e.kind == FileStatusKind::Conflicted)
        );
        self.status = status;
        self.status_rev = self.status_rev.wrapping_add(1);
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
        self.history_state.log_rev = self.history_state.log_rev.wrapping_add(1);
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
        self.history_state.log_rev = self.history_state.log_rev.wrapping_add(1);
    }

    pub(crate) fn set_log_scope(&mut self, scope: LogScope) {
        if self.history_state.history_scope == scope {
            return;
        }
        self.history_state.history_scope = scope;
        self.history_state.log_rev = self.history_state.log_rev.wrapping_add(1);
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

    pub(crate) fn bump_diff_state_rev(&mut self) {
        self.diff_state.diff_state_rev = self.diff_state.diff_state_rev.wrapping_add(1);
    }

    pub(crate) fn bump_ops_rev(&mut self) {
        self.ops_rev = self.ops_rev.wrapping_add(1);
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
            line: "line".to_string(),
        }]));
        repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
            id: CommitId("c1".into()),
            message: "m".to_string(),
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

    #[test]
    fn request_primary_refresh_batch_marks_all_primary_loads_when_idle() {
        let mut loads = RepoLoadsInFlight::default();

        assert!(loads.request_primary_refresh_batch());
        assert!(loads.is_in_flight(RepoLoadsInFlight::HEAD_BRANCH));
        assert!(loads.is_in_flight(RepoLoadsInFlight::UPSTREAM_DIVERGENCE));
        assert!(loads.is_in_flight(RepoLoadsInFlight::REBASE_STATE));
        assert!(loads.is_in_flight(RepoLoadsInFlight::MERGE_COMMIT_MESSAGE));
        assert!(loads.is_in_flight(RepoLoadsInFlight::STATUS));
        assert!(loads.is_in_flight(RepoLoadsInFlight::LOG));
    }

    #[test]
    fn request_primary_refresh_batch_skips_when_any_load_is_already_in_flight() {
        let mut loads = RepoLoadsInFlight::default();
        assert!(loads.request(RepoLoadsInFlight::STATUS));

        assert!(!loads.request_primary_refresh_batch());
        assert!(!loads.is_in_flight(RepoLoadsInFlight::HEAD_BRANCH));
        assert!(loads.is_in_flight(RepoLoadsInFlight::STATUS));
        assert!(!loads.is_in_flight(RepoLoadsInFlight::LOG));
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
    fn set_log_bumps_log_rev() {
        let mut repo = new_repo();
        let before = repo.history_state.log_rev;
        repo.set_log(Loadable::Loading);
        assert_eq!(repo.history_state.log_rev, before + 1);
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
        let before = repo.history_state.log_rev;
        repo.set_log_loading_more(true);
        assert_eq!(repo.history_state.log_rev, before + 1);
        repo.set_log_loading_more(false);
        assert_eq!(repo.history_state.log_rev, before + 2);
    }

    #[test]
    fn set_log_scope_bumps_log_rev() {
        let mut repo = new_repo();
        let before = repo.history_state.log_rev;
        repo.set_log_scope(LogScope::AllBranches);
        assert_eq!(repo.history_state.log_rev, before + 1);
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
        let rev = repo.history_state.log_rev;
        repo.set_log(Loadable::Loading);
        assert_eq!(repo.history_state.log_rev, rev);
    }

    #[test]
    fn set_log_loading_more_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_log_loading_more(true);
        let rev = repo.history_state.log_rev;
        repo.set_log_loading_more(true);
        assert_eq!(repo.history_state.log_rev, rev);
    }

    #[test]
    fn set_log_scope_skips_rev_bump_when_unchanged() {
        let mut repo = new_repo();
        repo.set_log_scope(LogScope::AllBranches);
        let rev = repo.history_state.log_rev;
        repo.set_log_scope(LogScope::AllBranches);
        assert_eq!(repo.history_state.log_rev, rev);
    }

    // --- Isolation tests: one setter does not bump another's rev ---

    #[test]
    fn setters_only_bump_their_own_rev_counter() {
        let mut repo = new_repo();
        let snap = (
            repo.status_rev,
            repo.history_state.log_rev,
            repo.history_state.selected_commit_rev,
            repo.history_state.commit_details_rev,
            repo.merge_message_rev,
            repo.upstream_divergence_rev,
            repo.open_rev,
            repo.conflict_state.conflict_rev,
            repo.diff_state.diff_state_rev,
            repo.ops_rev,
        );

        repo.set_status(Loadable::Loading);
        assert_eq!(repo.status_rev, snap.0 + 1);
        assert_eq!(repo.history_state.log_rev, snap.1);
        assert_eq!(repo.history_state.selected_commit_rev, snap.2);
        assert_eq!(repo.history_state.commit_details_rev, snap.3);
        assert_eq!(repo.merge_message_rev, snap.4);
        assert_eq!(repo.upstream_divergence_rev, snap.5);
        assert_eq!(repo.open_rev, snap.6);
        assert_eq!(repo.conflict_state.conflict_rev, snap.7);
        assert_eq!(repo.diff_state.diff_state_rev, snap.8);
        assert_eq!(repo.ops_rev, snap.9);
    }

    #[test]
    fn all_rev_counters_start_at_zero() {
        let repo = new_repo();
        assert_eq!(repo.status_rev, 0);
        assert_eq!(repo.history_state.log_rev, 0);
        assert_eq!(repo.history_state.selected_commit_rev, 0);
        assert_eq!(repo.history_state.commit_details_rev, 0);
        assert_eq!(repo.merge_message_rev, 0);
        assert_eq!(repo.upstream_divergence_rev, 0);
        assert_eq!(repo.open_rev, 0);
        assert_eq!(repo.conflict_state.conflict_rev, 0);
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
        assert_eq!(repo.history_state.history_scope, LogScope::CurrentBranch);
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
    }
}
