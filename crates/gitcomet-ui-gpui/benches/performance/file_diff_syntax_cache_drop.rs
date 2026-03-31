use super::common::*;

pub(crate) fn bench_file_diff_syntax_cache_drop(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_LINES", 2_048);
    let tokens_per_line = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_TOKENS_PER_LINE", 8);
    let replacements = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_REPLACEMENTS", 4);
    let fixture = FileDiffSyntaxCacheDropFixture::new(lines, tokens_per_line, replacements);

    let mut group = c.benchmark_group("file_diff_syntax_cache_drop");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("deferred_drop", replacements),
        &replacements,
        |b, &_replacements| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut seed = 0usize;
                for _ in 0..iters {
                    let _ = fixture.flush_deferred_drop_queue();
                    total = total.saturating_add(fixture.run_deferred_drop_timed_step(seed));
                    seed = seed.wrapping_add(1);
                }
                total
            })
        },
    );
    let _ = fixture.flush_deferred_drop_queue();
    group.bench_with_input(
        BenchmarkId::new("inline_drop_control", replacements),
        &replacements,
        |b, &_replacements| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut seed = 0usize;
                for _ in 0..iters {
                    total = total.saturating_add(fixture.run_inline_drop_control_timed_step(seed));
                    seed = seed.wrapping_add(1);
                }
                total
            })
        },
    );
    group.finish();

    let _ = fixture.flush_deferred_drop_queue();
    let _ = measure_sidecar_allocations(|| fixture.run_deferred_drop_timed_step(0usize));
    emit_allocation_only_sidecar(&format!(
        "file_diff_syntax_cache_drop/deferred_drop/{replacements}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_inline_drop_control_timed_step(0usize));
    emit_allocation_only_sidecar(&format!(
        "file_diff_syntax_cache_drop/inline_drop_control/{replacements}"
    ));
}
