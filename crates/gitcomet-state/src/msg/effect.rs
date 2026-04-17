use crate::model::{ConflictFileLoadMode, RepoId};
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::domain::*;
use gitcomet_core::services::{
    ConflictSide, PullMode, RemoteUrlKind, ResetMode, SubmoduleTrustTarget,
};
use std::path::PathBuf;

use super::RepoPathList;

#[derive(Clone, Debug)]
pub enum Effect {
    PersistSession {
        repo_id: Option<RepoId>,
        action: &'static str,
    },
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
    LoadWorktreeStatus {
        repo_id: RepoId,
    },
    LoadStagedStatus {
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
    LoadRemoteTags {
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
    LoadSubtrees {
        repo_id: RepoId,
    },
    LoadRebaseAndMergeState {
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
    LoadDiffPreviewTextFile {
        repo_id: RepoId,
        target: DiffTarget,
        side: DiffPreviewTextSide,
    },
    LoadDiffFileImage {
        repo_id: RepoId,
        target: DiffTarget,
    },
    LoadSelectedDiff {
        repo_id: RepoId,
        load_patch_diff: bool,
        load_file_text: bool,
        preview_text_side: Option<DiffPreviewTextSide>,
        load_file_image: bool,
    },
    LoadSelectedConflictFile {
        repo_id: RepoId,
        mode: ConflictFileLoadMode,
    },
    LoadConflictFile {
        repo_id: RepoId,
        path: PathBuf,
        mode: ConflictFileLoadMode,
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
        auth: Option<StagedGitAuth>,
    },
    ExtractSubtree {
        repo_id: RepoId,
        path: PathBuf,
        options: SubtreeExtractOptions,
        auth: Option<StagedGitAuth>,
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
    CheckSubmoduleAddTrust {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
    },
    CheckSubmoduleUpdateTrust {
        repo_id: RepoId,
    },
    AddSubmodule {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
        approved_sources: Vec<SubmoduleTrustTarget>,
        auth: Option<StagedGitAuth>,
    },
    UpdateSubmodules {
        repo_id: RepoId,
        approved_sources: Vec<SubmoduleTrustTarget>,
        auth: Option<StagedGitAuth>,
    },
    RemoveSubmodule {
        repo_id: RepoId,
        path: PathBuf,
    },
    AddSubtree {
        repo_id: RepoId,
        repository: String,
        reference: String,
        path: PathBuf,
        squash: bool,
        auth: Option<StagedGitAuth>,
    },
    PullSubtree {
        repo_id: RepoId,
        repository: String,
        reference: String,
        path: PathBuf,
        squash: bool,
        auth: Option<StagedGitAuth>,
    },
    PushSubtree {
        repo_id: RepoId,
        repository: String,
        refspec: String,
        path: PathBuf,
        auth: Option<StagedGitAuth>,
    },
    MergeSubtree {
        repo_id: RepoId,
        path: PathBuf,
        options: SubtreeMergeOptions,
    },
    SplitSubtree {
        repo_id: RepoId,
        path: PathBuf,
        branch: Option<String>,
    },
    RemoveSubtree {
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
    Commit {
        repo_id: RepoId,
        message: String,
        auth: Option<StagedGitAuth>,
    },
    CommitAmend {
        repo_id: RepoId,
        message: String,
        auth: Option<StagedGitAuth>,
    },
    FetchAll {
        repo_id: RepoId,
        prune: bool,
        auth: Option<StagedGitAuth>,
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
        auth: Option<StagedGitAuth>,
    },
    PullBranch {
        repo_id: RepoId,
        remote: String,
        branch: String,
        auth: Option<StagedGitAuth>,
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
        auth: Option<StagedGitAuth>,
    },
    ForcePush {
        repo_id: RepoId,
        auth: Option<StagedGitAuth>,
    },
    PushSetUpstream {
        repo_id: RepoId,
        remote: String,
        branch: String,
        auth: Option<StagedGitAuth>,
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
        auth: Option<StagedGitAuth>,
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
        auth: Option<StagedGitAuth>,
    },
    DeleteRemoteTag {
        repo_id: RepoId,
        remote: String,
        name: String,
        auth: Option<StagedGitAuth>,
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
}
