use super::*;
use gitcomet_core::domain::RepoSpec;
use gitcomet_core::git_operation::{GitOutputChunk, GitOutputStream, HookExecutionId};
use std::sync::Arc;

fn repo_state() -> RepoState {
    RepoState::new_opening(
        crate::model::RepoId(1),
        RepoSpec {
            workdir: std::env::temp_dir().join("gitcomet-hook-activity-test"),
        },
    )
}

#[test]
fn silent_annex_command_is_reportable_and_cancellable_from_startup() {
    let mut repo = repo_state();
    let id = GitOperationId(1);
    started(
        &mut repo,
        id,
        "git annex sync".into(),
        None,
        SystemTime::UNIX_EPOCH,
        false,
    );
    apply_event(&mut repo, id, GitOperationEvent::CommandStarted);
    assert!(repo.feedback.hook_activity[0].is_reportable());
    assert!(repo.feedback.hook_activity[0].transfer.is_none());
    assert!(request_cancel(&mut repo, id));
    finished(
        &mut repo,
        id,
        GitOperationOuterOutcome::Cancelled,
        Duration::from_millis(1),
    );
    assert_eq!(
        repo.feedback.hook_activity[0].status,
        GitHookOperationStatus::Cancelled
    );
}

#[test]
fn pull_command_metadata_retains_cancellable_activity_without_claiming_hook_feedback() {
    for (outcome, status) in [
        (
            GitOperationOuterOutcome::Succeeded,
            GitHookOperationStatus::Succeeded,
        ),
        (
            GitOperationOuterOutcome::Failed,
            GitHookOperationStatus::Failed,
        ),
        (
            GitOperationOuterOutcome::TimedOut,
            GitHookOperationStatus::TimedOut,
        ),
        (
            GitOperationOuterOutcome::Cancelled,
            GitHookOperationStatus::Cancelled,
        ),
    ] {
        let mut repo = repo_state();
        let id = GitOperationId(1);
        started(
            &mut repo,
            id,
            "Pull".into(),
            Some("origin/main → topic".into()),
            SystemTime::UNIX_EPOCH,
            true,
        );
        apply_event(
            &mut repo,
            id,
            GitOperationEvent::CommandInvocation {
                command: "git pull --rebase --no-prune -- origin refs/heads/main".into(),
            },
        );
        apply_event(
            &mut repo,
            id,
            GitOperationEvent::Output {
                chunks: vec![GitOutputChunk {
                    stream: GitOutputStream::Stderr,
                    text: "Rebasing (1/2)\n".into(),
                }],
            },
        );
        assert!(!repo.feedback.hook_activity[0].is_reportable());
        assert!(request_cancel(&mut repo, id));
        finished(&mut repo, id, outcome, Duration::from_secs(5));
        assert_eq!(repo.feedback.hook_activity.len(), 1);
        let operation = &repo.feedback.hook_activity[0];
        assert_eq!(
            operation.command.as_deref(),
            Some("git pull --rebase --no-prune -- origin refs/heads/main")
        );
        assert_eq!(operation.combined_output(), "Rebasing (1/2)\n");
        assert_eq!(operation.status, status);
        assert!(!operation.is_reportable());
    }
}

fn hook_id(child_id: u64) -> HookExecutionId {
    HookExecutionId {
        sid: Arc::from("test-session"),
        child_id,
    }
}

fn start_hook(repo: &mut RepoState, operation_id: GitOperationId, child_id: u64) {
    started(
        repo,
        operation_id,
        "Commit".to_string(),
        Some("Exercise hooks".to_string()),
        SystemTime::UNIX_EPOCH,
        false,
    );
    apply_event(
        repo,
        operation_id,
        GitOperationEvent::HookStarted {
            id: hook_id(child_id),
            name: "pre-commit".to_string(),
        },
    );
}

