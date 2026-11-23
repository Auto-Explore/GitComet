//! One classification per message: how the store queues it, what it prepares
//! before reducing it, and how the reducer admits it.

use super::{FetchMsg, InternalMsg, Msg};
use crate::indexed_history::IndexedHistoryMsg;
use crate::model::RepoId;
use crate::model::ViewNavDir::{self, Back, Forward};
use NavHistory::{Main, Viewer};
use ViewNavigation::{Replay, Request, Resolved, Step};
use gitcomet_core::domain::RepositoryRequirement;

/// How the store treats a message.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct MsgClass {
    /// Dropped while Git is unavailable.
    pub(crate) requires_git: bool,
    /// Its repository, and what that repository must provide.
    pub(crate) repository: Option<(RepoId, RepositoryRequirement)>,
    /// Which repository's main view it moves, and how; anything else is a
    /// background change folded into the current step.
    pub(crate) navigation: Option<(RepoId, ViewNavigation)>,
    /// The worker takes it ahead of queued completions.
    pub(crate) control: bool,
    /// Only releases entries the window has shown. The worker applies it in
    /// the next reducer pass instead of copying and publishing the whole
    /// state for it alone; the window already finds new entries by sequence,
    /// so entries it has shown may linger until then.
    pub(crate) feedback_acknowledgement: bool,
    /// Sent by the store's own work, so control messages may overtake it.
    pub(crate) completion: bool,
    /// What is resolved before its reduction.
    pub(crate) facts: PreparedFacts,
}

impl MsgClass {
    const fn control(self) -> Self {
        Self {
            control: true,
            ..self
        }
    }

    const fn acknowledgement(self) -> Self {
        Self {
            feedback_acknowledgement: true,
            ..self
        }
    }

    const fn navigates(self, repo_id: RepoId, navigation: ViewNavigation) -> Self {
        Self {
            navigation: Some((repo_id, navigation)),
            ..self
        }
    }

    const fn prepares(self, facts: PreparedFacts) -> Self {
        Self { facts, ..self }
    }
}

/// A request handled whether or not Git is available.
const PLAIN: MsgClass = MsgClass {
    requires_git: false,
    repository: None,
    navigation: None,
    control: false,
    feedback_acknowledgement: false,
    completion: false,
    facts: PreparedFacts {
        paths: false,
        queries: false,
    },
};
/// A request that runs Git.
const GIT: MsgClass = MsgClass {
    requires_git: true,
    ..PLAIN
};
const COMPLETION: MsgClass = MsgClass {
    completion: true,
    ..PLAIN
};

/// What `PreparedReduction` resolves for a message before the model write
/// lock. Its capture matches run only for a message that declares the fact.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PreparedFacts {
    /// Paths the message carries are resolved against the disk.
    pub(crate) paths: bool,
    /// Saved preferences, or the Git and disk state of a selected file, are
    /// read.
    pub(crate) queries: bool,
}

impl PreparedFacts {
    pub(crate) const fn any(self) -> bool {
        self.paths || self.queries
    }

    const fn with_queries(self) -> Self {
        Self {
            queries: true,
            ..self
        }
    }
}

const PATHS: PreparedFacts = PreparedFacts {
    paths: true,
    queries: false,
};
const QUERIES: PreparedFacts = PreparedFacts {
    paths: false,
    queries: true,
};
const PATHS_AND_QUERIES: PreparedFacts = PreparedFacts {
    paths: true,
    queries: true,
};

/// A back/forward history the user steps through.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NavHistory {
    /// The global history across the main view.
    Main,
    /// The file viewer's own history of versions, kept apart so it does not
    /// pollute the global one.
    Viewer,
}

/// How a message moves its repository's main view.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewNavigation {
    /// The user goes to a view: a new global back/forward step, and a request
    /// for the repository's canvas.
    Step,
    /// The user asks for the canvas, and the selection folds into the current
    /// step.
    Request,
    /// The user's Back or Forward through a history.
    Replay(NavHistory, ViewNavDir),
    /// A view asked for earlier arrives: its step, but the ask already
    /// counted.
    Resolved,
}

impl Msg {
    pub(crate) fn class(&self) -> MsgClass {
        MsgClass {
            repository: self.repository_requirement(),
            ..msg_class(self)
        }
    }
}

impl InternalMsg {
    pub(crate) fn class(&self) -> MsgClass {
        internal_class(self)
    }
}

