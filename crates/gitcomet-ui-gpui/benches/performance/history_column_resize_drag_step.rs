use super::common::*;

pub(crate) fn bench_history_column_resize_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("history_column_resize_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    let columns: &[(&str, HistoryResizeColumn)] = &[
        ("branch", HistoryResizeColumn::Branch),
        ("graph", HistoryResizeColumn::Graph),
        ("author", HistoryResizeColumn::Author),
        ("date", HistoryResizeColumn::Date),
        ("sha", HistoryResizeColumn::Sha),
    ];

    for &(name, column) in columns {
        group.bench_function(name, |b| {
            let mut fixture = HistoryColumnResizeDragStepFixture::new(column);
            b.iter(|| fixture.run(column))
        });

        // Emit sidecar from a final run.
        let mut fixture = HistoryColumnResizeDragStepFixture::new(column);
        let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics(column));
        emit_history_column_resize_sidecar(
            &format!("history_column_resize_drag_step/{name}"),
            &metrics,
        );
    }

    group.finish();
}
