use crate::model::RepoId;
use gitgpui_core::domain::*;
use gitgpui_core::services::{ConflictSide, PullMode, RemoteUrlKind, ResetMode};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum Effect {
    OpenRepo {
        repo_id: RepoId,
        path: PathBuf,
    },
    LoadBranches {
        repo_id: RepoId,
    },
    LoadRemotes {
        repo_id: RepoId,
    },
    LoadRemoteBranches {
        repo_id: RepoId,
    },
    LoadStatus {
        repo_id: RepoId,
    },
    LoadHeadBranch {
        repo_id: RepoId,
    },
    LoadUpstreamDivergence {
        repo_id: RepoId,
    },
    LoadLog {
        repo_id: RepoId,
        scope: LogScope,
        limit: usize,
        cursor: Option<LogCursor>,
    },
    LoadTags {
        repo_id: RepoId,
    },
    LoadStashes {
        repo_id: RepoId,
        limit: usize,
    },
    LoadReflog {
        repo_id: RepoId,
        limit: usize,
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
    LoadRebaseState {
        repo_id: RepoId,
    },
    LoadMergeCommitMessage {
        repo_id: RepoId,
    },
    LoadCommitDetails {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    LoadDiff {
        repo_id: RepoId,
        target: DiffTarget,
    },
    LoadDiffFile {
        repo_id: RepoId,
        target: DiffTarget,
    },
    LoadDiffFileImage {
        repo_id: RepoId,
        target: DiffTarget,
    },
    LoadConflictFile {
        repo_id: RepoId,
        path: PathBuf,
    },
    SaveWorktreeFile {
        repo_id: RepoId,
        path: PathBuf,
        contents: String,
        stage: bool,
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
    },
    CreateBranchAndCheckout {
        repo_id: RepoId,
        name: String,
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
    AddSubmodule {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
    },
    UpdateSubmodules {
        repo_id: RepoId,
    },
    RemoveSubmodule {
        repo_id: RepoId,
        path: PathBuf,
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
    StagePath {
        repo_id: RepoId,
        path: PathBuf,
    },
    StagePaths {
        repo_id: RepoId,
        paths: Vec<PathBuf>,
    },
    UnstagePath {
        repo_id: RepoId,
        path: PathBuf,
    },
    UnstagePaths {
        repo_id: RepoId,
        paths: Vec<PathBuf>,
    },
    DiscardWorktreeChangesPath {
        repo_id: RepoId,
        path: PathBuf,
    },
    DiscardWorktreeChangesPaths {
        repo_id: RepoId,
        paths: Vec<PathBuf>,
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
        prune: bool,
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
    CheckoutConflictBase {
        repo_id: RepoId,
        path: PathBuf,
    },
    LaunchMergetool {
        repo_id: RepoId,
        path: PathBuf,
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
    DropStash {
        repo_id: RepoId,
        index: usize,
    },
    PopStash {
        repo_id: RepoId,
        index: usize,
    },
}
