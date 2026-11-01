use super::super::{RepoId, executor::TaskExecutor, worker_channel::StoreWorkerSender};
use super::util::{send_or_log, spawn_with_repo};
use crate::msg::Msg;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::LogCursor;
use std::path::PathBuf;

pub(super) struct FileHistoryPageRequest {
    pub(super) request: crate::model::FileHistoryRequestId,
    pub(super) path: PathBuf,
    pub(super) limit: usize,
    pub(super) cursor: Option<LogCursor>,
}

pub(super) fn schedule_load_file_history(
    executor: &TaskExecutor,
    repos: &RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    page: FileHistoryPageRequest,
) {
    let FileHistoryPageRequest {
        request,
        path,
        limit,
        cursor,
    } = page;
    spawn_with_repo(executor, repos, repo_id, msg_tx, move |repo, msg_tx| {
        let result = repo.log_file_page(&path, limit, cursor.as_ref());
        send_or_log(
            &msg_tx,
            Msg::Internal(crate::msg::InternalMsg::FileHistoryLoaded {
                request,
                repo_id,
                path,
                cursor,
                result,
            }),
        );
    });
}
