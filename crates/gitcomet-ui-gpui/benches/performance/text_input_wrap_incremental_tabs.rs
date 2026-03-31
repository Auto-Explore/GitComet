use super::common::*;

pub(crate) fn bench_text_input_wrap_incremental_tabs(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);
    let mut full_fixture = TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let mut incremental_fixture =
        TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_wrap_incremental_tabs");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("full_recompute"), |b| {
        let mut edit_ix = 0usize;
        b.iter(|| {
            let h = full_fixture.run_full_recompute_step(edit_ix);
            edit_ix = edit_ix.wrapping_add(17);
            h
        })
    });
    group.bench_function(BenchmarkId::from_parameter("incremental_patch"), |b| {
        let mut edit_ix = 0usize;
        b.iter(|| {
            let h = incremental_fixture.run_incremental_step(edit_ix);
            edit_ix = edit_ix.wrapping_add(17);
            h
        })
    });
    group.finish();

    let mut sidecar_full = TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let (_, full_metrics) =
        measure_sidecar_allocations(|| sidecar_full.run_full_recompute_step_with_metrics(0));
    emit_text_input_wrap_incremental_tabs_sidecar(
        "text_input_wrap_incremental_tabs/full_recompute",
        &full_metrics,
    );

    let mut sidecar_incremental =
        TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let (_, incremental_metrics) =
        measure_sidecar_allocations(|| sidecar_incremental.run_incremental_step_with_metrics(0));
    emit_text_input_wrap_incremental_tabs_sidecar(
        "text_input_wrap_incremental_tabs/incremental_patch",
        &incremental_metrics,
    );
}
