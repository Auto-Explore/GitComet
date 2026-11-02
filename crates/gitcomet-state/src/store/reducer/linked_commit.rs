//! A commit made in a first-level submodule tab that its parent records too:
//! commit the submodule, record the new commit in the parent alone, then
//! push the submodule and only after it the parent.
use super::util::push_notification;
use crate::model::{
    AppNotificationKind, AppState, LinkedCommit, LinkedCommitStage, LinkedPushChoice,
    LinkedPushPrompt, RepoId, UnpushedPointerPrompt,
};
use crate::msg::{Effect, RepoCommandKind};
use crate::store::reducer::prepared::ReducerInputs;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{CommitId, SubmoduleCheckoutUpdate};
use gitcomet_core::error::Error;
use gitcomet_core::services::{CommitOperationOutcome, PullMode};

fn repo_name(state: &AppState, repo_id: RepoId) -> String {
    state
        .repos
        .iter()
        .find(|repo| repo.id == repo_id)
        .and_then(|repo| repo.spec.workdir.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the repository".to_string())
}

fn short(commit: &CommitId) -> &str {
    let id = commit.as_ref();
    id.get(..7).unwrap_or(id)
}

use crate::model::default_parent_message;

/// Start a linked commit. Returns `None` when the tab has no open parent or
/// another linked commit is running; the caller then commits normally.
pub(super) fn start(
    state: &mut AppState,
    child: RepoId,
    child_message: &str,
    parent_message: String,
    push: bool,
) -> Option<()> {
    if state.linked_commit.is_some() {
        return None;
    }
    let link = state
        .repos
        .iter()
        .find(|repo| repo.id == child)?
        .submodule_parent
        .clone()?;
    let parent = link.repo_id?;
    let parent_message = match parent_message.trim() {
        "" => default_parent_message(&link.path, child_message),
        message => message.to_string(),
    };
    state.linked_commit = Some(LinkedCommit {
        child,
        parent,
        path: link.path,
        parent_message,
        push,
        stage: LinkedCommitStage::CommitChild,
        parent_commit: None,
    });
    Some(())
}

fn begin_parent_commit(state: &mut AppState, parent: RepoId) {
    if let Some(repo) = state.repos.iter_mut().find(|repo| repo.id == parent) {
        repo.local_actions_in_flight = repo.local_actions_in_flight.saturating_add(1);
        repo.commit_in_flight = repo.commit_in_flight.saturating_add(1);
        repo.bump_ops_rev();
    }
}

fn record_effect(state: &mut AppState, plan: &LinkedCommit, amend: Option<CommitId>) -> Effect {
    begin_parent_commit(state, plan.parent);
    Effect::RecordSubmoduleCommit {
        repo_id: plan.parent,
        path: plan.path.clone(),
        message: plan.parent_message.clone(),
        amend_expected: amend,
    }
}

/// The submodule's own commit finished.
pub(super) fn child_commit_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    succeeded: bool,
) -> Vec<Effect> {
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.child != repo_id || plan.stage != LinkedCommitStage::CommitChild {
        return Vec::new();
    }
    if !succeeded {
        state.linked_commit = None;
        let parent = repo_name(state, plan.parent);
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            format!("Nothing was recorded in {parent} because the submodule commit failed."),
        );
        return Vec::new();
    }
    let effect = record_effect(state, &plan, None);
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.stage = LinkedCommitStage::RecordInParent;
    }
    vec![effect]
}

/// The parent recorded (or re-recorded) the submodule.
pub(super) fn recorded(
    inputs: &ReducerInputs,
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    amend: bool,
    result: Result<CommitOperationOutcome, Error>,
) -> Vec<Effect> {
    let outcome = result.as_ref().ok().cloned();
    let mut effects = super::actions_emit_effects::commit_finished(
        inputs,
        state,
        repo_id,
        result.map(|_| ()),
        amend,
        false,
    );
    let Some(plan) = state.linked_commit.clone() else {
        return effects;
    };
    if plan.parent != repo_id || plan.stage != LinkedCommitStage::RecordInParent {
        return effects;
    }
    let child = repo_name(state, plan.child);
    let parent = repo_name(state, plan.parent);
    let Some(outcome) = outcome else {
        state.linked_commit = None;
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            format!(
                "{child} is committed, but recording it in {parent} failed. Stage {} in {parent} and commit to record it.",
                plan.path.display()
            ),
        );
        return effects;
    };
    if !plan.push {
        state.linked_commit = None;
        push_notification(
            inputs,
            state,
            AppNotificationKind::Info,
            match outcome.post_head.as_ref() {
                Some(commit) => format!("Recorded {child} in {parent} as {}", short(commit)),
                None => format!("Recorded {child} in {parent}"),
            },
        );
        return effects;
    }
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.parent_commit = outcome.post_head;
        plan.stage = LinkedCommitStage::PushChild;
    }
    effects.extend(super::actions_emit_effects::push(repos, state, plan.child));
    effects
}

