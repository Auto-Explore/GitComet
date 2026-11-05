//! Linked commits: commits in first-level submodules that their parent
//! worktree records too, pushed submodules first and the parent last.
//!
//! A linked commit starts in one of two tabs. From a submodule's own tab it
//! commits that submodule through the tab, records the new commit in the
//! parent alone (`git commit --only`), and pushes the submodule tab before
//! the parent. From the parent's tab it commits inside each changed
//! submodule through a handle the parent opens, then commits the parent with
//! the new pointers and whatever else is staged there, and pushes every
//! submodule before the parent.
use super::util::push_notification;
use crate::model::{
    AppNotificationKind, AppState, AuthRetryOperation, BlockedPush, LinkedChild, LinkedCommit,
    LinkedCommitStage, LinkedPushChoice, LinkedPushPrompt, LoadKind, Loadable, RepoId, RepoState,
    SubmoduleShelve, UnpushedPointerPrompt, default_parent_message, submodule_display_name,
};
use crate::msg::{Effect, LinkedSubmodulePushFailure, RepoCommandKind};
use crate::store::reducer::prepared::ReducerInputs;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{SubmoduleChanges, SubmoduleCheckoutUpdate, short_commit_id};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CommitOperationOutcome, PullMode};
use std::path::PathBuf;

