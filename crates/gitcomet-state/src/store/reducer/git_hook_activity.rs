use crate::model::{
    GitHookOperation, GitHookOperationStatus, GitHookOutputChunk, GitHookRun, GitHookRunStatus,
    GitOperationOuterOutcome, RepoState,
};
use gitcomet_core::git_operation::{GitOperationEvent, GitOperationId};
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

const MAX_ACTIVITY_ENTRIES: usize = 200;
const MAX_OPERATION_OUTPUT_BYTES: usize = 256 * 1024;
const MAX_REPO_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
const MAX_LATEST_LINE_BYTES: usize = 4 * 1024;

pub(super) fn started(
    repo: &mut RepoState,
    operation_id: GitOperationId,
    label: String,
    context: Option<String>,
    time: SystemTime,
) {
    if repo
        .feedback
        .hook_activity
        .iter()
        .any(|entry| entry.id == operation_id)
    {
        return;
    }
    repo.feedback.hook_activity.push(GitHookOperation {
        id: operation_id,
        label,
        context,
        time,
        duration: None,
        status: GitHookOperationStatus::Running,
        hooks: Vec::new(),
        output: Arc::new(VecDeque::new()),
        output_bytes: 0,
        output_truncated: false,
        latest_line: String::new(),
    });
    repo.feedback.hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
}

pub(super) fn apply_event(
    repo: &mut RepoState,
    operation_id: GitOperationId,
    event: GitOperationEvent,
) {
    let Some(operation) = repo
        .feedback
        .hook_activity
        .iter_mut()
        .find(|operation| operation.id == operation_id)
    else {
        return;
    };

    let mut started_first_hook = false;
    match event {
        GitOperationEvent::Output { chunks } => {
            for chunk in chunks {
                let text = sanitize_activity_text(&chunk.text);
                if text.is_empty() {
                    continue;
                }
                if let Some(line) = text
                    .split(['\r', '\n'])
                    .rev()
                    .map(str::trim)
                    .find(|line| !line.is_empty())
                {
                    operation.latest_line = utf8_tail(line, MAX_LATEST_LINE_BYTES);
                }
                operation.output_bytes = operation.output_bytes.saturating_add(text.len());
                Arc::make_mut(&mut operation.output).push_back(GitHookOutputChunk {
                    stream: chunk.stream,
                    text: Arc::from(text),
                });
            }
            trim_operation_output(operation);
        }
        GitOperationEvent::HookStarted { id, name } => {
            if operation.hooks.iter().any(|hook| hook.id == id) {
                return;
            }
            started_first_hook = operation.hooks.is_empty();
            operation.hooks.push(GitHookRun {
                id,
                name,
                status: GitHookRunStatus::Running,
                exit_code: None,
                duration: None,
            });
        }
        GitOperationEvent::HookFinished {
            id,
            name,
            exit_code,
            duration,
        } => {
            if let Some(hook) = operation.hooks.iter_mut().find(|hook| hook.id == id) {
                hook.name = name;
                hook.exit_code = exit_code;
                hook.duration = Some(duration);
                hook.status = if exit_code == Some(0) {
                    GitHookRunStatus::Succeeded
                } else {
                    GitHookRunStatus::Failed
                };
            }
        }
    }
    if started_first_hook {
        trim_metadata(repo);
    }
    enforce_repo_output_budget(repo);
    repo.feedback.hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
}

pub(super) fn request_cancel(repo: &mut RepoState, operation_id: GitOperationId) -> bool {
    let Some(operation) = repo
        .feedback
        .hook_activity
        .iter_mut()
        .find(|operation| operation.id == operation_id && operation.status.is_active())
    else {
        return false;
    };
    operation.status = GitHookOperationStatus::Cancelling;
    repo.feedback.hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
    true
}

pub(super) fn finished(
    repo: &mut RepoState,
    operation_id: GitOperationId,
    outer_outcome: GitOperationOuterOutcome,
    duration: Duration,
) {
    let Some(index) = repo
        .feedback
        .hook_activity
        .iter()
        .position(|operation| operation.id == operation_id)
    else {
        return;
    };

    if !repo.feedback.hook_activity[index].has_hooks() {
        repo.feedback.hook_activity.remove(index);
        repo.feedback.hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
        return;
    }

    let operation = &mut repo.feedback.hook_activity[index];
    operation.duration = Some(duration);
    for hook in &mut operation.hooks {
        if hook.status == GitHookRunStatus::Running {
            hook.status = match outer_outcome {
                GitOperationOuterOutcome::Cancelled | GitOperationOuterOutcome::TimedOut => {
                    GitHookRunStatus::Cancelled
                }
                GitOperationOuterOutcome::Succeeded | GitOperationOuterOutcome::Failed => {
                    GitHookRunStatus::Failed
                }
            };
        }
    }
    let hook_failed = operation
        .hooks
        .iter()
        .any(|hook| hook.status == GitHookRunStatus::Failed);
    let only_post_checkout_failed = hook_failed
        && operation
            .hooks
            .iter()
            .filter(|hook| hook.status == GitHookRunStatus::Failed)
            .all(|hook| hook.name == "post-checkout");
    operation.status = match outer_outcome {
        GitOperationOuterOutcome::Succeeded if hook_failed => {
            GitHookOperationStatus::SucceededWithHookFailure
        }
        GitOperationOuterOutcome::Succeeded => GitHookOperationStatus::Succeeded,
        GitOperationOuterOutcome::Failed if only_post_checkout_failed => {
            GitHookOperationStatus::SucceededWithHookFailure
        }
        GitOperationOuterOutcome::Failed => GitHookOperationStatus::Failed,
        GitOperationOuterOutcome::Cancelled => GitHookOperationStatus::Cancelled,
        GitOperationOuterOutcome::TimedOut => GitHookOperationStatus::TimedOut,
    };
    trim_metadata(repo);
    enforce_repo_output_budget(repo);
    repo.feedback.hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
}

