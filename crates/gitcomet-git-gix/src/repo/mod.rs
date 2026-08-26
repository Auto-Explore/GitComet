use crate::util::git_workdir_cmd_for as util_git_workdir_cmd_for;
use gitcomet_core::conflict_session::ConflictSession;
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, Diff, DiffArea, DiffPreviewTextSide,
    DiffTarget, FileDiffImage, FileDiffText, FileEntry, HistoryMode, LogCursor, LogPage,
    RecentCommitMessage, RefMetadata, ReflogEntry, Remote, RemoteBranch, RemoteTag, RepoSpec,
    RepoStatus, StashEntry, Submodule, SubmoduleDiffSummary, Tag, UpstreamDivergence, Worktree,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::git_ops_trace::{self, GitOpTraceKind};
use gitcomet_core::services::{
    BlameLine, CancellationToken, CommandOutput, CommitOperationOutcome, ConflictFileStages,
    ConflictSide, ForcePushLease, GitRepository, InteractiveRebaseEntry, MergetoolResult, PullMode,
    RemoteUrlKind, ResetMode, Result, SafePushAfterCommitContext, SafePushAfterCommitDecision,
    SafePushAfterCommitTarget, SequencerState, SubmoduleTrustDecision, SubmoduleTrustTarget,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Convert a gix ObjectId to an `Arc<str>` hex string without intermediate String allocation.
/// Uses a stack buffer + `hex_to_buf` → `Arc::from(&str)` (one heap allocation instead of two).
#[inline]
pub(super) fn oid_to_arc_str(oid: &gix::oid) -> Arc<str> {
    let mut buf = gix::hash::Kind::hex_buf();
    let hex: &str = oid.hex_to_buf(&mut buf);
    Arc::from(hex)
}

/// Convert bytes to `Arc<str>`, avoiding an intermediate String allocation when the input is
/// valid UTF-8 (the common case for git commit metadata).
#[inline]
pub(super) fn bstr_to_arc_str(bytes: &[u8]) -> Arc<str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Arc::from(s),
        Err(_) => Arc::from(String::from_utf8_lossy(bytes).as_ref()),
    }
}

mod blame;
mod conflict_stages;
mod diff;
mod discard;
mod file_browser;
mod git_ops;
mod history;
mod log;
mod mergetool;
mod mergetool_builtin;
mod patch;
mod porcelain;
mod remotes;
mod status;
mod submodules;
mod tags;
mod worktrees;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RepoFileStamp {
    exists: bool,
    len: u64,
    modified: Option<std::time::SystemTime>,
    /// Content fingerprint of the file, when one is captured (currently only for
    /// `.git/index`, via its trailing hash). `len`/`modified` are a cheap change
    /// hint, but they are not reliable: an atomic index rewrite can land with an
    /// identical length (same tracked entries) and an unchanged mtime (coarse or
    /// cached filesystem timestamps, e.g. f2fs), which would otherwise let the
    /// staged-status cache serve a stale result. The content id makes the stamp
    /// content-exact. `None` for files where no fingerprint is read, and also when
    /// the trailer is the null hash (`index.skipHash`/`feature.manyFiles` write a
    /// null trailer regardless of content, so it cannot distinguish index states —
    /// the stat discriminators below cover that case instead).
    content_id: Option<gix::ObjectId>,
    /// Inode of `.git/index` (Unix only). Git rewrites the index atomically via a
    /// lock file + rename, so every rewrite yields a fresh inode. This detects index
    /// changes even when the content fingerprint is unavailable (`skipHash`) and the
    /// length + mtime collide. `None` for generic stamps and on non-Unix platforms.
    inode: Option<u64>,
    /// Change-time (ctime) of `.git/index` in nanoseconds (Unix only). Updated on
    /// every metadata/content change including the rename above, so it backs up the
    /// inode against reuse. `None` for generic stamps and on non-Unix platforms.
    ctime_nanos: Option<i128>,
    /// Set (to a process-unique value) only when a stamp could not be computed reliably — e.g.
    /// `.git/index` exists but is momentarily unreadable (a permission flip, or a Windows sharing
    /// / AV lock). A fresh value on every such call guarantees two of these stamps never compare
    /// equal, forcing a cache miss (a fresh read) instead of risking a stale cache hit from a weak
    /// length+mtime stamp that could collide with an atomic rewrite. `None` for every normally
    /// computed stamp.
    uncacheable_nonce: Option<u64>,
}

