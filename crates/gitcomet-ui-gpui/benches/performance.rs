use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use gitcomet_core::file_diff::BenchmarkReplacementDistanceBackend;
use gitcomet_ui_gpui::benchmarks::{
    BranchSidebarFixture, CommitDetailsFixture, ConflictLoadDuplicationFixture,
    ConflictResolvedOutputGutterScrollFixture, ConflictSearchQueryUpdateFixture,
    ConflictSplitResizeStepFixture, ConflictStreamedProviderFixture,
    ConflictStreamedResolvedOutputFixture, ConflictThreeWayScrollFixture,
    ConflictThreeWayVisibleMapBuildFixture, ConflictTwoWayDiffBuildFixture,
    ConflictTwoWaySplitScrollFixture, FileDiffInlineSyntaxProjectionFixture,
    FileDiffSyntaxCacheDropFixture, FileDiffSyntaxPrepareFixture, FileDiffSyntaxReparseFixture,
    HistoryGraphFixture, LargeFileDiffScrollFixture, LargeHtmlSyntaxFixture,
    MarkdownPreviewFixture, OpenRepoFixture, PatchDiffPagedRowsFixture,
    PatchDiffSearchQueryUpdateFixture, ReplacementAlignmentFixture,
    ResolvedOutputRecomputeIncrementalFixture, TextInputHighlightDensity,
    TextInputLongLineCapFixture, TextInputPrepaintWindowedFixture,
    TextInputRunsStreamedHighlightFixture, TextInputWrapIncrementalBurstEditsFixture,
    TextInputWrapIncrementalTabsFixture, TextModelBulkLoadLargeFixture,
    TextModelFragmentedEditFixture, TextModelSnapshotCloneCostFixture,
    WorktreePreviewRenderFixture,
};
use std::env;
use std::time::Duration;

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_string(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
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
    let merge_dense = HistoryGraphFixture::new(commits, merge_stride.clamp(5, 25), 0);
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

fn bench_file_diff_replacement_alignment(c: &mut Criterion) {
    let blocks = env_usize("GITCOMET_BENCH_REPLACEMENT_BLOCKS", 12);
    let balanced_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_BALANCED_LINES", 48);
    let skewed_old_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_SKEW_OLD_LINES", 40);
    let skewed_new_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_SKEW_NEW_LINES", 56);
    let context_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_CONTEXT_LINES", 3);
    let line_bytes = env_usize("GITCOMET_BENCH_REPLACEMENT_LINE_BYTES", 128);

    let balanced = ReplacementAlignmentFixture::new(
        blocks,
        balanced_lines,
        balanced_lines,
        context_lines,
        line_bytes,
    );
    let skewed = ReplacementAlignmentFixture::new(
        blocks,
        skewed_old_lines,
        skewed_new_lines,
        context_lines,
        line_bytes,
    );

    let mut group = c.benchmark_group("file_diff_replacement_alignment");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::new("balanced_blocks", "scratch"), |b| {
        b.iter(|| balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch))
    });
    group.bench_function(BenchmarkId::new("balanced_blocks", "strsim"), |b| {
        b.iter(|| balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim))
    });
    group.bench_function(BenchmarkId::new("skewed_blocks", "scratch"), |b| {
        b.iter(|| skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch))
    });
    group.bench_function(BenchmarkId::new("skewed_blocks", "strsim"), |b| {
        b.iter(|| skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim))
    });
    group.finish();
}

fn bench_text_input_prepaint_windowed(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let window_rows = env_usize("GITCOMET_BENCH_TEXT_INPUT_WINDOW_ROWS", 80);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);

    let mut windowed_fixture = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let mut full_fixture = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_prepaint_windowed");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("window_rows", window_rows),
        &window_rows,
        |b, &window_rows| {
            let mut start = 0usize;
            b.iter(|| {
                let h = windowed_fixture.run_windowed_step(start, window_rows.max(1));
                start = start.wrapping_add(window_rows.max(1) / 2 + 1)
                    % windowed_fixture.total_rows().max(1);
                h
            })
        },
    );
    group.bench_function(BenchmarkId::from_parameter("full_document_control"), |b| {
        b.iter(|| full_fixture.run_full_document_step())
    });
    group.finish();
}

