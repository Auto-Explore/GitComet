use gitcomet_ui_gpui::perf_sidecar::{PerfSidecarReport, criterion_sidecar_path, read_sidecar};
use serde::Deserialize;
use std::env;
use std::fmt::Write as _;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const NANOS_PER_MICROSECOND: f64 = 1_000.0;
const NANOS_PER_MILLISECOND: f64 = 1_000_000.0;
const SIDECAR_TIMING_MS_PREFIX: &str = "@sidecar_ms:";
const REQUIRED_APP_LAUNCH_ALLOCATION_METRICS: &[&str] = &[
    "first_paint_alloc_ops",
    "first_paint_alloc_bytes",
    "first_interactive_alloc_ops",
    "first_interactive_alloc_bytes",
];
const LARGE_HTML_BACKGROUND_PREPARE_BUDGET_NS: f64 = 225.0 * NANOS_PER_MILLISECOND;
const LARGE_HTML_VISIBLE_WINDOW_PENDING_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;
const LARGE_HTML_VISIBLE_WINDOW_STEADY_BUDGET_NS: f64 = 125.0 * NANOS_PER_MICROSECOND;
const LARGE_HTML_VISIBLE_WINDOW_SWEEP_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;
// External HTML fixture budgets — html5spec-single.html is 15.1MB / 105k lines,
// ~15x larger than the 20k-line synthetic fixture; tree-sitter parse and per-window
// highlight span counts scale proportionally.
const EXTERNAL_HTML_BACKGROUND_PREPARE_BUDGET_NS: f64 = 1500.0 * NANOS_PER_MILLISECOND;
const EXTERNAL_HTML_VISIBLE_WINDOW_PENDING_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;
const EXTERNAL_HTML_VISIBLE_WINDOW_STEADY_BUDGET_NS: f64 = 750.0 * NANOS_PER_MICROSECOND;
const EXTERNAL_HTML_VISIBLE_WINDOW_SWEEP_BUDGET_NS: f64 = 150.0 * NANOS_PER_MICROSECOND;

#[derive(Clone, Copy, Debug)]
struct PerfBudgetSpec {
    label: &'static str,
    estimate_path: &'static str,
    threshold_ns: f64,
}

