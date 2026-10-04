//! Workspace reducers: turn Workspace messages into state changes and the
//! effects that keep the repository in step.
//!
//! The pattern throughout is that a workspace action is optimistic about the
//! *repository* and pessimistic about the *state file*: the edit goes to the
//! backend first, and the reducer's copy is only updated once the backend
//! reports what actually happened. A failed apply therefore leaves the UI
//! showing the truth rather than a branch the user thinks exists.

use super::util::{push_diagnostic, push_notification};
use crate::model::{
    AppNotificationKind, AppState, DiagnosticKind, Loadable, RepoId, RepoState, WorkspaceEdit,
};
use crate::msg::Effect;
use gitcomet_core::workspace::{AssignmentIndex, BranchApplyState, WorkspaceState};
use std::path::PathBuf;
use std::sync::Arc;

/// The Workspace view asks for its data; nothing else does, so a repository
/// that never opens it never reads a workspace file.
pub(super) fn load_workspace(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.workspace.busy.loading = true;
    repo.workspace.state = Loadable::Loading;
    vec![Effect::LoadWorkspace { repo_id }]
}

/// Read the branch's row button: apply or unapply.
pub(super) fn set_branch_applied(
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    applied: BranchApplyState,
) -> Vec<Effect> {
    // Applying or unapplying always rewrites `gitcomet/workspace`, so it takes
    // the same busy gate as a stack change.
    apply_edit(state, repo_id, WorkspaceEdit::SetApplied { name, applied }, true)
}

/// Any other workspace edit from the UI.
pub(super) fn apply_workspace_edit(
    state: &mut AppState,
    repo_id: RepoId,
    edit: WorkspaceEdit,
) -> Vec<Effect> {
    // A file assignment writes only the assignment index, so it gets its own
    // effect rather than going through the workspace rewrite path. Hunk
    // assignments take the same road: they write the same file.
    let assignment = match edit {
        WorkspaceEdit::AssignFile { path, branch } => Some((path, None, branch)),
        WorkspaceEdit::AssignHunk { path, hunk, branch } => Some((path, Some(hunk), branch)),
        _ => None,
    };
    if let Some((path, hunk, branch)) = assignment {
        let Some(repo) = repo_mut(state, repo_id) else {
            return Vec::new();
        };
        if repo.workspace.busy.any() {
            return Vec::new();
        }
        repo.workspace.busy.applying = true;
        repo.workspace.bump_rev();
        return vec![Effect::AssignWorkspaceFile {
            repo_id,
            path,
            hunk,
            branch,
        }];
    }
    // Only the edits that rewrite branch history need the busy flag that
    // disables the rest of the UI; a reorder is instant and blocking the view
    // for it would be a visible stutter for nothing.
    let mutating = edit.rewrites_history() || edit.changes_applied_set();
    apply_edit(state, repo_id, edit, mutating)
}

fn apply_edit(
    state: &mut AppState,
    repo_id: RepoId,
    edit: WorkspaceEdit,
    mutating: bool,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.busy.any() {
        // Two workspace edits racing would each write the state file and the
        // loser's change would vanish silently.
        return Vec::new();
    }
    if mutating {
        repo.workspace.busy.mutating = true;
    } else {
        repo.workspace.busy.applying = true;
    }
    repo.workspace.bump_rev();

    vec![Effect::ApplyWorkspaceEdit { repo_id, edit }]
}

/// Commit every file assigned to `name`.
pub(super) fn commit_branch(
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    message: String,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.busy.any() {
        return Vec::new();
    }
    // An empty message would create a commit git refuses to describe; the
    // commit form is the right place to require one, and an empty submit here
    // means the user has not typed anything yet.
    if message.trim().is_empty() {
        return Vec::new();
    }
    let paths: Vec<PathBuf> = repo
        .workspace
        .assignments
        .paths_for(&name)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        push_notification(
            state,
            AppNotificationKind::Info,
            format!("No files are assigned to '{name}'"),
        );
        return Vec::new();
    }

    commit_paths(state, repo_id, name, message, paths)
}

