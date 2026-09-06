use crate::model::GitLogTagFetchMode;
use crate::model::{
    BranchExistsPromptState, ConflictFileLoadMode, DefaultTagType, GitOperationOuterOutcome,
    RemoteSettings, RepoId, SidebarDataRequest, SidebarMode,
};
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::conflict_session::ConflictSession;
use gitcomet_core::domain::*;
use gitcomet_core::error::Error;
use gitcomet_core::git_operation::{GitOperationEvent, GitOperationId};
use gitcomet_core::process::GitRuntimeState;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::GitRepository;
use gitcomet_core::services::{
    CheckoutRemoteBranchMode, CommandOutput, CommitOperationOutcome, ConflictSide, ForcePushLease,
    InteractiveRebaseEntry, PullMode, RemoteUrlKind, ResetMode, SafePushAfterCommitContext,
    SafePushAfterCommitDecision, SafePushAfterCommitTarget, SequencerState, SubmoduleTrustDecision,
    SubmoduleTrustTarget,
};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use super::repo_command_kind::RepoCommandKind;
use super::repo_external_change::RepoExternalChange;
use super::{RepoPath, RepoPathList};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepoActionKind {
    CheckoutBranch,
    CheckoutRemoteBranch,
    CheckoutCommit,
    CherryPickCommit,
    RevertCommit,
    CreateBranch,
    CreateBranchAndCheckout,
    RenameBranch,
    DeleteBranch,
    ForceDeleteBranch,
    DeleteBranches,
    StagePath,
    StagePaths,
    UnstagePath,
    UnstagePaths,
    DiscardWorktreeChangesPath,
    DiscardWorktreeChangesPaths,
    Stash,
    ApplyStash,
    PopStash,
    DropStash,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BranchExistsChoice {
    Cancel,
    CheckoutExisting,
    OverwriteAndCheckout,
}

impl RepoActionKind {
    pub(crate) fn hook_activity_label(self) -> &'static str {
        match self {
            Self::CheckoutBranch | Self::CheckoutRemoteBranch | Self::CheckoutCommit => "Checkout",
            Self::CherryPickCommit => "Cherry-pick",
            Self::RevertCommit => "Revert",
            Self::CreateBranch => "Create branch",
            Self::CreateBranchAndCheckout => "Create branch and checkout",
            Self::RenameBranch => "Rename branch",
            Self::DeleteBranch | Self::ForceDeleteBranch | Self::DeleteBranches => "Delete branch",
            Self::StagePath | Self::StagePaths => "Stage",
            Self::UnstagePath | Self::UnstagePaths => "Unstage",
            Self::DiscardWorktreeChangesPath | Self::DiscardWorktreeChangesPaths => {
                "Discard changes"
            }
            Self::Stash => "Stash",
            Self::ApplyStash => "Apply stash",
            Self::PopStash => "Pop stash",
            Self::DropStash => "Drop stash",
        }
    }
}

/// How a history-row click mutates the commit selection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitSelectMode {
    /// Plain click: collapse to the clicked commit.
    Single,
    /// Ctrl/Cmd click: add or remove the clicked commit.
    Toggle,
    /// Shift click: select the range between the anchor and the clicked commit.
    Range,
    /// Move focus to the clicked commit while preserving an existing
    /// multi-selection that already contains it (used by right-click so the
    /// details pane follows the menu target); collapses to the clicked commit
    /// when it is not part of the selection.
    PreserveIfSelected,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictAutosolveMode {
    Safe,
    Regex,
    History,
}

impl ConflictAutosolveMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::Regex => "regex",
            Self::History => "history",
        }
    }
}

/// A KDiff3-style source choice applied in bulk. The blocks it reaches depend
/// on the [`ConflictBulkScope`] it is dispatched with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictBulkChoice {
    Base,
    Ours,
    Theirs,
    Both,
}

