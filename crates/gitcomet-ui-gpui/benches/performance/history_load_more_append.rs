use super::common::*;

pub(crate) fn bench_history_load_more_append(c: &mut Criterion) {
    let fixture = HistoryLoadMoreAppendFixture::new(5_000, 500);

    let mut group = c.benchmark_group("history_load_more_append");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("page_500"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = fixture.fresh_state();
                let cursor = fixture.request_cursor();
                let page = fixture.append_page();
                let started_at = Instant::now();
                let _ = fixture.run_with_state_and_page(&mut state, cursor, page);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = fixture.fresh_state();
            let cursor = fixture.request_cursor();
            let page = fixture.append_page();
            let (_, metrics) = measure_sidecar_allocations(|| {
                fixture.run_with_state_and_page(&mut sidecar_state, cursor, page)
            });
            emit_history_load_more_append_sidecar("page_500", &metrics);
            elapsed
        });
    });
    group.finish();
}
