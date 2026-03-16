use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

static MERGETOOL_TRACE_CAPTURE_ENABLED: AtomicBool = AtomicBool::new(false);
static MERGETOOL_TRACE_EVENTS: LazyLock<Mutex<Vec<MergetoolTraceEvent>>> =
    LazyLock::new(|| Mutex::new(Vec::new()));
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

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MergetoolTraceSnapshot {
    pub events: Vec<MergetoolTraceEvent>,
}

#[cfg(any(test, feature = "test-support"))]
pub struct MergetoolTraceCaptureGuard {
    previous_enabled: bool,
}

#[cfg(any(test, feature = "test-support"))]
impl Drop for MergetoolTraceCaptureGuard {
    fn drop(&mut self) {
        clear();
        MERGETOOL_TRACE_CAPTURE_ENABLED.store(self.previous_enabled, Ordering::Relaxed);
    }
}

// Test-only: installs a capture guard that collects trace events for assertion.
#[cfg(any(test, feature = "test-support"))]
pub fn capture() -> MergetoolTraceCaptureGuard {
    let previous_enabled = MERGETOOL_TRACE_CAPTURE_ENABLED.swap(true, Ordering::Relaxed);
    clear();
    MergetoolTraceCaptureGuard { previous_enabled }
}

pub fn record(event: MergetoolTraceEvent) {
    if !is_enabled() {
        return;
    }

    if *MERGETOOL_TRACE_LOGGING_ENABLED {
        eprintln!("{}", format_event(&event));
    }

    if let Ok(mut events) = MERGETOOL_TRACE_EVENTS.lock() {
        events.push(event);
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

// Used only by tests to observe trace events recorded during mergetool operations.
#[cfg(any(test, feature = "test-support"))]
pub fn snapshot() -> MergetoolTraceSnapshot {
    let events = MERGETOOL_TRACE_EVENTS
        .lock()
        .map(|events| events.clone())
        .unwrap_or_default();
    MergetoolTraceSnapshot { events }
}

// Used internally by `capture()` which is test-only.
#[cfg(any(test, feature = "test-support"))]
pub fn clear() {
    if let Ok(mut events) = MERGETOOL_TRACE_EVENTS.lock() {
        events.clear();
    }
}

pub fn is_enabled() -> bool {
    MERGETOOL_TRACE_CAPTURE_ENABLED.load(Ordering::Relaxed) || *MERGETOOL_TRACE_LOGGING_ENABLED
}

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

#[cfg(all(debug_assertions, target_os = "linux"))]
fn current_rss_kib() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?;
        value.split_whitespace().next()?.parse::<u64>().ok()
    })
}

#[cfg(not(all(debug_assertions, target_os = "linux")))]
fn current_rss_kib() -> Option<u64> {
    None
}
