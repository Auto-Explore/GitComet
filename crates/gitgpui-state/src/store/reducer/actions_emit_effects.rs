use super::util::{
    diff_target_is_svg, diff_target_wants_image_preview, format_failure_summary, push_action_log,
    push_command_log, refresh_full_effects, refresh_primary_effects,
};
use crate::model::{AppState, Loadable, RepoId};
use crate::msg::{Effect, RepoCommandKind};
use gitgpui_core::domain::DiffTarget;
use gitgpui_core::error::Error;
use gitgpui_core::services::{CommandOutput, GitRepository, PullMode, RemoteUrlKind, ResetMode};
use rustc_hash::FxHashMap as HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub(super) fn checkout_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::CheckoutBranch { repo_id, name }]
}

pub(super) fn checkout_remote_branch(
    repo_id: RepoId,
    remote: String,
    branch: String,
    local_branch: String,
) -> Vec<Effect> {
    vec![Effect::CheckoutRemoteBranch {
        repo_id,
        remote,
        branch,
        local_branch,
    }]
}

pub(super) fn checkout_commit(
    repo_id: RepoId,
    commit_id: gitgpui_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::CheckoutCommit { repo_id, commit_id }]
}

pub(super) fn cherry_pick_commit(
    repo_id: RepoId,
    commit_id: gitgpui_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::CherryPickCommit { repo_id, commit_id }]
}

pub(super) fn revert_commit(
    repo_id: RepoId,
    commit_id: gitgpui_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::RevertCommit { repo_id, commit_id }]
}

pub(super) fn create_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::CreateBranch { repo_id, name }]
}

pub(super) fn create_branch_and_checkout(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::CreateBranchAndCheckout { repo_id, name }]
}

pub(super) fn delete_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::DeleteBranch { repo_id, name }]
}

pub(super) fn force_delete_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::ForceDeleteBranch { repo_id, name }]
}

pub(super) fn export_patch(
    repo_id: RepoId,
    commit_id: gitgpui_core::domain::CommitId,
    dest: PathBuf,
) -> Vec<Effect> {
    vec![Effect::ExportPatch {
        repo_id,
        commit_id,
        dest,
    }]
}

pub(super) fn apply_patch(repo_id: RepoId, patch: PathBuf) -> Vec<Effect> {
    vec![Effect::ApplyPatch { repo_id, patch }]
}

pub(super) fn add_worktree(
    repo_id: RepoId,
    path: PathBuf,
    reference: Option<String>,
) -> Vec<Effect> {
    vec![Effect::AddWorktree {
        repo_id,
        path,
        reference,
    }]
}

pub(super) fn remove_worktree(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::RemoveWorktree { repo_id, path }]
}

pub(super) fn add_submodule(repo_id: RepoId, url: String, path: PathBuf) -> Vec<Effect> {
    vec![Effect::AddSubmodule { repo_id, url, path }]
}

pub(super) fn update_submodules(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::UpdateSubmodules { repo_id }]
}

pub(super) fn remove_submodule(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::RemoveSubmodule { repo_id, path }]
}

pub(super) fn stage_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::StagePath { repo_id, path }]
}

pub(super) fn stage_paths(repo_id: RepoId, paths: Vec<PathBuf>) -> Vec<Effect> {
    vec![Effect::StagePaths { repo_id, paths }]
}

pub(super) fn unstage_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::UnstagePath { repo_id, path }]
}

pub(super) fn unstage_paths(repo_id: RepoId, paths: Vec<PathBuf>) -> Vec<Effect> {
    vec![Effect::UnstagePaths { repo_id, paths }]
}

pub(super) fn discard_worktree_changes_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::DiscardWorktreeChangesPath { repo_id, path }]
}

pub(super) fn discard_worktree_changes_paths(repo_id: RepoId, paths: Vec<PathBuf>) -> Vec<Effect> {
    vec![Effect::DiscardWorktreeChangesPaths { repo_id, paths }]
}

pub(super) fn save_worktree_file(
    repo_id: RepoId,
    path: PathBuf,
    contents: String,
    stage: bool,
) -> Vec<Effect> {
    vec![Effect::SaveWorktreeFile {
        repo_id,
        path,
        contents,
        stage,
    }]
}

pub(super) fn commit(repo_id: RepoId, message: String) -> Vec<Effect> {
    vec![Effect::Commit { repo_id, message }]
}

pub(super) fn commit_amend(repo_id: RepoId, message: String) -> Vec<Effect> {
    vec![Effect::CommitAmend { repo_id, message }]
}

pub(super) fn fetch_all(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    let prune = state
        .repos
        .iter()
        .find(|r| r.id == repo_id)
        .map(|r| r.fetch_prune_deleted_remote_tracking_branches)
        .unwrap_or(true);

    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::FetchAll { repo_id, prune }]
}

