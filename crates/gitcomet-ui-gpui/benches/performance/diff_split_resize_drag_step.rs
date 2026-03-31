use super::common::*;

pub(crate) fn bench_diff_split_resize_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_split_resize_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    group.bench_function("window_200", |b| {
        let mut fixture = DiffSplitResizeDragStepFixture::window_200();
        b.iter(|| fixture.run())
    });

    // Emit sidecar from a final run.
    let mut fixture = DiffSplitResizeDragStepFixture::window_200();
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
    emit_diff_split_resize_drag_sidecar("diff_split_resize_drag_step/window_200", &metrics);

    group.finish();
}
