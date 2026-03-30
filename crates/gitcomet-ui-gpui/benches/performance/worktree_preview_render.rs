use super::common::*;

pub(crate) fn bench_worktree_preview_render(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_LINES", 4_000);
    let window = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_LINE_BYTES", 128);
    let fixture = WorktreePreviewRenderFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("worktree_preview_render");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("cached_lookup_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_cached_lookup_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.bench_with_input(
        BenchmarkId::new("render_time_prepare_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_render_time_prepare_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    // Sidecar metrics emission for structural budgets.
    let (_, cached_metrics) =
        measure_sidecar_allocations(|| fixture.run_cached_lookup_with_metrics(0, window));
    emit_worktree_preview_render_sidecar(
        &format!("worktree_preview_render/cached_lookup_window/{window}"),
        &cached_metrics,
    );

    let (_, prepare_metrics) =
        measure_sidecar_allocations(|| fixture.run_render_time_prepare_with_metrics(0, window));
    emit_worktree_preview_render_sidecar(
        &format!("worktree_preview_render/render_time_prepare_window/{window}"),
        &prepare_metrics,
    );
}
