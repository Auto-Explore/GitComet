pub(crate) use criterion::{BenchmarkId, Criterion};
pub(crate) use gitcomet_core::file_diff::BenchmarkReplacementDistanceBackend;
pub(crate) use gitcomet_ui_gpui::benchmarks::{
    BranchSidebarCacheFixture, BranchSidebarCacheMetrics, BranchSidebarFixture,
    BranchSidebarMetrics, ClipboardFixture, ClipboardMetrics, CommitDetailsFixture,
    CommitDetailsMetrics, CommitSearchFilterFixture, CommitSearchFilterMetrics,
    CommitSelectReplaceFixture, CommitSelectReplaceMetrics, ConflictCompareFirstWindowMetrics,
    ConflictLoadDuplicationFixture, ConflictResolvedOutputGutterScrollFixture,
    ConflictSearchQueryUpdateFixture, ConflictSplitResizeStepFixture,
    ConflictStreamedProviderFixture, ConflictStreamedResolvedOutputFixture,
    ConflictThreeWayScrollFixture, ConflictThreeWayVisibleMapBuildFixture,
    ConflictTwoWayDiffBuildFixture, ConflictTwoWaySplitScrollFixture, DiffRefreshFixture,
    DiffRefreshMetrics, DiffSplitResizeDragMetrics, DiffSplitResizeDragStepFixture, DisplayFixture,
    DisplayMetrics, FileDiffCtrlFOpenTypeFixture, FileDiffCtrlFOpenTypeMetrics,
    FileDiffInlineSyntaxProjectionFixture, FileDiffOpenFixture, FileDiffOpenMetrics,
    FileDiffSyntaxCacheDropFixture, FileDiffSyntaxPrepareFixture, FileDiffSyntaxReparseFixture,
    FileFuzzyFindFixture, FileFuzzyFindMetrics, FilePreviewTextSearchFixture,
    FilePreviewTextSearchMetrics, FrameTimingCapture, FrameTimingStats, FsEventFixture,
    FsEventMetrics, GitOpsFixture, GitOpsMetrics, HistoryCacheBuildFixture,
    HistoryCacheBuildMetrics, HistoryColumnResizeDragStepFixture, HistoryColumnResizeMetrics,
    HistoryGraphFixture, HistoryGraphMetrics, HistoryListScrollFixture,
    HistoryLoadMoreAppendFixture, HistoryLoadMoreAppendMetrics, HistoryResizeColumn,
    HistoryScopeSwitchFixture, HistoryScopeSwitchMetrics, ImagePreviewFirstPaintFixture,
    ImagePreviewFirstPaintMetrics, InDiffTextSearchFixture, InDiffTextSearchMetrics,
    KeyboardArrowScrollFixture, KeyboardArrowScrollMetrics, KeyboardStageUnstageToggleFixture,
    KeyboardStageUnstageToggleMetrics, KeyboardTabFocusCycleFixture, KeyboardTabFocusCycleMetrics,
    LargeFileDiffScrollFixture, LargeFileDiffScrollMetrics, LargeHtmlSyntaxFixture,
    LargeHtmlSyntaxMetrics, MarkdownPreviewFirstWindowMetrics, MarkdownPreviewFixture,
    MarkdownPreviewScrollFixture, MarkdownPreviewScrollMetrics, MergeOpenBootstrapFixture,
    MergeOpenBootstrapMetrics, NetworkFixture, NetworkMetrics, OpenRepoFixture, OpenRepoMetrics,
    PaneResizeDragMetrics, PaneResizeDragStepFixture, PaneResizeTarget,
    PatchDiffFirstWindowMetrics, PatchDiffPagedRowsFixture, PatchDiffSearchQueryUpdateFixture,
    PathDisplayCacheChurnFixture, PathDisplayCacheChurnMetrics, RapidCommitSelectionFixture,
    RapidCommitSelectionMetrics, RealRepoFixture, RealRepoMetrics, RealRepoScenario,
    ReplacementAlignmentFixture, RepoSwitchDuringScrollFixture, RepoSwitchDuringScrollMetrics,
    RepoSwitchFixture, RepoSwitchMetrics, RepoTabDragFixture, RepoTabDragMetrics,
    ResolvedOutputRecomputeIncrementalFixture, ResolvedOutputRecomputeMetrics,
    ScrollbarDragStepFixture, ScrollbarDragStepMetrics, SidebarResizeDragSustainedFixture,
    SidebarResizeDragSustainedMetrics, StagingFixture, StagingMetrics, StatusListFixture,
    StatusListMetrics, StatusMultiSelectFixture, StatusMultiSelectMetrics,
    StatusSelectDiffOpenFixture, StatusSelectDiffOpenMetrics, SvgDualPathFirstWindowFixture,
    SvgDualPathFirstWindowMetrics, TextInputHighlightDensity, TextInputLongLineCapFixture,
    TextInputLongLineCapMetrics, TextInputPrepaintWindowedFixture,
    TextInputPrepaintWindowedMetrics, TextInputRunsStreamedHighlightFixture,
    TextInputRunsStreamedHighlightMetrics, TextInputWrapIncrementalBurstEditsFixture,
    TextInputWrapIncrementalBurstEditsMetrics, TextInputWrapIncrementalTabsFixture,
    TextInputWrapIncrementalTabsMetrics, TextModelBulkLoadLargeFixture,
    TextModelBulkLoadLargeMetrics, TextModelFragmentedEditFixture, TextModelFragmentedEditsMetrics,
    TextModelSnapshotCloneCostFixture, TextModelSnapshotCloneCostMetrics, UndoRedoFixture,
    UndoRedoMetrics, WindowResizeLayoutExtremeFixture, WindowResizeLayoutExtremeMetrics,
    WindowResizeLayoutFixture, WindowResizeLayoutMetrics, WorktreePreviewRenderFixture,
    WorktreePreviewRenderMetrics,
};
use gitcomet_ui_gpui::perf_alloc::{PerfAllocMetrics, measure_allocations};
use gitcomet_ui_gpui::perf_ram_guard::install_benchmark_process_ram_guard;
use gitcomet_ui_gpui::perf_sidecar::{PerfSidecarReport, write_criterion_sidecar};
pub(crate) use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
pub(crate) use std::env;
pub(crate) use std::time::{Duration, Instant};

thread_local! {
    static PENDING_SIDECAR_ALLOCATIONS: RefCell<VecDeque<PerfAllocMetrics>> = const {
        RefCell::new(VecDeque::new())
    };
}

pub(crate) const SUPPRESS_MISSING_REAL_REPO_NOTICE_ENV: &str =
    "GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE";

pub(crate) fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

