use super::common::*;

pub(crate) fn bench_history_scope_switch(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let fixture = HistoryScopeSwitchFixture::current_branch_to_all_refs(commits);

    let mut group = c.benchmark_group("history_scope_switch");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("current_branch_to_all_refs"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = fixture.fresh_state();
                    let started_at = Instant::now();
                    let _ = fixture.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = fixture.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| fixture.run_with_state(&mut sidecar_state));
                emit_history_scope_switch_sidecar("current_branch_to_all_refs", &metrics);
                elapsed
            });
        },
    );
    group.finish();
}
