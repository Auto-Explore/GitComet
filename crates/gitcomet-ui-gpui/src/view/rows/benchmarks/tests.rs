use super::*;
use crate::perf_alloc::measure_allocations;
use gitcomet_core::conflict_session::ConflictPayload;

#[test]
fn status_list_fixture_reports_visible_window_metrics() {
    let mut fixture = StatusListFixture::unstaged_large(10_000);
    let metrics = fixture.measure_window_step(0, 200);

    assert_eq!(
        metrics,
        StatusListMetrics {
            rows_requested: 200,
            rows_painted: 200,
            entries_total: 10_000,
            path_display_cache_hits: 0,
            path_display_cache_misses: 200,
            path_display_cache_clears: 0,
            max_path_depth: 5,
            prewarmed_entries: 0,
        }
    );
}

#[test]
fn status_list_staged_fixture_reports_visible_window_metrics() {
    let mut fixture = StatusListFixture::staged_large(10_000);
    let metrics = fixture.measure_window_step(0, 200);

    assert_eq!(
        metrics,
        StatusListMetrics {
            rows_requested: 200,
            rows_painted: 200,
            entries_total: 10_000,
            path_display_cache_hits: 0,
            path_display_cache_misses: 200,
            path_display_cache_clears: 0,
            max_path_depth: 5,
            prewarmed_entries: 0,
        }
    );
}

#[test]
fn status_list_mixed_depth_fixture_reports_cache_churn_metrics() {
    let mut fixture = StatusListFixture::mixed_depth(20_000);
    let metrics = fixture.measure_window_step_with_prewarm(8_193, 200, 8_193);

    assert_eq!(
        metrics,
        StatusListMetrics {
            rows_requested: 200,
            rows_painted: 200,
            entries_total: 20_000,
            path_display_cache_hits: 0,
            path_display_cache_misses: 200,
            path_display_cache_clears: 0,
            max_path_depth: 15,
            prewarmed_entries: 8_193,
        }
    );
}

#[test]
fn status_multi_select_fixture_reports_range_metrics() {
    let fixture = StatusMultiSelectFixture::range_select(20_000, 4_096, 512);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.entries_total, 20_000);
    assert_eq!(metrics.selected_paths, 512);
    assert_eq!(metrics.anchor_index, 4_096);
    assert_eq!(metrics.clicked_index, 4_607);
    assert_eq!(metrics.anchor_preserved, 1);
    // With anchor_index hints the lookup is O(1), so zero scan steps.
    assert_eq!(metrics.position_scan_steps, 0);
}

#[test]
fn git_ops_status_fixture_reports_dirty_status_metrics() {
    let fixture = GitOpsFixture::status_dirty(32, 8);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.tracked_files, 32);
    assert_eq!(metrics.dirty_files, 8);
    assert_eq!(metrics.status_calls, 1);
    assert_eq!(metrics.log_walk_calls, 0);
    assert_eq!(metrics.diff_calls, 0);
    assert_eq!(metrics.blame_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.status_ms > 0.0);
}

#[test]
fn git_ops_log_walk_fixture_reports_commit_count_metrics() {
    let fixture = GitOpsFixture::log_walk(512, 256);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.total_commits, 512);
    assert_eq!(metrics.requested_commits, 256);
    assert_eq!(metrics.commits_returned, 256);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 1);
    assert_eq!(metrics.diff_calls, 0);
    assert_eq!(metrics.blame_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.log_walk_ms > 0.0);
}

#[test]
fn git_ops_diff_rename_fixture_reports_rename_metrics() {
    let fixture = GitOpsFixture::diff_rename_heavy(8);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.changed_files, 8);
    assert_eq!(metrics.renamed_files, 8);
    assert_eq!(metrics.binary_files, 0);
    assert_eq!(metrics.line_count, 0);
    assert!(metrics.diff_lines > 0);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 0);
    assert_eq!(metrics.diff_calls, 1);
    assert_eq!(metrics.blame_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.diff_ms > 0.0);
}

#[test]
fn git_ops_diff_binary_fixture_reports_binary_metrics() {
    let fixture = GitOpsFixture::diff_binary_heavy(4, 1_024);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.changed_files, 4);
    assert_eq!(metrics.renamed_files, 0);
    assert_eq!(metrics.binary_files, 4);
    assert_eq!(metrics.line_count, 0);
    assert!(metrics.diff_lines >= 12);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 0);
    assert_eq!(metrics.diff_calls, 1);
    assert_eq!(metrics.blame_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.diff_ms > 0.0);
}

#[test]
fn git_ops_large_single_file_fixture_reports_line_count_metrics() {
    let fixture = GitOpsFixture::diff_large_single_file(512, 32);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.changed_files, 1);
    assert_eq!(metrics.renamed_files, 0);
    assert_eq!(metrics.binary_files, 0);
    assert_eq!(metrics.line_count, 512);
    assert!(metrics.diff_lines >= 1_024);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 0);
    assert_eq!(metrics.diff_calls, 1);
    assert_eq!(metrics.blame_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.diff_ms > 0.0);
}

#[test]
fn git_ops_blame_large_file_fixture_reports_blame_metrics() {
    let fixture = GitOpsFixture::blame_large_file(256, 8);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.total_commits, 8);
    assert_eq!(metrics.line_count, 256);
    assert_eq!(metrics.blame_lines, 256);
    assert_eq!(metrics.blame_distinct_commits, 8);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 0);
    assert_eq!(metrics.diff_calls, 0);
    assert_eq!(metrics.blame_calls, 1);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.blame_ms > 0.0);
}

#[test]
fn git_ops_file_history_fixture_reports_metrics() {
    let fixture = GitOpsFixture::file_history(1_000, 100, 5);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.total_commits, 1_000);
    assert_eq!(metrics.file_history_commits, 200);
    assert_eq!(metrics.requested_commits, 100);
    assert_eq!(metrics.commits_returned, 100);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 1);
    assert_eq!(metrics.diff_calls, 0);
    assert_eq!(metrics.blame_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.log_walk_ms > 0.0);
}

#[test]
fn merge_open_bootstrap_small_fixture_reports_eager_trace_metrics() {
    let fixture = MergeOpenBootstrapFixture::small(5_000);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_ne!(hash_with_trace, 0);
    assert_eq!(metrics.trace_event_count, 7);
    assert_eq!(metrics.conflict_block_count, 0);
    assert_eq!(metrics.rendering_mode_streamed, 0);
    assert_eq!(metrics.full_output_generated, 1);
    assert_eq!(metrics.whole_block_diff_ran, 0);
    assert_eq!(metrics.diff_row_count, 0);
    assert_eq!(metrics.inline_row_count, 0);
    assert_eq!(metrics.two_way_visible_rows, 0);
    assert_eq!(metrics.three_way_visible_rows, fixture.line_count() as u64);
    assert_eq!(
        metrics.resolved_output_line_count,
        fixture.line_count() as u64
    );
    assert!(metrics.bootstrap_total_ms >= metrics.parse_conflict_markers_ms);
}

#[test]
fn merge_open_bootstrap_fixture_reports_streamed_trace_metrics() {
    let fixture = MergeOpenBootstrapFixture::large_streamed(55_001, 1);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_ne!(hash_with_trace, 0);
    assert_eq!(metrics.trace_event_count, 7);
    assert_eq!(
        metrics.conflict_block_count,
        fixture.conflict_count() as u64
    );
    assert_eq!(metrics.rendering_mode_streamed, 1);
    assert_eq!(metrics.full_output_generated, 0);
    assert_eq!(metrics.full_syntax_parse_requested, 1);
    assert_eq!(metrics.whole_block_diff_ran, 0);
    assert_eq!(metrics.inline_row_count, 0);
    assert!(metrics.diff_row_count > 0);
    assert!(metrics.two_way_visible_rows > 0);
    assert_eq!(metrics.three_way_visible_rows, fixture.line_count() as u64);
    assert!(metrics.resolved_output_line_count >= fixture.line_count() as u64);
    assert!(metrics.bootstrap_total_ms >= metrics.parse_conflict_markers_ms);
}

#[test]
fn merge_open_bootstrap_many_conflicts_fixture_reports_correct_block_count() {
    let fixture = MergeOpenBootstrapFixture::many_conflicts(50);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_ne!(hash_with_trace, 0);
    assert_eq!(metrics.trace_event_count, 7);
    assert_eq!(metrics.conflict_block_count, 50);
    assert_eq!(metrics.rendering_mode_streamed, 1);
    assert_eq!(metrics.full_output_generated, 0);
    assert_eq!(metrics.whole_block_diff_ran, 0);
    assert_eq!(metrics.inline_row_count, 0);
    assert!(metrics.diff_row_count > 0);
    assert!(metrics.two_way_visible_rows > 0);
    assert!(metrics.three_way_visible_rows > 0);
    assert!(metrics.resolved_output_line_count > 0);
    assert!(metrics.bootstrap_total_ms >= metrics.parse_conflict_markers_ms);
}

#[test]
fn merge_open_bootstrap_large_many_conflicts_fixture_reports_extreme_scale() {
    let fixture = MergeOpenBootstrapFixture::large_many_conflicts(50_000, 500);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    // At least 7 bootstrap stages; with many blocks, inner functions may
    // emit additional trace events.
    assert!(metrics.trace_event_count >= 7);
    assert_eq!(metrics.conflict_block_count, 500);
    assert_eq!(metrics.rendering_mode_streamed, 1);
    assert_eq!(metrics.full_output_generated, 0);
    assert_eq!(metrics.whole_block_diff_ran, 0);
    assert_eq!(metrics.inline_row_count, 0);
    assert!(metrics.diff_row_count > 0);
    assert!(metrics.two_way_visible_rows > 0);
    assert!(metrics.resolved_output_line_count >= 20_000);
    assert!(metrics.bootstrap_total_ms > 0.0);
}

#[test]
fn conflict_three_way_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictThreeWayScrollFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert_eq!(fixture.visible_rows(), 120);
}

#[test]
fn conflict_three_way_fixture_wraps_start_offsets() {
    let fixture = ConflictThreeWayScrollFixture::new(180, 18);
    let hash_a = fixture.run_scroll_step(17, 40);
    let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn conflict_three_way_fixture_uses_auto_syntax_for_large_inputs() {
    let fixture = ConflictThreeWayScrollFixture::new(6_000, 24);
    assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
}

#[test]
fn conflict_three_way_prepared_fixture_has_documents_for_all_sides() {
    let fixture = ConflictThreeWayScrollFixture::new_with_prepared_documents(120, 12);
    assert!(
        fixture.has_prepared_documents(),
        "prepared fixture should produce documents for base, ours, and theirs"
    );
    assert_eq!(fixture.conflict_count(), 12);
    assert_eq!(fixture.visible_rows(), 120);
}

#[test]
fn conflict_three_way_prepared_fixture_scroll_step_differs_from_fallback() {
    let prepared = ConflictThreeWayScrollFixture::new_with_prepared_documents(180, 18);
    let fallback = ConflictThreeWayScrollFixture::new(180, 18);
    let prepared_hash = prepared.run_prepared_scroll_step(0, 40);
    let fallback_hash = fallback.run_scroll_step(0, 40);
    // The prepared-document path includes pending state and uses tree-sitter
    // highlights, so its hash should generally differ from the per-line
    // fallback path.
    assert_ne!(
        prepared_hash, fallback_hash,
        "prepared-document and fallback scroll steps should produce different hashes"
    );
}

#[test]
fn conflict_three_way_prepared_fixture_wraps_start_offsets() {
    let fixture = ConflictThreeWayScrollFixture::new_with_prepared_documents(180, 18);
    let hash_a = fixture.run_prepared_scroll_step(17, 40);
    let hash_b = fixture.run_prepared_scroll_step(17 + fixture.visible_rows() * 3, 40);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn conflict_three_way_plain_fixture_has_no_prepared_documents() {
    let fixture = ConflictThreeWayScrollFixture::new(120, 12);
    assert!(
        !fixture.has_prepared_documents(),
        "plain fixture should not have prepared documents"
    );
}

#[test]
fn conflict_three_way_visible_map_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictThreeWayVisibleMapBuildFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert!(fixture.visible_rows() > 0);
}

#[test]
fn conflict_three_way_visible_map_fixture_linear_matches_legacy_scan() {
    let fixture = ConflictThreeWayVisibleMapBuildFixture::new(240, 24);
    assert_eq!(fixture.build_linear_map(), fixture.build_legacy_map());
    assert_eq!(fixture.run_linear_step(), fixture.run_legacy_step());
}

#[test]
fn conflict_two_way_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictTwoWaySplitScrollFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert!(fixture.visible_rows() > 0);
}

#[test]
fn conflict_two_way_fixture_matches_block_local_rows_and_auto_syntax() {
    let fixture = ConflictTwoWaySplitScrollFixture::new(1_200, 12);
    let build_fixture = ConflictTwoWayDiffBuildFixture::new(1_200, 12);
    assert_eq!(fixture.diff_rows(), build_fixture.block_local_diff_rows());
    assert!(fixture.diff_rows() < build_fixture.full_diff_rows());
    assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
}

#[test]
fn conflict_two_way_fixture_wraps_start_offsets() {
    let fixture = ConflictTwoWaySplitScrollFixture::new(180, 18);
    let hash_a = fixture.run_scroll_step(17, 40);
    let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn conflict_two_way_diff_build_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictTwoWayDiffBuildFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert!(fixture.full_diff_rows() >= 120);
    assert!(fixture.block_local_diff_rows() > 0);
}

#[test]
fn conflict_two_way_diff_build_fixture_keeps_block_local_rows_sparse() {
    let fixture = ConflictTwoWayDiffBuildFixture::new(1_200, 12);
    assert!(
        fixture.block_local_diff_rows() < fixture.full_diff_rows(),
        "block-local rows should stay smaller than the full-file diff for sparse conflicts"
    );
}

#[test]
fn conflict_two_way_diff_build_fixture_runs_build_and_highlight_paths() {
    let fixture = ConflictTwoWayDiffBuildFixture::new(240, 24);
    assert_ne!(fixture.run_full_diff_build_step(), 0);
    assert_ne!(fixture.run_block_local_diff_build_step(), 0);
    assert_ne!(fixture.run_full_word_highlights_step(), 0);
    assert_ne!(fixture.run_block_local_word_highlights_step(), 0);
}

#[test]
fn conflict_load_duplication_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictLoadDuplicationFixture::new(1_200, 12);
    assert_eq!(fixture.line_count(), 1_200);
    assert_eq!(fixture.conflict_count(), 12);
}

#[test]
fn conflict_load_duplication_fixture_reuses_shared_payloads_only_in_shared_path() {
    let fixture = ConflictLoadDuplicationFixture::new(240, 24);
    let shared = fixture.build_shared_conflict_file();
    let duplicated = fixture.build_duplicated_conflict_file();

    let ConflictPayload::Text(base_payload) = &fixture.session().base else {
        panic!("synthetic conflict-load fixture should use text payloads");
    };
    let shared_base = shared
        .base
        .as_ref()
        .expect("shared conflict file should keep base text");
    let duplicated_base = duplicated
        .base
        .as_ref()
        .expect("duplicated conflict file should keep base text");

    assert!(Arc::ptr_eq(base_payload, shared_base));
    assert!(!Arc::ptr_eq(base_payload, duplicated_base));
    assert!(shared.base_bytes.is_none());
    assert!(duplicated.base_bytes.is_none());

    let shared_current = shared
        .current
        .as_ref()
        .expect("shared conflict file should keep current text");
    let duplicated_current = duplicated
        .current
        .as_ref()
        .expect("duplicated conflict file should keep current text");
    assert!(Arc::ptr_eq(fixture.current_text(), shared_current));
    assert!(!Arc::ptr_eq(fixture.current_text(), duplicated_current));
    assert!(shared.current_bytes.is_none());
    assert!(duplicated.current_bytes.is_none());
}

#[test]
fn conflict_load_duplication_fixture_runs_shared_and_duplicated_paths() {
    let fixture = ConflictLoadDuplicationFixture::new(240, 24);
    assert_ne!(fixture.run_shared_payload_forwarding_step(), 0);
    assert_ne!(fixture.run_duplicated_payload_forwarding_step(), 0);
}

#[test]
fn conflict_search_query_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictSearchQueryUpdateFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert!(fixture.visible_rows() > 0);
    assert!(fixture.stable_cache_entries() > 0);
}

#[test]
fn conflict_search_query_fixture_uses_block_local_rows_and_auto_syntax() {
    let fixture = ConflictSearchQueryUpdateFixture::new(1_200, 12);
    let build_fixture = ConflictTwoWayDiffBuildFixture::new(1_200, 12);
    assert_eq!(fixture.diff_rows(), build_fixture.block_local_diff_rows());
    assert!(fixture.diff_rows() < build_fixture.full_diff_rows());
    assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
}