/// "a", "a and b", "a, b and c".
fn name_list(names: impl IntoIterator<Item = String>) -> String {
    let names = names.into_iter().collect::<Vec<_>>();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn children_names(plan: &LinkedCommit) -> String {
    name_list(
        plan.children
            .iter()
            .map(|child| submodule_display_name(&child.path)),
    )
}

/// The message a parent commit recording several submodules starts with.
pub(crate) fn default_linked_parent_message(children: &[LinkedChild]) -> String {
    match children {
        [child] => default_parent_message(&child.path, &child.message),
        _ => format!(
            "Update {}",
            name_list(
                children
                    .iter()
                    .map(|child| child.path.display().to_string())
            )
        ),
    }
}

/// Start a linked commit from `origin`'s tab.
///
/// `shelve` stashes submodule changes before a commit of one tab: the
/// submodule tab's own commit, or the parent's when no submodule commits
/// with it. The parent's linked commit panel lists every changed submodule,
/// so its linked commit has nothing left behind to shelve.
pub(super) fn start(
    inputs: &ReducerInputs,
    state: &mut AppState,
    origin: RepoId,
    children: Vec<LinkedChild>,
    parent_message: String,
    push: bool,
    shelve: Option<SubmoduleShelve>,
) -> Vec<Effect> {
    let Some(repo) = state.repos.iter().find(|repo| repo.id == origin) else {
        return Vec::new();
    };
    if let Some(link) = repo.submodule_parent.clone() {
        // A commit in a submodule tab whose parent is not open here commits
        // normally.
        let message = children
            .into_iter()
            .next()
            .map(|child| child.message)
            .unwrap_or_default();
        let Some(parent) = link.repo_id else {
            return super::auth::commit(state, origin, message, push, shelve);
        };
        if let Some(running) = &state.linked_commit {
            let running = children_names(running);
            let child = state.repo_name(origin);
            let parent = state.repo_name(parent);
            push_notification(
                inputs,
                state,
                AppNotificationKind::Info,
                format!(
                    "{child} is committed without recording it in {parent}: the commit in {running} is still being recorded and pushed."
                ),
            );
            return super::auth::commit(state, origin, message, push, shelve);
        }
        let child = LinkedChild {
            path: link.path,
            message: message.clone(),
        };
        let parent_message = match parent_message.trim() {
            "" => default_linked_parent_message(std::slice::from_ref(&child)),
            message => message.to_string(),
        };
        state.linked_commit = Some(LinkedCommit {
            parent,
            child_tab: Some(origin),
            children: vec![child],
            parent_message,
            push,
            stage: LinkedCommitStage::CommitChildren,
            parent_commit: None,
        });
        // The linked plan pushes the submodule itself, after the parent
        // records it.
        return super::auth::commit(state, origin, message, false, shelve);
    }

    let mut unique: Vec<LinkedChild> = Vec::new();
    for child in children {
        let message = child.message.trim();
        if message.is_empty()
            || child.path.as_os_str().is_empty()
            || unique.iter().any(|seen| seen.path == child.path)
        {
            continue;
        }
        unique.push(LinkedChild {
            path: child.path,
            message: message.to_string(),
        });
    }
    if unique.is_empty() {
        return super::auth::commit(state, origin, parent_message, push, shelve);
    }
    super::begin_commit_action(state, origin);
    if state.linked_commit.is_some() {
        let parent = state.repo_name(origin);
        return super::actions_emit_effects::commit_finished(
            inputs,
            state,
            origin,
            Err(Error::new(ErrorKind::Backend(format!(
                "Another linked commit is still running. Commit {parent} once it finishes."
            )))),
            false,
            false,
        );
    }
    let parent_message = match parent_message.trim() {
        "" => default_linked_parent_message(&unique),
        message => message.to_string(),
    };
    state.linked_commit = Some(LinkedCommit {
        parent: origin,
        child_tab: None,
        children: unique.clone(),
        parent_message,
        push,
        stage: LinkedCommitStage::CommitChildren,
        parent_commit: None,
    });
    vec![Effect::CommitSubmodules {
        repo_id: origin,
        commits: unique,
        identities: Vec::new(),
    }]
}

/// Drop the plan and its prompt when the parent's tab, or the submodule tab
/// the commit started in, closed: nothing is left to record or push in
/// order.
pub(super) fn forget_closed_repos(state: &mut AppState) {
    let open =
        |state: &AppState, repo_id: RepoId| state.repos.iter().any(|repo| repo.id == repo_id);
    if let Some(plan) = &state.linked_commit
        && !(open(state, plan.parent) && plan.child_tab.is_none_or(|child| open(state, child)))
    {
        state.linked_commit = None;
        state.linked_push_prompt = None;
    }
    if let Some(prompt) = &state.unpushed_pointer_prompt
        && !open(state, prompt.repo_id)
    {
        state.unpushed_pointer_prompt = None;
    }
    if let Some(repo_id) = state.unpushed_pointer_override
        && !open(state, repo_id)
    {
        state.unpushed_pointer_override = None;
    }
}

/// A cancelled sign-in ends the step that asked for it, and with it the
/// plan: the rest would run out of order.
pub(super) fn cancel_auth(state: &mut AppState, operation: &AuthRetryOperation) {
    let Some(plan) = &state.linked_commit else {
        return;
    };
    let in_child_tab = |repo_id: &RepoId| plan.child_tab == Some(*repo_id);
    let waiting = match (&plan.stage, operation) {
        (LinkedCommitStage::CommitChildren, AuthRetryOperation::Commit { repo_id, .. }) => {
            in_child_tab(repo_id)
        }
        (
            LinkedCommitStage::PushChildren,
            AuthRetryOperation::RepoCommand {
                repo_id,
                command: RepoCommandKind::Push,
            },
        ) => in_child_tab(repo_id),
        (
            LinkedCommitStage::PushParent,
            AuthRetryOperation::RepoCommand {
                repo_id,
                command: RepoCommandKind::Push,
            },
        ) => *repo_id == plan.parent,
        (
            LinkedCommitStage::ReconcileChild { .. },
            AuthRetryOperation::RepoCommand {
                repo_id,
                command: RepoCommandKind::Pull { .. },
            },
        ) => in_child_tab(repo_id),
        _ => false,
    };
    if waiting {
        state.linked_commit = None;
    }
}

/// Whether the sign-in prompt now open retries the submodule tab's commit.
fn commit_retry_pending(state: &AppState, child: RepoId) -> bool {
    state.auth_prompt.as_ref().is_some_and(|prompt| {
        matches!(
            &prompt.operation,
            AuthRetryOperation::Commit { repo_id, amend: false, .. } if *repo_id == child
        )
    })
}

fn record_effect(state: &mut AppState, plan: &LinkedCommit) -> Effect {
    super::begin_commit_action(state, plan.parent);
    let child = plan
        .children
        .first()
        .map(|child| child.path.clone())
        .unwrap_or_default();
    Effect::RecordSubmoduleCommit {
        repo_id: plan.parent,
        path: child,
        message: plan.parent_message.clone(),
        amend_expected: None,
        // The store fills in the identity configured for the parent.
        identity: None,
    }
}

fn rerecord_effect(state: &mut AppState, plan: &LinkedCommit, path: PathBuf) -> Option<Effect> {
    let expected = plan.parent_commit.clone()?;
    super::begin_commit_action(state, plan.parent);
    Some(Effect::RecordSubmoduleCommit {
        repo_id: plan.parent,
        path,
        message: plan.parent_message.clone(),
        amend_expected: Some(expected),
        identity: None,
    })
}

/// Read the submodule rows of `repo_id` again, coalesced with any listing
/// already running; the rows stay shown meanwhile.
fn reload_submodules(state: &mut AppState, repo_id: RepoId) -> Option<Effect> {
    let repo_state = state.repos.iter_mut().find(|repo| repo.id == repo_id)?;
    super::effects::request_submodules_reload(repo_state)
}

fn set_stage(state: &mut AppState, stage: LinkedCommitStage) {
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.stage = stage;
    }
}

