mod actions_emit_effects;
mod conflict_interactions;
mod diff_selection;
mod effects;
mod external_and_history;
mod repo_management;
mod util;

use crate::model::{
    AppState, AuthPromptState, AuthRetryOperation, BannerErrorState, PendingCommitRetry, RepoId,
    SubmoduleAddProgressState, SubmoduleTrustPromptOperation, SubmoduleTrustPromptState,
};
use crate::msg::{ConflictRegionChoice, Effect, Msg, RepoCommandKind, RepoPath, RepoPathList};
use crate::store::repo_load_trace;
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::services::{GitRepository, SafePushAfterCommitContext};
use rustc_hash::FxHashMap as HashMap;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

#[cfg(feature = "benchmarks")]
pub(crate) use diff_selection::SelectDiffEffects;
pub(crate) use repo_management::{ReorderRepoTabsEffects, SetActiveRepoEffects};

pub(crate) const SINGLE_PATH_ACTION_INLINE_EFFECT_CAPACITY: usize = 1;
pub(crate) type SinglePathActionEffects =
    SmallVec<[Effect; SINGLE_PATH_ACTION_INLINE_EFFECT_CAPACITY]>;
pub(crate) type BatchPathActionEffects =
    SmallVec<[Effect; SINGLE_PATH_ACTION_INLINE_EFFECT_CAPACITY]>;

#[cfg(test)]
pub(super) fn normalize_repo_path(path: std::path::PathBuf) -> std::path::PathBuf {
    util::normalize_repo_path(path)
}

fn normalize_repo_relative_path(
    repo_workdir: &std::path::Path,
    path: std::path::PathBuf,
) -> std::path::PathBuf {
    let path = if path.is_relative() {
        repo_workdir.join(path)
    } else {
        path
    };
    util::canonicalize_path(path)
}

#[inline]
fn begin_local_action(state: &mut AppState, repo_id: RepoId) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
}

fn begin_commit_action(state: &mut AppState, repo_id: RepoId) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_add(1);
        repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_add(1);
        repo_state.pending_force_push_lease = None;
        repo_state.bump_ops_rev();
    }
}

fn begin_head_changing_local_action(state: &mut AppState, repo_id: RepoId) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_add(1);
        repo_state.clear_head_dependent_cached_state();
        repo_state.bump_ops_rev();
    }
}

fn start_submodule_add_progress(
    state: &mut AppState,
    repo_id: RepoId,
    url: &str,
    path: &std::path::Path,
) {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.submodule_add_in_flight = Some(SubmoduleAddProgressState {
            url: url.to_string(),
            path: path.to_path_buf(),
        });
    }
}

pub(crate) fn msg_requires_available_git(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::OpenRepo(_)
            | Msg::RestoreSession { .. }
            | Msg::ReloadRepo { .. }
            | Msg::RepoActivated { .. }
            | Msg::RepoExternallyChanged { .. }
            | Msg::SetHistoryScope { .. }
            | Msg::LoadMoreHistory { .. }
            | Msg::SelectCommit { .. }
            | Msg::SelectDiff { .. }
            | Msg::SelectConflictDiff { .. }
            | Msg::LoadStashes { .. }
            | Msg::LoadConflictFile { .. }
            | Msg::LoadReflog { .. }
            | Msg::LoadRecentCommitMessages { .. }
            | Msg::LoadFileHistory { .. }
            | Msg::LoadBlame { .. }
            | Msg::LoadWorktrees { .. }
            | Msg::LoadSubmodules { .. }
            | Msg::LoadSubmodule { .. }
            | Msg::LoadTags { .. }
            | Msg::LoadRemoteTags { .. }
            | Msg::RefreshBranches { .. }
            | Msg::LoadFileBrowser { .. }
            | Msg::OpenFileContent { .. }
            | Msg::OpenFileAtCommitParent { .. }
            | Msg::OpenFileAtCommit { .. }
            | Msg::BrowseRepositoryAtCommit { .. }
            | Msg::ResetBrowseToLive { .. }
            | Msg::ViewerNavBack { .. }
            | Msg::ViewerNavForward { .. }
            | Msg::GlobalNavBack { .. }
            | Msg::GlobalNavForward { .. }
            | Msg::StageHunk { .. }
            | Msg::UnstageHunk { .. }
            | Msg::ApplyWorktreePatch { .. }
            | Msg::CheckoutBranch { .. }
            | Msg::CheckoutRemoteBranch { .. }
            | Msg::CheckoutCommit { .. }
            | Msg::CherryPickCommit { .. }
            | Msg::RevertCommit { .. }
            | Msg::CreateBranch { .. }
            | Msg::CreateBranchAndCheckout { .. }
            | Msg::DeleteBranch { .. }
            | Msg::ForceDeleteBranch { .. }
            | Msg::CloneRepo { .. }
            | Msg::ExportPatch { .. }
            | Msg::ApplyPatch { .. }
            | Msg::AddWorktree { .. }
            | Msg::RemoveWorktree { .. }
            | Msg::ForceRemoveWorktree { .. }
            | Msg::AddSubmodule { .. }
            | Msg::UpdateSubmodules { .. }
            | Msg::ChangeSubmodulePointer { .. }
            | Msg::RemoveSubmodule { .. }
            | Msg::StagePath { .. }
            | Msg::StagePaths { .. }
            | Msg::UnstagePath { .. }
            | Msg::UnstagePaths { .. }
            | Msg::DiscardWorktreeChangesPath { .. }
            | Msg::DiscardWorktreeChangesPaths { .. }
            | Msg::SaveWorktreeFile { .. }
            | Msg::Commit { .. }
            | Msg::CommitAmend { .. }
            | Msg::SafePushAfterCommit { .. }
            | Msg::FetchAll { .. }
            | Msg::PruneMergedBranches { .. }
            | Msg::PruneLocalTags { .. }
            | Msg::Pull { .. }
            | Msg::PullBranch { .. }
            | Msg::MergeRef { .. }
            | Msg::SquashRef { .. }
            | Msg::Push { .. }
            | Msg::PushAfterCommit { .. }
            | Msg::ForcePush { .. }
            | Msg::ForcePushWithLease { .. }
            | Msg::PushSetUpstream { .. }
            | Msg::SetUpstreamBranch { .. }
            | Msg::UnsetUpstreamBranch { .. }
            | Msg::DeleteRemoteBranch { .. }
            | Msg::Reset { .. }
            | Msg::PrepareSquash { .. }
            | Msg::SquashCommits { .. }
            | Msg::Rebase { .. }
            | Msg::RebaseContinue { .. }
            | Msg::RebaseAbort { .. }
            | Msg::InteractiveRebase { .. }
            | Msg::MergeAbort { .. }
            | Msg::CreateTag { .. }
            | Msg::DeleteTag { .. }
            | Msg::PushTag { .. }
            | Msg::DeleteRemoteTag { .. }
            | Msg::AddRemote { .. }
            | Msg::RemoveRemote { .. }
            | Msg::SetRemoteUrl { .. }
            | Msg::CheckoutConflictSide { .. }
            | Msg::AcceptConflictDeletion { .. }
            | Msg::CheckoutConflictBase { .. }
            | Msg::LaunchMergetool { .. }
            | Msg::Stash { .. }
            | Msg::ApplyStash { .. }
            | Msg::PopStash { .. }
            | Msg::DropStash { .. }
    )
}

#[cfg(test)]
pub(super) fn push_diagnostic(
    repo_state: &mut crate::model::RepoState,
    kind: crate::model::DiagnosticKind,
    message: String,
) {
    util::push_diagnostic(repo_state, kind, message)
}

#[cfg(test)]
pub(super) fn handle_session_persist_result(
    state: &mut crate::model::AppState,
    repo_id: Option<crate::model::RepoId>,
    action: &'static str,
    result: std::io::Result<()>,
) {
    util::handle_session_persist_result(state, repo_id, action, result)
}

fn auth_prompt_for_repo_command(
    repo_id: RepoId,
    command: &RepoCommandKind,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = util::detect_auth_prompt_kind(error)?;
    let operation = AuthRetryOperation::RepoCommand {
        repo_id,
        command: command.clone(),
    };
    retry_msg_for_auth_operation(operation.clone())?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation,
    })
}

fn auth_prompt_for_safe_push_after_commit(
    repo_id: RepoId,
    context: SafePushAfterCommitContext,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = util::detect_auth_prompt_kind(error)?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation: AuthRetryOperation::SafePushAfterCommit { repo_id, context },
    })
}

fn auth_prompt_for_commit(
    repo_id: RepoId,
    pending: Option<PendingCommitRetry>,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = util::detect_auth_prompt_kind(error)?;
    let pending = pending?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: pending.message,
            amend: pending.amend,
            push_after_commit: pending.push_after_commit,
        },
    })
}