#[test]
fn sanitizer_strips_ansi_and_control_bytes_but_keeps_layout() {
    assert_eq!(
        sanitize_activity_text("\u{1b}[31mred\u{1b}[0m\tvalue\n\u{7}"),
        "red\tvalue\n"
    );
}

#[test]
fn operations_without_hooks_are_removed_when_they_finish() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(10);
    started(
        &mut repo,
        operation_id,
        "Fetch".to_string(),
        Some("All remotes".to_string()),
        SystemTime::UNIX_EPOCH,
        false,
    );

    finished(
        &mut repo,
        operation_id,
        GitOperationOuterOutcome::Succeeded,
        Duration::from_millis(10),
    );

    assert!(repo.feedback.hook_activity.is_empty());
}

/// An explicit `git lfs fetch` runs no hook; its progress alone must keep
/// the entry visible, or a long download would vanish from the panel.
#[test]
fn transfer_progress_is_shown_and_kept_without_hooks() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(12);
    started(
        &mut repo,
        operation_id,
        "LFS fetch".to_string(),
        Some("all refs".to_string()),
        SystemTime::UNIX_EPOCH,
        false,
    );
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::TransferProgress(gitcomet_core::git_operation::TransferProgress {
            direction: "download".into(),
            files_done: 2,
            files_total: 4,
            bytes_done: 1_000_000,
            bytes_total: 8_000_000,
            name: "a.bin".into(),
        }),
    );
    assert_eq!(
        repo.feedback.hook_activity[0].latest_line,
        "LFS download 2/4 files · 1 MB of 8 MB"
    );

    finished(
        &mut repo,
        operation_id,
        GitOperationOuterOutcome::Succeeded,
        Duration::from_millis(10),
    );
    assert_eq!(repo.feedback.hook_activity.len(), 1);
    assert!(repo.feedback.hook_activity[0].transfer.is_some());
}

#[test]
fn ignored_post_hook_failure_becomes_a_visible_warning() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(11);
    start_hook(&mut repo, operation_id, 1);
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::Output {
            chunks: vec![GitOutputChunk {
                stream: GitOutputStream::Stderr,
                text: "\u{1b}[31mcheck failed\u{1b}[0m\n".to_string(),
            }],
        },
    );
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::HookFinished {
            id: hook_id(1),
            name: "post-commit".to_string(),
            exit_code: Some(7),
            duration: Duration::from_millis(25),
        },
    );

    finished(
        &mut repo,
        operation_id,
        GitOperationOuterOutcome::Succeeded,
        Duration::from_millis(40),
    );

    let operation = &repo.feedback.hook_activity[0];
    assert_eq!(
        operation.status,
        GitHookOperationStatus::SucceededWithHookFailure
    );
    assert_eq!(operation.hooks[0].status, GitHookRunStatus::Failed);
    assert_eq!(operation.hooks[0].exit_code, Some(7));
    assert_eq!(operation.context.as_deref(), Some("Exercise hooks"));
    assert_eq!(operation.combined_output(), "check failed\n");
}

#[test]
fn non_blocking_post_checkout_failure_is_a_warning_even_when_git_exits_nonzero() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(13);
    started(
        &mut repo,
        operation_id,
        "Checkout branch".to_string(),
        Some("feature/hooks".to_string()),
        SystemTime::UNIX_EPOCH,
        false,
    );
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::HookStarted {
            id: hook_id(1),
            name: "post-checkout".to_string(),
        },
    );
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::HookFinished {
            id: hook_id(1),
            name: "post-checkout".to_string(),
            exit_code: Some(1),
            duration: Duration::from_millis(10),
        },
    );

    finished(
        &mut repo,
        operation_id,
        GitOperationOuterOutcome::Failed,
        Duration::from_millis(20),
    );

    assert_eq!(
        repo.feedback.hook_activity[0].status,
        GitHookOperationStatus::SucceededWithHookFailure,
        "post-checkout cannot roll back the checkout, so its failure is non-blocking"
    );
}

