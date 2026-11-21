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

/// Expands a table of one row per variant into a match with no wildcard arm:
/// a variant without a row does not compile.
macro_rules! classify {
    ($message:ident, $name:ident:
        $($variant:ident $(($($tuple:tt)*))? $({$($field:tt)*})? => $class:expr,)+
    ) => {
        fn $name(message: &$message) -> MsgClass {
            match message {
                $($message::$variant $(($($tuple)*))? $({$($field)*})? => $class,)+
            }
        }
    };
}

// In declaration order.
classify! {
    Msg, msg_class:
    OpenDocumentRepository { .. } => PLAIN.prepares(PATHS_AND_QUERIES),
    RememberDocumentInRepository { .. } => PLAIN,
    FilesystemRequest(..) => PLAIN,
    FilesystemProgress(..) => PLAIN,
    AcknowledgeFilesystemResults(..) => PLAIN,
    FilesystemJournalUpdated { .. } => PLAIN,
    FilesystemFinished(..) => PLAIN.prepares(QUERIES),
    FilesystemPathsChanged(..) => PLAIN.prepares(QUERIES),
    SelectExplorerPath { .. } => PLAIN,
    FocusExplorerPath { .. } => PLAIN,
    SelectAllExplorerPaths { .. } => PLAIN,
    SetExplorerVisibility { .. } => PLAIN,
    IndexedHistory(message) => indexed_history(message),
    DiffSession(..) => PLAIN,
    HistoryAuthors(..) => PLAIN,
    HistoryFind(..) => PLAIN,
    OpenRepo(..) => GIT.control().prepares(PATHS_AND_QUERIES),
    OpenRepoFromExternalDrop(..) => GIT.control().prepares(PATHS_AND_QUERIES),
    AcknowledgeRepoOpenFailures { .. } => PLAIN,
    AcknowledgeNotifications { .. } => PLAIN.acknowledgement(),
    AcknowledgeRepoFeedback { .. } => PLAIN.acknowledgement(),
    RestoreSession { .. } => GIT.prepares(PATHS_AND_QUERIES),
    CloseRepo { .. } => PLAIN.control().prepares(QUERIES),
    MoveRepoOut { .. } => PLAIN.control().prepares(QUERIES),
    CloseRepos { .. } => PLAIN.control().prepares(QUERIES),
    ReportError { .. } => PLAIN,
    DismissRepoError { .. } => PLAIN,
    CancelGitOperation { .. } => PLAIN.control(),
    // The worktree removal it retries carries a path.
    SubmitAuthPrompt { .. } => PLAIN.prepares(PATHS),
    CancelAuthPrompt => PLAIN,
    SetGitRuntimeState(..) => PLAIN,
    SetSigningToolsState(..) => PLAIN,
    SetLargeFileToolsState(..) => PLAIN,
    SetCommitSignatureTargets { .. } => PLAIN,
    SetRemoteUrlPolicy(..) => PLAIN,
    SetGitLogSettings { .. } => PLAIN,
    SetRemoteSettings(..) => PLAIN,
    SetMaintenanceSettings(..) => PLAIN,
    SetFileBrowserSettings(..) => PLAIN,
    SetLargeFileSettings(..) => PLAIN,
    SetHistoryHiddenRefs(..) => PLAIN,
    SetDefaultTagType(..) => PLAIN,
    SetActiveRepo { .. } => PLAIN.control().prepares(QUERIES),
    ReorderRepoTabs { .. } => PLAIN.control(),
    ReloadRepo { .. } => GIT.prepares(QUERIES),
    PreviewDisableBareMode { .. } => GIT,
    DisableBareMode { .. } => GIT,
    ClearBareConversionPreview { .. } => PLAIN,
    RepoActivated { .. } => GIT,
    AcquireWatchLease { .. } => PLAIN,
    ReleaseWatchLease { .. } => PLAIN,
    WatchWorktree { .. } => PLAIN,
    WorktreeExternallyChanged { .. } => PLAIN,
    RepoExternallyChanged { .. } => GIT.prepares(QUERIES),
    RepoWatchDegraded { .. } => PLAIN,
    UpdateRepositoryPreference { .. } => PLAIN,
    ApplyRepositoryPreferences(..) => PLAIN,
    SetHistoryScope { .. } => GIT,
    SetHistoryAuthorFilter { .. } => GIT,
    SetHistoryFocus { .. } => GIT,
    ToggleHistoryFocus { .. } => GIT,
    FocusHistoryRefs { .. } => GIT,
    LoadMoreHistory { .. } => GIT,
    SelectCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
    SelectCommitMulti { repo_id, .. } => PLAIN.navigates(*repo_id, Request),
    ClearCommitSelection { .. } => PLAIN,
    CompareCommitRange { repo_id, .. } => GIT.navigates(*repo_id, Step),
    CompareWithWorkingTree { repo_id, .. } => GIT.navigates(*repo_id, Step),
    CompareWithOptions { repo_id, .. } => GIT.navigates(*repo_id, Step),
    ClearComparison { .. } => PLAIN,
    MarkForComparison { .. } => PLAIN,
    CompareWithMarked { repo_id, .. } => GIT.navigates(*repo_id, Step),
    ClearComparisonMark { .. } => PLAIN,
    SelectDiff { repo_id, .. } => GIT.navigates(*repo_id, Step).prepares(QUERIES),
    SetTextOverride { .. } => GIT.prepares(QUERIES),
    OpenInlineSubmoduleDiff { repo_id, .. } => PLAIN.navigates(*repo_id, Step),
    SelectInlineSubmoduleDiff { repo_id, .. } => PLAIN.navigates(*repo_id, Step),
    CloseInlineSubmoduleDiff { .. } => PLAIN,
    SelectConflictDiff { repo_id, .. } => GIT.navigates(*repo_id, Step),
    ClearDiffSelection { .. } => PLAIN,
    EnsureSidebarData { .. } => PLAIN,
    LoadStashes { .. } => GIT,
    LoadConflictFile { .. } => GIT,
    LoadReflog { .. } => GIT,
    LoadRecentCommitMessages { .. } => GIT,
    LoadHoverCommitMessage { .. } => GIT,
    LoadFileHistory { .. } => GIT,
    LoadBlame { .. } => GIT,
    LoadWorktrees { .. } => GIT,
    LoadWorktreeDirty { .. } => GIT,
    // Selecting a linked-worktree row is a destination like any other history
    // selection; it just is not a commit.
    SelectWorktreeUncommitted { repo_id, .. } => GIT.navigates(*repo_id, Step),
    LoadRefMetadata { .. } => GIT,
    LoadSubmodules { .. } => GIT,
    LoadTags { .. } => GIT,
    LoadRemoteTags { .. } => GIT,
    RefreshBranches { .. } => GIT,
    LoadFileBrowser { .. } => GIT,
    ToggleFileBrowserDir { .. } => PLAIN,
    SetFileBrowserDirExpandedRecursive { .. } => PLAIN,
    SetFileBrowserSearch { .. } => PLAIN,
    RevealFileBrowserPath { .. } => PLAIN,
    SetFileBrowserSource { .. } => PLAIN,
    OpenFileContent { repo_id, .. } => GIT.navigates(*repo_id, Step).prepares(QUERIES),
    OpenFileEditor { repo_id, .. } => GIT.navigates(*repo_id, Step).prepares(QUERIES),
    // Leaving the editor is a destination of its own, so Back returns to the
    // editor rather than skipping past it to whatever preceded it.
    ExitDiffEditMode { repo_id, .. } => PLAIN.navigates(*repo_id, Step).prepares(QUERIES),
    OpenFileAtCommitParent { .. } => GIT,
    OpenFileAtCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
    ShowFileChangesAtCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
    BrowseRepositoryAtCommit { repo_id, .. } => GIT.navigates(*repo_id, Step),
    RevealCommit { repo_id, .. } => GIT.navigates(*repo_id, Request),
    FinishCommitReveal { .. } => PLAIN,
    ResolveCommitLookup { .. } => GIT,
    ResetBrowseToLive { repo_id, .. } => GIT.navigates(*repo_id, Step),
    ViewerNavBack { repo_id, .. } => {
        GIT.navigates(*repo_id, Replay(Viewer, Back)).prepares(QUERIES)
    },
    ViewerNavForward { repo_id, .. } => {
        GIT.navigates(*repo_id, Replay(Viewer, Forward)).prepares(QUERIES)
    },
    GlobalNavBack { repo_id, .. } => {
        GIT.navigates(*repo_id, Replay(Main, Back)).prepares(QUERIES)
    },
    GlobalNavForward { repo_id, .. } => {
        GIT.navigates(*repo_id, Replay(Main, Forward)).prepares(QUERIES)
    },
    SetSidebarMode { .. } => PLAIN,
    StageHunk { .. } => GIT,
    ApplyPreparedDiffPatch { .. } => GIT,
    UnstageHunk { .. } => GIT,
    ApplyWorktreePatch { .. } => GIT,
    CheckoutBranch { .. } => GIT,
    CheckoutRemoteBranch { .. } => GIT,
    CheckoutCommit { .. } => GIT,
    CherryPickCommit { .. } => GIT,
    RevertCommit { .. } => GIT,
    ApplyFileChange { .. } => GIT,
    CreateBranch { .. } => GIT,
    CreateBranchAndCheckout { .. } => GIT,
    ResolveBranchExistsPrompt { .. } => PLAIN,
    ShowBranchExistsPrompt { .. } => PLAIN,
    DismissForceDeleteBranchPrompt { .. } => PLAIN,
    RenameBranch { .. } => GIT,
    DeleteBranch { .. } => GIT,
    ForceDeleteBranch { .. } => GIT,
    DeleteBranches { .. } => GIT,
    CloneRepo { .. } => GIT,
    AbortCloneRepo { .. } => PLAIN,
    ExportPatch { .. } => GIT,
    ApplyPatch { .. } => GIT,
    AddWorktree { .. } => GIT,
    RemoveWorktree { .. } => GIT.prepares(PATHS),
    ForceRemoveWorktree { .. } => GIT.prepares(PATHS),
    DismissForceRemoveWorktreePrompt { .. } => PLAIN,
    AddSubmodule { .. } => GIT,
    AddSubmoduleTrusted { .. } => PLAIN,
    UpdateSubmodules { .. } => GIT,
    UpdateSubmodulesTrusted { .. } => PLAIN,
    LoadSubmodule { .. } => GIT,
    LoadSubmoduleTrusted { .. } => PLAIN,
    ConfirmSubmoduleTrustPrompt => PLAIN,
    CancelSubmoduleTrustPrompt => PLAIN,
    ChangeSubmodulePointer { .. } => GIT,
    RemoveSubmodule { .. } => GIT,
    StagePath { .. } => GIT,
    StagePaths { .. } => GIT,
    UnstagePath { .. } => GIT,
    UnstagePaths { .. } => GIT,
    DiscardWorktreeChangesPath { .. } => GIT,
    DiscardWorktreeChangesPaths { .. } => GIT,
    SaveWorktreeFile { .. } => GIT,
    AppendGitignorePatterns { .. } => GIT,
    RunLargeFileCommand { .. } => GIT,
    LoadLfsLocks { .. } => PLAIN,
    LoadAnnexWhereis { .. } => PLAIN,
    LoadAnnexUnused { .. } => PLAIN,
    AppendGitattributesRule { .. } => GIT,
    Commit { .. } => GIT,
    SafePushAfterCommit { .. } => GIT,
    Fetch(message) => fetch(message),
    PruneMergedBranches { .. } => GIT,
    PruneLocalTags { .. } => GIT,
    StartRepoMaintenance { .. } => GIT,
    SnoozeRepoMaintenance { .. } => PLAIN,
    Pull { .. } => GIT,
    PullBranch { .. } => GIT,
    MergeRef { .. } => GIT,
    SquashRef { .. } => GIT,
    PushWithTags { .. } => GIT,
    PreviewTagPush { .. } => GIT,
    Push { .. } => GIT,
    PushAfterCommit { .. } => GIT,
    ForcePush { .. } => GIT,
    ForcePushWithLease { .. } => GIT,
    PushSetUpstream { .. } => GIT,
    SetUpstreamBranch { .. } => GIT,
    UnsetUpstreamBranch { .. } => GIT,
    DeleteRemoteBranch { .. } => GIT,
    DeleteRemoteBranches { .. } => GIT,
    Reset { .. } => GIT,
    PrepareSquash { .. } => GIT,
    SquashCommits { .. } => GIT,
    RewordCommit { .. } => GIT,
    ContinueRewordCommit { .. } => GIT,
    DismissRewordCommit { .. } => PLAIN,
    Rebase { .. } => GIT,
    RebaseContinue { .. } => GIT,
    RebaseAbort { .. } => GIT,
    LoadInteractiveRebaseSetup { .. } => PLAIN,
    OpenInteractiveCherryPickSetup { .. } => PLAIN,
    InteractiveRebase { .. } => GIT,
    InteractiveCherryPick { .. } => GIT,
    CancelInteractiveRebaseSetup { .. } => PLAIN,
    CancelInteractiveCherryPickSetup { .. } => PLAIN,
    MergeAbort { .. } => GIT,
    CreateTag { .. } => GIT,
    DeleteTag { .. } => GIT,
    PushTag { .. } => GIT,
    DeleteRemoteTag { .. } => GIT,
    AddRemote { .. } => GIT,
    RemoveRemote { .. } => GIT,
    SetRemoteUrl { .. } => GIT,
    CheckoutConflictSide { .. } => GIT,
    AcceptConflictDeletion { .. } => GIT,
    CheckoutConflictBase { .. } => GIT,
    LaunchMergetool { .. } => GIT,
    RecordConflictAutosolveTelemetry { .. } => PLAIN,
    ConflictSetHideResolved { .. } => PLAIN,
    ConflictApplyBulkChoice { .. } => PLAIN,
    ConflictSetRegionChoice { .. } => PLAIN,
    ConflictToggleRegionSource { .. } => PLAIN,
    ConflictReplaceRegionSelection { .. } => PLAIN,
    ConflictTogglePlanBlockSource { .. } => PLAIN,
    ConflictReplacePlanBlockSelection { .. } => PLAIN,
    ConflictSyncRegionResolutions { .. } => PLAIN,
    ConflictApplyAutosolve { .. } => PLAIN,
    ConflictResetResolutions { .. } => PLAIN,
    ConflictSplitRegion { .. } => PLAIN,
    ConflictAddManualAlignment { .. } => PLAIN,
    ConflictClearManualAlignments { .. } => PLAIN,
    ConflictJoinRegions { .. } => PLAIN,
    Stash { .. } => GIT,
    ApplyStash { .. } => GIT,
    PopStash { .. } => GIT,
    DropStash { .. } => GIT,
    Internal(message) => message.class(),
}

