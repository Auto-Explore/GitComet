use super::common::*;

pub(crate) fn bench_pane_resize_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("pane_resize_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    let targets: &[(&str, PaneResizeTarget)] = &[
        ("sidebar", PaneResizeTarget::Sidebar),
        ("details", PaneResizeTarget::Details),
    ];

    for &(name, target) in targets {
        group.bench_function(name, |b| {
            let mut fixture = PaneResizeDragStepFixture::new(target);
            b.iter(|| fixture.run())
        });

        let mut fixture = PaneResizeDragStepFixture::new(target);
        let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
        emit_pane_resize_drag_sidecar(&format!("pane_resize_drag_step/{name}"), &metrics);
    }

    group.finish();
}
