use super::common::*;

pub(crate) fn bench_staging(c: &mut Criterion) {
    let stage_files = env_usize("GITCOMET_BENCH_STAGING_FILES", 10_000);
    let interleaved_files = env_usize("GITCOMET_BENCH_STAGING_INTERLEAVED_FILES", 1_000);

    let stage_all = StagingFixture::stage_all(stage_files);
    let unstage_all = StagingFixture::unstage_all(stage_files);
    let interleaved = StagingFixture::interleaved(interleaved_files);

    let mut group = c.benchmark_group("staging");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("stage_all_10k_files"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = stage_all.fresh_state();
                let started_at = Instant::now();
                let _ = stage_all.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = stage_all.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| stage_all.run_with_state(&mut sidecar_state));
            emit_staging_sidecar("stage_all_10k_files", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("unstage_all_10k_files"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = unstage_all.fresh_state();
                let started_at = Instant::now();
                let _ = unstage_all.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = unstage_all.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| unstage_all.run_with_state(&mut sidecar_state));
            emit_staging_sidecar("unstage_all_10k_files", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("stage_unstage_interleaved_1k_files"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = interleaved.fresh_state();
                    let started_at = Instant::now();
                    let _ = interleaved.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = interleaved.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| interleaved.run_with_state(&mut sidecar_state));
                emit_staging_sidecar("stage_unstage_interleaved_1k_files", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}
