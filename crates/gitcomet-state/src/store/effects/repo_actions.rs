use crate::model::{BranchExistsPromptOperation, BranchExistsPromptState};
use crate::msg::{InternalMsg, Msg, RepoActionKind, RepoPathList};
use gitcomet_core::auth::{ScopedStagedGitAuth, StagedGitAuth};
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use gitcomet_core::services::{CheckoutRemoteBranchMode, GitBackend, GitRepository};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::super::{RepoId, executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::{
    GitOperationTask, RepoMap, message_subject, path_context, paths_context, send_or_log,
    short_commit_id, single_line_context, spawn_with_repo,
};

fn schedule_repo_action_with_hook<F, H, M>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    action: RepoActionKind,
    context: Option<String>,
    run: F,
    hook: H,
    finish: M,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<(), Error> + Send + 'static,
    H: FnOnce(&StoreWorkerSender, RepoId, &Result<(), Error>) + Send + 'static,
    M: FnOnce(RepoId, Result<(), Error>) -> InternalMsg + Send + 'static,
{
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation =
            GitOperationTask::start(repo_id, action.hook_activity_label(), context, &msg_tx);
        let result = {
            let _scope = operation.attach();
            run(repo)
        };
        hook(&msg_tx, repo_id, &result);
        let outcome = GitOperationTask::outcome(&result);
        let message = finish(repo_id, result);
        operation.finish(outcome, message);
    });
}

fn schedule_repo_action_with_result<T, F, M>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    label: &'static str,
    context: Option<String>,
    run: F,
    finish: M,
) where
    T: Send + 'static,
    F: FnOnce(Arc<dyn GitRepository>) -> Result<T, Error> + Send + 'static,
    M: FnOnce(RepoId, Result<T, Error>) -> InternalMsg + Send + 'static,
{
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation = GitOperationTask::start(repo_id, label, context, &msg_tx);
        let result = {
            let _scope = operation.attach();
            run(repo)
        };
        let outcome = GitOperationTask::outcome(&result);
        let message = finish(repo_id, result);
        operation.finish(outcome, message);
    });
}

fn repo_action_finished(
    action: RepoActionKind,
) -> impl FnOnce(RepoId, Result<(), Error>) -> InternalMsg + Send + 'static {
    move |repo_id, result| InternalMsg::RepoActionFinished {
        repo_id,
        action,
        result,
    }
}

fn schedule_repo_action<F>(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    action: RepoActionKind,
    context: Option<String>,
    run: F,
) where
    F: FnOnce(Arc<dyn GitRepository>) -> Result<(), Error> + Send + 'static,
{
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        action,
        context,
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
        // The guard clears the staged auth on success, error, and panic.
        let _scoped = ScopedStagedGitAuth::stage(auth);
        run()
    } else {
        run()
    }
}

/// Where an action on a branch runs: here, or in the other worktree that has
/// the branch checked out (git refuses to check a branch out twice).
enum BranchActionHost {
    Here,
    OtherWorktree(PathBuf),
}

/// What to do when the branch lives in another worktree.
#[derive(Clone, Copy)]
enum WorktreeRedirect {
    /// Run nothing; just report the worktree so it gets opened.
    OpenOnly,
    /// Run the action through a handle opened at that worktree, then open it.
    RunThere,
}

fn branch_action_host(repo: &dyn GitRepository, branch: &str) -> Result<BranchActionHost, Error> {
    Ok(match repo.branch_checked_out_in_other_worktree(branch)? {
        Some(path) => BranchActionHost::OtherWorktree(path),
        None => BranchActionHost::Here,
    })
}

/// Opens `path` and checks it is a different worktree that still has `branch`
/// checked out, so the action never lands somewhere unexpected.
fn open_worktree_holding_branch(
    backend: &dyn GitBackend,
    origin: &dyn GitRepository,
    path: &Path,
    branch: &str,
) -> Result<Arc<dyn GitRepository>, Error> {
    let handle = backend.open(path)?;
    if handle.spec().workdir == origin.spec().workdir {
        return Err(Error::new(ErrorKind::Backend(format!(
            "worktree at '{}' is this repository's own working directory",
            path.display()
        ))));
    }
    let current = handle.current_branch()?;
    if current != branch {
        return Err(Error::new(ErrorKind::Backend(format!(
            "the worktree at '{}' no longer has branch '{branch}' checked out",
            path.display()
        ))));
    }
    Ok(handle)
}

