#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};
#[cfg(debug_assertions)]
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
    ThreeWay,
    ResolverDiff,
    ResolvedPreview,
    MarkdownPreview,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct SpanStats {
    pub calls: u64,
    pub total_nanos: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct RowBatchStats {
    pub calls: u64,
    pub requested_rows: u64,
    pub painted_rows: u64,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ViewPerfSnapshot {
    pub render_three_way_rows_batch: RowBatchStats,
    pub render_resolver_diff_rows_batch: RowBatchStats,
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
    #[cfg(debug_assertions)]
    span: ViewPerfSpan,
    #[cfg(debug_assertions)]
    started_at: Instant,
}

impl Drop for PerfScope {
    fn drop(&mut self) {
        #[cfg(debug_assertions)]
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
    #[cfg(not(debug_assertions))]
    let _ = span;

    PerfScope {
        #[cfg(debug_assertions)]
        span,
        #[cfg(debug_assertions)]
        started_at: Instant::now(),
    }
}

#[inline]
pub(crate) fn record_row_batch(
    lane: ViewPerfRenderLane,
    requested_rows: usize,
    painted_rows: usize,
) {
    #[cfg(debug_assertions)]
    {
        row_stats(lane).record(
            saturating_usize_to_u64(requested_rows),
            saturating_usize_to_u64(painted_rows),
        );
    }
    #[cfg(not(debug_assertions))]
    let _ = (lane, requested_rows, painted_rows);
}

#[cfg(test)]
#[inline]
pub(crate) fn snapshot() -> ViewPerfSnapshot {
    #[cfg(debug_assertions)]
    {
        ViewPerfSnapshot {
            render_three_way_rows_batch: RENDER_THREE_WAY_ROWS_BATCH.snapshot(),
            render_resolver_diff_rows_batch: RENDER_RESOLVER_DIFF_ROWS_BATCH.snapshot(),
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
    #[cfg(not(debug_assertions))]
    {
        ViewPerfSnapshot::default()
    }
}

#[cfg(test)]
#[inline]
pub(crate) fn reset() {
    #[cfg(debug_assertions)]
    {
        RENDER_THREE_WAY_ROWS_BATCH.reset();
        RENDER_RESOLVER_DIFF_ROWS_BATCH.reset();
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
#[cfg(debug_assertions)]
fn saturating_usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(debug_assertions)]
#[derive(Debug)]
struct AtomicSpanStats {
    calls: AtomicU64,
    total_nanos: AtomicU64,
}

#[cfg(debug_assertions)]
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

    #[cfg(test)]
    #[inline]
    fn snapshot(&self) -> SpanStats {
        SpanStats {
            calls: self.calls.load(Ordering::Relaxed),
            total_nanos: self.total_nanos.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    #[inline]
    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.total_nanos.store(0, Ordering::Relaxed);
    }
}

#[cfg(debug_assertions)]
#[derive(Debug)]
struct AtomicRowBatchStats {
    calls: AtomicU64,
    requested_rows: AtomicU64,
    painted_rows: AtomicU64,
}

#[cfg(debug_assertions)]
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

    #[cfg(test)]
    #[inline]
    fn snapshot(&self) -> RowBatchStats {
        RowBatchStats {
            calls: self.calls.load(Ordering::Relaxed),
            requested_rows: self.requested_rows.load(Ordering::Relaxed),
            painted_rows: self.painted_rows.load(Ordering::Relaxed),
        }
    }

    #[cfg(test)]
    #[inline]
    fn reset(&self) {
        self.calls.store(0, Ordering::Relaxed);
        self.requested_rows.store(0, Ordering::Relaxed);
        self.painted_rows.store(0, Ordering::Relaxed);
    }
}

#[cfg(debug_assertions)]
static RENDER_THREE_WAY_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static RENDER_RESOLVER_DIFF_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static RENDER_RESOLVED_PREVIEW_ROWS_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static RECOMPUTE_RESOLVED_OUTLINE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static STYLED_TEXT_BUILD_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static SYNTAX_HIGHLIGHTING_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static WORD_QUERY_HIGHLIGHTING_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static MARKDOWN_PREVIEW_PARSE_SPAN: AtomicSpanStats = AtomicSpanStats::new();
#[cfg(debug_assertions)]
static MARKDOWN_PREVIEW_STYLED_ROW_BUILD_SPAN: AtomicSpanStats = AtomicSpanStats::new();

#[cfg(debug_assertions)]
static RENDER_THREE_WAY_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();
#[cfg(debug_assertions)]
static RENDER_RESOLVER_DIFF_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();
#[cfg(debug_assertions)]
static RENDER_RESOLVED_PREVIEW_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();
#[cfg(debug_assertions)]
static MARKDOWN_PREVIEW_ROWS_BATCH: AtomicRowBatchStats = AtomicRowBatchStats::new();

#[inline]
#[cfg(debug_assertions)]
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
#[cfg(debug_assertions)]
fn row_stats(lane: ViewPerfRenderLane) -> &'static AtomicRowBatchStats {
    match lane {
        ViewPerfRenderLane::ThreeWay => &RENDER_THREE_WAY_ROWS_BATCH,
        ViewPerfRenderLane::ResolverDiff => &RENDER_RESOLVER_DIFF_ROWS_BATCH,
        ViewPerfRenderLane::ResolvedPreview => &RENDER_RESOLVED_PREVIEW_ROWS_BATCH,
        ViewPerfRenderLane::MarkdownPreview => &MARKDOWN_PREVIEW_ROWS_BATCH,
    }
}

#[cfg(all(test, debug_assertions))]
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
        record_row_batch(ViewPerfRenderLane::ThreeWay, 12, 9);
        let snapshot = snapshot();

        #[cfg(debug_assertions)]
        {
            assert_eq!(
                snapshot.render_three_way_rows_batch,
                RowBatchStats {
                    calls: 1,
                    requested_rows: 12,
                    painted_rows: 9,
                }
            );
        }
        #[cfg(not(debug_assertions))]
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
}