fn bench_text_input_runs_streamed_highlight(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let window_rows = env_usize("GITCOMET_BENCH_TEXT_INPUT_WINDOW_ROWS", 80);

    let dense_fixture = TextInputRunsStreamedHighlightFixture::new(
        lines,
        line_bytes,
        window_rows,
        TextInputHighlightDensity::Dense,
    );
    let sparse_fixture = TextInputRunsStreamedHighlightFixture::new(
        lines,
        line_bytes,
        window_rows,
        TextInputHighlightDensity::Sparse,
    );

    let mut dense_group = c.benchmark_group("text_input_runs_streamed_highlight_dense");
    dense_group.sample_size(10);
    dense_group.warm_up_time(Duration::from_secs(1));
    dense_group.bench_function(BenchmarkId::from_parameter("legacy_scan"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = dense_fixture.run_legacy_step(start);
            start = dense_fixture.next_start_row(start);
            h
        })
    });
    dense_group.bench_function(BenchmarkId::from_parameter("streamed_cursor"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = dense_fixture.run_streamed_step(start);
            start = dense_fixture.next_start_row(start);
            h
        })
    });
    dense_group.finish();

    let mut sparse_group = c.benchmark_group("text_input_runs_streamed_highlight_sparse");
    sparse_group.sample_size(10);
    sparse_group.warm_up_time(Duration::from_secs(1));
    sparse_group.bench_function(BenchmarkId::from_parameter("legacy_scan"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = sparse_fixture.run_legacy_step(start);
            start = sparse_fixture.next_start_row(start);
            h
        })
    });
    sparse_group.bench_function(BenchmarkId::from_parameter("streamed_cursor"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = sparse_fixture.run_streamed_step(start);
            start = sparse_fixture.next_start_row(start);
            h
        })
    });
    sparse_group.finish();
}

fn bench_text_input_long_line_cap(c: &mut Criterion) {
    let long_line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LONG_LINE_BYTES", 256 * 1024);
    let max_shape_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_MAX_LINE_SHAPE_BYTES", 4 * 1024);
    let fixture = TextInputLongLineCapFixture::new(long_line_bytes);

    let mut group = c.benchmark_group("text_input_long_line_cap");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::new("capped_bytes", max_shape_bytes), |b| {
        b.iter(|| fixture.run_with_cap(max_shape_bytes))
    });
    group.bench_function(BenchmarkId::from_parameter("uncapped_control"), |b| {
        b.iter(|| fixture.run_without_cap())
    });
    group.finish();
}

fn bench_text_input_wrap_incremental_tabs(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);
    let mut full_fixture = TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let mut incremental_fixture =
        TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_wrap_incremental_tabs");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("full_recompute"), |b| {
        let mut edit_ix = 0usize;
        b.iter(|| {
            let h = full_fixture.run_full_recompute_step(edit_ix);
            edit_ix = edit_ix.wrapping_add(17);
            h
        })
    });
    group.bench_function(BenchmarkId::from_parameter("incremental_patch"), |b| {
        let mut edit_ix = 0usize;
        b.iter(|| {
            let h = incremental_fixture.run_incremental_step(edit_ix);
            edit_ix = edit_ix.wrapping_add(17);
            h
        })
    });
    group.finish();
}

fn bench_text_input_wrap_incremental_burst_edits(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);
    let edits_per_burst = env_usize("GITCOMET_BENCH_TEXT_INPUT_BURST_EDITS", 12);
    let mut full_fixture =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let mut incremental_fixture =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_wrap_incremental_burst_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("full_recompute", edits_per_burst),
        &edits_per_burst,
        |b, &edits_per_burst| {
            b.iter(|| full_fixture.run_full_recompute_burst_step(edits_per_burst))
        },
    );
    group.bench_with_input(
        BenchmarkId::new("incremental_patch", edits_per_burst),
        &edits_per_burst,
        |b, &edits_per_burst| {
            b.iter(|| incremental_fixture.run_incremental_burst_step(edits_per_burst))
        },
    );
    group.finish();
}

fn bench_text_model_snapshot_clone_cost(c: &mut Criterion) {
    let bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_BYTES", 2 * 1024 * 1024);
    let clones = env_usize("GITCOMET_BENCH_TEXT_MODEL_SNAPSHOT_CLONES", 8_192);
    let fixture = TextModelSnapshotCloneCostFixture::new(bytes);

    let mut group = c.benchmark_group("text_model_snapshot_clone_cost");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("piece_table_snapshot_clone", clones),
        &clones,
        |b, &clones| b.iter(|| fixture.run_snapshot_clone_step(clones)),
    );
    group.bench_with_input(
        BenchmarkId::new("shared_string_clone_control", clones),
        &clones,
        |b, &clones| b.iter(|| fixture.run_string_clone_control_step(clones)),
    );
    group.finish();
}

