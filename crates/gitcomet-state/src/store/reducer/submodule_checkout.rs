//! Keeping a submodule's checkout on the commit its parent records: the
//! prompt a guarded discard or update opens, and the notices that report
//! what moved.
use super::util::push_notification;
use crate::model::{
    AppNotificationKind, AppState, RepoId, SubmoduleCheckoutOperation, SubmoduleCheckoutPrompt,
    SubmoduleSettings,
};
use crate::msg::Effect;
use crate::store::reducer::prepared::ReducerInputs;
use gitcomet_core::domain::{
    CommitId, DiscardOutcome, SubmoduleCheckoutPolicy, SubmoduleCheckoutUpdate,
};
use std::path::{Path, PathBuf};

pub(super) fn set_settings(state: &mut AppState, settings: SubmoduleSettings) -> Vec<Effect> {
    state.submodule_settings = settings;
    Vec::new()
}

/// The name the user knows a submodule by: the last part of its path.
pub(crate) fn submodule_display_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn short(commit: &CommitId) -> &str {
    let id = commit.as_ref();
    id.get(..7).unwrap_or(id)
}

pub(super) fn update_to_recorded(repo_id: RepoId, path: PathBuf) -> Vec<Effect> {
    vec![Effect::SubmoduleCheckout {
        repo_id,
        operation: SubmoduleCheckoutOperation::UpdateToRecorded { path },
        policy: SubmoduleCheckoutPolicy::Guarded,
    }]
}

/// The user picked how to keep the work the guard found. Returns whether an
/// effect was scheduled, so the caller counts it in flight.
pub(super) fn resolve_prompt(
    state: &mut AppState,
    prompt: SubmoduleCheckoutPrompt,
    policy: SubmoduleCheckoutPolicy,
) -> Vec<Effect> {
    if state.submodule_checkout_prompt.as_ref() == Some(&prompt) {
        state.submodule_checkout_prompt = None;
    }
    if !state.repos.iter().any(|repo| repo.id == prompt.repo_id) {
        return Vec::new();
    }
    vec![Effect::SubmoduleCheckout {
        repo_id: prompt.repo_id,
        operation: prompt.operation,
        policy,
    }]
}

pub(super) fn dismiss_prompt(state: &mut AppState, prompt: SubmoduleCheckoutPrompt) -> Vec<Effect> {
    if state.submodule_checkout_prompt.as_ref() == Some(&prompt) {
        state.submodule_checkout_prompt = None;
    }
    Vec::new()
}

/// A blocked checkout opens the prompt; anything else becomes a short notice.
pub(super) fn checkout_finished(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    operation: SubmoduleCheckoutOperation,
    outcome: DiscardOutcome,
) -> Vec<Effect> {
    if !state.repos.iter().any(|repo| repo.id == repo_id) {
        return Vec::new();
    }
    if outcome.is_blocked() {
        let generation = state.submodule_checkout_prompt_generation.wrapping_add(1);
        state.submodule_checkout_prompt_generation = generation;
        state.submodule_checkout_prompt = Some(SubmoduleCheckoutPrompt {
            generation,
            repo_id,
            operation,
            risks: outcome.blocked,
        });
        return Vec::new();
    }
    for message in outcome_notices(&outcome) {
        push_notification(inputs, state, AppNotificationKind::Info, message);
    }
    Vec::new()
}

pub(super) fn pointers_followed(
    inputs: &ReducerInputs,
    state: &mut AppState,
    repo_id: RepoId,
    updates: Vec<SubmoduleCheckoutUpdate>,
) -> Vec<Effect> {
    if updates.is_empty() || !state.repos.iter().any(|repo| repo.id == repo_id) {
        return Vec::new();
    }
    push_notification(
        inputs,
        state,
        AppNotificationKind::Info,
        checked_out_notice(&updates),
    );
    Vec::new()
}

