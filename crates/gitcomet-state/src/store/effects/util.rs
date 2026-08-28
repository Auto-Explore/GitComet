use crate::model::GitOperationOuterOutcome;
use crate::msg::{InternalMsg, Msg};
use gitcomet_core::error::{Error, ErrorKind, GitFailureId};
use gitcomet_core::git_operation::{self, GitOperationContext, GitOperationScope};
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use super::super::{
    RepoId, executor::TaskExecutor, repo_load_trace, worker_channel::StoreWorkerSender,
};

pub(super) type RepoMap = FxHashMap<RepoId, Arc<dyn GitRepository>>;

/// Keeps operation context safe for a one-line UI subtitle. Git permits odd
/// characters in paths and user-authored messages, so never let those create
/// extra layout rows or carry terminal controls into the activity view.
pub(super) fn single_line_context(value: impl AsRef<str>) -> Option<String> {
    let value = value.as_ref();
    let mut normalized = String::with_capacity(value.len());
    let mut pending_separator = false;
    for ch in value.chars() {
        if ch.is_control() {
            pending_separator = !normalized.is_empty();
            continue;
        }
        if pending_separator {
            if !ch.is_whitespace()
                && !normalized
                    .chars()
                    .next_back()
                    .is_some_and(char::is_whitespace)
            {
                normalized.push(' ');
            }
            pending_separator = false;
        }
        normalized.push(ch);
    }
    let trimmed = normalized.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

pub(super) fn message_subject(message: &str) -> Option<String> {
    message
        .lines()
        .find(|line| !line.trim().is_empty())
        .and_then(single_line_context)
}

pub(super) fn short_commit_id(id: &str) -> String {
    id.chars().take(8).collect()
}

pub(super) fn path_context(path: &Path) -> Option<String> {
    single_line_context(path.to_string_lossy())
}

pub(super) fn paths_context(paths: &[impl AsRef<Path>], noun: &str) -> Option<String> {
    let names = paths
        .iter()
        .filter_map(|path| path_context(path.as_ref()))
        .collect::<Vec<_>>();
    match names.as_slice() {
        [] => None,
        [only] => Some(only.clone()),
        many => Some(format!(
            "{} {noun}: {}",
            many.len(),
            crate::name_summary::elide_names(many, ", ")
        )),
    }
}

pub(super) struct GitOperationTask {
    repo_id: RepoId,
    context: GitOperationContext,
    started: Instant,
    msg_tx: StoreWorkerSender,
}

impl GitOperationTask {
    pub(super) fn start(
        repo_id: RepoId,
        label: impl Into<String>,
        display_context: Option<String>,
        msg_tx: &StoreWorkerSender,
    ) -> Self {
        let label = label.into();
        let event_tx = msg_tx.clone();
        let context = GitOperationContext::new(label.clone(), move |operation_id, event| {
            send_or_log(
                &event_tx,
                Msg::Internal(InternalMsg::GitOperationEvent {
                    repo_id,
                    operation_id,
                    event,
                }),
            );
        });
        let operation_id = context.id();
        send_or_log(
            msg_tx,
            Msg::Internal(InternalMsg::GitOperationStarted {
                repo_id,
                operation_id,
                label,
                context: display_context,
                time: SystemTime::now(),
            }),
        );
        Self {
            repo_id,
            context,
            started: Instant::now(),
            msg_tx: msg_tx.clone(),
        }
    }

    pub(super) fn attach(&self) -> GitOperationScope {
        git_operation::attach(&self.context)
    }

    pub(super) fn outcome<T>(result: &Result<T, Error>) -> GitOperationOuterOutcome {
        outer_outcome(result)
    }

    pub(super) fn finish(self, outer_outcome: GitOperationOuterOutcome, message: InternalMsg) {
        send_or_log(
            &self.msg_tx,
            Msg::Internal(InternalMsg::GitOperationFinished {
                repo_id: self.repo_id,
                operation_id: self.context.id(),
                outer_outcome,
                duration: self.started.elapsed(),
                message: Box::new(message),
            }),
        );
    }
}

fn outer_outcome<T>(result: &Result<T, Error>) -> GitOperationOuterOutcome {
    match result {
        Ok(_) => GitOperationOuterOutcome::Succeeded,
        Err(error) => match error.kind() {
            ErrorKind::Cancelled => GitOperationOuterOutcome::Cancelled,
            ErrorKind::Git(failure) if failure.id() == GitFailureId::Timeout => {
                GitOperationOuterOutcome::TimedOut
            }
            _ => GitOperationOuterOutcome::Failed,
        },
    }
}

pub(super) fn spawn_with_repo(
    executor: &TaskExecutor,
    repos: &RepoMap,
    repo_id: RepoId,
    msg_tx: StoreWorkerSender,
    task: impl FnOnce(Arc<dyn GitRepository>, StoreWorkerSender) + Send + 'static,
) -> bool {
    spawn_with_repo_or_else(executor, repos, repo_id, msg_tx, task, |_| {})
}

pub(super) fn spawn_with_repo_or_else(
    executor: &TaskExecutor,
    repos: &RepoMap,
    repo_id: RepoId,
    msg_tx: StoreWorkerSender,
    task: impl FnOnce(Arc<dyn GitRepository>, StoreWorkerSender) + Send + 'static,
    on_missing: impl FnOnce(StoreWorkerSender) + Send + 'static,
) -> bool {
    if let Some(repo) = repos.get(&repo_id).cloned() {
        repo_load_trace::trace!("queue_repo_task repo_id={:?}", repo_id);
        executor.spawn(move || {
            if msg_tx.is_cancelled() {
                repo_load_trace::trace!(
                    "skip_repo_task_cancelled_before_start repo_id={:?}",
                    repo_id
                );
                return;
            }
            repo_load_trace::trace!("start_repo_task repo_id={:?}", repo_id);
            task(repo, msg_tx);
            repo_load_trace::trace!("finish_repo_task repo_id={:?}", repo_id);
        });
        true
    } else {
        if msg_tx.is_cancelled() {
            repo_load_trace::trace!("skip_missing_repo_task_cancelled repo_id={:?}", repo_id);
            return false;
        }
        repo_load_trace::trace!("repo_task_missing_handle repo_id={:?}", repo_id);
        on_missing(msg_tx);
        false
    }
}

pub(super) fn spawn_detached_with_repo_or_else(
    executor: &TaskExecutor,
    task_name: &'static str,
    repos: &RepoMap,
    repo_id: RepoId,
    msg_tx: StoreWorkerSender,
    task: impl FnOnce(Arc<dyn GitRepository>, StoreWorkerSender) + Send + 'static,
    on_missing: impl FnOnce(StoreWorkerSender) + Send + 'static,
) -> bool {
    if let Some(repo) = repos.get(&repo_id).cloned() {
        repo_load_trace::trace!(
            "spawn_detached_repo_task task={} repo_id={:?}",
            task_name,
            repo_id
        );
        executor.spawn(move || {
            if msg_tx.is_cancelled() {
                repo_load_trace::trace!(
                    "skip_detached_repo_task_cancelled_before_start task={} repo_id={:?}",
                    task_name,
                    repo_id
                );
                return;
            }
            repo_load_trace::trace!(
                "start_detached_repo_task task={} repo_id={:?}",
                task_name,
                repo_id
            );
            task(repo, msg_tx);
            repo_load_trace::trace!(
                "finish_detached_repo_task task={} repo_id={:?}",
                task_name,
                repo_id
            );
        });
        true
    } else {
        if msg_tx.is_cancelled() {
            repo_load_trace::trace!(
                "skip_missing_detached_repo_task_cancelled task={} repo_id={:?}",
                task_name,
                repo_id
            );
            return false;
        }
        repo_load_trace::trace!(
            "detached_repo_task_missing_handle task={} repo_id={:?}",
            task_name,
            repo_id
        );
        on_missing(msg_tx);
        false
    }
}

pub(super) fn send_or_log(msg_tx: &StoreWorkerSender, msg: Msg) {
    msg_tx.send_effect_or_log(msg, "store effect pipeline");
}

#[cfg(test)]
mod hook_activity_context_tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn commit_subject_uses_first_non_empty_line_and_stays_single_line() {
        assert_eq!(
            message_subject("\n  Fix hook context  \n\nLong body\n"),
            Some("Fix hook context".to_string())
        );
        assert_eq!(
            single_line_context("branch\nname\twith\u{7}controls"),
            Some("branch name with controls".to_string())
        );
    }

    #[test]
    fn path_batches_keep_specific_names_but_elide_large_sets() {
        let paths = (0..10)
            .map(|index| PathBuf::from(format!("src/file-{index}.rs")))
            .collect::<Vec<_>>();
        let context = paths_context(&paths, "files").expect("path context");
        assert!(context.starts_with("10 files: src/file-0.rs"));
        assert!(context.contains("…and 2 more"));
    }
}
