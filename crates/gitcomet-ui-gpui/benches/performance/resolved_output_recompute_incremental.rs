use super::common::*;

pub(crate) fn bench_resolved_output_recompute_incremental(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let mut full_fixture = ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let mut incremental_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("resolved_output_recompute_incremental");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("full_recompute"), |b| {
        b.iter(|| full_fixture.run_full_recompute_step())
    });
    group.bench_function(BenchmarkId::from_parameter("incremental_recompute"), |b| {
        b.iter(|| incremental_fixture.run_incremental_recompute_step())
    });
    group.finish();

    let mut full_sidecar_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let (_, full_metrics) =
        measure_sidecar_allocations(|| full_sidecar_fixture.run_full_recompute_with_metrics());
    emit_resolved_output_recompute_sidecar(
        "resolved_output_recompute_incremental/full_recompute",
        &full_metrics,
    );

    let mut incremental_sidecar_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let (_, incremental_metrics) = measure_sidecar_allocations(|| {
        incremental_sidecar_fixture.run_incremental_recompute_with_metrics()
    });
    emit_resolved_output_recompute_sidecar(
        "resolved_output_recompute_incremental/incremental_recompute",
        &incremental_metrics,
    );
}
