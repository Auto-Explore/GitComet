mod actions_emit_effects;
mod conflict_interactions;
mod diff_selection;
mod effects;
mod external_and_history;
mod repo_management;
mod util;

use crate::model::{AppState, RepoId};
use crate::msg::{Effect, Msg, RepoCommandKind};
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

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
        Msg::DismissRepoError { repo_id } => {
            if let Some(repo_state) = state.repos.iter_mut().find(|r| r.id == repo_id) {
                repo_state.last_error = None;
            }
            Vec::new()
        }
        Msg::SetActiveRepo { repo_id } => repo_management::set_active_repo(state, repo_id),
        Msg::ReorderRepoTabs {
            repo_id,
            insert_before,
        } => repo_management::reorder_repo_tabs(state, repo_id, insert_before),
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
        Msg::ClearDiffSelection { repo_id } => diff_selection::clear_diff_selection(state, repo_id),
        Msg::LoadStashes { repo_id } => effects::load_stashes(state, repo_id),
        Msg::LoadConflictFile { repo_id, path } => {
            effects::load_conflict_file(state, repo_id, path)
        }
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
        Msg::StageHunk { repo_id, patch } => diff_selection::stage_hunk(repo_id, patch),
        Msg::UnstageHunk { repo_id, patch } => diff_selection::unstage_hunk(repo_id, patch),
        Msg::ApplyWorktreePatch {
            repo_id,
            patch,
            reverse,
        } => diff_selection::apply_worktree_patch(repo_id, patch, reverse),
        Msg::CheckoutBranch { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_branch(repo_id, name)
        }
        Msg::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
        } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::checkout_remote_branch(repo_id, remote, branch, local_branch)
        }
        Msg::CheckoutCommit { repo_id, commit_id } => {
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
        Msg::CreateBranch { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_branch(repo_id, name)
        }
        Msg::CreateBranchAndCheckout { repo_id, name } => {
            begin_local_action(state, repo_id);
            actions_emit_effects::create_branch_and_checkout(repo_id, name)
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
        Msg::CloneRepoProgress { dest, line } => {
            repo_management::clone_repo_progress(state, dest, line)
        }
        Msg::CloneRepoFinished { url, dest, result } => {
            repo_management::clone_repo_finished(state, url, dest, result)
        }
        Msg::ExportPatch {
            repo_id,
            commit_id,
            dest,
        } => actions_emit_effects::export_patch(repo_id, commit_id, dest),
        Msg::ApplyPatch { repo_id, patch } => actions_emit_effects::apply_patch(repo_id, patch),
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
        Msg::AddSubmodule { repo_id, url, path } => {
            actions_emit_effects::add_submodule(repo_id, url, path)
        }
        Msg::UpdateSubmodules { repo_id } => actions_emit_effects::update_submodules(repo_id),
        Msg::RemoveSubmodule { repo_id, path } => {
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
        } => actions_emit_effects::save_worktree_file(repo_id, path, contents, stage),
        Msg::Commit { repo_id, message } => {
            begin_commit_action(state, repo_id);
            actions_emit_effects::commit(repo_id, message)
        }
        Msg::CommitAmend { repo_id, message } => {
            begin_commit_action(state, repo_id);
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
        Msg::MergeRef { repo_id, reference } => actions_emit_effects::merge_ref(repo_id, reference),
        Msg::Push { repo_id } => actions_emit_effects::push(repos, state, repo_id),
        Msg::ForcePush { repo_id } => actions_emit_effects::force_push(repos, state, repo_id),
        Msg::PushSetUpstream {
            repo_id,
            remote,
            branch,
        } => actions_emit_effects::push_set_upstream(repos, state, repo_id, remote, branch),
        Msg::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
        } => actions_emit_effects::delete_remote_branch(repos, state, repo_id, remote, branch),
        Msg::Reset {
            repo_id,
            target,
            mode,
        } => actions_emit_effects::reset(repo_id, target, mode),
        Msg::Rebase { repo_id, onto } => actions_emit_effects::rebase(repo_id, onto),
        Msg::RebaseContinue { repo_id } => actions_emit_effects::rebase_continue(repo_id),
        Msg::RebaseAbort { repo_id } => actions_emit_effects::rebase_abort(repo_id),
        Msg::MergeAbort { repo_id } => actions_emit_effects::merge_abort(repo_id),
        Msg::CreateTag {
            repo_id,
            name,
            target,
        } => actions_emit_effects::create_tag(repo_id, name, target),
        Msg::DeleteTag { repo_id, name } => actions_emit_effects::delete_tag(repo_id, name),
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
            actions_emit_effects::add_remote(repo_id, name, url)
        }
        Msg::RemoveRemote { repo_id, name } => actions_emit_effects::remove_remote(repo_id, name),
        Msg::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        } => actions_emit_effects::set_remote_url(repo_id, name, url, kind),
        Msg::CheckoutConflictSide {
            repo_id,
            path,
            side,
        } => actions_emit_effects::checkout_conflict_side(repo_id, path, side),
        Msg::AcceptConflictDeletion { repo_id, path } => {
            actions_emit_effects::accept_conflict_deletion(repo_id, path)
        }
        Msg::CheckoutConflictBase { repo_id, path } => {
            actions_emit_effects::checkout_conflict_base(repo_id, path)
        }
        Msg::LaunchMergetool { repo_id, path } => {
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
        Msg::RepoOpenedOk {
            repo_id,
            spec,
            repo,
        } => repo_management::repo_opened_ok(repos, state, repo_id, spec, repo),
        Msg::RepoOpenedErr {
            repo_id,
            spec,
            error,
        } => repo_management::repo_opened_err(repos, state, repo_id, spec, error),
        Msg::BranchesLoaded { repo_id, result } => effects::branches_loaded(state, repo_id, result),
        Msg::RemotesLoaded { repo_id, result } => effects::remotes_loaded(state, repo_id, result),
        Msg::RemoteBranchesLoaded { repo_id, result } => {
            effects::remote_branches_loaded(state, repo_id, result)
        }
        Msg::StatusLoaded { repo_id, result } => effects::status_loaded(state, repo_id, result),
        Msg::HeadBranchLoaded { repo_id, result } => {
            effects::head_branch_loaded(state, repo_id, result)
        }
        Msg::UpstreamDivergenceLoaded { repo_id, result } => {
            effects::upstream_divergence_loaded(state, repo_id, result)
        }
        Msg::LogLoaded {
            repo_id,
            scope,
            cursor,
            result,
        } => external_and_history::log_loaded(state, repo_id, scope, cursor, result),
        Msg::TagsLoaded { repo_id, result } => effects::tags_loaded(state, repo_id, result),
        Msg::RemoteTagsLoaded { repo_id, result } => {
            effects::remote_tags_loaded(state, repo_id, result)
        }
        Msg::StashesLoaded { repo_id, result } => effects::stashes_loaded(state, repo_id, result),
        Msg::ReflogLoaded { repo_id, result } => effects::reflog_loaded(state, repo_id, result),
        Msg::RebaseStateLoaded { repo_id, result } => {
            external_and_history::rebase_state_loaded(state, repo_id, result)
        }
        Msg::MergeCommitMessageLoaded { repo_id, result } => {
            external_and_history::merge_commit_message_loaded(state, repo_id, result)
        }
        Msg::FileHistoryLoaded {
            repo_id,
            path,
            result,
        } => effects::file_history_loaded(state, repo_id, path, result),
        Msg::BlameLoaded {
            repo_id,
            path,
            rev,
            result,
        } => effects::blame_loaded(state, repo_id, path, rev, result),
        Msg::ConflictFileLoaded {
            repo_id,
            path,
            result,
            conflict_session,
        } => effects::conflict_file_loaded(state, repo_id, path, *result, conflict_session),
        Msg::WorktreesLoaded { repo_id, result } => {
            effects::worktrees_loaded(state, repo_id, result)
        }
        Msg::SubmodulesLoaded { repo_id, result } => {
            effects::submodules_loaded(state, repo_id, result)
        }
        Msg::CommitDetailsLoaded {
            repo_id,
            commit_id,
            result,
        } => effects::commit_details_loaded(state, repo_id, commit_id, result),
        Msg::DiffLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::diff_loaded(state, repo_id, target, result),
        Msg::DiffFileLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::diff_file_loaded(state, repo_id, target, result),
        Msg::DiffFileImageLoaded {
            repo_id,
            target,
            result,
        } => diff_selection::diff_file_image_loaded(state, repo_id, target, result),
        Msg::RepoActionFinished { repo_id, result } => {
            external_and_history::repo_action_finished(state, repo_id, result)
        }
        Msg::CommitFinished { repo_id, result } => {
            actions_emit_effects::commit_finished(state, repo_id, result)
        }
        Msg::CommitAmendFinished { repo_id, result } => {
            actions_emit_effects::commit_amend_finished(state, repo_id, result)
        }
        Msg::RepoCommandFinished {
            repo_id,
            command,
            result,
        } => {
            let removed_worktree_path = match (&command, &result) {
                (RepoCommandKind::RemoveWorktree { path }, Ok(_)) => Some(path.clone()),
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

            effects
        }
    }
}
