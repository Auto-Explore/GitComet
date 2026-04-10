use super::common::*;

pub(crate) fn bench_diff_open_file_inline_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_FILE_DIFF_WINDOW", 200);
    let fixture = FileDiffOpenFixture::new(lines);
    let bench_name = format!("diff_open_file_inline_first_window/{window}");
    let metrics = measure_sidecar_allocations_if_selected(&bench_name, || {
        fixture.measure_first_window(window)
    });

    let mut group = c.benchmark_group("diff_open_file_inline_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_inline_first_window(window)),
    );
    group.finish();
    if let Some(metrics) = metrics.as_ref() {
        emit_file_diff_open_sidecar(&bench_name, metrics);
    }
}
