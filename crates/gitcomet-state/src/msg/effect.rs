use crate::model::{ConflictFileLoadMode, RepoId};
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::domain::*;
use gitcomet_core::remote_url::RemoteUrlPolicy;
use gitcomet_core::services::CheckoutRemoteBranchMode;
use std::path::PathBuf;

use super::RepoPathList;

macro_rules! match_git_auth {
    ($effect:expr, $auth:ident => $then:expr, $work:ident => $command:expr) => {
        match $effect {
            Effect::RepoCommand($work) => $command,
            Effect::CloneRepo { auth: $auth, .. }
            | Effect::Commit { auth: $auth, .. }
            | Effect::CommitAmend { auth: $auth, .. } => $then,
            _ => None,
        }
    };
}

impl Effect {
    #[cfg(test)]
    pub(crate) fn git_auth(&self) -> Option<&StagedGitAuth> {
        match_git_auth!(self, auth => auth.as_ref(), work => work.git_auth())
    }

    pub(crate) fn git_auth_slot(&mut self) -> Option<&mut Option<StagedGitAuth>> {
        match_git_auth!(self, auth => Some(auth), work => work.git_auth_slot())
    }
}

#[derive(Clone, Debug, strum::IntoStaticStr)]
pub enum Effect {
    Persistence(super::PersistenceEffect),
    Control(super::ControlEffect),
    Status(super::StatusEffect),
    References(super::ReferenceEffect),
    RepoCommand(super::RepoCommandEffect),
    ValidateAuthPrompt {
        generation: u64,
        challenge: gitcomet_core::auth::AuthChallenge,
        auth: StagedGitAuth,
        cancellation: gitcomet_core::services::CancellationToken,
    },
    Filesystem(gitcomet_core::filesystem::Request),
    IndexedHistory(crate::indexed_history::IndexedHistoryEffect),
    DiffSession(crate::diff_session::DiffSessionEffect),
    HistoryAuthors(crate::history_authors::HistoryAuthorsEffect),
    HistoryFind(crate::history_find::HistoryFindEffect),

    OpenRepo {
        repo_id: RepoId,
        path: PathBuf,
    },

    LoadLog {
        repo_id: RepoId,
        /// Identifies this walk, so its replies can be told from those of a
        /// walk a newer request superseded. See [`crate::model::LogLoadSeq`].
        seq: crate::model::LogLoadSeq,
        scope: LogScope,
        /// Case-insensitive author filter, or `None` for all authors.
        author: Option<String>,
        limit: usize,
        cursor: Option<LogCursor>,
    },

    LoadStashes {
        repo_id: RepoId,
        limit: usize,
    },
    LoadReflog {
        repo_id: RepoId,
        limit: usize,
    },
    LoadRecentCommitMessages {
        repo_id: RepoId,
        limit: usize,
        request_rev: u64,
    },
    /// One page of a file's history. `cursor: None` is the bounded first
    /// page; a cursor page resumes after it and is served from the backend's
    /// cached full follow walk, so one such request can ask for everything.
    LoadFileHistory {
        request: crate::model::FileHistoryRequestId,
        repo_id: RepoId,
        path: PathBuf,
        limit: usize,
        cursor: Option<LogCursor>,
    },
    LoadBlame {
        repo_id: RepoId,
        path: PathBuf,
        source: gitcomet_core::domain::BlameSource,
    },
    LoadWorktrees {
        repo_id: RepoId,
    },
    LoadWorktreeDirty {
        repo_id: RepoId,
        workdir: PathBuf,
        scope: crate::model::WorktreeDirtyScope,
        /// Worktree whose changed-file lists the scan should carry back; every
        /// other worktree reports counts alone. `None` while no worktree row is
        /// selected. See [`gitcomet_core::domain::WorktreeDirtySummary`].
        files_for: Option<PathBuf>,
    },

