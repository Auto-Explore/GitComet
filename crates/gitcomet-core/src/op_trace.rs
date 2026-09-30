//! Opt-in stage trace that follows one operation across threads: scripted
//! input, store dispatch, store queue, reducer, worker tasks, state
//! publication and UI application.
//!
//! Inert until [`enable`] runs (the UI probe enables it for JSONL captures):
//! every call site costs one relaxed load. Records are fixed-size and buffered
//! in memory up to a fixed bound; the probe drains them off the hot path and
//! reports anything dropped.
//!
//! An operation id travels with the thread that works on it: [`scope`] sets it
//! for the current thread, [`Stamp::capture`] carries it (with a queue time)
//! across a channel, and [`wrap_task`] restores it on a worker. Id 0 means
//! work that no traced input caused, such as background refreshes.

use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const MAX_BUFFERED: usize = 1 << 16;

static ENABLED: AtomicBool = AtomicBool::new(false);
static ORIGIN: OnceLock<Instant> = OnceLock::new();
static NEXT_OP: AtomicU64 = AtomicU64::new(1);
static NEXT_THREAD: AtomicU32 = AtomicU32::new(1);
static DROPPED: AtomicU64 = AtomicU64::new(0);
static BUFFER: Mutex<Vec<Record>> = Mutex::new(Vec::new());
static THREADS: Mutex<Vec<ThreadInfo>> = Mutex::new(Vec::new());

