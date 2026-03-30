use super::common::*;

pub(crate) fn bench_frame_timing(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_FRAME_HISTORY_COMMITS", 50_000);
    let history_local_branches = env_usize("GITCOMET_BENCH_FRAME_HISTORY_LOCAL_BRANCHES", 400);
    let history_remote_branches = env_usize("GITCOMET_BENCH_FRAME_HISTORY_REMOTE_BRANCHES", 1_200);
    let history_window = env_usize("GITCOMET_BENCH_FRAME_HISTORY_WINDOW", 120);
    let history_scroll_step = env_usize("GITCOMET_BENCH_FRAME_HISTORY_SCROLL_STEP", 24);
    let diff_lines = env_usize("GITCOMET_BENCH_FRAME_DIFF_LINES", 100_000);
    let diff_window = env_usize("GITCOMET_BENCH_FRAME_DIFF_WINDOW", 200);
    let diff_line_bytes = env_usize("GITCOMET_BENCH_FRAME_DIFF_LINE_BYTES", 96);
    let diff_scroll_step = env_usize("GITCOMET_BENCH_FRAME_DIFF_SCROLL_STEP", 40);
    let frames = env_usize("GITCOMET_BENCH_FRAME_TIMING_FRAMES", 240);
    let frame_budget_ns =
        u64::try_from(env_usize("GITCOMET_BENCH_FRAME_BUDGET_NS", 16_666_667)).unwrap_or(u64::MAX);

    let history_fixture = HistoryListScrollFixture::new(
        history_commits,
        history_local_branches,
        history_remote_branches,
    );
    let diff_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(diff_lines, diff_line_bytes);

    let mut group = c.benchmark_group("frame_timing");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("continuous_scroll_history_list"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, stats, metrics) = capture_frame_timing_scroll_burst(
                        history_commits,
                        history_window,
                        history_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| history_fixture.run_scroll_step(start, window),
                    );
                    hash ^= burst_hash;
                    std::hint::black_box((stats, metrics));
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) = measure_sidecar_allocations(|| {
                    capture_frame_timing_scroll_burst(
                        history_commits,
                        history_window,
                        history_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| history_fixture.run_scroll_step(start, window),
                    )
                });
                emit_frame_timing_sidecar("continuous_scroll_history_list", &stats, metrics);
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("continuous_scroll_large_diff"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, stats, metrics) = capture_frame_timing_scroll_burst(
                        diff_lines,
                        diff_window,
                        diff_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| diff_fixture.run_scroll_step(start, window),
                    );
                    hash ^= burst_hash;
                    std::hint::black_box((stats, metrics));
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) = measure_sidecar_allocations(|| {
                    capture_frame_timing_scroll_burst(
                        diff_lines,
                        diff_window,
                        diff_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| diff_fixture.run_scroll_step(start, window),
                    )
                });
                emit_frame_timing_sidecar("continuous_scroll_large_diff", &stats, metrics);
                started.elapsed()
            });
        },
    );

    // --- sidebar_resize_drag_sustained ---
    let sidebar_drag_frames = env_usize("GITCOMET_BENCH_FRAME_SIDEBAR_DRAG_FRAMES", 240);
    group.bench_function(
        BenchmarkId::from_parameter("sidebar_resize_drag_sustained"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let mut fixture = SidebarResizeDragSustainedFixture::new(
                        sidebar_drag_frames,
                        frame_budget_ns,
                    );
                    let (burst_hash, _stats, _metrics) = fixture.run_with_metrics();
                    hash ^= burst_hash;
                }

                std::hint::black_box(hash);
                let mut fixture =
                    SidebarResizeDragSustainedFixture::new(sidebar_drag_frames, frame_budget_ns);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| fixture.run_with_metrics());
                emit_sidebar_resize_drag_sustained_sidecar(&stats, metrics);
                started.elapsed()
            });
        },
    );

    // --- rapid_commit_selection_changes ---
    let rapid_commit_count = env_usize("GITCOMET_BENCH_FRAME_RAPID_COMMIT_COUNT", 120);
    let rapid_commit_files = env_usize("GITCOMET_BENCH_FRAME_RAPID_COMMIT_FILES", 200);
    let rapid_commit_fixture =
        RapidCommitSelectionFixture::new(rapid_commit_count, rapid_commit_files, frame_budget_ns);
    group.bench_function(
        BenchmarkId::from_parameter("rapid_commit_selection_changes"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, _stats, _metrics) = rapid_commit_fixture.run_with_metrics();
                    hash ^= burst_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| rapid_commit_fixture.run_with_metrics());
                emit_rapid_commit_selection_sidecar(&stats, metrics);
                started.elapsed()
            });
        },
    );

    // --- repo_switch_during_scroll ---
    let switch_every = env_usize("GITCOMET_BENCH_FRAME_SWITCH_EVERY_N_FRAMES", 30);
    let repo_switch_scroll_fixture = RepoSwitchDuringScrollFixture::new(
        history_commits,
        history_local_branches,
        history_remote_branches,
        history_window,
        history_scroll_step,
        frames,
        switch_every,
        frame_budget_ns,
    );
    group.bench_function(
        BenchmarkId::from_parameter("repo_switch_during_scroll"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, _stats, _metrics) =
                        repo_switch_scroll_fixture.run_with_metrics();
                    hash ^= burst_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| repo_switch_scroll_fixture.run_with_metrics());
                emit_repo_switch_during_scroll_sidecar(&stats, metrics);
                started.elapsed()
            });
        },
    );

    group.finish();
}