fn is_branch_already_exists(result: &Result<(), Error>) -> bool {
    matches!(
        result,
        Err(error) if matches!(
            error.kind(),
            ErrorKind::Git(failure) if failure.id() == GitFailureId::BranchAlreadyExists
        )
    )
}

/// Finish an action that hit an existing branch: refresh the possibly stale
/// branch list and let shared state open the collision prompt.
fn finish_branch_collision(
    operation: GitOperationTask,
    msg_tx: &StoreWorkerSender,
    action: RepoActionKind,
    prompt: BranchExistsPromptState,
    result: &Result<(), Error>,
) {
    send_or_log(
        msg_tx,
        Msg::RefreshBranches {
            repo_id: prompt.repo_id,
        },
    );
    let outcome = GitOperationTask::outcome(result);
    operation.finish(outcome, InternalMsg::BranchAlreadyExists { action, prompt });
}

/// Runs `run` on `branch` where git allows it, redirecting to the worktree
/// that has the branch checked out.
#[allow(clippy::too_many_arguments)]
fn schedule_branch_action(
    executor: &TaskExecutor,
    repos: &RepoMap,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    action: RepoActionKind,
    context: Option<String>,
    branch: String,
    redirect: WorktreeRedirect,
    run: impl FnOnce(&dyn GitRepository) -> Result<(), Error> + Send + 'static,
) {
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation =
            GitOperationTask::start(repo_id, action.hook_activity_label(), context, &msg_tx);
        let (host, result, ran) = {
            let _scope = operation.attach();
            match branch_action_host(&*repo, &branch) {
                Err(err) => (BranchActionHost::Here, Err(err), false),
                Ok(BranchActionHost::Here) => (BranchActionHost::Here, run(&*repo), true),
                Ok(BranchActionHost::OtherWorktree(path)) => {
                    let (result, ran) = match redirect {
                        WorktreeRedirect::OpenOnly => (Ok(()), false),
                        WorktreeRedirect::RunThere => (
                            open_worktree_holding_branch(&*backend, &*repo, &path, &branch)
                                .and_then(|handle| run(&*handle)),
                            true,
                        ),
                    };
                    (BranchActionHost::OtherWorktree(path), result, ran)
                }
            }
        };
        if ran {
            send_refresh_branches_and_load_worktrees_on_success(&msg_tx, repo_id, &result);
        }
        let outcome = GitOperationTask::outcome(&result);
        let message = match host {
            BranchActionHost::Here => InternalMsg::RepoActionFinished {
                repo_id,
                action,
                result,
            },
            BranchActionHost::OtherWorktree(worktree_path) => {
                InternalMsg::RepoActionFinishedInWorktree {
                    repo_id,
                    action,
                    worktree_path,
                    result,
                }
            }
        };
        operation.finish(outcome, message);
    });
}

pub(super) fn schedule_checkout_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    let context = single_line_context(&name);
    let branch = name.clone();
    schedule_branch_action(
        executor,
        repos,
        backend,
        msg_tx,
        repo_id,
        RepoActionKind::CheckoutBranch,
        context,
        branch,
        WorktreeRedirect::OpenOnly,
        move |repo| repo.checkout_branch(&name),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_checkout_remote_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    remote: String,
    branch: String,
    local_branch: String,
    mode: CheckoutRemoteBranchMode,
) {
    let context = single_line_context(format!("{remote}/{branch} → {local_branch}"));
    match mode {
        CheckoutRemoteBranchMode::Create => schedule_repo_action_with_hook(
            executor,
            repos,
            msg_tx,
            repo_id,
            RepoActionKind::CheckoutRemoteBranch,
            context,
            move |repo| repo.checkout_remote_branch(&remote, &branch, &local_branch, mode),
            send_refresh_branches_and_load_worktrees_on_success,
            repo_action_finished(RepoActionKind::CheckoutRemoteBranch),
        ),
        CheckoutRemoteBranchMode::Overwrite => {
            let target = local_branch.clone();
            schedule_branch_action(
                executor,
                repos,
                backend,
                msg_tx,
                repo_id,
                RepoActionKind::CheckoutRemoteBranch,
                context,
                target,
                WorktreeRedirect::RunThere,
                move |repo| repo.checkout_remote_branch(&remote, &branch, &local_branch, mode),
            )
        }
    }
}