#[test]
fn conflict_search_query_fixture_reuses_stable_cache_across_queries() {
    let mut fixture = ConflictSearchQueryUpdateFixture::new(180, 18);
    let stable_before = fixture.stable_cache_entries();
    assert_eq!(fixture.query_cache_entries(), 0);

    let _ = fixture.run_query_update_step("conf", 5, 40);
    let first_query_cache = fixture.query_cache_entries();
    assert!(first_query_cache > 0);
    assert_eq!(fixture.stable_cache_entries(), stable_before);

    let _ = fixture.run_query_update_step("conflict", 5, 40);
    let second_query_cache = fixture.query_cache_entries();
    assert!(second_query_cache > 0);
    assert_eq!(fixture.stable_cache_entries(), stable_before);
}

#[test]
fn conflict_search_query_fixture_wraps_start_offsets() {
    let mut fixture = ConflictSearchQueryUpdateFixture::new(180, 18);
    let hash_a = fixture.run_query_update_step("shared", 17, 40);
    let hash_b = fixture.run_query_update_step("shared", 17 + fixture.visible_rows() * 3, 40);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn patch_diff_search_query_fixture_tracks_requested_rows() {
    let fixture = PatchDiffSearchQueryUpdateFixture::new(240);
    assert_eq!(fixture.visible_rows(), 240);
    assert!(fixture.stable_cache_entries() > 0);
    assert_eq!(fixture.query_cache_entries(), 0);
}

#[test]
fn patch_diff_search_query_fixture_reuses_stable_cache_across_queries() {
    let mut fixture = PatchDiffSearchQueryUpdateFixture::new(360);
    let stable_before = fixture.stable_cache_entries();
    assert_eq!(fixture.query_cache_entries(), 0);

    let _ = fixture.run_query_update_step("shared", 20, 80);
    let stable_after_first = fixture.stable_cache_entries();
    let first_query_entries = fixture.query_cache_entries();
    assert!(first_query_entries > 0);
    assert!(stable_after_first >= stable_before);

    let _ = fixture.run_query_update_step("compute_shared", 20, 80);
    let stable_after_second = fixture.stable_cache_entries();
    let second_query_entries = fixture.query_cache_entries();
    assert!(second_query_entries > 0);
    assert_eq!(stable_after_second, stable_after_first);
}

#[test]
fn patch_diff_search_query_fixture_wraps_start_offsets() {
    let mut fixture = PatchDiffSearchQueryUpdateFixture::new(420);
    let hash_a = fixture.run_query_update_step("shared", 31, 120);
    let hash_b = fixture.run_query_update_step("shared", 31 + fixture.visible_rows() * 2, 120);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn patch_diff_paged_rows_fixture_builds_requested_line_count() {
    let fixture = PatchDiffPagedRowsFixture::new(1_024);
    assert!(fixture.total_rows() >= 1_024);
}

#[test]
fn patch_diff_paged_rows_fixture_runs_eager_and_paged_paths() {
    let fixture = PatchDiffPagedRowsFixture::new(2_048);
    let eager = fixture.run_eager_full_materialize_step();
    let paged = fixture.run_paged_first_window_step(160);
    assert_ne!(eager, 0);
    assert_ne!(paged, 0);
}

#[test]
fn patch_diff_paged_rows_fixture_inline_visible_map_matches_eager_scan() {
    let fixture = PatchDiffPagedRowsFixture::new(2_048);
    assert_eq!(
        fixture.inline_visible_indices_map(),
        fixture.inline_visible_indices_eager()
    );
}

#[test]
fn patch_diff_paged_rows_fixture_runs_inline_visible_paths() {
    let fixture = PatchDiffPagedRowsFixture::new(2_048);
    let eager = fixture.run_inline_visible_eager_scan_step();
    let mapped = fixture.run_inline_visible_hidden_map_step();
    assert_ne!(eager, 0);
    assert_ne!(mapped, 0);
}

#[test]
fn patch_diff_paged_rows_fixture_reports_first_window_metrics() {
    let fixture = PatchDiffPagedRowsFixture::new(20_000);
    let metrics = fixture.measure_paged_first_window_step(200);
    assert_eq!(metrics.rows_requested, 200);
    assert_eq!(metrics.patch_rows_painted, 200);
    assert_eq!(metrics.patch_rows_materialized, 256);
    assert_eq!(metrics.patch_page_cache_entries, 1);
    assert_eq!(metrics.split_rows_painted, 200);
    assert!(metrics.split_rows_materialized < 256);
    assert_eq!(metrics.full_text_materializations, 0);
}

#[test]
fn conflict_split_resize_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictSplitResizeStepFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert!(fixture.visible_rows() > 0);
}

#[test]
fn conflict_split_resize_fixture_reuses_caches_across_drag_steps() {
    let mut fixture = ConflictSplitResizeStepFixture::new(180, 18);
    let stable_before = fixture.stable_cache_entries();
    assert_eq!(fixture.query_cache_entries(), 0);

    let _ = fixture.run_resize_step("shared", 5, 40);
    let ratio_after_first = fixture.split_ratio();
    let first_query_cache = fixture.query_cache_entries();
    assert!(first_query_cache > 0);
    assert_eq!(fixture.stable_cache_entries(), stable_before);

    let _ = fixture.run_resize_step("shared", 25, 40);
    let ratio_after_second = fixture.split_ratio();
    let second_query_cache = fixture.query_cache_entries();
    assert!((ratio_after_first - ratio_after_second).abs() > f32::EPSILON);
    assert!(second_query_cache >= first_query_cache);
    assert_eq!(fixture.stable_cache_entries(), stable_before);
}

#[test]
fn conflict_split_resize_fixture_clamps_ratio_bounds() {
    let mut fixture = ConflictSplitResizeStepFixture::new(180, 18);
    for _ in 0..400 {
        let _ = fixture.run_resize_step("shared", 0, 32);
        let ratio = fixture.split_ratio();
        assert!((0.1..=0.9).contains(&ratio));
    }
}

#[test]
fn conflict_resolved_output_gutter_fixture_tracks_requested_conflict_blocks() {
    let fixture = ConflictResolvedOutputGutterScrollFixture::new(120, 12);
    assert_eq!(fixture.conflict_count(), 12);
    assert!(fixture.visible_rows() > 0);
}

#[test]
fn conflict_resolved_output_gutter_fixture_wraps_start_offsets() {
    let fixture = ConflictResolvedOutputGutterScrollFixture::new(180, 18);
    let hash_a = fixture.run_scroll_step(17, 40);
    let hash_b = fixture.run_scroll_step(17 + fixture.visible_rows() * 3, 40);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn history_list_scroll_fixture_tracks_requested_commit_count() {
    let fixture = HistoryListScrollFixture::new(5_000, 200, 800);
    assert_eq!(fixture.total_rows(), 5_000);
}

#[test]
fn history_list_scroll_fixture_wraps_to_last_visible_window() {
    let fixture = HistoryListScrollFixture::new(5_000, 200, 800);
    let window = 120;
    let hash_a = fixture.run_scroll_step(4_880, window);
    let hash_b = fixture.run_scroll_step(5_200, window);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn resolved_output_recompute_incremental_fixture_tracks_rows() {
    let fixture = ResolvedOutputRecomputeIncrementalFixture::new(240, 24);
    assert!(fixture.visible_rows() > 0);
}

#[test]
fn resolved_output_recompute_incremental_fixture_runs_full_and_incremental_steps() {
    let mut fixture = ResolvedOutputRecomputeIncrementalFixture::new(240, 24);
    let full_hash = fixture.run_full_recompute_step();
    let incremental_hash = fixture.run_incremental_recompute_step();
    assert_ne!(full_hash, 0);
    assert_ne!(incremental_hash, 0);
}

#[test]
fn resolved_output_recompute_incremental_fixture_reports_expected_metrics() {
    let mut full_fixture = ResolvedOutputRecomputeIncrementalFixture::new(240, 24);
    let (_, full_metrics) = full_fixture.run_full_recompute_with_metrics();
    assert_eq!(full_metrics.requested_lines, 240);
    assert_eq!(full_metrics.conflict_blocks, 24);
    assert_eq!(full_metrics.unresolved_blocks, 19);
    assert_eq!(full_metrics.both_choice_blocks, 6);
    assert_eq!(full_metrics.outline_rows, 247);
    assert_eq!(full_metrics.marker_rows, 54);
    assert_eq!(full_metrics.manual_rows, 2);
    assert_eq!(full_metrics.dirty_rows, 0);
    assert_eq!(full_metrics.recomputed_rows, 247);
    assert!(!full_metrics.fallback_full_recompute);

    let mut incremental_fixture = ResolvedOutputRecomputeIncrementalFixture::new(240, 24);
    let (_, incremental_metrics) = incremental_fixture.run_incremental_recompute_with_metrics();
    assert_eq!(incremental_metrics.requested_lines, 240);
    assert_eq!(incremental_metrics.conflict_blocks, 24);
    assert_eq!(incremental_metrics.unresolved_blocks, 19);
    assert_eq!(incremental_metrics.both_choice_blocks, 6);
    assert_eq!(incremental_metrics.outline_rows, 247);
    assert_eq!(incremental_metrics.marker_rows, 54);
    assert_eq!(incremental_metrics.manual_rows, 2);
    assert_eq!(incremental_metrics.dirty_rows, 3);
    assert_eq!(incremental_metrics.recomputed_rows, 3);
    assert!(!incremental_metrics.fallback_full_recompute);
}

#[test]
fn branch_sidebar_fixture_scales_with_more_entries() {
    let small = BranchSidebarFixture::new(8, 16, 2, 0, 0, 0);
    let large = BranchSidebarFixture::new(120, 600, 6, 40, 40, 80);
    assert!(small.row_count() > 0);
    assert!(large.row_count() > small.row_count());
}

#[test]
fn branch_sidebar_extreme_fixture_reports_expected_structural_metrics() {
    let fixture = BranchSidebarFixture::twenty_thousand_branches_hundred_remotes();
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(
        metrics,
        BranchSidebarMetrics {
            local_branches: 1,
            remote_branches: 20_000,
            remotes: 100,
            worktrees: 0,
            submodules: 0,
            stashes: 0,
            sidebar_rows: 20_414,
            branch_rows: 20_002,
            remote_headers: 100,
            group_headers: 300,
            max_branch_depth: 4,
        }
    );
}

#[test]
fn open_repo_fixture_reports_structural_metrics() {
    let fixture = OpenRepoFixture::new(500, 20, 40, 2);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.commit_count, 500);
    assert_eq!(metrics.local_branches, 20);
    assert_eq!(metrics.remote_branches, 40);
    assert_eq!(metrics.remotes, 2);
    assert_eq!(metrics.worktrees, 0);
    assert_eq!(metrics.submodules, 0);
    assert_eq!(metrics.graph_rows, 500);
    assert!(metrics.sidebar_rows >= 60);
    assert!(metrics.max_graph_lanes >= 1);
}

#[test]
fn open_repo_fixture_reports_extreme_metadata_fanout_metrics() {
    let fixture = OpenRepoFixture::with_sidebar_fanout(1_000, 1_000, 10_000, 1, 5_000, 1_000);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.commit_count, 1_000);
    assert_eq!(metrics.local_branches, 1_000);
    assert_eq!(metrics.remote_branches, 10_000);
    assert_eq!(metrics.remotes, 1);
    assert_eq!(metrics.worktrees, 5_000);
    assert_eq!(metrics.submodules, 1_000);
    assert_eq!(metrics.graph_rows, 1_000);
    assert!(metrics.sidebar_rows >= 17_000);
}

#[test]
fn repo_switch_refocus_same_repo_stays_on_primary_refresh_path() {
    let fixture = RepoSwitchFixture::refocus_same_repo(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 5);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 0);
    assert_eq!(metrics.persist_session_effect_count, 0);
}

#[test]
fn repo_switch_two_hot_repos_reloads_selected_diff_and_persists_session() {
    let fixture = RepoSwitchFixture::two_hot_repos(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 7);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 2);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 2);
    assert_eq!(metrics.hydrated_repo_count, 2);
    assert_eq!(metrics.selected_commit_repo_count, 2);
    assert_eq!(metrics.selected_diff_repo_count, 2);
}

#[test]
fn repo_switch_selected_commit_and_details_skips_diff_reload_path() {
    let fixture = RepoSwitchFixture::selected_commit_and_details(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 6);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 0);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 2);
    assert_eq!(metrics.hydrated_repo_count, 2);
    assert_eq!(metrics.selected_commit_repo_count, 2);
    assert_eq!(metrics.selected_diff_repo_count, 0);
}

#[test]
fn repo_switch_twenty_tabs_scales_repo_count_without_heating_all_tabs() {
    let fixture = RepoSwitchFixture::twenty_tabs(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 7);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 2);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 20);
    assert_eq!(metrics.hydrated_repo_count, 2);
    assert_eq!(metrics.selected_commit_repo_count, 2);
    assert_eq!(metrics.selected_diff_repo_count, 2);
}

#[test]
fn repo_switch_fresh_state_restamps_hot_repos_only() {
    let fixture = RepoSwitchFixture::twenty_tabs(500, 20, 40, 2);
    let before = SystemTime::now();
    let state = fixture.fresh_state();
    let after = SystemTime::now();

    let hot_repos: Vec<_> = state
        .repos
        .iter()
        .filter_map(|repo| repo.last_active_at)
        .collect();
    assert_eq!(hot_repos.len(), 2);
    assert!(hot_repos.iter().all(|last_active_at| {
        last_active_at.duration_since(before).is_ok()
            && after.duration_since(*last_active_at).is_ok()
    }));
}

#[test]
fn repo_switch_twenty_repos_all_hot_tracks_extreme_hot_tab_scale() {
    let fixture = RepoSwitchFixture::twenty_repos_all_hot(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 7);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 2);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 20);
    assert_eq!(metrics.hydrated_repo_count, 20);
    assert_eq!(metrics.selected_commit_repo_count, 20);
    assert_eq!(metrics.selected_diff_repo_count, 20);
}

#[test]
fn repo_switch_selected_diff_file_triggers_diff_reload_with_loaded_content() {
    let fixture = RepoSwitchFixture::selected_diff_file(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 7);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 2);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 2);
    assert_eq!(metrics.hydrated_repo_count, 2);
    assert_eq!(metrics.selected_commit_repo_count, 2);
    assert_eq!(metrics.selected_diff_repo_count, 2);
}

#[test]
fn repo_switch_selected_conflict_target_dispatches_conflict_reload() {
    let fixture = RepoSwitchFixture::selected_conflict_target(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 7);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 1);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 2);
    assert_eq!(metrics.hydrated_repo_count, 2);
    assert_eq!(metrics.selected_diff_repo_count, 2);
}

#[test]
fn repo_switch_merge_active_with_draft_restore_includes_merge_message() {
    let fixture = RepoSwitchFixture::merge_active_with_draft_restore(500, 20, 40, 2);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 7);
    assert_eq!(metrics.refresh_effect_count, 6);
    assert_eq!(metrics.selected_diff_reload_effect_count, 2);
    assert_eq!(metrics.persist_session_effect_count, 1);
    assert_eq!(metrics.repo_count, 2);
    assert_eq!(metrics.hydrated_repo_count, 2);
    assert_eq!(metrics.selected_diff_repo_count, 2);
}

#[test]
fn status_select_diff_open_unstaged_produces_expected_effects() {
    let fixture = StatusSelectDiffOpenFixture::unstaged(500);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    // SelectDiff now emits one lightweight selected-diff intent and lets
    // effect scheduling expand it to the same file+patch reload work.
    assert_eq!(metrics.effect_count, 1);
    assert_eq!(metrics.load_diff_effect_count, 1);
    assert_eq!(metrics.load_diff_file_effect_count, 1);
    assert_eq!(metrics.load_diff_file_image_effect_count, 0);
    assert_eq!(metrics.diff_state_rev_delta, 1);
}

#[test]
fn status_select_diff_open_staged_produces_expected_effects() {
    let fixture = StatusSelectDiffOpenFixture::staged(500);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.effect_count, 1);
    assert_eq!(metrics.load_diff_effect_count, 1);
    assert_eq!(metrics.load_diff_file_effect_count, 1);
    assert_eq!(metrics.load_diff_file_image_effect_count, 0);
    assert_eq!(metrics.diff_state_rev_delta, 1);
}

#[test]
fn history_graph_fixture_preserves_requested_commit_count() {
    let fixture = HistoryGraphFixture::new(2_000, 7, 9);
    assert_eq!(fixture.commit_count(), 2_000);
    assert_ne!(fixture.run(), 0);
}