fn checked_out_notice(updates: &[SubmoduleCheckoutUpdate]) -> String {
    match updates {
        [update] => format!(
            "{} checked out at {}",
            submodule_display_name(&update.path),
            short(&update.commit)
        ),
        _ => {
            let names = updates
                .iter()
                .map(|update| {
                    format!(
                        "{} at {}",
                        submodule_display_name(&update.path),
                        short(&update.commit)
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            format!("Submodules checked out: {names}")
        }
    }
}

pub(crate) fn outcome_notices(outcome: &DiscardOutcome) -> Vec<String> {
    let mut notices = Vec::new();
    for path in &outcome.stashed {
        notices.push(format!(
            "Stashed the changes in {} first",
            submodule_display_name(path)
        ));
    }
    for (path, branch) in &outcome.kept_on_branch {
        notices.push(format!(
            "Kept the commit {} was on as branch {branch}",
            submodule_display_name(path)
        ));
    }
    if !outcome.updated.is_empty() {
        notices.push(checked_out_notice(&outcome.updated));
    }
    for path in &outcome.not_loaded {
        notices.push(format!(
            "{} is not loaded. Load it to check out the restored commit.",
            submodule_display_name(path)
        ));
    }
    notices
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoState;
    use gitcomet_core::domain::{RepoSpec, SubmoduleCheckoutRisk};

    fn state_with_repo() -> AppState {
        let mut state = AppState::test_default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/w/app"),
            },
        ));
        state
    }

    fn risk() -> SubmoduleCheckoutRisk {
        SubmoduleCheckoutRisk {
            path: PathBuf::from("vendor/lib"),
            checked_out: Some(CommitId("b".repeat(40).into())),
            target: CommitId("a".repeat(40).into()),
            has_inner_changes: true,
            unreferenced: false,
        }
    }

    fn discard(paths: &[&str]) -> SubmoduleCheckoutOperation {
        SubmoduleCheckoutOperation::Discard {
            paths: paths.iter().map(PathBuf::from).collect(),
        }
    }

    #[test]
    fn a_blocked_discard_opens_the_prompt_and_each_block_is_a_new_one() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state_with_repo();
        let blocked = DiscardOutcome {
            blocked: vec![risk()],
            ..Default::default()
        };
        checkout_finished(
            &inputs,
            &mut state,
            RepoId(1),
            discard(&["vendor/lib"]),
            blocked.clone(),
        );
        let first = state.submodule_checkout_prompt.clone().expect("prompt");
        assert_eq!(first.repo_id, RepoId(1));
        assert_eq!(first.risks, vec![risk()]);
        assert!(
            state.notifications.is_empty(),
            "the prompt replaces a notice"
        );

        checkout_finished(
            &inputs,
            &mut state,
            RepoId(1),
            discard(&["vendor/lib"]),
            blocked,
        );
        let second = state.submodule_checkout_prompt.clone().expect("prompt");
        assert_ne!(first.generation, second.generation);
    }

    #[test]
    fn answering_the_prompt_reruns_the_operation_with_the_policy() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state_with_repo();
        checkout_finished(
            &inputs,
            &mut state,
            RepoId(1),
            discard(&["vendor/lib", "README.md"]),
            DiscardOutcome {
                blocked: vec![risk()],
                ..Default::default()
            },
        );
        let prompt = state.submodule_checkout_prompt.clone().expect("prompt");
        let effects = resolve_prompt(&mut state, prompt, SubmoduleCheckoutPolicy::StashAndUpdate);
        assert!(state.submodule_checkout_prompt.is_none());
        assert!(matches!(
            effects.as_slice(),
            [Effect::SubmoduleCheckout {
                repo_id: RepoId(1),
                operation: SubmoduleCheckoutOperation::Discard { paths },
                policy: SubmoduleCheckoutPolicy::StashAndUpdate,
            }] if paths.len() == 2
        ));
    }

    #[test]
    fn a_dismissed_prompt_runs_nothing() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state_with_repo();
        checkout_finished(
            &inputs,
            &mut state,
            RepoId(1),
            SubmoduleCheckoutOperation::UpdateToRecorded {
                path: PathBuf::from("vendor/lib"),
            },
            DiscardOutcome {
                blocked: vec![risk()],
                ..Default::default()
            },
        );
        let prompt = state.submodule_checkout_prompt.clone().expect("prompt");
        assert!(dismiss_prompt(&mut state, prompt).is_empty());
        assert!(state.submodule_checkout_prompt.is_none());
    }

    #[test]
    fn an_update_reports_where_the_submodule_was_checked_out() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state_with_repo();
        checkout_finished(
            &inputs,
            &mut state,
            RepoId(1),
            discard(&["fixtures/syntax_test"]),
            DiscardOutcome {
                updated: vec![SubmoduleCheckoutUpdate {
                    path: PathBuf::from("fixtures/syntax_test"),
                    commit: CommitId("a1b2c3d4e5".into()),
                }],
                stashed: vec![PathBuf::from("fixtures/syntax_test")],
                ..Default::default()
            },
        );
        let messages = state
            .notifications
            .iter()
            .map(|notification| notification.message.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            messages,
            vec![
                "Stashed the changes in syntax_test first",
                "syntax_test checked out at a1b2c3d",
            ]
        );
        assert!(state.submodule_checkout_prompt.is_none());
    }

    #[test]
    fn followed_pointers_are_reported_together() {
        let inputs = super::super::test_inputs::clock();
        let mut state = state_with_repo();
        pointers_followed(
            &inputs,
            &mut state,
            RepoId(1),
            vec![
                SubmoduleCheckoutUpdate {
                    path: PathBuf::from("a"),
                    commit: CommitId("1111111aaa".into()),
                },
                SubmoduleCheckoutUpdate {
                    path: PathBuf::from("libs/b"),
                    commit: CommitId("2222222bbb".into()),
                },
            ],
        );
        assert_eq!(
            state.notifications.last().map(|n| n.message.as_str()),
            Some("Submodules checked out: a at 1111111, b at 2222222")
        );
    }
}
