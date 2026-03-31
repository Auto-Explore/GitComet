use super::common::*;

pub(crate) fn bench_conflict_split_resize_step(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let resize_query =
        env::var("GITCOMET_BENCH_CONFLICT_RESIZE_QUERY").unwrap_or_else(|_| "shared".to_string());
    let mut fixture = ConflictSplitResizeStepFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_split_resize_step");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &window| {
        let mut start = 0usize;
        b.iter(|| {
            let h = fixture.run_resize_step(resize_query.as_str(), start, window);
            start = start.wrapping_add(window.max(1) / 3 + 1) % fixture.visible_rows().max(1);
            h
        })
    });
    group.finish();

    let _ =
        measure_sidecar_allocations(|| fixture.run_resize_step(resize_query.as_str(), 0, window));
    emit_allocation_only_sidecar(&format!("conflict_split_resize_step/window/{window}"));
}
