use super::common::*;

pub(crate) fn bench_status_list(c: &mut Criterion) {
    let entries = env_usize("GITCOMET_BENCH_STATUS_ENTRIES", 10_000);
    let window = env_usize("GITCOMET_BENCH_STATUS_WINDOW", 200);
    let mixed_depth_entries = env_usize("GITCOMET_BENCH_STATUS_MIXED_DEPTH_ENTRIES", 20_000);
    let mixed_depth_prewarm = env_usize("GITCOMET_BENCH_STATUS_MIXED_DEPTH_PREWARM", 8_193);
    let mut unstaged_large = StatusListFixture::unstaged_large(entries);
    let mut staged_large = StatusListFixture::staged_large(entries);
    let mut mixed_depth = StatusListFixture::mixed_depth(mixed_depth_entries);
    let unstaged_metrics =
        measure_sidecar_allocations(|| unstaged_large.measure_window_step(0, window));
    let staged_metrics =
        measure_sidecar_allocations(|| staged_large.measure_window_step(0, window));
    let mixed_depth_metrics = measure_sidecar_allocations(|| {
        mixed_depth.measure_window_step_with_prewarm(
            mixed_depth_prewarm,
            window,
            mixed_depth_prewarm,
        )
    });

    let mut group = c.benchmark_group("status_list");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("unstaged_large"), |b| {
        b.iter(|| {
            unstaged_large.reset_runtime_state();
            unstaged_large.run_window_step(0, window)
        })
    });
    group.bench_function(BenchmarkId::from_parameter("staged_large"), |b| {
        b.iter(|| {
            staged_large.reset_runtime_state();
            staged_large.run_window_step(0, window)
        })
    });
    group.bench_function(
        BenchmarkId::from_parameter("20k_entries_mixed_depth"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    mixed_depth.reset_runtime_state();
                    mixed_depth.prewarm_cache(mixed_depth_prewarm);
                    let started_at = Instant::now();
                    let _ = mixed_depth.run_window_step(mixed_depth_prewarm, window);
                    elapsed += started_at.elapsed();
                }
                elapsed
            })
        },
    );
    group.finish();

    emit_status_list_sidecar("unstaged_large", &unstaged_metrics);
    emit_status_list_sidecar("staged_large", &staged_metrics);
    emit_status_list_sidecar("20k_entries_mixed_depth", &mixed_depth_metrics);
}
