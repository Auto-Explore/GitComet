use super::common::*;

pub(crate) fn bench_file_diff_replacement_alignment(c: &mut Criterion) {
    let blocks = env_usize("GITCOMET_BENCH_REPLACEMENT_BLOCKS", 12);
    let balanced_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_BALANCED_LINES", 48);
    let skewed_old_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_SKEW_OLD_LINES", 40);
    let skewed_new_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_SKEW_NEW_LINES", 56);
    let context_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_CONTEXT_LINES", 3);
    let line_bytes = env_usize("GITCOMET_BENCH_REPLACEMENT_LINE_BYTES", 128);

    let balanced = ReplacementAlignmentFixture::new(
        blocks,
        balanced_lines,
        balanced_lines,
        context_lines,
        line_bytes,
    );
    let skewed = ReplacementAlignmentFixture::new(
        blocks,
        skewed_old_lines,
        skewed_new_lines,
        context_lines,
        line_bytes,
    );

    let mut group = c.benchmark_group("file_diff_replacement_alignment");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::new("balanced_blocks", "scratch"), |b| {
        b.iter(|| balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch))
    });
    group.bench_function(BenchmarkId::new("balanced_blocks", "strsim"), |b| {
        b.iter(|| balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim))
    });
    group.bench_function(BenchmarkId::new("skewed_blocks", "scratch"), |b| {
        b.iter(|| skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch))
    });
    group.bench_function(BenchmarkId::new("skewed_blocks", "strsim"), |b| {
        b.iter(|| skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim))
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| {
        balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/balanced_blocks/scratch");
    let _ = measure_sidecar_allocations(|| {
        balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/balanced_blocks/strsim");
    let _ = measure_sidecar_allocations(|| {
        skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/skewed_blocks/scratch");
    let _ = measure_sidecar_allocations(|| {
        skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/skewed_blocks/strsim");
}
