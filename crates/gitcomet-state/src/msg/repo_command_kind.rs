use gitcomet_core::domain::{CommitId, Upstream};
use gitcomet_core::services::{
    ConflictSide, ForcePushLease, InteractiveRebaseEntry, PullMode, RemoteUrlKind, ResetMode,
    SafePushAfterCommitTarget, SubmoduleTrustTarget, WorktreeCheckout,
};
use std::path::PathBuf;

/// How the command log and toasts name what a new worktree checks out.
pub(crate) fn worktree_checkout_label(checkout: &WorktreeCheckout) -> String {
    match checkout {
        WorktreeCheckout::Existing { reference } => reference.clone(),
        WorktreeCheckout::NewBranch { name, start: None } => format!("new branch {name}"),
        WorktreeCheckout::NewBranch {
            name,
            start: Some(start),
        } => format!("new branch {name} from {start}"),
        WorktreeCheckout::TrackRemote { name, upstream } => format!(
            "new branch {name} tracking {}/{}",
            upstream.remote, upstream.branch
        ),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepoCommandKind {
    FetchAll,
    /// Exactly these refspecs from one remote.
    FetchRefspecs {
        remote: String,
        refspecs: Vec<String>,
    },
    PruneMergedBranches,
    PruneLocalTags,
    /// `git maintenance run --auto`, started by the user.
    RunMaintenance,
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
    PushWithTags {
        request: gitcomet_core::tag_push::TagPushRequest,
    },
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
        upstream: Upstream,
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
    RewordCommit {
        request_id: crate::model::RewordRequestId,
        commit_id: CommitId,
        message: String,
        /// Whether the failure left the reword's rebase paused, as a signing
        /// failure at its reword step does: a retry then continues that
        /// rebase, which kept the new message. Otherwise it replays whole.
        /// Read from the repository after the failure, not guessed from
        /// which path the target should take.
        rebase_paused: bool,
        /// The reworded commit's new id once the command succeeded, so the
        /// selection can follow it.
        rewritten_id: Option<CommitId>,
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
        /// False merges every pick into the index without committing.
        commit: bool,
    },
    CherryPick {
        commit_id: CommitId,
        commit: bool,
        /// Git's 1-based mainline parent for a single merge commit.
        mainline: Option<usize>,
        summary: String,
    },
    Revert {
        commit_id: CommitId,
        commit: bool,
        /// Git's 1-based mainline parent for a merge commit.
        mainline: Option<usize>,
        summary: String,
    },
    ApplyFileChange {
        target: gitcomet_core::domain::ApplyChangeTarget,
        commit: bool,
        commit_retry: Option<gitcomet_core::domain::ApplyFileChangeRetry>,
    },
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
    AppendGitattributesRule {
        rule: String,
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
        checkout: WorktreeCheckout,
        open_after: bool,
    },
    RemoveWorktree {
        path: PathBuf,
        delete_branch: Option<String>,
    },
    ForceRemoveWorktree {
        path: PathBuf,
        delete_branch: Option<String>,
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
    /// A Git LFS or git-annex operation.
    LargeFile {
        command: gitcomet_core::large_files::LargeFileCommand,
    },
}

impl RepoCommandKind {
    pub fn repository_requirement(&self) -> gitcomet_core::domain::RepositoryRequirement {
        use gitcomet_core::domain::RepositoryRequirement::{Repository, WorkingTree};
        match self {
            Self::FetchAll
            | Self::FetchRefspecs { .. }
            | Self::PruneMergedBranches
            | Self::PruneLocalTags
            | Self::RunMaintenance
            | Self::Push
            | Self::PushWithTags { .. }
            | Self::PushAfterCommit { .. }
            | Self::ForcePush
            | Self::ForcePushWithLease { .. }
            | Self::PushSetUpstream { .. }
            | Self::SetUpstreamBranch { .. }
            | Self::UnsetUpstreamBranch { .. }
            | Self::DeleteRemoteBranch { .. }
            | Self::DeleteRemoteBranches { .. }
            | Self::SquashCommits { .. }
            | Self::CreateTag { .. }
            | Self::DeleteTag { .. }
            | Self::PushTag { .. }
            | Self::DeleteRemoteTag { .. }
            | Self::AddRemote { .. }
            | Self::RemoveRemote { .. }
            | Self::SetRemoteUrl { .. }
            | Self::ExportPatch { .. }
            | Self::AddWorktree { .. }
            | Self::RemoveWorktree { .. }
            | Self::ForceRemoveWorktree { .. } => Repository,
            Self::Reset { mode, .. } => match mode {
                ResetMode::Soft => Repository,
                ResetMode::Mixed | ResetMode::Hard => WorkingTree,
            },
            Self::LargeFile { command } => command.repository_requirement(),
            Self::Pull { .. }
            | Self::PullBranch { .. }
            | Self::MergeRef { .. }
            | Self::SquashRef { .. }
            | Self::Rebase { .. }
            | Self::RebaseContinue
            | Self::RebaseAbort
            | Self::InteractiveRebase { .. }
            | Self::RewordCommit { .. }
            | Self::InteractiveCherryPick { .. }
            | Self::CherryPick { .. }
            | Self::Revert { .. }
            | Self::ApplyFileChange { .. }
            | Self::MergeAbort
            | Self::CheckoutConflict { .. }
            | Self::AcceptConflictDeletion { .. }
            | Self::CheckoutConflictBase { .. }
            | Self::LaunchMergetool { .. }
            | Self::SaveWorktreeFile { .. }
            | Self::AppendGitignorePatterns { .. }
            | Self::AppendGitattributesRule { .. }
            | Self::ApplyPatch { .. }
            | Self::AddSubmodule { .. }
            | Self::UpdateSubmodules { .. }
            | Self::LoadSubmodule { .. }
            | Self::ChangeSubmodulePointer { .. }
            | Self::RemoveSubmodule { .. }
            | Self::StageHunk
            | Self::UnstageHunk
            | Self::ApplyWorktreePatch { .. } => WorkingTree,
        }
    }

    pub(crate) fn finished(
        self,
        repo_id: crate::model::RepoId,
        result: gitcomet_core::services::Result<gitcomet_core::services::CommandOutput>,
    ) -> super::InternalMsg {
        super::InternalMsg::RepoCommandFinished {
            repo_id,
            command: self,
            result,
        }
    }

    /// Whether the command can rewrite files in the checkout, so a file that
    /// changed under the view when it finished was GitComet's doing. Our own
    /// editor save is not here: the view recognizes those bytes itself.
    pub fn writes_worktree(&self) -> bool {
        match self {
            Self::Pull { .. }
            | Self::PullBranch { .. }
            | Self::MergeRef { .. }
            | Self::SquashRef { .. }
            | Self::Reset { .. }
            | Self::SquashCommits { .. }
            | Self::RewordCommit { .. }
            | Self::Rebase { .. }
            | Self::RebaseContinue
            | Self::RebaseAbort
            | Self::InteractiveRebase { .. }
            | Self::InteractiveCherryPick { .. }
            | Self::CherryPick { .. }
            | Self::Revert { .. }
            | Self::ApplyFileChange { .. }
            | Self::MergeAbort
            | Self::CheckoutConflict { .. }
            | Self::AcceptConflictDeletion { .. }
            | Self::CheckoutConflictBase { .. }
            | Self::LaunchMergetool { .. }
            | Self::AppendGitignorePatterns { .. }
            | Self::AppendGitattributesRule { .. }
            | Self::ExportPatch { .. }
            | Self::ApplyPatch { .. }
            | Self::AddSubmodule { .. }
            | Self::UpdateSubmodules { .. }
            | Self::LoadSubmodule { .. }
            | Self::ChangeSubmodulePointer { .. }
            | Self::RemoveSubmodule { .. }
            | Self::ApplyWorktreePatch { .. } => true,
            Self::FetchAll
            | Self::FetchRefspecs { .. }
            | Self::PruneMergedBranches
            | Self::PruneLocalTags
            | Self::RunMaintenance
            | Self::Push
            | Self::PushWithTags { .. }
            | Self::PushAfterCommit { .. }
            | Self::ForcePush
            | Self::ForcePushWithLease { .. }
            | Self::PushSetUpstream { .. }
            | Self::SetUpstreamBranch { .. }
            | Self::UnsetUpstreamBranch { .. }
            | Self::DeleteRemoteBranch { .. }
            | Self::DeleteRemoteBranches { .. }
            | Self::CreateTag { .. }
            | Self::DeleteTag { .. }
            | Self::PushTag { .. }
            | Self::DeleteRemoteTag { .. }
            | Self::AddRemote { .. }
            | Self::RemoveRemote { .. }
            | Self::SetRemoteUrl { .. }
            | Self::SaveWorktreeFile { .. }
            | Self::AddWorktree { .. }
            | Self::RemoveWorktree { .. }
            | Self::ForceRemoveWorktree { .. }
            | Self::StageHunk
            | Self::UnstageHunk => false,
            Self::LargeFile { command } => command.writes_worktree(),
        }
    }

    /// The worktree this command opens as a tab once it succeeds.
    pub(crate) fn opens_worktree(&self) -> Option<&PathBuf> {
        match self {
            Self::AddWorktree {
                path,
                open_after: true,
                ..
            } => Some(path),
            _ => None,
        }
    }

    pub(crate) fn opens_worktree_mut(&mut self) -> Option<&mut PathBuf> {
        match self {
            Self::AddWorktree {
                path,
                open_after: true,
                ..
            } => Some(path),
            _ => None,
        }
    }

    /// Whether the command holds off prepared patch applies while it runs.
    /// Only commands that never write the index, the worktree, or what a
    /// prepared patch checks (attributes, `core.*`/`filter.*` config), and can
    /// wait minutes on the network or a credential prompt, let them through.
    pub(crate) fn holds_mutation_gate(&self) -> bool {
        match self {
            Self::FetchAll
            | Self::FetchRefspecs { .. }
            | Self::PruneMergedBranches
            | Self::Push
            | Self::PushWithTags { .. }
            | Self::PushAfterCommit { .. }
            | Self::ForcePush
            | Self::ForcePushWithLease { .. }
            | Self::PushSetUpstream { .. }
            | Self::DeleteRemoteBranch { .. }
            | Self::DeleteRemoteBranches { .. }
            | Self::PushTag { .. }
            | Self::DeleteRemoteTag { .. } => false,
            Self::Pull { .. }
            | Self::PullBranch { .. }
            | Self::MergeRef { .. }
            | Self::SquashRef { .. }
            | Self::Reset { .. }
            | Self::SquashCommits { .. }
            | Self::RewordCommit { .. }
            | Self::Rebase { .. }
            | Self::RebaseContinue
            | Self::RebaseAbort
            | Self::InteractiveRebase { .. }
            | Self::InteractiveCherryPick { .. }
            | Self::CherryPick { .. }
            | Self::Revert { .. }
            | Self::ApplyFileChange { .. }
            | Self::MergeAbort
            | Self::CheckoutConflict { .. }
            | Self::AcceptConflictDeletion { .. }
            | Self::CheckoutConflictBase { .. }
            | Self::LaunchMergetool { .. }
            | Self::AppendGitignorePatterns { .. }
            | Self::AppendGitattributesRule { .. }
            | Self::ExportPatch { .. }
            | Self::ApplyPatch { .. }
            | Self::AddSubmodule { .. }
            | Self::UpdateSubmodules { .. }
            | Self::LoadSubmodule { .. }
            | Self::ChangeSubmodulePointer { .. }
            | Self::RemoveSubmodule { .. }
            | Self::ApplyWorktreePatch { .. }
            | Self::PruneLocalTags
            | Self::RunMaintenance
            | Self::SetUpstreamBranch { .. }
            | Self::UnsetUpstreamBranch { .. }
            | Self::CreateTag { .. }
            | Self::DeleteTag { .. }
            | Self::AddRemote { .. }
            | Self::RemoveRemote { .. }
            | Self::SetRemoteUrl { .. }
            | Self::SaveWorktreeFile { .. }
            | Self::AddWorktree { .. }
            | Self::RemoveWorktree { .. }
            | Self::ForceRemoveWorktree { .. }
            | Self::StageHunk
            | Self::UnstageHunk
            | Self::LargeFile { .. } => true,
        }
    }

    /// Commands that fetch, and so may leave new packs or run a repack.
    pub(crate) fn fetches_objects(&self) -> bool {
        matches!(
            self,
            Self::FetchAll
                | Self::FetchRefspecs { .. }
                | Self::PruneMergedBranches
                | Self::Pull { .. }
                | Self::PullBranch { .. }
        )
    }

    /// Commands long enough to deserve a progress card with git's meters.
    pub(crate) fn shows_progress(&self) -> bool {
        self.fetches_objects() || matches!(self, Self::RunMaintenance)
    }

    pub(crate) fn hook_activity_label(&self) -> &'static str {
        match self {
            Self::FetchAll | Self::FetchRefspecs { .. } => "Fetch",
            Self::PruneMergedBranches => "Prune branches",
            Self::PruneLocalTags => "Prune tags",
            Self::RunMaintenance => "Maintenance",
            Self::Pull { .. } | Self::PullBranch { .. } => "Pull",
            Self::MergeRef { .. } => "Merge",
            Self::SquashRef { .. } | Self::SquashCommits { .. } => "Squash",
            Self::RewordCommit { .. } => "Edit commit message",
            Self::Push | Self::PushAfterCommit { .. } => "Push",
            Self::PushWithTags { request } => request.mode.label(),
            Self::ForcePush | Self::ForcePushWithLease { .. } => "Force push",
            Self::PushSetUpstream { .. } => "Push and set upstream",
            Self::SetUpstreamBranch { .. } => "Set upstream",
            Self::UnsetUpstreamBranch { .. } => "Unset upstream",
            Self::DeleteRemoteBranch { .. } | Self::DeleteRemoteBranches { .. } => {
                "Delete remote branch"
            }
            Self::Reset { .. } => "Reset",
            Self::Rebase { .. }
            | Self::RebaseContinue
            | Self::RebaseAbort
            | Self::InteractiveRebase { .. } => "Rebase",
            Self::InteractiveCherryPick { .. } | Self::CherryPick { .. } => "Cherry-pick",
            Self::Revert { .. } => "Revert",
            Self::ApplyFileChange { .. } => "Apply change",
            Self::MergeAbort => "Abort merge",
            Self::CreateTag { .. } => "Create tag",
            Self::DeleteTag { .. } => "Delete tag",
            Self::PushTag { .. } => "Push tag",
            Self::DeleteRemoteTag { .. } => "Delete remote tag",
            Self::AddRemote { .. } => "Add remote",
            Self::RemoveRemote { .. } => "Remove remote",
            Self::SetRemoteUrl { .. } => "Set remote URL",
            Self::CheckoutConflict { .. }
            | Self::CheckoutConflictBase { .. }
            | Self::AcceptConflictDeletion { .. } => "Resolve conflict",
            Self::LaunchMergetool { .. } => "Mergetool",
            Self::SaveWorktreeFile { .. } => "Save file",
            Self::AppendGitignorePatterns { .. } => "Update .gitignore",
            Self::AppendGitattributesRule { .. } => "Update .gitattributes",
            Self::ExportPatch { .. } => "Export patch",
            Self::ApplyPatch { .. } => "Apply patch",
            Self::AddWorktree { .. } => "Add worktree",
            Self::RemoveWorktree { .. } | Self::ForceRemoveWorktree { .. } => "Remove worktree",
            Self::AddSubmodule { .. } => "Add submodule",
            Self::UpdateSubmodules { .. } => "Update submodules",
            Self::LoadSubmodule { .. } => "Load submodule",
            Self::ChangeSubmodulePointer { .. } => "Change submodule",
            Self::RemoveSubmodule { .. } => "Remove submodule",
            Self::StageHunk => "Stage hunk",
            Self::UnstageHunk => "Unstage hunk",
            Self::ApplyWorktreePatch { .. } => "Apply worktree patch",
            Self::LargeFile { command } => command.label(),
        }
    }
}