#[test]
fn history_cache_build_balanced_produces_expected_metrics() {
    let fixture = HistoryCacheBuildFixture::balanced(500, 20, 40, 10, 5);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.visible_commits, 500);
    assert_eq!(metrics.graph_rows, 500);
    assert_eq!(metrics.commit_vms, 500);
    assert_eq!(metrics.stash_helpers_filtered, 0);
    assert!(metrics.max_lanes > 0);
}

#[test]
fn history_cache_build_stash_heavy_filters_helpers() {
    let fixture = HistoryCacheBuildFixture::stash_heavy(600, 50);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.stash_helpers_filtered, 50);
    // 500 base commits + 50 helpers + 50 tips = 600 total, minus 50 helpers = 550 visible
    assert_eq!(metrics.visible_commits, 550);
    assert_eq!(metrics.graph_rows, 550);
    assert_eq!(metrics.commit_vms, 550);
}

#[test]
fn history_cache_build_decorated_refs_heavy_tracks_decorations() {
    let fixture = HistoryCacheBuildFixture::decorated_refs_heavy(500, 100, 200, 100);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert!(
        metrics.decorated_commits > 100,
        "expected many decorated commits with 100+200 branches and 100 tags, got {}",
        metrics.decorated_commits
    );
}

#[test]
fn history_cache_build_merge_dense_has_dense_graph() {
    let fixture = HistoryCacheBuildFixture::merge_dense(500);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.visible_commits, 500);
    assert_eq!(metrics.graph_rows, 500);
    assert!(
        metrics.max_lanes > 1,
        "merge-dense graph should have multiple lanes"
    );
}

#[test]
fn history_cache_build_extreme_scale_reports_expected_metrics() {
    let fixture = HistoryCacheBuildFixture::extreme_scale_50k_2k_refs_200_stashes();
    let (hash, metrics) = fixture.run();

    assert_ne!(hash, 0);
    assert_eq!(metrics.visible_commits, 49_800);
    assert_eq!(metrics.graph_rows, 49_800);
    assert_eq!(metrics.commit_vms, 49_800);
    assert_eq!(metrics.stash_helpers_filtered, 200);
    assert_eq!(metrics.decorated_commits, 1_879);
    assert!(metrics.max_lanes > 0);
}

#[test]
fn history_load_more_append_fixture_appends_requested_page() {
    let fixture = HistoryLoadMoreAppendFixture::new(1_000, 500);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(
        metrics,
        HistoryLoadMoreAppendMetrics {
            existing_commits: 1_000,
            appended_commits: 500,
            total_commits_after_append: 1_500,
            next_cursor_present: 1,
            follow_up_effect_count: 0,
            log_rev_delta: 2,
            log_loading_more_cleared: 1,
        }
    );
}

#[test]
fn history_scope_switch_fixture_switches_current_branch_to_all_refs() {
    let fixture = HistoryScopeSwitchFixture::current_branch_to_all_refs(1_000);
    let (hash, metrics) = fixture.run();
    assert_ne!(hash, 0);
    assert_eq!(metrics.existing_commits, 1_000);
    assert_eq!(metrics.scope_changed, 1, "scope should have changed");
    assert!(
        metrics.log_rev_delta >= 1,
        "log_rev should bump at least once"
    );
    assert_eq!(
        metrics.log_set_to_loading, 1,
        "log should transition to Loading"
    );
    assert_eq!(
        metrics.load_log_effect_count, 1,
        "should emit exactly 1 LoadLog effect"
    );
}

#[test]
fn synthetic_source_lines_honor_requested_min_line_bytes() {
    let lines = build_synthetic_source_lines(64, 512);
    assert_eq!(lines.len(), 64);
    assert!(lines.iter().all(|line| line.len() >= 512));
}

#[test]
fn large_file_fixture_handles_very_long_lines() {
    let fixture = LargeFileDiffScrollFixture::new_with_line_bytes(512, 4_096);
    assert_ne!(fixture.run_scroll_step(0, 64), 0);
}

#[test]
fn large_file_diff_scroll_fixture_reports_expected_sidecar_metrics() {
    let fixture = LargeFileDiffScrollFixture::new_with_line_bytes(512, 256);
    let (hash, metrics) = fixture.run_scroll_step_with_metrics(17 + 512 * 2, 48);

    assert_ne!(hash, 0);
    assert_eq!(hash, fixture.run_scroll_step(17, 48));
    assert_eq!(metrics.total_lines, 512);
    assert_eq!(metrics.window_size, 48);
    assert_eq!(metrics.start_line, 17);
    assert!(metrics.visible_text_bytes >= 48 * 256);
    assert!(metrics.min_line_bytes >= 256);
    assert_eq!(metrics.language_detected, 1);
    assert_eq!(metrics.syntax_mode_auto, 1);
}

#[test]
fn text_input_prepaint_windowed_fixture_wraps_start_offsets() {
    let mut fixture = TextInputPrepaintWindowedFixture::new(512, 96, 640);
    let hash_a = fixture.run_windowed_step(17, 48);
    let hash_b = fixture.run_windowed_step(17 + fixture.total_rows() * 3, 48);
    assert_eq!(hash_a, hash_b);
    assert!(fixture.cache_entries() > 0);
}

#[test]
fn text_input_prepaint_windowed_fixture_reports_expected_sidecar_metrics() {
    // Cold windowed run: 512 lines, 48 viewport, guard_rows=2 → total_rows=52, all misses.
    let mut fixture = TextInputPrepaintWindowedFixture::new(512, 96, 640);
    let (hash, metrics) = fixture.run_windowed_step_with_metrics(0, 48);

    assert_ne!(hash, 0);
    assert_eq!(hash, {
        let mut f2 = TextInputPrepaintWindowedFixture::new(512, 96, 640);
        f2.run_windowed_step(0, 48)
    });
    assert_eq!(metrics.total_lines, 512);
    assert_eq!(metrics.viewport_rows, 48);
    assert_eq!(metrics.guard_rows, 2);
    assert_eq!(metrics.max_shape_bytes, 4096);
    // Cold run: 48 + 2*2 = 52 rows, all unique → 52 misses, 0 hits.
    assert_eq!(metrics.cache_entries_after, 52);
    assert_eq!(metrics.cache_misses, 52);
    assert_eq!(metrics.cache_hits, 0);

    // Full-document cold run: 512 lines + 4 guard rows wrapping.
    let mut full_fixture = TextInputPrepaintWindowedFixture::new(512, 96, 640);
    let (full_hash, full_metrics) = full_fixture.run_full_document_step_with_metrics();

    assert_ne!(full_hash, 0);
    assert_eq!(full_metrics.total_lines, 512);
    assert_eq!(full_metrics.viewport_rows, 512);
    assert_eq!(full_metrics.cache_entries_after, 512);
    // Guard rows wrap to lines 0-3 → 4 hits, 512 misses.
    assert_eq!(full_metrics.cache_misses, 512);
    assert_eq!(full_metrics.cache_hits, 4);
}

#[test]
fn text_input_runs_streamed_highlight_fixture_matches_legacy_dense() {
    let fixture =
        TextInputRunsStreamedHighlightFixture::new(512, 112, 96, TextInputHighlightDensity::Dense);
    assert!(fixture.highlights_len() > 0);

    let mut start = 0usize;
    for _ in 0..8 {
        let legacy = fixture.run_legacy_step(start);
        let streamed = fixture.run_streamed_step(start);
        assert_eq!(legacy, streamed);
        start = fixture.next_start_row(start);
    }
}

#[test]
fn text_input_runs_streamed_highlight_fixture_matches_legacy_sparse() {
    let fixture =
        TextInputRunsStreamedHighlightFixture::new(512, 112, 96, TextInputHighlightDensity::Sparse);
    assert!(fixture.highlights_len() > 0);

    let mut start = 0usize;
    for _ in 0..8 {
        let legacy = fixture.run_legacy_step(start);
        let streamed = fixture.run_streamed_step(start);
        assert_eq!(legacy, streamed);
        start = fixture.next_start_row(start);
    }
}

#[test]
fn text_input_runs_streamed_highlight_dense_fixture_reports_expected_sidecar_metrics() {
    let fixture =
        TextInputRunsStreamedHighlightFixture::new(512, 112, 96, TextInputHighlightDensity::Dense);
    let hash_without_metrics = fixture.run_legacy_step(0);
    let (legacy_hash, legacy_metrics) = fixture.run_legacy_step_with_metrics(0);
    let (streamed_hash, streamed_metrics) = fixture.run_streamed_step_with_metrics(0);

    assert_eq!(hash_without_metrics, legacy_hash);
    assert_eq!(legacy_hash, streamed_hash);
    assert_eq!(legacy_metrics.total_lines, 512);
    assert_eq!(legacy_metrics.visible_rows, 96);
    assert_eq!(legacy_metrics.scroll_step, 48);
    assert_eq!(legacy_metrics.visible_lines_with_highlights, 96);
    assert!(legacy_metrics.visible_highlights > legacy_metrics.visible_rows);
    assert!(legacy_metrics.total_highlights > legacy_metrics.visible_highlights);
    assert_eq!(legacy_metrics.density_dense, 1);
    assert_eq!(legacy_metrics.algorithm_streamed, 0);
    assert_eq!(
        streamed_metrics,
        TextInputRunsStreamedHighlightMetrics {
            algorithm_streamed: 1,
            ..legacy_metrics
        }
    );
}

#[test]
fn text_input_runs_streamed_highlight_sparse_fixture_reports_expected_sidecar_metrics() {
    let fixture =
        TextInputRunsStreamedHighlightFixture::new(512, 112, 96, TextInputHighlightDensity::Sparse);
    let hash_without_metrics = fixture.run_legacy_step(0);
    let (legacy_hash, legacy_metrics) = fixture.run_legacy_step_with_metrics(0);
    let (streamed_hash, streamed_metrics) = fixture.run_streamed_step_with_metrics(0);

    assert_eq!(hash_without_metrics, legacy_hash);
    assert_eq!(legacy_hash, streamed_hash);
    assert_eq!(legacy_metrics.total_lines, 512);
    assert_eq!(legacy_metrics.visible_rows, 96);
    assert_eq!(legacy_metrics.scroll_step, 48);
    assert_eq!(legacy_metrics.total_highlights, 86);
    assert_eq!(legacy_metrics.visible_highlights, 16);
    assert_eq!(legacy_metrics.visible_lines_with_highlights, 12);
    assert_eq!(legacy_metrics.density_dense, 0);
    assert_eq!(legacy_metrics.algorithm_streamed, 0);
    assert_eq!(
        streamed_metrics,
        TextInputRunsStreamedHighlightMetrics {
            algorithm_streamed: 1,
            ..legacy_metrics
        }
    );
}

#[test]
fn text_input_long_line_cap_fixture_bounds_shaping_slice() {
    let fixture = TextInputLongLineCapFixture::new(128 * 1024);
    let capped_len = fixture.capped_len(4 * 1024);
    let uncapped_len = fixture.capped_len(256 * 1024);
    assert!(capped_len < uncapped_len);
    assert_ne!(fixture.run_with_cap(4 * 1024), 0);
    assert_ne!(fixture.run_without_cap(), 0);
}

#[test]
fn text_input_long_line_cap_fixture_reports_expected_sidecar_metrics() {
    let fixture = TextInputLongLineCapFixture::new(256 * 1024);

    let (hash_capped, capped_metrics) = fixture.run_with_cap_with_metrics(4096);
    assert_ne!(hash_capped, 0);
    assert_eq!(capped_metrics.line_bytes, 262_144);
    assert_eq!(capped_metrics.max_shape_bytes, 4096);
    assert!(capped_metrics.capped_len <= 4096);
    assert_eq!(capped_metrics.iterations, 64);
    assert_eq!(capped_metrics.cap_active, 1);

    let (hash_uncapped, uncapped_metrics) = fixture.run_without_cap_with_metrics();
    assert_ne!(hash_uncapped, 0);
    assert_eq!(uncapped_metrics.line_bytes, 262_144);
    assert!(uncapped_metrics.max_shape_bytes >= 262_144);
    assert_eq!(uncapped_metrics.capped_len, 262_144);
    assert_eq!(uncapped_metrics.iterations, 64);
    assert_eq!(uncapped_metrics.cap_active, 0);
}

#[test]
fn text_input_wrap_incremental_tabs_fixture_matches_full_recompute() {
    let mut full = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
    let mut incremental = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
    for step in 0..48usize {
        let line_ix = step.wrapping_mul(17);
        let full_hash = full.run_full_recompute_step(line_ix);
        let incremental_hash = incremental.run_incremental_step(line_ix);
        assert_eq!(full_hash, incremental_hash);
    }
    assert_eq!(full.row_counts(), incremental.row_counts());
}

#[test]
fn text_input_wrap_incremental_tabs_fixture_matches_full_recompute_after_revisiting_lines() {
    let mut full = TextInputWrapIncrementalTabsFixture::new(3, 96, 680);
    let mut incremental = TextInputWrapIncrementalTabsFixture::new(3, 96, 680);
    for step in 0..384usize {
        let line_ix = step % 3;
        let full_hash = full.run_full_recompute_step(line_ix);
        let incremental_hash = incremental.run_incremental_step(line_ix);
        assert_eq!(full_hash, incremental_hash);
    }
    assert_eq!(full.row_counts(), incremental.row_counts());
}

#[test]
fn text_input_wrap_incremental_tabs_fixture_reports_expected_sidecar_metrics() {
    let mut full_fixture = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
    let full_hash_without_metrics = full_fixture.run_full_recompute_step(0);
    let mut full_with_metrics = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
    let (full_hash, full_metrics) = full_with_metrics.run_full_recompute_step_with_metrics(0);

    assert_eq!(full_hash_without_metrics, full_hash);
    assert_eq!(
        full_metrics,
        TextInputWrapIncrementalTabsMetrics {
            total_lines: 512,
            line_bytes: 101,
            wrap_columns: 87,
            edit_line_ix: 0,
            dirty_lines: 2,
            total_rows_after: 1024,
            recomputed_lines: 512,
            incremental_patch: 0,
        }
    );

    let mut incremental_fixture = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
    let incremental_hash_without_metrics = incremental_fixture.run_incremental_step(0);
    let mut incremental_with_metrics = TextInputWrapIncrementalTabsFixture::new(512, 96, 680);
    let (incremental_hash, incremental_metrics) =
        incremental_with_metrics.run_incremental_step_with_metrics(0);

    assert_eq!(incremental_hash_without_metrics, incremental_hash);
    assert_eq!(
        incremental_metrics,
        TextInputWrapIncrementalTabsMetrics {
            recomputed_lines: 2,
            incremental_patch: 1,
            ..full_metrics
        }
    );
}

#[test]
fn text_input_wrap_incremental_burst_fixture_matches_full_recompute() {
    let mut full = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
    let mut incremental = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
    for burst in [1usize, 3, 6, 9, 12] {
        let full_hash = full.run_full_recompute_burst_step(burst);
        let incremental_hash = incremental.run_incremental_burst_step(burst);
        assert_eq!(full_hash, incremental_hash);
    }
    assert_eq!(full.row_counts(), incremental.row_counts());
}

#[test]
fn text_input_wrap_incremental_burst_edits_fixture_reports_expected_sidecar_metrics() {
    let mut full_fixture = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
    let full_hash_without_metrics = full_fixture.run_full_recompute_burst_step(12);
    let mut full_with_metrics = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
    let (full_hash, full_metrics) =
        full_with_metrics.run_full_recompute_burst_step_with_metrics(12);

    assert_eq!(full_hash_without_metrics, full_hash);
    assert_eq!(full_metrics.total_lines, 768);
    assert_eq!(full_metrics.edits_per_burst, 12);
    assert_eq!(full_metrics.wrap_columns, 92);
    assert_eq!(full_metrics.total_dirty_lines, 12);
    assert_eq!(full_metrics.recomputed_lines, 768 * 12);
    assert_eq!(full_metrics.incremental_patch, 0);

    let mut incremental_fixture = TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
    let incremental_hash_without_metrics = incremental_fixture.run_incremental_burst_step(12);
    let mut incremental_with_metrics =
        TextInputWrapIncrementalBurstEditsFixture::new(768, 112, 720);
    let (incremental_hash, incremental_metrics) =
        incremental_with_metrics.run_incremental_burst_step_with_metrics(12);

    assert_eq!(incremental_hash_without_metrics, incremental_hash);
    assert_eq!(incremental_metrics.total_lines, 768);
    assert_eq!(incremental_metrics.edits_per_burst, 12);
    assert_eq!(incremental_metrics.wrap_columns, 92);
    assert_eq!(incremental_metrics.total_dirty_lines, 12);
    assert_eq!(incremental_metrics.recomputed_lines, 12);
    assert_eq!(incremental_metrics.incremental_patch, 1);

    // Both variants should agree on total_rows_after and total_dirty_lines
    assert_eq!(
        full_metrics.total_rows_after,
        incremental_metrics.total_rows_after
    );
    assert_eq!(
        full_metrics.total_dirty_lines,
        incremental_metrics.total_dirty_lines
    );

    // Incremental recomputes only dirty lines; full recomputes all lines per edit
    assert_eq!(
        incremental_metrics.recomputed_lines,
        incremental_metrics.total_dirty_lines
    );
    assert!(full_metrics.recomputed_lines > incremental_metrics.recomputed_lines);
}

