use super::common::*;

pub(crate) fn bench_status_multi_select(c: &mut Criterion) {
    let entries = env_usize("GITCOMET_BENCH_STATUS_MULTI_SELECT_ENTRIES", 20_000);
    let anchor_index = env_usize("GITCOMET_BENCH_STATUS_MULTI_SELECT_ANCHOR", 4_096);
    let selected_paths = env_usize("GITCOMET_BENCH_STATUS_MULTI_SELECT_RANGE", 512);
    let fixture = StatusMultiSelectFixture::range_select(entries, anchor_index, selected_paths);
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());

    let mut group = c.benchmark_group("status_multi_select");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("range_select"), |b| {
        b.iter(|| fixture.run())
    });
    group.finish();

    emit_status_multi_select_sidecar("range_select", &metrics);
}