const PERF_BUDGETS: &[PerfBudgetSpec] = &[
    PerfBudgetSpec {
        label: "conflict_three_way_scroll/style_window/200",
        estimate_path: "conflict_three_way_scroll/style_window/200/new/estimates.json",
        threshold_ns: 8.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_two_way_split_scroll/window_200",
        estimate_path: "conflict_two_way_split_scroll/window_200/new/estimates.json",
        threshold_ns: 6.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_search_query_update/window/200",
        estimate_path: "conflict_search_query_update/window/200/new/estimates.json",
        threshold_ns: 40.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_split_resize_step/window/200",
        estimate_path: "conflict_split_resize_step/window/200/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_provider/index_build",
        estimate_path: "conflict_streamed_provider/index_build/new/estimates.json",
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_provider/first_page/200",
        estimate_path: "conflict_streamed_provider/first_page/200/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_provider/first_page_cache_hit/200",
        estimate_path: "conflict_streamed_provider/first_page_cache_hit/200/new/estimates.json",
        threshold_ns: 30.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_provider/deep_scroll_90pct/200",
        estimate_path: "conflict_streamed_provider/deep_scroll_90pct/200/new/estimates.json",
        threshold_ns: 120.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_provider/search_rare_text",
        estimate_path: "conflict_streamed_provider/search_rare_text/new/estimates.json",
        threshold_ns: 200.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_resolved_output/projection_build",
        estimate_path: "conflict_streamed_resolved_output/projection_build/new/estimates.json",
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_resolved_output/window/200",
        estimate_path: "conflict_streamed_resolved_output/window/200/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "conflict_streamed_resolved_output/deep_window_90pct/200",
        estimate_path: "conflict_streamed_resolved_output/deep_window_90pct/200/new/estimates.json",
        threshold_ns: 25.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_parse_build/single_document/medium",
        estimate_path: "markdown_preview_parse_build/single_document/medium/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_parse_build/two_sided_diff/medium",
        estimate_path: "markdown_preview_parse_build/two_sided_diff/medium/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    // Turn 26 flattened the markdown element tree (−20%), bringing render_single
    // to ~1.02ms. Remaining cost is GPUI element construction — 200 rows with
    // ~15 property setters each. Budget allows marginal variance.
    PerfBudgetSpec {
        label: "markdown_preview_render_single/window_rows/200",
        estimate_path: "markdown_preview_render_single/window_rows/200/new/estimates.json",
        threshold_ns: 1.5 * NANOS_PER_MILLISECOND,
    },
    // render_diff builds 400 rows (2 × 200 window) through the same GPUI
    // element path; measured at ~2.09ms after Turn 26 element tree flattening.
    PerfBudgetSpec {
        label: "markdown_preview_render_diff/window_rows/200",
        estimate_path: "markdown_preview_render_diff/window_rows/200/new/estimates.json",
        threshold_ns: 2.5 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "open_repo/balanced",
        estimate_path: "open_repo/balanced/new/estimates.json",
        threshold_ns: 650.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "open_repo/history_heavy",
        estimate_path: "open_repo/history_heavy/new/estimates.json",
        threshold_ns: 7_500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "open_repo/branch_heavy",
        estimate_path: "open_repo/branch_heavy/new/estimates.json",
        threshold_ns: 30.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "open_repo/extreme_metadata_fanout",
        estimate_path: "open_repo/extreme_metadata_fanout/new/estimates.json",
        // Extreme sidebar fanout: 1k local branches, 10k remote branches,
        // 5k worktrees, and 1k submodules on a 1k-commit repo-open path.
        // Local baseline is ~2.73 ms; keep healthy CI headroom.
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_cache_build/balanced",
        estimate_path: "history_cache_build/balanced/new/estimates.json",
        threshold_ns: 1_200.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_cache_build/merge_dense",
        estimate_path: "history_cache_build/merge_dense/new/estimates.json",
        threshold_ns: 1_200.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_cache_build/decorated_refs_heavy",
        estimate_path: "history_cache_build/decorated_refs_heavy/new/estimates.json",
        threshold_ns: 1_200.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "history_cache_build/stash_heavy",
        estimate_path: "history_cache_build/stash_heavy/new/estimates.json",
        threshold_ns: 1_000.0 * NANOS_PER_MILLISECOND,
    },
    // history_cache_build/50k_commits_2k_refs_200_stashes — extreme-scale
    // stress case with 50k commits, 2k refs, and stash filtering enabled.
    PerfBudgetSpec {
        label: "history_cache_build/50k_commits_2k_refs_200_stashes",
        estimate_path: "history_cache_build/50k_commits_2k_refs_200_stashes/new/estimates.json",
        threshold_ns: 15_000.0 * NANOS_PER_MILLISECOND,
    },
    // history_load_more_append/page_500 — reducer append of a 500-commit page
    // into an already-loaded history page. Measured around 13 µs; keep ample
    // headroom for shared-runner noise while still catching regressions quickly.
    PerfBudgetSpec {
        label: "history_load_more_append/page_500",
        estimate_path: "history_load_more_append/page_500/new/estimates.json",
        threshold_ns: 250.0 * NANOS_PER_MICROSECOND,
    },
    // history_scope_switch/current_branch_to_all_refs — scope change dispatches
    // set_log_scope, transitions log to Loading, emits LoadLog effect, and
    // persists session. Measured around a few µs; generous budget for shared-runner.
    PerfBudgetSpec {
        label: "history_scope_switch/current_branch_to_all_refs",
        estimate_path: "history_scope_switch/current_branch_to_all_refs/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MICROSECOND,
    },
    // branch_sidebar/cache_hit_balanced — fingerprint check + Arc::clone should be sub-microsecond
    PerfBudgetSpec {
        label: "branch_sidebar/cache_hit_balanced",
        estimate_path: "branch_sidebar/cache_hit_balanced/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MICROSECOND,
    },
    // branch_sidebar/cache_miss_remote_fanout — full rebuild with heavy remote fanout
    PerfBudgetSpec {
        label: "branch_sidebar/cache_miss_remote_fanout",
        estimate_path: "branch_sidebar/cache_miss_remote_fanout/new/estimates.json",
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    // branch_sidebar/cache_invalidation_single_ref_change — single rev bump + rebuild
    PerfBudgetSpec {
        label: "branch_sidebar/cache_invalidation_single_ref_change",
        estimate_path: "branch_sidebar/cache_invalidation_single_ref_change/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // branch_sidebar/cache_invalidation_worktrees_ready — worktrees_rev bump + rebuild
    // with worktrees/submodules/stashes present in the sidebar shape.
    PerfBudgetSpec {
        label: "branch_sidebar/cache_invalidation_worktrees_ready",
        estimate_path: "branch_sidebar/cache_invalidation_worktrees_ready/new/estimates.json",
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    // branch_sidebar/20k_branches_100_remotes — cold extreme-scale sidebar row build
    // with 20k remote branches spread across 100 remotes.
    PerfBudgetSpec {
        label: "branch_sidebar/20k_branches_100_remotes",
        estimate_path: "branch_sidebar/20k_branches_100_remotes/new/estimates.json",
        threshold_ns: 250.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "repo_switch/refocus_same_repo",
        estimate_path: "repo_switch/refocus_same_repo/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "repo_switch/two_hot_repos",
        estimate_path: "repo_switch/two_hot_repos/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    // repo_switch/selected_commit_and_details — changed-repo switch with
    // commit details already active, but without a selected diff reload.
    PerfBudgetSpec {
        label: "repo_switch/selected_commit_and_details",
        estimate_path: "repo_switch/selected_commit_and_details/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "repo_switch/twenty_tabs",
        estimate_path: "repo_switch/twenty_tabs/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MICROSECOND,
    },
    PerfBudgetSpec {
        label: "repo_switch/20_repos_all_hot",
        estimate_path: "repo_switch/20_repos_all_hot/new/estimates.json",
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    // repo_switch/selected_diff_file — switch with fully loaded diff content
    // (diff lines + file text cached). Heavier state snapshot than two_hot_repos.
    PerfBudgetSpec {
        label: "repo_switch/selected_diff_file",
        estimate_path: "repo_switch/selected_diff_file/new/estimates.json",
        threshold_ns: 200.0 * NANOS_PER_MICROSECOND,
    },
    // repo_switch/selected_conflict_target — switch where the diff target is a
    // conflicted file, triggering LoadConflictFile instead of LoadDiff+LoadDiffFile.
    PerfBudgetSpec {
        label: "repo_switch/selected_conflict_target",
        estimate_path: "repo_switch/selected_conflict_target/new/estimates.json",
        threshold_ns: 200.0 * NANOS_PER_MICROSECOND,
    },
    // repo_switch/merge_active_with_draft_restore — switch to a repo mid-merge
    // with a loaded draft merge commit message. Same effect shape as two_hot_repos
    // but heavier state due to the merge message string.
    PerfBudgetSpec {
        label: "repo_switch/merge_active_with_draft_restore",
        estimate_path: "repo_switch/merge_active_with_draft_restore/new/estimates.json",
        threshold_ns: 200.0 * NANOS_PER_MICROSECOND,
    },
    // status_list/unstaged_large — visible-window row build with cold path-display cache
    PerfBudgetSpec {
        label: "status_list/unstaged_large",
        estimate_path: "status_list/unstaged_large/new/estimates.json",
        threshold_ns: 250.0 * NANOS_PER_MICROSECOND,
    },
    // status_list/staged_large — same visible-window surface with a staged-file mix
    PerfBudgetSpec {
        label: "status_list/staged_large",
        estimate_path: "status_list/staged_large/new/estimates.json",
        threshold_ns: 250.0 * NANOS_PER_MICROSECOND,
    },
    // status_list/20k_entries_mixed_depth — visible-window render after
    // prewarming the shared path-display cache past its clear threshold.
    PerfBudgetSpec {
        label: "status_list/20k_entries_mixed_depth",
        estimate_path: "status_list/20k_entries_mixed_depth/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MILLISECOND,
    },
    // status_multi_select/range_select — measured around 301 µs for a
    // 512-path shift-selection in a 20k-entry status list. Keep modest
    // headroom while still catching accidental extra scans or selection rebuilds.
    PerfBudgetSpec {
        label: "status_multi_select/range_select",
        estimate_path: "status_multi_select/range_select/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MILLISECOND,
    },
    // status_select_diff_open/unstaged — reducer dispatch cost for selecting
    // an unstaged status row to open its diff. Includes a linear conflict-check
    // scan over all 10k unstaged entries (~326 µs measured). Budget generous to
    // allow shared-runner noise.
    PerfBudgetSpec {
        label: "status_select_diff_open/unstaged",
        estimate_path: "status_select_diff_open/unstaged/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MILLISECOND,
    },
    // status_select_diff_open/staged — staged path skips the conflict-entry scan,
    // so the dispatch is pure state mutation + effect allocation (~208 ns measured).
    PerfBudgetSpec {
        label: "status_select_diff_open/staged",
        estimate_path: "status_select_diff_open/staged/new/estimates.json",
        threshold_ns: 10.0 * NANOS_PER_MICROSECOND,
    },
    // merge_open_bootstrap/small — eager no-marker bootstrap on a 5k-line
    // HTML fixture. Measured around 56 µs after skipping conflict-marker
    // parsing on clean inputs and collapsing the visible projection to one
    // full-file span when hide-resolved is off.
    PerfBudgetSpec {
        label: "merge_open_bootstrap/small",
        estimate_path: "merge_open_bootstrap/small/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MILLISECOND,
    },
    // merge_open_bootstrap/large_streamed — measured around 39 ms on the
    // synthetic 55k-line fixture; keep generous headroom for shared runners.
    PerfBudgetSpec {
        label: "merge_open_bootstrap/large_streamed",
        estimate_path: "merge_open_bootstrap/large_streamed/new/estimates.json",
        threshold_ns: 100.0 * NANOS_PER_MILLISECOND,
    },
    // merge_open_bootstrap/many_conflicts — 50 conflict blocks in a ~600-line
    // file; tests conflict-block-count scaling without large-file overhead.
    PerfBudgetSpec {
        label: "merge_open_bootstrap/many_conflicts",
        estimate_path: "merge_open_bootstrap/many_conflicts/new/estimates.json",
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    // merge_open_bootstrap/50k_lines_500_conflicts_streamed — extreme scale:
    // 50k lines + 500 conflict blocks.  Budget is generous to allow shared-runner noise.
    PerfBudgetSpec {
        label: "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
        estimate_path: "merge_open_bootstrap/50k_lines_500_conflicts_streamed/new/estimates.json",
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    // diff_refresh_rev_only_same_content/rekey — signature check + rev bump
    // The rekey path hashes the full file content (~5k lines) to compute the signature.
    // Measured at ~12 µs; budget allows headroom for shared-runner noise.
    PerfBudgetSpec {
        label: "diff_refresh_rev_only_same_content/rekey",
        estimate_path: "diff_refresh_rev_only_same_content/rekey/new/estimates.json",
        threshold_ns: 50.0 * NANOS_PER_MICROSECOND,
    },
    // diff_refresh_rev_only_same_content/rebuild — full side_by_side_plan
    // This is the expensive path; budget allows room for shared-runner noise.
    PerfBudgetSpec {
        label: "diff_refresh_rev_only_same_content/rebuild",
        estimate_path: "diff_refresh_rev_only_same_content/rebuild/new/estimates.json",
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
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
    // --- diff_open_image_preview_first_paint --- ready-image cache build + two-cell layout
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
    // --- frame_timing --- sustained interaction bursts with per-frame sidecar stats
    PerfBudgetSpec {
        label: "frame_timing/continuous_scroll_history_list",
        estimate_path: "frame_timing/continuous_scroll_history_list/new/estimates.json",
        // Measured around 292 µs for a 240-frame synthetic scroll burst.
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "frame_timing/continuous_scroll_large_diff",
        estimate_path: "frame_timing/continuous_scroll_large_diff/new/estimates.json",
        // Measured around 553 ms for 240 syntax-highlighted diff scroll steps.
        threshold_ns: 900.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "frame_timing/sidebar_resize_drag_sustained",
        estimate_path: "frame_timing/sidebar_resize_drag_sustained/new/estimates.json",
        // 240 frames × 200 drag steps each = 48k clamp+layout iterations.
        // PaneResizeDragStepFixture is pure arithmetic so this should be cheap.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "frame_timing/rapid_commit_selection_changes",
        estimate_path: "frame_timing/rapid_commit_selection_changes/new/estimates.json",
        // 120 commit selections each hashing 200 files through the row loop.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "frame_timing/repo_switch_during_scroll",
        estimate_path: "frame_timing/repo_switch_during_scroll/new/estimates.json",
        // 240 frames: ~232 scroll steps + ~8 repo switches. The repo switch
        // frames involve reducer dispatch + effect enumeration. Allow generous
        // budget as repo_switch work is heavier than pure scroll.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- keyboard --- sustained arrow-key repeat bursts with frame timing stats
    PerfBudgetSpec {
        label: "keyboard/arrow_scroll_history_sustained_repeat",
        estimate_path: "keyboard/arrow_scroll_history_sustained_repeat/new/estimates.json",
        // 240 one-row history scroll repeats over a cached 50k-commit list.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "keyboard/arrow_scroll_diff_sustained_repeat",
        estimate_path: "keyboard/arrow_scroll_diff_sustained_repeat/new/estimates.json",
        // 240 one-row repeats across a syntax-highlighted 100k-line diff window.
        threshold_ns: 900.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "keyboard/tab_focus_cycle_all_panes",
        estimate_path: "keyboard/tab_focus_cycle_all_panes/new/estimates.json",
        // 240 tab presses across repo tabs, two pane handles, and commit-details
        // inputs. This is mostly focus-order traversal with a small amount of
        // focus-ring bookkeeping, so it should remain comfortably sub-frame.
        threshold_ns: 3.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "keyboard/stage_unstage_toggle_rapid",
        estimate_path: "keyboard/stage_unstage_toggle_rapid/new/estimates.json",
        // 240 alternating StagePath/UnstagePath keyboard actions, each followed
        // by SelectDiff for the same partially staged path. This should stay in
        // the same ballpark as the lighter staging reducer benches.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- staging --- reducer dispatch cost of batch stage / unstage operations
    PerfBudgetSpec {
        label: "staging/stage_all_10k_files",
        estimate_path: "staging/stage_all_10k_files/new/estimates.json",
        // Single StagePaths dispatch for 10k paths — reducer increments
        // local_actions_in_flight and emits one Effect::StagePaths. The cost
        // is dominated by the path Vec clone. Budget generous for CI variance.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "staging/unstage_all_10k_files",
        estimate_path: "staging/unstage_all_10k_files/new/estimates.json",
        // Single UnstagePaths dispatch for 10k paths — symmetric to stage_all.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "staging/stage_unstage_interleaved_1k_files",
        estimate_path: "staging/stage_unstage_interleaved_1k_files/new/estimates.json",
        // 1k individual StagePath / UnstagePath dispatches, alternating.
        // Each dispatch does a linear repo lookup + ops_rev bump. 1k dispatches
        // should stay well under 10 ms.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- undo_redo --- conflict resolution undo/redo reducer cost
    PerfBudgetSpec {
        label: "undo_redo/conflict_resolution_deep_stack",
        estimate_path: "undo_redo/conflict_resolution_deep_stack/new/estimates.json",
        // 200 sequential ConflictSetRegionChoice dispatches. Each dispatch does
        // a linear repo lookup + region resolution update + conflict_rev bump.
        // 200 dispatches should stay well under 5 ms.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "undo_redo/conflict_resolution_undo_replay_50_steps",
        estimate_path: "undo_redo/conflict_resolution_undo_replay_50_steps/new/estimates.json",
        // 50 apply + 1 reset + 50 replay = 101 dispatches through the conflict
        // reducer. The reset is O(N) across all regions. Budget generous for CI.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    // --- clipboard --- data preparation cost for copy/paste/select operations
    PerfBudgetSpec {
        label: "clipboard/copy_10k_lines_from_diff",
        estimate_path: "clipboard/copy_10k_lines_from_diff/new/estimates.json",
        // Iterate 10k diff lines, concatenate into a clipboard string.
        // Pure string building — should stay under 5 ms.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "clipboard/paste_large_text_into_commit_message",
        estimate_path: "clipboard/paste_large_text_into_commit_message/new/estimates.json",
        // Insert ~200 KB into a fresh TextModel via replace_range.
        // TextModel rebuild is O(text) with chunk indexing. Budget generous for CI.
        threshold_ns: 5.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "clipboard/select_range_5k_lines_in_diff",
        estimate_path: "clipboard/select_range_5k_lines_in_diff/new/estimates.json",
        // Iterate 5k diff lines in a selection range, build extraction string.
        // Half the work of copy_10k_lines — budget at 3 ms.
        threshold_ns: 3.0 * NANOS_PER_MILLISECOND,
    },
    // --- git_ops --- backend entry-point latency with trace-sidecar breakdowns
    PerfBudgetSpec {
        label: "git_ops/status_dirty_500_files",
        estimate_path: "git_ops/status_dirty_500_files/new/estimates.json",
        // Measured around 2.8 ms on the synthetic 1k-file / 500-dirty fixture.
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/log_walk_10k_commits",
        estimate_path: "git_ops/log_walk_10k_commits/new/estimates.json",
        // Measured around 41.6 ms for a full 10k-commit head-page walk.
        threshold_ns: 200.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/log_walk_100k_commits_shallow",
        estimate_path: "git_ops/log_walk_100k_commits_shallow/new/estimates.json",
        // Initial-history page on a very deep repo: the request depth stays at
        // 200 commits even though total history is 100k commits.
        threshold_ns: 100.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/status_clean_10k_files",
        estimate_path: "git_ops/status_clean_10k_files/new/estimates.json",
        // Clean status on 10k tracked files — no dirty entries to collect.
        // Should be faster than the dirty variant.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/ref_enumerate_10k_refs",
        estimate_path: "git_ops/ref_enumerate_10k_refs/new/estimates.json",
        // Enumerate 10k local branch refs via list_branches().
        threshold_ns: 100.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/diff_rename_heavy",
        estimate_path: "git_ops/diff_rename_heavy/new/estimates.json",
        // Full commit diff over 256 rename-detected files.
        threshold_ns: 750.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/diff_binary_heavy",
        estimate_path: "git_ops/diff_binary_heavy/new/estimates.json",
        // Full commit diff over 128 binary file rewrites.
        threshold_ns: 500.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/diff_large_single_file_100k_lines",
        estimate_path: "git_ops/diff_large_single_file_100k_lines/new/estimates.json",
        // 100k-line full-file rewrite; backend diff generation is intentionally heavy.
        threshold_ns: 2_000.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/blame_large_file",
        estimate_path: "git_ops/blame_large_file/new/estimates.json",
        // 100k-line blame across 16 commits. Keep headroom for shared-runner noise.
        threshold_ns: 2_000.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "git_ops/file_history_first_page_sparse_100k_commits",
        estimate_path: "git_ops/file_history_first_page_sparse_100k_commits/new/estimates.json",
        // Path-limited first page over a 100k-commit repo where only every
        // 10th commit touches the target file. Much heavier than a shallow
        // head-log page, so keep generous CI headroom.
        threshold_ns: 1_500.0 * NANOS_PER_MILLISECOND,
    },
    // --- search --- commit filter by author and message
    PerfBudgetSpec {
        label: "search/commit_filter_by_author_50k_commits",
        estimate_path: "search/commit_filter_by_author_50k_commits/new/estimates.json",
        // Case-insensitive substring scan over 50k pre-lowercased author strings.
        // Should stay well under 10 ms for interactive responsiveness.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "search/commit_filter_by_message_50k_commits",
        estimate_path: "search/commit_filter_by_message_50k_commits/new/estimates.json",
        // Message strings are longer than author strings; allow slightly more headroom.
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "search/in_diff_text_search_100k_lines",
        estimate_path: "search/in_diff_text_search_100k_lines/new/estimates.json",
        // Full visible-row scan across a 100k-line synthetic unified diff.
        // Keep the budget interactive while allowing shared-runner headroom.
        threshold_ns: 60.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "search/in_diff_text_search_incremental_refinement",
        estimate_path: "search/in_diff_text_search_incremental_refinement/new/estimates.json",
        // Refined follow-up query on the same 100k-line diff; same scan shape,
        // but fewer matches than the broad query benchmark above.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "search/file_preview_text_search_100k_lines",
        estimate_path: "search/file_preview_text_search_100k_lines/new/estimates.json",
        // `Ctrl+F` over a 100k-line file preview scans reconstructed source
        // text line-by-line through the same path used by the main pane.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "search/file_fuzzy_find_100k_files",
        estimate_path: "search/file_fuzzy_find_100k_files/new/estimates.json",
        // Subsequence fuzzy match across 100k synthetic file paths.
        // Should stay well under 50 ms for interactive file-picker responsiveness.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "search/file_fuzzy_find_incremental_keystroke",
        estimate_path: "search/file_fuzzy_find_incremental_keystroke/new/estimates.json",
        // Two consecutive fuzzy scans (short query then extended query) simulating
        // incremental keystroke refinement. Budget is 2× single-scan.
        threshold_ns: 100.0 * NANOS_PER_MILLISECOND,
    },
    // --- scrollbar_drag_step --- vertical scrollbar thumb drag math
    PerfBudgetSpec {
        label: "scrollbar_drag_step/window_200",
        estimate_path: "scrollbar_drag_step/window_200/new/estimates.json",
        // 200-step sweep; pure thumb-metrics + offset arithmetic — should be well under 100 µs.
        threshold_ns: 100.0 * NANOS_PER_MICROSECOND,
    },
    // --- fs_event --- filesystem event to status update latency
    PerfBudgetSpec {
        label: "fs_event/single_file_save_to_status_update",
        estimate_path: "fs_event/single_file_save_to_status_update/new/estimates.json",
        // Single file write + git status on 1k-file repo. Dominated by status scan.
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "fs_event/git_checkout_200_files_to_status_update",
        estimate_path: "fs_event/git_checkout_200_files_to_status_update/new/estimates.json",
        // 200-file batch mutation + git status. Includes filesystem write overhead.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "fs_event/rapid_saves_debounce_coalesce",
        estimate_path: "fs_event/rapid_saves_debounce_coalesce/new/estimates.json",
        // 50 rapid file writes + single coalesced git status. Models debounce behavior.
        threshold_ns: 30.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "fs_event/false_positive_rate_under_churn",
        estimate_path: "fs_event/false_positive_rate_under_churn/new/estimates.json",
        // 100 files dirtied then reverted + status finding 0 dirty. The churn
        // write+revert is included; status should still be fast (no actual diff).
        threshold_ns: 30.0 * NANOS_PER_MILLISECOND,
    },
    // --- network --- mocked transport progress/cancel under UI load
    PerfBudgetSpec {
        label: "network/ui_responsiveness_during_fetch",
        estimate_path: "network/ui_responsiveness_during_fetch/new/estimates.json",
        // 240 frames of history scrolling interleaved with one progress update
        // render per frame. This should remain comfortably interactive.
        threshold_ns: 20.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "network/progress_bar_update_render_cost",
        estimate_path: "network/progress_bar_update_render_cost/new/estimates.json",
        // 360 progress updates through the mocked transport/render loop. This
        // is string-heavy but should stay well below a visible hitch.
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "network/cancel_operation_latency",
        estimate_path: "network/cancel_operation_latency/new/estimates.json",
        // 64 progress updates, then a cancel request with four queued updates
        // drained before the cancelled terminal render.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- idle --- sidecar-only long-running harness timings
    PerfBudgetSpec {
        label: "idle/background_refresh_cost_per_cycle",
        estimate_path: "@sidecar_ms:avg_refresh_cycle_ms",
        // Ten synthetic status refresh cycles across ten open repos should stay
        // comfortably sub-frame on average even on a dedicated runner.
        threshold_ns: 50.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "idle/wake_from_sleep_resume",
        estimate_path: "@sidecar_ms:wake_resume_ms",
        // Resume should coalesce into one bounded refresh burst across all repos.
        threshold_ns: 250.0 * NANOS_PER_MILLISECOND,
    },
    // --- display --- render cost at different scales, multi-window, DPI switch
    PerfBudgetSpec {
        label: "display/render_cost_1x_vs_2x_vs_3x_scale",
        estimate_path: "display/render_cost_1x_vs_2x_vs_3x_scale/new/estimates.json",
        // Three full layout+render passes (1x, 2x, 3x) of 10k-commit history +
        // 5k-line diff. Should stay well within interactive budget.
        threshold_ns: 15.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "display/two_windows_same_repo",
        estimate_path: "display/two_windows_same_repo/new/estimates.json",
        // Two simultaneous viewport renders (history top+bottom, diff split+inline)
        // from the same repo state.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "display/window_move_between_dpis",
        estimate_path: "display/window_move_between_dpis/new/estimates.json",
        // Render at 1x then re-render at 2x — simulates dragging a window to a
        // HiDPI monitor. Two full render passes total.
        threshold_ns: 10.0 * NANOS_PER_MILLISECOND,
    },
    // --- real_repo --- external snapshot-backed nightly-only reference benches
    PerfBudgetSpec {
        label: "real_repo/monorepo_open_and_history_load",
        estimate_path: "real_repo/monorepo_open_and_history_load/new/estimates.json",
        // Real monorepo open: status, ref enumeration, and a substantial
        // history load on a 100k+ file tree. Nightly-only and intentionally loose.
        threshold_ns: 15_000.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "real_repo/deep_history_open_and_scroll",
        estimate_path: "real_repo/deep_history_open_and_scroll/new/estimates.json",
        // Deep history reference case: load 50k commits from a complex graph
        // and hash three representative scroll windows.
        threshold_ns: 20_000.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "real_repo/mid_merge_conflict_list_and_open",
        estimate_path: "real_repo/mid_merge_conflict_list_and_open/new/estimates.json",
        // Mid-merge reference case: read conflicted status and open one
        // conflict session from an externally provisioned snapshot.
        threshold_ns: 5_000.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "real_repo/large_file_diff_open",
        estimate_path: "real_repo/large_file_diff_open/new/estimates.json",
        // Large generated-file diff reference case: parse diff + materialize
        // file-diff providers from a real snapshot-backed commit.
        threshold_ns: 5_000.0 * NANOS_PER_MILLISECOND,
    },
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

#[derive(Clone, Copy, Debug)]
struct StructuralBudgetSpec {
    bench: &'static str,
    metric: &'static str,
    comparator: StructuralBudgetComparator,
    threshold: f64,
}

const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
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
        threshold: 17_000.0,
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
    StructuralBudgetSpec {
        bench: "history_cache_build/stash_heavy",
        metric: "stash_helpers_filtered",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100.0,
    },
    // history_cache_build/decorated_refs_heavy — decoration map should touch many commits
    StructuralBudgetSpec {
        bench: "history_cache_build/decorated_refs_heavy",
        metric: "decorated_commits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 500.0,
    },
    // history_cache_build/50k_commits_2k_refs_200_stashes — 50k total commits
    // with 200 stash helpers removed from the visible history and 2k refs
    // spread across local branches, remotes, and tags.
    StructuralBudgetSpec {
        bench: "history_cache_build/50k_commits_2k_refs_200_stashes",
        metric: "visible_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 49_800.0,
    },
    StructuralBudgetSpec {
        bench: "history_cache_build/50k_commits_2k_refs_200_stashes",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 49_800.0,
    },
    StructuralBudgetSpec {
        bench: "history_cache_build/50k_commits_2k_refs_200_stashes",
        metric: "commit_vms",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 49_800.0,
    },
    StructuralBudgetSpec {
        bench: "history_cache_build/50k_commits_2k_refs_200_stashes",
        metric: "stash_helpers_filtered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "history_cache_build/50k_commits_2k_refs_200_stashes",
        metric: "decorated_commits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1_800.0,
    },
    StructuralBudgetSpec {
        bench: "history_load_more_append/page_500",
        metric: "existing_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "history_load_more_append/page_500",
        metric: "appended_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "history_load_more_append/page_500",
        metric: "total_commits_after_append",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_500.0,
    },
    StructuralBudgetSpec {
        bench: "history_load_more_append/page_500",
        metric: "log_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "history_load_more_append/page_500",
        metric: "follow_up_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "history_load_more_append/page_500",
        metric: "log_loading_more_cleared",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // history_scope_switch/current_branch_to_all_refs — scope must change
    StructuralBudgetSpec {
        bench: "history_scope_switch/current_branch_to_all_refs",
        metric: "scope_changed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "history_scope_switch/current_branch_to_all_refs",
        metric: "existing_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5_000.0,
    },
    // log_rev should bump exactly twice: once for set_log_scope, once for
    // set_log_loading_more(false)
    StructuralBudgetSpec {
        bench: "history_scope_switch/current_branch_to_all_refs",
        metric: "log_rev_delta",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "history_scope_switch/current_branch_to_all_refs",
        metric: "log_set_to_loading",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // Must emit exactly 1 LoadLog effect
    StructuralBudgetSpec {
        bench: "history_scope_switch/current_branch_to_all_refs",
        metric: "load_log_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // branch_sidebar/cache_hit_balanced — all iterations should be cache hits
    StructuralBudgetSpec {
        bench: "branch_sidebar/cache_hit_balanced",
        metric: "cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // branch_sidebar/cache_miss_remote_fanout — every iteration is an invalidation + miss
    StructuralBudgetSpec {
        bench: "branch_sidebar/cache_miss_remote_fanout",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // branch_sidebar/cache_invalidation_single_ref_change — every iteration is an invalidation
    StructuralBudgetSpec {
        bench: "branch_sidebar/cache_invalidation_single_ref_change",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // branch_sidebar/cache_invalidation_worktrees_ready — every iteration is an invalidation
    StructuralBudgetSpec {
        bench: "branch_sidebar/cache_invalidation_worktrees_ready",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "local_branches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "remote_branches",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "remotes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "branch_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_002.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "remote_headers",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "group_headers",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 300.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "max_branch_depth",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4.0,
    },
    StructuralBudgetSpec {
        bench: "branch_sidebar/20k_branches_100_remotes",
        metric: "sidebar_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_414.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/refocus_same_repo",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 6.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/refocus_same_repo",
        metric: "selected_diff_reload_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/two_hot_repos",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // Turn 27 hot-switch restamp fix reduced from 15 → 9 (skips cold-path refresh effects)
        threshold: 9.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/two_hot_repos",
        metric: "refresh_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // Turn 27 hot-switch restamp fix reduced from 12 → 6
        threshold: 6.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/two_hot_repos",
        metric: "selected_diff_reload_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/two_hot_repos",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_commit_and_details",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 13.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_commit_and_details",
        metric: "selected_diff_reload_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_commit_and_details",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_commit_and_details",
        metric: "selected_commit_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_commit_and_details",
        metric: "selected_diff_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/twenty_tabs",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 15.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/twenty_tabs",
        metric: "repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/twenty_tabs",
        metric: "hydrated_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/twenty_tabs",
        metric: "selected_commit_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/twenty_tabs",
        metric: "selected_diff_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/twenty_tabs",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/20_repos_all_hot",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 15.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/20_repos_all_hot",
        metric: "repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/20_repos_all_hot",
        metric: "hydrated_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/20_repos_all_hot",
        metric: "selected_commit_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/20_repos_all_hot",
        metric: "selected_diff_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/20_repos_all_hot",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // repo_switch/selected_diff_file — same effect shape as two_hot_repos but
    // with fully loaded diff content in the state snapshot.
    StructuralBudgetSpec {
        bench: "repo_switch/selected_diff_file",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // Turn 27 hot-switch restamp fix reduced from 15 → 9
        threshold: 9.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_diff_file",
        metric: "selected_diff_reload_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_diff_file",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_diff_file",
        metric: "selected_diff_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    // repo_switch/selected_conflict_target — 1 LoadConflictFile instead of
    // 2 diff reload effects, giving effect_count = 14.
    StructuralBudgetSpec {
        bench: "repo_switch/selected_conflict_target",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 14.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_conflict_target",
        metric: "selected_diff_reload_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_conflict_target",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/selected_conflict_target",
        metric: "selected_diff_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    // repo_switch/merge_active_with_draft_restore — same effect shape as
    // two_hot_repos; the merge message is part of the state snapshot cost.
    StructuralBudgetSpec {
        bench: "repo_switch/merge_active_with_draft_restore",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 15.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/merge_active_with_draft_restore",
        metric: "selected_diff_reload_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/merge_active_with_draft_restore",
        metric: "persist_session_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "repo_switch/merge_active_with_draft_restore",
        metric: "selected_diff_repo_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/unstaged_large",
        metric: "rows_requested",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/unstaged_large",
        metric: "rows_painted",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/unstaged_large",
        metric: "path_display_cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/unstaged_large",
        metric: "path_display_cache_clears",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/staged_large",
        metric: "rows_requested",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/staged_large",
        metric: "rows_painted",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/staged_large",
        metric: "path_display_cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/staged_large",
        metric: "path_display_cache_clears",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "rows_requested",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "rows_painted",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "entries_total",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "path_display_cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "path_display_cache_clears",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "max_path_depth",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "status_list/20k_entries_mixed_depth",
        metric: "prewarmed_entries",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 8_193.0,
    },
    StructuralBudgetSpec {
        bench: "status_multi_select/range_select",
        metric: "entries_total",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "status_multi_select/range_select",
        metric: "selected_paths",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 512.0,
    },
    StructuralBudgetSpec {
        bench: "status_multi_select/range_select",
        metric: "anchor_index",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4_096.0,
    },
    StructuralBudgetSpec {
        bench: "status_multi_select/range_select",
        metric: "clicked_index",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4_607.0,
    },
    StructuralBudgetSpec {
        bench: "status_multi_select/range_select",
        metric: "anchor_preserved",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_multi_select/range_select",
        metric: "position_scan_steps",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 9_000.0,
    },
    // --- status_select_diff_open --- reducer dispatch metrics
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "load_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/unstaged",
        metric: "load_diff_file_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
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
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "load_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "status_select_diff_open/staged",
        metric: "load_diff_file_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
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
    // --- keyboard structural budgets ---
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "window_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "scroll_step_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "repeat_events",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "rows_requested_total",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 28_800.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_history_sustained_repeat",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "total_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "window_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "scroll_step_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "repeat_events",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "rows_requested_total",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 48_000.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/arrow_scroll_diff_sustained_repeat",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "focus_target_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 26.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "repo_tab_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "detail_input_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "cycle_events",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "unique_targets_visited",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 26.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "wrap_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "max_scan_len",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/tab_focus_cycle_all_panes",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "path_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 128.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "toggle_events",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 720.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "stage_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "unstage_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "select_diff_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 480.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "ops_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "diff_state_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "area_flip_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "path_wrap_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "dropped_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "keyboard/stage_unstage_toggle_rapid",
        metric: "p99_exceeds_2x_budget",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- staging structural budgets ---
    StructuralBudgetSpec {
        bench: "staging/stage_all_10k_files",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_all_10k_files",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // One StagePaths effect per batch dispatch.
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_all_10k_files",
        metric: "stage_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_all_10k_files",
        metric: "ops_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "staging/unstage_all_10k_files",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "staging/unstage_all_10k_files",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // One UnstagePaths effect per batch dispatch.
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "staging/unstage_all_10k_files",
        metric: "unstage_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "staging/unstage_all_10k_files",
        metric: "ops_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_unstage_interleaved_1k_files",
        metric: "file_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_unstage_interleaved_1k_files",
        metric: "effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // One effect per individual dispatch: 1k dispatches = 1k effects.
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_unstage_interleaved_1k_files",
        metric: "stage_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // Half of 1k dispatches are stage operations.
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_unstage_interleaved_1k_files",
        metric: "unstage_effect_count",
        comparator: StructuralBudgetComparator::Exactly,
        // Other half are unstage operations.
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "staging/stage_unstage_interleaved_1k_files",
        metric: "ops_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        // Each dispatch bumps ops_rev once: 1k bumps.
        threshold: 1_000.0,
    },
    // --- undo_redo structural budgets ---
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_deep_stack",
        metric: "region_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_deep_stack",
        metric: "apply_dispatches",
        comparator: StructuralBudgetComparator::Exactly,
        // One dispatch per region.
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_deep_stack",
        metric: "conflict_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        // Each ConflictSetRegionChoice bumps conflict_rev once.
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_undo_replay_50_steps",
        metric: "region_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_undo_replay_50_steps",
        metric: "apply_dispatches",
        comparator: StructuralBudgetComparator::Exactly,
        // 50 initial apply dispatches.
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_undo_replay_50_steps",
        metric: "reset_dispatches",
        comparator: StructuralBudgetComparator::Exactly,
        // One ConflictResetResolutions dispatch.
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_undo_replay_50_steps",
        metric: "replay_dispatches",
        comparator: StructuralBudgetComparator::Exactly,
        // 50 replay dispatches.
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "undo_redo/conflict_resolution_undo_replay_50_steps",
        metric: "conflict_rev_delta",
        comparator: StructuralBudgetComparator::Exactly,
        // 50 apply + 1 reset + 50 replay = 101 conflict_rev bumps.
        threshold: 101.0,
    },
    // --- clipboard structural budgets ---
    StructuralBudgetSpec {
        bench: "clipboard/copy_10k_lines_from_diff",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/copy_10k_lines_from_diff",
        metric: "line_iterations",
        comparator: StructuralBudgetComparator::Exactly,
        // Iterates all 10k lines (including header/hunk lines that are skipped
        // for output but still iterated).
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/copy_10k_lines_from_diff",
        metric: "total_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        // At least 500 KB of text (10k lines × ~60 bytes average content line).
        threshold: 500_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/paste_large_text_into_commit_message",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/paste_large_text_into_commit_message",
        metric: "total_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        // 2k lines × ~96 bytes = ~192 KB minimum.
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/paste_large_text_into_commit_message",
        metric: "line_iterations",
        comparator: StructuralBudgetComparator::Exactly,
        // Single bulk insertion.
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/select_range_5k_lines_in_diff",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/select_range_5k_lines_in_diff",
        metric: "line_iterations",
        comparator: StructuralBudgetComparator::Exactly,
        // Only iterates the 5k-line selection range.
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "clipboard/select_range_5k_lines_in_diff",
        metric: "total_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        // At least 250 KB of text in the selection range.
        threshold: 250_000.0,
    },
    // --- git_ops structural budgets ---
    StructuralBudgetSpec {
        bench: "git_ops/status_dirty_500_files",
        metric: "tracked_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/status_dirty_500_files",
        metric: "dirty_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/status_dirty_500_files",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/status_dirty_500_files",
        metric: "log_walk_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_10k_commits",
        metric: "total_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_10k_commits",
        metric: "requested_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_10k_commits",
        metric: "commits_returned",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_10k_commits",
        metric: "log_walk_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_10k_commits",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_100k_commits_shallow",
        metric: "total_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_100k_commits_shallow",
        metric: "requested_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_100k_commits_shallow",
        metric: "commits_returned",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_100k_commits_shallow",
        metric: "log_walk_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/log_walk_100k_commits_shallow",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- git_ops/status_clean structural budgets ---
    StructuralBudgetSpec {
        bench: "git_ops/status_clean_10k_files",
        metric: "tracked_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/status_clean_10k_files",
        metric: "dirty_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/status_clean_10k_files",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- git_ops/ref_enumerate structural budgets ---
    StructuralBudgetSpec {
        bench: "git_ops/ref_enumerate_10k_refs",
        metric: "total_refs",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/ref_enumerate_10k_refs",
        metric: "branches_returned",
        comparator: StructuralBudgetComparator::AtLeast,
        // At least 10k branches + 1 for main.
        threshold: 10_001.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/ref_enumerate_10k_refs",
        metric: "ref_enumerate_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- git_ops/diff structural budgets ---
    StructuralBudgetSpec {
        bench: "git_ops/diff_rename_heavy",
        metric: "changed_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 256.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_rename_heavy",
        metric: "renamed_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 256.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_rename_heavy",
        metric: "diff_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_binary_heavy",
        metric: "changed_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 128.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_binary_heavy",
        metric: "binary_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 128.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_binary_heavy",
        metric: "diff_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_large_single_file_100k_lines",
        metric: "changed_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_large_single_file_100k_lines",
        metric: "line_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_large_single_file_100k_lines",
        metric: "diff_lines",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/diff_large_single_file_100k_lines",
        metric: "diff_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/blame_large_file",
        metric: "total_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 16.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/blame_large_file",
        metric: "line_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/blame_large_file",
        metric: "blame_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/blame_large_file",
        metric: "blame_distinct_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 16.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/blame_large_file",
        metric: "blame_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/file_history_first_page_sparse_100k_commits",
        metric: "total_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/file_history_first_page_sparse_100k_commits",
        metric: "file_history_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/file_history_first_page_sparse_100k_commits",
        metric: "requested_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/file_history_first_page_sparse_100k_commits",
        metric: "commits_returned",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/file_history_first_page_sparse_100k_commits",
        metric: "log_walk_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "git_ops/file_history_first_page_sparse_100k_commits",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- diff_open_svg_dual_path structural budgets ---
    StructuralBudgetSpec {
        bench: "diff_open_svg_dual_path_first_window/200",
        metric: "rasterize_success",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_svg_dual_path_first_window/200",
        metric: "fallback_triggered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_svg_dual_path_first_window/200",
        metric: "images_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_open_svg_dual_path_first_window/200",
        metric: "divider_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- app_launch structural budgets ---
    // The external launch harness is sidecar-only, so budget first-paint and
    // first-interactive timings directly from emitted metrics instead of
    // relying on Criterion estimate files.
    // The >= 0 allocation checks are presence/type gates for the required
    // milestone allocation schema, not tuned allocation budgets.
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "first_paint_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 2_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "first_interactive_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 3_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "first_paint_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "first_paint_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "first_interactive_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "first_interactive_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_empty_workspace",
        metric: "repos_loaded",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "first_paint_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 3_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "first_interactive_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 6_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "first_paint_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "first_paint_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "first_interactive_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "first_interactive_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_single_repo",
        metric: "repos_loaded",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "first_paint_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "first_interactive_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "first_paint_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "first_paint_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "first_interactive_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "first_interactive_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_five_repos",
        metric: "repos_loaded",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "first_paint_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 8_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "first_interactive_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "first_paint_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "first_paint_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "first_interactive_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "first_interactive_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/cold_twenty_repos",
        metric: "repos_loaded",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "first_paint_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 2_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "first_interactive_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 4_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "first_paint_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "first_paint_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "first_interactive_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "first_interactive_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_single_repo",
        metric: "repos_loaded",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "first_paint_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "first_interactive_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 15_000.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "first_paint_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "first_paint_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "first_interactive_alloc_ops",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "first_interactive_alloc_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "app_launch/warm_twenty_repos",
        metric: "repos_loaded",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20.0,
    },
    // --- idle structural budgets ---
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_single_repo_60s",
        metric: "open_repos",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_single_repo_60s",
        metric: "tracked_files_per_repo",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_single_repo_60s",
        metric: "sample_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 60.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_single_repo_60s",
        metric: "sample_duration_ms",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 59_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_single_repo_60s",
        metric: "avg_cpu_pct",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_single_repo_60s",
        metric: "rss_delta_kib",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 1_024.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_ten_repos_60s",
        metric: "open_repos",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_ten_repos_60s",
        metric: "tracked_files_per_repo",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_ten_repos_60s",
        metric: "sample_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 60.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_ten_repos_60s",
        metric: "sample_duration_ms",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 59_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_ten_repos_60s",
        metric: "avg_cpu_pct",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 5.0,
    },
    StructuralBudgetSpec {
        bench: "idle/cpu_usage_ten_repos_60s",
        metric: "rss_delta_kib",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 4_096.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_single_repo_10min",
        metric: "open_repos",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_single_repo_10min",
        metric: "tracked_files_per_repo",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_single_repo_10min",
        metric: "sample_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 600.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_single_repo_10min",
        metric: "sample_duration_ms",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 599_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_single_repo_10min",
        metric: "avg_cpu_pct",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_single_repo_10min",
        metric: "rss_delta_kib",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 1_024.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_ten_repos_10min",
        metric: "open_repos",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_ten_repos_10min",
        metric: "tracked_files_per_repo",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_ten_repos_10min",
        metric: "sample_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 600.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_ten_repos_10min",
        metric: "sample_duration_ms",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 599_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_ten_repos_10min",
        metric: "avg_cpu_pct",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 5.0,
    },
    StructuralBudgetSpec {
        bench: "idle/memory_growth_ten_repos_10min",
        metric: "rss_delta_kib",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 4_096.0,
    },
    StructuralBudgetSpec {
        bench: "idle/background_refresh_cost_per_cycle",
        metric: "open_repos",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/background_refresh_cost_per_cycle",
        metric: "tracked_files_per_repo",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/background_refresh_cost_per_cycle",
        metric: "refresh_cycles",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/background_refresh_cost_per_cycle",
        metric: "repos_refreshed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "idle/background_refresh_cost_per_cycle",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "idle/background_refresh_cost_per_cycle",
        metric: "max_refresh_cycle_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "idle/wake_from_sleep_resume",
        metric: "open_repos",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/wake_from_sleep_resume",
        metric: "tracked_files_per_repo",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "idle/wake_from_sleep_resume",
        metric: "refresh_cycles",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "idle/wake_from_sleep_resume",
        metric: "repos_refreshed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/wake_from_sleep_resume",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "idle/wake_from_sleep_resume",
        metric: "wake_resume_ms",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 250.0,
    },
    // --- scrollbar_drag_step structural budgets ---
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "steps",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "thumb_metric_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "scroll_offset_recomputes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "viewport_h",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 800.0,
    },
    // The drag must reach both track boundaries during the oscillation.
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "clamp_at_top_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "clamp_at_bottom_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // Scroll position must sweep a meaningful range.
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "min_scroll_y",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "scrollbar_drag_step/window_200",
        metric: "max_scroll_y",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100_000.0,
    },
    // --- search structural budgets ---
    StructuralBudgetSpec {
        bench: "search/commit_filter_by_author_50k_commits",
        metric: "total_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50_000.0,
    },
    // "Alice" matches 1 of 10 first names → ~10% of 50k = ~5000.
    StructuralBudgetSpec {
        bench: "search/commit_filter_by_author_50k_commits",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_000.0,
    },
    // Incremental refinement (appending 'x') should find fewer matches.
    StructuralBudgetSpec {
        bench: "search/commit_filter_by_author_50k_commits",
        metric: "incremental_matches",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/commit_filter_by_message_50k_commits",
        metric: "total_commits",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 50_000.0,
    },
    // "fix" matches 1 of 10 prefixes → ~10% of 50k = ~5000.
    StructuralBudgetSpec {
        bench: "search/commit_filter_by_message_50k_commits",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_000.0,
    },
    // Incremental refinement should find fewer matches.
    StructuralBudgetSpec {
        bench: "search/commit_filter_by_message_50k_commits",
        metric: "incremental_matches",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_100k_lines",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_100k_lines",
        metric: "visible_rows_scanned",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 110_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_100k_lines",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 7_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_incremental_refinement",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_incremental_refinement",
        metric: "visible_rows_scanned",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 110_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_incremental_refinement",
        metric: "prior_matches",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 7_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_incremental_refinement",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1_400.0,
    },
    StructuralBudgetSpec {
        bench: "search/in_diff_text_search_incremental_refinement",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 1_700.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_preview_text_search_100k_lines",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_preview_text_search_100k_lines",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 3_000_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_preview_text_search_100k_lines",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 6_250.0,
    },
    // --- file_fuzzy_find structural budgets ---
    StructuralBudgetSpec {
        bench: "search/file_fuzzy_find_100k_files",
        metric: "total_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_fuzzy_find_100k_files",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_fuzzy_find_100k_files",
        metric: "query_len",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_fuzzy_find_incremental_keystroke",
        metric: "total_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 100_000.0,
    },
    // Incremental: the extended query should find fewer matches than the short one.
    StructuralBudgetSpec {
        bench: "search/file_fuzzy_find_incremental_keystroke",
        metric: "prior_matches",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "search/file_fuzzy_find_incremental_keystroke",
        metric: "matches_found",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // --- fs_event --- filesystem event harness structural budgets
    StructuralBudgetSpec {
        bench: "fs_event/single_file_save_to_status_update",
        metric: "tracked_files",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/single_file_save_to_status_update",
        metric: "mutation_files",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/single_file_save_to_status_update",
        metric: "dirty_files_detected",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/single_file_save_to_status_update",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/git_checkout_200_files_to_status_update",
        metric: "tracked_files",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1_000.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/git_checkout_200_files_to_status_update",
        metric: "mutation_files",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/git_checkout_200_files_to_status_update",
        metric: "dirty_files_detected",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/git_checkout_200_files_to_status_update",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/rapid_saves_debounce_coalesce",
        metric: "coalesced_saves",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/rapid_saves_debounce_coalesce",
        metric: "dirty_files_detected",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/rapid_saves_debounce_coalesce",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/false_positive_rate_under_churn",
        metric: "mutation_files",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/false_positive_rate_under_churn",
        metric: "dirty_files_detected",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/false_positive_rate_under_churn",
        metric: "false_positives",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100.0,
    },
    StructuralBudgetSpec {
        bench: "fs_event/false_positive_rate_under_churn",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- network --- mocked transport progress/cancel structural budgets
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "total_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "scroll_frames",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "progress_updates",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "window_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "output_tail_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "network/ui_responsiveness_during_fetch",
        metric: "tail_trim_events",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "network/progress_bar_update_render_cost",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 360.0,
    },
    StructuralBudgetSpec {
        bench: "network/progress_bar_update_render_cost",
        metric: "progress_updates",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 360.0,
    },
    StructuralBudgetSpec {
        bench: "network/progress_bar_update_render_cost",
        metric: "render_passes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 360.0,
    },
    StructuralBudgetSpec {
        bench: "network/progress_bar_update_render_cost",
        metric: "bar_width",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 32.0,
    },
    StructuralBudgetSpec {
        bench: "network/progress_bar_update_render_cost",
        metric: "output_tail_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "network/progress_bar_update_render_cost",
        metric: "tail_trim_events",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 280.0,
    },
    StructuralBudgetSpec {
        bench: "network/cancel_operation_latency",
        metric: "frame_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 69.0,
    },
    StructuralBudgetSpec {
        bench: "network/cancel_operation_latency",
        metric: "progress_updates",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 68.0,
    },
    StructuralBudgetSpec {
        bench: "network/cancel_operation_latency",
        metric: "render_passes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 69.0,
    },
    StructuralBudgetSpec {
        bench: "network/cancel_operation_latency",
        metric: "cancel_frames_until_stopped",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 5.0,
    },
    StructuralBudgetSpec {
        bench: "network/cancel_operation_latency",
        metric: "drained_updates_after_cancel",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4.0,
    },
    StructuralBudgetSpec {
        bench: "network/cancel_operation_latency",
        metric: "output_tail_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 68.0,
    },
    // --- display --- render cost at different scales, multi-window, DPI switch
    StructuralBudgetSpec {
        bench: "display/render_cost_1x_vs_2x_vs_3x_scale",
        metric: "scale_factors_tested",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "display/render_cost_1x_vs_2x_vs_3x_scale",
        metric: "total_layout_passes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "display/render_cost_1x_vs_2x_vs_3x_scale",
        metric: "windows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "display/render_cost_1x_vs_2x_vs_3x_scale",
        metric: "history_rows_per_pass",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "display/render_cost_1x_vs_2x_vs_3x_scale",
        metric: "diff_rows_per_pass",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "display/two_windows_same_repo",
        metric: "windows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "display/two_windows_same_repo",
        metric: "total_layout_passes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "display/two_windows_same_repo",
        metric: "total_rows_rendered",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 640.0,
    },
    StructuralBudgetSpec {
        bench: "display/two_windows_same_repo",
        metric: "history_rows_per_pass",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "display/two_windows_same_repo",
        metric: "diff_rows_per_pass",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "display/window_move_between_dpis",
        metric: "scale_factors_tested",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "display/window_move_between_dpis",
        metric: "re_layout_passes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "display/window_move_between_dpis",
        metric: "total_layout_passes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "display/window_move_between_dpis",
        metric: "windows_rendered",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    // --- real_repo --- external snapshot-backed nightly-only structural budgets
    StructuralBudgetSpec {
        bench: "real_repo/monorepo_open_and_history_load",
        metric: "worktree_file_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 100_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/monorepo_open_and_history_load",
        metric: "commits_loaded",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/monorepo_open_and_history_load",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 5_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/monorepo_open_and_history_load",
        metric: "status_calls",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/monorepo_open_and_history_load",
        metric: "log_walk_calls",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/monorepo_open_and_history_load",
        metric: "ref_enumerate_calls",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/deep_history_open_and_scroll",
        metric: "commits_loaded",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/deep_history_open_and_scroll",
        metric: "graph_rows",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/deep_history_open_and_scroll",
        metric: "log_pages_loaded",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/deep_history_open_and_scroll",
        metric: "history_windows_scanned",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/deep_history_open_and_scroll",
        metric: "max_graph_lanes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/mid_merge_conflict_list_and_open",
        metric: "conflict_files",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/mid_merge_conflict_list_and_open",
        metric: "status_entries",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/mid_merge_conflict_list_and_open",
        metric: "conflict_regions",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/mid_merge_conflict_list_and_open",
        metric: "selected_conflict_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/large_file_diff_open",
        metric: "diff_lines",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/large_file_diff_open",
        metric: "file_new_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1_000_000.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/large_file_diff_open",
        metric: "split_rows_painted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/large_file_diff_open",
        metric: "inline_rows_painted",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "real_repo/large_file_diff_open",
        metric: "diff_calls",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 1.0,
    },
    // --- resolved_output_recompute_incremental structural budgets ---
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "requested_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "conflict_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 300.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "unresolved_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "both_choice_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 75.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "outline_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_076.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "marker_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 675.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "manual_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/full_recompute",
        metric: "recomputed_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_076.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "requested_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "conflict_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 300.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "unresolved_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "both_choice_blocks",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 75.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "outline_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_076.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "marker_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 675.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "manual_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "dirty_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "recomputed_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3.0,
    },
    StructuralBudgetSpec {
        bench: "resolved_output_recompute_incremental/incremental_recompute",
        metric: "fallback_full_recompute",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // large_html_syntax synthetic sidecar budgets. These pin the default 20k-line
    // synthetic fixture shape and the current prepared-window cache-hit profile.
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/background_prepare",
        metric: "line_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/background_prepare",
        metric: "text_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_000_000.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/background_prepare",
        metric: "prepared_document_available",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // These sidecars now live under the current Criterion bench ids without the
    // trailing `/160`; `window_lines` remains pinned below as a structural metric.
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "window_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "cache_document_present",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "pending",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_pending",
        metric: "loaded_chunks",
        comparator: StructuralBudgetComparator::AtLeast,
        // The pending path does not wait for background chunk building, so
        // loaded_chunks is inherently low and timing-dependent. Require at
        // least 1 chunk (document was prepared) but do not pin the full count.
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "window_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "cache_document_present",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "pending",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_steady",
        metric: "cache_misses",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "start_line",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 81.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "window_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "cache_document_present",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "pending",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "cache_hits",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 160.0,
    },
    StructuralBudgetSpec {
        bench: "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
        metric: "cache_misses",
        // Sweep window starts at line 81 which crosses chunk boundaries not
        // covered by the initial primed window; 49 misses is the stable
        // measured value for the current 160-line sweep position.
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 60.0,
    },
    // --- worktree_preview_render structural budgets ---
    // Pin the deterministic fixture shape for both cached-lookup and render-time-prepare paths.
    // Defaults: 4000 lines, 200-line window, 128 bytes/line, Rust syntax (Auto mode, prepared doc present).
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4_000.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "prepared_document_available",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/cached_lookup_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4_000.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 120.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "prepared_document_available",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "worktree_preview_render/render_time_prepare_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- diff_scroll structural budgets ---
    // Pin the default diff-scroll fixture shape for the normal and long-line variants.
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "start_line",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "visible_text_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 19_200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "min_line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "language_detected",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/normal_lines_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10_000.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "window_size",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "start_line",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "visible_text_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 819_200.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "min_line_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 4_096.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "language_detected",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "diff_scroll/long_lines_window/200",
        metric: "syntax_mode_auto",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_input_prepaint_windowed structural budgets ---
    // Pin the deterministic fixture shape for windowed and full-document paths.
    // Defaults: 20,000 lines, 80-row viewport, guard_rows=2, max_shape_bytes=4096.
    // Windowed variant (cold run): shapes 80 + 2*2 = 84 rows, all cache misses.
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "viewport_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "guard_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "cache_entries_after",
        // Cold run: 80 + 2*2 = 84 rows shaped, all new cache entries.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 84.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "cache_hits",
        // Cold run from empty cache — no hits expected.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/window_rows/80",
        metric: "cache_misses",
        // Cold run: all 84 rows are misses.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 84.0,
    },
    // Full-document control: shapes all 20,000 lines (+ guard rows), all misses.
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "viewport_rows",
        // Full doc: viewport_rows == total_lines == 20,000.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "guard_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "cache_entries_after",
        // Full doc cold run: 20,000 unique lines (guard rows wrap to existing indices).
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "cache_hits",
        // Cold run: guard rows (4) wrap to lines 0-3 which were already cached.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_prepaint_windowed/full_document_control",
        metric: "cache_misses",
        // Full doc cold run: 20,000 unique lines are misses; 4 guard rows are hits.
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    // --- text_input_runs_streamed_highlight structural budgets ---
    // Defaults: 20,000 lines, 80 visible rows, scroll step = 40.
    // Dense fixture highlights every visible line; sparse highlights every 8th
    // line plus every 24th line for the overlay spans.
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/legacy_scan",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_dense/streamed_cursor",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "total_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3334.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "visible_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 14.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/legacy_scan",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "visible_rows",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 80.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "scroll_step",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "total_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3334.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "visible_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 14.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "visible_lines_with_highlights",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 10.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "density_dense",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        metric: "algorithm_streamed",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_input_long_line_cap structural budgets ---
    // Defaults: 256 KiB line, 4096-byte cap, 64 iterations.
    // Capped variant truncates the line; uncapped variant processes the full line.
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "capped_len",
        comparator: StructuralBudgetComparator::AtMost,
        threshold: 4096.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "iterations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 64.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/capped_bytes/4096",
        metric: "cap_active",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "max_shape_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "capped_len",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 262_144.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "iterations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 64.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_long_line_cap/uncapped_control",
        metric: "cap_active",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- text_input_wrap_incremental_tabs structural budgets ---
    // Defaults: 20,000 tabbed lines, requested 128-byte minimum => 131-byte
    // generated lines, 720 px wrap width => 92 wrap columns. The first edit
    // mutates line 0 and invalidates the edited line plus one neighbor.
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 131.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "edit_line_ix",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/full_recompute",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "line_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 131.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "edit_line_ix",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_tabs/incremental_patch",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_input_wrap_incremental_burst_edits structural budgets ---
    // Defaults: 20,000 tabbed lines, 128-byte minimum => 131-byte generated
    // lines, 720 px wrap => 92 columns, 12 edits per burst. Each burst scatters
    // 12 edits across well-spaced lines (stride 17); each edit invalidates ~2
    // dirty lines. Full recompute recomputes all 20,000 lines per edit.
    // These sidecars follow the Criterion bench ids with the default `/12`
    // burst-size segment, so structural lookups must match that emitted path.
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "edits_per_burst",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "total_dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 240_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/full_recompute/12",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "total_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 20_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "edits_per_burst",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 12.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "wrap_columns",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 92.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "total_dirty_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "total_rows_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 40_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "recomputed_lines",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 24.0,
    },
    StructuralBudgetSpec {
        bench: "text_input_wrap_incremental_burst_edits/incremental_patch/12",
        metric: "incremental_patch",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    // --- text_model_snapshot_clone_cost structural budgets ---
    // Defaults: 2 MiB minimum text-model document expands to 2,097,154 bytes
    // across 37,183 stored line-start markers. Both variants clone 8,192
    // times and sample a 96-byte prefix from each clone.
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "document_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2_097_154.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "line_starts",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 37_183.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "clone_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 8_192.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "sampled_prefix_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
        metric: "snapshot_path",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "document_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2_097_154.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "line_starts",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 37_183.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "clone_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 8_192.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "sampled_prefix_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 96.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
        metric: "snapshot_path",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    // --- text_model_bulk_load_large structural budgets ---
    // Defaults: 20,000 lines × ~130 bytes/line + newlines ≈ 2.5+ MiB source.
    // Piece-table variants produce document_bytes_after == source_bytes and
    // line_starts_after >= 20,001.  String-push control has no line tracking.
    // append_large uses 2 chunks, from_large_text uses 1, string_push uses ≈80.
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "document_bytes_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 20_001.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "chunk_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_append_large",
        metric: "load_variant",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "document_bytes_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 20_001.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "chunk_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/piece_table_from_large_text",
        metric: "load_variant",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "source_bytes",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "document_bytes_after",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 2_500_000.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "chunk_count",
        comparator: StructuralBudgetComparator::AtLeast,
        threshold: 50.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_bulk_load_large/string_push_control",
        metric: "load_variant",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 2.0,
    },
    // --- text_model_fragmented_edits structural budgets ---
    // Defaults: 512 KiB minimum source expands to 524,295 bytes. The
    // deterministic 500-edit sequence deletes 3,681 bytes, inserts 3,990
    // bytes, and leaves a 524,604-byte document with 9,806 line starts.
    // readback_operations encodes post-edit validation work:
    // 0 = edit-only/control, 1 = single as_str(), 64 = shared-string loop.
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/piece_table_edits",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/materialize_after_edits",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 64.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/shared_string_after_edits/64",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "initial_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_295.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "edit_count",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 500.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "deleted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_681.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "inserted_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 3_990.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "final_bytes",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 524_604.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "line_starts_after",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 9_806.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "readback_operations",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 0.0,
    },
    StructuralBudgetSpec {
        bench: "text_model_fragmented_edits/string_edit_control",
        metric: "string_control",
        comparator: StructuralBudgetComparator::Exactly,
        threshold: 1.0,
    },
];

#[derive(Clone, Copy, Debug)]
enum StructuralBudgetComparator {
    AtMost,
    AtLeast,
    Exactly,
}

impl StructuralBudgetComparator {
    fn matches(self, observed: f64, threshold: f64) -> bool {
        match self {
            Self::AtMost => observed <= threshold,
            Self::AtLeast => observed >= threshold,
            Self::Exactly => (observed - threshold).abs() <= f64::EPSILON,
        }
    }

    fn format_expectation(self, threshold: f64) -> String {
        match self {
            Self::AtMost => format!("<= {}", format_metric_value(threshold)),
            Self::AtLeast => format!(">= {}", format_metric_value(threshold)),
            Self::Exactly => format!("== {}", format_metric_value(threshold)),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct CriterionEstimates {
    mean: EstimateDistribution,
}

#[derive(Debug, Clone, Deserialize)]
struct EstimateDistribution {
    point_estimate: f64,
    confidence_interval: ConfidenceInterval,
}

#[derive(Debug, Clone, Deserialize)]
struct ConfidenceInterval {
    upper_bound: f64,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum BudgetStatus {
    WithinBudget,
    Alert,
    /// Benchmark data was not found and `--skip-missing` was active.
    Skipped,
}

impl BudgetStatus {
    fn icon(self) -> &'static str {
        match self {
            Self::WithinBudget => "OK",
            Self::Alert => "ALERT",
            Self::Skipped => "SKIP",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::WithinBudget => "within budget",
            Self::Alert => "alert",
            Self::Skipped => "skipped (not run)",
        }
    }
}

#[derive(Debug, Clone)]
struct BudgetResult {
    spec: PerfBudgetSpec,
    status: BudgetStatus,
    mean_ns: Option<f64>,
    mean_upper_ns: Option<f64>,
    details: String,
}

#[derive(Debug, Clone)]
struct StructuralBudgetResult {
    spec: StructuralBudgetSpec,
    status: BudgetStatus,
    observed: Option<f64>,
    details: String,
}

#[derive(Debug, Clone)]
struct ArtifactFreshnessReference {
    path: PathBuf,
    modified: SystemTime,
}

#[derive(Debug, Clone)]
struct CliArgs {
    criterion_roots: Vec<PathBuf>,
    strict: bool,
    /// When true, benchmarks whose estimate/sidecar files are missing are
    /// silently skipped instead of treated as alerts. Useful for PR CI that
    /// only runs a subset of the full suite.
    skip_missing: bool,
    /// Optional freshness gate. Artifacts older than this file's mtime are
    /// treated like missing data.
    fresh_reference: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum CliParseResult {
    Run,
    Help,
}

fn main() {
    match parse_cli_args(env::args().skip(1)) {
        Ok((CliParseResult::Help, _)) => {
            println!("{}", usage());
        }
        Ok((CliParseResult::Run, cli)) => {
            if let Err(err) = run_report(cli) {
                eprintln!("{err}");
                std::process::exit(2);
            }
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!();
            eprintln!("{}", usage());
            std::process::exit(2);
        }
    }
}

fn run_report(cli: CliArgs) -> Result<(), String> {
    let freshness_reference = cli
        .fresh_reference
        .as_deref()
        .map(load_artifact_freshness_reference)
        .transpose()?;
    let mut timing_results = Vec::with_capacity(PERF_BUDGETS.len());
    for &spec in PERF_BUDGETS {
        timing_results.push(evaluate_budget(
            spec,
            &cli.criterion_roots,
            cli.skip_missing,
            freshness_reference.as_ref(),
        ));
    }
    let mut structural_results = Vec::with_capacity(STRUCTURAL_BUDGETS.len());
    for &spec in STRUCTURAL_BUDGETS {
        structural_results.push(evaluate_structural_budget(
            spec,
            &cli.criterion_roots,
            cli.skip_missing,
            freshness_reference.as_ref(),
        ));
    }

    let markdown = build_report_markdown(
        &timing_results,
        &structural_results,
        &cli.criterion_roots,
        cli.strict,
        freshness_reference.as_ref(),
    );
    println!("{markdown}");
    append_github_summary(&markdown)?;

    let mut has_alert = false;
    for result in &timing_results {
        if result.status == BudgetStatus::Alert {
            has_alert = true;
            emit_github_warning(&format!("{}: {}", result.spec.label, result.details));
        }
    }
    for result in &structural_results {
        if result.status == BudgetStatus::Alert {
            has_alert = true;
            emit_github_warning(&format!(
                "{} [{}]: {}",
                result.spec.bench, result.spec.metric, result.details
            ));
        }
    }

    if has_alert && cli.strict {
        return Err(
            "one or more performance budgets exceeded thresholds (strict mode enabled)".to_string(),
        );
    }

    Ok(())
}

fn evaluate_budget(
    spec: PerfBudgetSpec,
    criterion_roots: &[PathBuf],
    skip_missing: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> BudgetResult {
    if let Some(metric) = spec.estimate_path.strip_prefix(SIDECAR_TIMING_MS_PREFIX) {
        return evaluate_sidecar_timing_budget(
            spec,
            criterion_roots,
            skip_missing,
            freshness_reference,
            metric,
        );
    }

    let searched_paths = estimate_search_paths(criterion_roots, spec.estimate_path);
    let estimate_path = match select_artifact_path(&searched_paths, freshness_reference) {
        Ok(ArtifactSelection::Fresh(path)) => path,
        Ok(ArtifactSelection::Missing) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_missing_paths_message("estimate file", &searched_paths),
            };
        }
        Ok(ArtifactSelection::Stale(stale_paths)) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_stale_paths_message(
                    "estimate file",
                    &stale_paths,
                    freshness_reference.expect("stale artifacts require freshness reference"),
                ),
            };
        }
        Err(err) => {
            return BudgetResult {
                spec,
                status: BudgetStatus::Alert,
                mean_ns: None,
                mean_upper_ns: None,
                details: err,
            };
        }
    };

    match read_estimates(&estimate_path) {
        Ok(estimates) => {
            let mean_ns = estimates.mean.point_estimate;
            let mean_upper_ns = estimates.mean.confidence_interval.upper_bound;
            if mean_upper_ns <= spec.threshold_ns {
                BudgetResult {
                    spec,
                    status: BudgetStatus::WithinBudget,
                    mean_ns: Some(mean_ns),
                    mean_upper_ns: Some(mean_upper_ns),
                    details: format!(
                        "mean upper bound {} <= threshold {}",
                        format_duration_ns(mean_upper_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            } else {
                BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: Some(mean_ns),
                    mean_upper_ns: Some(mean_upper_ns),
                    details: format!(
                        "mean upper bound {} exceeds threshold {}",
                        format_duration_ns(mean_upper_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            }
        }
        Err(err) => BudgetResult {
            spec,
            status: BudgetStatus::Alert,
            mean_ns: None,
            mean_upper_ns: None,
            details: err,
        },
    }
}

fn evaluate_sidecar_timing_budget(
    spec: PerfBudgetSpec,
    criterion_roots: &[PathBuf],
    skip_missing: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
    metric: &str,
) -> BudgetResult {
    let searched_paths = sidecar_search_paths(criterion_roots, spec.label);
    let sidecar_path = match select_artifact_path(&searched_paths, freshness_reference) {
        Ok(ArtifactSelection::Fresh(path)) => path,
        Ok(ArtifactSelection::Missing) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_missing_paths_message("sidecar file", &searched_paths),
            };
        }
        Ok(ArtifactSelection::Stale(stale_paths)) => {
            return BudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                mean_ns: None,
                mean_upper_ns: None,
                details: format_stale_paths_message(
                    "sidecar file",
                    &stale_paths,
                    freshness_reference.expect("stale artifacts require freshness reference"),
                ),
            };
        }
        Err(err) => {
            return BudgetResult {
                spec,
                status: BudgetStatus::Alert,
                mean_ns: None,
                mean_upper_ns: None,
                details: err,
            };
        }
    };

    match read_sidecar(&sidecar_path) {
        Ok(report) => {
            if report.bench != spec.label {
                return BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: None,
                    mean_upper_ns: None,
                    details: format!(
                        "sidecar bench label {:?} does not match expected {:?}",
                        report.bench, spec.label
                    ),
                };
            }

            if let Some(details) = invalid_tracked_sidecar_details(&report) {
                return BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: None,
                    mean_upper_ns: None,
                    details,
                };
            }

            let Some(observed_ms) = sidecar_metric_value(&report, metric) else {
                return BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: None,
                    mean_upper_ns: None,
                    details: format!("missing numeric metric {:?}", metric),
                };
            };

            let observed_ns = observed_ms * NANOS_PER_MILLISECOND;
            if observed_ns <= spec.threshold_ns {
                BudgetResult {
                    spec,
                    status: BudgetStatus::WithinBudget,
                    mean_ns: Some(observed_ns),
                    mean_upper_ns: Some(observed_ns),
                    details: format!(
                        "sidecar metric {:?} {} <= threshold {}",
                        metric,
                        format_duration_ns(observed_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            } else {
                BudgetResult {
                    spec,
                    status: BudgetStatus::Alert,
                    mean_ns: Some(observed_ns),
                    mean_upper_ns: Some(observed_ns),
                    details: format!(
                        "sidecar metric {:?} {} exceeds threshold {}",
                        metric,
                        format_duration_ns(observed_ns),
                        format_duration_ns(spec.threshold_ns)
                    ),
                }
            }
        }
        Err(err) => BudgetResult {
            spec,
            status: BudgetStatus::Alert,
            mean_ns: None,
            mean_upper_ns: None,
            details: err,
        },
    }
}

fn evaluate_structural_budget(
    spec: StructuralBudgetSpec,
    criterion_roots: &[PathBuf],
    skip_missing: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> StructuralBudgetResult {
    let searched_paths = sidecar_search_paths(criterion_roots, spec.bench);
    let sidecar_path = match select_artifact_path(&searched_paths, freshness_reference) {
        Ok(ArtifactSelection::Fresh(path)) => path,
        Ok(ArtifactSelection::Missing) => {
            return StructuralBudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                observed: None,
                details: format_missing_paths_message("sidecar file", &searched_paths),
            };
        }
        Ok(ArtifactSelection::Stale(stale_paths)) => {
            return StructuralBudgetResult {
                spec,
                status: missing_or_stale_status(skip_missing),
                observed: None,
                details: format_stale_paths_message(
                    "sidecar file",
                    &stale_paths,
                    freshness_reference.expect("stale artifacts require freshness reference"),
                ),
            };
        }
        Err(err) => {
            return StructuralBudgetResult {
                spec,
                status: BudgetStatus::Alert,
                observed: None,
                details: err,
            };
        }
    };

    match read_sidecar(&sidecar_path) {
        Ok(report) => evaluate_structural_budget_from_report(spec, &report),
        Err(err) => StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details: err,
        },
    }
}

fn evaluate_structural_budget_from_report(
    spec: StructuralBudgetSpec,
    report: &PerfSidecarReport,
) -> StructuralBudgetResult {
    if report.bench != spec.bench {
        return StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details: format!(
                "sidecar bench label {:?} does not match expected {:?}",
                report.bench, spec.bench
            ),
        };
    }

    if let Some(details) = invalid_tracked_sidecar_details(report) {
        return StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details,
        };
    }

    let Some(observed) = sidecar_metric_value(report, spec.metric) else {
        return StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: None,
            details: format!("missing numeric metric {:?}", spec.metric),
        };
    };

    let expectation = spec.comparator.format_expectation(spec.threshold);
    if spec.comparator.matches(observed, spec.threshold) {
        StructuralBudgetResult {
            spec,
            status: BudgetStatus::WithinBudget,
            observed: Some(observed),
            details: format!(
                "observed {} satisfies {}",
                format_metric_value(observed),
                expectation
            ),
        }
    } else {
        StructuralBudgetResult {
            spec,
            status: BudgetStatus::Alert,
            observed: Some(observed),
            details: format!("{} violates {}", format_metric_value(observed), expectation),
        }
    }
}

fn invalid_tracked_sidecar_details(report: &PerfSidecarReport) -> Option<String> {
    if !report.bench.starts_with("app_launch/") {
        return None;
    }

    let missing_metrics = REQUIRED_APP_LAUNCH_ALLOCATION_METRICS
        .iter()
        .copied()
        .filter(|metric| sidecar_metric_value(report, metric).is_none())
        .collect::<Vec<_>>();
    if missing_metrics.is_empty() {
        return None;
    }

    Some(format!(
        "sidecar is missing required launch allocation metrics ({}) and is not a valid current app_launch baseline; timing-only launch sidecars must not be treated as comparable results",
        missing_metrics.join(", ")
    ))
}

fn sidecar_metric_value(report: &PerfSidecarReport, metric: &str) -> Option<f64> {
    report.metrics.get(metric).and_then(|value| match value {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    })
}

fn read_estimates(path: &Path) -> Result<CriterionEstimates, String> {
    let json = fs::read_to_string(path)
        .map_err(|err| format!("failed to read {}: {err}", path.display()))?;
    serde_json::from_str(&json).map_err(|err| format!("failed to parse {}: {err}", path.display()))
}

fn estimate_search_paths(criterion_roots: &[PathBuf], relative_path: &str) -> Vec<PathBuf> {
    criterion_roots
        .iter()
        .map(|root| root.join(relative_path))
        .collect()
}

fn sidecar_search_paths(criterion_roots: &[PathBuf], bench: &str) -> Vec<PathBuf> {
    criterion_roots
        .iter()
        .map(|root| criterion_sidecar_path(root, bench))
        .collect()
}

#[derive(Debug, Clone)]
enum ArtifactSelection {
    Fresh(PathBuf),
    Missing,
    Stale(Vec<PathBuf>),
}

fn load_artifact_freshness_reference(path: &Path) -> Result<ArtifactFreshnessReference, String> {
    let metadata = fs::metadata(path).map_err(|err| {
        format!(
            "failed to read freshness reference {}: {err}",
            path.display()
        )
    })?;
    let modified = metadata.modified().map_err(|err| {
        format!(
            "failed to read freshness reference timestamp {}: {err}",
            path.display()
        )
    })?;
    Ok(ArtifactFreshnessReference {
        path: path.to_path_buf(),
        modified,
    })
}

fn select_artifact_path(
    searched_paths: &[PathBuf],
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> Result<ArtifactSelection, String> {
    let mut stale_paths = Vec::new();

    for path in searched_paths {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => continue,
            Err(err) => return Err(format!("failed to read artifact {}: {err}", path.display())),
        };

        if let Some(freshness_reference) = freshness_reference {
            let modified = metadata.modified().map_err(|err| {
                format!(
                    "failed to read artifact timestamp {}: {err}",
                    path.display()
                )
            })?;
            if modified < freshness_reference.modified {
                stale_paths.push(path.clone());
                continue;
            }
        }

        return Ok(ArtifactSelection::Fresh(path.clone()));
    }

    if stale_paths.is_empty() {
        Ok(ArtifactSelection::Missing)
    } else {
        Ok(ArtifactSelection::Stale(stale_paths))
    }
}

fn missing_or_stale_status(skip_missing: bool) -> BudgetStatus {
    if skip_missing {
        BudgetStatus::Skipped
    } else {
        BudgetStatus::Alert
    }
}

fn format_missing_paths_message(kind: &str, searched_paths: &[PathBuf]) -> String {
    if searched_paths.len() == 1 {
        return format!("missing {kind} at {}", searched_paths[0].display());
    }

    let mut details = format!("missing {kind}; looked in ");
    for (ix, path) in searched_paths.iter().enumerate() {
        if ix > 0 {
            details.push_str(", ");
        }
        let _ = write!(details, "{}", path.display());
    }
    details
}

fn format_stale_paths_message(
    kind: &str,
    stale_paths: &[PathBuf],
    freshness_reference: &ArtifactFreshnessReference,
) -> String {
    if stale_paths.len() == 1 {
        return format!(
            "stale {kind} at {}; older than freshness reference {}",
            stale_paths[0].display(),
            freshness_reference.path.display()
        );
    }

    let mut details = format!(
        "stale {kind}; older than freshness reference {}; found only ",
        freshness_reference.path.display()
    );
    for (ix, path) in stale_paths.iter().enumerate() {
        if ix > 0 {
            details.push_str(", ");
        }
        let _ = write!(details, "{}", path.display());
    }
    details
}

fn build_report_markdown(
    timing_results: &[BudgetResult],
    structural_results: &[StructuralBudgetResult],
    criterion_roots: &[PathBuf],
    strict: bool,
    freshness_reference: Option<&ArtifactFreshnessReference>,
) -> String {
    let mut markdown = String::new();
    let _ = writeln!(markdown, "## View Performance Budget Report");
    let _ = writeln!(markdown);
    let criterion_label = if criterion_roots.len() == 1 {
        "criterion root"
    } else {
        "criterion roots"
    };
    let _ = writeln!(
        markdown,
        "- {criterion_label}: {}",
        format_criterion_roots_markdown(criterion_roots)
    );
    let _ = writeln!(
        markdown,
        "- mode: {}",
        if strict {
            "strict (fails on alert)"
        } else {
            "alert-only"
        }
    );
    if let Some(freshness_reference) = freshness_reference {
        let _ = writeln!(
            markdown,
            "- freshness reference: `{}`",
            freshness_reference.path.display()
        );
    }
    let _ = writeln!(markdown);

    if !timing_results.is_empty() {
        let _ = writeln!(markdown, "### Timing Budgets");
        let _ = writeln!(
            markdown,
            "| Benchmark | Threshold | Mean | Mean 95% upper | Status |"
        );
        let _ = writeln!(markdown, "| --- | --- | --- | --- | --- |");

        for result in timing_results {
            let mean = result
                .mean_ns
                .map(format_duration_ns)
                .unwrap_or_else(|| "n/a".to_string());
            let mean_upper = result
                .mean_upper_ns
                .map(format_duration_ns)
                .unwrap_or_else(|| "n/a".to_string());
            let _ = writeln!(
                markdown,
                "| `{}` | <= {} | {} | {} | {} {} |",
                result.spec.label,
                format_duration_ns(result.spec.threshold_ns),
                mean,
                mean_upper,
                result.status.icon(),
                result.status.label()
            );
        }
        let _ = writeln!(markdown);
    }

    if !structural_results.is_empty() {
        let _ = writeln!(markdown, "### Structural Budgets");
        let _ = writeln!(
            markdown,
            "| Benchmark | Metric | Expectation | Observed | Status |"
        );
        let _ = writeln!(markdown, "| --- | --- | --- | --- | --- |");

        for result in structural_results {
            let observed = result
                .observed
                .map(format_metric_value)
                .unwrap_or_else(|| "n/a".to_string());
            let _ = writeln!(
                markdown,
                "| `{}` | `{}` | {} | {} | {} {} |",
                result.spec.bench,
                result.spec.metric,
                result
                    .spec
                    .comparator
                    .format_expectation(result.spec.threshold),
                observed,
                result.status.icon(),
                result.status.label()
            );
        }
        let _ = writeln!(markdown);
    }

    let mut alert_count = 0usize;
    let mut skipped_count = 0usize;
    for result in timing_results {
        match result.status {
            BudgetStatus::Alert => alert_count = alert_count.saturating_add(1),
            BudgetStatus::Skipped => skipped_count = skipped_count.saturating_add(1),
            _ => {}
        }
    }
    for result in structural_results {
        match result.status {
            BudgetStatus::Alert => alert_count = alert_count.saturating_add(1),
            BudgetStatus::Skipped => skipped_count = skipped_count.saturating_add(1),
            _ => {}
        }
    }
    let total_budget_count = timing_results
        .len()
        .saturating_add(structural_results.len());

    if skipped_count > 0 {
        let skipped_reason = if freshness_reference.is_some() {
            "benchmark data not present or older than the freshness reference"
        } else {
            "benchmark data not present"
        };
        let _ = writeln!(
            markdown,
            "Skipped {skipped_count} budget(s) ({skipped_reason})."
        );
    }

    if alert_count == 0 {
        if skipped_count == total_budget_count && total_budget_count > 0 {
            let _ = writeln!(
                markdown,
                "No fresh benchmark data matched the requested report inputs; all tracked budgets were skipped."
            );
        } else if skipped_count > 0 {
            let _ = writeln!(
                markdown,
                "All non-skipped tracked view benchmarks are within budget."
            );
        } else {
            let _ = writeln!(markdown, "All tracked view benchmarks are within budget.");
        }
    } else {
        let _ = writeln!(markdown, "Budget alerts: {alert_count}");
        for result in timing_results {
            if result.status == BudgetStatus::Alert {
                let _ = writeln!(markdown, "- `{}`: {}", result.spec.label, result.details);
            }
        }
        for result in structural_results {
            if result.status == BudgetStatus::Alert {
                let _ = writeln!(
                    markdown,
                    "- `{}` / `{}`: {}",
                    result.spec.bench, result.spec.metric, result.details
                );
            }
        }
    }
    markdown
}

fn format_criterion_roots_markdown(criterion_roots: &[PathBuf]) -> String {
    let mut roots = String::new();
    for (ix, root) in criterion_roots.iter().enumerate() {
        if ix > 0 {
            roots.push_str(", ");
        }
        let _ = write!(roots, "`{}`", root.display());
    }
    roots
}

fn append_github_summary(markdown: &str) -> Result<(), String> {
    let Some(path) = env::var_os("GITHUB_STEP_SUMMARY") else {
        return Ok(());
    };
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|err| format!("failed to open {}: {err}", PathBuf::from(path).display()))?;
    file.write_all(markdown.as_bytes())
        .map_err(|err| format!("failed to append report to GITHUB_STEP_SUMMARY: {err}"))?;
    file.write_all(b"\n")
        .map_err(|err| format!("failed to append newline to GITHUB_STEP_SUMMARY: {err}"))?;
    Ok(())
}

fn emit_github_warning(message: &str) {
    println!("::warning title=View performance budget::{message}");
}

fn format_duration_ns(ns: f64) -> String {
    if !ns.is_finite() || ns < 0.0 {
        return "n/a".to_string();
    }
    if ns >= NANOS_PER_MILLISECOND {
        return format!("{:.3} ms", ns / NANOS_PER_MILLISECOND);
    }
    if ns >= NANOS_PER_MICROSECOND {
        return format!("{:.3} us", ns / NANOS_PER_MICROSECOND);
    }
    format!("{ns:.0} ns")
}

fn format_metric_value(value: f64) -> String {
    if !value.is_finite() {
        return "n/a".to_string();
    }
    if (value.fract()).abs() <= f64::EPSILON {
        return format!("{value:.0}");
    }
    format!("{value:.3}")
}

const DEFAULT_CRITERION_ROOTS: &[&str] = &["target/criterion", "criterion"];

fn default_criterion_roots() -> Vec<PathBuf> {
    let mut roots = Vec::with_capacity(DEFAULT_CRITERION_ROOTS.len());
    for root in DEFAULT_CRITERION_ROOTS {
        push_unique_criterion_root(&mut roots, PathBuf::from(root));
    }
    roots
}

fn push_unique_criterion_root(roots: &mut Vec<PathBuf>, candidate: PathBuf) {
    if roots.iter().any(|root| root == &candidate) {
        return;
    }
    roots.push(candidate);
}

fn parse_cli_args<I>(args: I) -> Result<(CliParseResult, CliArgs), String>
where
    I: IntoIterator<Item = String>,
{
    let mut criterion_roots = default_criterion_roots();
    let mut explicit_criterion_roots = false;
    let mut strict = strict_from_env();
    let mut skip_missing = false;
    let mut fresh_reference = None;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--criterion-root" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--criterion-root requires a path argument".to_string())?;
                if !explicit_criterion_roots {
                    criterion_roots.clear();
                    explicit_criterion_roots = true;
                }
                push_unique_criterion_root(&mut criterion_roots, PathBuf::from(value));
            }
            "--strict" => strict = true,
            "--skip-missing" => skip_missing = true,
            "--fresh-reference" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--fresh-reference requires a path argument".to_string())?;
                fresh_reference = Some(PathBuf::from(value));
            }
            "--help" | "-h" => {
                return Ok((
                    CliParseResult::Help,
                    CliArgs {
                        criterion_roots,
                        strict,
                        skip_missing,
                        fresh_reference,
                    },
                ));
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
    }

    Ok((
        CliParseResult::Run,
        CliArgs {
            criterion_roots,
            strict,
            skip_missing,
            fresh_reference,
        },
    ))
}

fn strict_from_env() -> bool {
    match env::var("GITCOMET_PERF_BUDGET_STRICT") {
        Ok(value) => is_truthy(&value),
        Err(_) => false,
    }
}

fn is_truthy(value: &str) -> bool {
    let normalized = value.trim().to_ascii_lowercase();
    matches!(normalized.as_str(), "1" | "true" | "yes" | "on")
}

fn usage() -> &'static str {
    "Usage: cargo run -p gitcomet-ui-gpui --bin perf_budget_report -- [--criterion-root PATH]... [--strict] [--skip-missing] [--fresh-reference PATH]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, SystemTime};
    use tempfile::TempDir;

    #[test]
    fn parse_estimates_reads_criterion_mean_shape() {
        let json = r#"{
            "mean": {
                "confidence_interval": {
                    "confidence_level": 0.95,
                    "lower_bound": 295963.49,
                    "upper_bound": 298962.86
                },
                "point_estimate": 297427.72,
                "standard_error": 771.75
            }
        }"#;
        let parsed: CriterionEstimates =
            serde_json::from_str(json).expect("criterion estimate json should parse");
        assert!((parsed.mean.point_estimate - 297_427.72).abs() < 0.01);
        assert!((parsed.mean.confidence_interval.upper_bound - 298_962.86).abs() < 0.01);
    }

    #[test]
    fn evaluate_budget_alerts_when_estimate_file_is_missing() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = PerfBudgetSpec {
            label: "missing",
            estimate_path: "missing/new/estimates.json",
            threshold_ns: 1_000.0,
        };
        let result = evaluate_budget(spec, &roots, false, None);
        assert_eq!(result.status, BudgetStatus::Alert);
        assert!(result.details.contains("missing estimate file"));
    }

    #[test]
    fn evaluate_budget_skips_when_estimate_file_is_missing_and_skip_missing() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = PerfBudgetSpec {
            label: "missing",
            estimate_path: "missing/new/estimates.json",
            threshold_ns: 1_000.0,
        };
        let result = evaluate_budget(spec, &roots, true, None);
        assert_eq!(result.status, BudgetStatus::Skipped);
        assert!(result.details.contains("missing estimate file"));
    }

    #[test]
    fn evaluate_structural_budget_skips_when_sidecar_missing_and_skip_missing() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = StructuralBudgetSpec {
            bench: "nonexistent/bench",
            metric: "some_metric",
            comparator: StructuralBudgetComparator::Exactly,
            threshold: 42.0,
        };
        let result = evaluate_structural_budget(spec, &roots, true, None);
        assert_eq!(result.status, BudgetStatus::Skipped);
        assert!(result.details.contains("missing sidecar file"));
    }

    #[test]
    fn evaluate_budget_within_budget_when_upper_bound_is_below_threshold() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = PerfBudgetSpec {
            label: "within",
            estimate_path: "within/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(temp_dir.path(), spec.estimate_path, 9_100.0, 9_800.0);

        let result = evaluate_budget(spec, &roots, false, None);
        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.mean_ns, Some(9_100.0));
        assert_eq!(result.mean_upper_ns, Some(9_800.0));
    }

    #[test]
    fn evaluate_budget_alerts_when_threshold_is_exceeded() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = PerfBudgetSpec {
            label: "over",
            estimate_path: "over/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(temp_dir.path(), spec.estimate_path, 11_000.0, 12_500.0);

        let result = evaluate_budget(spec, &roots, false, None);
        assert_eq!(result.status, BudgetStatus::Alert);
        assert_eq!(result.mean_ns, Some(11_000.0));
        assert_eq!(result.mean_upper_ns, Some(12_500.0));
        assert!(result.details.contains("exceeds threshold"));
    }

    #[test]
    fn evaluate_budget_reads_sidecar_timing_metric() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "idle/wake_from_sleep_resume",
            &[("wake_resume_ms", serde_json::json!(125.0))],
        );
        let spec = PerfBudgetSpec {
            label: "idle/wake_from_sleep_resume",
            estimate_path: "@sidecar_ms:wake_resume_ms",
            threshold_ns: 200.0 * NANOS_PER_MILLISECOND,
        };

        let result = evaluate_budget(spec, &roots, false, None);
        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.mean_ns, Some(125.0 * NANOS_PER_MILLISECOND));
        assert_eq!(result.mean_upper_ns, Some(125.0 * NANOS_PER_MILLISECOND));
    }

    #[test]
    fn evaluate_budget_alerts_when_launch_sidecar_is_timing_only() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "app_launch/cold_single_repo",
            &[
                ("first_paint_ms", json!(235.0)),
                ("first_interactive_ms", json!(515.0)),
                ("repos_loaded", json!(1)),
            ],
        );
        let spec = PerfBudgetSpec {
            label: "app_launch/cold_single_repo",
            estimate_path: "@sidecar_ms:first_paint_ms",
            threshold_ns: 3_000.0 * NANOS_PER_MILLISECOND,
        };

        let result = evaluate_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::Alert);
        assert_eq!(result.mean_ns, None);
        assert_eq!(result.mean_upper_ns, None);
        assert!(
            result
                .details
                .contains("not a valid current app_launch baseline")
        );
        assert!(result.details.contains("first_paint_alloc_ops"));
    }

    #[test]
    fn evaluate_budget_searches_secondary_criterion_root() {
        let first_root = TempDir::new().expect("first root");
        let second_root = TempDir::new().expect("second root");
        let roots = vec![
            first_root.path().to_path_buf(),
            second_root.path().to_path_buf(),
        ];
        let spec = PerfBudgetSpec {
            label: "secondary",
            estimate_path: "secondary/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(second_root.path(), spec.estimate_path, 9_500.0, 9_900.0);

        let result = evaluate_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.mean_ns, Some(9_500.0));
        assert_eq!(result.mean_upper_ns, Some(9_900.0));
    }

    #[test]
    fn evaluate_budget_skips_stale_estimate_with_fresh_reference() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = PerfBudgetSpec {
            label: "stale",
            estimate_path: "stale/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(temp_dir.path(), spec.estimate_path, 9_100.0, 9_800.0);

        let estimate_path = temp_dir.path().join(spec.estimate_path);
        let fresh_reference_path = temp_dir.path().join("fresh-reference");
        fs::write(&fresh_reference_path, "stamp").expect("write freshness reference");

        let reference_time = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        set_file_modified(&estimate_path, reference_time - Duration::from_secs(60));
        set_file_modified(&fresh_reference_path, reference_time);

        let fresh_reference =
            load_artifact_freshness_reference(&fresh_reference_path).expect("load freshness");
        let result = evaluate_budget(spec, &roots, true, Some(&fresh_reference));

        assert_eq!(result.status, BudgetStatus::Skipped);
        assert!(result.details.contains("stale estimate file"));
        assert!(result.details.contains("fresh-reference"));
    }

    #[test]
    fn evaluate_budget_prefers_fresh_secondary_root_with_fresh_reference() {
        let first_root = TempDir::new().expect("first root");
        let second_root = TempDir::new().expect("second root");
        let roots = vec![
            first_root.path().to_path_buf(),
            second_root.path().to_path_buf(),
        ];
        let spec = PerfBudgetSpec {
            label: "secondary-fresh",
            estimate_path: "secondary-fresh/new/estimates.json",
            threshold_ns: 10_000.0,
        };
        write_estimate_file(first_root.path(), spec.estimate_path, 12_000.0, 12_500.0);
        write_estimate_file(second_root.path(), spec.estimate_path, 9_500.0, 9_900.0);

        let first_path = first_root.path().join(spec.estimate_path);
        let second_path = second_root.path().join(spec.estimate_path);
        let fresh_reference_path = first_root.path().join("fresh-reference");
        fs::write(&fresh_reference_path, "stamp").expect("write freshness reference");

        let reference_time = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        set_file_modified(&first_path, reference_time - Duration::from_secs(60));
        set_file_modified(&fresh_reference_path, reference_time);
        set_file_modified(&second_path, reference_time + Duration::from_secs(60));

        let fresh_reference =
            load_artifact_freshness_reference(&fresh_reference_path).expect("load freshness");
        let result = evaluate_budget(spec, &roots, false, Some(&fresh_reference));

        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.mean_ns, Some(9_500.0));
        assert_eq!(result.mean_upper_ns, Some(9_900.0));
    }

    #[test]
    fn format_duration_ns_uses_human_units() {
        assert_eq!(format_duration_ns(999.0), "999 ns");
        assert_eq!(format_duration_ns(1_250.0), "1.250 us");
        assert_eq!(format_duration_ns(2_750_000.0), "2.750 ms");
    }

    #[test]
    fn parse_cli_args_defaults_to_alert_mode() {
        let (mode, cli) = parse_cli_args(Vec::<String>::new()).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert_eq!(
            cli.criterion_roots,
            vec![
                PathBuf::from("target/criterion"),
                PathBuf::from("criterion")
            ]
        );
        assert!(!cli.strict);
        assert!(!cli.skip_missing);
        assert_eq!(cli.fresh_reference, None);
    }

    #[test]
    fn parse_cli_args_supports_root_and_strict() {
        let args = vec![
            "--criterion-root".to_string(),
            "/tmp/criterion".to_string(),
            "--strict".to_string(),
        ];
        let (mode, cli) = parse_cli_args(args).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert_eq!(cli.criterion_roots, vec![PathBuf::from("/tmp/criterion")]);
        assert!(cli.strict);
        assert!(!cli.skip_missing);
        assert_eq!(cli.fresh_reference, None);
    }

    #[test]
    fn parse_cli_args_supports_skip_missing() {
        let args = vec!["--skip-missing".to_string()];
        let (mode, cli) = parse_cli_args(args).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert!(!cli.strict);
        assert!(cli.skip_missing);
        assert_eq!(cli.fresh_reference, None);
    }

    #[test]
    fn parse_cli_args_supports_strict_and_skip_missing() {
        let args = vec![
            "--strict".to_string(),
            "--skip-missing".to_string(),
            "--criterion-root".to_string(),
            "/tmp/cr".to_string(),
        ];
        let (mode, cli) = parse_cli_args(args).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert!(cli.strict);
        assert!(cli.skip_missing);
        assert_eq!(cli.criterion_roots, vec![PathBuf::from("/tmp/cr")]);
        assert_eq!(cli.fresh_reference, None);
    }

    #[test]
    fn parse_cli_args_supports_fresh_reference() {
        let args = vec![
            "--fresh-reference".to_string(),
            "/tmp/perf-suite.start".to_string(),
            "--skip-missing".to_string(),
        ];
        let (mode, cli) = parse_cli_args(args).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert!(cli.skip_missing);
        assert_eq!(
            cli.fresh_reference,
            Some(PathBuf::from("/tmp/perf-suite.start"))
        );
    }

    #[test]
    fn parse_cli_args_supports_multiple_criterion_roots() {
        let args = vec![
            "--criterion-root".to_string(),
            "/tmp/criterion-a".to_string(),
            "--criterion-root".to_string(),
            "/tmp/criterion-b".to_string(),
        ];
        let (mode, cli) = parse_cli_args(args).expect("parse args");
        assert_eq!(mode, CliParseResult::Run);
        assert_eq!(
            cli.criterion_roots,
            vec![
                PathBuf::from("/tmp/criterion-a"),
                PathBuf::from("/tmp/criterion-b")
            ]
        );
        assert_eq!(cli.fresh_reference, None);
    }

    #[test]
    fn perf_budgets_include_markdown_preview_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"markdown_preview_parse_build/single_document/medium"));
        assert!(labels.contains(&"markdown_preview_parse_build/two_sided_diff/medium"));
        assert!(labels.contains(&"markdown_preview_render_single/window_rows/200"));
        assert!(labels.contains(&"markdown_preview_render_diff/window_rows/200"));
    }

    #[test]
    fn perf_budgets_include_open_repo_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"open_repo/balanced"));
        assert!(labels.contains(&"open_repo/history_heavy"));
        assert!(labels.contains(&"open_repo/branch_heavy"));
        assert!(labels.contains(&"open_repo/extreme_metadata_fanout"));
    }

    #[test]
    fn perf_budgets_include_streamed_conflict_provider_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"conflict_streamed_provider/index_build"));
        assert!(labels.contains(&"conflict_streamed_provider/first_page/200"));
        assert!(labels.contains(&"conflict_streamed_provider/first_page_cache_hit/200"));
        assert!(labels.contains(&"conflict_streamed_provider/deep_scroll_90pct/200"));
        assert!(labels.contains(&"conflict_streamed_provider/search_rare_text"));
    }

    #[test]
    fn perf_budgets_include_streamed_resolved_output_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"conflict_streamed_resolved_output/projection_build"));
        assert!(labels.contains(&"conflict_streamed_resolved_output/window/200"));
        assert!(labels.contains(&"conflict_streamed_resolved_output/deep_window_90pct/200"));
    }

    #[test]
    fn perf_budgets_include_repo_switch_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"repo_switch/refocus_same_repo"));
        assert!(labels.contains(&"repo_switch/two_hot_repos"));
        assert!(labels.contains(&"repo_switch/selected_commit_and_details"));
        assert!(labels.contains(&"repo_switch/twenty_tabs"));
        assert!(labels.contains(&"repo_switch/20_repos_all_hot"));
        assert!(labels.contains(&"repo_switch/selected_diff_file"));
        assert!(labels.contains(&"repo_switch/selected_conflict_target"));
        assert!(labels.contains(&"repo_switch/merge_active_with_draft_restore"));
    }

    #[test]
    fn perf_budgets_include_branch_sidebar_extreme_target() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"branch_sidebar/20k_branches_100_remotes"));
    }

    #[test]
    fn perf_budgets_include_branch_sidebar_cache_invalidation_worktrees_ready() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"branch_sidebar/cache_invalidation_worktrees_ready"));
    }

    #[test]
    fn perf_budgets_include_history_load_more_append_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"history_load_more_append/page_500"));
    }

    #[test]
    fn perf_budgets_include_history_scope_switch_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"history_scope_switch/current_branch_to_all_refs"));
    }

    #[test]
    fn perf_budgets_include_status_list_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"status_list/unstaged_large"));
        assert!(labels.contains(&"status_list/staged_large"));
        assert!(labels.contains(&"status_list/20k_entries_mixed_depth"));
    }

    #[test]
    fn perf_budgets_include_status_multi_select_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"status_multi_select/range_select"));
    }

    #[test]
    fn perf_budgets_include_merge_open_bootstrap_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"merge_open_bootstrap/large_streamed"));
        assert!(labels.contains(&"merge_open_bootstrap/many_conflicts"));
        assert!(labels.contains(&"merge_open_bootstrap/50k_lines_500_conflicts_streamed"));
    }

    #[test]
    fn structural_budgets_include_diff_open_patch_first_window_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("diff_open_patch_first_window/200", "rows_materialized")));
        assert!(specs.contains(&("diff_open_patch_first_window/200", "patch_rows_painted")));
        assert!(specs.contains(&(
            "diff_open_patch_first_window/200",
            "patch_page_cache_entries"
        )));
        assert!(specs.contains(&(
            "diff_open_patch_first_window/200",
            "full_text_materializations"
        )));
    }

    #[test]
    fn perf_budgets_include_diff_open_file_preview_and_deep_window_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"diff_open_file_split_first_window/200"));
        assert!(labels.contains(&"diff_open_file_inline_first_window/200"));
        assert!(labels.contains(&"diff_open_image_preview_first_paint"));
        assert!(labels.contains(&"diff_open_patch_deep_window_90pct/200"));
    }

    #[test]
    fn structural_budgets_include_diff_open_file_preview_and_inline_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "diff_open_file_split_first_window/200",
            "split_rows_painted"
        )));
        assert!(specs.contains(&("diff_open_file_split_first_window/200", "split_total_rows")));
        assert!(specs.contains(&(
            "diff_open_file_inline_first_window/200",
            "inline_rows_painted"
        )));
        assert!(specs.contains(&(
            "diff_open_file_inline_first_window/200",
            "inline_total_rows"
        )));
        assert!(specs.contains(&("diff_open_image_preview_first_paint", "old_bytes")));
        assert!(specs.contains(&("diff_open_image_preview_first_paint", "new_bytes")));
        assert!(specs.contains(&("diff_open_image_preview_first_paint", "images_rendered")));
        assert!(specs.contains(&("diff_open_image_preview_first_paint", "placeholder_cells")));
        assert!(specs.contains(&("diff_open_image_preview_first_paint", "divider_count")));
    }

    #[test]
    fn structural_budgets_include_open_repo_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("open_repo/balanced", "commit_count")));
        assert!(specs.contains(&("open_repo/history_heavy", "graph_rows")));
        assert!(specs.contains(&("open_repo/branch_heavy", "remote_branches")));
        assert!(specs.contains(&("open_repo/branch_heavy", "sidebar_rows")));
        assert!(specs.contains(&("open_repo/extreme_metadata_fanout", "worktrees")));
        assert!(specs.contains(&("open_repo/extreme_metadata_fanout", "submodules")));
    }

    #[test]
    fn structural_budgets_include_repo_switch_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("repo_switch/refocus_same_repo", "effect_count")));
        assert!(specs.contains(&(
            "repo_switch/two_hot_repos",
            "selected_diff_reload_effect_count"
        )));
        assert!(specs.contains(&("repo_switch/two_hot_repos", "persist_session_effect_count")));
        assert!(specs.contains(&(
            "repo_switch/selected_commit_and_details",
            "selected_commit_repo_count"
        )));
        assert!(specs.contains(&(
            "repo_switch/selected_commit_and_details",
            "selected_diff_repo_count"
        )));
        assert!(specs.contains(&("repo_switch/twenty_tabs", "repo_count")));
        assert!(specs.contains(&("repo_switch/twenty_tabs", "hydrated_repo_count")));
        assert!(specs.contains(&("repo_switch/20_repos_all_hot", "repo_count")));
        assert!(specs.contains(&("repo_switch/20_repos_all_hot", "selected_diff_repo_count")));
        assert!(specs.contains(&(
            "repo_switch/selected_diff_file",
            "selected_diff_reload_effect_count"
        )));
        assert!(specs.contains(&("repo_switch/selected_diff_file", "selected_diff_repo_count")));
        assert!(specs.contains(&(
            "repo_switch/selected_conflict_target",
            "selected_diff_reload_effect_count"
        )));
        assert!(specs.contains(&("repo_switch/selected_conflict_target", "effect_count")));
        assert!(specs.contains(&(
            "repo_switch/merge_active_with_draft_restore",
            "selected_diff_reload_effect_count"
        )));
        assert!(specs.contains(&(
            "repo_switch/merge_active_with_draft_restore",
            "persist_session_effect_count"
        )));
    }

    #[test]
    fn structural_budgets_include_branch_sidebar_extreme_target() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("branch_sidebar/20k_branches_100_remotes", "remote_branches")));
        assert!(specs.contains(&("branch_sidebar/20k_branches_100_remotes", "remote_headers")));
        assert!(specs.contains(&("branch_sidebar/20k_branches_100_remotes", "sidebar_rows")));
    }

    #[test]
    fn structural_budgets_include_branch_sidebar_cache_invalidation_worktrees_ready() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "branch_sidebar/cache_invalidation_worktrees_ready",
            "cache_hits"
        )));
    }

    #[test]
    fn structural_budgets_include_history_load_more_append_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("history_load_more_append/page_500", "existing_commits")));
        assert!(specs.contains(&("history_load_more_append/page_500", "appended_commits")));
        assert!(specs.contains(&(
            "history_load_more_append/page_500",
            "total_commits_after_append"
        )));
        assert!(specs.contains(&("history_load_more_append/page_500", "log_rev_delta")));
        assert!(specs.contains(&(
            "history_load_more_append/page_500",
            "follow_up_effect_count"
        )));
    }

    #[test]
    fn structural_budgets_include_history_scope_switch_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "history_scope_switch/current_branch_to_all_refs",
            "scope_changed"
        )));
        assert!(specs.contains(&(
            "history_scope_switch/current_branch_to_all_refs",
            "existing_commits"
        )));
        assert!(specs.contains(&(
            "history_scope_switch/current_branch_to_all_refs",
            "log_rev_delta"
        )));
        assert!(specs.contains(&(
            "history_scope_switch/current_branch_to_all_refs",
            "log_set_to_loading"
        )));
        assert!(specs.contains(&(
            "history_scope_switch/current_branch_to_all_refs",
            "load_log_effect_count"
        )));
    }

    #[test]
    fn structural_budgets_include_status_list_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("status_list/unstaged_large", "rows_requested")));
        assert!(specs.contains(&("status_list/unstaged_large", "path_display_cache_misses")));
        assert!(specs.contains(&("status_list/unstaged_large", "path_display_cache_clears")));
        assert!(specs.contains(&("status_list/staged_large", "rows_requested")));
        assert!(specs.contains(&("status_list/staged_large", "path_display_cache_misses")));
        assert!(specs.contains(&("status_list/staged_large", "path_display_cache_clears")));
        assert!(specs.contains(&(
            "status_list/20k_entries_mixed_depth",
            "path_display_cache_clears"
        )));
        assert!(specs.contains(&("status_list/20k_entries_mixed_depth", "max_path_depth")));
        assert!(specs.contains(&("status_list/20k_entries_mixed_depth", "prewarmed_entries")));
    }

    #[test]
    fn structural_budgets_include_status_multi_select_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("status_multi_select/range_select", "entries_total")));
        assert!(specs.contains(&("status_multi_select/range_select", "selected_paths")));
        assert!(specs.contains(&("status_multi_select/range_select", "anchor_preserved")));
        assert!(specs.contains(&("status_multi_select/range_select", "position_scan_steps")));
    }

    #[test]
    fn perf_budgets_include_status_select_diff_open_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"status_select_diff_open/unstaged"));
        assert!(labels.contains(&"status_select_diff_open/staged"));
    }

    #[test]
    fn structural_budgets_include_status_select_diff_open_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("status_select_diff_open/unstaged", "effect_count")));
        assert!(specs.contains(&("status_select_diff_open/unstaged", "load_diff_effect_count")));
        assert!(specs.contains(&(
            "status_select_diff_open/unstaged",
            "load_diff_file_effect_count"
        )));
        assert!(specs.contains(&("status_select_diff_open/unstaged", "diff_state_rev_delta")));
        assert!(specs.contains(&("status_select_diff_open/staged", "effect_count")));
        assert!(specs.contains(&("status_select_diff_open/staged", "load_diff_effect_count")));
        assert!(specs.contains(&(
            "status_select_diff_open/staged",
            "load_diff_file_effect_count"
        )));
        assert!(specs.contains(&("status_select_diff_open/staged", "diff_state_rev_delta")));
    }

    #[test]
    fn structural_budgets_include_merge_open_bootstrap_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        // large_streamed
        assert!(specs.contains(&("merge_open_bootstrap/large_streamed", "trace_event_count")));
        assert!(specs.contains(&(
            "merge_open_bootstrap/large_streamed",
            "rendering_mode_streamed"
        )));
        assert!(specs.contains(&(
            "merge_open_bootstrap/large_streamed",
            "full_output_generated"
        )));
        assert!(specs.contains(&("merge_open_bootstrap/large_streamed", "diff_row_count")));
        assert!(specs.contains(&(
            "merge_open_bootstrap/large_streamed",
            "resolved_output_line_count"
        )));
        // many_conflicts
        assert!(specs.contains(&("merge_open_bootstrap/many_conflicts", "trace_event_count")));
        assert!(specs.contains(&(
            "merge_open_bootstrap/many_conflicts",
            "conflict_block_count"
        )));
        assert!(specs.contains(&(
            "merge_open_bootstrap/many_conflicts",
            "full_output_generated"
        )));
        // 50k_lines_500_conflicts_streamed
        assert!(specs.contains(&(
            "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
            "trace_event_count"
        )));
        assert!(specs.contains(&(
            "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
            "conflict_block_count"
        )));
        assert!(specs.contains(&(
            "merge_open_bootstrap/50k_lines_500_conflicts_streamed",
            "resolved_output_line_count"
        )));
    }

    #[test]
    fn evaluate_structural_budget_reads_sidecar_metrics() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "diff_open_patch_first_window/200",
            &[
                ("rows_materialized", json!(224)),
                ("rows_painted", json!(200)),
                ("patch_page_cache_entries", json!(1)),
                ("full_text_materializations", json!(0)),
            ],
        );
        let spec = StructuralBudgetSpec {
            bench: "diff_open_patch_first_window/200",
            metric: "rows_materialized",
            comparator: StructuralBudgetComparator::AtMost,
            threshold: 256.0,
        };

        let result = evaluate_structural_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.observed, Some(224.0));
        assert!(result.details.contains("satisfies <= 256"));
    }

    #[test]
    fn evaluate_structural_budget_alerts_when_metric_is_missing() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "diff_open_patch_first_window/200",
            &[("rows_painted", json!(200))],
        );
        let spec = StructuralBudgetSpec {
            bench: "diff_open_patch_first_window/200",
            metric: "rows_materialized",
            comparator: StructuralBudgetComparator::AtMost,
            threshold: 256.0,
        };

        let result = evaluate_structural_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::Alert);
        assert!(result.details.contains("missing numeric metric"));
    }

    #[test]
    fn evaluate_structural_budget_alerts_when_launch_allocation_metric_is_missing() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "app_launch/cold_single_repo",
            &[
                ("first_paint_ms", json!(235.0)),
                ("first_interactive_ms", json!(515.0)),
                ("repos_loaded", json!(1)),
            ],
        );
        let spec = StructuralBudgetSpec {
            bench: "app_launch/cold_single_repo",
            metric: "first_paint_alloc_bytes",
            comparator: StructuralBudgetComparator::AtLeast,
            threshold: 0.0,
        };

        let result = evaluate_structural_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::Alert);
        assert!(
            result
                .details
                .contains("not a valid current app_launch baseline")
        );
        assert!(result.details.contains("first_paint_alloc_bytes"));
    }

    #[test]
    fn evaluate_structural_budget_alerts_when_launch_timing_row_uses_timing_only_sidecar() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "app_launch/cold_single_repo",
            &[
                ("first_paint_ms", json!(235.0)),
                ("first_interactive_ms", json!(515.0)),
                ("repos_loaded", json!(1)),
            ],
        );
        let spec = StructuralBudgetSpec {
            bench: "app_launch/cold_single_repo",
            metric: "first_paint_ms",
            comparator: StructuralBudgetComparator::AtMost,
            threshold: 3_000.0,
        };

        let result = evaluate_structural_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::Alert);
        assert_eq!(result.observed, None);
        assert!(
            result
                .details
                .contains("not a valid current app_launch baseline")
        );
        assert!(result.details.contains("first_interactive_alloc_bytes"));
    }

    #[test]
    fn evaluate_structural_budget_accepts_zero_launch_allocation_metric() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        write_sidecar_file(
            temp_dir.path(),
            "app_launch/cold_single_repo",
            &[
                ("first_paint_alloc_bytes", json!(0)),
                ("first_paint_alloc_ops", json!(0)),
                ("first_interactive_alloc_bytes", json!(0)),
                ("first_interactive_alloc_ops", json!(0)),
            ],
        );
        let spec = StructuralBudgetSpec {
            bench: "app_launch/cold_single_repo",
            metric: "first_paint_alloc_bytes",
            comparator: StructuralBudgetComparator::AtLeast,
            threshold: 0.0,
        };

        let result = evaluate_structural_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.observed, Some(0.0));
    }

    #[test]
    fn evaluate_structural_budget_searches_secondary_criterion_root() {
        let first_root = TempDir::new().expect("first root");
        let second_root = TempDir::new().expect("second root");
        let roots = vec![
            first_root.path().to_path_buf(),
            second_root.path().to_path_buf(),
        ];
        write_sidecar_file(
            second_root.path(),
            "diff_open_patch_first_window/200",
            &[("rows_materialized", json!(224))],
        );
        let spec = StructuralBudgetSpec {
            bench: "diff_open_patch_first_window/200",
            metric: "rows_materialized",
            comparator: StructuralBudgetComparator::AtMost,
            threshold: 256.0,
        };

        let result = evaluate_structural_budget(spec, &roots, false, None);

        assert_eq!(result.status, BudgetStatus::WithinBudget);
        assert_eq!(result.observed, Some(224.0));
    }

    #[test]
    fn evaluate_structural_budget_skips_stale_sidecar_with_fresh_reference() {
        let temp_dir = TempDir::new().expect("tempdir");
        let roots = vec![temp_dir.path().to_path_buf()];
        let spec = StructuralBudgetSpec {
            bench: "app_launch/cold_single_repo",
            metric: "first_paint_ms",
            comparator: StructuralBudgetComparator::AtMost,
            threshold: 3000.0,
        };
        write_sidecar_file(
            temp_dir.path(),
            spec.bench,
            &[("first_paint_ms", json!(235.0))],
        );

        let sidecar_path = criterion_sidecar_path(temp_dir.path(), spec.bench);
        let fresh_reference_path = temp_dir.path().join("fresh-reference");
        fs::write(&fresh_reference_path, "stamp").expect("write freshness reference");

        let reference_time = SystemTime::UNIX_EPOCH + Duration::from_secs(2_000_000_000);
        set_file_modified(&sidecar_path, reference_time - Duration::from_secs(60));
        set_file_modified(&fresh_reference_path, reference_time);

        let fresh_reference =
            load_artifact_freshness_reference(&fresh_reference_path).expect("load freshness");
        let result = evaluate_structural_budget(spec, &roots, true, Some(&fresh_reference));

        assert_eq!(result.status, BudgetStatus::Skipped);
        assert!(result.details.contains("stale sidecar file"));
        assert!(result.details.contains("fresh-reference"));
    }

    #[test]
    fn build_report_markdown_uses_generic_view_heading() {
        let roots = [
            PathBuf::from("target/criterion"),
            PathBuf::from("criterion"),
        ];
        let markdown = build_report_markdown(&[], &[], &roots, false, None);
        assert!(markdown.contains("## View Performance Budget Report"));
        assert!(markdown.contains("criterion roots"));
        assert!(markdown.contains("`target/criterion`, `criterion`"));
        assert!(markdown.contains("All tracked view benchmarks are within budget."));
    }

    #[test]
    fn build_report_markdown_reports_when_all_budgets_are_skipped() {
        let roots = [PathBuf::from("target/criterion")];
        let freshness_reference = ArtifactFreshnessReference {
            path: PathBuf::from("/tmp/fresh-reference"),
            modified: SystemTime::UNIX_EPOCH,
        };
        let markdown = build_report_markdown(
            &[BudgetResult {
                spec: PERF_BUDGETS[0],
                status: BudgetStatus::Skipped,
                mean_ns: None,
                mean_upper_ns: None,
                details: "stale estimate file".to_string(),
            }],
            &[],
            &roots,
            true,
            Some(&freshness_reference),
        );

        assert!(markdown.contains("Skipped 1 budget(s)"));
        assert!(markdown.contains("all tracked budgets were skipped"));
        assert!(!markdown.contains("All tracked view benchmarks are within budget."));
    }

    #[test]
    fn build_report_markdown_reports_when_some_budgets_are_skipped() {
        let roots = [PathBuf::from("target/criterion")];
        let markdown = build_report_markdown(
            &[BudgetResult {
                spec: PERF_BUDGETS[0],
                status: BudgetStatus::WithinBudget,
                mean_ns: Some(1.0),
                mean_upper_ns: Some(1.0),
                details: "ok".to_string(),
            }],
            &[StructuralBudgetResult {
                spec: StructuralBudgetSpec {
                    bench: "diff_open_patch_first_window/200",
                    metric: "rows_materialized",
                    comparator: StructuralBudgetComparator::AtMost,
                    threshold: 256.0,
                },
                status: BudgetStatus::Skipped,
                observed: None,
                details: "missing sidecar".to_string(),
            }],
            &roots,
            true,
            None,
        );

        assert!(markdown.contains("All non-skipped tracked view benchmarks are within budget."));
        assert!(!markdown.contains("all tracked budgets were skipped"));
    }

    #[test]
    fn build_report_markdown_includes_structural_budget_table() {
        let roots = [PathBuf::from("target/criterion")];
        let markdown = build_report_markdown(
            &[],
            &[StructuralBudgetResult {
                spec: StructuralBudgetSpec {
                    bench: "diff_open_patch_first_window/200",
                    metric: "rows_materialized",
                    comparator: StructuralBudgetComparator::AtMost,
                    threshold: 256.0,
                },
                status: BudgetStatus::WithinBudget,
                observed: Some(224.0),
                details: "observed 224 satisfies <= 256".to_string(),
            }],
            &roots,
            false,
            None,
        );
        assert!(markdown.contains("### Structural Budgets"));
        assert!(markdown.contains("`diff_open_patch_first_window/200`"));
        assert!(markdown.contains("`rows_materialized`"));
        assert!(markdown.contains("<= 256"));
    }

    #[test]
    fn perf_budgets_include_history_graph_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"history_graph/linear_history"));
        assert!(labels.contains(&"history_graph/merge_dense"));
        assert!(labels.contains(&"history_graph/branch_heads_dense"));
    }

    #[test]
    fn perf_budgets_include_commit_details_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"commit_details/many_files"));
        assert!(labels.contains(&"commit_details/deep_paths"));
        assert!(labels.contains(&"commit_details/huge_file_list"));
        assert!(labels.contains(&"commit_details/large_message_body"));
        assert!(labels.contains(&"commit_details/10k_files_depth_12"));
        assert!(labels.contains(&"commit_details/select_commit_replace"));
        assert!(labels.contains(&"commit_details/path_display_cache_churn"));
    }

    #[test]
    fn perf_budgets_include_patch_diff_paged_rows_targets() {
        let labels = PERF_BUDGETS
            .iter()
            .map(|spec| spec.label)
            .collect::<Vec<_>>();
        assert!(labels.contains(&"patch_diff_paged_rows/eager_full_materialize"));
        assert!(labels.contains(&"patch_diff_paged_rows/paged_first_window/200"));
        assert!(labels.contains(&"patch_diff_paged_rows/inline_visible_eager_scan"));
        assert!(labels.contains(&"patch_diff_paged_rows/inline_visible_hidden_map"));
    }

    #[test]
    fn structural_budgets_include_history_graph_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("history_graph/linear_history", "graph_rows")));
        assert!(specs.contains(&("history_graph/linear_history", "merge_count")));
        assert!(specs.contains(&("history_graph/merge_dense", "merge_count")));
        assert!(specs.contains(&("history_graph/branch_heads_dense", "branch_heads")));
    }

    #[test]
    fn structural_budgets_include_commit_details_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("commit_details/many_files", "file_count")));
        assert!(specs.contains(&("commit_details/many_files", "max_path_depth")));
        assert!(specs.contains(&("commit_details/deep_paths", "max_path_depth")));
        assert!(specs.contains(&("commit_details/huge_file_list", "file_count")));
        assert!(specs.contains(&("commit_details/large_message_body", "message_bytes")));
        assert!(specs.contains(&("commit_details/large_message_body", "message_shaped_lines")));
        assert!(specs.contains(&("commit_details/10k_files_depth_12", "file_count")));
        assert!(specs.contains(&("commit_details/10k_files_depth_12", "max_path_depth")));
        assert!(specs.contains(&("commit_details/select_commit_replace", "commit_ids_differ")));
        assert!(specs.contains(&("commit_details/select_commit_replace", "files_a")));
        assert!(specs.contains(&("commit_details/select_commit_replace", "files_b")));
        assert!(specs.contains(&("commit_details/path_display_cache_churn", "file_count")));
        assert!(specs.contains(&(
            "commit_details/path_display_cache_churn",
            "path_display_cache_clears"
        )));
        assert!(specs.contains(&(
            "commit_details/path_display_cache_churn",
            "path_display_cache_misses"
        )));
    }

    #[test]
    fn timing_budgets_include_resize_drag_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"pane_resize_drag_step/sidebar"));
        assert!(labels.contains(&"pane_resize_drag_step/details"));
        assert!(labels.contains(&"diff_split_resize_drag_step/window_200"));
        assert!(labels.contains(&"window_resize_layout/sidebar_main_details"));
        assert!(labels.contains(&"window_resize_layout/history_50k_commits_diff_20k_lines"));
        assert!(labels.contains(&"history_column_resize_drag_step/branch"));
        assert!(labels.contains(&"history_column_resize_drag_step/graph"));
        assert!(labels.contains(&"history_column_resize_drag_step/author"));
        assert!(labels.contains(&"history_column_resize_drag_step/date"));
        assert!(labels.contains(&"history_column_resize_drag_step/sha"));
        assert!(labels.contains(&"repo_tab_drag/hit_test/20_tabs"));
        assert!(labels.contains(&"repo_tab_drag/hit_test/200_tabs"));
        assert!(labels.contains(&"repo_tab_drag/reorder_reduce/20_tabs"));
        assert!(labels.contains(&"repo_tab_drag/reorder_reduce/200_tabs"));
        assert!(labels.contains(&"scrollbar_drag_step/window_200"));
    }

    #[test]
    fn structural_budgets_include_resize_drag_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("pane_resize_drag_step/sidebar", "steps")));
        assert!(specs.contains(&("pane_resize_drag_step/sidebar", "width_bounds_recomputes")));
        assert!(specs.contains(&("pane_resize_drag_step/sidebar", "layout_recomputes")));
        assert!(specs.contains(&("pane_resize_drag_step/sidebar", "clamp_at_min_count")));
        assert!(specs.contains(&("pane_resize_drag_step/sidebar", "clamp_at_max_count")));
        assert!(specs.contains(&("pane_resize_drag_step/details", "steps")));
        assert!(specs.contains(&("pane_resize_drag_step/details", "width_bounds_recomputes")));
        assert!(specs.contains(&("pane_resize_drag_step/details", "layout_recomputes")));
        assert!(specs.contains(&("pane_resize_drag_step/details", "clamp_at_min_count")));
        assert!(specs.contains(&("pane_resize_drag_step/details", "clamp_at_max_count")));
        assert!(specs.contains(&("diff_split_resize_drag_step/window_200", "steps")));
        assert!(specs.contains(&("diff_split_resize_drag_step/window_200", "ratio_recomputes")));
        assert!(specs.contains(&(
            "diff_split_resize_drag_step/window_200",
            "column_width_recomputes"
        )));
        assert!(specs.contains(&(
            "diff_split_resize_drag_step/window_200",
            "clamp_at_min_count"
        )));
        assert!(specs.contains(&(
            "diff_split_resize_drag_step/window_200",
            "clamp_at_max_count"
        )));
        assert!(specs.contains(&(
            "diff_split_resize_drag_step/window_200",
            "narrow_fallback_count"
        )));
        assert!(specs.contains(&("diff_split_resize_drag_step/window_200", "min_ratio")));
        assert!(specs.contains(&("diff_split_resize_drag_step/window_200", "max_ratio")));
        assert!(specs.contains(&("window_resize_layout/sidebar_main_details", "steps")));
        assert!(specs.contains(&(
            "window_resize_layout/sidebar_main_details",
            "layout_recomputes"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/sidebar_main_details",
            "clamp_at_zero_count"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "steps"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "layout_recomputes"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "history_visibility_recomputes"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "diff_width_recomputes"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "history_commits"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "history_rows_processed_total"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "history_columns_hidden_steps"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "history_all_columns_visible_steps"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "diff_lines"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "diff_rows_processed_total"
        )));
        assert!(specs.contains(&(
            "window_resize_layout/history_50k_commits_diff_20k_lines",
            "diff_narrow_fallback_steps"
        )));
        assert!(specs.contains(&("history_column_resize_drag_step/branch", "steps")));
        assert!(specs.contains(&(
            "history_column_resize_drag_step/branch",
            "width_clamp_recomputes"
        )));
        assert!(specs.contains(&(
            "history_column_resize_drag_step/branch",
            "visible_column_recomputes"
        )));
        assert!(specs.contains(&(
            "history_column_resize_drag_step/branch",
            "clamp_at_max_count"
        )));
        assert!(specs.contains(&("repo_tab_drag/hit_test/20_tabs", "tab_count")));
        assert!(specs.contains(&("repo_tab_drag/hit_test/20_tabs", "hit_test_steps")));
        assert!(specs.contains(&("repo_tab_drag/hit_test/200_tabs", "tab_count")));
        assert!(specs.contains(&("repo_tab_drag/hit_test/200_tabs", "hit_test_steps")));
        assert!(specs.contains(&("repo_tab_drag/reorder_reduce/20_tabs", "reorder_steps")));
        assert!(specs.contains(&("repo_tab_drag/reorder_reduce/200_tabs", "effects_emitted")));
        assert!(specs.contains(&("repo_tab_drag/reorder_reduce/200_tabs", "reorder_steps")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "steps")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "thumb_metric_recomputes")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "scroll_offset_recomputes")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "viewport_h")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "clamp_at_top_count")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "clamp_at_bottom_count")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "min_scroll_y")));
        assert!(specs.contains(&("scrollbar_drag_step/window_200", "max_scroll_y")));
    }

    #[test]
    fn timing_budgets_include_frame_timing_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"frame_timing/continuous_scroll_history_list"));
        assert!(labels.contains(&"frame_timing/continuous_scroll_large_diff"));
        assert!(labels.contains(&"frame_timing/sidebar_resize_drag_sustained"));
        assert!(labels.contains(&"frame_timing/rapid_commit_selection_changes"));
        assert!(labels.contains(&"frame_timing/repo_switch_during_scroll"));
    }

    #[test]
    fn timing_budgets_include_keyboard_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"keyboard/arrow_scroll_history_sustained_repeat"));
        assert!(labels.contains(&"keyboard/arrow_scroll_diff_sustained_repeat"));
        assert!(labels.contains(&"keyboard/tab_focus_cycle_all_panes"));
        assert!(labels.contains(&"keyboard/stage_unstage_toggle_rapid"));
    }

    #[test]
    fn structural_budgets_include_frame_timing_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("frame_timing/continuous_scroll_history_list", "frame_count")));
        assert!(specs.contains(&("frame_timing/continuous_scroll_history_list", "total_rows")));
        assert!(specs.contains(&("frame_timing/continuous_scroll_history_list", "window_rows")));
        assert!(specs.contains(&(
            "frame_timing/continuous_scroll_history_list",
            "scroll_step_rows"
        )));
        assert!(specs.contains(&(
            "frame_timing/continuous_scroll_history_list",
            "p99_exceeds_2x_budget"
        )));
        assert!(specs.contains(&("frame_timing/continuous_scroll_large_diff", "frame_count")));
        assert!(specs.contains(&("frame_timing/continuous_scroll_large_diff", "total_rows")));
        assert!(specs.contains(&("frame_timing/continuous_scroll_large_diff", "window_rows")));
        assert!(specs.contains(&(
            "frame_timing/continuous_scroll_large_diff",
            "scroll_step_rows"
        )));
        assert!(specs.contains(&(
            "frame_timing/continuous_scroll_large_diff",
            "p99_exceeds_2x_budget"
        )));
        // sidebar_resize_drag_sustained
        assert!(specs.contains(&("frame_timing/sidebar_resize_drag_sustained", "frame_count")));
        assert!(specs.contains(&("frame_timing/sidebar_resize_drag_sustained", "frames")));
        assert!(specs.contains(&(
            "frame_timing/sidebar_resize_drag_sustained",
            "steps_per_frame"
        )));
        assert!(specs.contains(&(
            "frame_timing/sidebar_resize_drag_sustained",
            "p99_exceeds_2x_budget"
        )));
        // rapid_commit_selection_changes
        assert!(specs.contains(&("frame_timing/rapid_commit_selection_changes", "frame_count")));
        assert!(specs.contains(&(
            "frame_timing/rapid_commit_selection_changes",
            "commit_count"
        )));
        assert!(specs.contains(&(
            "frame_timing/rapid_commit_selection_changes",
            "files_per_commit"
        )));
        assert!(specs.contains(&("frame_timing/rapid_commit_selection_changes", "selections")));
        assert!(specs.contains(&(
            "frame_timing/rapid_commit_selection_changes",
            "p99_exceeds_2x_budget"
        )));
        // repo_switch_during_scroll
        assert!(specs.contains(&("frame_timing/repo_switch_during_scroll", "frame_count")));
        assert!(specs.contains(&("frame_timing/repo_switch_during_scroll", "total_frames")));
        assert!(specs.contains(&("frame_timing/repo_switch_during_scroll", "scroll_frames")));
        assert!(specs.contains(&("frame_timing/repo_switch_during_scroll", "switch_frames")));
        assert!(specs.contains(&("frame_timing/repo_switch_during_scroll", "total_rows")));
        assert!(specs.contains(&("frame_timing/repo_switch_during_scroll", "window_rows")));
        assert!(specs.contains(&(
            "frame_timing/repo_switch_during_scroll",
            "p99_exceeds_2x_budget"
        )));
    }

    #[test]
    fn structural_budgets_include_keyboard_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_history_sustained_repeat",
            "frame_count"
        )));
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_history_sustained_repeat",
            "repeat_events"
        )));
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_history_sustained_repeat",
            "rows_requested_total"
        )));
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_history_sustained_repeat",
            "p99_exceeds_2x_budget"
        )));
        assert!(specs.contains(&("keyboard/arrow_scroll_diff_sustained_repeat", "frame_count")));
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_diff_sustained_repeat",
            "repeat_events"
        )));
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_diff_sustained_repeat",
            "rows_requested_total"
        )));
        assert!(specs.contains(&(
            "keyboard/arrow_scroll_diff_sustained_repeat",
            "p99_exceeds_2x_budget"
        )));
        assert!(specs.contains(&("keyboard/tab_focus_cycle_all_panes", "frame_count")));
        assert!(specs.contains(&("keyboard/tab_focus_cycle_all_panes", "focus_target_count")));
        assert!(specs.contains(&("keyboard/tab_focus_cycle_all_panes", "cycle_events")));
        assert!(specs.contains(&("keyboard/tab_focus_cycle_all_panes", "wrap_count")));
        assert!(specs.contains(&(
            "keyboard/tab_focus_cycle_all_panes",
            "p99_exceeds_2x_budget"
        )));
        assert!(specs.contains(&("keyboard/stage_unstage_toggle_rapid", "frame_count")));
        assert!(specs.contains(&("keyboard/stage_unstage_toggle_rapid", "toggle_events")));
        assert!(specs.contains(&("keyboard/stage_unstage_toggle_rapid", "effect_count")));
        assert!(specs.contains(&("keyboard/stage_unstage_toggle_rapid", "ops_rev_delta")));
        assert!(specs.contains(&(
            "keyboard/stage_unstage_toggle_rapid",
            "p99_exceeds_2x_budget"
        )));
    }

    #[test]
    fn timing_budgets_include_staging_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"staging/stage_all_10k_files"));
        assert!(labels.contains(&"staging/unstage_all_10k_files"));
        assert!(labels.contains(&"staging/stage_unstage_interleaved_1k_files"));
    }

    #[test]
    fn structural_budgets_include_staging_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("staging/stage_all_10k_files", "file_count")));
        assert!(specs.contains(&("staging/stage_all_10k_files", "effect_count")));
        assert!(specs.contains(&("staging/stage_all_10k_files", "stage_effect_count")));
        assert!(specs.contains(&("staging/stage_all_10k_files", "ops_rev_delta")));
        assert!(specs.contains(&("staging/unstage_all_10k_files", "file_count")));
        assert!(specs.contains(&("staging/unstage_all_10k_files", "effect_count")));
        assert!(specs.contains(&("staging/unstage_all_10k_files", "unstage_effect_count")));
        assert!(specs.contains(&("staging/unstage_all_10k_files", "ops_rev_delta")));
        assert!(specs.contains(&("staging/stage_unstage_interleaved_1k_files", "file_count")));
        assert!(specs.contains(&("staging/stage_unstage_interleaved_1k_files", "effect_count")));
        assert!(specs.contains(&(
            "staging/stage_unstage_interleaved_1k_files",
            "stage_effect_count"
        )));
        assert!(specs.contains(&(
            "staging/stage_unstage_interleaved_1k_files",
            "unstage_effect_count"
        )));
        assert!(specs.contains(&(
            "staging/stage_unstage_interleaved_1k_files",
            "ops_rev_delta"
        )));
    }

    #[test]
    fn timing_budgets_include_undo_redo_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"undo_redo/conflict_resolution_deep_stack"));
        assert!(labels.contains(&"undo_redo/conflict_resolution_undo_replay_50_steps"));
    }

    #[test]
    fn structural_budgets_include_undo_redo_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("undo_redo/conflict_resolution_deep_stack", "region_count")));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_deep_stack",
            "apply_dispatches"
        )));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_deep_stack",
            "conflict_rev_delta"
        )));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_undo_replay_50_steps",
            "region_count"
        )));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_undo_replay_50_steps",
            "apply_dispatches"
        )));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_undo_replay_50_steps",
            "reset_dispatches"
        )));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_undo_replay_50_steps",
            "replay_dispatches"
        )));
        assert!(specs.contains(&(
            "undo_redo/conflict_resolution_undo_replay_50_steps",
            "conflict_rev_delta"
        )));
    }

    #[test]
    fn timing_budgets_include_clipboard_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"clipboard/copy_10k_lines_from_diff"));
        assert!(labels.contains(&"clipboard/paste_large_text_into_commit_message"));
        assert!(labels.contains(&"clipboard/select_range_5k_lines_in_diff"));
    }

    #[test]
    fn structural_budgets_include_clipboard_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("clipboard/copy_10k_lines_from_diff", "total_lines")));
        assert!(specs.contains(&("clipboard/copy_10k_lines_from_diff", "line_iterations")));
        assert!(specs.contains(&("clipboard/copy_10k_lines_from_diff", "total_bytes")));
        assert!(specs.contains(&(
            "clipboard/paste_large_text_into_commit_message",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "clipboard/paste_large_text_into_commit_message",
            "total_bytes"
        )));
        assert!(specs.contains(&(
            "clipboard/paste_large_text_into_commit_message",
            "line_iterations"
        )));
        assert!(specs.contains(&("clipboard/select_range_5k_lines_in_diff", "total_lines")));
        assert!(specs.contains(&("clipboard/select_range_5k_lines_in_diff", "line_iterations")));
        assert!(specs.contains(&("clipboard/select_range_5k_lines_in_diff", "total_bytes")));
    }

    #[test]
    fn timing_budgets_include_git_ops_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"git_ops/status_dirty_500_files"));
        assert!(labels.contains(&"git_ops/log_walk_10k_commits"));
        assert!(labels.contains(&"git_ops/log_walk_100k_commits_shallow"));
        assert!(labels.contains(&"git_ops/diff_rename_heavy"));
        assert!(labels.contains(&"git_ops/diff_binary_heavy"));
        assert!(labels.contains(&"git_ops/diff_large_single_file_100k_lines"));
        assert!(labels.contains(&"git_ops/blame_large_file"));
        assert!(labels.contains(&"git_ops/file_history_first_page_sparse_100k_commits"));
    }

    #[test]
    fn structural_budgets_include_git_ops_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("git_ops/status_dirty_500_files", "tracked_files")));
        assert!(specs.contains(&("git_ops/status_dirty_500_files", "dirty_files")));
        assert!(specs.contains(&("git_ops/status_dirty_500_files", "status_calls")));
        assert!(specs.contains(&("git_ops/status_dirty_500_files", "log_walk_calls")));
        assert!(specs.contains(&("git_ops/log_walk_10k_commits", "total_commits")));
        assert!(specs.contains(&("git_ops/log_walk_10k_commits", "requested_commits")));
        assert!(specs.contains(&("git_ops/log_walk_10k_commits", "commits_returned")));
        assert!(specs.contains(&("git_ops/log_walk_10k_commits", "log_walk_calls")));
        assert!(specs.contains(&("git_ops/log_walk_10k_commits", "status_calls")));
        assert!(specs.contains(&("git_ops/log_walk_100k_commits_shallow", "requested_commits")));
        assert!(specs.contains(&("git_ops/diff_rename_heavy", "renamed_files")));
        assert!(specs.contains(&("git_ops/diff_binary_heavy", "binary_files")));
        assert!(specs.contains(&("git_ops/diff_large_single_file_100k_lines", "line_count")));
        assert!(specs.contains(&("git_ops/blame_large_file", "blame_lines")));
        assert!(specs.contains(&(
            "git_ops/file_history_first_page_sparse_100k_commits",
            "file_history_commits"
        )));
    }

    #[test]
    fn structural_budgets_include_app_launch_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        let launch_benches = [
            "app_launch/cold_empty_workspace",
            "app_launch/cold_single_repo",
            "app_launch/cold_five_repos",
            "app_launch/cold_twenty_repos",
            "app_launch/warm_single_repo",
            "app_launch/warm_twenty_repos",
        ];
        let required_metrics = [
            "first_paint_ms",
            "first_interactive_ms",
            "first_paint_alloc_ops",
            "first_paint_alloc_bytes",
            "first_interactive_alloc_ops",
            "first_interactive_alloc_bytes",
            "repos_loaded",
        ];

        for bench in launch_benches {
            for metric in required_metrics {
                assert!(
                    specs.contains(&(bench, metric)),
                    "missing app_launch structural budget for {bench} {metric}"
                );
            }
        }
    }

    #[test]
    fn timing_budgets_include_idle_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"idle/background_refresh_cost_per_cycle"));
        assert!(labels.contains(&"idle/wake_from_sleep_resume"));
    }

    #[test]
    fn structural_budgets_include_idle_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("idle/cpu_usage_single_repo_60s", "open_repos")));
        assert!(specs.contains(&("idle/cpu_usage_single_repo_60s", "sample_count")));
        assert!(specs.contains(&("idle/cpu_usage_single_repo_60s", "avg_cpu_pct")));
        assert!(specs.contains(&("idle/cpu_usage_ten_repos_60s", "open_repos")));
        assert!(specs.contains(&("idle/cpu_usage_ten_repos_60s", "rss_delta_kib")));
        assert!(specs.contains(&("idle/memory_growth_single_repo_10min", "sample_duration_ms")));
        assert!(specs.contains(&("idle/memory_growth_ten_repos_10min", "rss_delta_kib")));
        assert!(specs.contains(&("idle/background_refresh_cost_per_cycle", "refresh_cycles")));
        assert!(specs.contains(&("idle/background_refresh_cost_per_cycle", "status_calls")));
        assert!(specs.contains(&("idle/wake_from_sleep_resume", "wake_resume_ms")));
        assert!(specs.contains(&("idle/wake_from_sleep_resume", "repos_refreshed")));
    }

    #[test]
    fn timing_budgets_include_search_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"search/commit_filter_by_author_50k_commits"));
        assert!(labels.contains(&"search/commit_filter_by_message_50k_commits"));
        assert!(labels.contains(&"search/in_diff_text_search_100k_lines"));
        assert!(labels.contains(&"search/in_diff_text_search_incremental_refinement"));
        assert!(labels.contains(&"search/file_preview_text_search_100k_lines"));
        assert!(labels.contains(&"search/file_fuzzy_find_100k_files"));
        assert!(labels.contains(&"search/file_fuzzy_find_incremental_keystroke"));
    }

    #[test]
    fn structural_budgets_include_search_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "search/commit_filter_by_author_50k_commits",
            "total_commits"
        )));
        assert!(specs.contains(&(
            "search/commit_filter_by_author_50k_commits",
            "matches_found"
        )));
        assert!(specs.contains(&(
            "search/commit_filter_by_author_50k_commits",
            "incremental_matches"
        )));
        assert!(specs.contains(&(
            "search/commit_filter_by_message_50k_commits",
            "total_commits"
        )));
        assert!(specs.contains(&(
            "search/commit_filter_by_message_50k_commits",
            "matches_found"
        )));
        assert!(specs.contains(&(
            "search/commit_filter_by_message_50k_commits",
            "incremental_matches"
        )));
        assert!(specs.contains(&("search/in_diff_text_search_100k_lines", "total_lines")));
        assert!(specs.contains(&(
            "search/in_diff_text_search_100k_lines",
            "visible_rows_scanned"
        )));
        assert!(specs.contains(&("search/in_diff_text_search_100k_lines", "matches_found")));
        assert!(specs.contains(&(
            "search/in_diff_text_search_incremental_refinement",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "search/in_diff_text_search_incremental_refinement",
            "visible_rows_scanned"
        )));
        assert!(specs.contains(&(
            "search/in_diff_text_search_incremental_refinement",
            "prior_matches"
        )));
        assert!(specs.contains(&(
            "search/in_diff_text_search_incremental_refinement",
            "matches_found"
        )));
        assert!(specs.contains(&("search/file_preview_text_search_100k_lines", "total_lines")));
        assert!(specs.contains(&("search/file_preview_text_search_100k_lines", "source_bytes")));
        assert!(specs.contains(&(
            "search/file_preview_text_search_100k_lines",
            "matches_found"
        )));
        // file_fuzzy_find structural budgets
        assert!(specs.contains(&("search/file_fuzzy_find_100k_files", "total_files")));
        assert!(specs.contains(&("search/file_fuzzy_find_100k_files", "matches_found")));
        assert!(specs.contains(&("search/file_fuzzy_find_100k_files", "query_len")));
        assert!(specs.contains(&(
            "search/file_fuzzy_find_incremental_keystroke",
            "total_files"
        )));
        assert!(specs.contains(&(
            "search/file_fuzzy_find_incremental_keystroke",
            "prior_matches"
        )));
        assert!(specs.contains(&(
            "search/file_fuzzy_find_incremental_keystroke",
            "matches_found"
        )));
    }

    #[test]
    fn timing_budgets_include_fs_event_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"fs_event/single_file_save_to_status_update"));
        assert!(labels.contains(&"fs_event/git_checkout_200_files_to_status_update"));
        assert!(labels.contains(&"fs_event/rapid_saves_debounce_coalesce"));
        assert!(labels.contains(&"fs_event/false_positive_rate_under_churn"));
    }

    #[test]
    fn structural_budgets_include_fs_event_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        // single_file_save
        assert!(specs.contains(&(
            "fs_event/single_file_save_to_status_update",
            "tracked_files"
        )));
        assert!(specs.contains(&(
            "fs_event/single_file_save_to_status_update",
            "mutation_files"
        )));
        assert!(specs.contains(&(
            "fs_event/single_file_save_to_status_update",
            "dirty_files_detected"
        )));
        assert!(specs.contains(&("fs_event/single_file_save_to_status_update", "status_calls")));
        // git_checkout_200_files
        assert!(specs.contains(&(
            "fs_event/git_checkout_200_files_to_status_update",
            "tracked_files"
        )));
        assert!(specs.contains(&(
            "fs_event/git_checkout_200_files_to_status_update",
            "mutation_files"
        )));
        assert!(specs.contains(&(
            "fs_event/git_checkout_200_files_to_status_update",
            "dirty_files_detected"
        )));
        assert!(specs.contains(&(
            "fs_event/git_checkout_200_files_to_status_update",
            "status_calls"
        )));
        // rapid_saves_debounce
        assert!(specs.contains(&("fs_event/rapid_saves_debounce_coalesce", "coalesced_saves")));
        assert!(specs.contains(&(
            "fs_event/rapid_saves_debounce_coalesce",
            "dirty_files_detected"
        )));
        assert!(specs.contains(&("fs_event/rapid_saves_debounce_coalesce", "status_calls")));
        // false_positive_under_churn
        assert!(specs.contains(&("fs_event/false_positive_rate_under_churn", "mutation_files")));
        assert!(specs.contains(&(
            "fs_event/false_positive_rate_under_churn",
            "dirty_files_detected"
        )));
        assert!(specs.contains(&(
            "fs_event/false_positive_rate_under_churn",
            "false_positives"
        )));
        assert!(specs.contains(&("fs_event/false_positive_rate_under_churn", "status_calls")));
    }

    #[test]
    fn timing_budgets_include_network_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"network/ui_responsiveness_during_fetch"));
        assert!(labels.contains(&"network/progress_bar_update_render_cost"));
        assert!(labels.contains(&"network/cancel_operation_latency"));
    }

    #[test]
    fn structural_budgets_include_network_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("network/ui_responsiveness_during_fetch", "frame_count")));
        assert!(specs.contains(&("network/ui_responsiveness_during_fetch", "scroll_frames")));
        assert!(specs.contains(&("network/ui_responsiveness_during_fetch", "progress_updates")));
        assert!(specs.contains(&("network/ui_responsiveness_during_fetch", "window_rows")));
        assert!(specs.contains(&("network/ui_responsiveness_during_fetch", "tail_trim_events")));
        assert!(specs.contains(&("network/progress_bar_update_render_cost", "frame_count")));
        assert!(specs.contains(&("network/progress_bar_update_render_cost", "render_passes")));
        assert!(specs.contains(&("network/progress_bar_update_render_cost", "bar_width")));
        assert!(specs.contains(&(
            "network/progress_bar_update_render_cost",
            "output_tail_lines"
        )));
        assert!(specs.contains(&("network/cancel_operation_latency", "frame_count")));
        assert!(specs.contains(&(
            "network/cancel_operation_latency",
            "cancel_frames_until_stopped"
        )));
        assert!(specs.contains(&(
            "network/cancel_operation_latency",
            "drained_updates_after_cancel"
        )));
        assert!(specs.contains(&("network/cancel_operation_latency", "output_tail_lines")));
    }

    #[test]
    fn timing_budgets_include_display_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"display/render_cost_1x_vs_2x_vs_3x_scale"));
        assert!(labels.contains(&"display/two_windows_same_repo"));
        assert!(labels.contains(&"display/window_move_between_dpis"));
    }

    #[test]
    fn structural_budgets_include_display_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "display/render_cost_1x_vs_2x_vs_3x_scale",
            "scale_factors_tested"
        )));
        assert!(specs.contains(&(
            "display/render_cost_1x_vs_2x_vs_3x_scale",
            "total_layout_passes"
        )));
        assert!(specs.contains(&(
            "display/render_cost_1x_vs_2x_vs_3x_scale",
            "windows_rendered"
        )));
        assert!(specs.contains(&(
            "display/render_cost_1x_vs_2x_vs_3x_scale",
            "history_rows_per_pass"
        )));
        assert!(specs.contains(&(
            "display/render_cost_1x_vs_2x_vs_3x_scale",
            "diff_rows_per_pass"
        )));
        assert!(specs.contains(&("display/two_windows_same_repo", "windows_rendered")));
        assert!(specs.contains(&("display/two_windows_same_repo", "total_layout_passes")));
        assert!(specs.contains(&("display/two_windows_same_repo", "total_rows_rendered")));
        assert!(specs.contains(&("display/two_windows_same_repo", "history_rows_per_pass")));
        assert!(specs.contains(&("display/two_windows_same_repo", "diff_rows_per_pass")));
        assert!(specs.contains(&("display/window_move_between_dpis", "scale_factors_tested")));
        assert!(specs.contains(&("display/window_move_between_dpis", "re_layout_passes")));
        assert!(specs.contains(&("display/window_move_between_dpis", "total_layout_passes")));
        assert!(specs.contains(&("display/window_move_between_dpis", "windows_rendered")));
    }

    #[test]
    fn timing_budgets_include_real_repo_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"real_repo/monorepo_open_and_history_load"));
        assert!(labels.contains(&"real_repo/deep_history_open_and_scroll"));
        assert!(labels.contains(&"real_repo/mid_merge_conflict_list_and_open"));
        assert!(labels.contains(&"real_repo/large_file_diff_open"));
    }

    #[test]
    fn structural_budgets_include_real_repo_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "real_repo/monorepo_open_and_history_load",
            "worktree_file_count"
        )));
        assert!(specs.contains(&("real_repo/monorepo_open_and_history_load", "commits_loaded")));
        assert!(specs.contains(&(
            "real_repo/monorepo_open_and_history_load",
            "ref_enumerate_calls"
        )));
        assert!(specs.contains(&(
            "real_repo/deep_history_open_and_scroll",
            "history_windows_scanned"
        )));
        assert!(specs.contains(&("real_repo/deep_history_open_and_scroll", "log_pages_loaded")));
        assert!(specs.contains(&(
            "real_repo/mid_merge_conflict_list_and_open",
            "conflict_files"
        )));
        assert!(specs.contains(&(
            "real_repo/mid_merge_conflict_list_and_open",
            "selected_conflict_bytes"
        )));
        assert!(specs.contains(&("real_repo/large_file_diff_open", "diff_lines")));
        assert!(specs.contains(&("real_repo/large_file_diff_open", "split_rows_painted")));
        assert!(specs.contains(&("real_repo/large_file_diff_open", "inline_rows_painted")));
    }

    #[test]
    fn timing_budgets_include_diff_open_patch_first_window() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"diff_open_patch_first_window/200"));
    }

    #[test]
    fn timing_budgets_include_pre_existing_diff_scroll_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"diff_scroll/normal_lines_window/200"));
        assert!(labels.contains(&"diff_scroll/long_lines_window/200"));
        assert!(labels.contains(&"patch_diff_search_query_update/window_200"));
    }

    #[test]
    fn timing_budgets_include_pre_existing_file_diff_alignment_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"file_diff_replacement_alignment/balanced_blocks/scratch"));
        assert!(labels.contains(&"file_diff_replacement_alignment/balanced_blocks/strsim"));
        assert!(labels.contains(&"file_diff_replacement_alignment/skewed_blocks/scratch"));
        assert!(labels.contains(&"file_diff_replacement_alignment/skewed_blocks/strsim"));
    }

    #[test]
    fn timing_budgets_include_pre_existing_text_input_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"text_input_prepaint_windowed/window_rows/80"));
        assert!(labels.contains(&"text_input_prepaint_windowed/full_document_control"));
        assert!(labels.contains(&"text_input_runs_streamed_highlight_dense/legacy_scan"));
        assert!(labels.contains(&"text_input_runs_streamed_highlight_dense/streamed_cursor"));
        assert!(labels.contains(&"text_input_runs_streamed_highlight_sparse/legacy_scan"));
        assert!(labels.contains(&"text_input_runs_streamed_highlight_sparse/streamed_cursor"));
        assert!(labels.contains(&"text_input_long_line_cap/capped_bytes/4096"));
        assert!(labels.contains(&"text_input_long_line_cap/uncapped_control"));
        assert!(labels.contains(&"text_input_wrap_incremental_tabs/full_recompute"));
        assert!(labels.contains(&"text_input_wrap_incremental_tabs/incremental_patch"));
        assert!(labels.contains(&"text_input_wrap_incremental_burst_edits/full_recompute/12"));
        assert!(labels.contains(&"text_input_wrap_incremental_burst_edits/incremental_patch/12"));
    }

    #[test]
    fn timing_budgets_include_pre_existing_text_model_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192"));
        assert!(
            labels.contains(&"text_model_snapshot_clone_cost/shared_string_clone_control/8192")
        );
        assert!(labels.contains(&"text_model_bulk_load_large/piece_table_append_large"));
        assert!(labels.contains(&"text_model_bulk_load_large/piece_table_from_large_text"));
        assert!(labels.contains(&"text_model_bulk_load_large/string_push_control"));
        assert!(labels.contains(&"text_model_fragmented_edits/piece_table_edits"));
        assert!(labels.contains(&"text_model_fragmented_edits/materialize_after_edits"));
        assert!(labels.contains(&"text_model_fragmented_edits/shared_string_after_edits/64"));
        assert!(labels.contains(&"text_model_fragmented_edits/string_edit_control"));
    }

    #[test]
    fn timing_budgets_include_pre_existing_syntax_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"file_diff_syntax_prepare/file_diff_syntax_prepare_cold"));
        assert!(labels.contains(&"file_diff_syntax_prepare/file_diff_syntax_prepare_warm"));
        assert!(labels.contains(&"file_diff_syntax_query_stress/nested_long_lines_cold"));
        assert!(labels.contains(&"file_diff_syntax_reparse/file_diff_syntax_reparse_small_edit"));
        assert!(labels.contains(&"file_diff_syntax_reparse/file_diff_syntax_reparse_large_edit"));
        assert!(labels.contains(&"file_diff_inline_syntax_projection/visible_window_pending/200"));
        assert!(labels.contains(&"file_diff_inline_syntax_projection/visible_window_ready/200"));
        assert!(labels.contains(&"file_diff_syntax_cache_drop/deferred_drop/4"));
        assert!(labels.contains(&"file_diff_syntax_cache_drop/inline_drop_control/4"));
        assert!(labels.contains(&"prepared_syntax_multidoc_cache_hit_rate/hot_docs/6"));
        assert!(labels.contains(&"prepared_syntax_chunk_miss_cost/chunk_miss"));
    }

    #[test]
    fn timing_budgets_include_pre_existing_large_html_syntax_targets() {
        let specs: Vec<(&str, &str)> = PERF_BUDGETS
            .iter()
            .filter(|spec| spec.label.starts_with("large_html_syntax/"))
            .map(|spec| (spec.label, spec.estimate_path))
            .collect();
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/background_prepare",
            "large_html_syntax/synthetic_html_fixture/background_prepare/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_pending/160",
            "large_html_syntax/synthetic_html_fixture/visible_window_pending/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_steady/160",
            "large_html_syntax/synthetic_html_fixture/visible_window_steady/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_sweep/160",
            "large_html_syntax/synthetic_html_fixture/visible_window_sweep/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/external_html_fixture/background_prepare",
            "large_html_syntax/external_html_fixture/background_prepare/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/external_html_fixture/visible_window_pending/160",
            "large_html_syntax/external_html_fixture/visible_window_pending/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/external_html_fixture/visible_window_steady/160",
            "large_html_syntax/external_html_fixture/visible_window_steady/new/estimates.json"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/external_html_fixture/visible_window_sweep/160",
            "large_html_syntax/external_html_fixture/visible_window_sweep/new/estimates.json"
        )));
    }

    #[test]
    fn structural_budgets_include_large_html_syntax_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/background_prepare",
            "line_count"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/background_prepare",
            "prepared_document_available"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_pending",
            "cache_hits"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_pending",
            "cache_misses"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_steady",
            "cache_document_present"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_steady",
            "pending"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
            "start_line"
        )));
        assert!(specs.contains(&(
            "large_html_syntax/synthetic_html_fixture/visible_window_sweep",
            "cache_hits"
        )));
    }

    #[test]
    fn timing_budgets_include_pre_existing_worktree_preview_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"worktree_preview_render/cached_lookup_window/200"));
        assert!(labels.contains(&"worktree_preview_render/render_time_prepare_window/200"));
    }

    #[test]
    fn structural_budgets_include_diff_scroll_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("diff_scroll/normal_lines_window/200", "total_lines")));
        assert!(specs.contains(&("diff_scroll/normal_lines_window/200", "visible_text_bytes")));
        assert!(specs.contains(&("diff_scroll/normal_lines_window/200", "min_line_bytes")));
        assert!(specs.contains(&("diff_scroll/long_lines_window/200", "total_lines")));
        assert!(specs.contains(&("diff_scroll/long_lines_window/200", "visible_text_bytes")));
        assert!(specs.contains(&("diff_scroll/long_lines_window/200", "min_line_bytes")));
    }

    #[test]
    fn structural_budgets_include_worktree_preview_render_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "worktree_preview_render/cached_lookup_window/200",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "worktree_preview_render/cached_lookup_window/200",
            "window_size"
        )));
        assert!(specs.contains(&(
            "worktree_preview_render/cached_lookup_window/200",
            "prepared_document_available"
        )));
        assert!(specs.contains(&(
            "worktree_preview_render/cached_lookup_window/200",
            "syntax_mode_auto"
        )));
        assert!(specs.contains(&(
            "worktree_preview_render/render_time_prepare_window/200",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "worktree_preview_render/render_time_prepare_window/200",
            "prepared_document_available"
        )));
        assert!(specs.contains(&(
            "worktree_preview_render/render_time_prepare_window/200",
            "syntax_mode_auto"
        )));
    }

    #[test]
    fn structural_budgets_include_text_input_prepaint_windowed_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        // windowed variant
        assert!(specs.contains(&("text_input_prepaint_windowed/window_rows/80", "total_lines")));
        assert!(specs.contains(&(
            "text_input_prepaint_windowed/window_rows/80",
            "viewport_rows"
        )));
        assert!(specs.contains(&(
            "text_input_prepaint_windowed/window_rows/80",
            "cache_entries_after"
        )));
        assert!(specs.contains(&("text_input_prepaint_windowed/window_rows/80", "cache_hits")));
        assert!(specs.contains(&(
            "text_input_prepaint_windowed/window_rows/80",
            "cache_misses"
        )));
        // full-document variant
        assert!(specs.contains(&(
            "text_input_prepaint_windowed/full_document_control",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "text_input_prepaint_windowed/full_document_control",
            "cache_entries_after"
        )));
        assert!(specs.contains(&(
            "text_input_prepaint_windowed/full_document_control",
            "cache_misses"
        )));
    }

    #[test]
    fn structural_budgets_include_text_input_runs_streamed_highlight_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_dense/legacy_scan",
            "visible_lines_with_highlights"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_dense/legacy_scan",
            "algorithm_streamed"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_dense/streamed_cursor",
            "visible_lines_with_highlights"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_dense/streamed_cursor",
            "algorithm_streamed"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_sparse/legacy_scan",
            "visible_highlights"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_sparse/legacy_scan",
            "total_highlights"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_sparse/streamed_cursor",
            "visible_highlights"
        )));
        assert!(specs.contains(&(
            "text_input_runs_streamed_highlight_sparse/streamed_cursor",
            "algorithm_streamed"
        )));
    }

    #[test]
    fn structural_budgets_include_text_input_long_line_cap_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&("text_input_long_line_cap/capped_bytes/4096", "line_bytes")));
        assert!(specs.contains(&("text_input_long_line_cap/capped_bytes/4096", "capped_len")));
        assert!(specs.contains(&("text_input_long_line_cap/capped_bytes/4096", "cap_active")));
        assert!(specs.contains(&("text_input_long_line_cap/uncapped_control", "line_bytes")));
        assert!(specs.contains(&("text_input_long_line_cap/uncapped_control", "capped_len")));
        assert!(specs.contains(&("text_input_long_line_cap/uncapped_control", "cap_active")));
    }

    #[test]
    fn structural_budgets_include_text_input_wrap_incremental_tabs_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/full_recompute",
            "line_bytes"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/full_recompute",
            "dirty_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/full_recompute",
            "recomputed_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/full_recompute",
            "incremental_patch"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/incremental_patch",
            "line_bytes"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/incremental_patch",
            "dirty_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/incremental_patch",
            "recomputed_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_tabs/incremental_patch",
            "incremental_patch"
        )));
    }

    #[test]
    fn structural_budgets_include_text_input_wrap_incremental_burst_edits_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/full_recompute/12",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/full_recompute/12",
            "edits_per_burst"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/full_recompute/12",
            "total_dirty_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/full_recompute/12",
            "recomputed_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/full_recompute/12",
            "incremental_patch"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/incremental_patch/12",
            "total_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/incremental_patch/12",
            "edits_per_burst"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/incremental_patch/12",
            "total_dirty_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/incremental_patch/12",
            "recomputed_lines"
        )));
        assert!(specs.contains(&(
            "text_input_wrap_incremental_burst_edits/incremental_patch/12",
            "incremental_patch"
        )));
    }

    #[test]
    fn structural_budgets_include_text_model_snapshot_clone_cost_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
            "document_bytes"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
            "line_starts"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
            "clone_count"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
            "sampled_prefix_bytes"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/piece_table_snapshot_clone/8192",
            "snapshot_path"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
            "document_bytes"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
            "line_starts"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
            "clone_count"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
            "sampled_prefix_bytes"
        )));
        assert!(specs.contains(&(
            "text_model_snapshot_clone_cost/shared_string_clone_control/8192",
            "snapshot_path"
        )));
    }

    #[test]
    fn structural_budgets_include_text_model_bulk_load_large_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        for variant in &[
            "text_model_bulk_load_large/piece_table_append_large",
            "text_model_bulk_load_large/piece_table_from_large_text",
            "text_model_bulk_load_large/string_push_control",
        ] {
            for metric in &[
                "source_bytes",
                "document_bytes_after",
                "line_starts_after",
                "chunk_count",
                "load_variant",
            ] {
                assert!(
                    specs.contains(&(variant, metric)),
                    "missing structural budget for {variant}/{metric}"
                );
            }
        }
    }

    #[test]
    fn structural_budgets_include_text_model_fragmented_edits_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        for variant in &[
            "text_model_fragmented_edits/piece_table_edits",
            "text_model_fragmented_edits/materialize_after_edits",
            "text_model_fragmented_edits/shared_string_after_edits/64",
            "text_model_fragmented_edits/string_edit_control",
        ] {
            for metric in &[
                "initial_bytes",
                "edit_count",
                "deleted_bytes",
                "inserted_bytes",
                "final_bytes",
                "line_starts_after",
                "readback_operations",
                "string_control",
            ] {
                assert!(
                    specs.contains(&(variant, metric)),
                    "missing structural budget for {variant}/{metric}"
                );
            }
        }
    }

    #[test]
    fn timing_budgets_include_pre_existing_resolved_output_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"resolved_output_recompute_incremental/full_recompute"));
        assert!(labels.contains(&"resolved_output_recompute_incremental/incremental_recompute"));
    }

    #[test]
    fn structural_budgets_include_pre_existing_resolved_output_targets() {
        let specs = STRUCTURAL_BUDGETS
            .iter()
            .map(|spec| (spec.bench, spec.metric))
            .collect::<Vec<_>>();
        assert!(specs.contains(&(
            "resolved_output_recompute_incremental/full_recompute",
            "outline_rows"
        )));
        assert!(specs.contains(&(
            "resolved_output_recompute_incremental/full_recompute",
            "recomputed_rows"
        )));
        assert!(specs.contains(&(
            "resolved_output_recompute_incremental/full_recompute",
            "manual_rows"
        )));
        assert!(specs.contains(&(
            "resolved_output_recompute_incremental/incremental_recompute",
            "dirty_rows"
        )));
        assert!(specs.contains(&(
            "resolved_output_recompute_incremental/incremental_recompute",
            "recomputed_rows"
        )));
        assert!(specs.contains(&(
            "resolved_output_recompute_incremental/incremental_recompute",
            "fallback_full_recompute"
        )));
    }

    #[test]
    fn timing_budgets_include_pre_existing_conflict_extra_targets() {
        let labels: Vec<&str> = PERF_BUDGETS.iter().map(|spec| spec.label).collect();
        assert!(labels.contains(&"conflict_three_way_prepared_syntax_scroll/style_window/200"));
        assert!(labels.contains(&"conflict_three_way_visible_map_build/linear_two_pointer"));
        assert!(labels.contains(&"conflict_three_way_visible_map_build/legacy_find_scan"));
        assert!(
            labels.contains(&"conflict_load_duplication/shared_payload_forwarding/low_density")
        );
        assert!(
            labels.contains(&"conflict_load_duplication/duplicated_text_and_bytes/low_density")
        );
        assert!(
            labels.contains(&"conflict_load_duplication/shared_payload_forwarding/high_density")
        );
        assert!(
            labels.contains(&"conflict_load_duplication/duplicated_text_and_bytes/high_density")
        );
        assert!(labels.contains(&"conflict_two_way_diff_build/full_file/low_density"));
        assert!(labels.contains(&"conflict_two_way_diff_build/block_local/low_density"));
        assert!(labels.contains(&"conflict_two_way_diff_build/full_file/high_density"));
        assert!(labels.contains(&"conflict_two_way_diff_build/block_local/high_density"));
        assert!(labels.contains(&"conflict_two_way_word_highlights/full_file/low_density"));
        assert!(labels.contains(&"conflict_two_way_word_highlights/block_local/low_density"));
        assert!(labels.contains(&"conflict_two_way_word_highlights/full_file/high_density"));
        assert!(labels.contains(&"conflict_two_way_word_highlights/block_local/high_density"));
        assert!(labels.contains(&"conflict_resolved_output_gutter_scroll/window_100"));
        assert!(labels.contains(&"conflict_resolved_output_gutter_scroll/window_200"));
        assert!(labels.contains(&"conflict_resolved_output_gutter_scroll/window_400"));
    }

    fn write_estimate_file(root: &Path, relative_path: &str, mean: f64, upper: f64) {
        let path = root.join(relative_path);
        let parent = path.parent().expect("estimate path parent");
        fs::create_dir_all(parent).expect("create estimate directories");
        let content = format!(
            r#"{{
                "mean": {{
                    "confidence_interval": {{
                        "confidence_level": 0.95,
                        "lower_bound": {mean},
                        "upper_bound": {upper}
                    }},
                    "point_estimate": {mean},
                    "standard_error": 1.0
                }}
            }}"#
        );
        fs::write(path, content).expect("write estimate file");
    }

    fn write_sidecar_file(root: &Path, bench: &str, metrics: &[(&str, serde_json::Value)]) {
        let mut payload = serde_json::Map::new();
        for (metric, value) in metrics {
            payload.insert((*metric).to_string(), value.clone());
        }
        let report = PerfSidecarReport::new(bench, payload);
        let path = criterion_sidecar_path(root, bench);
        gitcomet_ui_gpui::perf_sidecar::write_sidecar(&report, &path).expect("write sidecar");
    }

    fn set_file_modified(path: &Path, modified: SystemTime) {
        let file = OpenOptions::new()
            .write(true)
            .open(path)
            .expect("open file for timestamp update");
        file.set_times(fs::FileTimes::new().set_modified(modified))
            .expect("set file modified time");
    }
}