impl RepoFileStamp {
    /// A stamp that never compares equal to any other (not even another uncacheable one). Used when
    /// a file's real fingerprint cannot be read, so the cache treats it as changed rather than risk
    /// serving a stale result. See [`RepoFileStamp::uncacheable_nonce`].
    fn uncacheable() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Self {
            uncacheable_nonce: Some(NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)),
            ..Self::default()
        }
    }
}

/// Cheap stat-based file stamp (existence, length, mtime). A reliable change *hint* but not
/// content-exact — `.git/index` uses the hardened `repo_index_stamp` instead. Shared by the status
/// and git-ops cache keys (do not duplicate this mapping; see `status.rs` / `git_ops.rs`).
fn repo_file_stamp(path: &Path) -> RepoFileStamp {
    match std::fs::metadata(path) {
        Ok(metadata) => RepoFileStamp {
            exists: true,
            len: metadata.len(),
            modified: metadata.modified().ok(),
            ..RepoFileStamp::default()
        },
        Err(_) => RepoFileStamp::default(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitlinkStatusCapabilityCacheEntry {
    gitmodules: RepoFileStamp,
    index: RepoFileStamp,
    may_have_gitlinks: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchTrackingConfigCacheEntry {
    local_config: RepoFileStamp,
    worktree_config: RepoFileStamp,
    has_branch_sections: bool,
}

/// Caches the Tree→Index (HEAD vs index) result so that background refresh
/// cycles can skip the tree comparison when HEAD and the index file are
/// unchanged since the last status call.
#[derive(Clone, Debug)]
struct TreeIndexCacheEntry {
    head_oid: Option<gix::ObjectId>,
    index_stamp: RepoFileStamp,
    staged: Vec<gitcomet_core::domain::FileStatus>,
}

/// What a cached log page was walked from.
#[derive(Clone, Debug, Eq, PartialEq)]
enum LogPageSeed {
    Head(Option<gix::ObjectId>),
    Tips(Arc<[gix::ObjectId]>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LogPageCacheKey {
    mode: HistoryMode,
    seed: LogPageSeed,
    /// Invalidates cached pages when a shallow repository is deepened.
    shallow: RepoFileStamp,
    limit: usize,
    last_seen: Option<CommitId>,
    resume_from: Option<CommitId>,
    /// Author filter, or `None` for the unfiltered walk.
    author: Option<log::AuthorFilter>,
}

#[derive(Clone, Debug)]
struct LogPageCacheEntry {
    key: LogPageCacheKey,
    page: LogPage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LogFileFollowCacheKey {
    head_oid: Option<gix::ObjectId>,
    path: PathBuf,
}

#[derive(Clone, Debug)]
struct LogFileFollowCacheEntry {
    key: LogFileFollowCacheKey,
    commits: Arc<Vec<Commit>>,
}

/// The paged walk's commit filter. Boxed rather than a plain `fn` because a
/// shallow repository needs one that carries state — the grafted parents still
/// to be skipped — and the walk it belongs to is parked in [`LogPagedWalkCache`],
/// so it can borrow nothing.
type LogPagedWalkFilter = Box<dyn FnMut(&gix::oid) -> bool + Send>;

enum LogPagedWalk {
    CommitTime(gix::traverse::commit::Simple<gix::OdbHandleArc, LogPagedWalkFilter>),
    DateOrder(gix::traverse::commit::Topo<gix::OdbHandleArc, LogPagedWalkFilter>),
}

impl LogPagedWalk {
    fn is_date_order(&self) -> bool {
        matches!(self, Self::DateOrder(_))
    }
}

impl Iterator for LogPagedWalk {
    type Item = std::result::Result<
        gix::traverse::commit::Info,
        Box<dyn std::error::Error + Send + Sync + 'static>,
    >;

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::CommitTime(walk) => walk.next().map(|info| info.map_err(Into::into)),
            Self::DateOrder(walk) => walk.next().map(|info| info.map_err(Into::into)),
        }
    }
}

struct LogPagedWalkState {
    pending: std::collections::VecDeque<gix::traverse::commit::Info>,
    walk: LogPagedWalk,
}

struct LogPagedWalkCacheEntry {
    token: Arc<str>,
    mode: HistoryMode,
    /// The commits the walk was seeded from — one head for most modes, every
    /// ref for `AllBranches`. A walk started from different tips covers a
    /// different history, so a token minted for one must not resume the other.
    tips: Arc<[gix::ObjectId]>,
    /// Author filter the walk was started with, or `None` for the unfiltered
    /// walk. The walk's *position* depends on the filter — every non-matching
    /// commit was already consumed — so resuming one walk under a different
    /// filter would silently skip whatever the first pass rejected.
    author: Option<log::AuthorFilter>,
    state: LogPagedWalkState,
}

#[derive(Default)]
struct LogPagedWalkCache {
    next_id: u64,
    entries: Vec<LogPagedWalkCacheEntry>,
}

const LOG_PAGE_CACHE_LIMIT: usize = 32;
const LOG_FILE_FOLLOW_CACHE_LIMIT: usize = 16;
const LOG_PAGED_WALK_CACHE_LIMIT: usize = 32;
/// Date-order walks retain in-degree state for the reachable history.
const LOG_PAGED_TOPO_WALK_CACHE_LIMIT: usize = 4;

pub(crate) struct GixRepo {
    spec: RepoSpec,
    _repo: gix::ThreadSafeRepository,
    gitlink_status_capability: std::sync::Mutex<Option<GitlinkStatusCapabilityCacheEntry>>,
    branch_tracking_config: std::sync::Mutex<Option<BranchTrackingConfigCacheEntry>>,
    tree_index_cache: std::sync::Mutex<Option<TreeIndexCacheEntry>>,
    log_page_cache: std::sync::Mutex<Vec<LogPageCacheEntry>>,
    all_branches_tips: std::sync::Mutex<Option<Arc<[gix::ObjectId]>>>,
    log_file_follow_cache: std::sync::Mutex<Vec<LogFileFollowCacheEntry>>,
    log_paged_walk_cache: std::sync::Mutex<LogPagedWalkCache>,
}

impl GixRepo {
    pub(crate) fn new(workdir: PathBuf, repo: gix::ThreadSafeRepository) -> Self {
        Self {
            spec: RepoSpec { workdir },
            _repo: repo,
            gitlink_status_capability: std::sync::Mutex::new(None),
            branch_tracking_config: std::sync::Mutex::new(None),
            tree_index_cache: std::sync::Mutex::new(None),
            log_page_cache: std::sync::Mutex::new(Vec::new()),
            all_branches_tips: std::sync::Mutex::new(None),
            log_file_follow_cache: std::sync::Mutex::new(Vec::new()),
            log_paged_walk_cache: std::sync::Mutex::new(LogPagedWalkCache::default()),
        }
    }

    /// Returns a `Command` pre-configured with `git -C <workdir>`.
    pub(super) fn git_workdir_cmd(&self) -> Command {
        util_git_workdir_cmd_for(&self.spec.workdir)
    }

    pub(super) fn reopen_repo(&self) -> Result<gix::Repository> {
        crate::open::open_worktree_repo(&self.spec.workdir).map_err(|e| match e {
            gix::open::Error::NotARepository { .. } => Error::new(ErrorKind::NotARepository),
            gix::open::Error::Io(io) => Error::new(ErrorKind::Io(io.kind())),
            e => Error::new(ErrorKind::Backend(format!("gix open fresh repo: {e}"))),
        })
    }
}

pub(crate) fn allow_test_repo_local_mergetool_command(workdir: &Path, tool_name: &str) {
    mergetool::allow_test_repo_local_mergetool_command(workdir, tool_name);
}

impl GitRepository for GixRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_history_mode_page(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_history_mode_page_impl(mode, limit, cursor)
    }

    fn log_history_mode_page_cancellable(
        &self,
        mode: HistoryMode,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_history_mode_page_cancellable_impl(mode, limit, cursor, cancellation)
    }

    fn log_history_mode_page_streaming(
        &self,
        mode: HistoryMode,
        author: Option<&str>,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
        on_chunk: &mut dyn FnMut(gitcomet_core::services::LogChunk),
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_history_mode_page_streaming_impl(
            mode,
            author,
            limit,
            cursor,
            cancellation,
            on_chunk,
        )
    }

    fn log_head_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_head_page_impl(limit, cursor)
    }

    fn log_head_page_cancellable(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_head_page_cancellable_impl(limit, cursor, cancellation)
    }

    fn log_all_branches_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_all_branches_page_impl(limit, cursor)
    }

    fn log_all_branches_page_cancellable(
        &self,
        limit: usize,
        cursor: Option<&LogCursor>,
        cancellation: &CancellationToken,
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_all_branches_page_cancellable_impl(limit, cursor, cancellation)
    }

    fn log_file_page(
        &self,
        path: &Path,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_file_page_impl(path, limit, cursor)
    }

    fn commit_details(&self, id: &CommitId) -> Result<CommitDetails> {
        self.commit_details_impl(id)
    }

    fn diff_range_files(
        &self,
        from: &CommitId,
        to: Option<&CommitId>,
    ) -> Result<Vec<CommitFileChange>> {
        self.diff_range_files_impl(from, to)
    }

    fn commit_messages(&self, ids: &[CommitId]) -> Result<Vec<String>> {
        self.commit_messages_impl(ids)
    }

    fn topologically_order_commits(&self, ids: &[CommitId]) -> Result<Vec<CommitId>> {
        self.topologically_order_commits_impl(ids)
    }

    fn recent_commit_messages(&self, limit: usize) -> Result<Vec<RecentCommitMessage>> {
        self.recent_commit_messages_impl(limit)
    }

    fn reflog_head(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        self.reflog_head_impl(limit)
    }

    fn current_branch(&self) -> Result<String> {
        self.current_branch_impl()
    }

    fn current_branch_cancellable(&self, cancellation: &CancellationToken) -> Result<String> {
        cancellation.check_cancelled()?;
        let branch = self.current_branch_impl()?;
        cancellation.check_cancelled()?;
        Ok(branch)
    }

    fn head_commit_id(&self) -> Result<Option<CommitId>> {
        self.head_commit_id_impl()
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_branches_impl()
    }

    fn list_branches_cancellable(&self, cancellation: &CancellationToken) -> Result<Vec<Branch>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        cancellation.check_cancelled()?;
        let branches = self.list_branches_impl()?;
        cancellation.check_cancelled()?;
        Ok(branches)
    }

    fn list_tags(&self) -> Result<Vec<Tag>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_tags_impl()
    }

    fn list_tags_cancellable(&self, cancellation: &CancellationToken) -> Result<Vec<Tag>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_tags_cancellable_impl(cancellation)
    }

    fn list_remote_tags(&self) -> Result<Vec<RemoteTag>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remote_tags_impl()
    }

    fn list_remote_tags_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<RemoteTag>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remote_tags_cancellable_impl(cancellation)
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remotes_impl()
    }

    fn list_remotes_cancellable(&self, cancellation: &CancellationToken) -> Result<Vec<Remote>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        cancellation.check_cancelled()?;
        let remotes = self.list_remotes_impl()?;
        cancellation.check_cancelled()?;
        Ok(remotes)
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remote_branches_impl()
    }

    fn list_remote_branches_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remote_branches_cancellable_impl(cancellation)
    }

    fn worktree_status(&self) -> Result<Vec<gitcomet_core::domain::FileStatus>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.worktree_status_impl()
    }

    fn worktree_status_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<gitcomet_core::domain::FileStatus>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.worktree_status_cancellable_impl(cancellation)
    }

    fn staged_status(&self) -> Result<Vec<gitcomet_core::domain::FileStatus>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.staged_status_impl()
    }

    fn staged_status_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<gitcomet_core::domain::FileStatus>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.staged_status_cancellable_impl(cancellation)
    }

    fn status(&self) -> Result<RepoStatus> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.status_impl()
    }

    fn status_cancellable(&self, cancellation: &CancellationToken) -> Result<RepoStatus> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.status_cancellable_impl(cancellation)
    }

    fn upstream_divergence(&self) -> Result<Option<UpstreamDivergence>> {
        self.upstream_divergence_impl()
    }

    fn upstream_divergence_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<UpstreamDivergence>> {
        self.upstream_divergence_cancellable_impl(cancellation)
    }

    fn pull_branch_with_output(&self, remote: &str, branch: &str) -> Result<CommandOutput> {
        self.pull_branch_with_output_impl(remote, branch)
    }

    fn merge_ref_with_output(&self, reference: &str) -> Result<CommandOutput> {
        self.merge_ref_with_output_impl(reference)
    }

    fn squash_ref_with_output(&self, reference: &str) -> Result<CommandOutput> {
        self.squash_ref_with_output_impl(reference)
    }

    fn squash_message_preview(&self, oldest: &CommitId, head: &CommitId) -> Result<String> {
        self.squash_message_preview_impl(oldest, head)
    }

    fn squash_commits_with_output(
        &self,
        oldest: &CommitId,
        expected_head: &CommitId,
        message: &str,
    ) -> Result<CommandOutput> {
        self.squash_commits_with_output_impl(oldest, expected_head, message)
    }

    fn diff_unified(&self, target: &DiffTarget) -> Result<String> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Diff);
        self.diff_unified_impl(target)
    }

    fn diff_parsed(&self, target: &DiffTarget) -> Result<Diff> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Diff);
        self.diff_parsed_impl(target)
    }

    fn diff_parsed_cancellable(
        &self,
        target: &DiffTarget,
        cancellation: &CancellationToken,
    ) -> Result<Diff> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Diff);
        self.diff_parsed_cancellable_impl(target, cancellation)
    }

    fn diff_file_text(&self, target: &DiffTarget) -> Result<Option<FileDiffText>> {
        self.diff_file_text_impl(target)
    }

    fn diff_preview_text_file(
        &self,
        target: &DiffTarget,
        side: DiffPreviewTextSide,
    ) -> Result<Option<PathBuf>> {
        self.diff_preview_text_file_impl(target, side)
    }

    fn diff_file_image(&self, target: &DiffTarget) -> Result<Option<FileDiffImage>> {
        self.diff_file_image_impl(target)
    }

    fn conflict_file_stages(&self, path: &Path) -> Result<Option<ConflictFileStages>> {
        self.conflict_file_stages_impl(path)
    }

    fn conflict_session(&self, path: &Path) -> Result<Option<ConflictSession>> {
        self.conflict_session_impl(path)
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
        self.create_branch_impl(name, target)
    }

    fn rename_branch(&self, old_name: &str, new_name: &str) -> Result<()> {
        self.rename_branch_impl(old_name, new_name)
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        self.delete_branch_impl(name)
    }

    fn delete_branch_force(&self, name: &str) -> Result<()> {
        self.delete_branch_force_impl(name)
    }

    fn checkout_branch(&self, name: &str) -> Result<()> {
        self.checkout_branch_impl(name)
    }

    fn checkout_remote_branch(&self, remote: &str, branch: &str, local_branch: &str) -> Result<()> {
        self.checkout_remote_branch_impl(remote, branch, local_branch)
    }

    fn checkout_commit(&self, id: &CommitId) -> Result<()> {
        self.checkout_commit_impl(id)
    }

    fn cherry_pick(&self, id: &CommitId) -> Result<()> {
        self.cherry_pick_impl(id)
    }

    fn cherry_pick_with_output(
        &self,
        id: &CommitId,
        commit: bool,
        mainline: Option<usize>,
    ) -> Result<CommandOutput> {
        self.cherry_pick_with_output_impl(id, commit, mainline)
    }

    fn revert(&self, id: &CommitId) -> Result<()> {
        self.revert_impl(id)
    }

    fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
        self.stash_create_impl(message, include_untracked)
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        self.stash_list_impl()
    }

    fn stash_list_cancellable(&self, cancellation: &CancellationToken) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let stashes = self.stash_list_impl()?;
        cancellation.check_cancelled()?;
        Ok(stashes)
    }

    fn stash_apply(&self, index: usize) -> Result<()> {
        self.stash_apply_impl(index)
    }

    fn stash_drop(&self, index: usize) -> Result<()> {
        self.stash_drop_impl(index)
    }

    fn stage(&self, paths: &[&Path]) -> Result<()> {
        self.stage_impl(paths)
    }

    fn unstage(&self, paths: &[&Path]) -> Result<()> {
        self.unstage_impl(paths)
    }

    fn commit(&self, message: &str) -> Result<()> {
        self.commit_impl(message)
    }

    fn commit_with_outcome(&self, message: &str) -> Result<CommitOperationOutcome> {
        self.commit_with_outcome_impl(message)
    }

    fn commit_amend(&self, message: &str) -> Result<()> {
        self.commit_amend_impl(message)
    }

    fn commit_amend_with_outcome(&self, message: &str) -> Result<CommitOperationOutcome> {
        self.commit_amend_with_outcome_impl(message)
    }

    fn fetch_all(&self) -> Result<()> {
        self.fetch_all_impl(true)
    }

    fn fetch_all_with_output(&self) -> Result<CommandOutput> {
        self.fetch_all_with_output_impl(true)
    }

    fn fetch_all_with_output_prune(&self, prune: bool) -> Result<CommandOutput> {
        self.fetch_all_with_output_impl(prune)
    }

    fn pull(&self, mode: PullMode) -> Result<()> {
        self.pull_impl(mode)
    }

    fn pull_with_output(&self, mode: PullMode) -> Result<CommandOutput> {
        self.pull_with_output_impl(mode)
    }

    fn push(&self) -> Result<()> {
        self.push_impl()
    }

    fn push_with_output(&self) -> Result<CommandOutput> {
        self.push_with_output_impl()
    }

    fn push_force(&self) -> Result<()> {
        self.push_force_impl()
    }

    fn push_force_with_output(&self) -> Result<CommandOutput> {
        self.push_force_with_output_impl()
    }

    fn safe_push_after_commit(
        &self,
        context: &SafePushAfterCommitContext,
    ) -> Result<SafePushAfterCommitDecision> {
        self.safe_push_after_commit_impl(context)
    }

    fn push_after_commit_with_output(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_with_output_impl(target)
    }

    fn push_after_commit_set_upstream_with_output(
        &self,
        target: &SafePushAfterCommitTarget,
    ) -> Result<CommandOutput> {
        self.push_after_commit_set_upstream_with_output_impl(target)
    }

    fn push_force_with_lease_with_output(&self, lease: &ForcePushLease) -> Result<CommandOutput> {
        self.push_force_with_lease_with_output_impl(lease)
    }

    fn reset_with_output(&self, target: &str, mode: ResetMode) -> Result<CommandOutput> {
        self.reset_with_output_impl(target, mode)
    }

    fn rebase_with_output(&self, onto: &str) -> Result<CommandOutput> {
        self.rebase_with_output_impl(onto)
    }

    fn rebase_continue_with_output(&self) -> Result<CommandOutput> {
        self.rebase_continue_with_output_impl()
    }

    fn rebase_abort_with_output(&self) -> Result<CommandOutput> {
        self.rebase_abort_with_output_impl()
    }

    fn list_commits_for_interactive_rebase(
        &self,
        base: &str,
    ) -> Result<Vec<InteractiveRebaseEntry>> {
        self.list_commits_for_interactive_rebase_impl(base)
    }

    fn interactive_rebase_with_output(
        &self,
        base: &str,
        entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        self.interactive_rebase_with_output_impl(base, entries)
    }

    fn interactive_cherry_pick_with_output(
        &self,
        entries: &[InteractiveRebaseEntry],
    ) -> Result<CommandOutput> {
        self.interactive_cherry_pick_with_output_impl(entries)
    }

    fn merge_abort_with_output(&self) -> Result<CommandOutput> {
        self.merge_abort_with_output_impl()
    }

    fn rebase_in_progress(&self) -> Result<bool> {
        self.rebase_in_progress_impl()
    }

    fn rebase_in_progress_cancellable(&self, cancellation: &CancellationToken) -> Result<bool> {
        cancellation.check_cancelled()?;
        let in_progress = self.rebase_in_progress_impl()?;
        cancellation.check_cancelled()?;
        Ok(in_progress)
    }

    fn sequencer_state(&self) -> Result<SequencerState> {
        self.sequencer_state_impl()
    }

    fn sequencer_state_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<SequencerState> {
        cancellation.check_cancelled()?;
        let state = self.sequencer_state_impl()?;
        cancellation.check_cancelled()?;
        Ok(state)
    }

    fn merge_commit_message(&self) -> Result<Option<String>> {
        self.merge_commit_message_impl()
    }

    fn merge_commit_message_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Option<String>> {
        cancellation.check_cancelled()?;
        let message = self.merge_commit_message_impl()?;
        cancellation.check_cancelled()?;
        Ok(message)
    }

    fn create_tag_with_output(
        &self,
        name: &str,
        target: &str,
        message: Option<&str>,
        annotated: bool,
    ) -> Result<CommandOutput> {
        self.create_tag_with_output_impl(name, target, message, annotated)
    }

    fn delete_tag_with_output(&self, name: &str) -> Result<CommandOutput> {
        self.delete_tag_with_output_impl(name)
    }

    fn prune_merged_branches_with_output(&self) -> Result<CommandOutput> {
        self.prune_merged_branches_with_output_impl()
    }

    fn prune_local_tags_with_output(&self) -> Result<CommandOutput> {
        self.prune_local_tags_with_output_impl()
    }

    fn push_tag_with_output(&self, remote: &str, name: &str) -> Result<CommandOutput> {
        self.push_tag_with_output_impl(remote, name)
    }

    fn delete_remote_tag_with_output(&self, remote: &str, name: &str) -> Result<CommandOutput> {
        self.delete_remote_tag_with_output_impl(remote, name)
    }

    fn add_remote_with_output(&self, name: &str, url: &str) -> Result<CommandOutput> {
        self.add_remote_with_output_impl(name, url)
    }

    fn remove_remote_with_output(&self, name: &str) -> Result<CommandOutput> {
        self.remove_remote_with_output_impl(name)
    }

    fn set_remote_url_with_output(
        &self,
        name: &str,
        url: &str,
        kind: RemoteUrlKind,
    ) -> Result<CommandOutput> {
        self.set_remote_url_with_output_impl(name, url, kind)
    }

    fn push_set_upstream(&self, remote: &str, branch: &str) -> Result<()> {
        self.push_set_upstream_impl(remote, branch)
    }

    fn push_set_upstream_with_output(&self, remote: &str, branch: &str) -> Result<CommandOutput> {
        self.push_set_upstream_with_output_impl(remote, branch)
    }

    fn set_upstream_branch_with_output(
        &self,
        branch: &str,
        upstream: &str,
    ) -> Result<CommandOutput> {
        self.set_upstream_branch_with_output_impl(branch, upstream)
    }

    fn unset_upstream_branch_with_output(&self, branch: &str) -> Result<CommandOutput> {
        self.unset_upstream_branch_with_output_impl(branch)
    }

    fn delete_remote_branch_with_output(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        self.delete_remote_branch_with_output_impl(remote, branch)
    }

    fn delete_remote_branches_with_output(
        &self,
        remote: &str,
        branches: &[String],
    ) -> Result<CommandOutput> {
        self.delete_remote_branches_with_output_impl(remote, branches)
    }

    fn blame_file(&self, path: &Path, rev: Option<&str>) -> Result<Vec<BlameLine>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Blame);
        self.blame_file_impl(path, rev)
    }

    fn blame_worktree_file(&self, path: &Path, area: DiffArea) -> Result<Vec<BlameLine>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Blame);
        self.blame_worktree_file_impl(path, area)
    }

    fn resolve_file_path_at_commit(
        &self,
        path: &Path,
        commit: &CommitId,
    ) -> Result<Option<PathBuf>> {
        self.resolve_file_path_at_commit_impl(path, commit)
    }

    fn checkout_conflict_side(&self, path: &Path, side: ConflictSide) -> Result<CommandOutput> {
        self.checkout_conflict_side_impl(path, side)
    }

    fn accept_conflict_deletion(&self, path: &Path) -> Result<CommandOutput> {
        self.accept_conflict_deletion_impl(path)
    }

    fn checkout_conflict_base(&self, path: &Path) -> Result<CommandOutput> {
        self.checkout_conflict_base_impl(path)
    }

    fn launch_mergetool(&self, path: &Path) -> Result<MergetoolResult> {
        self.launch_mergetool_impl(path)
    }

    fn export_patch_with_output(&self, commit_id: &CommitId, dest: &Path) -> Result<CommandOutput> {
        self.export_patch_with_output_impl(commit_id, dest)
    }

    fn apply_patch_with_output(&self, patch: &Path) -> Result<CommandOutput> {
        self.apply_patch_with_output_impl(patch)
    }

    fn apply_unified_patch_to_index_with_output(
        &self,
        patch: &str,
        reverse: bool,
    ) -> Result<CommandOutput> {
        self.apply_unified_patch_to_index_with_output_impl(patch, reverse)
    }

    fn apply_unified_patch_to_worktree_with_output(
        &self,
        patch: &str,
        reverse: bool,
    ) -> Result<CommandOutput> {
        self.apply_unified_patch_to_worktree_with_output_impl(patch, reverse)
    }

    fn list_worktrees(&self) -> Result<Vec<Worktree>> {
        self.list_worktrees_impl()
    }

    fn list_worktrees_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Worktree>> {
        cancellation.check_cancelled()?;
        let worktrees = self.list_worktrees_impl()?;
        cancellation.check_cancelled()?;
        Ok(worktrees)
    }

    fn list_ref_metadata(&self) -> Result<Vec<(String, RefMetadata)>> {
        self.list_ref_metadata_impl()
    }

    fn list_ref_metadata_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<(String, RefMetadata)>> {
        cancellation.check_cancelled()?;
        let metadata = self.list_ref_metadata_impl()?;
        cancellation.check_cancelled()?;
        Ok(metadata)
    }

    fn add_worktree_with_output(
        &self,
        path: &Path,
        reference: Option<&str>,
    ) -> Result<CommandOutput> {
        self.add_worktree_with_output_impl(path, reference)
    }

    fn remove_worktree_with_output(&self, path: &Path) -> Result<CommandOutput> {
        self.remove_worktree_with_output_impl(path)
    }

    fn force_remove_worktree_with_output(&self, path: &Path) -> Result<CommandOutput> {
        self.force_remove_worktree_with_output_impl(path)
    }

    fn list_submodules(&self) -> Result<Vec<Submodule>> {
        self.list_submodules_impl()
    }

    fn list_submodules_cancellable(
        &self,
        cancellation: &CancellationToken,
    ) -> Result<Vec<Submodule>> {
        self.list_submodules_cancellable_impl(cancellation)
    }

    fn list_worktree_files(&self) -> Result<Vec<FileEntry>> {
        self.list_worktree_files_impl()
    }

    fn list_tree_files_at_commit(&self, commit_id: &CommitId) -> Result<Vec<FileEntry>> {
        self.list_tree_files_at_commit_impl(commit_id)
    }

    fn submodule_diff_summary(&self, target: &DiffTarget) -> Result<SubmoduleDiffSummary> {
        self.submodule_diff_summary_impl(target)
    }

    fn check_submodule_add_trust(&self, url: &str, path: &Path) -> Result<SubmoduleTrustDecision> {
        self.check_submodule_add_trust_impl(url, path)
    }

    fn check_submodule_update_trust(&self) -> Result<SubmoduleTrustDecision> {
        self.check_submodule_update_trust_impl()
    }

    fn check_submodule_load_trust(&self, path: &Path) -> Result<SubmoduleTrustDecision> {
        self.check_submodule_load_trust_impl(path)
    }

    fn add_submodule_with_output(
        &self,
        url: &str,
        path: &Path,
        branch: Option<&str>,
        name: Option<&str>,
        force: bool,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        self.add_submodule_with_output_impl(url, path, branch, name, force, approved_sources)
    }

    fn update_submodules_with_output(
        &self,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        self.update_submodules_with_output_impl(approved_sources)
    }

    fn load_submodule_with_output(
        &self,
        path: &Path,
        approved_sources: &[SubmoduleTrustTarget],
    ) -> Result<CommandOutput> {
        self.load_submodule_with_output_impl(path, approved_sources)
    }

    fn change_submodule_pointer_with_output(
        &self,
        path: &Path,
        reference: &str,
    ) -> Result<CommandOutput> {
        self.change_submodule_pointer_with_output_impl(path, reference)
    }

    fn remove_submodule_with_output(&self, path: &Path) -> Result<CommandOutput> {
        self.remove_submodule_with_output_impl(path)
    }

    fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()> {
        self.discard_worktree_changes_impl(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_to_arc_str_round_trips_hex_object_id() {
        let expected = "0123456789abcdef0123456789abcdef01234567";
        let oid = gix::ObjectId::from_hex(expected.as_bytes()).expect("valid object id");

        assert_eq!(oid_to_arc_str(oid.as_ref()).as_ref(), expected);
    }

    #[test]
    fn bstr_to_arc_str_preserves_utf8_bytes() {
        assert_eq!(
            bstr_to_arc_str("hello git".as_bytes()).as_ref(),
            "hello git"
        );
    }

    #[test]
    fn bstr_to_arc_str_uses_lossy_conversion_for_invalid_utf8() {
        assert_eq!(bstr_to_arc_str(b"foo\x80bar").as_ref(), "foo\u{fffd}bar");
    }
}
