use super::common::*;

pub(crate) fn bench_text_input_prepaint_windowed(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let window_rows = env_usize("GITCOMET_BENCH_TEXT_INPUT_WINDOW_ROWS", 80);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);

    let mut windowed_fixture = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let mut full_fixture = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_prepaint_windowed");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("window_rows", window_rows),
        &window_rows,
        |b, &window_rows| {
            let mut start = 0usize;
            b.iter(|| {
                let h = windowed_fixture.run_windowed_step(start, window_rows.max(1));
                start = start.wrapping_add(window_rows.max(1) / 2 + 1)
                    % windowed_fixture.total_rows().max(1);
                h
            })
        },
    );
    group.bench_function(BenchmarkId::from_parameter("full_document_control"), |b| {
        b.iter(|| full_fixture.run_full_document_step())
    });
    group.finish();

    // Collect metrics from a fresh run for sidecar emission.
    let mut sidecar_windowed = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let (_, windowed_metrics) = measure_sidecar_allocations(|| {
        sidecar_windowed.run_windowed_step_with_metrics(0, window_rows.max(1))
    });
    emit_text_input_prepaint_windowed_sidecar(
        &format!("text_input_prepaint_windowed/window_rows/{window_rows}"),
        &windowed_metrics,
    );

    let mut sidecar_full = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let (_, full_metrics) =
        measure_sidecar_allocations(|| sidecar_full.run_full_document_step_with_metrics());
    emit_text_input_prepaint_windowed_sidecar(
        "text_input_prepaint_windowed/full_document_control",
        &full_metrics,
    );
}
