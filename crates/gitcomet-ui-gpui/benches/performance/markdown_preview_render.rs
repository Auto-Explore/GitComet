use super::common::*;

pub(crate) fn bench_markdown_preview_render(c: &mut Criterion) {
    let sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_SECTIONS", 384);
    let window = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_LINE_BYTES", 128);
    let measurement_time = markdown_preview_measurement_time();

    {
        let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

        let mut single_group = c.benchmark_group("markdown_preview_render_single");
        single_group.sample_size(10);
        single_group.warm_up_time(measurement_time);
        single_group.measurement_time(measurement_time);
        single_group.bench_with_input(
            BenchmarkId::new("window_rows", window),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let hash = fixture.run_render_single_step(start, window);
                    start = start.wrapping_add(window);
                    hash
                })
            },
        );
        single_group.finish();

        let _ = measure_sidecar_allocations(|| fixture.run_render_single_step(0, window));
        emit_allocation_only_sidecar(&format!(
            "markdown_preview_render_single/window_rows/{window}"
        ));
    }

    settle_markdown_allocator_pages();

    {
        let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

        let mut diff_group = c.benchmark_group("markdown_preview_render_diff");
        diff_group.sample_size(10);
        diff_group.warm_up_time(measurement_time);
        diff_group.measurement_time(measurement_time);
        diff_group.bench_with_input(
            BenchmarkId::new("window_rows", window),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let hash = fixture.run_render_diff_step(start, window);
                    start = start.wrapping_add(window);
                    hash
                })
            },
        );
        diff_group.finish();

        let _ = measure_sidecar_allocations(|| fixture.run_render_diff_step(0, window));
        emit_allocation_only_sidecar(&format!(
            "markdown_preview_render_diff/window_rows/{window}"
        ));
    }

    settle_markdown_allocator_pages();
}
