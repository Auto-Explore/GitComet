use super::common::*;

pub(crate) fn bench_conflict_streamed_provider(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_STREAMED_LINES", 50_000);
    let window = env_usize("GITCOMET_BENCH_STREAMED_WINDOW", 200);

    let fixture = ConflictStreamedProviderFixture::new(lines);

    let mut group = c.benchmark_group("conflict_streamed_provider");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("index_build"), |b| {
        b.iter(|| fixture.run_index_build_step())
    });
    group.bench_function(BenchmarkId::from_parameter("projection_build"), |b| {
        b.iter(|| fixture.run_projection_build_step())
    });
    group.bench_with_input(BenchmarkId::new("first_page", window), &window, |b, &w| {
        b.iter(|| fixture.run_first_page_step(w))
    });
    fixture.prime_first_page_cache(window);
    group.bench_with_input(
        BenchmarkId::new("first_page_cache_hit", window),
        &window,
        |b, &w| b.iter(|| fixture.run_first_page_cache_hit_step(w)),
    );
    group.bench_with_input(
        BenchmarkId::new("deep_scroll_50pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_scroll_step(0.5, w)),
    );
    group.bench_with_input(
        BenchmarkId::new("deep_scroll_90pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_scroll_step(0.9, w)),
    );
    group.bench_function(BenchmarkId::from_parameter("search_rare_text"), |b| {
        b.iter(|| fixture.run_search_step("shared_42("))
    });
    group.bench_function(BenchmarkId::from_parameter("search_common_text"), |b| {
        b.iter(|| fixture.run_search_step("compute"))
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_index_build_step());
    emit_allocation_only_sidecar("conflict_streamed_provider/index_build");
    let _ = measure_sidecar_allocations(|| fixture.run_projection_build_step());
    emit_allocation_only_sidecar("conflict_streamed_provider/projection_build");
    let _ = measure_sidecar_allocations(|| fixture.run_first_page_step(window));
    emit_allocation_only_sidecar(&format!("conflict_streamed_provider/first_page/{window}"));
    fixture.prime_first_page_cache(window);
    let _ = measure_sidecar_allocations(|| fixture.run_first_page_cache_hit_step(window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_provider/first_page_cache_hit/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_deep_scroll_step(0.5, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_provider/deep_scroll_50pct/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_deep_scroll_step(0.9, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_provider/deep_scroll_90pct/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_search_step("shared_42("));
    emit_allocation_only_sidecar("conflict_streamed_provider/search_rare_text");
    let _ = measure_sidecar_allocations(|| fixture.run_search_step("compute"));
    emit_allocation_only_sidecar("conflict_streamed_provider/search_common_text");
}
