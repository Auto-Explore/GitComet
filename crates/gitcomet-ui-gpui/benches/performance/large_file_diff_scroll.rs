use super::common::*;

pub(crate) fn bench_large_file_diff_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_DIFF_LINES", 10_000);
    let window = env_usize("GITCOMET_BENCH_DIFF_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_DIFF_LINE_BYTES", 96);
    let long_line_bytes = env_usize("GITCOMET_BENCH_DIFF_LONG_LINE_BYTES", 4_096);
    let normal_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(lines, line_bytes);
    let long_line_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(lines, long_line_bytes);

    let mut group = c.benchmark_group("diff_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("normal_lines_window", window),
        &window,
        |b, &window| {
            // Use a varying start index per-iteration to reduce cache effects in allocators.
            let mut start = 0usize;
            b.iter(|| {
                let h = normal_fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.bench_with_input(
        BenchmarkId::new("long_lines_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = long_line_fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    let (_, normal_metrics) =
        measure_sidecar_allocations(|| normal_fixture.run_scroll_step_with_metrics(0, window));
    emit_diff_scroll_sidecar(
        &format!("diff_scroll/normal_lines_window/{window}"),
        &normal_metrics,
    );

    let (_, long_metrics) =
        measure_sidecar_allocations(|| long_line_fixture.run_scroll_step_with_metrics(0, window));
    emit_diff_scroll_sidecar(
        &format!("diff_scroll/long_lines_window/{window}"),
        &long_metrics,
    );
}
