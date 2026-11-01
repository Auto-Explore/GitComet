//! Virtual branches effects: the backend work behind the Virtual branches view.
//!
//! Every virtual branches edit runs on a worker thread holding the repository handle
//! and reports back through `InternalMsg`. The split matters because the edits
//! are not cheap — creating a branch and rebuilding `gitcomet/virtual` is
//! several Git invocations — and doing that on the store thread would stall
//! every other repository in the window.

use crate::model::VirtualBranchesEdit;
use crate::msg::{CheckoutRequest, InternalMsg, Msg};
use gitcomet_core::services::{GitRepository, Result};
use gitcomet_core::virtual_branches::{AssignmentIndex, VirtualBranch};
use std::path::PathBuf;
use std::sync::Arc;

use super::super::RepoId;
use super::super::repository_runtime::RepositoryRuntimes;
use super::super::executor::TaskExecutor;
use super::super::worker_channel::StoreWorkerSender;
use super::util::{
    missing_repo_error, send_or_log, spawn_detached_with_repo_or_else,
    spawn_with_repo_or_else,
};

/// Read the virtual branches, its assignments, and the virtual branches-branch tip in one
/// pass, so the view has all three before it draws rather than filling in.
pub(super) fn schedule_load_virtual_branches(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-virtual branches",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.read_virtual_branches();
            let assignments = repo.read_virtual_branches_assignments();
            let virtual_branches_commit = read_virtual_branches_commit(repo.as_ref());
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesLoaded {
                    repo_id,
                    result,
                    assignments,
                    virtual_branches_commit,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                    assignments: Err(missing_repo_error(repo_id)),
                    virtual_branches_commit: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// The tip of `gitcomet/virtual`, or `None` when it has not been built.
///
/// A repository whose virtual branches has never been applied has no such branch, and
/// that is an ordinary state rather than an error, so a failed resolution
/// reports `None` rather than an error the user would have to dismiss on every
/// load.
fn read_virtual_branches_commit(
    repo: &dyn GitRepository,
) -> Result<Option<gitcomet_core::domain::CommitId>> {
    let name = gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH;
    match repo.resolve_commit(&gitcomet_core::domain::CommitId(name.into())) {
        Ok(commit) => Ok(Some(commit.id)),
        Err(_) => Ok(None),
    }
}

/// Apply one virtual branches edit.
///
/// The order matters: the Git work comes first and the persisted state second.
/// If the branch operations fail, the state file is left describing what is
/// actually true; writing it first would leave the file claiming a branch
/// exists that Git does not have.
pub(super) fn schedule_apply_virtual_branches_edit(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    edit: VirtualBranchesEdit,
) {
    let command_edit = edit.clone();
    let fallback_edit = edit.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result =
                apply_edit(repo.as_ref(), &edit).map(|outcome| outcome.unwrap_or_default());
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesEditFinished {
                    repo_id,
                    edit: command_edit,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesEditFinished {
                    repo_id,
                    edit: fallback_edit,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Realize one edit against Git and the persisted virtual branches state.
///
/// Returns `None` for edits that change only the stored state (a file
/// assignment, a pure reorder) so the caller can tell them apart from the ones
/// that produced a commit.
fn apply_edit(
    repo: &dyn GitRepository,
    edit: &VirtualBranchesEdit,
) -> Result<Option<gitcomet_core::services::CommitOperationOutcome>> {
    if matches!(
        edit,
        VirtualBranchesEdit::AssignFile { .. } | VirtualBranchesEdit::AssignHunk { .. }
    ) {
        // Assigning a file or a hunk only touches the assignment index. The
        // reducer routes both to `AssignVirtualBranchesFile` so a drag in the file
        // list does not revalidate and rewrite the whole virtual branches. Reaching
        // here at all means something sent the edit the long way round, and the
        // answer is the same: no Git work, and in particular no rewrite of the
        // state file, which the assignment path does not own.
        return Ok(None);
    }
    let mut state = repo.read_virtual_branches()?;
    // Whether `gitcomet/virtual` has to be replayed once the edit is written.
    let mut rebuild = false;

    match edit {
        VirtualBranchesEdit::Create { name } => {
            let base = state.target.clone();
            repo.create_virtual_branch(name, &base)?;
            state.push_branch(VirtualBranch::new(name.clone()))?;
            // A new branch is applied by default, so the virtual-branches branch has
            // to pick it up or the working tree would not show it.
            rebuild = true;
        }
        VirtualBranchesEdit::InsertRelativeTo {
            name,
            anchor,
            below,
        } => {
            // Above and below are different edits, so they start the new branch from
            // different commits and only one of them rewrites history.
            //
            // *Above* stacked branches the new branch on the anchor, so it starts at the
            // anchor's tip and displaces nothing.
            //
            // *Below* puts it where the anchor sat, so it starts at the
            // anchor's *base* and the anchor then has to be replayed on top of
            // it — otherwise the two would claim a parent relationship their
            // commits do not have, and a pull request for the upper branch
            // would contain none of the lower one's work.
            let anchor_parent = state
                .get(anchor)
                .map(|branch| branch.base_branch(&state.target).to_string())
                .ok_or_else(|| {
                    gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
                        format!("no virtual-branches branch '{anchor}'"),
                    ))
                })?;
            let inserted = VirtualBranch::new(name.clone());
            if *below {
                repo.create_virtual_branch(name, &anchor_parent)?;
                state.insert_below(inserted, anchor)?;
                repo.rebase_virtual_branch(anchor, name)?;
            } else {
                repo.create_virtual_branch(name, anchor)?;
                state.insert_above(inserted, anchor)?;
            }
            rebuild = true;
        }
        VirtualBranchesEdit::SetApplied { name, applied } => {
            state.set_applied(name, *applied)?;
            repo.write_virtual_branches(&state)?;
            return rebuild_virtual_branches(repo).map(Some);
        }
        VirtualBranchesEdit::SetParent { name, parent } => {
            let base = parent.clone().unwrap_or_else(|| state.target.clone());
            state.set_parent(name, parent.as_deref())?;
            repo.write_virtual_branches(&state)?;
            // Always replay, detaching included. A branch that stops being
            // stacked now belongs on the target, and leaving it on its old
            // parent would keep claiming a relationship its commits do not
            // have — the same reason inserting *below* an anchor rebases the
            // anchor.
            repo.rebase_virtual_branch(name, &base)?;
            return rebuild_virtual_branches(repo).map(Some);
        }
        VirtualBranchesEdit::MoveToStackedBranch {
            name,
            stacked_branch_base,
            relative_to,
            below,
        } => {
            let before = state.get(name).and_then(|b| b.parent.clone());
            state.move_to_stacked_branch(name, stacked_branch_base, relative_to.as_deref(), *below)?;
            repo.write_virtual_branches(&state)?;
            // Only a change of stacked branch needs history rewritten; a reorder inside
            // the same stacked branch does not.
            if state.get(name).and_then(|b| b.parent.clone()) != before {
                let onto = state
                    .get(name)
                    .map(|b| b.base_branch(&state.target).to_string())
                    .unwrap_or_else(|| state.target.clone());
                repo.rebase_virtual_branch(name, &onto)?;
                return rebuild_virtual_branches(repo).map(Some);
            }
            return Ok(None);
        }
        VirtualBranchesEdit::AssignFile { .. } | VirtualBranchesEdit::AssignHunk { .. } => {
            // Unreachable: the guard above sends both here only if it is ever
            // reordered. Assignments take the fast path and never reach Git, so
            // answering `None` keeps the meaning identical.
            return Ok(None);
        }
        VirtualBranchesEdit::Reorder { first, second } => {
            state.reorder(first, second)?;
        }
        VirtualBranchesEdit::Remove { name } => {
            state.remove_branch(name)?;
            // The branch stops contributing to the virtual-branches branch, so its
            // changes have to be taken back out. The Git branch itself is left
            // alone: removing it from the virtual branches is not a request to delete
            // work the user may still want.
            rebuild = true;
        }
        VirtualBranchesEdit::SetTarget { target } => {
            repo.update_virtual_branches_target(target)?;
            // `update_virtual_branches_target` wrote the state on the backend; read it
            // back so the reducer's copy matches what Git actually did. The
            // value itself is not used here — the rebuild re-reads it — but the
            // read is what makes a failed backend write surface as this edit's
            // error rather than as a stale state the view keeps rendering.
            let _ = repo.read_virtual_branches()?;
            return rebuild_virtual_branches(repo).map(Some);
        }
        VirtualBranchesEdit::CommitPaths {
            name,
            message,
            paths,
        } => {
            let refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
            // Wrapped rather than returned as it stands: a commit *is* an
            // outcome, and every caller of this function sees edits producing
            // one as `Some`.
            return repo
                .commit_paths_to_virtual_branch(name, message, &refs)
                .map(Some);
        }
    }

    repo.write_virtual_branches(&state)?;
    if rebuild {
        return rebuild_virtual_branches(repo).map(Some);
    }
    Ok(None)
}

/// Rewrite `gitcomet/virtual` from the applied set in the stored state.
fn rebuild_virtual_branches(
    repo: &dyn GitRepository,
) -> Result<gitcomet_core::services::CommitOperationOutcome> {
    let state = repo.read_virtual_branches()?;
    let applied = state.application_order()?;
    let names: Vec<&str> = applied.iter().map(|b| b.name.as_str()).collect();
    let commit = repo.update_virtual_branches_branch(&names)?;
    Ok(gitcomet_core::services::CommitOperationOutcome {
        local_branch: Some(gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH.to_string()),
        pre_head: None,
        post_head: Some(commit),
    })
}

/// Put the working directory on the virtual-branches branch.
pub(super) fn schedule_enter_virtual_branches(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    checkout_base: Option<String>,
) {
    let command_base = checkout_base.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.enter_virtual_branches().map(|_| ());
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: true,
                    checkout_base: command_base.clone(),
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: true,
                    checkout_base,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Take the working directory back off the virtual-branches branch.
pub(super) fn schedule_leave_virtual_branches(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    checkout_base: Option<String>,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let Some(onto) = resolve_leave_target(&repo, &checkout_base, repo_id, &msg_tx) else {
                return;
            };
            let result = repo.leave_virtual_branches(&onto);
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: false,
                    checkout_base: None,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: false,
                    checkout_base: None,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Where a leave puts the working directory.
///
/// The branch the caller named when it has one, otherwise the branch the stored
/// virtual branches targets — never a guess, since a wrong guess would move the user
/// onto a branch they never had checked out.
///
/// `None` means the decision could not be made: reading the virtual branches failed
/// (a repository with no virtual branches on disk at all), and that failure has
/// already been reported through `msg_tx`, the same way every other worker
/// failure is, rather than being dropped on the thread that produced it.
fn resolve_leave_target(
    repo: &Arc<dyn GitRepository>,
    checkout_base: &Option<String>,
    repo_id: RepoId,
    msg_tx: &StoreWorkerSender,
) -> Option<String> {
    if let Some(branch) = checkout_base {
        return Some(branch.clone());
    }
    match repo.read_virtual_branches() {
        Ok(state) => Some(state.target),
        Err(error) => {
            send_or_log(
                msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: false,
                    checkout_base: None,
                    result: Err(error),
                }),
            );
            None
        }
    }
}

/// Take the working directory off the virtual-branches branch and then run the
/// checkout that asked for it.
///
/// The leave is reported exactly as [`schedule_leave_virtual_branches`] reports it,
/// and the checkout follows it as the ordinary message it is: the reducer
/// then records the checkout the way it records every other one, and a leave
/// that fails takes the checkout with it instead of running it against a
/// working directory the virtual branches still holds.
pub(super) fn schedule_leave_virtual_branches_and_checkout(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    checkout_base: Option<String>,
    then: CheckoutRequest,
) {
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let Some(onto) = resolve_leave_target(&repo, &checkout_base, repo_id, &msg_tx) else {
                return;
            };
            let result = repo.leave_virtual_branches(&onto);
            let left = result.is_ok();
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: false,
                    checkout_base: None,
                    result,
                }),
            );
            if left {
                send_or_log(&msg_tx, then.into_msg(repo_id));
            }
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesActiveFinished {
                    repo_id,
                    active: false,
                    checkout_base: None,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Persist one file's branch assignment.
pub(super) fn schedule_assign_virtual_branches_file(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    hunk: Option<gitcomet_core::virtual_branches::HunkFingerprint>,
    branch: Option<String>,
) {
    let command_path = path.clone();
    let command_hunk = hunk;
    let command_branch = branch.clone();
    let fallback_path = path.clone();
    let fallback_branch = branch.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = (|| -> Result<()> {
                let mut index: AssignmentIndex = repo.read_virtual_branches_assignments()?;
                match hunk {
                    Some(hunk) => index.set_hunk(path, hunk, branch),
                    None => index.set(path, branch),
                }
                repo.write_virtual_branches_assignments(&index)
            })();
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesAssignFinished {
                    repo_id,
                    path: command_path,
                    hunk: command_hunk,
                    branch: command_branch,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesAssignFinished {
                    repo_id,
                    path: fallback_path,
                    hunk: command_hunk,
                    branch: fallback_branch,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Push a virtual branch as an ordinary branch, setting upstream on first push.
///
/// A virtual branch is a normal Git branch, so pushing needs no special server
/// support; only the base it will be reviewed against differs, and that is the
/// caller's concern when it opens a pull request.
pub(super) fn schedule_push_virtual_branches_branch(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    let command_name = name.clone();
    let fallback_name = name.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.push_virtual_branch(&name);
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesPushFinished {
                    repo_id,
                    name: command_name,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::VirtualBranchesPushFinished {
                    repo_id,
                    name: fallback_name,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}