fn adjust_push_in_flight(state: &mut AppState, repo_id: RepoId, start: bool) {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        repo.push_in_flight = if start {
            repo.push_in_flight.saturating_add(1)
        } else {
            repo.push_in_flight.saturating_sub(1)
        };
        repo.bump_ops_rev();
    }
}

fn adjust_pull_in_flight(state: &mut AppState, repo_id: RepoId, start: bool) {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == repo_id) {
        repo.pull_in_flight = if start {
            repo.pull_in_flight.saturating_add(1)
        } else {
            repo.pull_in_flight.saturating_sub(1)
        };
        repo.bump_ops_rev();
    }
}

/// Push the submodules, the parent waiting for them.
fn push_children(
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    plan: &LinkedCommit,
) -> Vec<Effect> {
    set_stage(state, LinkedCommitStage::PushChildren);
    match plan.child_tab {
        Some(child) => super::actions_emit_effects::push(repos, state, child),
        None => {
            adjust_push_in_flight(state, plan.parent, true);
            vec![Effect::PushLinkedSubmodules {
                repo_id: plan.parent,
                paths: plan.paths(),
            }]
        }
    }
}

fn push_parent(
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    plan: &LinkedCommit,
) -> Vec<Effect> {
    set_stage(state, LinkedCommitStage::PushParent);
    // The submodule commits are on their remotes now, so the parent's own
    // check lets this push through.
    super::actions_emit_effects::push(repos, state, plan.parent)
}

/// A submodule push was refused: hold the parent back and ask.
fn stop_push(state: &mut AppState, plan: &LinkedCommit, path: PathBuf, error: String) {
    set_stage(state, LinkedCommitStage::PushStopped { path: path.clone() });
    let generation = state.linked_push_prompt_generation.wrapping_add(1);
    state.linked_push_prompt_generation = generation;
    state.linked_push_prompt = Some(LinkedPushPrompt {
        generation,
        parent: plan.parent,
        path,
        child_tab: plan.child_tab,
        error,
    });
}

/// The submodule tab's own commit finished.
pub(super) fn child_commit_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    succeeded: bool,
) -> Vec<Effect> {
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.child_tab != Some(repo_id) || plan.stage != LinkedCommitStage::CommitChildren {
        return Vec::new();
    }
    // A commit that asked for a signing passphrase or sign-in is retried
    // from that prompt, and its result comes back here.
    if !succeeded && commit_retry_pending(state, repo_id) {
        return Vec::new();
    }
    if !succeeded {
        state.linked_commit = None;
        let parent = state.repo_name(plan.parent);
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            format!("Nothing was recorded in {parent} because the submodule commit failed."),
        );
        return Vec::new();
    }
    let effect = record_effect(state, &plan);
    set_stage(state, LinkedCommitStage::RecordInParent);
    vec![effect]
}

