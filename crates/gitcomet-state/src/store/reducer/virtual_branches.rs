use super::util::push_diagnostic;
use crate::model::{AppState, DiagnosticKind, RepoId};
use crate::msg::Effect;
use gitcomet_core::domain::VirtualBranch;
use gitcomet_core::error::Error;

/// Builds the session persist effect for a repo's virtual branch workspace.
fn persist_effect(repo: &crate::model::RepoState, action: &'static str) -> Effect {
    Effect::PersistVirtualBranches {
        repo_id: Some(repo.id),
        workdir: repo.spec.workdir.clone(),
        data: crate::session::VirtualBranchesSessionFile {
            next_id: repo.next_virtual_branch_id,
            branches: repo.virtual_branches.iter().map(Into::into).collect(),
        },
        action,
    }
}

pub(super) fn create_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let id = repo.next_virtual_branch_id;
    repo.next_virtual_branch_id += 1;
    let name = if name.trim().is_empty() {
        format!("Branch {id}")
    } else {
        name
    };
    repo.virtual_branches.push(VirtualBranch {
        id,
        name: name.into(),
        paths: Vec::new(),
        applied: true,
        stored_patch: None,
        pending: false,
    });
    repo.virtual_branches_rev += 1;
    vec![persist_effect(repo, "creating a virtual branch")]
}

pub(super) fn rename_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    name: String,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    if !name.trim().is_empty() {
        branch.name = name.into();
        repo.virtual_branches_rev += 1;
        vec![persist_effect(repo, "renaming a virtual branch")]
    } else {
        Vec::new()
    }
}

pub(super) fn delete_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.retain(|b| b.id != branch_id);
    repo.virtual_branches_rev += 1;
    vec![persist_effect(repo, "deleting a virtual branch")]
}

/// Bulk-removes virtual branches (used by the stale-branch cleanup after
/// user confirmation).
pub(super) fn prune_virtual_branches(
    state: &mut AppState,
    repo_id: RepoId,
    branch_ids: Vec<u64>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if branch_ids.is_empty() {
        return Vec::new();
    }
    let before = repo.virtual_branches.len();
    repo.virtual_branches
        .retain(|branch| !branch_ids.contains(&branch.id));
    if repo.virtual_branches.len() == before {
        return Vec::new();
    }
    repo.virtual_branches_rev += 1;
    vec![persist_effect(repo, "pruning stale virtual branches")]
}

/// Assigns `path` to the branch: the path is removed from every other branch
/// first (a path belongs to at most one virtual branch).
pub(super) fn assign_path_to_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    if !repo.virtual_branches.iter().any(|b| b.id == branch_id) {
        return Vec::new();
    }
    for other in repo.virtual_branches.iter_mut() {
        other.paths.retain(|p| p != &path);
    }
    let changed = if let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id)
        && !branch.paths.contains(&path)
    {
        branch.paths.push(path);
        repo.virtual_branches_rev += 1;
        true
    } else {
        false
    };
    if changed {
        vec![persist_effect(repo, "assigning a path to a virtual branch")]
    } else {
        Vec::new()
    }
}

pub(super) fn unassign_path_from_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    path: std::path::PathBuf,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    let old_len = branch.paths.len();
    branch.paths.retain(|p| p != &path);
    if branch.paths.len() != old_len {
        repo.virtual_branches_rev += 1;
        vec![persist_effect(repo, "unassigning a path from a virtual branch")]
    } else {
        Vec::new()
    }
}

pub(super) fn unapply_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    if !branch.applied || branch.pending || branch.paths.is_empty() {
        return Vec::new();
    }
    branch.pending = true;
    vec![Effect::UnapplyVirtualBranch {
        repo_id,
        branch_id,
        paths: branch.paths.clone(),
    }]
}

pub(super) fn apply_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    if branch.applied || branch.pending {
        return Vec::new();
    }
    let Some(patch) = branch.stored_patch.clone() else {
        return Vec::new();
    };
    branch.pending = true;
    vec![Effect::ApplyVirtualBranch {
        repo_id,
        branch_id,
        patch: patch.to_string(),
    }]
}

pub(super) fn virtual_branch_unapplied(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    result: Result<String, Error>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    branch.pending = false;
    match result {
        Ok(patch) => {
            // A branch may already park hunks moved out of the worktree; the
            // newly captured diff is appended rather than replacing them.
            let patch = patch.trim_end().to_string();
            branch.stored_patch = Some(match branch.stored_patch.take() {
                Some(previous) if !previous.trim().is_empty() => {
                    format!("{}\n{}", previous.trim_end(), patch).into()
                }
                _ => patch.clone().into(),
            });
            branch.applied = false;
            repo.virtual_branches_rev += 1;
            vec![persist_effect(repo, "unapplying a virtual branch")]
        }
        Err(e) => {
            push_diagnostic(
                repo,
                DiagnosticKind::Error,
                format!("Unapply virtual branch: {e}"),
            );
            Vec::new()
        }
    }
}

pub(super) fn virtual_branch_applied(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    result: Result<(), Error>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    branch.pending = false;
    match result {
        Ok(()) => {
            branch.applied = true;
            branch.stored_patch = None;
            repo.virtual_branches_rev += 1;
            vec![persist_effect(repo, "applying a virtual branch")]
        }
        Err(e) => {
            push_diagnostic(
                repo,
                DiagnosticKind::Error,
                format!("Apply virtual branch: {e}"),
            );
            Vec::new()
        }
    }
}

/// Parks a single hunk (already reverse-applied to the worktree by the
/// worker) into the branch's stored patch collection. The branch is marked
/// unapplied so `Apply` restores the parked hunks (undo).
pub(super) fn move_hunk_to_virtual_branch(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    patch: String,
    _path: std::path::PathBuf,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
        return Vec::new();
    };
    if branch.pending || patch.trim().is_empty() {
        return Vec::new();
    }
    branch.pending = true;
    vec![Effect::MoveHunkToVirtualBranch {
        repo_id,
        branch_id,
        patch,
        path: _path,
    }]
}

pub(super) fn virtual_branch_hunk_moved(
    state: &mut AppState,
    repo_id: RepoId,
    branch_id: u64,
    path: std::path::PathBuf,
    result: Result<String, Error>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter_mut().find(|r| r.id == repo_id) else {
        return Vec::new();
    };
    {
        let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
            return Vec::new();
        };
        branch.pending = false;
    }
    match result {
        Ok(patch) => {
            let patch = patch.trim_end().to_string();
            {
                let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id) else {
                    return Vec::new();
                };
                branch.stored_patch = Some(match branch.stored_patch.take() {
                    Some(previous) if !previous.trim().is_empty() => {
                        format!("{}\n{}", previous.trim_end(), patch).into()
                    }
                    _ => patch.clone().into(),
                });
                branch.applied = false;
            }
            // The hunk's file now belongs to this branch.
            for other in repo.virtual_branches.iter_mut() {
                other.paths.retain(|p| p != &path);
            }
            if let Some(branch) = repo.virtual_branches.iter_mut().find(|b| b.id == branch_id)
                && !branch.paths.contains(&path)
            {
                branch.paths.push(path);
            }
            repo.virtual_branches_rev += 1;
            vec![persist_effect(repo, "moving a hunk to a virtual branch")]
        }
        Err(e) => {
            push_diagnostic(
                repo,
                DiagnosticKind::Error,
                format!("Move hunk to virtual branch: {e}"),
            );
            Vec::new()
        }
    }
}
