use super::common::*;
use gitcomet_ui_gpui::benchmarks::SidebarStickyFrameFixture;

pub(crate) fn bench_sidebar_sticky(c: &mut Criterion) {
    let mut group = c.benchmark_group("sidebar_sticky");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (name, branches, remotes, stashes, pins) in [
        ("typical", 1_000, 2, 50, 8),
        ("20k_refs_100_remotes", 20_000, 100, 50, 8),
        ("50k_stashes_2k_pins", 20_000, 100, 50_000, 2_000),
    ] {
        for drag in [false, true] {
            let mut fixture = SidebarStickyFrameFixture::new(branches, remotes, stashes, pins);
            let input = if drag { "drag_frame" } else { "wheel_frame" };
            group.bench_function(BenchmarkId::new(input, name), |b| {
                b.iter(|| fixture.run_frame(drag));
            });
            let rows = measure_sidecar_allocations(|| fixture.run_frame(drag));
            let mut metrics = Map::new();
            metrics.insert("rows_rendered".into(), json!(rows));
            metrics.insert("branches".into(), json!(branches));
            metrics.insert("pins".into(), json!(pins));
            metrics.insert("stashes".into(), json!(stashes));
            emit_sidecar_metrics(&format!("sidebar_sticky/{input}/{name}"), metrics);
        }
    }
    group.bench_function("geometry", |b| {
        b.iter(|| SidebarStickyFrameFixture::geometry_step(std::hint::black_box(48_321.25)))
    });
    let (_, allocations) = gitcomet_ui_gpui::perf_alloc::measure_allocations(|| {
        SidebarStickyFrameFixture::geometry_step(std::hint::black_box(48_321.25))
    });
    assert_eq!(
        allocations.alloc_ops, 0,
        "sticky geometry must not allocate"
    );
    assert_eq!(allocations.realloc_ops, 0);
    measure_sidecar_allocations(|| SidebarStickyFrameFixture::geometry_step(48_321.25));
    emit_allocation_only_sidecar("sidebar_sticky/geometry");
    group.finish();
}
