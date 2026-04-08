use super::common::*;

pub(crate) fn bench_file_diff_syntax_query_stress(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINES", 256);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINE_BYTES", 4_096);
    let nesting_depth = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_NESTING", 128);
    let fixture =
        FileDiffSyntaxPrepareFixture::new_json_query_stress(lines, line_bytes, nesting_depth);

    let mut group = c.benchmark_group("file_diff_syntax_query_stress");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let mut nonce = 0u64;
    group.bench_function(BenchmarkId::from_parameter("nested_long_lines_cold"), |b| {
        b.iter_batched_ref(
            || {
                nonce = nonce.wrapping_add(1);
                fixture.prepare_cold_source(nonce)
            },
            |source| fixture.run_prepare_cold_from_source(source),
            BatchSize::PerIteration,
        )
    });
    group.finish();

    let bench_name = "file_diff_syntax_query_stress/nested_long_lines_cold";
    nonce = nonce.wrapping_add(1);
    let source = fixture.prepare_cold_source(nonce);
    if measure_sidecar_allocations_if_selected(bench_name, || {
        fixture.run_prepare_cold_from_source(&source)
    })
    .is_some()
    {
        emit_allocation_only_sidecar(bench_name);
    }
}
