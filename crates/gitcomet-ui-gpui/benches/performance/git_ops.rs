use super::common::*;

pub(crate) fn bench_git_ops(c: &mut Criterion) {
    let status_files = env_usize("GITCOMET_BENCH_GIT_STATUS_FILES", 1_000);
    let status_dirty_files = env_usize("GITCOMET_BENCH_GIT_STATUS_DIRTY_FILES", 500);
    let log_commits = env_usize("GITCOMET_BENCH_GIT_LOG_COMMITS", 10_000);
    let log_shallow_total_commits =
        env_usize("GITCOMET_BENCH_GIT_LOG_SHALLOW_TOTAL_COMMITS", 100_000);
    let log_shallow_requested_commits =
        env_usize("GITCOMET_BENCH_GIT_LOG_SHALLOW_REQUESTED_COMMITS", 200);
    let status_clean_files = env_usize("GITCOMET_BENCH_GIT_STATUS_CLEAN_FILES", 10_000);
    let ref_count = env_usize("GITCOMET_BENCH_GIT_REF_COUNT", 10_000);
    let diff_rename_files = env_usize("GITCOMET_BENCH_GIT_DIFF_RENAME_FILES", 256);
    let diff_binary_files = env_usize("GITCOMET_BENCH_GIT_DIFF_BINARY_FILES", 128);
    let diff_binary_bytes = env_usize("GITCOMET_BENCH_GIT_DIFF_BINARY_BYTES", 4_096);
    let diff_large_file_lines = env_usize("GITCOMET_BENCH_GIT_DIFF_LARGE_FILE_LINES", 100_000);
    let diff_large_file_line_bytes = env_usize("GITCOMET_BENCH_GIT_DIFF_LARGE_FILE_LINE_BYTES", 48);
    let blame_large_file_lines = env_usize("GITCOMET_BENCH_GIT_BLAME_LINES", 100_000);
    let blame_large_file_commits = env_usize("GITCOMET_BENCH_GIT_BLAME_COMMITS", 16);
    let file_history_total_commits =
        env_usize("GITCOMET_BENCH_GIT_FILE_HISTORY_TOTAL_COMMITS", 100_000);
    let file_history_requested_commits =
        env_usize("GITCOMET_BENCH_GIT_FILE_HISTORY_REQUESTED_COMMITS", 200);
    let file_history_touch_every = env_usize("GITCOMET_BENCH_GIT_FILE_HISTORY_TOUCH_EVERY", 10);

    let status_dirty = GitOpsFixture::status_dirty(status_files, status_dirty_files);
    let log_walk = GitOpsFixture::log_walk(log_commits, log_commits);
    let log_walk_shallow =
        GitOpsFixture::log_walk(log_shallow_total_commits, log_shallow_requested_commits);
    let status_clean = GitOpsFixture::status_clean(status_clean_files);
    let ref_enumerate = GitOpsFixture::ref_enumerate(ref_count);
    let diff_rename = GitOpsFixture::diff_rename_heavy(diff_rename_files);
    let diff_binary = GitOpsFixture::diff_binary_heavy(diff_binary_files, diff_binary_bytes);
    let diff_large_single_file =
        GitOpsFixture::diff_large_single_file(diff_large_file_lines, diff_large_file_line_bytes);
    let blame_large_file =
        GitOpsFixture::blame_large_file(blame_large_file_lines, blame_large_file_commits);
    let file_history = GitOpsFixture::file_history(
        file_history_total_commits,
        file_history_requested_commits,
        file_history_touch_every,
    );

    let mut group = c.benchmark_group("git_ops");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("status_dirty_500_files"), |b| {
        b.iter(|| status_dirty.run())
    });
    group.bench_function(BenchmarkId::from_parameter("log_walk_10k_commits"), |b| {
        b.iter(|| log_walk.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("log_walk_100k_commits_shallow"),
        |b| b.iter(|| log_walk_shallow.run()),
    );
    group.bench_function(BenchmarkId::from_parameter("status_clean_10k_files"), |b| {
        b.iter(|| status_clean.run())
    });
    group.bench_function(BenchmarkId::from_parameter("ref_enumerate_10k_refs"), |b| {
        b.iter(|| ref_enumerate.run())
    });
    group.bench_function(BenchmarkId::from_parameter("diff_rename_heavy"), |b| {
        b.iter(|| diff_rename.run())
    });
    group.bench_function(BenchmarkId::from_parameter("diff_binary_heavy"), |b| {
        b.iter(|| diff_binary.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("diff_large_single_file_100k_lines"),
        |b| b.iter(|| diff_large_single_file.run()),
    );
    group.bench_function(BenchmarkId::from_parameter("blame_large_file"), |b| {
        b.iter(|| blame_large_file.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("file_history_first_page_sparse_100k_commits"),
        |b| b.iter(|| file_history.run()),
    );
    group.finish();

    let (_, status_metrics) = measure_sidecar_allocations(|| status_dirty.run_with_metrics());
    let (_, log_metrics) = measure_sidecar_allocations(|| log_walk.run_with_metrics());
    let (_, log_shallow_metrics) =
        measure_sidecar_allocations(|| log_walk_shallow.run_with_metrics());
    let (_, status_clean_metrics) = measure_sidecar_allocations(|| status_clean.run_with_metrics());
    let (_, ref_enumerate_metrics) =
        measure_sidecar_allocations(|| ref_enumerate.run_with_metrics());
    let (_, diff_rename_metrics) = measure_sidecar_allocations(|| diff_rename.run_with_metrics());
    let (_, diff_binary_metrics) = measure_sidecar_allocations(|| diff_binary.run_with_metrics());
    let (_, diff_large_metrics) =
        measure_sidecar_allocations(|| diff_large_single_file.run_with_metrics());
    let (_, blame_large_metrics) =
        measure_sidecar_allocations(|| blame_large_file.run_with_metrics());
    let (_, file_history_metrics) = measure_sidecar_allocations(|| file_history.run_with_metrics());
    emit_git_ops_sidecar("status_dirty_500_files", &status_metrics);
    emit_git_ops_sidecar("log_walk_10k_commits", &log_metrics);
    emit_git_ops_sidecar("log_walk_100k_commits_shallow", &log_shallow_metrics);
    emit_git_ops_sidecar("status_clean_10k_files", &status_clean_metrics);
    emit_git_ops_sidecar("ref_enumerate_10k_refs", &ref_enumerate_metrics);
    emit_git_ops_sidecar("diff_rename_heavy", &diff_rename_metrics);
    emit_git_ops_sidecar("diff_binary_heavy", &diff_binary_metrics);
    emit_git_ops_sidecar("diff_large_single_file_100k_lines", &diff_large_metrics);
    emit_git_ops_sidecar("blame_large_file", &blame_large_metrics);
    emit_git_ops_sidecar(
        "file_history_first_page_sparse_100k_commits",
        &file_history_metrics,
    );
}
