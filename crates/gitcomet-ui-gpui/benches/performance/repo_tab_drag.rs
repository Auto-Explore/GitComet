use super::common::*;

pub(crate) fn bench_repo_tab_drag(c: &mut Criterion) {
    let mut group = c.benchmark_group("repo_tab_drag");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    for &tab_count in &[20usize, 200usize] {
        let fixture = RepoTabDragFixture::new(tab_count);

        group.bench_function(
            BenchmarkId::new("hit_test", format!("{tab_count}_tabs")),
            |b| b.iter(|| fixture.run_hit_test()),
        );
        group.bench_function(
            BenchmarkId::new("reorder_reduce", format!("{tab_count}_tabs")),
            |b| b.iter(|| fixture.run_reorder()),
        );

        // Emit sidecars from final runs.
        let (_, hit_metrics) = measure_sidecar_allocations(|| fixture.run_hit_test());
        emit_repo_tab_drag_sidecar(
            &format!("repo_tab_drag/hit_test/{tab_count}_tabs"),
            &hit_metrics,
        );
        let (_, reorder_metrics) = measure_sidecar_allocations(|| fixture.run_reorder());
        emit_repo_tab_drag_sidecar(
            &format!("repo_tab_drag/reorder_reduce/{tab_count}_tabs"),
            &reorder_metrics,
        );
    }

    group.finish();
}
