use super::common::*;

pub(crate) fn bench_conflict_load_duplication(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictLoadDuplicationFixture::new(lines, low_density_blocks);
    let high_density = ConflictLoadDuplicationFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_load_duplication");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("shared_payload_forwarding", label), |b| {
            b.iter(|| fixture.run_shared_payload_forwarding_step())
        });
        group.bench_function(BenchmarkId::new("duplicated_text_and_bytes", label), |b| {
            b.iter(|| fixture.run_duplicated_payload_forwarding_step())
        });
    }
    group.finish();

    let _ = measure_sidecar_allocations(|| low_density.run_shared_payload_forwarding_step());
    emit_allocation_only_sidecar("conflict_load_duplication/shared_payload_forwarding/low_density");
    let _ = measure_sidecar_allocations(|| low_density.run_duplicated_payload_forwarding_step());
    emit_allocation_only_sidecar("conflict_load_duplication/duplicated_text_and_bytes/low_density");
    let _ = measure_sidecar_allocations(|| high_density.run_shared_payload_forwarding_step());
    emit_allocation_only_sidecar(
        "conflict_load_duplication/shared_payload_forwarding/high_density",
    );
    let _ = measure_sidecar_allocations(|| high_density.run_duplicated_payload_forwarding_step());
    emit_allocation_only_sidecar(
        "conflict_load_duplication/duplicated_text_and_bytes/high_density",
    );
}
