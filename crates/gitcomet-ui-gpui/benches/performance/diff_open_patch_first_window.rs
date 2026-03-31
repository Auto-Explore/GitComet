use super::common::*;

pub(crate) fn bench_diff_open_patch_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let sidecar_started_at = Instant::now();
    let metrics = measure_sidecar_allocations(|| fixture.measure_paged_first_window_step(window));
    let first_window_ns = sidecar_started_at
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64;

    let mut group = c.benchmark_group("diff_open_patch_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.finish();
    emit_patch_diff_first_window_sidecar(window, first_window_ns, metrics);
}
