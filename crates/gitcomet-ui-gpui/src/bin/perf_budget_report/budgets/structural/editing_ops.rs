use super::super::*;

pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
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
];
