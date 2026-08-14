use crate::msg::{Msg, RepoActionKind, RepoPathList};
use gitcomet_core::auth::{
    StagedGitAuth, clear_staged_git_auth, stage_git_auth_for_current_thread,
};
use gitcomet_core::error::Error;
use gitcomet_core::services::GitRepository;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::{RepoId, executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::{RepoMap, send_or_log, spawn_with_repo};

fn schedule_repo_action_with_hook<F, H, M>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    run: F,
    hook: H,
    finish: M,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<(), Error> + Send + 'static,
    H: FnOnce(&StoreWorkerSender, RepoId, &Result<(), Error>) + Send + 'static,
    M: FnOnce(RepoId, Result<(), Error>) -> Msg + Send + 'static,
{
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = run(repo);
        hook(&msg_tx, repo_id, &result);
        send_or_log(&msg_tx, finish(repo_id, result));
    });
}

fn schedule_repo_action_with_result<T, F, M>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    run: F,
    finish: M,
) where
    T: Send + 'static,
    F: FnOnce(Arc<dyn GitRepository>) -> Result<T, Error> + Send + 'static,
    M: FnOnce(RepoId, Result<T, Error>) -> Msg + Send + 'static,
{
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = run(repo);
        send_or_log(&msg_tx, finish(repo_id, result));
    });
}

fn repo_action_finished(
    action: RepoActionKind,
) -> impl FnOnce(RepoId, Result<(), Error>) -> Msg + Send + 'static {
    move |repo_id, result| {
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action,
            result,
        })
    }
}

fn schedule_repo_action<F>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    action: RepoActionKind,
    run: F,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<(), Error> + Send + 'static,
{
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        run,
        |_msg_tx, _repo_id, _result| {},
        repo_action_finished(action),
    );
}

fn send_refresh_branches_on_success(
    msg_tx: &StoreWorkerSender,
    repo_id: RepoId,
    result: &Result<(), Error>,
) {
    if result.is_ok() {
        send_or_log(msg_tx, Msg::RefreshBranches { repo_id });
    }
}

fn send_load_worktrees_on_success(
    msg_tx: &StoreWorkerSender,
    repo_id: RepoId,
    result: &Result<(), Error>,
) {
    if result.is_ok() {
        send_or_log(msg_tx, Msg::LoadWorktrees { repo_id });
        send_or_log(msg_tx, Msg::LoadWorktreeDirty { repo_id });
    }
}

fn send_refresh_branches_and_load_worktrees_on_success(
    msg_tx: &StoreWorkerSender,
    repo_id: RepoId,
    result: &Result<(), Error>,
) {
    send_refresh_branches_on_success(msg_tx, repo_id, result);
    send_load_worktrees_on_success(msg_tx, repo_id, result);
}

fn dedup_paths(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    paths
}

fn run_with_git_auth<R>(
    auth: Option<StagedGitAuth>,
    run: impl FnOnce() -> Result<R, Error>,
) -> Result<R, Error> {
    if let Some(auth) = auth {
        stage_git_auth_for_current_thread(auth);
        let result = run();
        clear_staged_git_auth();
        result
    } else {
        run()
    }
}

pub(super) fn schedule_checkout_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.checkout_branch(&name),
        send_refresh_branches_and_load_worktrees_on_success,
        repo_action_finished(RepoActionKind::CheckoutBranch),
    );
}

pub(super) fn schedule_checkout_remote_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    branch: String,
    local_branch: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.checkout_remote_branch(&remote, &branch, &local_branch),
        send_refresh_branches_and_load_worktrees_on_success,
        repo_action_finished(RepoActionKind::CheckoutRemoteBranch),
    );
}

pub(super) fn schedule_checkout_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.checkout_commit(&commit_id),
        send_load_worktrees_on_success,
        repo_action_finished(RepoActionKind::CheckoutCommit),
    );
}

pub(super) fn schedule_revert_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::RevertCommit,
        move |repo| repo.revert(&commit_id),
    );
}

pub(super) fn schedule_create_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
    target: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| {
            let target = gitcomet_core::domain::CommitId(target.into());
            repo.create_branch(&name, &target)
        },
        send_refresh_branches_on_success,
        repo_action_finished(RepoActionKind::CreateBranch),
    );
}

pub(super) fn schedule_create_branch_and_checkout(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
    target: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let target = gitcomet_core::domain::CommitId(target.into());
        let created = repo.create_branch(&name, &target);
        let refresh = created.is_ok();
        let result = created.and_then(|()| repo.checkout_branch(&name));
        if refresh {
            send_or_log(&msg_tx, Msg::RefreshBranches { repo_id });
        }
        if result.is_ok() {
            send_or_log(&msg_tx, Msg::LoadWorktrees { repo_id });
            send_or_log(&msg_tx, Msg::LoadWorktreeDirty { repo_id });
        }
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id,
                action: RepoActionKind::CreateBranchAndCheckout,
                result,
            }),
        );
    });
}

