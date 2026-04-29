use criterion::{Criterion, criterion_group, criterion_main};
use gitcomet_ui_gpui::perf_alloc::{PerfTrackingAllocator, TRACKING_MIMALLOC};

#[global_allocator]
static GLOBAL: &PerfTrackingAllocator = &TRACKING_MIMALLOC;

#[path = "performance/mod.rs"]
mod performance_benches;

fn register_benchmark_if_selected(
    c: &mut Criterion,
    selectors: &[&str],
    target: fn(&mut Criterion),
) {
    if performance_benches::benchmark_selectors_match_filter(selectors) {
        target(c);
    }
}

macro_rules! exact_filtered_target {
    ($wrapper:ident => $target:ident) => {
        fn $wrapper(c: &mut Criterion) {
            let selector = stringify!($target).trim_start_matches("bench_");
            register_benchmark_if_selected(c, &[selector], performance_benches::$target);
        }
    };
    ($wrapper:ident => $target:ident, [$($selector:expr),+ $(,)?]) => {
        fn $wrapper(c: &mut Criterion) {
            register_benchmark_if_selected(c, &[$($selector),+], performance_benches::$target);
        }
    };
}

exact_filtered_target!(bench_open_repo_selected => bench_open_repo);
exact_filtered_target!(bench_branch_sidebar_selected => bench_branch_sidebar, [
    "branch_sidebar/local_heavy",
    "branch_sidebar/remote_fanout",
    "branch_sidebar/aux_lists_heavy",
]);
exact_filtered_target!(
    bench_branch_sidebar_extreme_scale_selected => bench_branch_sidebar_extreme_scale,
    ["branch_sidebar/20k_branches_100_remotes",]
);
exact_filtered_target!(bench_branch_sidebar_cache_selected => bench_branch_sidebar_cache, [
    "branch_sidebar/cache_hit_balanced",
    "branch_sidebar/cache_miss_remote_fanout",
    "branch_sidebar/cache_invalidation_single_ref_change",
    "branch_sidebar/cache_invalidation_worktrees_ready",
]);
exact_filtered_target!(bench_history_graph_selected => bench_history_graph);
exact_filtered_target!(bench_history_cache_build_selected => bench_history_cache_build, [
    "history_cache_build/balanced",
    "history_cache_build/merge_dense",
    "history_cache_build/decorated_refs_heavy",
    "history_cache_build/stash_heavy",
]);
exact_filtered_target!(
    bench_history_cache_build_extreme_scale_selected => bench_history_cache_build_extreme_scale,
    ["history_cache_build/50k_commits_2k_refs_200_stashes",]
);
exact_filtered_target!(bench_history_load_more_append_selected => bench_history_load_more_append);
exact_filtered_target!(bench_history_scope_switch_selected => bench_history_scope_switch);
exact_filtered_target!(bench_repo_switch_selected => bench_repo_switch);
exact_filtered_target!(bench_commit_details_selected => bench_commit_details);
exact_filtered_target!(bench_status_list_selected => bench_status_list);
exact_filtered_target!(bench_status_truncation_selected => bench_status_truncation, [
    "status_truncation/path_aligned_long_paths",
    "status_truncation/middle_long_paths",
    "status_truncation/end_long_paths",
    "status_truncation/focus_long_paths",
]);
exact_filtered_target!(bench_status_multi_select_selected => bench_status_multi_select);
exact_filtered_target!(bench_status_select_diff_open_selected => bench_status_select_diff_open);
exact_filtered_target!(bench_merge_open_bootstrap_selected => bench_merge_open_bootstrap);
exact_filtered_target!(bench_frame_timing_selected => bench_frame_timing);
exact_filtered_target!(bench_keyboard_selected => bench_keyboard);
exact_filtered_target!(bench_staging_selected => bench_staging);
exact_filtered_target!(bench_undo_redo_selected => bench_undo_redo);
exact_filtered_target!(bench_git_ops_selected => bench_git_ops);
exact_filtered_target!(
    bench_large_file_diff_scroll_selected => bench_large_file_diff_scroll,
    ["diff_scroll",]
);
exact_filtered_target!(
    bench_file_diff_replacement_alignment_selected => bench_file_diff_replacement_alignment
);
exact_filtered_target!(
    bench_text_input_prepaint_windowed_selected => bench_text_input_prepaint_windowed
);
exact_filtered_target!(
    bench_text_input_runs_streamed_highlight_selected => bench_text_input_runs_streamed_highlight,
    [
        "text_input_runs_streamed_highlight_dense",
        "text_input_runs_streamed_highlight_sparse",
    ]
);
exact_filtered_target!(bench_text_input_long_line_cap_selected => bench_text_input_long_line_cap);
exact_filtered_target!(
    bench_text_input_wrap_incremental_tabs_selected => bench_text_input_wrap_incremental_tabs
);
exact_filtered_target!(
    bench_text_input_wrap_incremental_burst_edits_selected => bench_text_input_wrap_incremental_burst_edits
);
exact_filtered_target!(
    bench_text_model_snapshot_clone_cost_selected => bench_text_model_snapshot_clone_cost
);
exact_filtered_target!(bench_text_model_bulk_load_large_selected => bench_text_model_bulk_load_large);
exact_filtered_target!(
    bench_text_model_fragmented_edits_selected => bench_text_model_fragmented_edits
);
exact_filtered_target!(bench_file_diff_syntax_prepare_selected => bench_file_diff_syntax_prepare);
exact_filtered_target!(
    bench_file_diff_syntax_query_stress_selected => bench_file_diff_syntax_query_stress
);
exact_filtered_target!(bench_file_diff_syntax_reparse_selected => bench_file_diff_syntax_reparse);
exact_filtered_target!(
    bench_file_diff_inline_syntax_projection_selected => bench_file_diff_inline_syntax_projection
);
exact_filtered_target!(
    bench_file_diff_syntax_cache_drop_selected => bench_file_diff_syntax_cache_drop
);
exact_filtered_target!(
    bench_prepared_syntax_multidoc_cache_hit_rate_selected => bench_prepared_syntax_multidoc_cache_hit_rate
);
exact_filtered_target!(
    bench_prepared_syntax_chunk_miss_cost_selected => bench_prepared_syntax_chunk_miss_cost
);
exact_filtered_target!(bench_large_html_syntax_selected => bench_large_html_syntax);
exact_filtered_target!(bench_worktree_preview_render_selected => bench_worktree_preview_render);
exact_filtered_target!(
    bench_markdown_preview_parse_build_selected => bench_markdown_preview_parse_build
);
exact_filtered_target!(bench_markdown_preview_render_selected => bench_markdown_preview_render, [
    "markdown_preview_render_single",
    "markdown_preview_render_diff",
]);
exact_filtered_target!(bench_markdown_preview_scroll_selected => bench_markdown_preview_scroll);
exact_filtered_target!(
    bench_diff_open_markdown_preview_first_window_selected => bench_diff_open_markdown_preview_first_window
);
exact_filtered_target!(
    bench_diff_open_image_preview_first_paint_selected => bench_diff_open_image_preview_first_paint,
    ["diff_open_image_preview_first_paint",]
);
exact_filtered_target!(
    bench_diff_open_svg_dual_path_first_window_selected => bench_diff_open_svg_dual_path_first_window
);
exact_filtered_target!(bench_conflict_three_way_scroll_selected => bench_conflict_three_way_scroll);
exact_filtered_target!(
    bench_conflict_three_way_prepared_syntax_scroll_selected => bench_conflict_three_way_prepared_syntax_scroll
);
exact_filtered_target!(
    bench_conflict_three_way_visible_map_build_selected => bench_conflict_three_way_visible_map_build
);
exact_filtered_target!(
    bench_conflict_two_way_split_scroll_selected => bench_conflict_two_way_split_scroll
);
exact_filtered_target!(bench_conflict_load_duplication_selected => bench_conflict_load_duplication);
exact_filtered_target!(
    bench_conflict_two_way_diff_build_selected => bench_conflict_two_way_diff_build
);
exact_filtered_target!(
    bench_conflict_two_way_word_highlights_selected => bench_conflict_two_way_word_highlights
);
exact_filtered_target!(
    bench_conflict_resolved_output_gutter_scroll_selected => bench_conflict_resolved_output_gutter_scroll
);
exact_filtered_target!(
    bench_conflict_search_query_update_selected => bench_conflict_search_query_update
);
exact_filtered_target!(
    bench_patch_diff_search_query_update_selected => bench_patch_diff_search_query_update
);
exact_filtered_target!(bench_patch_diff_paged_rows_selected => bench_patch_diff_paged_rows);
exact_filtered_target!(
    bench_diff_open_patch_first_window_selected => bench_diff_open_patch_first_window
);
exact_filtered_target!(
    bench_diff_open_file_split_first_window_selected => bench_diff_open_file_split_first_window
);
exact_filtered_target!(
    bench_diff_open_file_inline_first_window_selected => bench_diff_open_file_inline_first_window
);
exact_filtered_target!(
    bench_diff_open_patch_deep_window_selected => bench_diff_open_patch_deep_window,
    ["diff_open_patch_deep_window_90pct",]
);
exact_filtered_target!(
    bench_diff_open_patch_100k_lines_first_window_selected => bench_diff_open_patch_100k_lines_first_window
);
exact_filtered_target!(
    bench_diff_open_conflict_compare_first_window_selected => bench_diff_open_conflict_compare_first_window
);
exact_filtered_target!(
    bench_diff_refresh_rev_only_same_content_selected => bench_diff_refresh_rev_only_same_content
);
exact_filtered_target!(
    bench_conflict_split_resize_step_selected => bench_conflict_split_resize_step
);
exact_filtered_target!(
    bench_conflict_streamed_provider_selected => bench_conflict_streamed_provider
);
exact_filtered_target!(
    bench_conflict_streamed_resolved_output_selected => bench_conflict_streamed_resolved_output
);
exact_filtered_target!(bench_pane_resize_drag_step_selected => bench_pane_resize_drag_step);
exact_filtered_target!(
    bench_diff_split_resize_drag_step_selected => bench_diff_split_resize_drag_step
);
exact_filtered_target!(bench_window_resize_layout_selected => bench_window_resize_layout, [
    "window_resize_layout/sidebar_main_details",
]);
exact_filtered_target!(
    bench_window_resize_layout_extreme_scale_selected => bench_window_resize_layout_extreme_scale,
    ["window_resize_layout/history_50k_commits_diff_20k_lines",]
);
exact_filtered_target!(
    bench_history_column_resize_drag_step_selected => bench_history_column_resize_drag_step
);
exact_filtered_target!(bench_repo_tab_drag_selected => bench_repo_tab_drag);
exact_filtered_target!(
    bench_resolved_output_recompute_incremental_selected => bench_resolved_output_recompute_incremental
);
exact_filtered_target!(bench_scrollbar_drag_step_selected => bench_scrollbar_drag_step);
exact_filtered_target!(bench_search_selected => bench_search);
exact_filtered_target!(bench_fs_event_selected => bench_fs_event);
exact_filtered_target!(bench_network_selected => bench_network);
exact_filtered_target!(bench_clipboard_selected => bench_clipboard);
exact_filtered_target!(bench_display_selected => bench_display);
exact_filtered_target!(bench_real_repo_selected => bench_real_repo);

