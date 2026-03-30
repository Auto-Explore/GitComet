use super::common::*;

pub(crate) fn bench_prepared_syntax_multidoc_cache_hit_rate(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINE_BYTES", 128);
    let docs = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_HOT_DOCS", 6);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("prepared_syntax_multidoc_cache_hit_rate");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    let mut nonce = 0u64;
    group.bench_with_input(BenchmarkId::new("hot_docs", docs), &docs, |b, &docs| {
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(docs, nonce)
        })
    });
    group.finish();

    nonce = nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| {
        fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(docs, nonce)
    });
    emit_allocation_only_sidecar(&format!(
        "prepared_syntax_multidoc_cache_hit_rate/hot_docs/{docs}"
    ));
}
