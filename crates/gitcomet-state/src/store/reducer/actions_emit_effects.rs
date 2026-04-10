use super::util::{
    SelectedConflictTarget, clear_banner_error_for_repo, diff_reload_effects,
    diff_target_preview_flags, format_failure_summary, push_action_log, push_command_log,
    refresh_full_effects, refresh_primary_effects, selected_conflict_target,
    start_conflict_target_reload, start_current_conflict_target_reload,
};
use crate::model::{AppState, Loadable, RepoId, RepoState};
use crate::msg::{Effect, RepoCommandKind, RepoPathList};
use gitcomet_core::conflict_session::{ConflictRegionResolution, ConflictResolverStrategy};
use gitcomet_core::domain::{DiffTarget, FileConflictKind};
use gitcomet_core::error::Error;
use gitcomet_core::services::{CommandOutput, GitRepository, PullMode, RemoteUrlKind, ResetMode};
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
    commit_id: gitcomet_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::CheckoutCommit { repo_id, commit_id }]
}

pub(super) fn cherry_pick_commit(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::CherryPickCommit { repo_id, commit_id }]
}

pub(super) fn revert_commit(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) -> Vec<Effect> {
    vec![Effect::RevertCommit { repo_id, commit_id }]
}

pub(super) fn create_branch(repo_id: RepoId, name: String, target: String) -> Vec<Effect> {
    vec![Effect::CreateBranch {
        repo_id,
        name,
        target,
    }]
}

pub(super) fn create_branch_and_checkout(
    repo_id: RepoId,
    name: String,
    target: String,
) -> Vec<Effect> {
    vec![Effect::CreateBranchAndCheckout {
        repo_id,
        name,
        target,
    }]
}

pub(super) fn delete_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::DeleteBranch { repo_id, name }]
}

pub(super) fn force_delete_branch(repo_id: RepoId, name: String) -> Vec<Effect> {
    vec![Effect::ForceDeleteBranch { repo_id, name }]
}

pub(super) fn export_patch(
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
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

pub(super) fn force_remove_worktree(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::ForceRemoveWorktree { repo_id, path }]
}

pub(super) fn add_submodule(
    repo_id: RepoId,
    url: String,
    path: PathBuf,
    branch: Option<String>,
    name: Option<String>,
    force: bool,
    approved_sources: Vec<gitcomet_core::services::SubmoduleTrustTarget>,
) -> Vec<Effect> {
    vec![Effect::AddSubmodule {
        repo_id,
        url,
        path,
        branch,
        name,
        force,
        approved_sources,
        auth: None,
    }]
}

pub(super) fn update_submodules(
    repo_id: RepoId,
    approved_sources: Vec<gitcomet_core::services::SubmoduleTrustTarget>,
) -> Vec<Effect> {
    vec![Effect::UpdateSubmodules {
        repo_id,
        approved_sources,
        auth: None,
    }]
}

pub(super) fn remove_submodule(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::RemoveSubmodule { repo_id, path }]
}

pub(super) fn stage_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::StagePath { repo_id, path }]
}

pub(super) fn stage_paths(repo_id: RepoId, paths: RepoPathList) -> Vec<Effect> {
    vec![Effect::StagePaths { repo_id, paths }]
}

pub(super) fn unstage_path(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::UnstagePath { repo_id, path }]
}

pub(super) fn unstage_paths(repo_id: RepoId, paths: RepoPathList) -> Vec<Effect> {
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
    vec![Effect::Commit {
        repo_id,
        message,
        auth: None,
    }]
}

pub(super) fn commit_amend(repo_id: RepoId, message: String) -> Vec<Effect> {
    vec![Effect::CommitAmend {
        repo_id,
        message,
        auth: None,
    }]
}

enum InFlightKind {
    Pull,
    Push,
}

fn bump_in_flight(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    kind: InFlightKind,
) {
    if !repos.contains_key(&repo_id) {
        return;
    }
    if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
        match kind {
            InFlightKind::Pull => {
                repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_add(1);
            }
            InFlightKind::Push => {
                repo_state.push_in_flight = repo_state.push_in_flight.saturating_add(1);
            }
        }
        repo_state.bump_ops_rev();
    }
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
        .is_some_and(|repo_state| repo_state.fetch_prune_deleted_remote_tracking_branches);
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::FetchAll {
        repo_id,
        prune,
        auth: None,
    }]
}

pub(super) fn prune_merged_branches(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::PruneMergedBranches { repo_id }]
}

pub(super) fn prune_local_tags(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::PruneLocalTags { repo_id }]
}

pub(super) fn pull(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    mode: PullMode,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::Pull {
        repo_id,
        mode,
        auth: None,
    }]
}

