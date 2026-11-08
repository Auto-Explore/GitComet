use super::*;
use crate::model::{RepoState, SubmoduleParentLink};
use gitcomet_core::domain::{
    CommitId, FileStatus, FileStatusKind, HeadTracking, RepoSpec, Submodule, SubmoduleStatus,
};
use std::path::PathBuf;
use std::sync::Arc;

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

fn child(path: &str, message: &str) -> LinkedChild {
    LinkedChild {
        path: PathBuf::from(path),
        message: message.to_string(),
    }
}

fn start_from_child(state: &mut AppState, parent_message: &str, push: bool) {
    let inputs = super::super::test_inputs::clock();
    start(
        &inputs,
        state,
        RepoId(2),
        vec![child("vendor/lib", "x")],
        parent_message.to_string(),
        push,
        GitHooks::Run,
        None,
    );
    assert!(state.linked_commit.is_some(), "linked");
}

fn start_from_parent(state: &mut AppState, children: Vec<LinkedChild>, push: bool) -> Vec<Effect> {
    let inputs = super::super::test_inputs::clock();
    start(
        &inputs,
        state,
        RepoId(1),
        children,
        "Use the new fixtures".to_string(),
        push,
        GitHooks::Run,
        None,
    )
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
    assert_eq!(
        default_linked_parent_message(&[child("a/b", "one"), child("c", "two")]),
        "Update a/b and c"
    );
}

#[test]
fn a_failed_submodule_commit_records_nothing_in_the_parent() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_child(&mut state, "Update vendor/lib", true);
    let effects = child_commit_finished(&inputs, &mut state, RepoId(2), false);
    assert!(effects.is_empty());
    assert!(state.linked_commit.is_none());
    assert_eq!(state.repos[0].commit_in_flight, 0);
}

#[test]
fn a_committed_submodule_is_recorded_in_the_parent_alone() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_child(&mut state, "Update vendor/lib: x", false);
    let effects = child_commit_finished(&inputs, &mut state, RepoId(2), true);
    assert!(matches!(
        effects.as_slice(),
        [Effect::RecordSubmoduleCommit { repo_id: RepoId(1), path, message, amend_expected: None, .. }]
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
    start_from_child(&mut state, "m", true);
    state.linked_commit.as_mut().unwrap().stage = LinkedCommitStage::PushChildren;
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
    assert_eq!(
        (prompt.child_tab, prompt.parent, prompt.path.clone()),
        (Some(RepoId(2)), RepoId(1), PathBuf::from("vendor/lib"))
    );

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
    start_from_child(&mut state, "m", true);
    {
        let plan = state.linked_commit.as_mut().unwrap();
        plan.stage = LinkedCommitStage::ReconcileChild {
            path: PathBuf::from("vendor/lib"),
            rebase: true,
        };
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

fn pointer() -> SubmoduleCheckoutUpdate {
    SubmoduleCheckoutUpdate {
        path: PathBuf::from("vendor/lib"),
        commit: CommitId("c".into()),
    }
}

fn commit_auth_prompt(repo_id: RepoId) -> crate::model::AuthPromptState {
    crate::model::AuthPromptState {
        generation: 1,
        challenge: None,
        status: crate::model::AuthPromptStatus::AwaitingInput,
        kind: crate::model::AuthPromptKind::Passphrase,
        reason: "signing key".into(),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: "x".into(),
            amend: false,
            push_after_commit: false,
            hooks: GitHooks::Run,
        },
    }
}

#[test]
fn a_submodule_commit_waiting_for_a_passphrase_keeps_the_plan() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_child(&mut state, "m", true);
    state.auth_prompt = Some(commit_auth_prompt(RepoId(2)));
    let effects = child_commit_finished(&inputs, &mut state, RepoId(2), false);
    assert!(effects.is_empty());
    assert_eq!(
        state.linked_commit.as_ref().map(|plan| &plan.stage),
        Some(&LinkedCommitStage::CommitChildren),
        "the retried commit is recorded in the parent"
    );

    cancel_auth(&mut state, &commit_auth_prompt(RepoId(2)).operation);
    assert!(state.linked_commit.is_none(), "a cancelled sign-in ends it");
}

#[test]
fn closing_the_parent_tab_drops_the_stopped_push() {
    let mut state = state();
    start_from_child(&mut state, "m", true);
    state.linked_commit.as_mut().unwrap().stage = LinkedCommitStage::PushStopped {
        path: PathBuf::from("vendor/lib"),
    };
    state.linked_push_prompt = Some(LinkedPushPrompt {
        generation: 1,
        parent: RepoId(1),
        path: PathBuf::from("vendor/lib"),
        child_tab: Some(RepoId(2)),
        error: "rejected".into(),
    });
    state.repos.retain(|repo| repo.id != RepoId(1));
    forget_closed_repos(&mut state);
    assert!(state.linked_commit.is_none());
    assert!(state.linked_push_prompt.is_none());
}

#[test]
fn a_parent_linked_commit_ends_when_the_parent_tab_closes() {
    let mut state = state();
    start_from_parent(&mut state, vec![child("vendor/ui", "b")], true);
    // Closing a submodule tab leaves the parent's own plan running.
    state.repos.retain(|repo| repo.id != RepoId(2));
    forget_closed_repos(&mut state);
    assert!(state.linked_commit.is_some());
    state.repos.clear();
    forget_closed_repos(&mut state);
    assert!(state.linked_commit.is_none());
}

#[test]
fn a_second_linked_commit_while_one_runs_says_it_is_not_recorded() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_parent(&mut state, vec![child("vendor/ui", "b")], true);
    let seq = state.notifications_seq;
    let effects = start(
        &inputs,
        &mut state,
        RepoId(2),
        vec![child("vendor/lib", "y")],
        "m".into(),
        true,
        GitHooks::Run,
        None,
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::Commit {
                repo_id: RepoId(2),
                ..
            }]
        ),
        "the submodule tab commits on its own"
    );
    assert_eq!(
        state.linked_commit.as_ref().map(|plan| plan.child_tab),
        Some(None),
        "the running commit is kept"
    );
    let notices = state.notifications_after(seq);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0]
            .message
            .starts_with("lib is committed without recording it in app"),
        "{}",
        notices[0].message
    );
}