fn bench_text_model_bulk_load_large(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_MODEL_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_LINE_BYTES", 128);
    let fixture = TextModelBulkLoadLargeFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("text_model_bulk_load_large");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("piece_table_append_large"),
        |b| b.iter(|| fixture.run_piece_table_bulk_load_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("piece_table_from_large_text"),
        |b| b.iter(|| fixture.run_piece_table_from_large_text_step()),
    );
    group.bench_function(BenchmarkId::from_parameter("string_push_control"), |b| {
        b.iter(|| fixture.run_string_bulk_load_control_step())
    });
    group.finish();
}

fn bench_text_model_fragmented_edits(c: &mut Criterion) {
    let bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_BYTES", 512 * 1024);
    let edits = env_usize("GITCOMET_BENCH_TEXT_MODEL_EDITS", 500);
    let reads = env_usize("GITCOMET_BENCH_TEXT_MODEL_READS_AFTER_EDIT", 64);
    let fixture = TextModelFragmentedEditFixture::new(bytes, edits);

    let mut group = c.benchmark_group("text_model_fragmented_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("piece_table_edits"), |b| {
        b.iter(|| fixture.run_fragmented_edit_step())
    });
    group.bench_function(
        BenchmarkId::from_parameter("materialize_after_edits"),
        |b| b.iter(|| fixture.run_materialize_after_edits_step()),
    );
    group.bench_function(BenchmarkId::new("shared_string_after_edits", reads), |b| {
        b.iter(|| fixture.run_shared_string_after_edits_step(reads))
    });
    group.bench_function(BenchmarkId::from_parameter("string_edit_control"), |b| {
        b.iter(|| fixture.run_string_edit_control_step())
    });
    group.finish();
}

fn bench_file_diff_syntax_prepare(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);
    fixture.prewarm();

    let mut group = c.benchmark_group("file_diff_syntax_prepare");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let mut cold_nonce = 0u64;
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_prepare_cold"),
        |b| {
            b.iter(|| {
                cold_nonce = cold_nonce.wrapping_add(1);
                fixture.run_prepare_cold(cold_nonce)
            })
        },
    );
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_prepare_warm"),
        |b| b.iter(|| fixture.run_prepare_warm()),
    );
    group.finish();
}

fn bench_file_diff_syntax_query_stress(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINES", 256);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINE_BYTES", 4_096);
    let nesting_depth = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_NESTING", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new_query_stress(lines, line_bytes, nesting_depth);

    let mut group = c.benchmark_group("file_diff_syntax_query_stress");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let mut nonce = 0u64;
    group.bench_function(BenchmarkId::from_parameter("nested_long_lines_cold"), |b| {
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            fixture.run_prepare_cold(nonce)
        })
    });
    group.finish();
}

fn bench_file_diff_syntax_reparse(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES", 128);
    let mut small_fixture = FileDiffSyntaxReparseFixture::new(lines, line_bytes);
    let mut large_fixture = FileDiffSyntaxReparseFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("file_diff_syntax_reparse");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_reparse_small_edit"),
        |b| b.iter(|| small_fixture.run_small_edit_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_reparse_large_edit"),
        |b| b.iter(|| large_fixture.run_large_edit_step()),
    );
    group.finish();
}

fn bench_file_diff_inline_syntax_projection(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_LINE_BYTES", 128);
    let window = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_WINDOW", 200);
    let pending_fixture = FileDiffInlineSyntaxProjectionFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("file_diff_inline_syntax_projection");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("visible_window_pending", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let hash = pending_fixture.run_window_pending_step(start, window);
                start = pending_fixture.next_start_row(start, window);
                hash
            })
        },
    );

    let ready_fixture = FileDiffInlineSyntaxProjectionFixture::new(lines, line_bytes);
    ready_fixture.prime_window(window);
    group.bench_with_input(
        BenchmarkId::new("visible_window_ready", window),
        &window,
        |b, &window| b.iter(|| ready_fixture.run_window_step(0, window)),
    );
    group.finish();
}

