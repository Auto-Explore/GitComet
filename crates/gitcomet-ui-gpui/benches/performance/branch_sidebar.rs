use super::common::*;

pub(crate) fn bench_branch_sidebar(c: &mut Criterion) {
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let worktrees = env_usize("GITCOMET_BENCH_WORKTREES", 80);
    let submodules = env_usize("GITCOMET_BENCH_SUBMODULES", 150);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 300);

    let local_heavy = BranchSidebarFixture::new(
        local_branches.saturating_mul(8),
        remote_branches.max(32) / 8,
        remotes.max(1),
        0,
        0,
        0,
    );
    let remote_fanout = BranchSidebarFixture::new(
        local_branches.max(32) / 4,
        remote_branches.saturating_mul(6),
        remotes.max(12),
        0,
        0,
        0,
    );
    let aux_lists_heavy = BranchSidebarFixture::new(
        local_branches,
        remote_branches,
        remotes.max(2),
        worktrees,
        submodules,
        stashes,
    );

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("local_heavy"), |b| {
        b.iter(|| local_heavy.run())
    });
    group.bench_function(BenchmarkId::from_parameter("remote_fanout"), |b| {
        b.iter(|| remote_fanout.run())
    });
    group.bench_function(BenchmarkId::from_parameter("aux_lists_heavy"), |b| {
        b.iter(|| aux_lists_heavy.run())
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| local_heavy.run());
    emit_allocation_only_sidecar("branch_sidebar/local_heavy");
    let _ = measure_sidecar_allocations(|| remote_fanout.run());
    emit_allocation_only_sidecar("branch_sidebar/remote_fanout");
    let _ = measure_sidecar_allocations(|| aux_lists_heavy.run());
    emit_allocation_only_sidecar("branch_sidebar/aux_lists_heavy");
}
