use super::common::*;

pub(crate) fn bench_patch_diff_paged_rows(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let mut group = c.benchmark_group("patch_diff_paged_rows");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("eager_full_materialize"), |b| {
        b.iter(|| fixture.run_eager_full_materialize_step())
    });
    group.bench_with_input(
        BenchmarkId::new("paged_first_window", window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.bench_function(
        BenchmarkId::from_parameter("inline_visible_eager_scan"),
        |b| b.iter(|| fixture.run_inline_visible_eager_scan_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("inline_visible_hidden_map"),
        |b| b.iter(|| fixture.run_inline_visible_hidden_map_step()),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_eager_full_materialize_step());
    emit_allocation_only_sidecar("patch_diff_paged_rows/eager_full_materialize");
    let _ = measure_sidecar_allocations(|| fixture.run_paged_first_window_step(window));
    emit_allocation_only_sidecar(&format!(
        "patch_diff_paged_rows/paged_first_window/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_inline_visible_eager_scan_step());
    emit_allocation_only_sidecar("patch_diff_paged_rows/inline_visible_eager_scan");
    let _ = measure_sidecar_allocations(|| fixture.run_inline_visible_hidden_map_step());
    emit_allocation_only_sidecar("patch_diff_paged_rows/inline_visible_hidden_map");
}
