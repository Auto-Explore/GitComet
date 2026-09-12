use super::*;
use crate::indexed_history::{IndexedHistoryEffect as Work, IndexedHistoryMsg as Event};
use gitcomet_core::history_index::HISTORY_BLOCK_SIZE;
use std::sync::OnceLock;

pub(super) fn schedule(
    range_executor: &TaskExecutor,
    repos: &util::RepoMap,
    msg_tx: StoreWorkerSender,
    work: Work,
    parent: CancellationToken,
) {
    // The index must not occupy either interactive repo-load worker.
    static INDEX_EXECUTOR: OnceLock<TaskExecutor> = OnceLock::new();
    let executor = match &work {
        Work::Build { .. } => INDEX_EXECUTOR.get_or_init(|| TaskExecutor::new(1)),
        Work::Range { .. } => range_executor,
    };
    let repo_id = work.repo_id();
    let on_missing = work.clone();
    util::spawn_detached_with_repo_or_else(
        executor,
        "indexed-history",
        repos,
        repo_id,
        msg_tx,
        move |repo, tx| {
            let reply = match work {
                Work::Build {
                    repo_id,
                    seq,
                    mode,
                    author,
                    cancellation,
                } => {
                    let cancellation = cancellation.with_parent(parent);
                    let result = repo.build_history_index(
                        mode,
                        author.as_deref(),
                        &cancellation,
                        &mut |progress| {
                            util::send_or_log(
                                &tx,
                                Msg::IndexedHistory(Event::Progress {
                                    repo_id,
                                    seq,
                                    progress,
                                }),
                            );
                        },
                    );
                    Event::Built {
                        repo_id,
                        seq,
                        result,
                    }
                }
                Work::Range {
                    repo_id,
                    seq,
                    index,
                    start,
                    cancellation,
                } => {
                    let cancellation = cancellation.with_parent(parent);
                    let end = (start + HISTORY_BLOCK_SIZE).min(index.len());
                    let result = repo.read_history_range(&index, start..end, &cancellation);
                    Event::RangeLoaded {
                        repo_id,
                        seq,
                        start,
                        snapshot: index.snapshot.clone(),
                        result,
                    }
                }
            };
            util::send_or_log(&tx, Msg::IndexedHistory(reply));
        },
        move |tx| {
            util::send_or_log(
                &tx,
                Msg::IndexedHistory(
                    on_missing.failed(Error::new(ErrorKind::Backend("Repository closed".into()))),
                ),
            );
        },
    );
}