#[test]
fn text_model_snapshot_clone_fixture_runs_model_and_string_control_paths() {
    let fixture = TextModelSnapshotCloneCostFixture::new(512 * 1024);
    let model_hash = fixture.run_snapshot_clone_step(2_048);
    let string_hash = fixture.run_string_clone_control_step(2_048);
    assert_ne!(model_hash, 0);
    assert_ne!(string_hash, 0);
}

#[test]
fn text_model_snapshot_clone_fixture_reports_expected_sidecar_metrics() {
    let fixture = TextModelSnapshotCloneCostFixture::new(128 * 1024);
    let clones = 256;
    let (_, snapshot_metrics) = fixture.run_snapshot_clone_step_with_metrics(clones);
    let (_, control_metrics) = fixture.run_string_clone_control_step_with_metrics(clones);

    assert_eq!(
        snapshot_metrics.document_bytes as usize,
        fixture.model.len()
    );
    assert_eq!(
        snapshot_metrics.line_starts as usize,
        fixture.model.line_starts().len()
    );
    assert_eq!(snapshot_metrics.clone_count, clones as u64);
    assert_eq!(snapshot_metrics.sampled_prefix_bytes, 96);
    assert_eq!(snapshot_metrics.snapshot_path, 1);

    assert_eq!(
        control_metrics.document_bytes as usize,
        fixture.string_control.len()
    );
    assert_eq!(
        control_metrics.line_starts as usize,
        fixture.model.line_starts().len()
    );
    assert_eq!(control_metrics.clone_count, clones as u64);
    assert_eq!(control_metrics.sampled_prefix_bytes, 96);
    assert_eq!(control_metrics.snapshot_path, 0);
}

#[test]
fn text_model_bulk_load_fixture_runs_piece_table_and_control_paths() {
    let fixture = TextModelBulkLoadLargeFixture::new(4_096, 96);
    let piece_table_hash = fixture.run_piece_table_bulk_load_step();
    let piece_table_from_large_hash = fixture.run_piece_table_from_large_text_step();
    let control_hash = fixture.run_string_bulk_load_control_step();
    assert_ne!(piece_table_hash, 0);
    assert_ne!(piece_table_from_large_hash, 0);
    assert_ne!(control_hash, 0);
}

#[test]
fn text_model_bulk_load_large_fixture_reports_expected_sidecar_metrics() {
    let fixture = TextModelBulkLoadLargeFixture::new(4_096, 96);

    let (_, append_metrics) = fixture.run_piece_table_bulk_load_step_with_metrics();
    assert_eq!(append_metrics.source_bytes as usize, fixture.text.len());
    assert_eq!(
        append_metrics.document_bytes_after as usize,
        fixture.text.len()
    );
    assert!(append_metrics.line_starts_after >= 4_097); // 4096 lines + 1
    assert_eq!(append_metrics.chunk_count, 2);
    assert_eq!(append_metrics.load_variant, 0);

    let (_, from_large_metrics) = fixture.run_piece_table_from_large_text_step_with_metrics();
    assert_eq!(from_large_metrics.source_bytes as usize, fixture.text.len());
    assert_eq!(
        from_large_metrics.document_bytes_after as usize,
        fixture.text.len()
    );
    assert!(from_large_metrics.line_starts_after >= 4_097);
    assert_eq!(from_large_metrics.chunk_count, 1);
    assert_eq!(from_large_metrics.load_variant, 1);

    let (_, control_metrics) = fixture.run_string_bulk_load_control_step_with_metrics();
    assert_eq!(control_metrics.source_bytes as usize, fixture.text.len());
    assert_eq!(
        control_metrics.document_bytes_after as usize,
        fixture.text.len()
    );
    assert_eq!(control_metrics.line_starts_after, 0);
    assert_eq!(
        control_metrics.chunk_count as usize,
        fixture.control_chunk_ranges.len()
    );
    assert!(fixture.control_chunk_ranges.len() > 1);
    assert_eq!(control_metrics.load_variant, 2);

    // Both piece-table variants should agree on source_bytes and line_starts
    assert_eq!(append_metrics.source_bytes, from_large_metrics.source_bytes);
    assert_eq!(
        append_metrics.line_starts_after,
        from_large_metrics.line_starts_after
    );
}

#[test]
fn text_model_fragmented_edit_fixture_reports_expected_sidecar_metrics() {
    let fixture = TextModelFragmentedEditFixture::new(512 * 1024, 500);

    let (edit_hash, edit_metrics) = fixture.run_fragmented_edit_step_with_metrics();
    assert_ne!(edit_hash, 0);
    assert_eq!(
        edit_metrics,
        TextModelFragmentedEditsMetrics {
            initial_bytes: 524_295,
            edit_count: 500,
            deleted_bytes: 3_681,
            inserted_bytes: 3_990,
            final_bytes: 524_604,
            line_starts_after: 9_806,
            readback_operations: 0,
            string_control: 0,
        }
    );

    let (materialize_hash, materialize_metrics) =
        fixture.run_materialize_after_edits_step_with_metrics();
    assert_ne!(materialize_hash, 0);
    assert_eq!(
        materialize_metrics,
        TextModelFragmentedEditsMetrics {
            readback_operations: 1,
            ..edit_metrics
        }
    );

    let (shared_hash, shared_metrics) = fixture.run_shared_string_after_edits_step_with_metrics(64);
    assert_ne!(shared_hash, 0);
    assert_eq!(
        shared_metrics,
        TextModelFragmentedEditsMetrics {
            readback_operations: 64,
            ..edit_metrics
        }
    );

    let (control_hash, control_metrics) = fixture.run_string_edit_control_step_with_metrics();
    assert_ne!(control_hash, 0);
    assert_eq!(
        control_metrics,
        TextModelFragmentedEditsMetrics {
            string_control: 1,
            ..edit_metrics
        }
    );
}

#[test]
fn nested_query_stress_source_lines_honor_requested_min_line_bytes() {
    let fixture = FileDiffSyntaxPrepareFixture::new_query_stress(32, 2_048, 64);
    let lines = fixture.lines();
    assert_eq!(lines.len(), 32);
    assert!(lines.iter().all(|line| line.len() >= 2_048));
    assert!(lines.iter().all(|line| line.contains("nested")));
}

#[test]
fn file_diff_syntax_stress_fixture_has_bounded_latency_distribution() {
    let fixture = FileDiffSyntaxPrepareFixture::new_query_stress(64, 1_536, 96);
    let mut samples = Vec::new();
    for nonce in 0..10u64 {
        let start = std::time::Instant::now();
        let _ = fixture.run_prepare_cold(nonce);
        samples.push(start.elapsed().as_secs_f64());
    }
    samples.sort_by(|a, b| a.total_cmp(b));
    let median = samples[samples.len() / 2].max(f64::EPSILON);
    let p95 = samples[samples.len() - 1];
    assert!(
        p95 <= median * 12.0,
        "query stress latency distribution widened too far: median={median:.6}s p95={p95:.6}s"
    );
}

#[test]
fn file_diff_syntax_reparse_fixture_runs_small_and_large_edit_steps() {
    let mut fixture = FileDiffSyntaxReparseFixture::new(512, 128);
    let small = fixture.run_small_edit_step();
    let large = fixture.run_large_edit_step();
    assert_ne!(small, 0);
    assert_ne!(large, 0);
}

#[test]
fn file_diff_syntax_cache_drop_fixture_runs_both_modes() {
    let fixture = FileDiffSyntaxCacheDropFixture::new(1_024, 8, 4);
    assert_ne!(fixture.run_deferred_drop_step(), 0);
    assert_ne!(fixture.run_inline_drop_control_step(), 0);
}

#[test]
fn file_diff_inline_syntax_projection_fixture_runs_pending_and_ready_windows() {
    let fixture = FileDiffInlineSyntaxProjectionFixture::new(384, 96);
    assert!(fixture.visible_rows() > 0);
    assert_ne!(fixture.run_window_pending_step(0, 64), 0);
    assert_ne!(fixture.run_window_step(0, 64), 0);
}

#[test]
fn file_diff_inline_syntax_projection_fixture_wraps_start_offsets() {
    let fixture = FileDiffInlineSyntaxProjectionFixture::new(512, 128);
    let hash_a = fixture.run_window_step(17, 48);
    let hash_b = fixture.run_window_step(17 + fixture.visible_rows() * 3, 48);
    assert_eq!(hash_a, hash_b);
}

#[test]
fn prepared_syntax_multidoc_cache_hit_rate_fixture_runs() {
    let fixture = FileDiffSyntaxPrepareFixture::new(512, 96);
    let hash = fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(4, 1);
    assert_ne!(hash, 0);
}

#[test]
fn prepared_syntax_chunk_miss_cost_fixture_runs() {
    let fixture = FileDiffSyntaxPrepareFixture::new(1_024, 96);
    let elapsed = fixture.run_prepared_syntax_chunk_miss_cost_step(1);
    assert!(elapsed >= Duration::ZERO);
}

#[test]
fn file_diff_syntax_prepare_fixture_keeps_prepared_document_for_large_documents() {
    let fixture = FileDiffSyntaxPrepareFixture::new(4_001, 96);
    let prepared = fixture.prepare_document(fixture.lines());

    assert!(
        prepared.is_some(),
        "file diff syntax prepare should stay enabled above the old 4,000-line gate"
    );
    assert_ne!(fixture.run_prepare_warm(), 0);
}

#[test]
fn large_html_syntax_fixture_synthetic_fallback_runs() {
    let prepare_fixture = LargeHtmlSyntaxFixture::new(None, 128, 160);
    let visible_fixture = LargeHtmlSyntaxFixture::new_prewarmed(None, 128, 160);

    assert_eq!(prepare_fixture.source_label(), "synthetic_html_fixture");
    assert_eq!(visible_fixture.source_label(), "synthetic_html_fixture");
    assert_eq!(visible_fixture.line_count(), 128);
    let (prepare_hash, prepare_metrics) = prepare_fixture.run_background_prepare_with_metrics();
    assert_ne!(prepare_hash, 0);
    assert_eq!(prepare_metrics.line_count, 128);
    assert_eq!(prepare_metrics.prepared_document_available, 1);

    visible_fixture.prime_visible_window_until_ready(48);
    let (visible_hash, visible_metrics) = visible_fixture.run_visible_window_with_metrics(0, 48);
    assert_ne!(visible_hash, 0);
    assert_eq!(visible_metrics.line_count, 128);
    assert_eq!(visible_metrics.window_lines, 48);
    assert_eq!(visible_metrics.start_line, 0);
    assert_eq!(visible_metrics.prepared_document_available, 1);
    assert_eq!(visible_metrics.cache_document_present, 1);
    assert_eq!(visible_metrics.pending, 0);
    assert!(visible_metrics.highlight_spans > 0);
}