pub(crate) fn env_string(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn env_flag(key: &str) -> bool {
    env::var(key)
        .ok()
        .as_deref()
        .map(parse_bool_flag)
        .unwrap_or(false)
}

pub(crate) fn parse_bool_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

pub(crate) fn markdown_preview_measurement_time() -> Duration {
    Duration::from_millis(
        env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_MEASUREMENT_MS", 250).max(250) as u64,
    )
}

pub(crate) fn benchmark_criterion() -> Criterion {
    install_benchmark_process_ram_guard();
    Criterion::default()
}

pub(crate) fn settle_markdown_allocator_pages() {
    // The markdown preview fixtures are large enough that dropping them and giving
    // mimalloc a short purge window keeps the process-wide RAM guard aligned with
    // live working set before the next benchmark group begins.
    std::thread::sleep(Duration::from_secs(1));
}

pub(crate) fn measure_sidecar_allocations<T>(f: impl FnOnce() -> T) -> T {
    let (value, allocations) = measure_allocations(f);
    PENDING_SIDECAR_ALLOCATIONS.with(|pending| pending.borrow_mut().push_back(allocations));
    value
}

pub(crate) fn take_pending_sidecar_allocations() -> PerfAllocMetrics {
    PENDING_SIDECAR_ALLOCATIONS
        .with(|pending| pending.borrow_mut().pop_front())
        .unwrap_or_else(|| {
            panic!("missing allocation snapshot for sidecar emission; wrap the representative sidecar run with measure_sidecar_allocations")
        })
}

pub(crate) fn emit_sidecar_metrics(bench: &str, mut metrics: Map<String, Value>) {
    let allocations = take_pending_sidecar_allocations();
    allocations.append_to_payload(&mut metrics);
    let report = PerfSidecarReport::new(bench, metrics);
    write_criterion_sidecar(&report).unwrap_or_else(|err| panic!("{err}"));
}

pub(crate) fn emit_allocation_only_sidecar(bench: &str) {
    emit_sidecar_metrics(bench, Map::new());
}

pub(crate) fn emit_patch_diff_first_window_sidecar(
    window: usize,
    first_window_ns: u64,
    metrics: PatchDiffFirstWindowMetrics,
) {
    emit_patch_diff_sidecar(
        &format!("diff_open_patch_first_window/{window}"),
        first_window_ns,
        metrics,
    );
}

pub(crate) fn emit_patch_diff_sidecar(
    bench: &str,
    first_window_ns: u64,
    metrics: PatchDiffFirstWindowMetrics,
) {
    let mut payload = Map::new();
    payload.insert("first_window_ns".to_string(), json!(first_window_ns));
    payload.insert("rows_requested".to_string(), json!(metrics.rows_requested));
    payload.insert(
        "rows_painted".to_string(),
        json!(metrics.split_rows_painted),
    );
    payload.insert(
        "rows_materialized".to_string(),
        json!(metrics.split_rows_materialized),
    );
    payload.insert(
        "patch_rows_painted".to_string(),
        json!(metrics.patch_rows_painted),
    );
    payload.insert(
        "patch_rows_materialized".to_string(),
        json!(metrics.patch_rows_materialized),
    );
    payload.insert(
        "patch_page_cache_entries".to_string(),
        json!(metrics.patch_page_cache_entries),
    );
    payload.insert(
        "split_rows_painted".to_string(),
        json!(metrics.split_rows_painted),
    );
    payload.insert(
        "split_rows_materialized".to_string(),
        json!(metrics.split_rows_materialized),
    );
    payload.insert(
        "full_text_materializations".to_string(),
        json!(metrics.full_text_materializations),
    );
    emit_sidecar_metrics(bench, payload);
}

pub(crate) fn emit_open_repo_sidecar(case_name: &str, metrics: &OpenRepoMetrics) {
    let mut payload = Map::new();
    payload.insert("commit_count".to_string(), json!(metrics.commit_count));
    payload.insert("local_branches".to_string(), json!(metrics.local_branches));
    payload.insert(
        "remote_branches".to_string(),
        json!(metrics.remote_branches),
    );
    payload.insert("remotes".to_string(), json!(metrics.remotes));
    payload.insert("worktrees".to_string(), json!(metrics.worktrees));
    payload.insert("submodules".to_string(), json!(metrics.submodules));
    payload.insert("sidebar_rows".to_string(), json!(metrics.sidebar_rows));
    payload.insert("graph_rows".to_string(), json!(metrics.graph_rows));
    payload.insert(
        "max_graph_lanes".to_string(),
        json!(metrics.max_graph_lanes),
    );
    emit_sidecar_metrics(&format!("open_repo/{case_name}"), payload);
}

pub(crate) fn emit_branch_sidebar_sidecar(case_name: &str, metrics: &BranchSidebarMetrics) {
    let mut payload = Map::new();
    payload.insert("local_branches".to_string(), json!(metrics.local_branches));
    payload.insert(
        "remote_branches".to_string(),
        json!(metrics.remote_branches),
    );
    payload.insert("remotes".to_string(), json!(metrics.remotes));
    payload.insert("worktrees".to_string(), json!(metrics.worktrees));
    payload.insert("submodules".to_string(), json!(metrics.submodules));
    payload.insert("stashes".to_string(), json!(metrics.stashes));
    payload.insert("sidebar_rows".to_string(), json!(metrics.sidebar_rows));
    payload.insert("branch_rows".to_string(), json!(metrics.branch_rows));
    payload.insert("remote_headers".to_string(), json!(metrics.remote_headers));
    payload.insert("group_headers".to_string(), json!(metrics.group_headers));
    payload.insert(
        "max_branch_depth".to_string(),
        json!(metrics.max_branch_depth),
    );
    emit_sidecar_metrics(&format!("branch_sidebar/{case_name}"), payload);
}

pub(crate) fn emit_branch_sidebar_cache_sidecar(
    case_name: &str,
    metrics: &BranchSidebarCacheMetrics,
) {
    let mut payload = Map::new();
    payload.insert("cache_hits".to_string(), json!(metrics.cache_hits));
    payload.insert("cache_misses".to_string(), json!(metrics.cache_misses));
    payload.insert("rows_count".to_string(), json!(metrics.rows_count));
    payload.insert("invalidations".to_string(), json!(metrics.invalidations));
    emit_sidecar_metrics(&format!("branch_sidebar/{case_name}"), payload);
}

pub(crate) fn emit_history_cache_build_sidecar(
    case_name: &str,
    metrics: &HistoryCacheBuildMetrics,
) {
    let mut payload = Map::new();
    payload.insert(
        "visible_commits".to_string(),
        json!(metrics.visible_commits),
    );
    payload.insert("graph_rows".to_string(), json!(metrics.graph_rows));
    payload.insert("max_lanes".to_string(), json!(metrics.max_lanes));
    payload.insert("commit_vms".to_string(), json!(metrics.commit_vms));
    payload.insert(
        "stash_helpers_filtered".to_string(),
        json!(metrics.stash_helpers_filtered),
    );
    payload.insert(
        "decorated_commits".to_string(),
        json!(metrics.decorated_commits),
    );
    emit_sidecar_metrics(&format!("history_cache_build/{case_name}"), payload);
}

pub(crate) fn emit_history_load_more_append_sidecar(
    case_name: &str,
    metrics: &HistoryLoadMoreAppendMetrics,
) {
    let mut payload = Map::new();
    payload.insert(
        "existing_commits".to_string(),
        json!(metrics.existing_commits),
    );
    payload.insert(
        "appended_commits".to_string(),
        json!(metrics.appended_commits),
    );
    payload.insert(
        "total_commits_after_append".to_string(),
        json!(metrics.total_commits_after_append),
    );
    payload.insert(
        "next_cursor_present".to_string(),
        json!(metrics.next_cursor_present),
    );
    payload.insert(
        "follow_up_effect_count".to_string(),
        json!(metrics.follow_up_effect_count),
    );
    payload.insert("log_rev_delta".to_string(), json!(metrics.log_rev_delta));
    payload.insert(
        "log_loading_more_cleared".to_string(),
        json!(metrics.log_loading_more_cleared),
    );
    emit_sidecar_metrics(&format!("history_load_more_append/{case_name}"), payload);
}

pub(crate) fn emit_history_scope_switch_sidecar(
    case_name: &str,
    metrics: &HistoryScopeSwitchMetrics,
) {
    let mut payload = Map::new();
    payload.insert(
        "existing_commits".to_string(),
        json!(metrics.existing_commits),
    );
    payload.insert("scope_changed".to_string(), json!(metrics.scope_changed));
    payload.insert("log_rev_delta".to_string(), json!(metrics.log_rev_delta));
    payload.insert(
        "log_set_to_loading".to_string(),
        json!(metrics.log_set_to_loading),
    );
    payload.insert(
        "load_log_effect_count".to_string(),
        json!(metrics.load_log_effect_count),
    );
    payload.insert(
        "persist_session_effect_count".to_string(),
        json!(metrics.persist_session_effect_count),
    );
    emit_sidecar_metrics(&format!("history_scope_switch/{case_name}"), payload);
}

pub(crate) fn emit_repo_switch_sidecar(case_name: &str, metrics: &RepoSwitchMetrics) {
    let mut payload = Map::new();
    payload.insert("effect_count".to_string(), json!(metrics.effect_count));
    payload.insert(
        "refresh_effect_count".to_string(),
        json!(metrics.refresh_effect_count),
    );
    payload.insert(
        "selected_diff_reload_effect_count".to_string(),
        json!(metrics.selected_diff_reload_effect_count),
    );
    payload.insert(
        "persist_session_effect_count".to_string(),
        json!(metrics.persist_session_effect_count),
    );
    payload.insert("repo_count".to_string(), json!(metrics.repo_count));
    payload.insert(
        "hydrated_repo_count".to_string(),
        json!(metrics.hydrated_repo_count),
    );
    payload.insert(
        "selected_commit_repo_count".to_string(),
        json!(metrics.selected_commit_repo_count),
    );
    payload.insert(
        "selected_diff_repo_count".to_string(),
        json!(metrics.selected_diff_repo_count),
    );
    emit_sidecar_metrics(&format!("repo_switch/{case_name}"), payload);
}

pub(crate) fn emit_status_list_sidecar(case_name: &str, metrics: &StatusListMetrics) {
    let mut payload = Map::new();
    payload.insert("rows_requested".to_string(), json!(metrics.rows_requested));
    payload.insert("rows_painted".to_string(), json!(metrics.rows_painted));
    payload.insert("entries_total".to_string(), json!(metrics.entries_total));
    payload.insert(
        "path_display_cache_hits".to_string(),
        json!(metrics.path_display_cache_hits),
    );
    payload.insert(
        "path_display_cache_misses".to_string(),
        json!(metrics.path_display_cache_misses),
    );
    payload.insert(
        "path_display_cache_clears".to_string(),
        json!(metrics.path_display_cache_clears),
    );
    payload.insert("max_path_depth".to_string(), json!(metrics.max_path_depth));
    payload.insert(
        "prewarmed_entries".to_string(),
        json!(metrics.prewarmed_entries),
    );
    emit_sidecar_metrics(&format!("status_list/{case_name}"), payload);
}

pub(crate) fn emit_status_multi_select_sidecar(
    case_name: &str,
    metrics: &StatusMultiSelectMetrics,
) {
    let mut payload = Map::new();
    payload.insert("entries_total".to_string(), json!(metrics.entries_total));
    payload.insert("selected_paths".to_string(), json!(metrics.selected_paths));
    payload.insert("anchor_index".to_string(), json!(metrics.anchor_index));
    payload.insert("clicked_index".to_string(), json!(metrics.clicked_index));
    payload.insert(
        "anchor_preserved".to_string(),
        json!(metrics.anchor_preserved),
    );
    payload.insert(
        "position_scan_steps".to_string(),
        json!(metrics.position_scan_steps),
    );
    emit_sidecar_metrics(&format!("status_multi_select/{case_name}"), payload);
}

pub(crate) fn emit_status_select_diff_open_sidecar(
    case_name: &str,
    metrics: &StatusSelectDiffOpenMetrics,
) {
    let mut payload = Map::new();
    payload.insert("effect_count".to_string(), json!(metrics.effect_count));
    payload.insert(
        "load_diff_effect_count".to_string(),
        json!(metrics.load_diff_effect_count),
    );
    payload.insert(
        "load_diff_file_effect_count".to_string(),
        json!(metrics.load_diff_file_effect_count),
    );
    payload.insert(
        "load_diff_file_image_effect_count".to_string(),
        json!(metrics.load_diff_file_image_effect_count),
    );
    payload.insert(
        "diff_state_rev_delta".to_string(),
        json!(metrics.diff_state_rev_delta),
    );
    emit_sidecar_metrics(&format!("status_select_diff_open/{case_name}"), payload);
}

pub(crate) fn emit_history_graph_sidecar(case_name: &str, metrics: &HistoryGraphMetrics) {
    let mut payload = Map::new();
    payload.insert("commit_count".to_string(), json!(metrics.commit_count));
    payload.insert("graph_rows".to_string(), json!(metrics.graph_rows));
    payload.insert("max_lanes".to_string(), json!(metrics.max_lanes));
    payload.insert("merge_count".to_string(), json!(metrics.merge_count));
    payload.insert("branch_heads".to_string(), json!(metrics.branch_heads));
    emit_sidecar_metrics(&format!("history_graph/{case_name}"), payload);
}

pub(crate) fn emit_commit_details_sidecar(case_name: &str, metrics: &CommitDetailsMetrics) {
    let mut payload = Map::new();
    payload.insert("file_count".to_string(), json!(metrics.file_count));
    payload.insert("max_path_depth".to_string(), json!(metrics.max_path_depth));
    payload.insert("message_bytes".to_string(), json!(metrics.message_bytes));
    payload.insert("message_lines".to_string(), json!(metrics.message_lines));
    payload.insert(
        "message_shaped_lines".to_string(),
        json!(metrics.message_shaped_lines),
    );
    payload.insert(
        "message_shaped_bytes".to_string(),
        json!(metrics.message_shaped_bytes),
    );
    payload.insert("added_files".to_string(), json!(metrics.added_files));
    payload.insert("modified_files".to_string(), json!(metrics.modified_files));
    payload.insert("deleted_files".to_string(), json!(metrics.deleted_files));
    payload.insert("renamed_files".to_string(), json!(metrics.renamed_files));
    emit_sidecar_metrics(&format!("commit_details/{case_name}"), payload);
}

pub(crate) fn emit_commit_select_replace_sidecar(
    case_name: &str,
    metrics: &CommitSelectReplaceMetrics,
) {
    let mut payload = Map::new();
    payload.insert("files_a".to_string(), json!(metrics.files_a));
    payload.insert("files_b".to_string(), json!(metrics.files_b));
    payload.insert(
        "commit_ids_differ".to_string(),
        json!(metrics.commit_ids_differ),
    );
    emit_sidecar_metrics(&format!("commit_details/{case_name}"), payload);
}

pub(crate) fn emit_path_display_cache_churn_sidecar(
    case_name: &str,
    metrics: &PathDisplayCacheChurnMetrics,
) {
    let mut payload = Map::new();
    payload.insert("file_count".to_string(), json!(metrics.file_count));
    payload.insert(
        "path_display_cache_hits".to_string(),
        json!(metrics.path_display_cache_hits),
    );
    payload.insert(
        "path_display_cache_misses".to_string(),
        json!(metrics.path_display_cache_misses),
    );
    payload.insert(
        "path_display_cache_clears".to_string(),
        json!(metrics.path_display_cache_clears),
    );
    emit_sidecar_metrics(&format!("commit_details/{case_name}"), payload);
}

pub(crate) fn emit_merge_open_bootstrap_sidecar(
    case_name: &str,
    metrics: &MergeOpenBootstrapMetrics,
) {
    let mut payload = Map::new();
    payload.insert(
        "trace_event_count".to_string(),
        json!(metrics.trace_event_count),
    );
    payload.insert(
        "conflict_block_count".to_string(),
        json!(metrics.conflict_block_count),
    );
    payload.insert("diff_row_count".to_string(), json!(metrics.diff_row_count));
    payload.insert(
        "inline_row_count".to_string(),
        json!(metrics.inline_row_count),
    );
    payload.insert(
        "resolved_output_line_count".to_string(),
        json!(metrics.resolved_output_line_count),
    );
    payload.insert(
        "two_way_visible_rows".to_string(),
        json!(metrics.two_way_visible_rows),
    );
    payload.insert(
        "three_way_visible_rows".to_string(),
        json!(metrics.three_way_visible_rows),
    );
    payload.insert(
        "rendering_mode_streamed".to_string(),
        json!(metrics.rendering_mode_streamed),
    );
    payload.insert(
        "full_output_generated".to_string(),
        json!(metrics.full_output_generated),
    );
    payload.insert(
        "full_syntax_parse_requested".to_string(),
        json!(metrics.full_syntax_parse_requested),
    );
    payload.insert(
        "whole_block_diff_ran".to_string(),
        json!(metrics.whole_block_diff_ran),
    );
    payload.insert("rss_kib".to_string(), json!(metrics.rss_kib));
    payload.insert(
        "parse_conflict_markers_ms".to_string(),
        json!(metrics.parse_conflict_markers_ms),
    );
    payload.insert(
        "generate_resolved_text_ms".to_string(),
        json!(metrics.generate_resolved_text_ms),
    );
    payload.insert(
        "side_by_side_rows_ms".to_string(),
        json!(metrics.side_by_side_rows_ms),
    );
    payload.insert(
        "build_three_way_conflict_maps_ms".to_string(),
        json!(metrics.build_three_way_conflict_maps_ms),
    );
    payload.insert(
        "compute_three_way_word_highlights_ms".to_string(),
        json!(metrics.compute_three_way_word_highlights_ms),
    );
    payload.insert(
        "compute_two_way_word_highlights_ms".to_string(),
        json!(metrics.compute_two_way_word_highlights_ms),
    );
    payload.insert(
        "bootstrap_total_ms".to_string(),
        json!(metrics.bootstrap_total_ms),
    );
    emit_sidecar_metrics(&format!("merge_open_bootstrap/{case_name}"), payload);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct FrameTimingScenarioMetrics {
    total_rows: u64,
    window_rows: u64,
    scroll_step_rows: u64,
}

pub(crate) fn capture_frame_timing_scroll_burst<F>(
    total_rows: usize,
    window_rows: usize,
    scroll_step_rows: usize,
    frame_budget_ns: u64,
    frames: usize,
    mut run_step: F,
) -> (u64, FrameTimingStats, FrameTimingScenarioMetrics)
where
    F: FnMut(usize, usize) -> u64,
{
    let frames = frames.max(1);
    let window_rows = window_rows.max(1);
    let scroll_step_rows = scroll_step_rows.max(1);
    let total_rows = total_rows.max(window_rows);
    let max_start = total_rows.saturating_sub(window_rows);

    let mut capture = FrameTimingCapture::with_expected_frames(frame_budget_ns.max(1), frames);
    let mut hash = 0u64;
    let mut start = 0usize;

    for _ in 0..frames {
        let frame_started = Instant::now();
        hash ^= run_step(start, window_rows);
        capture.record_frame(frame_started.elapsed());

        if max_start > 0 {
            start = start.saturating_add(scroll_step_rows);
            if start > max_start {
                start %= max_start + 1;
            }
        }
    }

    (
        hash,
        capture.finish(),
        FrameTimingScenarioMetrics {
            total_rows: u64::try_from(total_rows).unwrap_or(u64::MAX),
            window_rows: u64::try_from(window_rows).unwrap_or(u64::MAX),
            scroll_step_rows: u64::try_from(scroll_step_rows).unwrap_or(u64::MAX),
        },
    )
}

pub(crate) fn emit_frame_timing_sidecar(
    case_name: &str,
    stats: &FrameTimingStats,
    metrics: FrameTimingScenarioMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("total_rows".to_string(), json!(metrics.total_rows));
    payload.insert("window_rows".to_string(), json!(metrics.window_rows));
    payload.insert(
        "scroll_step_rows".to_string(),
        json!(metrics.scroll_step_rows),
    );
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics(&format!("frame_timing/{case_name}"), payload);
}

pub(crate) fn emit_sidebar_resize_drag_sustained_sidecar(
    stats: &FrameTimingStats,
    metrics: SidebarResizeDragSustainedMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("frames".to_string(), json!(metrics.frames));
    payload.insert(
        "steps_per_frame".to_string(),
        json!(metrics.steps_per_frame),
    );
    payload.insert(
        "total_clamp_at_min".to_string(),
        json!(metrics.total_clamp_at_min),
    );
    payload.insert(
        "total_clamp_at_max".to_string(),
        json!(metrics.total_clamp_at_max),
    );
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics("frame_timing/sidebar_resize_drag_sustained", payload);
}

pub(crate) fn emit_rapid_commit_selection_sidecar(
    stats: &FrameTimingStats,
    metrics: RapidCommitSelectionMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("commit_count".to_string(), json!(metrics.commit_count));
    payload.insert(
        "files_per_commit".to_string(),
        json!(metrics.files_per_commit),
    );
    payload.insert("selections".to_string(), json!(metrics.selections));
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics("frame_timing/rapid_commit_selection_changes", payload);
}

pub(crate) fn emit_repo_switch_during_scroll_sidecar(
    stats: &FrameTimingStats,
    metrics: RepoSwitchDuringScrollMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("total_frames".to_string(), json!(metrics.total_frames));
    payload.insert("scroll_frames".to_string(), json!(metrics.scroll_frames));
    payload.insert("switch_frames".to_string(), json!(metrics.switch_frames));
    payload.insert("total_rows".to_string(), json!(metrics.total_rows));
    payload.insert("window_rows".to_string(), json!(metrics.window_rows));
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics("frame_timing/repo_switch_during_scroll", payload);
}

pub(crate) fn emit_keyboard_arrow_scroll_sidecar(
    case_name: &str,
    stats: &FrameTimingStats,
    metrics: KeyboardArrowScrollMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("total_rows".to_string(), json!(metrics.total_rows));
    payload.insert("window_rows".to_string(), json!(metrics.window_rows));
    payload.insert(
        "scroll_step_rows".to_string(),
        json!(metrics.scroll_step_rows),
    );
    payload.insert("repeat_events".to_string(), json!(metrics.repeat_events));
    payload.insert(
        "rows_requested_total".to_string(),
        json!(metrics.rows_requested_total),
    );
    payload.insert(
        "unique_windows_visited".to_string(),
        json!(metrics.unique_windows_visited),
    );
    payload.insert("wrap_count".to_string(), json!(metrics.wrap_count));
    payload.insert(
        "final_start_row".to_string(),
        json!(metrics.final_start_row),
    );
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics(&format!("keyboard/{case_name}"), payload);
}

pub(crate) fn emit_keyboard_tab_focus_sidecar(
    case_name: &str,
    stats: &FrameTimingStats,
    metrics: KeyboardTabFocusCycleMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert(
        "focus_target_count".to_string(),
        json!(metrics.focus_target_count),
    );
    payload.insert("repo_tab_count".to_string(), json!(metrics.repo_tab_count));
    payload.insert(
        "detail_input_count".to_string(),
        json!(metrics.detail_input_count),
    );
    payload.insert("cycle_events".to_string(), json!(metrics.cycle_events));
    payload.insert(
        "unique_targets_visited".to_string(),
        json!(metrics.unique_targets_visited),
    );
    payload.insert("wrap_count".to_string(), json!(metrics.wrap_count));
    payload.insert("max_scan_len".to_string(), json!(metrics.max_scan_len));
    payload.insert(
        "final_target_index".to_string(),
        json!(metrics.final_target_index),
    );
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics(&format!("keyboard/{case_name}"), payload);
}

pub(crate) fn emit_keyboard_stage_unstage_toggle_sidecar(
    case_name: &str,
    stats: &FrameTimingStats,
    metrics: KeyboardStageUnstageToggleMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("path_count".to_string(), json!(metrics.path_count));
    payload.insert("toggle_events".to_string(), json!(metrics.toggle_events));
    payload.insert("effect_count".to_string(), json!(metrics.effect_count));
    payload.insert(
        "stage_effect_count".to_string(),
        json!(metrics.stage_effect_count),
    );
    payload.insert(
        "unstage_effect_count".to_string(),
        json!(metrics.unstage_effect_count),
    );
    payload.insert(
        "select_diff_effect_count".to_string(),
        json!(metrics.select_diff_effect_count),
    );
    payload.insert("ops_rev_delta".to_string(), json!(metrics.ops_rev_delta));
    payload.insert(
        "diff_state_rev_delta".to_string(),
        json!(metrics.diff_state_rev_delta),
    );
    payload.insert(
        "area_flip_count".to_string(),
        json!(metrics.area_flip_count),
    );
    payload.insert(
        "path_wrap_count".to_string(),
        json!(metrics.path_wrap_count),
    );
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics(&format!("keyboard/{case_name}"), payload);
}

pub(crate) fn emit_git_ops_sidecar(case_name: &str, metrics: &GitOpsMetrics) {
    let mut payload = Map::new();
    payload.insert("tracked_files".to_string(), json!(metrics.tracked_files));
    payload.insert("dirty_files".to_string(), json!(metrics.dirty_files));
    payload.insert("total_commits".to_string(), json!(metrics.total_commits));
    payload.insert(
        "requested_commits".to_string(),
        json!(metrics.requested_commits),
    );
    payload.insert(
        "commits_returned".to_string(),
        json!(metrics.commits_returned),
    );
    payload.insert("changed_files".to_string(), json!(metrics.changed_files));
    payload.insert("renamed_files".to_string(), json!(metrics.renamed_files));
    payload.insert("binary_files".to_string(), json!(metrics.binary_files));
    payload.insert("line_count".to_string(), json!(metrics.line_count));
    payload.insert("diff_lines".to_string(), json!(metrics.diff_lines));
    payload.insert("blame_lines".to_string(), json!(metrics.blame_lines));
    payload.insert(
        "blame_distinct_commits".to_string(),
        json!(metrics.blame_distinct_commits),
    );
    payload.insert(
        "file_history_commits".to_string(),
        json!(metrics.file_history_commits),
    );
    payload.insert("total_refs".to_string(), json!(metrics.total_refs));
    payload.insert(
        "branches_returned".to_string(),
        json!(metrics.branches_returned),
    );
    payload.insert("status_calls".to_string(), json!(metrics.status_calls));
    payload.insert("log_walk_calls".to_string(), json!(metrics.log_walk_calls));
    payload.insert("diff_calls".to_string(), json!(metrics.diff_calls));
    payload.insert("blame_calls".to_string(), json!(metrics.blame_calls));
    payload.insert(
        "ref_enumerate_calls".to_string(),
        json!(metrics.ref_enumerate_calls),
    );
    payload.insert("status_ms".to_string(), json!(metrics.status_ms));
    payload.insert("log_walk_ms".to_string(), json!(metrics.log_walk_ms));
    payload.insert("diff_ms".to_string(), json!(metrics.diff_ms));
    payload.insert("blame_ms".to_string(), json!(metrics.blame_ms));
    payload.insert(
        "ref_enumerate_ms".to_string(),
        json!(metrics.ref_enumerate_ms),
    );
    emit_sidecar_metrics(&format!("git_ops/{case_name}"), payload);
}

pub(crate) fn emit_staging_sidecar(case_name: &str, metrics: &StagingMetrics) {
    let mut payload = Map::new();
    payload.insert("file_count".to_string(), json!(metrics.file_count));
    payload.insert("effect_count".to_string(), json!(metrics.effect_count));
    payload.insert("ops_rev_delta".to_string(), json!(metrics.ops_rev_delta));
    payload.insert(
        "local_actions_delta".to_string(),
        json!(metrics.local_actions_delta),
    );
    payload.insert(
        "stage_effect_count".to_string(),
        json!(metrics.stage_effect_count),
    );
    payload.insert(
        "unstage_effect_count".to_string(),
        json!(metrics.unstage_effect_count),
    );
    emit_sidecar_metrics(&format!("staging/{case_name}"), payload);
}

pub(crate) fn emit_undo_redo_sidecar(case_name: &str, metrics: &UndoRedoMetrics) {
    let mut payload = Map::new();
    payload.insert("region_count".to_string(), json!(metrics.region_count));
    payload.insert(
        "apply_dispatches".to_string(),
        json!(metrics.apply_dispatches),
    );
    payload.insert(
        "reset_dispatches".to_string(),
        json!(metrics.reset_dispatches),
    );
    payload.insert(
        "replay_dispatches".to_string(),
        json!(metrics.replay_dispatches),
    );
    payload.insert(
        "conflict_rev_delta".to_string(),
        json!(metrics.conflict_rev_delta),
    );
    payload.insert("total_effects".to_string(), json!(metrics.total_effects));
    emit_sidecar_metrics(&format!("undo_redo/{case_name}"), payload);
}

pub(crate) fn emit_diff_scroll_sidecar(bench: &str, metrics: &LargeFileDiffScrollMetrics) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("total_lines".to_string(), json!(metrics.total_lines)),
            ("window_size".to_string(), json!(metrics.window_size)),
            ("start_line".to_string(), json!(metrics.start_line)),
            (
                "visible_text_bytes".to_string(),
                json!(metrics.visible_text_bytes),
            ),
            ("min_line_bytes".to_string(), json!(metrics.min_line_bytes)),
            (
                "language_detected".to_string(),
                json!(metrics.language_detected),
            ),
            (
                "syntax_mode_auto".to_string(),
                json!(metrics.syntax_mode_auto),
            ),
        ]),
    );
}

