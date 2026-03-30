use super::common::*;

pub(crate) fn bench_diff_open_patch_100k_lines_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_100K_LINES", 100_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let sidecar_started_at = Instant::now();
    let metrics = measure_sidecar_allocations(|| fixture.measure_paged_first_window_step(window));
    let first_window_ns = sidecar_started_at
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64;

    let mut group = c.benchmark_group("diff_open_patch_100k_lines_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.finish();

    // Emit sidecar with the standard patch diff format.
    let mut payload = Map::new();
    payload.insert("first_window_ns".to_string(), json!(first_window_ns));
    payload.insert("rows_requested".to_string(), json!(metrics.rows_requested));
    payload.insert(
        "rows_painted".to_string(),
        json!(metrics.split_rows_painted),
    );
    payload.insert(
        "rows_materialized".to_string(),
        json!(metrics.split_rows_materialized),
    );
    payload.insert(
        "patch_page_cache_entries".to_string(),
        json!(metrics.patch_page_cache_entries),
    );
    payload.insert(
        "full_text_materializations".to_string(),
        json!(metrics.full_text_materializations),
    );
    emit_sidecar_metrics(
        &format!("diff_open_patch_100k_lines_first_window/{window}"),
        payload,
    );
}
