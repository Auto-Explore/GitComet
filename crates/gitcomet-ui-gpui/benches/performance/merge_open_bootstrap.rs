use super::common::*;

pub(crate) fn bench_merge_open_bootstrap(c: &mut Criterion) {
    let small_lines = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_SMALL_LINES", 5_000);
    let small = MergeOpenBootstrapFixture::small(small_lines);
    let (_, small_metrics) = measure_sidecar_allocations(|| small.run_with_metrics());

    let lines = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_LINES", 55_001);
    let conflict_blocks = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_CONFLICTS", 1);
    let large_streamed = MergeOpenBootstrapFixture::large_streamed(lines, conflict_blocks);
    let (_, large_streamed_metrics) =
        measure_sidecar_allocations(|| large_streamed.run_with_metrics());

    let many_conflicts_blocks = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_MANY_CONFLICTS", 50);
    let many_conflicts = MergeOpenBootstrapFixture::many_conflicts(many_conflicts_blocks);
    let (_, many_conflicts_metrics) =
        measure_sidecar_allocations(|| many_conflicts.run_with_metrics());

    let extreme_lines = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_EXTREME_LINES", 50_000);
    let extreme_conflicts = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_EXTREME_CONFLICTS", 500);
    let large_many =
        MergeOpenBootstrapFixture::large_many_conflicts(extreme_lines, extreme_conflicts);
    let (_, large_many_metrics) = measure_sidecar_allocations(|| large_many.run_with_metrics());

    let mut group = c.benchmark_group("merge_open_bootstrap");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("small"), |b| {
        b.iter(|| small.run())
    });
    group.bench_function(BenchmarkId::from_parameter("large_streamed"), |b| {
        b.iter(|| large_streamed.run())
    });
    group.bench_function(BenchmarkId::from_parameter("many_conflicts"), |b| {
        b.iter(|| many_conflicts.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("50k_lines_500_conflicts_streamed"),
        |b| b.iter(|| large_many.run()),
    );
    group.finish();

    emit_merge_open_bootstrap_sidecar("small", &small_metrics);
    emit_merge_open_bootstrap_sidecar("large_streamed", &large_streamed_metrics);
    emit_merge_open_bootstrap_sidecar("many_conflicts", &many_conflicts_metrics);
    emit_merge_open_bootstrap_sidecar("50k_lines_500_conflicts_streamed", &large_many_metrics);
}
