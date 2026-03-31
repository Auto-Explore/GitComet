use super::common::*;

pub(crate) fn bench_conflict_two_way_split_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictTwoWaySplitScrollFixture::new(lines, conflict_blocks);
    let windows = [100usize, 200, 400];

    let mut group = c.benchmark_group("conflict_two_way_split_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for &window in &windows {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("window_{window}")),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let h = fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(window) % fixture.visible_rows().max(1);
                    h
                })
            },
        );
    }
    group.finish();

    for &window in &windows {
        let _ = measure_sidecar_allocations(|| fixture.run_scroll_step(0, window));
        emit_allocation_only_sidecar(&format!("conflict_two_way_split_scroll/window_{window}"));
    }
}
