use super::common::*;

pub(crate) fn bench_diff_open_markdown_preview_first_window(c: &mut Criterion) {
    {
        let sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_SECTIONS", 384);
        let window = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_WINDOW", 200);
        let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_LINE_BYTES", 128);
        let measurement_time = markdown_preview_measurement_time();
        let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

        let metrics = measure_sidecar_allocations(|| fixture.measure_first_window_diff(window));

        let mut group = c.benchmark_group("diff_open_markdown_preview_first_window");
        group.sample_size(10);
        group.warm_up_time(measurement_time);
        group.measurement_time(measurement_time);
        group.bench_with_input(
            BenchmarkId::from_parameter(window),
            &window,
            |b, &window| b.iter(|| fixture.run_first_window_diff_step(window)),
        );
        group.finish();
        emit_markdown_preview_first_window_sidecar(window, &metrics);
    }

    settle_markdown_allocator_pages();
}
