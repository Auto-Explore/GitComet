#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::cell::Cell;
#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitOpTraceKind {
    Status,
    LogWalk,
    Diff,
    Blame,
    RefEnumerate,
}

impl GitOpTraceKind {
    pub const ALL: [Self; 5] = [
        Self::Status,
        Self::LogWalk,
        Self::Diff,
        Self::Blame,
        Self::RefEnumerate,
    ];

    pub fn sidecar_metric_key(self) -> &'static str {
        match self {
            Self::Status => "status_ms",
            Self::LogWalk => "log_walk_ms",
            Self::Diff => "diff_ms",
            Self::Blame => "blame_ms",
            Self::RefEnumerate => "ref_enumerate_ms",
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitOpTraceStats {
    pub calls: u64,
    pub total_nanos: u64,
    pub last_nanos: u64,
    pub max_nanos: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl GitOpTraceStats {
    pub fn total_millis(self) -> f64 {
        nanos_to_millis(self.total_nanos)
    }

    pub fn last_millis(self) -> f64 {
        nanos_to_millis(self.last_nanos)
    }

    pub fn max_millis(self) -> f64 {
        nanos_to_millis(self.max_nanos)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct GitOpTraceSnapshot {
    pub status: GitOpTraceStats,
    pub log_walk: GitOpTraceStats,
    pub diff: GitOpTraceStats,
    pub blame: GitOpTraceStats,
    pub ref_enumerate: GitOpTraceStats,
}

#[cfg(any(test, feature = "benchmarks"))]
impl GitOpTraceSnapshot {
    pub fn stats(self, kind: GitOpTraceKind) -> GitOpTraceStats {
        match kind {
            GitOpTraceKind::Status => self.status,
            GitOpTraceKind::LogWalk => self.log_walk,
            GitOpTraceKind::Diff => self.diff,
            GitOpTraceKind::Blame => self.blame,
            GitOpTraceKind::RefEnumerate => self.ref_enumerate,
        }
    }
}

pub struct GitOpTraceScope {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    kind: GitOpTraceKind,
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    started_at: Option<Instant>,
}

impl Drop for GitOpTraceScope {
    fn drop(&mut self) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        {
            let Some(started_at) = self.started_at.take() else {
                return;
            };
            let elapsed_nanos = started_at.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
            record_ns(self.kind, elapsed_nanos);
        }
    }
}

#[inline]
pub fn scope(kind: GitOpTraceKind) -> GitOpTraceScope {
    #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
    let _ = kind;

    GitOpTraceScope {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        kind,
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        started_at: is_enabled().then(Instant::now),
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct GitOpTraceCaptureGuard {
    previous_enabled: bool,
}

#[cfg(any(test, feature = "benchmarks"))]
impl Drop for GitOpTraceCaptureGuard {
    fn drop(&mut self) {
        clear();
        TRACE_STATE.with(|state| state.set_enabled(self.previous_enabled));
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn capture() -> GitOpTraceCaptureGuard {
    let previous_enabled = TRACE_STATE.with(|state| {
        let previous_enabled = state.enabled();
        state.set_enabled(true);
        state.clear();
        previous_enabled
    });
    GitOpTraceCaptureGuard { previous_enabled }
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn snapshot() -> GitOpTraceSnapshot {
    TRACE_STATE.with(ThreadGitOpTraceState::snapshot)
}

#[cfg(any(test, feature = "benchmarks"))]
pub fn clear() {
    TRACE_STATE.with(ThreadGitOpTraceState::clear);
}

#[inline]
#[cfg(any(debug_assertions, feature = "benchmarks"))]
pub fn is_enabled() -> bool {
    TRACE_STATE.with(ThreadGitOpTraceState::enabled)
}

#[inline]
#[cfg(not(any(debug_assertions, feature = "benchmarks")))]
pub fn is_enabled() -> bool {
    false
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
#[derive(Debug)]
struct ThreadGitOpTraceStats {
    calls: Cell<u64>,
    total_nanos: Cell<u64>,
    last_nanos: Cell<u64>,
    max_nanos: Cell<u64>,
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
impl ThreadGitOpTraceStats {
    const fn new() -> Self {
        Self {
            calls: Cell::new(0),
            total_nanos: Cell::new(0),
            last_nanos: Cell::new(0),
            max_nanos: Cell::new(0),
        }
    }

    fn record_ns(&self, elapsed_nanos: u64) {
        self.calls.set(self.calls.get().saturating_add(1));
        self.total_nanos
            .set(self.total_nanos.get().saturating_add(elapsed_nanos));
        self.last_nanos.set(elapsed_nanos);
        self.max_nanos.set(self.max_nanos.get().max(elapsed_nanos));
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn snapshot(&self) -> GitOpTraceStats {
        GitOpTraceStats {
            calls: self.calls.get(),
            total_nanos: self.total_nanos.get(),
            last_nanos: self.last_nanos.get(),
            max_nanos: self.max_nanos.get(),
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn clear(&self) {
        self.calls.set(0);
        self.total_nanos.set(0);
        self.last_nanos.set(0);
        self.max_nanos.set(0);
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
#[derive(Debug)]
struct ThreadGitOpTraceState {
    enabled: Cell<bool>,
    status: ThreadGitOpTraceStats,
    log_walk: ThreadGitOpTraceStats,
    diff: ThreadGitOpTraceStats,
    blame: ThreadGitOpTraceStats,
    ref_enumerate: ThreadGitOpTraceStats,
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
impl ThreadGitOpTraceState {
    fn new() -> Self {
        Self {
            enabled: Cell::new(false),
            status: ThreadGitOpTraceStats::new(),
            log_walk: ThreadGitOpTraceStats::new(),
            diff: ThreadGitOpTraceStats::new(),
            blame: ThreadGitOpTraceStats::new(),
            ref_enumerate: ThreadGitOpTraceStats::new(),
        }
    }

    fn enabled(&self) -> bool {
        self.enabled.get()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn set_enabled(&self, enabled: bool) {
        self.enabled.set(enabled);
    }

    fn stats(&self, kind: GitOpTraceKind) -> &ThreadGitOpTraceStats {
        match kind {
            GitOpTraceKind::Status => &self.status,
            GitOpTraceKind::LogWalk => &self.log_walk,
            GitOpTraceKind::Diff => &self.diff,
            GitOpTraceKind::Blame => &self.blame,
            GitOpTraceKind::RefEnumerate => &self.ref_enumerate,
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn snapshot(&self) -> GitOpTraceSnapshot {
        GitOpTraceSnapshot {
            status: self.status.snapshot(),
            log_walk: self.log_walk.snapshot(),
            diff: self.diff.snapshot(),
            blame: self.blame.snapshot(),
            ref_enumerate: self.ref_enumerate.snapshot(),
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn clear(&self) {
        self.status.clear();
        self.log_walk.clear();
        self.diff.clear();
        self.blame.clear();
        self.ref_enumerate.clear();
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
thread_local! {
    // Capture state is per-thread so concurrent fixture tests do not observe
    // each other's synthetic git operations.
    static TRACE_STATE: ThreadGitOpTraceState = ThreadGitOpTraceState::new();
}

#[inline]
#[cfg(any(debug_assertions, feature = "benchmarks"))]
fn record_ns(kind: GitOpTraceKind, elapsed_nanos: u64) {
    TRACE_STATE.with(|state| state.stats(kind).record_ns(elapsed_nanos));
}

#[inline]
#[cfg(any(test, feature = "benchmarks"))]
fn nanos_to_millis(value: u64) -> f64 {
    value as f64 / 1_000_000.0
}

#[cfg(all(test, any(debug_assertions, feature = "benchmarks")))]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};

    #[test]
    fn scope_records_only_when_capture_is_enabled() {
        clear();
        {
            let _scope = scope(GitOpTraceKind::Status);
        }
        assert_eq!(snapshot().status, GitOpTraceStats::default());

        let _capture = capture();
        {
            let _scope = scope(GitOpTraceKind::Status);
        }
        let snapshot = snapshot();
        assert_eq!(snapshot.status.calls, 1);
        assert!(snapshot.status.total_nanos > 0);
        assert_eq!(snapshot.log_walk, GitOpTraceStats::default());
    }

    #[test]
    fn capture_guard_clears_snapshot_on_drop() {
        {
            let _capture = capture();
            let _scope = scope(GitOpTraceKind::Diff);
        }
        assert_eq!(snapshot().diff, GitOpTraceStats::default());
    }

    #[test]
    fn snapshot_stats_route_by_kind() {
        let _capture = capture();
        {
            let _scope = scope(GitOpTraceKind::RefEnumerate);
        }
        let snapshot = snapshot();
        assert_eq!(snapshot.stats(GitOpTraceKind::RefEnumerate).calls, 1);
        assert_eq!(snapshot.stats(GitOpTraceKind::Status).calls, 0);
    }

    #[test]
    fn sidecar_metric_keys_stay_stable() {
        let keys = GitOpTraceKind::ALL.map(GitOpTraceKind::sidecar_metric_key);
        assert_eq!(
            keys,
            [
                "status_ms",
                "log_walk_ms",
                "diff_ms",
                "blame_ms",
                "ref_enumerate_ms",
            ]
        );
    }

    #[test]
    fn millis_helpers_convert_nanoseconds() {
        let stats = GitOpTraceStats {
            calls: 1,
            total_nanos: 2_500_000,
            last_nanos: 1_250_000,
            max_nanos: 3_750_000,
        };
        assert!((stats.total_millis() - 2.5).abs() < f64::EPSILON);
        assert!((stats.last_millis() - 1.25).abs() < f64::EPSILON);
        assert!((stats.max_millis() - 3.75).abs() < f64::EPSILON);
    }

    #[test]
    fn concurrent_captures_are_isolated_per_thread() {
        let ready = Arc::new(Barrier::new(2));
        let done = Arc::new(Barrier::new(2));
        let ready_thread = ready.clone();
        let done_thread = done.clone();

        let handle = std::thread::spawn(move || {
            let _capture = capture();
            ready_thread.wait();
            {
                let _scope = scope(GitOpTraceKind::Status);
            }
            done_thread.wait();

            let snapshot = snapshot();
            assert_eq!(snapshot.status.calls, 1);
            assert_eq!(snapshot.diff.calls, 0);
        });

        let _capture = capture();
        ready.wait();
        {
            let _scope = scope(GitOpTraceKind::Diff);
        }
        done.wait();

        let snapshot = snapshot();
        assert_eq!(snapshot.diff.calls, 1);
        assert_eq!(snapshot.status.calls, 0);

        handle.join().expect("join git_ops_trace test thread");
    }
}
