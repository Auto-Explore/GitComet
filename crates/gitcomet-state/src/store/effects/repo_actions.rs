use crate::msg::Msg;
use gitcomet_core::error::Error;
use gitcomet_core::services::GitRepository;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc};

use super::super::{executor::TaskExecutor, RepoId};
use super::util::{send_or_log, spawn_with_repo, RepoMap};

fn schedule_repo_action_with_hook<F, H, M>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    run: F,
    hook: H,
    finish: M,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<(), Error> + Send + 'static,
    H: FnOnce(&mpsc::Sender<Msg>, RepoId, &Result<(), Error>) + Send + 'static,
    M: FnOnce(RepoId, Result<(), Error>) -> Msg + Send + 'static,
{
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = run(repo);
        hook(&msg_tx, repo_id, &result);
        send_or_log(&msg_tx, finish(repo_id, result));
    });
}

fn schedule_repo_action<F>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
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
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}

fn send_refresh_branches_on_success(
    msg_tx: &mpsc::Sender<Msg>,
    repo_id: RepoId,
    result: &Result<(), Error>,
) {
    if result.is_ok() {
        send_or_log(msg_tx, Msg::RefreshBranches { repo_id });
    }
}

fn dedup_paths(mut paths: Vec<PathBuf>) -> Vec<PathBuf> {
    paths.sort();
    paths.dedup();
    paths
}

pub(super) fn schedule_checkout_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        repo.checkout_branch(&name)
    });
}

pub(super) fn schedule_checkout_remote_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        send_refresh_branches_on_success,
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}

pub(super) fn schedule_checkout_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        repo.checkout_commit(&commit_id)
    });
}

pub(super) fn schedule_cherry_pick_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        repo.cherry_pick(&commit_id)
    });
}

pub(super) fn schedule_revert_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        repo.revert(&commit_id)
    });
}

pub(super) fn schedule_create_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| {
            let target = gitcomet_core::domain::CommitId("HEAD".to_string());
            repo.create_branch(&name, &target)
        },
        send_refresh_branches_on_success,
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}

pub(super) fn schedule_create_branch_and_checkout(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    name: String,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let target = gitcomet_core::domain::CommitId("HEAD".to_string());
        let created = repo.create_branch(&name, &target);
        let refresh = created.is_ok();
        let result = created.and_then(|()| repo.checkout_branch(&name));
        if refresh {
            send_or_log(&msg_tx, Msg::RefreshBranches { repo_id });
        }
        send_or_log(&msg_tx, Msg::RepoActionFinished { repo_id, result });
    });
}

pub(super) fn schedule_delete_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}

pub(super) fn schedule_force_delete_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}

pub(super) fn schedule_stage_path(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        let path_ref: &Path = &path;
        repo.stage(&[path_ref])
    });
}

pub(super) fn schedule_stage_paths(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    paths: Vec<PathBuf>,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        let unique = dedup_paths(paths);
        let refs = unique.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        repo.stage(&refs)
    });
}

pub(super) fn schedule_unstage_path(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        let path_ref: &Path = &path;
        repo.unstage(&[path_ref])
    });
}

pub(super) fn schedule_unstage_paths(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    paths: Vec<PathBuf>,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        let unique = dedup_paths(paths);
        let refs = unique.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        repo.unstage(&refs)
    });
}

pub(super) fn schedule_discard_worktree_changes_path(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    path: PathBuf,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        let path_ref: &Path = &path;
        repo.discard_worktree_changes(&[path_ref])
    });
}

pub(super) fn schedule_discard_worktree_changes_paths(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    paths: Vec<PathBuf>,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        let unique = dedup_paths(paths);
        let refs = unique.iter().map(|p| p.as_path()).collect::<Vec<_>>();
        repo.discard_worktree_changes(&refs)
    });
}

pub(super) fn schedule_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    message: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.commit(&message),
        |_msg_tx, _repo_id, _result| {},
        |repo_id, result| Msg::CommitFinished { repo_id, result },
    );
}

pub(super) fn schedule_commit_amend(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    message: String,
) {
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        move |repo| repo.commit_amend(&message),
        |_msg_tx, _repo_id, _result| {},
        |repo_id, result| Msg::CommitAmendFinished { repo_id, result },
    );
}

pub(super) fn schedule_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}

pub(super) fn schedule_apply_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
    repo_id: RepoId,
    index: usize,
) {
    schedule_repo_action(executor, repos, msg_tx, repo_id, move |repo| {
        repo.stash_apply(index)
    });
}

pub(super) fn schedule_pop_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
                send_or_log(&msg_tx, Msg::RepoActionFinished { repo_id, result });
            }
            Err(err) => {
                send_or_log(
                    &msg_tx,
                    Msg::RepoActionFinished {
                        repo_id,
                        result: Err(err),
                    },
                );
            }
        },
    );
}

pub(super) fn schedule_drop_stash(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: mpsc::Sender<Msg>,
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
        |repo_id, result| Msg::RepoActionFinished { repo_id, result },
    );
}
