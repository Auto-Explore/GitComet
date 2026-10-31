use super::super::executor::TaskExecutor;
use super::super::repository_runtime::RepositoryRuntimes;
use super::super::worker_channel::StoreWorkerSender;
use super::util::{GitOperationTask, missing_repo_error, send_or_log, spawn_with_repo_or_else};
use crate::model::RepoId;
use crate::msg::{InternalMsg, Msg};
use gitcomet_core::domain::{BareConversionPreview, RepositoryMode};
use gitcomet_core::services::GitRepository;
use std::sync::Arc;

pub(super) fn preview(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    let msg_tx = if let Some(scope) = repos.load_scope(&repo_id) {
        msg_tx.with_repo_load_guard(repo_id, scope.load_epoch, scope.cancellation.clone())
    } else {
        msg_tx
    };
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::BareConversionPreviewLoaded {
                    repo_id,
                    result: repo.preview_disable_bare_mode(),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::BareConversionPreviewLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}

pub(super) fn disable(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    common_dir: std::path::PathBuf,
    preview: Arc<BareConversionPreview>,
) {
    let Some(repo) = repos.repository(&repo_id).cloned() else {
        send_or_log(
            &msg_tx,
            Msg::Internal(InternalMsg::BareModeDisabled {
                common_dir,
                result: Err(missing_repo_error(repo_id)),
            }),
        );
        return;
    };
    executor.spawn(move || {
        let operation = GitOperationTask::start_with_progress_lane(
            repo_id,
            "Disable bare mode",
            Some(preview.root.display().to_string()),
            true,
            &msg_tx,
        );
        let result = (|| {
            super::super::bare_conversion::quiesce(&common_dir, operation.cancellation())?;
            let gate = super::super::mutation_gate::for_repo(repo.as_ref());
            let _mutation = gate.write().unwrap_or_else(|error| error.into_inner());
            let _scope = operation.attach();
            let result = repo.disable_bare_mode_cancellable(&preview, operation.cancellation())?;
            if let Err(error) = msg_tx
                .preferences
                .rekey(&result.old_common_dir, &result.new_common_dir)
            {
                send_or_log(
                    &msg_tx,
                    Msg::Internal(InternalMsg::SessionPersistFailed {
                        repo_id: Some(repo_id),
                        action: "migrating bare repository preferences",
                        error: error.to_string(),
                    }),
                );
            }
            Ok(result)
        })();
        super::super::bare_conversion::finished(&common_dir, &result, msg_tx.store_id().get());
        let outcome = GitOperationTask::outcome(&result);
        operation.finish(
            outcome,
            InternalMsg::BareModeDisabled { common_dir, result },
        );
    });
}

pub(super) fn mode_result(
    repo: &dyn GitRepository,
) -> Result<(RepositoryMode, Option<gitcomet_core::domain::CommitId>), gitcomet_core::error::Error>
{
    let mode = repo.repository_mode()?;
    let head = if mode == RepositoryMode::Bare {
        repo.head_commit_id()?
    } else {
        None
    };
    Ok((mode, head))
}

pub(super) fn refresh_mode(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
) {
    let Some(scope) = repos.load_scope(&repo_id) else {
        return;
    };
    let msg_tx = msg_tx.with_repo_load_guard(repo_id, scope.load_epoch, scope.cancellation.clone());
    spawn_with_repo_or_else(
        executor,
        repos,
        repo_id,
        msg_tx,
        move |repo, msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::RepositoryModeLoaded {
                    repo_id,
                    result: mode_result(repo.as_ref()),
                }),
            );
        },
        move |msg_tx| {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::RepositoryModeLoaded {
                    repo_id,
                    result: Err(missing_repo_error(repo_id)),
                }),
            );
        },
    );
}