/// Commit one file to whatever branch it is assigned to.
///
/// A file with no branch has nowhere to go, so the message says so instead of
/// committing it to the workspace branch, which is never a user commit target.
pub(super) fn commit_single_file(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    let branch = {
        let Some(repo) = repo_mut(state, repo_id) else {
            return Vec::new();
        };
        let Some(workspace) = repo.workspace.workspace() else {
            return Vec::new();
        };
        repo.workspace
            .assignments
            .resolve(&path, workspace)
            .map(str::to_string)
    };
    let Some(branch) = branch else {
        push_notification(
            state,
            AppNotificationKind::Info,
            format!(
                "Assign '{}' to a branch before committing it",
                path.display()
            ),
        );
        return Vec::new();
    };

    // Only this file, not the rest of the branch: the row the user clicked is
    // the file they meant to commit.
    let message = format!("Update {}", path.display());
    commit_paths(state, repo_id, branch, message, vec![path])
}

/// The one path from a commit message to a running commit, shared by the
/// whole-branch and single-file entry points so both take the same busy gate.
fn commit_paths(
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    message: String,
    paths: Vec<PathBuf>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.busy.any() {
        return Vec::new();
    }
    repo.workspace.busy.committing = true;
    repo.workspace.bump_rev();

    vec![Effect::ApplyWorkspaceEdit {
        repo_id,
        edit: WorkspaceEdit::CommitPaths {
            name,
            message,
            paths,
        },
    }]
}

/// Push a virtual branch, setting upstream on first push.
pub(super) fn push_branch(state: &mut AppState, repo_id: RepoId, name: String) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.busy.any() {
        return Vec::new();
    }
    repo.workspace.busy.mutating = true;
    repo.workspace.bump_rev();
    vec![Effect::PushWorkspaceBranch { repo_id, name }]
}

pub(super) fn dismiss_conflict(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.conflict.take().is_some() {
        repo.workspace.bump_rev();
    }
    Vec::new()
}

// ── Backend replies ─────────────────────────────────────────────

pub(super) fn workspace_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: gitcomet_core::services::Result<WorkspaceState>,
    assignments: gitcomet_core::services::Result<AssignmentIndex>,
    workspace_commit: gitcomet_core::services::Result<Option<gitcomet_core::domain::CommitId>>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.workspace.busy.loading = false;
    match &result {
        // `set_state` derives the stacks and stores the state, so the loadable
        // and the cached stacks can never disagree.
        Ok(loaded) => {
            repo.workspace.set_state(loaded.clone());
            repo.workspace.workspace_commit = match &workspace_commit {
                Ok(commit) => Loadable::Ready(commit.clone()),
                Err(error) => Loadable::Error(error.clone()),
            };
        }
        Err(error) => repo.workspace.state = Loadable::Error(error.clone()),
    };
    if let Ok(index) = assignments {
        repo.workspace.assignments = Arc::new(index);
    }
    repo.workspace.bump_rev();

    let mut effects = Vec::new();
    if let Err(error) = result {
        push_notification(
            state,
            AppNotificationKind::Error,
            format!("Could not load the workspace: {error}"),
        );
    }
    // A branch the workspace does not know about yet cannot be applied, so the
    // view needs the ordinary branch list alongside the workspace.
    effects.push(Effect::LoadBranches { repo_id });
    effects
}

pub(super) fn workspace_edit_finished(
    state: &mut AppState,
    repo_id: RepoId,
    edit: WorkspaceEdit,
    result: gitcomet_core::services::Result<gitcomet_core::services::CommitOperationOutcome>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.workspace.busy = Default::default();
    repo.workspace.bump_rev();

    if let Err(error) = result {
        let branch = edit.branch_name().unwrap_or("workspace");
        repo.workspace.conflict = Some(crate::model::WorkspaceConflict {
            branch: branch.to_string(),
            against: None,
            paths: Arc::new(Vec::new()),
            message: error.to_string(),
        });
        push_diagnostic(repo, DiagnosticKind::Error, error.to_string());
        // The edit did not happen, so the cached state is stale in the other
        // direction: reload rather than guess what survived.
        return vec![Effect::LoadWorkspace { repo_id }];
    }

    repo.workspace.conflict = None;
    vec![
        Effect::LoadWorkspace { repo_id },
        // A commit or an apply can change what the working tree reports.
        Effect::LoadWorktreeStatus { repo_id },
        Effect::LoadBranches { repo_id },
    ]
}