/// Which blocks a bulk choice reaches, mirroring KDiff3's `chooseGlobal`
/// `bConflictsOnly` / `bWhiteSpaceOnly` pair.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum ConflictBulkScope {
    /// "Choose A/B/C Everywhere": every merge-plan delta, including the ones
    /// the planner already selected automatically.
    #[default]
    AllDeltas,
    /// "Choose A/B/C for All Unsolved Whitespace Conflicts": only blocks still
    /// unresolved and classified as whitespace-only, skipping hand-edited ones.
    UnsolvedWhitespace,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictRegionChoice {
    Base,
    Ours,
    Theirs,
    Both,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConflictRegionResolutionUpdate {
    pub region_index: usize,
    pub resolution: gitcomet_core::conflict_session::ConflictRegionResolution,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ConflictAutosolveStats {
    pub pass1: usize,
    pub pass2_split: usize,
    pub pass1_after_split: usize,
    pub regex: usize,
    pub history: usize,
}

impl ConflictAutosolveStats {
    pub fn total_resolved(self) -> usize {
        self.pass1 + self.pass2_split + self.pass1_after_split + self.regex + self.history
    }
}

/// Why the file-system watcher is in a degraded state (carried by [`Msg::RepoWatchDegraded`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepoWatchDegradedReason {
    /// The worktree has more non-ignored folders than the watch budget, so its source folders are
    /// not watched live at all. Carries the folder count.
    TooManyFolders { dir_count: usize },
    /// Some per-directory watches could not be added (the kernel inotify limit was reached), so part
    /// of the worktree is not watched live. Carries the number of folders left unwatched.
    WatchLimitReached { unwatched_dirs: usize },
}

// Dispatch keeps internal messages inline so the hot reducer path does not
// require an additional allocation for every effect completion.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum Msg {
    OpenRepo(PathBuf),
    /// Opens a repository candidate supplied by an external file-system drop.
    /// The candidate is not persisted until the backend has opened it
    /// successfully, and any open failure discards its temporary tab.
    OpenRepoFromExternalDrop(PathBuf),
    RestoreSession {
        open_repos: Vec<PathBuf>,
        active_repo: Option<PathBuf>,
    },
    CloseRepo {
        repo_id: RepoId,
    },
    CloseRepos {
        repo_ids: Vec<RepoId>,
        activate_after: Option<RepoId>,
    },
    ShowBannerError {
        repo_id: Option<RepoId>,
        message: String,
    },
    DismissBannerError,
    DismissRepoError {
        repo_id: RepoId,
    },
    CancelGitOperation {
        repo_id: RepoId,
        operation_id: GitOperationId,
    },
    SubmitAuthPrompt {
        username: Option<String>,
        secret: String,
    },
    CancelAuthPrompt,
    SetGitRuntimeState(GitRuntimeState),
    SetRemoteUrlPolicy(RemoteUrlPolicy),
    SetGitLogSettings {
        show_history_tags: bool,
        tag_fetch_mode: GitLogTagFetchMode,
    },
    SetRemoteSettings(RemoteSettings),
    SetDefaultTagType(DefaultTagType),
    SetActiveRepo {
        repo_id: RepoId,
    },
    ReorderRepoTabs {
        repo_id: RepoId,
        insert_before: Option<RepoId>,
    },
    ReloadRepo {
        repo_id: RepoId,
    },
    RepoActivated {
        repo_id: RepoId,
    },
    RepoExternallyChanged {
        repo_id: RepoId,
        change: RepoExternalChange,
    },
    /// The file-system watcher could not fully watch the worktree, so live change detection is
    /// degraded. The repository still refreshes when the window regains focus; the `reason` carries
    /// the detail for the user-facing warning.
    RepoWatchDegraded {
        repo_id: RepoId,
        reason: RepoWatchDegradedReason,
    },
    SetHistoryScope {
        repo_id: RepoId,
        scope: LogScope,
    },
    /// Restricts the history to commits authored by `author` (case-insensitive
    /// match on the author name). `None` clears the filter.
    SetHistoryAuthorFilter {
        repo_id: RepoId,
        author: Option<String>,
    },
    LoadMoreHistory {
        repo_id: RepoId,
    },
    SelectCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    /// Modifier-aware history selection. `visible_order` (the visible commit
    /// ids in log order) is only provided for `Range` clicks.
    SelectCommitMulti {
        repo_id: RepoId,
        commit_id: CommitId,
        mode: CommitSelectMode,
        clicked_index: Option<usize>,
        visible_order: Option<Vec<CommitId>>,
    },
    ClearCommitSelection {
        repo_id: RepoId,
    },
    /// Compare two points (commits, or branch/tag tips resolved to commit ids).
    /// `from` is the base/older side. Loads the changed-file list and drives the
    /// whole-range patch into the diff pane.
    CompareCommitRange {
        repo_id: RepoId,
        from: CommitId,
        to: CommitId,
        from_label: String,
        to_label: String,
    },
    /// Compare a commit/branch/tag (resolved to `from`) against the live working
    /// tree. Loads the changed-file list; the tip tracks uncommitted changes.
    CompareWithWorkingTree {
        repo_id: RepoId,
        from: CommitId,
        from_label: String,
    },
    /// Clear an active range comparison, returning to single/empty selection.
    ClearComparison {
        repo_id: RepoId,
    },
    /// Mark a commit/branch/tag (resolved to `commit_id`) as the base for a
    /// later "Compare with marked".
    MarkForComparison {
        repo_id: RepoId,
        commit_id: CommitId,
        label: String,
    },
    /// Compare the previously marked point (base) against this commit/branch/tag.
    CompareWithMarked {
        repo_id: RepoId,
        commit_id: CommitId,
        label: String,
    },
    /// Forget the marked-for-comparison point.
    ClearComparisonMark {
        repo_id: RepoId,
    },
    SelectDiff {
        repo_id: RepoId,
        target: DiffTarget,
    },
    OpenInlineSubmoduleDiff {
        repo_id: RepoId,
        origin: crate::model::ForeignDiffOrigin,
        submodule_repo_path: PathBuf,
        parent_submodule_path: PathBuf,
        entries: Vec<crate::model::InlineSubmoduleDiffEntry>,
        selected_ix: usize,
    },
    SelectInlineSubmoduleDiff {
        repo_id: RepoId,
        selected_ix: usize,
    },
    CloseInlineSubmoduleDiff {
        repo_id: RepoId,
    },
    SelectConflictDiff {
        repo_id: RepoId,
        path: PathBuf,
    },
    ClearDiffSelection {
        repo_id: RepoId,
    },
    EnsureSidebarData {
        repo_id: RepoId,
        request: SidebarDataRequest,
    },
    LoadStashes {
        repo_id: RepoId,
    },
    LoadConflictFile {
        repo_id: RepoId,
        path: PathBuf,
        mode: ConflictFileLoadMode,
    },
    LoadReflog {
        repo_id: RepoId,
    },
    LoadRecentCommitMessages {
        repo_id: RepoId,
        limit: usize,
    },
    /// Full `%B` message of a single commit, for the history hover card.
    /// Message-only, so it skips the tree diff `commit_details` pays for.
    LoadHoverCommitMessage {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    LoadFileHistory {
        repo_id: RepoId,
        path: PathBuf,
        limit: usize,
    },
    LoadBlame {
        repo_id: RepoId,
        path: PathBuf,
        source: gitcomet_core::domain::BlameSource,
    },
    LoadWorktrees {
        repo_id: RepoId,
    },
    /// Uncommitted-change counts for the other linked worktrees. Opens a
    /// throwaway handle per worktree path, so it is scheduled deliberately
    /// rather than on every refresh.
    LoadWorktreeDirty {
        repo_id: RepoId,
    },
    /// Select the history row for a linked worktree's uncommitted changes, so
    /// the details pane shows that worktree's files instead of a commit.
    SelectWorktreeUncommitted {
        repo_id: RepoId,
        path: PathBuf,
    },
    /// On-demand load of tip-commit author/date/summary for every ref. Only
    /// requested by pickers that render it.
    LoadRefMetadata {
        repo_id: RepoId,
    },
    LoadSubmodules {
        repo_id: RepoId,
    },
    LoadTags {
        repo_id: RepoId,
    },
    LoadRemoteTags {
        repo_id: RepoId,
    },
    RefreshBranches {
        repo_id: RepoId,
    },
    LoadFileBrowser {
        repo_id: RepoId,
        source: FileSource,
    },
    ToggleFileBrowserDir {
        repo_id: RepoId,
        path: PathBuf,
    },
    /// Expand or collapse `path` in the file explorer together with every
    /// directory beneath it. The whole tree is enumerated up front, so the
    /// descendants are already known without loading anything.
    SetFileBrowserDirExpandedRecursive {
        repo_id: RepoId,
        path: PathBuf,
        expanded: bool,
    },
    SetFileBrowserSearch {
        repo_id: RepoId,
        query: String,
    },
    /// Expand every directory leading to `path` in the file explorer, so the
    /// row for that file becomes visible. Clears any active file search.
    RevealFileBrowserPath {
        repo_id: RepoId,
        path: PathBuf,
    },
    SetFileBrowserSource {
        repo_id: RepoId,
        source: FileSource,
    },
    OpenFileContent {
        repo_id: RepoId,
        source: FileSource,
        path: PathBuf,
    },
    /// Open `path` as an editable buffer over the working-tree file. Always
    /// edits the workspace copy, so it re-targets the working tree even when it
    /// was invoked from a commit's file list.
    OpenFileEditor {
        repo_id: RepoId,
        path: PathBuf,
    },
    /// Leave the editor, restoring the diff or read-only content preview that
    /// opened it. *Entering* always goes through `OpenFileEditor`, which
    /// re-targets the working tree while retaining that return destination.
    ExitDiffEditMode {
        repo_id: RepoId,
    },
    /// Open the given file as it was in the parent of `commit_id` (the
    /// revision just before that commit's change). The parent is resolved
    /// asynchronously; if `commit_id` is a root commit this is a no-op.
    OpenFileAtCommitParent {
        repo_id: RepoId,
        commit_id: CommitId,
        path: PathBuf,
    },
    /// Open the file's content at `commit_id`, resolving `path` to the name the
    /// file has in that commit's tree (following renames) before opening. Used
    /// by the file-history list so navigating across a rename does not look up a
    /// name that is absent from the target commit's tree. Resolved
    /// asynchronously; falls back to `path` when no rename mapping is found.
    OpenFileAtCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        path: PathBuf,
    },
    /// Show this file's diff using its name at the selected commit.
    ShowFileChangesAtCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        path: PathBuf,
    },
    BrowseRepositoryAtCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    /// Show a commit referenced from elsewhere — a SHA in a commit message, a
    /// branch tip — without waiting for the history walk to reach its row.
    ///
    /// `reference` may be abbreviated. It is resolved against the object
    /// database, so an ambiguous or unknown reference is reported instead of
    /// sending the log on a walk that can only end in silence.
    RevealCommit {
        repo_id: RepoId,
        reference: CommitId,
    },
    /// The history view has finished (or given up on) the pending reveal.
    FinishCommitReveal {
        repo_id: RepoId,
    },
    ResetBrowseToLive {
        repo_id: RepoId,
    },
    /// Step back through the cross-file viewer history (browser-style),
    /// replaying the previously viewed file/version without recording it.
    ViewerNavBack {
        repo_id: RepoId,
    },
    /// Step forward through the cross-file viewer history.
    ViewerNavForward {
        repo_id: RepoId,
    },
    /// Step back through the broad global navigation history (mouse back
    /// button): diffs, file-content views, and commit selections.
    GlobalNavBack {
        repo_id: RepoId,
    },
    /// Step forward through the global navigation history (mouse forward button).
    GlobalNavForward {
        repo_id: RepoId,
    },
    SetSidebarMode {
        mode: SidebarMode,
    },
    StageHunk {
        repo_id: RepoId,
        patch: String,
    },
    UnstageHunk {
        repo_id: RepoId,
        patch: String,
    },
    ApplyWorktreePatch {
        repo_id: RepoId,
        patch: String,
        reverse: bool,
    },
    CheckoutBranch {
        repo_id: RepoId,
        name: String,
    },
    CheckoutRemoteBranch {
        repo_id: RepoId,
        remote: String,
        branch: String,
        local_branch: String,
        mode: CheckoutRemoteBranchMode,
    },
    CheckoutCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    CherryPickCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        commit: bool,
        mainline: Option<usize>,
        summary: String,
    },
    RevertCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    CreateBranch {
        repo_id: RepoId,
        name: String,
        target: String,
    },
    CreateBranchAndCheckout {
        repo_id: RepoId,
        name: String,
        target: String,
        /// Reset the branch to `target` first when a branch with this name
        /// already exists, instead of failing with "already exists".
        force: bool,
    },
    ResolveBranchExistsPrompt {
        prompt: BranchExistsPromptState,
        choice: BranchExistsChoice,
    },
    ShowBranchExistsPrompt {
        prompt: BranchExistsPromptState,
    },
    RenameBranch {
        repo_id: RepoId,
        old_name: String,
        new_name: String,
        /// Replace an existing `new_name` instead of failing with "already exists".
        force: bool,
    },
    DeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    ForceDeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    /// Delete every named local branch in one action.
    ///
    /// `force` picks `-D` over `-d` for the whole batch: a folder of finished
    /// feature branches is exactly the case where an unforced delete fails on
    /// every one of them, so the choice is made once up front rather than
    /// escalated per branch.
    DeleteBranches {
        repo_id: RepoId,
        names: Vec<String>,
        force: bool,
    },
    CloneRepo {
        url: String,
        dest: PathBuf,
    },
    AbortCloneRepo {
        dest: PathBuf,
    },
    ExportPatch {
        repo_id: RepoId,
        commit_id: CommitId,
        dest: PathBuf,
    },
    ApplyPatch {
        repo_id: RepoId,
        patch: PathBuf,
    },
    AddWorktree {
        repo_id: RepoId,
        path: PathBuf,
        reference: Option<String>,
    },
    RemoveWorktree {
        repo_id: RepoId,
        path: PathBuf,
    },
    ForceRemoveWorktree {
        repo_id: RepoId,
        path: PathBuf,
    },
    AddSubmodule {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
    },
    AddSubmoduleTrusted {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
        approved_sources: Vec<SubmoduleTrustTarget>,
    },
    UpdateSubmodules {
        repo_id: RepoId,
    },
    UpdateSubmodulesTrusted {
        repo_id: RepoId,
        approved_sources: Vec<SubmoduleTrustTarget>,
    },
    LoadSubmodule {
        repo_id: RepoId,
        path: PathBuf,
    },
    LoadSubmoduleTrusted {
        repo_id: RepoId,
        path: PathBuf,
        approved_sources: Vec<SubmoduleTrustTarget>,
    },
    ConfirmSubmoduleTrustPrompt,
    CancelSubmoduleTrustPrompt,
    ChangeSubmodulePointer {
        repo_id: RepoId,
        path: PathBuf,
        reference: String,
    },
    RemoveSubmodule {
        repo_id: RepoId,
        path: PathBuf,
    },
    StagePath {
        repo_id: RepoId,
        path: PathBuf,
    },
    StagePaths {
        repo_id: RepoId,
        paths: RepoPathList,
    },
    UnstagePath {
        repo_id: RepoId,
        path: PathBuf,
    },
    UnstagePaths {
        repo_id: RepoId,
        paths: RepoPathList,
    },
    DiscardWorktreeChangesPath {
        repo_id: RepoId,
        path: PathBuf,
    },
    DiscardWorktreeChangesPaths {
        repo_id: RepoId,
        paths: Vec<PathBuf>,
    },
    SaveWorktreeFile {
        repo_id: RepoId,
        path: PathBuf,
        contents: String,
        stage: bool,
    },
    /// Append patterns to the repository-root `.gitignore`, creating it when
    /// absent. Patterns already present are skipped, so re-running is a no-op.
    AppendGitignorePatterns {
        repo_id: RepoId,
        patterns: Vec<String>,
    },
    Commit {
        repo_id: RepoId,
        message: String,
        push_after_commit: bool,
    },
    CommitAmend {
        repo_id: RepoId,
        message: String,
        push_after_commit: bool,
    },
    SafePushAfterCommit {
        repo_id: RepoId,
        context: SafePushAfterCommitContext,
    },
    FetchAll {
        repo_id: RepoId,
    },
    PruneMergedBranches {
        repo_id: RepoId,
    },
    PruneLocalTags {
        repo_id: RepoId,
    },
    Pull {
        repo_id: RepoId,
        mode: PullMode,
    },
    PullBranch {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    MergeRef {
        repo_id: RepoId,
        reference: String,
    },
    SquashRef {
        repo_id: RepoId,
        reference: String,
    },
    Push {
        repo_id: RepoId,
    },
    PushAfterCommit {
        repo_id: RepoId,
        target: SafePushAfterCommitTarget,
        set_upstream: bool,
    },
    ForcePush {
        repo_id: RepoId,
    },
    ForcePushWithLease {
        repo_id: RepoId,
        lease: ForcePushLease,
    },
    PushSetUpstream {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    SetUpstreamBranch {
        repo_id: RepoId,
        branch: String,
        upstream: Upstream,
    },
    UnsetUpstreamBranch {
        repo_id: RepoId,
        branch: String,
    },
    DeleteRemoteBranch {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    /// Delete several branches on one remote. Kept to a single remote so it
    /// stays one `git push --delete` rather than a round trip per branch.
    DeleteRemoteBranches {
        repo_id: RepoId,
        remote: String,
        branches: Vec<String>,
    },
    Reset {
        repo_id: RepoId,
        target: String,
        mode: ResetMode,
    },
    /// Builds the squash message preview for the current multi-selection so
    /// the squash prompt can prefill its message input.
    PrepareSquash {
        repo_id: RepoId,
    },
    /// Squashes the linear range `oldest..=expected_head` into one commit.
    /// The reducer re-validates the range against the current selection and
    /// log before emitting the effect.
    SquashCommits {
        repo_id: RepoId,
        oldest: CommitId,
        expected_head: CommitId,
        message: String,
        count: usize,
    },
    Rebase {
        repo_id: RepoId,
        onto: String,
    },
    RebaseContinue {
        repo_id: RepoId,
    },
    RebaseAbort {
        repo_id: RepoId,
    },
    LoadInteractiveRebaseSetup {
        repo_id: RepoId,
        base: String,
    },
    OpenInteractiveCherryPickSetup {
        repo_id: RepoId,
        entries: Vec<InteractiveRebaseEntry>,
        source_colors: Vec<(String, u8)>,
    },
    InteractiveRebase {
        repo_id: RepoId,
        base: String,
        entries: Vec<InteractiveRebaseEntry>,
    },
    InteractiveCherryPick {
        repo_id: RepoId,
        entries: Vec<InteractiveRebaseEntry>,
    },
    CancelInteractiveRebaseSetup {
        repo_id: RepoId,
    },
    CancelInteractiveCherryPickSetup {
        repo_id: RepoId,
    },
    MergeAbort {
        repo_id: RepoId,
    },
    CreateTag {
        repo_id: RepoId,
        name: String,
        target: String,
        message: Option<String>,
        annotated: bool,
    },
    DeleteTag {
        repo_id: RepoId,
        name: String,
    },
    PushTag {
        repo_id: RepoId,
        remote: String,
        name: String,
    },
    DeleteRemoteTag {
        repo_id: RepoId,
        remote: String,
        name: String,
    },
    AddRemote {
        repo_id: RepoId,
        name: String,
        url: String,
    },
    RemoveRemote {
        repo_id: RepoId,
        name: String,
    },
    SetRemoteUrl {
        repo_id: RepoId,
        name: String,
        url: String,
        kind: RemoteUrlKind,
    },
    CheckoutConflictSide {
        repo_id: RepoId,
        path: PathBuf,
        side: ConflictSide,
    },
    AcceptConflictDeletion {
        repo_id: RepoId,
        path: PathBuf,
    },
    CheckoutConflictBase {
        repo_id: RepoId,
        path: PathBuf,
    },
    LaunchMergetool {
        repo_id: RepoId,
        path: PathBuf,
    },
    RecordConflictAutosolveTelemetry {
        repo_id: RepoId,
        path: Option<PathBuf>,
        mode: ConflictAutosolveMode,
        total_conflicts_before: usize,
        total_conflicts_after: usize,
        unresolved_before: usize,
        unresolved_after: usize,
        stats: ConflictAutosolveStats,
    },
    ConflictSetHideResolved {
        repo_id: RepoId,
        path: RepoPath,
        hide_resolved: bool,
    },
    ConflictApplyBulkChoice {
        repo_id: RepoId,
        path: RepoPath,
        choice: ConflictBulkChoice,
        scope: ConflictBulkScope,
    },
    ConflictSetRegionChoice {
        repo_id: RepoId,
        path: RepoPath,
        region_index: usize,
        choice: ConflictRegionChoice,
    },
    /// Toggle one merge source, appending it after already-selected sources.
    ConflictToggleRegionSource {
        repo_id: RepoId,
        path: RepoPath,
        region_index: usize,
        source: gitcomet_core::merge::MergeSource,
    },
    /// Replace a region's complete ordered source selection.
    ConflictReplaceRegionSelection {
        repo_id: RepoId,
        path: RepoPath,
        region_index: usize,
        selection: gitcomet_core::merge::OrderedSelection,
    },
    /// Toggle one source on a semantic merge-plan block. Unlike region
    /// actions, this also addresses automatically selected deltas that do not
    /// render conflict markers.
    ConflictTogglePlanBlockSource {
        repo_id: RepoId,
        path: RepoPath,
        block_id: gitcomet_core::merge::MergeBlockId,
        source: gitcomet_core::merge::MergeSource,
    },
    /// Replace a semantic merge-plan block's complete ordered selection.
    ConflictReplacePlanBlockSelection {
        repo_id: RepoId,
        path: RepoPath,
        block_id: gitcomet_core::merge::MergeBlockId,
        selection: gitcomet_core::merge::OrderedSelection,
    },
    ConflictSyncRegionResolutions {
        repo_id: RepoId,
        path: RepoPath,
        updates: Vec<ConflictRegionResolutionUpdate>,
    },
    ConflictApplyAutosolve {
        repo_id: RepoId,
        path: RepoPath,
        mode: ConflictAutosolveMode,
        whitespace_normalize: bool,
    },
    ConflictResetResolutions {
        repo_id: RepoId,
        path: RepoPath,
    },
    /// section 30 split: rewrite one in-memory conflict block into 2–3 blocks
    /// at block-local line boundaries.
    ConflictSplitRegion {
        repo_id: RepoId,
        path: RepoPath,
        region_index: usize,
        boundaries: gitcomet_core::conflict_session::ConflictRegionSplitBoundaries,
        /// Resolver revision from which the region index and boundaries were
        /// calculated. Stale requests are rejected before editing the session.
        expected_conflict_rev: u64,
    },
    /// KDiff3 manual diff help: pin one line range per source so the planner
    /// must align them, then replan the file around that constraint.
    ConflictAddManualAlignment {
        repo_id: RepoId,
        path: RepoPath,
        alignment: gitcomet_core::merge::ManualAlignment,
        /// Resolver revision the pinned ranges were read from. Stale requests
        /// are rejected before replanning.
        expected_conflict_rev: u64,
    },
    /// Drop every manual alignment and replan from the automatic one.
    ConflictClearManualAlignments {
        repo_id: RepoId,
        path: RepoPath,
        expected_conflict_rev: u64,
    },
    /// section 30 join: merge conflict blocks `region_index` and `region_index + 1`,
    /// absorbing the context between them into every side.
    ConflictJoinRegions {
        repo_id: RepoId,
        path: RepoPath,
        region_index: usize,
        /// Resolver revision captured when the menu entry was built. The
        /// reducer rejects stale actions atomically after region indices move.
        expected_conflict_rev: u64,
    },
    Stash {
        repo_id: RepoId,
        message: String,
        include_untracked: bool,
    },
    ApplyStash {
        repo_id: RepoId,
        index: usize,
    },
    PopStash {
        repo_id: RepoId,
        index: usize,
    },
    DropStash {
        repo_id: RepoId,
        index: usize,
    },
    Internal(InternalMsg),
}

pub enum InternalMsg {
    GitOperationStarted {
        repo_id: RepoId,
        operation_id: GitOperationId,
        label: String,
        context: Option<String>,
        time: SystemTime,
    },
    GitOperationEvent {
        repo_id: RepoId,
        operation_id: GitOperationId,
        event: GitOperationEvent,
    },
    /// Wraps the operation's ordinary completion so the command log and hook
    /// activity are reduced atomically and cannot produce duplicate notices.
    GitOperationFinished {
        repo_id: RepoId,
        operation_id: GitOperationId,
        outer_outcome: GitOperationOuterOutcome,
        duration: Duration,
        message: Box<InternalMsg>,
    },
    SessionPersistFailed {
        repo_id: Option<RepoId>,
        action: &'static str,
        error: String,
    },
    CloneRepoProgress {
        dest: Arc<PathBuf>,
        line: String,
    },
    CloneRepoFinished {
        url: String,
        dest: PathBuf,
        result: Result<CommandOutput, Error>,
    },
    RepoLoadFinished {
        repo_id: RepoId,
        load_epoch: u64,
        message: Box<InternalMsg>,
    },
    RepoOpenedOk {
        repo_id: RepoId,
        spec: RepoSpec,
        repo: Arc<dyn GitRepository>,
    },
    RepoOpenedErr {
        repo_id: RepoId,
        spec: RepoSpec,
        error: Error,
    },
    BranchesLoaded {
        repo_id: RepoId,
        result: Result<Vec<Branch>, Error>,
    },
    RemotesLoaded {
        repo_id: RepoId,
        result: Result<Vec<Remote>, Error>,
    },
    RemoteBranchesLoaded {
        repo_id: RepoId,
        result: Result<Vec<RemoteBranch>, Error>,
    },
    WorktreeStatusLoaded {
        repo_id: RepoId,
        result: Result<Vec<FileStatus>, Error>,
    },
    StagedStatusLoaded {
        repo_id: RepoId,
        result: Result<Vec<FileStatus>, Error>,
    },
    StatusLoaded {
        repo_id: RepoId,
        result: Result<RepoStatus, Error>,
    },
    HeadBranchLoaded {
        repo_id: RepoId,
        result: Result<String, Error>,
    },
    UpstreamDivergenceLoaded {
        repo_id: RepoId,
        result: Result<Option<UpstreamDivergence>, Error>,
    },
    LogLoaded {
        repo_id: RepoId,
        seq: crate::model::LogLoadSeq,
        scope: LogScope,
        cursor: Option<LogCursor>,
        result: Result<LogPage, Error>,
    },
    /// A partially built log page, reported while the walk is still running so
    /// an author filter on a large repository shows what it has found instead
    /// of nothing. `commits` is the page so far — successive chunks are
    /// prefixes of each other — and `scanned` counts commits visited.
    LogChunkLoaded {
        repo_id: RepoId,
        seq: crate::model::LogLoadSeq,
        commits: Vec<Commit>,
        scanned: u64,
    },
    TagsLoaded {
        repo_id: RepoId,
        result: Result<Vec<Tag>, Error>,
    },
    RemoteTagsLoaded {
        repo_id: RepoId,
        result: Result<Vec<RemoteTag>, Error>,
    },
    StashesLoaded {
        repo_id: RepoId,
        result: Result<Vec<StashEntry>, Error>,
    },
    ReflogLoaded {
        repo_id: RepoId,
        result: Result<Vec<ReflogEntry>, Error>,
    },
    RecentCommitMessagesLoaded {
        repo_id: RepoId,
        request_rev: u64,
        result: Result<Vec<RecentCommitMessage>, Error>,
    },
    RebaseStateLoaded {
        repo_id: RepoId,
        result: Result<SequencerState, Error>,
    },
    InteractiveRebaseSetupLoaded {
        repo_id: RepoId,
        base: String,
        result: Result<Vec<InteractiveRebaseEntry>, Error>,
    },
    /// Repository-ordered selected commit ids with their full `%B` messages.
    /// `requested_ids` identifies the setup that launched the detached load
    /// so a late response cannot alter a newer selection.
    InteractiveCherryPickMessagesLoaded {
        repo_id: RepoId,
        requested_ids: Vec<String>,
        result: Result<Vec<(String, String)>, Error>,
    },
    MergeCommitMessageLoaded {
        repo_id: RepoId,
        result: Result<Option<String>, Error>,
    },
    HoverCommitMessageLoaded {
        repo_id: RepoId,
        commit_id: CommitId,
        result: Result<String, Error>,
    },
    FileHistoryLoaded {
        repo_id: RepoId,
        path: PathBuf,
        /// The cursor the page was requested with: `None` for the first page,
        /// `Some` for a continuation to append to it.
        cursor: Option<LogCursor>,
        result: Result<LogPage, Error>,
    },
    BlameLoaded {
        repo_id: RepoId,
        path: PathBuf,
        source: gitcomet_core::domain::BlameSource,
        result: Result<Vec<gitcomet_core::services::BlameLine>, Error>,
    },
    ConflictFileLoaded {
        repo_id: RepoId,
        path: PathBuf,
        result: Box<Result<Option<crate::model::ConflictFile>, Error>>,
        conflict_session: Option<ConflictSession>,
    },
    WorktreesLoaded {
        repo_id: RepoId,
        result: Result<Vec<Worktree>, Error>,
    },
    WorktreeDirtyLoaded {
        repo_id: RepoId,
        result: Result<Vec<WorktreeDirtySummary>, Error>,
    },
    RefMetadataLoaded {
        repo_id: RepoId,
        result: Result<Vec<(String, RefMetadata)>, Error>,
    },
    SubmodulesLoaded {
        repo_id: RepoId,
        result: Result<Vec<Submodule>, Error>,
    },
    FileBrowserLoaded {
        repo_id: RepoId,
        source: FileSource,
        result: Result<Vec<FileEntry>, Error>,
    },
    SubmoduleAddTrustChecked {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
        result: Result<SubmoduleTrustDecision, Error>,
    },
    SubmoduleUpdateTrustChecked {
        repo_id: RepoId,
        result: Result<SubmoduleTrustDecision, Error>,
    },
    SubmoduleLoadTrustChecked {
        repo_id: RepoId,
        path: PathBuf,
        result: Result<SubmoduleTrustDecision, Error>,
    },
    CommitDetailsLoaded {
        repo_id: RepoId,
        commit_id: CommitId,
        result: Result<CommitDetails, Error>,
    },
    /// A [`Msg::RevealCommit`] reference resolved (or failed to).
    CommitRevealResolved {
        repo_id: RepoId,
        reference: CommitId,
        result: Result<CommitDetails, Error>,
    },
    RangeFilesLoaded {
        repo_id: RepoId,
        from: CommitId,
        /// `None` when the tip is the working tree.
        to: Option<CommitId>,
        /// The `Effect::LoadRangeFiles` request this answers.
        request: u64,
        result: Result<Vec<CommitFileChange>, Error>,
    },
    SquashMessagePreviewLoaded {
        repo_id: RepoId,
        oldest: CommitId,
        head: CommitId,
        result: Result<String, Error>,
    },
    SquashRebaseSetupLoaded {
        repo_id: RepoId,
        base: String,
        actual_head: CommitId,
        selected_ids: Vec<CommitId>,
        reword_id: CommitId,
        message: String,
        count: usize,
        result: Result<Vec<InteractiveRebaseEntry>, Error>,
    },
    DiffLoaded {
        repo_id: RepoId,
        target: DiffTarget,
        result: Result<Diff, Error>,
    },
    DiffFileLoaded {
        repo_id: RepoId,
        target: DiffTarget,
        result: Result<Option<FileDiffText>, Error>,
    },
    DiffPreviewTextFileLoaded {
        repo_id: RepoId,
        target: DiffTarget,
        side: DiffPreviewTextSide,
        result: Result<Option<PathBuf>, Error>,
    },
    SubmoduleSummaryLoaded {
        repo_id: RepoId,
        target: DiffTarget,
        result: Result<SubmoduleDiffSummary, Error>,
    },
    InlineSubmoduleDiffLoaded {
        repo_id: RepoId,
        inline_rev: u64,
        target: DiffTarget,
        result: Result<Diff, Error>,
    },
    InlineSubmoduleDiffFileLoaded {
        repo_id: RepoId,
        inline_rev: u64,
        target: DiffTarget,
        result: Result<Option<FileDiffText>, Error>,
    },
    InlineSubmoduleDiffFileImageLoaded {
        repo_id: RepoId,
        inline_rev: u64,
        target: DiffTarget,
        result: Result<Option<FileDiffImage>, Error>,
    },
    DiffFileImageLoaded {
        repo_id: RepoId,
        target: DiffTarget,
        result: Result<Option<FileDiffImage>, Error>,
    },
    RepoActionFinished {
        repo_id: RepoId,
        action: RepoActionKind,
        result: Result<(), Error>,
    },
    /// The action ran into an existing branch; open the collision prompt.
    BranchAlreadyExists {
        action: RepoActionKind,
        prompt: BranchExistsPromptState,
    },
    /// The action was carried out in (or redirected to) another worktree that
    /// has the branch checked out; on success that worktree is opened.
    RepoActionFinishedInWorktree {
        repo_id: RepoId,
        action: RepoActionKind,
        worktree_path: PathBuf,
        result: Result<(), Error>,
    },
    CommitFinished {
        repo_id: RepoId,
        result: Result<CommitOperationOutcome, Error>,
    },
    CommitAmendFinished {
        repo_id: RepoId,
        result: Result<CommitOperationOutcome, Error>,
    },
    SafePushAfterCommitFinished {
        repo_id: RepoId,
        context: SafePushAfterCommitContext,
        auth: Option<StagedGitAuth>,
        result: Result<SafePushAfterCommitDecision, Error>,
    },
    RepoCommandFinished {
        repo_id: RepoId,
        command: RepoCommandKind,
        result: Result<CommandOutput, Error>,
    },
}

impl From<InternalMsg> for Msg {
    fn from(value: InternalMsg) -> Self {
        Self::Internal(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{InternalMsg, Msg, RepoActionKind};
    use crate::model::RepoId;
    use gitcomet_core::error::{Error, ErrorKind};
    use std::path::PathBuf;

    #[test]
    fn wraps_internal_messages() {
        let msg: Msg = InternalMsg::RepoActionFinished {
            repo_id: RepoId(7),
            action: RepoActionKind::CheckoutBranch,
            result: Ok(()),
        }
        .into();

        assert!(matches!(
            msg,
            Msg::Internal(InternalMsg::RepoActionFinished {
                repo_id: RepoId(7),
                action: RepoActionKind::CheckoutBranch,
                result: Ok(())
            })
        ));
    }

    #[test]
    fn clone_repo_finished_debug_keeps_result_compact() {
        let msg: Msg = InternalMsg::CloneRepoFinished {
            url: "https://example.invalid/repo.git".to_string(),
            dest: PathBuf::from("/tmp/repo"),
            result: Err(Error::new(ErrorKind::Backend("clone failed".to_string()))),
        }
        .into();
        let debug = format!("{msg:?}");

        assert!(debug.contains("CloneRepoFinished"));
        assert!(debug.contains("ok: false"));
        assert!(!debug.contains("clone failed"));
    }
}
