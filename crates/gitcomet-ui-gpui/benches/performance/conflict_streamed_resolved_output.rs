use super::common::*;

pub(crate) fn bench_conflict_streamed_resolved_output(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_STREAMED_LINES", 50_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 500);
    let window = env_usize("GITCOMET_BENCH_STREAMED_WINDOW", 200);

    let fixture = ConflictStreamedResolvedOutputFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_streamed_resolved_output");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("projection_build"), |b| {
        b.iter(|| fixture.run_projection_build_step())
    });
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &w| {
        b.iter(|| fixture.run_window_step(w))
    });
    group.bench_with_input(
        BenchmarkId::new("deep_window_90pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_window_step(0.9, w)),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_projection_build_step());
    emit_allocation_only_sidecar("conflict_streamed_resolved_output/projection_build");
    let _ = measure_sidecar_allocations(|| fixture.run_window_step(window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_resolved_output/window/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_deep_window_step(0.9, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_resolved_output/deep_window_90pct/{window}"
    ));
}
