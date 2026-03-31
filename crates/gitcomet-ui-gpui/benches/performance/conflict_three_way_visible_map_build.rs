use super::common::*;

pub(crate) fn bench_conflict_three_way_visible_map_build(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictThreeWayVisibleMapBuildFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_visible_map_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("linear_two_pointer"), |b| {
        b.iter(|| fixture.run_linear_step())
    });
    group.bench_function(BenchmarkId::from_parameter("legacy_find_scan"), |b| {
        b.iter(|| fixture.run_legacy_step())
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_linear_step());
    emit_allocation_only_sidecar("conflict_three_way_visible_map_build/linear_two_pointer");
    let _ = measure_sidecar_allocations(|| fixture.run_legacy_step());
    emit_allocation_only_sidecar("conflict_three_way_visible_map_build/legacy_find_scan");
}
