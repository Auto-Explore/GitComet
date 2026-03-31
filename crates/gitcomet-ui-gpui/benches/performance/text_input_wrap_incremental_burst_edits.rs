use super::common::*;

pub(crate) fn bench_text_input_wrap_incremental_burst_edits(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);
    let edits_per_burst = env_usize("GITCOMET_BENCH_TEXT_INPUT_BURST_EDITS", 12);
    let mut full_fixture =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let mut incremental_fixture =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_wrap_incremental_burst_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("full_recompute", edits_per_burst),
        &edits_per_burst,
        |b, &edits_per_burst| {
            b.iter(|| full_fixture.run_full_recompute_burst_step(edits_per_burst))
        },
    );
    group.bench_with_input(
        BenchmarkId::new("incremental_patch", edits_per_burst),
        &edits_per_burst,
        |b, &edits_per_burst| {
            b.iter(|| incremental_fixture.run_incremental_burst_step(edits_per_burst))
        },
    );
    group.finish();

    let mut sidecar_full =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let (_, full_metrics) = measure_sidecar_allocations(|| {
        sidecar_full.run_full_recompute_burst_step_with_metrics(edits_per_burst)
    });
    emit_text_input_wrap_incremental_burst_edits_sidecar(
        &format!("text_input_wrap_incremental_burst_edits/full_recompute/{edits_per_burst}"),
        &full_metrics,
    );

    let mut sidecar_incremental =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let (_, incremental_metrics) = measure_sidecar_allocations(|| {
        sidecar_incremental.run_incremental_burst_step_with_metrics(edits_per_burst)
    });
    emit_text_input_wrap_incremental_burst_edits_sidecar(
        &format!("text_input_wrap_incremental_burst_edits/incremental_patch/{edits_per_burst}"),
        &incremental_metrics,
    );
}