pub(crate) fn emit_text_input_prepaint_windowed_sidecar(
    bench: &str,
    metrics: &TextInputPrepaintWindowedMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("total_lines".to_string(), json!(metrics.total_lines)),
            ("viewport_rows".to_string(), json!(metrics.viewport_rows)),
            ("guard_rows".to_string(), json!(metrics.guard_rows)),
            (
                "max_shape_bytes".to_string(),
                json!(metrics.max_shape_bytes),
            ),
            (
                "cache_entries_after".to_string(),
                json!(metrics.cache_entries_after),
            ),
            ("cache_hits".to_string(), json!(metrics.cache_hits)),
            ("cache_misses".to_string(), json!(metrics.cache_misses)),
        ]),
    );
}

pub(crate) fn emit_text_input_runs_streamed_highlight_sidecar(
    bench: &str,
    metrics: &TextInputRunsStreamedHighlightMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("total_lines".to_string(), json!(metrics.total_lines)),
            ("visible_rows".to_string(), json!(metrics.visible_rows)),
            ("scroll_step".to_string(), json!(metrics.scroll_step)),
            (
                "total_highlights".to_string(),
                json!(metrics.total_highlights),
            ),
            (
                "visible_highlights".to_string(),
                json!(metrics.visible_highlights),
            ),
            (
                "visible_lines_with_highlights".to_string(),
                json!(metrics.visible_lines_with_highlights),
            ),
            ("density_dense".to_string(), json!(metrics.density_dense)),
            (
                "algorithm_streamed".to_string(),
                json!(metrics.algorithm_streamed),
            ),
        ]),
    );
}