/// A pull or push of a repository in the linked commit finished.
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
    let child = repo_name(state, plan.child);
    let parent = repo_name(state, plan.parent);
    match (&plan.stage, command) {
        (LinkedCommitStage::PushChild, RepoCommandKind::Push) if repo_id == plan.child => {
            match error {
                None => {
                    if let Some(plan) = state.linked_commit.as_mut() {
                        plan.stage = LinkedCommitStage::PushParent;
                    }
                    // The submodule commit is on its remote now, so the
                    // parent's own check lets this push through.
                    super::actions_emit_effects::push(repos, state, plan.parent)
                }
                Some(error) => {
                    if let Some(plan) = state.linked_commit.as_mut() {
                        plan.stage = LinkedCommitStage::PushStopped;
                    }
                    let generation = state.linked_push_prompt_generation.wrapping_add(1);
                    state.linked_push_prompt_generation = generation;
                    state.linked_push_prompt = Some(LinkedPushPrompt {
                        generation,
                        child: plan.child,
                        parent: plan.parent,
                        error,
                    });
                    Vec::new()
                }
            }
        }
        (LinkedCommitStage::PushParent, RepoCommandKind::Push) if repo_id == plan.parent => {
            state.linked_commit = None;
            if error.is_none() {
                push_notification(
                    inputs,
                    state,
                    AppNotificationKind::Info,
                    format!("Pushed {child}, then {parent}"),
                );
            }
            Vec::new()
        }
        (LinkedCommitStage::ReconcileChild { rebase }, RepoCommandKind::Pull { .. })
            if repo_id == plan.child =>
        {
            if let Some(error) = error {
                state.linked_commit = None;
                push_notification(
                    inputs,
                    state,
                    AppNotificationKind::Info,
                    format!("Pulling {child} stopped, so nothing was pushed in {parent}: {error}"),
                );
                return Vec::new();
            }
            if *rebase && let Some(expected) = plan.parent_commit.clone() {
                if let Some(plan) = state.linked_commit.as_mut() {
                    plan.stage = LinkedCommitStage::RerecordInParent;
                }
                return vec![record_effect(state, &plan, Some(expected))];
            }
            // After a merge the recorded commit is part of what gets pushed.
            if let Some(plan) = state.linked_commit.as_mut() {
                plan.stage = LinkedCommitStage::PushChild;
            }
            super::actions_emit_effects::push(repos, state, plan.child)
        }
        _ => Vec::new(),
    }
}

/// The parent re-recorded the rebased submodule commit.
pub(super) fn rerecorded(
    repos: &RepositoryRuntimes,
    state: &mut AppState,
    repo_id: RepoId,
    outcome: Option<&CommitOperationOutcome>,
) -> Vec<Effect> {
    let Some(plan) = state.linked_commit.clone() else {
        return Vec::new();
    };
    if plan.parent != repo_id || plan.stage != LinkedCommitStage::RerecordInParent {
        return Vec::new();
    }
    let Some(outcome) = outcome else {
        state.linked_commit = None;
        return Vec::new();
    };
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.parent_commit = outcome.post_head.clone();
        plan.stage = LinkedCommitStage::PushChild;
    }
    super::actions_emit_effects::push(repos, state, plan.child)
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
    if plan.child != prompt.child || plan.stage != LinkedCommitStage::PushStopped {
        return Vec::new();
    }
    let rebase = match choice {
        LinkedPushChoice::KeepLocal => {
            state.linked_commit = None;
            let child = repo_name(state, plan.child);
            let parent = repo_name(state, plan.parent);
            push_notification(
                inputs,
                state,
                AppNotificationKind::Info,
                format!("{child} and {parent} keep their commits local. Push {child} first."),
            );
            return Vec::new();
        }
        LinkedPushChoice::Rebase => true,
        LinkedPushChoice::Merge => false,
    };
    if let Some(plan) = state.linked_commit.as_mut() {
        plan.stage = LinkedCommitStage::ReconcileChild { rebase };
    }
    super::actions_emit_effects::pull(
        repos,
        state,
        plan.child,
        if rebase {
            PullMode::Rebase
        } else {
            PullMode::Merge
        },
    )
}

pub(super) fn push_blocked(
    state: &mut AppState,
    repo_id: RepoId,
    pointers: Vec<SubmoduleCheckoutUpdate>,
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
            paths: prompt
                .pointers
                .into_iter()
                .map(|pointer| pointer.path)
                .collect(),
        }];
    }
    state.unpushed_pointer_override = Some(prompt.repo_id);
    super::actions_emit_effects::push(repos, state, prompt.repo_id)
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
mod tests {
    use super::*;
    use crate::model::{RepoState, SubmoduleParentLink};
    use gitcomet_core::domain::RepoSpec;
    use std::path::PathBuf;