pub(super) fn workspace_assign_finished(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    hunk: Option<gitcomet_core::workspace::HunkKey>,
    branch: Option<String>,
    result: gitcomet_core::services::Result<()>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.workspace.busy.applying = false;

    if let Err(error) = result {
        repo.workspace.bump_rev();
        push_notification(
            state,
            AppNotificationKind::Error,
            format!("Could not assign '{}': {error}", path.display()),
        );
        return vec![Effect::LoadWorkspace { repo_id }];
    }

    // Mirror the write into the store. Reloading the whole workspace to learn
    // something the view already knows would discard the derived stacks and
    // rebuild them for nothing, and on a hunk-by-hunk assignment that is once
    // per hunk.
    let index = std::sync::Arc::make_mut(&mut repo.workspace.assignments);
    match hunk {
        Some(hunk) => index.set_hunk(path, hunk, branch),
        None => index.set(path, branch),
    }
    repo.workspace.bump_rev();
    Vec::new()
}

pub(super) fn workspace_push_finished(
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    result: gitcomet_core::services::Result<gitcomet_core::services::CommandOutput>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.workspace.busy.mutating = false;
    repo.workspace.bump_rev();

    match result {
        Ok(_) => {
            push_notification(
                state,
                AppNotificationKind::Info,
                format!("Pushed '{name}'"),
            );
            vec![Effect::LoadBranches { repo_id }]
        }
        Err(error) => {
            push_notification(
                state,
                AppNotificationKind::Error,
                format!("Could not push '{name}': {error}"),
            );
            Vec::new()
        }
    }
}

