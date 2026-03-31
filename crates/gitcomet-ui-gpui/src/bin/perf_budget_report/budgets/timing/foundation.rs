use super::super::*;

pub(crate) const PERF_BUDGETS: &[PerfBudgetSpec] = &[
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
        label: "markdown_preview_scroll/window_rows/200",
        estimate_path: "markdown_preview_scroll/window_rows/200/new/estimates.json",
        // Steady-state Preview-mode scroll over a large single markdown document
        // reuses styled-row caches, so it should stay close to render_single.
        threshold_ns: 2.0 * NANOS_PER_MILLISECOND,
    },
    PerfBudgetSpec {
        label: "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        estimate_path: "markdown_preview_scroll/rich_5000_rows_window_rows/200/new/estimates.json",
        // Heavier steady-state Preview-mode scroll case: 5k rendered rows with
        // 500 long 2k-character rows plus mixed headings, lists, tables, and code.
        threshold_ns: 25.0 * NANOS_PER_MILLISECOND,
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
    // an unstaged status row to open its diff. The hot non-conflict path now
    // stores the selected target once and emits one pathless selected-diff
    // intent, so this stays well below 1 µs with wide runner-noise headroom.
    PerfBudgetSpec {
        label: "status_select_diff_open/unstaged",
        estimate_path: "status_select_diff_open/unstaged/new/estimates.json",
        threshold_ns: 1.0 * NANOS_PER_MILLISECOND,
    },
    // status_select_diff_open/staged — staged path shares the same pathless
    // selected-diff intent and avoids the unstaged conflict probe entirely.
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
    // diff_refresh_rev_only_same_content/rekey — cached signature check + rev bump
    // The rekey path now reads a precomputed content signature from `FileDiffText`.
    // Keep a generous budget because this benchmark can approach the measurement floor.
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
];
