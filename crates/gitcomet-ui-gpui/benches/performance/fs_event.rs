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
        |b| b.iter_custom(|iters| time_status_refresh(&single_save, iters)),
    );
    group.bench_function(
        BenchmarkId::from_parameter("git_checkout_200_files_to_status_update"),
        |b| b.iter_custom(|iters| time_status_refresh(&checkout_batch, iters)),
    );
    group.bench_function(
        BenchmarkId::from_parameter("rapid_saves_debounce_coalesce"),
        |b| b.iter_custom(|iters| time_status_refresh(&rapid_saves, iters)),
    );
    group.bench_function(
        BenchmarkId::from_parameter("false_positive_rate_under_churn"),
        |b| b.iter_custom(|iters| time_status_refresh(&false_positive, iters)),
    );
    group.finish();

    // Sidecar allocations cover the refresh alone, like the timings.
    let single_save_metrics = measure_refresh_allocations(&single_save);
    emit_fs_event_sidecar("single_file_save_to_status_update", &single_save_metrics);
    let checkout_metrics = measure_refresh_allocations(&checkout_batch);
    emit_fs_event_sidecar("git_checkout_200_files_to_status_update", &checkout_metrics);
    let rapid_metrics = measure_refresh_allocations(&rapid_saves);
    emit_fs_event_sidecar("rapid_saves_debounce_coalesce", &rapid_metrics);
    let fp_metrics = measure_refresh_allocations(&false_positive);
    emit_fs_event_sidecar("false_positive_rate_under_churn", &fp_metrics);
}

/// Only the status refresh is timed; the disk writes that trigger it and the
/// restoration afterwards are setup.
fn time_status_refresh(fixture: &FsEventFixture, iters: u64) -> Duration {
    let mut elapsed = Duration::ZERO;
    for _ in 0..iters {
        let mutation = fixture.apply_mutation();
        let started = Instant::now();
        let refreshed = fixture.refresh_status(&mutation);
        elapsed += started.elapsed();
        std::hint::black_box(refreshed);
        fixture.restore(mutation);
    }
    elapsed
}

fn measure_refresh_allocations(fixture: &FsEventFixture) -> FsEventMetrics {
    let mutation = fixture.apply_mutation();
    let (_, metrics) = measure_sidecar_allocations(|| fixture.refresh_status(&mutation));
    fixture.restore(mutation);
    metrics
}
