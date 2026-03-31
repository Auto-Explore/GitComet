use super::common::*;

pub(crate) fn bench_network(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_COMMITS", 50_000);
    let history_local_branches = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_LOCAL_BRANCHES", 400);
    let history_remote_branches =
        env_usize("GITCOMET_BENCH_NETWORK_HISTORY_REMOTE_BRANCHES", 1_200);
    let history_window = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_WINDOW", 120);
    let history_scroll_step = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_SCROLL_STEP", 24);
    let ui_frames = env_usize("GITCOMET_BENCH_NETWORK_UI_FRAMES", 240);
    let progress_updates = env_usize("GITCOMET_BENCH_NETWORK_PROGRESS_UPDATES", 360);
    let cancel_after_updates = env_usize("GITCOMET_BENCH_NETWORK_CANCEL_AFTER_UPDATES", 64);
    let cancel_drain_events = env_usize("GITCOMET_BENCH_NETWORK_CANCEL_DRAIN_EVENTS", 4);
    let cancel_total_updates = env_usize("GITCOMET_BENCH_NETWORK_CANCEL_TOTAL_UPDATES", 160);
    let line_bytes = env_usize("GITCOMET_BENCH_NETWORK_PROGRESS_LINE_BYTES", 72);
    let bar_width = env_usize("GITCOMET_BENCH_NETWORK_BAR_WIDTH", 32);
    let frame_budget_ns = u64::try_from(env_usize(
        "GITCOMET_BENCH_NETWORK_FRAME_BUDGET_NS",
        16_666_667,
    ))
    .unwrap_or(u64::MAX);

    let ui_fixture = NetworkFixture::ui_responsiveness_during_fetch(
        history_commits,
        history_local_branches,
        history_remote_branches,
        history_window,
        history_scroll_step,
        ui_frames,
        line_bytes,
        bar_width,
        frame_budget_ns,
    );
    let progress_fixture = NetworkFixture::progress_bar_update_render_cost(
        progress_updates,
        line_bytes,
        bar_width,
        frame_budget_ns,
    );
    let cancel_fixture = NetworkFixture::cancel_operation_latency(
        cancel_after_updates,
        cancel_drain_events,
        cancel_total_updates,
        line_bytes,
        bar_width,
        frame_budget_ns,
    );

    let mut group = c.benchmark_group("network");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("ui_responsiveness_during_fetch"),
        |b| b.iter(|| ui_fixture.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("progress_bar_update_render_cost"),
        |b| b.iter(|| progress_fixture.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("cancel_operation_latency"),
        |b| b.iter(|| cancel_fixture.run()),
    );

    group.finish();

    let (_, ui_stats, ui_metrics) = measure_sidecar_allocations(|| ui_fixture.run_with_metrics());
    emit_network_sidecar("ui_responsiveness_during_fetch", &ui_stats, &ui_metrics);

    let (_, progress_stats, progress_metrics) =
        measure_sidecar_allocations(|| progress_fixture.run_with_metrics());
    emit_network_sidecar(
        "progress_bar_update_render_cost",
        &progress_stats,
        &progress_metrics,
    );

    let (_, cancel_stats, cancel_metrics) =
        measure_sidecar_allocations(|| cancel_fixture.run_with_metrics());
    emit_network_sidecar("cancel_operation_latency", &cancel_stats, &cancel_metrics);
}