pub(crate) fn emit_text_input_long_line_cap_sidecar(
    bench: &str,
    metrics: &TextInputLongLineCapMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("line_bytes".to_string(), json!(metrics.line_bytes)),
            (
                "max_shape_bytes".to_string(),
                json!(metrics.max_shape_bytes),
            ),
            ("capped_len".to_string(), json!(metrics.capped_len)),
            ("iterations".to_string(), json!(metrics.iterations)),
            ("cap_active".to_string(), json!(metrics.cap_active)),
        ]),
    );
}

pub(crate) fn emit_text_input_wrap_incremental_tabs_sidecar(
    bench: &str,
    metrics: &TextInputWrapIncrementalTabsMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("total_lines".to_string(), json!(metrics.total_lines)),
            ("line_bytes".to_string(), json!(metrics.line_bytes)),
            ("wrap_columns".to_string(), json!(metrics.wrap_columns)),
            ("edit_line_ix".to_string(), json!(metrics.edit_line_ix)),
            ("dirty_lines".to_string(), json!(metrics.dirty_lines)),
            (
                "total_rows_after".to_string(),
                json!(metrics.total_rows_after),
            ),
            (
                "recomputed_lines".to_string(),
                json!(metrics.recomputed_lines),
            ),
            (
                "incremental_patch".to_string(),
                json!(metrics.incremental_patch),
            ),
        ]),
    );
}

