use super::*;
use gitcomet_core::conflict_session::ConflictPayload;

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
    assert!(duplicated.base_bytes.is_some());

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
    assert!(duplicated.current_bytes.is_some());
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
fn branch_sidebar_fixture_scales_with_more_entries() {
    let small = BranchSidebarFixture::new(8, 16, 2, 0, 0, 0);
    let large = BranchSidebarFixture::new(120, 600, 6, 40, 40, 80);
    assert!(small.row_count() > 0);
    assert!(large.row_count() > small.row_count());
}

#[test]
fn history_graph_fixture_preserves_requested_commit_count() {
    let fixture = HistoryGraphFixture::new(2_000, 7, 9);
    assert_eq!(fixture.commit_count(), 2_000);
    assert_ne!(fixture.run(), 0);
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
fn text_input_prepaint_windowed_fixture_wraps_start_offsets() {
    let mut fixture = TextInputPrepaintWindowedFixture::new(512, 96, 640);
    let hash_a = fixture.run_windowed_step(17, 48);
    let hash_b = fixture.run_windowed_step(17 + fixture.total_rows() * 3, 48);
    assert_eq!(hash_a, hash_b);
    assert!(fixture.cache_entries() > 0);
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
fn text_input_long_line_cap_fixture_bounds_shaping_slice() {
    let fixture = TextInputLongLineCapFixture::new(128 * 1024);
    let capped_len = fixture.capped_len(4 * 1024);
    let uncapped_len = fixture.capped_len(256 * 1024);
    assert!(capped_len < uncapped_len);
    assert_ne!(fixture.run_with_cap(4 * 1024), 0);
    assert_ne!(fixture.run_without_cap(), 0);
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
fn text_model_snapshot_clone_fixture_runs_model_and_string_control_paths() {
    let fixture = TextModelSnapshotCloneCostFixture::new(512 * 1024);
    let model_hash = fixture.run_snapshot_clone_step(2_048);
    let string_hash = fixture.run_string_clone_control_step(2_048);
    assert_ne!(model_hash, 0);
    assert_ne!(string_hash, 0);
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
fn text_model_fragmented_edit_fixture_runs_all_paths() {
    let fixture = TextModelFragmentedEditFixture::new(64 * 1024, 200);
    let edit_hash = fixture.run_fragmented_edit_step();
    let materialize_hash = fixture.run_materialize_after_edits_step();
    let shared_hash = fixture.run_shared_string_after_edits_step(8);
    let control_hash = fixture.run_string_edit_control_step();
    assert_ne!(edit_hash, 0);
    assert_ne!(materialize_hash, 0);
    assert_ne!(shared_hash, 0);
    assert_ne!(control_hash, 0);
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
    assert_ne!(prepare_fixture.run_background_prepare_step(), 0);

    visible_fixture.prime_visible_window(48);
    assert_ne!(visible_fixture.run_visible_window_step(0, 48), 0);
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

    assert_ne!(fixture.run_visible_window_pending_step(0, 48), 0);

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

    fixture.prime_visible_window(96);
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
    fixture.prime_visible_window(6);
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
