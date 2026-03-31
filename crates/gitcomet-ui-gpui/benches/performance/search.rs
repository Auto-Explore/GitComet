use super::common::*;

pub(crate) fn bench_search(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_SEARCH_COMMITS", 50_000);
    let diff_lines = env_usize("GITCOMET_BENCH_SEARCH_DIFF_LINES", 100_000);
    let file_preview_lines = env_usize("GITCOMET_BENCH_SEARCH_FILE_PREVIEW_LINES", 100_000);
    let file_diff_lines = env_usize("GITCOMET_BENCH_SEARCH_FILE_DIFF_LINES", 100_000);
    let file_diff_window = env_usize("GITCOMET_BENCH_SEARCH_FILE_DIFF_WINDOW", 200);
    let fuzzy_files = env_usize("GITCOMET_BENCH_SEARCH_FUZZY_FILES", 100_000);

    let fixture = CommitSearchFilterFixture::new(commits);
    let diff_fixture = InDiffTextSearchFixture::new(diff_lines);
    let file_preview_fixture = FilePreviewTextSearchFixture::new(file_preview_lines);
    let file_diff_fixture = FileDiffCtrlFOpenTypeFixture::new(file_diff_lines, file_diff_window);
    let fuzzy_fixture = FileFuzzyFindFixture::new(fuzzy_files);

    // Author query: "Alice" matches ~10% of commits (1 of 10 first names).
    let author_query =
        env_string("GITCOMET_BENCH_SEARCH_AUTHOR_QUERY").unwrap_or_else(|| "Alice".to_string());
    // Message query: "fix" matches ~10% of commits (1 of 10 prefixes).
    let message_query =
        env_string("GITCOMET_BENCH_SEARCH_MESSAGE_QUERY").unwrap_or_else(|| "fix".to_string());
    // Diff query: `render_cache` matches context rows and modified rows; the
    // refined query narrows to the hot-path subset of modified rows.
    let diff_query = env_string("GITCOMET_BENCH_SEARCH_DIFF_QUERY")
        .unwrap_or_else(|| "render_cache".to_string());
    let diff_refined_query = env_string("GITCOMET_BENCH_SEARCH_DIFF_REFINED_QUERY")
        .unwrap_or_else(|| "render_cache_hot_path".to_string());
    let diff_refinement_matches = diff_fixture.prepare_matches(&diff_query);
    let file_preview_query = env_string("GITCOMET_BENCH_SEARCH_FILE_PREVIEW_QUERY")
        .unwrap_or_else(|| "render_cache".to_string());
    let file_diff_query = env_string("GITCOMET_BENCH_SEARCH_FILE_DIFF_QUERY")
        .unwrap_or_else(|| "render_cache_hot_path".to_string());
    // Fuzzy file-find query: "dcrs" is a realistic 4-char subsequence that
    // matches paths containing d…c…r…s (e.g. "diff_cache…rs"). The incremental
    // keystroke benchmark types "dc" first then extends to "dcrs".
    let fuzzy_query =
        env_string("GITCOMET_BENCH_SEARCH_FUZZY_QUERY").unwrap_or_else(|| "dcrs".to_string());
    let fuzzy_short_query =
        env_string("GITCOMET_BENCH_SEARCH_FUZZY_SHORT_QUERY").unwrap_or_else(|| "dc".to_string());

    let mut group = c.benchmark_group("search");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("commit_filter_by_author_50k_commits"),
        |b| {
            b.iter(|| fixture.run_filter_by_author(&author_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("commit_filter_by_message_50k_commits"),
        |b| {
            b.iter(|| fixture.run_filter_by_message(&message_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("in_diff_text_search_100k_lines"),
        |b| {
            b.iter(|| diff_fixture.run_search(&diff_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("in_diff_text_search_incremental_refinement"),
        |b| {
            b.iter(|| {
                diff_fixture
                    .run_refinement_from_matches(&diff_refined_query, &diff_refinement_matches)
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_preview_text_search_100k_lines"),
        |b| {
            b.iter(|| file_preview_fixture.run_search(&file_preview_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_diff_ctrl_f_open_and_type_100k_lines"),
        |b| {
            b.iter(|| file_diff_fixture.run_open_and_type(&file_diff_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_fuzzy_find_100k_files"),
        |b| {
            b.iter(|| fuzzy_fixture.run_find(&fuzzy_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_fuzzy_find_incremental_keystroke"),
        |b| {
            b.iter(|| fuzzy_fixture.run_incremental(&fuzzy_short_query, &fuzzy_query));
        },
    );

    // Emit sidecar metrics from a final run.
    let (_, author_metrics) =
        measure_sidecar_allocations(|| fixture.run_filter_by_author_with_metrics(&author_query));
    emit_commit_search_filter_sidecar("commit_filter_by_author_50k_commits", &author_metrics);
    let (_, message_metrics) =
        measure_sidecar_allocations(|| fixture.run_filter_by_message_with_metrics(&message_query));
    emit_commit_search_filter_sidecar("commit_filter_by_message_50k_commits", &message_metrics);
    let (_, diff_metrics) =
        measure_sidecar_allocations(|| diff_fixture.run_search_with_metrics(&diff_query));
    emit_in_diff_text_search_sidecar("in_diff_text_search_100k_lines", &diff_metrics);
    let (_, refinement_metrics) = measure_sidecar_allocations(|| {
        diff_fixture.run_refinement_with_metrics(&diff_query, &diff_refined_query)
    });
    emit_in_diff_text_search_sidecar(
        "in_diff_text_search_incremental_refinement",
        &refinement_metrics,
    );
    let (_, file_preview_metrics) = measure_sidecar_allocations(|| {
        file_preview_fixture.run_search_with_metrics(&file_preview_query)
    });
    emit_file_preview_text_search_sidecar(
        "file_preview_text_search_100k_lines",
        &file_preview_metrics,
    );
    let (_, file_diff_ctrl_f_metrics) = measure_sidecar_allocations(|| {
        file_diff_fixture.run_open_and_type_with_metrics(&file_diff_query)
    });
    emit_file_diff_ctrl_f_open_type_sidecar(
        "file_diff_ctrl_f_open_and_type_100k_lines",
        &file_diff_ctrl_f_metrics,
    );
    let (_, fuzzy_metrics) =
        measure_sidecar_allocations(|| fuzzy_fixture.run_find_with_metrics(&fuzzy_query));
    emit_file_fuzzy_find_sidecar("file_fuzzy_find_100k_files", &fuzzy_metrics);
    let (_, fuzzy_incr_metrics) = measure_sidecar_allocations(|| {
        fuzzy_fixture.run_incremental_with_metrics(&fuzzy_short_query, &fuzzy_query)
    });
    emit_file_fuzzy_find_sidecar("file_fuzzy_find_incremental_keystroke", &fuzzy_incr_metrics);

    group.finish();
}