pub(crate) fn emit_text_input_wrap_incremental_burst_edits_sidecar(
    bench: &str,
    metrics: &TextInputWrapIncrementalBurstEditsMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("total_lines".to_string(), json!(metrics.total_lines)),
            (
                "edits_per_burst".to_string(),
                json!(metrics.edits_per_burst),
            ),
            ("wrap_columns".to_string(), json!(metrics.wrap_columns)),
            (
                "total_dirty_lines".to_string(),
                json!(metrics.total_dirty_lines),
            ),
            (
                "total_rows_after".to_string(),
                json!(metrics.total_rows_after),
            ),
            (
                "recomputed_lines".to_string(),
                json!(metrics.recomputed_lines),
            ),
            (
                "incremental_patch".to_string(),
                json!(metrics.incremental_patch),
            ),
        ]),
    );
}

pub(crate) fn emit_text_model_snapshot_clone_cost_sidecar(
    bench: &str,
    metrics: &TextModelSnapshotCloneCostMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("document_bytes".to_string(), json!(metrics.document_bytes)),
            ("line_starts".to_string(), json!(metrics.line_starts)),
            ("clone_count".to_string(), json!(metrics.clone_count)),
            (
                "sampled_prefix_bytes".to_string(),
                json!(metrics.sampled_prefix_bytes),
            ),
            ("snapshot_path".to_string(), json!(metrics.snapshot_path)),
        ]),
    );
}

pub(crate) fn emit_text_model_bulk_load_large_sidecar(
    bench: &str,
    metrics: &TextModelBulkLoadLargeMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("source_bytes".to_string(), json!(metrics.source_bytes)),
            (
                "document_bytes_after".to_string(),
                json!(metrics.document_bytes_after),
            ),
            (
                "line_starts_after".to_string(),
                json!(metrics.line_starts_after),
            ),
            ("chunk_count".to_string(), json!(metrics.chunk_count)),
            ("load_variant".to_string(), json!(metrics.load_variant)),
        ]),
    );
}

pub(crate) fn emit_text_model_fragmented_edits_sidecar(
    bench: &str,
    metrics: &TextModelFragmentedEditsMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("initial_bytes".to_string(), json!(metrics.initial_bytes)),
            ("edit_count".to_string(), json!(metrics.edit_count)),
            ("deleted_bytes".to_string(), json!(metrics.deleted_bytes)),
            ("inserted_bytes".to_string(), json!(metrics.inserted_bytes)),
            ("final_bytes".to_string(), json!(metrics.final_bytes)),
            (
                "line_starts_after".to_string(),
                json!(metrics.line_starts_after),
            ),
            (
                "readback_operations".to_string(),
                json!(metrics.readback_operations),
            ),
            ("string_control".to_string(), json!(metrics.string_control)),
        ]),
    );
}

pub(crate) fn emit_large_html_syntax_sidecar(bench_name: &str, metrics: LargeHtmlSyntaxMetrics) {
    let mut payload = Map::new();
    payload.insert("text_bytes".to_string(), json!(metrics.text_bytes));
    payload.insert("line_count".to_string(), json!(metrics.line_count));
    payload.insert("window_lines".to_string(), json!(metrics.window_lines));
    payload.insert("start_line".to_string(), json!(metrics.start_line));
    payload.insert(
        "visible_byte_len".to_string(),
        json!(metrics.visible_byte_len),
    );
    payload.insert(
        "prepared_document_available".to_string(),
        json!(metrics.prepared_document_available),
    );
    payload.insert(
        "cache_document_present".to_string(),
        json!(metrics.cache_document_present),
    );
    payload.insert("pending".to_string(), json!(metrics.pending));
    payload.insert(
        "highlight_spans".to_string(),
        json!(metrics.highlight_spans),
    );
    payload.insert("cache_hits".to_string(), json!(metrics.cache_hits));
    payload.insert("cache_misses".to_string(), json!(metrics.cache_misses));
    payload.insert(
        "cache_evictions".to_string(),
        json!(metrics.cache_evictions),
    );
    payload.insert("chunk_build_ms".to_string(), json!(metrics.chunk_build_ms));
    payload.insert("loaded_chunks".to_string(), json!(metrics.loaded_chunks));
    emit_sidecar_metrics(bench_name, payload);
}