/// The parent's tab committed inside its submodules.
pub(super) fn submodules_committed(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    committed: Vec<PathBuf>,
    failed: Option<(PathBuf, Error)>,
) -> Vec<Effect> {
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.parent != repo_id
        || plan.child_tab.is_some()
        || plan.stage != LinkedCommitStage::CommitChildren
    {
        return Vec::new();
    }
    let Some((path, error)) = failed else {
        set_stage(state, LinkedCommitStage::RecordInParent);
        return vec![Effect::CommitWithSubmodules {
            repo_id,
            paths: plan.paths(),
            message: plan.parent_message,
            // The store fills in the identity configured for the parent.
            identity: None,
        }];
    };
    state.linked_commit = None;
    let parent = state.repo_name(repo_id);
    // The parent's tab runs these commits without the passphrase and sign-in
    // prompt; the submodule's own tab has it.
    let mut summary = if super::auth::detect_auth_prompt_kind(&error).is_some() {
        format!(
            "Committing {} needs a passphrase or sign-in, which a commit from {parent}'s tab cannot ask for. Commit {} once from its own tab, where it can be entered.",
            path.display(),
            submodule_display_name(&path)
        )
    } else {
        format!("Committing {} failed: {error}", path.display())
    };
    if !committed.is_empty() {
        let names = name_list(committed.iter().map(|path| submodule_display_name(path)));
        summary.push_str(&format!(
            " {names} {} committed; nothing was recorded in {parent}.",
            if committed.len() == 1 { "is" } else { "are" }
        ));
    }
    let mut effects = super::actions_emit_effects::commit_finished(
        inputs,
        state,
        repo_id,
        Err(Error::new(ErrorKind::Backend(summary))),
        false,
        false,
    );
    effects.extend(reload_submodules(state, repo_id));
    effects
}

/// The parent recorded the submodules.
pub(super) fn recorded(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<CommitOperationOutcome, Error>,
) -> Vec<Effect> {
    let outcome = result.as_ref().ok().cloned();
    let mut effects = super::actions_emit_effects::commit_finished(
        inputs,
        state,
        repo_id,
        result.map(|_| ()),
        false,
        false,
    );
    let Some(plan) = state.linked_commit.clone() else {
        return effects;
    };
    if plan.parent != repo_id || plan.stage != LinkedCommitStage::RecordInParent {
        return effects;
    }
    let children = children_names(&plan);
    let parent = state.repo_name(plan.parent);
    let Some(outcome) = outcome else {
        state.linked_commit = None;
        let paths = name_list(
            plan.children
                .iter()
                .map(|child| child.path.display().to_string()),
        );
        let message = if plan.child_tab.is_some() {
            format!(
                "{children} is committed, but recording it in {parent} failed. Stage {paths} in {parent} and commit to record it."
            )
        } else {
            format!(
                "{children} {} committed, but the commit in {parent} failed. Stage {paths} in {parent} and commit to record {}.",
                if plan.children.len() == 1 {
                    "is"
                } else {
                    "are"
                },
                if plan.children.len() == 1 {
                    "it"
                } else {
                    "them"
                },
            )
        };
        push_notification(inputs, state, AppNotificationKind::Info, message);
        return effects;
    };
    if !plan.push {
        state.linked_commit = None;
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            match outcome.post_head.as_ref() {
                Some(commit) => format!(
                    "Recorded {children} in {parent} as {}",
                    short_commit_id(commit.as_ref())
                ),
                None => format!("Recorded {children} in {parent}"),
            },
        );
        return effects;
    }
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.parent_commit = outcome.post_head;
    }
    effects.extend(push_children(repos, state, &plan));
    effects
}

