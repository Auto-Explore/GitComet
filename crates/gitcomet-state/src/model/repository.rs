//! One open repository's state, identity, and lifetime.

use super::LargeFileSettings;
use super::{
    CommandLogEntry, ConflictState, DiffState, FileBrowserState, GitHookOperation, HistoryState,
    InteractiveCherryPickSetup, InteractiveRebaseSetup, Loadable, RepoLoadsInFlight,
    RepoNavigationState, RepoPendingState, Shared, SubmoduleAddProgressState, TagPushPreviewState,
};
use crate::session;
use gitcomet_core::domain::*;
use gitcomet_core::git_operation::GitOperationId;
use gitcomet_core::services::SequencerState;
use rustc_hash::{FxHashMap, FxHashSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::SystemTime;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct RepoId(pub u64);

/// GitComet's side of a repository's maintenance: git's recommendation and a
/// run the user started. Git never runs it by itself for GitComet's commands.
#[derive(Clone, Debug, Default)]
pub struct RepoMaintenanceState {
    /// When a check was last requested; focus and tab switches ask at most
    /// hourly, and the effect enforces the daily limit.
    pub check_requested_at: Option<SystemTime>,
    /// Git recommends maintenance and the user has not answered yet.
    pub recommended: bool,
    /// A maintenance run is in flight.
    pub running: bool,
    /// Watcher changes held back while maintenance rewrites these objects.
    pub deferred_change: Option<crate::msg::RepoExternalChange>,
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
    /// The shared git directory (the main `.git` of a linked worktree), set
    /// once the repository opens; tabs of one repository share maintenance.
    pub common_dir: Option<Arc<std::path::Path>>,
    pub shared_preferences: Option<super::RepositoryPreferencesSnapshot>,
    pub maintenance: RepoMaintenanceState,

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
    /// Git LFS / git-annex facts; drives whether per-row state is computed.
    pub large_file_support: Loadable<Arc<gitcomet_core::large_files::LargeFileSupport>>,
    pub large_file_support_rev: u64,
    /// Per-row large-file state, produced with line stats under the same
    /// generation and kept across rescans like them.
    pub uncommitted_large_files: Arc<gitcomet_core::large_files::UncommittedLargeFiles>,
    pub large_files_rev: u64,
    pub lfs_locks: Loadable<Arc<Vec<gitcomet_core::large_files::LfsLock>>>,
    pub lfs_locks_rev: u64,
    /// Location results keyed by content, bounded to the last requested sides.
    pub annex_whereis:
        std::collections::BTreeMap<String, Loadable<Arc<gitcomet_core::large_files::AnnexWhereis>>>,
    pub annex_whereis_rev: u64,
    /// `git annex unused`, loaded when its prompt opens.
    pub annex_unused: Loadable<Arc<gitcomet_core::large_files::AnnexUnused>>,
    pub annex_unused_rev: u64,
    /// Branch lists leave out `git-annex` and `synced/*`: the repo uses
    /// git-annex and the preference is on. Kept here so sidebar caches,
    /// which key on repo data, see it change.
    pub annex_refs_hidden: bool,
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
    /// Hosted file lists' changes.
    pub change_lists:
        Arc<FxHashMap<crate::diff_session::DiffViewId, crate::diff_session::ChangeListSession>>,
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SidebarDataRequest {
    pub worktrees: bool,
    pub submodules: bool,
    pub stashes: bool,
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
            common_dir: None,
            shared_preferences: None,
            maintenance: RepoMaintenanceState::default(),
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
            large_file_support: Loadable::NotLoaded,
            large_file_support_rev: 0,
            uncommitted_large_files: Arc::default(),
            large_files_rev: 0,
            lfs_locks: Loadable::NotLoaded,
            lfs_locks_rev: 0,
            annex_whereis: Default::default(),
            annex_whereis_rev: 0,
            annex_unused: Loadable::NotLoaded,
            annex_unused_rev: 0,
            annex_refs_hidden: false,
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
            change_lists: Arc::default(),
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

    pub(crate) fn set_large_file_support(
        &mut self,
        support: Loadable<gitcomet_core::large_files::LargeFileSupport>,
    ) {
        let support = loadable_into_arc(support);
        if self.large_file_support == support {
            return;
        }
        self.large_file_support = support;
        self.large_file_support_rev = self.large_file_support_rev.wrapping_add(1);
        // The Annex sidebar section is built from this summary.
        self.bump_branch_sidebar_rev();
        if !self.large_file_support_active() {
            self.set_uncommitted_large_files(Arc::default());
        }
    }

    /// The repository uses Git LFS or git-annex, so rows carry their state.
    pub fn large_file_support_active(&self) -> bool {
        matches!(&self.large_file_support, Loadable::Ready(support) if support.is_active())
    }

    pub(crate) fn set_uncommitted_large_files(
        &mut self,
        files: Arc<gitcomet_core::large_files::UncommittedLargeFiles>,
    ) {
        if self.uncommitted_large_files == files {
            return;
        }
        self.uncommitted_large_files = files;
        self.large_files_rev = self.large_files_rev.wrapping_add(1);
    }

    pub(crate) fn set_lfs_locks(
        &mut self,
        locks: Loadable<Vec<gitcomet_core::large_files::LfsLock>>,
    ) {
        let locks = loadable_into_arc(locks);
        if self.lfs_locks == locks {
            return;
        }
        self.lfs_locks = locks;
        self.lfs_locks_rev = self.lfs_locks_rev.wrapping_add(1);
    }

    /// True for a git-annex bookkeeping branch the user asked to hide.
    pub fn hides_annex_ref(&self, branch_name: &str) -> bool {
        self.annex_refs_hidden && gitcomet_core::annex::is_annex_ref(branch_name)
    }

    /// Recompute whether annex bookkeeping branches are hidden.
    pub(crate) fn sync_annex_refs_hidden(&mut self, hide_preference: bool) {
        let hidden = hide_preference
            && matches!(&self.large_file_support, Loadable::Ready(support) if support.annex.in_use());
        if self.annex_refs_hidden != hidden {
            self.annex_refs_hidden = hidden;
            self.bump_branch_sidebar_rev();
        }
    }

    pub(crate) fn set_annex_whereis(
        &mut self,
        key: String,
        whereis: Loadable<Arc<gitcomet_core::large_files::AnnexWhereis>>,
    ) {
        if self.annex_whereis.get(&key) == Some(&whereis) {
            return;
        }
        self.annex_whereis.insert(key, whereis);
        self.annex_whereis_rev = self.annex_whereis_rev.wrapping_add(1);
    }

    pub(crate) fn set_annex_unused(
        &mut self,
        unused: Loadable<Arc<gitcomet_core::large_files::AnnexUnused>>,
    ) {
        if self.annex_unused == unused {
            return;
        }
        self.annex_unused = unused;
        self.annex_unused_rev = self.annex_unused_rev.wrapping_add(1);
    }

    /// Locations for this exact content, independent of its current path.
    pub fn annex_whereis_for(
        &self,
        key: &str,
    ) -> Option<&Loadable<Arc<gitcomet_core::large_files::AnnexWhereis>>> {
        self.annex_whereis.get(key)
    }

    /// The adjusted branch HEAD is on, as `(base, mode)`. Until detection
    /// succeeds, conservatively treat the name as adjusted: plain Git must
    /// never merge into an adjusted branch just because support is loading.
    pub fn annex_adjusted_branch(&self) -> Option<(&str, &str)> {
        let Loadable::Ready(head) = &self.head_branch else {
            return None;
        };
        let adjusted = gitcomet_core::annex::adjusted_branch(head)?;
        match &self.large_file_support {
            // Outside an annex repo the name is an ordinary branch.
            Loadable::Ready(support) if !support.annex.in_use() => None,
            _ => Some(adjusted),
        }
    }

    /// Pull and Push of the current branch go through git-annex.
    pub fn annex_takes_over_pull_push(&self, settings: &LargeFileSettings) -> bool {
        settings.annex_pull_push && self.annex_adjusted_branch().is_some()
    }

    pub fn large_file_command_busy(
        &self,
        command: &gitcomet_core::large_files::LargeFileCommand,
    ) -> bool {
        (command.pulls() && self.worktree_pull_in_flight > 0)
            || (command.pushes() && self.push_in_flight > 0)
    }

    /// The lock held on `path`, when the lock list has loaded.
    pub fn lfs_lock_for(
        &self,
        path: &std::path::Path,
    ) -> Option<&gitcomet_core::large_files::LfsLock> {
        match &self.lfs_locks {
            Loadable::Ready(locks) => locks.iter().find(|lock| lock.path == path),
            _ => None,
        }
    }

    pub fn large_file_state(
        &self,
        area: DiffArea,
        path: &std::path::Path,
    ) -> Option<&gitcomet_core::large_files::LargeFileState> {
        match area {
            DiffArea::Staged => self.uncommitted_large_files.staged.get(path),
            DiffArea::Unstaged => self.uncommitted_large_files.unstaged.get(path),
        }
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
            DiffTarget::WorkingTree { path, .. } | DiffTarget::Commit { path, .. } => {
                Some(path.as_path())
            }
            DiffTarget::CommitRange { path, .. } => path.as_deref(),
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
        self.history_state.find.interrupt();
        previous
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

fn loadable_into_arc<T>(loadable: Loadable<T>) -> Loadable<Arc<T>> {
    match loadable {
        Loadable::Ready(v) => Loadable::Ready(Arc::new(v)),
        Loadable::Loading => Loadable::Loading,
        Loadable::NotLoaded => Loadable::NotLoaded,
        Loadable::Error(e) => Loadable::Error(e),
    }
}

impl RepoState {
    pub fn repository_key(&self) -> super::RepositoryKey {
        self.shared_preferences
            .as_ref()
            .map(|snapshot| snapshot.key.clone())
            .unwrap_or_else(|| {
                self.common_dir
                    .as_deref()
                    .map(|path| super::RepositoryKey::CommonDir(path.to_path_buf()))
                    .unwrap_or_else(|| super::RepositoryKey::Worktree(self.spec.workdir.clone()))
            })
    }
}
