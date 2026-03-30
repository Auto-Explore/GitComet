use super::common::*;

pub(crate) fn bench_file_diff_inline_syntax_projection(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_LINE_BYTES", 128);
    let window = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_WINDOW", 200);
    let pending_fixture = FileDiffInlineSyntaxProjectionFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("file_diff_inline_syntax_projection");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("visible_window_pending", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let hash = pending_fixture.run_window_pending_step(start, window);
                start = pending_fixture.next_start_row(start, window);
                hash
            })
        },
    );

    let ready_fixture = FileDiffInlineSyntaxProjectionFixture::new(lines, line_bytes);
    ready_fixture.prime_window(window);
    group.bench_with_input(
        BenchmarkId::new("visible_window_ready", window),
        &window,
        |b, &window| b.iter(|| ready_fixture.run_window_step(0, window)),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| pending_fixture.run_window_pending_step(0, window));
    emit_allocation_only_sidecar(&format!(
        "file_diff_inline_syntax_projection/visible_window_pending/{window}"
    ));
    let _ = measure_sidecar_allocations(|| ready_fixture.run_window_step(0, window));
    emit_allocation_only_sidecar(&format!(
        "file_diff_inline_syntax_projection/visible_window_ready/{window}"
    ));
}
