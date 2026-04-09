use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{ConflictSide, PullMode, RemoteUrlKind, ResetMode};
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
    ForcePush,
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
    Reset {
        mode: ResetMode,
        target: String,
    },
    Rebase {
        onto: String,
    },
    RebaseContinue,
    RebaseAbort,
    MergeAbort,
    CreateTag {
        name: String,
        target: String,
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
    },
    UpdateSubmodules,
    RemoveSubmodule {
        path: PathBuf,
    },
    LfsFetch,
    LfsPull,
    LfsTrack {
        pattern: String,
    },
    LfsUntrack {
        pattern: String,
    },
    LfsPrune,
    LfsMigrateImport {
        pattern: String,
    },
    AnnexInit,
    AnnexSync,
    AnnexGet {
        path: PathBuf,
    },
    AnnexUnlock {
        path: PathBuf,
    },
    AnnexLock {
        path: PathBuf,
    },
    AnnexAdd {
        path: PathBuf,
    },
    AnnexDrop {
        path: PathBuf,
    },
    StageHunk,
    UnstageHunk,
    ApplyWorktreePatch {
        reverse: bool,
    },
}