pub(super) fn pull_branch(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Pull);
    vec![Effect::PullBranch {
        repo_id,
        remote,
        branch,
        auth: None,
    }]
}

pub(super) fn merge_ref(repo_id: RepoId, reference: String) -> Vec<Effect> {
    vec![Effect::MergeRef { repo_id, reference }]
}

pub(super) fn squash_ref(repo_id: RepoId, reference: String) -> Vec<Effect> {
    vec![Effect::SquashRef { repo_id, reference }]
}

pub(super) fn push(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::Push {
        repo_id,
        auth: None,
    }]
}

pub(super) fn force_push(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::ForcePush {
        repo_id,
        auth: None,
    }]
}

pub(super) fn push_set_upstream(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::PushSetUpstream {
        repo_id,
        remote,
        branch,
        auth: None,
    }]
}

pub(super) fn set_upstream_branch(
    repo_id: RepoId,
    branch: String,
    upstream: String,
) -> Vec<Effect> {
    vec![Effect::SetUpstreamBranch {
        repo_id,
        branch,
        upstream,
    }]
}

pub(super) fn unset_upstream_branch(repo_id: RepoId, branch: String) -> Vec<Effect> {
    vec![Effect::UnsetUpstreamBranch { repo_id, branch }]
}

pub(super) fn delete_remote_branch(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    branch: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::DeleteRemoteBranch {
        repo_id,
        remote,
        branch,
        auth: None,
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

pub(super) fn push_tag(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    name: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::PushTag {
        repo_id,
        remote,
        name,
        auth: None,
    }]
}

pub(super) fn delete_remote_tag(
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    repo_id: RepoId,
    remote: String,
    name: String,
) -> Vec<Effect> {
    bump_in_flight(repos, state, repo_id, InFlightKind::Push);
    vec![Effect::DeleteRemoteTag {
        repo_id,
        remote,
        name,
        auth: None,
    }]
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
    side: gitcomet_core::services::ConflictSide,
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

pub(super) fn pop_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::PopStash { repo_id, index }]
}

pub(super) fn drop_stash(repo_id: RepoId, index: usize) -> Vec<Effect> {
    vec![Effect::DropStash { repo_id, index }]
}