fn auth_prompt_for_clone(
    url: &str,
    dest: &std::path::Path,
    error: &gitcomet_core::error::Error,
) -> Option<AuthPromptState> {
    let kind = util::detect_auth_prompt_kind(error)?;
    Some(AuthPromptState {
        kind,
        reason: util::format_error_for_user(error),
        operation: AuthRetryOperation::Clone {
            url: url.to_string(),
            dest: dest.to_path_buf(),
        },
    })
}

fn retry_msg_for_auth_operation(operation: AuthRetryOperation) -> Option<Msg> {
    match operation {
        AuthRetryOperation::RepoCommand { repo_id, command } => {
            retry_msg_for_repo_command(repo_id, command)
        }
        AuthRetryOperation::SafePushAfterCommit { repo_id, context } => {
            Some(Msg::SafePushAfterCommit { repo_id, context })
        }
        AuthRetryOperation::Commit {
            repo_id,
            message,
            amend,
            push_after_commit,
        } => Some(if amend {
            Msg::CommitAmend {
                repo_id,
                message,
                push_after_commit,
            }
        } else {
            Msg::Commit {
                repo_id,
                message,
                push_after_commit,
            }
        }),
        AuthRetryOperation::Clone { url, dest } => Some(Msg::CloneRepo { url, dest }),
    }
}

fn clear_banner_error_for_auth_operation(state: &mut AppState, operation: &AuthRetryOperation) {
    match operation {
        AuthRetryOperation::RepoCommand { repo_id, .. }
        | AuthRetryOperation::SafePushAfterCommit { repo_id, .. }
        | AuthRetryOperation::Commit { repo_id, .. } => {
            util::clear_banner_error_for_repo(state, *repo_id);
        }
        AuthRetryOperation::Clone { .. } => clear_stale_clone_banner_error(state),
    }
}

fn clear_stale_clone_banner_error(state: &mut AppState) {
    if state
        .banner_error
        .as_ref()
        .is_some_and(|banner| banner.message.starts_with("Clone failed"))
    {
        state.banner_error = None;
    }
}

fn retry_msg_for_repo_command(repo_id: RepoId, command: RepoCommandKind) -> Option<Msg> {
    Some(match command {
        RepoCommandKind::FetchAll => Msg::FetchAll { repo_id },
        RepoCommandKind::PruneMergedBranches => Msg::PruneMergedBranches { repo_id },
        RepoCommandKind::PruneLocalTags => Msg::PruneLocalTags { repo_id },
        RepoCommandKind::Pull { mode } => Msg::Pull { repo_id, mode },
        RepoCommandKind::PullBranch { remote, branch } => Msg::PullBranch {
            repo_id,
            remote,
            branch,
        },
        RepoCommandKind::MergeRef { reference } => Msg::MergeRef { repo_id, reference },
        RepoCommandKind::SquashRef { reference } => Msg::SquashRef { repo_id, reference },
        RepoCommandKind::Push => Msg::Push { repo_id },
        RepoCommandKind::PushAfterCommit {
            target,
            set_upstream,
        } => Msg::PushAfterCommit {
            repo_id,
            target,
            set_upstream,
        },
        RepoCommandKind::ForcePush => Msg::ForcePush { repo_id },
        RepoCommandKind::ForcePushWithLease { lease } => Msg::ForcePushWithLease { repo_id, lease },
        RepoCommandKind::PushSetUpstream { remote, branch } => Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        },
        RepoCommandKind::SetUpstreamBranch { branch, upstream } => Msg::SetUpstreamBranch {
            repo_id,
            branch,
            upstream,
        },
        RepoCommandKind::UnsetUpstreamBranch { branch } => {
            Msg::UnsetUpstreamBranch { repo_id, branch }
        }
        RepoCommandKind::DeleteRemoteBranch { remote, branch } => Msg::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
        },
        RepoCommandKind::Reset { mode, target } => Msg::Reset {
            repo_id,
            target,
            mode,
        },
        RepoCommandKind::SquashCommits {
            oldest,
            expected_head,
            message,
            count,
        } => Msg::SquashCommits {
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        },
        RepoCommandKind::Rebase { onto } => Msg::Rebase { repo_id, onto },
        RepoCommandKind::RebaseContinue => Msg::RebaseContinue { repo_id },
        RepoCommandKind::RebaseAbort => Msg::RebaseAbort { repo_id },
        RepoCommandKind::MergeAbort => Msg::MergeAbort { repo_id },
        RepoCommandKind::CreateTag {
            name,
            target,
            message,
            annotated,
        } => Msg::CreateTag {
            repo_id,
            name,
            target,
            message,
            annotated,
        },
        RepoCommandKind::DeleteTag { name } => Msg::DeleteTag { repo_id, name },
        RepoCommandKind::PushTag { remote, name } => Msg::PushTag {
            repo_id,
            remote,
            name,
        },
        RepoCommandKind::DeleteRemoteTag { remote, name } => Msg::DeleteRemoteTag {
            repo_id,
            remote,
            name,
        },
        RepoCommandKind::AddRemote { name, url } => Msg::AddRemote { repo_id, name, url },
        RepoCommandKind::RemoveRemote { name } => Msg::RemoveRemote { repo_id, name },
        RepoCommandKind::SetRemoteUrl { name, url, kind } => Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        },
        RepoCommandKind::CheckoutConflict { path, side } => Msg::CheckoutConflictSide {
            repo_id,
            path,
            side,
        },
        RepoCommandKind::AcceptConflictDeletion { path } => {
            Msg::AcceptConflictDeletion { repo_id, path }
        }
        RepoCommandKind::CheckoutConflictBase { path } => {
            Msg::CheckoutConflictBase { repo_id, path }
        }
        RepoCommandKind::LaunchMergetool { path } => Msg::LaunchMergetool { repo_id, path },
        RepoCommandKind::ExportPatch { commit_id, dest } => Msg::ExportPatch {
            repo_id,
            commit_id,
            dest,
        },
        RepoCommandKind::ApplyPatch { patch } => Msg::ApplyPatch { repo_id, patch },
        RepoCommandKind::AddWorktree { path, reference } => Msg::AddWorktree {
            repo_id,
            path,
            reference,
        },
        RepoCommandKind::RemoveWorktree { path } => Msg::RemoveWorktree { repo_id, path },
        RepoCommandKind::ForceRemoveWorktree { path } => Msg::ForceRemoveWorktree { repo_id, path },
        RepoCommandKind::AddSubmodule {
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        } => Msg::AddSubmoduleTrusted {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        },
        RepoCommandKind::UpdateSubmodules { approved_sources } => Msg::UpdateSubmodulesTrusted {
            repo_id,
            approved_sources,
        },
        RepoCommandKind::LoadSubmodule {
            path,
            approved_sources,
        } => Msg::LoadSubmoduleTrusted {
            repo_id,
            path,
            approved_sources,
        },
        RepoCommandKind::ChangeSubmodulePointer { path, reference } => {
            Msg::ChangeSubmodulePointer {
                repo_id,
                path,
                reference,
            }
        }
        RepoCommandKind::RemoveSubmodule { path } => Msg::RemoveSubmodule { repo_id, path },
        // Not replayable because command metadata does not retain original content.
        RepoCommandKind::SaveWorktreeFile { .. }
        | RepoCommandKind::StageHunk
        | RepoCommandKind::UnstageHunk
        | RepoCommandKind::ApplyWorktreePatch { .. }
        | RepoCommandKind::InteractiveRebase { .. } => return None,
    })
}

fn attach_git_auth_to_effects(mut effects: Vec<Effect>, auth: StagedGitAuth) -> Vec<Effect> {
    let Some(first) = effects.first_mut() else {
        return effects;
    };

    match first {
        Effect::CloneRepo { auth: slot, .. }
        | Effect::AddSubmodule { auth: slot, .. }
        | Effect::UpdateSubmodules { auth: slot, .. }
        | Effect::LoadSubmodule { auth: slot, .. }
        | Effect::Commit { auth: slot, .. }
        | Effect::CommitAmend { auth: slot, .. }
        | Effect::SafePushAfterCommit { auth: slot, .. }
        | Effect::FetchAll { auth: slot, .. }
        | Effect::Pull { auth: slot, .. }
        | Effect::PullBranch { auth: slot, .. }
        | Effect::Push { auth: slot, .. }
        | Effect::PushAfterCommit { auth: slot, .. }
        | Effect::ForcePush { auth: slot, .. }
        | Effect::ForcePushWithLease { auth: slot, .. }
        | Effect::PushSetUpstream { auth: slot, .. }
        | Effect::DeleteRemoteBranch { auth: slot, .. }
        | Effect::PushTag { auth: slot, .. }
        | Effect::DeleteRemoteTag { auth: slot, .. } => {
            *slot = Some(auth);
        }
        _ => {}
    }

    effects
}

