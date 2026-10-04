//! Workspace effects: the backend work behind the Workspace view.
//!
//! Every workspace edit runs on a worker thread holding the repository handle
//! and reports back through `InternalMsg`. The split matters because the edits
//! are not cheap — creating a branch and rebuilding `gitcomet/workspace` is
//! several Git invocations — and doing that on the store thread would stall
//! every other repository in the window.

use crate::model::WorkspaceEdit;
use crate::msg::{InternalMsg, Msg};
use gitcomet_core::services::{GitRepository, Result};
use gitcomet_core::workspace::{AssignmentIndex, VirtualBranch};
use std::path::PathBuf;

use super::super::RepoId;
use super::super::executor::TaskExecutor;
use super::super::worker_channel::StoreWorkerSender;
use super::util::{
    RepoMap, missing_repo_error, send_or_log, spawn_detached_with_repo_or_else,
    spawn_with_repo_or_else,
};

/// Read the workspace, its assignments, and the workspace-branch tip in one
/// pass, so the view has all three before it draws rather than filling in.
pub(super) fn schedule_load_workspace(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    spawn_detached_with_repo_or_else(
        executor,
        "load-workspace",
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.read_workspace();
            let assignments = repo.read_workspace_assignments();
            let workspace_commit = read_workspace_commit(repo.as_ref());
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspaceLoaded {
                    repo_id,
                    result,
                    assignments,
                    workspace_commit,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspaceLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                    assignments: Err(missing_repo_error(repo_id)),
                    workspace_commit: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// The tip of `gitcomet/workspace`, or `None` when it has not been built.
///
/// A repository whose workspace has never been applied has no such branch, and
/// that is an ordinary state rather than an error, so a failed resolution
/// reports `None` rather than an error the user would have to dismiss on every
/// load.
fn read_workspace_commit(
    repo: &dyn GitRepository,
) -> Result<Option<gitcomet_core::domain::CommitId>> {
    let name = gitcomet_core::workspace::WORKSPACE_BRANCH;
    match repo.resolve_commit(&gitcomet_core::domain::CommitId(name.into())) {
        Ok(commit) => Ok(Some(commit.id)),
        Err(_) => Ok(None),
    }
}

/// Apply one workspace edit.
///
/// The order matters: the Git work comes first and the persisted state second.
/// If the branch operations fail, the state file is left describing what is
/// actually true; writing it first would leave the file claiming a branch
/// exists that Git does not have.
pub(super) fn schedule_apply_workspace_edit(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    edit: WorkspaceEdit,
) {
    let command_edit = edit.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = apply_edit(repo.as_ref(), &edit).map(|outcome| outcome.unwrap_or_default());
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspaceEditFinished {
                    repo_id,
                    edit: command_edit,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspaceEditFinished {
                    repo_id,
                    edit: command_edit,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

/// Realize one edit against Git and the persisted workspace state.
///
/// Returns `None` for edits that change only the stored state (a file
/// assignment, a pure reorder) so the caller can tell them apart from the ones
/// that produced a commit.
fn apply_edit(
    repo: &dyn GitRepository,
    edit: &WorkspaceEdit,
) -> Result<Option<gitcomet_core::services::CommitOperationOutcome>> {
    if let WorkspaceEdit::AssignFile { .. } = edit {
        // Assigning a file only touches the assignment index. The reducer
        // routes it to `AssignWorkspaceFile` so a drag in the file list does
        // not revalidate and rewrite the whole workspace.
        return Ok(None);
    }
    let mut state = repo.read_workspace()?;
    // Whether `gitcomet/workspace` has to be replayed once the edit is written.
    let mut rebuild = false;

    match edit {
        WorkspaceEdit::Create { name } => {
            let base = state.target.clone();
            repo.create_virtual_branch(name, &base)?;
            state.push_branch(VirtualBranch::new(name.clone()))?;
            // A new branch is applied by default, so the workspace branch has
            // to pick it up or the working tree would not show it.
            rebuild = true;
        }
        WorkspaceEdit::InsertRelativeTo {
            name,
            anchor,
            below,
        } => {
            // Above and below are different edits, so they start the new branch from
            // different commits and only one of them rewrites history.
            //
            // *Above* stacks the new branch on the anchor, so it starts at the
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
                        format!("no workspace branch '{anchor}'"),
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
        WorkspaceEdit::SetApplied { name, applied } => {
            state.set_applied(name, *applied)?;
            repo.write_workspace(&state)?;
            return rebuild_workspace(repo).map(Some);
        }
        WorkspaceEdit::SetParent { name, parent } => {
            let base = parent.clone().unwrap_or_else(|| state.target.clone());
            state.set_parent(name, parent.as_deref())?;
            repo.write_workspace(&state)?;
            if parent.is_some() {
                repo.rebase_virtual_branch(name, &base)?;
            }
            return rebuild_workspace(repo).map(Some);
        }
        WorkspaceEdit::MoveToStack {
            name,
            stack_base,
            relative_to,
            below,
        } => {
            let before = state.get(name).and_then(|b| b.parent.clone());
            state.move_to_stack(
                name,
                stack_base,
                relative_to.as_deref(),
                *below,
            )?;
            repo.write_workspace(&state)?;
            // Only a change of stack needs history rewritten; a reorder inside
            // the same stack does not.
            if state.get(name).and_then(|b| b.parent.clone()) != before {
                let onto = state
                    .get(name)
                    .map(|b| b.base_branch(&state.target).to_string())
                    .unwrap_or_else(|| state.target.clone());
                repo.rebase_virtual_branch(name, &onto)?;
                return rebuild_workspace(repo).map(Some);
            }
            return Ok(None);
        }
        WorkspaceEdit::Reorder { first, second } => {
            state.reorder(first, second)?;
        }
        WorkspaceEdit::Remove { name } => {
            state.remove_branch(name)?;
            // The branch stops contributing to the workspace branch, so its
            // changes have to be taken back out. The Git branch itself is left
            // alone: removing it from the workspace is not a request to delete
            // work the user may still want.
            rebuild = true;
        }
        WorkspaceEdit::SetTarget { target } => {
            repo.update_workspace_target(target)?;
            // `update_workspace_target` wrote the state on the backend; read it
            // back so the reducer's copy matches what Git actually did.
            state = repo.read_workspace()?;
            return rebuild_workspace(repo).map(Some);
        }
        WorkspaceEdit::CommitPaths {
            name,
            message,
            paths,
        } => {
            let refs: Vec<&std::path::Path> = paths.iter().map(|p| p.as_path()).collect();
            return repo.commit_paths_to_virtual_branch(name, message, &refs);
        }
    }

    repo.write_workspace(&state)?;
    if rebuild {
        return rebuild_workspace(repo).map(Some);
    }
    Ok(None)
}

/// Rewrite `gitcomet/workspace` from the applied set in the stored state.
fn rebuild_workspace(
    repo: &dyn GitRepository,
) -> Result<gitcomet_core::services::CommitOperationOutcome> {
    let state = repo.read_workspace()?;
    let applied = state.application_order()?;
    let names: Vec<&str> = applied.iter().map(|b| b.name.as_str()).collect();
    let commit = repo.update_workspace_branch(&names)?;
    Ok(gitcomet_core::services::CommitOperationOutcome {
        local_branch: Some(gitcomet_core::workspace::WORKSPACE_BRANCH.to_string()),
        pre_head: None,
        post_head: Some(commit),
    })
}

/// Persist one file's branch assignment.
pub(super) fn schedule_assign_workspace_file(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    hunk: Option<gitcomet_core::workspace::HunkFingerprint>,
    branch: Option<String>,
) {
    let command_path = path.clone();
    let command_hunk = hunk;
    let command_branch = branch.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = (|| -> Result<()> {
                let mut index: AssignmentIndex = repo.read_workspace_assignments()?;
                match hunk {
                    Some(hunk) => index.set_hunk(path, hunk, branch),
                    None => index.set(path, branch),
                }
                repo.write_workspace_assignments(&index)
            })();
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspaceAssignFinished {
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
                Msg::Internal(InternalMsg::WorkspaceAssignFinished {
                    repo_id,
                    path: command_path,
                    hunk: command_hunk,
                    branch: command_branch,
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
pub(super) fn schedule_push_workspace_branch(
    executor: &TaskExecutor,
    repos: &RepoMap,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    name: String,
) {
    let command_name = name.clone();
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            let result = repo.push_virtual_branch(&name);
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspacePushFinished {
                    repo_id,
                    name: command_name,
                    result,
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::WorkspacePushFinished {
                    repo_id,
                    name: command_name,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}