fn indexed_history(message: &IndexedHistoryMsg) -> MsgClass {
    match message {
        IndexedHistoryMsg::Select { repo_id, .. } => PLAIN.navigates(*repo_id, Request),
        IndexedHistoryMsg::Retry { .. }
        | IndexedHistoryMsg::Ensure { .. }
        | IndexedHistoryMsg::Publish { .. }
        | IndexedHistoryMsg::RequestRanges { .. }
        | IndexedHistoryMsg::Progress { .. }
        | IndexedHistoryMsg::Built { .. }
        | IndexedHistoryMsg::RangeLoaded { .. } => PLAIN,
    }
}

fn fetch(message: &FetchMsg) -> MsgClass {
    match message {
        FetchMsg::All { .. } | FetchMsg::Remote { .. } | FetchMsg::Refspecs { .. } => GIT,
    }
}

/// One row per variant, in declaration order, and no wildcard arm: a
/// variant without a row does not compile.
fn msg_class(message: &Msg) -> MsgClass {
    match message {
        Msg::OpenDocumentRepository { .. } => PLAIN.prepares(PATHS_AND_QUERIES),
        Msg::RememberDocumentInRepository { .. } => PLAIN,
        Msg::FilesystemRequest(..) => PLAIN,
        Msg::FilesystemProgress(..) => PLAIN,
        Msg::AcknowledgeFilesystemResults(..) => PLAIN,
        Msg::FilesystemJournalUpdated { .. } => PLAIN,
        Msg::FilesystemFinished(..) => PLAIN.prepares(QUERIES),
        Msg::FilesystemPathsChanged(..) => PLAIN.prepares(QUERIES),
        Msg::SelectExplorerPath { .. } => PLAIN,
        Msg::FocusExplorerPath { .. } => PLAIN,
        Msg::SelectAllExplorerPaths { .. } => PLAIN,
        Msg::SetExplorerVisibility { .. } => PLAIN,
        Msg::IndexedHistory(message) => indexed_history(message),
        Msg::DiffSession(..) => PLAIN,
        Msg::HistoryAuthors(..) => PLAIN,
        Msg::HistoryFind(..) => PLAIN,
        Msg::OpenRepo(..) => GIT.control().prepares(PATHS_AND_QUERIES),
        Msg::OpenRepoFromExternalDrop(..) => GIT.control().prepares(PATHS_AND_QUERIES),
        Msg::AcknowledgeRepoOpenFailures { .. } => PLAIN,
        Msg::AcknowledgeNotifications { .. } => PLAIN.acknowledgement(),
        Msg::AcknowledgeRepoFeedback { .. } => PLAIN.acknowledgement(),
        Msg::RestoreSession { .. } => GIT.prepares(PATHS_AND_QUERIES),
        Msg::CloseRepo { .. } => PLAIN.control().prepares(QUERIES),
        Msg::MoveRepoOut { .. } => PLAIN.control().prepares(QUERIES),
        Msg::CloseRepos { .. } => PLAIN.control().prepares(QUERIES),
        Msg::ReportError { .. } => PLAIN,
        Msg::DismissRepoError { .. } => PLAIN,
        Msg::CancelGitOperation { .. } => PLAIN.control(),
        // The worktree removal it retries carries a path.
        Msg::SubmitAuthPrompt { .. } => PLAIN.prepares(PATHS),
        Msg::CancelAuthPrompt => PLAIN,
        Msg::SetGitRuntimeState(..) => PLAIN,
        Msg::SetSigningToolsState(..) => PLAIN,
        Msg::SetLargeFileToolsState(..) => PLAIN,
        Msg::SetCommitSignatureTargets { .. } => PLAIN,
        Msg::SetRemoteUrlPolicy(..) => PLAIN,
        Msg::SetGitLogSettings { .. } => PLAIN,
        Msg::SetRemoteSettings(..) => PLAIN,
        Msg::SetMaintenanceSettings(..) => PLAIN,
        Msg::SetFileBrowserSettings(..) => PLAIN,
        Msg::SetLargeFileSettings(..) => PLAIN,
        Msg::SetHistoryHiddenRefs(..) => PLAIN,
        Msg::SetDefaultTagType(..) => PLAIN,
        Msg::SetActiveRepo { .. } => PLAIN.control().prepares(QUERIES),
        Msg::ReorderRepoTabs { .. } => PLAIN.control(),
        Msg::ReloadRepo { .. } => GIT.prepares(QUERIES),
        Msg::PreviewDisableBareMode { .. } => GIT,
        Msg::DisableBareMode { .. } => GIT,
        Msg::ClearBareConversionPreview { .. } => PLAIN,
        Msg::RepoActivated { .. } => GIT,
        Msg::AcquireWatchLease { .. } => PLAIN,
        Msg::ReleaseWatchLease { .. } => PLAIN,
        Msg::WatchWorktree { .. } => PLAIN,
        Msg::WorktreeExternallyChanged { .. } => PLAIN,
        Msg::RepoExternallyChanged { .. } => GIT.prepares(QUERIES),
        Msg::RepoWatchDegraded { .. } => PLAIN,
        Msg::UpdateRepositoryPreference { .. } => PLAIN,
        Msg::ApplyRepositoryPreferences(..) => PLAIN,
        Msg::HistoryFilter(..) => GIT,
        Msg::LoadMoreHistory { .. } => GIT,
        Msg::SelectCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::SelectCommitMulti { repo_id, .. } => PLAIN.navigates(*repo_id, Request),
        Msg::ClearCommitSelection { .. } => PLAIN,
        Msg::CompareCommitRange { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::CompareWithWorkingTree { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::CompareWithOptions { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::ClearComparison { .. } => PLAIN,
        Msg::MarkForComparison { .. } => PLAIN,
        Msg::CompareWithMarked { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::ClearComparisonMark { .. } => PLAIN,
        Msg::SelectDiff { repo_id, .. } => GIT.navigates(*repo_id, Step).prepares(QUERIES),
        Msg::SetTextOverride { .. } => GIT.prepares(QUERIES),
        Msg::OpenInlineSubmoduleDiff { repo_id, .. } => PLAIN.navigates(*repo_id, Step),
        Msg::SelectInlineSubmoduleDiff { repo_id, .. } => PLAIN.navigates(*repo_id, Step),
        Msg::CloseInlineSubmoduleDiff { .. } => PLAIN,
        Msg::SelectConflictDiff { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::ClearDiffSelection { .. } => PLAIN,
        Msg::EnsureSidebarData { .. } => PLAIN,
        Msg::LoadStashes { .. } => GIT,
        Msg::LoadConflictFile { .. } => GIT,
        Msg::LoadReflog { .. } => GIT,
        Msg::LoadRecentCommitMessages { .. } => GIT,
        Msg::LoadHoverCommitMessage { .. } => GIT,
        Msg::LoadFileHistory { .. } => GIT,
        Msg::LoadBlame { .. } => GIT,
        Msg::LoadWorktrees { .. } => GIT,
        Msg::LoadWorktreeDirty { .. } => GIT,
        // Selecting a linked-worktree row is a destination like any other history
        // selection; it just is not a commit.
        Msg::SelectWorktreeUncommitted { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::LoadRefMetadata { .. } => GIT,
        Msg::LoadSubmodules { .. } => GIT,
        Msg::LoadTags { .. } => GIT,
        Msg::LoadRemoteTags { .. } => GIT,
        Msg::RefreshBranches { .. } => GIT,
        Msg::LoadFileBrowser { .. } => GIT,
        Msg::ToggleFileBrowserDir { .. } => PLAIN,
        Msg::SetFileBrowserDirExpandedRecursive { .. } => PLAIN,
        Msg::SetFileBrowserSearch { .. } => PLAIN,
        Msg::RevealFileBrowserPath { .. } => PLAIN,
        Msg::SetFileBrowserSource { .. } => PLAIN,
        Msg::OpenFileContent { repo_id, .. } => GIT.navigates(*repo_id, Step).prepares(QUERIES),
        Msg::OpenFileEditor { repo_id, .. } => GIT.navigates(*repo_id, Step).prepares(QUERIES),
        // Leaving the editor is a destination of its own, so Back returns to the
        // editor rather than skipping past it to whatever preceded it.
        Msg::ExitDiffEditMode { repo_id, .. } => PLAIN.navigates(*repo_id, Step).prepares(QUERIES),
        Msg::OpenFileAtCommitParent { .. } => GIT,
        Msg::OpenFileAtCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::ShowFileChangesAtCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::BrowseRepositoryAtCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::RevealCommit { repo_id, .. } => GIT.navigates(*repo_id, Request),
        Msg::FinishCommitReveal { .. } => PLAIN,
        Msg::ResolveCommitLookup { .. } => GIT,
        Msg::ResetBrowseToLive { repo_id, .. } => GIT.navigates(*repo_id, Step),
        Msg::ViewerNavBack { repo_id, .. } => GIT
            .navigates(*repo_id, Replay(Viewer, Back))
            .prepares(QUERIES),
        Msg::ViewerNavForward { repo_id, .. } => GIT
            .navigates(*repo_id, Replay(Viewer, Forward))
            .prepares(QUERIES),
        Msg::GlobalNavBack { repo_id, .. } => GIT
            .navigates(*repo_id, Replay(Main, Back))
            .prepares(QUERIES),
        Msg::GlobalNavForward { repo_id, .. } => GIT
            .navigates(*repo_id, Replay(Main, Forward))
            .prepares(QUERIES),
        Msg::SetSidebarMode { .. } => PLAIN,
        Msg::StageHunk { .. } => GIT,
        Msg::ApplyPreparedDiffPatch { .. } => GIT,
        Msg::UnstageHunk { .. } => GIT,
        Msg::ApplyWorktreePatch { .. } => GIT,
        Msg::CheckoutBranch { .. } => GIT,
        Msg::CheckoutRemoteBranch { .. } => GIT,
        Msg::CheckoutCommit { .. } => GIT,
        Msg::CherryPickCommit { .. } => GIT,
        Msg::RevertCommit { .. } => GIT,
        Msg::ApplyFileChange { .. } => GIT,
        Msg::CreateBranch { .. } => GIT,
        Msg::CreateBranchAndCheckout { .. } => GIT,
        Msg::ResolveBranchExistsPrompt { .. } => PLAIN,
        Msg::ShowBranchExistsPrompt { .. } => PLAIN,
        Msg::DismissForceDeleteBranchPrompt { .. } => PLAIN,
        Msg::RenameBranch { .. } => GIT,
        Msg::DeleteBranch { .. } => GIT,
        Msg::ForceDeleteBranch { .. } => GIT,
        Msg::DeleteBranches { .. } => GIT,
        Msg::CloneRepo { .. } => GIT,
        Msg::AbortCloneRepo { .. } => PLAIN,
        Msg::ExportPatch { .. } => GIT,
        Msg::ApplyPatch { .. } => GIT,
        Msg::AddWorktree { .. } => GIT,
        Msg::RemoveWorktree { .. } => GIT.prepares(PATHS),
        Msg::ForceRemoveWorktree { .. } => GIT.prepares(PATHS),
        Msg::DismissForceRemoveWorktreePrompt { .. } => PLAIN,
        Msg::AddSubmodule { .. } => GIT,
        Msg::AddSubmoduleTrusted { .. } => PLAIN,
        Msg::UpdateSubmodules { .. } => GIT,
        Msg::UpdateSubmodulesTrusted { .. } => PLAIN,
        Msg::LoadSubmodule { .. } => GIT,
        Msg::LoadSubmoduleTrusted { .. } => PLAIN,
        Msg::ConfirmSubmoduleTrustPrompt => PLAIN,
        Msg::CancelSubmoduleTrustPrompt => PLAIN,
        Msg::ChangeSubmodulePointer { .. } => GIT,
        Msg::RemoveSubmodule { .. } => GIT,
        Msg::StagePath { .. } => GIT,
        Msg::StagePaths { .. } => GIT,
        Msg::UnstagePath { .. } => GIT,
        Msg::UnstagePaths { .. } => GIT,
        Msg::DiscardWorktreeChangesPath { .. } => GIT,
        Msg::DiscardWorktreeChangesPaths { .. } => GIT,
        Msg::SaveWorktreeFile { .. } => GIT,
        Msg::AppendGitignorePatterns { .. } => GIT,
        Msg::RunLargeFileCommand { .. } => GIT,
        Msg::LoadLfsLocks { .. } => PLAIN,
        Msg::LoadAnnexWhereis { .. } => PLAIN,
        Msg::LoadAnnexUnused { .. } => PLAIN,
        Msg::AppendGitattributesRule { .. } => GIT,
        Msg::Commit { .. } => GIT,
        Msg::SafePushAfterCommit { .. } => GIT,
        Msg::Fetch(message) => fetch(message),
        Msg::PruneMergedBranches { .. } => GIT,
        Msg::PruneLocalTags { .. } => GIT,
        Msg::StartRepoMaintenance { .. } => GIT,
        Msg::SnoozeRepoMaintenance { .. } => PLAIN,
        Msg::Pull { .. } => GIT,
        Msg::PullBranch { .. } => GIT,
        Msg::MergeRef { .. } => GIT,
        Msg::SquashRef { .. } => GIT,
        Msg::PushWithTags { .. } => GIT,
        Msg::PreviewTagPush { .. } => GIT,
        Msg::Push { .. } => GIT,
        Msg::PushAfterCommit { .. } => GIT,
        Msg::ForcePush { .. } => GIT,
        Msg::ForcePushWithLease { .. } => GIT,
        Msg::PushSetUpstream { .. } => GIT,
        Msg::SetUpstreamBranch { .. } => GIT,
        Msg::UnsetUpstreamBranch { .. } => GIT,
        Msg::DeleteRemoteBranch { .. } => GIT,
        Msg::DeleteRemoteBranches { .. } => GIT,
        Msg::Reset { .. } => GIT,
        Msg::PrepareSquash { .. } => GIT,
        Msg::SquashCommits { .. } => GIT,
        Msg::RewordCommit { .. } => GIT,
        Msg::ContinueRewordCommit { .. } => GIT,
        Msg::DismissRewordCommit { .. } => PLAIN,
        Msg::Rebase { .. } => GIT,
        Msg::RebaseContinue { .. } => GIT,
        Msg::RebaseAbort { .. } => GIT,
        Msg::LoadInteractiveRebaseSetup { .. } => PLAIN,
        Msg::OpenInteractiveCherryPickSetup { .. } => PLAIN,
        Msg::InteractiveRebase { .. } => GIT,
        Msg::InteractiveCherryPick { .. } => GIT,
        Msg::CancelInteractiveRebaseSetup { .. } => PLAIN,
        Msg::CancelInteractiveCherryPickSetup { .. } => PLAIN,
        Msg::MergeAbort { .. } => GIT,
        Msg::CreateTag { .. } => GIT,
        Msg::DeleteTag { .. } => GIT,
        Msg::PushTag { .. } => GIT,
        Msg::DeleteRemoteTag { .. } => GIT,
        Msg::AddRemote { .. } => GIT,
        Msg::RemoveRemote { .. } => GIT,
        Msg::SetRemoteUrl { .. } => GIT,
        Msg::CheckoutConflictSide { .. } => GIT,
        Msg::AcceptConflictDeletion { .. } => GIT,
        Msg::CheckoutConflictBase { .. } => GIT,
        Msg::LaunchMergetool { .. } => GIT,
        Msg::RecordConflictAutosolveTelemetry { .. } => PLAIN,
        Msg::ConflictSetHideResolved { .. } => PLAIN,
        Msg::ConflictApplyBulkChoice { .. } => PLAIN,
        Msg::ConflictSetRegionChoice { .. } => PLAIN,
        Msg::ConflictToggleRegionSource { .. } => PLAIN,
        Msg::ConflictReplaceRegionSelection { .. } => PLAIN,
        Msg::ConflictTogglePlanBlockSource { .. } => PLAIN,
        Msg::ConflictReplacePlanBlockSelection { .. } => PLAIN,
        Msg::ConflictSyncRegionResolutions { .. } => PLAIN,
        Msg::ConflictApplyAutosolve { .. } => PLAIN,
        Msg::ConflictResetResolutions { .. } => PLAIN,
        Msg::ConflictSplitRegion { .. } => PLAIN,
        Msg::ConflictAddManualAlignment { .. } => PLAIN,
        Msg::ConflictClearManualAlignments { .. } => PLAIN,
        Msg::ConflictJoinRegions { .. } => PLAIN,
        Msg::Stash { .. } => GIT,
        Msg::ApplyStash { .. } => GIT,
        Msg::PopStash { .. } => GIT,
        Msg::DropStash { .. } => GIT,
        Msg::Internal(message) => message.class(),
    }
}

/// One row per variant, in declaration order, and no wildcard arm: a
/// variant without a row does not compile.
fn internal_class(message: &InternalMsg) -> MsgClass {
    match message {
        InternalMsg::AutoFetchUpdated { .. } => COMPLETION,
        InternalMsg::AutoFetchRefresh { .. } => COMPLETION,
        InternalMsg::QuiesceRepository { .. } => COMPLETION,
        InternalMsg::RepositoryModeLoaded { .. } => COMPLETION,
        InternalMsg::BareConversionPreviewLoaded { .. } => COMPLETION,
        InternalMsg::BareModeDisabled { .. } => COMPLETION,
        InternalMsg::AuthPromptValidated { .. } => COMPLETION,
        InternalMsg::TagPushPreviewLoaded { .. } => COMPLETION,
        InternalMsg::GitOperationStarted { .. } => COMPLETION,
        InternalMsg::GitOperationEvent { .. } => COMPLETION,
        // The operation may have been cancelled, which re-reads the selection.
        InternalMsg::GitOperationFinished { message, .. } => {
            COMPLETION.prepares(message.class().facts.with_queries())
        }
        InternalMsg::SessionPersistFailed { .. } => COMPLETION,
        InternalMsg::CloneRepoProgress { .. } => COMPLETION,
        InternalMsg::CloneRepoFinished { .. } => COMPLETION,
        // A wrapper prepares what its completion does, and navigates nowhere.
        InternalMsg::RepoLoadFinished { message, .. } => COMPLETION.prepares(message.class().facts),
        InternalMsg::RepoOpenedOk { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
        InternalMsg::RepoMaintenanceChecked { .. } => COMPLETION,
        InternalMsg::RepoOpenedErr { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
        InternalMsg::BranchesLoaded { .. } => COMPLETION,
        InternalMsg::RemotesLoaded { .. } => COMPLETION,
        InternalMsg::RemoteBranchesLoaded { .. } => COMPLETION,
        InternalMsg::WorktreeStatusLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::StagedStatusLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::UncommittedLineStatsLoaded { .. } => COMPLETION,
        InternalMsg::StatusLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::HeadBranchLoaded { .. } => COMPLETION,
        InternalMsg::UpstreamDivergenceLoaded { .. } => COMPLETION,
        InternalMsg::HeadTrackingLoaded { .. } => COMPLETION,
        InternalMsg::LogLoaded { .. } => COMPLETION,
        InternalMsg::LogChunkLoaded { .. } => COMPLETION,
        InternalMsg::TagsLoaded { .. } => COMPLETION,
        InternalMsg::RemoteTagsLoaded { .. } => COMPLETION,
        InternalMsg::StashesLoaded { .. } => COMPLETION,
        InternalMsg::ReflogLoaded { .. } => COMPLETION,
        InternalMsg::RecentCommitMessagesLoaded { .. } => COMPLETION,
        InternalMsg::RebaseStateLoaded { .. } => COMPLETION,
        InternalMsg::InteractiveRebaseSetupLoaded { .. } => COMPLETION,
        InternalMsg::InteractiveCherryPickMessagesLoaded { .. } => COMPLETION,
        InternalMsg::MergeCommitMessageLoaded { .. } => COMPLETION,
        InternalMsg::CommitMessageSuggested { .. } => COMPLETION,
        InternalMsg::CommitMessageSuggestionConsumed { .. } => COMPLETION,
        InternalMsg::HoverCommitMessageLoaded { .. } => COMPLETION,
        InternalMsg::FileHistoryLoaded { .. } => COMPLETION,
        InternalMsg::BlameLoaded { .. } => COMPLETION,
        InternalMsg::ConflictFileLoaded { .. } => COMPLETION,
        InternalMsg::WorktreesLoaded { .. } => COMPLETION,
        InternalMsg::WorktreeDirtyLoaded { .. } => COMPLETION,
        InternalMsg::RefMetadataLoaded { .. } => COMPLETION,
        InternalMsg::SubmodulesLoaded { .. } => COMPLETION,
        InternalMsg::LargeFileSupportLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::LfsLocksLoaded { .. } => COMPLETION,
        InternalMsg::AnnexWhereisLoaded { .. } => COMPLETION,
        InternalMsg::AnnexUnusedLoaded { .. } => COMPLETION,
        InternalMsg::FileBrowserLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::SubmoduleAddTrustChecked { .. } => COMPLETION,
        InternalMsg::SubmoduleUpdateTrustChecked { .. } => COMPLETION,
        InternalMsg::SubmoduleLoadTrustChecked { .. } => COMPLETION,
        InternalMsg::CommitDetailsLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::CommitSignaturesVerified { .. } => COMPLETION,
        // A reveal moves the main view when its reference resolves, not when it is
        // asked for; a stale or failed answer moves nothing.
        InternalMsg::CommitRevealResolved { repo_id, .. } => {
            COMPLETION.navigates(*repo_id, Resolved)
        }
        InternalMsg::CommitLookupResolved { .. } => COMPLETION,
        InternalMsg::RangeFilesLoaded { .. } => COMPLETION,
        InternalMsg::SquashMessagePreviewLoaded { .. } => COMPLETION,
        InternalMsg::SquashRebaseSetupLoaded { .. } => COMPLETION,
        InternalMsg::DiffLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::DiffFileLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::TextAttributesLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::DiffPreviewTextFileLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::SubmoduleSummaryLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::InlineSubmoduleDiffLoaded { .. } => COMPLETION,
        InternalMsg::InlineSubmoduleDiffFileLoaded { .. } => COMPLETION,
        InternalMsg::InlineSubmoduleDiffFileImageLoaded { .. } => COMPLETION,
        InternalMsg::DiffFileImageLoaded { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::RepoActionFinished { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::RepoPathsActionFinished { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::BranchAlreadyExists { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::BranchNotFullyMerged { .. } => COMPLETION.prepares(QUERIES),
        InternalMsg::RepoActionFinishedInWorktree { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
        InternalMsg::CommitFinished { .. } => COMPLETION,
        InternalMsg::SafePushAfterCommitFinished { .. } => COMPLETION,
        InternalMsg::RepoCommandFinished { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::CommitId;
    use gitcomet_core::error::{Error, ErrorKind};

    fn error() -> Error {
        Error::new(ErrorKind::Backend("failed".to_string()))
    }

    fn loaded(message: InternalMsg) -> InternalMsg {
        InternalMsg::RepoLoadFinished {
            repo_id: RepoId(1),
            load_epoch: crate::model::RepoLoadEpoch(0),
            message: Box::new(message),
        }
    }

    fn operation(message: InternalMsg) -> InternalMsg {
        InternalMsg::GitOperationFinished {
            repo_id: RepoId(1),
            operation_id: gitcomet_core::git_operation::GitOperationId(1),
            outer_outcome: crate::model::GitOperationOuterOutcome::Succeeded,
            duration: std::time::Duration::ZERO,
            message: Box::new(message),
        }
    }

    fn status() -> InternalMsg {
        InternalMsg::StatusLoaded {
            repo_id: RepoId(1),
            result: Err(error()),
        }
    }

    fn tags() -> InternalMsg {
        InternalMsg::TagsLoaded {
            repo_id: RepoId(1),
            result: Err(error()),
        }
    }

    fn open_failed() -> InternalMsg {
        InternalMsg::RepoOpenedErr {
            repo_id: RepoId(1),
            spec: gitcomet_core::domain::RepoSpec {
                workdir: "repo".into(),
            },
            error: error(),
        }
    }

    #[test]
    fn a_wrapper_prepares_what_its_completion_does() {
        assert_eq!(tags().class().facts, PLAIN.facts);
        assert_eq!(loaded(tags()).class().facts, PLAIN.facts);
        assert_eq!(loaded(status()).class().facts, QUERIES);
        assert_eq!(loaded(open_failed()).class().facts, PATHS_AND_QUERIES);
        assert_eq!(
            operation(loaded(open_failed())).class().facts,
            PATHS_AND_QUERIES
        );
    }

    #[test]
    fn a_finished_operation_always_reads_the_selection() {
        assert_eq!(operation(tags()).class().facts, QUERIES);
        assert_eq!(loaded(operation(tags())).class().facts, QUERIES);
    }

    #[test]
    fn only_an_unwrapped_reveal_navigates() {
        let resolved = || InternalMsg::CommitRevealResolved {
            repo_id: RepoId(7),
            reference: CommitId("a".into()),
            result: Err(error()),
        };
        assert_eq!(
            Msg::Internal(resolved()).class().navigation,
            Some((RepoId(7), Resolved))
        );
        assert_eq!(Msg::Internal(loaded(resolved())).class().navigation, None);
        assert_eq!(
            Msg::Internal(operation(resolved())).class().navigation,
            None
        );
    }

    #[test]
    fn an_internal_message_is_a_completion_without_a_repository_requirement() {
        for message in [tags(), loaded(status()), operation(open_failed())] {
            let class = Msg::Internal(message).class();
            assert!(class.completion && !class.control && !class.requires_git);
            assert_eq!(class.repository, None);
        }
        assert!(!Msg::CancelAuthPrompt.class().completion);
    }
}