#[test]
fn pushing_anyway_skips_the_check_once() {
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    push_blocked(&mut state, RepoId(1), vec![pointer()], BlockedPush::Push);
    let prompt = state.unpushed_pointer_prompt.clone().expect("prompt");
    resolve_unpushed_pointer_prompt(&repos, &mut state, prompt, false);
    assert_eq!(state.unpushed_pointer_override, Some(RepoId(1)));
    push_finished(&mut state, RepoId(1), &RepoCommandKind::Push);
    assert_eq!(state.unpushed_pointer_override, None);
}

#[test]
fn a_stopped_push_after_commit_runs_again_as_it_was_asked() {
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    let target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".into(),
        branch: "topic".into(),
        local_branch: "topic".into(),
        local_head: CommitId("h".into()),
    };
    let blocked = BlockedPush::AfterCommit {
        target: target.clone(),
        set_upstream: true,
        hooks: GitHooks::Run,
    };
    push_blocked(&mut state, RepoId(1), vec![pointer()], blocked.clone());
    let prompt = state.unpushed_pointer_prompt.clone().expect("prompt");

    let effects = resolve_unpushed_pointer_prompt(&repos, &mut state, prompt.clone(), false);
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(crate::msg::RepoCommandEffect::PushAfterCommit {
            repo_id: RepoId(1),
            target: pushed,
            set_upstream: true,
            ..
        })] if *pushed == target
    ));

    let effects = resolve_unpushed_pointer_prompt(&repos, &mut state, prompt, true);
    assert!(matches!(
        effects.as_slice(),
        [Effect::PushSubmodulesThenRepo { repo_id: RepoId(1), pointers, push }]
            if pointers == &vec![pointer()] && *push == blocked
    ));
}

// ── From the parent's tab ───────────────────────────────────────