pub(crate) fn fill_set_active_repo_inline(
    state: &mut AppState,
    repo_id: RepoId,
    effects: &mut SetActiveRepoEffects,
) {
    repo_management::fill_set_active_repo_inline(state, repo_id, effects)
}

pub(crate) fn fill_reorder_repo_tabs_inline(
    state: &mut AppState,
    repo_id: RepoId,
    insert_before: Option<RepoId>,
    effects: &mut ReorderRepoTabsEffects,
) {
    repo_management::fill_reorder_repo_tabs_inline(state, repo_id, insert_before, effects)
}

// The only non-benchmark consumers of `fill_select_diff_inline` live inside
// the reducer submodule (via the unconditional `pub(super)` definition in
// `diff_selection.rs`). This public re-export exists solely for the benchmark
// helper in `store/mod.rs` so that the inline reduce path can be measured.
#[cfg(feature = "benchmarks")]
pub(crate) fn fill_select_diff_inline(
    state: &mut AppState,
    repo_id: RepoId,
    target: gitcomet_core::domain::DiffTarget,
    content_preview: bool,
    effects: &mut SelectDiffEffects,
) {
    diff_selection::fill_select_diff_inline(state, repo_id, target, content_preview, effects)
}

#[inline]
pub(crate) fn fill_stage_path_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    effects: &mut SinglePathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::StagePath { repo_id, path });
}

#[inline]
pub(crate) fn fill_stage_paths_inline(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    effects: &mut BatchPathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::StagePaths { repo_id, paths });
}

#[inline]
pub(crate) fn fill_unstage_path_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: std::path::PathBuf,
    effects: &mut SinglePathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::UnstagePath { repo_id, path });
}

#[inline]
pub(crate) fn fill_unstage_paths_inline(
    state: &mut AppState,
    repo_id: RepoId,
    paths: RepoPathList,
    effects: &mut BatchPathActionEffects,
) {
    begin_local_action(state, repo_id);
    effects.push(Effect::UnstagePaths { repo_id, paths });
}

#[inline]
pub(crate) fn set_conflict_region_choice_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
    region_index: usize,
    choice: ConflictRegionChoice,
) {
    conflict_interactions::set_region_choice_inline(state, repo_id, path, region_index, choice);
}

#[inline]
pub(crate) fn reset_conflict_resolutions_inline(
    state: &mut AppState,
    repo_id: RepoId,
    path: RepoPath,
) {
    conflict_interactions::reset_resolutions_inline(state, repo_id, path);
}

fn submit_auth_prompt(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    username: Option<String>,
    secret: String,
) -> Vec<Effect> {
    let Some(prompt) = state.auth_prompt.take() else {
        return Vec::new();
    };

    let username = username
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty());
    let auth = match util::prepare_staged_git_auth(prompt.kind, username.as_deref(), &secret) {
        Ok(auth) => auth,
        Err(err) => {
            state.auth_prompt = Some(prompt);
            return if let Some(repo_state) = state
                .active_repo
                .and_then(|repo_id| state.repos.iter_mut().find(|r| r.id == repo_id))
            {
                util::push_diagnostic(
                    repo_state,
                    crate::model::DiagnosticKind::Error,
                    util::format_error_for_user(&err),
                );
                Vec::new()
            } else {
                Vec::new()
            };
        }
    };

    clear_banner_error_for_auth_operation(state, &prompt.operation);

    match retry_msg_for_auth_operation(prompt.operation) {
        Some(msg) => attach_git_auth_to_effects(reduce(repos, id_alloc, state, msg), auth),
        None => Vec::new(),
    }
}

pub(super) fn reduce(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    msg: Msg,
) -> Vec<Effect> {
    let reconcile = !matches!(
        msg,
        Msg::GlobalNavBack { .. } | Msg::GlobalNavForward { .. }
    );
    let push = is_view_navigation(&msg);

    if reconcile {
        reconcile_active_nav_history(state, false);
    }

    let effects = reduce_inner(repos, id_alloc, state, msg);

    if reconcile {
        reconcile_active_nav_history(state, push);
    }

    effects
}

/// Whether `msg` is a user-initiated navigation that should create a new global
/// back/forward step (as opposed to a background change folded into the current
/// step). `GlobalNav*` replays are handled separately and never reach here as a
/// "push".
fn is_view_navigation(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::SelectDiff { .. }
            | Msg::SelectConflictDiff { .. }
            | Msg::SelectCommit { .. }
            | Msg::OpenFileContent { .. }
            | Msg::OpenFileAtCommit { .. }
            | Msg::BrowseRepositoryAtCommit { .. }
            | Msg::ResetBrowseToLive { .. }
            | Msg::OpenInlineSubmoduleDiff { .. }
            | Msg::SelectInlineSubmoduleDiff { .. }
    )
}

/// Sync the active repo's global navigation history against the current
/// main-view snapshot. See [`crate::model::NavStack::reconcile`].
fn reconcile_active_nav_history(state: &mut AppState, push: bool) {
    let Some(repo_id) = state.active_repo else {
        return;
    };
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return;
    };
    // Hot path: most messages don't move the main view, so the snapshot still
    // matches the current entry and `reconcile` would no-op. Compare by borrow
    // first and bail before cloning a `MainViewSnapshot` (which owns a `PathBuf`)
    // — this runs twice per dispatched message.
    let cursor = repo.nav_history.cursor;
    if let Some(current) = repo.nav_history.entries.get(cursor)
        && repo.main_view_snapshot_matches(current)
    {
        return;
    }
    let cur = repo.main_view_snapshot();
    repo.nav_history.reconcile(cur, push);
}