#[test]
fn retained_output_storage_is_shared_across_repo_snapshots() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(14);
    start_hook(&mut repo, operation_id, 1);
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::Output {
            chunks: vec![GitOutputChunk {
                stream: GitOutputStream::Stdout,
                text: "retained hook output\n".repeat(1024),
            }],
        },
    );

    let cloned = repo.clone();
    let original_queue_ptr = repo.feedback.hook_activity[0].output.as_slices().0.as_ptr();
    let cloned_queue_ptr = cloned.feedback.hook_activity[0]
        .output
        .as_slices()
        .0
        .as_ptr();
    let original_text_ptr = repo.feedback.hook_activity[0].output[0].text.as_ptr();
    let cloned_text_ptr = cloned.feedback.hook_activity[0].output[0].text.as_ptr();

    assert_eq!(
        original_queue_ptr, cloned_queue_ptr,
        "cloning RepoState should retain the shared output collection"
    );
    assert_eq!(
        original_text_ptr, cloned_text_ptr,
        "cloning RepoState should not copy retained output strings"
    );
}

#[test]
fn cancelling_marks_the_operation_and_unfinished_hook() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(12);
    start_hook(&mut repo, operation_id, 1);

    assert!(request_cancel(&mut repo, operation_id));
    assert_eq!(
        repo.feedback.hook_activity[0].status,
        GitHookOperationStatus::Cancelling
    );
    finished(
        &mut repo,
        operation_id,
        GitOperationOuterOutcome::Cancelled,
        Duration::from_millis(50),
    );

    assert_eq!(
        repo.feedback.hook_activity[0].status,
        GitHookOperationStatus::Cancelled
    );
    assert_eq!(
        repo.feedback.hook_activity[0].hooks[0].status,
        GitHookRunStatus::Cancelled
    );
}

#[test]
fn activity_history_respects_the_metadata_cap() {
    let mut repo = repo_state();
    for index in 0..=MAX_ACTIVITY_ENTRIES {
        let operation_id = GitOperationId(index as u64 + 1);
        start_hook(&mut repo, operation_id, index as u64 + 1);
        finished(
            &mut repo,
            operation_id,
            GitOperationOuterOutcome::Succeeded,
            Duration::from_millis(1),
        );
    }

    assert_eq!(repo.feedback.hook_activity.len(), MAX_ACTIVITY_ENTRIES);
    assert_eq!(repo.feedback.hook_activity[0].id, GitOperationId(2));
}

#[test]
fn pull_command_records_share_the_activity_metadata_and_output_budgets() {
    let mut repo = repo_state();
    for index in 0..=MAX_ACTIVITY_ENTRIES {
        let id = GitOperationId(index as u64 + 1);
        started(
            &mut repo,
            id,
            "Pull".into(),
            None,
            SystemTime::UNIX_EPOCH,
            true,
        );
        apply_event(
            &mut repo,
            id,
            GitOperationEvent::CommandInvocation {
                command: "git pull --rebase --no-prune -- origin refs/heads/main".into(),
            },
        );
        apply_event(
            &mut repo,
            id,
            GitOperationEvent::Output {
                chunks: vec![GitOutputChunk {
                    stream: GitOutputStream::Stderr,
                    text: "x".repeat(MAX_OPERATION_OUTPUT_BYTES + 1),
                }],
            },
        );
        finished(
            &mut repo,
            id,
            GitOperationOuterOutcome::Succeeded,
            Duration::from_secs(1),
        );
    }
    assert_eq!(repo.feedback.hook_activity.len(), MAX_ACTIVITY_ENTRIES);
    assert_eq!(repo.feedback.hook_activity[0].id, GitOperationId(2));
    assert_eq!(
        repo.feedback
            .hook_activity
            .iter()
            .map(|operation| operation.output_bytes)
            .sum::<usize>(),
        MAX_REPO_OUTPUT_BYTES
    );
    assert!(
        repo.feedback
            .hook_activity
            .iter()
            .all(|operation| operation.has_activity_details() && !operation.is_reportable())
    );
    assert!(repo.feedback.hook_activity.last().unwrap().output_truncated);
    assert_eq!(
        repo.feedback.hook_activity.last().unwrap().output_bytes,
        MAX_OPERATION_OUTPUT_BYTES
    );
}

