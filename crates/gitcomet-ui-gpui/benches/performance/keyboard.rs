use super::common::*;

pub(crate) fn bench_keyboard(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_COMMITS", 50_000);
    let history_local_branches = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_LOCAL_BRANCHES", 400);
    let history_remote_branches =
        env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_REMOTE_BRANCHES", 1_200);
    let history_window = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_WINDOW", 120);
    let history_scroll_step = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_SCROLL_STEP", 1);
    let history_repeat_events = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_REPEAT_EVENTS", 240);
    let diff_lines = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_LINES", 100_000);
    let diff_window = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_WINDOW", 200);
    let diff_line_bytes = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_LINE_BYTES", 96);
    let diff_scroll_step = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_SCROLL_STEP", 1);
    let diff_repeat_events = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_REPEAT_EVENTS", 240);
    let tab_focus_repo_tabs = env_usize("GITCOMET_BENCH_KEYBOARD_TAB_FOCUS_REPO_TABS", 20);
    let tab_focus_cycle_events = env_usize("GITCOMET_BENCH_KEYBOARD_TAB_FOCUS_CYCLE_EVENTS", 240);
    let stage_toggle_paths = env_usize("GITCOMET_BENCH_KEYBOARD_STAGE_TOGGLE_PATHS", 128);
    let stage_toggle_events = env_usize("GITCOMET_BENCH_KEYBOARD_STAGE_TOGGLE_EVENTS", 240);
    let frame_budget_ns = u64::try_from(env_usize(
        "GITCOMET_BENCH_KEYBOARD_FRAME_BUDGET_NS",
        16_666_667,
    ))
    .unwrap_or(u64::MAX);

    let history_fixture = KeyboardArrowScrollFixture::history(
        history_commits,
        history_local_branches,
        history_remote_branches,
        history_window,
        history_scroll_step,
        history_repeat_events,
        frame_budget_ns,
    );
    let diff_fixture = KeyboardArrowScrollFixture::diff(
        diff_lines,
        diff_line_bytes,
        diff_window,
        diff_scroll_step,
        diff_repeat_events,
        frame_budget_ns,
    );
    let tab_focus_fixture = KeyboardTabFocusCycleFixture::all_panes(
        tab_focus_repo_tabs,
        tab_focus_cycle_events,
        frame_budget_ns,
    );
    let stage_toggle_fixture = KeyboardStageUnstageToggleFixture::rapid_toggle(
        stage_toggle_paths,
        stage_toggle_events,
        frame_budget_ns,
    );

    let mut group = c.benchmark_group("keyboard");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("arrow_scroll_history_sustained_repeat"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = history_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| history_fixture.run_with_metrics());
                emit_keyboard_arrow_scroll_sidecar(
                    "arrow_scroll_history_sustained_repeat",
                    &stats,
                    metrics,
                );
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("arrow_scroll_diff_sustained_repeat"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = diff_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| diff_fixture.run_with_metrics());
                emit_keyboard_arrow_scroll_sidecar(
                    "arrow_scroll_diff_sustained_repeat",
                    &stats,
                    metrics,
                );
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("tab_focus_cycle_all_panes"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = tab_focus_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| tab_focus_fixture.run_with_metrics());
                emit_keyboard_tab_focus_sidecar("tab_focus_cycle_all_panes", &stats, metrics);
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("stage_unstage_toggle_rapid"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = stage_toggle_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| stage_toggle_fixture.run_with_metrics());
                emit_keyboard_stage_unstage_toggle_sidecar(
                    "stage_unstage_toggle_rapid",
                    &stats,
                    metrics,
                );
                started.elapsed()
            });
        },
    );

    group.finish();
}