#[test]
fn a_linked_commit_from_the_parent_commits_each_submodule_first() {
    let mut state = state();
    let effects = start_from_parent(
        &mut state,
        vec![
            child("vendor/lib", " Add fixtures "),
            child("vendor/ui", "Bump ui"),
            child("vendor/lib", "duplicate"),
            child("vendor/empty", "  "),
        ],
        true,
    );
    let [
        Effect::CommitSubmodules {
            repo_id, commits, ..
        },
    ] = effects.as_slice()
    else {
        panic!("unexpected effects: {effects:?}");
    };
    assert_eq!(*repo_id, RepoId(1));
    assert_eq!(
        commits,
        &vec![
            child("vendor/lib", "Add fixtures"),
            child("vendor/ui", "Bump ui")
        ],
        "submodules without a message, and repeats, are left out"
    );
    assert_eq!(state.repos[0].commit_in_flight, 1, "the parent waits");
    let plan = state.linked_commit.clone().expect("linked");
    assert_eq!(plan.child_tab, None);
    assert_eq!(plan.stage, LinkedCommitStage::CommitChildren);
    assert_eq!(plan.parent_message, "Use the new fixtures");
}

#[test]
fn a_parent_linked_commit_without_submodule_messages_commits_the_parent_alone() {
    let mut state = state();
    let effects = start_from_parent(&mut state, vec![child("vendor/lib", "")], false);
    assert!(matches!(
        effects.as_slice(),
        [Effect::Commit { repo_id: RepoId(1), message, .. }] if message == "Use the new fixtures"
    ));
    assert!(state.linked_commit.is_none());
}

#[test]
fn committed_submodules_are_staged_and_committed_in_the_parent() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_parent(
        &mut state,
        vec![child("vendor/lib", "a"), child("vendor/ui", "b")],
        false,
    );
    let effects = submodules_committed(
        &inputs,
        &mut state,
        RepoId(1),
        vec![PathBuf::from("vendor/lib"), PathBuf::from("vendor/ui")],
        None,
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CommitWithSubmodules { repo_id: RepoId(1), paths, message, .. }]
            if paths == &vec![PathBuf::from("vendor/lib"), PathBuf::from("vendor/ui")]
                && message == "Use the new fixtures"
    ));
    assert_eq!(
        state.linked_commit.as_ref().map(|plan| &plan.stage),
        Some(&LinkedCommitStage::RecordInParent)
    );
}

#[test]
fn a_failed_submodule_commit_from_the_parent_stops_before_the_parent() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_parent(
        &mut state,
        vec![child("vendor/lib", "a"), child("vendor/ui", "b")],
        true,
    );
    let effects = submodules_committed(
        &inputs,
        &mut state,
        RepoId(1),
        vec![PathBuf::from("vendor/lib")],
        Some((
            PathBuf::from("vendor/ui"),
            Error::new(ErrorKind::Backend("hook refused".into())),
        )),
    );
    assert!(state.linked_commit.is_none());
    assert_eq!(
        state.repos[0].commit_in_flight, 0,
        "the parent is free again"
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::CommitWithSubmodules { .. })),
        "nothing is recorded in the parent"
    );
    let entry = state.repos[0]
        .feedback
        .command_log
        .last()
        .expect("the parent's commit failure is logged");
    assert!(!entry.ok);
    assert!(entry.summary.contains("vendor/ui"), "{}", entry.summary);
    assert!(
        entry.summary.contains("lib is committed"),
        "{}",
        entry.summary
    );
}

#[test]
fn recorded_submodules_push_before_the_parent() {
    let inputs = super::super::test_inputs::clock();
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    start_from_parent(
        &mut state,
        vec![child("vendor/lib", "a"), child("vendor/ui", "b")],
        true,
    );
    state.linked_commit.as_mut().unwrap().stage = LinkedCommitStage::RecordInParent;
    let effects = recorded(
        &inputs,
        &repos,
        &mut state,
        RepoId(1),
        Ok(CommitOperationOutcome {
            local_branch: Some("main".into()),
            pre_head: None,
            post_head: Some(CommitId("p1".into())),
        }),
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PushLinkedSubmodules { repo_id: RepoId(1), paths } if paths.len() == 2
    )));
    let plan = state.linked_commit.clone().expect("still running");
    assert_eq!(plan.stage, LinkedCommitStage::PushChildren);
    assert_eq!(plan.parent_commit, Some(CommitId("p1".into())));
    assert_eq!(state.repos[0].push_in_flight, 1);

    // All submodules pushed: the parent goes next.
    let effects = submodules_pushed(&inputs, &repos, &mut state, RepoId(1), None);
    assert_eq!(state.repos[0].push_in_flight, 0);
    assert_eq!(
        state.linked_commit.as_ref().map(|plan| &plan.stage),
        Some(&LinkedCommitStage::PushParent)
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadSubmodules { repo_id: RepoId(1) })),
    );
}