pub(super) fn schedule_rename_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    old_name: String,
    new_name: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.rename_branch(&old_name, &new_name),
        send_refresh_branches_and_load_worktrees_on_success,
        repo_action_finished(RepoActionKind::RenameBranch),
    );
}

pub(super) fn schedule_delete_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.delete_branch(&name),
        send_refresh_branches_on_success,
        repo_action_finished(RepoActionKind::DeleteBranch),
    );
}

/// Delete a batch of local branches in one action.
///
/// Loops the existing single-branch backend calls rather than adding a batch
/// trait method: local deletes touch only refs, so there is no round trip to
/// save, and looping keeps every backend working unchanged.
///
/// A partial failure is the normal case, not an exception — some branches in a
/// group are merged and some are not — so every name is attempted and the
/// failures are summarised into one error rather than aborting on the first.
pub(super) fn schedule_delete_branches(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    names: Vec<String>,
    force: bool,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| {
            let total = names.len();
            let mut failed: Vec<String> = Vec::new();
            let mut first_error: Option<String> = None;
            for name in &names {
                let result = if force {
                    repo.delete_branch_force(name)
                } else {
                    repo.delete_branch(name)
                };
                if let Err(err) = result {
                    failed.push(name.clone());
                    first_error.get_or_insert_with(|| err.to_string());
                }
            }

            if failed.is_empty() {
                return Ok(());
            }
            Err(Error::new(gitcomet_core::error::ErrorKind::Backend(
                delete_branches_failure_message(total, &failed, force, first_error.as_deref()),
            )))
        },
        // Refresh even on a partial failure: the branches that did get deleted
        // are gone, and leaving them in the sidebar would be a lie.
        |msg_tx, repo_id, _result| send_or_log(msg_tx, Msg::RefreshBranches { repo_id }),
        repo_action_finished(RepoActionKind::DeleteBranches),
    );
}

/// Summarises which branches survived a batch delete, and why.
///
/// Carries git's own first error: "not fully merged" is the common cause but
/// not the only one — a branch checked out in a linked worktree fails too, and
/// pointing that user at Force delete would send them round the same loop.
///
/// The name list is elided through [`crate::name_summary`], so this toast stops
/// at the same count and in the same words as the confirm dialog that produced
/// the delete.
fn delete_branches_failure_message(
    total: usize,
    failed: &[String],
    force: bool,
    first_error: Option<&str>,
) -> String {
    let deleted = total - failed.len();
    let noun = crate::name_summary::branch_noun(total);
    let names = crate::name_summary::elide_names(failed, ", ");
    let mut message = format!("Deleted {deleted} of {total} {noun}. Failed: {names}");
    if let Some(error) = first_error.map(str::trim).filter(|error| !error.is_empty()) {
        message.push_str(&format!(". {error}"));
    }
    if !force {
        message.push_str(". Branches that are not fully merged need Force delete.");
    }
    message
}

pub(super) fn schedule_force_delete_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.delete_branch_force(&name),
        send_refresh_branches_on_success,
        repo_action_finished(RepoActionKind::ForceDeleteBranch),
    );
}

pub(super) fn schedule_stage_path(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::StagePath,
        move |repo| {
            let path_ref: &Path = &path;
            repo.stage(&[path_ref])
        },
    );
}

pub(super) fn schedule_stage_paths(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    paths: RepoPathList,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::StagePaths,
        move |repo| {
            let unique = dedup_paths(paths.as_slice().to_vec());
            let refs = unique.iter().map(|p| p.as_path()).collect::<Vec<_>>();
            repo.stage(&refs)
        },
    );
}

pub(super) fn schedule_unstage_path(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::UnstagePath,
        move |repo| {
            let path_ref: &Path = &path;
            repo.unstage(&[path_ref])
        },
    );
}

pub(super) fn schedule_unstage_paths(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    paths: RepoPathList,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::UnstagePaths,
        move |repo| {
            let unique = dedup_paths(paths.as_slice().to_vec());
            let refs = unique.iter().map(|p| p.as_path()).collect::<Vec<_>>();
            repo.unstage(&refs)
        },
    );
}

pub(super) fn schedule_discard_worktree_changes_path(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::DiscardWorktreeChangesPath,
        move |repo| {
            let path_ref: &Path = &path;
            repo.discard_worktree_changes(&[path_ref])
        },
    );
}

pub(super) fn schedule_discard_worktree_changes_paths(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    paths: Vec<PathBuf>,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::DiscardWorktreeChangesPaths,
        move |repo| {
            let unique = dedup_paths(paths);
            let refs = unique.iter().map(|p| p.as_path()).collect::<Vec<_>>();
            repo.discard_worktree_changes(&refs)
        },
    );
}