pub(crate) fn emit_worktree_preview_render_sidecar(
    bench: &str,
    metrics: &WorktreePreviewRenderMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("total_lines".to_string(), json!(metrics.total_lines)),
            ("window_size".to_string(), json!(metrics.window_size)),
            ("line_bytes".to_string(), json!(metrics.line_bytes)),
            (
                "prepared_document_available".to_string(),
                json!(metrics.prepared_document_available),
            ),
            (
                "syntax_mode_auto".to_string(),
                json!(metrics.syntax_mode_auto),
            ),
        ]),
    );
}

pub(crate) fn emit_markdown_preview_scroll_sidecar(
    bench: &str,
    metrics: &MarkdownPreviewScrollMetrics,
) {
    emit_sidecar_metrics(
        bench,
        Map::from_iter([
            ("total_rows".to_string(), json!(metrics.total_rows)),
            ("start_row".to_string(), json!(metrics.start_row)),
            ("window_size".to_string(), json!(metrics.window_size)),
            ("rows_rendered".to_string(), json!(metrics.rows_rendered)),
            (
                "scroll_step_rows".to_string(),
                json!(metrics.scroll_step_rows),
            ),
            ("long_rows".to_string(), json!(metrics.long_rows)),
            ("long_row_bytes".to_string(), json!(metrics.long_row_bytes)),
            ("heading_rows".to_string(), json!(metrics.heading_rows)),
            ("list_rows".to_string(), json!(metrics.list_rows)),
            ("table_rows".to_string(), json!(metrics.table_rows)),
            ("code_rows".to_string(), json!(metrics.code_rows)),
            (
                "blockquote_rows".to_string(),
                json!(metrics.blockquote_rows),
            ),
            ("details_rows".to_string(), json!(metrics.details_rows)),
        ]),
    );
}

pub(crate) fn emit_markdown_preview_first_window_sidecar(
    window: usize,
    metrics: &MarkdownPreviewFirstWindowMetrics,
) {
    let mut payload = Map::new();
    payload.insert("old_total_rows".to_string(), json!(metrics.old_total_rows));
    payload.insert("new_total_rows".to_string(), json!(metrics.new_total_rows));
    payload.insert(
        "old_rows_rendered".to_string(),
        json!(metrics.old_rows_rendered),
    );
    payload.insert(
        "new_rows_rendered".to_string(),
        json!(metrics.new_rows_rendered),
    );
    emit_sidecar_metrics(
        &format!("diff_open_markdown_preview_first_window/{window}"),
        payload,
    );
}

pub(crate) fn emit_image_preview_first_paint_sidecar(metrics: &ImagePreviewFirstPaintMetrics) {
    let mut payload = Map::new();
    payload.insert("old_bytes".to_string(), json!(metrics.old_bytes));
    payload.insert("new_bytes".to_string(), json!(metrics.new_bytes));
    payload.insert("total_bytes".to_string(), json!(metrics.total_bytes));
    payload.insert(
        "images_rendered".to_string(),
        json!(metrics.images_rendered),
    );
    payload.insert(
        "placeholder_cells".to_string(),
        json!(metrics.placeholder_cells),
    );
    payload.insert("divider_count".to_string(), json!(metrics.divider_count));
    emit_sidecar_metrics("diff_open_image_preview_first_paint", payload);
}

pub(crate) fn emit_svg_dual_path_first_window_sidecar(
    window: usize,
    metrics: &SvgDualPathFirstWindowMetrics,
) {
    let mut payload = Map::new();
    payload.insert("old_svg_bytes".to_string(), json!(metrics.old_svg_bytes));
    payload.insert("new_svg_bytes".to_string(), json!(metrics.new_svg_bytes));
    payload.insert(
        "rasterize_success".to_string(),
        json!(metrics.rasterize_success),
    );
    payload.insert(
        "fallback_triggered".to_string(),
        json!(metrics.fallback_triggered),
    );
    payload.insert(
        "rasterized_png_bytes".to_string(),
        json!(metrics.rasterized_png_bytes),
    );
    payload.insert(
        "images_rendered".to_string(),
        json!(metrics.images_rendered),
    );
    payload.insert("divider_count".to_string(), json!(metrics.divider_count));
    emit_sidecar_metrics(
        &format!("diff_open_svg_dual_path_first_window/{window}"),
        payload,
    );
}

pub(crate) fn emit_file_diff_open_sidecar(bench_name: &str, metrics: &FileDiffOpenMetrics) {
    let mut payload = Map::new();
    payload.insert("rows_requested".to_string(), json!(metrics.rows_requested));
    payload.insert(
        "split_total_rows".to_string(),
        json!(metrics.split_total_rows),
    );
    payload.insert(
        "split_rows_painted".to_string(),
        json!(metrics.split_rows_painted),
    );
    payload.insert(
        "inline_total_rows".to_string(),
        json!(metrics.inline_total_rows),
    );
    payload.insert(
        "inline_rows_painted".to_string(),
        json!(metrics.inline_rows_painted),
    );
    emit_sidecar_metrics(bench_name, payload);
}

pub(crate) fn emit_conflict_compare_first_window_sidecar(
    window: usize,
    metrics: &ConflictCompareFirstWindowMetrics,
) {
    let mut payload = Map::new();
    payload.insert(
        "total_diff_rows".to_string(),
        json!(metrics.total_diff_rows),
    );
    payload.insert(
        "total_visible_rows".to_string(),
        json!(metrics.total_visible_rows),
    );
    payload.insert("rows_rendered".to_string(), json!(metrics.rows_rendered));
    payload.insert("conflict_count".to_string(), json!(metrics.conflict_count));
    emit_sidecar_metrics(
        &format!("diff_open_conflict_compare_first_window/{window}"),
        payload,
    );
}

pub(crate) fn emit_diff_refresh_sidecar(sub: &str, metrics: &DiffRefreshMetrics) {
    let mut payload = Map::new();
    payload.insert(
        "diff_cache_rekeys".to_string(),
        json!(metrics.diff_cache_rekeys),
    );
    payload.insert("full_rebuilds".to_string(), json!(metrics.full_rebuilds));
    payload.insert(
        "content_signature_matches".to_string(),
        json!(metrics.content_signature_matches),
    );
    payload.insert("rows_preserved".to_string(), json!(metrics.rows_preserved));
    payload.insert("rebuild_rows".to_string(), json!(metrics.rebuild_rows));
    emit_sidecar_metrics(
        &format!("diff_refresh_rev_only_same_content/{sub}"),
        payload,
    );
}

pub(crate) fn emit_window_resize_layout_sidecar(bench: &str, metrics: &WindowResizeLayoutMetrics) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("steps".to_string(), json!(metrics.steps)),
            (
                "layout_recomputes".to_string(),
                json!(metrics.layout_recomputes),
            ),
            ("min_main_w_px".to_string(), json!(metrics.min_main_w_px)),
            ("max_main_w_px".to_string(), json!(metrics.max_main_w_px)),
            (
                "clamp_at_zero_count".to_string(),
                json!(metrics.clamp_at_zero_count),
            ),
        ]),
    );
}

pub(crate) fn emit_window_resize_layout_extreme_sidecar(
    bench: &str,
    metrics: &WindowResizeLayoutExtremeMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("steps".to_string(), json!(metrics.steps)),
            (
                "layout_recomputes".to_string(),
                json!(metrics.layout_recomputes),
            ),
            (
                "history_visibility_recomputes".to_string(),
                json!(metrics.history_visibility_recomputes),
            ),
            (
                "diff_width_recomputes".to_string(),
                json!(metrics.diff_width_recomputes),
            ),
            (
                "history_commits".to_string(),
                json!(metrics.history_commits),
            ),
            (
                "history_window_rows".to_string(),
                json!(metrics.history_window_rows),
            ),
            (
                "history_rows_processed_total".to_string(),
                json!(metrics.history_rows_processed_total),
            ),
            (
                "history_columns_hidden_steps".to_string(),
                json!(metrics.history_columns_hidden_steps),
            ),
            (
                "history_all_columns_visible_steps".to_string(),
                json!(metrics.history_all_columns_visible_steps),
            ),
            ("diff_lines".to_string(), json!(metrics.diff_lines)),
            (
                "diff_window_rows".to_string(),
                json!(metrics.diff_window_rows),
            ),
            (
                "diff_split_total_rows".to_string(),
                json!(metrics.diff_split_total_rows),
            ),
            (
                "diff_rows_processed_total".to_string(),
                json!(metrics.diff_rows_processed_total),
            ),
            (
                "diff_narrow_fallback_steps".to_string(),
                json!(metrics.diff_narrow_fallback_steps),
            ),
            ("min_main_w_px".to_string(), json!(metrics.min_main_w_px)),
            ("max_main_w_px".to_string(), json!(metrics.max_main_w_px)),
        ]),
    );
}

