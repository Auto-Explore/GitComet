use super::common::*;

pub(crate) fn bench_branch_sidebar_extreme_scale(c: &mut Criterion) {
    let extreme_scale = BranchSidebarFixture::twenty_thousand_branches_hundred_remotes();
    let (_, metrics) = measure_sidecar_allocations(|| extreme_scale.run_with_metrics());

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("20k_branches_100_remotes"),
        |b| b.iter(|| extreme_scale.run()),
    );
    group.finish();

    emit_branch_sidebar_sidecar("20k_branches_100_remotes", &metrics);
}
