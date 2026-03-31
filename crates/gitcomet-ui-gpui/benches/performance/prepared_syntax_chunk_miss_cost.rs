use super::common::*;

pub(crate) fn bench_prepared_syntax_chunk_miss_cost(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINE_BYTES", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("prepared_syntax_chunk_miss_cost");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    let mut nonce = 0u64;
    group.bench_function(BenchmarkId::from_parameter("chunk_miss"), |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                nonce = nonce.wrapping_add(1);
                total =
                    total.saturating_add(fixture.run_prepared_syntax_chunk_miss_cost_step(nonce));
            }
            total
        })
    });
    group.finish();

    nonce = nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| fixture.run_prepared_syntax_chunk_miss_cost_step(nonce));
    emit_allocation_only_sidecar("prepared_syntax_chunk_miss_cost/chunk_miss");
}