#[test]
fn large_html_syntax_fixture_pending_window_is_nonblocking_until_primed() {
    let path = std::env::temp_dir().join(format!(
        "gitcomet-large-html-pending-bench-{}.html",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    let text = (0..96)
        .map(|ix| {
            format!(
                "<div class=\"row-{ix}\" style=\"color: red\" onclick=\"const value = {ix};\">row {ix}</div>"
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, text).expect("temp html fixture should be writable");

    let fixture = LargeHtmlSyntaxFixture::new_prewarmed(path.to_str(), 64, 128);
    let document = fixture
        .prepared_document_handle()
        .expect("HTML fixture should prepare a document");

    let first = fixture
        .request_visible_window_for_lines(document, 0, 48)
        .expect("cold visible-window request should return fallback highlights");
    assert!(
        first.pending,
        "cold request should stay nonblocking until chunk work completes"
    );

    let pending_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(path.to_str(), 64, 128);
    let (pending_hash, pending_metrics) =
        pending_metrics_fixture.run_visible_window_pending_with_metrics(0, 48);
    assert_ne!(pending_hash, 0);
    assert_eq!(pending_metrics.line_count, 96);
    assert_eq!(pending_metrics.window_lines, 48);
    assert_eq!(pending_metrics.start_line, 0);
    assert_eq!(pending_metrics.prepared_document_available, 1);
    assert_eq!(pending_metrics.cache_document_present, 1);
    assert_eq!(pending_metrics.pending, 1);
    assert!(pending_metrics.highlight_spans > 0);

    let started = std::time::Instant::now();
    let mut second = fixture
        .request_visible_window_for_lines(document, 0, 48)
        .expect("second visible-window request should still succeed");
    while second.pending && started.elapsed() < Duration::from_secs(2) {
        if drain_completed_prepared_diff_syntax_chunk_builds_for_document(document) == 0 {
            std::thread::sleep(Duration::from_millis(5));
        }
        second = fixture
            .request_visible_window_for_lines(document, 0, 48)
            .expect("ready visible-window request should still succeed");
    }
    assert!(
        !second.pending,
        "drained request should return ready prepared-document highlights"
    );

    let _ = std::fs::remove_file(path);
}

#[test]
fn large_html_syntax_fixture_keeps_prepared_document_for_large_documents() {
    let fixture = LargeHtmlSyntaxFixture::new_prewarmed(None, 4_001, 192);

    assert_eq!(fixture.source_label(), "synthetic_html_fixture");
    assert!(
        fixture.prepared_document_handle().is_some(),
        "large HTML fixture should still produce a prepared document above the old 4,000-line gate"
    );

    fixture.prime_visible_window_until_ready(96);
    assert_ne!(fixture.run_visible_window_step(128, 96), 0);
}

#[test]
fn large_html_syntax_fixture_uses_external_text_when_available() {
    let path = std::env::temp_dir().join(format!(
        "gitcomet-large-html-bench-{}.html",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock should be after unix epoch")
            .as_nanos()
    ));
    let text = [
        "<!doctype html>",
        "<html>",
        "<body>",
        "<div style=\"color: red\" onclick=\"const value = 1;\">hi</div>",
        "</body>",
        "</html>",
    ]
    .join("\n");
    std::fs::write(&path, text).expect("temp html fixture should be writable");

    let fixture = LargeHtmlSyntaxFixture::new_prewarmed(path.to_str(), 64, 128);
    assert_eq!(fixture.source_label(), "external_html_fixture");
    assert_eq!(fixture.line_count(), 6);
    fixture.prime_visible_window_until_ready(6);
    assert_ne!(fixture.run_visible_window_step(0, 6), 0);

    let _ = std::fs::remove_file(path);
}

#[test]
fn worktree_preview_render_fixture_preserves_output_with_cached_lookup() {
    let fixture = WorktreePreviewRenderFixture::new(1_024, 128);
    let cached = fixture.run_cached_lookup_step(96, 160);
    let render_time_prepare = fixture.run_render_time_prepare_step(96, 160);
    assert_eq!(cached, render_time_prepare);
}

#[test]
fn worktree_preview_render_fixture_keeps_auto_mode_for_large_documents() {
    let fixture = WorktreePreviewRenderFixture::new(8_192, 128);
    assert_eq!(fixture.syntax_mode(), DiffSyntaxMode::Auto);
    assert!(fixture.has_prepared_document());
}

#[test]
fn worktree_preview_render_fixture_handles_long_windows() {
    let fixture = WorktreePreviewRenderFixture::new(2_048, 192);
    assert_ne!(fixture.run_cached_lookup_step(0, 256), 0);
    assert_ne!(fixture.run_render_time_prepare_step(0, 256), 0);
}

#[test]
fn worktree_preview_render_fixture_reports_expected_sidecar_metrics() {
    let fixture = WorktreePreviewRenderFixture::new(4_000, 128);
    let (cached_hash, cached_metrics) = fixture.run_cached_lookup_with_metrics(0, 200);
    assert_ne!(cached_hash, 0);
    assert_eq!(cached_metrics.total_lines, 4_000);
    assert_eq!(cached_metrics.window_size, 200);
    assert!(cached_metrics.line_bytes >= 120);
    assert_eq!(cached_metrics.prepared_document_available, 1);
    assert_eq!(cached_metrics.syntax_mode_auto, 1);

    let (prepare_hash, prepare_metrics) = fixture.run_render_time_prepare_with_metrics(0, 200);
    assert_ne!(prepare_hash, 0);
    assert_eq!(prepare_metrics.total_lines, 4_000);
    assert_eq!(prepare_metrics.window_size, 200);
    assert_eq!(prepare_metrics.prepared_document_available, 1);
    assert_eq!(prepare_metrics.syntax_mode_auto, 1);

    // Both paths should produce the same hash for the same window.
    assert_eq!(cached_hash, prepare_hash);
}

#[test]
fn markdown_preview_fixture_runs_parse_steps() {
    let fixture = MarkdownPreviewFixture::new(64, 96);
    assert_ne!(fixture.run_parse_single_step(), 0);
    assert_ne!(fixture.run_parse_diff_step(), 0);
}

#[test]
fn markdown_preview_fixture_runs_render_steps() {
    let fixture = MarkdownPreviewFixture::new(96, 112);
    assert_ne!(fixture.run_render_single_step(24, 64), 0);
    assert_ne!(fixture.run_render_diff_step(24, 64), 0);
}

#[test]
fn markdown_preview_fixture_reports_scroll_metrics() {
    let fixture = MarkdownPreviewScrollFixture::new_sectioned(128, 112);
    let (hash, metrics) = fixture.run_scroll_step_with_metrics(24, 200, 24);
    assert_ne!(hash, 0);
    assert!(metrics.total_rows > metrics.window_size);
    assert_eq!(metrics.start_row, 24);
    assert_eq!(metrics.window_size, 200);
    assert_eq!(metrics.rows_rendered, 200);
    assert_eq!(metrics.scroll_step_rows, 24);
    assert_eq!(metrics.long_rows, 0);
}

#[test]
fn markdown_preview_scroll_fixture_reports_rich_5000_row_metrics() {
    let fixture = MarkdownPreviewScrollFixture::new_rich_5000_rows();
    let (hash, metrics) = fixture.run_scroll_step_with_metrics(24, 200, 24);
    assert_ne!(hash, 0);
    assert_eq!(metrics.total_rows, 5_000);
    assert_eq!(metrics.long_rows, 500);
    assert_eq!(metrics.long_row_bytes, 2_000);
    assert_eq!(metrics.start_row, 24);
    assert_eq!(metrics.window_size, 200);
    assert_eq!(metrics.rows_rendered, 200);
    assert_eq!(metrics.scroll_step_rows, 24);
    assert!(metrics.heading_rows > 0);
    assert!(metrics.list_rows > 0);
    assert!(metrics.table_rows > 0);
    assert!(metrics.code_rows > 0);
    assert!(metrics.blockquote_rows > 0);
    assert!(metrics.details_rows > 0);
}

#[test]
fn streamed_provider_fixture_builds_with_expected_row_counts() {
    let fixture = ConflictStreamedProviderFixture::new(1_000);
    // Total rows should cover all lines (ours + theirs via anchor mapping).
    assert!(fixture.total_rows() > 0);
    assert!(fixture.visible_rows() > 0);
    assert_eq!(fixture.total_rows(), fixture.visible_rows());
}

#[test]
fn streamed_provider_fixture_generates_rows_at_all_positions() {
    let fixture = ConflictStreamedProviderFixture::new(200);
    assert_ne!(fixture.run_first_page_step(40), 0);
    assert_ne!(fixture.run_deep_scroll_step(0.5, 40), 0);
    assert_ne!(fixture.run_deep_scroll_step(0.9, 40), 0);
}

#[test]
fn streamed_provider_fixture_search_finds_known_patterns() {
    let fixture = ConflictStreamedProviderFixture::new(500);
    // "shared_" lines exist in both ours and theirs.
    let h = fixture.run_search_step("shared_");
    assert_ne!(h, 0, "search should find shared lines");
    // "ours_only_" lines exist only in ours.
    let h = fixture.run_search_step("ours_only_");
    assert_ne!(h, 0, "search should find ours-only lines");
}

#[test]
fn streamed_provider_fixture_index_build_is_deterministic() {
    let fixture = ConflictStreamedProviderFixture::new(500);
    let h1 = fixture.run_index_build_step();
    let h2 = fixture.run_index_build_step();
    assert_eq!(h1, h2, "index build should be deterministic");
}

#[test]
fn streamed_provider_fixture_projection_build_is_deterministic() {
    let fixture = ConflictStreamedProviderFixture::new(500);
    let h1 = fixture.run_projection_build_step();
    let h2 = fixture.run_projection_build_step();
    assert_eq!(h1, h2, "projection build should be deterministic");
}

#[test]
fn streamed_provider_fixture_reuses_first_page_cache() {
    let fixture = ConflictStreamedProviderFixture::new(1_000);
    fixture.prime_first_page_cache(160);
    assert_ne!(fixture.run_first_page_cache_hit_step(160), 0);
    assert_eq!(
        fixture.cached_page_count(),
        1,
        "cache-hit benchmark should keep the warmed first page resident"
    );
}

/// Phase 8 RSS invariant: streamed provider metadata is much smaller than
/// what eager mode would allocate.  Verifies that memory scales with
/// metadata (anchors, spans, line starts) not with rendered rows.
#[test]
fn streamed_provider_metadata_is_sublinear_in_line_count() {
    let small_lines = 2_000;
    let large_lines = 20_000;
    let small = ConflictStreamedProviderFixture::new(small_lines);
    let large = ConflictStreamedProviderFixture::new(large_lines);

    let small_meta = small.metadata_byte_size();
    let large_meta = large.metadata_byte_size();

    // At 10x the line count, metadata should grow less than 10x.
    // The split row index stores per-line-start data (O(N)) but NOT
    // per-rendered-row data; the projection stores O(segments) spans.
    let growth_ratio = large_meta as f64 / small_meta.max(1) as f64;
    assert!(
        growth_ratio < 12.0,
        "metadata should grow sublinearly vs eager rows: \
         small({small_lines})={small_meta}B, large({large_lines})={large_meta}B, \
         ratio={growth_ratio:.1}x (expected <12x for 10x line count)"
    );

    // The large fixture's metadata should be much smaller than the equivalent
    // eager allocation: N rows * ~100 bytes/FileDiffRow.
    let eager_estimate = large.total_rows() * 100;
    assert!(
        large_meta < eager_estimate / 2,
        "streamed metadata ({large_meta}B) should be well under eager estimate ({eager_estimate}B)"
    );
}

/// Phase 8 RSS invariant: page cache stays bounded regardless of how many
/// distinct positions are accessed.
#[test]
fn streamed_provider_page_cache_stays_bounded() {
    let fixture = ConflictStreamedProviderFixture::new(10_000);
    let visible_len = fixture.visible_rows();

    // Access pages at many distinct positions.
    for pct in [0.0, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95, 0.99] {
        let _ = fixture.run_deep_scroll_step(pct, 200);
    }

    let cached = fixture.cached_page_count();
    // CONFLICT_SPLIT_PAGE_CACHE_MAX_PAGES = 8.
    assert!(
        cached <= 8,
        "page cache should be bounded at 8 pages, got {cached} after scrolling \
         through {visible_len} visible rows"
    );
}

#[test]
fn streamed_resolved_output_fixture_builds_visible_rows() {
    let fixture = ConflictStreamedResolvedOutputFixture::new(1_000, 200);
    assert!(fixture.visible_rows() > 0);
}

#[test]
fn streamed_resolved_output_fixture_generates_windows_at_all_positions() {
    let fixture = ConflictStreamedResolvedOutputFixture::new(2_000, 300);
    assert_ne!(fixture.run_window_step(160), 0);
    assert_ne!(fixture.run_deep_window_step(0.5, 160), 0);
    assert_ne!(fixture.run_deep_window_step(0.9, 160), 0);
}

#[test]
fn streamed_resolved_output_fixture_projection_build_is_deterministic() {
    let fixture = ConflictStreamedResolvedOutputFixture::new(2_000, 300);
    let first = fixture.run_projection_build_step();
    let second = fixture.run_projection_build_step();
    assert_eq!(first, second);
}

#[test]
fn streamed_resolved_output_metadata_stays_compact() {
    let small_lines = 2_000;
    let large_lines = 20_000;
    let small = ConflictStreamedResolvedOutputFixture::new(small_lines, 200);
    let large = ConflictStreamedResolvedOutputFixture::new(large_lines, 2_000);

    let small_meta = small.metadata_byte_size();
    let large_meta = large.metadata_byte_size();
    let growth_ratio = large_meta as f64 / small_meta.max(1) as f64;
    assert!(
        growth_ratio < 12.0,
        "streamed resolved-output metadata should scale with spans/line starts, not rendered rows: \
         small({small_lines})={small_meta}B large({large_lines})={large_meta}B \
         ratio={growth_ratio:.1}x"
    );

    let materialized_len = large.materialized_output_len().max(1);
    assert!(
        large_meta < materialized_len,
        "streamed resolved-output metadata ({large_meta}B) should stay below \
         the materialized output size ({materialized_len}B)"
    );
}

#[test]
fn history_graph_fixture_reports_structural_metrics() {
    let linear = HistoryGraphFixture::new(500, 0, 0);
    let (hash, metrics) = linear.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.commit_count, 500);
    assert_eq!(metrics.graph_rows, 500);
    assert_eq!(metrics.merge_count, 0);
    assert_eq!(metrics.branch_heads, 0);
    assert_eq!(metrics.max_lanes, 1);

    let merge_dense = HistoryGraphFixture::new(500, 7, 0);
    let (hash2, merge_metrics) = merge_dense.run_with_metrics();
    assert_ne!(hash2, 0);
    assert_eq!(merge_metrics.graph_rows, 500);
    assert!(merge_metrics.merge_count > 0);
    assert!(merge_metrics.max_lanes > 1);

    let branch_heads = HistoryGraphFixture::new(500, 7, 5);
    let (hash3, bh_metrics) = branch_heads.run_with_metrics();
    assert_ne!(hash3, 0);
    assert!(bh_metrics.branch_heads > 50);
}

#[test]
fn commit_details_fixture_reports_structural_metrics() {
    let fixture = CommitDetailsFixture::new(1_000, 6);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.file_count, 1_000);
    // depth 6 dirs + 1 filename component = 7
    assert_eq!(metrics.max_path_depth, 7);
    assert!(metrics.message_bytes > 0);
    assert_eq!(metrics.message_lines, 3);
    assert_eq!(metrics.message_shaped_lines, 0);
    assert_eq!(metrics.message_shaped_bytes, 0);
    // The synthetic distribution should produce files across all kinds
    assert!(metrics.modified_files > 0);
    assert!(metrics.added_files > 0);
    assert!(metrics.deleted_files > 0);
    assert!(metrics.renamed_files > 0);

    let deep = CommitDetailsFixture::new(500, 12);
    let (_, deep_metrics) = deep.run_with_metrics();
    assert_eq!(deep_metrics.file_count, 500);
    assert_eq!(deep_metrics.max_path_depth, 13); // 12 dirs + 1 filename

    let large_message = CommitDetailsFixture::large_message_body(256, 4, 8_192, 160, 24, 560);
    let (_, large_message_metrics) = large_message.run_with_metrics();
    assert_eq!(large_message_metrics.file_count, 256);
    assert!(large_message_metrics.message_bytes >= 8_192);
    assert!(large_message_metrics.message_lines >= 24);
    assert_eq!(large_message_metrics.message_shaped_lines, 24);
    assert!(large_message_metrics.message_shaped_bytes > 0);
}

#[test]
fn commit_select_replace_produces_different_hashes() {
    let fixture = CommitSelectReplaceFixture::new(500, 4);
    let (hash_b, metrics) = fixture.run_with_metrics();
    assert_ne!(hash_b, 0);
    assert!(metrics.commit_ids_differ);
    assert_eq!(metrics.files_a, 500);
    assert_eq!(metrics.files_b, 500);
    // The two commits should produce different hashes (different IDs).
    assert_ne!(metrics.hash_a, metrics.hash_b);
}

#[test]
fn path_display_cache_churn_triggers_clears() {
    // 10k unique paths with an 8192-entry cache → at least 1 clear.
    let mut fixture = PathDisplayCacheChurnFixture::new(10_000, 6);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.file_count, 10_000);
    assert!(
        metrics.path_display_cache_clears >= 1,
        "expected at least 1 cache clear, got {}",
        metrics.path_display_cache_clears
    );
    // All paths are unique, so all should be misses.
    assert_eq!(metrics.path_display_cache_misses, 10_000);
    // No hits on a fresh cache with all-unique paths.
    assert_eq!(metrics.path_display_cache_hits, 0);
}

#[test]
fn pane_resize_drag_sidebar_fixture_clamps_and_preserves_main_min_width() {
    let mut fixture = PaneResizeDragStepFixture::new(PaneResizeTarget::Sidebar);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.width_bounds_recomputes, metrics.steps);
    assert_eq!(metrics.layout_recomputes, metrics.steps);
    assert!(metrics.clamp_at_min_count >= 1);
    assert!(metrics.clamp_at_max_count >= 1);
    assert!(metrics.min_main_width_px >= 280.0);
    let (sidebar_width, details_width) = fixture.pane_widths();
    assert!((200.0..=564.0).contains(&sidebar_width));
    assert_eq!(details_width, 420.0);
}

#[test]
fn pane_resize_drag_details_fixture_clamps_and_preserves_main_min_width() {
    let mut fixture = PaneResizeDragStepFixture::new(PaneResizeTarget::Details);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.width_bounds_recomputes, metrics.steps);
    assert_eq!(metrics.layout_recomputes, metrics.steps);
    assert!(metrics.clamp_at_min_count >= 1);
    assert!(metrics.clamp_at_max_count >= 1);
    assert!(metrics.min_main_width_px >= 280.0);
    let (sidebar_width, details_width) = fixture.pane_widths();
    assert_eq!(sidebar_width, 280.0);
    assert!((280.0..=704.0).contains(&details_width));
}

#[test]
fn window_resize_layout_fixture_sweeps_and_reports_metrics() {
    let fixture = WindowResizeLayoutFixture::sidebar_main_details();
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.layout_recomputes, metrics.steps);
    // The sweep goes from 800 to 1800; with sidebar 280 + details 420 + 16 handles = 716,
    // so the narrow end (800) has only 84px of main pane — should be positive.
    // But since the sweep starts at 800 and handles total 16px: 800 - 280 - 420 - 16 = 84 > 0.
    // The zero-clamp should occur if the window is narrow enough.  Let's just assert it's reported.
    assert!(metrics.max_main_w_px > 0.0);
}

#[test]
fn window_resize_layout_extreme_fixture_reports_expected_metrics() {
    let fixture = WindowResizeLayoutExtremeFixture::history_50k_commits_diff_20k_lines();
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.layout_recomputes, metrics.steps);
    assert_eq!(metrics.history_visibility_recomputes, metrics.steps);
    assert_eq!(metrics.diff_width_recomputes, metrics.steps);
    assert_eq!(metrics.history_commits, 50_000);
    assert_eq!(metrics.history_window_rows, 64);
    assert_eq!(metrics.history_rows_processed_total, 12_800);
    assert!(metrics.history_columns_hidden_steps > 0);
    assert!(metrics.history_all_columns_visible_steps > 0);
    assert_eq!(metrics.diff_lines, 20_000);
    assert_eq!(metrics.diff_window_rows, 200);
    assert!(metrics.diff_split_total_rows >= 20_000);
    assert_eq!(metrics.diff_rows_processed_total, 40_000);
    assert!(metrics.diff_narrow_fallback_steps > 0);
    assert!(metrics.max_main_w_px > metrics.min_main_w_px);
}

#[test]
fn history_column_resize_fixture_clamps_and_bounces() {
    let mut fixture = HistoryColumnResizeDragStepFixture::new(HistoryResizeColumn::Branch);
    let (hash, metrics) = fixture.run_with_metrics(HistoryResizeColumn::Branch);
    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.width_clamp_recomputes, metrics.steps);
    assert_eq!(metrics.visible_column_recomputes, 0);
    // The drag should clamp at least once at either min or max.
    assert!(
        metrics.clamp_at_min_count + metrics.clamp_at_max_count >= 1,
        "expected at least 1 clamp event, got min={} max={}",
        metrics.clamp_at_min_count,
        metrics.clamp_at_max_count
    );
}

#[test]
fn history_column_resize_fixture_hash_only_matches_metrics_path() {
    let mut hash_only = HistoryColumnResizeDragStepFixture::new(HistoryResizeColumn::Branch);
    let mut with_metrics = HistoryColumnResizeDragStepFixture::new(HistoryResizeColumn::Branch);
    let hash = hash_only.run(HistoryResizeColumn::Branch);
    let (metrics_hash, metrics) = with_metrics.run_with_metrics(HistoryResizeColumn::Branch);
    assert_eq!(hash, metrics_hash);
    assert_eq!(metrics.steps, 200);
}

