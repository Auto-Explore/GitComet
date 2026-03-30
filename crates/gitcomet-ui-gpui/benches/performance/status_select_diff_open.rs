use super::common::*;

pub(crate) fn bench_status_select_diff_open(c: &mut Criterion) {
    let status_entries = env_usize("GITCOMET_BENCH_STATUS_SELECT_DIFF_ENTRIES", 10_000);

    let unstaged = StatusSelectDiffOpenFixture::unstaged(status_entries);
    let staged = StatusSelectDiffOpenFixture::staged(status_entries);

    let mut group = c.benchmark_group("status_select_diff_open");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("unstaged"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = unstaged.fresh_state();
                let started_at = Instant::now();
                let _ = unstaged.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = unstaged.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| unstaged.run_with_state(&mut sidecar_state));
            emit_status_select_diff_open_sidecar("unstaged", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("staged"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = staged.fresh_state();
                let started_at = Instant::now();
                let _ = staged.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = staged.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| staged.run_with_state(&mut sidecar_state));
            emit_status_select_diff_open_sidecar("staged", &metrics);
            elapsed
        });
    });

    group.finish();
}
