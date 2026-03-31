use super::common::*;

pub(crate) fn bench_open_repo(c: &mut Criterion) {
    // Note: Criterion's "Warming up for Xs" can look "stuck" if a single iteration takes longer
    // than the warm-up duration. Keep defaults moderate; scale up via env vars for stress runs.
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let history_heavy_commits = env_usize(
        "GITCOMET_BENCH_HISTORY_HEAVY_COMMITS",
        commits.saturating_mul(3),
    );
    let branch_heavy_local_branches = env_usize(
        "GITCOMET_BENCH_BRANCH_HEAVY_LOCAL_BRANCHES",
        local_branches.saturating_mul(6),
    );
    let branch_heavy_remote_branches = env_usize(
        "GITCOMET_BENCH_BRANCH_HEAVY_REMOTE_BRANCHES",
        remote_branches.saturating_mul(4),
    );
    let branch_heavy_remotes = env_usize("GITCOMET_BENCH_BRANCH_HEAVY_REMOTES", remotes.max(8));
    let extreme_fanout_commits = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_COMMITS", 1_000);
    let extreme_fanout_local_branches =
        env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_LOCAL_BRANCHES", 1_000);
    let extreme_fanout_remote_branches =
        env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_REMOTE_BRANCHES", 10_000);
    let extreme_fanout_remotes = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_REMOTES", 1);
    let extreme_fanout_worktrees = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_WORKTREES", 5_000);
    let extreme_fanout_submodules = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_SUBMODULES", 1_000);

    let balanced = OpenRepoFixture::new(commits, local_branches, remote_branches, remotes);
    let history_heavy = OpenRepoFixture::new(
        history_heavy_commits,
        local_branches.max(8) / 2,
        remote_branches.max(16) / 2,
        remotes.max(1),
    );
    let branch_heavy = OpenRepoFixture::new(
        commits.max(500) / 5,
        branch_heavy_local_branches,
        branch_heavy_remote_branches,
        branch_heavy_remotes,
    );
    let extreme_metadata_fanout = OpenRepoFixture::with_sidebar_fanout(
        extreme_fanout_commits,
        extreme_fanout_local_branches,
        extreme_fanout_remote_branches,
        extreme_fanout_remotes,
        extreme_fanout_worktrees,
        extreme_fanout_submodules,
    );

    let mut group = c.benchmark_group("open_repo");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("balanced"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = balanced.run_with_metrics();
            }
            let elapsed = start.elapsed();
            let (_, metrics) = measure_sidecar_allocations(|| balanced.run_with_metrics());
            emit_open_repo_sidecar("balanced", &metrics);
            elapsed
        })
    });
    group.bench_function(BenchmarkId::from_parameter("history_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = history_heavy.run_with_metrics();
            }
            let elapsed = start.elapsed();
            let (_, metrics) = measure_sidecar_allocations(|| history_heavy.run_with_metrics());
            emit_open_repo_sidecar("history_heavy", &metrics);
            elapsed
        })
    });
    group.bench_function(BenchmarkId::from_parameter("branch_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = branch_heavy.run_with_metrics();
            }
            let elapsed = start.elapsed();
            let (_, metrics) = measure_sidecar_allocations(|| branch_heavy.run_with_metrics());
            emit_open_repo_sidecar("branch_heavy", &metrics);
            elapsed
        })
    });
    group.bench_function(
        BenchmarkId::from_parameter("extreme_metadata_fanout"),
        |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    let _ = extreme_metadata_fanout.run_with_metrics();
                }
                let elapsed = start.elapsed();
                let (_, metrics) =
                    measure_sidecar_allocations(|| extreme_metadata_fanout.run_with_metrics());
                emit_open_repo_sidecar("extreme_metadata_fanout", &metrics);
                elapsed
            })
        },
    );
    group.finish();
}
