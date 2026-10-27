use super::{InternalMsg, RepoCommandKind};
use crate::model::RepoId;
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::domain::*;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::{
    ConflictSide, ForcePushLease, InteractiveRebaseEntry, PullMode, RemoteUrlKind, ResetMode,
    SafePushAfterCommitContext, SafePushAfterCommitTarget, SubmoduleTrustTarget, WorktreeCheckout,
};
use std::path::PathBuf;

#[derive(Clone, Debug, strum::IntoStaticStr)]
pub enum RepoCommandEffect {
    SaveWorktreeFile {
        repo_id: RepoId,
        path: PathBuf,
        contents: super::message::ContentBytes,
        expected_contents: Option<std::sync::Arc<[u8]>>,
        stage: bool,
        completion: Option<smol::channel::Sender<bool>>,
    },
    AppendGitignorePatterns {
        repo_id: RepoId,
        patterns: Vec<String>,
    },
    RunLargeFileCommand {
        repo_id: RepoId,
        command: gitcomet_core::large_files::LargeFileCommand,
        auth: Option<StagedGitAuth>,
    },
    AppendGitattributesRule {
        repo_id: RepoId,
        rule: String,
    },
    CherryPickCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        commit: bool,
        mainline: Option<usize>,
        summary: String,
        /// Signing or fetch auth staged when a failed pick is replayed.
        auth: Option<StagedGitAuth>,
    },
    RevertCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        commit: bool,
        mainline: Option<usize>,
        summary: String,
        /// Signing or fetch auth staged when a failed revert is replayed.
        auth: Option<StagedGitAuth>,
    },
    ApplyFileChange {
        repo_id: RepoId,
        target: gitcomet_core::domain::ApplyChangeTarget,
        commit: bool,
        commit_retry: Option<gitcomet_core::domain::ApplyFileChangeRetry>,
        /// Signing auth staged when a failed commit step is replayed.
        auth: Option<StagedGitAuth>,
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
        checkout: WorktreeCheckout,
        open_after: bool,
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
        remote_url_policy: RemoteUrlPolicy,
    },
    CheckSubmoduleUpdateTrust {
        repo_id: RepoId,
        remote_url_policy: RemoteUrlPolicy,
    },
    CheckSubmoduleLoadTrust {
        repo_id: RepoId,
        path: PathBuf,
        remote_url_policy: RemoteUrlPolicy,
    },
    AddSubmodule {
        repo_id: RepoId,
        url: String,
        path: PathBuf,
        branch: Option<String>,
        name: Option<String>,
        force: bool,
        approved_sources: Vec<SubmoduleTrustTarget>,
        remote_url_policy: RemoteUrlPolicy,
        auth: Option<StagedGitAuth>,
    },
    UpdateSubmodules {
        repo_id: RepoId,
        approved_sources: Vec<SubmoduleTrustTarget>,
        remote_url_policy: RemoteUrlPolicy,
        auth: Option<StagedGitAuth>,
    },
    LoadSubmodule {
        repo_id: RepoId,
        path: PathBuf,
        approved_sources: Vec<SubmoduleTrustTarget>,
        remote_url_policy: RemoteUrlPolicy,
        auth: Option<StagedGitAuth>,
    },
    ChangeSubmodulePointer {
        repo_id: RepoId,
        path: PathBuf,
        reference: String,
    },
    RemoveSubmodule {
        repo_id: RepoId,
        path: PathBuf,
    },
    StageHunk {
        repo_id: RepoId,
        patch: super::ContentBytes,
    },
    ApplyPreparedDiffPatch {
        repo_id: RepoId,
        lifetime: u64,
        view: Option<crate::diff_session::DiffViewId>,
        file_rev: u64,
        generation: u64,
        prepared: std::sync::Arc<gitcomet_core::patch_action::PreparedDiffPatch>,
    },
    UnstageHunk {
        repo_id: RepoId,
        patch: super::ContentBytes,
    },
    ApplyWorktreePatch {
        repo_id: RepoId,
        patch: super::ContentBytes,
        reverse: bool,
    },
    SafePushAfterCommit {
        repo_id: RepoId,
        context: SafePushAfterCommitContext,
        auth: Option<StagedGitAuth>,
    },
    FetchAll {
        repo_id: RepoId,
        prune: bool,
        auth: Option<StagedGitAuth>,
    },
    FetchRefspecs {
        repo_id: RepoId,
        remote: String,
        refspecs: Vec<String>,
        auth: Option<StagedGitAuth>,
    },
    PruneMergedBranches {
        repo_id: RepoId,
    },
    PruneLocalTags {
        repo_id: RepoId,
    },
    CheckRepoMaintenance {
        repo_id: RepoId,
    },
    RunMaintenance {
        repo_id: RepoId,
    },
    Pull {
        repo_id: RepoId,
        mode: PullMode,
        prune: bool,
        auth: Option<StagedGitAuth>,
    },
    PullBranch {
        repo_id: RepoId,
        remote: String,
        branch: String,
        prune: bool,
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
    PushWithTags {
        repo_id: RepoId,
        request: gitcomet_core::tag_push::TagPushRequest,
        auth: Option<StagedGitAuth>,
    },
    Push {
        repo_id: RepoId,
        auth: Option<StagedGitAuth>,
    },
    PushAfterCommit {
        repo_id: RepoId,
        target: SafePushAfterCommitTarget,
        set_upstream: bool,
        auth: Option<StagedGitAuth>,
    },
    ForcePush {
        repo_id: RepoId,
        auth: Option<StagedGitAuth>,
    },
    ForcePushWithLease {
        repo_id: RepoId,
        lease: ForcePushLease,
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
        upstream: Upstream,
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
    DeleteRemoteBranches {
        repo_id: RepoId,
        remote: String,
        branches: Vec<String>,
        auth: Option<StagedGitAuth>,
    },
    Reset {
        repo_id: RepoId,
        target: String,
        mode: ResetMode,
    },
    SquashCommits {
        repo_id: RepoId,
        oldest: CommitId,
        expected_head: CommitId,
        message: String,
        count: usize,
    },
    RewordCommit {
        repo_id: RepoId,
        request_id: crate::model::RewordRequestId,
        commit_id: CommitId,
        message: String,
        resume: bool,
        /// Signing auth staged when a failed reword is replayed.
        auth: Option<StagedGitAuth>,
    },
    Rebase {
        repo_id: RepoId,
        onto: String,
    },
    RebaseContinue {
        repo_id: RepoId,
        /// Signing auth (e.g. an ssh/gpg key passphrase) staged for the
        /// replayed commit when the continue retries a sequencer step that
        /// previously failed on a passphrase prompt.
        auth: Option<StagedGitAuth>,
    },
    RebaseAbort {
        repo_id: RepoId,
    },
    InteractiveRebase {
        repo_id: RepoId,
        base: String,
        entries: Vec<InteractiveRebaseEntry>,
        /// True for the user-opened editor; false for automated todo-list
        /// rebases (e.g. squashing history without HEAD).
        interactive: bool,
    },
    InteractiveCherryPick {
        repo_id: RepoId,
        entries: Vec<InteractiveRebaseEntry>,
        commit: bool,
    },
    MergeAbort {
        repo_id: RepoId,
    },
    CreateTag {
        repo_id: RepoId,
        name: String,
        target: String,
        message: Option<String>,
        annotated: bool,
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
        remote_url_policy: RemoteUrlPolicy,
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
        remote_url_policy: RemoteUrlPolicy,
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
}

macro_rules! match_git_auth {
    ($effect:expr, $auth:ident => $then:expr) => {
        match $effect {
            RepoCommandEffect::RunLargeFileCommand { auth: $auth, .. }
            | RepoCommandEffect::CherryPickCommit { auth: $auth, .. }
            | RepoCommandEffect::RevertCommit { auth: $auth, .. }
            | RepoCommandEffect::ApplyFileChange { auth: $auth, .. }
            | RepoCommandEffect::AddSubmodule { auth: $auth, .. }
            | RepoCommandEffect::UpdateSubmodules { auth: $auth, .. }
            | RepoCommandEffect::LoadSubmodule { auth: $auth, .. }
            | RepoCommandEffect::SafePushAfterCommit { auth: $auth, .. }
            | RepoCommandEffect::FetchAll { auth: $auth, .. }
            | RepoCommandEffect::FetchRefspecs { auth: $auth, .. }
            | RepoCommandEffect::Pull { auth: $auth, .. }
            | RepoCommandEffect::PullBranch { auth: $auth, .. }
            | RepoCommandEffect::PushWithTags { auth: $auth, .. }
            | RepoCommandEffect::Push { auth: $auth, .. }
            | RepoCommandEffect::PushAfterCommit { auth: $auth, .. }
            | RepoCommandEffect::ForcePush { auth: $auth, .. }
            | RepoCommandEffect::ForcePushWithLease { auth: $auth, .. }
            | RepoCommandEffect::PushSetUpstream { auth: $auth, .. }
            | RepoCommandEffect::DeleteRemoteBranch { auth: $auth, .. }
            | RepoCommandEffect::DeleteRemoteBranches { auth: $auth, .. }
            | RepoCommandEffect::RebaseContinue { auth: $auth, .. }
            | RepoCommandEffect::RewordCommit { auth: $auth, .. }
            | RepoCommandEffect::PushTag { auth: $auth, .. }
            | RepoCommandEffect::DeleteRemoteTag { auth: $auth, .. } => $then,
            RepoCommandEffect::SaveWorktreeFile { .. }
            | RepoCommandEffect::AppendGitignorePatterns { .. }
            | RepoCommandEffect::AppendGitattributesRule { .. }
            | RepoCommandEffect::ExportPatch { .. }
            | RepoCommandEffect::ApplyPatch { .. }
            | RepoCommandEffect::AddWorktree { .. }
            | RepoCommandEffect::RemoveWorktree { .. }
            | RepoCommandEffect::ForceRemoveWorktree { .. }
            | RepoCommandEffect::CheckSubmoduleAddTrust { .. }
            | RepoCommandEffect::CheckSubmoduleUpdateTrust { .. }
            | RepoCommandEffect::CheckSubmoduleLoadTrust { .. }
            | RepoCommandEffect::ChangeSubmodulePointer { .. }
            | RepoCommandEffect::RemoveSubmodule { .. }
            | RepoCommandEffect::StageHunk { .. }
            | RepoCommandEffect::ApplyPreparedDiffPatch { .. }
            | RepoCommandEffect::UnstageHunk { .. }
            | RepoCommandEffect::ApplyWorktreePatch { .. }
            | RepoCommandEffect::PruneMergedBranches { .. }
            | RepoCommandEffect::PruneLocalTags { .. }
            | RepoCommandEffect::CheckRepoMaintenance { .. }
            | RepoCommandEffect::RunMaintenance { .. }
            | RepoCommandEffect::MergeRef { .. }
            | RepoCommandEffect::SquashRef { .. }
            | RepoCommandEffect::SetUpstreamBranch { .. }
            | RepoCommandEffect::UnsetUpstreamBranch { .. }
            | RepoCommandEffect::Reset { .. }
            | RepoCommandEffect::SquashCommits { .. }
            | RepoCommandEffect::Rebase { .. }
            | RepoCommandEffect::RebaseAbort { .. }
            | RepoCommandEffect::InteractiveRebase { .. }
            | RepoCommandEffect::InteractiveCherryPick { .. }
            | RepoCommandEffect::MergeAbort { .. }
            | RepoCommandEffect::CreateTag { .. }
            | RepoCommandEffect::DeleteTag { .. }
            | RepoCommandEffect::AddRemote { .. }
            | RepoCommandEffect::RemoveRemote { .. }
            | RepoCommandEffect::SetRemoteUrl { .. }
            | RepoCommandEffect::CheckoutConflictSide { .. }
            | RepoCommandEffect::AcceptConflictDeletion { .. }
            | RepoCommandEffect::CheckoutConflictBase { .. }
            | RepoCommandEffect::LaunchMergetool { .. } => None,
        }
    };
}

impl RepoCommandEffect {
    #[cfg(test)]
    pub(crate) fn git_auth(&self) -> Option<&StagedGitAuth> {
        match_git_auth!(self, auth => auth.as_ref())
    }

    pub(crate) fn git_auth_slot(&mut self) -> Option<&mut Option<StagedGitAuth>> {
        match_git_auth!(self, auth => Some(auth))
    }
}

impl RepoCommandEffect {
    pub(crate) fn failed(self, error: gitcomet_core::error::Error) -> Option<InternalMsg> {
        match self {
            RepoCommandEffect::SaveWorktreeFile {
                repo_id,
                path,
                stage,
                ..
            } => Some(
                RepoCommandKind::SaveWorktreeFile { path, stage }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::AppendGitignorePatterns { repo_id, patterns } => Some(
                RepoCommandKind::AppendGitignorePatterns { patterns }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::RunLargeFileCommand {
                repo_id, command, ..
            } => Some(RepoCommandKind::LargeFile { command }.finished(repo_id, Err(error))),
            RepoCommandEffect::AppendGitattributesRule { repo_id, rule } => Some(
                RepoCommandKind::AppendGitattributesRule { rule }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::CherryPickCommit {
                repo_id,
                commit_id,
                commit,
                mainline,
                summary,
                ..
            } => Some(
                RepoCommandKind::CherryPick {
                    commit_id,
                    commit,
                    mainline,
                    summary,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::RevertCommit {
                repo_id,
                commit_id,
                commit,
                mainline,
                summary,
                ..
            } => Some(
                RepoCommandKind::Revert {
                    commit_id,
                    commit,
                    mainline,
                    summary,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::ApplyFileChange {
                repo_id,
                target,
                commit,
                commit_retry,
                ..
            } => Some(
                RepoCommandKind::ApplyFileChange {
                    target,
                    commit,
                    commit_retry,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::ExportPatch {
                repo_id,
                commit_id,
                dest,
            } => {
                Some(RepoCommandKind::ExportPatch { commit_id, dest }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::ApplyPatch { repo_id, patch } => {
                Some(RepoCommandKind::ApplyPatch { patch }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::AddWorktree {
                repo_id,
                path,
                checkout,
                open_after,
            } => Some(
                RepoCommandKind::AddWorktree {
                    path,
                    checkout,
                    open_after,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::RemoveWorktree { repo_id, path } => {
                Some(RepoCommandKind::RemoveWorktree { path }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::ForceRemoveWorktree { repo_id, path } => {
                Some(RepoCommandKind::ForceRemoveWorktree { path }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::CheckSubmoduleAddTrust {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                ..
            } => Some(InternalMsg::SubmoduleAddTrustChecked {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                result: Err(error),
            }),
            RepoCommandEffect::CheckSubmoduleUpdateTrust { repo_id, .. } => {
                Some(InternalMsg::SubmoduleUpdateTrustChecked {
                    repo_id,
                    result: Err(error),
                })
            }
            RepoCommandEffect::CheckSubmoduleLoadTrust { repo_id, path, .. } => {
                Some(InternalMsg::SubmoduleLoadTrustChecked {
                    repo_id,
                    path,
                    result: Err(error),
                })
            }
            RepoCommandEffect::AddSubmodule {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                approved_sources,
                ..
            } => Some(
                RepoCommandKind::AddSubmodule {
                    url,
                    path,
                    branch,
                    name,
                    force,
                    approved_sources,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::UpdateSubmodules {
                repo_id,
                approved_sources,
                ..
            } => Some(
                RepoCommandKind::UpdateSubmodules { approved_sources }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::LoadSubmodule {
                repo_id,
                path,
                approved_sources,
                ..
            } => Some(
                RepoCommandKind::LoadSubmodule {
                    path,
                    approved_sources,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::ChangeSubmodulePointer {
                repo_id,
                path,
                reference,
            } => Some(
                RepoCommandKind::ChangeSubmodulePointer { path, reference }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::RemoveSubmodule { repo_id, path } => {
                Some(RepoCommandKind::RemoveSubmodule { path }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::StageHunk { repo_id, .. } => {
                Some(RepoCommandKind::StageHunk.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::ApplyPreparedDiffPatch {
                repo_id, prepared, ..
            } => {
                use gitcomet_core::patch_action::PatchOperation;
                let command = match prepared.operation() {
                    PatchOperation::Stage => RepoCommandKind::StageHunk,
                    PatchOperation::Unstage => RepoCommandKind::UnstageHunk,
                    PatchOperation::Discard => {
                        RepoCommandKind::ApplyWorktreePatch { reverse: true }
                    }
                };
                Some(command.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::UnstageHunk { repo_id, .. } => {
                Some(RepoCommandKind::UnstageHunk.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::ApplyWorktreePatch {
                repo_id, reverse, ..
            } => {
                Some(RepoCommandKind::ApplyWorktreePatch { reverse }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::SafePushAfterCommit {
                repo_id, context, ..
            } => Some(InternalMsg::SafePushAfterCommitFinished {
                repo_id,
                context,
                auth: None,
                result: Err(error),
            }),
            RepoCommandEffect::FetchAll { repo_id, .. } => {
                Some(RepoCommandKind::FetchAll.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::FetchRefspecs {
                repo_id,
                remote,
                refspecs,
                ..
            } => Some(
                RepoCommandKind::FetchRefspecs { remote, refspecs }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::PruneMergedBranches { repo_id } => {
                Some(RepoCommandKind::PruneMergedBranches.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::PruneLocalTags { repo_id } => {
                Some(RepoCommandKind::PruneLocalTags.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::CheckRepoMaintenance { .. } => None,
            RepoCommandEffect::RunMaintenance { repo_id } => {
                Some(RepoCommandKind::RunMaintenance.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::Pull { repo_id, mode, .. } => {
                Some(RepoCommandKind::Pull { mode }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::PullBranch {
                repo_id,
                remote,
                branch,
                ..
            } => Some(RepoCommandKind::PullBranch { remote, branch }.finished(repo_id, Err(error))),
            RepoCommandEffect::MergeRef { repo_id, reference } => {
                Some(RepoCommandKind::MergeRef { reference }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::SquashRef { repo_id, reference } => {
                Some(RepoCommandKind::SquashRef { reference }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::PushWithTags {
                repo_id, request, ..
            } => Some(RepoCommandKind::PushWithTags { request }.finished(repo_id, Err(error))),
            RepoCommandEffect::Push { repo_id, .. } => {
                Some(RepoCommandKind::Push.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::PushAfterCommit {
                repo_id,
                target,
                set_upstream,
                ..
            } => Some(
                RepoCommandKind::PushAfterCommit {
                    target,
                    set_upstream,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::ForcePush { repo_id, .. } => {
                Some(RepoCommandKind::ForcePush.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::ForcePushWithLease { repo_id, lease, .. } => {
                Some(RepoCommandKind::ForcePushWithLease { lease }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::PushSetUpstream {
                repo_id,
                remote,
                branch,
                ..
            } => Some(
                RepoCommandKind::PushSetUpstream { remote, branch }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::SetUpstreamBranch {
                repo_id,
                branch,
                upstream,
            } => Some(
                RepoCommandKind::SetUpstreamBranch { branch, upstream }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::UnsetUpstreamBranch { repo_id, branch } => {
                Some(RepoCommandKind::UnsetUpstreamBranch { branch }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::DeleteRemoteBranch {
                repo_id,
                remote,
                branch,
                ..
            } => Some(
                RepoCommandKind::DeleteRemoteBranch { remote, branch }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::DeleteRemoteBranches {
                repo_id,
                remote,
                branches,
                ..
            } => Some(
                RepoCommandKind::DeleteRemoteBranches { remote, branches }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::Reset {
                repo_id,
                target,
                mode,
            } => Some(RepoCommandKind::Reset { mode, target }.finished(repo_id, Err(error))),
            RepoCommandEffect::SquashCommits {
                repo_id,
                oldest,
                expected_head,
                message,
                count,
            } => Some(
                RepoCommandKind::SquashCommits {
                    oldest,
                    expected_head,
                    message,
                    count,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::RewordCommit {
                repo_id,
                request_id,
                commit_id,
                message,
                resume,
                ..
            } => Some(
                RepoCommandKind::RewordCommit {
                    request_id,
                    commit_id,
                    message,
                    rebase_paused: resume,
                    rewritten_id: None,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::Rebase { repo_id, onto } => {
                Some(RepoCommandKind::Rebase { onto }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::RebaseContinue { repo_id, .. } => {
                Some(RepoCommandKind::RebaseContinue.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::RebaseAbort { repo_id } => {
                Some(RepoCommandKind::RebaseAbort.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::InteractiveRebase {
                repo_id,
                base,
                entries: _,
                interactive,
            } => Some(
                RepoCommandKind::InteractiveRebase { base, interactive }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::InteractiveCherryPick {
                repo_id,
                entries,
                commit,
            } => Some(
                RepoCommandKind::InteractiveCherryPick { entries, commit }
                    .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::MergeAbort { repo_id } => {
                Some(RepoCommandKind::MergeAbort.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::CreateTag {
                repo_id,
                name,
                target,
                message,
                annotated,
            } => Some(
                RepoCommandKind::CreateTag {
                    name,
                    target,
                    message,
                    annotated,
                }
                .finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::DeleteTag { repo_id, name } => {
                Some(RepoCommandKind::DeleteTag { name }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::PushTag {
                repo_id,
                remote,
                name,
                ..
            } => Some(RepoCommandKind::PushTag { remote, name }.finished(repo_id, Err(error))),
            RepoCommandEffect::DeleteRemoteTag {
                repo_id,
                remote,
                name,
                ..
            } => Some(
                RepoCommandKind::DeleteRemoteTag { remote, name }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::AddRemote {
                repo_id, name, url, ..
            } => Some(RepoCommandKind::AddRemote { name, url }.finished(repo_id, Err(error))),
            RepoCommandEffect::RemoveRemote { repo_id, name } => {
                Some(RepoCommandKind::RemoveRemote { name }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::SetRemoteUrl {
                repo_id,
                name,
                url,
                kind,
                ..
            } => Some(
                RepoCommandKind::SetRemoteUrl { name, url, kind }.finished(repo_id, Err(error)),
            ),
            RepoCommandEffect::CheckoutConflictSide {
                repo_id,
                path,
                side,
            } => {
                Some(RepoCommandKind::CheckoutConflict { path, side }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::AcceptConflictDeletion { repo_id, path } => {
                Some(RepoCommandKind::AcceptConflictDeletion { path }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::CheckoutConflictBase { repo_id, path } => {
                Some(RepoCommandKind::CheckoutConflictBase { path }.finished(repo_id, Err(error)))
            }
            RepoCommandEffect::LaunchMergetool { repo_id, path } => {
                Some(RepoCommandKind::LaunchMergetool { path }.finished(repo_id, Err(error)))
            }
        }
    }
}