    LoadSubmodules {
        repo_id: RepoId,
    },
    LoadLargeFileSupport {
        repo_id: RepoId,
    },
    LoadLfsLocks {
        repo_id: RepoId,
    },
    LoadAnnexWhereis {
        repo_id: RepoId,
        key: String,
    },
    LoadAnnexUnused {
        repo_id: RepoId,
    },
    LoadFileBrowser {
        repo_id: RepoId,
        source: FileSource,
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
    /// Verify the signatures of `commit_ids`. Batched because verification
    /// shells out to `git`; the backend drops unsigned commits for free.
    VerifyCommitSignatures {
        repo_id: RepoId,
        epoch: u64,
        batch: u64,
        cancellation: gitcomet_core::services::CancellationToken,
        commit_ids: std::sync::Arc<[CommitId]>,
        /// Only signatures in these formats are checked: the others have no
        /// verifier installed.
        formats: gitcomet_core::domain::SignatureFormats,
    },
    LoadHoverCommitMessage {
        repo_id: RepoId,
        commit_id: CommitId,
    },
    /// Resolve a possibly abbreviated commit reference and load its details in
    /// one call, so a reveal can show the commit before the log reaches it.
    ResolveCommitForReveal {
        repo_id: RepoId,
        reference: CommitId,
    },
    /// Resolve a commit reference for the Reveal Commit dialog's preview row.
    /// Lighter than `ResolveCommitForReveal` — no parent diff — because it runs
    /// while the user is still typing.
    ResolveCommitLookup {
        repo_id: RepoId,
        reference: CommitId,
        purpose: crate::model::CommitLookupPurpose,
        /// Echoed back on the reply so a completion that lost a race against a
        /// newer lookup can be dropped. See `CommitLookup::request`.
        request: u64,
    },
    LoadRangeFiles {
        repo_id: RepoId,
        from: CommitId,
        /// `None` lists files between `from` and the working tree.
        to: Option<CommitId>,
        options: gitcomet_core::services::ComparisonOptions,
        /// Echoed back on the reply so a completion that lost a race against a
        /// newer load can be dropped. See `HistoryState::range_files_request`.
        request: u64,
    },
    LoadSquashMessagePreview {
        repo_id: RepoId,
        oldest: CommitId,
        head: CommitId,
    },
    LoadSquashRebaseSetup {
        repo_id: RepoId,
        base: CommitId,
        /// The repo HEAD the plan was validated against. Re-checked once the
        /// live `base..HEAD` list loads, so a HEAD move during the async gap
        /// cancels the squash instead of rewriting an unintended range.
        actual_head: CommitId,
        selected_ids: Vec<CommitId>,
        reword_id: CommitId,
        message: String,
        count: usize,
    },
    OpenFileAtCommitParent {
        repo_id: RepoId,
        commit_id: CommitId,
        path: PathBuf,
    },
    OpenFileAtCommit {
        repo_id: RepoId,
        commit_id: CommitId,
        path: PathBuf,
        content_preview: bool,
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
    LoadSubmoduleSummary {
        repo_id: RepoId,
        target: DiffTarget,
    },
    LoadInlineSubmoduleSelectedDiff {
        repo_id: RepoId,
        inline_rev: u64,
    },
    LoadInlineSubmoduleSelectedDiffFile {
        repo_id: RepoId,
        inline_rev: u64,
    },
    LoadInlineSubmoduleSelectedDiffFileImage {
        repo_id: RepoId,
        inline_rev: u64,
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
        load_submodule_summary: bool,
        load_file_image: bool,
    },
    LoadSelectedConflictFile {
        request: crate::model::ConflictLoadRequestId,
        repo_id: RepoId,
        mode: ConflictFileLoadMode,
    },
    LoadConflictFile {
        request: crate::model::ConflictLoadRequestId,
        repo_id: RepoId,
        path: PathBuf,
        mode: ConflictFileLoadMode,
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
        mode: CheckoutRemoteBranchMode,
    },
    CheckoutCommit {
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
        /// Reset the branch to `target` first when a branch with this name
        /// already exists, instead of failing with "already exists".
        force: bool,
    },
    RenameBranch {
        repo_id: RepoId,
        old_name: String,
        new_name: String,
        force: bool,
    },
    DeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    ForceDeleteBranch {
        repo_id: RepoId,
        name: String,
    },
    DeleteBranches {
        repo_id: RepoId,
        names: Vec<String>,
        force: bool,
    },
    CloneRepo {
        url: String,
        dest: PathBuf,
        remote_url_policy: RemoteUrlPolicy,
        auth: Option<StagedGitAuth>,
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
        identity: Option<gitcomet_core::commit_identity::CommitIdentity>,
        auth: Option<StagedGitAuth>,
    },
    CommitAmend {
        repo_id: RepoId,
        message: String,
        identity: Option<gitcomet_core::commit_identity::CommitIdentity>,
        auth: Option<StagedGitAuth>,
    },

    PreviewTagPush {
        repo_id: RepoId,
        request: gitcomet_core::tag_push::TagPushRequest,
        cancellation: gitcomet_core::services::CancellationToken,
        generation: u64,
    },

    LoadInteractiveRebaseSetup {
        repo_id: RepoId,
        base: String,
    },
    // entries held here so the effect dispatcher can pass them to the scheduler
    /// Load the full `%B` messages of the commits selected for an
    /// interactive cherry-pick: the log page only carries subjects, and a
    /// reword edited from a subject-only seed would silently drop the body.
    LoadInteractiveCherryPickMessages {
        repo_id: RepoId,
        ids: Vec<String>,
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

impl Effect {
    pub(crate) fn requires_available_git(&self) -> bool {
        match self {
            Self::Filesystem(_)
            | Self::Persistence(_)
            | Self::Control(_)
            | Self::ValidateAuthPrompt { .. } => false,
            Self::Status(_)
            | Self::References(_)
            | Self::RepoCommand(_)
            | Self::IndexedHistory(_)
            | Self::DiffSession(_)
            | Self::HistoryAuthors(_)
            | Self::HistoryFind(_)
            | Self::OpenRepo { .. }
            | Self::LoadLog { .. }
            | Self::LoadStashes { .. }
            | Self::LoadReflog { .. }
            | Self::LoadRecentCommitMessages { .. }
            | Self::LoadFileHistory { .. }
            | Self::LoadBlame { .. }
            | Self::LoadWorktrees { .. }
            | Self::LoadWorktreeDirty { .. }
            | Self::LoadSubmodules { .. }
            | Self::LoadLargeFileSupport { .. }
            | Self::LoadLfsLocks { .. }
            | Self::LoadAnnexWhereis { .. }
            | Self::LoadAnnexUnused { .. }
            | Self::LoadFileBrowser { .. }
            | Self::LoadRebaseAndMergeState { .. }
            | Self::LoadRebaseState { .. }
            | Self::LoadMergeCommitMessage { .. }
            | Self::LoadCommitDetails { .. }
            | Self::VerifyCommitSignatures { .. }
            | Self::LoadHoverCommitMessage { .. }
            | Self::ResolveCommitForReveal { .. }
            | Self::ResolveCommitLookup { .. }
            | Self::LoadRangeFiles { .. }
            | Self::LoadSquashMessagePreview { .. }
            | Self::LoadSquashRebaseSetup { .. }
            | Self::OpenFileAtCommitParent { .. }
            | Self::OpenFileAtCommit { .. }
            | Self::LoadDiff { .. }
            | Self::LoadDiffFile { .. }
            | Self::LoadDiffPreviewTextFile { .. }
            | Self::LoadSubmoduleSummary { .. }
            | Self::LoadInlineSubmoduleSelectedDiff { .. }
            | Self::LoadInlineSubmoduleSelectedDiffFile { .. }
            | Self::LoadInlineSubmoduleSelectedDiffFileImage { .. }
            | Self::LoadDiffFileImage { .. }
            | Self::LoadSelectedDiff { .. }
            | Self::LoadSelectedConflictFile { .. }
            | Self::LoadConflictFile { .. }
            | Self::CheckoutBranch { .. }
            | Self::CheckoutRemoteBranch { .. }
            | Self::CheckoutCommit { .. }
            | Self::CreateBranch { .. }
            | Self::CreateBranchAndCheckout { .. }
            | Self::RenameBranch { .. }
            | Self::DeleteBranch { .. }
            | Self::ForceDeleteBranch { .. }
            | Self::DeleteBranches { .. }
            | Self::CloneRepo { .. }
            | Self::StagePath { .. }
            | Self::StagePaths { .. }
            | Self::UnstagePath { .. }
            | Self::UnstagePaths { .. }
            | Self::DiscardWorktreeChangesPath { .. }
            | Self::DiscardWorktreeChangesPaths { .. }
            | Self::Commit { .. }
            | Self::CommitAmend { .. }
            | Self::PreviewTagPush { .. }
            | Self::LoadInteractiveRebaseSetup { .. }
            | Self::LoadInteractiveCherryPickMessages { .. }
            | Self::Stash { .. }
            | Self::ApplyStash { .. }
            | Self::PopStash { .. }
            | Self::DropStash { .. } => true,
        }
    }
}