#[test]
fn repo_tab_drag_hit_test_covers_all_tabs() {
    let fixture = RepoTabDragFixture::new(20);
    let (hash, metrics) = fixture.run_hit_test();
    assert_ne!(hash, 0);
    assert_eq!(metrics.tab_count, 20);
    assert_eq!(metrics.hit_test_steps, 60); // 20 * 3 steps
}

#[test]
fn repo_tab_drag_hit_test_precomputes_expected_sweep() {
    let fixture = RepoTabDragFixture::new(4);
    assert_eq!(fixture.hit_test_steps.len(), 12);
    assert_eq!(
        fixture.hit_test_steps.first().unwrap().target.repo_id,
        RepoId(1)
    );
    assert_eq!(
        fixture.hit_test_steps.last().unwrap().target.repo_id,
        RepoId(4)
    );
}

#[test]
fn repo_tab_drag_reorder_moves_tabs() {
    let fixture = RepoTabDragFixture::new(20);
    let (hash, metrics) = fixture.run_reorder();
    assert_ne!(hash, 0);
    assert_eq!(metrics.tab_count, 20);
    assert_eq!(metrics.reorder_steps, 40); // 20 * 2 steps
    // At least some steps should produce effects (persist session).
    assert!(
        metrics.effects_emitted >= 1,
        "expected at least 1 effect, got {}",
        metrics.effects_emitted
    );
}

#[test]
fn diff_split_resize_drag_fixture_oscillates_and_clamps() {
    let mut fixture = DiffSplitResizeDragStepFixture::window_200();
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.ratio_recomputes, metrics.steps);
    assert_eq!(metrics.column_width_recomputes, metrics.steps);
    // The oscillation must reach both column-min boundaries.
    assert!(
        metrics.clamp_at_min_count >= 1,
        "expected at least 1 min clamp, got {}",
        metrics.clamp_at_min_count
    );
    assert!(
        metrics.clamp_at_max_count >= 1,
        "expected at least 1 max clamp, got {}",
        metrics.clamp_at_max_count
    );
    // With 564 px main pane, the window is wide enough — no narrow fallbacks.
    assert_eq!(
        metrics.narrow_fallback_count, 0,
        "unexpected narrow fallback"
    );
    // The ratio must stay within valid bounds.
    assert!(metrics.min_ratio >= 0.0);
    assert!(metrics.max_ratio <= 1.0);
    // Column widths must be positive.
    assert!(metrics.min_left_col_px > 0.0);
    assert!(metrics.min_right_col_px > 0.0);
    // The final ratio should be within the valid range.
    let final_ratio = fixture.current_ratio();
    assert!((0.0..=1.0).contains(&final_ratio));
}

#[test]
fn scrollbar_drag_step_fixture_oscillates_and_clamps() {
    let mut fixture = ScrollbarDragStepFixture::window_200();
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.steps, 200);
    assert_eq!(metrics.thumb_metric_recomputes, metrics.steps);
    assert_eq!(metrics.scroll_offset_recomputes, metrics.steps);
    assert_eq!(metrics.viewport_h, 800.0);
    assert_eq!(metrics.max_offset, 239200.0);
    // The oscillation must reach both track boundaries.
    assert!(
        metrics.clamp_at_top_count >= 1,
        "expected at least 1 top clamp, got {}",
        metrics.clamp_at_top_count
    );
    assert!(
        metrics.clamp_at_bottom_count >= 1,
        "expected at least 1 bottom clamp, got {}",
        metrics.clamp_at_bottom_count
    );
    // Scroll position must sweep a meaningful range.
    assert!(
        metrics.min_scroll_y < 100.0,
        "min_scroll_y should be near zero, got {}",
        metrics.min_scroll_y
    );
    assert!(
        metrics.max_scroll_y > 100_000.0,
        "max_scroll_y should sweep a large range, got {}",
        metrics.max_scroll_y
    );
    // Thumb metrics must be positive.
    assert!(metrics.min_thumb_length_px > 0.0);
    assert!(metrics.max_thumb_length_px > 0.0);
    assert!(metrics.min_thumb_offset_px >= 0.0);
    // The final scroll position should be within valid bounds.
    let final_scroll = fixture.current_scroll_y();
    assert!(final_scroll >= 0.0);
    assert!(final_scroll <= 239200.0);
}

// --- CommitSearchFilterFixture tests ---

#[test]
fn commit_search_filter_fixture_has_expected_commit_count() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    assert_eq!(fixture.commit_count(), 50_000);
}

#[test]
fn commit_search_filter_fixture_has_100_distinct_authors() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    assert_eq!(fixture.distinct_authors(), 100);
}

#[test]
fn commit_search_filter_fixture_builds_message_trigram_index() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    assert!(fixture.distinct_message_trigrams() > 0);
}

#[test]
fn commit_search_filter_by_author_finds_expected_matches() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    // "Alice" is 1 of 10 first names → matches ~10% of commits.
    let (_, metrics) = fixture.run_filter_by_author_with_metrics("Alice");
    assert_eq!(metrics.total_commits, 50_000);
    assert_eq!(metrics.query_len, 5);
    // Alice appears at indices 0, 10, 20, ... → exactly 5000 matches.
    assert_eq!(metrics.matches_found, 5_000);
    // "Alicex" should match zero (no author contains "alicex").
    assert_eq!(metrics.incremental_matches, 0);
}

#[test]
fn commit_search_filter_by_message_finds_expected_matches() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    // "fix" is 1 of 10 prefixes → matches ~10% of commits.
    let (_, metrics) = fixture.run_filter_by_message_with_metrics("fix");
    assert_eq!(metrics.total_commits, 50_000);
    assert_eq!(metrics.query_len, 3);
    // "fix" appears at indices 0, 10, 20, ... → exactly 5000 matches.
    assert_eq!(metrics.matches_found, 5_000);
    // "fixx" should match zero.
    assert_eq!(metrics.incremental_matches, 0);
}

#[test]
fn commit_search_filter_by_message_short_query_matches_same_prefix_family() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    let (_, metrics) = fixture.run_filter_by_message_with_metrics("fi");
    assert_eq!(metrics.matches_found, 5_000);
}

#[test]
fn commit_search_filter_by_message_can_match_unique_commit_suffix() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    let (_, metrics) = fixture.run_filter_by_message_with_metrics("commit 12345");
    assert_eq!(metrics.matches_found, 1);
    assert_eq!(metrics.incremental_matches, 0);
}

#[test]
fn commit_search_filter_incremental_refinement_narrows_results() {
    let fixture = CommitSearchFilterFixture::new(50_000);
    // "commit" appears in all summaries ("... for commit NNN").
    let (_, metrics) = fixture.run_filter_by_message_with_metrics("commit");
    assert_eq!(metrics.matches_found, 50_000);
    // "commitx" should match zero.
    assert_eq!(metrics.incremental_matches, 0);
}

#[test]
fn commit_search_filter_run_is_deterministic() {
    let fixture = CommitSearchFilterFixture::new(1_000);
    let h1 = fixture.run_filter_by_author("Alice");
    let h2 = fixture.run_filter_by_author("Alice");
    assert_eq!(h1, h2);
    let h3 = fixture.run_filter_by_message("fix");
    let h4 = fixture.run_filter_by_message("fix");
    assert_eq!(h3, h4);
}

// --- InDiffTextSearchFixture tests ---

fn count_multiples(limit: usize, step: usize) -> u64 {
    if limit == 0 {
        0
    } else {
        ((limit - 1) / step + 1) as u64
    }
}

#[test]
fn in_diff_text_search_fixture_tracks_requested_line_count() {
    let fixture = InDiffTextSearchFixture::new(10_000);
    assert_eq!(fixture.total_lines(), 10_000);
    assert!(fixture.visible_rows() > 10_000);
}

#[test]
fn in_diff_text_search_fixture_reports_expected_match_counts() {
    let fixture = InDiffTextSearchFixture::new(10_000);
    let (_, metrics) = fixture.run_search_with_metrics("render_cache");

    let expected_broad_matches = count_multiples(10_000, 16) + count_multiples(10_000, 64);

    assert_eq!(metrics.total_lines, 10_000);
    assert_eq!(metrics.query_len, "render_cache".len() as u64);
    assert_eq!(metrics.matches_found, expected_broad_matches);
    assert_eq!(metrics.prior_matches, 0);
    assert!(metrics.visible_rows_scanned >= metrics.matches_found);
    assert!(metrics.visible_rows_scanned < fixture.visible_rows() as u64);
}

#[test]
fn in_diff_text_search_fixture_reports_refinement_counts() {
    let fixture = InDiffTextSearchFixture::new(10_000);
    let (_, metrics) = fixture.run_refinement_with_metrics("render_cache", "render_cache_hot_path");

    let expected_broad_matches = count_multiples(10_000, 16) + count_multiples(10_000, 64);
    let expected_refined_matches = count_multiples(10_000, 64);

    assert_eq!(metrics.total_lines, 10_000);
    assert_eq!(metrics.query_len, "render_cache_hot_path".len() as u64);
    assert_eq!(metrics.matches_found, expected_refined_matches);
    assert_eq!(metrics.prior_matches, expected_broad_matches);
    assert!(metrics.prior_matches > metrics.matches_found);
    assert_eq!(metrics.visible_rows_scanned, metrics.matches_found);
}

#[test]
fn in_diff_text_search_refinement_from_matches_matches_full_scan_hash() {
    let fixture = InDiffTextSearchFixture::new(10_000);
    let prior_matches = fixture.prepare_matches("render_cache");

    assert_eq!(
        fixture.run_refinement_from_matches("render_cache_hot_path", &prior_matches),
        fixture.run_search("render_cache_hot_path")
    );
}

#[test]
fn in_diff_text_search_run_is_deterministic() {
    let fixture = InDiffTextSearchFixture::new(2_048);
    let h1 = fixture.run_search("render_cache");
    let h2 = fixture.run_search("render_cache");
    assert_eq!(h1, h2);
}

#[test]
fn file_preview_text_search_fixture_tracks_requested_line_count() {
    let fixture = FilePreviewTextSearchFixture::new(10_000);
    assert_eq!(fixture.total_lines(), 10_000);
    assert!(fixture.source_bytes() > 10_000);
}

#[test]
fn file_preview_text_search_fixture_reports_expected_match_counts() {
    let fixture = FilePreviewTextSearchFixture::new(10_000);
    let (_, metrics) = fixture.run_search_with_metrics("render_cache");

    assert_eq!(metrics.total_lines, 10_000);
    assert_eq!(metrics.query_len, "render_cache".len() as u64);
    assert_eq!(metrics.matches_found, count_multiples(10_000, 16));
    assert_eq!(metrics.prior_matches, 0);
    assert!(metrics.source_bytes > metrics.total_lines);
}

#[test]
fn file_preview_text_search_fixture_reports_refinement_counts() {
    let fixture = FilePreviewTextSearchFixture::new(10_000);
    let (_, metrics) = fixture.run_refinement_with_metrics("render_cache", "render_cache_hot_path");

    assert_eq!(metrics.total_lines, 10_000);
    assert_eq!(metrics.query_len, "render_cache_hot_path".len() as u64);
    assert_eq!(metrics.prior_matches, count_multiples(10_000, 16));
    assert_eq!(metrics.matches_found, count_multiples(10_000, 64));
    assert!(metrics.prior_matches > metrics.matches_found);
}

#[test]
fn file_diff_ctrl_f_open_type_fixture_tracks_requested_sizes() {
    let fixture = FileDiffCtrlFOpenTypeFixture::new(10_000, 160);
    assert_eq!(fixture.total_lines(), 10_000);
    assert!(fixture.total_rows() >= 10_000);
    assert_eq!(fixture.visible_window_rows(), 160);
}

#[test]
fn file_diff_ctrl_f_open_type_fixture_reports_incremental_metrics() {
    let fixture = FileDiffCtrlFOpenTypeFixture::new(10_000, 160);
    let query = "render_cache_hot_path";
    let (_, metrics) = fixture.run_open_and_type_with_metrics(query);

    assert_eq!(metrics.total_lines, 10_000);
    assert!(metrics.total_rows >= metrics.total_lines);
    assert_eq!(metrics.visible_window_rows, 160);
    assert_eq!(metrics.search_opened, 1);
    assert_eq!(metrics.typed_chars, query.len() as u64);
    assert_eq!(metrics.query_steps, query.len() as u64);
    assert_eq!(metrics.final_query_len, query.len() as u64);
    assert_eq!(metrics.full_rescans, 1);
    assert_eq!(metrics.refinement_steps, query.len() as u64 - 1);
    assert_eq!(metrics.final_matches, count_multiples(10_000, 64));
    assert!(metrics.rows_scanned > metrics.total_rows);
}

#[test]
fn file_diff_ctrl_f_open_type_run_is_deterministic() {
    let fixture = FileDiffCtrlFOpenTypeFixture::new(2_048, 128);
    let h1 = fixture.run_open_and_type("render_cache_hot_path");
    let h2 = fixture.run_open_and_type("render_cache_hot_path");
    assert_eq!(h1, h2);
}

// ---------------------------------------------------------------------------
// File diff open fixture tests
// ---------------------------------------------------------------------------

#[test]
fn file_diff_open_fixture_runs_split_and_inline_windows() {
    let fixture = FileDiffOpenFixture::new(2_048);
    let split_h = fixture.run_split_first_window(200);
    let inline_h = fixture.run_inline_first_window(200);
    assert_ne!(split_h, 0);
    assert_ne!(inline_h, 0);
    // Split and inline hash should differ (different row representations).
    assert_ne!(split_h, inline_h);
}

#[test]
fn file_diff_open_fixture_split_is_deterministic() {
    let fixture = FileDiffOpenFixture::new(1_024);
    let h1 = fixture.run_split_first_window(100);
    let h2 = fixture.run_split_first_window(100);
    assert_eq!(h1, h2);
}

#[test]
fn file_diff_open_fixture_inline_is_deterministic() {
    let fixture = FileDiffOpenFixture::new(1_024);
    let h1 = fixture.run_inline_first_window(100);
    let h2 = fixture.run_inline_first_window(100);
    assert_eq!(h1, h2);
}

#[test]
fn file_diff_open_fixture_reports_metrics() {
    let fixture = FileDiffOpenFixture::new(2_048);
    let metrics = fixture.measure_first_window(200);
    assert_eq!(metrics.rows_requested, 200);
    assert!(metrics.split_total_rows > 0);
    assert_eq!(metrics.split_rows_painted, 200);
    assert!(metrics.inline_total_rows > 0);
    assert_eq!(metrics.inline_rows_painted, 200);
}

// ---------------------------------------------------------------------------
// Patch diff deep window tests
// ---------------------------------------------------------------------------

#[test]
fn patch_diff_paged_rows_fixture_runs_deep_window() {
    let fixture = PatchDiffPagedRowsFixture::new(2_048);
    let total = fixture.total_rows();
    let start_row = total * 9 / 10;
    let h = fixture.run_paged_window_at_step(start_row, 200);
    assert_ne!(h, 0);
    // Deep window hash should differ from first window hash.
    let first_h = fixture.run_paged_first_window_step(200);
    assert_ne!(h, first_h);
}

#[test]
fn patch_diff_paged_rows_fixture_deep_window_metrics() {
    let fixture = PatchDiffPagedRowsFixture::new(2_048);
    let total = fixture.total_rows();
    let start_row = total * 9 / 10;
    let metrics = fixture.measure_paged_deep_window_step(start_row, 200);
    assert_eq!(metrics.rows_requested, 200);
    assert!(metrics.patch_rows_painted > 0);
    assert!(metrics.split_rows_painted > 0);
}

// ---------------------------------------------------------------------------
// Branch sidebar cache invalidation reuse tests
// ---------------------------------------------------------------------------

#[test]
fn branch_sidebar_cache_single_ref_change_reuses_cached_rows_when_sidebar_inputs_match() {
    let mut fixture = BranchSidebarCacheFixture::balanced(20, 80, 2, 10, 5, 8);
    fixture.run_cached();
    fixture.reset_metrics();
    let cached_hash = fixture.run_cached();
    fixture.reset_metrics();

    let reused_hash = fixture.run_invalidate_single_ref();
    assert_eq!(reused_hash, cached_hash);

    let metrics = fixture.metrics();
    assert_eq!(metrics.invalidations, 1);
    assert_eq!(metrics.cache_hits, 1);
    assert_eq!(metrics.cache_misses, 0);
    assert!(metrics.rows_count > 0);
}

