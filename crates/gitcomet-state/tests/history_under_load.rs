//! Measure uncached 256-row viewport reads through the store during a real status scan.
//! Set `GITCOMET_HISTORY_TEST_REPO` to the fixture path and run
//! `cargo test --release -p gitcomet-state --test history_under_load -- --ignored --nocapture`.
//! `GITCOMET_HISTORY_TEST_REFRESH=all` also reloads references and history.
#![allow(
    clippy::disallowed_methods,
    reason = "the manual latency probe measures wall time"
)]
use gitcomet_core::history_index::HISTORY_BLOCK_SIZE;
use gitcomet_core::op_trace::{self, Record, Stage};
use gitcomet_core::process::{GitExecutablePreference, install_git_executable_preference};
use gitcomet_git_gix::GixBackend;
use gitcomet_state::indexed_history::{HistoryViewport, IndexedHistoryMsg};
use gitcomet_state::msg::{Msg, RepoExternalChange};
use gitcomet_state::store::AppStore;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[path = "support/poll.rs"]
mod poll;

fn wait_for<T>(label: &str, check: impl FnMut() -> Option<T>) -> T {
    poll::until(Duration::from_secs(90), Duration::from_micros(200), check)
        .unwrap_or_else(|| panic!("timed out waiting for {label}"))
}

fn trace_event(records: &[Record], op: u64, stage: Stage, labels: &[&str]) -> Option<Record> {
    records
        .iter()
        .copied()
        .find(|event| event.op == op && event.stage == stage && labels.contains(&event.label))
}

fn drain_records(records: &mut Vec<Record>) {
    let batch = op_trace::drain();
    assert_eq!(batch.dropped, 0, "operation trace lost samples");
    records.extend(batch.records);
}

#[test]
#[ignore = "measures status and history against GITCOMET_HISTORY_TEST_REPO"]
fn history_under_load() {
    const SAMPLES: usize = 9;
    let path = std::env::var_os("GITCOMET_HISTORY_TEST_REPO").expect("repository path");
    let change = match std::env::var("GITCOMET_HISTORY_TEST_REFRESH").as_deref() {
        Err(std::env::VarError::NotPresent) | Ok("worktree") => RepoExternalChange::worktree(),
        Ok("all") => RepoExternalChange::all(),
        _ => panic!("GITCOMET_HISTORY_TEST_REFRESH must be worktree or all"),
    };
    assert!(install_git_executable_preference(GitExecutablePreference::SystemPath).is_available());
    op_trace::enable(Instant::now());
    let (store, _events) = AppStore::new(Arc::new(GixBackend));
    store.set_auto_fetch_allowed(false);
    store.dispatch(Msg::SetGitLogSettings {
        show_history_tags: false,
        tag_fetch_mode: gitcomet_state::model::GitLogTagFetchMode::Disabled,
        verify_commit_signatures: false,
    });
    store.dispatch(Msg::OpenRepo(path.into()));
    let repo_id = wait_for("initial history page", || {
        let state = store.snapshot();
        let repo = state.repos.first()?;
        repo.history_state.log_snapshot.as_ref().map(|_| repo.id)
    });
    store.dispatch(Msg::IndexedHistory(IndexedHistoryMsg::Ensure { repo_id }));
    let index = wait_for("history index", || {
        store
            .snapshot()
            .repos
            .first()?
            .history_state
            .indexed
            .index
            .clone()
    });
    assert!(
        SAMPLES < index.len() / HISTORY_BLOCK_SIZE,
        "too few rows for fresh samples"
    );
    let request = |block: usize| {
        let op = op_trace::next_op();
        let _scope = op_trace::scope(op);
        store.dispatch(Msg::IndexedHistory(IndexedHistoryMsg::RequestViewport {
            repo_id,
            snapshot: index.snapshot.clone(),
            viewport: HistoryViewport {
                blocks: vec![block],
                visible_count: 1,
            },
            retry: false,
        }));
        op
    };
    let wait_for_block = |block: usize| {
        wait_for("range publication", || {
            let state = store.snapshot();
            let history = &state.repos.first()?.history_state.indexed;
            assert!(
                !history.range_errors.contains_key(&block),
                "{:?}",
                history.range_errors
            );
            history.ranges.get(&block).map(|range| range.commits.len())
        })
    };
    request(0);
    assert_eq!(wait_for_block(0), HISTORY_BLOCK_SIZE);

    let status_labels = ["LoadWorktreeStatus", "LoadStatus"];
    for sample in 1..=SAMPLES {
        wait_for("previous refresh to settle", || {
            (!store
                .snapshot()
                .repos
                .first()?
                .loads_in_flight
                .any_foreground_in_flight())
            .then_some(())
        });
        let refresh_op = op_trace::next_op();
        {
            let _scope = op_trace::scope(refresh_op);
            store.dispatch(Msg::RepoExternallyChanged {
                repo_id,
                change: change.clone(),
            });
        }
        let mut records = Vec::new();
        let status_start = wait_for("status task start", || {
            drain_records(&mut records);
            trace_event(&records, refresh_op, Stage::TaskStarted, &status_labels)
        });
        let start = sample * HISTORY_BLOCK_SIZE;
        let range_op = request(start);
        assert_eq!(wait_for_block(start), HISTORY_BLOCK_SIZE);
        let status_end = wait_for("status task finish", || {
            drain_records(&mut records);
            trace_event(
                &records,
                refresh_op,
                Stage::TaskFinished,
                &[status_start.label],
            )
        });
        let dispatch = trace_event(&records, range_op, Stage::Dispatch, &["IndexedHistory"])
            .expect("range dispatch");
        assert!(
            dispatch.at_ns < status_end.at_ns,
            "status completed before range request"
        );
        let range_start = trace_event(&records, range_op, Stage::TaskStarted, &["IndexedHistory"])
            .expect("range task start");
        let published = wait_for("range result reduction", || {
            drain_records(&mut records);
            records
                .iter()
                .filter(|record| record.op == range_op && record.stage == Stage::Reduced)
                .nth(1)
                .copied()
        });
        println!(
            "{}",
            serde_json::json!({
                "start": start,
                "rows": HISTORY_BLOCK_SIZE,
                "status_ms": status_end.a as f64 / 1e6,
                "range_queue_ms": range_start.a as f64 / 1e6,
                "range_publication_ms": (published.at_ns - dispatch.at_ns) as f64 / 1e6,
            })
        );
    }
}
