use super::common::*;

pub(crate) fn bench_conflict_three_way_prepared_syntax_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let fixture =
        ConflictThreeWayScrollFixture::new_with_prepared_documents(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_prepared_syntax_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("style_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_prepared_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_prepared_scroll_step(0, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_three_way_prepared_syntax_scroll/style_window/{window}"
    ));
}
