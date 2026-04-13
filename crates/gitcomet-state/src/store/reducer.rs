mod actions_emit_effects;
mod conflict_interactions;
mod diff_selection;
mod effects;
mod external_and_history;
mod repo_management;
mod util;

use crate::model::{
    AppState, AuthPromptState, AuthRetryOperation, BannerErrorState, PendingCommitRetry, RepoId,
    SubmoduleTrustPromptOperation, SubmoduleTrustPromptState,
};
use crate::msg::{ConflictRegionChoice, Effect, Msg, RepoCommandKind, RepoPath, RepoPathList};
use gitcomet_core::auth::StagedGitAuth;
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap as HashMap;
use smallvec::SmallVec;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

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
        repo_state.bump_ops_rev();
    }
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
        AuthRetryOperation::Commit {
            repo_id,
            message,
            amend,
        } => Some(if amend {
            Msg::CommitAmend { repo_id, message }
        } else {
            Msg::Commit { repo_id, message }
        }),
        AuthRetryOperation::Clone { url, dest } => Some(Msg::CloneRepo { url, dest }),
    }
}

fn clear_banner_error_for_auth_operation(state: &mut AppState, operation: &AuthRetryOperation) {
    match operation {
        AuthRetryOperation::RepoCommand { repo_id, .. }
        | AuthRetryOperation::Commit { repo_id, .. } => {
            util::clear_banner_error_for_repo(state, *repo_id);
        }
        AuthRetryOperation::Clone { .. } => {}
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
        RepoCommandKind::ForcePush => Msg::ForcePush { repo_id },
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
        RepoCommandKind::Rebase { onto } => Msg::Rebase { repo_id, onto },
        RepoCommandKind::RebaseContinue => Msg::RebaseContinue { repo_id },
        RepoCommandKind::RebaseAbort => Msg::RebaseAbort { repo_id },
        RepoCommandKind::MergeAbort => Msg::MergeAbort { repo_id },
        RepoCommandKind::CreateTag { name, target } => Msg::CreateTag {
            repo_id,
            name,
            target,
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
        RepoCommandKind::RemoveSubmodule { path } => Msg::RemoveSubmodule { repo_id, path },
        // Not replayable because command metadata does not retain original content.
        RepoCommandKind::SaveWorktreeFile { .. }
        | RepoCommandKind::StageHunk
        | RepoCommandKind::UnstageHunk
        | RepoCommandKind::ApplyWorktreePatch { .. } => return None,
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
        | Effect::Commit { auth: slot, .. }
        | Effect::CommitAmend { auth: slot, .. }
        | Effect::FetchAll { auth: slot, .. }
        | Effect::Pull { auth: slot, .. }
        | Effect::PullBranch { auth: slot, .. }
        | Effect::Push { auth: slot, .. }
        | Effect::ForcePush { auth: slot, .. }
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

pub(crate) fn fill_select_diff_inline(
    state: &mut AppState,
    repo_id: RepoId,
    target: gitcomet_core::domain::DiffTarget,
    effects: &mut SelectDiffEffects,
) {
    diff_selection::fill_select_diff_inline(state, repo_id, target, effects)
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
    match msg {
        Msg::OpenRepo(path) => repo_management::open_repo(id_alloc, state, path),
        Msg::RestoreSession {
            open_repos,
            active_repo,
        } => repo_management::restore_session(repos, id_alloc, state, open_repos, active_repo),
        Msg::CloseRepo { repo_id } => repo_management::close_repo(repos, state, repo_id),
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
        Msg::RepoExternallyChanged { repo_id, change } => {
            external_and_history::repo_externally_changed(state, repo_id, change)
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
        Msg::ClearCommitSelection { repo_id } => effects::clear_commit_selection(state, repo_id),
        Msg::SelectDiff { repo_id, target } => diff_selection::select_diff(state, repo_id, target),
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
        Msg::LoadFileHistory {
            repo_id,
            path,
            limit,
        } => effects::load_file_history(state, repo_id, path, limit),
        Msg::LoadBlame { repo_id, path, rev } => effects::load_blame(state, repo_id, path, rev),
        Msg::LoadWorktrees { repo_id } => effects::load_worktrees(state, repo_id),
        Msg::LoadSubmodules { repo_id } => effects::load_submodules(state, repo_id),
        Msg::RefreshBranches { repo_id } => effects::refresh_branches(state, repo_id),
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
            begin_local_action(state, repo_id);
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
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_remote_branch(repo_id, remote, branch, local_branch)
        }
        Msg::CheckoutCommit { repo_id, commit_id } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.set_detached_head_commit(Some(commit_id.clone()));
            }
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_commit(repo_id, commit_id)
        }
        Msg::CherryPickCommit { repo_id, commit_id } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::cherry_pick_commit(repo_id, commit_id)
        }
        Msg::RevertCommit { repo_id, commit_id } => {
            begin_local_action(state, repo_id);
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
            begin_local_action(state, repo_id);
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
            }
        }
        Msg::CancelSubmoduleTrustPrompt => {
            state.submodule_trust_prompt = None;
            Vec::new()
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
        Msg::Commit { repo_id, message } => {
            begin_commit_action(state, repo_id);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = Some(PendingCommitRetry {
                    message: message.clone(),
                    amend: false,
                });
            }
            actions_emit_effects::commit(repo_id, message)
        }
        Msg::CommitAmend { repo_id, message } => {
            begin_commit_action(state, repo_id);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = Some(PendingCommitRetry {
                    message: message.clone(),
                    amend: true,
                });
            }
            actions_emit_effects::commit_amend(repo_id, message)
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
        Msg::ForcePush { repo_id } => actions_emit_effects::force_push(repos, state, repo_id),
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
        Msg::MergeAbort { repo_id } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::merge_abort(repo_id)
        }
        Msg::CreateTag {
            repo_id,
            name,
            target,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_tag(repo_id, name, target)
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
            rev,
            result,
        }) => effects::blame_loaded(state, repo_id, path, rev, result),
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
        Msg::Internal(crate::msg::InternalMsg::CommitDetailsLoaded {
            repo_id,
            commit_id,
            result,
        }) => effects::commit_details_loaded(state, repo_id, commit_id, result),
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
        Msg::Internal(crate::msg::InternalMsg::DiffFileImageLoaded {
            repo_id,
            target,
            result,
        }) => diff_selection::diff_file_image_loaded(state, repo_id, target, result),
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished { repo_id, result }) => {
            external_and_history::repo_action_finished(state, repo_id, result)
        }
        Msg::Internal(crate::msg::InternalMsg::CommitFinished { repo_id, result }) => {
            let pending_commit = state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.pending_commit_retry.clone());
            let auth_prompt = result
                .as_ref()
                .err()
                .and_then(|error| auth_prompt_for_commit(repo_id, pending_commit, error));
            let effects = actions_emit_effects::commit_finished(state, repo_id, result);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = None;
            }
            if let Some(prompt) = auth_prompt {
                util::clear_staged_git_auth_env();
                state.auth_prompt = Some(prompt);
            }
            effects
        }
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished { repo_id, result }) => {
            let pending_commit = state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .and_then(|r| r.pending_commit_retry.clone());
            let auth_prompt = result
                .as_ref()
                .err()
                .and_then(|error| auth_prompt_for_commit(repo_id, pending_commit, error));
            let effects = actions_emit_effects::commit_amend_finished(state, repo_id, result);
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.pending_commit_retry = None;
            }
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