fn repo_mut(state: &mut AppState, repo_id: RepoId) -> Option<&mut RepoState> {
    state.repos.iter_mut().find(|repo| repo.id == repo_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RepoState, WorkspaceRepoState};
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::workspace::VirtualBranch;

    fn repo_with_workspace() -> AppState {
        let mut state = AppState::test_default();
        let spec = RepoSpec {
            workdir: std::path::PathBuf::from("/repo"),
        };
        let mut repo = RepoState::new_opening(RepoId(1), spec);
        let mut workspace = WorkspaceRepoState::default();
        workspace.set_state(
            WorkspaceState::new("main").with_branches(vec![
                VirtualBranch::new("api"),
                VirtualBranch::new("ui").with_parent("api"),
            ]),
        );
        repo.workspace = workspace;
        state.repos.push(repo);
        state
    }

    #[test]
    fn load_workspace_marks_loading_and_emits_the_effect() {
        let mut state = repo_with_workspace();
        let effects = load_workspace(&mut state, RepoId(1));
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::LoadWorkspace { .. }));
        assert!(state.repos[0].workspace.busy.loading);
        assert!(matches!(
            state.repos[0].workspace.state,
            Loadable::Loading
        ));
    }

    #[test]
    fn applying_a_branch_emits_the_edit_and_sets_mutating() {
        let mut state = repo_with_workspace();
        let effects = set_branch_applied(
            &mut state,
            RepoId(1),
            "api".into(),
            BranchApplyState::Unapplied,
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::ApplyWorkspaceEdit {
                repo_id: RepoId(1),
                edit: WorkspaceEdit::SetApplied { .. },
            }
        ));
        assert!(state.repos[0].workspace.busy.mutating);
    }

    #[test]
    fn a_second_edit_is_refused_while_one_is_running() {
        let mut state = repo_with_workspace();
        set_branch_applied(
            &mut state,
            RepoId(1),
            "api".into(),
            BranchApplyState::Unapplied,
        );
        let effects = set_branch_applied(
            &mut state,
            RepoId(1),
            "ui".into(),
            BranchApplyState::Unapplied,
        );
        assert!(effects.is_empty());
    }

    #[test]
    fn an_assignment_emits_the_assignment_effect() {
        let mut state = repo_with_workspace();
        let effects = apply_workspace_edit(
            &mut state,
            RepoId(1),
            WorkspaceEdit::AssignFile {
                path: PathBuf::from("a.rs"),
                branch: Some("api".into()),
            },
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::AssignWorkspaceFile {
                repo_id: RepoId(1),
                path,
                branch: Some(branch),
                ..
            } if path == &PathBuf::from("a.rs") && branch == "api"
        ));
        // It is workspace bookkeeping, not a history rewrite.
        assert!(!state.repos[0].workspace.busy.mutating);
        assert!(state.repos[0].workspace.busy.applying);
    }

    #[test]
    fn a_hunk_assignment_takes_the_same_effect_and_names_its_hunk() {
        let mut state = repo_with_workspace();
        let hunk = gitcomet_core::workspace::HunkKey {
            base_start: 3,
            base_lines: 1,
            new_lines: 2,
        };
        let effects = apply_workspace_edit(
            &mut state,
            RepoId(1),
            WorkspaceEdit::AssignHunk {
                path: PathBuf::from("a.rs"),
                hunk,
                branch: Some("ui".into()),
            },
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::AssignWorkspaceFile {
                repo_id: RepoId(1),
                hunk: Some(key),
                branch: Some(branch),
                ..
            } if *key == hunk && branch == "ui"
        ));
    }

    #[test]
    fn a_finished_assignment_lands_in_the_store_without_a_reload() {
        let mut state = repo_with_workspace();
        workspace::workspace_assign_finished(
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
            None,
            Some("api".into()),
            Ok(()),
        );
        assert_eq!(
            state.repos[0].workspace.assignments.branch_of(Path::new("a.rs")),
            Some("api"),
            "the row has to update without waiting for the next load"
        );
        assert!(!state.repos[0].workspace.busy.applying);
    }

    #[test]
    fn a_failed_assignment_reloads_rather_than_guessing() {
        let mut state = repo_with_workspace();
        let effects = workspace::workspace_assign_finished(
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
            None,
            Some("api".into()),
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("no".into()),
            )),
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadWorkspace { .. })),
            "the store copy is left alone, so it has to come back from disk"
        );
    }

    #[test]
    fn committing_an_empty_message_is_refused() {
        let mut state = repo_with_workspace();
        let effects = commit_branch(&mut state, RepoId(1), "api".into(), "   ".into());
        assert!(effects.is_empty());
        assert!(!state.repos[0].workspace.busy.committing);
    }

    #[test]
    fn committing_a_branch_with_no_assigned_files_is_refused() {
        let mut state = repo_with_workspace();
        let effects = commit_branch(&mut state, RepoId(1), "api".into(), "Add API".into());
        assert!(effects.is_empty());
    }

    #[test]
    fn committing_a_branch_with_assigned_files_emits_the_edit() {
        let mut state = repo_with_workspace();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        state.repos[0].workspace.assignments = Arc::new(index);

        let effects = commit_branch(&mut state, RepoId(1), "api".into(), "Add API".into());
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ApplyWorkspaceEdit {
                repo_id,
                edit: WorkspaceEdit::CommitPaths { name, paths, .. },
            } => {
                assert_eq!(*repo_id, RepoId(1));
                assert_eq!(name, "api");
                assert_eq!(paths, &[PathBuf::from("a.rs")]);
            }
            other => panic!("unexpected effect {other:?}"),
        }
        assert!(state.repos[0].workspace.busy.committing);
    }

    #[test]
    fn a_failed_edit_records_a_conflict_and_reloads() {
        let mut state = repo_with_workspace();
        let effects = workspace_edit_finished(
            &mut state,
            RepoId(1),
            WorkspaceEdit::SetApplied {
                name: "api".into(),
                applied: BranchApplyState::Applied,
            },
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("conflict".into()),
            )),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::LoadWorkspace { .. }));
        let conflict = state.repos[0].workspace.conflict.as_ref().unwrap();
        assert_eq!(conflict.branch, "api");
        assert!(!state.repos[0].workspace.busy.any());
    }

    #[test]
    fn a_successful_edit_clears_the_conflict_and_reloads_status() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.conflict = None;
        let effects = workspace_edit_finished(
            &mut state,
            RepoId(1),
            WorkspaceEdit::SetApplied {
                name: "api".into(),
                applied: BranchApplyState::Applied,
            },
            Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        );
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadWorktreeStatus { .. })));
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadWorkspace { .. })));
        assert!(state.repos[0].workspace.conflict.is_none());
        assert!(!state.repos[0].workspace.busy.any());
    }

    #[test]
    fn dismissing_a_conflict_clears_it() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.conflict = Some(crate::model::WorkspaceConflict {
            branch: "api".into(),
            against: None,
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        });
        dismiss_conflict(&mut state, RepoId(1));
        assert!(state.repos[0].workspace.conflict.is_none());
    }

    #[test]
    fn a_failed_push_notifies_without_reloading() {
        let mut state = repo_with_workspace();
        let effects = workspace_push_finished(
            &mut state,
            RepoId(1),
            "api".into(),
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("no remote".into()),
            )),
        );
        assert!(effects.is_empty());
        assert!(!state.repos[0].workspace.busy.mutating);
    }

    #[test]
    fn messages_for_an_unknown_repository_are_ignored() {
        let mut state = repo_with_workspace();
        let effects = load_workspace(&mut state, RepoId(99));
        assert!(effects.is_empty());
    }
}