#[test]
fn hookless_provisional_operation_does_not_evict_real_activity_metadata() {
    let mut repo = repo_state();
    for index in 0..MAX_ACTIVITY_ENTRIES {
        let operation_id = GitOperationId(index as u64 + 1);
        start_hook(&mut repo, operation_id, index as u64 + 1);
        finished(
            &mut repo,
            operation_id,
            GitOperationOuterOutcome::Succeeded,
            Duration::from_millis(1),
        );
    }
    let provisional_id = GitOperationId(10_000);

    started(
        &mut repo,
        provisional_id,
        "Fetch".to_string(),
        Some("All remotes".to_string()),
        SystemTime::UNIX_EPOCH,
        false,
    );

    assert!(
        repo.feedback
            .hook_activity
            .iter()
            .any(|operation| operation.id == GitOperationId(1)),
        "a hookless operation must not consume the retained-activity budget"
    );
    finished(
        &mut repo,
        provisional_id,
        GitOperationOuterOutcome::Succeeded,
        Duration::from_millis(1),
    );
    assert_eq!(repo.feedback.hook_activity.len(), MAX_ACTIVITY_ENTRIES);
    assert_eq!(repo.feedback.hook_activity[0].id, GitOperationId(1));
}

#[test]
fn hookless_provisional_output_does_not_evict_real_activity_logs() {
    let mut repo = repo_state();
    for index in 0..16 {
        let operation_id = GitOperationId(index + 1);
        start_hook(&mut repo, operation_id, index + 1);
        apply_event(
            &mut repo,
            operation_id,
            GitOperationEvent::Output {
                chunks: vec![GitOutputChunk {
                    stream: GitOutputStream::Stdout,
                    text: "x".repeat(MAX_OPERATION_OUTPUT_BYTES),
                }],
            },
        );
        finished(
            &mut repo,
            operation_id,
            GitOperationOuterOutcome::Succeeded,
            Duration::from_millis(1),
        );
    }
    let provisional_id = GitOperationId(10_001);
    started(
        &mut repo,
        provisional_id,
        "Fetch".to_string(),
        Some("All remotes".to_string()),
        SystemTime::UNIX_EPOCH,
        false,
    );

    apply_event(
        &mut repo,
        provisional_id,
        GitOperationEvent::Output {
            chunks: vec![GitOutputChunk {
                stream: GitOutputStream::Stderr,
                text: "y".repeat(MAX_OPERATION_OUTPUT_BYTES),
            }],
        },
    );

    let retained_hook_output = repo
        .feedback
        .hook_activity
        .iter()
        .filter(|operation| operation.is_reportable())
        .map(|operation| operation.output_bytes)
        .sum::<usize>();
    assert_eq!(
        retained_hook_output, MAX_REPO_OUTPUT_BYTES,
        "provisional output must not consume the saved hook-log budget"
    );
    assert_eq!(
        repo.feedback
            .hook_activity
            .iter()
            .find(|operation| operation.id == GitOperationId(1))
            .expect("oldest retained hook run")
            .output_bytes,
        MAX_OPERATION_OUTPUT_BYTES
    );
}

