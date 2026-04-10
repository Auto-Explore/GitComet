use crate::model::{ConflictFileLoadMode, RepoId};
use gitcomet_core::conflict_session::ConflictSession;
use gitcomet_core::domain::*;
use gitcomet_core::error::Error;
use gitcomet_core::services::GitRepository;
use gitcomet_core::services::{
    CommandOutput, ConflictSide, PullMode, RemoteUrlKind, ResetMode, SubmoduleTrustDecision,
    SubmoduleTrustTarget,
};
use std::path::PathBuf;
use std::sync::Arc;

use super::repo_command_kind::RepoCommandKind;
use super::repo_external_change::RepoExternalChange;
use super::{RepoPath, RepoPathList};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConflictBulkChoice {
    Base,
    Ours,
    Theirs,
    Both,
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

#[derive(Debug)]
pub enum Msg {
    OpenRepo(PathBuf),
    RestoreSession {
        open_repos: Vec<PathBuf>,
        active_repo: Option<PathBuf>,
    },
    CloseRepo {
        repo_id: RepoId,
    },
    ShowBannerError {
        repo_id: Option<RepoId>,
        message: String,
    },
    DismissBannerError,
    DismissRepoError {
        repo_id: RepoId,
    },
    SubmitAuthPrompt {
        username: Option<String>,
        secret: String,
    },
    CancelAuthPrompt,
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
    RepoExternallyChanged {
        repo_id: RepoId,
        change: RepoExternalChange,
    },
    SetHistoryScope {
        repo_id: RepoId,
        scope: LogScope,
    },
    SetFetchPruneDeletedRemoteTrackingBranches {
        repo_id: RepoId,
        enabled: bool,
    },
    LoadMoreHistory {
        repo_id: RepoId,
    },
    SelectCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    ClearCommitSelection {
        repo_id: RepoId,
    },
    SelectDiff {
        repo_id: RepoId,
        target: DiffTarget,
    },
    SelectConflictDiff {
        repo_id: RepoId,
        path: PathBuf,
    },
    ClearDiffSelection {
        repo_id: RepoId,
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
    LoadFileHistory {
        repo_id: RepoId,
        path: PathBuf,
        limit: usize,
    },
    LoadBlame {
        repo_id: RepoId,
        path: PathBuf,
        rev: Option<String>,
    },
    LoadWorktrees {
        repo_id: RepoId,
    },
    LoadSubmodules {
        repo_id: RepoId,
    },
    RefreshBranches {
        repo_id: RepoId,
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
    },
    CheckoutCommit {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    CherryPickCommit {
        repo_id: RepoId,
        commit_id: CommitId,
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
    },
    DeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    ForceDeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    CloneRepo {
        url: String,
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
    ConfirmSubmoduleTrustPrompt,
    CancelSubmoduleTrustPrompt,
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
    Commit {
        repo_id: RepoId,
        message: String,
    },
    CommitAmend {
        repo_id: RepoId,
        message: String,
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
    ForcePush {
        repo_id: RepoId,
    },
    PushSetUpstream {
        repo_id: RepoId,
        remote: String,
        branch: String,
    },
    SetUpstreamBranch {
        repo_id: RepoId,
        branch: String,
        upstream: String,
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
    Reset {
        repo_id: RepoId,
        target: String,
        mode: ResetMode,
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
    MergeAbort {
        repo_id: RepoId,
    },
    CreateTag {
        repo_id: RepoId,
        name: String,
        target: String,
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
    },
    ConflictSetRegionChoice {
        repo_id: RepoId,
        path: RepoPath,
        region_index: usize,
        choice: ConflictRegionChoice,
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
        scope: LogScope,
        cursor: Option<LogCursor>,
        result: Result<LogPage, Error>,
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
    RebaseStateLoaded {
        repo_id: RepoId,
        result: Result<bool, Error>,
    },
    MergeCommitMessageLoaded {
        repo_id: RepoId,
        result: Result<Option<String>, Error>,
    },
    FileHistoryLoaded {
        repo_id: RepoId,
        path: PathBuf,
        result: Result<LogPage, Error>,
    },
    BlameLoaded {
        repo_id: RepoId,
        path: PathBuf,
        rev: Option<String>,
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
    SubmodulesLoaded {
        repo_id: RepoId,
        result: Result<Vec<Submodule>, Error>,
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
    CommitDetailsLoaded {
        repo_id: RepoId,
        commit_id: CommitId,
        result: Result<CommitDetails, Error>,
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
    DiffFileImageLoaded {
        repo_id: RepoId,
        target: DiffTarget,
        result: Result<Option<FileDiffImage>, Error>,
    },
    RepoActionFinished {
        repo_id: RepoId,
        result: Result<(), Error>,
    },
    CommitFinished {
        repo_id: RepoId,
        result: Result<(), Error>,
    },
    CommitAmendFinished {
        repo_id: RepoId,
        result: Result<(), Error>,
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
    use super::{InternalMsg, Msg};
    use crate::model::RepoId;
    use gitcomet_core::error::{Error, ErrorKind};
    use std::path::PathBuf;

    #[test]
    fn wraps_internal_messages() {
        let msg: Msg = InternalMsg::RepoActionFinished {
            repo_id: RepoId(7),
            result: Ok(()),
        }
        .into();

        assert!(matches!(
            msg,
            Msg::Internal(InternalMsg::RepoActionFinished {
                repo_id: RepoId(7),
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
