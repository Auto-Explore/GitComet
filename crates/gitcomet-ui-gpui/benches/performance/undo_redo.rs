use super::common::*;

pub(crate) fn bench_undo_redo(c: &mut Criterion) {
    let deep_stack_regions = env_usize("GITCOMET_BENCH_UNDO_REDO_DEEP_REGIONS", 200);
    let replay_regions = env_usize("GITCOMET_BENCH_UNDO_REDO_REPLAY_REGIONS", 50);

    let deep_stack = UndoRedoFixture::deep_stack(deep_stack_regions);
    let undo_replay = UndoRedoFixture::undo_replay(replay_regions);

    let mut group = c.benchmark_group("undo_redo");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("conflict_resolution_deep_stack"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = deep_stack.fresh_state();
                    let started_at = Instant::now();
                    let _ = deep_stack.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = deep_stack.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| deep_stack.run_with_state(&mut sidecar_state));
                emit_undo_redo_sidecar("conflict_resolution_deep_stack", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("conflict_resolution_undo_replay_50_steps"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = undo_replay.fresh_state();
                    let started_at = Instant::now();
                    let _ = undo_replay.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = undo_replay.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| undo_replay.run_with_state(&mut sidecar_state));
                emit_undo_redo_sidecar("conflict_resolution_undo_replay_50_steps", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}