thread_local! {
    static CURRENT_OP: Cell<u64> = const { Cell::new(0) };
    static CURRENT_LABEL: Cell<&'static str> = const { Cell::new("") };
    static THREAD_TAG: Cell<u32> = const { Cell::new(0) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Stage {
    /// A scripted input event started dispatching. `a`: scheduled time (ns
    /// since the origin), so `at_ns - a` is the dispatch delay.
    Input,
    /// The input's handlers returned. `a`: handler duration (ns).
    InputHandled,
    /// A message was handed to the store. `label`: message kind.
    Dispatch,
    /// The store worker took a message off its queue. `a`: queue delay (ns).
    Received,
    /// A reducer pass finished and published state. `a`: reducer duration
    /// (ns), `b`: publication sequence number.
    Reduced,
    /// The reducer scheduled an effect. `label`: effect kind.
    EffectQueued,
    /// A worker started a task. `label`: the effect that spawned it (or the
    /// pool), `a`: queue delay (ns).
    TaskStarted,
    /// A worker finished a task. `label` as for `TaskStarted`, `a`: run time
    /// (ns).
    TaskFinished,
    /// The UI applied a published state. `a`: publication sequence number,
    /// `b`: time spent applying it on the UI thread (ns).
    Applied,
    /// A scenario completion witness held. `label`: witness name.
    Witness,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::InputHandled => "input_handled",
            Self::Dispatch => "dispatch",
            Self::Received => "received",
            Self::Reduced => "reduced",
            Self::EffectQueued => "effect_queued",
            Self::TaskStarted => "task_started",
            Self::TaskFinished => "task_finished",
            Self::Applied => "applied",
            Self::Witness => "witness",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Record {
    /// Nanoseconds since the origin passed to [`enable`].
    pub at_ns: u64,
    pub thread: u32,
    pub op: u64,
    pub stage: Stage,
    pub label: &'static str,
    pub a: u64,
    pub b: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThreadInfo {
    pub thread: u32,
    pub name: String,
    /// Kernel thread id where the platform exposes one, for joining with
    /// per-thread CPU samples.
    pub os_tid: Option<u64>,
}

/// Starts recording. `origin` anchors every timestamp; pass the clock the
/// consumer already reports against. Later calls keep the first origin.
pub fn enable(origin: Instant) {
    let _ = ORIGIN.set(origin);
    ENABLED.store(true, Ordering::Release);
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Nanoseconds since the origin, or 0 while disabled.
fn now_ns() -> u64 {
    ORIGIN
        .get()
        .map_or(0, |origin| since(*origin, Instant::now()))
}

/// Converts an instant to trace time, clamping instants before the origin.
pub fn instant_ns(at: Instant) -> u64 {
    ORIGIN.get().map_or(0, |origin| since(*origin, at))
}

fn since(origin: Instant, at: Instant) -> u64 {
    duration_ns(at.saturating_duration_since(origin))
}

/// A duration in trace units (nanoseconds), saturating.
pub fn duration_ns(duration: Duration) -> u64 {
    u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
}

/// A fresh operation id; ids are never reused within a process.
pub fn next_op() -> u64 {
    NEXT_OP.fetch_add(1, Ordering::Relaxed)
}

/// The operation the current thread is working on, or 0.
fn current() -> u64 {
    CURRENT_OP.with(Cell::get)
}

/// Attributes work on this thread to `op` until the guard drops.
pub fn scope(op: u64) -> ScopeGuard {
    let previous = CURRENT_OP.with(|current| current.replace(op));
    ScopeGuard { previous }
}

#[must_use = "the operation scope ends when the guard drops"]
pub struct ScopeGuard {
    previous: u64,
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        CURRENT_OP.with(|current| current.set(self.previous));
    }
}

/// Names the tasks spawned on this thread until the guard drops, e.g. with
/// the effect being scheduled, so worker records say what they ran.
pub fn label_scope(label: &'static str) -> LabelGuard {
    let previous = CURRENT_LABEL.with(|current| current.replace(label));
    LabelGuard { previous }
}

#[must_use = "the label scope ends when the guard drops"]
pub struct LabelGuard {
    previous: &'static str,
}

impl Drop for LabelGuard {
    fn drop(&mut self) {
        CURRENT_LABEL.with(|current| current.set(self.previous));
    }
}

pub fn record(stage: Stage, op: u64, label: &'static str, a: u64, b: u64) {
    if !enabled() {
        return;
    }
    let record = Record {
        at_ns: now_ns(),
        thread: thread_tag(),
        op,
        stage,
        label,
        a,
        b,
    };
    let mut buffer = BUFFER.lock().unwrap_or_else(|e| e.into_inner());
    if buffer.len() >= MAX_BUFFERED {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    buffer.push(record);
}

/// Records against the current thread's operation.
pub fn record_current(stage: Stage, label: &'static str, a: u64, b: u64) {
    if enabled() {
        record(stage, current(), label, a, b);
    }
}

fn thread_tag() -> u32 {
    THREAD_TAG.with(|tag| {
        if tag.get() == 0 {
            let id = NEXT_THREAD.fetch_add(1, Ordering::Relaxed);
            tag.set(id);
            let name = std::thread::current()
                .name()
                .map_or_else(|| format!("unnamed-{id}"), str::to_owned);
            THREADS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push(ThreadInfo {
                    thread: id,
                    name,
                    os_tid: current_os_tid(),
                });
        }
        tag.get()
    })
}

/// The calling thread's kernel id (Linux only).
pub fn current_os_tid() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        // `/proc/thread-self` links to `<pid>/task/<tid>`; no unsafe syscall needed.
        let link = std::fs::read_link("/proc/thread-self").ok()?;
        link.file_name()?.to_str()?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// Everything recorded since the last drain.
pub struct Drained {
    pub records: Vec<Record>,
    /// Threads first seen since the last drain.
    pub threads: Vec<ThreadInfo>,
    /// Total records lost to a full buffer since tracing began.
    pub dropped: u64,
}

pub fn drain() -> Drained {
    let records = std::mem::take(&mut *BUFFER.lock().unwrap_or_else(|e| e.into_inner()));
    let threads = std::mem::take(&mut *THREADS.lock().unwrap_or_else(|e| e.into_inner()));
    Drained {
        records,
        threads,
        dropped: DROPPED.load(Ordering::Relaxed),
    }
}

/// An operation id and enqueue time carried across a channel.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Stamp {
    pub op: u64,
    queued_ns: u64,
    /// The spawning thread's [`label_scope`], or empty.
    label: &'static str,
}

impl Stamp {
    /// `None` while tracing is off, so untraced runs carry nothing.
    pub fn capture() -> Option<Self> {
        enabled().then(|| Self {
            op: current(),
            queued_ns: now_ns(),
            label: CURRENT_LABEL.with(Cell::get),
        })
    }

    /// Queue delay from capture until now.
    pub fn waited_ns(self) -> u64 {
        now_ns().saturating_sub(self.queued_ns)
    }
}

/// Wraps a task for a worker pool: the spawning thread's operation is
/// restored while it runs, and its queue delay and run time are recorded.
pub fn wrap_task<F: FnOnce()>(pool: &'static str, task: F) -> impl FnOnce() {
    let stamp = Stamp::capture();
    move || match stamp {
        None => task(),
        Some(stamp) => {
            let label = if stamp.label.is_empty() {
                pool
            } else {
                stamp.label
            };
            record(Stage::TaskStarted, stamp.op, label, stamp.waited_ns(), 0);
            let _scope = scope(stamp.op);
            let started = Instant::now();
            task();
            record(
                Stage::TaskFinished,
                stamp.op,
                label,
                duration_ns(started.elapsed()),
                0,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The trace is process-global; one test exercises it end to end so
    // parallel tests cannot interleave records.
    #[test]
    fn operations_follow_work_across_threads_and_record_queue_delay() {
        enable(Instant::now());
        let _ = drain();

        let op = next_op();
        let labelled = {
            let _scope = scope(op);
            let _label = label_scope("LoadLog");
            wrap_task("pool", || {})
        };
        let task = {
            let _scope = scope(op);
            record_current(Stage::Dispatch, "Msg", 0, 0);
            wrap_task("pool", move || {
                assert_eq!(current(), op, "the worker sees the spawning operation");
                record_current(Stage::Dispatch, "FollowUp", 0, 0);
            })
        };
        assert_eq!(current(), 0, "the scope ends with its guard");
        std::thread::Builder::new()
            .name("op-trace-worker".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(2));
                task();
            })
            .unwrap()
            .join()
            .unwrap();

        labelled();
        let drained = drain();
        let ours: Vec<_> = drained
            .records
            .iter()
            .filter(|r| r.op == op && r.label != "LoadLog")
            .collect();
        assert!(
            drained
                .records
                .iter()
                .any(|r| r.op == op && r.stage == Stage::TaskStarted && r.label == "LoadLog"),
            "a labelled spawn names the task after the effect"
        );
        let stages: Vec<_> = ours.iter().map(|r| (r.stage, r.label)).collect();
        assert_eq!(
            stages,
            [
                (Stage::Dispatch, "Msg"),
                (Stage::TaskStarted, "pool"),
                (Stage::Dispatch, "FollowUp"),
                (Stage::TaskFinished, "pool"),
            ]
        );
        assert!(ours[1].a >= 2_000_000, "queue delay covers the 2 ms wait");
        assert_ne!(ours[0].thread, ours[1].thread);
        let worker = drained
            .threads
            .iter()
            .find(|t| t.name == "op-trace-worker")
            .expect("worker thread registered");
        assert_eq!(worker.os_tid.is_some(), cfg!(target_os = "linux"));

        // A consumer that stops draining costs bounded memory, and says so.
        let dropped = drain().dropped;
        for _ in 0..=MAX_BUFFERED {
            record(Stage::Dispatch, 0, "flood", 0, 0);
        }
        let flooded = drain();
        assert_eq!(flooded.records.len(), MAX_BUFFERED);
        assert_eq!(flooded.dropped, dropped + 1);
    }
}
