use super::common::*;

pub(crate) fn bench_conflict_two_way_word_highlights(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictTwoWayDiffBuildFixture::new(lines, low_density_blocks);
    let high_density = ConflictTwoWayDiffBuildFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_two_way_word_highlights");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("full_file", label), |b| {
            b.iter(|| fixture.run_full_word_highlights_step())
        });
        group.bench_function(BenchmarkId::new("block_local", label), |b| {
            b.iter(|| fixture.run_block_local_word_highlights_step())
        });
    }
    group.finish();

    let _ = measure_sidecar_allocations(|| low_density.run_full_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/full_file/low_density");
    let _ = measure_sidecar_allocations(|| low_density.run_block_local_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/block_local/low_density");
    let _ = measure_sidecar_allocations(|| high_density.run_full_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/full_file/high_density");
    let _ = measure_sidecar_allocations(|| high_density.run_block_local_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/block_local/high_density");
}