#[test]
fn a_refused_submodule_push_from_the_parent_asks_and_rebases_in_place() {
    let inputs = super::super::test_inputs::clock();
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    start_from_parent(
        &mut state,
        vec![child("vendor/lib", "a"), child("vendor/ui", "b")],
        true,
    );
    {
        let plan = state.linked_commit.as_mut().unwrap();
        plan.stage = LinkedCommitStage::PushChildren;
        plan.parent_commit = Some(CommitId("p1".into()));
    }
    let effects = submodules_pushed(
        &inputs,
        &repos,
        &mut state,
        RepoId(1),
        Some((
            PathBuf::from("vendor/ui"),
            LinkedSubmodulePushFailure::Push(Error::new(ErrorKind::Backend(
                "rejected (fetch first)".into(),
            ))),
        )),
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::RepoCommand(_))),
        "the parent is not pushed"
    );
    let prompt = state.linked_push_prompt.clone().expect("prompt");
    assert_eq!(prompt.path, PathBuf::from("vendor/ui"));
    assert_eq!(prompt.child_tab, None);

    let effects = resolve_push_prompt(
        &inputs,
        &repos,
        &mut state,
        prompt,
        LinkedPushChoice::Rebase,
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::PullLinkedSubmodule { repo_id: RepoId(1), path, mode: PullMode::Rebase }]
            if path == &PathBuf::from("vendor/ui")
    ));

    let effects = submodule_pulled(
        &inputs,
        &repos,
        &mut state,
        RepoId(1),
        PathBuf::from("vendor/ui"),
        None,
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RecordSubmoduleCommit { repo_id: RepoId(1), path, amend_expected: Some(expected), .. }
            if path == &PathBuf::from("vendor/ui") && expected.as_ref() == "p1"
    )));

    let effects = rerecorded(
        &repos,
        &mut state,
        RepoId(1),
        Some(&CommitOperationOutcome {
            local_branch: Some("main".into()),
            pre_head: Some(CommitId("p1".into())),
            post_head: Some(CommitId("p2".into())),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::PushLinkedSubmodules {
            repo_id: RepoId(1),
            ..
        }]
    ));
    assert_eq!(
        state
            .linked_commit
            .as_ref()
            .and_then(|plan| plan.parent_commit.clone()),
        Some(CommitId("p2".into()))
    );
}

#[test]
fn a_merged_submodule_pushes_again_without_rerecording() {
    let inputs = super::super::test_inputs::clock();
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    start_from_parent(&mut state, vec![child("vendor/lib", "a")], true);
    state.linked_commit.as_mut().unwrap().stage = LinkedCommitStage::ReconcileChild {
        path: PathBuf::from("vendor/lib"),
        rebase: false,
    };
    let effects = submodule_pulled(
        &inputs,
        &repos,
        &mut state,
        RepoId(1),
        PathBuf::from("vendor/lib"),
        None,
    );
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PushLinkedSubmodules { .. }))
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::RecordSubmoduleCommit { .. }))
    );
}

#[test]
fn a_second_linked_commit_from_the_parent_waits_for_the_first() {
    let mut state = state();
    start_from_child(&mut state, "m", false);
    let effects = start_from_parent(&mut state, vec![child("vendor/ui", "b")], false);
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::CommitSubmodules { .. }))
    );
    assert_eq!(state.repos[0].commit_in_flight, 0);
    assert!(
        state.repos[0]
            .feedback
            .command_log
            .last()
            .is_some_and(|entry| !entry.ok),
        "the refusal is logged so the commit box settles"
    );
    assert_eq!(
        state.linked_commit.as_ref().map(|plan| plan.child_tab),
        Some(Some(RepoId(2))),
        "the running commit is kept"
    );
}