fn bench_file_diff_syntax_cache_drop(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_LINES", 2_048);
    let tokens_per_line = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_TOKENS_PER_LINE", 8);
    let replacements = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_REPLACEMENTS", 4);
    let fixture = FileDiffSyntaxCacheDropFixture::new(lines, tokens_per_line, replacements);

    let mut group = c.benchmark_group("file_diff_syntax_cache_drop");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("deferred_drop", replacements),
        &replacements,
        |b, &_replacements| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut seed = 0usize;
                for _ in 0..iters {
                    let _ = fixture.flush_deferred_drop_queue();
                    total = total.saturating_add(fixture.run_deferred_drop_timed_step(seed));
                    seed = seed.wrapping_add(1);
                }
                total
            })
        },
    );
    let _ = fixture.flush_deferred_drop_queue();
    group.bench_with_input(
        BenchmarkId::new("inline_drop_control", replacements),
        &replacements,
        |b, &_replacements| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut seed = 0usize;
                for _ in 0..iters {
                    total = total.saturating_add(fixture.run_inline_drop_control_timed_step(seed));
                    seed = seed.wrapping_add(1);
                }
                total
            })
        },
    );
    group.finish();
}

fn bench_prepared_syntax_multidoc_cache_hit_rate(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINE_BYTES", 128);
    let docs = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_HOT_DOCS", 6);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("prepared_syntax_multidoc_cache_hit_rate");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    let mut nonce = 0u64;
    group.bench_with_input(BenchmarkId::new("hot_docs", docs), &docs, |b, &docs| {
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(docs, nonce)
        })
    });
    group.finish();
}

fn bench_prepared_syntax_chunk_miss_cost(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINE_BYTES", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("prepared_syntax_chunk_miss_cost");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    let mut nonce = 0u64;
    group.bench_function(BenchmarkId::from_parameter("chunk_miss"), |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                nonce = nonce.wrapping_add(1);
                total =
                    total.saturating_add(fixture.run_prepared_syntax_chunk_miss_cost_step(nonce));
            }
            total
        })
    });
    group.finish();
}

fn bench_large_html_syntax(c: &mut Criterion) {
    let fixture_path = env_string("GITCOMET_BENCH_HTML_FIXTURE_PATH");
    let synthetic_lines = env_usize("GITCOMET_BENCH_HTML_LINES", 20_000);
    let synthetic_line_bytes = env_usize("GITCOMET_BENCH_HTML_LINE_BYTES", 192);
    let window_lines = env_usize("GITCOMET_BENCH_HTML_WINDOW_LINES", 160);
    let prepare_fixture = LargeHtmlSyntaxFixture::new(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let source_label = prepare_fixture.source_label().to_string();

    let mut group = c.benchmark_group("large_html_syntax");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::new(source_label.as_str(), "background_prepare"),
        |b| b.iter(|| prepare_fixture.run_background_prepare_step()),
    );
    let pending_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let mut pending_start_line = 0usize;
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_pending"),
        &window_lines,
        |b, &window_lines| {
            b.iter(|| {
                let hash = pending_fixture
                    .run_visible_window_pending_step(pending_start_line, window_lines);
                pending_start_line =
                    pending_fixture.next_start_line(pending_start_line, window_lines);
                hash
            })
        },
    );
    let visible_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    visible_fixture.prime_visible_window(window_lines);
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_steady"),
        &window_lines,
        |b, &window_lines| b.iter(|| visible_fixture.run_visible_window_step(0, window_lines)),
    );
    let mut start_line = 0usize;
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_sweep"),
        &window_lines,
        |b, &window_lines| {
            b.iter(|| {
                let hash = visible_fixture.run_visible_window_step(start_line, window_lines);
                start_line = visible_fixture.next_start_line(start_line, window_lines);
                hash
            })
        },
    );
    group.finish();
}