pub(crate) fn emit_history_column_resize_sidecar(
    bench: &str,
    metrics: &HistoryColumnResizeMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("steps".to_string(), json!(metrics.steps)),
            (
                "width_clamp_recomputes".to_string(),
                json!(metrics.width_clamp_recomputes),
            ),
            (
                "visible_column_recomputes".to_string(),
                json!(metrics.visible_column_recomputes),
            ),
            (
                "columns_hidden_count".to_string(),
                json!(metrics.columns_hidden_count),
            ),
            (
                "clamp_at_min_count".to_string(),
                json!(metrics.clamp_at_min_count),
            ),
            (
                "clamp_at_max_count".to_string(),
                json!(metrics.clamp_at_max_count),
            ),
        ]),
    );
}

pub(crate) fn emit_repo_tab_drag_sidecar(bench: &str, metrics: &RepoTabDragMetrics) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("tab_count".to_string(), json!(metrics.tab_count)),
            ("hit_test_steps".to_string(), json!(metrics.hit_test_steps)),
            ("reorder_steps".to_string(), json!(metrics.reorder_steps)),
            (
                "effects_emitted".to_string(),
                json!(metrics.effects_emitted),
            ),
            ("noop_reorders".to_string(), json!(metrics.noop_reorders)),
        ]),
    );
}

pub(crate) fn emit_pane_resize_drag_sidecar(bench: &str, metrics: &PaneResizeDragMetrics) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("steps".to_string(), json!(metrics.steps)),
            (
                "width_bounds_recomputes".to_string(),
                json!(metrics.width_bounds_recomputes),
            ),
            (
                "layout_recomputes".to_string(),
                json!(metrics.layout_recomputes),
            ),
            (
                "min_pane_width_px".to_string(),
                json!(metrics.min_pane_width_px),
            ),
            (
                "max_pane_width_px".to_string(),
                json!(metrics.max_pane_width_px),
            ),
            (
                "min_main_width_px".to_string(),
                json!(metrics.min_main_width_px),
            ),
            (
                "max_main_width_px".to_string(),
                json!(metrics.max_main_width_px),
            ),
            (
                "clamp_at_min_count".to_string(),
                json!(metrics.clamp_at_min_count),
            ),
            (
                "clamp_at_max_count".to_string(),
                json!(metrics.clamp_at_max_count),
            ),
        ]),
    );
}

pub(crate) fn emit_diff_split_resize_drag_sidecar(
    bench: &str,
    metrics: &DiffSplitResizeDragMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("steps".to_string(), json!(metrics.steps)),
            (
                "ratio_recomputes".to_string(),
                json!(metrics.ratio_recomputes),
            ),
            (
                "column_width_recomputes".to_string(),
                json!(metrics.column_width_recomputes),
            ),
            ("min_ratio".to_string(), json!(metrics.min_ratio)),
            ("max_ratio".to_string(), json!(metrics.max_ratio)),
            (
                "min_left_col_px".to_string(),
                json!(metrics.min_left_col_px),
            ),
            (
                "max_left_col_px".to_string(),
                json!(metrics.max_left_col_px),
            ),
            (
                "min_right_col_px".to_string(),
                json!(metrics.min_right_col_px),
            ),
            (
                "max_right_col_px".to_string(),
                json!(metrics.max_right_col_px),
            ),
            (
                "clamp_at_min_count".to_string(),
                json!(metrics.clamp_at_min_count),
            ),
            (
                "clamp_at_max_count".to_string(),
                json!(metrics.clamp_at_max_count),
            ),
            (
                "narrow_fallback_count".to_string(),
                json!(metrics.narrow_fallback_count),
            ),
        ]),
    );
}

pub(crate) fn emit_resolved_output_recompute_sidecar(
    bench: &str,
    metrics: &ResolvedOutputRecomputeMetrics,
) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            (
                "requested_lines".to_string(),
                json!(metrics.requested_lines),
            ),
            (
                "conflict_blocks".to_string(),
                json!(metrics.conflict_blocks),
            ),
            (
                "unresolved_blocks".to_string(),
                json!(metrics.unresolved_blocks),
            ),
            (
                "both_choice_blocks".to_string(),
                json!(metrics.both_choice_blocks),
            ),
            ("outline_rows".to_string(), json!(metrics.outline_rows)),
            ("marker_rows".to_string(), json!(metrics.marker_rows)),
            ("manual_rows".to_string(), json!(metrics.manual_rows)),
            ("dirty_rows".to_string(), json!(metrics.dirty_rows)),
            (
                "recomputed_rows".to_string(),
                json!(metrics.recomputed_rows),
            ),
            (
                "fallback_full_recompute".to_string(),
                json!(u64::from(metrics.fallback_full_recompute)),
            ),
        ]),
    );
}

pub(crate) fn emit_scrollbar_drag_step_sidecar(bench: &str, metrics: &ScrollbarDragStepMetrics) {
    emit_sidecar_metrics(
        bench,
        serde_json::Map::from_iter([
            ("steps".to_string(), json!(metrics.steps)),
            (
                "thumb_metric_recomputes".to_string(),
                json!(metrics.thumb_metric_recomputes),
            ),
            (
                "scroll_offset_recomputes".to_string(),
                json!(metrics.scroll_offset_recomputes),
            ),
            ("viewport_h".to_string(), json!(metrics.viewport_h)),
            ("max_offset".to_string(), json!(metrics.max_offset)),
            ("min_scroll_y".to_string(), json!(metrics.min_scroll_y)),
            ("max_scroll_y".to_string(), json!(metrics.max_scroll_y)),
            (
                "min_thumb_offset_px".to_string(),
                json!(metrics.min_thumb_offset_px),
            ),
            (
                "max_thumb_offset_px".to_string(),
                json!(metrics.max_thumb_offset_px),
            ),
            (
                "min_thumb_length_px".to_string(),
                json!(metrics.min_thumb_length_px),
            ),
            (
                "max_thumb_length_px".to_string(),
                json!(metrics.max_thumb_length_px),
            ),
            (
                "clamp_at_top_count".to_string(),
                json!(metrics.clamp_at_top_count),
            ),
            (
                "clamp_at_bottom_count".to_string(),
                json!(metrics.clamp_at_bottom_count),
            ),
        ]),
    );
}

