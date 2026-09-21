use super::*;
use crate::history_authors::{HistoryAuthorsEffect, HistoryAuthorsMsg};
use std::sync::OnceLock;

pub(super) fn schedule(
    repos: &util::RepoMap,
    tx: StoreWorkerSender,
    work: HistoryAuthorsEffect,
    parent: CancellationToken,
) {
    // A complete author scan must not occupy interactive range-load workers.
    static EXECUTOR: OnceLock<TaskExecutor> = OnceLock::new();
    let failed = work.clone();
    util::spawn_detached_with_repo_or_else(
        EXECUTOR.get_or_init(|| TaskExecutor::new(1)),
        "history-authors",
        repos,
        work.repo_id,
        tx,
        move |repo, tx| {
            let cancellation = work.cancellation.with_parent(parent);
            let result = repo.history_authors(work.mode, &cancellation);
            util::send_or_log(
                &tx,
                Msg::HistoryAuthors(HistoryAuthorsMsg::Loaded {
                    repo_id: work.repo_id,
                    seq: work.seq,
                    result,
                }),
            );
        },
        move |tx| {
            util::send_or_log(
                &tx,
                Msg::HistoryAuthors(
                    failed.failed(Error::new(ErrorKind::Backend("Repository closed".into()))),
                ),
            )
        },
    );
}
