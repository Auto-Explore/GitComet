use super::super::*;

pub(crate) const STRUCTURAL_BUDGETS: &[StructuralBudgetSpec] = &[
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
];
