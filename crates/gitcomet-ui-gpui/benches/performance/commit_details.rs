use super::common::*;

pub(crate) fn bench_commit_details(c: &mut Criterion) {
    let files = env_usize("GITCOMET_BENCH_COMMIT_FILES", 5_000);
    let depth = env_usize("GITCOMET_BENCH_COMMIT_PATH_DEPTH", 4);
    let deep_depth = env_usize(
        "GITCOMET_BENCH_COMMIT_DEEP_PATH_DEPTH",
        depth.saturating_mul(4).max(12),
    );
    let huge_files = env_usize("GITCOMET_BENCH_COMMIT_HUGE_FILES", files.saturating_mul(2));
    let large_message_files = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_FILES", files.max(1));
    let large_message_depth = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_DEPTH", depth.max(1));
    let large_message_bytes = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_BYTES", 96 * 1024);
    let large_message_line_bytes = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_LINE_BYTES", 192);
    let large_message_visible_lines =
        env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_VISIBLE_LINES", 48);
    let large_message_wrap_width_px =
        env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_WRAP_WIDTH_PX", 560);
    let extreme_files = env_usize("GITCOMET_BENCH_COMMIT_EXTREME_FILES", 10_000);
    let extreme_depth = env_usize("GITCOMET_BENCH_COMMIT_EXTREME_DEPTH", 12);
    let balanced = CommitDetailsFixture::new(files, depth);
    let deep_paths = CommitDetailsFixture::new(files, deep_depth);
    let huge_list = CommitDetailsFixture::new(huge_files, depth);
    let large_message = CommitDetailsFixture::large_message_body(
        large_message_files,
        large_message_depth,
        large_message_bytes,
        large_message_line_bytes,
        large_message_visible_lines,
        large_message_wrap_width_px,
    );
    let extreme_scale = CommitDetailsFixture::new(extreme_files, extreme_depth);
    let select_replace = CommitSelectReplaceFixture::new(files, depth);

    let churn_files = env_usize("GITCOMET_BENCH_PATH_CHURN_FILES", 10_000);
    let churn_depth = env_usize("GITCOMET_BENCH_PATH_CHURN_DEPTH", 6);
    let mut path_churn = PathDisplayCacheChurnFixture::new(churn_files, churn_depth);

    balanced.prewarm_runtime_state();
    deep_paths.prewarm_runtime_state();
    huge_list.prewarm_runtime_state();
    large_message.prewarm_runtime_state();
    extreme_scale.prewarm_runtime_state();

    // Collect sidecar metrics before the timed benchmark loop.
    let (_, balanced_metrics) = measure_sidecar_allocations(|| balanced.run_with_metrics());
    let (_, deep_metrics) = measure_sidecar_allocations(|| deep_paths.run_with_metrics());
    let (_, huge_metrics) = measure_sidecar_allocations(|| huge_list.run_with_metrics());
    let (_, large_message_metrics) =
        measure_sidecar_allocations(|| large_message.run_with_metrics());
    let (_, extreme_scale_metrics) =
        measure_sidecar_allocations(|| extreme_scale.run_with_metrics());
    let (_, select_replace_metrics) =
        measure_sidecar_allocations(|| select_replace.run_with_metrics());
    let (_, churn_metrics) = measure_sidecar_allocations(|| path_churn.run_with_metrics());

    let mut group = c.benchmark_group("commit_details");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("many_files"), |b| {
        b.iter(|| balanced.run())
    });
    group.bench_function(BenchmarkId::from_parameter("deep_paths"), |b| {
        b.iter(|| deep_paths.run())
    });
    group.bench_function(BenchmarkId::from_parameter("huge_file_list"), |b| {
        b.iter(|| huge_list.run())
    });
    group.bench_function(BenchmarkId::from_parameter("large_message_body"), |b| {
        b.iter(|| large_message.run())
    });
    group.bench_function(BenchmarkId::from_parameter("10k_files_depth_12"), |b| {
        b.iter(|| extreme_scale.run())
    });
    group.bench_function(BenchmarkId::from_parameter("select_commit_replace"), |b| {
        b.iter(|| select_replace.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("path_display_cache_churn"),
        |b| {
            b.iter(|| {
                path_churn.reset_runtime_state();
                path_churn.run()
            })
        },
    );
    group.finish();

    emit_commit_details_sidecar("many_files", &balanced_metrics);
    emit_commit_details_sidecar("deep_paths", &deep_metrics);
    emit_commit_details_sidecar("huge_file_list", &huge_metrics);
    emit_commit_details_sidecar("large_message_body", &large_message_metrics);
    emit_commit_details_sidecar("10k_files_depth_12", &extreme_scale_metrics);
    emit_commit_select_replace_sidecar("select_commit_replace", &select_replace_metrics);
    emit_path_display_cache_churn_sidecar("path_display_cache_churn", &churn_metrics);
}
