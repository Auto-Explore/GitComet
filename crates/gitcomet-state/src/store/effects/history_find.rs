use super::*;
use crate::history_find::{HistoryFindChunk, HistoryFindEffect, HistoryFindMsg};
use gitcomet_core::history_index::HISTORY_BLOCK_SIZE;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Minimum spacing between progress reports, so a long scan does not flood
/// the store with a message per block. Matches the index build's cadence.
const REPORT_INTERVAL: Duration = Duration::from_millis(100);

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
            let len = work.index.len();
            let mut pending = Vec::new();
            let mut last_report = Instant::now();
            let mut start = 0;
            while start < len {
                // The range reader's store-reopen budget is counted in blocks of this size.
                let end = (start + HISTORY_BLOCK_SIZE).min(len);
                let range = match repo.read_history_range(&work.index, start..end, &cancellation) {
                    Ok(range) => range,
                    Err(error) => return send(Err(error)),
                };
                pending.extend(
                    range
                        .commits
                        .iter()
                        .enumerate()
                        .filter(|(_, commit)| work.query.matches(commit))
                        .map(|(offset, _)| start + offset),
                );
                start = end;
                // A block without matches changes nothing the view shows.
                if start < len && !pending.is_empty() && last_report.elapsed() >= REPORT_INTERVAL {
                    last_report = Instant::now();
                    send(Ok(HistoryFindChunk {
                        matches: std::mem::take(&mut pending),
                        done: false,
                    }));
                }
            }
            send(Ok(HistoryFindChunk {
                matches: pending,
                done: true,
            }));
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
