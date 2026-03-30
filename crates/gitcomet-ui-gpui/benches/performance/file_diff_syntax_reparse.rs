use super::common::*;

pub(crate) fn bench_file_diff_syntax_reparse(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES", 128);
    let mut small_fixture = FileDiffSyntaxReparseFixture::new(lines, line_bytes);
    let mut large_fixture = FileDiffSyntaxReparseFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("file_diff_syntax_reparse");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_reparse_small_edit"),
        |b| b.iter(|| small_fixture.run_small_edit_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_reparse_large_edit"),
        |b| b.iter(|| large_fixture.run_large_edit_step()),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| small_fixture.run_small_edit_step());
    emit_allocation_only_sidecar("file_diff_syntax_reparse/file_diff_syntax_reparse_small_edit");
    let _ = measure_sidecar_allocations(|| large_fixture.run_large_edit_step());
    emit_allocation_only_sidecar("file_diff_syntax_reparse/file_diff_syntax_reparse_large_edit");
}
