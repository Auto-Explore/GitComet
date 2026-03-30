use super::common::*;

pub(crate) fn bench_fs_event(c: &mut Criterion) {
    let tracked_files = env_usize("GITCOMET_BENCH_FS_EVENT_TRACKED_FILES", 1_000);
    let checkout_files = env_usize("GITCOMET_BENCH_FS_EVENT_CHECKOUT_FILES", 200);
    let rapid_save_count = env_usize("GITCOMET_BENCH_FS_EVENT_RAPID_SAVES", 50);
    let churn_files = env_usize("GITCOMET_BENCH_FS_EVENT_CHURN_FILES", 100);

    let single_save = FsEventFixture::single_file_save(tracked_files);
    let checkout_batch = FsEventFixture::git_checkout_batch(tracked_files, checkout_files);
    let rapid_saves = FsEventFixture::rapid_saves_debounce(tracked_files, rapid_save_count);
    let false_positive = FsEventFixture::false_positive_under_churn(tracked_files, churn_files);

    let mut group = c.benchmark_group("fs_event");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("single_file_save_to_status_update"),
        |b| b.iter(|| single_save.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("git_checkout_200_files_to_status_update"),
        |b| b.iter(|| checkout_batch.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("rapid_saves_debounce_coalesce"),
        |b| b.iter(|| rapid_saves.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("false_positive_rate_under_churn"),
        |b| b.iter(|| false_positive.run()),
    );
    group.finish();

    // Emit sidecar metrics from a final run.
    let (_, single_save_metrics) = measure_sidecar_allocations(|| single_save.run_with_metrics());
    emit_fs_event_sidecar("single_file_save_to_status_update", &single_save_metrics);
    let (_, checkout_metrics) = measure_sidecar_allocations(|| checkout_batch.run_with_metrics());
    emit_fs_event_sidecar("git_checkout_200_files_to_status_update", &checkout_metrics);
    let (_, rapid_metrics) = measure_sidecar_allocations(|| rapid_saves.run_with_metrics());
    emit_fs_event_sidecar("rapid_saves_debounce_coalesce", &rapid_metrics);
    let (_, fp_metrics) = measure_sidecar_allocations(|| false_positive.run_with_metrics());
    emit_fs_event_sidecar("false_positive_rate_under_churn", &fp_metrics);
}
