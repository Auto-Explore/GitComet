use super::common::*;

pub(crate) fn bench_text_model_fragmented_edits(c: &mut Criterion) {
    let bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_BYTES", 512 * 1024);
    let edits = env_usize("GITCOMET_BENCH_TEXT_MODEL_EDITS", 500);
    let reads = env_usize("GITCOMET_BENCH_TEXT_MODEL_READS_AFTER_EDIT", 64);
    let fixture = TextModelFragmentedEditFixture::new(bytes, edits);

    let mut group = c.benchmark_group("text_model_fragmented_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("piece_table_edits"), |b| {
        b.iter(|| fixture.run_fragmented_edit_step())
    });
    group.bench_function(
        BenchmarkId::from_parameter("materialize_after_edits"),
        |b| b.iter(|| fixture.run_materialize_after_edits_step()),
    );
    group.bench_function(BenchmarkId::new("shared_string_after_edits", reads), |b| {
        b.iter(|| fixture.run_shared_string_after_edits_step(reads))
    });
    group.bench_function(BenchmarkId::from_parameter("string_edit_control"), |b| {
        b.iter(|| fixture.run_string_edit_control_step())
    });
    group.finish();

    let (_, piece_table_metrics) =
        measure_sidecar_allocations(|| fixture.run_fragmented_edit_step_with_metrics());
    emit_text_model_fragmented_edits_sidecar(
        "text_model_fragmented_edits/piece_table_edits",
        &piece_table_metrics,
    );

    let (_, materialize_metrics) =
        measure_sidecar_allocations(|| fixture.run_materialize_after_edits_step_with_metrics());
    emit_text_model_fragmented_edits_sidecar(
        "text_model_fragmented_edits/materialize_after_edits",
        &materialize_metrics,
    );

    let (_, shared_metrics) = measure_sidecar_allocations(|| {
        fixture.run_shared_string_after_edits_step_with_metrics(reads)
    });
    emit_text_model_fragmented_edits_sidecar(
        &format!("text_model_fragmented_edits/shared_string_after_edits/{reads}"),
        &shared_metrics,
    );

    let (_, control_metrics) =
        measure_sidecar_allocations(|| fixture.run_string_edit_control_step_with_metrics());
    emit_text_model_fragmented_edits_sidecar(
        "text_model_fragmented_edits/string_edit_control",
        &control_metrics,
    );
}
