use super::common::*;

pub(crate) fn bench_history_cache_build_extreme_scale(c: &mut Criterion) {
    let extreme_scale = HistoryCacheBuildFixture::extreme_scale_50k_2k_refs_200_stashes();

    let mut group = c.benchmark_group("history_cache_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("50k_commits_2k_refs_200_stashes"),
        |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    let _ = extreme_scale.run();
                }
                let (_, metrics) = measure_sidecar_allocations(|| extreme_scale.run());
                emit_history_cache_build_sidecar("50k_commits_2k_refs_200_stashes", &metrics);
                start.elapsed()
            });
        },
    );
    group.finish();
}