criterion_group! {
    name = benches;
    config = performance_benches::benchmark_criterion();
    targets =
        bench_open_repo_selected,
        bench_branch_sidebar_selected,
        bench_branch_sidebar_extreme_scale_selected,
        bench_branch_sidebar_cache_selected,
        bench_history_graph_selected,
        bench_history_cache_build_selected,
        bench_history_cache_build_extreme_scale_selected,
        bench_history_load_more_append_selected,
        bench_history_scope_switch_selected,
        bench_repo_switch_selected,
        bench_commit_details_selected,
        bench_status_list_selected,
        bench_status_truncation_selected,
        bench_status_multi_select_selected,
        bench_status_select_diff_open_selected,
        bench_merge_open_bootstrap_selected,
        bench_frame_timing_selected,
        bench_keyboard_selected,
        bench_staging_selected,
        bench_undo_redo_selected,
        bench_git_ops_selected,
        bench_large_file_diff_scroll_selected,
        bench_file_diff_replacement_alignment_selected,
        bench_text_input_prepaint_windowed_selected,
        bench_text_input_runs_streamed_highlight_selected,
        bench_text_input_long_line_cap_selected,
        bench_text_input_wrap_incremental_tabs_selected,
        bench_text_input_wrap_incremental_burst_edits_selected,
        bench_text_model_snapshot_clone_cost_selected,
        bench_text_model_bulk_load_large_selected,
        bench_text_model_fragmented_edits_selected,
        bench_file_diff_syntax_prepare_selected,
        bench_file_diff_syntax_query_stress_selected,
        bench_file_diff_syntax_reparse_selected,
        bench_file_diff_inline_syntax_projection_selected,
        bench_file_diff_syntax_cache_drop_selected,
        bench_prepared_syntax_multidoc_cache_hit_rate_selected,
        bench_prepared_syntax_chunk_miss_cost_selected,
        bench_large_html_syntax_selected,
        bench_worktree_preview_render_selected,
        bench_markdown_preview_parse_build_selected,
        bench_markdown_preview_render_selected,
        bench_markdown_preview_scroll_selected,
        bench_diff_open_markdown_preview_first_window_selected,
        bench_diff_open_image_preview_first_paint_selected,
        bench_diff_open_svg_dual_path_first_window_selected,
        bench_conflict_three_way_scroll_selected,
        bench_conflict_three_way_prepared_syntax_scroll_selected,
        bench_conflict_three_way_visible_map_build_selected,
        bench_conflict_two_way_split_scroll_selected,
        bench_conflict_load_duplication_selected,
        bench_conflict_two_way_diff_build_selected,
        bench_conflict_two_way_word_highlights_selected,
        bench_conflict_resolved_output_gutter_scroll_selected,
        bench_conflict_search_query_update_selected,
        bench_patch_diff_search_query_update_selected,
        bench_patch_diff_paged_rows_selected,
        bench_diff_open_patch_first_window_selected,
        bench_diff_open_file_split_first_window_selected,
        bench_diff_open_file_inline_first_window_selected,
        bench_diff_open_patch_deep_window_selected,
        bench_diff_open_patch_100k_lines_first_window_selected,
        bench_diff_open_conflict_compare_first_window_selected,
        bench_diff_refresh_rev_only_same_content_selected,
        bench_conflict_split_resize_step_selected,
        bench_conflict_streamed_provider_selected,
        bench_conflict_streamed_resolved_output_selected,
        bench_pane_resize_drag_step_selected,
        bench_diff_split_resize_drag_step_selected,
        bench_window_resize_layout_selected,
        bench_window_resize_layout_extreme_scale_selected,
        bench_history_column_resize_drag_step_selected,
        bench_repo_tab_drag_selected,
        bench_resolved_output_recompute_incremental_selected,
        bench_scrollbar_drag_step_selected,
        bench_search_selected,
        bench_fs_event_selected,
        bench_network_selected,
        bench_clipboard_selected,
        bench_display_selected,
        bench_real_repo_selected,
}
criterion_main!(benches);
