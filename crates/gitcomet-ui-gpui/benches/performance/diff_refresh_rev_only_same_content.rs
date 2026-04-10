use super::common::*;

pub(crate) fn bench_diff_refresh_rev_only_same_content(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_DIFF_REFRESH_LINES", 5_000);
    let fixture = DiffRefreshFixture::new(lines);

    let rekey_bench = "diff_refresh_rev_only_same_content/rekey";
    let rebuild_bench = "diff_refresh_rev_only_same_content/rebuild";
    let rekey_metrics =
        measure_sidecar_allocations_if_selected(rekey_bench, || fixture.measure_rekey());
    let rebuild_metrics =
        measure_sidecar_allocations_if_selected(rebuild_bench, || fixture.measure_rebuild());

    let mut group = c.benchmark_group("diff_refresh_rev_only_same_content");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("rekey"), |b| {
        b.iter(|| fixture.run_rekey_step())
    });
    group.bench_function(BenchmarkId::from_parameter("rebuild"), |b| {
        b.iter(|| fixture.run_rebuild_step())
    });
    group.finish();

    if let Some(metrics) = rekey_metrics.as_ref() {
        emit_diff_refresh_sidecar("rekey", metrics);
    }
    if let Some(metrics) = rebuild_metrics.as_ref() {
        emit_diff_refresh_sidecar("rebuild", metrics);
    }
}
