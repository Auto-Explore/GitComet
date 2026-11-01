//! Virtual branches reducers: turn Virtual branches messages into state changes and the
//! effects that keep the repository in step.
//!
//! The pattern throughout is that a virtual branches action is optimistic about the
//! *repository* and pessimistic about the *state file*: the edit goes to the
//! backend first, and the reducer's copy is only updated once the backend
//! reports what actually happened. A failed apply therefore leaves the UI
//! showing the truth rather than a branch the user thinks exists.

use super::prepared::ReducerInputs;
use super::util::{push_diagnostic, push_notification};
use crate::model::{
    AppNotificationKind, AppState, DiagnosticKind, Loadable, RepoId, RepoState, VirtualBranchesEdit,
};
use crate::msg::{CheckoutRequest, Effect};
use crate::msg::{ReferenceEffect, StatusEffect};
use gitcomet_core::virtual_branches::{AssignmentIndex, BranchApplyState, VirtualBranchesState};
use std::path::PathBuf;
use std::sync::Arc;

/// The Virtual branches view asks for its data; nothing else does, so a repository
/// that never opens it never reads a virtual branches file.
pub(super) fn load_virtual_branches(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.busy.loading = true;
    repo.virtual_branches.state = Loadable::Loading;
    vec![Effect::LoadVirtualBranches { repo_id }]
}

/// Read the branch's row button: apply or unapply.
pub(super) fn set_branch_applied(
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    applied: BranchApplyState,
) -> Vec<Effect> {
    // Applying or unapplying always rewrites `gitcomet/virtual`, so it takes
    // the same busy gate as a stacked branch change.
    apply_edit(
        state,
        repo_id,
        VirtualBranchesEdit::SetApplied { name, applied },
        true,
    )
}

