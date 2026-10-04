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
use std::path::{Path, PathBuf};
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

/// Switch the working directory into the workspace.
///
/// The branch to return to is read here rather than in the effect, because it
/// is the reducer that sees the repository state and the effect that has
/// already queued a command by the time anybody could ask.
pub(super) fn enter_workspace(
    state: &mut AppState,
    repo_id: RepoId,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.busy.any() {
        return Vec::new();
    }
    // Already on the workspace branch: entering again is a no-op, and re-running
    // the checkout would only risk a refusal over changes that are already in
    // place.
    if repo.workspace.active {
        return Vec::new();
    }
    let checkout_base = match &repo.head_branch {
        Loadable::Ready(branch) => Some(branch.clone()),
        // Detached HEAD, or not loaded yet: there is no branch to go back to,
        // and leaving then falls back to the workspace target.
        _ => None,
    };
    repo.workspace.busy.switching = true;
    repo.workspace.bump_rev();
    vec![Effect::EnterWorkspace {
        repo_id,
        checkout_base,
    }]
}

/// Switch the working directory back off the workspace branch.
pub(super) fn leave_workspace(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.workspace.busy.any() || !repo.workspace.active {
        return Vec::new();
    }
    let checkout_base = repo.workspace.checkout_base.clone();
    repo.workspace.busy.switching = true;
    repo.workspace.bump_rev();
    vec![Effect::LeaveWorkspace {
        repo_id,
        checkout_base,
    }]
}

/// Record the outcome of entering or leaving.
///
/// A failure leaves `active` and `checkout_base` exactly as they were, so the
/// view keeps describing the repository as it really is: a refused checkout
/// never moved HEAD, and pretending it had would make the next leave try to
/// undo a switch that never happened.
pub(super) fn workspace_active_finished(
    state: &mut AppState,
    repo_id: RepoId,
    active: bool,
    checkout_base: Option<String>,
    result: gitcomet_core::services::Result<()>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.workspace.busy.switching = false;

    if let Err(error) = result {
        repo.workspace.bump_rev();
        push_notification(
            state,
            AppNotificationKind::Error,
            if active {
                format!("Could not open the workspace: {error}")
            } else {
                format!("Could not switch back out of the workspace: {error}")
            },
        );
        return vec![Effect::LoadWorkspace { repo_id }];
    }

    repo.workspace.active = active;
    repo.workspace.checkout_base = if active { checkout_base } else { None };
    repo.workspace.bump_rev();
    // Leaving changes which branch HEAD is on, and entering changes the files,
    // so the ordinary views have to catch up either way.
    vec![
        Effect::LoadWorktreeStatus { repo_id },
        Effect::LoadBranches { repo_id },
    ]
}

/// The other branch named by a workspace conflict message.
///
/// The backend writes `"'x' conflicts with 'y' in ..."` when it could attribute
/// the conflict to a specific already-applied branch, and falls back to prose
/// when it could not. Reading the quote out is deliberately narrow: a message
/// that happens to contain the word "with" is not a conflict attribution.
fn conflict_against(message: &str) -> Option<String> {
    let rest = message.strip_prefix('\'')?;
    let (_, after) = rest.split_once("' conflicts with '")?;
    let (other, _) = after.split_once('\'')?;
    (!other.is_empty()).then(|| other.to_string())
}

