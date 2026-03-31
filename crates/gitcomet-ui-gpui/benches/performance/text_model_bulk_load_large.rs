use super::common::*;

pub(crate) fn bench_text_model_bulk_load_large(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_MODEL_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_LINE_BYTES", 128);
    let fixture = TextModelBulkLoadLargeFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("text_model_bulk_load_large");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("piece_table_append_large"),
        |b| b.iter(|| fixture.run_piece_table_bulk_load_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("piece_table_from_large_text"),
        |b| b.iter(|| fixture.run_piece_table_from_large_text_step()),
    );
    group.bench_function(BenchmarkId::from_parameter("string_push_control"), |b| {
        b.iter(|| fixture.run_string_bulk_load_control_step())
    });
    group.finish();

    let (_, append_metrics) =
        measure_sidecar_allocations(|| fixture.run_piece_table_bulk_load_step_with_metrics());
    emit_text_model_bulk_load_large_sidecar(
        "text_model_bulk_load_large/piece_table_append_large",
        &append_metrics,
    );

    let (_, from_large_metrics) =
        measure_sidecar_allocations(|| fixture.run_piece_table_from_large_text_step_with_metrics());
    emit_text_model_bulk_load_large_sidecar(
        "text_model_bulk_load_large/piece_table_from_large_text",
        &from_large_metrics,
    );

    let (_, control_metrics) =
        measure_sidecar_allocations(|| fixture.run_string_bulk_load_control_step_with_metrics());
    emit_text_model_bulk_load_large_sidecar(
        "text_model_bulk_load_large/string_push_control",
        &control_metrics,
    );
}
