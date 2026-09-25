//! Opt-in, thread-local diagnostics for time spent around Git subprocesses.
//! Enabled only by tests or the `benchmarks` feature. Stages partition a
//! command's wall time; totals from concurrent commands must not be added.

#[cfg(any(test, feature = "benchmarks"))]
use std::{
    cell::RefCell,
    time::{Duration, Instant},
};

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Debug)]
pub struct CommandTiming {
    pub label: String,
    pub elapsed: Duration,
    pub stages: Vec<(&'static str, Duration)>,
}

#[cfg(any(test, feature = "benchmarks"))]
thread_local! {
    static RECORDS: RefCell<Option<Vec<CommandTiming>>> = const { RefCell::new(None) };
}

/// Capture only commands on the calling thread. Attach this inside an app's
/// worker when diagnosing asynchronous work, not around its dispatch alone.
/// Nested captures and unwinding restore the previous capture.
#[cfg(any(test, feature = "benchmarks"))]
pub fn capture<T>(run: impl FnOnce() -> T) -> (T, Vec<CommandTiming>) {
    struct Restore(Option<Vec<CommandTiming>>);
    impl Drop for Restore {
        fn drop(&mut self) {
            RECORDS.with(|records| *records.borrow_mut() = self.0.take());
        }
    }
    let restore = Restore(RECORDS.with(|records| records.replace(Some(Vec::new()))));
    let value = run();
    let records = RECORDS.with(|records| records.borrow_mut().take().unwrap());
    drop(restore);
    (value, records)
}

pub(crate) struct CommandTimer {
    #[cfg(any(test, feature = "benchmarks"))]
    state: Option<(Instant, Instant, CommandTiming)>,
}

impl CommandTimer {
    #[inline]
    pub(crate) fn new(_label: &str) -> Self {
        Self {
            #[cfg(any(test, feature = "benchmarks"))]
            state: RECORDS.with(|records| records.borrow().is_some()).then(|| {
                let now = Instant::now();
                (
                    now,
                    now,
                    CommandTiming {
                        label: _label.to_owned(),
                        elapsed: Duration::ZERO,
                        stages: Vec::new(),
                    },
                )
            }),
        }
    }

    #[inline]
    pub(crate) fn stage(&mut self, _stage: &'static str) {
        #[cfg(any(test, feature = "benchmarks"))]
        if let Some((_, previous, timing)) = self.state.as_mut() {
            let now = Instant::now();
            timing.stages.push((_stage, now.duration_since(*previous)));
            *previous = now;
        }
    }
}

impl Drop for CommandTimer {
    #[inline]
    fn drop(&mut self) {
        #[cfg(any(test, feature = "benchmarks"))]
        if let Some((start, previous, mut timing)) = self.state.take() {
            let now = Instant::now();
            timing.stages.push(("finish", now.duration_since(previous)));
            timing.elapsed = now.duration_since(start);
            RECORDS.with(|records| {
                if let Some(records) = records.borrow_mut().as_mut() {
                    records.push(timing);
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_partition_time_and_do_not_leak_across_threads_or_nested_scopes() {
        let (_, outer) = capture(|| {
            let (_, inner) = capture(|| {
                let mut timer = CommandTimer::new("inner");
                timer.stage("spawn");
                std::thread::spawn(|| drop(CommandTimer::new("other-thread")))
                    .join()
                    .unwrap();
            });
            assert_eq!(inner.len(), 1);
            assert_eq!(inner[0].label, "inner");
            assert_eq!(
                inner[0]
                    .stages
                    .iter()
                    .map(|(_, time)| *time)
                    .sum::<Duration>(),
                inner[0].elapsed
            );
            assert!(std::panic::catch_unwind(|| capture(|| panic!("probe failure"))).is_err());
            drop(CommandTimer::new("outer"));
        });
        assert_eq!(outer.len(), 1);
        assert_eq!(outer[0].label, "outer");
        assert!(CommandTimer::new("disabled").state.is_none());
    }
}
