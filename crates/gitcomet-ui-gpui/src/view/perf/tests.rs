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

    assert_eq!(
        snapshot.render_resolved_preview_rows_batch,
        RowBatchStats {
            calls: 1,
            requested_rows: 12,
            painted_rows: 9,
        }
    );
}

#[test]
fn snapshot_exposes_markdown_preview_metrics() {
    reset();
    record_row_batch(ViewPerfRenderLane::MarkdownPreview, 7, 6);
    {
        let _scope = span(ViewPerfSpan::MarkdownPreviewParse);
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
    assert!(snapshot.markdown_preview_parse.calls >= 1);
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
