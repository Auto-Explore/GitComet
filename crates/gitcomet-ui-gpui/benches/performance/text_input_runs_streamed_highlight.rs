use super::common::*;

pub(crate) fn bench_text_input_runs_streamed_highlight(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let window_rows = env_usize("GITCOMET_BENCH_TEXT_INPUT_WINDOW_ROWS", 80);

    let dense_fixture = TextInputRunsStreamedHighlightFixture::new(
        lines,
        line_bytes,
        window_rows,
        TextInputHighlightDensity::Dense,
    );
    let sparse_fixture = TextInputRunsStreamedHighlightFixture::new(
        lines,
        line_bytes,
        window_rows,
        TextInputHighlightDensity::Sparse,
    );

    let mut dense_group = c.benchmark_group("text_input_runs_streamed_highlight_dense");
    dense_group.sample_size(10);
    dense_group.warm_up_time(Duration::from_secs(1));
    dense_group.bench_function(BenchmarkId::from_parameter("legacy_scan"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = dense_fixture.run_legacy_step(start);
            start = dense_fixture.next_start_row(start);
            h
        })
    });
    dense_group.bench_function(BenchmarkId::from_parameter("streamed_cursor"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = dense_fixture.run_streamed_step(start);
            start = dense_fixture.next_start_row(start);
            h
        })
    });
    dense_group.finish();

    let mut sparse_group = c.benchmark_group("text_input_runs_streamed_highlight_sparse");
    sparse_group.sample_size(10);
    sparse_group.warm_up_time(Duration::from_secs(1));
    sparse_group.bench_function(BenchmarkId::from_parameter("legacy_scan"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = sparse_fixture.run_legacy_step(start);
            start = sparse_fixture.next_start_row(start);
            h
        })
    });
    sparse_group.bench_function(BenchmarkId::from_parameter("streamed_cursor"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = sparse_fixture.run_streamed_step(start);
            start = sparse_fixture.next_start_row(start);
            h
        })
    });
    sparse_group.finish();

    let (_, dense_legacy_metrics) =
        measure_sidecar_allocations(|| dense_fixture.run_legacy_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_dense/legacy_scan",
        &dense_legacy_metrics,
    );
    let (_, dense_streamed_metrics) =
        measure_sidecar_allocations(|| dense_fixture.run_streamed_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_dense/streamed_cursor",
        &dense_streamed_metrics,
    );
    let (_, sparse_legacy_metrics) =
        measure_sidecar_allocations(|| sparse_fixture.run_legacy_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_sparse/legacy_scan",
        &sparse_legacy_metrics,
    );
    let (_, sparse_streamed_metrics) =
        measure_sidecar_allocations(|| sparse_fixture.run_streamed_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        &sparse_streamed_metrics,
    );
}