pub(super) fn schedule_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    message: String,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_action_with_result(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| run_with_git_auth(auth, || repo.commit_with_outcome(&message)),
        |repo_id, result| {
            Msg::Internal(crate::msg::InternalMsg::CommitFinished { repo_id, result })
        },
    );
}

pub(super) fn schedule_commit_amend(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    message: String,
    auth: Option<StagedGitAuth>,
) {
    schedule_repo_action_with_result(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| run_with_git_auth(auth, || repo.commit_amend_with_outcome(&message)),
        |repo_id, result| {
            Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished { repo_id, result })
        },
    );
}

pub(super) fn schedule_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    message: String,
    include_untracked: bool,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.stash_create(&message, include_untracked),
        |msg_tx, repo_id, result| {
            if result.is_ok() {
                send_or_log(msg_tx, Msg::LoadStashes { repo_id });
            }
        },
        repo_action_finished(RepoActionKind::Stash),
    );
}

pub(super) fn schedule_apply_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    index: usize,
) {
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::ApplyStash,
        move |repo| repo.stash_apply(index),
    );
}

pub(super) fn schedule_pop_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    index: usize,
) {
    spawn_with_repo(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| match repo.stash_apply(index) {
            Ok(()) => {
                let result = repo.stash_drop(index);
                send_or_log(&msg_tx, Msg::LoadStashes { repo_id });
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                        repo_id,
                        action: RepoActionKind::PopStash,
                        result,
                    }),
                );
            }
            Err(err) => {
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                        repo_id,
                        action: RepoActionKind::PopStash,
                        result: Err(err),
                    }),
                );
            }
        },
    );
}

pub(super) fn schedule_drop_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    index: usize,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.stash_drop(index),
        |msg_tx, repo_id, _result| {
            send_or_log(msg_tx, Msg::LoadStashes { repo_id });
        },
        repo_action_finished(RepoActionKind::DropStash),
    );
}

#[cfg(test)]
mod delete_branches_tests {
    use super::delete_branches_failure_message;

    #[test]
    fn failure_message_reports_how_many_survived_and_names_them() {
        let failed = vec!["feat/a".to_string(), "feat/b".to_string()];
        let message = delete_branches_failure_message(5, &failed, false, None);

        assert!(message.starts_with("Deleted 3 of 5 branches. Failed: feat/a, feat/b"));
        // The unforced attempt has to point at the remedy, since a group of
        // finished feature branches fails exactly this way.
        assert!(message.contains("need Force delete"));
    }

    /// Force already failed, so repeating the Force hint would send the user
    /// round the same loop.
    #[test]
    fn failure_message_omits_the_force_hint_when_force_was_already_used() {
        let failed = vec!["feat/a".to_string()];
        let message = delete_branches_failure_message(2, &failed, true, None);

        assert!(!message.contains("Force delete"));
    }

    /// "Not fully merged" is the common cause but not the only one — a branch
    /// checked out in a linked worktree fails too, and the hint alone would
    /// point at the wrong remedy.
    #[test]
    fn failure_message_carries_gits_own_first_error() {
        let failed = vec!["feat/a".to_string()];
        let message = delete_branches_failure_message(
            1,
            &failed,
            true,
            Some("branch is checked out at /tmp/wt"),
        );

        assert!(message.contains("branch is checked out at /tmp/wt"));
    }

    /// Elided through `name_summary`, so the toast stops where the confirm
    /// dialog's own list stopped rather than at a second, private cap.
    #[test]
    fn failure_message_truncates_a_long_failure_list() {
        let failed: Vec<String> = (0..12).map(|ix| format!("feat/{ix}")).collect();
        let message = delete_branches_failure_message(12, &failed, true, None);

        assert!(message.contains("feat/7"), "the first eight are named");
        assert!(!message.contains("feat/8"), "the rest are summarised");
        assert!(message.contains("…and 4 more"), "got {message}");
    }

    /// Every other count message in this flow pluralises; a one-branch group
    /// that fails must not report "Deleted 0 of 1 branches".
    #[test]
    fn failure_message_is_singular_for_a_single_branch() {
        let failed = vec!["feat/a".to_string()];
        let message = delete_branches_failure_message(1, &failed, true, None);

        assert!(
            message.starts_with("Deleted 0 of 1 branch. Failed: feat/a"),
            "got {message}"
        );
    }

    /// A blank error string must not leave a dangling ". ." in the message.
    #[test]
    fn failure_message_ignores_a_blank_error() {
        let failed = vec!["feat/a".to_string()];
        let message = delete_branches_failure_message(1, &failed, true, Some("   "));

        assert!(!message.contains(". ."));
        assert!(message.ends_with("feat/a"));
    }
}