fn reduce_inner(
    repos: &mut HashMap<RepoId, Arc<dyn GitRepository>>,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    msg: Msg,
) -> Vec<Effect> {
    if msg_requires_available_git(&msg) && !state.git_runtime.is_available() {
        return Vec::new();
    }

    match msg {
        Msg::OpenRepo(path) => repo_management::open_repo(id_alloc, state, path),
        Msg::RestoreSession {
            open_repos,
            active_repo,
        } => repo_management::restore_session(repos, id_alloc, state, open_repos, active_repo),
        Msg::CloseRepo { repo_id } => repo_management::close_repo(repos, state, repo_id),
        Msg::CloseRepos {
            repo_ids,
            activate_after,
        } => repo_management::close_repos(repos, state, repo_ids, activate_after),
        Msg::ShowBannerError { repo_id, message } => {
            if !message.trim().is_empty() {
                state.banner_error = Some(BannerErrorState { repo_id, message });
            }
            Vec::new()
        }
        Msg::DismissBannerError => {
            state.banner_error = None;
            Vec::new()
        }
        Msg::DismissRepoError { repo_id } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.last_error = None;
            }
            util::clear_banner_error_for_repo(state, repo_id);
            Vec::new()
        }
        Msg::SubmitAuthPrompt { username, secret } => {
            submit_auth_prompt(repos, id_alloc, state, username, secret)
        }
        Msg::CancelAuthPrompt => {
            state.auth_prompt = None;
            util::clear_staged_git_auth_env();
            Vec::new()
        }
        Msg::SetGitRuntimeState(runtime) => {
            state.git_runtime = runtime;
            Vec::new()
        }
        Msg::SetGitLogSettings {
            show_history_tags,
            tag_fetch_mode,
        } => {
            state.git_log_settings.show_history_tags = show_history_tags;
            state.git_log_settings.tag_fetch_mode = tag_fetch_mode;
            Vec::new()
        }
        Msg::SetDefaultTagType(tag_type) => {
            state.default_tag_type = tag_type;
            Vec::new()
        }
        Msg::SetActiveRepo { repo_id } => repo_management::set_active_repo(state, repo_id),
        Msg::ReorderRepoTabs {
            repo_id,
            insert_before,
        } => repo_management::reorder_repo_tabs(state, repo_id, insert_before),
        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
            repo_id,
            action,
            error,
        }) => {
            util::handle_session_persist_result(
                state,
                repo_id,
                action,
                Err(std::io::Error::other(error)),
            );
            Vec::new()
        }
        Msg::ReloadRepo { repo_id } => external_and_history::reload_repo(state, repo_id),
        Msg::RepoActivated { .. } => Vec::new(),
        Msg::RepoExternallyChanged { repo_id, change } => {
            external_and_history::repo_externally_changed(state, repo_id, change)
        }
        Msg::RepoWatchDegraded { repo_id: _, reason } => {
            let message = match reason {
                crate::msg::RepoWatchDegradedReason::TooManyFolders { dir_count } => format!(
                    "This repository has {dir_count} folders — live file watching is disabled to \
                     stay within system limits. Changes refresh when the window regains focus. Add \
                     build/output dirs to .gitignore or raise fs.inotify.max_user_watches to \
                     re-enable."
                ),
                crate::msg::RepoWatchDegradedReason::WatchLimitReached { unwatched_dirs } => {
                    format!(
                        "Live file watching is partial: {unwatched_dirs} folders could not be watched \
                     (the system inotify limit was reached). Changes in them refresh when the window \
                     regains focus. Raise fs.inotify.max_user_watches to watch everything."
                    )
                }
            };
            util::push_notification(state, crate::model::AppNotificationKind::Warning, message);
            Vec::new()
        }
        Msg::SetHistoryScope { repo_id, scope } => {
            external_and_history::set_history_scope(state, repo_id, scope)
        }
        Msg::SetFetchPruneDeletedRemoteTrackingBranches { repo_id, enabled } => {
            repo_management::set_fetch_prune_deleted_remote_tracking_branches(
                state, repo_id, enabled,
            )
        }
        Msg::LoadMoreHistory { repo_id } => external_and_history::load_more_history(state, repo_id),
        Msg::SelectCommit { repo_id, commit_id } => {
            effects::select_commit(state, repo_id, commit_id)
        }
        Msg::SelectCommitMulti {
            repo_id,
            commit_id,
            mode,
            clicked_index,
            visible_order,
        } => effects::select_commit_multi(
            state,
            repo_id,
            commit_id,
            mode,
            clicked_index,
            visible_order,
        ),
        Msg::ClearCommitSelection { repo_id } => effects::clear_commit_selection(state, repo_id),
        Msg::SelectDiff { repo_id, target } => diff_selection::select_diff(state, repo_id, target),
        Msg::OpenInlineSubmoduleDiff {
            repo_id,
            submodule_repo_path,
            parent_submodule_path,
            entries,
            selected_ix,
        } => diff_selection::open_inline_submodule_diff(
            state,
            repo_id,
            submodule_repo_path,
            parent_submodule_path,
            entries,
            selected_ix,
        ),
        Msg::SelectInlineSubmoduleDiff {
            repo_id,
            selected_ix,
        } => diff_selection::select_inline_submodule_diff(state, repo_id, selected_ix),
        Msg::CloseInlineSubmoduleDiff { repo_id } => {
            diff_selection::close_inline_submodule_diff(state, repo_id)
        }
        Msg::SelectConflictDiff { repo_id, path } => {
            diff_selection::select_conflict_diff(state, repo_id, path)
        }
        Msg::ClearDiffSelection { repo_id } => diff_selection::clear_diff_selection(state, repo_id),
        Msg::EnsureSidebarData { repo_id, request } => {
            effects::ensure_sidebar_data(state, repo_id, request)
        }
        Msg::LoadStashes { repo_id } => effects::load_stashes(state, repo_id),
        Msg::LoadConflictFile {
            repo_id,
            path,
            mode,
        } => effects::load_conflict_file(state, repo_id, path, mode),
        Msg::LoadReflog { repo_id } => effects::load_reflog(state, repo_id),
        Msg::LoadRecentCommitMessages { repo_id, limit } => {
            effects::load_recent_commit_messages(state, repo_id, limit)
        }
        Msg::LoadFileHistory {
            repo_id,
            path,
            limit,
        } => effects::load_file_history(state, repo_id, path, limit),
        Msg::LoadBlame {
            repo_id,
            path,
            source,
        } => effects::load_blame(state, repo_id, path, source),
        Msg::LoadWorktrees { repo_id } => effects::load_worktrees(state, repo_id),
        Msg::LoadSubmodules { repo_id } => effects::load_submodules(state, repo_id),
        Msg::LoadTags { repo_id } => effects::load_tags(state, repo_id),
        Msg::LoadRemoteTags { repo_id } => effects::load_remote_tags(state, repo_id),
        Msg::RefreshBranches { repo_id } => effects::refresh_branches(state, repo_id),
        Msg::LoadFileBrowser { repo_id, source } => {
            effects::load_file_browser(state, repo_id, source)
        }
        Msg::ToggleFileBrowserDir { repo_id, path } => {
            effects::toggle_file_browser_dir(state, repo_id, path)
        }
        Msg::SetFileBrowserSearch { repo_id, query } => {
            effects::set_file_browser_search(state, repo_id, query)
        }
        Msg::SetFileBrowserSource { repo_id, source } => {
            effects::set_file_browser_source(state, repo_id, source)
        }
        Msg::OpenFileContent {
            repo_id,
            source,
            path,
        } => diff_selection::open_file_content(state, repo_id, source, path),
        Msg::OpenFileAtCommitParent {
            repo_id,
            commit_id,
            path,
        } => vec![Effect::OpenFileAtCommitParent {
            repo_id,
            commit_id,
            path,
        }],
        Msg::OpenFileAtCommit {
            repo_id,
            commit_id,
            path,
        } => vec![Effect::OpenFileAtCommit {
            repo_id,
            commit_id,
            path,
        }],
        Msg::BrowseRepositoryAtCommit { repo_id, commit_id } => {
            effects::browse_repository_at_commit(state, repo_id, commit_id)
        }
        Msg::ResetBrowseToLive { repo_id } => effects::reset_browse_to_live(state, repo_id),
        Msg::ViewerNavBack { repo_id } => {
            diff_selection::viewer_nav(state, repo_id, crate::model::ViewNavDir::Back)
        }
        Msg::ViewerNavForward { repo_id } => {
            diff_selection::viewer_nav(state, repo_id, crate::model::ViewNavDir::Forward)
        }
        Msg::GlobalNavBack { repo_id } => {
            diff_selection::global_nav(state, repo_id, crate::model::ViewNavDir::Back)
        }
        Msg::GlobalNavForward { repo_id } => {
            diff_selection::global_nav(state, repo_id, crate::model::ViewNavDir::Forward)
        }
        Msg::SetSidebarMode { mode } => effects::set_sidebar_mode(state, mode),
        Msg::StageHunk { repo_id, patch } => {
            begin_local_action(state, repo_id);
            diff_selection::stage_hunk(repo_id, patch)
        }
        Msg::UnstageHunk { repo_id, patch } => {
            begin_local_action(state, repo_id);
            diff_selection::unstage_hunk(repo_id, patch)
        }
        Msg::ApplyWorktreePatch {
            repo_id,
            patch,
            reverse,
        } => {
            begin_local_action(state, repo_id);
            diff_selection::apply_worktree_patch(repo_id, patch, reverse)
        }
        Msg::CheckoutBranch { repo_id, name } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::checkout_branch(repo_id, name)
        }
        Msg::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::checkout_remote_branch(repo_id, remote, branch, local_branch)
        }
        Msg::CheckoutCommit { repo_id, commit_id } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(Some(commit_id.clone()));
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::checkout_commit(repo_id, commit_id)
        }
        Msg::CherryPickCommit { repo_id, commit_id } => {
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::cherry_pick_commit(repo_id, commit_id)
        }
        Msg::RevertCommit { repo_id, commit_id } => {
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::revert_commit(repo_id, commit_id)
        }
        Msg::CreateBranch {
            repo_id,
            name,
            target,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_branch(repo_id, name, target)
        }
        Msg::CreateBranchAndCheckout {
            repo_id,
            name,
            target,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(None);
            }
            begin_head_changing_local_action(state, repo_id);
            actions_emit_effects::create_branch_and_checkout(repo_id, name, target)
        }
        Msg::DeleteBranch { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::delete_branch(repo_id, name)
        }
        Msg::ForceDeleteBranch { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::force_delete_branch(repo_id, name)
        }
        Msg::CloneRepo { url, dest } => repo_management::clone_repo(state, url, dest),
        Msg::AbortCloneRepo { dest } => repo_management::abort_clone_repo(state, dest),
        Msg::Internal(crate::msg::InternalMsg::CloneRepoProgress { dest, line }) => {
            repo_management::clone_repo_progress(state, dest, line)
        }
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished { url, dest, result }) => {
            let auth_prompt = result
                .as_ref()
                .err()
                .and_then(|error| auth_prompt_for_clone(&url, &dest, error));
            let effects = repo_management::clone_repo_finished(state, url, dest, result);
            if let Some(prompt) = auth_prompt {
                util::clear_staged_git_auth_env();
                state.auth_prompt = Some(prompt);
            }
            effects
        }
        Msg::ExportPatch {
            repo_id,
            commit_id,
            dest,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::export_patch(repo_id, commit_id, dest)
        }
        Msg::ApplyPatch { repo_id, patch } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::apply_patch(repo_id, patch)
        }
        Msg::AddWorktree {
            repo_id,
            path,
            reference,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_add(1);
            }
            actions_emit_effects::add_worktree(repo_id, path, reference)
        }
        Msg::RemoveWorktree { repo_id, path } => {
            let normalized_path = if let Some(repo_state) =
                state.repos.iter_mut().find(|r| r.id == repo_id)
            {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_add(1);
                normalize_repo_relative_path(&repo_state.spec.workdir, path)
            } else {
                path
            };
            actions_emit_effects::remove_worktree(repo_id, normalized_path)
        }
        Msg::ForceRemoveWorktree { repo_id, path } => {
            let normalized_path = if let Some(repo_state) =
                state.repos.iter_mut().find(|r| r.id == repo_id)
            {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_add(1);
                normalize_repo_relative_path(&repo_state.spec.workdir, path)
            } else {
                path
            };
            actions_emit_effects::force_remove_worktree(repo_id, normalized_path)
        }
        Msg::AddSubmodule {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
        } => {
            state.submodule_trust_prompt = None;
            vec![Effect::CheckSubmoduleAddTrust {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
            }]
        }
        Msg::AddSubmoduleTrusted {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            approved_sources,
        } => {
            begin_local_action(state, repo_id);
            start_submodule_add_progress(state, repo_id, &url, &path);
            actions_emit_effects::add_submodule(
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                approved_sources,
            )
        }
        Msg::UpdateSubmodules { repo_id } => {
            state.submodule_trust_prompt = None;
            vec![Effect::CheckSubmoduleUpdateTrust { repo_id }]
        }
        Msg::UpdateSubmodulesTrusted {
            repo_id,
            approved_sources,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::update_submodules(repo_id, approved_sources)
        }
        Msg::LoadSubmodule { repo_id, path } => {
            state.submodule_trust_prompt = None;
            vec![Effect::CheckSubmoduleLoadTrust { repo_id, path }]
        }
        Msg::LoadSubmoduleTrusted {
            repo_id,
            path,
            approved_sources,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::load_submodule(repo_id, path, approved_sources)
        }
        Msg::ConfirmSubmoduleTrustPrompt => {
            let Some(prompt) = state.submodule_trust_prompt.take() else {
                return Vec::new();
            };
            match prompt.operation {
                SubmoduleTrustPromptOperation::Add {
                    url,
                    path,
                    branch,
                    name,
                    force,
                } => {
                    begin_local_action(state, prompt.repo_id);
                    start_submodule_add_progress(state, prompt.repo_id, &url, &path);
                    actions_emit_effects::add_submodule(
                        prompt.repo_id,
                        url,
                        path,
                        branch,
                        name,
                        force,
                        prompt.sources,
                    )
                }
                SubmoduleTrustPromptOperation::Update => {
                    begin_local_action(state, prompt.repo_id);
                    actions_emit_effects::update_submodules(prompt.repo_id, prompt.sources)
                }
                SubmoduleTrustPromptOperation::Load { path } => {
                    begin_local_action(state, prompt.repo_id);
                    actions_emit_effects::load_submodule(prompt.repo_id, path, prompt.sources)
                }
            }
        }
        Msg::CancelSubmoduleTrustPrompt => {
            state.submodule_trust_prompt = None;
            Vec::new()
        }
        Msg::ChangeSubmodulePointer {
            repo_id,
            path,
            reference,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::change_submodule_pointer(repo_id, path, reference)
        }
        Msg::RemoveSubmodule { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::remove_submodule(repo_id, path)
        }
        Msg::StagePath { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::stage_path(repo_id, path)
        }
        Msg::StagePaths { repo_id, paths } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::stage_paths(repo_id, paths)
        }
        Msg::UnstagePath { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::unstage_path(repo_id, path)
        }
        Msg::UnstagePaths { repo_id, paths } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::unstage_paths(repo_id, paths)
        }
        Msg::DiscardWorktreeChangesPath { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::discard_worktree_changes_path(repo_id, path)
        }
        Msg::DiscardWorktreeChangesPaths { repo_id, paths } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::discard_worktree_changes_paths(repo_id, paths)
        }
        Msg::SaveWorktreeFile {
            repo_id,
            path,
            contents,
            stage,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::save_worktree_file(repo_id, path, contents, stage)
        }
        Msg::Commit {
            repo_id,
            message,
            push_after_commit,
        } => {
            begin_commit_action(state, repo_id);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = Some(PendingCommitRetry {
                    message: message.clone(),
                    amend: false,
                    push_after_commit,
                });
            }
            actions_emit_effects::commit(repo_id, message)
        }
        Msg::CommitAmend {
            repo_id,
            message,
            push_after_commit,
        } => {
            begin_commit_action(state, repo_id);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = Some(PendingCommitRetry {
                    message: message.clone(),
                    amend: true,
                    push_after_commit,
                });
            }
            actions_emit_effects::commit_amend(repo_id, message)
        }
        Msg::SafePushAfterCommit { repo_id, context } => {
            actions_emit_effects::safe_push_after_commit(repo_id, context)
        }
        Msg::FetchAll { repo_id } => actions_emit_effects::fetch_all(repos, state, repo_id),
        Msg::PruneMergedBranches { repo_id } => {
            actions_emit_effects::prune_merged_branches(repos, state, repo_id)
        }
        Msg::PruneLocalTags { repo_id } => {
            actions_emit_effects::prune_local_tags(repos, state, repo_id)
        }
        Msg::Pull { repo_id, mode } => actions_emit_effects::pull(repos, state, repo_id, mode),
        Msg::PullBranch {
            repo_id,
            remote,
            branch,
        } => actions_emit_effects::pull_branch(repos, state, repo_id, remote, branch),
        Msg::MergeRef { repo_id, reference } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::merge_ref(repo_id, reference)
        }
        Msg::SquashRef { repo_id, reference } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::squash_ref(repo_id, reference)
        }
        Msg::Push { repo_id } => actions_emit_effects::push(repos, state, repo_id),
        Msg::PushAfterCommit {
            repo_id,
            target,
            set_upstream,
        } => actions_emit_effects::push_after_commit(repos, state, repo_id, target, set_upstream),
        Msg::ForcePush { repo_id } => actions_emit_effects::force_push(repos, state, repo_id),
        Msg::ForcePushWithLease { repo_id, lease } => {
            actions_emit_effects::force_push_with_lease(repos, state, repo_id, lease)
        }
        Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        } => actions_emit_effects::push_set_upstream(repos, state, repo_id, remote, branch),
        Msg::SetUpstreamBranch {
            repo_id,
            branch,
            upstream,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::set_upstream_branch(repo_id, branch, upstream)
        }
        Msg::UnsetUpstreamBranch { repo_id, branch } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::unset_upstream_branch(repo_id, branch)
        }
        Msg::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
        } => actions_emit_effects::delete_remote_branch(repos, state, repo_id, remote, branch),
        Msg::Reset {
            repo_id,
            target,
            mode,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::reset(repo_id, target, mode)
        }
        Msg::PrepareSquash { repo_id } => effects::prepare_squash(state, repo_id),
        Msg::SquashCommits {
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        } => actions_emit_effects::squash_commits(
            state,
            repo_id,
            oldest,
            expected_head,
            message,
            count,
        ),
        Msg::Rebase { repo_id, onto } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::rebase(repo_id, onto)
        }
        Msg::RebaseContinue { repo_id } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::rebase_continue(repo_id)
        }
        Msg::RebaseAbort { repo_id } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::rebase_abort(repo_id)
        }
        Msg::LoadInteractiveRebaseSetup { repo_id, base } => {
            actions_emit_effects::load_interactive_rebase_setup(state, repo_id, base)
        }
        Msg::InteractiveRebase {
            repo_id,
            base,
            entries,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::interactive_rebase(repo_id, base, entries)
        }
        Msg::CancelInteractiveRebaseSetup { repo_id } => {
            actions_emit_effects::cancel_interactive_rebase_setup(state, repo_id)
        }
        Msg::MergeAbort { repo_id } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::merge_abort(repo_id)
        }
        Msg::CreateTag {
            repo_id,
            name,
            target,
            message,
            annotated,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_tag(repo_id, name, target, message, annotated)
        }
        Msg::DeleteTag { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::delete_tag(repo_id, name)
        }
        Msg::PushTag {
            repo_id,
            remote,
            name,
        } => actions_emit_effects::push_tag(repos, state, repo_id, remote, name),
        Msg::DeleteRemoteTag {
            repo_id,
            remote,
            name,
        } => actions_emit_effects::delete_remote_tag(repos, state, repo_id, remote, name),
        Msg::AddRemote { repo_id, name, url } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::add_remote(repo_id, name, url)
        }
        Msg::RemoveRemote { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::remove_remote(repo_id, name)
        }
        Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::set_remote_url(repo_id, name, url, kind)
        }
        Msg::CheckoutConflictSide {
            repo_id,
            path,
            side,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_conflict_side(repo_id, path, side)
        }
        Msg::AcceptConflictDeletion { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::accept_conflict_deletion(repo_id, path)
        }
        Msg::CheckoutConflictBase { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_conflict_base(repo_id, path)
        }
        Msg::LaunchMergetool { repo_id, path } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::launch_mergetool(repo_id, path)
        }
        Msg::RecordConflictAutosolveTelemetry {
            repo_id,
            path,
            mode,
            total_conflicts_before,
            total_conflicts_after,
            unresolved_before,
            unresolved_after,
            stats,
        } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                util::push_action_log(
                    repo_state,
                    true,
                    util::conflict_autosolve_telemetry_command(mode, path.as_deref()),
                    util::conflict_autosolve_telemetry_summary(
                        mode,
                        path.as_deref(),
                        total_conflicts_before,
                        total_conflicts_after,
                        unresolved_before,
                        unresolved_after,
                        stats,
                    ),
                    None,
                );
            }
            Vec::new()
        }
        Msg::ConflictSetHideResolved {
            repo_id,
            path,
            hide_resolved,
        } => conflict_interactions::set_hide_resolved(state, repo_id, path, hide_resolved),
        Msg::ConflictApplyBulkChoice {
            repo_id,
            path,
            choice,
        } => conflict_interactions::apply_bulk_choice(state, repo_id, path, choice),
        Msg::ConflictSetRegionChoice {
            repo_id,
            path,
            region_index,
            choice,
        } => conflict_interactions::set_region_choice(state, repo_id, path, region_index, choice),
        Msg::ConflictSyncRegionResolutions {
            repo_id,
            path,
            updates,
        } => conflict_interactions::sync_region_resolutions(state, repo_id, path, updates),
        Msg::ConflictApplyAutosolve {
            repo_id,
            path,
            mode,
            whitespace_normalize,
        } => {
            conflict_interactions::apply_autosolve(state, repo_id, path, mode, whitespace_normalize)
        }
        Msg::ConflictResetResolutions { repo_id, path } => {
            conflict_interactions::reset_resolutions(state, repo_id, path)
        }
        Msg::Stash {
            repo_id,
            message,
            include_untracked,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::stash(repo_id, message, include_untracked)
        }
        Msg::ApplyStash { repo_id, index } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::apply_stash(repo_id, index)
        }
        Msg::PopStash { repo_id, index } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::pop_stash(repo_id, index)
        }
        Msg::DropStash { repo_id, index } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::drop_stash(repo_id, index)
        }
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id,
            spec,
            repo,
        }) => repo_management::repo_opened_ok(repos, state, repo_id, spec, repo),
        Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
            repo_id,
            load_epoch,
            message,
        }) => {
            let current_load_epoch = state
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .map(|repo| repo.load_epoch);
            if current_load_epoch == Some(load_epoch) {
                repo_load_trace::trace!(
                    "apply_repo_load_finished repo_id={:?} load_epoch={} inner={}",
                    repo_id,
                    load_epoch,
                    repo_load_trace::internal_msg_name(&message)
                );
                reduce(repos, id_alloc, state, Msg::Internal(*message))
            } else {
                repo_load_trace::trace!(
                    "drop_stale_repo_load_finished repo_id={:?} load_epoch={} current_load_epoch={:?} inner={}",
                    repo_id,
                    load_epoch,
                    current_load_epoch,
                    repo_load_trace::internal_msg_name(&message)
                );
                Vec::new()
            }
        }
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id,
            spec,
            error,
        }) => repo_management::repo_opened_err(repos, state, repo_id, spec, error),
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded { repo_id, result }) => {
            effects::branches_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RemotesLoaded { repo_id, result }) => {
            effects::remotes_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded { repo_id, result }) => {
            effects::remote_branches_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded { repo_id, result }) => {
            effects::worktree_status_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded { repo_id, result }) => {
            effects::staged_status_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded { repo_id, result }) => {
            effects::status_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded { repo_id, result }) => {
            effects::head_branch_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded { repo_id, result }) => {
            effects::upstream_divergence_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            scope,
            cursor,
            result,
        }) => external_and_history::log_loaded(state, repo_id, scope, cursor, result),
        Msg::Internal(crate::msg::InternalMsg::TagsLoaded { repo_id, result }) => {
            effects::tags_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RemoteTagsLoaded { repo_id, result }) => {
            effects::remote_tags_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::StashesLoaded { repo_id, result }) => {
            effects::stashes_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::ReflogLoaded { repo_id, result }) => {
            effects::reflog_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded { repo_id, result }) => {
            external_and_history::rebase_state_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::InteractiveRebaseSetupLoaded {
            repo_id,
            base,
            result,
        }) => external_and_history::interactive_rebase_setup_loaded(state, repo_id, base, result),
        Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded { repo_id, result }) => {
            external_and_history::merge_commit_message_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
            repo_id,
            path,
            result,
        }) => effects::file_history_loaded(state, repo_id, path, result),
        Msg::Internal(crate::msg::InternalMsg::BlameLoaded {
            repo_id,
            path,
            source,
            result,
        }) => effects::blame_loaded(state, repo_id, path, source, result),
        Msg::Internal(crate::msg::InternalMsg::ConflictFileLoaded {
            repo_id,
            path,
            result,
            conflict_session,
        }) => effects::conflict_file_loaded(state, repo_id, path, *result, conflict_session),
        Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded { repo_id, result }) => {
            effects::worktrees_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded { repo_id, result }) => {
            effects::submodules_loaded(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::FileBrowserLoaded {
            repo_id,
            source,
            result,
        }) => effects::file_browser_loaded(state, repo_id, source, result),
        Msg::Internal(crate::msg::InternalMsg::SubmoduleAddTrustChecked {
            repo_id,
            url,
            path,
            branch,
            name,
            force,
            result,
        }) => match result {
            Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed) => {
                begin_local_action(state, repo_id);
                start_submodule_add_progress(state, repo_id, &url, &path);
                actions_emit_effects::add_submodule(
                    repo_id,
                    url,
                    path,
                    branch,
                    name,
                    force,
                    Vec::new(),
                )
            }
            Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt { sources }) => {
                state.submodule_trust_prompt = Some(SubmoduleTrustPromptState {
                    repo_id,
                    operation: SubmoduleTrustPromptOperation::Add {
                        url,
                        path,
                        branch,
                        name,
                        force,
                    },
                    sources,
                });
                Vec::new()
            }
            Err(error) => {
                state.banner_error = Some(BannerErrorState {
                    repo_id: Some(repo_id),
                    message: util::format_failure_summary("Submodule trust check", &error),
                });
                Vec::new()
            }
        },
        Msg::Internal(crate::msg::InternalMsg::SubmoduleUpdateTrustChecked { repo_id, result }) => {
            match result {
                Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed) => {
                    begin_local_action(state, repo_id);
                    actions_emit_effects::update_submodules(repo_id, Vec::new())
                }
                Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt { sources }) => {
                    state.submodule_trust_prompt = Some(SubmoduleTrustPromptState {
                        repo_id,
                        operation: SubmoduleTrustPromptOperation::Update,
                        sources,
                    });
                    Vec::new()
                }
                Err(error) => {
                    state.banner_error = Some(BannerErrorState {
                        repo_id: Some(repo_id),
                        message: util::format_failure_summary("Submodule trust check", &error),
                    });
                    Vec::new()
                }
            }
        }
        Msg::Internal(crate::msg::InternalMsg::SubmoduleLoadTrustChecked {
            repo_id,
            path,
            result,
        }) => match result {
            Ok(gitcomet_core::services::SubmoduleTrustDecision::Proceed) => {
                begin_local_action(state, repo_id);
                actions_emit_effects::load_submodule(repo_id, path, Vec::new())
            }
            Ok(gitcomet_core::services::SubmoduleTrustDecision::Prompt { sources }) => {
                state.submodule_trust_prompt = Some(SubmoduleTrustPromptState {
                    repo_id,
                    operation: SubmoduleTrustPromptOperation::Load { path },
                    sources,
                });
                Vec::new()
            }
            Err(error) => {
                state.banner_error = Some(BannerErrorState {
                    repo_id: Some(repo_id),
                    message: util::format_failure_summary("Submodule trust check", &error),
                });
                Vec::new()
            }
        },
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id,
            commit_id,
            result,
        }) => effects::commit_details_loaded(state, repo_id, commit_id, result),
        Msg::Internal(crate::msg::InternalMsg::SquashMessagePreviewLoaded {
            repo_id,
            oldest,
            head,
            result,
        }) => effects::squash_message_preview_loaded(state, repo_id, oldest, head, result),
        Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev,
            result,
        }) => effects::recent_commit_messages_loaded(state, repo_id, request_rev, result),
        Msg::Internal(crate::msg::InternalMsg::DiffLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::DiffFileLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_file_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::DiffPreviewTextFileLoaded {
            repo_id,
            target,
            side,
            result,
        }) => diff_selection::diff_preview_text_file_loaded(state, repo_id, target, side, result),
        Msg::Internal(crate::msg::InternalMsg::SubmoduleSummaryLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::submodule_summary_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        }) => {
            diff_selection::inline_submodule_diff_loaded(state, repo_id, inline_rev, target, result)
        }
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        }) => diff_selection::inline_submodule_diff_file_loaded(
            state, repo_id, inline_rev, target, result,
        ),
        Msg::Internal(crate::msg::InternalMsg::InlineSubmoduleDiffFileImageLoaded {
            repo_id,
            inline_rev,
            target,
            result,
        }) => diff_selection::inline_submodule_diff_file_image_loaded(
            state, repo_id, inline_rev, target, result,
        ),
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_file_image_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action,
            result,
        }) => external_and_history::repo_action_finished(state, repo_id, action, result),
        Msg::Internal(crate::msg::InternalMsg::CommitFinished { repo_id, result }) => {
            let pending_commit = state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.pending_commit_retry.clone());
            let outcome = result.as_ref().ok().cloned();
            let push_after_commit = outcome.is_some()
                && pending_commit
                    .as_ref()
                    .is_some_and(|pending| pending.push_after_commit);
            let auth_prompt = result
                .as_ref()
                .err()
                .and_then(|error| auth_prompt_for_commit(repo_id, pending_commit.clone(), error));
            let commit_result = result.map(|_| ());
            let mut effects = actions_emit_effects::commit_finished(state, repo_id, commit_result);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = None;
            }
            if let Some(prompt) = auth_prompt {
                util::clear_staged_git_auth_env();
                state.auth_prompt = Some(prompt);
            }
            if push_after_commit
                && let (Some(outcome), Some(pending_commit)) = (outcome, pending_commit)
            {
                effects.extend(actions_emit_effects::safe_push_after_commit(
                    repo_id,
                    SafePushAfterCommitContext {
                        amend: pending_commit.amend,
                        local_branch: outcome.local_branch,
                        pre_head: outcome.pre_head,
                        post_head: outcome.post_head,
                    },
                ));
            }
            effects
        }
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished { repo_id, result }) => {
            let pending_commit = state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.pending_commit_retry.clone());
            let outcome = result.as_ref().ok().cloned();
            let push_after_commit = outcome.is_some()
                && pending_commit
                    .as_ref()
                    .is_some_and(|pending| pending.push_after_commit);
            let auth_prompt = result
                .as_ref()
                .err()
                .and_then(|error| auth_prompt_for_commit(repo_id, pending_commit.clone(), error));
            let commit_result = result.map(|_| ());
            let mut effects =
                actions_emit_effects::commit_amend_finished(state, repo_id, commit_result);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = None;
            }
            if let Some(prompt) = auth_prompt {
                util::clear_staged_git_auth_env();
                state.auth_prompt = Some(prompt);
            }
            if push_after_commit
                && let (Some(outcome), Some(pending_commit)) = (outcome, pending_commit)
            {
                effects.extend(actions_emit_effects::safe_push_after_commit(
                    repo_id,
                    SafePushAfterCommitContext {
                        amend: pending_commit.amend,
                        local_branch: outcome.local_branch,
                        pre_head: outcome.pre_head,
                        post_head: outcome.post_head,
                    },
                ));
            }
            effects
        }
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context,
            auth,
            result,
        }) => {
            let auth_prompt = result.as_ref().err().and_then(|error| {
                auth_prompt_for_safe_push_after_commit(repo_id, context.clone(), error)
            });
            let effects = actions_emit_effects::safe_push_after_commit_finished(
                repos, state, repo_id, auth, result,
            );
            if let Some(prompt) = auth_prompt {
                util::clear_staged_git_auth_env();
                state.auth_prompt = Some(prompt);
            }
            effects
        }
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command,
            result,
        }) => {
            let auth_prompt = result
                .as_ref()
                .err()
                .and_then(|error| auth_prompt_for_repo_command(repo_id, &command, error));
            let removed_worktree_path = match (&command, &result) {
                (RepoCommandKind::RemoveWorktree { path }, Ok(_)) => Some(path.clone()),
                (RepoCommandKind::ForceRemoveWorktree { path }, Ok(_)) => Some(path.clone()),
                _ => None,
            };

            let effects =
                actions_emit_effects::repo_command_finished(state, repo_id, command, result);

            if let Some(path) = removed_worktree_path {
                let repo_ids_to_close = state
                    .repos
                    .iter()
                    .filter(|repo| repo.spec.workdir == path)
                    .map(|repo| repo.id)
                    .collect::<Vec<_>>();
                for repo_id in repo_ids_to_close {
                    let _ = repo_management::close_repo(repos, state, repo_id);
                }
            }

            if let Some(prompt) = auth_prompt {
                util::clear_staged_git_auth_env();
                state.auth_prompt = Some(prompt);
            }

            effects
        }
    }
}

