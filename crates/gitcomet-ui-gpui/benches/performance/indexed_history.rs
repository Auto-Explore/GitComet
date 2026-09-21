use super::common::*;
use gitcomet_core::history_perf::{self, Work};
use gitcomet_ui_gpui::benchmarks::IndexedHistoryFixture;

pub(crate) fn bench_indexed_history(c: &mut Criterion) {
    let count = env_usize("GITCOMET_BENCH_COMMITS", 100_000);
    let first_touch_start = (count / 2 / 1024) * 1024 + 1023;
    let mut group = c.benchmark_group("indexed_history");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for width in [1, 64, 512, 5261] {
        let fixture = IndexedHistoryFixture::new(count, width, 20);
        let case = format!("{count}_rows_{width}_columns");
        group.bench_function(BenchmarkId::new(&case, "build_graph"), |b| {
            b.iter(|| fixture.build_graph())
        });
        group.bench_function(BenchmarkId::new(&case, "first_touch"), |b| {
            b.iter(|| {
                fixture.clear_window();
                fixture.window(first_touch_start, 40, None)
            })
        });
        group.bench_function(BenchmarkId::new(&case, "warm"), |b| {
            b.iter(|| fixture.window(first_touch_start, 40, Some(count / 4)))
        });
        group.bench_function(BenchmarkId::new(&case, "distant_jump"), |b| {
            let mut start = 0usize;
            b.iter(|| {
                start = (start + 97171) % count;
                fixture.window(start, 40, Some(count / 4))
            })
        });
        group.bench_function(BenchmarkId::new(&case, "changing_selection"), |b| {
            let mut anchor = 0usize;
            b.iter(|| {
                anchor = (anchor + 97171) % count;
                fixture.window(first_touch_start, 40, Some(anchor))
            })
        });
        fixture.window(first_touch_start, 40, Some(count / 4));
        let _capture = history_perf::capture();
        measure_sidecar_allocations(|| fixture.window(first_touch_start, 40, Some(count / 4)));
        let mut payload = Map::new();
        payload.insert(
            "retained_index_bytes".into(),
            json!(fixture.retained_bytes()),
        );
        payload.insert("checkpoint_bytes".into(), json!(fixture.checkpoint_bytes()));
        payload.insert(
            "estimated_index_peak_bytes".into(),
            json!(fixture.estimated_index_peak_bytes()),
        );
        payload.insert(
            "paint_rows".into(),
            json!(history_perf::count(Work::PaintRow)),
        );
        emit_sidecar_metrics(&format!("indexed_history/{case}/warm"), payload);
    }
    for hash_len in [20, 32] {
        let fixture = IndexedHistoryFixture::new(count, 1, hash_len);
        group.bench_function(format!("index_{count}_hash_{hash_len}"), |b| {
            b.iter(|| fixture.build_index())
        });
    }
    group.finish();
}