    fn state() -> AppState {
        let mut state = AppState::test_default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/w/app"),
            },
        ));
        let mut child = RepoState::new_opening(
            RepoId(2),
            RepoSpec {
                workdir: PathBuf::from("/w/app/vendor/lib"),
            },
        );
        child.submodule_parent = Some(SubmoduleParentLink {
            workdir: PathBuf::from("/w/app"),
            path: PathBuf::from("vendor/lib"),
            repo_id: Some(RepoId(1)),
        });
        state.repos.push(child);
        state
    }

    #[test]
    fn the_parent_message_names_the_path_and_the_subject() {
        assert_eq!(
            default_parent_message(
                std::path::Path::new("fixtures/syntax_test"),
                "\n Fix x\n\nbody"
            ),
            "Update fixtures/syntax_test: Fix x"
        );
        assert_eq!(
            default_parent_message(std::path::Path::new("a"), ""),
            "Update a"
        );
    }

    #[test]
    fn a_failed_submodule_commit_records_nothing_in_the_parent() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state();
        start(&mut state, RepoId(2), "x", "Update vendor/lib".into(), true).expect("linked");
        let effects = child_commit_finished(&inputs, &mut state, RepoId(2), false);
        assert!(effects.is_empty());
        assert!(state.linked_commit.is_none());
        assert_eq!(state.repos[0].commit_in_flight, 0);
    }

    #[test]
    fn a_committed_submodule_is_recorded_in_the_parent_alone() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state();
        start(
            &mut state,
            RepoId(2),
            "x",
            "Update vendor/lib: x".into(),
            false,
        )
        .expect("linked");
        let effects = child_commit_finished(&inputs, &mut state, RepoId(2), true);
        assert!(matches!(
            effects.as_slice(),
            [Effect::RecordSubmoduleCommit { repo_id: RepoId(1), path, message, amend_expected: None }]
                if path == &PathBuf::from("vendor/lib") && message == "Update vendor/lib: x"
        ));
        assert_eq!(
            state.repos[0].commit_in_flight, 1,
            "the parent's commit box waits"
        );
        assert_eq!(
            state.linked_commit.as_ref().map(|plan| &plan.stage),
            Some(&LinkedCommitStage::RecordInParent)
        );
    }

    #[test]
    fn a_refused_submodule_push_holds_the_parent_push_back() {
        let inputs = super::super::test_inputs::clock();
        let repos = RepositoryRuntimes::default();
        let mut state = state();
        start(&mut state, RepoId(2), "x", "m".into(), true).expect("linked");
        state.linked_commit.as_mut().unwrap().stage = LinkedCommitStage::PushChild;
        let effects = command_finished(
            &inputs,
            &repos,
            &mut state,
            RepoId(2),
            &RepoCommandKind::Push,
            Some("rejected".into()),
        );
        assert!(effects.is_empty(), "the parent is not pushed");
        let prompt = state
            .linked_push_prompt
            .clone()
            .expect("push stopped prompt");
        assert_eq!((prompt.child, prompt.parent), (RepoId(2), RepoId(1)));

        let effects = resolve_push_prompt(
            &inputs,
            &repos,
            &mut state,
            prompt,
            LinkedPushChoice::KeepLocal,
        );
        assert!(effects.is_empty());
        assert!(state.linked_commit.is_none());
    }

    #[test]
    fn a_rebased_submodule_is_recorded_again_before_pushing() {
        let inputs = super::super::test_inputs::clock();
        let repos = RepositoryRuntimes::default();
        let mut state = state();
        start(&mut state, RepoId(2), "x", "m".into(), true).expect("linked");
        {
            let plan = state.linked_commit.as_mut().unwrap();
            plan.stage = LinkedCommitStage::ReconcileChild { rebase: true };
            plan.parent_commit = Some(CommitId("p1".into()));
        }
        let effects = command_finished(
            &inputs,
            &repos,
            &mut state,
            RepoId(2),
            &RepoCommandKind::Pull {
                mode: PullMode::Rebase,
            },
            None,
        );
        assert!(matches!(
            effects.as_slice(),
            [Effect::RecordSubmoduleCommit { repo_id: RepoId(1), amend_expected: Some(expected), .. }]
                if expected.as_ref() == "p1"
        ));
    }

    #[test]
    fn pushing_anyway_skips_the_check_once() {
        let repos = RepositoryRuntimes::default();
        let mut state = state();
        push_blocked(
            &mut state,
            RepoId(1),
            vec![SubmoduleCheckoutUpdate {
                path: PathBuf::from("vendor/lib"),
                commit: CommitId("c".into()),
            }],
        );
        let prompt = state.unpushed_pointer_prompt.clone().expect("prompt");
        resolve_unpushed_pointer_prompt(&repos, &mut state, prompt, false);
        assert_eq!(state.unpushed_pointer_override, Some(RepoId(1)));
        push_finished(&mut state, RepoId(1), &RepoCommandKind::Push);
        assert_eq!(state.unpushed_pointer_override, None);
    }
}