/// After pulling a refused submodule: record the rebased commit again, or
/// push again.
fn reconciled(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    plan: &LinkedCommit,
    path: PathBuf,
    rebase: bool,
    error: Option<String>,
) -> Vec<Effect> {
    if let Some(error) = error {
        state.linked_commit = None;
        let child = submodule_display_name(&path);
        let parent = state.repo_name(plan.parent);
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            format!("Pulling {child} stopped, so nothing was pushed in {parent}: {error}"),
        );
        return Vec::new();
    }
    if rebase && let Some(effect) = rerecord_effect(state, plan, path.clone()) {
        set_stage(state, LinkedCommitStage::RerecordInParent { path });
        return vec![effect];
    }
    // After a merge the recorded commit is part of what gets pushed.
    push_children(repos, state, plan)
}

/// A pull or push of a tab in the linked commit finished.
pub(super) fn command_finished(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    command: &RepoCommandKind,
    error: Option<String>,
) -> Vec<Effect> {
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    // A failure that opened the sign-in prompt is retried from there.
    if error.is_some() && state.auth_prompt.is_some() {
        return Vec::new();
    }
    match (&plan.stage, command) {
        (LinkedCommitStage::PushChildren, RepoCommandKind::Push)
            if plan.child_tab == Some(repo_id) =>
        {
            match error {
                None => push_parent(repos, state, &plan),
                Some(error) => {
                    let path = plan
                        .children
                        .first()
                        .map(|child| child.path.clone())
                        .unwrap_or_default();
                    stop_push(state, &plan, path, error);
                    Vec::new()
                }
            }
        }
        (LinkedCommitStage::PushParent, RepoCommandKind::Push) if repo_id == plan.parent => {
            state.linked_commit = None;
            if error.is_none() {
                let children = children_names(&plan);
                let parent = state.repo_name(plan.parent);
                push_notification(
                    inputs,
                    state,
                    AppNotificationKind::Info,
                    format!("Pushed {children}, then {parent}"),
                );
            }
            Vec::new()
        }
        (LinkedCommitStage::ReconcileChild { path, rebase }, RepoCommandKind::Pull { .. })
            if plan.child_tab == Some(repo_id) =>
        {
            reconciled(inputs, repos, state, &plan, path.clone(), *rebase, error)
        }
        _ => Vec::new(),
    }
}

/// The parent's tab pushed its submodules.
pub(super) fn submodules_pushed(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    failed: Option<(PathBuf, LinkedSubmodulePushFailure)>,
) -> Vec<Effect> {
    adjust_push_in_flight(state, repo_id, false);
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.parent != repo_id
        || plan.child_tab.is_some()
        || plan.stage != LinkedCommitStage::PushChildren
    {
        return Vec::new();
    }
    // The submodules' remote-tracking refs moved; their rows follow.
    let mut effects: Vec<Effect> = reload_submodules(state, repo_id).into_iter().collect();
    let Some((path, failure)) = failed else {
        effects.extend(push_parent(repos, state, &plan));
        return effects;
    };
    let child = submodule_display_name(&path);
    let parent = state.repo_name(plan.parent);
    let notice = match failure {
        // Pulling would not help: the submodule's own submodules have to be
        // pushed first, and the submodule's tab offers that.
        LinkedSubmodulePushFailure::UnpushedPointers(pointers) => {
            let nested = name_list(
                pointers
                    .iter()
                    .map(|pointer| pointer.path.display().to_string()),
            );
            let their = if pointers.len() == 1 { "its" } else { "their" };
            format!(
                "{child} was not pushed, so {parent} was not pushed either: {child}'s commits record commits of {nested} that are not on {their} remote. Push {child} from its own tab, which offers to push {nested} first, then push {parent}."
            )
        }
        // The parent's tab pushes without the sign-in prompt, so a rebase
        // offered here would fail the same way.
        LinkedSubmodulePushFailure::Push(error)
            if super::auth::detect_auth_prompt_kind(&error).is_some() =>
        {
            format!(
                "Pushing {child} needs a sign-in, which a push from {parent}'s tab cannot ask for, so {parent} was not pushed. Push {child} from its own tab, then push {parent}."
            )
        }
        LinkedSubmodulePushFailure::Push(error) => {
            stop_push(state, &plan, path, error.to_string());
            return effects;
        }
    };
    state.linked_commit = None;
    push_notification(inputs, state, AppNotificationKind::Info, notice);
    effects
}

