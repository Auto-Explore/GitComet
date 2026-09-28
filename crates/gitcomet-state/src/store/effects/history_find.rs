use super::*;
use crate::history_find::{HistoryFindEffect, HistoryFindMsg};
use std::sync::OnceLock;

pub(super) fn schedule(
    repos: &util::RepoMap,
    tx: StoreWorkerSender,
    work: HistoryFindEffect,
    parent: CancellationToken,
) {
    // A complete history scan must not occupy interactive range-load workers.
    static EXECUTOR: OnceLock<TaskExecutor> = OnceLock::new();
    let failed = work.clone();
    util::spawn_detached_with_repo_or_else(
        EXECUTOR.get_or_init(|| TaskExecutor::new(1)),
        "history-find",
        repos,
        work.repo_id,
        tx,
        move |repo, tx| {
            let cancellation = work.cancellation.with_parent(parent);
            let send = |result| {
                util::send_or_log(
                    &tx,
                    Msg::HistoryFind(HistoryFindMsg::Found {
                        repo_id: work.repo_id,
                        seq: work.seq,
                        result,
                    }),
                )
            };
            let result = work
                .cache
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .search(
                    &work.index,
                    &work.query,
                    &cancellation,
                    |range| repo.read_history_range(&work.index, range, &cancellation),
                    |chunk| send(Ok(chunk)),
                );
            if let Err(error) = result {
                send(Err(error));
            }
        },
        move |tx| {
            util::send_or_log(
                &tx,
                Msg::HistoryFind(
                    failed.failed(Error::new(ErrorKind::Backend("Repository closed".into()))),
                ),
            )
        },
    );
}
