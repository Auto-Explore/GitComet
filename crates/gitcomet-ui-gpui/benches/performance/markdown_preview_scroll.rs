use super::common::*;

pub(crate) fn bench_markdown_preview_scroll(c: &mut Criterion) {
    let sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_SCROLL_SECTIONS", 768);
    let window = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_WINDOW", 200);
    let scroll_step_rows = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_SCROLL_STEP", 24).max(1);
    let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_LINE_BYTES", 128);
    let measurement_time = markdown_preview_measurement_time();

    {
        let fixture = MarkdownPreviewScrollFixture::new_sectioned(sections, line_bytes);
        let rich_fixture = MarkdownPreviewScrollFixture::new_rich_5000_rows();

        let mut group = c.benchmark_group("markdown_preview_scroll");
        group.sample_size(10);
        group.warm_up_time(measurement_time);
        group.measurement_time(measurement_time);
        group.bench_with_input(
            BenchmarkId::new("window_rows", window),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let hash = fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(scroll_step_rows);
                    hash
                })
            },
        );
        group.bench_with_input(
            BenchmarkId::new("rich_5000_rows_window_rows", window),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let hash = rich_fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(scroll_step_rows);
                    hash
                })
            },
        );
        group.finish();

        let _ = fixture.run_scroll_step(0, window);
        let (_, metrics) = measure_sidecar_allocations(|| {
            fixture.run_scroll_step_with_metrics(scroll_step_rows, window, scroll_step_rows)
        });
        emit_markdown_preview_scroll_sidecar(
            &format!("markdown_preview_scroll/window_rows/{window}"),
            &metrics,
        );

        let _ = rich_fixture.run_scroll_step(0, window);
        let (_, rich_metrics) = measure_sidecar_allocations(|| {
            rich_fixture.run_scroll_step_with_metrics(scroll_step_rows, window, scroll_step_rows)
        });
        emit_markdown_preview_scroll_sidecar(
            &format!("markdown_preview_scroll/rich_5000_rows_window_rows/{window}"),
            &rich_metrics,
        );
    }

    settle_markdown_allocator_pages();
}