pub(super) fn pull(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    mode: PullMode,
) -> Vec<Effect> {
    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::Pull { repo_id, mode }]
}

pub(super) fn pull_branch(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::PullBranch {
        repo_id,
        remote,
        branch,
    }]
}

pub(super) fn merge_ref(repo_id: RepoId, reference: String) -> Vec<Effect> {
    vec![Effect::MergeRef { repo_id, reference }]
}

pub(super) fn push(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.push_in_flight = repo_state.push_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::Push { repo_id }]
}

pub(super) fn force_push(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.push_in_flight = repo_state.push_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::ForcePush { repo_id }]
}

pub(super) fn push_set_upstream(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.push_in_flight = repo_state.push_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::PushSetUpstream {
        repo_id,
        remote,
        branch,
    }]
}

pub(super) fn delete_remote_branch(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    if repos.contains_key(&repo_id)
        && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
    {
        repo_state.push_in_flight = repo_state.push_in_flight.saturating_add(1);
        repo_state.bump_ops_rev();
    }
    vec![Effect::DeleteRemoteBranch {
        repo_id,
        remote,
        branch,
    }]
}

pub(super) fn reset(repo_id: RepoId, target: String, mode: ResetMode) -> Vec<Effect> {
    vec![Effect::Reset {
        repo_id,
        target,
        mode,
    }]
}

pub(super) fn rebase(repo_id: RepoId, onto: String) -> Vec<Effect> {
    vec![Effect::Rebase { repo_id, onto }]
}

pub(super) fn rebase_continue(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::RebaseContinue { repo_id }]
}

pub(super) fn rebase_abort(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::RebaseAbort { repo_id }]
}

pub(super) fn merge_abort(repo_id: RepoId) -> Vec<Effect> {
    vec![Effect::MergeAbort { repo_id }]
}

pub(super) fn create_tag(repo_id: RepoId, name: String, target: String) -> Vec<Effect> {
    vec![Effect::CreateTag {
        repo_id,
        name,
        target,
    }]
}

pub(super) fn delete_tag(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::DeleteTag { repo_id, name }]
}

pub(super) fn add_remote(repo_id: RepoId, name: String, url: String) -> Vec<Effect> {
    vec![Effect::AddRemote { repo_id, name, url }]
}

pub(super) fn remove_remote(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::RemoveRemote { repo_id, name }]
}

pub(super) fn set_remote_url(
    repo_id: RepoId,
    name: String,
    url: String,
    kind: RemoteUrlKind,
) -> Vec<Effect> {
    vec![Effect::SetRemoteUrl {
        repo_id,
        name,
        url,
        kind,
    }]
}

pub(super) fn checkout_conflict_side(
    repo_id: RepoId,
    path: PathBuf,
    side: gitgpui_core::services::ConflictSide,
) -> Vec<Effect> {
    vec![Effect::CheckoutConflictSide {
        repo_id,
        path,
        side,
    }]
}

pub(super) fn accept_conflict_deletion(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::AcceptConflictDeletion { repo_id, path }]
}

pub(super) fn checkout_conflict_base(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::CheckoutConflictBase { repo_id, path }]
}

pub(super) fn launch_mergetool(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::LaunchMergetool { repo_id, path }]
}

pub(super) fn stash(repo_id: RepoId, message: String, include_untracked: bool) -> Vec<Effect> {
    vec![Effect::Stash {
        repo_id,
        message,
        include_untracked,
    }]
}

pub(super) fn apply_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::ApplyStash { repo_id, index }]
}

pub(super) fn drop_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::DropStash { repo_id, index }]
}

pub(super) fn pop_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::PopStash { repo_id, index }]
}

pub(super) fn commit_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
        repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_sub(1);
        repo_state.bump_ops_rev();
        match result {
            Ok(()) => {
                repo_state.last_error = None;
                repo_state.diff_target = None;
                repo_state.diff = Loadable::NotLoaded;
                repo_state.diff_file = Loadable::NotLoaded;
                repo_state.diff_file_image = Loadable::NotLoaded;
                repo_state.bump_diff_state_rev();
                push_action_log(
                    repo_state,
                    true,
                    "Commit".to_string(),
                    "Commit: Completed".to_string(),
                    None,
                );
            }
            Err(e) => {
                let summary = format_failure_summary("Commit", &e);
                repo_state.last_error = Some(summary.clone());
                push_action_log(repo_state, false, "Commit".to_string(), summary, Some(&e));
            }
        }
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    refresh_primary_effects(repo_state)
}