/// Any other virtual branches edit from the UI.
pub(super) fn apply_virtual_branches_edit(
    state: &mut AppState,
    repo_id: RepoId,
    edit: VirtualBranchesEdit,
) -> Vec<Effect> {
    // A file assignment writes only the assignment index, so it gets its own
    // effect rather than going through the virtual branches rewrite path. Hunk
    // assignments take the same road: they write the same file.
    //
    // Matched by reference so the edit survives for `rewrites_history` and for
    // the effect below; the assignment owns copies rather than the fields.
    let assignment = match &edit {
        VirtualBranchesEdit::AssignFile { path, branch } => {
            Some((path.clone(), None, branch.clone()))
        }
        VirtualBranchesEdit::AssignHunk { path, hunk, branch } => {
            Some((path.clone(), Some(*hunk), branch.clone()))
        }
        _ => None,
    };
    if let Some((path, hunk, branch)) = assignment {
        let Some(repo) = repo_mut(state, repo_id) else {
            return Vec::new();
        };
        if repo.virtual_branches.busy.any() {
            return Vec::new();
        }
        repo.virtual_branches.busy.applying = true;
        repo.virtual_branches.bump_rev();
        return vec![Effect::AssignVirtualBranchesFile {
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
    edit: VirtualBranchesEdit,
    mutating: bool,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.virtual_branches.busy.any() {
        // Two virtual branches edits racing would each write the state file and the
        // loser's change would vanish silently.
        return Vec::new();
    }
    if mutating {
        repo.virtual_branches.busy.mutating = true;
    } else {
        repo.virtual_branches.busy.applying = true;
    }
    repo.virtual_branches.bump_rev();

    vec![Effect::ApplyVirtualBranchesEdit { repo_id, edit }]
}

/// Commit every file assigned to `name`.
pub(super) fn commit_branch(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    message: String,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.virtual_branches.busy.any() {
        return Vec::new();
    }
    // An empty message would create a commit git refuses to describe; the
    // commit form is the right place to require one, and an empty submit here
    // means the user has not typed anything yet.
    if message.trim().is_empty() {
        return Vec::new();
    }
    let paths: Vec<PathBuf> = repo
        .virtual_branches
        .assignments
        .paths_for(&name)
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if paths.is_empty() {
        push_notification(
            inputs,
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
/// committing it to the virtual-branches branch, which is never a user commit target.
pub(super) fn commit_single_file(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
) -> Vec<Effect> {
    let branch = {
        let Some(repo) = repo_mut(state, repo_id) else {
            return Vec::new();
        };
        let Some(virtual_branches) = repo.virtual_branches.virtual_branches() else {
            return Vec::new();
        };
        repo.virtual_branches
            .assignments
            .resolve(&path, virtual_branches)
            .map(str::to_string)
    };
    let Some(branch) = branch else {
        push_notification(
            inputs,
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
    if repo.virtual_branches.busy.any() {
        return Vec::new();
    }
    repo.virtual_branches.busy.committing = true;
    repo.virtual_branches.bump_rev();

    vec![Effect::ApplyVirtualBranchesEdit {
        repo_id,
        edit: VirtualBranchesEdit::CommitPaths {
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
    if repo.virtual_branches.busy.any() {
        return Vec::new();
    }
    repo.virtual_branches.busy.mutating = true;
    repo.virtual_branches.bump_rev();
    vec![Effect::PushVirtualBranchesBranch { repo_id, name }]
}

/// Switch the working directory into the virtual branches.
///
/// The branch to return to is read here rather than in the effect, because it
/// is the reducer that sees the repository state and the effect that has
/// already queued a command by the time anybody could ask.
pub(super) fn enter_virtual_branches(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.virtual_branches.busy.any() {
        return Vec::new();
    }
    // Already on the virtual-branches branch: entering again is a no-op, and re-running
    // the checkout would only risk a refusal over changes that are already in
    // place.
    if repo.virtual_branches.active {
        return Vec::new();
    }
    let checkout_base = match &repo.head_branch {
        // A detached HEAD reports its name as "HEAD" — and an empty name is
        // no branch either. Not loaded is the same answer from less
        // information. Either way there is no branch to go back to, and
        // leaving then falls back to the virtual branches target.
        Loadable::Ready(branch) if !branch.is_empty() && branch != "HEAD" => Some(branch.clone()),
        _ => None,
    };
    repo.virtual_branches.busy.switching = true;
    repo.virtual_branches.bump_rev();
    vec![Effect::EnterVirtualBranches {
        repo_id,
        checkout_base,
    }]
}

/// Switch the working directory back off the virtual-branches branch.
pub(super) fn leave_virtual_branches(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if !begin_leave(repo) {
        return Vec::new();
    }
    let checkout_base = repo.virtual_branches.checkout_base.clone();
    vec![Effect::LeaveVirtualBranches {
        repo_id,
        checkout_base,
    }]
}

/// Whether the working directory is the virtual branches's to move.
///
/// This is the test every checkout asks before it may run: a virtual branches that
/// owns the working directory has to be left first, both because moving HEAD
/// out from under it would strand the two in disagreement and because the
/// views promise that the virtual branches stays on `gitcomet/virtual` while they
/// are showing it.
pub(super) fn owns_workdir(state: &AppState, repo_id: RepoId) -> bool {
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .is_some_and(|repo| repo.virtual_branches.active)
}

/// Mark the virtual branches as switching, answering whether a switch may start.
///
/// A virtual branches that is already busy — an apply in flight, or the working
/// directory being moved by an earlier request — cannot leave again, and the
/// caller reports the refusal instead of believing the move happened.
fn begin_leave(repo: &mut RepoState) -> bool {
    if repo.virtual_branches.busy.any() || !repo.virtual_branches.active {
        return false;
    }
    repo.virtual_branches.busy.switching = true;
    repo.virtual_branches.bump_rev();
    true
}

/// Leave the virtual branches to run a checkout that asked for what it holds.
///
/// `checkout_base` is where the leave puts the working directory: the branch
/// or commit the user asked for when it already exists, or `None` to fall back
/// to the branch the virtual branches remembers (which is what a checkout that still
/// has to create its branch needs). `then` runs only once the leave has been
/// reported, so the two moves cannot be split by a failure in between.
pub(super) fn leave_for_checkout(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    checkout_base: Option<String>,
    then: CheckoutRequest,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if !repo.virtual_branches.active {
        // The working directory stopped being the virtual branches's between the
        // check and here; the ordinary checkout path already handles that.
        return Vec::new();
    }
    let may_leave = begin_leave(repo);
    if !may_leave {
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            "The virtual branches is busy; try changing branch again in a moment.".to_string(),
        );
        return Vec::new();
    }
    vec![Effect::LeaveVirtualBranchesAndCheckout {
        repo_id,
        checkout_base,
        then,
    }]
}

/// Record the outcome of entering or leaving.
///
/// A failure leaves `active` and `checkout_base` exactly as they were, so the
/// view keeps describing the repository as it really is: a refused checkout
/// never moved HEAD, and pretending it had would make the next leave try to
/// undo a switch that never happened.
pub(super) fn virtual_branches_active_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    active: bool,
    checkout_base: Option<String>,
    result: gitcomet_core::services::Result<()>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.busy.switching = false;

    if let Err(error) = result {
        repo.virtual_branches.bump_rev();
        push_notification(
            inputs,
            state,
            AppNotificationKind::Error,
            if active {
                format!("Could not open the virtual branches: {error}")
            } else {
                format!("Could not switch back out of the virtual branches: {error}")
            },
        );
        return vec![Effect::LoadVirtualBranches { repo_id }];
    }

    repo.virtual_branches.active = active;
    repo.virtual_branches.checkout_base = if active { checkout_base } else { None };
    repo.virtual_branches.bump_rev();
    // Leaving changes which branch HEAD is on, and entering changes the files,
    // so the ordinary views have to catch up either way.
    vec![
        Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id }),
        Effect::References(ReferenceEffect::LoadBranches { repo_id }),
    ]
}

/// The other branch named by a virtual branches conflict message.
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

/// The conflicting path a virtual branches conflict message names.
///
/// Only the first one is recovered: the backend collapses longer lists into
/// prose, because a conflict the user cannot act on is better shown as a
/// sentence than as a truncated array. The full text stays in `message`, which
/// is what the banner actually shows.
fn conflict_paths(message: &str) -> Vec<PathBuf> {
    let Some((_, tail)) = message.rsplit_once(" in ") else {
        return Vec::new();
    };
    // `conflict_message` puts the paths on one line and then a sentence saying
    // what to do about them, so only the first line of the tail is the path
    // list. Taking the whole tail — which is what this used to do — always
    // swallowed that sentence, and since a single-path conflict carries no
    // " (" to cut on, it recovered nothing at all: the common case reported no
    // paths while the backend had named one.
    let listed = tail.lines().next().unwrap_or_default().trim();
    // A longer list is collapsed to "<first> (and N more)", which is prose.
    let listed = listed.split(" (").next().unwrap_or(listed).trim();
    // So is "files git did not name". A path git reports has no whitespace in
    // it, which is the only thing separating the two.
    if listed.is_empty() || listed.contains(char::is_whitespace) {
        return Vec::new();
    }
    vec![PathBuf::from(listed)]
}

pub(super) fn dismiss_conflict(state: &mut AppState, repo_id: RepoId) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    if repo.virtual_branches.conflict.take().is_some() {
        repo.virtual_branches.bump_rev();
    }
    Vec::new()
}

// ── Backend replies ─────────────────────────────────────────────

pub(super) fn virtual_branches_loaded(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    result: gitcomet_core::services::Result<VirtualBranchesState>,
    assignments: gitcomet_core::services::Result<AssignmentIndex>,
    virtual_branches_commit: gitcomet_core::services::Result<
        Option<gitcomet_core::domain::CommitId>,
    >,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.busy.loading = false;
    match &result {
        // `set_state` derives the stacked branches and stores the state, so the loadable
        // and the cached stacked branches can never disagree.
        Ok(loaded) => {
            repo.virtual_branches.set_state(loaded.clone());
            repo.virtual_branches.virtual_branches_commit = match &virtual_branches_commit {
                Ok(commit) => Loadable::Ready(commit.clone()),
                Err(error) => Loadable::Error(error.to_string()),
            };
        }
        Err(error) => repo.virtual_branches.state = Loadable::Error(error.to_string()),
    };
    if let Ok(mut index) = assignments {
        // A branch that was removed from the virtual branches must not leave its file
        // assignments behind: re-creating a branch with the same name would
        // otherwise silently re-adopt files the user assigned to it months ago,
        // which is a change nobody asked for and cannot see.
        //
        // Only the branch half is pruned here. Dropping assignments for paths
        // Git no longer reports as changed needs a diff, and that belongs to
        // the effects layer; doing it without one would drop assignments for
        // files that are merely unmodified at this instant — the very mistake
        // the assignments file exists to avoid.
        if let Some(state) = repo.virtual_branches.virtual_branches().cloned() {
            index.retain(&state, &|_| true, None);
        }
        repo.virtual_branches.assignments = Arc::new(index);
    }
    // A reload can land while the working directory is sitting on the virtual branches
    // branch — the user was there before a refresh, or after a restart that left
    // HEAD there. Nothing is going to tell us we are "active" any other way, and
    // guessing wrong would either hide the fact or send the user to a branch
    // they were never on.
    repo.virtual_branches.active = matches!(&repo.head_branch, Loadable::Ready(branch) if branch == gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH);
    repo.virtual_branches.bump_rev();

    let mut effects = Vec::new();
    if let Err(error) = result {
        push_notification(
            inputs,
            state,
            AppNotificationKind::Error,
            format!("Could not load the virtual branches: {error}"),
        );
    }
    // A branch the virtual branches does not know about yet cannot be applied, so the
    // view needs the ordinary branch list alongside the virtual branches.
    effects.push(Effect::References(ReferenceEffect::LoadBranches {
        repo_id,
    }));
    effects
}

pub(super) fn virtual_branches_edit_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    edit: VirtualBranchesEdit,
    result: gitcomet_core::services::Result<gitcomet_core::services::CommitOperationOutcome>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.busy = Default::default();
    repo.virtual_branches.bump_rev();

    if let Err(error) = result {
        let branch = edit.branch_name().unwrap_or("virtual branches");
        // The backend names the other branch when it can work out which one is
        // in the way. Without that the banner can only say "X could not be
        // applied", which is the traditional conflict message the Virtual branches is
        // supposed to replace with one about virtual branches.
        let message = error.to_string();
        repo.virtual_branches.conflict = Some(crate::model::VirtualBranchesConflict {
            branch: branch.to_string(),
            against: conflict_against(&message),
            paths: Arc::new(conflict_paths(&message)),
            message,
        });
        push_diagnostic(inputs, repo, DiagnosticKind::Error, error.to_string());
        // The edit did not happen, so the cached state is stale in the other
        // direction: reload rather than guess what survived.
        return vec![Effect::LoadVirtualBranches { repo_id }];
    }

    repo.virtual_branches.conflict = None;
    vec![
        Effect::LoadVirtualBranches { repo_id },
        // A commit or an apply can change what the working tree reports.
        Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id }),
        Effect::References(ReferenceEffect::LoadBranches { repo_id }),
    ]
}

pub(super) fn virtual_branches_assign_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    hunk: Option<gitcomet_core::virtual_branches::HunkFingerprint>,
    branch: Option<String>,
    result: gitcomet_core::services::Result<()>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.busy.applying = false;

    if let Err(error) = result {
        repo.virtual_branches.bump_rev();
        push_notification(
            inputs,
            state,
            AppNotificationKind::Error,
            format!("Could not assign '{}': {error}", path.display()),
        );
        return vec![Effect::LoadVirtualBranches { repo_id }];
    }

    // Mirror the write into the store. Reloading the whole virtual branches to learn
    // something the view already knows would discard the derived stacked branches and
    // rebuild them for nothing, and on a hunk-by-hunk assignment that is once
    // per hunk.
    let index = std::sync::Arc::make_mut(&mut repo.virtual_branches.assignments);
    match hunk {
        Some(hunk) => index.set_hunk(path, hunk, branch),
        None => index.set(path, branch),
    }
    repo.virtual_branches.bump_rev();
    Vec::new()
}

pub(super) fn virtual_branches_push_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    name: String,
    result: gitcomet_core::services::Result<gitcomet_core::services::CommandOutput>,
) -> Vec<Effect> {
    let Some(repo) = repo_mut(state, repo_id) else {
        return Vec::new();
    };
    repo.virtual_branches.busy.mutating = false;
    repo.virtual_branches.bump_rev();

    match result {
        Ok(_) => {
            push_notification(
                inputs,
                state,
                AppNotificationKind::Info,
                format!("Pushed '{name}'"),
            );
            vec![Effect::References(ReferenceEffect::LoadBranches {
                repo_id,
            })]
        }
        Err(error) => {
            push_notification(
                inputs,
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
    use crate::model::{RepoState, VirtualBranchesRepoState};
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::virtual_branches::VirtualBranch;
    use std::path::Path;

    fn repo_with_virtual_branches() -> AppState {
        let mut state = AppState::test_default();
        let spec = RepoSpec {
            workdir: std::path::PathBuf::from("/repo"),
        };
        let mut repo = RepoState::new_opening(RepoId(1), spec);
        let mut virtual_branches = VirtualBranchesRepoState::default();
        virtual_branches.set_state(VirtualBranchesState::new("main").with_branches(vec![
            VirtualBranch::new("api"),
            VirtualBranch::new("ui").with_parent("api"),
        ]));
        repo.virtual_branches = virtual_branches;
        state.repos.push(repo);
        state
    }

    #[test]
    fn load_virtual_branches_marks_loading_and_emits_the_effect() {
        let mut state = repo_with_virtual_branches();
        let effects = load_virtual_branches(&mut state, RepoId(1));
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::LoadVirtualBranches { .. }));
        assert!(state.repos[0].virtual_branches.busy.loading);
        assert!(matches!(
            state.repos[0].virtual_branches.state,
            Loadable::Loading
        ));
    }

    #[test]
    fn applying_a_branch_emits_the_edit_and_sets_mutating() {
        let mut state = repo_with_virtual_branches();
        let effects = set_branch_applied(
            &mut state,
            RepoId(1),
            "api".into(),
            BranchApplyState::Unapplied,
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::ApplyVirtualBranchesEdit {
                repo_id: RepoId(1),
                edit: VirtualBranchesEdit::SetApplied { .. },
            }
        ));
        assert!(state.repos[0].virtual_branches.busy.mutating);
    }

    #[test]
    fn a_second_edit_is_refused_while_one_is_running() {
        let mut state = repo_with_virtual_branches();
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
        let mut state = repo_with_virtual_branches();
        let effects = apply_virtual_branches_edit(
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::AssignFile {
                path: PathBuf::from("a.rs"),
                branch: Some("api".into()),
            },
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::AssignVirtualBranchesFile {
                repo_id: RepoId(1),
                path,
                branch: Some(branch),
                ..
            } if path == &PathBuf::from("a.rs") && branch == "api"
        ));
        // It is virtual branches bookkeeping, not a history rewrite.
        assert!(!state.repos[0].virtual_branches.busy.mutating);
        assert!(state.repos[0].virtual_branches.busy.applying);
    }

    #[test]
    fn a_conflict_names_the_branch_it_is_in_the_way_of() {
        // This is the whole point of the Virtual branches conflict banner: "api and ui
        // changed the same lines" is something the user can act on, where the
        // git-native message only says a merge failed.
        let mut state = repo_with_virtual_branches();
        let error = gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
            "'ui' conflicts with 'api' in src/auth.ts\n\n\
                 Both are applied, and both changed the same lines."
                .into(),
        ));
        virtual_branches_edit_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::SetApplied {
                name: "ui".into(),
                applied: BranchApplyState::Applied,
            },
            Err(error),
        );

        let conflict = state.repos[0].virtual_branches.conflict.as_ref().unwrap();
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
        let mut state = repo_with_virtual_branches();
        let error = gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Backend(
            "'ui' could not be applied: it conflicts with the branches already \
                 in the virtual branches, in files git did not name"
                .into(),
        ));
        virtual_branches_edit_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::SetApplied {
                name: "ui".into(),
                applied: BranchApplyState::Applied,
            },
            Err(error),
        );

        let conflict = state.repos[0].virtual_branches.conflict.as_ref().unwrap();
        assert_eq!(conflict.against, None, "better silent than wrong");
        assert!(
            conflict.paths.is_empty(),
            "git named none, so there are none to show"
        );
        assert_eq!(conflict.summary(), "ui could not be applied");
    }

    #[test]
    fn loading_drops_assignments_naming_a_branch_that_is_gone() {
        // Without this, removing `api` from the virtual branches and re-creating it
        // later silently hands it every file it was ever given, with no action
        // from the user and nothing on screen saying so.
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("live.rs"), Some("api".into()));
        index.set(PathBuf::from("gone.rs"), Some("removed".into()));

        // `repo_with_virtual_branches` has api and ui, so `removed` is the odd one out.
        virtual_branches_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            Ok(VirtualBranchesState::new("main").with_branches(vec![VirtualBranch::new("api")])),
            Ok(index),
            Ok(None),
        );

        let assignments = &state.repos[0].virtual_branches.assignments;
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
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("clean.rs"), Some("api".into()));

        virtual_branches_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            Ok(VirtualBranchesState::new("main").with_branches(vec![VirtualBranch::new("api")])),
            Ok(index),
            Ok(None),
        );

        assert_eq!(
            state.repos[0]
                .virtual_branches
                .assignments
                .branch_of(Path::new("clean.rs")),
            Some("api"),
            "pruning here must not turn this into a diff-derived answer"
        );
    }

    #[test]
    fn switching_repositories_hands_back_a_virtual_branches_the_user_was_in() {
        // The checkout belongs to the repository; the tab that explains it
        // belongs to the window. Walking away from a repository tab must not
        // leave that repository sitting on `gitcomet/virtual`, because the
        // sidebar stays on Virtual branches and the next time it is picked up the
        // user would be looking at the wrong repository's branches.
        let mut state = repo_with_virtual_branches();
        state.active_repo = Some(RepoId(1));
        state.repos[0].virtual_branches.active = true;
        state.repos[0].virtual_branches.checkout_base = Some("main".into());
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/other"),
            },
        ));

        let effects = super::super::repo_management::set_active_repo(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(2),
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::LeaveVirtualBranches {
                    repo_id: RepoId(1),
                    ..
                }
            )),
            "the repository left behind has to be handed its working directory back"
        );
    }

    #[test]
    fn switching_repositories_untouched_by_the_virtual_branches_costs_nothing() {
        // The common case: a repository that was never in the virtual branches must not
        // grow a checkout on every repository switch.
        let mut state = repo_with_virtual_branches();
        state.active_repo = Some(RepoId(1));
        state.repos.push(RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: std::path::PathBuf::from("/other"),
            },
        ));

        let effects = super::super::repo_management::set_active_repo(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(2),
        );
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::LeaveVirtualBranches { .. })),
            "nothing to hand back"
        );
    }

    #[test]
    fn entering_the_virtual_branches_remembers_where_to_go_back_to() {
        let mut state = repo_with_virtual_branches();
        let repo = state.repos.iter_mut().find(|r| r.id == RepoId(1)).unwrap();
        repo.head_branch = Loadable::Ready("main".into());
        let before_rev = state.repos[0].virtual_branches.rev;

        let effects = enter_virtual_branches(&mut state, RepoId(1));
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::EnterVirtualBranches {
                repo_id: RepoId(1),
                checkout_base: Some(base),
            } if base == "main"
        ));
        assert!(
            state.repos[0].virtual_branches.busy.switching,
            "moving HEAD under the user is its own kind of busy"
        );
        assert_ne!(
            state.repos[0].virtual_branches.rev, before_rev,
            "the presentation is cached on this counter, so a switch that does not bump it would leave the tab describing a working directory that has moved"
        );
    }

    #[test]
    fn entering_the_virtual_branches_twice_does_nothing_the_second_time() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.active = true;
        // Re-running the checkout would only risk a refusal over changes that
        // are already in place.
        assert!(enter_virtual_branches(&mut state, RepoId(1)).is_empty());
    }

    #[test]
    fn entering_the_virtual_branches_is_refused_while_something_else_is_running() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.busy.committing = true;
        assert!(enter_virtual_branches(&mut state, RepoId(1)).is_empty());
        assert!(!state.repos[0].virtual_branches.busy.switching);
    }

    #[test]
    fn entering_with_a_detached_head_remembers_nothing_to_go_back_to() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].head_branch = Loadable::Ready(String::new());
        assert!(matches!(
            &enter_virtual_branches(&mut state, RepoId(1))[0],
            Effect::EnterVirtualBranches {
                checkout_base: None,
                ..
            }
        ));
    }

    #[test]
    fn leaving_the_virtual_branches_asks_for_the_branch_it_came_from() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.active = true;
        state.repos[0].virtual_branches.checkout_base = Some("main".into());

        assert!(matches!(
            &leave_virtual_branches(&mut state, RepoId(1))[0],
            Effect::LeaveVirtualBranches {
                repo_id: RepoId(1),
                checkout_base: Some(base),
            } if base == "main"
        ));
    }

    #[test]
    fn leaving_a_virtual_branches_that_was_never_entered_does_nothing() {
        let mut state = repo_with_virtual_branches();
        assert!(leave_virtual_branches(&mut state, RepoId(1)).is_empty());
        assert!(!state.repos[0].virtual_branches.busy.switching);
    }

    #[test]
    fn a_finished_switch_records_where_the_working_directory_is() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.busy.switching = true;
        virtual_branches_active_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            true,
            Some("main".into()),
            Ok(()),
        );

        let virtual_branches = &state.repos[0].virtual_branches;
        assert!(virtual_branches.active);
        assert_eq!(virtual_branches.checkout_base.as_deref(), Some("main"));
        assert!(!virtual_branches.busy.switching);
    }

    #[test]
    fn a_finished_leave_forgets_where_to_go_back_to() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.active = true;
        state.repos[0].virtual_branches.checkout_base = Some("main".into());
        virtual_branches_active_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            false,
            None,
            Ok(()),
        );

        let virtual_branches = &state.repos[0].virtual_branches;
        assert!(!virtual_branches.active);
        assert!(
            virtual_branches.checkout_base.is_none(),
            "the remembered branch is only meaningful while the virtual branches is checked out"
        );
    }

    #[test]
    fn a_refused_switch_leaves_the_working_directory_described_as_it_is() {
        // The property that stops the view lying after a refused checkout: git
        // never moved HEAD, so claiming we are on the virtual branches would make the
        // next leave try to undo a switch that never happened.
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.busy.switching = true;
        let effects = virtual_branches_active_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            true,
            Some("main".into()),
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("would overwrite local changes".into()),
            )),
        );

        let virtual_branches = &state.repos[0].virtual_branches;
        assert!(!virtual_branches.active, "HEAD never moved");
        assert!(virtual_branches.checkout_base.is_none());
        assert!(
            !virtual_branches.busy.switching,
            "but the operation is over"
        );
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadVirtualBranches { repo_id: RepoId(1) })),
            "and the virtual branches is re-read so the view matches the repository"
        );
    }

    #[test]
    fn a_switch_reloads_the_branches_and_the_working_tree() {
        // Entering changes the files and leaving changes which branch HEAD is
        // on, so both ordinary views are now stale.
        let mut state = repo_with_virtual_branches();
        let effects = virtual_branches_active_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            true,
            None,
            Ok(()),
        );
        assert_ne!(
            state.repos[0].virtual_branches.rev, 0,
            "the switch has to reach the cached presentation, which only sees this counter"
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id: RepoId(1) })
        )));
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::References(ReferenceEffect::LoadBranches { repo_id: RepoId(1) })
        )));
    }

    #[test]
    fn a_load_infers_that_the_working_directory_is_on_the_virtual_branches() {
        // Nothing else will say so, and it has to survive a restart that left
        // HEAD on `gitcomet/virtual`.
        let mut state = repo_with_virtual_branches();
        state.repos[0].head_branch =
            Loadable::Ready(gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH.into());
        virtual_branches_loaded(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            Ok(VirtualBranchesState::new("main")),
            Ok(AssignmentIndex::new()),
            Ok(None),
        );
        assert!(state.repos[0].virtual_branches.active);
    }

    #[test]
    fn a_hunk_assignment_takes_the_same_effect_and_names_its_hunk() {
        let mut state = repo_with_virtual_branches();
        let hunk = gitcomet_core::virtual_branches::HunkFingerprint::of(&["new\n"], 1);
        let effects = apply_virtual_branches_edit(
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::AssignHunk {
                path: PathBuf::from("a.rs"),
                hunk,
                branch: Some("ui".into()),
            },
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(
            &effects[0],
            Effect::AssignVirtualBranchesFile {
                repo_id: RepoId(1),
                hunk: Some(key),
                branch: Some(branch),
                ..
            } if *key == hunk && branch == "ui"
        ));
    }

    #[test]
    fn a_finished_assignment_lands_in_the_store_without_a_reload() {
        let mut state = repo_with_virtual_branches();
        virtual_branches_assign_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
            None,
            Some("api".into()),
            Ok(()),
        );
        assert_eq!(
            state.repos[0]
                .virtual_branches
                .assignments
                .branch_of(Path::new("a.rs")),
            Some("api"),
            "the row has to update without waiting for the next load"
        );
        assert!(!state.repos[0].virtual_branches.busy.applying);
    }

    #[test]
    fn a_failed_assignment_reloads_rather_than_guessing() {
        let mut state = repo_with_virtual_branches();
        let effects = virtual_branches_assign_finished(
            &crate::store::reducer::test_inputs::clock(),
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
                .any(|effect| matches!(effect, Effect::LoadVirtualBranches { .. })),
            "the store copy is left alone, so it has to come back from disk"
        );
    }

    #[test]
    fn committing_an_empty_message_is_refused() {
        let mut state = repo_with_virtual_branches();
        let effects = commit_branch(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            "api".into(),
            "   ".into(),
        );
        assert!(effects.is_empty());
        assert!(!state.repos[0].virtual_branches.busy.committing);
    }

    #[test]
    fn committing_a_branch_with_no_assigned_files_is_refused() {
        let mut state = repo_with_virtual_branches();
        let effects = commit_branch(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            "api".into(),
            "Add API".into(),
        );
        assert!(effects.is_empty());
    }

    #[test]
    fn committing_a_branch_with_assigned_files_emits_the_edit() {
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        state.repos[0].virtual_branches.assignments = Arc::new(index);

        let effects = commit_branch(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            "api".into(),
            "Add API".into(),
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ApplyVirtualBranchesEdit {
                repo_id,
                edit: VirtualBranchesEdit::CommitPaths { name, paths, .. },
            } => {
                assert_eq!(*repo_id, RepoId(1));
                assert_eq!(name, "api");
                assert_eq!(paths, &[PathBuf::from("a.rs")]);
            }
            other => panic!("unexpected effect {other:?}"),
        }
        assert!(state.repos[0].virtual_branches.busy.committing);
    }

    #[test]
    fn a_failed_edit_records_a_conflict_and_reloads() {
        let mut state = repo_with_virtual_branches();
        let effects = virtual_branches_edit_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::SetApplied {
                name: "api".into(),
                applied: BranchApplyState::Applied,
            },
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("conflict".into()),
            )),
        );
        assert_eq!(effects.len(), 1);
        assert!(matches!(effects[0], Effect::LoadVirtualBranches { .. }));
        let conflict = state.repos[0].virtual_branches.conflict.as_ref().unwrap();
        assert_eq!(conflict.branch, "api");
        assert!(!state.repos[0].virtual_branches.busy.any());
    }

    #[test]
    fn a_successful_edit_clears_the_conflict_and_reloads_status() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.conflict = None;
        let effects = virtual_branches_edit_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::SetApplied {
                name: "api".into(),
                applied: BranchApplyState::Applied,
            },
            Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        );
        assert!(effects.iter().any(|effect| matches!(
            effect,
            Effect::Status(StatusEffect::LoadWorktreeStatus { .. })
        )));
        assert!(
            effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadVirtualBranches { .. }))
        );
        assert!(state.repos[0].virtual_branches.conflict.is_none());
        assert!(!state.repos[0].virtual_branches.busy.any());
    }

    #[test]
    fn dismissing_a_conflict_clears_it() {
        let mut state = repo_with_virtual_branches();
        state.repos[0].virtual_branches.conflict = Some(crate::model::VirtualBranchesConflict {
            branch: "api".into(),
            against: None,
            paths: Arc::new(Vec::new()),
            message: "conflict".into(),
        });
        dismiss_conflict(&mut state, RepoId(1));
        assert!(state.repos[0].virtual_branches.conflict.is_none());
    }

    #[test]
    fn a_failed_push_notifies_without_reloading() {
        let mut state = repo_with_virtual_branches();
        let effects = virtual_branches_push_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            "api".into(),
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("no remote".into()),
            )),
        );
        assert!(effects.is_empty());
        assert!(!state.repos[0].virtual_branches.busy.mutating);
    }

    /// The conflict message the backend builds, copied from
    /// `gitcomet_gix::repo::virtual branches::conflict_message` so these tests exercise
    /// the shape that really arrives. Nothing enforces that the two stay in
    /// step — if one changes, this is the place that has to follow.
    fn conflict_message(name: &str, against: Option<&str>, listed: &str) -> String {
        let head = match against {
            Some(against) => format!("'{name}' conflicts with '{against}' in {listed}"),
            None => format!(
                "'{name}' could not be applied: it conflicts with the branches \
                 already in the virtual branches, in {listed}"
            ),
        };
        format!(
            "{head}\n\nBoth are applied, and both changed the same lines. \
             Unapply one of them, or resolve the conflict before applying \
             {name} again."
        )
    }

    /// Run a failing edit carrying `message` and return the conflict recorded.
    fn conflict_from(message: &str) -> crate::model::VirtualBranchesConflict {
        let mut state = repo_with_virtual_branches();
        virtual_branches_edit_finished(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            VirtualBranchesEdit::SetApplied {
                name: "ui".into(),
                applied: BranchApplyState::Applied,
            },
            Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend(message.into()),
            )),
        );
        state.repos[0]
            .virtual_branches
            .conflict
            .clone()
            .expect("a failed edit has to record the conflict")
    }

    #[test]
    fn a_conflict_names_the_file_the_two_branches_clashed_on() {
        // The single-file case, which is the one that matters most and the one
        // that used to come back empty: with one path there is no " (" for the
        // parser to cut on, so it took the rest of the sentence as the path and
        // rejected it as prose. The backend names the file and the user never
        // saw it.
        let conflict = conflict_from(&conflict_message(
            "feature/ui",
            Some("feature/api"),
            "src/lib.rs",
        ));
        assert_eq!(conflict.paths.as_ref(), &[PathBuf::from("src/lib.rs")]);
        assert_eq!(conflict.against.as_deref(), Some("feature/api"));
    }

    #[test]
    fn a_conflict_over_several_files_names_the_first_and_leaves_the_rest_to_the_text() {
        let conflict = conflict_from(&conflict_message(
            "feature/ui",
            Some("feature/api"),
            "src/a.rs (and 2 more)",
        ));
        assert_eq!(conflict.paths.as_ref(), &[PathBuf::from("src/a.rs")]);
    }

    #[test]
    fn a_conflict_git_named_no_file_for_reports_none() {
        // "files git did not name" is prose the backend substituted, not a path.
        // Reporting it as one would put a fake entry in the list the user is
        // asked to open.
        let conflict = conflict_from(&conflict_message(
            "feature/ui",
            Some("feature/api"),
            "files git did not name",
        ));
        assert!(
            conflict.paths.is_empty(),
            "prose must not be reported as a file: {:?}",
            conflict.paths
        );
    }

    #[test]
    fn a_conflict_the_backend_could_not_blame_on_a_branch_still_names_the_file() {
        // Not knowing *which* branch is in the way does not mean not knowing
        // *where*. Losing both together left the user with nothing to act on.
        let conflict = conflict_from(&conflict_message("feature/ui", None, "src/lib.rs"));
        assert_eq!(conflict.paths.as_ref(), &[PathBuf::from("src/lib.rs")]);
        assert_eq!(conflict.against, None);
    }

    #[test]
    fn committing_one_file_commits_only_that_file() {
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        index.set(PathBuf::from("b.rs"), Some("api".into()));
        state.repos[0].virtual_branches.assignments = Arc::new(index);

        let effects = commit_single_file(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
        );
        assert_eq!(effects.len(), 1);
        match &effects[0] {
            Effect::ApplyVirtualBranchesEdit {
                repo_id,
                edit:
                    VirtualBranchesEdit::CommitPaths {
                        name,
                        message,
                        paths,
                    },
            } => {
                assert_eq!(*repo_id, RepoId(1));
                assert_eq!(name, "api");
                assert_eq!(
                    paths,
                    &[PathBuf::from("a.rs")],
                    "the row the user clicked is the file they meant to commit"
                );
                assert!(message.contains("a.rs"), "{message}");
                assert!(
                    !message.contains("b.rs"),
                    "a per-file commit must not borrow another file's name: {message}"
                );
            }
            other => panic!("unexpected effect {other:?}"),
        }
        assert!(state.repos[0].virtual_branches.busy.committing);
    }

    #[test]
    fn committing_a_file_that_is_on_no_branch_says_so_instead_of_committing_it_anywhere() {
        let mut state = repo_with_virtual_branches();
        let effects = commit_single_file(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
        );

        assert!(effects.is_empty(), "there is nowhere for it to go");
        assert_eq!(state.notifications.len(), 1);
        assert!(
            state.notifications[0].message.contains("a.rs"),
            "the message has to name the file the user clicked: {}",
            state.notifications[0].message
        );
        assert!(!state.repos[0].virtual_branches.busy.any());
    }

    #[test]
    fn committing_one_hunk_of_a_split_file_is_refused() {
        // A split file is on no single branch, so there is no branch to commit
        // it to; committing the whole file to either one would take hunks
        // belonging to the other.
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set_hunk(
            PathBuf::from("a.rs"),
            gitcomet_core::virtual_branches::HunkFingerprint::of(&["API\n"], 1),
            Some("api".into()),
        );
        index.set_hunk(
            PathBuf::from("a.rs"),
            gitcomet_core::virtual_branches::HunkFingerprint::of(&["UI\n"], 1),
            Some("ui".into()),
        );
        state.repos[0].virtual_branches.assignments = Arc::new(index);

        let effects = commit_single_file(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
        );

        assert!(
            effects.is_empty(),
            "a split file has no single branch to commit to"
        );
        assert_eq!(state.notifications.len(), 1);
    }

    #[test]
    fn committing_a_file_whose_branch_is_gone_is_refused() {
        // The branch was removed elsewhere. Falling back to the raw assignment
        // would commit to a ref the virtual branches no longer has.
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("vanished".into()));
        state.repos[0].virtual_branches.assignments = Arc::new(index);

        let effects = commit_single_file(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
        );

        assert!(effects.is_empty());
        assert_eq!(state.notifications.len(), 1);
    }

    #[test]
    fn committing_one_file_while_something_else_runs_does_nothing() {
        // Same gate as a whole-branch commit: two commits racing would leave a
        // ref moved by whichever lost.
        let mut state = repo_with_virtual_branches();
        let mut index = AssignmentIndex::new();
        index.set(PathBuf::from("a.rs"), Some("api".into()));
        state.repos[0].virtual_branches.assignments = Arc::new(index);
        state.repos[0].virtual_branches.busy.mutating = true;

        let effects = commit_single_file(
            &crate::store::reducer::test_inputs::clock(),
            &mut state,
            RepoId(1),
            PathBuf::from("a.rs"),
        );

        assert!(effects.is_empty());
        assert!(
            !state.repos[0].virtual_branches.busy.committing,
            "and it must not claim it is committing either"
        );
    }

    #[test]
    fn messages_for_an_unknown_repository_are_ignored() {
        let mut state = repo_with_virtual_branches();
        let effects = load_virtual_branches(&mut state, RepoId(99));
        assert!(effects.is_empty());
    }
}