/// The parent's tab pulled a refused submodule.
pub(super) fn submodule_pulled(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    error: Option<String>,
) -> Vec<Effect> {
    adjust_pull_in_flight(state, repo_id, false);
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    let LinkedCommitStage::ReconcileChild {
        path: stage_path,
        rebase,
    } = &plan.stage
    else {
        return Vec::new();
    };
    if plan.parent != repo_id || plan.child_tab.is_some() || *stage_path != path {
        return Vec::new();
    }
    let rebase = *rebase;
    let mut effects: Vec<Effect> = reload_submodules(state, repo_id).into_iter().collect();
    effects.extend(reconciled(inputs, repos, state, &plan, path, rebase, error));
    effects
}

/// The parent recorded a rebased submodule commit again.
pub(super) fn rerecorded(
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    outcome: Option<&CommitOperationOutcome>,
) -> Vec<Effect> {
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.parent != repo_id || !matches!(plan.stage, LinkedCommitStage::RerecordInParent { .. }) {
        return Vec::new();
    }
    let Some(outcome) = outcome else {
        state.linked_commit = None;
        return Vec::new();
    };
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.parent_commit = outcome.post_head.clone();
    }
    push_children(repos, state, &plan)
}

pub(super) fn resolve_push_prompt(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    prompt: LinkedPushPrompt,
    choice: LinkedPushChoice,
) -> Vec<Effect> {
    if state.linked_push_prompt.as_ref() == Some(&prompt) {
        state.linked_push_prompt = None;
    }
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.parent != prompt.parent
        || plan.stage
            != (LinkedCommitStage::PushStopped {
                path: prompt.path.clone(),
            })
    {
        return Vec::new();
    }
    let rebase = match choice {
        LinkedPushChoice::KeepLocal => {
            state.linked_commit = None;
            let child = submodule_display_name(&prompt.path);
            let children = children_names(&plan);
            let parent = state.repo_name(plan.parent);
            push_notification(
                inputs,
                state,
                AppNotificationKind::Info,
                format!("{children} and {parent} keep their commits local. Push {child} first."),
            );
            return Vec::new();
        }
        LinkedPushChoice::Rebase => true,
        LinkedPushChoice::Merge => false,
    };
    set_stage(
        state,
        LinkedCommitStage::ReconcileChild {
            path: prompt.path.clone(),
            rebase,
        },
    );
    let mode = if rebase {
        PullMode::Rebase
    } else {
        PullMode::Merge
    };
    match plan.child_tab {
        Some(child) => super::actions_emit_effects::pull(repos, state, child, mode),
        None => {
            adjust_pull_in_flight(state, plan.parent, true);
            vec![Effect::PullLinkedSubmodule {
                repo_id: plan.parent,
                path: prompt.path,
                mode,
            }]
        }
    }
}

// ── What the changed submodules hold ────────────────────────────

/// Read again what the changed first-level submodules hold, after their
/// rows loaded; with none changed the list is simply empty. A submodule's own
/// tab and a bare repository never show the linked commit panel, so they
/// read nothing.
pub(super) fn refresh_submodule_changes(repo_state: &mut RepoState) -> Option<Effect> {
    if repo_state.submodule_parent.is_some() || repo_state.is_bare() {
        return None;
    }
    let paths = repo_state.changed_first_level_submodules();
    if paths.is_empty() {
        repo_state.set_submodule_changes(Loadable::Ready(Vec::new()));
        return None;
    }
    repo_state
        .loads_in_flight
        .request(LoadKind::SubmoduleChanges)
        .then(|| Effect::LoadSubmoduleChanges {
            repo_id: repo_state.id,
            paths,
        })
}

