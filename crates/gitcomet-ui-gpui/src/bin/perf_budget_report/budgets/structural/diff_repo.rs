use super::super::*;

pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
    // The initial diff-open sidecar budgets pin the current deterministic work profile.
    // Later phases should tighten these once first-window work is reduced to visible-window scope.
    StructuralBudgetSpec {
        bench: "diff_open_patch_first_window/200",
        metric: "rows_materialized",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 20_500.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_patch_first_window/200",
        // rows_painted is the top-level container count (1); patch_rows_painted
        // is the actual visible window row count emitted by the paged split rows.
        metric: "patch_rows_painted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_patch_first_window/200",
        metric: "patch_page_cache_entries",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_patch_first_window/200",
        metric: "full_text_materializations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // File diff split/inline first window structural budgets.
    StructuralBudgetSpec {
        bench: "diff_open_file_split_first_window/200",
        metric: "split_rows_painted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_file_split_first_window/200",
        metric: "split_total_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_file_inline_first_window/200",
        metric: "inline_rows_painted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_file_inline_first_window/200",
        metric: "inline_total_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    // Markdown preview diff first window structural budgets.
    StructuralBudgetSpec {
        bench: "diff_open_markdown_preview_first_window/200",
        metric: "old_rows_rendered",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_markdown_preview_first_window/200",
        metric: "new_rows_rendered",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // Markdown preview single-document scroll structural budgets.
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/window_rows/200",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/window_rows/200",
        metric: "start_row",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/window_rows/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/window_rows/200",
        metric: "rows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/window_rows/200",
        metric: "scroll_step_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    // Rich markdown preview scroll structural budgets.
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "long_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "long_row_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2_000.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "start_row",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "rows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "scroll_step_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "heading_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "list_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "table_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "code_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "blockquote_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        metric: "details_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // Image preview first paint structural budgets.
    StructuralBudgetSpec {
        bench: "diff_open_image_preview_first_paint",
        metric: "old_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_image_preview_first_paint",
        metric: "new_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 393_216.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_image_preview_first_paint",
        metric: "total_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 655_360.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_image_preview_first_paint",
        metric: "images_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_image_preview_first_paint",
        metric: "placeholder_cells",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_image_preview_first_paint",
        metric: "divider_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // Patch diff 100k lines first window structural budgets.
    StructuralBudgetSpec {
        bench: "diff_open_patch_100k_lines_first_window/200",
        metric: "rows_painted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_patch_100k_lines_first_window/200",
        metric: "full_text_materializations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // Conflict compare first window structural budgets.
    StructuralBudgetSpec {
        bench: "diff_open_conflict_compare_first_window/200",
        metric: "rows_rendered",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_conflict_compare_first_window/200",
        metric: "conflict_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/balanced",
        metric: "commit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/balanced",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/history_heavy",
        metric: "commit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 15_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/history_heavy",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 15_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/branch_heavy",
        metric: "local_branches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_200.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/branch_heavy",
        metric: "remote_branches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_200.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/branch_heavy",
        metric: "sidebar_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_400.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/extreme_metadata_fanout",
        metric: "local_branches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/extreme_metadata_fanout",
        metric: "remote_branches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/extreme_metadata_fanout",
        metric: "worktrees",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/extreme_metadata_fanout",
        metric: "submodules",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "open_repo/extreme_metadata_fanout",
        metric: "sidebar_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        // Initial open keeps Worktrees and Submodules collapsed, so the visible
        // row count should cover branch fanout plus section headers rather than
        // every worktree/submodule entry.
        threshold: 11_400.0,
    },
    // history_cache_build/balanced — visible commits should be close to input count
    // (only stash helpers are filtered out; with 20 stashes the delta is small).
    StructuralBudgetSpec {
        bench: "history_cache_build/balanced",
        metric: "visible_commits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_900.0,
    },
    StructuralBudgetSpec {
        bench: "history_cache_build/balanced",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_900.0,
    },
    // history_cache_build/stash_heavy — must actually filter stash helpers
];
