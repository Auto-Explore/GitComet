#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::sync::LazyLock;
#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
use std::sync::{Arc, Mutex};
use std::time::Duration;

static MERGETOOL_TRACE_LOGGING_ENABLED: LazyLock<bool> = LazyLock::new(|| {
    std::env::var_os("GITCOMET_TRACE_MERGETOOL_BOOTSTRAP").is_some_and(|value| value != "0")
});

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergetoolTraceStage {
    LoadConflictSession,
    LoadConflictFileStages,
    LoadCurrentReuse,
    LoadCurrentRead,
    ParseConflictMarkers,
    GenerateResolvedText,
    SideBySideRows,
    // Dead variant: never constructed in production. Matched only in test assertions.
    #[cfg(any(test, feature = "test-support"))]
    BuildInlineRows,
    BuildThreeWayConflictMaps,
    ComputeThreeWayWordHighlights,
    ComputeTwoWayWordHighlights,
    ConflictResolverInputSetText,
    ResolvedOutlineRecompute,
    ConflictResolverBootstrapTotal,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MergetoolTraceRenderingMode {
    EagerSmallFile,
    StreamedLargeFile,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MergetoolTraceSideStats {
    pub bytes: Option<usize>,
    pub lines: Option<usize>,
}

impl MergetoolTraceSideStats {
    pub fn from_text(text: Option<&str>) -> Self {
        Self {
            bytes: text.map(str::len),
            lines: text.map(text_line_count),
        }
    }

    pub fn from_bytes_and_text(bytes: Option<&[u8]>, text: Option<&str>) -> Self {
        Self {
            bytes: bytes.map(<[u8]>::len).or_else(|| text.map(str::len)),
            lines: text.map(text_line_count),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MergetoolTraceEvent {
    pub stage: MergetoolTraceStage,
    pub path: Option<PathBuf>,
    pub elapsed: Duration,
    pub rss_kib: Option<u64>,
    pub rendering_mode: Option<MergetoolTraceRenderingMode>,
    pub base: MergetoolTraceSideStats,
    pub ours: MergetoolTraceSideStats,
    pub theirs: MergetoolTraceSideStats,
    pub current: MergetoolTraceSideStats,
    pub whole_block_diff_ran: Option<bool>,
    pub full_output_generated: Option<bool>,
    pub full_syntax_parse_requested: Option<bool>,
    pub diff_row_count: Option<usize>,
    pub inline_row_count: Option<usize>,
    pub conflict_block_count: Option<usize>,
    pub resolved_output_line_count: Option<usize>,
}

impl MergetoolTraceEvent {
    pub fn new(stage: MergetoolTraceStage, path: Option<PathBuf>, elapsed: Duration) -> Self {
        Self {
            stage,
            path,
            elapsed,
            rss_kib: current_rss_kib(),
            rendering_mode: None,
            base: MergetoolTraceSideStats::default(),
            ours: MergetoolTraceSideStats::default(),
            theirs: MergetoolTraceSideStats::default(),
            current: MergetoolTraceSideStats::default(),
            whole_block_diff_ran: None,
            full_output_generated: None,
            full_syntax_parse_requested: None,
            diff_row_count: None,
            inline_row_count: None,
            conflict_block_count: None,
            resolved_output_line_count: None,
        }
    }

    pub fn with_rendering_mode(mut self, mode: Option<MergetoolTraceRenderingMode>) -> Self {
        self.rendering_mode = mode;
        self
    }

    pub fn with_base(mut self, stats: MergetoolTraceSideStats) -> Self {
        self.base = stats;
        self
    }

    pub fn with_ours(mut self, stats: MergetoolTraceSideStats) -> Self {
        self.ours = stats;
        self
    }

    pub fn with_theirs(mut self, stats: MergetoolTraceSideStats) -> Self {
        self.theirs = stats;
        self
    }

    pub fn with_current(mut self, stats: MergetoolTraceSideStats) -> Self {
        self.current = stats;
        self
    }

    pub fn with_whole_block_diff_ran(mut self, ran: Option<bool>) -> Self {
        self.whole_block_diff_ran = ran;
        self
    }

    pub fn with_full_output_generated(mut self, generated: Option<bool>) -> Self {
        self.full_output_generated = generated;
        self
    }

    pub fn with_full_syntax_parse_requested(mut self, requested: Option<bool>) -> Self {
        self.full_syntax_parse_requested = requested;
        self
    }

    pub fn with_diff_row_count(mut self, count: Option<usize>) -> Self {
        self.diff_row_count = count;
        self
    }

    pub fn with_inline_row_count(mut self, count: Option<usize>) -> Self {
        self.inline_row_count = count;
        self
    }

    pub fn with_conflict_block_count(mut self, count: Option<usize>) -> Self {
        self.conflict_block_count = count;
        self
    }

    pub fn with_resolved_output_line_count(mut self, count: Option<usize>) -> Self {
        self.resolved_output_line_count = count;
        self
    }
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MergetoolTraceSnapshot {
    pub events: Vec<MergetoolTraceEvent>,
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
#[derive(Clone, Debug)]
struct MergetoolTraceCaptureSink {
    events: Arc<Mutex<Vec<MergetoolTraceEvent>>>,
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
impl Default for MergetoolTraceCaptureSink {
    fn default() -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
impl MergetoolTraceCaptureSink {
    fn push(&self, event: MergetoolTraceEvent) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(event);
    }

    fn snapshot(&self) -> Vec<MergetoolTraceEvent> {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn clear(&self) {
        self.events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
#[derive(Clone, Debug)]
pub struct MergetoolTraceCaptureContext {
    sink: MergetoolTraceCaptureSink,
}

#[cfg(not(any(test, feature = "test-support", feature = "benchmarks")))]
#[derive(Clone, Debug, Default)]
pub struct MergetoolTraceCaptureContext;

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub struct MergetoolTraceCaptureGuard {
    previous_enabled: bool,
    previous_sink: Option<MergetoolTraceCaptureSink>,
    installed_sink: Option<MergetoolTraceCaptureSink>,
    clear_installed_sink_on_drop: bool,
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
impl Drop for MergetoolTraceCaptureGuard {
    fn drop(&mut self) {
        if self.clear_installed_sink_on_drop
            && let Some(sink) = self.installed_sink.as_ref()
        {
            sink.clear();
        }
        let previous_sink = self.previous_sink.take();
        MERGETOOL_TRACE_STATE
            .with(|state| state.restore_capture(self.previous_enabled, previous_sink));
    }
}

// Test/benchmark-only: installs a capture guard that collects trace events for assertion.
#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub fn capture() -> MergetoolTraceCaptureGuard {
    let sink = MergetoolTraceCaptureSink::default();
    let (previous_enabled, previous_sink) =
        MERGETOOL_TRACE_STATE.with(|state| state.replace_capture(true, Some(sink.clone())));
    MergetoolTraceCaptureGuard {
        previous_enabled,
        previous_sink,
        installed_sink: Some(sink),
        clear_installed_sink_on_drop: true,
    }
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub fn current_capture_context() -> Option<MergetoolTraceCaptureContext> {
    MERGETOOL_TRACE_STATE.with(|state| {
        state
            .capture_enabled()
            .then(|| state.capture_sink())
            .flatten()
            .map(|sink| MergetoolTraceCaptureContext { sink })
    })
}

#[cfg(not(any(test, feature = "test-support", feature = "benchmarks")))]
pub fn current_capture_context() -> Option<MergetoolTraceCaptureContext> {
    None
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub fn attach_capture(context: &MergetoolTraceCaptureContext) -> MergetoolTraceCaptureGuard {
    let sink = context.sink.clone();
    let (previous_enabled, previous_sink) =
        MERGETOOL_TRACE_STATE.with(|state| state.replace_capture(true, Some(sink.clone())));
    MergetoolTraceCaptureGuard {
        previous_enabled,
        previous_sink,
        installed_sink: Some(sink),
        clear_installed_sink_on_drop: false,
    }
}

#[cfg(not(any(test, feature = "test-support", feature = "benchmarks")))]
pub fn attach_capture(_context: &MergetoolTraceCaptureContext) -> MergetoolTraceCaptureGuard {
    MergetoolTraceCaptureGuard
}

pub fn record(event: MergetoolTraceEvent) {
    let capture_enabled = current_thread_capture_enabled();
    let logging_enabled = *MERGETOOL_TRACE_LOGGING_ENABLED;
    if !capture_enabled && !logging_enabled {
        return;
    }

    if logging_enabled {
        eprintln!("{}", format_event(&event));
    }

    #[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
    if capture_enabled {
        MERGETOOL_TRACE_STATE.with(|state| state.push(event));
    }
}

/// Like [`record`], but defers event construction until tracing is enabled.
/// Use this to avoid allocations and O(n) text scans when tracing is off.
pub fn record_with(f: impl FnOnce() -> MergetoolTraceEvent) {
    if !is_enabled() {
        return;
    }
    record(f());
}

// Used by tests and benches to observe trace events recorded during mergetool operations.
#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub fn snapshot() -> MergetoolTraceSnapshot {
    MERGETOOL_TRACE_STATE.with(|state| MergetoolTraceSnapshot {
        events: state.snapshot(),
    })
}

// Used internally by `capture()` when trace capture is enabled for tests or benches.
#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
pub fn clear() {
    MERGETOOL_TRACE_STATE.with(MergetoolTraceThreadState::clear);
}

pub fn is_enabled() -> bool {
    current_thread_capture_enabled() || *MERGETOOL_TRACE_LOGGING_ENABLED
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
struct MergetoolTraceThreadState {
    capture_enabled: Cell<bool>,
    capture_sink: RefCell<Option<MergetoolTraceCaptureSink>>,
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
impl MergetoolTraceThreadState {
    fn new() -> Self {
        Self {
            capture_enabled: Cell::new(false),
            capture_sink: RefCell::new(None),
        }
    }

    fn capture_enabled(&self) -> bool {
        self.capture_enabled.get()
    }

    fn capture_sink(&self) -> Option<MergetoolTraceCaptureSink> {
        self.capture_sink.borrow().clone()
    }

    fn replace_capture(
        &self,
        enabled: bool,
        sink: Option<MergetoolTraceCaptureSink>,
    ) -> (bool, Option<MergetoolTraceCaptureSink>) {
        let previous_enabled = self.capture_enabled.replace(enabled);
        let previous_sink = self.capture_sink.replace(sink);
        (previous_enabled, previous_sink)
    }

    fn restore_capture(&self, enabled: bool, sink: Option<MergetoolTraceCaptureSink>) {
        self.capture_enabled.set(enabled);
        self.capture_sink.replace(sink);
    }

    fn push(&self, event: MergetoolTraceEvent) {
        if let Some(sink) = self.capture_sink() {
            sink.push(event);
        }
    }

    fn snapshot(&self) -> Vec<MergetoolTraceEvent> {
        self.capture_sink()
            .map_or_else(Vec::new, |sink| sink.snapshot())
    }

    fn clear(&self) {
        if let Some(sink) = self.capture_sink() {
            sink.clear();
        }
    }
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
thread_local! {
    // Captured trace buffers are per-thread so concurrent fixture tests do not
    // merge unrelated bootstrap events into the same snapshot.
    static MERGETOOL_TRACE_STATE: MergetoolTraceThreadState = MergetoolTraceThreadState::new();
}

#[cfg(any(test, feature = "test-support", feature = "benchmarks"))]
fn current_thread_capture_enabled() -> bool {
    MERGETOOL_TRACE_STATE.with(MergetoolTraceThreadState::capture_enabled)
}

#[cfg(not(any(test, feature = "test-support", feature = "benchmarks")))]
fn current_thread_capture_enabled() -> bool {
    false
}

#[cfg(not(any(test, feature = "test-support", feature = "benchmarks")))]
pub struct MergetoolTraceCaptureGuard;

fn text_line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.as_bytes()
            .iter()
            .filter(|&&byte| byte == b'\n')
            .count()
            + 1
    }
}

fn format_event(event: &MergetoolTraceEvent) -> String {
    let path = event
        .path
        .as_deref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "-".to_string());
    format!(
        "[mergetool-trace] stage={:?} path={path} elapsed_ms={:.3} rss_kib={:?} mode={:?} whole_block_diff={:?} full_output={:?} full_syntax={:?} base={:?} ours={:?} theirs={:?} current={:?} diff_rows={:?} inline_rows={:?} conflicts={:?} resolved_lines={:?}",
        event.stage,
        event.elapsed.as_secs_f64() * 1_000.0,
        event.rss_kib,
        event.rendering_mode,
        event.whole_block_diff_ran,
        event.full_output_generated,
        event.full_syntax_parse_requested,
        event.base,
        event.ours,
        event.theirs,
        event.current,
        event.diff_row_count,
        event.inline_row_count,
        event.conflict_block_count,
        event.resolved_output_line_count,
    )
}

#[cfg(all(any(debug_assertions, feature = "benchmarks"), target_os = "linux"))]
fn current_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })
}

#[cfg(not(all(any(debug_assertions, feature = "benchmarks"), target_os = "linux")))]
fn current_rss_kib() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::{Arc, Barrier};

    #[test]
    fn concurrent_captures_are_isolated_per_thread() {
        let ready = Arc::new(Barrier::new(2));
        let done = Arc::new(Barrier::new(2));
        let ready_thread = ready.clone();
        let done_thread = done.clone();

        let handle = std::thread::spawn(move || {
            let _capture = capture();
            ready_thread.wait();
            record(MergetoolTraceEvent::new(
                MergetoolTraceStage::LoadConflictSession,
                None,
                Duration::from_millis(1),
            ));
            done_thread.wait();

            let snapshot = snapshot();
            assert_eq!(snapshot.events.len(), 1);
            assert_eq!(
                snapshot.events[0].stage,
                MergetoolTraceStage::LoadConflictSession
            );
        });

        let _capture = capture();
        ready.wait();
        record(MergetoolTraceEvent::new(
            MergetoolTraceStage::GenerateResolvedText,
            None,
            Duration::from_millis(2),
        ));
        done.wait();

        let snapshot = snapshot();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(
            snapshot.events[0].stage,
            MergetoolTraceStage::GenerateResolvedText
        );

        handle.join().expect("join mergetool_trace test thread");
    }

    #[test]
    fn inherited_capture_context_collects_worker_thread_events() {
        let _capture = capture();
        let context = current_capture_context().expect("capture context should be available");

        let handle = std::thread::spawn(move || {
            let _attached = attach_capture(&context);
            record(MergetoolTraceEvent::new(
                MergetoolTraceStage::LoadCurrentReuse,
                None,
                Duration::from_millis(1),
            ));
        });

        handle.join().expect("join inherited capture thread");

        let snapshot = snapshot();
        assert_eq!(snapshot.events.len(), 1);
        assert_eq!(
            snapshot.events[0].stage,
            MergetoolTraceStage::LoadCurrentReuse
        );
    }

    #[test]
    fn record_with_skips_builder_when_capture_is_disabled() {
        let invoked = Cell::new(false);

        record_with(|| {
            invoked.set(true);
            MergetoolTraceEvent::new(
                MergetoolTraceStage::GenerateResolvedText,
                None,
                Duration::from_millis(1),
            )
        });

        assert!(
            !invoked.get(),
            "record_with should not construct events when tracing is disabled"
        );
    }

    #[test]
    fn nested_capture_restores_previous_sink() {
        clear();
        assert!(!is_enabled());

        let outer = capture();
        record(MergetoolTraceEvent::new(
            MergetoolTraceStage::LoadConflictSession,
            None,
            Duration::from_millis(1),
        ));
        assert_eq!(snapshot().events.len(), 1);

        {
            let _inner = capture();
            record(MergetoolTraceEvent::new(
                MergetoolTraceStage::GenerateResolvedText,
                None,
                Duration::from_millis(2),
            ));
            let inner_snapshot = snapshot();
            assert_eq!(inner_snapshot.events.len(), 1);
            assert_eq!(
                inner_snapshot.events[0].stage,
                MergetoolTraceStage::GenerateResolvedText
            );
        }

        assert!(is_enabled());
        let restored = snapshot();
        assert_eq!(restored.events.len(), 1);
        assert_eq!(
            restored.events[0].stage,
            MergetoolTraceStage::LoadConflictSession
        );

        record(MergetoolTraceEvent::new(
            MergetoolTraceStage::LoadCurrentRead,
            None,
            Duration::from_millis(3),
        ));
        let restored = snapshot();
        assert_eq!(restored.events.len(), 2);
        assert_eq!(
            restored.events[1].stage,
            MergetoolTraceStage::LoadCurrentRead
        );

        drop(outer);
        assert!(!is_enabled());
        assert!(snapshot().events.is_empty());
    }

    #[test]
    fn attach_capture_restores_previous_sink_after_drop() {
        let outer = capture();
        let outer_context = current_capture_context().expect("outer capture context");

        let inner = capture();
        record(MergetoolTraceEvent::new(
            MergetoolTraceStage::GenerateResolvedText,
            None,
            Duration::from_millis(1),
        ));

        {
            let _attached = attach_capture(&outer_context);
            record(MergetoolTraceEvent::new(
                MergetoolTraceStage::LoadCurrentReuse,
                None,
                Duration::from_millis(1),
            ));
        }

        let inner_snapshot = snapshot();
        assert_eq!(inner_snapshot.events.len(), 1);
        assert_eq!(
            inner_snapshot.events[0].stage,
            MergetoolTraceStage::GenerateResolvedText
        );

        drop(inner);

        let outer_snapshot = snapshot();
        assert_eq!(outer_snapshot.events.len(), 1);
        assert_eq!(
            outer_snapshot.events[0].stage,
            MergetoolTraceStage::LoadCurrentReuse
        );

        drop(outer);
    }
}
