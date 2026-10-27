use crate::msg::Msg;
use gitcomet_core::domain::RepoSpec;
use gitcomet_core::services::{CancellationToken, GitBackend};
use std::path::PathBuf;
use std::sync::Arc;

use super::super::{
    RepoId, executor::TaskExecutor, repo_load_trace, worker_channel::StoreWorkerSender,
};
use super::util::send_or_log;

pub(super) fn schedule_open_repo(
    executor: &TaskExecutor,
    backend: Arc<dyn GitBackend>,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    path: PathBuf,
    cancellation: CancellationToken,
) {
    let task = move || {
        if cancellation.is_cancelled() || msg_tx.is_cancelled() {
            repo_load_trace::trace!(
                "skip_open_repo_cancelled_before_start repo_id={:?} path={}",
                repo_id,
                path.display()
            );
            return;
        }
        let spec = RepoSpec { workdir: path };
        repo_load_trace::trace!(
            "start_open_repo repo_id={:?} path={}",
            repo_id,
            spec.workdir.display()
        );
        match backend
            .open_cancellable(&spec.workdir, &cancellation)
            .and_then(|repo| {
                let repository_mode = repo.repository_mode()?;
                let head_commit = if repository_mode == gitcomet_core::domain::RepositoryMode::Bare
                {
                    repo.head_commit_id()?
                } else {
                    None
                };
                Ok((repo, repository_mode, head_commit))
            }) {
            Ok((repo, repository_mode, head_commit)) => {
                if cancellation.is_cancelled() || msg_tx.is_cancelled() {
                    repo_load_trace::trace!(
                        "drop_open_repo_ok_after_cancel repo_id={:?} path={}",
                        repo_id,
                        spec.workdir.display()
                    );
                    return;
                }
                repo_load_trace::trace!(
                    "finish_open_repo_ok repo_id={:?} path={}",
                    repo_id,
                    spec.workdir.display()
                );
                let key = repo
                    .common_dir()
                    .map(gitcomet_core::path_utils::canonicalize_or_original)
                    .map(crate::model::RepositoryKey::CommonDir)
                    .unwrap_or_else(|| crate::model::RepositoryKey::Worktree(spec.workdir.clone()));
                let (preferences, persistence) =
                    msg_tx
                        .preferences
                        .initialize(key, &spec.workdir, repo.as_ref());
                if let Err(error) = persistence {
                    send_or_log(
                        &msg_tx,
                        Msg::Internal(crate::msg::InternalMsg::SessionPersistFailed {
                            repo_id: Some(repo_id),
                            action: "initializing repository preferences",
                            error: error.to_string(),
                        }),
                    );
                }
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
                        repository_mode,
                        head_commit,
                        preferences: Some(preferences),
                        repo_id,
                        spec,
                        repo,
                    }),
                );
            }
            Err(error) => {
                if cancellation.is_cancelled() || msg_tx.is_cancelled() {
                    repo_load_trace::trace!(
                        "drop_open_repo_err_after_cancel repo_id={:?} path={} error={}",
                        repo_id,
                        spec.workdir.display(),
                        error
                    );
                    return;
                }
                repo_load_trace::trace!(
                    "finish_open_repo_err repo_id={:?} path={} error={}",
                    repo_id,
                    spec.workdir.display(),
                    error
                );
                send_or_log(
                    &msg_tx,
                    Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
                        repo_id,
                        spec,
                        error,
                    }),
                );
            }
        }
    };

    executor.spawn(task);
}
