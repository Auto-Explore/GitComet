use crate::msg::Msg;
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap as HashMap;
use std::sync::Arc;

use super::super::{
    RepoId, executor::TaskExecutor, repo_load_trace, worker_channel::StoreWorkerSender,
};

pub(super) type RepoMap = HashMap<RepoId, Arc<dyn GitRepository>>;

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
        TaskExecutor::spawn_detached(format!("gitcomet-{task_name}-{}", repo_id.0), move || {
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
