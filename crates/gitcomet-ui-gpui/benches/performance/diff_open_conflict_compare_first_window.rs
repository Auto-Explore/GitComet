use super::common::*;

pub(crate) fn bench_diff_open_conflict_compare_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_COMPARE_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_COMPARE_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_COMPARE_WINDOW", 200);
    let fixture = ConflictTwoWaySplitScrollFixture::new(lines, conflict_blocks);

    let metrics = measure_sidecar_allocations(|| fixture.measure_first_window(window));

    let mut group = c.benchmark_group("diff_open_conflict_compare_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_scroll_step(0, window)),
    );
    group.finish();
    emit_conflict_compare_first_window_sidecar(window, &metrics);
}
