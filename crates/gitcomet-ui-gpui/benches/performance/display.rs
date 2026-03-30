use super::common::*;

pub(crate) fn bench_display(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_DISPLAY_HISTORY_COMMITS", 10_000);
    let local_branches = env_usize("GITCOMET_BENCH_DISPLAY_LOCAL_BRANCHES", 100);
    let remote_branches = env_usize("GITCOMET_BENCH_DISPLAY_REMOTE_BRANCHES", 400);
    let diff_lines = env_usize("GITCOMET_BENCH_DISPLAY_DIFF_LINES", 5_000);
    let history_window = env_usize("GITCOMET_BENCH_DISPLAY_HISTORY_WINDOW", 120);
    let diff_window = env_usize("GITCOMET_BENCH_DISPLAY_DIFF_WINDOW", 200);
    let base_width = 1920.0f32;
    let sidebar_w = 280.0f32;
    let details_w = 420.0f32;

    let scale_fixture = DisplayFixture::render_cost_by_scale(
        history_commits,
        local_branches,
        remote_branches,
        diff_lines,
        history_window,
        diff_window,
        base_width,
        sidebar_w,
        details_w,
    );
    let two_win_fixture = DisplayFixture::two_windows_same_repo(
        history_commits,
        local_branches,
        remote_branches,
        diff_lines,
        history_window,
        diff_window,
        base_width,
        sidebar_w,
        details_w,
    );
    let dpi_move_fixture = DisplayFixture::window_move_between_dpis(
        history_commits,
        local_branches,
        remote_branches,
        diff_lines,
        history_window,
        diff_window,
        base_width,
        sidebar_w,
        details_w,
    );

    let mut group = c.benchmark_group("display");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("render_cost_1x_vs_2x_vs_3x_scale"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = scale_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) = measure_sidecar_allocations(|| scale_fixture.run_with_metrics());
                emit_display_sidecar("render_cost_1x_vs_2x_vs_3x_scale", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(BenchmarkId::from_parameter("two_windows_same_repo"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let started_at = Instant::now();
                let _ = two_win_fixture.run_with_metrics();
                elapsed += started_at.elapsed();
            }
            let (_, metrics) = measure_sidecar_allocations(|| two_win_fixture.run_with_metrics());
            emit_display_sidecar("two_windows_same_repo", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("window_move_between_dpis"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = dpi_move_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) =
                    measure_sidecar_allocations(|| dpi_move_fixture.run_with_metrics());
                emit_display_sidecar("window_move_between_dpis", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}
