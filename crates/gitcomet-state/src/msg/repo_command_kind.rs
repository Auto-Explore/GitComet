use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{
    ConflictSide, ForcePushLease, InteractiveRebaseEntry, PullMode, RemoteUrlKind, ResetMode,
    SafePushAfterCommitTarget, SubmoduleTrustTarget,
};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepoCommandKind {
    FetchAll,
    PruneMergedBranches,
    PruneLocalTags,
    Pull {
        mode: PullMode,
    },
    PullBranch {
        remote: String,
        branch: String,
    },
    MergeRef {
        reference: String,
    },
    SquashRef {
        reference: String,
    },
    Push,
    PushAfterCommit {
        target: SafePushAfterCommitTarget,
        set_upstream: bool,
    },
    ForcePush,
    ForcePushWithLease {
        lease: ForcePushLease,
    },
    PushSetUpstream {
        remote: String,
        branch: String,
    },
    SetUpstreamBranch {
        branch: String,
        upstream: String,
    },
    UnsetUpstreamBranch {
        branch: String,
    },
    DeleteRemoteBranch {
        remote: String,
        branch: String,
    },
    DeleteRemoteBranches {
        remote: String,
        branches: Vec<String>,
    },
    Reset {
        mode: ResetMode,
        target: String,
    },
    SquashCommits {
        oldest: CommitId,
        expected_head: CommitId,
        message: String,
        count: usize,
    },
    Rebase {
        onto: String,
    },
    RebaseContinue,
    RebaseAbort,
    InteractiveRebase {
        base: String,
        /// True when the interactive-rebase editor was opened by the user;
        /// false for automated todo-list rebases (e.g. squashing history that
        /// doesn't include HEAD), which report as a plain "Rebase".
        interactive: bool,
    },
    InteractiveCherryPick {
        entries: Vec<InteractiveRebaseEntry>,
    },
    CherryPick {
        commit_id: CommitId,
        commit: bool,
        /// Git's 1-based mainline parent for a single merge commit.
        mainline: Option<usize>,
        summary: String,
    },
<<<<<<< New base: Support explicit commit ranges when cherry-picking onto a new branch (#17)
    /// Creates a new branch `new_branch` from `base`, checks it out, and
    /// cherry-picks `range..source`'s commits (oldest first, merges skipped,
    /// `range` must be an ancestor of `source`) onto it.
    CherryPickRangeOntoNewBranch {
        base: String,
        range: String,
        source: String,
        new_branch: String,
    },
||||||| Common ancestor
=======
    /// Creates a new branch from `base`, checks it out, and cherry-picks
    /// `source..base`'s commits (oldest first, merges skipped) onto it.
    CherryPickRangeOntoNewBranch {
        base: String,
        source: String,
        new_branch: String,
    },
>>>>>>> Current commit: Add cherry-pick branch A onto B as new branch C from the action bar
    MergeAbort,
    CreateTag {
        name: String,
        target: String,
        message: Option<String>,
        annotated: bool,
    },
    DeleteTag {
        name: String,
    },
    PushTag {
        remote: String,
        name: String,
    },
    DeleteRemoteTag {
        remote: String,
        name: String,
    },
    AddRemote {
        name: String,
        url: String,
    },
    RemoveRemote {
        name: String,
    },
    SetRemoteUrl {
        name: String,
        url: String,
        kind: RemoteUrlKind,
    },
    CheckoutConflict {
        path: PathBuf,
        side: ConflictSide,
    },
    AcceptConflictDeletion {
        path: PathBuf,
    },
    CheckoutConflictBase {
        path: PathBuf,
    },
    LaunchMergetool {
        path: PathBuf,
    },
    SaveWorktreeFile {
        path: PathBuf,
        stage: bool,
    },
    AppendGitignorePatterns {
        patterns: Vec<String>,
    },
    ExportPatch {
        commit_id: CommitId,
        dest: PathBuf,
    },
    ApplyPatch {
        patch: PathBuf,
    },
    AddWorktree {
        path: PathBuf,
        reference: Option<String>,
    },
    RemoveWorktree {
        path: PathBuf,
    },
    ForceRemoveWorktree {
        path: PathBuf,
    },
    AddSubmodule {
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
        approved_sources: Vec<SubmoduleTrustTarget>,
    },
    UpdateSubmodules {
        approved_sources: Vec<SubmoduleTrustTarget>,
    },
    LoadSubmodule {
        path: PathBuf,
        approved_sources: Vec<SubmoduleTrustTarget>,
    },
    ChangeSubmodulePointer {
        path: PathBuf,
        reference: String,
    },
    RemoveSubmodule {
        path: PathBuf,
    },
    StageHunk,
    UnstageHunk,
    ApplyWorktreePatch {
        reverse: bool,
    },
}