/// The conflicting path a workspace conflict message names.
///
/// Only the first one is recovered: the backend collapses longer lists into
/// prose, because a conflict the user cannot act on is better shown as a
/// sentence than as a truncated array. The full text stays in `message`, which
/// is what the banner actually shows.
fn conflict_paths(message: &str) -> Vec<PathBuf> {
    let Some((_, tail)) = message.rsplit_once(" in ") else {
        return Vec::new();
    };
    let listed = tail.split(" (").next().unwrap_or(tail).trim();
    // A list git did not provide, or a count, is not a path.
    if listed.is_empty() || listed.contains(' ') || listed.contains('/') && listed.matches('/').count() > 8 {
        return Vec::new();
    }
    vec![PathBuf::from(listed)]
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
    if let Ok(mut index) = assignments {
        // A branch that was removed from the workspace must not leave its file
        // assignments behind: re-creating a branch with the same name would
        // otherwise silently re-adopt files the user assigned to it months ago,
        // which is a change nobody asked for and cannot see.
        //
        // Only the branch half is pruned here. Dropping assignments for paths
        // Git no longer reports as changed needs a diff, and that belongs to
        // the effects layer; doing it without one would drop assignments for
        // files that are merely unmodified at this instant — the very mistake
        // the assignments file exists to avoid.
        if let Some(state) = repo.workspace.workspace().cloned() {
            index.retain(&state, &|_| true, None);
        }
        repo.workspace.assignments = Arc::new(index);
    }
    // A reload can land while the working directory is sitting on the workspace
    // branch — the user was there before a refresh, or after a restart that left
    // HEAD there. Nothing is going to tell us we are "active" any other way, and
    // guessing wrong would either hide the fact or send the user to a branch
    // they were never on.
    repo.workspace.active = matches!(&repo.head_branch, Loadable::Ready(branch) if branch == gitcomet_core::workspace::WORKSPACE_BRANCH);
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
        // The backend names the other branch when it can work out which one is
        // in the way. Without that the banner can only say "X could not be
        // applied", which is the traditional conflict message the Workspace is
        // supposed to replace with one about virtual branches.
        let message = error.to_string();
        repo.workspace.conflict = Some(crate::model::WorkspaceConflict {
            branch: branch.to_string(),
            against: conflict_against(&message),
            paths: Arc::new(conflict_paths(&message)),
            message,
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
    hunk: Option<gitcomet_core::workspace::HunkFingerprint>,
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
    fn a_conflict_names_the_branch_it_is_in_the_way_of() {
        // This is the whole point of the Workspace conflict banner: "api and ui
        // changed the same lines" is something the user can act on, where the
        // git-native message only says a merge failed.
        let mut state = repo_with_workspace();
        let error = gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Backend(
                "'ui' conflicts with 'api' in src/auth.ts\n\n\
                 Both are applied, and both changed the same lines."
                    .into(),
            ),
        );
        workspace_edit_finished(
            &mut state,
            RepoId(1),
            WorkspaceEdit::SetApplied {
                name: "ui".into(),
                applied: BranchApplyState::Applied,
            },
            Err(error),
        );

        let conflict = state.repos[0].workspace.conflict.as_ref().unwrap();
        assert_eq!(conflict.branch, "ui");
        assert_eq!(conflict.against.as_deref(), Some("api"));
        assert_eq!(conflict.paths.len(), 1);
        assert_eq!(conflict.paths[0], PathBuf::from("src/auth.ts"));
        assert_eq!(conflict.summary(), "ui conflicts with api");
    }

    #[test]
    fn a_conflict_the_backend_could_not_attribute_still_reports_the_branch() {
        // git named no partner, so the banner falls back rather than inventing
        // one — and must not read a branch name out of unrelated prose.
        let mut state = repo_with_workspace();
        let error = gitcomet_core::error::Error::new(
            gitcomet_core::error::ErrorKind::Backend(
                "'ui' could not be applied: it conflicts with the branches already \
                 in the workspace, in files git did not name"
                    .into(),
            ),
        );
        workspace_edit_finished(
            &mut state,
            RepoId(1),
            WorkspaceEdit::SetApplied {
                name: "ui".into(),
                applied: BranchApplyState::Applied,
            },
            Err(error),
        );

        let conflict = state.repos[0].workspace.conflict.as_ref().unwrap();
        assert_eq!(conflict.against, None, "better silent than wrong");
        assert!(
            conflict.paths.is_empty(),
            "git named none, so there are none to show"
        );
        assert_eq!(conflict.summary(), "ui could not be applied");
    }

    #[test]
    fn loading_drops_assignments_naming_a_branch_that_is_gone() {
        // Without this, removing `api` from the workspace and re-creating it
        // later silently hands it every file it was ever given, with no action
        // from the user and nothing on screen saying so.
        let mut state = repo_with_workspace();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("live.rs"), Some("api".into()));
        index.set(PathBuf::from("gone.rs"), Some("removed".into()));

        // `repo_with_workspace` has api and ui, so `removed` is the odd one out.
        workspace_loaded(
            &mut state,
            RepoId(1),
            Ok(WorkspaceState::new("main").with_branches(vec![VirtualBranch::new("api")])),
            Ok(index),
            Ok(None),
        );

        let assignments = &state.repos[0].workspace.assignments;
        assert_eq!(assignments.branch_of(Path::new("live.rs")), Some("api"));
        assert!(
            assignments.file(Path::new("gone.rs")).is_none(),
            "an assignment for a branch that no longer exists has nothing to act on"
        );
    }

    #[test]
    fn loading_keeps_assignments_for_files_that_are_merely_unmodified() {
        // The whole reason assignments live in a file rather than being derived
        // from the diff: a file that is saved-but-unchanged must keep its branch.
        let mut state = repo_with_workspace();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("clean.rs"), Some("api".into()));

        workspace_loaded(
            &mut state,
            RepoId(1),
            Ok(WorkspaceState::new("main").with_branches(vec![VirtualBranch::new("api")])),
            Ok(index),
            Ok(None),
        );

        assert_eq!(
            state.repos[0]
                .workspace
                .assignments
                .branch_of(Path::new("clean.rs")),
            Some("api"),
            "pruning here must not turn this into a diff-derived answer"
        );
    }

    #[test]
    fn switching_repositories_hands_back_a_workspace_the_user_was_in() {
        // The checkout belongs to the repository; the tab that explains it
        // belongs to the window. Walking away from a repository tab must not
        // leave that repository sitting on `gitcomet/workspace`, because the
        // sidebar stays on Workspace and the next time it is picked up the
        // user would be looking at the wrong repository's branches.
        let mut state = repo_with_workspace();
        state.active_repo = Some(RepoId(1));
        state.repos[0].workspace.active = true;
        state.repos[0].workspace.checkout_base = Some("main".into());
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/other"),
            },
        ));

        let effects = super::super::repo_management::set_active_repo(
            &rustc_hash::FxHashMap::default(),
            &mut state,
            RepoId(2),
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(
                    effect,
                    Effect::LeaveWorkspace { repo_id: RepoId(1), .. }
                )),
            "the repository left behind has to be handed its working directory back"
        );
    }

    #[test]
    fn switching_repositories_untouched_by_the_workspace_costs_nothing() {
        // The common case: a repository that was never in the workspace must not
        // grow a checkout on every repository switch.
        let mut state = repo_with_workspace();
        state.active_repo = Some(RepoId(1));
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/other"),
            },
        ));

        let effects = super::super::repo_management::set_active_repo(
            &rustc_hash::FxHashMap::default(),
            &mut state,
            RepoId(2),
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::LeaveWorkspace { .. })),
            "nothing to hand back"
        );
    }

    #[test]
    fn entering_the_workspace_remembers_where_to_go_back_to() {
        let mut state = repo_with_workspace();
        let repo = state.repos.iter_mut().find(|r| r.id == RepoId(1)).unwrap();
        repo.head_branch = Loadable::Ready("main".into());

        let effects = enter_workspace(&mut state, RepoId(1));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::EnterWorkspace {
                repo_id: RepoId(1),
                checkout_base: Some(base),
            } if base == "main"
        ));
        assert!(
            state.repos[0].workspace.busy.switching,
            "moving HEAD under the user is its own kind of busy"
        );
    }

    #[test]
    fn entering_the_workspace_twice_does_nothing_the_second_time() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.active = true;
        // Re-running the checkout would only risk a refusal over changes that
        // are already in place.
        assert!(enter_workspace(&mut state, RepoId(1)).is_empty());
    }

    #[test]
    fn entering_the_workspace_is_refused_while_something_else_is_running() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.busy.committing = true;
        assert!(enter_workspace(&mut state, RepoId(1)).is_empty());
        assert!(!state.repos[0].workspace.busy.switching);
    }

    #[test]
    fn entering_with_a_detached_head_remembers_nothing_to_go_back_to() {
        let mut state = repo_with_workspace();
        state.repos[0].head_branch = Loadable::Ready(String::new());
        assert!(matches!(
            &enter_workspace(&mut state, RepoId(1))[0],
            Effect::EnterWorkspace { checkout_base: None, .. }
        ));
    }

    #[test]
    fn leaving_the_workspace_asks_for_the_branch_it_came_from() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.active = true;
        state.repos[0].workspace.checkout_base = Some("main".into());

        assert!(matches!(
            &leave_workspace(&mut state, RepoId(1))[0],
            Effect::LeaveWorkspace {
                repo_id: RepoId(1),
                checkout_base: Some(base),
            } if base == "main"
        ));
    }

    #[test]
    fn leaving_a_workspace_that_was_never_entered_does_nothing() {
        let mut state = repo_with_workspace();
        assert!(leave_workspace(&mut state, RepoId(1)).is_empty());
        assert!(!state.repos[0].workspace.busy.switching);
    }

    #[test]
    fn a_finished_switch_records_where_the_working_directory_is() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.busy.switching = true;
        workspace_active_finished(&mut state, RepoId(1), true, Some("main".into()), Ok(()));

        let workspace = &state.repos[0].workspace;
        assert!(workspace.active);
        assert_eq!(workspace.checkout_base.as_deref(), Some("main"));
        assert!(!workspace.busy.switching);
    }

    #[test]
    fn a_finished_leave_forgets_where_to_go_back_to() {
        let mut state = repo_with_workspace();
        state.repos[0].workspace.active = true;
        state.repos[0].workspace.checkout_base = Some("main".into());
        workspace_active_finished(&mut state, RepoId(1), false, None, Ok(()));

        let workspace = &state.repos[0].workspace;
        assert!(!workspace.active);
        assert!(
            workspace.checkout_base.is_none(),
            "the remembered branch is only meaningful while the workspace is checked out"
        );
    }

    #[test]
    fn a_refused_switch_leaves_the_working_directory_described_as_it_is() {
        // The property that stops the view lying after a refused checkout: git
        // never moved HEAD, so claiming we are on the workspace would make the
        // next leave try to undo a switch that never happened.
        let mut state = repo_with_workspace();
        state.repos[0].workspace.busy.switching = true;
        let effects = workspace_active_finished(
            &mut state,
            RepoId(1),
            true,
            Some("main".into()),
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("would overwrite local changes".into()),
            )),
        );

        let workspace = &state.repos[0].workspace;
        assert!(!workspace.active, "HEAD never moved");
        assert!(workspace.checkout_base.is_none());
        assert!(!workspace.busy.switching, "but the operation is over");
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadWorkspace { repo_id: RepoId(1) })),
            "and the workspace is re-read so the view matches the repository"
        );
    }

    #[test]
    fn a_switch_reloads_the_branches_and_the_working_tree() {
        // Entering changes the files and leaving changes which branch HEAD is
        // on, so both ordinary views are now stale.
        let mut state = repo_with_workspace();
        let effects = workspace_active_finished(&mut state, RepoId(1), true, None, Ok(()));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadWorktreeStatus { repo_id: RepoId(1) }
        )));
        assert!(effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadBranches { repo_id: RepoId(1) })));
    }

    #[test]
    fn a_load_infers_that_the_working_directory_is_on_the_workspace() {
        // Nothing else will say so, and it has to survive a restart that left
        // HEAD on `gitcomet/workspace`.
        let mut state = repo_with_workspace();
        state.repos[0].head_branch =
            Loadable::Ready(gitcomet_core::workspace::WORKSPACE_BRANCH.into());
        workspace_loaded(
            &mut state,
            RepoId(1),
            Ok(WorkspaceState::new("main")),
            Ok(AssignmentIndex::new()),
            Ok(None),
        );
        assert!(state.repos[0].workspace.active);
    }

    #[test]
    fn a_hunk_assignment_takes_the_same_effect_and_names_its_hunk() {
        let mut state = repo_with_workspace();
        let hunk = gitcomet_core::workspace::HunkFingerprint::of(&["new\n"], 1);
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