pub(super) fn commit_amend_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
        repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_sub(1);
        repo_state.bump_ops_rev();
        match result {
            Ok(()) => {
                repo_state.last_error = None;
                repo_state.diff_target = None;
                repo_state.diff = Loadable::NotLoaded;
                repo_state.diff_file = Loadable::NotLoaded;
                repo_state.diff_file_image = Loadable::NotLoaded;
                repo_state.bump_diff_state_rev();
                push_action_log(
                    repo_state,
                    true,
                    "Amend".to_string(),
                    "Amend: Completed".to_string(),
                    None,
                );
            }
            Err(e) => {
                let summary = format_failure_summary("Amend", &e);
                repo_state.last_error = Some(summary.clone());
                push_action_log(repo_state, false, "Amend".to_string(), summary, Some(&e));
            }
        }
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    refresh_primary_effects(repo_state)
}

pub(super) fn repo_command_finished(
    state: &mut AppState,
    repo_id: RepoId,
    command: RepoCommandKind,
    result: std::result::Result<CommandOutput, Error>,
) -> Vec<Effect> {
    let refresh_worktrees = matches!(
        &command,
        RepoCommandKind::AddWorktree { .. } | RepoCommandKind::RemoveWorktree { .. }
    ) && result.is_ok();
    let refresh_submodules = matches!(
        &command,
        RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules
            | RepoCommandKind::RemoveSubmodule { .. }
    ) && result.is_ok();

    let mut extra_effects = Vec::new();
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match command {
            RepoCommandKind::FetchAll
            | RepoCommandKind::Pull { .. }
            | RepoCommandKind::PullBranch { .. } => {
                repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_sub(1);
                repo_state.bump_ops_rev();
            }
            RepoCommandKind::Push
            | RepoCommandKind::ForcePush
            | RepoCommandKind::PushSetUpstream { .. }
            | RepoCommandKind::DeleteRemoteBranch { .. } => {
                repo_state.push_in_flight = repo_state.push_in_flight.saturating_sub(1);
                repo_state.bump_ops_rev();
            }
            RepoCommandKind::AddWorktree { .. } | RepoCommandKind::RemoveWorktree { .. } => {
                repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_sub(1);
            }
            _ => {}
        }

        match result {
            Ok(output) => {
                repo_state.last_error = None;
                if matches!(
                    &command,
                    RepoCommandKind::Reset { .. }
                        | RepoCommandKind::Rebase { .. }
                        | RepoCommandKind::RebaseContinue
                        | RepoCommandKind::RebaseAbort
                        | RepoCommandKind::MergeAbort
                ) {
                    repo_state.diff_target = None;
                    repo_state.diff = Loadable::NotLoaded;
                    repo_state.diff_file = Loadable::NotLoaded;
                    repo_state.diff_file_image = Loadable::NotLoaded;
                    repo_state.bump_diff_state_rev();
                }
                push_command_log(repo_state, true, &command, &output, None);
            }
            Err(e) => {
                push_command_log(
                    repo_state,
                    false,
                    &command,
                    &CommandOutput::default(),
                    Some(&e),
                );
                repo_state.last_error = repo_state
                    .command_log
                    .last()
                    .map(|entry| entry.summary.clone());
            }
        }

        if refresh_worktrees {
            repo_state.set_worktrees(Loadable::Loading);
            extra_effects.push(Effect::LoadWorktrees { repo_id });
        }
        if refresh_submodules {
            repo_state.set_submodules(Loadable::Loading);
            extra_effects.push(Effect::LoadSubmodules { repo_id });
        }
    }
    if matches!(
        &command,
        RepoCommandKind::StageHunk
            | RepoCommandKind::UnstageHunk
            | RepoCommandKind::ApplyWorktreePatch { .. }
    ) && let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id)
        && let Some(target) = repo_state.diff_target.clone()
    {
        repo_state.diff = Loadable::Loading;
        let supports_file = matches!(
            &target,
            DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. }
        );
        let wants_image = diff_target_wants_image_preview(&target);
        let is_svg = diff_target_is_svg(&target);
        repo_state.diff_file = if supports_file && (!wants_image || is_svg) {
            Loadable::Loading
        } else {
            Loadable::NotLoaded
        };
        repo_state.diff_file_image = if supports_file && wants_image {
            Loadable::Loading
        } else {
            Loadable::NotLoaded
        };
        repo_state.bump_diff_state_rev();
        extra_effects.push(Effect::LoadDiff {
            repo_id,
            target: target.clone(),
        });
        if supports_file {
            if wants_image {
                extra_effects.push(Effect::LoadDiffFileImage {
                    repo_id,
                    target: target.clone(),
                });
            }
            if !wants_image || is_svg {
                extra_effects.push(Effect::LoadDiffFile { repo_id, target });
            }
        }
    }
    let mut effects = if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        refresh_full_effects(repo_state)
    } else {
        Vec::new()
    };
    effects.extend(extra_effects);
    effects
}