pub(super) fn schedule_checkout_commit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: gitcomet_core::domain::CommitId,
) {
    let context = Some(short_commit_id(commit_id.as_ref()));
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::CheckoutCommit,
        context,
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
    let context = Some(short_commit_id(commit_id.as_ref()));
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::RevertCommit,
        context,
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
    let context = single_line_context(format!("{name} at {target}"));
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::CreateBranch,
        context,
        move |repo| {
            let target = gitcomet_core::domain::CommitId(target.into());
            repo.create_branch(&name, &target)
        },
        send_refresh_branches_on_success,
        repo_action_finished(RepoActionKind::CreateBranch),
    );
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_create_branch_and_checkout(
    executor: &TaskExecutor,
    repos: &RepoMap,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
    target: String,
    force: bool,
) {
    let context = single_line_context(format!("{name} at {target}"));
    if force {
        let branch = name.clone();
        schedule_branch_action(
            executor,
            repos,
            backend,
            msg_tx,
            repo_id,
            RepoActionKind::CreateBranchAndCheckout,
            context,
            branch,
            WorktreeRedirect::RunThere,
            move |repo| {
                let target_id = gitcomet_core::domain::CommitId(target.into());
                repo.create_branch_force_and_checkout(&name, &target_id)
            },
        );
        return;
    }

    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation = GitOperationTask::start(
            repo_id,
            RepoActionKind::CreateBranchAndCheckout.hook_activity_label(),
            context,
            &msg_tx,
        );
        let created = {
            let _scope = operation.attach();
            let target_id = gitcomet_core::domain::CommitId(target.clone().into());
            repo.create_branch(&name, &target_id)
        };
        if is_branch_already_exists(&created) {
            finish_branch_collision(
                operation,
                &msg_tx,
                RepoActionKind::CreateBranchAndCheckout,
                BranchExistsPromptState {
                    repo_id,
                    name,
                    target,
                    operation: BranchExistsPromptOperation::CreateBranch,
                },
                &created,
            );
            return;
        }

        let refresh = created.is_ok();
        let result = {
            let _scope = operation.attach();
            created.and_then(|()| repo.checkout_branch(&name))
        };
        if refresh {
            send_or_log(&msg_tx, Msg::RefreshBranches { repo_id });
        }
        if result.is_ok() {
            send_or_log(&msg_tx, Msg::LoadWorktrees { repo_id });
            send_or_log(&msg_tx, Msg::LoadWorktreeDirty { repo_id });
        }
        let outcome = GitOperationTask::outcome(&result);
        operation.finish(
            outcome,
            InternalMsg::RepoActionFinished {
                repo_id,
                action: RepoActionKind::CreateBranchAndCheckout,
                result,
            },
        );
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn schedule_rename_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    old_name: String,
    new_name: String,
    force: bool,
) {
    let context = single_line_context(format!("{old_name} → {new_name}"));
    if force {
        let branch = new_name.clone();
        schedule_branch_action(
            executor,
            repos,
            backend,
            msg_tx,
            repo_id,
            RepoActionKind::RenameBranch,
            context,
            branch,
            WorktreeRedirect::RunThere,
            move |repo| repo.rename_branch_force(&old_name, &new_name),
        );
        return;
    }

    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation = GitOperationTask::start(
            repo_id,
            RepoActionKind::RenameBranch.hook_activity_label(),
            context,
            &msg_tx,
        );
        let result = {
            let _scope = operation.attach();
            repo.rename_branch(&old_name, &new_name)
        };
        if is_branch_already_exists(&result) {
            finish_branch_collision(
                operation,
                &msg_tx,
                RepoActionKind::RenameBranch,
                BranchExistsPromptState {
                    repo_id,
                    name: new_name,
                    target: old_name.clone(),
                    operation: BranchExistsPromptOperation::RenameBranch { old_name },
                },
                &result,
            );
            return;
        }
        send_refresh_branches_and_load_worktrees_on_success(&msg_tx, repo_id, &result);
        let outcome = GitOperationTask::outcome(&result);
        operation.finish(
            outcome,
            InternalMsg::RepoActionFinished {
                repo_id,
                action: RepoActionKind::RenameBranch,
                result,
            },
        );
    });
}