// In declaration order.
classify! {
    InternalMsg, internal_class:
    AutoFetchUpdated { .. } => COMPLETION,
    AutoFetchRefresh { .. } => COMPLETION,
    QuiesceRepository { .. } => COMPLETION,
    RepositoryModeLoaded { .. } => COMPLETION,
    BareConversionPreviewLoaded { .. } => COMPLETION,
    BareModeDisabled { .. } => COMPLETION,
    AuthPromptValidated { .. } => COMPLETION,
    TagPushPreviewLoaded { .. } => COMPLETION,
    GitOperationStarted { .. } => COMPLETION,
    GitOperationEvent { .. } => COMPLETION,
    // The operation may have been cancelled, which re-reads the selection.
    GitOperationFinished { message, .. } => {
        COMPLETION.prepares(message.class().facts.with_queries())
    },
    SessionPersistFailed { .. } => COMPLETION,
    CloneRepoProgress { .. } => COMPLETION,
    CloneRepoFinished { .. } => COMPLETION,
    // A wrapper prepares what its completion does, and navigates nowhere.
    RepoLoadFinished { message, .. } => COMPLETION.prepares(message.class().facts),
    RepoOpenedOk { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
    RepoMaintenanceChecked { .. } => COMPLETION,
    RepoOpenedErr { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
    BranchesLoaded { .. } => COMPLETION,
    RemotesLoaded { .. } => COMPLETION,
    RemoteBranchesLoaded { .. } => COMPLETION,
    WorktreeStatusLoaded { .. } => COMPLETION.prepares(QUERIES),
    StagedStatusLoaded { .. } => COMPLETION.prepares(QUERIES),
    UncommittedLineStatsLoaded { .. } => COMPLETION,
    StatusLoaded { .. } => COMPLETION.prepares(QUERIES),
    HeadBranchLoaded { .. } => COMPLETION,
    UpstreamDivergenceLoaded { .. } => COMPLETION,
    HeadTrackingLoaded { .. } => COMPLETION,
    LogLoaded { .. } => COMPLETION,
    LogChunkLoaded { .. } => COMPLETION,
    TagsLoaded { .. } => COMPLETION,
    RemoteTagsLoaded { .. } => COMPLETION,
    StashesLoaded { .. } => COMPLETION,
    ReflogLoaded { .. } => COMPLETION,
    RecentCommitMessagesLoaded { .. } => COMPLETION,
    RebaseStateLoaded { .. } => COMPLETION,
    InteractiveRebaseSetupLoaded { .. } => COMPLETION,
    InteractiveCherryPickMessagesLoaded { .. } => COMPLETION,
    MergeCommitMessageLoaded { .. } => COMPLETION,
    CommitMessageSuggested { .. } => COMPLETION,
    CommitMessageSuggestionConsumed { .. } => COMPLETION,
    HoverCommitMessageLoaded { .. } => COMPLETION,
    FileHistoryLoaded { .. } => COMPLETION,
    BlameLoaded { .. } => COMPLETION,
    ConflictFileLoaded { .. } => COMPLETION,
    WorktreesLoaded { .. } => COMPLETION,
    WorktreeDirtyLoaded { .. } => COMPLETION,
    RefMetadataLoaded { .. } => COMPLETION,
    SubmodulesLoaded { .. } => COMPLETION,
    LargeFileSupportLoaded { .. } => COMPLETION.prepares(QUERIES),
    LfsLocksLoaded { .. } => COMPLETION,
    AnnexWhereisLoaded { .. } => COMPLETION,
    AnnexUnusedLoaded { .. } => COMPLETION,
    FileBrowserLoaded { .. } => COMPLETION.prepares(QUERIES),
    SubmoduleAddTrustChecked { .. } => COMPLETION,
    SubmoduleUpdateTrustChecked { .. } => COMPLETION,
    SubmoduleLoadTrustChecked { .. } => COMPLETION,
    CommitDetailsLoaded { .. } => COMPLETION.prepares(QUERIES),
    CommitSignaturesVerified { .. } => COMPLETION,
    // A reveal moves the main view when its reference resolves, not when it is
    // asked for; a stale or failed answer moves nothing.
    CommitRevealResolved { repo_id, .. } => COMPLETION.navigates(*repo_id, Resolved),
    CommitLookupResolved { .. } => COMPLETION,
    RangeFilesLoaded { .. } => COMPLETION,
    SquashMessagePreviewLoaded { .. } => COMPLETION,
    SquashRebaseSetupLoaded { .. } => COMPLETION,
    DiffLoaded { .. } => COMPLETION.prepares(QUERIES),
    DiffFileLoaded { .. } => COMPLETION.prepares(QUERIES),
    TextAttributesLoaded { .. } => COMPLETION.prepares(QUERIES),
    DiffPreviewTextFileLoaded { .. } => COMPLETION.prepares(QUERIES),
    SubmoduleSummaryLoaded { .. } => COMPLETION.prepares(QUERIES),
    InlineSubmoduleDiffLoaded { .. } => COMPLETION,
    InlineSubmoduleDiffFileLoaded { .. } => COMPLETION,
    InlineSubmoduleDiffFileImageLoaded { .. } => COMPLETION,
    DiffFileImageLoaded { .. } => COMPLETION.prepares(QUERIES),
    RepoActionFinished { .. } => COMPLETION.prepares(QUERIES),
    RepoPathsActionFinished { .. } => COMPLETION.prepares(QUERIES),
    BranchAlreadyExists { .. } => COMPLETION.prepares(QUERIES),
    BranchNotFullyMerged { .. } => COMPLETION.prepares(QUERIES),
    RepoActionFinishedInWorktree { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
    CommitFinished { .. } => COMPLETION,
    SafePushAfterCommitFinished { .. } => COMPLETION,
    RepoCommandFinished { .. } => COMPLETION.prepares(PATHS_AND_QUERIES),
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
