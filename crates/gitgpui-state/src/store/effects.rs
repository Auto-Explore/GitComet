mod clone;
mod open_repo;
mod repo_actions;
mod repo_commands;
mod repo_load;
mod util;

use crate::msg::{Effect, Msg};
use gitgpui_core::services::{GitBackend, GitRepository};
use rustc_hash::FxHashMap as HashMap;
use std::sync::{Arc, mpsc};

use super::RepoId;
use super::executor::TaskExecutor;

pub(super) fn schedule_effect(
    executor: &TaskExecutor,
    backend: &Arc<dyn GitBackend>,
    repos: &HashMap<RepoId, Arc<dyn GitRepository>>,
    msg_tx: mpsc::Sender<Msg>,
    effect: Effect,
) {
    match effect {
        Effect::OpenRepo { repo_id, path } => {
            open_repo::schedule_open_repo(executor, Arc::clone(backend), msg_tx, repo_id, path);
        }
        Effect::LoadBranches { repo_id } => {
            repo_load::schedule_load_branches(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRemotes { repo_id } => {
            repo_load::schedule_load_remotes(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRemoteBranches { repo_id } => {
            repo_load::schedule_load_remote_branches(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadStatus { repo_id } => {
            repo_load::schedule_load_status(executor, repos, msg_tx, repo_id)
        }
        Effect::LoadHeadBranch { repo_id } => {
            repo_load::schedule_load_head_branch(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadUpstreamDivergence { repo_id } => {
            repo_load::schedule_load_upstream_divergence(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadLog {
            repo_id,
            scope,
            limit,
            cursor,
        } => repo_load::schedule_load_log(executor, repos, msg_tx, repo_id, scope, limit, cursor),
        Effect::LoadTags { repo_id } => {
            repo_load::schedule_load_tags(executor, repos, msg_tx, repo_id)
        }
        Effect::LoadStashes { repo_id, limit } => {
            repo_load::schedule_load_stashes(executor, repos, msg_tx, repo_id, limit);
        }
        Effect::LoadConflictFile { repo_id, path } => {
            repo_load::schedule_load_conflict_file(executor, repos, msg_tx, repo_id, path);
        }
        Effect::LoadReflog { repo_id, limit } => {
            repo_load::schedule_load_reflog(executor, repos, msg_tx, repo_id, limit);
        }
        Effect::SaveWorktreeFile {
            repo_id,
            path,
            contents,
            stage,
        } => repo_commands::schedule_save_worktree_file(
            executor, repos, msg_tx, repo_id, path, contents, stage,
        ),
        Effect::LoadFileHistory {
            repo_id,
            path,
            limit,
        } => repo_load::schedule_load_file_history(executor, repos, msg_tx, repo_id, path, limit),
        Effect::LoadBlame { repo_id, path, rev } => {
            repo_load::schedule_load_blame(executor, repos, msg_tx, repo_id, path, rev);
        }
        Effect::LoadWorktrees { repo_id } => {
            repo_load::schedule_load_worktrees(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadSubmodules { repo_id } => {
            repo_load::schedule_load_submodules(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadRebaseState { repo_id } => {
            repo_load::schedule_load_rebase_state(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadMergeCommitMessage { repo_id } => {
            repo_load::schedule_load_merge_commit_message(executor, repos, msg_tx, repo_id);
        }
        Effect::LoadCommitDetails { repo_id, commit_id } => {
            repo_load::schedule_load_commit_details(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::LoadDiff { repo_id, target } => {
            repo_load::schedule_load_diff(executor, repos, msg_tx, repo_id, target);
        }
        Effect::LoadDiffFile { repo_id, target } => {
            repo_load::schedule_load_diff_file(executor, repos, msg_tx, repo_id, target);
        }
        Effect::LoadDiffFileImage { repo_id, target } => {
            repo_load::schedule_load_diff_file_image(executor, repos, msg_tx, repo_id, target);
        }
        Effect::CheckoutBranch { repo_id, name } => {
            repo_actions::schedule_checkout_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::CheckoutRemoteBranch {
            repo_id,
            remote,
            branch,
            local_branch,
        } => repo_actions::schedule_checkout_remote_branch(
            executor,
            repos,
            msg_tx,
            repo_id,
            remote,
            branch,
            local_branch,
        ),
        Effect::CheckoutCommit { repo_id, commit_id } => {
            repo_actions::schedule_checkout_commit(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::CherryPickCommit { repo_id, commit_id } => {
            repo_actions::schedule_cherry_pick_commit(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::RevertCommit { repo_id, commit_id } => {
            repo_actions::schedule_revert_commit(executor, repos, msg_tx, repo_id, commit_id);
        }
        Effect::CreateBranch { repo_id, name } => {
            repo_actions::schedule_create_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::CreateBranchAndCheckout { repo_id, name } => {
            repo_actions::schedule_create_branch_and_checkout(
                executor, repos, msg_tx, repo_id, name,
            );
        }
        Effect::DeleteBranch { repo_id, name } => {
            repo_actions::schedule_delete_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::ForceDeleteBranch { repo_id, name } => {
            repo_actions::schedule_force_delete_branch(executor, repos, msg_tx, repo_id, name);
        }
        Effect::CloneRepo { url, dest } => clone::schedule_clone_repo(executor, msg_tx, url, dest),
        Effect::ExportPatch {
            repo_id,
            commit_id,
            dest,
        } => {
            repo_commands::schedule_export_patch(executor, repos, msg_tx, repo_id, commit_id, dest)
        }
        Effect::ApplyPatch { repo_id, patch } => {
            repo_commands::schedule_apply_patch(executor, repos, msg_tx, repo_id, patch);
        }
        Effect::AddWorktree {
            repo_id,
            path,
            reference,
        } => {
            repo_commands::schedule_add_worktree(executor, repos, msg_tx, repo_id, path, reference)
        }
        Effect::RemoveWorktree { repo_id, path } => {
            repo_commands::schedule_remove_worktree(executor, repos, msg_tx, repo_id, path);
        }
        Effect::AddSubmodule { repo_id, url, path } => {
            repo_commands::schedule_add_submodule(executor, repos, msg_tx, repo_id, url, path);
        }
        Effect::UpdateSubmodules { repo_id } => {
            repo_commands::schedule_update_submodules(executor, repos, msg_tx, repo_id);
        }
        Effect::RemoveSubmodule { repo_id, path } => {
            repo_commands::schedule_remove_submodule(executor, repos, msg_tx, repo_id, path);
        }
        Effect::StageHunk { repo_id, patch } => {
            repo_commands::schedule_stage_hunk(executor, repos, msg_tx, repo_id, patch);
        }
        Effect::UnstageHunk { repo_id, patch } => {
            repo_commands::schedule_unstage_hunk(executor, repos, msg_tx, repo_id, patch);
        }
        Effect::ApplyWorktreePatch {
            repo_id,
            patch,
            reverse,
        } => repo_commands::schedule_apply_worktree_patch(
            executor, repos, msg_tx, repo_id, patch, reverse,
        ),
        Effect::StagePath { repo_id, path } => {
            repo_actions::schedule_stage_path(executor, repos, msg_tx, repo_id, path);
        }
        Effect::StagePaths { repo_id, paths } => {
            repo_actions::schedule_stage_paths(executor, repos, msg_tx, repo_id, paths);
        }
        Effect::UnstagePath { repo_id, path } => {
            repo_actions::schedule_unstage_path(executor, repos, msg_tx, repo_id, path);
        }
        Effect::UnstagePaths { repo_id, paths } => {
            repo_actions::schedule_unstage_paths(executor, repos, msg_tx, repo_id, paths);
        }
        Effect::DiscardWorktreeChangesPath { repo_id, path } => {
            repo_actions::schedule_discard_worktree_changes_path(
                executor, repos, msg_tx, repo_id, path,
            );
        }
        Effect::DiscardWorktreeChangesPaths { repo_id, paths } => {
            repo_actions::schedule_discard_worktree_changes_paths(
                executor, repos, msg_tx, repo_id, paths,
            )
        }
        Effect::Commit { repo_id, message } => {
            repo_actions::schedule_commit(executor, repos, msg_tx, repo_id, message);
        }
        Effect::CommitAmend { repo_id, message } => {
            repo_actions::schedule_commit_amend(executor, repos, msg_tx, repo_id, message);
        }
        Effect::FetchAll { repo_id, prune } => {
            repo_commands::schedule_fetch_all(executor, repos, msg_tx, repo_id, prune)
        }
        Effect::Pull { repo_id, mode } => {
            repo_commands::schedule_pull(executor, repos, msg_tx, repo_id, mode)
        }
        Effect::PullBranch {
            repo_id,
            remote,
            branch,
        } => repo_commands::schedule_pull_branch(executor, repos, msg_tx, repo_id, remote, branch),
        Effect::MergeRef { repo_id, reference } => {
            repo_commands::schedule_merge_ref(executor, repos, msg_tx, repo_id, reference);
        }
        Effect::Push { repo_id } => repo_commands::schedule_push(executor, repos, msg_tx, repo_id),
        Effect::ForcePush { repo_id } => {
            repo_commands::schedule_force_push(executor, repos, msg_tx, repo_id)
        }
        Effect::PushSetUpstream {
            repo_id,
            remote,
            branch,
        } => repo_commands::schedule_push_set_upstream(
            executor, repos, msg_tx, repo_id, remote, branch,
        ),
        Effect::DeleteRemoteBranch {
            repo_id,
            remote,
            branch,
        } => repo_commands::schedule_delete_remote_branch(
            executor, repos, msg_tx, repo_id, remote, branch,
        ),
        Effect::Reset {
            repo_id,
            target,
            mode,
        } => repo_commands::schedule_reset(executor, repos, msg_tx, repo_id, target, mode),
        Effect::Rebase { repo_id, onto } => {
            repo_commands::schedule_rebase(executor, repos, msg_tx, repo_id, onto)
        }
        Effect::RebaseContinue { repo_id } => {
            repo_commands::schedule_rebase_continue(executor, repos, msg_tx, repo_id);
        }
        Effect::RebaseAbort { repo_id } => {
            repo_commands::schedule_rebase_abort(executor, repos, msg_tx, repo_id)
        }
        Effect::MergeAbort { repo_id } => {
            repo_commands::schedule_merge_abort(executor, repos, msg_tx, repo_id)
        }
        Effect::CreateTag {
            repo_id,
            name,
            target,
        } => repo_commands::schedule_create_tag(executor, repos, msg_tx, repo_id, name, target),
        Effect::DeleteTag { repo_id, name } => {
            repo_commands::schedule_delete_tag(executor, repos, msg_tx, repo_id, name);
        }
        Effect::AddRemote { repo_id, name, url } => {
            repo_commands::schedule_add_remote(executor, repos, msg_tx, repo_id, name, url);
        }
        Effect::RemoveRemote { repo_id, name } => {
            repo_commands::schedule_remove_remote(executor, repos, msg_tx, repo_id, name);
        }
        Effect::SetRemoteUrl {
            repo_id,
            name,
            url,
            kind,
        } => repo_commands::schedule_set_remote_url(
            executor, repos, msg_tx, repo_id, name, url, kind,
        ),
        Effect::CheckoutConflictSide {
            repo_id,
            path,
            side,
        } => repo_commands::schedule_checkout_conflict_side(
            executor, repos, msg_tx, repo_id, path, side,
        ),
        Effect::LaunchMergetool { repo_id, path } => {
            repo_commands::schedule_launch_mergetool(executor, repos, msg_tx, repo_id, path);
        }
        Effect::Stash {
            repo_id,
            message,
            include_untracked,
        } => repo_actions::schedule_stash(
            executor,
            repos,
            msg_tx,
            repo_id,
            message,
            include_untracked,
        ),
        Effect::ApplyStash { repo_id, index } => {
            repo_actions::schedule_apply_stash(executor, repos, msg_tx, repo_id, index);
        }
        Effect::DropStash { repo_id, index } => {
            repo_actions::schedule_drop_stash(executor, repos, msg_tx, repo_id, index);
        }
        Effect::PopStash { repo_id, index } => {
            repo_actions::schedule_pop_stash(executor, repos, msg_tx, repo_id, index);
        }
    }
}
