use super::common::*;

pub(crate) fn bench_real_repo(c: &mut Criterion) {
    let Some(snapshot_root) = env_string("GITCOMET_PERF_REAL_REPO_ROOT") else {
        if !env_flag(SUPPRESS_MISSING_REAL_REPO_NOTICE_ENV) {
            eprintln!("skipping real_repo benchmarks: GITCOMET_PERF_REAL_REPO_ROOT is not set");
        }
        return;
    };

    let mut group = c.benchmark_group("real_repo");
    group.sample_size(10);

    let monorepo = RealRepoFixture::from_snapshot_root(
        &snapshot_root,
        RealRepoScenario::MonorepoOpenAndHistoryLoad,
    )
    .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("monorepo_open_and_history_load", |b| {
        b.iter(|| monorepo.run())
    });
    let (_, monorepo_metrics) = measure_sidecar_allocations(|| monorepo.run_with_metrics());
    emit_real_repo_sidecar("monorepo_open_and_history_load", &monorepo_metrics);

    let deep_history = RealRepoFixture::from_snapshot_root(
        &snapshot_root,
        RealRepoScenario::DeepHistoryOpenAndScroll,
    )
    .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("deep_history_open_and_scroll", |b| {
        b.iter(|| deep_history.run())
    });
    let (_, deep_history_metrics) = measure_sidecar_allocations(|| deep_history.run_with_metrics());
    emit_real_repo_sidecar("deep_history_open_and_scroll", &deep_history_metrics);

    let conflict = RealRepoFixture::from_snapshot_root(
        &snapshot_root,
        RealRepoScenario::MidMergeConflictListAndOpen,
    )
    .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("mid_merge_conflict_list_and_open", |b| {
        b.iter(|| conflict.run())
    });
    let (_, conflict_metrics) = measure_sidecar_allocations(|| conflict.run_with_metrics());
    emit_real_repo_sidecar("mid_merge_conflict_list_and_open", &conflict_metrics);

    let large_diff =
        RealRepoFixture::from_snapshot_root(&snapshot_root, RealRepoScenario::LargeFileDiffOpen)
            .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("large_file_diff_open", |b| b.iter(|| large_diff.run()));
    let (_, large_diff_metrics) = measure_sidecar_allocations(|| large_diff.run_with_metrics());
    emit_real_repo_sidecar("large_file_diff_open", &large_diff_metrics);

    group.finish();
}