/// A plan from the parent's tab whose submodules are committed and recorded,
/// now pushing them.
fn pushing_from_parent(state: &mut AppState) {
    start_from_parent(
        state,
        vec![child("vendor/lib", "a"), child("vendor/ui", "b")],
        true,
    );
    let plan = state.linked_commit.as_mut().unwrap();
    plan.stage = LinkedCommitStage::PushChildren;
    plan.parent_commit = Some(CommitId("p1".into()));
}

#[test]
fn a_submodule_push_needing_sign_in_from_the_parent_skips_the_rebase_prompt() {
    let inputs = super::super::test_inputs::clock();
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    pushing_from_parent(&mut state);
    let seq = state.notifications_seq;
    let effects = submodules_pushed(
        &inputs,
        &repos,
        &mut state,
        RepoId(1),
        Some((
            PathBuf::from("vendor/ui"),
            LinkedSubmodulePushFailure::Push(Error::new(ErrorKind::Backend(
                "git push failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled"
                    .into(),
            ))),
        )),
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::RepoCommand(_))),
        "the parent is not pushed"
    );
    assert!(
        state.linked_push_prompt.is_none(),
        "a rebase would fail the same way"
    );
    assert!(state.linked_commit.is_none(), "the plan ends");
    let notices = state.notifications_after(seq);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].message.starts_with("Pushing ui needs a sign-in")
            && notices[0].message.contains("from its own tab"),
        "{}",
        notices[0].message
    );
}

#[test]
fn unpushed_nested_submodule_commits_stop_the_push_from_the_parent() {
    let inputs = super::super::test_inputs::clock();
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    pushing_from_parent(&mut state);
    let seq = state.notifications_seq;
    let effects = submodules_pushed(
        &inputs,
        &repos,
        &mut state,
        RepoId(1),
        Some((
            PathBuf::from("vendor/lib"),
            LinkedSubmodulePushFailure::UnpushedPointers(vec![SubmoduleCheckoutUpdate {
                path: PathBuf::from("vendor/deps"),
                commit: CommitId("d".into()),
            }]),
        )),
    );
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::RepoCommand(_))),
        "the parent is not pushed"
    );
    assert!(
        state.linked_push_prompt.is_none(),
        "pulling would not make the nested commit fetchable"
    );
    assert!(state.linked_commit.is_none(), "the plan ends");
    let notices = state.notifications_after(seq);
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].message.contains("vendor/deps")
            && notices[0].message.starts_with("lib was not pushed"),
        "the notice names the nested submodule: {}",
        notices[0].message
    );
}

#[test]
fn a_submodule_commit_needing_a_passphrase_from_the_parent_says_where_to_commit_it() {
    let inputs = super::super::test_inputs::clock();
    let mut state = state();
    start_from_parent(
        &mut state,
        vec![child("vendor/lib", "a"), child("vendor/ui", "b")],
        true,
    );
    submodules_committed(
        &inputs,
        &mut state,
        RepoId(1),
        vec![PathBuf::from("vendor/lib")],
        Some((
            PathBuf::from("vendor/ui"),
            Error::new(ErrorKind::Backend(
                "git commit failed: Enter passphrase for \"/home/dev/.ssh/id_ed25519\":".into(),
            )),
        )),
    );
    assert!(state.linked_commit.is_none());
    assert_eq!(state.repos[0].commit_in_flight, 0);
    let entry = state.repos[0]
        .feedback
        .command_log
        .last()
        .expect("the parent's commit failure is logged");
    assert!(
        entry.summary.contains("needs a passphrase or sign-in")
            && entry.summary.contains("Commit ui once from its own tab")
            && entry.summary.contains("lib is committed"),
        "{}",
        entry.summary
    );
}

