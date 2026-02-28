use gitgpui_core::domain::CommitId;
use gitgpui_core::services::{ConflictSide, PullMode, RemoteUrlKind, ResetMode};
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RepoCommandKind {
    FetchAll,
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
    Push,
    ForcePush,
    PushSetUpstream {
        remote: String,
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
    AddSubmodule {
        url: String,
        path: PathBuf,
    },
    UpdateSubmodules,
    RemoveSubmodule {
        path: PathBuf,
    },
    StageHunk,
    UnstageHunk,
    ApplyWorktreePatch {
        reverse: bool,
    },
}
