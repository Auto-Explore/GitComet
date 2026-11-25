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
    progress_lane: bool,
) {
    if repo
        .feedback
        .hook_activity
        .iter()
        .any(|entry| entry.id == operation_id)
    {
        return;
    }
    repo.feedback_mut().hook_activity.push(GitHookOperation {
        id: operation_id,
        label,
        context,
        time,
        duration: None,
        status: GitHookOperationStatus::Running,
        handled_by_prompt: false,
        hooks: Vec::new(),
        output: Arc::new(VecDeque::new()),
        output_bytes: 0,
        output_truncated: false,
        latest_line: String::new(),
        command: None,
        command_started: false,
        transfer: None,
        progress_lane,
        progress: None,
    });
    repo.feedback_mut().hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
}

pub(super) fn apply_event(
    repo: &mut RepoState,
    operation_id: GitOperationId,
    event: GitOperationEvent,
) {
    let Some(operation) = repo
        .feedback_mut()
        .hook_activity
        .iter_mut()
        .find(|operation| operation.id == operation_id)
    else {
        return;
    };

    let mut became_retained = false;
    match event {
        GitOperationEvent::CommandStarted => operation.command_started = true,
        GitOperationEvent::CommandInvocation { command } => {
            became_retained = !operation.has_activity_details();
            operation.command = Some(Arc::from(sanitize_activity_text(&command)));
        }
        GitOperationEvent::Output { chunks } => {
            for chunk in chunks {
                let text = sanitize_activity_text(&chunk.text);
                if text.is_empty() {
                    continue;
                }
                if !operation.progress_lane
                    && let Some(line) = text
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
            if operation.progress_lane {
                if let Some(line) = latest_line_in(&operation.output) {
                    operation.latest_line = utf8_tail(&line, MAX_LATEST_LINE_BYTES);
                }
                if let Some(meter) = latest_progress_in(&operation.output) {
                    operation.progress = Some(meter);
                }
            }
            trim_operation_output(operation);
        }
        GitOperationEvent::Progress(meter) => {
            operation.progress = Some(meter);
        }
        GitOperationEvent::HookStarted { id, name } => {
            if operation.hooks.iter().any(|hook| hook.id == id) {
                return;
            }
            became_retained = operation.hooks.is_empty();
            operation.hooks.push(GitHookRun {
                id,
                name,
                status: GitHookRunStatus::Running,
                exit_code: None,
                duration: None,
            });
        }
        GitOperationEvent::TransferProgress(progress) => {
            operation.latest_line = progress.summary();
            operation.transfer = Some(progress);
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
    if became_retained {
        trim_metadata(repo);
    }
    enforce_repo_output_budget(repo);
    repo.feedback_mut().hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
}

fn latest_line_in(output: &VecDeque<GitHookOutputChunk>) -> Option<String> {
    let tail = tail_text_in(output, MAX_LATEST_LINE_BYTES * 2);
    let complete = tail.rfind(['\r', '\n']).map(|end| &tail[..end])?;
    complete
        .split(['\r', '\n'])
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_owned)
}

fn tail_text_in(output: &VecDeque<GitHookOutputChunk>, max_bytes: usize) -> String {
    let mut tail: Vec<&str> = Vec::new();
    let mut bytes = 0;
    for chunk in output.iter().rev() {
        tail.push(&chunk.text);
        bytes += chunk.text.len();
        if bytes >= max_bytes {
            break;
        }
    }
    tail.reverse();
    tail.concat()
}

fn latest_progress_in(
    output: &VecDeque<GitHookOutputChunk>,
) -> Option<gitcomet_core::git_progress::GitProgressMeter> {
    const TAIL_BYTES: usize = 1024;
    gitcomet_core::git_progress::latest_progress(&tail_text_in(output, TAIL_BYTES))
}

pub(super) fn request_cancel(repo: &mut RepoState, operation_id: GitOperationId) -> bool {
    let Some(operation) = repo
        .feedback_mut()
        .hook_activity
        .iter_mut()
        .find(|operation| operation.id == operation_id && operation.status.is_active())
    else {
        return false;
    };
    operation.status = GitHookOperationStatus::Cancelling;
    repo.feedback_mut().hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
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

    if !repo.feedback.hook_activity[index].has_activity_details() {
        repo.feedback_mut().hook_activity.remove(index);
        repo.feedback_mut().hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
        return;
    }

    let operation = &mut repo.feedback_mut().hook_activity[index];
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
    repo.feedback_mut().hook_activity_rev = repo.feedback.hook_activity_rev.wrapping_add(1);
}

fn trim_metadata(repo: &mut RepoState) {
    while repo
        .feedback
        .hook_activity
        .iter()
        .filter(|operation| operation.has_activity_details())
        .count()
        > MAX_ACTIVITY_ENTRIES
    {
        let Some(index) = repo.feedback.hook_activity.iter().position(|operation| {
            operation.has_activity_details() && !operation.status.is_active()
        }) else {
            break;
        };
        repo.feedback_mut().hook_activity.remove(index);
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
        .filter(|operation| operation.has_activity_details())
        .map(|operation| operation.output_bytes)
        .sum::<usize>();
    if total <= MAX_REPO_OUTPUT_BYTES {
        return;
    }
    for operation in &mut repo.feedback_mut().hook_activity {
        if total <= MAX_REPO_OUTPUT_BYTES {
            break;
        }
        if !operation.has_activity_details()
            || operation.status.is_active()
            || operation.output_bytes == 0
        {
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
    for operation in &mut repo.feedback_mut().hook_activity {
        if total <= MAX_REPO_OUTPUT_BYTES {
            break;
        }
        if !operation.has_activity_details() {
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
mod tests;
