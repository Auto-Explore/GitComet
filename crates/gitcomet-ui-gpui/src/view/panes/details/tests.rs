use super::*;
use gitcomet_state::model::{AuthPromptState, PendingCommitRetry};
use std::path::PathBuf;
use std::time::{Duration, UNIX_EPOCH};

fn repo_state(id: RepoId, path: &str) -> RepoState {
    RepoState::new_opening(
        id,
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from(path),
        },
    )
}

fn command_log_entry(command: &str, ok: bool, seconds: u64) -> CommandLogEntry {
    CommandLogEntry {
        time: UNIX_EPOCH + Duration::from_secs(seconds),
        ok,
        command: command.to_string(),
        summary: format!("{command}: test"),
        stdout: "".into(),
        stderr: "".into(),
        announce_success: true,
        handled_by_auth_prompt: false,
        failure_id: None,
        hook_operation_id: None,
    }
}

fn pending_amend_after(repo_id: RepoId, repo: &RepoState) -> PendingCommitAmend {
    PendingCommitAmend {
        repo_id,
        command_log_seq: repo.feedback.command_log_seq,
        failed_awaiting_retry: false,
    }
}

#[test]
fn pending_amend_ignores_previous_amend_log_entry() {
    let repo_id = RepoId(1);
    let mut repo = repo_state(repo_id, "/tmp/repo");
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 1));
    let pending = pending_amend_after(repo_id, &repo);

    assert!(DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo).is_none());
}

#[test]
fn pending_amend_observes_later_amend_log_entry() {
    let repo_id = RepoId(1);
    let mut repo = repo_state(repo_id, "/tmp/repo");
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 1));
    let pending = pending_amend_after(repo_id, &repo);
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 2));

    assert_eq!(
        DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo)
            .map(|entry| entry.time),
        Some(UNIX_EPOCH + Duration::from_secs(2))
    );
}

#[test]
fn pending_amend_observes_amend_before_later_push_log_entry() {
    let repo_id = RepoId(1);
    let mut repo = repo_state(repo_id, "/tmp/repo");
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 1));
    let pending = pending_amend_after(repo_id, &repo);
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 2));
    repo.feedback_mut()
        .push_command_result(command_log_entry("Push after commit", true, 3));

    assert_eq!(
        DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo)
            .map(|entry| entry.time),
        Some(UNIX_EPOCH + Duration::from_secs(2))
    );
}

#[test]
fn pending_amend_observes_successful_retry_after_failed_amend_log_entry() {
    let repo_id = RepoId(1);
    let mut repo = repo_state(repo_id, "/tmp/repo");
    repo.feedback_mut()
        .push_command_result(command_log_entry("Commit", true, 1));
    let pending = pending_amend_after(repo_id, &repo);
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", false, 2));
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 3));

    let completed_entry = DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo);

    assert_eq!(
        completed_entry.map(|entry| (entry.ok, entry.time)),
        Some((true, UNIX_EPOCH + Duration::from_secs(3)))
    );
}

#[test]
fn pending_amend_observes_retry_after_shown_entries_were_dropped() {
    let repo_id = RepoId(1);
    let mut repo = repo_state(repo_id, "/tmp/repo");
    repo.feedback_mut()
        .push_command_result(command_log_entry("Commit", true, 1));
    let pending = pending_amend_after(repo_id, &repo);
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", false, 2));
    // The window acknowledged everything up to the failed attempt.
    repo.feedback_mut().command_log.clear();

    assert!(DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo).is_none());

    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 3));
    assert_eq!(
        DetailsPaneView::pending_commit_amend_completed_entry(&pending, &repo)
            .map(|entry| (entry.ok, entry.time)),
        Some((true, UNIX_EPOCH + Duration::from_secs(3)))
    );
}

fn amend_retry_prompt(repo_id: RepoId) -> AuthPromptState {
    AuthPromptState {
        generation: 1,
        challenge: None,
        status: AuthPromptStatus::AwaitingInput,
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".into(),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: "message".into(),
            amend: true,
            push_after_commit: false,
        },
    }
}

/// An amend that failed into a passphrase prompt, as the window saw it.
fn failed_amend_awaiting_retry(repo_id: RepoId) -> (Option<PendingCommitAmend>, AppState) {
    let mut repo = repo_state(repo_id, "/tmp/repo");
    let mut pending = Some(pending_amend_after(repo_id, &repo));
    repo.feedback_mut()
        .push_command_result(command_log_entry("Amend", false, 1));
    let state = AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        auth_prompt: Some(amend_retry_prompt(repo_id)),
        ..AppState::test_default()
    };
    assert_eq!(
        DetailsPaneView::advance_pending_commit_amend(&mut pending, &state, Some(repo_id)),
        Some(false)
    );
    assert!(
        pending.is_some_and(|pending| pending.failed_awaiting_retry),
        "the prompt will retry the amend"
    );
    (pending, state)
}

#[test]
fn pending_amend_clears_when_its_retry_is_abandoned_after_the_failure_was_dropped() {
    let repo_id = RepoId(1);
    let (mut pending, mut state) = failed_amend_awaiting_retry(repo_id);
    // The window acknowledged the failure, then the prompt was cancelled.
    state.repos[0].feedback_mut().acknowledge(0, 1);
    state.auth_prompt = None;

    assert_eq!(
        DetailsPaneView::advance_pending_commit_amend(&mut pending, &state, Some(repo_id)),
        Some(false)
    );
    assert_eq!(pending, None);
}