pub(super) fn schedule_delete_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    let context = single_line_context(&name);
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::DeleteBranch,
        context,
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
    let context = single_line_context(format!(
        "{} branches: {}",
        names.len(),
        crate::name_summary::elide_names(&names, ", ")
    ));
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::DeleteBranches,
        context,
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
                    if matches!(err.kind(), gitcomet_core::error::ErrorKind::Cancelled) {
                        return Err(err);
                    }
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
    let context = single_line_context(&name);
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::ForceDeleteBranch,
        context,
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
    let context = path_context(&path);
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::StagePath,
        context,
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
    let context = paths_context(paths.as_slice(), "files");
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::StagePaths,
        context,
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
    let context = path_context(&path);
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::UnstagePath,
        context,
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
    let context = paths_context(paths.as_slice(), "files");
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::UnstagePaths,
        context,
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
    let context = path_context(&path);
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::DiscardWorktreeChangesPath,
        context,
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
    let context = paths_context(&paths, "files");
    schedule_repo_action(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::DiscardWorktreeChangesPaths,
        context,
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
    let context = message_subject(&message).or_else(|| Some("No commit message".to_string()));
    schedule_repo_action_with_result(
        executor,
        repos,
        msg_tx,
        repo_id,
        "Commit",
        context,
        move |repo| run_with_git_auth(auth, || repo.commit_with_outcome(&message)),
        |repo_id, result| InternalMsg::CommitFinished { repo_id, result },
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
    let context = message_subject(&message).or_else(|| Some("No commit message".to_string()));
    schedule_repo_action_with_result(
        executor,
        repos,
        msg_tx,
        repo_id,
        "Amend commit",
        context,
        move |repo| run_with_git_auth(auth, || repo.commit_amend_with_outcome(&message)),
        |repo_id, result| InternalMsg::CommitAmendFinished { repo_id, result },
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
    let subject = message_subject(&message).unwrap_or_else(|| "Working tree changes".to_string());
    let context = single_line_context(if include_untracked {
        format!("{subject} · including untracked files")
    } else {
        subject
    });
    schedule_repo_action_with_hook(
        executor,
        repos,
        msg_tx,
        repo_id,
        RepoActionKind::Stash,
        context,
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
        Some(format!("stash@{{{index}}}")),
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
    let context = Some(format!("stash@{{{index}}}"));
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let operation = GitOperationTask::start(
            repo_id,
            RepoActionKind::PopStash.hook_activity_label(),
            context,
            &msg_tx,
        );
        let (applied, result) = {
            let _scope = operation.attach();
            let apply_result = repo.stash_apply(index);
            let applied = apply_result.is_ok();
            let result = apply_result.and_then(|()| repo.stash_drop(index));
            (applied, result)
        };
        if applied {
            send_or_log(&msg_tx, Msg::LoadStashes { repo_id });
        }
        let outcome = GitOperationTask::outcome(&result);
        operation.finish(
            outcome,
            InternalMsg::RepoActionFinished {
                repo_id,
                action: RepoActionKind::PopStash,
                result,
            },
        );
    });
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
        RepoActionKind::DropStash,
        Some(format!("stash@{{{index}}}")),
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