fn bench_worktree_preview_render(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_LINES", 4_000);
    let window = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_LINE_BYTES", 128);
    let fixture = WorktreePreviewRenderFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("worktree_preview_render");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("cached_lookup_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_cached_lookup_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.bench_with_input(
        BenchmarkId::new("render_time_prepare_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_render_time_prepare_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();
}

fn bench_markdown_preview_parse_build(c: &mut Criterion) {
    let medium_sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_MEDIUM_SECTIONS", 256);
    let large_sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_LARGE_SECTIONS", 768);
    let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_LINE_BYTES", 128);
    let medium = MarkdownPreviewFixture::new(medium_sections, line_bytes);
    let large = MarkdownPreviewFixture::new(large_sections, line_bytes);

    let mut group = c.benchmark_group("markdown_preview_parse_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    for (label, fixture) in [("medium", &medium), ("large", &large)] {
        group.bench_function(BenchmarkId::new("single_document", label), |b| {
            b.iter(|| fixture.run_parse_single_step())
        });
        group.bench_function(BenchmarkId::new("two_sided_diff", label), |b| {
            b.iter(|| fixture.run_parse_diff_step())
        });
    }

    group.finish();
}

fn bench_markdown_preview_render(c: &mut Criterion) {
    let sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_SECTIONS", 384);
    let window = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_LINE_BYTES", 128);
    let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

    let mut single_group = c.benchmark_group("markdown_preview_render_single");
    single_group.sample_size(10);
    single_group.warm_up_time(Duration::from_secs(1));
    single_group.bench_with_input(
        BenchmarkId::new("window_rows", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let hash = fixture.run_render_single_step(start, window);
                start = start.wrapping_add(window);
                hash
            })
        },
    );
    single_group.finish();

    let mut diff_group = c.benchmark_group("markdown_preview_render_diff");
    diff_group.sample_size(10);
    diff_group.warm_up_time(Duration::from_secs(1));
    diff_group.bench_with_input(
        BenchmarkId::new("window_rows", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let hash = fixture.run_render_diff_step(start, window);
                start = start.wrapping_add(window);
                hash
            })
        },
    );
    diff_group.finish();
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

fn bench_conflict_three_way_prepared_syntax_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let fixture =
        ConflictThreeWayScrollFixture::new_with_prepared_documents(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_prepared_syntax_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("style_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_prepared_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();
}

fn bench_conflict_three_way_visible_map_build(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictThreeWayVisibleMapBuildFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_visible_map_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("linear_two_pointer"), |b| {
        b.iter(|| fixture.run_linear_step())
    });
    group.bench_function(BenchmarkId::from_parameter("legacy_find_scan"), |b| {
        b.iter(|| fixture.run_legacy_step())
    });
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

fn bench_conflict_load_duplication(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictLoadDuplicationFixture::new(lines, low_density_blocks);
    let high_density = ConflictLoadDuplicationFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_load_duplication");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("shared_payload_forwarding", label), |b| {
            b.iter(|| fixture.run_shared_payload_forwarding_step())
        });
        group.bench_function(BenchmarkId::new("duplicated_text_and_bytes", label), |b| {
            b.iter(|| fixture.run_duplicated_payload_forwarding_step())
        });
    }
    group.finish();
}

fn bench_conflict_two_way_diff_build(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictTwoWayDiffBuildFixture::new(lines, low_density_blocks);
    let high_density = ConflictTwoWayDiffBuildFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_two_way_diff_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("full_file", label), |b| {
            b.iter(|| fixture.run_full_diff_build_step())
        });
        group.bench_function(BenchmarkId::new("block_local", label), |b| {
            b.iter(|| fixture.run_block_local_diff_build_step())
        });
    }
    group.finish();
}

fn bench_conflict_two_way_word_highlights(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictTwoWayDiffBuildFixture::new(lines, low_density_blocks);
    let high_density = ConflictTwoWayDiffBuildFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_two_way_word_highlights");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("full_file", label), |b| {
            b.iter(|| fixture.run_full_word_highlights_step())
        });
        group.bench_function(BenchmarkId::new("block_local", label), |b| {
            b.iter(|| fixture.run_block_local_word_highlights_step())
        });
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

fn bench_patch_diff_search_query_update(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 10_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let mut fixture = PatchDiffSearchQueryUpdateFixture::new(lines);
    let query_cycle = [
        "s", "sh", "sha", "shar", "share", "shared", "shared_", "shared_1",
    ];

    let mut group = c.benchmark_group("patch_diff_search_query_update");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("window_{window}")),
        &window,
        |b, &window| {
            let mut start = 0usize;
            let mut query_ix = 0usize;
            b.iter(|| {
                let query = query_cycle[query_ix % query_cycle.len()];
                let h = fixture.run_query_update_step(query, start, window);
                query_ix = query_ix.wrapping_add(1);
                start = start.wrapping_add(window.max(1) / 2 + 1) % fixture.visible_rows().max(1);
                h
            })
        },
    );
    group.finish();
}