#[test]
fn linked_reloads_of_the_submodule_rows_wait_for_the_one_running() {
    let inputs = super::super::test_inputs::clock();
    let repos = RepositoryRuntimes::default();
    let mut state = state();
    pushing_from_parent(&mut state);
    assert!(
        state.repos[0].loads_in_flight.request(LoadKind::Submodules),
        "a listing is running"
    );
    state.repos[0].set_submodules(Loadable::Ready(vec![changed_row("vendor/lib")]));
    let effects = submodules_pushed(&inputs, &repos, &mut state, RepoId(1), None);
    assert!(
        effects
            .iter()
            .all(|effect| !matches!(effect, Effect::LoadSubmodules { .. })),
        "the reload queues behind the running listing: {effects:?}"
    );
    assert!(
        matches!(state.repos[0].submodules, Loadable::Ready(_)),
        "the rows stay shown"
    );
    assert!(
        state.repos[0].loads_in_flight.finish(LoadKind::Submodules),
        "the queued listing runs after it"
    );
}

// ── What changed submodules hold ────────────────────────────────

fn changed_row(path: &str) -> Submodule {
    let mut row = Submodule::new(
        PathBuf::from(path),
        CommitId("r".into()),
        Some(CommitId("r".into())),
        SubmoduleStatus::UpToDate,
    );
    row.has_inner_changes = true;
    row
}

#[test]
fn changed_submodule_rows_read_their_changes() {
    let mut state = state();
    let repo = &mut state.repos[0];
    let mut nested = changed_row("vendor/lib/deep");
    nested.nested = true;
    repo.set_submodules(Loadable::Ready(vec![
        changed_row("vendor/lib"),
        Submodule::new(
            PathBuf::from("vendor/clean"),
            CommitId("r".into()),
            Some(CommitId("r".into())),
            SubmoduleStatus::UpToDate,
        ),
        nested,
    ]));
    let effect = refresh_submodule_changes(repo);
    assert!(matches!(
        effect,
        Some(Effect::LoadSubmoduleChanges { repo_id: RepoId(1), paths })
            if paths == vec![PathBuf::from("vendor/lib")]
    ));
    // A second refresh while one runs waits for it.
    assert!(refresh_submodule_changes(&mut state.repos[0]).is_none());

    let changes = SubmoduleChanges {
        path: PathBuf::from("vendor/lib"),
        head: HeadTracking::Detached,
        staged: Arc::new(vec![FileStatus {
            path: PathBuf::from("a.txt"),
            kind: FileStatusKind::Added,
            conflict: None,
        }]),
        unstaged: Arc::new(Vec::new()),
    };
    let effects = submodule_changes_loaded(&mut state, RepoId(1), Ok(vec![changes.clone()]));
    assert!(
        matches!(effects.as_slice(), [Effect::LoadSubmoduleChanges { .. }]),
        "the queued refresh runs"
    );
    assert_eq!(
        state.repos[0].submodule_changes,
        Loadable::Ready(Arc::new(vec![changes]))
    );
    assert!(submodule_changes_loaded(&mut state, RepoId(1), Ok(Vec::new())).is_empty());
}

#[test]
fn clean_submodules_clear_their_changes() {
    let mut state = state();
    let repo = &mut state.repos[0];
    repo.set_submodule_changes(Loadable::Ready(vec![SubmoduleChanges {
        path: PathBuf::from("vendor/lib"),
        head: HeadTracking::Detached,
        staged: Arc::default(),
        unstaged: Arc::default(),
    }]));
    repo.set_submodules(Loadable::Ready(vec![Submodule::new(
        PathBuf::from("vendor/lib"),
        CommitId("r".into()),
        Some(CommitId("r".into())),
        SubmoduleStatus::UpToDate,
    )]));
    assert!(refresh_submodule_changes(repo).is_none());
    assert_eq!(
        repo.submodule_changes,
        Loadable::Ready(Arc::new(Vec::new()))
    );
}

#[test]
fn submodule_tabs_and_bare_repositories_read_no_submodule_changes() {
    let mut state = state();
    let child_tab = &mut state.repos[1];
    child_tab.set_submodules(Loadable::Ready(vec![changed_row("nested/deps")]));
    assert!(
        refresh_submodule_changes(child_tab).is_none(),
        "a submodule's tab shows no linked commit panel"
    );

    let repo = &mut state.repos[0];
    repo.repository_mode = Some(gitcomet_core::domain::RepositoryMode::Bare);
    repo.set_submodules(Loadable::Ready(vec![changed_row("vendor/lib")]));
    assert!(refresh_submodule_changes(repo).is_none());
}
