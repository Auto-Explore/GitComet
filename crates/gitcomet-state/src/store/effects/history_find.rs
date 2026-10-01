use super::*;
use crate::history_find::{HistoryFindEffect, HistoryFindMsg};

/// Runs on the store's own find worker: a complete history scan must not
/// occupy interactive range-load workers, nor wait on another window's scan.
pub(super) fn schedule(
    executor: &TaskExecutor,
    repos: &util::RepoMap,
    tx: StoreWorkerSender,
    work: HistoryFindEffect,
    parent: CancellationToken,
) {
    let failed = work.clone();
    util::spawn_detached_with_repo_or_else(
        executor,
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
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut cache = work.cache.lock().unwrap_or_else(|error| {
                    // Never reuse half-written cache state after an earlier unwind.
                    let mut cache = error.into_inner();
                    *cache = Default::default();
                    work.cache.clear_poison();
                    cache
                });
                cache.search(
                    &work.index,
                    &work.query,
                    &work.stashes,
                    &cancellation,
                    |range| repo.read_history_range(&work.index, range, &cancellation),
                    |chunk| send(Ok(chunk)),
                )
            }));
            match result {
                Ok(Ok(())) => {}
                Ok(Err(error)) => send(Err(error)),
                Err(payload) => {
                    // The executor's outer catch keeps its thread alive, but
                    // find must also retire this request and discard its cache.
                    *work.cache.lock().unwrap_or_else(|error| error.into_inner()) =
                        Default::default();
                    work.cache.clear_poison();
                    send(Err(Error::new(ErrorKind::Backend(format!(
                        "History search panicked: {}",
                        super::super::send_diagnostics::panic_payload_to_string(payload.as_ref()),
                    )))));
                }
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