#[cfg(test)]
mod nav_history_tests {
    use super::*;
    use crate::model::{AppState, RepoState};
    use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, RepoSpec};
    use gitcomet_core::process::{
        GitExecutableAvailability, GitExecutablePreference, GitRuntimeState,
    };
    use std::sync::atomic::AtomicU64;

    fn available_state_with_repo(repo_id: RepoId) -> AppState {
        let mut state = AppState::default();
        state.git_runtime = GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability: GitExecutableAvailability::Available {
                version_output: "git version 2.0.0".to_string(),
            },
        };
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: std::path::PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(repo_id);
        state
    }

    fn dispatch(state: &mut AppState, msg: Msg) {
        let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
        let id_alloc = AtomicU64::new(99);
        let _ = reduce(&mut repos, &id_alloc, state, msg);
    }

    fn repo(state: &AppState, repo_id: RepoId) -> &RepoState {
        state.repos.iter().find(|r| r.id == repo_id).unwrap()
    }

    #[test]
    fn repo_watch_degraded_pushes_warning_notification() {
        let mut state = AppState::default();
        dispatch(
            &mut state,
            Msg::RepoWatchDegraded {
                repo_id: RepoId(1),
                reason: crate::msg::RepoWatchDegradedReason::TooManyFolders { dir_count: 9000 },
            },
        );
        assert_eq!(state.notifications.len(), 1);
        let note = &state.notifications[0];
        assert_eq!(note.kind, crate::model::AppNotificationKind::Warning);
        assert!(
            note.message.contains("9000"),
            "warning should mention the folder count: {}",
            note.message
        );

        // A partial watch failure surfaces a (distinct) warning too — not just the stderr log.
        dispatch(
            &mut state,
            Msg::RepoWatchDegraded {
                repo_id: RepoId(1),
                reason: crate::msg::RepoWatchDegradedReason::WatchLimitReached {
                    unwatched_dirs: 42,
                },
            },
        );
        assert_eq!(state.notifications.len(), 2);
        let note = &state.notifications[1];
        assert_eq!(note.kind, crate::model::AppNotificationKind::Warning);
        assert!(
            note.message.contains("42"),
            "partial-watch warning should mention the unwatched count: {}",
            note.message
        );
    }

    #[test]
    fn opening_a_file_diff_is_recorded_and_back_restores_the_log() {
        let repo_id = RepoId(1);
        let mut state = available_state_with_repo(repo_id);
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        };

        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: target.clone(),
            },
        );
        assert_eq!(repo(&state, repo_id).diff_state.diff_target, Some(target));
        // Origin (history log) seeded + the diff.
        assert_eq!(repo(&state, repo_id).nav_history.entries.len(), 2);

        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        assert_eq!(
            repo(&state, repo_id).diff_state.diff_target,
            None,
            "back closes the file diff and shows the history log"
        );

        dispatch(&mut state, Msg::GlobalNavForward { repo_id });
        assert!(
            repo(&state, repo_id).diff_state.diff_target.is_some(),
            "forward reopens the file diff"
        );
    }

    #[test]
    fn commit_then_file_diffs_are_all_remembered() {
        let repo_id = RepoId(1);
        let mut state = available_state_with_repo(repo_id);
        let commit_a = CommitId("aaa".into());
        let file1 = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("file1.rs")),
        };
        let file2 = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("file2.rs")),
        };

        dispatch(
            &mut state,
            Msg::SelectCommit {
                repo_id,
                commit_id: commit_a.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file1.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file2.clone(),
            },
        );

        let entries = &repo(&state, repo_id).nav_history.entries;
        assert!(entries.iter().any(|e| e.diff_target == Some(file1.clone())));
        assert!(entries.iter().any(|e| e.diff_target == Some(file2.clone())));

        // Back must step one-by-one: file2 diff -> file1 diff -> commit details
        // (commit selected, no diff) -> history log.
        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        assert_eq!(
            repo(&state, repo_id).diff_state.diff_target,
            Some(file1.clone())
        );

        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(r.diff_state.diff_target, None, "should show commit details");
        assert_eq!(
            r.history_state.selected_commit.as_ref(),
            Some(&commit_a),
            "commit should still be selected at the details step"
        );

        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        assert_eq!(
            repo(&state, repo_id).history_state.selected_commit,
            None,
            "final back returns to the history log with no commit selected"
        );
    }

    #[test]
    fn view_navigation_messages_push_others_fold_in_place() {
        // User navigations create a new global back/forward step.
        assert!(is_view_navigation(&Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: std::path::PathBuf::from("a.txt"),
                area: DiffArea::Unstaged,
            },
        }));
        assert!(is_view_navigation(&Msg::SelectCommit {
            repo_id: RepoId(1),
            commit_id: CommitId("a".into()),
        }));
        // The file-content viewer's own back/forward does NOT land a global
        // step — it operates on a separate viewer-level stack so it does not
        // pollute the global back/forward history.
        assert!(!is_view_navigation(&Msg::ViewerNavBack {
            repo_id: RepoId(1)
        }));
        // Background / non-navigation messages do not push a step (they are
        // folded into the current entry in place, so they can't pollute history).
        assert!(!is_view_navigation(&Msg::DismissBannerError));
    }

    #[test]
    fn closure_and_replay_messages_are_not_view_navigations() {
        assert!(!is_view_navigation(&Msg::ClearDiffSelection {
            repo_id: RepoId(1),
        }));
        assert!(!is_view_navigation(&Msg::ClearCommitSelection {
            repo_id: RepoId(1),
        }));
        assert!(!is_view_navigation(&Msg::ViewerNavBack {
            repo_id: RepoId(1),
        }));
        assert!(!is_view_navigation(&Msg::ViewerNavForward {
            repo_id: RepoId(1),
        }));
        assert!(!is_view_navigation(&Msg::CloseInlineSubmoduleDiff {
            repo_id: RepoId(1),
        }));
        assert!(is_view_navigation(&Msg::OpenInlineSubmoduleDiff {
            repo_id: RepoId(1),
            submodule_repo_path: std::path::PathBuf::from("/tmp/sub"),
            parent_submodule_path: std::path::PathBuf::from("sub"),
            entries: vec![],
            selected_ix: 0,
        }));
    }

    #[test]
    fn close_inline_submodule_diff_folds_in_place_and_does_not_bloat_nav_history() {
        // Closing a sub-view must fold in-place: if the snapshot after
        // closing matches a previous entry, it should collapse back to that
        // entry rather than pushing a duplicate.
        let repo_id = RepoId(1);
        let mut state = available_state_with_repo(repo_id);

        // Seed: select a working tree diff (entries: [origin, diff], cursor=1).
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("a.txt"),
            area: DiffArea::Unstaged,
        };
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: target.clone(),
            },
        );
        assert_eq!(repo(&state, repo_id).nav_history.entries.len(), 2);
        assert_eq!(repo(&state, repo_id).nav_history.cursor, 1);

        // Open inline submodule diff.
        dispatch(
            &mut state,
            Msg::OpenInlineSubmoduleDiff {
                repo_id,
                submodule_repo_path: std::path::PathBuf::from("/tmp/repo/vendor/first"),
                parent_submodule_path: std::path::PathBuf::from("vendor/first"),
                entries: vec![],
                selected_ix: 0,
            },
        );

        // Close inline submodule diff — must fold, not push.
        dispatch(&mut state, Msg::CloseInlineSubmoduleDiff { repo_id });
        assert_eq!(
            repo(&state, repo_id).nav_history.entries.len(),
            2,
            "close must not add a new nav entry"
        );
        assert_eq!(
            repo(&state, repo_id).nav_history.cursor,
            1,
            "cursor must not advance past the parent diff"
        );
    }

    #[test]
    fn clearing_diff_folds_in_place_and_single_back_goes_to_commit_details() {
        let repo_id = RepoId(1);
        let mut state = available_state_with_repo(repo_id);
        let commit_a = CommitId("aaa".into());
        let file = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("file1.rs")),
        };

        dispatch(
            &mut state,
            Msg::SelectCommit {
                repo_id,
                commit_id: commit_a.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file.clone(),
            },
        );
        // User clicks the same committed file again, which dispatches
        // ClearDiffSelection to close the diff view.
        dispatch(&mut state, Msg::ClearDiffSelection { repo_id });

        let entries = &repo(&state, repo_id).nav_history.entries;
        // After folding in-place, no duplicate entry remains—the file
        // diff entry is collapsed back into the commit-details entry.
        assert_eq!(
            entries.len(),
            2,
            "fold-and-collapse must not create a new entry"
        );
        assert_eq!(
            repo(&state, repo_id).nav_history.cursor,
            1,
            "cursor should be back at the commit-details step"
        );

        // One GlobalNavBack from the commit-details view goes to the
        // history log (origin), confirming the stack did not bloat.
        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(r.diff_state.diff_target, None);
        assert_eq!(r.history_state.selected_commit, None);
        assert!(!r.nav_history.can_back());
    }

    #[test]
    fn clearing_diff_without_folding_previous_allows_correct_back() {
        let repo_id = RepoId(1);
        let mut state = available_state_with_repo(repo_id);
        let commit_a = CommitId("aaa".into());
        let commit_b = CommitId("bbb".into());
        let file = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("file1.rs")),
        };

        dispatch(
            &mut state,
            Msg::SelectCommit {
                repo_id,
                commit_id: commit_a.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file.clone(),
            },
        );
        // Switch to a different commit (no fold-collapse because the
        // new state differs from the previous entry).
        dispatch(
            &mut state,
            Msg::SelectCommit {
                repo_id,
                commit_id: commit_b.clone(),
            },
        );

        let r = repo(&state, repo_id);
        assert_eq!(
            r.nav_history.entries.len(),
            4,
            "select-commit pushes a new entry when the commit changes"
        );
        assert_eq!(r.nav_history.cursor, 3);
        assert_eq!(r.history_state.selected_commit.as_ref(), Some(&commit_b));

        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(
            r.diff_state.diff_target,
            Some(file),
            "back should reopen the file diff"
        );
        assert_eq!(r.history_state.selected_commit.as_ref(), Some(&commit_a));
    }

    #[test]
    fn browsing_committed_files_within_a_commit_keeps_commit_selected_on_back() {
        let repo_id = RepoId(1);
        let mut state = available_state_with_repo(repo_id);
        let commit_a = CommitId("aaa".into());
        let file_a = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("src/a.rs")),
        };
        let file_b = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("src/b.rs")),
        };
        let file_c = DiffTarget::Commit {
            commit_id: commit_a.clone(),
            path: Some(std::path::PathBuf::from("src/c.rs")),
        };

        dispatch(
            &mut state,
            Msg::SelectCommit {
                repo_id,
                commit_id: commit_a.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file_a.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file_b.clone(),
            },
        );
        dispatch(
            &mut state,
            Msg::SelectDiff {
                repo_id,
                target: file_c.clone(),
            },
        );

        let r = repo(&state, repo_id);
        // Origin + commit details + three file diffs = 5 entries.
        assert_eq!(
            r.nav_history.entries.len(),
            5,
            "each file selection must push a distinct history entry"
        );
        assert_eq!(r.nav_history.cursor, 4);
        assert_eq!(r.diff_state.diff_target, Some(file_c.clone()));

        // ── Back 1: file_c → file_b ──
        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(
            r.diff_state.diff_target,
            Some(file_b.clone()),
            "first back must return to the previously viewed file (b)"
        );
        assert_eq!(
            r.history_state.selected_commit.as_ref(),
            Some(&commit_a),
            "commit must remain selected while browsing files"
        );
        assert_eq!(r.nav_history.cursor, 3);

        // ── Back 2: file_b → file_a ──
        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(
            r.diff_state.diff_target,
            Some(file_a.clone()),
            "second back must return to the first opened file (a)"
        );
        assert_eq!(r.history_state.selected_commit.as_ref(), Some(&commit_a));
        assert_eq!(r.nav_history.cursor, 2);

        // ── Back 3: file_a → commit details (no diff, commit still selected) ──
        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(
            r.diff_state.diff_target, None,
            "third back closes the last file diff and shows commit details"
        );
        assert_eq!(
            r.history_state.selected_commit.as_ref(),
            Some(&commit_a),
            "commit must still be selected — back must not deselect the commit"
        );
        assert_eq!(r.nav_history.cursor, 1);

        // ── Back 4: commit details → history log ──
        dispatch(&mut state, Msg::GlobalNavBack { repo_id });
        let r = repo(&state, repo_id);
        assert_eq!(r.diff_state.diff_target, None);
        assert_eq!(
            r.history_state.selected_commit, None,
            "only the fourth back returns to the history log"
        );
        assert_eq!(r.nav_history.cursor, 0);
        assert!(!r.nav_history.can_back());
    }
}
