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
    assert!(labels.contains(&"markdown_preview_scroll/window_rows/200"));
    assert!(labels.contains(&"markdown_preview_scroll/rich_5000_rows_window_rows/200"));
}

#[test]
fn structural_budgets_include_markdown_preview_scroll_target() {
    let specs = STRUCTURAL_BUDGETS
        .iter()
        .map(|spec| (spec.bench, spec.metric))
        .collect::<Vec<_>>();
    assert!(specs.contains(&("markdown_preview_scroll/window_rows/200", "total_rows")));
    assert!(specs.contains(&("markdown_preview_scroll/window_rows/200", "start_row")));
    assert!(specs.contains(&("markdown_preview_scroll/window_rows/200", "window_size")));
    assert!(specs.contains(&("markdown_preview_scroll/window_rows/200", "rows_rendered")));
    assert!(specs.contains(&(
        "markdown_preview_scroll/window_rows/200",
        "scroll_step_rows"
    )));
    assert!(specs.contains(&(
        "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        "total_rows"
    )));
    assert!(specs.contains(&(
        "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        "long_rows"
    )));
    assert!(specs.contains(&(
        "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        "long_row_bytes"
    )));
    assert!(specs.contains(&(
        "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        "table_rows"
    )));
    assert!(specs.contains(&(
        "markdown_preview_scroll/rich_5000_rows_window_rows/200",
        "code_rows"
    )));
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
    assert!(labels.contains(&"text_model_snapshot_clone_cost/shared_string_clone_control/8192"));
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
    assert!(labels.contains(&"conflict_load_duplication/shared_payload_forwarding/low_density"));
    assert!(labels.contains(&"conflict_load_duplication/duplicated_text_and_bytes/low_density"));
    assert!(labels.contains(&"conflict_load_duplication/shared_payload_forwarding/high_density"));
    assert!(labels.contains(&"conflict_load_duplication/duplicated_text_and_bytes/high_density"));
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
