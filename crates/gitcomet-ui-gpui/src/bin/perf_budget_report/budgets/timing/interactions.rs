use super::super::*;

pub(crate) const PERF_BUDGETS: &[PerfBudgetSpec] = &[
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
];
