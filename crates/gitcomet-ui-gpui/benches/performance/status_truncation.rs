use super::common::*;

pub(crate) fn bench_status_truncation(c: &mut Criterion) {
    let entries_per_section = env_usize("GITCOMET_BENCH_STATUS_TRUNCATION_SECTION_ENTRIES", 240);
    let max_width_px = env_usize("GITCOMET_BENCH_STATUS_TRUNCATION_WIDTH_PX", 180) as f32;

    let mut path_aligned = StatusTruncationRenderFixture::long_untracked_unstaged(
        entries_per_section,
        StatusTruncationScenario::PathAligned,
        max_width_px,
    );
    let mut middle = StatusTruncationRenderFixture::long_untracked_unstaged(
        entries_per_section,
        StatusTruncationScenario::Middle,
        max_width_px,
    );
    let mut end = StatusTruncationRenderFixture::long_untracked_unstaged(
        entries_per_section,
        StatusTruncationScenario::End,
        max_width_px,
    );
    let mut focus = StatusTruncationRenderFixture::long_untracked_unstaged(
        entries_per_section,
        StatusTruncationScenario::Focus,
        max_width_px,
    );

    let mut group = c.benchmark_group("status_truncation");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("path_aligned_long_paths"),
        |b| b.iter(|| path_aligned.run()),
    );
    group.bench_function(BenchmarkId::from_parameter("middle_long_paths"), |b| {
        b.iter(|| middle.run())
    });
    group.bench_function(BenchmarkId::from_parameter("end_long_paths"), |b| {
        b.iter(|| end.run())
    });
    group.bench_function(BenchmarkId::from_parameter("focus_long_paths"), |b| {
        b.iter(|| focus.run())
    });
    group.finish();

    let (_, path_metrics) = measure_sidecar_allocations(|| path_aligned.run_with_metrics());
    emit_status_truncation_sidecar("path_aligned_long_paths", &path_metrics);
    let (_, middle_metrics) = measure_sidecar_allocations(|| middle.run_with_metrics());
    emit_status_truncation_sidecar("middle_long_paths", &middle_metrics);
    let (_, end_metrics) = measure_sidecar_allocations(|| end.run_with_metrics());
    emit_status_truncation_sidecar("end_long_paths", &end_metrics);
    let (_, focus_metrics) = measure_sidecar_allocations(|| focus.run_with_metrics());
    emit_status_truncation_sidecar("focus_long_paths", &focus_metrics);
}
