use crate::model::AppState;
use crate::msg::{InternalMsg, Msg, StatusEffect};
use crate::store::repository_runtime::RepositoryRuntimes;
use crate::store::{executor::TaskExecutor, worker_channel::StoreWorkerSender};
use gitcomet_core::services::{CancellationToken, GitRepository};
use std::sync::{Arc, RwLock};

use super::load_tokens::repo_load_context;
use super::util::{missing_repo_error, send_or_log, spawn_detached_with_repo_or_else};

pub(super) fn schedule(
    executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    work: StatusEffect,
) {
    let repo_id = work.repo_id();
    let Some((msg_tx, cancellation)) = repo_load_context(thread_state, repos, msg_tx, repo_id)
    else {
        return;
    };
    let task_name = match &work {
        StatusEffect::LoadWorktreeStatus { .. } => "load-worktree-status",
        StatusEffect::LoadStagedStatus { .. } => "load-staged-status",
        StatusEffect::LoadUncommittedLineStats { .. } => "load-uncommitted-line-stats",
        StatusEffect::LoadStatus { .. } => "load-status",
    };
    let missing = work.clone();
    spawn_detached_with_repo_or_else(
        executor,
        task_name,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(run(work, repo.as_ref(), &cancellation)),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(missing.failed(missing_repo_error(repo_id))),
            );
        },
    );
}

fn run(
    work: StatusEffect,
    repo: &dyn GitRepository,
    cancellation: &CancellationToken,
) -> InternalMsg {
    match work {
        StatusEffect::LoadWorktreeStatus { repo_id } => InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: repo.worktree_status(cancellation),
        },
        StatusEffect::LoadStagedStatus { repo_id } => InternalMsg::StagedStatusLoaded {
            repo_id,
            result: repo.staged_status(cancellation),
        },
        StatusEffect::LoadStatus { repo_id } => InternalMsg::StatusLoaded {
            repo_id,
            result: repo.status(cancellation),
        },
        StatusEffect::LoadUncommittedLineStats {
            repo_id,
            generation,
            status,
            large_files,
        } => {
            let result = repo.uncommitted_line_stats_for_status(&status, cancellation);
            let large_files =
                large_files.then(|| repo.uncommitted_large_files_for_status(&status, cancellation));
            InternalMsg::UncommittedLineStatsLoaded {
                repo_id,
                generation,
                result,
                large_files,
            }
        }
    }
}