fn bench_patch_diff_paged_rows(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let mut group = c.benchmark_group("patch_diff_paged_rows");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("eager_full_materialize"), |b| {
        b.iter(|| fixture.run_eager_full_materialize_step())
    });
    group.bench_with_input(
        BenchmarkId::new("paged_first_window", window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.bench_function(
        BenchmarkId::from_parameter("inline_visible_eager_scan"),
        |b| b.iter(|| fixture.run_inline_visible_eager_scan_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("inline_visible_hidden_map"),
        |b| b.iter(|| fixture.run_inline_visible_hidden_map_step()),
    );
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

fn bench_conflict_streamed_provider(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_STREAMED_LINES", 50_000);
    let window = env_usize("GITCOMET_BENCH_STREAMED_WINDOW", 200);

    let fixture = ConflictStreamedProviderFixture::new(lines);

    let mut group = c.benchmark_group("conflict_streamed_provider");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("index_build"), |b| {
        b.iter(|| fixture.run_index_build_step())
    });
    group.bench_function(BenchmarkId::from_parameter("projection_build"), |b| {
        b.iter(|| fixture.run_projection_build_step())
    });
    group.bench_with_input(BenchmarkId::new("first_page", window), &window, |b, &w| {
        b.iter(|| fixture.run_first_page_step(w))
    });
    fixture.prime_first_page_cache(window);
    group.bench_with_input(
        BenchmarkId::new("first_page_cache_hit", window),
        &window,
        |b, &w| b.iter(|| fixture.run_first_page_cache_hit_step(w)),
    );
    group.bench_with_input(
        BenchmarkId::new("deep_scroll_50pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_scroll_step(0.5, w)),
    );
    group.bench_with_input(
        BenchmarkId::new("deep_scroll_90pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_scroll_step(0.9, w)),
    );
    group.bench_function(BenchmarkId::from_parameter("search_rare_text"), |b| {
        b.iter(|| fixture.run_search_step("shared_42("))
    });
    group.bench_function(BenchmarkId::from_parameter("search_common_text"), |b| {
        b.iter(|| fixture.run_search_step("compute"))
    });
    group.finish();
}

fn bench_conflict_streamed_resolved_output(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_STREAMED_LINES", 50_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 500);
    let window = env_usize("GITCOMET_BENCH_STREAMED_WINDOW", 200);

    let fixture = ConflictStreamedResolvedOutputFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_streamed_resolved_output");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("projection_build"), |b| {
        b.iter(|| fixture.run_projection_build_step())
    });
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &w| {
        b.iter(|| fixture.run_window_step(w))
    });
    group.bench_with_input(
        BenchmarkId::new("deep_window_90pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_window_step(0.9, w)),
    );
    group.finish();
}

fn bench_resolved_output_recompute_incremental(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let mut full_fixture = ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let mut incremental_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("resolved_output_recompute_incremental");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("full_recompute"), |b| {
        b.iter(|| full_fixture.run_full_recompute_step())
    });
    group.bench_function(BenchmarkId::from_parameter("incremental_recompute"), |b| {
        b.iter(|| incremental_fixture.run_incremental_recompute_step())
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
    bench_file_diff_replacement_alignment,
    bench_text_input_prepaint_windowed,
    bench_text_input_runs_streamed_highlight,
    bench_text_input_long_line_cap,
    bench_text_input_wrap_incremental_tabs,
    bench_text_input_wrap_incremental_burst_edits,
    bench_text_model_snapshot_clone_cost,
    bench_text_model_bulk_load_large,
    bench_text_model_fragmented_edits,
    bench_file_diff_syntax_prepare,
    bench_file_diff_syntax_query_stress,
    bench_file_diff_syntax_reparse,
    bench_file_diff_inline_syntax_projection,
    bench_file_diff_syntax_cache_drop,
    bench_prepared_syntax_multidoc_cache_hit_rate,
    bench_prepared_syntax_chunk_miss_cost,
    bench_large_html_syntax,
    bench_worktree_preview_render,
    bench_markdown_preview_parse_build,
    bench_markdown_preview_render,
    bench_conflict_three_way_scroll,
    bench_conflict_three_way_prepared_syntax_scroll,
    bench_conflict_three_way_visible_map_build,
    bench_conflict_two_way_split_scroll,
    bench_conflict_load_duplication,
    bench_conflict_two_way_diff_build,
    bench_conflict_two_way_word_highlights,
    bench_conflict_resolved_output_gutter_scroll,
    bench_conflict_search_query_update,
    bench_patch_diff_search_query_update,
    bench_patch_diff_paged_rows,
    bench_conflict_split_resize_step,
    bench_conflict_streamed_provider,
    bench_conflict_streamed_resolved_output,
    bench_resolved_output_recompute_incremental
);
criterion_main!(benches);
