//! Benchmark-only diagnostics for native status work on the calling thread.
//! Stages are disjoint wall-time regions; the native walk includes concurrent
//! tracked-file checks and untracked traversal, whose times cannot be added.

use std::{cell::RefCell, time::Duration, time::Instant};

#[derive(Debug)]
pub struct StageTiming {
    pub label: &'static str,
    pub elapsed: Duration,
}

#[derive(Debug)]
pub struct WalkCounters {
    pub outcome: gix::status::plumbing::index_as_worktree_with_renames::Outcome,
    pub filesystem_cache: bool,
    pub thread_limit: Option<usize>,
}

#[derive(Debug, Default)]
pub struct StatusTrace {
    pub stages: Vec<StageTiming>,
    pub walks: Vec<WalkCounters>,
}

thread_local! {
    static RECORDS: RefCell<Option<StatusTrace>> = const { RefCell::new(None) };
}

/// Capture only work on this thread. Nested captures and unwinding restore
/// the previous capture; gix worker counters are collected after they join.
pub fn capture<T>(run: impl FnOnce() -> T) -> (T, StatusTrace) {
    struct Restore(Option<StatusTrace>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RECORDS.with(|records| *records.borrow_mut() = self.0.take());
        }
    }
    let restore = Restore(RECORDS.with(|records| records.replace(Some(StatusTrace::default()))));
    let value = run();
    let records = RECORDS.with(|records| records.borrow_mut().take().unwrap());
    drop(restore);
    (value, records)
}

pub(crate) struct StageTimer {
    label: &'static str,
    started: Option<Instant>,
}

impl StageTimer {
    pub(crate) fn new(label: &'static str) -> Self {
        Self {
            label,
            started: RECORDS
                .with(|records| records.borrow().is_some())
                .then(Instant::now),
        }
    }
}

impl Drop for StageTimer {
    fn drop(&mut self) {
        if let Some(started) = self.started {
            let elapsed = started.elapsed();
            RECORDS.with(|records| {
                if let Some(records) = records.borrow_mut().as_mut() {
                    records.stages.push(StageTiming {
                        label: self.label,
                        elapsed,
                    });
                }
            });
        }
    }
}

pub(crate) fn record_walk(
    outcome: &gix::status::plumbing::index_as_worktree_with_renames::Outcome,
    filesystem_cache: bool,
    thread_limit: Option<usize>,
) {
    RECORDS.with(|records| {
        if let Some(records) = records.borrow_mut().as_mut() {
            records.walks.push(WalkCounters {
                outcome: outcome.clone(),
                filesystem_cache,
                thread_limit,
            });
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_captures_unwinding_and_threads_keep_separate_records() {
        let (_, outer) = capture(|| {
            drop(StageTimer::new("outer"));
            let (_, inner) = capture(|| {
                drop(StageTimer::new("inner"));
                std::thread::spawn(|| drop(StageTimer::new("other-thread")))
                    .join()
                    .unwrap();
            });
            assert_eq!(inner.stages.len(), 1);
            assert_eq!(inner.stages[0].label, "inner");
            assert!(std::panic::catch_unwind(|| capture(|| panic!("probe failure"))).is_err());
            drop(StageTimer::new("after-unwind"));
        });
        assert_eq!(
            outer
                .stages
                .iter()
                .map(|stage| stage.label)
                .collect::<Vec<_>>(),
            ["outer", "after-unwind"]
        );
        assert!(StageTimer::new("disabled").started.is_none());
    }
}
