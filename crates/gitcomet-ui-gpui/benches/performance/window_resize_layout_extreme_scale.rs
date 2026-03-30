use super::common::*;

pub(crate) fn bench_window_resize_layout_extreme_scale(c: &mut Criterion) {
    let fixture = WindowResizeLayoutExtremeFixture::history_50k_commits_diff_20k_lines();
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());

    let mut group = c.benchmark_group("window_resize_layout");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("history_50k_commits_diff_20k_lines"),
        |b| b.iter(|| fixture.run()),
    );
    group.finish();

    emit_window_resize_layout_extreme_sidecar(
        "window_resize_layout/history_50k_commits_diff_20k_lines",
        &metrics,
    );
}