#[test]
fn branch_sidebar_cache_single_ref_change_rebuilds_when_branch_rows_change() {
    let mut fixture = BranchSidebarCacheFixture::balanced(20, 80, 2, 10, 5, 8);
    fixture.run_cached();
    fixture.reset_metrics();

    let Loadable::Ready(branches) = &fixture.repo.branches else {
        panic!("expected ready branches");
    };
    let mut next_branches = branches.as_ref().clone();
    next_branches[0].name.push_str("-renamed");
    fixture.repo.branches = Loadable::Ready(Arc::new(next_branches));
    fixture.repo.branches_rev = fixture.repo.branches_rev.wrapping_add(1);
    fixture.repo.branch_sidebar_rev = fixture.repo.branch_sidebar_rev.wrapping_add(1);

    let rebuilt_hash = fixture.run_cached();
    assert_ne!(rebuilt_hash, 0);

    let metrics = fixture.metrics();
    assert_eq!(metrics.cache_hits, 0);
    assert_eq!(metrics.cache_misses, 1);
    assert_eq!(metrics.invalidations, 0);
    assert!(metrics.rows_count > 0);
}

#[test]
fn branch_sidebar_cache_worktrees_ready_reuses_cached_rows_when_sidebar_inputs_match() {
    let mut fixture = BranchSidebarCacheFixture::balanced(20, 80, 2, 10, 5, 8);
    fixture.run_cached();
    fixture.reset_metrics();
    let cached_hash = fixture.run_cached();
    fixture.reset_metrics();

    // Invalidate via worktrees_rev bump.
    let reused_hash = fixture.run_invalidate_worktrees_ready();
    assert_eq!(reused_hash, cached_hash);

    let metrics = fixture.metrics();
    assert_eq!(metrics.invalidations, 1);
    assert_eq!(metrics.cache_hits, 1);
    assert_eq!(metrics.cache_misses, 0);
    assert!(metrics.rows_count > 0);
}

#[test]
fn branch_sidebar_cache_worktrees_ready_is_deterministic() {
    let mut f1 = BranchSidebarCacheFixture::balanced(20, 80, 2, 10, 5, 8);
    f1.run_cached();
    f1.reset_metrics();
    let h1 = f1.run_invalidate_worktrees_ready();

    let mut f2 = BranchSidebarCacheFixture::balanced(20, 80, 2, 10, 5, 8);
    f2.run_cached();
    f2.reset_metrics();
    let h2 = f2.run_invalidate_worktrees_ready();

    assert_eq!(h1, h2);
}

#[test]
fn markdown_preview_first_window_diff_metrics_are_populated() {
    let fixture = MarkdownPreviewFixture::new(96, 112);
    let metrics = fixture.measure_first_window_diff(200);
    assert!(metrics.old_total_rows > 0);
    assert!(metrics.new_total_rows > 0);
    assert!(metrics.old_rows_rendered > 0);
    assert!(metrics.new_rows_rendered > 0);
}

#[test]
fn markdown_preview_first_window_diff_step_returns_nonzero() {
    let fixture = MarkdownPreviewFixture::new(96, 112);
    assert_ne!(fixture.run_first_window_diff_step(200), 0);
}

#[test]
fn image_preview_first_paint_metrics_are_populated() {
    let fixture = ImagePreviewFirstPaintFixture::new(256 * 1024, 384 * 1024);
    let metrics = fixture.measure_first_paint();
    assert_eq!(metrics.old_bytes, 256 * 1024);
    assert_eq!(metrics.new_bytes, 384 * 1024);
    assert_eq!(metrics.total_bytes, 640 * 1024);
    assert_eq!(metrics.images_rendered, 2);
    assert_eq!(metrics.placeholder_cells, 0);
    assert_eq!(metrics.divider_count, 1);
}

#[test]
fn image_preview_first_paint_step_returns_nonzero() {
    let fixture = ImagePreviewFirstPaintFixture::new(256 * 1024, 384 * 1024);
    assert_ne!(fixture.run_first_paint_step(), 0);
}

#[test]
fn conflict_compare_first_window_metrics_are_populated() {
    let fixture = ConflictTwoWaySplitScrollFixture::new(1_000, 30);
    let metrics = fixture.measure_first_window(200);
    assert!(metrics.total_diff_rows > 0);
    assert!(metrics.total_visible_rows > 0);
    assert!(metrics.rows_rendered > 0);
    assert!(metrics.conflict_count > 0);
}

#[test]
fn svg_dual_path_first_window_metrics_are_populated() {
    let fixture = SvgDualPathFirstWindowFixture::new(50, 8 * 1024);
    let metrics = fixture.measure_first_window();
    assert!(metrics.old_svg_bytes > 0);
    assert!(metrics.new_svg_bytes > 0);
    assert_eq!(metrics.rasterize_success, 1);
    assert_eq!(metrics.fallback_triggered, 1);
    assert!(metrics.rasterized_png_bytes > 0);
    assert_eq!(metrics.images_rendered, 1);
    assert_eq!(metrics.divider_count, 1);
}

#[test]
fn svg_dual_path_first_window_step_returns_nonzero() {
    let fixture = SvgDualPathFirstWindowFixture::new(20, 4 * 1024);
    assert_ne!(fixture.run_first_window_step(200), 0);
}

#[test]
fn git_ops_status_clean_fixture_reports_zero_dirty_metrics() {
    let fixture = GitOpsFixture::status_clean(32);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.tracked_files, 32);
    assert_eq!(metrics.dirty_files, 0);
    assert_eq!(metrics.status_calls, 1);
    assert_eq!(metrics.log_walk_calls, 0);
    assert_eq!(metrics.ref_enumerate_calls, 0);
    assert!(metrics.status_ms > 0.0);
}

#[test]
fn git_ops_ref_enumerate_fixture_reports_branch_count_metrics() {
    let fixture = GitOpsFixture::ref_enumerate(64);
    let hash_without_trace = fixture.run();
    let (hash_with_trace, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_trace, hash_with_trace);
    assert_eq!(metrics.total_refs, 64);
    // At least 64 branches + 1 for main.
    assert!(metrics.branches_returned >= 65);
    assert_eq!(metrics.ref_enumerate_calls, 1);
    assert_eq!(metrics.status_calls, 0);
    assert_eq!(metrics.log_walk_calls, 0);
    assert!(metrics.ref_enumerate_ms > 0.0);
}

#[test]
fn keyboard_arrow_scroll_history_fixture_reports_metrics() {
    let fixture = KeyboardArrowScrollFixture::history(512, 16, 32, 64, 1, 40, 16_666_667);
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.total_rows, 512);
    assert_eq!(metrics.window_rows, 64);
    assert_eq!(metrics.scroll_step_rows, 1);
    assert_eq!(metrics.repeat_events, 40);
    assert_eq!(metrics.rows_requested_total, 2_560);
    assert_eq!(metrics.unique_windows_visited, 40);
    assert_eq!(metrics.wrap_count, 0);
    assert_eq!(metrics.final_start_row, 40);
    assert_eq!(stats.frame_count, 40);
}

#[test]
fn keyboard_arrow_scroll_diff_fixture_wraps_and_reports_metrics() {
    let fixture = KeyboardArrowScrollFixture::diff(32, 32, 8, 5, 10, 16_666_667);
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.total_rows, 32);
    assert_eq!(metrics.window_rows, 8);
    assert_eq!(metrics.scroll_step_rows, 5);
    assert_eq!(metrics.repeat_events, 10);
    assert_eq!(metrics.rows_requested_total, 80);
    assert_eq!(metrics.unique_windows_visited, 5);
    assert_eq!(metrics.wrap_count, 2);
    assert_eq!(metrics.final_start_row, 0);
    assert_eq!(stats.frame_count, 10);
}

#[test]
fn keyboard_tab_focus_cycle_fixture_wraps_and_reports_metrics() {
    let fixture = KeyboardTabFocusCycleFixture::all_panes(3, 11, 16_666_667);
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.focus_target_count, 9);
    assert_eq!(metrics.repo_tab_count, 3);
    assert_eq!(metrics.detail_input_count, 4);
    assert_eq!(metrics.cycle_events, 11);
    assert_eq!(metrics.unique_targets_visited, 9);
    assert_eq!(metrics.wrap_count, 1);
    assert_eq!(metrics.max_scan_len, 2);
    assert_eq!(metrics.final_target_index, 2);
    assert_eq!(stats.frame_count, 11);
}

#[test]
fn keyboard_stage_unstage_toggle_fixture_reports_effect_counts() {
    let fixture = KeyboardStageUnstageToggleFixture::rapid_toggle(5, 10, 16_666_667);
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.path_count, 5);
    assert_eq!(metrics.toggle_events, 10);
    assert_eq!(metrics.effect_count, 30);
    assert_eq!(metrics.stage_effect_count, 5);
    assert_eq!(metrics.unstage_effect_count, 5);
    assert_eq!(metrics.select_diff_effect_count, 20);
    assert_eq!(metrics.ops_rev_delta, 10);
    assert_eq!(metrics.diff_state_rev_delta, 10);
    assert_eq!(metrics.area_flip_count, 10);
    assert_eq!(metrics.path_wrap_count, 2);
    assert_eq!(stats.frame_count, 10);
}

#[test]
fn staging_stage_all_fixture_reports_expected_metrics() {
    let fixture = StagingFixture::stage_all(100);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.file_count, 100);
    // Single batch dispatch → 1 effect.
    assert_eq!(metrics.effect_count, 1);
    assert_eq!(metrics.stage_effect_count, 1);
    assert_eq!(metrics.unstage_effect_count, 0);
    // begin_local_action bumps ops_rev once.
    assert_eq!(metrics.ops_rev_delta, 1);
    assert_eq!(metrics.local_actions_delta, 1);
}

#[test]
fn staging_unstage_all_fixture_reports_expected_metrics() {
    let fixture = StagingFixture::unstage_all(100);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.file_count, 100);
    assert_eq!(metrics.effect_count, 1);
    assert_eq!(metrics.stage_effect_count, 0);
    assert_eq!(metrics.unstage_effect_count, 1);
    assert_eq!(metrics.ops_rev_delta, 1);
    assert_eq!(metrics.local_actions_delta, 1);
}

#[test]
fn staging_interleaved_fixture_reports_expected_metrics() {
    let fixture = StagingFixture::interleaved(100);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    // interleaved(100) → half unstaged (50) + half staged (50) = 100 paths.
    assert_eq!(metrics.file_count, 100);
    // 100 individual dispatches → 100 effects.
    assert_eq!(metrics.effect_count, 100);
    // Even-indexed paths → StagePath, odd-indexed → UnstagePath.
    assert_eq!(metrics.stage_effect_count, 50);
    assert_eq!(metrics.unstage_effect_count, 50);
    // Each dispatch bumps ops_rev once: 100 bumps.
    assert_eq!(metrics.ops_rev_delta, 100);
    assert_eq!(metrics.local_actions_delta, 100);
}

// ---------------------------------------------------------------------------
// Undo/redo fixture tests
// ---------------------------------------------------------------------------

#[test]
fn undo_redo_deep_stack_fixture_reports_expected_metrics() {
    let fixture = UndoRedoFixture::deep_stack(50);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.region_count, 50);
    // One ConflictSetRegionChoice dispatch per region.
    assert_eq!(metrics.apply_dispatches, 50);
    // No reset or replay in deep-stack scenario.
    assert_eq!(metrics.reset_dispatches, 0);
    assert_eq!(metrics.replay_dispatches, 0);
    // Each dispatch bumps conflict_rev once.
    assert_eq!(metrics.conflict_rev_delta, 50);
}

#[test]
fn undo_redo_undo_replay_fixture_reports_expected_metrics() {
    let fixture = UndoRedoFixture::undo_replay(30);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.region_count, 30);
    // 30 initial apply dispatches.
    assert_eq!(metrics.apply_dispatches, 30);
    // 1 ConflictResetResolutions dispatch.
    assert_eq!(metrics.reset_dispatches, 1);
    // 30 replay dispatches.
    assert_eq!(metrics.replay_dispatches, 30);
    // 30 apply + 1 reset + 30 replay = 61 conflict_rev bumps.
    assert_eq!(metrics.conflict_rev_delta, 61);
}

#[test]
fn undo_redo_deep_stack_is_deterministic() {
    let fixture = UndoRedoFixture::deep_stack(20);
    let (hash1, _) = fixture.run_with_metrics();
    let (hash2, _) = fixture.run_with_metrics();
    assert_eq!(hash1, hash2);
}

// ---------------------------------------------------------------------------
// File fuzzy find fixture tests
// ---------------------------------------------------------------------------

#[test]
fn file_fuzzy_find_fixture_tracks_file_count() {
    let fixture = FileFuzzyFindFixture::new(1_000);
    assert_eq!(fixture.total_files(), 1_000);
}

#[test]
fn file_fuzzy_find_fixture_reports_expected_metrics() {
    let fixture = FileFuzzyFindFixture::new(1_000);
    let (_, metrics) = fixture.run_find_with_metrics("dcrs");

    assert_eq!(metrics.total_files, 1_000);
    assert_eq!(metrics.query_len, 4);
    assert_eq!(metrics.files_scanned, 1_000);
    // "dcrs" as a subsequence should match some paths (those containing
    // d…c…r…s in order, e.g. "diff_cache…rs").
    assert!(metrics.matches_found > 0);
    assert_eq!(metrics.prior_matches, 0);
}

#[test]
fn file_fuzzy_find_incremental_narrows_matches() {
    let fixture = FileFuzzyFindFixture::new(1_000);
    let (_, metrics) = fixture.run_incremental_with_metrics("dc", "dcrs");

    assert_eq!(metrics.total_files, 1_000);
    assert_eq!(metrics.query_len, 4);
    // The longer query "dcrs" should match fewer or equal paths than the
    // shorter query "dc".
    assert!(metrics.prior_matches >= metrics.matches_found);
    assert!(metrics.prior_matches > 0);
    assert!(metrics.matches_found > 0);
    assert!(metrics.files_scanned >= 1_000);
    assert!(metrics.files_scanned <= 1_000 + metrics.prior_matches);
    assert!(
        metrics.files_scanned < 1_000 + metrics.prior_matches,
        "strict-extension refinement should skip obviously impossible prior matches",
    );
}

#[test]
fn file_fuzzy_find_run_is_deterministic() {
    let fixture = FileFuzzyFindFixture::new(2_048);
    let h1 = fixture.run_find("dcrs");
    let h2 = fixture.run_find("dcrs");
    assert_eq!(h1, h2);
}

#[test]
fn file_fuzzy_find_incremental_matches_full_scan_hash() {
    let fixture = FileFuzzyFindFixture::new(2_048);
    let incremental = fixture.run_incremental("dc", "dcrs");
    let full = fixture.run_find("dcrs");
    assert_eq!(incremental, full);
}

#[test]
fn file_fuzzy_find_ordered_pair_prefilter_matches_naive_hashes() {
    let fixture = FileFuzzyFindFixture::new(4_096);
    for query in ["dcrs", "dc", "ss", "src", "a/b", "render.rs"] {
        assert_eq!(
            fixture.run_find(query),
            fixture.run_find_without_ordered_pair_prefilter(query),
            "query {query:?} should preserve fuzzy-search results",
        );
    }
}

#[test]
fn file_fuzzy_find_direct_metrics_are_allocation_free() {
    let fixture = FileFuzzyFindFixture::new(100_000);
    let ((_hash, _metrics), alloc_metrics) =
        measure_allocations(|| fixture.run_find_with_metrics("dcrs"));
    assert_eq!(
        alloc_metrics.alloc_ops, 0,
        "unexpected broad alloc metrics: {alloc_metrics:?}"
    );
    assert_eq!(
        alloc_metrics.alloc_bytes, 0,
        "unexpected broad alloc metrics: {alloc_metrics:?}"
    );
}

#[test]
fn file_fuzzy_find_incremental_direct_metrics_are_allocation_free() {
    let fixture = FileFuzzyFindFixture::new(100_000);
    let ((_hash, _metrics), alloc_metrics) =
        measure_allocations(|| fixture.run_incremental_with_metrics("dc", "dcrs"));
    assert_eq!(
        alloc_metrics.alloc_ops, 0,
        "unexpected incremental alloc metrics: {alloc_metrics:?}"
    );
    assert_eq!(
        alloc_metrics.alloc_bytes, 0,
        "unexpected incremental alloc metrics: {alloc_metrics:?}"
    );
}

// ---------------------------------------------------------------------------
// frame_timing/sidebar_resize_drag_sustained
// ---------------------------------------------------------------------------

#[test]
fn sidebar_resize_drag_sustained_reports_frame_timing_metrics() {
    let mut fixture = SidebarResizeDragSustainedFixture::new(10, 16_666_667);
    let (hash, stats, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.frames, 10);
    assert_eq!(metrics.steps_per_frame, 200);
    assert!(stats.frame_count >= 10);
    assert!(stats.p50_frame_ns > 0);
}