#[test]
fn output_caps_keep_a_valid_utf8_tail_and_bound_the_repository() {
    let mut repo = repo_state();
    for index in 0..17 {
        let operation_id = GitOperationId(index + 1);
        start_hook(&mut repo, operation_id, index + 1);
        apply_event(
            &mut repo,
            operation_id,
            GitOperationEvent::Output {
                chunks: vec![GitOutputChunk {
                    stream: GitOutputStream::Stdout,
                    text: "é".repeat(MAX_OPERATION_OUTPUT_BYTES),
                }],
            },
        );
        finished(
            &mut repo,
            operation_id,
            GitOperationOuterOutcome::Succeeded,
            Duration::from_millis(1),
        );
    }

    assert!(
        repo.feedback
            .hook_activity
            .iter()
            .all(|operation| operation.output_bytes <= MAX_OPERATION_OUTPUT_BYTES)
    );
    assert!(
        repo.feedback
            .hook_activity
            .iter()
            .all(|operation| operation.latest_line.len() <= MAX_LATEST_LINE_BYTES)
    );
    assert!(repo.feedback.hook_activity.iter().all(|operation| {
        operation.output.is_empty()
            || operation
                .output
                .iter()
                .all(|chunk| chunk.text.starts_with('é'))
    }));
    assert!(
        repo.feedback
            .hook_activity
            .iter()
            .map(|operation| operation.output_bytes)
            .sum::<usize>()
            <= MAX_REPO_OUTPUT_BYTES
    );
}

#[test]
fn repository_output_cap_also_applies_to_concurrent_hooks() {
    let mut repo = repo_state();
    for index in 0..17 {
        let operation_id = GitOperationId(index + 1);
        start_hook(&mut repo, operation_id, index + 1);
        apply_event(
            &mut repo,
            operation_id,
            GitOperationEvent::Output {
                chunks: vec![GitOutputChunk {
                    stream: GitOutputStream::Stderr,
                    text: "x".repeat(MAX_OPERATION_OUTPUT_BYTES),
                }],
            },
        );
    }

    assert!(
        repo.feedback
            .hook_activity
            .iter()
            .map(|operation| operation.output_bytes)
            .sum::<usize>()
            <= MAX_REPO_OUTPUT_BYTES
    );
    assert!(repo.feedback.hook_activity[0].output_truncated);
}

fn stderr(text: &str) -> GitOperationEvent {
    GitOperationEvent::Output {
        chunks: vec![GitOutputChunk {
            stream: GitOutputStream::Stderr,
            text: text.to_string(),
        }],
    }
}

#[test]
fn progress_lane_operation_follows_git_meters_split_across_chunks() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(41);
    started(
        &mut repo,
        operation_id,
        "Fetch".to_string(),
        None,
        SystemTime::UNIX_EPOCH,
        true,
    );

    apply_event(
        &mut repo,
        operation_id,
        stderr(
            "remote: Counting objects: 100% (5/5), done.\nReceiving objects:  40% (2/5)\rReceiving obj",
        ),
    );
    let progress = repo.feedback.hook_activity[0]
        .progress
        .clone()
        .expect("meter");
    assert_eq!(
        (&*progress.title, progress.percent),
        ("Receiving objects", Some(40))
    );

    apply_event(&mut repo, operation_id, stderr("ects:  60% (3/5)\r"));
    let progress = repo.feedback.hook_activity[0]
        .progress
        .clone()
        .expect("meter");
    assert_eq!(
        (&*progress.title, progress.percent),
        ("Receiving objects", Some(60))
    );

    let measured = gitcomet_core::git_progress::GitProgressMeter::estimated("Writing new pack", 46);
    apply_event(
        &mut repo,
        operation_id,
        GitOperationEvent::Progress(measured.clone()),
    );
    assert_eq!(repo.feedback.hook_activity[0].progress, Some(measured));
}

#[test]
fn operations_without_a_progress_lane_do_not_parse_meters() {
    let mut repo = repo_state();
    let operation_id = GitOperationId(42);
    started(
        &mut repo,
        operation_id,
        "Commit".to_string(),
        None,
        SystemTime::UNIX_EPOCH,
        false,
    );
    apply_event(
        &mut repo,
        operation_id,
        stderr("Receiving objects:  40% (2/5)\r"),
    );
    assert_eq!(repo.feedback.hook_activity[0].progress, None);
}
