use super::super::*;

pub(crate) const PERF_BUDGETS: &[PerfBudgetSpec] = &[
    // --- history_graph --- graph computation budgets
    PerfBudgetSpec {
        label: "history_graph/linear_history",
        estimate_path: "history_graph/linear_history/new/estimates.json",
        threshold_ns: 1_500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_graph/merge_dense",
        estimate_path: "history_graph/merge_dense/new/estimates.json",
        threshold_ns: 1_500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_graph/branch_heads_dense",
        estimate_path: "history_graph/branch_heads_dense/new/estimates.json",
        threshold_ns: 1_500.0 * NANOS_PER_MILLISECOND,
    },
    // --- commit_details --- file list row construction budgets
    PerfBudgetSpec {
        label: "commit_details/many_files",
        estimate_path: "commit_details/many_files/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "commit_details/deep_paths",
        estimate_path: "commit_details/deep_paths/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "commit_details/huge_file_list",
        estimate_path: "commit_details/huge_file_list/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "commit_details/large_message_body",
        estimate_path: "commit_details/large_message_body/new/estimates.json",
        threshold_ns: 30.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "commit_details/10k_files_depth_12",
        estimate_path: "commit_details/10k_files_depth_12/new/estimates.json",
        threshold_ns: 45.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "commit_details/select_commit_replace",
        estimate_path: "commit_details/select_commit_replace/new/estimates.json",
        // Replacement should be roughly 2x a single commit details render (two commits processed).
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "commit_details/path_display_cache_churn",
        estimate_path: "commit_details/path_display_cache_churn/new/estimates.json",
        // 10k unique paths with cache clears — allow more headroom than normal.
        threshold_ns: 30.0 * NANOS_PER_MILLISECOND,
    },
    // --- patch_diff_paged_rows --- paged vs eager diff row budgets
    PerfBudgetSpec {
        label: "patch_diff_paged_rows/eager_full_materialize",
        estimate_path: "patch_diff_paged_rows/eager_full_materialize/new/estimates.json",
        threshold_ns: 200.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "patch_diff_paged_rows/paged_first_window/200",
        estimate_path: "patch_diff_paged_rows/paged_first_window/200/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "patch_diff_paged_rows/inline_visible_eager_scan",
        estimate_path: "patch_diff_paged_rows/inline_visible_eager_scan/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "patch_diff_paged_rows/inline_visible_hidden_map",
        estimate_path: "patch_diff_paged_rows/inline_visible_hidden_map/new/estimates.json",
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_file_split/inline_first_window --- file diff first window
    PerfBudgetSpec {
        label: "diff_open_file_split_first_window/200",
        estimate_path: "diff_open_file_split_first_window/200/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "diff_open_file_inline_first_window/200",
        estimate_path: "diff_open_file_inline_first_window/200/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_patch_deep_window_90pct --- deep scroll first paint
    PerfBudgetSpec {
        label: "diff_open_patch_deep_window_90pct/200",
        estimate_path: "diff_open_patch_deep_window_90pct/200/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_markdown_preview_first_window --- markdown preview diff first paint
    PerfBudgetSpec {
        label: "diff_open_markdown_preview_first_window/200",
        estimate_path: "diff_open_markdown_preview_first_window/200/new/estimates.json",
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_image_preview_first_paint --- ready-image two-cell layout from cached previews
    PerfBudgetSpec {
        label: "diff_open_image_preview_first_paint",
        estimate_path: "diff_open_image_preview_first_paint/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_patch_100k_lines_first_window --- extreme large file first paint
    PerfBudgetSpec {
        label: "diff_open_patch_100k_lines_first_window/200",
        estimate_path: "diff_open_patch_100k_lines_first_window/200/new/estimates.json",
        threshold_ns: 30.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_conflict_compare_first_window --- conflict compare first paint
    PerfBudgetSpec {
        label: "diff_open_conflict_compare_first_window/200",
        estimate_path: "diff_open_conflict_compare_first_window/200/new/estimates.json",
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    // --- diff_open_svg_dual_path_first_window --- SVG rasterize + fallback dual path
    PerfBudgetSpec {
        label: "diff_open_svg_dual_path_first_window/200",
        estimate_path: "diff_open_svg_dual_path_first_window/200/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    // --- pane_resize_drag_step --- sidebar/details drag-step clamp math
    PerfBudgetSpec {
        label: "pane_resize_drag_step/sidebar",
        estimate_path: "pane_resize_drag_step/sidebar/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "pane_resize_drag_step/details",
        estimate_path: "pane_resize_drag_step/details/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    // --- diff_split_resize_drag_step --- diff split divider drag clamp math
    PerfBudgetSpec {
        label: "diff_split_resize_drag_step/window_200",
        estimate_path: "diff_split_resize_drag_step/window_200/new/estimates.json",
        // 200-step sweep; pure ratio arithmetic — should be well under 100 µs.
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    // --- window_resize_layout --- pane width recomputation during resize drag
    PerfBudgetSpec {
        label: "window_resize_layout/sidebar_main_details",
        estimate_path: "window_resize_layout/sidebar_main_details/new/estimates.json",
        // 200-step sweep; pure arithmetic — should be well under 100 µs.
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "window_resize_layout/history_50k_commits_diff_20k_lines",
        estimate_path: "window_resize_layout/history_50k_commits_diff_20k_lines/new/estimates.json",
        // Combined resize-layout + visible-window repaint on a 50k-commit
        // history cache and 20k-line split diff. Keep the budget generous
        // enough for shared-runner noise while still catching accidental
        // full-list work during resize.
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    // --- history_column_resize_drag_step --- column width clamping + visible column recomputation
    PerfBudgetSpec {
        label: "history_column_resize_drag_step/branch",
        estimate_path: "history_column_resize_drag_step/branch/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "history_column_resize_drag_step/graph",
        estimate_path: "history_column_resize_drag_step/graph/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "history_column_resize_drag_step/author",
        estimate_path: "history_column_resize_drag_step/author/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "history_column_resize_drag_step/date",
        estimate_path: "history_column_resize_drag_step/date/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "history_column_resize_drag_step/sha",
        estimate_path: "history_column_resize_drag_step/sha/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    // --- repo_tab_drag --- hit-test and reducer reorder
    PerfBudgetSpec {
        label: "repo_tab_drag/hit_test/20_tabs",
        estimate_path: "repo_tab_drag/hit_test/20_tabs/new/estimates.json",
        // Pure position arithmetic over 60 steps — sub-microsecond expected.
        threshold_ns: 50.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "repo_tab_drag/hit_test/200_tabs",
        estimate_path: "repo_tab_drag/hit_test/200_tabs/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "repo_tab_drag/reorder_reduce/20_tabs",
        estimate_path: "repo_tab_drag/reorder_reduce/20_tabs/new/estimates.json",
        // Reducer dispatch: Vec insert/remove × 40 steps.
        threshold_ns: 500.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "repo_tab_drag/reorder_reduce/200_tabs",
        estimate_path: "repo_tab_drag/reorder_reduce/200_tabs/new/estimates.json",
        // 200-tab reorder with Vec shifts — allow more headroom.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
];
