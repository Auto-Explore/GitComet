use super::super::*;

pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
    // --- status_select_diff_open --- reducer dispatch metrics
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "load_selected_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "load_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "load_diff_file_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "diff_state_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "load_selected_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "load_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "load_diff_file_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "diff_state_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "trace_event_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 7.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "rendering_mode_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "full_output_generated",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "full_syntax_parse_requested",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "whole_block_diff_ran",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "inline_row_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "diff_row_count",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 16.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "conflict_block_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/large_streamed",
        metric: "resolved_output_line_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50_000.0,
    },
    // merge_open_bootstrap/many_conflicts — 50 conflict blocks, moderate file
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/many_conflicts",
        metric: "trace_event_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 7.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/many_conflicts",
        metric: "rendering_mode_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/many_conflicts",
        metric: "full_output_generated",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/many_conflicts",
        metric: "conflict_block_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/many_conflicts",
        metric: "inline_row_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/many_conflicts",
        metric: "whole_block_diff_ran",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // merge_open_bootstrap/50k_lines_500_conflicts_streamed — extreme scale
    // With many conflict blocks, inner functions may emit additional trace events
    // beyond the 7 bootstrap stages.
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "trace_event_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 7.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "rendering_mode_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "full_output_generated",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "conflict_block_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "inline_row_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "resolved_output_line_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        metric: "whole_block_diff_ran",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // diff_refresh_rev_only_same_content/rekey — same-content refresh must rekey, not rebuild
    StructuralBudgetSpec {
        bench: "diff_refresh_rev_only_same_content/rekey",
        metric: "diff_cache_rekeys",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_refresh_rev_only_same_content/rekey",
        metric: "full_rebuilds",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_refresh_rev_only_same_content/rekey",
        metric: "content_signature_matches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // diff_refresh_rev_only_same_content/rebuild — full rebuild must report rebuild count
    StructuralBudgetSpec {
        bench: "diff_refresh_rev_only_same_content/rebuild",
        metric: "full_rebuilds",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_refresh_rev_only_same_content/rebuild",
        metric: "diff_cache_rekeys",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- history_graph structural budgets ---
    // Graph row count should equal commit count for all cases.
    StructuralBudgetSpec {
        bench: "history_graph/linear_history",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "history_graph/linear_history",
        metric: "merge_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "history_graph/merge_dense",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    // merge_dense should have a significant number of merges
    StructuralBudgetSpec {
        bench: "history_graph/merge_dense",
        metric: "merge_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "history_graph/branch_heads_dense",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    // branch_heads_dense should have branch heads decorating the graph
    StructuralBudgetSpec {
        bench: "history_graph/branch_heads_dense",
        metric: "branch_heads",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100.0,
    },
    // --- commit_details structural budgets ---
    StructuralBudgetSpec {
        bench: "commit_details/many_files",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/many_files",
        metric: "max_path_depth",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/deep_paths",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    // deep_paths should have significantly deeper paths than many_files
    StructuralBudgetSpec {
        bench: "commit_details/deep_paths",
        metric: "max_path_depth",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/huge_file_list",
        metric: "file_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/large_message_body",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/large_message_body",
        metric: "message_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 96_000.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/large_message_body",
        metric: "message_shaped_lines",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 48.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/10k_files_depth_12",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/10k_files_depth_12",
        metric: "max_path_depth",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 13.0,
    },
    // --- commit_details/select_commit_replace structural budgets ---
    StructuralBudgetSpec {
        bench: "commit_details/select_commit_replace",
        metric: "commit_ids_differ",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0, // true — the two commits must have different IDs
    },
    StructuralBudgetSpec {
        bench: "commit_details/select_commit_replace",
        metric: "files_a",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "commit_details/select_commit_replace",
        metric: "files_b",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 5_000.0,
    },
    // --- commit_details/path_display_cache_churn structural budgets ---
    StructuralBudgetSpec {
        bench: "commit_details/path_display_cache_churn",
        metric: "file_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 10_000.0,
    },
    // With 10k unique paths and an 8192-entry cache, at least 1 clear must occur.
    StructuralBudgetSpec {
        bench: "commit_details/path_display_cache_churn",
        metric: "path_display_cache_clears",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // All paths should be cache misses (no hits on first pass with unique paths).
    StructuralBudgetSpec {
        bench: "commit_details/path_display_cache_churn",
        metric: "path_display_cache_misses",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 10_000.0,
    },
    // --- pane_resize_drag_step structural budgets ---
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/sidebar",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/sidebar",
        metric: "width_bounds_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/sidebar",
        metric: "layout_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/sidebar",
        metric: "clamp_at_min_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/sidebar",
        metric: "clamp_at_max_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/details",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/details",
        metric: "width_bounds_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/details",
        metric: "layout_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/details",
        metric: "clamp_at_min_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "pane_resize_drag_step/details",
        metric: "clamp_at_max_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // --- diff_split_resize_drag_step structural budgets ---
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "ratio_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "column_width_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    // The oscillation must hit both column-min boundaries.
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "clamp_at_min_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "clamp_at_max_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // With a 564 px main pane, the window is wide enough — no narrow fallbacks.
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "narrow_fallback_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // The ratio must sweep between the min and max column boundaries.
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "min_ratio",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 0.35,
    },
    StructuralBudgetSpec {
        bench: "diff_split_resize_drag_step/window_200",
        metric: "max_ratio",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.65,
    },
    // --- window_resize_layout structural budgets ---
    StructuralBudgetSpec {
        bench: "window_resize_layout/sidebar_main_details",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/sidebar_main_details",
        metric: "layout_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    // The sidebar_main_details sweep (800→1800 px, sidebar=280+details=420=700)
    // never drives the main pane to zero — minimum main width is ~84 px.
    StructuralBudgetSpec {
        bench: "window_resize_layout/sidebar_main_details",
        metric: "clamp_at_zero_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "layout_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "history_visibility_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "diff_width_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "history_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "history_rows_processed_total",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12_800.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "history_columns_hidden_steps",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "history_all_columns_visible_steps",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "diff_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "diff_rows_processed_total",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "window_resize_layout/history_50k_commits_diff_20k_lines",
        metric: "diff_narrow_fallback_steps",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // --- history_column_resize_drag_step structural budgets ---
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/branch",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/branch",
        metric: "width_clamp_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/branch",
        metric: "visible_column_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/branch",
        metric: "clamp_at_max_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/graph",
        metric: "width_clamp_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/graph",
        metric: "visible_column_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/author",
        metric: "width_clamp_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/author",
        metric: "visible_column_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/date",
        metric: "width_clamp_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/date",
        metric: "visible_column_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/sha",
        metric: "width_clamp_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_column_resize_drag_step/sha",
        metric: "visible_column_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    // --- repo_tab_drag structural budgets ---
    StructuralBudgetSpec {
        bench: "repo_tab_drag/hit_test/20_tabs",
        metric: "tab_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/hit_test/20_tabs",
        metric: "hit_test_steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 60.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/hit_test/200_tabs",
        metric: "tab_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/hit_test/200_tabs",
        metric: "hit_test_steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 600.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/reorder_reduce/20_tabs",
        metric: "tab_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/reorder_reduce/20_tabs",
        metric: "reorder_steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/reorder_reduce/200_tabs",
        metric: "tab_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "repo_tab_drag/reorder_reduce/200_tabs",
        metric: "reorder_steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 400.0,
    },
    // 200-tab reorder should produce at least some effects (PersistSession).
    StructuralBudgetSpec {
        bench: "repo_tab_drag/reorder_reduce/200_tabs",
        metric: "effects_emitted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // --- frame_timing structural budgets ---
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_history_list",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_history_list",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_history_list",
        metric: "window_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_history_list",
        metric: "scroll_step_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_history_list",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_history_list",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_large_diff",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_large_diff",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_large_diff",
        metric: "window_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_large_diff",
        metric: "scroll_step_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_large_diff",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/continuous_scroll_large_diff",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- frame_timing/sidebar_resize_drag_sustained structural budgets ---
    StructuralBudgetSpec {
        bench: "frame_timing/sidebar_resize_drag_sustained",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/sidebar_resize_drag_sustained",
        metric: "frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/sidebar_resize_drag_sustained",
        metric: "steps_per_frame",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/sidebar_resize_drag_sustained",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/sidebar_resize_drag_sustained",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- frame_timing/rapid_commit_selection_changes structural budgets ---
    StructuralBudgetSpec {
        bench: "frame_timing/rapid_commit_selection_changes",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/rapid_commit_selection_changes",
        metric: "commit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/rapid_commit_selection_changes",
        metric: "files_per_commit",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/rapid_commit_selection_changes",
        metric: "selections",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/rapid_commit_selection_changes",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/rapid_commit_selection_changes",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- frame_timing/repo_switch_during_scroll structural budgets ---
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "total_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "scroll_frames",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "switch_frames",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "window_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "frame_timing/repo_switch_during_scroll",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
];