pub(super) fn submodule_changes_loaded(
    state: &mut AppState,
    repo_id: RepoId,
    result: Result<Vec<SubmoduleChanges>, Error>,
) -> Vec<Effect> {
    let Some(repo_state) = state.repos.iter_mut().find(|repo| repo.id == repo_id) else {
        return Vec::new();
    };
    match result {
        Ok(changes) => repo_state.set_submodule_changes(Loadable::Ready(changes)),
        Err(error) if matches!(error.kind(), ErrorKind::Cancelled) => {}
        Err(error) => repo_state.set_submodule_changes(Loadable::Error(error.to_string())),
    }
    if !repo_state
        .loads_in_flight
        .finish(LoadKind::SubmoduleChanges)
    {
        return Vec::new();
    }
    let paths = repo_state.changed_first_level_submodules();
    if paths.is_empty() {
        repo_state
            .loads_in_flight
            .finish(LoadKind::SubmoduleChanges);
        repo_state.set_submodule_changes(Loadable::Ready(Vec::new()));
        return Vec::new();
    }
    vec![Effect::LoadSubmoduleChanges { repo_id, paths }]
}

pub(super) fn stage_in_submodule(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    files: Vec<PathBuf>,
    stage: bool,
) -> Vec<Effect> {
    if files.is_empty() || !state.repos.iter().any(|repo| repo.id == repo_id) {
        return Vec::new();
    }
    super::begin_local_action(state, repo_id);
    vec![Effect::StageInSubmodule {
        repo_id,
        path,
        files,
        stage,
    }]
}

// ── The unpushed-pointer guard ──────────────────────────────────

pub(super) fn push_blocked(
    state: &mut AppState,
    repo_id: RepoId,
    pointers: Vec<SubmoduleCheckoutUpdate>,
    push: BlockedPush,
) -> Vec<Effect> {
    if pointers.is_empty() || !state.repos.iter().any(|repo| repo.id == repo_id) {
        return Vec::new();
    }
    // A linked commit pushes the submodule itself; its parent push only
    // gets here when that push did not make the commit fetchable.
    let generation = state.unpushed_pointer_prompt_generation.wrapping_add(1);
    state.unpushed_pointer_prompt_generation = generation;
    state.unpushed_pointer_prompt = Some(UnpushedPointerPrompt {
        generation,
        repo_id,
        pointers,
        push,
    });
    Vec::new()
}

pub(super) fn resolve_unpushed_pointer_prompt(
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    prompt: UnpushedPointerPrompt,
    push_submodules_first: bool,
) -> Vec<Effect> {
    if state.unpushed_pointer_prompt.as_ref() == Some(&prompt) {
        state.unpushed_pointer_prompt = None;
    }
    if !state.repos.iter().any(|repo| repo.id == prompt.repo_id) {
        return Vec::new();
    }
    if push_submodules_first {
        return vec![Effect::PushSubmodulesThenRepo {
            repo_id: prompt.repo_id,
            pointers: prompt.pointers,
            push: prompt.push,
        }];
    }
    state.unpushed_pointer_override = Some(prompt.repo_id);
    match prompt.push {
        BlockedPush::Push => super::actions_emit_effects::push(repos, state, prompt.repo_id),
        BlockedPush::AfterCommit {
            target,
            set_upstream,
        } => super::actions_emit_effects::push_after_commit(
            repos,
            state,
            prompt.repo_id,
            target,
            set_upstream,
        ),
    }
}

pub(super) fn dismiss_unpushed_pointer_prompt(
    state: &mut AppState,
    prompt: UnpushedPointerPrompt,
) -> Vec<Effect> {
    if state.unpushed_pointer_prompt.as_ref() == Some(&prompt) {
        state.unpushed_pointer_prompt = None;
    }
    Vec::new()
}

/// The override answers one push only.
pub(super) fn push_finished(state: &mut AppState, repo_id: RepoId, command: &RepoCommandKind) {
    if matches!(
        command,
        RepoCommandKind::Push | RepoCommandKind::PushAfterCommit { .. }
    ) && state.unpushed_pointer_override == Some(repo_id)
    {
        state.unpushed_pointer_override = None;
    }
}

#[cfg(test)]
mod tests;
