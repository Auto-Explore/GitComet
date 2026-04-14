#![cfg_attr(any(test, feature = "benchmarks"), allow(dead_code))]

#[cfg(any(test, feature = "benchmarks"))]
use smallvec::SmallVec;
#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(any(debug_assertions, feature = "benchmarks"))]
use std::time::Instant;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewPerfSpan {
    RenderThreeWayRows,
    RenderResolverDiffRows,
    RenderResolvedPreviewRows,
    RecomputeResolvedOutline,
    StyledTextBuild,
    SyntaxHighlighting,
    WordQueryHighlighting,
    MarkdownPreviewParse,
    MarkdownPreviewStyledRowBuild,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ViewPerfRenderLane {
    ResolvedPreview,
    MarkdownPreview,
}

#[cfg(any(test, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SpanStats {
    pub calls: u64,
    pub total_nanos: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RowBatchStats {
    pub calls: u64,
    pub requested_rows: u64,
    pub painted_rows: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ViewPerfSnapshot {
    pub render_resolved_preview_rows_batch: RowBatchStats,
    pub markdown_preview_rows_batch: RowBatchStats,
    pub render_three_way_rows: SpanStats,
    pub render_resolver_diff_rows: SpanStats,
    pub render_resolved_preview_rows: SpanStats,
    pub recompute_resolved_outline: SpanStats,
    pub styled_text_build: SpanStats,
    pub syntax_highlighting: SpanStats,
    pub word_query_highlighting: SpanStats,
    pub markdown_preview_parse: SpanStats,
    pub markdown_preview_styled_row_build: SpanStats,
}

pub(crate) struct PerfScope {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    span: ViewPerfSpan,
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    started_at: Instant,
}

impl Drop for PerfScope {
    fn drop(&mut self) {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
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
    #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
    let _ = span;

    PerfScope {
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        span,
        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        started_at: Instant::now(),
    }
}

#[inline]
pub(crate) fn record_row_batch(
    lane: ViewPerfRenderLane,
    requested_rows: usize,
    painted_rows: usize,
) {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        row_stats(lane).record(
            saturating_usize_to_u64(requested_rows),
            saturating_usize_to_u64(painted_rows),
        );
    }
    #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
    let _ = (lane, requested_rows, painted_rows);
}

#[cfg(any(test, feature = "benchmarks"))]
#[inline]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
pub(crate) fn snapshot() -> ViewPerfSnapshot {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        ViewPerfSnapshot {
            render_resolved_preview_rows_batch: RENDER_RESOLVED_PREVIEW_ROWS_BATCH.snapshot(),
            markdown_preview_rows_batch: MARKDOWN_PREVIEW_ROWS_BATCH.snapshot(),
            render_three_way_rows: RENDER_THREE_WAY_ROWS_SPAN.snapshot(),
            render_resolver_diff_rows: RENDER_RESOLVER_DIFF_ROWS_SPAN.snapshot(),
            render_resolved_preview_rows: RENDER_RESOLVED_PREVIEW_ROWS_SPAN.snapshot(),
            recompute_resolved_outline: RECOMPUTE_RESOLVED_OUTLINE_SPAN.snapshot(),
            styled_text_build: STYLED_TEXT_BUILD_SPAN.snapshot(),
            syntax_highlighting: SYNTAX_HIGHLIGHTING_SPAN.snapshot(),
            word_query_highlighting: WORD_QUERY_HIGHLIGHTING_SPAN.snapshot(),
            markdown_preview_parse: MARKDOWN_PREVIEW_PARSE_SPAN.snapshot(),
            markdown_preview_styled_row_build: MARKDOWN_PREVIEW_STYLED_ROW_BUILD_SPAN.snapshot(),
        }
    }
    #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
    {
        ViewPerfSnapshot::default()
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[inline]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
pub(crate) fn reset() {
    #[cfg(any(debug_assertions, feature = "benchmarks"))]
    {
        RENDER_RESOLVED_PREVIEW_ROWS_BATCH.reset();
        MARKDOWN_PREVIEW_ROWS_BATCH.reset();

        RENDER_THREE_WAY_ROWS_SPAN.reset();
        RENDER_RESOLVER_DIFF_ROWS_SPAN.reset();
        RENDER_RESOLVED_PREVIEW_ROWS_SPAN.reset();
        RECOMPUTE_RESOLVED_OUTLINE_SPAN.reset();
        STYLED_TEXT_BUILD_SPAN.reset();
        SYNTAX_HIGHLIGHTING_SPAN.reset();
        WORD_QUERY_HIGHLIGHTING_SPAN.reset();
        MARKDOWN_PREVIEW_PARSE_SPAN.reset();
        MARKDOWN_PREVIEW_STYLED_ROW_BUILD_SPAN.reset();
    }
}

#[inline]
#[cfg(any(debug_assertions, feature = "benchmarks"))]
fn saturating_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
#[derive(Debug)]
struct AtomicSpanStats {
    calls: AtomicU64,
    total_nanos: AtomicU64,
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
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

    #[cfg(any(test, feature = "benchmarks"))]
    #[inline]
    fn snapshot(&self) -> SpanStats {
        SpanStats {
            calls: self.calls.load(Ordering::Relaxed),
            total_nanos: self.total_nanos.load(Ordering::Relaxed),
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[inline]
    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.total_nanos.store(0, Ordering::Relaxed);
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
#[derive(Debug)]
struct AtomicRowBatchStats {
    calls: AtomicU64,
    requested_rows: AtomicU64,
    painted_rows: AtomicU64,
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
#[cfg_attr(feature = "benchmarks", allow(dead_code))]
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

    #[cfg(any(test, feature = "benchmarks"))]
    #[inline]
    fn snapshot(&self) -> RowBatchStats {
        RowBatchStats {
            calls: self.calls.load(Ordering::Relaxed),
            requested_rows: self.requested_rows.load(Ordering::Relaxed),
            painted_rows: self.painted_rows.load(Ordering::Relaxed),
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    #[inline]
    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.requested_rows.store(0, Ordering::Relaxed);
        self.painted_rows.store(0, Ordering::Relaxed);
    }
}

#[cfg(any(debug_assertions, feature = "benchmarks"))]
static RENDER_THREE_WAY_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static RENDER_RESOLVER_DIFF_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static RENDER_RESOLVED_PREVIEW_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static RECOMPUTE_RESOLVED_OUTLINE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static STYLED_TEXT_BUILD_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static SYNTAX_HIGHLIGHTING_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static WORD_QUERY_HIGHLIGHTING_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static MARKDOWN_PREVIEW_PARSE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static MARKDOWN_PREVIEW_STYLED_ROW_BUILD_SPAN: AtomicSpanStats = AtomicSpanStats::new();

#[cfg(any(debug_assertions, feature = "benchmarks"))]
static RENDER_RESOLVED_PREVIEW_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();
#[cfg(any(debug_assertions, feature = "benchmarks"))]
static MARKDOWN_PREVIEW_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();

#[inline]
#[cfg(any(debug_assertions, feature = "benchmarks"))]
fn span_stats(span: ViewPerfSpan) -> &'static AtomicSpanStats {
    match span {
        ViewPerfSpan::RenderThreeWayRows => &RENDER_THREE_WAY_ROWS_SPAN,
        ViewPerfSpan::RenderResolverDiffRows => &RENDER_RESOLVER_DIFF_ROWS_SPAN,
        ViewPerfSpan::RenderResolvedPreviewRows => &RENDER_RESOLVED_PREVIEW_ROWS_SPAN,
        ViewPerfSpan::RecomputeResolvedOutline => &RECOMPUTE_RESOLVED_OUTLINE_SPAN,
        ViewPerfSpan::StyledTextBuild => &STYLED_TEXT_BUILD_SPAN,
        ViewPerfSpan::SyntaxHighlighting => &SYNTAX_HIGHLIGHTING_SPAN,
        ViewPerfSpan::WordQueryHighlighting => &WORD_QUERY_HIGHLIGHTING_SPAN,
        ViewPerfSpan::MarkdownPreviewParse => &MARKDOWN_PREVIEW_PARSE_SPAN,
        ViewPerfSpan::MarkdownPreviewStyledRowBuild => &MARKDOWN_PREVIEW_STYLED_ROW_BUILD_SPAN,
    }
}

#[inline]
#[cfg(any(debug_assertions, feature = "benchmarks"))]
fn row_stats(lane: ViewPerfRenderLane) -> &'static AtomicRowBatchStats {
    match lane {
        ViewPerfRenderLane::ResolvedPreview => &RENDER_RESOLVED_PREVIEW_ROWS_BATCH,
        ViewPerfRenderLane::MarkdownPreview => &MARKDOWN_PREVIEW_ROWS_BATCH,
    }
}

// ---------------------------------------------------------------------------
// Frame timing capture – Phase 0 benchmark infrastructure
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

#[cfg(all(test, any(debug_assertions, feature = "benchmarks")))]
mod tests {
    use super::*;

    #[test]
    fn atomic_span_stats_accumulate_and_reset() {
        let stats = AtomicSpanStats::new();
        stats.record_ns(11);
        stats.record_ns(29);
        assert_eq!(
            stats.snapshot(),
            SpanStats {
                calls: 2,
                total_nanos: 40,
            }
        );
        stats.reset();
        assert_eq!(stats.snapshot(), SpanStats::default());
    }

    #[test]
    fn atomic_row_batch_stats_accumulate_and_reset() {
        let stats = AtomicRowBatchStats::new();
        stats.record(8, 5);
        stats.record(7, 6);
        assert_eq!(
            stats.snapshot(),
            RowBatchStats {
                calls: 2,
                requested_rows: 15,
                painted_rows: 11,
            }
        );
        stats.reset();
        assert_eq!(stats.snapshot(), RowBatchStats::default());
    }

    #[test]
    fn snapshot_exposes_recorded_row_batches() {
        reset();
        record_row_batch(ViewPerfRenderLane::ResolvedPreview, 12, 9);
        let snapshot = snapshot();

        #[cfg(any(debug_assertions, feature = "benchmarks"))]
        {
            assert_eq!(
                snapshot.render_resolved_preview_rows_batch,
                RowBatchStats {
                    calls: 1,
                    requested_rows: 12,
                    painted_rows: 9,
                }
            );
        }
        #[cfg(not(any(debug_assertions, feature = "benchmarks")))]
        {
            assert_eq!(snapshot, ViewPerfSnapshot::default());
        }
    }

    #[test]
    fn snapshot_exposes_markdown_preview_metrics() {
        reset();
        record_row_batch(ViewPerfRenderLane::MarkdownPreview, 7, 6);
        {
            let _scope = span(ViewPerfSpan::MarkdownPreviewParse);
        }
        {
            let _scope = span(ViewPerfSpan::MarkdownPreviewStyledRowBuild);
        }

        let snapshot = snapshot();

        assert_eq!(
            snapshot.markdown_preview_rows_batch,
            RowBatchStats {
                calls: 1,
                requested_rows: 7,
                painted_rows: 6,
            }
        );
        assert_eq!(snapshot.markdown_preview_parse.calls, 1);
        assert_eq!(snapshot.markdown_preview_styled_row_build.calls, 1);
    }

    // -- Frame timing capture tests ------------------------------------------

    #[test]
    fn frame_timing_empty_capture_returns_zero_stats() {
        let capture = FrameTimingCapture::new(16_000_000);
        let stats = capture.finish_with_duration(1_000_000_000);
        assert_eq!(stats.frame_count, 0);
        assert_eq!(stats.p50_frame_ns, 0);
        assert_eq!(stats.dropped_frames, 0);
        assert_eq!(stats.budget_violations_per_sec, 0.0);
    }

    #[test]
    fn frame_timing_single_frame_within_budget() {
        let mut capture = FrameTimingCapture::new(16_000_000);
        capture.record_frame_ns(10_000_000); // 10 ms < 16 ms budget
        let stats = capture.finish_with_duration(1_000_000_000);
        assert_eq!(stats.frame_count, 1);
        assert_eq!(stats.p50_frame_ns, 10_000_000);
        assert_eq!(stats.p95_frame_ns, 10_000_000);
        assert_eq!(stats.p99_frame_ns, 10_000_000);
        assert_eq!(stats.max_frame_ns, 10_000_000);
        assert_eq!(stats.dropped_frames, 0);
        assert_eq!(stats.budget_violations_per_sec, 0.0);
    }

    #[test]
    fn frame_timing_detects_dropped_frames() {
        let budget_ns = 16_000_000; // 16 ms
        let mut capture = FrameTimingCapture::new(budget_ns);
        // 8 good frames at 10ms, 2 bad frames at 20ms
        for _ in 0..8 {
            capture.record_frame_ns(10_000_000);
        }
        capture.record_frame_ns(20_000_000);
        capture.record_frame_ns(25_000_000);

        // total duration = 1 second
        let stats = capture.finish_with_duration(1_000_000_000);
        assert_eq!(stats.frame_count, 10);
        assert_eq!(stats.dropped_frames, 2);
        assert!((stats.budget_violations_per_sec - 2.0).abs() < f64::EPSILON);
        assert_eq!(stats.max_frame_ns, 25_000_000);
    }

    #[test]
    fn frame_timing_percentiles_are_correct() {
        let mut capture = FrameTimingCapture::new(16_000_000);
        // 100 frames: 1ms, 2ms, 3ms, ..., 100ms
        for i in 1..=100 {
            capture.record_frame_ns(i * 1_000_000);
        }
        let stats = capture.finish_with_duration(5_000_000_000);
        assert_eq!(stats.frame_count, 100);
        // P50 = 50th value = 50ms
        assert_eq!(stats.p50_frame_ns, 50_000_000);
        // P95 = 95th value = 95ms
        assert_eq!(stats.p95_frame_ns, 95_000_000);
        // P99 = 99th value = 99ms
        assert_eq!(stats.p99_frame_ns, 99_000_000);
        assert_eq!(stats.max_frame_ns, 100_000_000);
    }

    #[test]
    fn frame_timing_p99_exceeds_2x_budget_check() {
        let budget_ns = 16_000_000;
        let mut capture = FrameTimingCapture::new(budget_ns);
        // 8 frames within budget, 2 frames at 3x budget → P99 lands on an outlier
        for _ in 0..8 {
            capture.record_frame_ns(10_000_000);
        }
        capture.record_frame_ns(50_000_000); // 50ms > 2 * 16ms = 32ms
        capture.record_frame_ns(55_000_000);

        let stats = capture.finish_with_duration(1_000_000_000);
        assert!(stats.p99_exceeds_2x_budget());
    }

    #[test]
    fn frame_timing_p99_within_2x_budget() {
        let budget_ns = 16_000_000;
        let mut capture = FrameTimingCapture::new(budget_ns);
        // All frames at 10ms — well within 2x budget (32ms)
        for _ in 0..100 {
            capture.record_frame_ns(10_000_000);
        }
        let stats = capture.finish_with_duration(1_000_000_000);
        assert!(!stats.p99_exceeds_2x_budget());
    }

    #[test]
    fn frame_timing_sidecar_metrics_shape() {
        let mut capture = FrameTimingCapture::new(16_666_667);
        capture.record_frame_ns(5_000_000);
        capture.record_frame_ns(8_000_000);
        capture.record_frame_ns(20_000_000);
        let stats = capture.finish_with_duration(1_000_000_000);
        let metrics = stats.to_sidecar_metrics();

        assert!(metrics.contains_key("frame_count"));
        assert!(metrics.contains_key("p50_frame_ms"));
        assert!(metrics.contains_key("p95_frame_ms"));
        assert!(metrics.contains_key("p99_frame_ms"));
        assert!(metrics.contains_key("max_frame_ms"));
        assert!(metrics.contains_key("dropped_frames"));
        assert!(metrics.contains_key("budget_violations_per_sec"));
        assert!(metrics.contains_key("frame_budget_ms"));

        assert_eq!(metrics["frame_count"], serde_json::json!(3));
        assert_eq!(metrics["dropped_frames"], serde_json::json!(1));
    }

    #[test]
    fn frame_timing_record_frame_duration() {
        let mut capture = FrameTimingCapture::new(16_000_000);
        capture.record_frame(std::time::Duration::from_millis(12));
        let stats = capture.finish_with_duration(1_000_000_000);
        assert_eq!(stats.frame_count, 1);
        assert_eq!(stats.p50_frame_ns, 12_000_000);
    }

    #[test]
    fn percentile_nearest_rank_boundary_cases() {
        // Single element
        assert_eq!(percentile_nearest_rank(&[42], 0), 42);
        assert_eq!(percentile_nearest_rank(&[42], 50), 42);
        assert_eq!(percentile_nearest_rank(&[42], 100), 42);

        // Two elements
        assert_eq!(percentile_nearest_rank(&[10, 20], 50), 10);
        assert_eq!(percentile_nearest_rank(&[10, 20], 100), 20);
    }
}
