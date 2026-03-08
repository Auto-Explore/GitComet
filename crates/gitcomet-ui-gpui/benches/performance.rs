use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use gitcomet_ui_gpui::benchmarks::{
    BranchSidebarFixture, CommitDetailsFixture, ConflictResolvedOutputGutterScrollFixture,
    ConflictSearchQueryUpdateFixture, ConflictSplitResizeStepFixture,
    ConflictThreeWayScrollFixture, ConflictTwoWaySplitScrollFixture, HistoryGraphFixture,
    LargeFileDiffScrollFixture, OpenRepoFixture,
};
use std::env;
use std::time::Duration;

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn bench_open_repo(c: &mut Criterion) {
    // Note: Criterion's "Warming up for Xs" can look "stuck" if a single iteration takes longer
    // than the warm-up duration. Keep defaults moderate; scale up via env vars for stress runs.
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let history_heavy_commits = env_usize(
        "GITCOMET_BENCH_HISTORY_HEAVY_COMMITS",
        commits.saturating_mul(3),
    );
    let branch_heavy_local_branches = env_usize(
        "GITCOMET_BENCH_BRANCH_HEAVY_LOCAL_BRANCHES",
        local_branches.saturating_mul(6),
    );
    let branch_heavy_remote_branches = env_usize(
        "GITCOMET_BENCH_BRANCH_HEAVY_REMOTE_BRANCHES",
        remote_branches.saturating_mul(4),
    );
    let branch_heavy_remotes = env_usize("GITCOMET_BENCH_BRANCH_HEAVY_REMOTES", remotes.max(8));

    let balanced = OpenRepoFixture::new(commits, local_branches, remote_branches, remotes);
    let history_heavy = OpenRepoFixture::new(
        history_heavy_commits,
        local_branches.max(8) / 2,
        remote_branches.max(16) / 2,
        remotes.max(1),
    );
    let branch_heavy = OpenRepoFixture::new(
        commits.max(500) / 5,
        branch_heavy_local_branches,
        branch_heavy_remote_branches,
        branch_heavy_remotes,
    );

    let mut group = c.benchmark_group("open_repo");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("balanced"), |b| {
        b.iter(|| balanced.run())
    });
    group.bench_function(BenchmarkId::from_parameter("history_heavy"), |b| {
        b.iter(|| history_heavy.run())
    });
    group.bench_function(BenchmarkId::from_parameter("branch_heavy"), |b| {
        b.iter(|| branch_heavy.run())
    });
    group.finish();
}

fn bench_branch_sidebar(c: &mut Criterion) {
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let worktrees = env_usize("GITCOMET_BENCH_WORKTREES", 80);
    let submodules = env_usize("GITCOMET_BENCH_SUBMODULES", 150);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 300);

    let local_heavy = BranchSidebarFixture::new(
        local_branches.saturating_mul(8),
        remote_branches.max(32) / 8,
        remotes.max(1),
        0,
        0,
        0,
    );
    let remote_fanout = BranchSidebarFixture::new(
        local_branches.max(32) / 4,
        remote_branches.saturating_mul(6),
        remotes.max(12),
        0,
        0,
        0,
    );
    let aux_lists_heavy = BranchSidebarFixture::new(
        local_branches,
        remote_branches,
        remotes.max(2),
        worktrees,
        submodules,
        stashes,
    );

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("local_heavy"), |b| {
        b.iter(|| local_heavy.run())
    });
    group.bench_function(BenchmarkId::from_parameter("remote_fanout"), |b| {
        b.iter(|| remote_fanout.run())
    });
    group.bench_function(BenchmarkId::from_parameter("aux_lists_heavy"), |b| {
        b.iter(|| aux_lists_heavy.run())
    });
    group.finish();
}

fn bench_history_graph(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let merge_stride = env_usize("GITCOMET_BENCH_HISTORY_MERGE_EVERY", 50);
    let branch_head_every = env_usize("GITCOMET_BENCH_HISTORY_BRANCH_HEAD_EVERY", 11);

    let linear_history = HistoryGraphFixture::new(commits, 0, 0);
    let merge_dense = HistoryGraphFixture::new(commits, merge_stride.max(5).min(25), 0);
    let branch_heads_dense =
        HistoryGraphFixture::new(commits, merge_stride.max(1), branch_head_every.max(2));

    let mut group = c.benchmark_group("history_graph");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("linear_history"), |b| {
        b.iter(|| linear_history.run())
    });
    group.bench_function(BenchmarkId::from_parameter("merge_dense"), |b| {
        b.iter(|| merge_dense.run())
    });
    group.bench_function(BenchmarkId::from_parameter("branch_heads_dense"), |b| {
        b.iter(|| branch_heads_dense.run())
    });
    group.finish();
}

