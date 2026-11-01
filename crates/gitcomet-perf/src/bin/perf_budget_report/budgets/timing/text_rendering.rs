use super::super::*;

pub(crate) const PERF_BUDGETS: &[PerfBudgetSpec] = &[
    // -----------------------------------------------------------------------
    // Pre-existing benchmark groups — timing budgets added to close coverage
    // gaps. These groups were already registered in criterion_group! but had no
    // entries in PERF_BUDGETS. Thresholds are intentionally generous (first-run
    // conservative) and should be tightened once stable-runner baselines exist.
    // -----------------------------------------------------------------------
    // --- diff_open_patch_first_window --- first-window latency (had structural budgets only)
    PerfBudgetSpec {
        label: "diff_open_patch_first_window/200",
        estimate_path: "diff_open_patch_first_window/200/new/estimates.json",
        // Paged diff open: materialize ~200 visible rows from a 5k-line diff.
        // Similar to other diff-open first-window cases at 15 ms.
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_scroll --- large file diff scroll step
    PerfBudgetSpec {
        label: "diff_scroll/normal_lines_window/200",
        estimate_path: "diff_scroll/normal_lines_window/200/new/estimates.json",
        // One scroll step rendering 200 diff rows with normal-length lines.
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "diff_scroll/long_lines_window/200",
        estimate_path: "diff_scroll/long_lines_window/200/new/estimates.json",
        // Long lines increase per-row shaping cost.
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    // --- patch_diff_search_query_update --- search query against paged diff rows
    PerfBudgetSpec {
        label: "patch_diff_search_query_update/window_200",
        estimate_path: "patch_diff_search_query_update/window_200/new/estimates.json",
        // Full scan + highlight update across visible diff window.
        threshold_ns: 40.0 * NANOS_PER_MILLISECOND,
    },
    // --- file_diff_replacement_alignment --- alignment algorithms for replacement blocks
    // These benchmarks compute full LCS-based side-by-side alignment plans across
    // 12 replacement blocks of 48 lines each. Scratch (from-scratch LCS) is slower
    // than strsim (character-level similarity). Budgets set conservatively from
    // measured ~240-410 ms range.
    PerfBudgetSpec {
        label: "file_diff_replacement_alignment/balanced_blocks/scratch",
        estimate_path: "file_diff_replacement_alignment/balanced_blocks/scratch/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_replacement_alignment/balanced_blocks/strsim",
        estimate_path: "file_diff_replacement_alignment/balanced_blocks/strsim/new/estimates.json",
        threshold_ns: 300.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_replacement_alignment/skewed_blocks/scratch",
        estimate_path: "file_diff_replacement_alignment/skewed_blocks/scratch/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_replacement_alignment/skewed_blocks/strsim",
        estimate_path: "file_diff_replacement_alignment/skewed_blocks/strsim/new/estimates.json",
        threshold_ns: 300.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_input_prepaint_windowed --- windowed text input rendering
    PerfBudgetSpec {
        label: "text_input_prepaint_windowed/window_rows/80",
        estimate_path: "text_input_prepaint_windowed/window_rows/80/new/estimates.json",
        // Visible-window shaping of 80 rows — should be frame-safe.
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_prepaint_windowed/full_document_control",
        estimate_path: "text_input_prepaint_windowed/full_document_control/new/estimates.json",
        // Full-document control path — heavier than windowed.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_input_runs_streamed_highlight --- dense and sparse highlight cursors
    PerfBudgetSpec {
        label: "text_input_runs_streamed_highlight_dense/legacy_scan",
        estimate_path: "text_input_runs_streamed_highlight_dense/legacy_scan/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        estimate_path: "text_input_runs_streamed_highlight_dense/streamed_cursor/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        estimate_path: "text_input_runs_streamed_highlight_sparse/legacy_scan/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        estimate_path: "text_input_runs_streamed_highlight_sparse/streamed_cursor/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_input_long_line_cap --- capped vs uncapped long-line shaping
    PerfBudgetSpec {
        label: "text_input_long_line_cap/capped_bytes/4096",
        estimate_path: "text_input_long_line_cap/capped_bytes/4096/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_long_line_cap/uncapped_control",
        estimate_path: "text_input_long_line_cap/uncapped_control/new/estimates.json",
        // Uncapped is intentionally heavier — this budget validates the cap's value.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_input_wrap_incremental_tabs --- tab-aware wrapping
    PerfBudgetSpec {
        label: "text_input_wrap_incremental_tabs/full_recompute",
        estimate_path: "text_input_wrap_incremental_tabs/full_recompute/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_wrap_incremental_tabs/incremental_patch",
        estimate_path: "text_input_wrap_incremental_tabs/incremental_patch/new/estimates.json",
        // Incremental should be cheaper than full recompute.
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_input_wrap_incremental_burst_edits --- burst edit wrapping
    // Full recompute of 20k lines × 12 burst rounds = 240k line recomputations;
    // measured at ~17.6ms after Turn 14 ASCII+memchr optimization. The 5ms
    // budget was aggressive — actual floor is dominated by per-line wrap
    // estimation across 240k recomputations.
    PerfBudgetSpec {
        label: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        estimate_path: "text_input_wrap_incremental_burst_edits/full_recompute/12/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        estimate_path: "text_input_wrap_incremental_burst_edits/incremental_patch/12/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_model_snapshot_clone_cost --- piece table vs shared string clone overhead
    PerfBudgetSpec {
        label: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        estimate_path: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        estimate_path: "text_model_snapshot_clone_cost/shared_string_clone_control/8192/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    // --- text_model_bulk_load_large --- large text model construction
    PerfBudgetSpec {
        label: "text_model_bulk_load_large/piece_table_append_large",
        estimate_path: "text_model_bulk_load_large/piece_table_append_large/new/estimates.json",
        // Tightened from the initial 10 ms placeholder after a local
        // baseline run landed around 2.42-2.47 ms.
        threshold_ns: 4.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_model_bulk_load_large/piece_table_from_large_text",
        estimate_path: "text_model_bulk_load_large/piece_table_from_large_text/new/estimates.json",
        // Tightened from the initial 10 ms placeholder after a local
        // baseline run landed around 1.69-1.71 ms.
        threshold_ns: 3.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_model_bulk_load_large/string_push_control",
        estimate_path: "text_model_bulk_load_large/string_push_control/new/estimates.json",
        // Tightened from the initial 10 ms placeholder after a local
        // baseline run landed around 107-108 us.
        threshold_ns: 0.3 * NANOS_PER_MILLISECOND,
    },
    // --- text_model_fragmented_edits --- fragmented edit patterns
    PerfBudgetSpec {
        label: "text_model_fragmented_edits/piece_table_edits",
        estimate_path: "text_model_fragmented_edits/piece_table_edits/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_model_fragmented_edits/materialize_after_edits",
        estimate_path: "text_model_fragmented_edits/materialize_after_edits/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_model_fragmented_edits/shared_string_after_edits/64",
        estimate_path: "text_model_fragmented_edits/shared_string_after_edits/64/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "text_model_fragmented_edits/string_edit_control",
        estimate_path: "text_model_fragmented_edits/string_edit_control/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    // --- file_diff_syntax_prepare --- cold and warm syntax tree preparation
    PerfBudgetSpec {
        label: "file_diff_syntax_prepare/file_diff_syntax_prepare_cold",
        estimate_path: "file_diff_syntax_prepare/file_diff_syntax_prepare_cold/new/estimates.json",
        // Cold parse of a full syntax tree from source text.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_syntax_prepare/file_diff_syntax_prepare_warm",
        estimate_path: "file_diff_syntax_prepare/file_diff_syntax_prepare_warm/new/estimates.json",
        // Warm path reuses existing parse — should be much faster.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- file_diff_syntax_query_stress --- nested long-line query cost
    PerfBudgetSpec {
        label: "file_diff_syntax_query_stress/nested_long_lines_cold",
        estimate_path: "file_diff_syntax_query_stress/nested_long_lines_cold/new/estimates.json",
        // Stress test: deeply nested syntax with long lines. Intentionally generous.
        threshold_ns: 100.0 * NANOS_PER_MILLISECOND,
    },
    // --- file_diff_syntax_reparse --- incremental reparse after edits
    PerfBudgetSpec {
        label: "file_diff_syntax_reparse/file_diff_syntax_reparse_small_edit",
        estimate_path: "file_diff_syntax_reparse/file_diff_syntax_reparse_small_edit/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_syntax_reparse/file_diff_syntax_reparse_large_edit",
        estimate_path: "file_diff_syntax_reparse/file_diff_syntax_reparse_large_edit/new/estimates.json",
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    // --- file_diff_inline_syntax_projection --- inline syntax projection windows
    PerfBudgetSpec {
        label: "file_diff_inline_syntax_projection/visible_window_pending/200",
        estimate_path: "file_diff_inline_syntax_projection/visible_window_pending/200/new/estimates.json",
        // Pending syntax: projection from partial parse.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_inline_syntax_projection/visible_window_ready/200",
        estimate_path: "file_diff_inline_syntax_projection/visible_window_ready/200/new/estimates.json",
        // Ready syntax: projection from completed parse — should be cheaper.
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    // --- file_diff_syntax_cache_drop --- deferred vs inline cache eviction
    PerfBudgetSpec {
        label: "file_diff_syntax_cache_drop/deferred_drop/4",
        estimate_path: "file_diff_syntax_cache_drop/deferred_drop/4/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "file_diff_syntax_cache_drop/inline_drop_control/4",
        estimate_path: "file_diff_syntax_cache_drop/inline_drop_control/4/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    // --- prepared_syntax_multidoc_cache_hit_rate --- multidoc LRU cache hot path
    // 6 documents cycled through the LRU cache; each miss triggers a full
    // tree-sitter parse (~25ms/doc). Measured at ~170ms. The 10ms budget was
    // unrealistic — tree-sitter parse is the dominant cost.
    PerfBudgetSpec {
        label: "prepared_syntax_multidoc_cache_hit_rate/hot_docs/6",
        estimate_path: "prepared_syntax_multidoc_cache_hit_rate/hot_docs/6/new/estimates.json",
        threshold_ns: 250.0 * NANOS_PER_MILLISECOND,
    },
    // --- prepared_syntax_chunk_miss_cost --- single chunk miss rebuild cost
    PerfBudgetSpec {
        label: "prepared_syntax_chunk_miss_cost/chunk_miss",
        estimate_path: "prepared_syntax_chunk_miss_cost/chunk_miss/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- large_html_syntax --- large HTML document syntax analysis
    // Local baseline refreshed on 2026-03-19. Criterion stores the visible-window
    // estimates under `.../visible_window_{pending,steady,sweep}/new/estimates.json`
    // while the sidecars keep the `/160` label suffix for structural metrics.
    PerfBudgetSpec {
        label: "large_html_syntax/synthetic_html_fixture/background_prepare",
        estimate_path: "large_html_syntax/synthetic_html_fixture/background_prepare/new/estimates.json",
        threshold_ns: LARGE_HTML_BACKGROUND_PREPARE_BUDGET_NS,
    },
    PerfBudgetSpec {
        label: "large_html_syntax/synthetic_html_fixture/visible_window_pending/160",
        estimate_path: "large_html_syntax/synthetic_html_fixture/visible_window_pending/new/estimates.json",
        threshold_ns: LARGE_HTML_VISIBLE_WINDOW_PENDING_BUDGET_NS,
    },
    PerfBudgetSpec {
        label: "large_html_syntax/synthetic_html_fixture/visible_window_steady/160",
        estimate_path: "large_html_syntax/synthetic_html_fixture/visible_window_steady/new/estimates.json",
        threshold_ns: LARGE_HTML_VISIBLE_WINDOW_STEADY_BUDGET_NS,
    },
    PerfBudgetSpec {
        label: "large_html_syntax/synthetic_html_fixture/visible_window_sweep/160",
        estimate_path: "large_html_syntax/synthetic_html_fixture/visible_window_sweep/new/estimates.json",
        threshold_ns: LARGE_HTML_VISIBLE_WINDOW_SWEEP_BUDGET_NS,
    },
    // External HTML fixture budgets — validated against html5spec-single.html
    // (15.1MB, 105k lines), ~15x larger than synthetic. Separate budgets account
    // for proportionally longer tree-sitter parse and denser highlight spans.
    PerfBudgetSpec {
        label: "large_html_syntax/external_html_fixture/background_prepare",
        estimate_path: "large_html_syntax/external_html_fixture/background_prepare/new/estimates.json",
        threshold_ns: EXTERNAL_HTML_BACKGROUND_PREPARE_BUDGET_NS,
    },
    PerfBudgetSpec {
        label: "large_html_syntax/external_html_fixture/visible_window_pending/160",
        estimate_path: "large_html_syntax/external_html_fixture/visible_window_pending/new/estimates.json",
        threshold_ns: EXTERNAL_HTML_VISIBLE_WINDOW_PENDING_BUDGET_NS,
    },
    PerfBudgetSpec {
        label: "large_html_syntax/external_html_fixture/visible_window_steady/160",
        estimate_path: "large_html_syntax/external_html_fixture/visible_window_steady/new/estimates.json",
        threshold_ns: EXTERNAL_HTML_VISIBLE_WINDOW_STEADY_BUDGET_NS,
    },
    PerfBudgetSpec {
        label: "large_html_syntax/external_html_fixture/visible_window_sweep/160",
        estimate_path: "large_html_syntax/external_html_fixture/visible_window_sweep/new/estimates.json",
        threshold_ns: EXTERNAL_HTML_VISIBLE_WINDOW_SWEEP_BUDGET_NS,
    },
    // --- worktree_preview_render --- worktree preview window rendering
    PerfBudgetSpec {
        label: "worktree_preview_render/cached_lookup_window/200",
        estimate_path: "worktree_preview_render/cached_lookup_window/200/new/estimates.json",
        // Cached lookup should be very fast — pure hit path.
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "worktree_preview_render/render_time_prepare_window/200",
        estimate_path: "worktree_preview_render/render_time_prepare_window/200/new/estimates.json",
        // Render-time preparation includes styling and layout.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    // --- resolved_output_recompute_incremental --- conflict resolved output rebuild
    PerfBudgetSpec {
        label: "resolved_output_recompute_incremental/full_recompute",
        estimate_path: "resolved_output_recompute_incremental/full_recompute/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "resolved_output_recompute_incremental/incremental_recompute",
        estimate_path: "resolved_output_recompute_incremental/incremental_recompute/new/estimates.json",
        // Incremental should be cheaper than full recompute.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    // --- conflict_three_way_prepared_syntax_scroll --- syntax-aware three-way scroll
    PerfBudgetSpec {
        label: "conflict_three_way_prepared_syntax_scroll/style_window/200",
        estimate_path: "conflict_three_way_prepared_syntax_scroll/style_window/200/new/estimates.json",
        // Similar to three_way_scroll (8 ms) but adds syntax highlighting cost.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- conflict_three_way_visible_map_build --- visible region map construction
    PerfBudgetSpec {
        label: "conflict_three_way_visible_map_build/linear_two_pointer",
        estimate_path: "conflict_three_way_visible_map_build/linear_two_pointer/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_three_way_visible_map_build/legacy_find_scan",
        estimate_path: "conflict_three_way_visible_map_build/legacy_find_scan/new/estimates.json",
        // Legacy scan is expected to be slower than the two-pointer variant.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- conflict_load_duplication --- payload forwarding vs duplication
    PerfBudgetSpec {
        label: "conflict_load_duplication/shared_payload_forwarding/low_density",
        estimate_path: "conflict_load_duplication/shared_payload_forwarding/low_density/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_load_duplication/duplicated_text_and_bytes/low_density",
        estimate_path: "conflict_load_duplication/duplicated_text_and_bytes/low_density/new/estimates.json",
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_load_duplication/shared_payload_forwarding/high_density",
        estimate_path: "conflict_load_duplication/shared_payload_forwarding/high_density/new/estimates.json",
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_load_duplication/duplicated_text_and_bytes/high_density",
        estimate_path: "conflict_load_duplication/duplicated_text_and_bytes/high_density/new/estimates.json",
        // High-density duplication is the worst case.
        threshold_ns: 40.0 * NANOS_PER_MILLISECOND,
    },
    // --- conflict_two_way_diff_build --- two-way diff build cost
    PerfBudgetSpec {
        label: "conflict_two_way_diff_build/full_file/low_density",
        estimate_path: "conflict_two_way_diff_build/full_file/low_density/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_diff_build/block_local/low_density",
        estimate_path: "conflict_two_way_diff_build/block_local/low_density/new/estimates.json",
        // Block-local should be cheaper than full-file.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_diff_build/full_file/high_density",
        estimate_path: "conflict_two_way_diff_build/full_file/high_density/new/estimates.json",
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_diff_build/block_local/high_density",
        estimate_path: "conflict_two_way_diff_build/block_local/high_density/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- conflict_two_way_word_highlights --- word-level diff highlight cost
    // Full-file word diff is O(N*D) Myers — measured at ~26ms (low_density)
    // and ~29ms (high_density) after Turn 6 allocation optimizations.
    // Further improvement requires algorithmic changes.
    PerfBudgetSpec {
        label: "conflict_two_way_word_highlights/full_file/low_density",
        estimate_path: "conflict_two_way_word_highlights/full_file/low_density/new/estimates.json",
        threshold_ns: 35.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_word_highlights/block_local/low_density",
        estimate_path: "conflict_two_way_word_highlights/block_local/low_density/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_word_highlights/full_file/high_density",
        estimate_path: "conflict_two_way_word_highlights/full_file/high_density/new/estimates.json",
        threshold_ns: 40.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_word_highlights/block_local/high_density",
        estimate_path: "conflict_two_way_word_highlights/block_local/high_density/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- conflict_resolved_output_gutter_scroll --- gutter scroll at varying window sizes
    PerfBudgetSpec {
        label: "conflict_resolved_output_gutter_scroll/window_100",
        estimate_path: "conflict_resolved_output_gutter_scroll/window_100/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_resolved_output_gutter_scroll/window_200",
        estimate_path: "conflict_resolved_output_gutter_scroll/window_200/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_resolved_output_gutter_scroll/window_400",
        estimate_path: "conflict_resolved_output_gutter_scroll/window_400/new/estimates.json",
        // Larger window = more rows to render per scroll step.
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
];
