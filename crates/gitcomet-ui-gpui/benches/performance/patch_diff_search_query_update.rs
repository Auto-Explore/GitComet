use super::common::*;

pub(crate) fn bench_patch_diff_search_query_update(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 10_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let mut fixture = PatchDiffSearchQueryUpdateFixture::new(lines);
    let query_cycle = [
        "s", "sh", "sha", "shar", "share", "shared", "shared_", "shared_1",
    ];

    let mut group = c.benchmark_group("patch_diff_search_query_update");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("window_{window}")),
        &window,
        |b, &window| {
            let mut start = 0usize;
            let mut query_ix = 0usize;
            b.iter(|| {
                let query = query_cycle[query_ix % query_cycle.len()];
                let h = fixture.run_query_update_step(query, start, window);
                query_ix = query_ix.wrapping_add(1);
                start = start.wrapping_add(window.max(1) / 2 + 1) % fixture.visible_rows().max(1);
                h
            })
        },
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_query_update_step("shared_1", 0, window));
    emit_allocation_only_sidecar(&format!("patch_diff_search_query_update/window_{window}"));
}
