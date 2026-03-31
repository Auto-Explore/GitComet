use super::common::*;

pub(crate) fn bench_text_input_long_line_cap(c: &mut Criterion) {
    let long_line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LONG_LINE_BYTES", 256 * 1024);
    let max_shape_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_MAX_LINE_SHAPE_BYTES", 4 * 1024);
    let fixture = TextInputLongLineCapFixture::new(long_line_bytes);

    let mut group = c.benchmark_group("text_input_long_line_cap");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::new("capped_bytes", max_shape_bytes), |b| {
        b.iter(|| fixture.run_with_cap(max_shape_bytes))
    });
    group.bench_function(BenchmarkId::from_parameter("uncapped_control"), |b| {
        b.iter(|| fixture.run_without_cap())
    });
    group.finish();

    let (_, capped_metrics) =
        measure_sidecar_allocations(|| fixture.run_with_cap_with_metrics(max_shape_bytes));
    emit_text_input_long_line_cap_sidecar(
        &format!("text_input_long_line_cap/capped_bytes/{max_shape_bytes}"),
        &capped_metrics,
    );
    let (_, uncapped_metrics) =
        measure_sidecar_allocations(|| fixture.run_without_cap_with_metrics());
    emit_text_input_long_line_cap_sidecar(
        "text_input_long_line_cap/uncapped_control",
        &uncapped_metrics,
    );
}
