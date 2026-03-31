use super::common::*;

pub(crate) fn bench_text_model_snapshot_clone_cost(c: &mut Criterion) {
    let bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_BYTES", 2 * 1024 * 1024);
    let clones = env_usize("GITCOMET_BENCH_TEXT_MODEL_SNAPSHOT_CLONES", 8_192);
    let fixture = TextModelSnapshotCloneCostFixture::new(bytes);

    let mut group = c.benchmark_group("text_model_snapshot_clone_cost");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("piece_table_snapshot_clone", clones),
        &clones,
        |b, &clones| b.iter(|| fixture.run_snapshot_clone_step(clones)),
    );
    group.bench_with_input(
        BenchmarkId::new("shared_string_clone_control", clones),
        &clones,
        |b, &clones| b.iter(|| fixture.run_string_clone_control_step(clones)),
    );
    group.finish();

    let (_, snapshot_metrics) =
        measure_sidecar_allocations(|| fixture.run_snapshot_clone_step_with_metrics(clones));
    emit_text_model_snapshot_clone_cost_sidecar(
        &format!("text_model_snapshot_clone_cost/piece_table_snapshot_clone/{clones}"),
        &snapshot_metrics,
    );

    let (_, control_metrics) =
        measure_sidecar_allocations(|| fixture.run_string_clone_control_step_with_metrics(clones));
    emit_text_model_snapshot_clone_cost_sidecar(
        &format!("text_model_snapshot_clone_cost/shared_string_clone_control/{clones}"),
        &control_metrics,
    );
}
