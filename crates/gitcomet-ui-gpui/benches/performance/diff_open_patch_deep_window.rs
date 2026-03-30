use super::common::*;

pub(crate) fn bench_diff_open_patch_deep_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    // Compute start row at 90% depth.
    let total = fixture.total_rows_hint();
    let start_row = total.saturating_mul(9) / 10;

    let metrics =
        measure_sidecar_allocations(|| fixture.measure_paged_deep_window_step(start_row, window));

    let mut group = c.benchmark_group("diff_open_patch_deep_window_90pct");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_window_at_step(start_row, window)),
    );
    group.finish();

    // Re-use patch diff sidecar format for deep-window metrics.
    emit_patch_diff_sidecar(
        &format!("diff_open_patch_deep_window_90pct/{window}"),
        0,
        metrics,
    );
}