// ---------------------------------------------------------------------------
// frame_timing/rapid_commit_selection_changes
// ---------------------------------------------------------------------------

#[test]
fn rapid_commit_selection_reports_expected_metrics() {
    let fixture = RapidCommitSelectionFixture::new(8, 50, 16_666_667);
    let (hash, stats, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.commit_count, 8);
    assert_eq!(metrics.files_per_commit, 50);
    assert_eq!(metrics.selections, 8);
    assert!(stats.frame_count >= 8);
    assert!(stats.p50_frame_ns > 0);
}

#[test]
fn rapid_commit_selection_is_deterministic() {
    let fixture = RapidCommitSelectionFixture::new(6, 30, 16_666_667);
    let h1 = fixture.run();
    let h2 = fixture.run();
    assert_eq!(h1, h2);
}

// ---------------------------------------------------------------------------
// frame_timing/repo_switch_during_scroll
// ---------------------------------------------------------------------------

#[test]
fn repo_switch_during_scroll_reports_expected_metrics() {
    let fixture = RepoSwitchDuringScrollFixture::new(
        1_000, // commits
        10,    // local branches
        30,    // remote branches
        60,    // window rows
        12,    // scroll step rows
        30,    // frames
        10,    // switch every 10 frames
        16_666_667,
    );
    let (hash, stats, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.total_frames, 30);
    // Switches at frames 10, 20 → 2 switch frames
    assert_eq!(metrics.switch_frames, 2);
    assert_eq!(metrics.scroll_frames, 28);
    assert!(stats.frame_count >= 30);
    assert!(stats.p50_frame_ns > 0);
}

#[test]
fn repo_switch_during_scroll_is_deterministic() {
    let fixture = RepoSwitchDuringScrollFixture::new(1_000, 10, 30, 60, 12, 30, 10, 16_666_667);
    let h1 = fixture.run();
    let h2 = fixture.run();
    assert_eq!(h1, h2);
}

// ---------------------------------------------------------------------------
// fs_event — filesystem event to status update harness
// ---------------------------------------------------------------------------

#[test]
fn fs_event_single_file_save_detects_one_dirty_file() {
    let fixture = FsEventFixture::single_file_save(50);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.mutation_files, 1);
    assert_eq!(metrics.dirty_files_detected, 1);
    assert_eq!(metrics.status_calls, 1);
    assert!(metrics.tracked_files >= 50);
    assert!(metrics.status_ms > 0.0);
}

#[test]
fn fs_event_git_checkout_batch_detects_all_dirty_files() {
    let fixture = FsEventFixture::git_checkout_batch(100, 30);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.mutation_files, 30);
    assert_eq!(metrics.dirty_files_detected, 30);
    assert_eq!(metrics.status_calls, 1);
    assert!(metrics.tracked_files >= 100);
}

#[test]
fn fs_event_rapid_saves_debounce_coalesces_into_single_status() {
    let fixture = FsEventFixture::rapid_saves_debounce(80, 20);
    let (hash, metrics) = fixture.run_with_metrics();
    assert_ne!(hash, 0);
    assert_eq!(metrics.coalesced_saves, 20);
    assert_eq!(metrics.dirty_files_detected, 20);
    assert_eq!(metrics.status_calls, 1);
    assert!(metrics.tracked_files >= 80);
}

#[test]
fn fs_event_false_positive_under_churn_finds_zero_dirty() {
    let fixture = FsEventFixture::false_positive_under_churn(80, 20);
    let (_hash, metrics) = fixture.run_with_metrics();
    // hash may be 0 — clean status (0 staged + 0 unstaged) hashes deterministically.
    assert_eq!(metrics.mutation_files, 20);
    assert_eq!(metrics.dirty_files_detected, 0);
    assert_eq!(metrics.false_positives, 20);
    assert_eq!(metrics.status_calls, 1);
}

#[test]
fn fs_event_single_file_save_is_deterministic() {
    let fixture = FsEventFixture::single_file_save(50);
    let h1 = fixture.run();
    let h2 = fixture.run();
    assert_eq!(h1, h2);
}

// ---------------------------------------------------------------------------
// idle_resource — long-running CPU/RSS sampling harness
// ---------------------------------------------------------------------------

#[test]
fn idle_cpu_usage_short_window_reports_expected_counts() {
    let fixture = IdleResourceFixture::with_config(
        IdleResourceScenario::CpuUsageSingleRepo60s,
        IdleResourceConfig {
            repo_count: 1,
            tracked_files_per_repo: 16,
            sample_window: Duration::from_millis(30),
            sample_interval: Duration::from_millis(10),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        },
    );
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.open_repos, 1);
    assert_eq!(metrics.tracked_files_per_repo, 16);
    assert_eq!(metrics.sample_count, 3);
    assert_eq!(metrics.refresh_cycles, 0);
    assert_eq!(metrics.repos_refreshed, 0);
    assert_eq!(metrics.status_calls, 0);
    assert!(metrics.sample_duration_ms >= 20.0);
}

#[test]
fn idle_background_refresh_short_window_reports_status_calls() {
    let fixture = IdleResourceFixture::with_config(
        IdleResourceScenario::BackgroundRefreshCostPerCycle,
        IdleResourceConfig {
            repo_count: 3,
            tracked_files_per_repo: 24,
            sample_window: Duration::ZERO,
            sample_interval: Duration::from_millis(1),
            refresh_cycles: 4,
            wake_gap: Duration::ZERO,
        },
    );
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.open_repos, 3);
    assert_eq!(metrics.refresh_cycles, 4);
    assert_eq!(metrics.repos_refreshed, 12);
    assert_eq!(metrics.status_calls, 12);
    assert!(metrics.status_ms > 0.0);
    assert!(metrics.avg_refresh_cycle_ms > 0.0);
    assert!(metrics.max_refresh_cycle_ms >= metrics.avg_refresh_cycle_ms);
}

#[test]
fn idle_wake_resume_reports_single_refresh_burst() {
    let fixture = IdleResourceFixture::with_config(
        IdleResourceScenario::WakeFromSleepResume,
        IdleResourceConfig {
            repo_count: 2,
            tracked_files_per_repo: 24,
            sample_window: Duration::ZERO,
            sample_interval: Duration::ZERO,
            refresh_cycles: 1,
            wake_gap: Duration::from_millis(1),
        },
    );
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.open_repos, 2);
    assert_eq!(metrics.refresh_cycles, 1);
    assert_eq!(metrics.repos_refreshed, 2);
    assert_eq!(metrics.status_calls, 2);
    assert!(metrics.status_ms > 0.0);
    assert!(metrics.wake_resume_ms > 0.0);
}

#[test]
fn idle_cpu_usage_hash_is_deterministic_for_fixed_config() {
    let fixture = IdleResourceFixture::with_config(
        IdleResourceScenario::CpuUsageTenRepos60s,
        IdleResourceConfig {
            repo_count: 2,
            tracked_files_per_repo: 8,
            sample_window: Duration::from_millis(20),
            sample_interval: Duration::from_millis(10),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        },
    );

    assert_eq!(fixture.run(), fixture.run());
}

#[cfg(target_os = "linux")]
#[test]
fn idle_linux_proc_parsers_extract_runtime_and_rss() {
    assert_eq!(
        parse_first_u64_ascii_token(b"123456789 456 789\n"),
        Some(123_456_789)
    );
    assert_eq!(
        parse_vmrss_kib(b"Name:\ttest\nState:\tR\nVmRSS:\t  24696 kB\nThreads:\t1\n"),
        Some(24_696)
    );
}

// ---------------------------------------------------------------------------
// network — mocked transport progress and cancellation
// ---------------------------------------------------------------------------

#[test]
fn network_ui_responsiveness_fixture_reports_expected_metrics() {
    let fixture = NetworkFixture::ui_responsiveness_during_fetch(
        128, // commits
        8,   // local branches
        16,  // remote branches
        32,  // window rows
        4,   // scroll step rows
        12,  // frames
        48,  // progress line bytes
        16,  // progress bar width
        16_666_667,
    );
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.total_frames, 12);
    assert_eq!(metrics.scroll_frames, 12);
    assert_eq!(metrics.progress_updates, 12);
    assert_eq!(metrics.render_passes, 12);
    assert_eq!(metrics.total_rows, 128);
    assert_eq!(metrics.window_rows, 32);
    assert_eq!(metrics.output_tail_lines, 12);
    assert_eq!(metrics.tail_trim_events, 0);
    assert_eq!(metrics.bar_width, 16);
    assert_eq!(stats.frame_count, 12);
    assert!(metrics.rendered_bytes > 0);
}

#[test]
fn network_progress_bar_fixture_trims_tail_and_reports_render_counts() {
    let fixture = NetworkFixture::progress_bar_update_render_cost(90, 48, 24, 16_666_667);
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.total_frames, 90);
    assert_eq!(metrics.scroll_frames, 0);
    assert_eq!(metrics.progress_updates, 90);
    assert_eq!(metrics.render_passes, 90);
    assert_eq!(metrics.output_tail_lines, 80);
    assert_eq!(metrics.tail_trim_events, 10);
    assert_eq!(metrics.bar_width, 24);
    assert_eq!(stats.frame_count, 90);
    assert!(metrics.rendered_bytes > 0);
}

#[test]
fn network_cancel_fixture_bounds_latency_and_post_cancel_drain() {
    let fixture = NetworkFixture::cancel_operation_latency(
        12, // cancel after updates
        3,  // queued updates drained after cancel
        40, // total snapshots available
        48, // progress line bytes
        20, // progress bar width
        16_666_667,
    );
    let hash_without_capture = fixture.run();
    let (hash_with_capture, stats, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_without_capture, hash_with_capture);
    assert_eq!(metrics.total_frames, 16);
    assert_eq!(metrics.progress_updates, 15);
    assert_eq!(metrics.render_passes, 16);
    assert_eq!(metrics.cancel_frames_until_stopped, 4);
    assert_eq!(metrics.drained_updates_after_cancel, 3);
    assert_eq!(metrics.output_tail_lines, 15);
    assert_eq!(metrics.tail_trim_events, 0);
    assert_eq!(metrics.bar_width, 20);
    assert_eq!(stats.frame_count, 16);
    assert!(metrics.rendered_bytes > 0);
}

// ---------------------------------------------------------------------------
// clipboard — copy from diff, paste into commit message, selection range
// ---------------------------------------------------------------------------

#[test]
fn clipboard_copy_from_diff_extracts_expected_text() {
    let fixture = ClipboardFixture::copy_from_diff(200);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.total_lines, 200);
    assert_eq!(metrics.line_iterations, 200);
    // Content lines should produce some bytes (excluding header/hunk lines).
    assert!(metrics.total_bytes > 0);
    assert!(metrics.allocations_approx > 0);
}

#[test]
fn clipboard_paste_into_commit_message_inserts_text() {
    let fixture = ClipboardFixture::paste_into_commit_message(100, 96);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.total_lines, 100);
    assert!(metrics.total_bytes > 0);
    assert_eq!(metrics.line_iterations, 1);
    assert_eq!(metrics.allocations_approx, 1);
}

#[test]
fn clipboard_select_range_iterates_correct_range() {
    let fixture = ClipboardFixture::select_range_in_diff(500, 250);
    let (hash, metrics) = fixture.run_with_metrics();

    assert_ne!(hash, 0);
    assert_eq!(metrics.total_lines, 500);
    assert_eq!(metrics.line_iterations, 1);
    assert!(metrics.total_bytes > 0);
    assert_eq!(metrics.allocations_approx, 0);
}

#[test]
fn clipboard_copy_is_deterministic() {
    let fixture = ClipboardFixture::copy_from_diff(500);
    assert_eq!(fixture.run(), fixture.run());
}

#[test]
fn clipboard_copy_from_diff_preallocates_without_reallocating() {
    let fixture = ClipboardFixture::copy_from_diff(10_000);
    let ((_hash, metrics), alloc_metrics) = measure_allocations(|| fixture.run_with_metrics());

    assert!(metrics.total_bytes > 0);
    assert_eq!(alloc_metrics.alloc_ops, 1);
    assert_eq!(alloc_metrics.realloc_ops, 0);
    assert!(alloc_metrics.alloc_bytes >= metrics.total_bytes);
}

#[test]
fn clipboard_paste_is_deterministic() {
    let fixture = ClipboardFixture::paste_into_commit_message(200, 96);
    assert_eq!(fixture.run(), fixture.run());
}

#[test]
fn clipboard_select_range_is_allocation_free() {
    let fixture = ClipboardFixture::select_range_in_diff(10_000, 5_000);
    let ((_hash, metrics), alloc_metrics) = measure_allocations(|| fixture.run_with_metrics());

    assert_eq!(metrics.line_iterations, 1);
    assert_eq!(metrics.allocations_approx, 0);
    assert_eq!(alloc_metrics.alloc_ops, 0);
    assert_eq!(alloc_metrics.alloc_bytes, 0);
}

// ---------------------------------------------------------------------------
// display — render cost by scale, two windows, DPI move
// ---------------------------------------------------------------------------

#[test]
fn display_render_cost_by_scale_reports_expected_metrics() {
    let fixture = DisplayFixture::render_cost_by_scale(
        200, // commits
        10,  // local branches
        20,  // remote branches
        100, // diff lines
        40,  // history window
        50,  // diff window
        1920.0, 280.0, 420.0,
    );
    let hash_plain = fixture.run();
    let (hash_metrics, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_plain, hash_metrics);
    assert_eq!(metrics.scale_factors_tested, 3);
    assert_eq!(metrics.total_layout_passes, 3);
    assert_eq!(metrics.windows_rendered, 3);
    assert_eq!(metrics.history_rows_per_pass, 40);
    assert_eq!(metrics.diff_rows_per_pass, 50);
    // 3 passes × (40 history + 50 diff) = 270
    assert_eq!(metrics.total_rows_rendered, 270);
    assert!(metrics.layout_width_min_px > 0.0);
    assert!(metrics.layout_width_max_px > metrics.layout_width_min_px);
    assert_eq!(metrics.re_layout_passes, 0);
}

#[test]
fn display_two_windows_same_repo_reports_expected_metrics() {
    let fixture = DisplayFixture::two_windows_same_repo(
        200, // commits
        10,  // local branches
        20,  // remote branches
        100, // diff lines
        40,  // history window
        50,  // diff window
        1920.0, 280.0, 420.0,
    );
    let hash_plain = fixture.run();
    let (hash_metrics, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_plain, hash_metrics);
    assert_eq!(metrics.scale_factors_tested, 1);
    assert_eq!(metrics.total_layout_passes, 1);
    assert_eq!(metrics.windows_rendered, 2);
    assert_eq!(metrics.history_rows_per_pass, 40);
    assert_eq!(metrics.diff_rows_per_pass, 50);
    // 2 windows × (40 history + 50 diff) = 180
    assert_eq!(metrics.total_rows_rendered, 180);
    assert!(metrics.layout_width_min_px > 0.0);
    assert_eq!(metrics.re_layout_passes, 0);
}

#[test]
fn display_window_move_between_dpis_reports_expected_metrics() {
    let fixture = DisplayFixture::window_move_between_dpis(
        200, // commits
        10,  // local branches
        20,  // remote branches
        100, // diff lines
        40,  // history window
        50,  // diff window
        1920.0, 280.0, 420.0,
    );
    let hash_plain = fixture.run();
    let (hash_metrics, metrics) = fixture.run_with_metrics();

    assert_eq!(hash_plain, hash_metrics);
    assert_eq!(metrics.scale_factors_tested, 2);
    assert_eq!(metrics.total_layout_passes, 2);
    assert_eq!(metrics.re_layout_passes, 1);
    assert_eq!(metrics.windows_rendered, 2);
    assert_eq!(metrics.history_rows_per_pass, 40);
    assert_eq!(metrics.diff_rows_per_pass, 50);
    // 2 passes × (40 history + 50 diff) = 180
    assert_eq!(metrics.total_rows_rendered, 180);
    assert!(metrics.layout_width_max_px > metrics.layout_width_min_px);
}

#[test]
fn display_render_cost_by_scale_is_deterministic() {
    let fixture =
        DisplayFixture::render_cost_by_scale(200, 10, 20, 100, 40, 50, 1920.0, 280.0, 420.0);
    assert_eq!(fixture.run(), fixture.run());
}

#[test]
fn display_two_windows_is_deterministic() {
    let fixture =
        DisplayFixture::two_windows_same_repo(200, 10, 20, 100, 40, 50, 1920.0, 280.0, 420.0);
    assert_eq!(fixture.run(), fixture.run());
}