pub(crate) fn emit_commit_search_filter_sidecar(
    case_name: &str,
    metrics: &CommitSearchFilterMetrics,
) {
    let mut payload = Map::new();
    payload.insert("total_commits".to_string(), json!(metrics.total_commits));
    payload.insert("query_len".to_string(), json!(metrics.query_len));
    payload.insert("matches_found".to_string(), json!(metrics.matches_found));
    payload.insert(
        "incremental_matches".to_string(),
        json!(metrics.incremental_matches),
    );
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

pub(crate) fn emit_file_fuzzy_find_sidecar(case_name: &str, metrics: &FileFuzzyFindMetrics) {
    let mut payload = Map::new();
    payload.insert("total_files".to_string(), json!(metrics.total_files));
    payload.insert("query_len".to_string(), json!(metrics.query_len));
    payload.insert("matches_found".to_string(), json!(metrics.matches_found));
    payload.insert("prior_matches".to_string(), json!(metrics.prior_matches));
    payload.insert("files_scanned".to_string(), json!(metrics.files_scanned));
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

pub(crate) fn emit_in_diff_text_search_sidecar(case_name: &str, metrics: &InDiffTextSearchMetrics) {
    let mut payload = Map::new();
    payload.insert("total_lines".to_string(), json!(metrics.total_lines));
    payload.insert(
        "visible_rows_scanned".to_string(),
        json!(metrics.visible_rows_scanned),
    );
    payload.insert("query_len".to_string(), json!(metrics.query_len));
    payload.insert("matches_found".to_string(), json!(metrics.matches_found));
    payload.insert("prior_matches".to_string(), json!(metrics.prior_matches));
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

pub(crate) fn emit_file_preview_text_search_sidecar(
    case_name: &str,
    metrics: &FilePreviewTextSearchMetrics,
) {
    let mut payload = Map::new();
    payload.insert("total_lines".to_string(), json!(metrics.total_lines));
    payload.insert("source_bytes".to_string(), json!(metrics.source_bytes));
    payload.insert("query_len".to_string(), json!(metrics.query_len));
    payload.insert("matches_found".to_string(), json!(metrics.matches_found));
    payload.insert("prior_matches".to_string(), json!(metrics.prior_matches));
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

pub(crate) fn emit_file_diff_ctrl_f_open_type_sidecar(
    case_name: &str,
    metrics: &FileDiffCtrlFOpenTypeMetrics,
) {
    let mut payload = Map::new();
    payload.insert("total_lines".to_string(), json!(metrics.total_lines));
    payload.insert("total_rows".to_string(), json!(metrics.total_rows));
    payload.insert(
        "visible_window_rows".to_string(),
        json!(metrics.visible_window_rows),
    );
    payload.insert("search_opened".to_string(), json!(metrics.search_opened));
    payload.insert("typed_chars".to_string(), json!(metrics.typed_chars));
    payload.insert("query_steps".to_string(), json!(metrics.query_steps));
    payload.insert(
        "final_query_len".to_string(),
        json!(metrics.final_query_len),
    );
    payload.insert("rows_scanned".to_string(), json!(metrics.rows_scanned));
    payload.insert("full_rescans".to_string(), json!(metrics.full_rescans));
    payload.insert(
        "refinement_steps".to_string(),
        json!(metrics.refinement_steps),
    );
    payload.insert("final_matches".to_string(), json!(metrics.final_matches));
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

pub(crate) fn emit_fs_event_sidecar(scenario: &str, metrics: &FsEventMetrics) {
    let mut payload = Map::new();
    payload.insert("tracked_files".to_string(), json!(metrics.tracked_files));
    payload.insert("mutation_files".to_string(), json!(metrics.mutation_files));
    payload.insert(
        "dirty_files_detected".to_string(),
        json!(metrics.dirty_files_detected),
    );
    payload.insert(
        "status_entries_total".to_string(),
        json!(metrics.status_entries_total),
    );
    payload.insert(
        "false_positives".to_string(),
        json!(metrics.false_positives),
    );
    payload.insert(
        "coalesced_saves".to_string(),
        json!(metrics.coalesced_saves),
    );
    payload.insert("status_calls".to_string(), json!(metrics.status_calls));
    payload.insert("status_ms".to_string(), json!(metrics.status_ms));
    emit_sidecar_metrics(&format!("fs_event/{scenario}"), payload);
}

pub(crate) fn emit_network_sidecar(
    case_name: &str,
    stats: &FrameTimingStats,
    metrics: &NetworkMetrics,
) {
    let mut payload = stats.to_sidecar_metrics();
    payload.insert("total_frames".to_string(), json!(metrics.total_frames));
    payload.insert("scroll_frames".to_string(), json!(metrics.scroll_frames));
    payload.insert(
        "progress_updates".to_string(),
        json!(metrics.progress_updates),
    );
    payload.insert("render_passes".to_string(), json!(metrics.render_passes));
    payload.insert(
        "output_tail_lines".to_string(),
        json!(metrics.output_tail_lines),
    );
    payload.insert(
        "tail_trim_events".to_string(),
        json!(metrics.tail_trim_events),
    );
    payload.insert("rendered_bytes".to_string(), json!(metrics.rendered_bytes));
    payload.insert("total_rows".to_string(), json!(metrics.total_rows));
    payload.insert("window_rows".to_string(), json!(metrics.window_rows));
    payload.insert("bar_width".to_string(), json!(metrics.bar_width));
    payload.insert(
        "cancel_frames_until_stopped".to_string(),
        json!(metrics.cancel_frames_until_stopped),
    );
    payload.insert(
        "drained_updates_after_cancel".to_string(),
        json!(metrics.drained_updates_after_cancel),
    );
    payload.insert(
        "total_capture_ms".to_string(),
        json!(stats.total_capture_ns as f64 / 1_000_000.0),
    );
    payload.insert(
        "p99_exceeds_2x_budget".to_string(),
        json!(u64::from(stats.p99_exceeds_2x_budget())),
    );
    emit_sidecar_metrics(&format!("network/{case_name}"), payload);
}

pub(crate) fn emit_clipboard_sidecar(case_name: &str, metrics: &ClipboardMetrics) {
    let mut payload = Map::new();
    payload.insert("total_lines".to_string(), json!(metrics.total_lines));
    payload.insert("total_bytes".to_string(), json!(metrics.total_bytes));
    payload.insert(
        "line_iterations".to_string(),
        json!(metrics.line_iterations),
    );
    payload.insert(
        "allocations_approx".to_string(),
        json!(metrics.allocations_approx),
    );
    emit_sidecar_metrics(&format!("clipboard/{case_name}"), payload);
}

pub(crate) fn emit_display_sidecar(case_name: &str, metrics: &DisplayMetrics) {
    let mut payload = Map::new();
    payload.insert(
        "scale_factors_tested".to_string(),
        json!(metrics.scale_factors_tested),
    );
    payload.insert(
        "total_layout_passes".to_string(),
        json!(metrics.total_layout_passes),
    );
    payload.insert(
        "total_rows_rendered".to_string(),
        json!(metrics.total_rows_rendered),
    );
    payload.insert(
        "history_rows_per_pass".to_string(),
        json!(metrics.history_rows_per_pass),
    );
    payload.insert(
        "diff_rows_per_pass".to_string(),
        json!(metrics.diff_rows_per_pass),
    );
    payload.insert(
        "windows_rendered".to_string(),
        json!(metrics.windows_rendered),
    );
    payload.insert(
        "re_layout_passes".to_string(),
        json!(metrics.re_layout_passes),
    );
    payload.insert(
        "layout_width_min_px".to_string(),
        json!(metrics.layout_width_min_px),
    );
    payload.insert(
        "layout_width_max_px".to_string(),
        json!(metrics.layout_width_max_px),
    );
    emit_sidecar_metrics(&format!("display/{case_name}"), payload);
}

pub(crate) fn emit_real_repo_sidecar(case_name: &str, metrics: &RealRepoMetrics) {
    let mut payload = Map::new();
    payload.insert(
        "worktree_file_count".to_string(),
        json!(metrics.worktree_file_count),
    );
    payload.insert("status_entries".to_string(), json!(metrics.status_entries));
    payload.insert("local_branches".to_string(), json!(metrics.local_branches));
    payload.insert(
        "remote_branches".to_string(),
        json!(metrics.remote_branches),
    );
    payload.insert("remotes".to_string(), json!(metrics.remotes));
    payload.insert("commits_loaded".to_string(), json!(metrics.commits_loaded));
    payload.insert(
        "log_pages_loaded".to_string(),
        json!(metrics.log_pages_loaded),
    );
    payload.insert(
        "next_cursor_present".to_string(),
        json!(metrics.next_cursor_present),
    );
    payload.insert("sidebar_rows".to_string(), json!(metrics.sidebar_rows));
    payload.insert("graph_rows".to_string(), json!(metrics.graph_rows));
    payload.insert(
        "max_graph_lanes".to_string(),
        json!(metrics.max_graph_lanes),
    );
    payload.insert(
        "history_windows_scanned".to_string(),
        json!(metrics.history_windows_scanned),
    );
    payload.insert(
        "history_rows_scanned".to_string(),
        json!(metrics.history_rows_scanned),
    );
    payload.insert("conflict_files".to_string(), json!(metrics.conflict_files));
    payload.insert(
        "conflict_regions".to_string(),
        json!(metrics.conflict_regions),
    );
    payload.insert(
        "selected_conflict_bytes".to_string(),
        json!(metrics.selected_conflict_bytes),
    );
    payload.insert("diff_lines".to_string(), json!(metrics.diff_lines));
    payload.insert("file_old_bytes".to_string(), json!(metrics.file_old_bytes));
    payload.insert("file_new_bytes".to_string(), json!(metrics.file_new_bytes));
    payload.insert(
        "split_rows_painted".to_string(),
        json!(metrics.split_rows_painted),
    );
    payload.insert(
        "inline_rows_painted".to_string(),
        json!(metrics.inline_rows_painted),
    );
    payload.insert("status_calls".to_string(), json!(metrics.status_calls));
    payload.insert("log_walk_calls".to_string(), json!(metrics.log_walk_calls));
    payload.insert("diff_calls".to_string(), json!(metrics.diff_calls));
    payload.insert(
        "ref_enumerate_calls".to_string(),
        json!(metrics.ref_enumerate_calls),
    );
    payload.insert("status_ms".to_string(), json!(metrics.status_ms));
    payload.insert("log_walk_ms".to_string(), json!(metrics.log_walk_ms));
    payload.insert("diff_ms".to_string(), json!(metrics.diff_ms));
    payload.insert(
        "ref_enumerate_ms".to_string(),
        json!(metrics.ref_enumerate_ms),
    );
    emit_sidecar_metrics(&format!("real_repo/{case_name}"), payload);
}
