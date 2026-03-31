use super::common::*;

pub(crate) fn bench_window_resize_layout(c: &mut Criterion) {
    let fixture = WindowResizeLayoutFixture::sidebar_main_details();
    let mut group = c.benchmark_group("window_resize_layout");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    group.bench_function("sidebar_main_details", |b| {
        b.iter(|| {
            let (hash, _metrics) = fixture.run_with_metrics();
            hash
        })
    });

    // Emit sidecar from a final run.
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
    emit_window_resize_layout_sidecar("window_resize_layout/sidebar_main_details", &metrics);

    group.finish();
}