fn trim_metadata(repo: &mut RepoState) {
    while repo
        .feedback
        .hook_activity
        .iter()
        .filter(|operation| operation.has_hooks())
        .count()
        > MAX_ACTIVITY_ENTRIES
    {
        let Some(index) = repo
            .feedback
            .hook_activity
            .iter()
            .position(|operation| operation.has_hooks() && !operation.status.is_active())
        else {
            break;
        };
        repo.feedback.hook_activity.remove(index);
    }
}

fn trim_operation_output(operation: &mut GitHookOperation) {
    trim_operation_output_to(operation, MAX_OPERATION_OUTPUT_BYTES);
}

fn trim_operation_output_to(operation: &mut GitHookOperation, max_bytes: usize) {
    while operation.output_bytes > max_bytes {
        let Some(mut front) = Arc::make_mut(&mut operation.output).pop_front() else {
            operation.output_bytes = 0;
            break;
        };
        let excess = operation.output_bytes - max_bytes;
        if front.text.len() > excess {
            let mut cut = excess;
            while cut < front.text.len() && !front.text.is_char_boundary(cut) {
                cut += 1;
            }
            front.text = Arc::from(&front.text[cut..]);
            operation.output_bytes = operation.output_bytes.saturating_sub(cut);
            Arc::make_mut(&mut operation.output).push_front(front);
            operation.output_truncated = true;
            break;
        }
        operation.output_bytes = operation.output_bytes.saturating_sub(front.text.len());
        operation.output_truncated = true;
    }
}

fn enforce_repo_output_budget(repo: &mut RepoState) {
    let mut total = repo
        .feedback
        .hook_activity
        .iter()
        .filter(|operation| operation.has_hooks())
        .map(|operation| operation.output_bytes)
        .sum::<usize>();
    if total <= MAX_REPO_OUTPUT_BYTES {
        return;
    }
    for operation in &mut repo.feedback.hook_activity {
        if total <= MAX_REPO_OUTPUT_BYTES {
            break;
        }
        if !operation.has_hooks() || operation.status.is_active() || operation.output_bytes == 0 {
            continue;
        }
        total = total.saturating_sub(operation.output_bytes);
        Arc::make_mut(&mut operation.output).clear();
        operation.output_bytes = 0;
        operation.output_truncated = true;
    }
    if total <= MAX_REPO_OUTPUT_BYTES {
        return;
    }
    for operation in &mut repo.feedback.hook_activity {
        if total <= MAX_REPO_OUTPUT_BYTES {
            break;
        }
        if !operation.has_hooks() {
            continue;
        }
        let previous = operation.output_bytes;
        let target = previous.saturating_sub(total - MAX_REPO_OUTPUT_BYTES);
        trim_operation_output_to(operation, target);
        total = total.saturating_sub(previous - operation.output_bytes);
    }
}

pub(super) fn utf8_tail(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    let mut start = text.len() - max_bytes;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

fn sanitize_activity_text(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            0x1b if bytes.get(index + 1) == Some(&b'[') => {
                index += 2;
                while index < bytes.len() {
                    let byte = bytes[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            0x1b if bytes.get(index + 1) == Some(&b']') => {
                index += 2;
                while index < bytes.len() {
                    if bytes[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            byte if byte == b'\n' || byte == b'\r' || byte == b'\t' || byte >= 0x20 => {
                output.push(byte);
                index += 1;
            }
            _ => index += 1,
        }
    }
    String::from_utf8(output).unwrap_or_default()
}

#[cfg(test)]
mod tests {
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
        );

        finished(
            &mut repo,
            operation_id,
            GitOperationOuterOutcome::Succeeded,
            Duration::from_millis(10),
        );

        assert!(repo.feedback.hook_activity.is_empty());
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
            .filter(|operation| operation.has_hooks())
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
}
