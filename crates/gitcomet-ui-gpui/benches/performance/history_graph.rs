use super::common::*;

pub(crate) fn bench_history_graph(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let merge_stride = env_usize("GITCOMET_BENCH_HISTORY_MERGE_EVERY", 50);
    let branch_head_every = env_usize("GITCOMET_BENCH_HISTORY_BRANCH_HEAD_EVERY", 11);

    let linear_history = HistoryGraphFixture::new(commits, 0, 0);
    let merge_dense = HistoryGraphFixture::new(commits, merge_stride.clamp(5, 25), 0);
    let branch_heads_dense =
        HistoryGraphFixture::new(commits, merge_stride.max(1), branch_head_every.max(2));

    // Collect sidecar metrics before the timed benchmark loop.
    let (_, linear_metrics) = measure_sidecar_allocations(|| linear_history.run_with_metrics());
    let (_, merge_metrics) = measure_sidecar_allocations(|| merge_dense.run_with_metrics());
    let (_, branch_metrics) = measure_sidecar_allocations(|| branch_heads_dense.run_with_metrics());

    let mut group = c.benchmark_group("history_graph");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("linear_history"), |b| {
        b.iter(|| linear_history.run())
    });
    group.bench_function(BenchmarkId::from_parameter("merge_dense"), |b| {
        b.iter(|| merge_dense.run())
    });
    group.bench_function(BenchmarkId::from_parameter("branch_heads_dense"), |b| {
        b.iter(|| branch_heads_dense.run())
    });
    group.finish();

    emit_history_graph_sidecar("linear_history", &linear_metrics);
    emit_history_graph_sidecar("merge_dense", &merge_metrics);
    emit_history_graph_sidecar("branch_heads_dense", &branch_metrics);
}
