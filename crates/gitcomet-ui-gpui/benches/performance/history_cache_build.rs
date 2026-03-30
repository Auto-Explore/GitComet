use super::common::*;

pub(crate) fn bench_history_cache_build(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let tags = env_usize("GITCOMET_BENCH_TAGS", 50);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 20);

    let balanced =
        HistoryCacheBuildFixture::balanced(commits, local_branches, remote_branches, tags, stashes);
    let merge_dense = HistoryCacheBuildFixture::merge_dense(commits);
    let decorated_refs_heavy = HistoryCacheBuildFixture::decorated_refs_heavy(
        commits,
        local_branches.saturating_mul(10),
        remote_branches.saturating_mul(5),
        tags.saturating_mul(40),
    );
    let stash_heavy = HistoryCacheBuildFixture::stash_heavy(commits, stashes.saturating_mul(10));

    let mut group = c.benchmark_group("history_cache_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("balanced"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = balanced.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| balanced.run());
            emit_history_cache_build_sidecar("balanced", &metrics);
            start.elapsed()
        });
    });

    group.bench_function(BenchmarkId::from_parameter("merge_dense"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = merge_dense.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| merge_dense.run());
            emit_history_cache_build_sidecar("merge_dense", &metrics);
            start.elapsed()
        });
    });

    group.bench_function(BenchmarkId::from_parameter("decorated_refs_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = decorated_refs_heavy.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| decorated_refs_heavy.run());
            emit_history_cache_build_sidecar("decorated_refs_heavy", &metrics);
            start.elapsed()
        });
    });

    group.bench_function(BenchmarkId::from_parameter("stash_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = stash_heavy.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| stash_heavy.run());
            emit_history_cache_build_sidecar("stash_heavy", &metrics);
            start.elapsed()
        });
    });

    group.finish();
}