#[test]
fn pending_amend_sees_a_successful_retry_after_the_failure_was_dropped() {
    let repo_id = RepoId(1);
    let (mut pending, mut state) = failed_amend_awaiting_retry(repo_id);
    state.repos[0].feedback_mut().acknowledge(0, 1);
    assert_eq!(
        DetailsPaneView::advance_pending_commit_amend(&mut pending, &state, Some(repo_id)),
        Some(false)
    );
    assert!(pending.is_some(), "the prompt is still open");

    state.auth_prompt = None;
    state.repos[0]
        .feedback_mut()
        .push_command_result(command_log_entry("Amend", true, 2));
    assert_eq!(
        DetailsPaneView::advance_pending_commit_amend(&mut pending, &state, Some(repo_id)),
        Some(true)
    );
    assert_eq!(pending, None);
}

#[test]
fn pending_amend_marker_survives_auth_prompt_retry() {
    let repo_id = RepoId(1);
    let state = AppState {
        repos: vec![repo_state(repo_id, "/tmp/repo")],
        active_repo: Some(repo_id),
        auth_prompt: Some(AuthPromptState {
            generation: 1,
            challenge: None,
            status: AuthPromptStatus::AwaitingInput,
            kind: AuthPromptKind::UsernamePassword,
            reason: "auth required".into(),
            operation: AuthRetryOperation::Commit {
                repo_id,
                message: "message".into(),
                amend: true,
                push_after_commit: false,
            },
        }),
        ..AppState::test_default()
    };

    assert!(
        DetailsPaneView::should_preserve_pending_commit_amend_after_failed_log_entry(
            &state, repo_id
        )
    );
    assert!(
        !DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(&state, repo_id, false)
    );
}

#[test]
fn pending_amend_marker_survives_in_flight_auth_retry() {
    let repo_id = RepoId(1);
    let mut repo = repo_state(repo_id, "/tmp/repo");
    repo.pending.commit_retry = Some(PendingCommitRetry {
        message: "message".into(),
        amend: true,
        push_after_commit: false,
    });
    let state = AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };

    assert!(
        DetailsPaneView::should_preserve_pending_commit_amend_after_failed_log_entry(
            &state, repo_id
        )
    );
    assert!(
        !DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(&state, repo_id, false)
    );
    assert!(
        DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(&state, repo_id, true)
    );
}

#[test]
fn pending_amend_marker_is_not_preserved_for_non_retry_failure() {
    let repo_id = RepoId(1);
    let state = AppState {
        repos: vec![repo_state(repo_id, "/tmp/repo")],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };

    assert!(
        !DetailsPaneView::should_preserve_pending_commit_amend_after_failed_log_entry(
            &state, repo_id
        )
    );
    assert!(
        DetailsPaneView::should_clear_pending_commit_amend_after_log_entry(&state, repo_id, false)
    );
}

#[test]
fn notify_fingerprint_ignores_inactive_repo_revisions() {
    let active = repo_state(RepoId(1), "/tmp/active");
    let inactive = repo_state(RepoId(2), "/tmp/inactive");
    let mut state = AppState {
        repos: vec![active, inactive],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = DetailsPaneView::notify_fingerprint(&state);

    state.repos[1].worktree_status_rev = 1;
    state.repos[1].staged_status_rev = 1;
    state.repos[1].ops_rev = 1;
    state.repos[1].history_state.selected_commit_rev = 1;
    state.repos[1].history_state.commit_details_rev = 1;
    state.repos[1].merge_message_rev = 1;

    assert_eq!(DetailsPaneView::notify_fingerprint(&state), initial);
}

#[test]
fn indexed_regression_details_fingerprint_tracks_comparison_metadata() {
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        repos: vec![repo_state(RepoId(1), "/tmp/indexed-comparison")],
        ..AppState::test_default()
    };
    let before = DetailsPaneView::notify_fingerprint(&state);
    state.repos[0].history_state.indexed.rev += 1;
    assert_ne!(before, DetailsPaneView::notify_fingerprint(&state));
}

#[test]
fn notify_fingerprint_tracks_active_repo_relevant_revisions() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = DetailsPaneView::notify_fingerprint(&state);

    state.repos[0].worktree_status_rev = 1;
    let after_status = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_status, initial);

    state.repos[0].ops_rev = 1;
    let after_ops = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_ops, after_status);

    state.repos[0].history_state.selected_commit_rev = 1;
    let after_selected = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_selected, after_ops);

    state.repos[0].history_state.commit_details_rev = 1;
    let after_details = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_details, after_selected);

    state.repos[0].history_state.range_files_rev = 1;
    let after_range_files = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_range_files, after_details);

    state.repos[0].merge_message_rev = 1;
    let after_merge = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_merge, after_range_files);

    state.repos[0].suggested_commit_message_rev = 1;
    assert_ne!(DetailsPaneView::notify_fingerprint(&state), after_merge);
}

#[test]
fn notify_fingerprint_tracks_amend_availability_revisions() {
    let mut state = AppState {
        repos: vec![repo_state(RepoId(1), "/tmp/repo")],
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };

    let initial = DetailsPaneView::notify_fingerprint(&state);

    state.repos[0].head_branch_rev = 1;
    let after_head_branch = DetailsPaneView::notify_fingerprint(&state);
    assert_ne!(after_head_branch, initial);

    state.repos[0].branches_rev = 1;
    assert_ne!(
        DetailsPaneView::notify_fingerprint(&state),
        after_head_branch
    );
}
