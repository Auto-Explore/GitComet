use super::common::*;

pub(crate) fn bench_file_diff_syntax_prepare(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);
    fixture.prewarm();

    let mut group = c.benchmark_group("file_diff_syntax_prepare");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let mut cold_nonce = 0u64;
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_prepare_cold"),
        |b| {
            b.iter(|| {
                cold_nonce = cold_nonce.wrapping_add(1);
                fixture.run_prepare_cold(cold_nonce)
            })
        },
    );
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_prepare_warm"),
        |b| b.iter(|| fixture.run_prepare_warm()),
    );
    group.finish();

    cold_nonce = cold_nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| fixture.run_prepare_cold(cold_nonce));
    emit_allocation_only_sidecar("file_diff_syntax_prepare/file_diff_syntax_prepare_cold");
    let _ = measure_sidecar_allocations(|| fixture.run_prepare_warm());
    emit_allocation_only_sidecar("file_diff_syntax_prepare/file_diff_syntax_prepare_warm");
}