fn bench_commit_details(c: &mut Criterion) {
    let files = env_usize("GITCOMET_BENCH_COMMIT_FILES", 5_000);
    let depth = env_usize("GITCOMET_BENCH_COMMIT_PATH_DEPTH", 4);
    let deep_depth = env_usize(
        "GITCOMET_BENCH_COMMIT_DEEP_PATH_DEPTH",
        depth.saturating_mul(4).max(12),
    );
    let huge_files = env_usize("GITCOMET_BENCH_COMMIT_HUGE_FILES", files.saturating_mul(2));
    let balanced = CommitDetailsFixture::new(files, depth);
    let deep_paths = CommitDetailsFixture::new(files, deep_depth);
    let huge_list = CommitDetailsFixture::new(huge_files, depth);

    let mut group = c.benchmark_group("commit_details");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("many_files"), |b| {
        b.iter(|| balanced.run())
    });
    group.bench_function(BenchmarkId::from_parameter("deep_paths"), |b| {
        b.iter(|| deep_paths.run())
    });
    group.bench_function(BenchmarkId::from_parameter("huge_file_list"), |b| {
        b.iter(|| huge_list.run())
    });
    group.finish();
}

fn bench_large_file_diff_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_DIFF_LINES", 10_000);
    let window = env_usize("GITCOMET_BENCH_DIFF_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_DIFF_LINE_BYTES", 96);
    let long_line_bytes = env_usize("GITCOMET_BENCH_DIFF_LONG_LINE_BYTES", 4_096);
    let normal_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(lines, line_bytes);
    let long_line_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(lines, long_line_bytes);

    let mut group = c.benchmark_group("diff_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("normal_lines_window", window),
        &window,
        |b, &window| {
            // Use a varying start index per-iteration to reduce cache effects in allocators.
            let mut start = 0usize;
            b.iter(|| {
                let h = normal_fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.bench_with_input(
        BenchmarkId::new("long_lines_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = long_line_fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();
}

fn bench_conflict_three_way_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let fixture = ConflictThreeWayScrollFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("style_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();
}

fn bench_conflict_two_way_split_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictTwoWaySplitScrollFixture::new(lines, conflict_blocks);
    let windows = [100usize, 200, 400];

    let mut group = c.benchmark_group("conflict_two_way_split_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for &window in &windows {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("window_{window}")),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let h = fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(window) % fixture.visible_rows().max(1);
                    h
                })
            },
        );
    }
    group.finish();
}

fn bench_conflict_resolved_output_gutter_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictResolvedOutputGutterScrollFixture::new(lines, conflict_blocks);
    let windows = [100usize, 200, 400];

    let mut group = c.benchmark_group("conflict_resolved_output_gutter_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for &window in &windows {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("window_{window}")),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let h = fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(window) % fixture.visible_rows().max(1);
                    h
                })
            },
        );
    }
    group.finish();
}

fn bench_conflict_search_query_update(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let mut fixture = ConflictSearchQueryUpdateFixture::new(lines, conflict_blocks);
    let query_cycle = [
        "s", "sh", "sha", "shar", "share", "shared", "shared_", "shared_1",
    ];

    let mut group = c.benchmark_group("conflict_search_query_update");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &window| {
        let mut start = 0usize;
        let mut query_ix = 0usize;
        b.iter(|| {
            let query = query_cycle[query_ix % query_cycle.len()];
            let h = fixture.run_query_update_step(query, start, window);
            query_ix = query_ix.wrapping_add(1);
            start = start.wrapping_add(window.max(1) / 2 + 1) % fixture.visible_rows().max(1);
            h
        })
    });
    group.finish();
}

fn bench_conflict_split_resize_step(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let resize_query =
        env::var("GITCOMET_BENCH_CONFLICT_RESIZE_QUERY").unwrap_or_else(|_| "shared".to_string());
    let mut fixture = ConflictSplitResizeStepFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_split_resize_step");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &window| {
        let mut start = 0usize;
        b.iter(|| {
            let h = fixture.run_resize_step(resize_query.as_str(), start, window);
            start = start.wrapping_add(window.max(1) / 3 + 1) % fixture.visible_rows().max(1);
            h
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_open_repo,
    bench_branch_sidebar,
    bench_history_graph,
    bench_commit_details,
    bench_large_file_diff_scroll,
    bench_conflict_three_way_scroll,
    bench_conflict_two_way_split_scroll,
    bench_conflict_resolved_output_gutter_scroll,
    bench_conflict_search_query_update,
    bench_conflict_split_resize_step
);
criterion_main!(benches);
