use super::*;
use gitcomet_core::history_perf::{self, Work};

#[test]
fn indexed_history_skipped_rows_and_warm_windows_materialize_no_paint() {
    for width in [1, 64, 512, 5261] {
        let fixture = IndexedHistoryFixture::new(20_000, width, 20);
        assert_eq!(fixture.build_index(), fixture.retained_bytes());
        assert!(fixture.estimated_index_peak_bytes() >= fixture.retained_bytes());
        {
            let _capture = history_perf::capture();
            fixture.build_graph();
            assert_eq!(history_perf::count(Work::PaintRow), 0);
            assert_eq!(history_perf::count(Work::GraphTransition), 20_000);
        }
        let _capture = history_perf::capture();
        fixture.window(10_239, 40, None);
        assert_eq!(history_perf::count(Work::PaintRow), 40);
        let transitions = history_perf::count(Work::GraphTransition);
        assert_eq!(transitions, 1023 + 40);
        fixture.window(10_239, 40, None);
        assert_eq!(history_perf::count(Work::GraphTransition), transitions);
        fixture.window(10_239, 40, Some(100));
        let transitions = history_perf::count(Work::GraphTransition);
        fixture.window(10_239, 40, Some(100));
        assert_eq!(history_perf::count(Work::GraphTransition), transitions);
        assert!(fixture.checkpoint_bytes() > 0);
    }
}

#[test]
#[ignore = "production index/graph phase and window latency benchmark"]
fn indexed_history_wide_graph_phase_benchmark() {
    use std::time::Instant;
    for count in [100_000, 2_000_000] {
        for width in [1, 64, 512, 5261] {
            let started = Instant::now();
            let fixture = IndexedHistoryFixture::new(count, width, 20);
            let construction = started.elapsed();
            let mut cold = Vec::new();
            let mut warm = Vec::new();
            for sample in 0..100 {
                let start = (sample * 97171) % (count - 40);
                fixture.clear_window();
                let started = Instant::now();
                fixture.window(start, 40, Some(count / 3));
                cold.push(started.elapsed().as_secs_f64() * 1000.0);
                let started = Instant::now();
                fixture.window(start, 40, Some(count / 3));
                warm.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            cold.sort_by(f64::total_cmp);
            warm.sort_by(f64::total_cmp);
            eprintln!(
                "rows={count} width={width} construct_s={:.3} retained_bytes={} checkpoint_bytes={} lane_span_bytes={} first_touch_ms_p50_p95_p99={:?} warm_ms_p50_p95_p99={:?}",
                construction.as_secs_f64(),
                fixture.retained_bytes(),
                fixture.checkpoint_bytes(),
                fixture.lane_span_bytes(),
                [cold[50], cold[95], cold[99]],
                [warm[50], warm[95], warm[99]]
            );
        }
    }
}
