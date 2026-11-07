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
        // The virtual-branches branch itself is no place to go back to: HEAD can
        // still be there before the view has marked the repository active.
        Loadable::Ready(branch)
            if !branch.is_empty()
                && branch != "HEAD"
                && branch != gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH =>
        {
            Some(branch.clone())
        }
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
    // Leaving moves HEAD off the virtual branches branch, but `head_branch`
    // still names it until the reload below lands.
    repo.virtual_branches.head_unconfirmed = !active;
    repo.virtual_branches.bump_rev();
    // Leaving changes which branch HEAD is on, and entering changes the files,
    // so the ordinary views have to catch up either way.
    vec![
        Effect::Status(StatusEffect::LoadWorktreeStatus { repo_id }),
        Effect::References(ReferenceEffect::LoadBranches { repo_id }),
        Effect::References(ReferenceEffect::LoadHeadBranch { repo_id }),
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
    //
    // This only ever turns `active` on. Entering sets it before `head_branch`
    // follows, so a load landing in that gap sees the old HEAD, and reading
    // that as "not on the branch" would undo the enter. Leaving is reported by
    // the switch itself. A switch in flight owns `active` altogether, and so
    // does a leave whose `head_branch` has not been reloaded yet.
    if !repo.virtual_branches.busy.switching
        && !repo.virtual_branches.head_unconfirmed
        && matches!(&repo.head_branch, Loadable::Ready(branch) if branch == gitcomet_core::virtual_branches::VIRTUAL_BRANCHES_BRANCH)
    {
        repo.virtual_branches.active = true;
    }
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
mod tests;
