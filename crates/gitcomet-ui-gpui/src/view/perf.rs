#![cfg_attr(any(test, feature = "benchmarks"), allow(dead_code))]

#[cfg(any(test, feature = "benchmarks"))]
use smallvec::SmallVec;
#[cfg(any(test, feature = "benchmarks"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(test, feature = "benchmarks"))]
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewPerfSpan {
    RenderThreeWayRows,
    RenderResolverDiffRows,
    RenderResolvedPreviewRows,
    RenderConflictResolverPane,
    ResolvedOutputEditObserve,
    RecomputeResolvedOutline,
    StyledTextBuild,
    SyntaxHighlighting,
    WordQueryHighlighting,
    MarkdownPreviewParse,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewPerfRenderLane {
    ResolvedPreview,
    MarkdownPreview,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SpanStats {
    pub calls: u64,
    pub total_nanos: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RowBatchStats {
    pub calls: u64,
    pub requested_rows: u64,
    pub painted_rows: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ViewPerfSnapshot {
    pub render_resolved_preview_rows_batch: RowBatchStats,
    pub markdown_preview_rows_batch: RowBatchStats,
    pub render_three_way_rows: SpanStats,
    pub render_resolver_diff_rows: SpanStats,
    pub render_resolved_preview_rows: SpanStats,
    pub render_conflict_resolver_pane: SpanStats,
    pub resolved_output_edit_observe: SpanStats,
    pub recompute_resolved_outline: SpanStats,
    pub styled_text_build: SpanStats,
    pub syntax_highlighting: SpanStats,
    pub word_query_highlighting: SpanStats,
    pub markdown_preview_parse: SpanStats,
}

pub(crate) struct PerfScope {
    #[cfg(any(test, feature = "benchmarks"))]
    span: ViewPerfSpan,
    #[cfg(any(test, feature = "benchmarks"))]
    started_at: Instant,
}

impl Drop for PerfScope {
    fn drop(&mut self) {
        #[cfg(any(test, feature = "benchmarks"))]
        {
            let elapsed_nanos = self
                .started_at
                .elapsed()
                .as_nanos()
                .min(u128::from(u64::MAX));
            span_stats(self.span).record_ns(elapsed_nanos as u64);
        }
    }
}

#[inline]
pub(crate) fn span(span: ViewPerfSpan) -> PerfScope {
    #[cfg(not(any(test, feature = "benchmarks")))]
    let _ = span;

    PerfScope {
        #[cfg(any(test, feature = "benchmarks"))]
        span,
        #[cfg(any(test, feature = "benchmarks"))]
        started_at: Instant::now(),
    }
}

#[inline]
pub(crate) fn record_row_batch(
    lane: ViewPerfRenderLane,
    requested_rows: usize,
    painted_rows: usize,
) {
    #[cfg(any(test, feature = "benchmarks"))]
    {
        row_stats(lane).record(
            saturating_usize_to_u64(requested_rows),
            saturating_usize_to_u64(painted_rows),
        );
    }
    #[cfg(not(any(test, feature = "benchmarks")))]
    let _ = (lane, requested_rows, painted_rows);
}

#[cfg(any(test, feature = "benchmarks"))]
#[inline]
pub(crate) fn snapshot() -> ViewPerfSnapshot {
    ViewPerfSnapshot {
        render_resolved_preview_rows_batch: RENDER_RESOLVED_PREVIEW_ROWS_BATCH.snapshot(),
        markdown_preview_rows_batch: MARKDOWN_PREVIEW_ROWS_BATCH.snapshot(),
        render_three_way_rows: RENDER_THREE_WAY_ROWS_SPAN.snapshot(),
        render_resolver_diff_rows: RENDER_RESOLVER_DIFF_ROWS_SPAN.snapshot(),
        render_resolved_preview_rows: RENDER_RESOLVED_PREVIEW_ROWS_SPAN.snapshot(),
        render_conflict_resolver_pane: RENDER_CONFLICT_RESOLVER_PANE_SPAN.snapshot(),
        resolved_output_edit_observe: RESOLVED_OUTPUT_EDIT_OBSERVE_SPAN.snapshot(),
        recompute_resolved_outline: RECOMPUTE_RESOLVED_OUTLINE_SPAN.snapshot(),
        styled_text_build: STYLED_TEXT_BUILD_SPAN.snapshot(),
        syntax_highlighting: SYNTAX_HIGHLIGHTING_SPAN.snapshot(),
        word_query_highlighting: WORD_QUERY_HIGHLIGHTING_SPAN.snapshot(),
        markdown_preview_parse: MARKDOWN_PREVIEW_PARSE_SPAN.snapshot(),
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[inline]
pub(crate) fn reset() {
    RENDER_RESOLVED_PREVIEW_ROWS_BATCH.reset();
    MARKDOWN_PREVIEW_ROWS_BATCH.reset();

    RENDER_THREE_WAY_ROWS_SPAN.reset();
    RENDER_RESOLVER_DIFF_ROWS_SPAN.reset();
    RENDER_RESOLVED_PREVIEW_ROWS_SPAN.reset();
    RENDER_CONFLICT_RESOLVER_PANE_SPAN.reset();
    RESOLVED_OUTPUT_EDIT_OBSERVE_SPAN.reset();
    RECOMPUTE_RESOLVED_OUTLINE_SPAN.reset();
    STYLED_TEXT_BUILD_SPAN.reset();
    SYNTAX_HIGHLIGHTING_SPAN.reset();
    WORD_QUERY_HIGHLIGHTING_SPAN.reset();
    MARKDOWN_PREVIEW_PARSE_SPAN.reset();
}

#[inline]
#[cfg(any(test, feature = "benchmarks"))]
fn saturating_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Debug)]
struct AtomicSpanStats {
    calls: AtomicU64,
    total_nanos: AtomicU64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl AtomicSpanStats {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            total_nanos: AtomicU64::new(0),
        }
    }

    #[inline]
    fn record_ns(&self, elapsed_nanos: u64) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.total_nanos.fetch_add(elapsed_nanos, Ordering::Relaxed);
    }

    #[inline]
    fn snapshot(&self) -> SpanStats {
        SpanStats {
            calls: self.calls.load(Ordering::Relaxed),
            total_nanos: self.total_nanos.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.total_nanos.store(0, Ordering::Relaxed);
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Debug)]
struct AtomicRowBatchStats {
    calls: AtomicU64,
    requested_rows: AtomicU64,
    painted_rows: AtomicU64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl AtomicRowBatchStats {
    const fn new() -> Self {
        Self {
            calls: AtomicU64::new(0),
            requested_rows: AtomicU64::new(0),
            painted_rows: AtomicU64::new(0),
        }
    }

    #[inline]
    fn record(&self, requested_rows: u64, painted_rows: u64) {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.requested_rows
            .fetch_add(requested_rows, Ordering::Relaxed);
        self.painted_rows.fetch_add(painted_rows, Ordering::Relaxed);
    }

    #[inline]
    fn snapshot(&self) -> RowBatchStats {
        RowBatchStats {
            calls: self.calls.load(Ordering::Relaxed),
            requested_rows: self.requested_rows.load(Ordering::Relaxed),
            painted_rows: self.painted_rows.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.requested_rows.store(0, Ordering::Relaxed);
        self.painted_rows.store(0, Ordering::Relaxed);
    }
}

#[cfg(any(test, feature = "benchmarks"))]
static RENDER_THREE_WAY_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static RENDER_RESOLVER_DIFF_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static RENDER_RESOLVED_PREVIEW_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static RENDER_CONFLICT_RESOLVER_PANE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static RESOLVED_OUTPUT_EDIT_OBSERVE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static RECOMPUTE_RESOLVED_OUTLINE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static STYLED_TEXT_BUILD_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static SYNTAX_HIGHLIGHTING_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static WORD_QUERY_HIGHLIGHTING_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static MARKDOWN_PREVIEW_PARSE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static RENDER_RESOLVED_PREVIEW_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();
#[cfg(any(test, feature = "benchmarks"))]
static MARKDOWN_PREVIEW_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();

#[inline]
#[cfg(any(test, feature = "benchmarks"))]
fn span_stats(span: ViewPerfSpan) -> &'static AtomicSpanStats {
    match span {
        ViewPerfSpan::RenderThreeWayRows => &RENDER_THREE_WAY_ROWS_SPAN,
        ViewPerfSpan::RenderResolverDiffRows => &RENDER_RESOLVER_DIFF_ROWS_SPAN,
        ViewPerfSpan::RenderResolvedPreviewRows => &RENDER_RESOLVED_PREVIEW_ROWS_SPAN,
        ViewPerfSpan::RenderConflictResolverPane => &RENDER_CONFLICT_RESOLVER_PANE_SPAN,
        ViewPerfSpan::ResolvedOutputEditObserve => &RESOLVED_OUTPUT_EDIT_OBSERVE_SPAN,
        ViewPerfSpan::RecomputeResolvedOutline => &RECOMPUTE_RESOLVED_OUTLINE_SPAN,
        ViewPerfSpan::StyledTextBuild => &STYLED_TEXT_BUILD_SPAN,
        ViewPerfSpan::SyntaxHighlighting => &SYNTAX_HIGHLIGHTING_SPAN,
        ViewPerfSpan::WordQueryHighlighting => &WORD_QUERY_HIGHLIGHTING_SPAN,
        ViewPerfSpan::MarkdownPreviewParse => &MARKDOWN_PREVIEW_PARSE_SPAN,
    }
}

#[inline]
#[cfg(any(test, feature = "benchmarks"))]
fn row_stats(lane: ViewPerfRenderLane) -> &'static AtomicRowBatchStats {
    match lane {
        ViewPerfRenderLane::ResolvedPreview => &RENDER_RESOLVED_PREVIEW_ROWS_BATCH,
        ViewPerfRenderLane::MarkdownPreview => &MARKDOWN_PREVIEW_ROWS_BATCH,
    }
}

// ---------------------------------------------------------------------------
// Frame timing capture for the benchmarks
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
type FrameTimingDurations = SmallVec<[u64; 256]>;

/// Captures per-frame durations during a benchmark interaction loop and
/// computes percentile statistics, dropped-frame counts, and budget violation
/// rates.  Results can be emitted as sidecar JSON via [`FrameTimingStats::to_sidecar_metrics`].
#[cfg(any(test, feature = "benchmarks"))]
pub struct FrameTimingCapture {
    frame_durations_ns: FrameTimingDurations,
    frame_budget_ns: u64,
    #[cfg(feature = "benchmarks")]
    capture_start: Instant,
}

#[cfg(any(test, feature = "benchmarks"))]
impl FrameTimingCapture {
    #[cfg(feature = "benchmarks")]
    /// 60 fps ≈ 16.667 ms per frame.
    pub const DEFAULT_FRAME_BUDGET_NS: u64 = 16_666_667;

    pub fn new(frame_budget_ns: u64) -> Self {
        Self::with_expected_frames(frame_budget_ns, 0)
    }

    pub fn with_expected_frames(frame_budget_ns: u64, expected_frames: usize) -> Self {
        Self {
            frame_durations_ns: FrameTimingDurations::with_capacity(expected_frames),
            frame_budget_ns,
            #[cfg(feature = "benchmarks")]
            capture_start: Instant::now(),
        }
    }

    #[cfg(feature = "benchmarks")]
    pub fn with_default_budget() -> Self {
        Self::new(Self::DEFAULT_FRAME_BUDGET_NS)
    }

    /// Record one frame's duration in nanoseconds.
    #[inline]
    pub fn record_frame_ns(&mut self, duration_ns: u64) {
        self.frame_durations_ns.push(duration_ns);
    }

    /// Record one frame's duration.
    #[inline]
    pub fn record_frame(&mut self, duration: std::time::Duration) {
        self.record_frame_ns(duration.as_nanos().min(u128::from(u64::MAX)) as u64);
    }

    #[cfg(feature = "benchmarks")]
    /// Consume the capture and compute statistics.  Total capture wall time is
    /// derived from the [`Instant`] recorded at construction.
    pub fn finish(self) -> FrameTimingStats {
        let total_capture_ns = self
            .capture_start
            .elapsed()
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        compute_frame_timing_stats(
            self.frame_durations_ns,
            self.frame_budget_ns,
            total_capture_ns,
        )
    }

    /// Finish with an explicit total duration (useful for deterministic tests).
    pub fn finish_with_duration(self, total_capture_ns: u64) -> FrameTimingStats {
        compute_frame_timing_stats(
            self.frame_durations_ns,
            self.frame_budget_ns,
            total_capture_ns,
        )
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug, PartialEq)]
pub struct FrameTimingStats {
    pub frame_count: usize,
    pub p50_frame_ns: u64,
    pub p95_frame_ns: u64,
    pub p99_frame_ns: u64,
    pub max_frame_ns: u64,
    pub dropped_frames: usize,
    pub budget_violations_per_sec: f64,
    pub total_capture_ns: u64,
    pub frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl FrameTimingStats {
    /// Convert to a sidecar metrics map matching the shape described in perf.md:
    ///
    /// ```json
    /// {
    ///   "frame_count": 120,
    ///   "p50_frame_ms": 4.2,
    ///   "p95_frame_ms": 8.1,
    ///   "p99_frame_ms": 12.3,
    ///   "max_frame_ms": 18.0,
    ///   "dropped_frames": 2,
    ///   "budget_violations_per_sec": 0.3,
    ///   "frame_budget_ms": 16.667
    /// }
    /// ```
    pub fn to_sidecar_metrics(&self) -> serde_json::Map<String, serde_json::Value> {
        use serde_json::json;
        let ns_to_ms = |ns: u64| ns as f64 / 1_000_000.0;
        let mut m = serde_json::Map::new();
        m.insert("frame_count".to_string(), json!(self.frame_count));
        m.insert(
            "p50_frame_ms".to_string(),
            json!(ns_to_ms(self.p50_frame_ns)),
        );
        m.insert(
            "p95_frame_ms".to_string(),
            json!(ns_to_ms(self.p95_frame_ns)),
        );
        m.insert(
            "p99_frame_ms".to_string(),
            json!(ns_to_ms(self.p99_frame_ns)),
        );
        m.insert(
            "max_frame_ms".to_string(),
            json!(ns_to_ms(self.max_frame_ns)),
        );
        m.insert("dropped_frames".to_string(), json!(self.dropped_frames));
        m.insert(
            "budget_violations_per_sec".to_string(),
            json!(self.budget_violations_per_sec),
        );
        m.insert(
            "frame_budget_ms".to_string(),
            json!(ns_to_ms(self.frame_budget_ns)),
        );
        m
    }

    /// Returns `true` when P99 exceeds 2× the frame budget – the alert
    /// threshold recommended by perf.md.
    pub fn p99_exceeds_2x_budget(&self) -> bool {
        self.p99_frame_ns > self.frame_budget_ns.saturating_mul(2)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn compute_frame_timing_stats(
    mut frame_durations_ns: FrameTimingDurations,
    frame_budget_ns: u64,
    total_capture_ns: u64,
) -> FrameTimingStats {
    if frame_durations_ns.is_empty() {
        return FrameTimingStats {
            frame_count: 0,
            p50_frame_ns: 0,
            p95_frame_ns: 0,
            p99_frame_ns: 0,
            max_frame_ns: 0,
            dropped_frames: 0,
            budget_violations_per_sec: 0.0,
            total_capture_ns,
            frame_budget_ns,
        };
    }

    frame_durations_ns.sort_unstable();

    let frame_count = frame_durations_ns.len();
    let p50_frame_ns = percentile_nearest_rank(&frame_durations_ns, 50);
    let p95_frame_ns = percentile_nearest_rank(&frame_durations_ns, 95);
    let p99_frame_ns = percentile_nearest_rank(&frame_durations_ns, 99);
    let max_frame_ns = frame_durations_ns[frame_count - 1];

    let dropped_frames = frame_durations_ns
        .iter()
        .filter(|&&d| d > frame_budget_ns)
        .count();

    let total_seconds = total_capture_ns as f64 / 1_000_000_000.0;
    let budget_violations_per_sec = if total_seconds > 0.0 {
        dropped_frames as f64 / total_seconds
    } else {
        0.0
    };

    FrameTimingStats {
        frame_count,
        p50_frame_ns,
        p95_frame_ns,
        p99_frame_ns,
        max_frame_ns,
        dropped_frames,
        budget_violations_per_sec,
        total_capture_ns,
        frame_budget_ns,
    }
}

/// Nearest-rank percentile over a **sorted** slice.
#[cfg(any(test, feature = "benchmarks"))]
fn percentile_nearest_rank(sorted: &[u64], pct: usize) -> u64 {
    debug_assert!(!sorted.is_empty());
    debug_assert!(pct <= 100);
    let rank = ((pct as f64 / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(sorted.len() - 1)]
}

#[cfg(any(test, feature = "benchmarks"))]
thread_local! {
    static EXTENSION_DISPATCH_CALLS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SETTINGS_PAGES_RENDERED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    static SETTINGS_RENDERS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Count calls into extension code; compiled away in normal builds.
#[inline]
pub(in crate::view) fn extension_dispatch() {
    #[cfg(any(test, feature = "benchmarks"))]
    EXTENSION_DISPATCH_CALLS.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn take_extension_dispatch_calls() -> u64 {
    EXTENSION_DISPATCH_CALLS.with(|calls| calls.replace(0))
}

#[inline]
pub(in crate::view) fn settings_page_rendered() {
    #[cfg(any(test, feature = "benchmarks"))]
    SETTINGS_PAGES_RENDERED.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn take_settings_pages_rendered() -> u64 {
    SETTINGS_PAGES_RENDERED.with(|calls| calls.replace(0))
}

#[inline]
pub(in crate::view) fn settings_rendered() {
    #[cfg(any(test, feature = "benchmarks"))]
    SETTINGS_RENDERS.with(|calls| calls.set(calls.get() + 1));
}

#[cfg(any(test, feature = "benchmarks"))]
pub(in crate::view) fn take_settings_renders() -> u64 {
    SETTINGS_RENDERS.with(|calls| calls.replace(0))
}

#[cfg(test)]
mod tests;