pub(super) fn commit_finished(
    state: &mut AppState,
    repo_id: RepoId,
    result: std::result::Result<(), Error>,
) -> Vec<Effect> {
    let mut clear_banner = false;
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
    repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_sub(1);
    repo_state.bump_ops_rev();
    match result {
        Ok(()) => {
            repo_state.last_error = None;
            clear_banner = true;
            repo_state.diff_state.diff_target = None;
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
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
    if clear_banner {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        let effects = refresh_primary_effects(repo_state);
        clear_banner_error_for_repo(state, repo_id);
        return effects;
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
    let mut clear_banner = false;
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo_state.local_actions_in_flight = repo_state.local_actions_in_flight.saturating_sub(1);
    repo_state.commit_in_flight = repo_state.commit_in_flight.saturating_sub(1);
    repo_state.bump_ops_rev();
    match result {
        Ok(()) => {
            repo_state.last_error = None;
            clear_banner = true;
            repo_state.diff_state.diff_target = None;
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
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
    if clear_banner {
        let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
            return Vec::new();
        };
        let effects = refresh_primary_effects(repo_state);
        clear_banner_error_for_repo(state, repo_id);
        return effects;
    }
    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    refresh_primary_effects(repo_state)
}

fn tracks_local_actions_in_flight(command: &RepoCommandKind) -> bool {
    matches!(
        command,
        RepoCommandKind::MergeRef { .. }
            | RepoCommandKind::SquashRef { .. }
            | RepoCommandKind::Reset { .. }
            | RepoCommandKind::Rebase { .. }
            | RepoCommandKind::RebaseContinue
            | RepoCommandKind::RebaseAbort
            | RepoCommandKind::MergeAbort
            | RepoCommandKind::CreateTag { .. }
            | RepoCommandKind::DeleteTag { .. }
            | RepoCommandKind::AddRemote { .. }
            | RepoCommandKind::RemoveRemote { .. }
            | RepoCommandKind::SetRemoteUrl { .. }
            | RepoCommandKind::SetUpstreamBranch { .. }
            | RepoCommandKind::UnsetUpstreamBranch { .. }
            | RepoCommandKind::CheckoutConflict { .. }
            | RepoCommandKind::AcceptConflictDeletion { .. }
            | RepoCommandKind::CheckoutConflictBase { .. }
            | RepoCommandKind::LaunchMergetool { .. }
            | RepoCommandKind::SaveWorktreeFile { .. }
            | RepoCommandKind::ExportPatch { .. }
            | RepoCommandKind::ApplyPatch { .. }
            | RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules { .. }
            | RepoCommandKind::RemoveSubmodule { .. }
            | RepoCommandKind::StageHunk
            | RepoCommandKind::UnstageHunk
            | RepoCommandKind::ApplyWorktreePatch { .. }
    )
}

pub(super) fn repo_command_finished(
    state: &mut AppState,
    repo_id: RepoId,
    command: RepoCommandKind,
    result: std::result::Result<CommandOutput, Error>,
) -> Vec<Effect> {
    let refresh_worktrees = matches!(
        &command,
        RepoCommandKind::AddWorktree { .. }
            | RepoCommandKind::RemoveWorktree { .. }
            | RepoCommandKind::ForceRemoveWorktree { .. }
    ) && result.is_ok();
    let refresh_submodules = matches!(
        &command,
        RepoCommandKind::AddSubmodule { .. }
            | RepoCommandKind::UpdateSubmodules { .. }
            | RepoCommandKind::RemoveSubmodule { .. }
    ) && result.is_ok();
    let command_succeeded = result.is_ok();
    let mut clear_banner = false;

    let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };

    let mut extra_effects = Vec::new();
    match &command {
        RepoCommandKind::FetchAll
        | RepoCommandKind::PruneMergedBranches
        | RepoCommandKind::PruneLocalTags
        | RepoCommandKind::Pull { .. }
        | RepoCommandKind::PullBranch { .. } => {
            repo_state.pull_in_flight = repo_state.pull_in_flight.saturating_sub(1);
            repo_state.bump_ops_rev();
        }
        RepoCommandKind::Push
        | RepoCommandKind::ForcePush
        | RepoCommandKind::PushSetUpstream { .. }
        | RepoCommandKind::DeleteRemoteBranch { .. }
        | RepoCommandKind::PushTag { .. }
        | RepoCommandKind::DeleteRemoteTag { .. } => {
            repo_state.push_in_flight = repo_state.push_in_flight.saturating_sub(1);
            repo_state.bump_ops_rev();
        }
        RepoCommandKind::AddWorktree { .. }
        | RepoCommandKind::RemoveWorktree { .. }
        | RepoCommandKind::ForceRemoveWorktree { .. } => {
            repo_state.worktrees_in_flight = repo_state.worktrees_in_flight.saturating_sub(1);
        }
        _ if tracks_local_actions_in_flight(&command) => {
            repo_state.local_actions_in_flight =
                repo_state.local_actions_in_flight.saturating_sub(1);
            repo_state.bump_ops_rev();
        }
        _ => {}
    }

    match result {
        Ok(output) => {
            repo_state.last_error = None;
            clear_banner = true;
            if matches!(
                &command,
                RepoCommandKind::Reset { .. }
                    | RepoCommandKind::Rebase { .. }
                    | RepoCommandKind::RebaseContinue
                    | RepoCommandKind::RebaseAbort
                    | RepoCommandKind::MergeAbort
            ) {
                repo_state.diff_state.diff_target = None;
                repo_state.diff_state.diff = Loadable::NotLoaded;
                repo_state.diff_state.diff_file = Loadable::NotLoaded;
                repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
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
    if command_succeeded && sync_conflict_session_after_resolution_command(repo_state, &command) {
        repo_state.bump_conflict_rev();
    }

    if refresh_worktrees {
        repo_state.set_worktrees(Loadable::Loading);
        extra_effects.push(Effect::LoadWorktrees { repo_id });
    }
    if refresh_submodules {
        repo_state.set_submodules(Loadable::Loading);
        extra_effects.push(Effect::LoadSubmodules { repo_id });
    }
    if matches!(
        &command,
        RepoCommandKind::StageHunk
            | RepoCommandKind::UnstageHunk
            | RepoCommandKind::ApplyWorktreePatch { .. }
    ) && let Some(target) = repo_state.diff_state.diff_target.clone()
    {
        if let Some(conflict_target) = selected_conflict_target(repo_state, &target) {
            repo_state.diff_state.diff = Loadable::NotLoaded;
            repo_state.diff_state.diff_file = Loadable::NotLoaded;
            repo_state.diff_state.diff_file_image = Loadable::NotLoaded;
            repo_state.bump_diff_state_rev();
            match conflict_target {
                SelectedConflictTarget::Current => {
                    extra_effects.extend(start_current_conflict_target_reload(repo_state));
                }
                SelectedConflictTarget::Path(path) => {
                    extra_effects.extend(start_conflict_target_reload(repo_state, path));
                }
            }
        } else {
            repo_state.diff_state.diff = Loadable::Loading;
            let supports_file = matches!(
                &target,
                DiffTarget::WorkingTree { .. } | DiffTarget::Commit { path: Some(_), .. }
            );
            let preview = diff_target_preview_flags(&target);
            repo_state.diff_state.diff_file =
                if supports_file && (!preview.wants_image || preview.is_svg) {
                    Loadable::Loading
                } else {
                    Loadable::NotLoaded
                };
            repo_state.diff_state.diff_file_image = if supports_file && preview.wants_image {
                Loadable::Loading
            } else {
                Loadable::NotLoaded
            };
            repo_state.bump_diff_state_rev();
            extra_effects.extend(diff_reload_effects(repo_id, target));
        }
    }
    let mut effects = refresh_full_effects(repo_state);
    effects.extend(extra_effects);
    if clear_banner {
        clear_banner_error_for_repo(state, repo_id);
    }
    effects
}

fn sync_conflict_session_after_resolution_command(
    repo_state: &mut RepoState,
    command: &RepoCommandKind,
) -> bool {
    let Some(path) = resolution_command_path(command) else {
        return false;
    };

    let tracked_path_matches = repo_state
        .conflict_state
        .conflict_file_path
        .as_ref()
        .is_some_and(|tracked| tracked.as_path() == path.as_path());
    if !tracked_path_matches {
        return false;
    }

    if matches!(command, RepoCommandKind::LaunchMergetool { .. }) {
        clear_conflict_context(repo_state);
        return true;
    }

    let Some(session_view) = repo_state.conflict_state.conflict_session.as_ref() else {
        return false;
    };
    if session_view.path.as_path() != path.as_path() {
        return false;
    }

    if session_view.strategy == ConflictResolverStrategy::BinarySidePick
        && session_view.regions.is_empty()
    {
        clear_conflict_context(repo_state);
        return true;
    }

    let resolution = match command {
        RepoCommandKind::CheckoutConflict { side, .. } => match side {
            gitcomet_core::services::ConflictSide::Ours => ConflictRegionResolution::PickOurs,
            gitcomet_core::services::ConflictSide::Theirs => ConflictRegionResolution::PickTheirs,
        },
        RepoCommandKind::CheckoutConflictBase { .. } => ConflictRegionResolution::PickBase,
        RepoCommandKind::AcceptConflictDeletion { .. } => {
            deletion_resolution_for_kind(session_view.conflict_kind)
        }
        _ => return false,
    };

    let Some(session) = repo_state.conflict_state.conflict_session.as_mut() else {
        return false;
    };

    apply_resolution_to_all_regions(session, &resolution) > 0
}

fn resolution_command_path(command: &RepoCommandKind) -> Option<&std::path::PathBuf> {
    match command {
        RepoCommandKind::CheckoutConflict { path, .. }
        | RepoCommandKind::CheckoutConflictBase { path }
        | RepoCommandKind::AcceptConflictDeletion { path }
        | RepoCommandKind::LaunchMergetool { path } => Some(path),
        _ => None,
    }
}

fn clear_conflict_context(repo_state: &mut RepoState) {
    repo_state.conflict_state.conflict_file_path = None;
    repo_state.conflict_state.conflict_file_load_mode =
        crate::model::ConflictFileLoadMode::CurrentOnly;
    repo_state.conflict_state.conflict_file = Loadable::NotLoaded;
    repo_state.conflict_state.conflict_session = None;
    repo_state.conflict_state.conflict_hide_resolved = false;
}

fn deletion_resolution_for_kind(conflict_kind: FileConflictKind) -> ConflictRegionResolution {
    match conflict_kind {
        FileConflictKind::DeletedByUs
        | FileConflictKind::AddedByThem
        | FileConflictKind::BothDeleted => ConflictRegionResolution::PickOurs,
        FileConflictKind::DeletedByThem | FileConflictKind::AddedByUs => {
            ConflictRegionResolution::PickTheirs
        }
        FileConflictKind::BothAdded | FileConflictKind::BothModified => {
            ConflictRegionResolution::PickOurs
        }
    }
}

fn apply_resolution_to_all_regions(
    session: &mut gitcomet_core::conflict_session::ConflictSession,
    resolution: &ConflictRegionResolution,
) -> usize {
    let mut changed = 0usize;
    for region in &mut session.regions {
        if matches!(resolution, ConflictRegionResolution::PickBase) && region.base.is_none() {
            continue;
        }
        if &region.resolution != resolution {
            region.resolution = resolution.clone();
            changed += 1;
        }
    }
    changed
}
