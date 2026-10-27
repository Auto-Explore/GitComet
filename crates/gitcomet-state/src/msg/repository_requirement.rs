use super::Msg;
use crate::model::RepoId;
use gitcomet_core::domain::{BlameSource, DiffTarget, FileSource, RepositoryRequirement};
use gitcomet_core::services::ResetMode;

impl Msg {
    pub fn repository_requirement(&self) -> Option<(RepoId, RepositoryRequirement)> {
        use RepositoryRequirement::{Repository, WorkingTree};
        match self {
            Self::AcceptConflictDeletion { repo_id, .. }
            | Self::AddSubmodule { repo_id, .. }
            | Self::AddSubmoduleTrusted { repo_id, .. }
            | Self::AppendGitattributesRule { repo_id, .. }
            | Self::AppendGitignorePatterns { repo_id, .. }
            | Self::ApplyFileChange { repo_id, .. }
            | Self::ApplyPatch { repo_id, .. }
            | Self::ApplyPreparedDiffPatch { repo_id, .. }
            | Self::ApplyStash { repo_id, .. }
            | Self::ApplyWorktreePatch { repo_id, .. }
            | Self::ChangeSubmodulePointer { repo_id, .. }
            | Self::CheckoutBranch { repo_id, .. }
            | Self::CheckoutCommit { repo_id, .. }
            | Self::CheckoutConflictBase { repo_id, .. }
            | Self::CheckoutConflictSide { repo_id, .. }
            | Self::CheckoutRemoteBranch { repo_id, .. }
            | Self::CherryPickCommit { repo_id, .. }
            | Self::Commit { repo_id, .. }
            | Self::CommitAmend { repo_id, .. }
            | Self::CompareWithWorkingTree { repo_id, .. }
            | Self::ConflictAddManualAlignment { repo_id, .. }
            | Self::ConflictApplyAutosolve { repo_id, .. }
            | Self::ConflictApplyBulkChoice { repo_id, .. }
            | Self::ConflictClearManualAlignments { repo_id, .. }
            | Self::ConflictJoinRegions { repo_id, .. }
            | Self::ConflictReplacePlanBlockSelection { repo_id, .. }
            | Self::ConflictReplaceRegionSelection { repo_id, .. }
            | Self::ConflictResetResolutions { repo_id, .. }
            | Self::ConflictSetRegionChoice { repo_id, .. }
            | Self::ConflictSplitRegion { repo_id, .. }
            | Self::ConflictSyncRegionResolutions { repo_id, .. }
            | Self::ConflictTogglePlanBlockSource { repo_id, .. }
            | Self::ConflictToggleRegionSource { repo_id, .. }
            | Self::CreateBranchAndCheckout { repo_id, .. }
            | Self::DiscardWorktreeChangesPath { repo_id, .. }
            | Self::DiscardWorktreeChangesPaths { repo_id, .. }
            | Self::DropStash { repo_id, .. }
            | Self::InteractiveCherryPick { repo_id, .. }
            | Self::InteractiveRebase { repo_id, .. }
            | Self::LaunchMergetool { repo_id, .. }
            | Self::LoadConflictFile { repo_id, .. }
            | Self::LoadInteractiveRebaseSetup { repo_id, .. }
            | Self::LoadSubmodule { repo_id, .. }
            | Self::LoadSubmoduleTrusted { repo_id, .. }
            | Self::MergeAbort { repo_id, .. }
            | Self::MergeRef { repo_id, .. }
            | Self::OpenFileEditor { repo_id, .. }
            | Self::OpenInteractiveCherryPickSetup { repo_id, .. }
            | Self::PopStash { repo_id, .. }
            | Self::Pull { repo_id, .. }
            | Self::PullBranch { repo_id, .. }
            | Self::Rebase { repo_id, .. }
            | Self::RebaseAbort { repo_id, .. }
            | Self::RebaseContinue { repo_id, .. }
            | Self::RemoveSubmodule { repo_id, .. }
            | Self::RevertCommit { repo_id, .. }
            | Self::RewordCommit { repo_id, .. }
            | Self::ContinueRewordCommit { repo_id, .. }
            | Self::SaveWorktreeFile { repo_id, .. }
            | Self::SelectConflictDiff { repo_id, .. }
            | Self::SquashRef { repo_id, .. }
            | Self::StageHunk { repo_id, .. }
            | Self::StagePath { repo_id, .. }
            | Self::StagePaths { repo_id, .. }
            | Self::Stash { repo_id, .. }
            | Self::UnstageHunk { repo_id, .. }
            | Self::UnstagePath { repo_id, .. }
            | Self::UnstagePaths { repo_id, .. }
            | Self::UpdateSubmodules { repo_id, .. }
            | Self::UpdateSubmodulesTrusted { repo_id, .. } => Some((*repo_id, WorkingTree)),
            Self::RememberDocumentInRepository { repo_id, .. }
            | Self::SelectExplorerPath { repo_id, .. }
            | Self::FocusExplorerPath { repo_id, .. }
            | Self::SelectAllExplorerPaths { repo_id, .. }
            | Self::SetExplorerVisibility { repo_id, .. }
            | Self::AcknowledgeRepoFeedback { repo_id, .. }
            | Self::CloseRepo { repo_id, .. }
            | Self::MoveRepoOut { repo_id, .. }
            | Self::DismissRepoError { repo_id, .. }
            | Self::CancelGitOperation { repo_id, .. }
            | Self::DismissRewordCommit { repo_id, .. }
            | Self::SetCommitSignatureTargets { repo_id, .. }
            | Self::SetActiveRepo { repo_id, .. }
            | Self::ReorderRepoTabs { repo_id, .. }
            | Self::ReloadRepo { repo_id, .. }
            | Self::PreviewDisableBareMode { repo_id, .. }
            | Self::DisableBareMode { repo_id, .. }
            | Self::ClearBareConversionPreview { repo_id, .. }
            | Self::RepoActivated { repo_id, .. }
            | Self::AcquireWatchLease { repo_id, .. }
            | Self::ReleaseWatchLease { repo_id, .. }
            | Self::WatchWorktree { repo_id, .. }
            | Self::WorktreeExternallyChanged { repo_id, .. }
            | Self::RepoExternallyChanged { repo_id, .. }
            | Self::RepoWatchDegraded { repo_id, .. }
            | Self::UpdateRepositoryPreference { repo_id, .. }
            | Self::SetHistoryScope { repo_id, .. }
            | Self::SetHistoryAuthorFilter { repo_id, .. }
            | Self::SetHistoryFocus { repo_id, .. }
            | Self::ToggleHistoryFocus { repo_id, .. }
            | Self::FocusHistoryRefs { repo_id, .. }
            | Self::LoadMoreHistory { repo_id, .. }
            | Self::SelectCommit { repo_id, .. }
            | Self::SelectCommitMulti { repo_id, .. }
            | Self::ClearCommitSelection { repo_id, .. }
            | Self::CompareCommitRange { repo_id, .. }
            | Self::ClearComparison { repo_id, .. }
            | Self::MarkForComparison { repo_id, .. }
            | Self::CompareWithMarked { repo_id, .. }
            | Self::ClearComparisonMark { repo_id, .. }
            | Self::SetTextOverride { repo_id, .. }
            | Self::OpenInlineSubmoduleDiff { repo_id, .. }
            | Self::SelectInlineSubmoduleDiff { repo_id, .. }
            | Self::CloseInlineSubmoduleDiff { repo_id, .. }
            | Self::ClearDiffSelection { repo_id, .. }
            | Self::EnsureSidebarData { repo_id, .. }
            | Self::LoadStashes { repo_id, .. }
            | Self::LoadReflog { repo_id, .. }
            | Self::LoadRecentCommitMessages { repo_id, .. }
            | Self::LoadHoverCommitMessage { repo_id, .. }
            | Self::LoadFileHistory { repo_id, .. }
            | Self::LoadWorktrees { repo_id, .. }
            | Self::LoadWorktreeDirty { repo_id, .. }
            | Self::SelectWorktreeUncommitted { repo_id, .. }
            | Self::LoadRefMetadata { repo_id, .. }
            | Self::LoadSubmodules { repo_id, .. }
            | Self::LoadTags { repo_id, .. }
            | Self::LoadRemoteTags { repo_id, .. }
            | Self::RefreshBranches { repo_id, .. }
            | Self::LoadFileBrowser { repo_id, .. }
            | Self::ToggleFileBrowserDir { repo_id, .. }
            | Self::SetFileBrowserDirExpandedRecursive { repo_id, .. }
            | Self::SetFileBrowserSearch { repo_id, .. }
            | Self::RevealFileBrowserPath { repo_id, .. }
            | Self::SetFileBrowserSource { repo_id, .. }
            | Self::ExitDiffEditMode { repo_id, .. }
            | Self::OpenFileAtCommitParent { repo_id, .. }
            | Self::OpenFileAtCommit { repo_id, .. }
            | Self::ShowFileChangesAtCommit { repo_id, .. }
            | Self::BrowseRepositoryAtCommit { repo_id, .. }
            | Self::RevealCommit { repo_id, .. }
            | Self::FinishCommitReveal { repo_id, .. }
            | Self::ResolveCommitLookup { repo_id, .. }
            | Self::ResetBrowseToLive { repo_id, .. }
            | Self::ViewerNavBack { repo_id, .. }
            | Self::ViewerNavForward { repo_id, .. }
            | Self::GlobalNavBack { repo_id, .. }
            | Self::GlobalNavForward { repo_id, .. }
            | Self::CreateBranch { repo_id, .. }
            | Self::RenameBranch { repo_id, .. }
            | Self::DeleteBranch { repo_id, .. }
            | Self::ForceDeleteBranch { repo_id, .. }
            | Self::DeleteBranches { repo_id, .. }
            | Self::ExportPatch { repo_id, .. }
            | Self::AddWorktree { repo_id, .. }
            | Self::RemoveWorktree { repo_id, .. }
            | Self::ForceRemoveWorktree { repo_id, .. }
            | Self::LoadLfsLocks { repo_id, .. }
            | Self::LoadAnnexWhereis { repo_id, .. }
            | Self::LoadAnnexUnused { repo_id, .. }
            | Self::SafePushAfterCommit { repo_id, .. }
            | Self::PruneMergedBranches { repo_id, .. }
            | Self::PruneLocalTags { repo_id, .. }
            | Self::StartRepoMaintenance { repo_id, .. }
            | Self::SnoozeRepoMaintenance { repo_id, .. }
            | Self::PushWithTags { repo_id, .. }
            | Self::PreviewTagPush { repo_id, .. }
            | Self::Push { repo_id, .. }
            | Self::PushAfterCommit { repo_id, .. }
            | Self::ForcePush { repo_id, .. }
            | Self::ForcePushWithLease { repo_id, .. }
            | Self::PushSetUpstream { repo_id, .. }
            | Self::SetUpstreamBranch { repo_id, .. }
            | Self::UnsetUpstreamBranch { repo_id, .. }
            | Self::DeleteRemoteBranch { repo_id, .. }
            | Self::DeleteRemoteBranches { repo_id, .. }
            | Self::PrepareSquash { repo_id, .. }
            | Self::SquashCommits { repo_id, .. }
            | Self::CancelInteractiveRebaseSetup { repo_id, .. }
            | Self::CancelInteractiveCherryPickSetup { repo_id, .. }
            | Self::CreateTag { repo_id, .. }
            | Self::DeleteTag { repo_id, .. }
            | Self::PushTag { repo_id, .. }
            | Self::DeleteRemoteTag { repo_id, .. }
            | Self::AddRemote { repo_id, .. }
            | Self::RemoveRemote { repo_id, .. }
            | Self::SetRemoteUrl { repo_id, .. }
            | Self::RecordConflictAutosolveTelemetry { repo_id, .. }
            | Self::ConflictSetHideResolved { repo_id, .. } => Some((*repo_id, Repository)),
            Self::CompareWithOptions { repo_id, to, .. } => Some((
                *repo_id,
                if to.is_some() {
                    Repository
                } else {
                    WorkingTree
                },
            )),
            Self::SelectDiff { repo_id, target } => Some((
                *repo_id,
                match target {
                    DiffTarget::WorkingTree { worktree: None, .. }
                    | DiffTarget::CommitRange {
                        to_commit_id: None, ..
                    } => WorkingTree,
                    DiffTarget::WorkingTree {
                        worktree: Some(_), ..
                    }
                    | DiffTarget::Commit { .. }
                    | DiffTarget::CommitRange {
                        to_commit_id: Some(_),
                        ..
                    } => Repository,
                },
            )),
            Self::LoadBlame {
                repo_id, source, ..
            } => Some((
                *repo_id,
                match source {
                    BlameSource::WorkingTree(_) => WorkingTree,
                    BlameSource::Revision(_) => Repository,
                },
            )),
            Self::OpenFileContent {
                repo_id, source, ..
            } => Some((
                *repo_id,
                match source {
                    FileSource::WorkingDirectory => WorkingTree,
                    FileSource::Commit(_) | FileSource::Branch(_) => Repository,
                },
            )),
            Self::Reset { repo_id, mode, .. } => Some((
                *repo_id,
                match mode {
                    ResetMode::Soft => Repository,
                    ResetMode::Mixed | ResetMode::Hard => WorkingTree,
                },
            )),
            Self::RunLargeFileCommand { repo_id, command } => {
                Some((*repo_id, command.repository_requirement()))
            }
            Self::ResolveBranchExistsPrompt { prompt, choice } => Some((
                prompt.repo_id,
                match choice {
                    super::BranchExistsChoice::Cancel => Repository,
                    super::BranchExistsChoice::CheckoutExisting => WorkingTree,
                    super::BranchExistsChoice::OverwriteAndCheckout => match prompt.operation {
                        crate::model::BranchExistsPromptOperation::RenameBranch { .. } => {
                            Repository
                        }
                        crate::model::BranchExistsPromptOperation::CreateBranch
                        | crate::model::BranchExistsPromptOperation::CheckoutRemoteBranch {
                            ..
                        } => WorkingTree,
                    },
                },
            )),
            Self::OpenDocumentRepository { .. }
            | Self::FilesystemRequest(..)
            | Self::FilesystemProgress(..)
            | Self::AcknowledgeFilesystemResults(..)
            | Self::FilesystemJournalUpdated { .. }
            | Self::FilesystemFinished(..)
            | Self::FilesystemPathsChanged(..)
            | Self::IndexedHistory(..)
            | Self::DiffSession(..)
            | Self::HistoryAuthors(..)
            | Self::HistoryFind(..)
            | Self::OpenRepo(..)
            | Self::OpenRepoFromExternalDrop(..)
            | Self::AcknowledgeRepoOpenFailures { .. }
            | Self::AcknowledgeNotifications { .. }
            | Self::RestoreSession { .. }
            | Self::CloseRepos { .. }
            | Self::ReportError { .. }
            | Self::SubmitAuthPrompt { .. }
            | Self::CancelAuthPrompt
            | Self::SetGitRuntimeState(..)
            | Self::SetSigningToolsState(..)
            | Self::SetLargeFileToolsState(..)
            | Self::SetRemoteUrlPolicy(..)
            | Self::SetGitLogSettings { .. }
            | Self::SetRemoteSettings(..)
            | Self::SetMaintenanceSettings(..)
            | Self::SetFileBrowserSettings(..)
            | Self::SetLargeFileSettings(..)
            | Self::SetDefaultTagType(..)
            | Self::ApplyRepositoryPreferences(..)
            | Self::SetSidebarMode { .. }
            | Self::ShowBranchExistsPrompt { .. }
            | Self::DismissForceDeleteBranchPrompt { .. }
            | Self::DismissForceRemoveWorktreePrompt { .. }
            | Self::CloneRepo { .. }
            | Self::AbortCloneRepo { .. }
            | Self::ConfirmSubmoduleTrustPrompt
            | Self::CancelSubmoduleTrustPrompt
            | Self::Fetch(..)
            | Self::Internal(..) => None,
        }
    }
}
