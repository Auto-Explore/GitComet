use super::common::*;

pub(crate) fn bench_branch_sidebar_cache(c: &mut Criterion) {
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let worktrees = env_usize("GITCOMET_BENCH_WORKTREES", 80);
    let submodules = env_usize("GITCOMET_BENCH_SUBMODULES", 150);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 300);

    let mut cache_hit_balanced = BranchSidebarCacheFixture::balanced(
        local_branches,
        remote_branches,
        remotes,
        worktrees,
        submodules,
        stashes,
    );
    // Warm the cache with an initial build.
    cache_hit_balanced.run_cached();
    cache_hit_balanced.reset_metrics();

    let mut cache_miss_remote_fanout = BranchSidebarCacheFixture::remote_fanout(
        local_branches.max(32) / 4,
        remote_branches.saturating_mul(6),
        remotes.max(12),
    );

    let mut cache_invalidation =
        BranchSidebarCacheFixture::balanced(local_branches, remote_branches, remotes, 0, 0, 0);
    // Warm the cache so each iteration measures a source-changing rebuild.
    cache_invalidation.run_cached();
    cache_invalidation.reset_metrics();

    // Worktrees-ready invalidation: keep the aux sections expanded and mutate
    // the worktree snapshot so the rebuild reflects the full sidebar shape.
    let mut cache_invalidation_wt = BranchSidebarCacheFixture::balanced(
        local_branches,
        remote_branches,
        remotes,
        worktrees,
        submodules,
        stashes,
    );
    cache_invalidation_wt.run_cached();
    cache_invalidation_wt.reset_metrics();

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("cache_hit_balanced"), |b| {
        b.iter_custom(|iters| {
            cache_hit_balanced.reset_metrics();
            let start = Instant::now();
            for _ in 0..iters {
                cache_hit_balanced.run_cached();
            }
            let elapsed = start.elapsed();
            cache_hit_balanced.reset_metrics();
            measure_sidecar_allocations(|| {
                cache_hit_balanced.run_cached();
            });
            cache_hit_balanced.capture_cached_row_breakdown();
            emit_branch_sidebar_cache_sidecar("cache_hit_balanced", &cache_hit_balanced.metrics());
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("cache_miss_remote_fanout"),
        |b| {
            b.iter_custom(|iters| {
                cache_miss_remote_fanout.reset_metrics();
                let start = Instant::now();
                for _ in 0..iters {
                    cache_miss_remote_fanout.run_rebuild_remote_fanout();
                }
                let elapsed = start.elapsed();
                cache_miss_remote_fanout.reset_metrics();
                measure_sidecar_allocations(|| {
                    cache_miss_remote_fanout.run_rebuild_remote_fanout();
                });
                cache_miss_remote_fanout.capture_cached_row_breakdown();
                emit_branch_sidebar_cache_sidecar(
                    "cache_miss_remote_fanout",
                    &cache_miss_remote_fanout.metrics(),
                );
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("cache_invalidation_single_ref_change"),
        |b| {
            b.iter_custom(|iters| {
                cache_invalidation.reset_metrics();
                let start = Instant::now();
                for _ in 0..iters {
                    cache_invalidation.run_rebuild_single_ref_change();
                }
                let elapsed = start.elapsed();
                cache_invalidation.reset_metrics();
                measure_sidecar_allocations(|| {
                    cache_invalidation.run_rebuild_single_ref_change();
                });
                cache_invalidation.capture_cached_row_breakdown();
                emit_branch_sidebar_cache_sidecar(
                    "cache_invalidation_single_ref_change",
                    &cache_invalidation.metrics(),
                );
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("cache_invalidation_worktrees_ready"),
        |b| {
            b.iter_custom(|iters| {
                cache_invalidation_wt.reset_metrics();
                let start = Instant::now();
                for _ in 0..iters {
                    cache_invalidation_wt.run_rebuild_worktrees_ready();
                }
                let elapsed = start.elapsed();
                cache_invalidation_wt.reset_metrics();
                measure_sidecar_allocations(|| {
                    cache_invalidation_wt.run_rebuild_worktrees_ready();
                });
                cache_invalidation_wt.capture_cached_row_breakdown();
                emit_branch_sidebar_cache_sidecar(
                    "cache_invalidation_worktrees_ready",
                    &cache_invalidation_wt.metrics(),
                );
                elapsed
            });
        },
    );

    group.finish();
}
