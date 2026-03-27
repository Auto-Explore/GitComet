use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use gitcomet_core::file_diff::BenchmarkReplacementDistanceBackend;
use gitcomet_ui_gpui::benchmarks::{
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
    MergeOpenBootstrapFixture, MergeOpenBootstrapMetrics, NetworkFixture, NetworkMetrics,
    OpenRepoFixture, OpenRepoMetrics, PaneResizeDragMetrics, PaneResizeDragStepFixture,
    PaneResizeTarget, PatchDiffFirstWindowMetrics, PatchDiffPagedRowsFixture,
    PatchDiffSearchQueryUpdateFixture, PathDisplayCacheChurnFixture, PathDisplayCacheChurnMetrics,
    RapidCommitSelectionFixture, RapidCommitSelectionMetrics, RealRepoFixture, RealRepoMetrics,
    RealRepoScenario, ReplacementAlignmentFixture, RepoSwitchDuringScrollFixture,
    RepoSwitchDuringScrollMetrics, RepoSwitchFixture, RepoSwitchMetrics, RepoTabDragFixture,
    RepoTabDragMetrics, ResolvedOutputRecomputeIncrementalFixture, ResolvedOutputRecomputeMetrics,
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
use gitcomet_ui_gpui::perf_alloc::{
    PerfAllocMetrics, PerfTrackingAllocator, TRACKING_MIMALLOC, measure_allocations,
};
use gitcomet_ui_gpui::perf_ram_guard::install_benchmark_process_ram_guard;
use gitcomet_ui_gpui::perf_sidecar::{PerfSidecarReport, write_criterion_sidecar};
use serde_json::{Map, Value, json};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::env;
use std::time::{Duration, Instant};

#[global_allocator]
static GLOBAL: &PerfTrackingAllocator = &TRACKING_MIMALLOC;

thread_local! {
    static PENDING_SIDECAR_ALLOCATIONS: RefCell<VecDeque<PerfAllocMetrics>> = const {
        RefCell::new(VecDeque::new())
    };
}

const SUPPRESS_MISSING_REAL_REPO_NOTICE_ENV: &str =
    "GITCOMET_PERF_SUPPRESS_MISSING_REAL_REPO_NOTICE";

fn env_usize(key: &str, default: usize) -> usize {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(default)
}

fn env_string(key: &str) -> Option<String> {
    let value = env::var(key).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

fn env_flag(key: &str) -> bool {
    env::var(key)
        .ok()
        .as_deref()
        .map(parse_bool_flag)
        .unwrap_or(false)
}

fn parse_bool_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn markdown_preview_measurement_time() -> Duration {
    Duration::from_millis(
        env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_MEASUREMENT_MS", 250).max(250) as u64,
    )
}

fn benchmark_criterion() -> Criterion {
    install_benchmark_process_ram_guard();
    Criterion::default()
}

fn settle_markdown_allocator_pages() {
    // The markdown preview fixtures are large enough that dropping them and giving
    // mimalloc a short purge window keeps the process-wide RAM guard aligned with
    // live working set before the next benchmark group begins.
    std::thread::sleep(Duration::from_secs(1));
}

fn measure_sidecar_allocations<T>(f: impl FnOnce() -> T) -> T {
    let (value, allocations) = measure_allocations(f);
    PENDING_SIDECAR_ALLOCATIONS.with(|pending| pending.borrow_mut().push_back(allocations));
    value
}

fn take_pending_sidecar_allocations() -> PerfAllocMetrics {
    PENDING_SIDECAR_ALLOCATIONS
        .with(|pending| pending.borrow_mut().pop_front())
        .unwrap_or_else(|| {
            panic!("missing allocation snapshot for sidecar emission; wrap the representative sidecar run with measure_sidecar_allocations")
        })
}

fn emit_sidecar_metrics(bench: &str, mut metrics: Map<String, Value>) {
    let allocations = take_pending_sidecar_allocations();
    allocations.append_to_payload(&mut metrics);
    let report = PerfSidecarReport::new(bench, metrics);
    write_criterion_sidecar(&report).unwrap_or_else(|err| panic!("{err}"));
}

fn emit_allocation_only_sidecar(bench: &str) {
    emit_sidecar_metrics(bench, Map::new());
}

fn emit_patch_diff_first_window_sidecar(
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

fn emit_patch_diff_sidecar(
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

fn emit_open_repo_sidecar(case_name: &str, metrics: &OpenRepoMetrics) {
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

fn bench_open_repo(c: &mut Criterion) {
    // Note: Criterion's "Warming up for Xs" can look "stuck" if a single iteration takes longer
    // than the warm-up duration. Keep defaults moderate; scale up via env vars for stress runs.
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let history_heavy_commits = env_usize(
        "GITCOMET_BENCH_HISTORY_HEAVY_COMMITS",
        commits.saturating_mul(3),
    );
    let branch_heavy_local_branches = env_usize(
        "GITCOMET_BENCH_BRANCH_HEAVY_LOCAL_BRANCHES",
        local_branches.saturating_mul(6),
    );
    let branch_heavy_remote_branches = env_usize(
        "GITCOMET_BENCH_BRANCH_HEAVY_REMOTE_BRANCHES",
        remote_branches.saturating_mul(4),
    );
    let branch_heavy_remotes = env_usize("GITCOMET_BENCH_BRANCH_HEAVY_REMOTES", remotes.max(8));
    let extreme_fanout_commits = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_COMMITS", 1_000);
    let extreme_fanout_local_branches =
        env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_LOCAL_BRANCHES", 1_000);
    let extreme_fanout_remote_branches =
        env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_REMOTE_BRANCHES", 10_000);
    let extreme_fanout_remotes = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_REMOTES", 1);
    let extreme_fanout_worktrees = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_WORKTREES", 5_000);
    let extreme_fanout_submodules = env_usize("GITCOMET_BENCH_OPEN_REPO_EXTREME_SUBMODULES", 1_000);

    let balanced = OpenRepoFixture::new(commits, local_branches, remote_branches, remotes);
    let history_heavy = OpenRepoFixture::new(
        history_heavy_commits,
        local_branches.max(8) / 2,
        remote_branches.max(16) / 2,
        remotes.max(1),
    );
    let branch_heavy = OpenRepoFixture::new(
        commits.max(500) / 5,
        branch_heavy_local_branches,
        branch_heavy_remote_branches,
        branch_heavy_remotes,
    );
    let extreme_metadata_fanout = OpenRepoFixture::with_sidebar_fanout(
        extreme_fanout_commits,
        extreme_fanout_local_branches,
        extreme_fanout_remote_branches,
        extreme_fanout_remotes,
        extreme_fanout_worktrees,
        extreme_fanout_submodules,
    );

    let mut group = c.benchmark_group("open_repo");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("balanced"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = balanced.run_with_metrics();
            }
            let elapsed = start.elapsed();
            let (_, metrics) = measure_sidecar_allocations(|| balanced.run_with_metrics());
            emit_open_repo_sidecar("balanced", &metrics);
            elapsed
        })
    });
    group.bench_function(BenchmarkId::from_parameter("history_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = history_heavy.run_with_metrics();
            }
            let elapsed = start.elapsed();
            let (_, metrics) = measure_sidecar_allocations(|| history_heavy.run_with_metrics());
            emit_open_repo_sidecar("history_heavy", &metrics);
            elapsed
        })
    });
    group.bench_function(BenchmarkId::from_parameter("branch_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = branch_heavy.run_with_metrics();
            }
            let elapsed = start.elapsed();
            let (_, metrics) = measure_sidecar_allocations(|| branch_heavy.run_with_metrics());
            emit_open_repo_sidecar("branch_heavy", &metrics);
            elapsed
        })
    });
    group.bench_function(
        BenchmarkId::from_parameter("extreme_metadata_fanout"),
        |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    let _ = extreme_metadata_fanout.run_with_metrics();
                }
                let elapsed = start.elapsed();
                let (_, metrics) =
                    measure_sidecar_allocations(|| extreme_metadata_fanout.run_with_metrics());
                emit_open_repo_sidecar("extreme_metadata_fanout", &metrics);
                elapsed
            })
        },
    );
    group.finish();
}

fn bench_branch_sidebar(c: &mut Criterion) {
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let worktrees = env_usize("GITCOMET_BENCH_WORKTREES", 80);
    let submodules = env_usize("GITCOMET_BENCH_SUBMODULES", 150);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 300);

    let local_heavy = BranchSidebarFixture::new(
        local_branches.saturating_mul(8),
        remote_branches.max(32) / 8,
        remotes.max(1),
        0,
        0,
        0,
    );
    let remote_fanout = BranchSidebarFixture::new(
        local_branches.max(32) / 4,
        remote_branches.saturating_mul(6),
        remotes.max(12),
        0,
        0,
        0,
    );
    let aux_lists_heavy = BranchSidebarFixture::new(
        local_branches,
        remote_branches,
        remotes.max(2),
        worktrees,
        submodules,
        stashes,
    );

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("local_heavy"), |b| {
        b.iter(|| local_heavy.run())
    });
    group.bench_function(BenchmarkId::from_parameter("remote_fanout"), |b| {
        b.iter(|| remote_fanout.run())
    });
    group.bench_function(BenchmarkId::from_parameter("aux_lists_heavy"), |b| {
        b.iter(|| aux_lists_heavy.run())
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| local_heavy.run());
    emit_allocation_only_sidecar("branch_sidebar/local_heavy");
    let _ = measure_sidecar_allocations(|| remote_fanout.run());
    emit_allocation_only_sidecar("branch_sidebar/remote_fanout");
    let _ = measure_sidecar_allocations(|| aux_lists_heavy.run());
    emit_allocation_only_sidecar("branch_sidebar/aux_lists_heavy");
}

fn emit_branch_sidebar_sidecar(case_name: &str, metrics: &BranchSidebarMetrics) {
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

fn bench_branch_sidebar_extreme_scale(c: &mut Criterion) {
    let extreme_scale = BranchSidebarFixture::twenty_thousand_branches_hundred_remotes();
    let (_, metrics) = measure_sidecar_allocations(|| extreme_scale.run_with_metrics());

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("20k_branches_100_remotes"),
        |b| b.iter(|| extreme_scale.run()),
    );
    group.finish();

    emit_branch_sidebar_sidecar("20k_branches_100_remotes", &metrics);
}

fn emit_branch_sidebar_cache_sidecar(case_name: &str, metrics: &BranchSidebarCacheMetrics) {
    let mut payload = Map::new();
    payload.insert("cache_hits".to_string(), json!(metrics.cache_hits));
    payload.insert("cache_misses".to_string(), json!(metrics.cache_misses));
    payload.insert("rows_count".to_string(), json!(metrics.rows_count));
    payload.insert("invalidations".to_string(), json!(metrics.invalidations));
    emit_sidecar_metrics(&format!("branch_sidebar/{case_name}"), payload);
}

fn bench_branch_sidebar_cache(c: &mut Criterion) {
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);
    let worktrees = env_usize("GITCOMET_BENCH_WORKTREES", 80);
    let submodules = env_usize("GITCOMET_BENCH_SUBMODULES", 150);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 300);

    let mut cache_hit_balanced = BranchSidebarCacheFixture::balanced(
        local_branches,
        remote_branches,
        remotes,
        worktrees,
        submodules,
        stashes,
    );
    // Warm the cache with an initial build.
    cache_hit_balanced.run_cached();
    cache_hit_balanced.reset_metrics();

    let mut cache_miss_remote_fanout = BranchSidebarCacheFixture::remote_fanout(
        local_branches.max(32) / 4,
        remote_branches.saturating_mul(6),
        remotes.max(12),
    );

    let mut cache_invalidation =
        BranchSidebarCacheFixture::balanced(local_branches, remote_branches, remotes, 0, 0, 0);
    // Warm the cache so each iteration measures invalidation + rebuild.
    cache_invalidation.run_cached();
    cache_invalidation.reset_metrics();

    // Worktrees-ready invalidation: includes worktrees + submodules so the
    // rebuild reflects the full sidebar shape after async worktree loads land.
    let mut cache_invalidation_wt = BranchSidebarCacheFixture::balanced(
        local_branches,
        remote_branches,
        remotes,
        worktrees,
        submodules,
        stashes,
    );
    cache_invalidation_wt.run_cached();
    cache_invalidation_wt.reset_metrics();

    let mut group = c.benchmark_group("branch_sidebar");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("cache_hit_balanced"), |b| {
        b.iter_custom(|iters| {
            cache_hit_balanced.reset_metrics();
            let start = Instant::now();
            for _ in 0..iters {
                cache_hit_balanced.run_cached();
            }
            let elapsed = start.elapsed();
            cache_hit_balanced.reset_metrics();
            measure_sidecar_allocations(|| {
                cache_hit_balanced.run_cached();
            });
            emit_branch_sidebar_cache_sidecar("cache_hit_balanced", &cache_hit_balanced.metrics());
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("cache_miss_remote_fanout"),
        |b| {
            b.iter_custom(|iters| {
                cache_miss_remote_fanout.reset_metrics();
                let start = Instant::now();
                for _ in 0..iters {
                    // Invalidate before each iteration so every call is a miss.
                    cache_miss_remote_fanout.run_invalidate_single_ref();
                }
                let elapsed = start.elapsed();
                cache_miss_remote_fanout.reset_metrics();
                measure_sidecar_allocations(|| {
                    cache_miss_remote_fanout.run_invalidate_single_ref();
                });
                emit_branch_sidebar_cache_sidecar(
                    "cache_miss_remote_fanout",
                    &cache_miss_remote_fanout.metrics(),
                );
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("cache_invalidation_single_ref_change"),
        |b| {
            b.iter_custom(|iters| {
                cache_invalidation.reset_metrics();
                let start = Instant::now();
                for _ in 0..iters {
                    cache_invalidation.run_invalidate_single_ref();
                }
                let elapsed = start.elapsed();
                cache_invalidation.reset_metrics();
                measure_sidecar_allocations(|| {
                    cache_invalidation.run_invalidate_single_ref();
                });
                emit_branch_sidebar_cache_sidecar(
                    "cache_invalidation_single_ref_change",
                    &cache_invalidation.metrics(),
                );
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("cache_invalidation_worktrees_ready"),
        |b| {
            b.iter_custom(|iters| {
                cache_invalidation_wt.reset_metrics();
                let start = Instant::now();
                for _ in 0..iters {
                    cache_invalidation_wt.run_invalidate_worktrees_ready();
                }
                let elapsed = start.elapsed();
                cache_invalidation_wt.reset_metrics();
                measure_sidecar_allocations(|| {
                    cache_invalidation_wt.run_invalidate_worktrees_ready();
                });
                emit_branch_sidebar_cache_sidecar(
                    "cache_invalidation_worktrees_ready",
                    &cache_invalidation_wt.metrics(),
                );
                elapsed
            });
        },
    );

    group.finish();
}

fn bench_history_graph(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let merge_stride = env_usize("GITCOMET_BENCH_HISTORY_MERGE_EVERY", 50);
    let branch_head_every = env_usize("GITCOMET_BENCH_HISTORY_BRANCH_HEAD_EVERY", 11);

    let linear_history = HistoryGraphFixture::new(commits, 0, 0);
    let merge_dense = HistoryGraphFixture::new(commits, merge_stride.clamp(5, 25), 0);
    let branch_heads_dense =
        HistoryGraphFixture::new(commits, merge_stride.max(1), branch_head_every.max(2));

    // Collect sidecar metrics before the timed benchmark loop.
    let (_, linear_metrics) = measure_sidecar_allocations(|| linear_history.run_with_metrics());
    let (_, merge_metrics) = measure_sidecar_allocations(|| merge_dense.run_with_metrics());
    let (_, branch_metrics) = measure_sidecar_allocations(|| branch_heads_dense.run_with_metrics());

    let mut group = c.benchmark_group("history_graph");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("linear_history"), |b| {
        b.iter(|| linear_history.run())
    });
    group.bench_function(BenchmarkId::from_parameter("merge_dense"), |b| {
        b.iter(|| merge_dense.run())
    });
    group.bench_function(BenchmarkId::from_parameter("branch_heads_dense"), |b| {
        b.iter(|| branch_heads_dense.run())
    });
    group.finish();

    emit_history_graph_sidecar("linear_history", &linear_metrics);
    emit_history_graph_sidecar("merge_dense", &merge_metrics);
    emit_history_graph_sidecar("branch_heads_dense", &branch_metrics);
}

fn emit_history_cache_build_sidecar(case_name: &str, metrics: &HistoryCacheBuildMetrics) {
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

fn emit_history_load_more_append_sidecar(case_name: &str, metrics: &HistoryLoadMoreAppendMetrics) {
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

fn emit_history_scope_switch_sidecar(case_name: &str, metrics: &HistoryScopeSwitchMetrics) {
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

fn emit_repo_switch_sidecar(case_name: &str, metrics: &RepoSwitchMetrics) {
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

fn emit_status_list_sidecar(case_name: &str, metrics: &StatusListMetrics) {
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

fn emit_status_multi_select_sidecar(case_name: &str, metrics: &StatusMultiSelectMetrics) {
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

fn emit_status_select_diff_open_sidecar(case_name: &str, metrics: &StatusSelectDiffOpenMetrics) {
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

fn emit_history_graph_sidecar(case_name: &str, metrics: &HistoryGraphMetrics) {
    let mut payload = Map::new();
    payload.insert("commit_count".to_string(), json!(metrics.commit_count));
    payload.insert("graph_rows".to_string(), json!(metrics.graph_rows));
    payload.insert("max_lanes".to_string(), json!(metrics.max_lanes));
    payload.insert("merge_count".to_string(), json!(metrics.merge_count));
    payload.insert("branch_heads".to_string(), json!(metrics.branch_heads));
    emit_sidecar_metrics(&format!("history_graph/{case_name}"), payload);
}

fn emit_commit_details_sidecar(case_name: &str, metrics: &CommitDetailsMetrics) {
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

fn emit_commit_select_replace_sidecar(case_name: &str, metrics: &CommitSelectReplaceMetrics) {
    let mut payload = Map::new();
    payload.insert("files_a".to_string(), json!(metrics.files_a));
    payload.insert("files_b".to_string(), json!(metrics.files_b));
    payload.insert(
        "commit_ids_differ".to_string(),
        json!(metrics.commit_ids_differ),
    );
    emit_sidecar_metrics(&format!("commit_details/{case_name}"), payload);
}

fn emit_path_display_cache_churn_sidecar(case_name: &str, metrics: &PathDisplayCacheChurnMetrics) {
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

fn emit_merge_open_bootstrap_sidecar(case_name: &str, metrics: &MergeOpenBootstrapMetrics) {
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
struct FrameTimingScenarioMetrics {
    total_rows: u64,
    window_rows: u64,
    scroll_step_rows: u64,
}

fn capture_frame_timing_scroll_burst<F>(
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

    let mut capture = FrameTimingCapture::new(frame_budget_ns.max(1));
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

fn emit_frame_timing_sidecar(
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

fn emit_sidebar_resize_drag_sustained_sidecar(
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

fn emit_rapid_commit_selection_sidecar(
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

fn emit_repo_switch_during_scroll_sidecar(
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

fn emit_keyboard_arrow_scroll_sidecar(
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

fn emit_keyboard_tab_focus_sidecar(
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

fn emit_keyboard_stage_unstage_toggle_sidecar(
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

fn emit_git_ops_sidecar(case_name: &str, metrics: &GitOpsMetrics) {
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

fn bench_history_cache_build(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let tags = env_usize("GITCOMET_BENCH_TAGS", 50);
    let stashes = env_usize("GITCOMET_BENCH_STASHES", 20);

    let balanced =
        HistoryCacheBuildFixture::balanced(commits, local_branches, remote_branches, tags, stashes);
    let merge_dense = HistoryCacheBuildFixture::merge_dense(commits);
    let decorated_refs_heavy = HistoryCacheBuildFixture::decorated_refs_heavy(
        commits,
        local_branches.saturating_mul(10),
        remote_branches.saturating_mul(5),
        tags.saturating_mul(40),
    );
    let stash_heavy = HistoryCacheBuildFixture::stash_heavy(commits, stashes.saturating_mul(10));

    let mut group = c.benchmark_group("history_cache_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("balanced"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = balanced.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| balanced.run());
            emit_history_cache_build_sidecar("balanced", &metrics);
            start.elapsed()
        });
    });

    group.bench_function(BenchmarkId::from_parameter("merge_dense"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = merge_dense.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| merge_dense.run());
            emit_history_cache_build_sidecar("merge_dense", &metrics);
            start.elapsed()
        });
    });

    group.bench_function(BenchmarkId::from_parameter("decorated_refs_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = decorated_refs_heavy.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| decorated_refs_heavy.run());
            emit_history_cache_build_sidecar("decorated_refs_heavy", &metrics);
            start.elapsed()
        });
    });

    group.bench_function(BenchmarkId::from_parameter("stash_heavy"), |b| {
        b.iter_custom(|iters| {
            let start = Instant::now();
            for _ in 0..iters {
                let _ = stash_heavy.run();
            }
            let (_, metrics) = measure_sidecar_allocations(|| stash_heavy.run());
            emit_history_cache_build_sidecar("stash_heavy", &metrics);
            start.elapsed()
        });
    });

    group.finish();
}

fn bench_history_cache_build_extreme_scale(c: &mut Criterion) {
    let extreme_scale = HistoryCacheBuildFixture::extreme_scale_50k_2k_refs_200_stashes();

    let mut group = c.benchmark_group("history_cache_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("50k_commits_2k_refs_200_stashes"),
        |b| {
            b.iter_custom(|iters| {
                let start = Instant::now();
                for _ in 0..iters {
                    let _ = extreme_scale.run();
                }
                let (_, metrics) = measure_sidecar_allocations(|| extreme_scale.run());
                emit_history_cache_build_sidecar("50k_commits_2k_refs_200_stashes", &metrics);
                start.elapsed()
            });
        },
    );
    group.finish();
}

fn bench_history_load_more_append(c: &mut Criterion) {
    let fixture = HistoryLoadMoreAppendFixture::new(5_000, 500);

    let mut group = c.benchmark_group("history_load_more_append");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("page_500"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = fixture.fresh_state();
                let cursor = fixture.request_cursor();
                let page = fixture.append_page();
                let started_at = Instant::now();
                let _ = fixture.run_with_state_and_page(&mut state, cursor, page);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = fixture.fresh_state();
            let cursor = fixture.request_cursor();
            let page = fixture.append_page();
            let (_, metrics) = measure_sidecar_allocations(|| {
                fixture.run_with_state_and_page(&mut sidecar_state, cursor, page)
            });
            emit_history_load_more_append_sidecar("page_500", &metrics);
            elapsed
        });
    });
    group.finish();
}

fn bench_history_scope_switch(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let fixture = HistoryScopeSwitchFixture::current_branch_to_all_refs(commits);

    let mut group = c.benchmark_group("history_scope_switch");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("current_branch_to_all_refs"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = fixture.fresh_state();
                    let started_at = Instant::now();
                    let _ = fixture.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = fixture.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| fixture.run_with_state(&mut sidecar_state));
                emit_history_scope_switch_sidecar("current_branch_to_all_refs", &metrics);
                elapsed
            });
        },
    );
    group.finish();
}

fn bench_repo_switch(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_COMMITS", 5_000);
    let local_branches = env_usize("GITCOMET_BENCH_LOCAL_BRANCHES", 200);
    let remote_branches = env_usize("GITCOMET_BENCH_REMOTE_BRANCHES", 800);
    let remotes = env_usize("GITCOMET_BENCH_REMOTES", 2);

    let refocus_same_repo =
        RepoSwitchFixture::refocus_same_repo(commits, local_branches, remote_branches, remotes);
    let two_hot_repos =
        RepoSwitchFixture::two_hot_repos(commits, local_branches, remote_branches, remotes);
    let selected_commit_and_details = RepoSwitchFixture::selected_commit_and_details(
        commits,
        local_branches,
        remote_branches,
        remotes,
    );
    let twenty_tabs =
        RepoSwitchFixture::twenty_tabs(commits, local_branches, remote_branches, remotes);
    let twenty_repos_all_hot =
        RepoSwitchFixture::twenty_repos_all_hot(commits, local_branches, remote_branches, remotes);
    let selected_diff_file =
        RepoSwitchFixture::selected_diff_file(commits, local_branches, remote_branches, remotes);
    let selected_conflict_target = RepoSwitchFixture::selected_conflict_target(
        commits,
        local_branches,
        remote_branches,
        remotes,
    );
    let merge_active_with_draft_restore = RepoSwitchFixture::merge_active_with_draft_restore(
        commits,
        local_branches,
        remote_branches,
        remotes,
    );

    let mut group = c.benchmark_group("repo_switch");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("refocus_same_repo"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = refocus_same_repo.fresh_state();
                let started_at = Instant::now();
                let _ = refocus_same_repo.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = refocus_same_repo.fresh_state();
            let (_, metrics) = measure_sidecar_allocations(|| {
                refocus_same_repo.run_with_state(&mut sidecar_state)
            });
            emit_repo_switch_sidecar("refocus_same_repo", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("two_hot_repos"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = two_hot_repos.fresh_state();
                let started_at = Instant::now();
                let _ = two_hot_repos.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = two_hot_repos.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| two_hot_repos.run_with_state(&mut sidecar_state));
            emit_repo_switch_sidecar("two_hot_repos", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("selected_commit_and_details"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = selected_commit_and_details.fresh_state();
                    let started_at = Instant::now();
                    let _ = selected_commit_and_details.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = selected_commit_and_details.fresh_state();
                let (_, metrics) = measure_sidecar_allocations(|| {
                    selected_commit_and_details.run_with_state(&mut sidecar_state)
                });
                emit_repo_switch_sidecar("selected_commit_and_details", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(BenchmarkId::from_parameter("twenty_tabs"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = twenty_tabs.fresh_state();
                let started_at = Instant::now();
                let _ = twenty_tabs.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = twenty_tabs.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| twenty_tabs.run_with_state(&mut sidecar_state));
            emit_repo_switch_sidecar("twenty_tabs", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("20_repos_all_hot"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = twenty_repos_all_hot.fresh_state();
                let started_at = Instant::now();
                let _ = twenty_repos_all_hot.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = twenty_repos_all_hot.fresh_state();
            let (_, metrics) = measure_sidecar_allocations(|| {
                twenty_repos_all_hot.run_with_state(&mut sidecar_state)
            });
            emit_repo_switch_sidecar("20_repos_all_hot", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("selected_diff_file"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = selected_diff_file.fresh_state();
                let started_at = Instant::now();
                let _ = selected_diff_file.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = selected_diff_file.fresh_state();
            let (_, metrics) = measure_sidecar_allocations(|| {
                selected_diff_file.run_with_state(&mut sidecar_state)
            });
            emit_repo_switch_sidecar("selected_diff_file", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("selected_conflict_target"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = selected_conflict_target.fresh_state();
                    let started_at = Instant::now();
                    let _ = selected_conflict_target.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = selected_conflict_target.fresh_state();
                let (_, metrics) = measure_sidecar_allocations(|| {
                    selected_conflict_target.run_with_state(&mut sidecar_state)
                });
                emit_repo_switch_sidecar("selected_conflict_target", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("merge_active_with_draft_restore"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = merge_active_with_draft_restore.fresh_state();
                    let started_at = Instant::now();
                    let _ = merge_active_with_draft_restore.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = merge_active_with_draft_restore.fresh_state();
                let (_, metrics) = measure_sidecar_allocations(|| {
                    merge_active_with_draft_restore.run_with_state(&mut sidecar_state)
                });
                emit_repo_switch_sidecar("merge_active_with_draft_restore", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}

fn bench_commit_details(c: &mut Criterion) {
    let files = env_usize("GITCOMET_BENCH_COMMIT_FILES", 5_000);
    let depth = env_usize("GITCOMET_BENCH_COMMIT_PATH_DEPTH", 4);
    let deep_depth = env_usize(
        "GITCOMET_BENCH_COMMIT_DEEP_PATH_DEPTH",
        depth.saturating_mul(4).max(12),
    );
    let huge_files = env_usize("GITCOMET_BENCH_COMMIT_HUGE_FILES", files.saturating_mul(2));
    let large_message_files = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_FILES", files.max(1));
    let large_message_depth = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_DEPTH", depth.max(1));
    let large_message_bytes = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_BYTES", 96 * 1024);
    let large_message_line_bytes = env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_LINE_BYTES", 192);
    let large_message_visible_lines =
        env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_VISIBLE_LINES", 48);
    let large_message_wrap_width_px =
        env_usize("GITCOMET_BENCH_COMMIT_LARGE_MESSAGE_WRAP_WIDTH_PX", 560);
    let extreme_files = env_usize("GITCOMET_BENCH_COMMIT_EXTREME_FILES", 10_000);
    let extreme_depth = env_usize("GITCOMET_BENCH_COMMIT_EXTREME_DEPTH", 12);
    let balanced = CommitDetailsFixture::new(files, depth);
    let deep_paths = CommitDetailsFixture::new(files, deep_depth);
    let huge_list = CommitDetailsFixture::new(huge_files, depth);
    let large_message = CommitDetailsFixture::large_message_body(
        large_message_files,
        large_message_depth,
        large_message_bytes,
        large_message_line_bytes,
        large_message_visible_lines,
        large_message_wrap_width_px,
    );
    let extreme_scale = CommitDetailsFixture::new(extreme_files, extreme_depth);
    let select_replace = CommitSelectReplaceFixture::new(files, depth);

    let churn_files = env_usize("GITCOMET_BENCH_PATH_CHURN_FILES", 10_000);
    let churn_depth = env_usize("GITCOMET_BENCH_PATH_CHURN_DEPTH", 6);
    let mut path_churn = PathDisplayCacheChurnFixture::new(churn_files, churn_depth);

    balanced.prewarm_runtime_state();
    deep_paths.prewarm_runtime_state();
    huge_list.prewarm_runtime_state();
    large_message.prewarm_runtime_state();
    extreme_scale.prewarm_runtime_state();

    // Collect sidecar metrics before the timed benchmark loop.
    let (_, balanced_metrics) = measure_sidecar_allocations(|| balanced.run_with_metrics());
    let (_, deep_metrics) = measure_sidecar_allocations(|| deep_paths.run_with_metrics());
    let (_, huge_metrics) = measure_sidecar_allocations(|| huge_list.run_with_metrics());
    let (_, large_message_metrics) =
        measure_sidecar_allocations(|| large_message.run_with_metrics());
    let (_, extreme_scale_metrics) =
        measure_sidecar_allocations(|| extreme_scale.run_with_metrics());
    let (_, select_replace_metrics) =
        measure_sidecar_allocations(|| select_replace.run_with_metrics());
    let (_, churn_metrics) = measure_sidecar_allocations(|| path_churn.run_with_metrics());

    let mut group = c.benchmark_group("commit_details");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("many_files"), |b| {
        b.iter(|| balanced.run())
    });
    group.bench_function(BenchmarkId::from_parameter("deep_paths"), |b| {
        b.iter(|| deep_paths.run())
    });
    group.bench_function(BenchmarkId::from_parameter("huge_file_list"), |b| {
        b.iter(|| huge_list.run())
    });
    group.bench_function(BenchmarkId::from_parameter("large_message_body"), |b| {
        b.iter(|| large_message.run())
    });
    group.bench_function(BenchmarkId::from_parameter("10k_files_depth_12"), |b| {
        b.iter(|| extreme_scale.run())
    });
    group.bench_function(BenchmarkId::from_parameter("select_commit_replace"), |b| {
        b.iter(|| select_replace.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("path_display_cache_churn"),
        |b| {
            b.iter(|| {
                path_churn.reset_runtime_state();
                path_churn.run()
            })
        },
    );
    group.finish();

    emit_commit_details_sidecar("many_files", &balanced_metrics);
    emit_commit_details_sidecar("deep_paths", &deep_metrics);
    emit_commit_details_sidecar("huge_file_list", &huge_metrics);
    emit_commit_details_sidecar("large_message_body", &large_message_metrics);
    emit_commit_details_sidecar("10k_files_depth_12", &extreme_scale_metrics);
    emit_commit_select_replace_sidecar("select_commit_replace", &select_replace_metrics);
    emit_path_display_cache_churn_sidecar("path_display_cache_churn", &churn_metrics);
}

fn bench_status_list(c: &mut Criterion) {
    let entries = env_usize("GITCOMET_BENCH_STATUS_ENTRIES", 10_000);
    let window = env_usize("GITCOMET_BENCH_STATUS_WINDOW", 200);
    let mixed_depth_entries = env_usize("GITCOMET_BENCH_STATUS_MIXED_DEPTH_ENTRIES", 20_000);
    let mixed_depth_prewarm = env_usize("GITCOMET_BENCH_STATUS_MIXED_DEPTH_PREWARM", 8_193);
    let mut unstaged_large = StatusListFixture::unstaged_large(entries);
    let mut staged_large = StatusListFixture::staged_large(entries);
    let mut mixed_depth = StatusListFixture::mixed_depth(mixed_depth_entries);
    let unstaged_metrics =
        measure_sidecar_allocations(|| unstaged_large.measure_window_step(0, window));
    let staged_metrics =
        measure_sidecar_allocations(|| staged_large.measure_window_step(0, window));
    let mixed_depth_metrics = measure_sidecar_allocations(|| {
        mixed_depth.measure_window_step_with_prewarm(
            mixed_depth_prewarm,
            window,
            mixed_depth_prewarm,
        )
    });

    let mut group = c.benchmark_group("status_list");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("unstaged_large"), |b| {
        b.iter(|| {
            unstaged_large.reset_runtime_state();
            unstaged_large.run_window_step(0, window)
        })
    });
    group.bench_function(BenchmarkId::from_parameter("staged_large"), |b| {
        b.iter(|| {
            staged_large.reset_runtime_state();
            staged_large.run_window_step(0, window)
        })
    });
    group.bench_function(
        BenchmarkId::from_parameter("20k_entries_mixed_depth"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    mixed_depth.reset_runtime_state();
                    mixed_depth.prewarm_cache(mixed_depth_prewarm);
                    let started_at = Instant::now();
                    let _ = mixed_depth.run_window_step(mixed_depth_prewarm, window);
                    elapsed += started_at.elapsed();
                }
                elapsed
            })
        },
    );
    group.finish();

    emit_status_list_sidecar("unstaged_large", &unstaged_metrics);
    emit_status_list_sidecar("staged_large", &staged_metrics);
    emit_status_list_sidecar("20k_entries_mixed_depth", &mixed_depth_metrics);
}

fn bench_status_multi_select(c: &mut Criterion) {
    let entries = env_usize("GITCOMET_BENCH_STATUS_MULTI_SELECT_ENTRIES", 20_000);
    let anchor_index = env_usize("GITCOMET_BENCH_STATUS_MULTI_SELECT_ANCHOR", 4_096);
    let selected_paths = env_usize("GITCOMET_BENCH_STATUS_MULTI_SELECT_RANGE", 512);
    let fixture = StatusMultiSelectFixture::range_select(entries, anchor_index, selected_paths);
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());

    let mut group = c.benchmark_group("status_multi_select");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("range_select"), |b| {
        b.iter(|| fixture.run())
    });
    group.finish();

    emit_status_multi_select_sidecar("range_select", &metrics);
}

fn bench_status_select_diff_open(c: &mut Criterion) {
    let status_entries = env_usize("GITCOMET_BENCH_STATUS_SELECT_DIFF_ENTRIES", 10_000);

    let unstaged = StatusSelectDiffOpenFixture::unstaged(status_entries);
    let staged = StatusSelectDiffOpenFixture::staged(status_entries);

    let mut group = c.benchmark_group("status_select_diff_open");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("unstaged"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = unstaged.fresh_state();
                let started_at = Instant::now();
                let _ = unstaged.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = unstaged.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| unstaged.run_with_state(&mut sidecar_state));
            emit_status_select_diff_open_sidecar("unstaged", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("staged"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = staged.fresh_state();
                let started_at = Instant::now();
                let _ = staged.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = staged.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| staged.run_with_state(&mut sidecar_state));
            emit_status_select_diff_open_sidecar("staged", &metrics);
            elapsed
        });
    });

    group.finish();
}

fn bench_merge_open_bootstrap(c: &mut Criterion) {
    let small_lines = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_SMALL_LINES", 5_000);
    let small = MergeOpenBootstrapFixture::small(small_lines);
    let (_, small_metrics) = measure_sidecar_allocations(|| small.run_with_metrics());

    let lines = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_LINES", 55_001);
    let conflict_blocks = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_CONFLICTS", 1);
    let large_streamed = MergeOpenBootstrapFixture::large_streamed(lines, conflict_blocks);
    let (_, large_streamed_metrics) =
        measure_sidecar_allocations(|| large_streamed.run_with_metrics());

    let many_conflicts_blocks = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_MANY_CONFLICTS", 50);
    let many_conflicts = MergeOpenBootstrapFixture::many_conflicts(many_conflicts_blocks);
    let (_, many_conflicts_metrics) =
        measure_sidecar_allocations(|| many_conflicts.run_with_metrics());

    let extreme_lines = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_EXTREME_LINES", 50_000);
    let extreme_conflicts = env_usize("GITCOMET_BENCH_MERGE_BOOTSTRAP_EXTREME_CONFLICTS", 500);
    let large_many =
        MergeOpenBootstrapFixture::large_many_conflicts(extreme_lines, extreme_conflicts);
    let (_, large_many_metrics) = measure_sidecar_allocations(|| large_many.run_with_metrics());

    let mut group = c.benchmark_group("merge_open_bootstrap");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("small"), |b| {
        b.iter(|| small.run())
    });
    group.bench_function(BenchmarkId::from_parameter("large_streamed"), |b| {
        b.iter(|| large_streamed.run())
    });
    group.bench_function(BenchmarkId::from_parameter("many_conflicts"), |b| {
        b.iter(|| many_conflicts.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("50k_lines_500_conflicts_streamed"),
        |b| b.iter(|| large_many.run()),
    );
    group.finish();

    emit_merge_open_bootstrap_sidecar("small", &small_metrics);
    emit_merge_open_bootstrap_sidecar("large_streamed", &large_streamed_metrics);
    emit_merge_open_bootstrap_sidecar("many_conflicts", &many_conflicts_metrics);
    emit_merge_open_bootstrap_sidecar("50k_lines_500_conflicts_streamed", &large_many_metrics);
}

fn bench_frame_timing(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_FRAME_HISTORY_COMMITS", 50_000);
    let history_local_branches = env_usize("GITCOMET_BENCH_FRAME_HISTORY_LOCAL_BRANCHES", 400);
    let history_remote_branches = env_usize("GITCOMET_BENCH_FRAME_HISTORY_REMOTE_BRANCHES", 1_200);
    let history_window = env_usize("GITCOMET_BENCH_FRAME_HISTORY_WINDOW", 120);
    let history_scroll_step = env_usize("GITCOMET_BENCH_FRAME_HISTORY_SCROLL_STEP", 24);
    let diff_lines = env_usize("GITCOMET_BENCH_FRAME_DIFF_LINES", 100_000);
    let diff_window = env_usize("GITCOMET_BENCH_FRAME_DIFF_WINDOW", 200);
    let diff_line_bytes = env_usize("GITCOMET_BENCH_FRAME_DIFF_LINE_BYTES", 96);
    let diff_scroll_step = env_usize("GITCOMET_BENCH_FRAME_DIFF_SCROLL_STEP", 40);
    let frames = env_usize("GITCOMET_BENCH_FRAME_TIMING_FRAMES", 240);
    let frame_budget_ns =
        u64::try_from(env_usize("GITCOMET_BENCH_FRAME_BUDGET_NS", 16_666_667)).unwrap_or(u64::MAX);

    let history_fixture = HistoryListScrollFixture::new(
        history_commits,
        history_local_branches,
        history_remote_branches,
    );
    let diff_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(diff_lines, diff_line_bytes);

    let mut group = c.benchmark_group("frame_timing");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("continuous_scroll_history_list"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, stats, metrics) = capture_frame_timing_scroll_burst(
                        history_commits,
                        history_window,
                        history_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| history_fixture.run_scroll_step(start, window),
                    );
                    hash ^= burst_hash;
                    std::hint::black_box((stats, metrics));
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) = measure_sidecar_allocations(|| {
                    capture_frame_timing_scroll_burst(
                        history_commits,
                        history_window,
                        history_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| history_fixture.run_scroll_step(start, window),
                    )
                });
                emit_frame_timing_sidecar("continuous_scroll_history_list", &stats, metrics);
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("continuous_scroll_large_diff"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, stats, metrics) = capture_frame_timing_scroll_burst(
                        diff_lines,
                        diff_window,
                        diff_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| diff_fixture.run_scroll_step(start, window),
                    );
                    hash ^= burst_hash;
                    std::hint::black_box((stats, metrics));
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) = measure_sidecar_allocations(|| {
                    capture_frame_timing_scroll_burst(
                        diff_lines,
                        diff_window,
                        diff_scroll_step,
                        frame_budget_ns,
                        frames,
                        |start, window| diff_fixture.run_scroll_step(start, window),
                    )
                });
                emit_frame_timing_sidecar("continuous_scroll_large_diff", &stats, metrics);
                started.elapsed()
            });
        },
    );

    // --- sidebar_resize_drag_sustained ---
    let sidebar_drag_frames = env_usize("GITCOMET_BENCH_FRAME_SIDEBAR_DRAG_FRAMES", 240);
    group.bench_function(
        BenchmarkId::from_parameter("sidebar_resize_drag_sustained"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let mut fixture = SidebarResizeDragSustainedFixture::new(
                        sidebar_drag_frames,
                        frame_budget_ns,
                    );
                    let (burst_hash, _stats, _metrics) = fixture.run_with_metrics();
                    hash ^= burst_hash;
                }

                std::hint::black_box(hash);
                let mut fixture =
                    SidebarResizeDragSustainedFixture::new(sidebar_drag_frames, frame_budget_ns);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| fixture.run_with_metrics());
                emit_sidebar_resize_drag_sustained_sidecar(&stats, metrics);
                started.elapsed()
            });
        },
    );

    // --- rapid_commit_selection_changes ---
    let rapid_commit_count = env_usize("GITCOMET_BENCH_FRAME_RAPID_COMMIT_COUNT", 120);
    let rapid_commit_files = env_usize("GITCOMET_BENCH_FRAME_RAPID_COMMIT_FILES", 200);
    let rapid_commit_fixture =
        RapidCommitSelectionFixture::new(rapid_commit_count, rapid_commit_files, frame_budget_ns);
    group.bench_function(
        BenchmarkId::from_parameter("rapid_commit_selection_changes"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, _stats, _metrics) = rapid_commit_fixture.run_with_metrics();
                    hash ^= burst_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| rapid_commit_fixture.run_with_metrics());
                emit_rapid_commit_selection_sidecar(&stats, metrics);
                started.elapsed()
            });
        },
    );

    // --- repo_switch_during_scroll ---
    let switch_every = env_usize("GITCOMET_BENCH_FRAME_SWITCH_EVERY_N_FRAMES", 30);
    let repo_switch_scroll_fixture = RepoSwitchDuringScrollFixture::new(
        history_commits,
        history_local_branches,
        history_remote_branches,
        history_window,
        history_scroll_step,
        frames,
        switch_every,
        frame_budget_ns,
    );
    group.bench_function(
        BenchmarkId::from_parameter("repo_switch_during_scroll"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (burst_hash, _stats, _metrics) =
                        repo_switch_scroll_fixture.run_with_metrics();
                    hash ^= burst_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| repo_switch_scroll_fixture.run_with_metrics());
                emit_repo_switch_during_scroll_sidecar(&stats, metrics);
                started.elapsed()
            });
        },
    );

    group.finish();
}

fn bench_keyboard(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_COMMITS", 50_000);
    let history_local_branches = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_LOCAL_BRANCHES", 400);
    let history_remote_branches =
        env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_REMOTE_BRANCHES", 1_200);
    let history_window = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_WINDOW", 120);
    let history_scroll_step = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_SCROLL_STEP", 1);
    let history_repeat_events = env_usize("GITCOMET_BENCH_KEYBOARD_HISTORY_REPEAT_EVENTS", 240);
    let diff_lines = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_LINES", 100_000);
    let diff_window = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_WINDOW", 200);
    let diff_line_bytes = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_LINE_BYTES", 96);
    let diff_scroll_step = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_SCROLL_STEP", 1);
    let diff_repeat_events = env_usize("GITCOMET_BENCH_KEYBOARD_DIFF_REPEAT_EVENTS", 240);
    let tab_focus_repo_tabs = env_usize("GITCOMET_BENCH_KEYBOARD_TAB_FOCUS_REPO_TABS", 20);
    let tab_focus_cycle_events = env_usize("GITCOMET_BENCH_KEYBOARD_TAB_FOCUS_CYCLE_EVENTS", 240);
    let stage_toggle_paths = env_usize("GITCOMET_BENCH_KEYBOARD_STAGE_TOGGLE_PATHS", 128);
    let stage_toggle_events = env_usize("GITCOMET_BENCH_KEYBOARD_STAGE_TOGGLE_EVENTS", 240);
    let frame_budget_ns = u64::try_from(env_usize(
        "GITCOMET_BENCH_KEYBOARD_FRAME_BUDGET_NS",
        16_666_667,
    ))
    .unwrap_or(u64::MAX);

    let history_fixture = KeyboardArrowScrollFixture::history(
        history_commits,
        history_local_branches,
        history_remote_branches,
        history_window,
        history_scroll_step,
        history_repeat_events,
        frame_budget_ns,
    );
    let diff_fixture = KeyboardArrowScrollFixture::diff(
        diff_lines,
        diff_line_bytes,
        diff_window,
        diff_scroll_step,
        diff_repeat_events,
        frame_budget_ns,
    );
    let tab_focus_fixture = KeyboardTabFocusCycleFixture::all_panes(
        tab_focus_repo_tabs,
        tab_focus_cycle_events,
        frame_budget_ns,
    );
    let stage_toggle_fixture = KeyboardStageUnstageToggleFixture::rapid_toggle(
        stage_toggle_paths,
        stage_toggle_events,
        frame_budget_ns,
    );

    let mut group = c.benchmark_group("keyboard");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("arrow_scroll_history_sustained_repeat"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = history_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| history_fixture.run_with_metrics());
                emit_keyboard_arrow_scroll_sidecar(
                    "arrow_scroll_history_sustained_repeat",
                    &stats,
                    metrics,
                );
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("arrow_scroll_diff_sustained_repeat"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = diff_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| diff_fixture.run_with_metrics());
                emit_keyboard_arrow_scroll_sidecar(
                    "arrow_scroll_diff_sustained_repeat",
                    &stats,
                    metrics,
                );
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("tab_focus_cycle_all_panes"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = tab_focus_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| tab_focus_fixture.run_with_metrics());
                emit_keyboard_tab_focus_sidecar("tab_focus_cycle_all_panes", &stats, metrics);
                started.elapsed()
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("stage_unstage_toggle_rapid"),
        |b| {
            b.iter_custom(|iters| {
                let started = Instant::now();
                let mut hash = 0u64;

                for _ in 0..iters {
                    let (case_hash, _stats, _metrics) = stage_toggle_fixture.run_with_metrics();
                    hash ^= case_hash;
                }

                std::hint::black_box(hash);
                let (_hash, stats, metrics) =
                    measure_sidecar_allocations(|| stage_toggle_fixture.run_with_metrics());
                emit_keyboard_stage_unstage_toggle_sidecar(
                    "stage_unstage_toggle_rapid",
                    &stats,
                    metrics,
                );
                started.elapsed()
            });
        },
    );

    group.finish();
}

fn emit_staging_sidecar(case_name: &str, metrics: &StagingMetrics) {
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

fn bench_staging(c: &mut Criterion) {
    let stage_files = env_usize("GITCOMET_BENCH_STAGING_FILES", 10_000);
    let interleaved_files = env_usize("GITCOMET_BENCH_STAGING_INTERLEAVED_FILES", 1_000);

    let stage_all = StagingFixture::stage_all(stage_files);
    let unstage_all = StagingFixture::unstage_all(stage_files);
    let interleaved = StagingFixture::interleaved(interleaved_files);

    let mut group = c.benchmark_group("staging");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("stage_all_10k_files"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = stage_all.fresh_state();
                let started_at = Instant::now();
                let _ = stage_all.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = stage_all.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| stage_all.run_with_state(&mut sidecar_state));
            emit_staging_sidecar("stage_all_10k_files", &metrics);
            elapsed
        });
    });

    group.bench_function(BenchmarkId::from_parameter("unstage_all_10k_files"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let mut state = unstage_all.fresh_state();
                let started_at = Instant::now();
                let _ = unstage_all.run_with_state(&mut state);
                elapsed += started_at.elapsed();
            }
            let mut sidecar_state = unstage_all.fresh_state();
            let (_, metrics) =
                measure_sidecar_allocations(|| unstage_all.run_with_state(&mut sidecar_state));
            emit_staging_sidecar("unstage_all_10k_files", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("stage_unstage_interleaved_1k_files"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = interleaved.fresh_state();
                    let started_at = Instant::now();
                    let _ = interleaved.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = interleaved.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| interleaved.run_with_state(&mut sidecar_state));
                emit_staging_sidecar("stage_unstage_interleaved_1k_files", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}

fn emit_undo_redo_sidecar(case_name: &str, metrics: &UndoRedoMetrics) {
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

fn bench_undo_redo(c: &mut Criterion) {
    let deep_stack_regions = env_usize("GITCOMET_BENCH_UNDO_REDO_DEEP_REGIONS", 200);
    let replay_regions = env_usize("GITCOMET_BENCH_UNDO_REDO_REPLAY_REGIONS", 50);

    let deep_stack = UndoRedoFixture::deep_stack(deep_stack_regions);
    let undo_replay = UndoRedoFixture::undo_replay(replay_regions);

    let mut group = c.benchmark_group("undo_redo");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("conflict_resolution_deep_stack"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = deep_stack.fresh_state();
                    let started_at = Instant::now();
                    let _ = deep_stack.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = deep_stack.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| deep_stack.run_with_state(&mut sidecar_state));
                emit_undo_redo_sidecar("conflict_resolution_deep_stack", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("conflict_resolution_undo_replay_50_steps"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let mut state = undo_replay.fresh_state();
                    let started_at = Instant::now();
                    let _ = undo_replay.run_with_state(&mut state);
                    elapsed += started_at.elapsed();
                }
                let mut sidecar_state = undo_replay.fresh_state();
                let (_, metrics) =
                    measure_sidecar_allocations(|| undo_replay.run_with_state(&mut sidecar_state));
                emit_undo_redo_sidecar("conflict_resolution_undo_replay_50_steps", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}

fn bench_git_ops(c: &mut Criterion) {
    let status_files = env_usize("GITCOMET_BENCH_GIT_STATUS_FILES", 1_000);
    let status_dirty_files = env_usize("GITCOMET_BENCH_GIT_STATUS_DIRTY_FILES", 500);
    let log_commits = env_usize("GITCOMET_BENCH_GIT_LOG_COMMITS", 10_000);
    let log_shallow_total_commits =
        env_usize("GITCOMET_BENCH_GIT_LOG_SHALLOW_TOTAL_COMMITS", 100_000);
    let log_shallow_requested_commits =
        env_usize("GITCOMET_BENCH_GIT_LOG_SHALLOW_REQUESTED_COMMITS", 200);
    let status_clean_files = env_usize("GITCOMET_BENCH_GIT_STATUS_CLEAN_FILES", 10_000);
    let ref_count = env_usize("GITCOMET_BENCH_GIT_REF_COUNT", 10_000);
    let diff_rename_files = env_usize("GITCOMET_BENCH_GIT_DIFF_RENAME_FILES", 256);
    let diff_binary_files = env_usize("GITCOMET_BENCH_GIT_DIFF_BINARY_FILES", 128);
    let diff_binary_bytes = env_usize("GITCOMET_BENCH_GIT_DIFF_BINARY_BYTES", 4_096);
    let diff_large_file_lines = env_usize("GITCOMET_BENCH_GIT_DIFF_LARGE_FILE_LINES", 100_000);
    let diff_large_file_line_bytes = env_usize("GITCOMET_BENCH_GIT_DIFF_LARGE_FILE_LINE_BYTES", 48);
    let blame_large_file_lines = env_usize("GITCOMET_BENCH_GIT_BLAME_LINES", 100_000);
    let blame_large_file_commits = env_usize("GITCOMET_BENCH_GIT_BLAME_COMMITS", 16);
    let file_history_total_commits =
        env_usize("GITCOMET_BENCH_GIT_FILE_HISTORY_TOTAL_COMMITS", 100_000);
    let file_history_requested_commits =
        env_usize("GITCOMET_BENCH_GIT_FILE_HISTORY_REQUESTED_COMMITS", 200);
    let file_history_touch_every = env_usize("GITCOMET_BENCH_GIT_FILE_HISTORY_TOUCH_EVERY", 10);

    let status_dirty = GitOpsFixture::status_dirty(status_files, status_dirty_files);
    let log_walk = GitOpsFixture::log_walk(log_commits, log_commits);
    let log_walk_shallow =
        GitOpsFixture::log_walk(log_shallow_total_commits, log_shallow_requested_commits);
    let status_clean = GitOpsFixture::status_clean(status_clean_files);
    let ref_enumerate = GitOpsFixture::ref_enumerate(ref_count);
    let diff_rename = GitOpsFixture::diff_rename_heavy(diff_rename_files);
    let diff_binary = GitOpsFixture::diff_binary_heavy(diff_binary_files, diff_binary_bytes);
    let diff_large_single_file =
        GitOpsFixture::diff_large_single_file(diff_large_file_lines, diff_large_file_line_bytes);
    let blame_large_file =
        GitOpsFixture::blame_large_file(blame_large_file_lines, blame_large_file_commits);
    let file_history = GitOpsFixture::file_history(
        file_history_total_commits,
        file_history_requested_commits,
        file_history_touch_every,
    );

    let mut group = c.benchmark_group("git_ops");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("status_dirty_500_files"), |b| {
        b.iter(|| status_dirty.run())
    });
    group.bench_function(BenchmarkId::from_parameter("log_walk_10k_commits"), |b| {
        b.iter(|| log_walk.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("log_walk_100k_commits_shallow"),
        |b| b.iter(|| log_walk_shallow.run()),
    );
    group.bench_function(BenchmarkId::from_parameter("status_clean_10k_files"), |b| {
        b.iter(|| status_clean.run())
    });
    group.bench_function(BenchmarkId::from_parameter("ref_enumerate_10k_refs"), |b| {
        b.iter(|| ref_enumerate.run())
    });
    group.bench_function(BenchmarkId::from_parameter("diff_rename_heavy"), |b| {
        b.iter(|| diff_rename.run())
    });
    group.bench_function(BenchmarkId::from_parameter("diff_binary_heavy"), |b| {
        b.iter(|| diff_binary.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("diff_large_single_file_100k_lines"),
        |b| b.iter(|| diff_large_single_file.run()),
    );
    group.bench_function(BenchmarkId::from_parameter("blame_large_file"), |b| {
        b.iter(|| blame_large_file.run())
    });
    group.bench_function(
        BenchmarkId::from_parameter("file_history_first_page_sparse_100k_commits"),
        |b| b.iter(|| file_history.run()),
    );
    group.finish();

    let (_, status_metrics) = measure_sidecar_allocations(|| status_dirty.run_with_metrics());
    let (_, log_metrics) = measure_sidecar_allocations(|| log_walk.run_with_metrics());
    let (_, log_shallow_metrics) =
        measure_sidecar_allocations(|| log_walk_shallow.run_with_metrics());
    let (_, status_clean_metrics) = measure_sidecar_allocations(|| status_clean.run_with_metrics());
    let (_, ref_enumerate_metrics) =
        measure_sidecar_allocations(|| ref_enumerate.run_with_metrics());
    let (_, diff_rename_metrics) = measure_sidecar_allocations(|| diff_rename.run_with_metrics());
    let (_, diff_binary_metrics) = measure_sidecar_allocations(|| diff_binary.run_with_metrics());
    let (_, diff_large_metrics) =
        measure_sidecar_allocations(|| diff_large_single_file.run_with_metrics());
    let (_, blame_large_metrics) =
        measure_sidecar_allocations(|| blame_large_file.run_with_metrics());
    let (_, file_history_metrics) = measure_sidecar_allocations(|| file_history.run_with_metrics());
    emit_git_ops_sidecar("status_dirty_500_files", &status_metrics);
    emit_git_ops_sidecar("log_walk_10k_commits", &log_metrics);
    emit_git_ops_sidecar("log_walk_100k_commits_shallow", &log_shallow_metrics);
    emit_git_ops_sidecar("status_clean_10k_files", &status_clean_metrics);
    emit_git_ops_sidecar("ref_enumerate_10k_refs", &ref_enumerate_metrics);
    emit_git_ops_sidecar("diff_rename_heavy", &diff_rename_metrics);
    emit_git_ops_sidecar("diff_binary_heavy", &diff_binary_metrics);
    emit_git_ops_sidecar("diff_large_single_file_100k_lines", &diff_large_metrics);
    emit_git_ops_sidecar("blame_large_file", &blame_large_metrics);
    emit_git_ops_sidecar(
        "file_history_first_page_sparse_100k_commits",
        &file_history_metrics,
    );
}

fn bench_large_file_diff_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_DIFF_LINES", 10_000);
    let window = env_usize("GITCOMET_BENCH_DIFF_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_DIFF_LINE_BYTES", 96);
    let long_line_bytes = env_usize("GITCOMET_BENCH_DIFF_LONG_LINE_BYTES", 4_096);
    let normal_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(lines, line_bytes);
    let long_line_fixture = LargeFileDiffScrollFixture::new_with_line_bytes(lines, long_line_bytes);

    let mut group = c.benchmark_group("diff_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("normal_lines_window", window),
        &window,
        |b, &window| {
            // Use a varying start index per-iteration to reduce cache effects in allocators.
            let mut start = 0usize;
            b.iter(|| {
                let h = normal_fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.bench_with_input(
        BenchmarkId::new("long_lines_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = long_line_fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    let (_, normal_metrics) =
        measure_sidecar_allocations(|| normal_fixture.run_scroll_step_with_metrics(0, window));
    emit_diff_scroll_sidecar(
        &format!("diff_scroll/normal_lines_window/{window}"),
        &normal_metrics,
    );

    let (_, long_metrics) =
        measure_sidecar_allocations(|| long_line_fixture.run_scroll_step_with_metrics(0, window));
    emit_diff_scroll_sidecar(
        &format!("diff_scroll/long_lines_window/{window}"),
        &long_metrics,
    );
}

fn emit_diff_scroll_sidecar(bench: &str, metrics: &LargeFileDiffScrollMetrics) {
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

fn bench_file_diff_replacement_alignment(c: &mut Criterion) {
    let blocks = env_usize("GITCOMET_BENCH_REPLACEMENT_BLOCKS", 12);
    let balanced_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_BALANCED_LINES", 48);
    let skewed_old_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_SKEW_OLD_LINES", 40);
    let skewed_new_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_SKEW_NEW_LINES", 56);
    let context_lines = env_usize("GITCOMET_BENCH_REPLACEMENT_CONTEXT_LINES", 3);
    let line_bytes = env_usize("GITCOMET_BENCH_REPLACEMENT_LINE_BYTES", 128);

    let balanced = ReplacementAlignmentFixture::new(
        blocks,
        balanced_lines,
        balanced_lines,
        context_lines,
        line_bytes,
    );
    let skewed = ReplacementAlignmentFixture::new(
        blocks,
        skewed_old_lines,
        skewed_new_lines,
        context_lines,
        line_bytes,
    );

    let mut group = c.benchmark_group("file_diff_replacement_alignment");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::new("balanced_blocks", "scratch"), |b| {
        b.iter(|| balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch))
    });
    group.bench_function(BenchmarkId::new("balanced_blocks", "strsim"), |b| {
        b.iter(|| balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim))
    });
    group.bench_function(BenchmarkId::new("skewed_blocks", "scratch"), |b| {
        b.iter(|| skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch))
    });
    group.bench_function(BenchmarkId::new("skewed_blocks", "strsim"), |b| {
        b.iter(|| skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim))
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| {
        balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/balanced_blocks/scratch");
    let _ = measure_sidecar_allocations(|| {
        balanced.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/balanced_blocks/strsim");
    let _ = measure_sidecar_allocations(|| {
        skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Scratch)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/skewed_blocks/scratch");
    let _ = measure_sidecar_allocations(|| {
        skewed.run_plan_step_with_backend(BenchmarkReplacementDistanceBackend::Strsim)
    });
    emit_allocation_only_sidecar("file_diff_replacement_alignment/skewed_blocks/strsim");
}

fn emit_text_input_prepaint_windowed_sidecar(
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

fn bench_text_input_prepaint_windowed(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let window_rows = env_usize("GITCOMET_BENCH_TEXT_INPUT_WINDOW_ROWS", 80);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);

    let mut windowed_fixture = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let mut full_fixture = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_prepaint_windowed");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("window_rows", window_rows),
        &window_rows,
        |b, &window_rows| {
            let mut start = 0usize;
            b.iter(|| {
                let h = windowed_fixture.run_windowed_step(start, window_rows.max(1));
                start = start.wrapping_add(window_rows.max(1) / 2 + 1)
                    % windowed_fixture.total_rows().max(1);
                h
            })
        },
    );
    group.bench_function(BenchmarkId::from_parameter("full_document_control"), |b| {
        b.iter(|| full_fixture.run_full_document_step())
    });
    group.finish();

    // Collect metrics from a fresh run for sidecar emission.
    let mut sidecar_windowed = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let (_, windowed_metrics) = measure_sidecar_allocations(|| {
        sidecar_windowed.run_windowed_step_with_metrics(0, window_rows.max(1))
    });
    emit_text_input_prepaint_windowed_sidecar(
        &format!("text_input_prepaint_windowed/window_rows/{window_rows}"),
        &windowed_metrics,
    );

    let mut sidecar_full = TextInputPrepaintWindowedFixture::new(lines, line_bytes, wrap_width);
    let (_, full_metrics) =
        measure_sidecar_allocations(|| sidecar_full.run_full_document_step_with_metrics());
    emit_text_input_prepaint_windowed_sidecar(
        "text_input_prepaint_windowed/full_document_control",
        &full_metrics,
    );
}

fn emit_text_input_runs_streamed_highlight_sidecar(
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

fn bench_text_input_runs_streamed_highlight(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let window_rows = env_usize("GITCOMET_BENCH_TEXT_INPUT_WINDOW_ROWS", 80);

    let dense_fixture = TextInputRunsStreamedHighlightFixture::new(
        lines,
        line_bytes,
        window_rows,
        TextInputHighlightDensity::Dense,
    );
    let sparse_fixture = TextInputRunsStreamedHighlightFixture::new(
        lines,
        line_bytes,
        window_rows,
        TextInputHighlightDensity::Sparse,
    );

    let mut dense_group = c.benchmark_group("text_input_runs_streamed_highlight_dense");
    dense_group.sample_size(10);
    dense_group.warm_up_time(Duration::from_secs(1));
    dense_group.bench_function(BenchmarkId::from_parameter("legacy_scan"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = dense_fixture.run_legacy_step(start);
            start = dense_fixture.next_start_row(start);
            h
        })
    });
    dense_group.bench_function(BenchmarkId::from_parameter("streamed_cursor"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = dense_fixture.run_streamed_step(start);
            start = dense_fixture.next_start_row(start);
            h
        })
    });
    dense_group.finish();

    let mut sparse_group = c.benchmark_group("text_input_runs_streamed_highlight_sparse");
    sparse_group.sample_size(10);
    sparse_group.warm_up_time(Duration::from_secs(1));
    sparse_group.bench_function(BenchmarkId::from_parameter("legacy_scan"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = sparse_fixture.run_legacy_step(start);
            start = sparse_fixture.next_start_row(start);
            h
        })
    });
    sparse_group.bench_function(BenchmarkId::from_parameter("streamed_cursor"), |b| {
        let mut start = 0usize;
        b.iter(|| {
            let h = sparse_fixture.run_streamed_step(start);
            start = sparse_fixture.next_start_row(start);
            h
        })
    });
    sparse_group.finish();

    let (_, dense_legacy_metrics) =
        measure_sidecar_allocations(|| dense_fixture.run_legacy_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_dense/legacy_scan",
        &dense_legacy_metrics,
    );
    let (_, dense_streamed_metrics) =
        measure_sidecar_allocations(|| dense_fixture.run_streamed_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_dense/streamed_cursor",
        &dense_streamed_metrics,
    );
    let (_, sparse_legacy_metrics) =
        measure_sidecar_allocations(|| sparse_fixture.run_legacy_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_sparse/legacy_scan",
        &sparse_legacy_metrics,
    );
    let (_, sparse_streamed_metrics) =
        measure_sidecar_allocations(|| sparse_fixture.run_streamed_step_with_metrics(0));
    emit_text_input_runs_streamed_highlight_sidecar(
        "text_input_runs_streamed_highlight_sparse/streamed_cursor",
        &sparse_streamed_metrics,
    );
}

fn emit_text_input_long_line_cap_sidecar(bench: &str, metrics: &TextInputLongLineCapMetrics) {
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

fn bench_text_input_long_line_cap(c: &mut Criterion) {
    let long_line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LONG_LINE_BYTES", 256 * 1024);
    let max_shape_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_MAX_LINE_SHAPE_BYTES", 4 * 1024);
    let fixture = TextInputLongLineCapFixture::new(long_line_bytes);

    let mut group = c.benchmark_group("text_input_long_line_cap");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::new("capped_bytes", max_shape_bytes), |b| {
        b.iter(|| fixture.run_with_cap(max_shape_bytes))
    });
    group.bench_function(BenchmarkId::from_parameter("uncapped_control"), |b| {
        b.iter(|| fixture.run_without_cap())
    });
    group.finish();

    let (_, capped_metrics) =
        measure_sidecar_allocations(|| fixture.run_with_cap_with_metrics(max_shape_bytes));
    emit_text_input_long_line_cap_sidecar(
        &format!("text_input_long_line_cap/capped_bytes/{max_shape_bytes}"),
        &capped_metrics,
    );
    let (_, uncapped_metrics) =
        measure_sidecar_allocations(|| fixture.run_without_cap_with_metrics());
    emit_text_input_long_line_cap_sidecar(
        "text_input_long_line_cap/uncapped_control",
        &uncapped_metrics,
    );
}

fn bench_text_input_wrap_incremental_tabs(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);
    let mut full_fixture = TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let mut incremental_fixture =
        TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_wrap_incremental_tabs");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("full_recompute"), |b| {
        let mut edit_ix = 0usize;
        b.iter(|| {
            let h = full_fixture.run_full_recompute_step(edit_ix);
            edit_ix = edit_ix.wrapping_add(17);
            h
        })
    });
    group.bench_function(BenchmarkId::from_parameter("incremental_patch"), |b| {
        let mut edit_ix = 0usize;
        b.iter(|| {
            let h = incremental_fixture.run_incremental_step(edit_ix);
            edit_ix = edit_ix.wrapping_add(17);
            h
        })
    });
    group.finish();

    let mut sidecar_full = TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let (_, full_metrics) =
        measure_sidecar_allocations(|| sidecar_full.run_full_recompute_step_with_metrics(0));
    emit_text_input_wrap_incremental_tabs_sidecar(
        "text_input_wrap_incremental_tabs/full_recompute",
        &full_metrics,
    );

    let mut sidecar_incremental =
        TextInputWrapIncrementalTabsFixture::new(lines, line_bytes, wrap_width);
    let (_, incremental_metrics) =
        measure_sidecar_allocations(|| sidecar_incremental.run_incremental_step_with_metrics(0));
    emit_text_input_wrap_incremental_tabs_sidecar(
        "text_input_wrap_incremental_tabs/incremental_patch",
        &incremental_metrics,
    );
}

fn emit_text_input_wrap_incremental_tabs_sidecar(
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

fn emit_text_input_wrap_incremental_burst_edits_sidecar(
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

fn bench_text_input_wrap_incremental_burst_edits(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_INPUT_LINE_BYTES", 128);
    let wrap_width = env_usize("GITCOMET_BENCH_TEXT_INPUT_WRAP_WIDTH_PX", 720);
    let edits_per_burst = env_usize("GITCOMET_BENCH_TEXT_INPUT_BURST_EDITS", 12);
    let mut full_fixture =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let mut incremental_fixture =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);

    let mut group = c.benchmark_group("text_input_wrap_incremental_burst_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("full_recompute", edits_per_burst),
        &edits_per_burst,
        |b, &edits_per_burst| {
            b.iter(|| full_fixture.run_full_recompute_burst_step(edits_per_burst))
        },
    );
    group.bench_with_input(
        BenchmarkId::new("incremental_patch", edits_per_burst),
        &edits_per_burst,
        |b, &edits_per_burst| {
            b.iter(|| incremental_fixture.run_incremental_burst_step(edits_per_burst))
        },
    );
    group.finish();

    let mut sidecar_full =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let (_, full_metrics) = measure_sidecar_allocations(|| {
        sidecar_full.run_full_recompute_burst_step_with_metrics(edits_per_burst)
    });
    emit_text_input_wrap_incremental_burst_edits_sidecar(
        &format!("text_input_wrap_incremental_burst_edits/full_recompute/{edits_per_burst}"),
        &full_metrics,
    );

    let mut sidecar_incremental =
        TextInputWrapIncrementalBurstEditsFixture::new(lines, line_bytes, wrap_width);
    let (_, incremental_metrics) = measure_sidecar_allocations(|| {
        sidecar_incremental.run_incremental_burst_step_with_metrics(edits_per_burst)
    });
    emit_text_input_wrap_incremental_burst_edits_sidecar(
        &format!("text_input_wrap_incremental_burst_edits/incremental_patch/{edits_per_burst}"),
        &incremental_metrics,
    );
}

fn bench_text_model_snapshot_clone_cost(c: &mut Criterion) {
    let bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_BYTES", 2 * 1024 * 1024);
    let clones = env_usize("GITCOMET_BENCH_TEXT_MODEL_SNAPSHOT_CLONES", 8_192);
    let fixture = TextModelSnapshotCloneCostFixture::new(bytes);

    let mut group = c.benchmark_group("text_model_snapshot_clone_cost");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("piece_table_snapshot_clone", clones),
        &clones,
        |b, &clones| b.iter(|| fixture.run_snapshot_clone_step(clones)),
    );
    group.bench_with_input(
        BenchmarkId::new("shared_string_clone_control", clones),
        &clones,
        |b, &clones| b.iter(|| fixture.run_string_clone_control_step(clones)),
    );
    group.finish();

    let (_, snapshot_metrics) =
        measure_sidecar_allocations(|| fixture.run_snapshot_clone_step_with_metrics(clones));
    emit_text_model_snapshot_clone_cost_sidecar(
        &format!("text_model_snapshot_clone_cost/piece_table_snapshot_clone/{clones}"),
        &snapshot_metrics,
    );

    let (_, control_metrics) =
        measure_sidecar_allocations(|| fixture.run_string_clone_control_step_with_metrics(clones));
    emit_text_model_snapshot_clone_cost_sidecar(
        &format!("text_model_snapshot_clone_cost/shared_string_clone_control/{clones}"),
        &control_metrics,
    );
}

fn emit_text_model_snapshot_clone_cost_sidecar(
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

fn bench_text_model_bulk_load_large(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_TEXT_MODEL_LINES", 20_000);
    let line_bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_LINE_BYTES", 128);
    let fixture = TextModelBulkLoadLargeFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("text_model_bulk_load_large");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("piece_table_append_large"),
        |b| b.iter(|| fixture.run_piece_table_bulk_load_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("piece_table_from_large_text"),
        |b| b.iter(|| fixture.run_piece_table_from_large_text_step()),
    );
    group.bench_function(BenchmarkId::from_parameter("string_push_control"), |b| {
        b.iter(|| fixture.run_string_bulk_load_control_step())
    });
    group.finish();

    let (_, append_metrics) =
        measure_sidecar_allocations(|| fixture.run_piece_table_bulk_load_step_with_metrics());
    emit_text_model_bulk_load_large_sidecar(
        "text_model_bulk_load_large/piece_table_append_large",
        &append_metrics,
    );

    let (_, from_large_metrics) =
        measure_sidecar_allocations(|| fixture.run_piece_table_from_large_text_step_with_metrics());
    emit_text_model_bulk_load_large_sidecar(
        "text_model_bulk_load_large/piece_table_from_large_text",
        &from_large_metrics,
    );

    let (_, control_metrics) =
        measure_sidecar_allocations(|| fixture.run_string_bulk_load_control_step_with_metrics());
    emit_text_model_bulk_load_large_sidecar(
        "text_model_bulk_load_large/string_push_control",
        &control_metrics,
    );
}

fn emit_text_model_bulk_load_large_sidecar(bench: &str, metrics: &TextModelBulkLoadLargeMetrics) {
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

fn bench_text_model_fragmented_edits(c: &mut Criterion) {
    let bytes = env_usize("GITCOMET_BENCH_TEXT_MODEL_BYTES", 512 * 1024);
    let edits = env_usize("GITCOMET_BENCH_TEXT_MODEL_EDITS", 500);
    let reads = env_usize("GITCOMET_BENCH_TEXT_MODEL_READS_AFTER_EDIT", 64);
    let fixture = TextModelFragmentedEditFixture::new(bytes, edits);

    let mut group = c.benchmark_group("text_model_fragmented_edits");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("piece_table_edits"), |b| {
        b.iter(|| fixture.run_fragmented_edit_step())
    });
    group.bench_function(
        BenchmarkId::from_parameter("materialize_after_edits"),
        |b| b.iter(|| fixture.run_materialize_after_edits_step()),
    );
    group.bench_function(BenchmarkId::new("shared_string_after_edits", reads), |b| {
        b.iter(|| fixture.run_shared_string_after_edits_step(reads))
    });
    group.bench_function(BenchmarkId::from_parameter("string_edit_control"), |b| {
        b.iter(|| fixture.run_string_edit_control_step())
    });
    group.finish();

    let (_, piece_table_metrics) =
        measure_sidecar_allocations(|| fixture.run_fragmented_edit_step_with_metrics());
    emit_text_model_fragmented_edits_sidecar(
        "text_model_fragmented_edits/piece_table_edits",
        &piece_table_metrics,
    );

    let (_, materialize_metrics) =
        measure_sidecar_allocations(|| fixture.run_materialize_after_edits_step_with_metrics());
    emit_text_model_fragmented_edits_sidecar(
        "text_model_fragmented_edits/materialize_after_edits",
        &materialize_metrics,
    );

    let (_, shared_metrics) = measure_sidecar_allocations(|| {
        fixture.run_shared_string_after_edits_step_with_metrics(reads)
    });
    emit_text_model_fragmented_edits_sidecar(
        &format!("text_model_fragmented_edits/shared_string_after_edits/{reads}"),
        &shared_metrics,
    );

    let (_, control_metrics) =
        measure_sidecar_allocations(|| fixture.run_string_edit_control_step_with_metrics());
    emit_text_model_fragmented_edits_sidecar(
        "text_model_fragmented_edits/string_edit_control",
        &control_metrics,
    );
}

fn emit_text_model_fragmented_edits_sidecar(
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

fn bench_file_diff_syntax_prepare(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);
    fixture.prewarm();

    let mut group = c.benchmark_group("file_diff_syntax_prepare");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let mut cold_nonce = 0u64;
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_prepare_cold"),
        |b| {
            b.iter(|| {
                cold_nonce = cold_nonce.wrapping_add(1);
                fixture.run_prepare_cold(cold_nonce)
            })
        },
    );
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_prepare_warm"),
        |b| b.iter(|| fixture.run_prepare_warm()),
    );
    group.finish();

    cold_nonce = cold_nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| fixture.run_prepare_cold(cold_nonce));
    emit_allocation_only_sidecar("file_diff_syntax_prepare/file_diff_syntax_prepare_cold");
    let _ = measure_sidecar_allocations(|| fixture.run_prepare_warm());
    emit_allocation_only_sidecar("file_diff_syntax_prepare/file_diff_syntax_prepare_warm");
}

fn bench_file_diff_syntax_query_stress(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINES", 256);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_LINE_BYTES", 4_096);
    let nesting_depth = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_STRESS_NESTING", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new_query_stress(lines, line_bytes, nesting_depth);

    let mut group = c.benchmark_group("file_diff_syntax_query_stress");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    let mut nonce = 0u64;
    group.bench_function(BenchmarkId::from_parameter("nested_long_lines_cold"), |b| {
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            fixture.run_prepare_cold(nonce)
        })
    });
    group.finish();

    nonce = nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| fixture.run_prepare_cold(nonce));
    emit_allocation_only_sidecar("file_diff_syntax_query_stress/nested_long_lines_cold");
}

fn bench_file_diff_syntax_reparse(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_LINE_BYTES", 128);
    let mut small_fixture = FileDiffSyntaxReparseFixture::new(lines, line_bytes);
    let mut large_fixture = FileDiffSyntaxReparseFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("file_diff_syntax_reparse");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_reparse_small_edit"),
        |b| b.iter(|| small_fixture.run_small_edit_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("file_diff_syntax_reparse_large_edit"),
        |b| b.iter(|| large_fixture.run_large_edit_step()),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| small_fixture.run_small_edit_step());
    emit_allocation_only_sidecar("file_diff_syntax_reparse/file_diff_syntax_reparse_small_edit");
    let _ = measure_sidecar_allocations(|| large_fixture.run_large_edit_step());
    emit_allocation_only_sidecar("file_diff_syntax_reparse/file_diff_syntax_reparse_large_edit");
}

fn bench_file_diff_inline_syntax_projection(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_LINE_BYTES", 128);
    let window = env_usize("GITCOMET_BENCH_FILE_DIFF_INLINE_WINDOW", 200);
    let pending_fixture = FileDiffInlineSyntaxProjectionFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("file_diff_inline_syntax_projection");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("visible_window_pending", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let hash = pending_fixture.run_window_pending_step(start, window);
                start = pending_fixture.next_start_row(start, window);
                hash
            })
        },
    );

    let ready_fixture = FileDiffInlineSyntaxProjectionFixture::new(lines, line_bytes);
    ready_fixture.prime_window(window);
    group.bench_with_input(
        BenchmarkId::new("visible_window_ready", window),
        &window,
        |b, &window| b.iter(|| ready_fixture.run_window_step(0, window)),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| pending_fixture.run_window_pending_step(0, window));
    emit_allocation_only_sidecar(&format!(
        "file_diff_inline_syntax_projection/visible_window_pending/{window}"
    ));
    let _ = measure_sidecar_allocations(|| ready_fixture.run_window_step(0, window));
    emit_allocation_only_sidecar(&format!(
        "file_diff_inline_syntax_projection/visible_window_ready/{window}"
    ));
}

fn bench_file_diff_syntax_cache_drop(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_LINES", 2_048);
    let tokens_per_line = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_TOKENS_PER_LINE", 8);
    let replacements = env_usize("GITCOMET_BENCH_FILE_DIFF_SYNTAX_DROP_REPLACEMENTS", 4);
    let fixture = FileDiffSyntaxCacheDropFixture::new(lines, tokens_per_line, replacements);

    let mut group = c.benchmark_group("file_diff_syntax_cache_drop");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("deferred_drop", replacements),
        &replacements,
        |b, &_replacements| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut seed = 0usize;
                for _ in 0..iters {
                    let _ = fixture.flush_deferred_drop_queue();
                    total = total.saturating_add(fixture.run_deferred_drop_timed_step(seed));
                    seed = seed.wrapping_add(1);
                }
                total
            })
        },
    );
    let _ = fixture.flush_deferred_drop_queue();
    group.bench_with_input(
        BenchmarkId::new("inline_drop_control", replacements),
        &replacements,
        |b, &_replacements| {
            b.iter_custom(|iters| {
                let mut total = Duration::ZERO;
                let mut seed = 0usize;
                for _ in 0..iters {
                    total = total.saturating_add(fixture.run_inline_drop_control_timed_step(seed));
                    seed = seed.wrapping_add(1);
                }
                total
            })
        },
    );
    group.finish();

    let _ = fixture.flush_deferred_drop_queue();
    let _ = measure_sidecar_allocations(|| fixture.run_deferred_drop_timed_step(0usize));
    emit_allocation_only_sidecar(&format!(
        "file_diff_syntax_cache_drop/deferred_drop/{replacements}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_inline_drop_control_timed_step(0usize));
    emit_allocation_only_sidecar(&format!(
        "file_diff_syntax_cache_drop/inline_drop_control/{replacements}"
    ));
}

fn bench_prepared_syntax_multidoc_cache_hit_rate(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINE_BYTES", 128);
    let docs = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_HOT_DOCS", 6);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("prepared_syntax_multidoc_cache_hit_rate");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    let mut nonce = 0u64;
    group.bench_with_input(BenchmarkId::new("hot_docs", docs), &docs, |b, &docs| {
        b.iter(|| {
            nonce = nonce.wrapping_add(1);
            fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(docs, nonce)
        })
    });
    group.finish();

    nonce = nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| {
        fixture.run_prepared_syntax_multidoc_cache_hit_rate_step(docs, nonce)
    });
    emit_allocation_only_sidecar(&format!(
        "prepared_syntax_multidoc_cache_hit_rate/hot_docs/{docs}"
    ));
}

fn bench_prepared_syntax_chunk_miss_cost(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINES", 4_000);
    let line_bytes = env_usize("GITCOMET_BENCH_PREPARED_SYNTAX_LINE_BYTES", 128);
    let fixture = FileDiffSyntaxPrepareFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("prepared_syntax_chunk_miss_cost");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    let mut nonce = 0u64;
    group.bench_function(BenchmarkId::from_parameter("chunk_miss"), |b| {
        b.iter_custom(|iters| {
            let mut total = Duration::ZERO;
            for _ in 0..iters {
                nonce = nonce.wrapping_add(1);
                total =
                    total.saturating_add(fixture.run_prepared_syntax_chunk_miss_cost_step(nonce));
            }
            total
        })
    });
    group.finish();

    nonce = nonce.wrapping_add(1);
    let _ = measure_sidecar_allocations(|| fixture.run_prepared_syntax_chunk_miss_cost_step(nonce));
    emit_allocation_only_sidecar("prepared_syntax_chunk_miss_cost/chunk_miss");
}

fn bench_large_html_syntax(c: &mut Criterion) {
    let fixture_path = env_string("GITCOMET_BENCH_HTML_FIXTURE_PATH");
    let synthetic_lines = env_usize("GITCOMET_BENCH_HTML_LINES", 20_000);
    let synthetic_line_bytes = env_usize("GITCOMET_BENCH_HTML_LINE_BYTES", 192);
    let window_lines = env_usize("GITCOMET_BENCH_HTML_WINDOW_LINES", 160);
    let prepare_fixture = LargeHtmlSyntaxFixture::new(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let source_label = prepare_fixture.source_label().to_string();

    let mut group = c.benchmark_group("large_html_syntax");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::new(source_label.as_str(), "background_prepare"),
        |b| b.iter(|| prepare_fixture.run_background_prepare_step()),
    );
    let pending_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let mut pending_start_line = 0usize;
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_pending"),
        &window_lines,
        |b, &window_lines| {
            b.iter(|| {
                let hash = pending_fixture
                    .run_visible_window_pending_step(pending_start_line, window_lines);
                pending_start_line =
                    pending_fixture.next_start_line(pending_start_line, window_lines);
                hash
            })
        },
    );
    let visible_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    visible_fixture.prime_visible_window(window_lines);
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_steady"),
        &window_lines,
        |b, &window_lines| b.iter(|| visible_fixture.run_visible_window_step(0, window_lines)),
    );
    let mut start_line = 0usize;
    group.bench_with_input(
        BenchmarkId::new(source_label.as_str(), "visible_window_sweep"),
        &window_lines,
        |b, &window_lines| {
            b.iter(|| {
                let hash = visible_fixture.run_visible_window_step(start_line, window_lines);
                start_line = visible_fixture.next_start_line(start_line, window_lines);
                hash
            })
        },
    );
    group.finish();

    let (_, prepare_metrics) = measure_sidecar_allocations(|| {
        LargeHtmlSyntaxFixture::new(
            fixture_path.as_deref(),
            synthetic_lines,
            synthetic_line_bytes,
        )
        .run_background_prepare_with_metrics()
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/background_prepare"),
        prepare_metrics,
    );

    let pending_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    let (_, pending_metrics) = measure_sidecar_allocations(|| {
        pending_metrics_fixture.run_visible_window_pending_with_metrics(0, window_lines)
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/visible_window_pending"),
        pending_metrics,
    );

    let steady_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    steady_metrics_fixture.prime_visible_window_until_ready(window_lines);
    let (_, steady_metrics) = measure_sidecar_allocations(|| {
        steady_metrics_fixture.run_visible_window_with_metrics(0, window_lines)
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/visible_window_steady"),
        steady_metrics,
    );

    let sweep_metrics_fixture = LargeHtmlSyntaxFixture::new_prewarmed(
        fixture_path.as_deref(),
        synthetic_lines,
        synthetic_line_bytes,
    );
    sweep_metrics_fixture.prime_visible_window_until_ready(window_lines);
    let sweep_start_line = sweep_metrics_fixture.next_start_line(0, window_lines);
    let (_, sweep_metrics) = measure_sidecar_allocations(|| {
        sweep_metrics_fixture.run_visible_window_with_metrics(sweep_start_line, window_lines)
    });
    emit_large_html_syntax_sidecar(
        &format!("large_html_syntax/{source_label}/visible_window_sweep"),
        sweep_metrics,
    );
}

fn emit_large_html_syntax_sidecar(bench_name: &str, metrics: LargeHtmlSyntaxMetrics) {
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

fn bench_worktree_preview_render(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_LINES", 4_000);
    let window = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_WORKTREE_PREVIEW_LINE_BYTES", 128);
    let fixture = WorktreePreviewRenderFixture::new(lines, line_bytes);

    let mut group = c.benchmark_group("worktree_preview_render");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("cached_lookup_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_cached_lookup_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.bench_with_input(
        BenchmarkId::new("render_time_prepare_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_render_time_prepare_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    // Sidecar metrics emission for structural budgets.
    let (_, cached_metrics) =
        measure_sidecar_allocations(|| fixture.run_cached_lookup_with_metrics(0, window));
    emit_worktree_preview_render_sidecar(
        &format!("worktree_preview_render/cached_lookup_window/{window}"),
        &cached_metrics,
    );

    let sidecar_fixture = WorktreePreviewRenderFixture::new(lines, line_bytes);
    let (_, prepare_metrics) = measure_sidecar_allocations(|| {
        sidecar_fixture.run_render_time_prepare_with_metrics(0, window)
    });
    emit_worktree_preview_render_sidecar(
        &format!("worktree_preview_render/render_time_prepare_window/{window}"),
        &prepare_metrics,
    );
}

fn emit_worktree_preview_render_sidecar(bench: &str, metrics: &WorktreePreviewRenderMetrics) {
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

fn bench_markdown_preview_parse_build(c: &mut Criterion) {
    {
        let medium_sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_MEDIUM_SECTIONS", 256);
        let large_sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_LARGE_SECTIONS", 768);
        let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_LINE_BYTES", 128);
        let medium = MarkdownPreviewFixture::new(medium_sections, line_bytes);
        let large = MarkdownPreviewFixture::new(large_sections, line_bytes);

        let mut group = c.benchmark_group("markdown_preview_parse_build");
        group.sample_size(10);
        group.warm_up_time(Duration::from_secs(1));

        for (label, fixture) in [("medium", &medium), ("large", &large)] {
            group.bench_function(BenchmarkId::new("single_document", label), |b| {
                b.iter(|| fixture.run_parse_single_step())
            });
            group.bench_function(BenchmarkId::new("two_sided_diff", label), |b| {
                b.iter(|| fixture.run_parse_diff_step())
            });
        }

        group.finish();

        let _ = measure_sidecar_allocations(|| medium.run_parse_single_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/single_document/medium");
        let _ = measure_sidecar_allocations(|| medium.run_parse_diff_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/two_sided_diff/medium");
        let _ = measure_sidecar_allocations(|| large.run_parse_single_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/single_document/large");
        let _ = measure_sidecar_allocations(|| large.run_parse_diff_step());
        emit_allocation_only_sidecar("markdown_preview_parse_build/two_sided_diff/large");
    }

    settle_markdown_allocator_pages();
}

fn bench_markdown_preview_render(c: &mut Criterion) {
    let sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_SECTIONS", 384);
    let window = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_WINDOW", 200);
    let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_LINE_BYTES", 128);
    let measurement_time = markdown_preview_measurement_time();

    {
        let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

        let mut single_group = c.benchmark_group("markdown_preview_render_single");
        single_group.sample_size(10);
        single_group.warm_up_time(measurement_time);
        single_group.measurement_time(measurement_time);
        single_group.bench_with_input(
            BenchmarkId::new("window_rows", window),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let hash = fixture.run_render_single_step(start, window);
                    start = start.wrapping_add(window);
                    hash
                })
            },
        );
        single_group.finish();

        let _ = measure_sidecar_allocations(|| fixture.run_render_single_step(0, window));
        emit_allocation_only_sidecar(&format!(
            "markdown_preview_render_single/window_rows/{window}"
        ));
    }

    settle_markdown_allocator_pages();

    {
        let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

        let mut diff_group = c.benchmark_group("markdown_preview_render_diff");
        diff_group.sample_size(10);
        diff_group.warm_up_time(measurement_time);
        diff_group.measurement_time(measurement_time);
        diff_group.bench_with_input(
            BenchmarkId::new("window_rows", window),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let hash = fixture.run_render_diff_step(start, window);
                    start = start.wrapping_add(window);
                    hash
                })
            },
        );
        diff_group.finish();

        let _ = measure_sidecar_allocations(|| fixture.run_render_diff_step(0, window));
        emit_allocation_only_sidecar(&format!(
            "markdown_preview_render_diff/window_rows/{window}"
        ));
    }

    settle_markdown_allocator_pages();
}

fn emit_markdown_preview_first_window_sidecar(
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

fn bench_diff_open_markdown_preview_first_window(c: &mut Criterion) {
    {
        let sections = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_SECTIONS", 384);
        let window = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_WINDOW", 200);
        let line_bytes = env_usize("GITCOMET_BENCH_MARKDOWN_PREVIEW_RENDER_LINE_BYTES", 128);
        let measurement_time = markdown_preview_measurement_time();
        let fixture = MarkdownPreviewFixture::new(sections, line_bytes);

        let metrics = measure_sidecar_allocations(|| fixture.measure_first_window_diff(window));

        let mut group = c.benchmark_group("diff_open_markdown_preview_first_window");
        group.sample_size(10);
        group.warm_up_time(measurement_time);
        group.measurement_time(measurement_time);
        group.bench_with_input(
            BenchmarkId::from_parameter(window),
            &window,
            |b, &window| b.iter(|| fixture.run_first_window_diff_step(window)),
        );
        group.finish();
        emit_markdown_preview_first_window_sidecar(window, &metrics);
    }

    settle_markdown_allocator_pages();
}

fn emit_image_preview_first_paint_sidecar(metrics: &ImagePreviewFirstPaintMetrics) {
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

fn bench_diff_open_image_preview_first_paint(c: &mut Criterion) {
    let old_bytes = env_usize("GITCOMET_BENCH_IMAGE_PREVIEW_OLD_BYTES", 256 * 1024);
    let new_bytes = env_usize("GITCOMET_BENCH_IMAGE_PREVIEW_NEW_BYTES", 384 * 1024);
    let fixture = ImagePreviewFirstPaintFixture::new(old_bytes, new_bytes);
    let metrics = measure_sidecar_allocations(|| fixture.measure_first_paint());

    c.bench_function("diff_open_image_preview_first_paint", |b| {
        b.iter(|| fixture.run_first_paint_step())
    });
    emit_image_preview_first_paint_sidecar(&metrics);
}

fn emit_svg_dual_path_first_window_sidecar(window: usize, metrics: &SvgDualPathFirstWindowMetrics) {
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

fn bench_diff_open_svg_dual_path_first_window(c: &mut Criterion) {
    let shapes = env_usize("GITCOMET_BENCH_SVG_SHAPES", 200);
    let fallback_bytes = env_usize("GITCOMET_BENCH_SVG_FALLBACK_BYTES", 64 * 1024);
    let window = env_usize("GITCOMET_BENCH_SVG_WINDOW", 200);
    let fixture = SvgDualPathFirstWindowFixture::new(shapes, fallback_bytes);
    let metrics = measure_sidecar_allocations(|| fixture.measure_first_window());

    let mut group = c.benchmark_group("diff_open_svg_dual_path_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_first_window_step(window)),
    );
    group.finish();
    emit_svg_dual_path_first_window_sidecar(window, &metrics);
}

fn bench_conflict_three_way_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let fixture = ConflictThreeWayScrollFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("style_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_scroll_step(0, window));
    emit_allocation_only_sidecar(&format!("conflict_three_way_scroll/style_window/{window}"));
}

fn bench_conflict_three_way_prepared_syntax_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let fixture =
        ConflictThreeWayScrollFixture::new_with_prepared_documents(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_prepared_syntax_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::new("style_window", window),
        &window,
        |b, &window| {
            let mut start = 0usize;
            b.iter(|| {
                let h = fixture.run_prepared_scroll_step(start, window);
                start = start.wrapping_add(window) % lines.max(1);
                h
            })
        },
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_prepared_scroll_step(0, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_three_way_prepared_syntax_scroll/style_window/{window}"
    ));
}

fn bench_conflict_three_way_visible_map_build(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictThreeWayVisibleMapBuildFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_three_way_visible_map_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("linear_two_pointer"), |b| {
        b.iter(|| fixture.run_linear_step())
    });
    group.bench_function(BenchmarkId::from_parameter("legacy_find_scan"), |b| {
        b.iter(|| fixture.run_legacy_step())
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_linear_step());
    emit_allocation_only_sidecar("conflict_three_way_visible_map_build/linear_two_pointer");
    let _ = measure_sidecar_allocations(|| fixture.run_legacy_step());
    emit_allocation_only_sidecar("conflict_three_way_visible_map_build/legacy_find_scan");
}

fn bench_conflict_two_way_split_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictTwoWaySplitScrollFixture::new(lines, conflict_blocks);
    let windows = [100usize, 200, 400];

    let mut group = c.benchmark_group("conflict_two_way_split_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for &window in &windows {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("window_{window}")),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let h = fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(window) % fixture.visible_rows().max(1);
                    h
                })
            },
        );
    }
    group.finish();

    for &window in &windows {
        let _ = measure_sidecar_allocations(|| fixture.run_scroll_step(0, window));
        emit_allocation_only_sidecar(&format!("conflict_two_way_split_scroll/window_{window}"));
    }
}

fn bench_conflict_load_duplication(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictLoadDuplicationFixture::new(lines, low_density_blocks);
    let high_density = ConflictLoadDuplicationFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_load_duplication");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("shared_payload_forwarding", label), |b| {
            b.iter(|| fixture.run_shared_payload_forwarding_step())
        });
        group.bench_function(BenchmarkId::new("duplicated_text_and_bytes", label), |b| {
            b.iter(|| fixture.run_duplicated_payload_forwarding_step())
        });
    }
    group.finish();

    let _ = measure_sidecar_allocations(|| low_density.run_shared_payload_forwarding_step());
    emit_allocation_only_sidecar("conflict_load_duplication/shared_payload_forwarding/low_density");
    let _ = measure_sidecar_allocations(|| low_density.run_duplicated_payload_forwarding_step());
    emit_allocation_only_sidecar("conflict_load_duplication/duplicated_text_and_bytes/low_density");
    let _ = measure_sidecar_allocations(|| high_density.run_shared_payload_forwarding_step());
    emit_allocation_only_sidecar(
        "conflict_load_duplication/shared_payload_forwarding/high_density",
    );
    let _ = measure_sidecar_allocations(|| high_density.run_duplicated_payload_forwarding_step());
    emit_allocation_only_sidecar(
        "conflict_load_duplication/duplicated_text_and_bytes/high_density",
    );
}

fn bench_conflict_two_way_diff_build(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictTwoWayDiffBuildFixture::new(lines, low_density_blocks);
    let high_density = ConflictTwoWayDiffBuildFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_two_way_diff_build");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("full_file", label), |b| {
            b.iter(|| fixture.run_full_diff_build_step())
        });
        group.bench_function(BenchmarkId::new("block_local", label), |b| {
            b.iter(|| fixture.run_block_local_diff_build_step())
        });
    }
    group.finish();

    let _ = measure_sidecar_allocations(|| low_density.run_full_diff_build_step());
    emit_allocation_only_sidecar("conflict_two_way_diff_build/full_file/low_density");
    let _ = measure_sidecar_allocations(|| low_density.run_block_local_diff_build_step());
    emit_allocation_only_sidecar("conflict_two_way_diff_build/block_local/low_density");
    let _ = measure_sidecar_allocations(|| high_density.run_full_diff_build_step());
    emit_allocation_only_sidecar("conflict_two_way_diff_build/full_file/high_density");
    let _ = measure_sidecar_allocations(|| high_density.run_block_local_diff_build_step());
    emit_allocation_only_sidecar("conflict_two_way_diff_build/block_local/high_density");
}

fn bench_conflict_two_way_word_highlights(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_MERGETOOL_LINES", 50_000);
    let low_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_LOW_CONFLICT_BLOCKS", 12);
    let high_density_blocks = env_usize("GITCOMET_BENCH_MERGETOOL_HIGH_CONFLICT_BLOCKS", 1_024);
    let low_density = ConflictTwoWayDiffBuildFixture::new(lines, low_density_blocks);
    let high_density = ConflictTwoWayDiffBuildFixture::new(lines, high_density_blocks);

    let mut group = c.benchmark_group("conflict_two_way_word_highlights");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for (label, fixture) in [
        ("low_density", &low_density),
        ("high_density", &high_density),
    ] {
        group.bench_function(BenchmarkId::new("full_file", label), |b| {
            b.iter(|| fixture.run_full_word_highlights_step())
        });
        group.bench_function(BenchmarkId::new("block_local", label), |b| {
            b.iter(|| fixture.run_block_local_word_highlights_step())
        });
    }
    group.finish();

    let _ = measure_sidecar_allocations(|| low_density.run_full_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/full_file/low_density");
    let _ = measure_sidecar_allocations(|| low_density.run_block_local_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/block_local/low_density");
    let _ = measure_sidecar_allocations(|| high_density.run_full_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/full_file/high_density");
    let _ = measure_sidecar_allocations(|| high_density.run_block_local_word_highlights_step());
    emit_allocation_only_sidecar("conflict_two_way_word_highlights/block_local/high_density");
}

fn bench_conflict_resolved_output_gutter_scroll(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let fixture = ConflictResolvedOutputGutterScrollFixture::new(lines, conflict_blocks);
    let windows = [100usize, 200, 400];

    let mut group = c.benchmark_group("conflict_resolved_output_gutter_scroll");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    for &window in &windows {
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("window_{window}")),
            &window,
            |b, &window| {
                let mut start = 0usize;
                b.iter(|| {
                    let h = fixture.run_scroll_step(start, window);
                    start = start.wrapping_add(window) % fixture.visible_rows().max(1);
                    h
                })
            },
        );
    }
    group.finish();

    for &window in &windows {
        let _ = measure_sidecar_allocations(|| fixture.run_scroll_step(0, window));
        emit_allocation_only_sidecar(&format!(
            "conflict_resolved_output_gutter_scroll/window_{window}"
        ));
    }
}

fn bench_conflict_search_query_update(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let mut fixture = ConflictSearchQueryUpdateFixture::new(lines, conflict_blocks);
    let query_cycle = [
        "s", "sh", "sha", "shar", "share", "shared", "shared_", "shared_1",
    ];

    let mut group = c.benchmark_group("conflict_search_query_update");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &window| {
        let mut start = 0usize;
        let mut query_ix = 0usize;
        b.iter(|| {
            let query = query_cycle[query_ix % query_cycle.len()];
            let h = fixture.run_query_update_step(query, start, window);
            query_ix = query_ix.wrapping_add(1);
            start = start.wrapping_add(window.max(1) / 2 + 1) % fixture.visible_rows().max(1);
            h
        })
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_query_update_step("shared_1", 0, window));
    emit_allocation_only_sidecar(&format!("conflict_search_query_update/window/{window}"));
}

fn bench_patch_diff_search_query_update(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 10_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let mut fixture = PatchDiffSearchQueryUpdateFixture::new(lines);
    let query_cycle = [
        "s", "sh", "sha", "shar", "share", "shared", "shared_", "shared_1",
    ];

    let mut group = c.benchmark_group("patch_diff_search_query_update");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(format!("window_{window}")),
        &window,
        |b, &window| {
            let mut start = 0usize;
            let mut query_ix = 0usize;
            b.iter(|| {
                let query = query_cycle[query_ix % query_cycle.len()];
                let h = fixture.run_query_update_step(query, start, window);
                query_ix = query_ix.wrapping_add(1);
                start = start.wrapping_add(window.max(1) / 2 + 1) % fixture.visible_rows().max(1);
                h
            })
        },
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_query_update_step("shared_1", 0, window));
    emit_allocation_only_sidecar(&format!("patch_diff_search_query_update/window_{window}"));
}

fn bench_patch_diff_paged_rows(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let mut group = c.benchmark_group("patch_diff_paged_rows");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("eager_full_materialize"), |b| {
        b.iter(|| fixture.run_eager_full_materialize_step())
    });
    group.bench_with_input(
        BenchmarkId::new("paged_first_window", window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.bench_function(
        BenchmarkId::from_parameter("inline_visible_eager_scan"),
        |b| b.iter(|| fixture.run_inline_visible_eager_scan_step()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("inline_visible_hidden_map"),
        |b| b.iter(|| fixture.run_inline_visible_hidden_map_step()),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_eager_full_materialize_step());
    emit_allocation_only_sidecar("patch_diff_paged_rows/eager_full_materialize");
    let _ = measure_sidecar_allocations(|| fixture.run_paged_first_window_step(window));
    emit_allocation_only_sidecar(&format!(
        "patch_diff_paged_rows/paged_first_window/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_inline_visible_eager_scan_step());
    emit_allocation_only_sidecar("patch_diff_paged_rows/inline_visible_eager_scan");
    let _ = measure_sidecar_allocations(|| fixture.run_inline_visible_hidden_map_step());
    emit_allocation_only_sidecar("patch_diff_paged_rows/inline_visible_hidden_map");
}

fn bench_diff_open_patch_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let sidecar_started_at = Instant::now();
    let metrics = measure_sidecar_allocations(|| fixture.measure_paged_first_window_step(window));
    let first_window_ns = sidecar_started_at
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64;

    let mut group = c.benchmark_group("diff_open_patch_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.finish();
    emit_patch_diff_first_window_sidecar(window, first_window_ns, metrics);
}

fn emit_file_diff_open_sidecar(bench_name: &str, metrics: &FileDiffOpenMetrics) {
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

fn bench_diff_open_file_split_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_FILE_DIFF_WINDOW", 200);
    let fixture = FileDiffOpenFixture::new(lines);

    let metrics = measure_sidecar_allocations(|| fixture.measure_first_window(window));

    let mut group = c.benchmark_group("diff_open_file_split_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_split_first_window(window)),
    );
    group.finish();
    emit_file_diff_open_sidecar(
        &format!("diff_open_file_split_first_window/{window}"),
        &metrics,
    );
}

fn bench_diff_open_file_inline_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_FILE_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_FILE_DIFF_WINDOW", 200);
    let fixture = FileDiffOpenFixture::new(lines);

    let metrics = measure_sidecar_allocations(|| fixture.measure_first_window(window));

    let mut group = c.benchmark_group("diff_open_file_inline_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_inline_first_window(window)),
    );
    group.finish();
    emit_file_diff_open_sidecar(
        &format!("diff_open_file_inline_first_window/{window}"),
        &metrics,
    );
}

fn bench_diff_open_patch_deep_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_LINES", 20_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    // Compute start row at 90% depth.
    let total = fixture.total_rows_hint();
    let start_row = total.saturating_mul(9) / 10;

    let metrics =
        measure_sidecar_allocations(|| fixture.measure_paged_deep_window_step(start_row, window));

    let mut group = c.benchmark_group("diff_open_patch_deep_window_90pct");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_window_at_step(start_row, window)),
    );
    group.finish();

    // Re-use patch diff sidecar format for deep-window metrics.
    emit_patch_diff_sidecar(
        &format!("diff_open_patch_deep_window_90pct/{window}"),
        0,
        metrics,
    );
}

fn bench_diff_open_patch_100k_lines_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_PATCH_DIFF_100K_LINES", 100_000);
    let window = env_usize("GITCOMET_BENCH_PATCH_DIFF_WINDOW", 200);
    let fixture = PatchDiffPagedRowsFixture::new(lines);

    let sidecar_started_at = Instant::now();
    let metrics = measure_sidecar_allocations(|| fixture.measure_paged_first_window_step(window));
    let first_window_ns = sidecar_started_at
        .elapsed()
        .as_nanos()
        .min(u128::from(u64::MAX)) as u64;

    let mut group = c.benchmark_group("diff_open_patch_100k_lines_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_paged_first_window_step(window)),
    );
    group.finish();

    // Emit sidecar with the standard patch diff format.
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
        "patch_page_cache_entries".to_string(),
        json!(metrics.patch_page_cache_entries),
    );
    payload.insert(
        "full_text_materializations".to_string(),
        json!(metrics.full_text_materializations),
    );
    emit_sidecar_metrics(
        &format!("diff_open_patch_100k_lines_first_window/{window}"),
        payload,
    );
}

fn emit_conflict_compare_first_window_sidecar(
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

fn bench_diff_open_conflict_compare_first_window(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_COMPARE_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_COMPARE_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_COMPARE_WINDOW", 200);
    let fixture = ConflictTwoWaySplitScrollFixture::new(lines, conflict_blocks);

    let metrics = measure_sidecar_allocations(|| fixture.measure_first_window(window));

    let mut group = c.benchmark_group("diff_open_conflict_compare_first_window");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(
        BenchmarkId::from_parameter(window),
        &window,
        |b, &window| b.iter(|| fixture.run_scroll_step(0, window)),
    );
    group.finish();
    emit_conflict_compare_first_window_sidecar(window, &metrics);
}

fn emit_diff_refresh_sidecar(sub: &str, metrics: &DiffRefreshMetrics) {
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

fn bench_diff_refresh_rev_only_same_content(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_DIFF_REFRESH_LINES", 5_000);
    let fixture = DiffRefreshFixture::new(lines);

    let rekey_metrics = measure_sidecar_allocations(|| fixture.measure_rekey());
    let rebuild_metrics = measure_sidecar_allocations(|| fixture.measure_rebuild());

    let mut group = c.benchmark_group("diff_refresh_rev_only_same_content");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("rekey"), |b| {
        b.iter(|| fixture.run_rekey_step())
    });
    group.bench_function(BenchmarkId::from_parameter("rebuild"), |b| {
        b.iter(|| fixture.run_rebuild_step())
    });
    group.finish();

    emit_diff_refresh_sidecar("rekey", &rekey_metrics);
    emit_diff_refresh_sidecar("rebuild", &rebuild_metrics);
}

fn bench_conflict_split_resize_step(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let window = env_usize("GITCOMET_BENCH_CONFLICT_WINDOW", 200);
    let resize_query =
        env::var("GITCOMET_BENCH_CONFLICT_RESIZE_QUERY").unwrap_or_else(|_| "shared".to_string());
    let mut fixture = ConflictSplitResizeStepFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_split_resize_step");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &window| {
        let mut start = 0usize;
        b.iter(|| {
            let h = fixture.run_resize_step(resize_query.as_str(), start, window);
            start = start.wrapping_add(window.max(1) / 3 + 1) % fixture.visible_rows().max(1);
            h
        })
    });
    group.finish();

    let _ =
        measure_sidecar_allocations(|| fixture.run_resize_step(resize_query.as_str(), 0, window));
    emit_allocation_only_sidecar(&format!("conflict_split_resize_step/window/{window}"));
}

fn bench_conflict_streamed_provider(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_STREAMED_LINES", 50_000);
    let window = env_usize("GITCOMET_BENCH_STREAMED_WINDOW", 200);

    let fixture = ConflictStreamedProviderFixture::new(lines);

    let mut group = c.benchmark_group("conflict_streamed_provider");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(BenchmarkId::from_parameter("index_build"), |b| {
        b.iter(|| fixture.run_index_build_step())
    });
    group.bench_function(BenchmarkId::from_parameter("projection_build"), |b| {
        b.iter(|| fixture.run_projection_build_step())
    });
    group.bench_with_input(BenchmarkId::new("first_page", window), &window, |b, &w| {
        b.iter(|| fixture.run_first_page_step(w))
    });
    fixture.prime_first_page_cache(window);
    group.bench_with_input(
        BenchmarkId::new("first_page_cache_hit", window),
        &window,
        |b, &w| b.iter(|| fixture.run_first_page_cache_hit_step(w)),
    );
    group.bench_with_input(
        BenchmarkId::new("deep_scroll_50pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_scroll_step(0.5, w)),
    );
    group.bench_with_input(
        BenchmarkId::new("deep_scroll_90pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_scroll_step(0.9, w)),
    );
    group.bench_function(BenchmarkId::from_parameter("search_rare_text"), |b| {
        b.iter(|| fixture.run_search_step("shared_42("))
    });
    group.bench_function(BenchmarkId::from_parameter("search_common_text"), |b| {
        b.iter(|| fixture.run_search_step("compute"))
    });
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_index_build_step());
    emit_allocation_only_sidecar("conflict_streamed_provider/index_build");
    let _ = measure_sidecar_allocations(|| fixture.run_projection_build_step());
    emit_allocation_only_sidecar("conflict_streamed_provider/projection_build");
    let _ = measure_sidecar_allocations(|| fixture.run_first_page_step(window));
    emit_allocation_only_sidecar(&format!("conflict_streamed_provider/first_page/{window}"));
    fixture.prime_first_page_cache(window);
    let _ = measure_sidecar_allocations(|| fixture.run_first_page_cache_hit_step(window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_provider/first_page_cache_hit/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_deep_scroll_step(0.5, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_provider/deep_scroll_50pct/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_deep_scroll_step(0.9, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_provider/deep_scroll_90pct/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_search_step("shared_42("));
    emit_allocation_only_sidecar("conflict_streamed_provider/search_rare_text");
    let _ = measure_sidecar_allocations(|| fixture.run_search_step("compute"));
    emit_allocation_only_sidecar("conflict_streamed_provider/search_common_text");
}

fn bench_conflict_streamed_resolved_output(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_STREAMED_LINES", 50_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 500);
    let window = env_usize("GITCOMET_BENCH_STREAMED_WINDOW", 200);

    let fixture = ConflictStreamedResolvedOutputFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("conflict_streamed_resolved_output");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("projection_build"), |b| {
        b.iter(|| fixture.run_projection_build_step())
    });
    group.bench_with_input(BenchmarkId::new("window", window), &window, |b, &w| {
        b.iter(|| fixture.run_window_step(w))
    });
    group.bench_with_input(
        BenchmarkId::new("deep_window_90pct", window),
        &window,
        |b, &w| b.iter(|| fixture.run_deep_window_step(0.9, w)),
    );
    group.finish();

    let _ = measure_sidecar_allocations(|| fixture.run_projection_build_step());
    emit_allocation_only_sidecar("conflict_streamed_resolved_output/projection_build");
    let _ = measure_sidecar_allocations(|| fixture.run_window_step(window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_resolved_output/window/{window}"
    ));
    let _ = measure_sidecar_allocations(|| fixture.run_deep_window_step(0.9, window));
    emit_allocation_only_sidecar(&format!(
        "conflict_streamed_resolved_output/deep_window_90pct/{window}"
    ));
}

fn emit_window_resize_layout_sidecar(bench: &str, metrics: &WindowResizeLayoutMetrics) {
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

fn emit_window_resize_layout_extreme_sidecar(
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

fn emit_history_column_resize_sidecar(bench: &str, metrics: &HistoryColumnResizeMetrics) {
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

fn emit_repo_tab_drag_sidecar(bench: &str, metrics: &RepoTabDragMetrics) {
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

fn emit_pane_resize_drag_sidecar(bench: &str, metrics: &PaneResizeDragMetrics) {
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

fn bench_pane_resize_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("pane_resize_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    let targets: &[(&str, PaneResizeTarget)] = &[
        ("sidebar", PaneResizeTarget::Sidebar),
        ("details", PaneResizeTarget::Details),
    ];

    for &(name, target) in targets {
        group.bench_function(name, |b| {
            let mut fixture = PaneResizeDragStepFixture::new(target);
            b.iter(|| fixture.run())
        });

        let mut fixture = PaneResizeDragStepFixture::new(target);
        let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
        emit_pane_resize_drag_sidecar(&format!("pane_resize_drag_step/{name}"), &metrics);
    }

    group.finish();
}

fn emit_diff_split_resize_drag_sidecar(bench: &str, metrics: &DiffSplitResizeDragMetrics) {
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

fn bench_diff_split_resize_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("diff_split_resize_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    group.bench_function("window_200", |b| {
        let mut fixture = DiffSplitResizeDragStepFixture::window_200();
        b.iter(|| fixture.run())
    });

    // Emit sidecar from a final run.
    let mut fixture = DiffSplitResizeDragStepFixture::window_200();
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
    emit_diff_split_resize_drag_sidecar("diff_split_resize_drag_step/window_200", &metrics);

    group.finish();
}

fn bench_window_resize_layout(c: &mut Criterion) {
    let fixture = WindowResizeLayoutFixture::sidebar_main_details();
    let mut group = c.benchmark_group("window_resize_layout");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    group.bench_function("sidebar_main_details", |b| {
        b.iter(|| {
            let (hash, _metrics) = fixture.run_with_metrics();
            hash
        })
    });

    // Emit sidecar from a final run.
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
    emit_window_resize_layout_sidecar("window_resize_layout/sidebar_main_details", &metrics);

    group.finish();
}

fn bench_window_resize_layout_extreme_scale(c: &mut Criterion) {
    let fixture = WindowResizeLayoutExtremeFixture::history_50k_commits_diff_20k_lines();
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());

    let mut group = c.benchmark_group("window_resize_layout");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(
        BenchmarkId::from_parameter("history_50k_commits_diff_20k_lines"),
        |b| b.iter(|| fixture.run()),
    );
    group.finish();

    emit_window_resize_layout_extreme_sidecar(
        "window_resize_layout/history_50k_commits_diff_20k_lines",
        &metrics,
    );
}

fn bench_history_column_resize_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("history_column_resize_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    let columns: &[(&str, HistoryResizeColumn)] = &[
        ("branch", HistoryResizeColumn::Branch),
        ("graph", HistoryResizeColumn::Graph),
        ("author", HistoryResizeColumn::Author),
        ("date", HistoryResizeColumn::Date),
        ("sha", HistoryResizeColumn::Sha),
    ];

    for &(name, column) in columns {
        group.bench_function(name, |b| {
            let mut fixture = HistoryColumnResizeDragStepFixture::new(column);
            b.iter(|| fixture.run(column))
        });

        // Emit sidecar from a final run.
        let mut fixture = HistoryColumnResizeDragStepFixture::new(column);
        let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics(column));
        emit_history_column_resize_sidecar(
            &format!("history_column_resize_drag_step/{name}"),
            &metrics,
        );
    }

    group.finish();
}

fn bench_repo_tab_drag(c: &mut Criterion) {
    let mut group = c.benchmark_group("repo_tab_drag");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    for &tab_count in &[20usize, 200usize] {
        let fixture = RepoTabDragFixture::new(tab_count);

        group.bench_function(
            BenchmarkId::new("hit_test", format!("{tab_count}_tabs")),
            |b| b.iter(|| fixture.run_hit_test()),
        );
        group.bench_function(
            BenchmarkId::new("reorder_reduce", format!("{tab_count}_tabs")),
            |b| b.iter(|| fixture.run_reorder()),
        );

        // Emit sidecars from final runs.
        let (_, hit_metrics) = measure_sidecar_allocations(|| fixture.run_hit_test());
        emit_repo_tab_drag_sidecar(
            &format!("repo_tab_drag/hit_test/{tab_count}_tabs"),
            &hit_metrics,
        );
        let (_, reorder_metrics) = measure_sidecar_allocations(|| fixture.run_reorder());
        emit_repo_tab_drag_sidecar(
            &format!("repo_tab_drag/reorder_reduce/{tab_count}_tabs"),
            &reorder_metrics,
        );
    }

    group.finish();
}

fn bench_resolved_output_recompute_incremental(c: &mut Criterion) {
    let lines = env_usize("GITCOMET_BENCH_CONFLICT_LINES", 10_000);
    let conflict_blocks = env_usize("GITCOMET_BENCH_CONFLICT_BLOCKS", 300);
    let mut full_fixture = ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let mut incremental_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);

    let mut group = c.benchmark_group("resolved_output_recompute_incremental");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.bench_function(BenchmarkId::from_parameter("full_recompute"), |b| {
        b.iter(|| full_fixture.run_full_recompute_step())
    });
    group.bench_function(BenchmarkId::from_parameter("incremental_recompute"), |b| {
        b.iter(|| incremental_fixture.run_incremental_recompute_step())
    });
    group.finish();

    let mut full_sidecar_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let (_, full_metrics) =
        measure_sidecar_allocations(|| full_sidecar_fixture.run_full_recompute_with_metrics());
    emit_resolved_output_recompute_sidecar(
        "resolved_output_recompute_incremental/full_recompute",
        &full_metrics,
    );

    let mut incremental_sidecar_fixture =
        ResolvedOutputRecomputeIncrementalFixture::new(lines, conflict_blocks);
    let (_, incremental_metrics) = measure_sidecar_allocations(|| {
        incremental_sidecar_fixture.run_incremental_recompute_with_metrics()
    });
    emit_resolved_output_recompute_sidecar(
        "resolved_output_recompute_incremental/incremental_recompute",
        &incremental_metrics,
    );
}

fn emit_resolved_output_recompute_sidecar(bench: &str, metrics: &ResolvedOutputRecomputeMetrics) {
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

fn emit_scrollbar_drag_step_sidecar(bench: &str, metrics: &ScrollbarDragStepMetrics) {
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

fn bench_scrollbar_drag_step(c: &mut Criterion) {
    let mut group = c.benchmark_group("scrollbar_drag_step");
    group.sample_size(100);
    group.warm_up_time(Duration::from_millis(500));

    group.bench_function("window_200", |b| {
        let mut fixture = ScrollbarDragStepFixture::window_200();
        b.iter(|| fixture.run())
    });

    // Emit sidecar from a final run.
    let mut fixture = ScrollbarDragStepFixture::window_200();
    let (_, metrics) = measure_sidecar_allocations(|| fixture.run_with_metrics());
    emit_scrollbar_drag_step_sidecar("scrollbar_drag_step/window_200", &metrics);

    group.finish();
}

fn emit_commit_search_filter_sidecar(case_name: &str, metrics: &CommitSearchFilterMetrics) {
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

fn emit_file_fuzzy_find_sidecar(case_name: &str, metrics: &FileFuzzyFindMetrics) {
    let mut payload = Map::new();
    payload.insert("total_files".to_string(), json!(metrics.total_files));
    payload.insert("query_len".to_string(), json!(metrics.query_len));
    payload.insert("matches_found".to_string(), json!(metrics.matches_found));
    payload.insert("prior_matches".to_string(), json!(metrics.prior_matches));
    payload.insert("files_scanned".to_string(), json!(metrics.files_scanned));
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

fn emit_in_diff_text_search_sidecar(case_name: &str, metrics: &InDiffTextSearchMetrics) {
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

fn emit_file_preview_text_search_sidecar(case_name: &str, metrics: &FilePreviewTextSearchMetrics) {
    let mut payload = Map::new();
    payload.insert("total_lines".to_string(), json!(metrics.total_lines));
    payload.insert("source_bytes".to_string(), json!(metrics.source_bytes));
    payload.insert("query_len".to_string(), json!(metrics.query_len));
    payload.insert("matches_found".to_string(), json!(metrics.matches_found));
    payload.insert("prior_matches".to_string(), json!(metrics.prior_matches));
    emit_sidecar_metrics(&format!("search/{case_name}"), payload);
}

fn emit_file_diff_ctrl_f_open_type_sidecar(
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

fn bench_search(c: &mut Criterion) {
    let commits = env_usize("GITCOMET_BENCH_SEARCH_COMMITS", 50_000);
    let diff_lines = env_usize("GITCOMET_BENCH_SEARCH_DIFF_LINES", 100_000);
    let file_preview_lines = env_usize("GITCOMET_BENCH_SEARCH_FILE_PREVIEW_LINES", 100_000);
    let file_diff_lines = env_usize("GITCOMET_BENCH_SEARCH_FILE_DIFF_LINES", 100_000);
    let file_diff_window = env_usize("GITCOMET_BENCH_SEARCH_FILE_DIFF_WINDOW", 200);
    let fuzzy_files = env_usize("GITCOMET_BENCH_SEARCH_FUZZY_FILES", 100_000);

    let fixture = CommitSearchFilterFixture::new(commits);
    let diff_fixture = InDiffTextSearchFixture::new(diff_lines);
    let file_preview_fixture = FilePreviewTextSearchFixture::new(file_preview_lines);
    let file_diff_fixture = FileDiffCtrlFOpenTypeFixture::new(file_diff_lines, file_diff_window);
    let fuzzy_fixture = FileFuzzyFindFixture::new(fuzzy_files);

    // Author query: "Alice" matches ~10% of commits (1 of 10 first names).
    let author_query =
        env_string("GITCOMET_BENCH_SEARCH_AUTHOR_QUERY").unwrap_or_else(|| "Alice".to_string());
    // Message query: "fix" matches ~10% of commits (1 of 10 prefixes).
    let message_query =
        env_string("GITCOMET_BENCH_SEARCH_MESSAGE_QUERY").unwrap_or_else(|| "fix".to_string());
    // Diff query: `render_cache` matches context rows and modified rows; the
    // refined query narrows to the hot-path subset of modified rows.
    let diff_query = env_string("GITCOMET_BENCH_SEARCH_DIFF_QUERY")
        .unwrap_or_else(|| "render_cache".to_string());
    let diff_refined_query = env_string("GITCOMET_BENCH_SEARCH_DIFF_REFINED_QUERY")
        .unwrap_or_else(|| "render_cache_hot_path".to_string());
    let diff_refinement_matches = diff_fixture.prepare_matches(&diff_query);
    let file_preview_query = env_string("GITCOMET_BENCH_SEARCH_FILE_PREVIEW_QUERY")
        .unwrap_or_else(|| "render_cache".to_string());
    let file_diff_query = env_string("GITCOMET_BENCH_SEARCH_FILE_DIFF_QUERY")
        .unwrap_or_else(|| "render_cache_hot_path".to_string());
    // Fuzzy file-find query: "dcrs" is a realistic 4-char subsequence that
    // matches paths containing d…c…r…s (e.g. "diff_cache…rs"). The incremental
    // keystroke benchmark types "dc" first then extends to "dcrs".
    let fuzzy_query =
        env_string("GITCOMET_BENCH_SEARCH_FUZZY_QUERY").unwrap_or_else(|| "dcrs".to_string());
    let fuzzy_short_query =
        env_string("GITCOMET_BENCH_SEARCH_FUZZY_SHORT_QUERY").unwrap_or_else(|| "dc".to_string());

    let mut group = c.benchmark_group("search");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("commit_filter_by_author_50k_commits"),
        |b| {
            b.iter(|| fixture.run_filter_by_author(&author_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("commit_filter_by_message_50k_commits"),
        |b| {
            b.iter(|| fixture.run_filter_by_message(&message_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("in_diff_text_search_100k_lines"),
        |b| {
            b.iter(|| diff_fixture.run_search(&diff_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("in_diff_text_search_incremental_refinement"),
        |b| {
            b.iter(|| {
                diff_fixture
                    .run_refinement_from_matches(&diff_refined_query, &diff_refinement_matches)
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_preview_text_search_100k_lines"),
        |b| {
            b.iter(|| file_preview_fixture.run_search(&file_preview_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_diff_ctrl_f_open_and_type_100k_lines"),
        |b| {
            b.iter(|| file_diff_fixture.run_open_and_type(&file_diff_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_fuzzy_find_100k_files"),
        |b| {
            b.iter(|| fuzzy_fixture.run_find(&fuzzy_query));
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("file_fuzzy_find_incremental_keystroke"),
        |b| {
            b.iter(|| fuzzy_fixture.run_incremental(&fuzzy_short_query, &fuzzy_query));
        },
    );

    // Emit sidecar metrics from a final run.
    let (_, author_metrics) =
        measure_sidecar_allocations(|| fixture.run_filter_by_author_with_metrics(&author_query));
    emit_commit_search_filter_sidecar("commit_filter_by_author_50k_commits", &author_metrics);
    let (_, message_metrics) =
        measure_sidecar_allocations(|| fixture.run_filter_by_message_with_metrics(&message_query));
    emit_commit_search_filter_sidecar("commit_filter_by_message_50k_commits", &message_metrics);
    let (_, diff_metrics) =
        measure_sidecar_allocations(|| diff_fixture.run_search_with_metrics(&diff_query));
    emit_in_diff_text_search_sidecar("in_diff_text_search_100k_lines", &diff_metrics);
    let (_, refinement_metrics) = measure_sidecar_allocations(|| {
        diff_fixture.run_refinement_with_metrics(&diff_query, &diff_refined_query)
    });
    emit_in_diff_text_search_sidecar(
        "in_diff_text_search_incremental_refinement",
        &refinement_metrics,
    );
    let (_, file_preview_metrics) = measure_sidecar_allocations(|| {
        file_preview_fixture.run_search_with_metrics(&file_preview_query)
    });
    emit_file_preview_text_search_sidecar(
        "file_preview_text_search_100k_lines",
        &file_preview_metrics,
    );
    let (_, file_diff_ctrl_f_metrics) = measure_sidecar_allocations(|| {
        file_diff_fixture.run_open_and_type_with_metrics(&file_diff_query)
    });
    emit_file_diff_ctrl_f_open_type_sidecar(
        "file_diff_ctrl_f_open_and_type_100k_lines",
        &file_diff_ctrl_f_metrics,
    );
    let (_, fuzzy_metrics) =
        measure_sidecar_allocations(|| fuzzy_fixture.run_find_with_metrics(&fuzzy_query));
    emit_file_fuzzy_find_sidecar("file_fuzzy_find_100k_files", &fuzzy_metrics);
    let (_, fuzzy_incr_metrics) = measure_sidecar_allocations(|| {
        fuzzy_fixture.run_incremental_with_metrics(&fuzzy_short_query, &fuzzy_query)
    });
    emit_file_fuzzy_find_sidecar("file_fuzzy_find_incremental_keystroke", &fuzzy_incr_metrics);

    group.finish();
}

fn bench_fs_event(c: &mut Criterion) {
    let tracked_files = env_usize("GITCOMET_BENCH_FS_EVENT_TRACKED_FILES", 1_000);
    let checkout_files = env_usize("GITCOMET_BENCH_FS_EVENT_CHECKOUT_FILES", 200);
    let rapid_save_count = env_usize("GITCOMET_BENCH_FS_EVENT_RAPID_SAVES", 50);
    let churn_files = env_usize("GITCOMET_BENCH_FS_EVENT_CHURN_FILES", 100);

    let single_save = FsEventFixture::single_file_save(tracked_files);
    let checkout_batch = FsEventFixture::git_checkout_batch(tracked_files, checkout_files);
    let rapid_saves = FsEventFixture::rapid_saves_debounce(tracked_files, rapid_save_count);
    let false_positive = FsEventFixture::false_positive_under_churn(tracked_files, churn_files);

    let mut group = c.benchmark_group("fs_event");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("single_file_save_to_status_update"),
        |b| b.iter(|| single_save.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("git_checkout_200_files_to_status_update"),
        |b| b.iter(|| checkout_batch.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("rapid_saves_debounce_coalesce"),
        |b| b.iter(|| rapid_saves.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("false_positive_rate_under_churn"),
        |b| b.iter(|| false_positive.run()),
    );
    group.finish();

    // Emit sidecar metrics from a final run.
    let (_, single_save_metrics) = measure_sidecar_allocations(|| single_save.run_with_metrics());
    emit_fs_event_sidecar("single_file_save_to_status_update", &single_save_metrics);
    let (_, checkout_metrics) = measure_sidecar_allocations(|| checkout_batch.run_with_metrics());
    emit_fs_event_sidecar("git_checkout_200_files_to_status_update", &checkout_metrics);
    let (_, rapid_metrics) = measure_sidecar_allocations(|| rapid_saves.run_with_metrics());
    emit_fs_event_sidecar("rapid_saves_debounce_coalesce", &rapid_metrics);
    let (_, fp_metrics) = measure_sidecar_allocations(|| false_positive.run_with_metrics());
    emit_fs_event_sidecar("false_positive_rate_under_churn", &fp_metrics);
}

fn emit_fs_event_sidecar(scenario: &str, metrics: &FsEventMetrics) {
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

fn bench_network(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_COMMITS", 50_000);
    let history_local_branches = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_LOCAL_BRANCHES", 400);
    let history_remote_branches =
        env_usize("GITCOMET_BENCH_NETWORK_HISTORY_REMOTE_BRANCHES", 1_200);
    let history_window = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_WINDOW", 120);
    let history_scroll_step = env_usize("GITCOMET_BENCH_NETWORK_HISTORY_SCROLL_STEP", 24);
    let ui_frames = env_usize("GITCOMET_BENCH_NETWORK_UI_FRAMES", 240);
    let progress_updates = env_usize("GITCOMET_BENCH_NETWORK_PROGRESS_UPDATES", 360);
    let cancel_after_updates = env_usize("GITCOMET_BENCH_NETWORK_CANCEL_AFTER_UPDATES", 64);
    let cancel_drain_events = env_usize("GITCOMET_BENCH_NETWORK_CANCEL_DRAIN_EVENTS", 4);
    let cancel_total_updates = env_usize("GITCOMET_BENCH_NETWORK_CANCEL_TOTAL_UPDATES", 160);
    let line_bytes = env_usize("GITCOMET_BENCH_NETWORK_PROGRESS_LINE_BYTES", 72);
    let bar_width = env_usize("GITCOMET_BENCH_NETWORK_BAR_WIDTH", 32);
    let frame_budget_ns = u64::try_from(env_usize(
        "GITCOMET_BENCH_NETWORK_FRAME_BUDGET_NS",
        16_666_667,
    ))
    .unwrap_or(u64::MAX);

    let ui_fixture = NetworkFixture::ui_responsiveness_during_fetch(
        history_commits,
        history_local_branches,
        history_remote_branches,
        history_window,
        history_scroll_step,
        ui_frames,
        line_bytes,
        bar_width,
        frame_budget_ns,
    );
    let progress_fixture = NetworkFixture::progress_bar_update_render_cost(
        progress_updates,
        line_bytes,
        bar_width,
        frame_budget_ns,
    );
    let cancel_fixture = NetworkFixture::cancel_operation_latency(
        cancel_after_updates,
        cancel_drain_events,
        cancel_total_updates,
        line_bytes,
        bar_width,
        frame_budget_ns,
    );

    let mut group = c.benchmark_group("network");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("ui_responsiveness_during_fetch"),
        |b| b.iter(|| ui_fixture.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("progress_bar_update_render_cost"),
        |b| b.iter(|| progress_fixture.run()),
    );
    group.bench_function(
        BenchmarkId::from_parameter("cancel_operation_latency"),
        |b| b.iter(|| cancel_fixture.run()),
    );

    group.finish();

    let (_, ui_stats, ui_metrics) = measure_sidecar_allocations(|| ui_fixture.run_with_metrics());
    emit_network_sidecar("ui_responsiveness_during_fetch", &ui_stats, &ui_metrics);

    let (_, progress_stats, progress_metrics) =
        measure_sidecar_allocations(|| progress_fixture.run_with_metrics());
    emit_network_sidecar(
        "progress_bar_update_render_cost",
        &progress_stats,
        &progress_metrics,
    );

    let (_, cancel_stats, cancel_metrics) =
        measure_sidecar_allocations(|| cancel_fixture.run_with_metrics());
    emit_network_sidecar("cancel_operation_latency", &cancel_stats, &cancel_metrics);
}

fn emit_network_sidecar(case_name: &str, stats: &FrameTimingStats, metrics: &NetworkMetrics) {
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

fn bench_clipboard(c: &mut Criterion) {
    let copy_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_COPY_LINES", 10_000);
    let paste_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_PASTE_LINES", 2_000);
    let paste_line_bytes = env_usize("GITCOMET_BENCH_CLIPBOARD_PASTE_LINE_BYTES", 96);
    let select_total_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_SELECT_TOTAL_LINES", 10_000);
    let select_range_lines = env_usize("GITCOMET_BENCH_CLIPBOARD_SELECT_RANGE_LINES", 5_000);

    let copy_fixture = ClipboardFixture::copy_from_diff(copy_lines);
    let paste_fixture = ClipboardFixture::paste_into_commit_message(paste_lines, paste_line_bytes);
    let select_fixture =
        ClipboardFixture::select_range_in_diff(select_total_lines, select_range_lines);

    let mut group = c.benchmark_group("clipboard");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("copy_10k_lines_from_diff"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = copy_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) = measure_sidecar_allocations(|| copy_fixture.run_with_metrics());
                emit_clipboard_sidecar("copy_10k_lines_from_diff", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("paste_large_text_into_commit_message"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = paste_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) = measure_sidecar_allocations(|| paste_fixture.run_with_metrics());
                emit_clipboard_sidecar("paste_large_text_into_commit_message", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(
        BenchmarkId::from_parameter("select_range_5k_lines_in_diff"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = select_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) =
                    measure_sidecar_allocations(|| select_fixture.run_with_metrics());
                emit_clipboard_sidecar("select_range_5k_lines_in_diff", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}

fn emit_clipboard_sidecar(case_name: &str, metrics: &ClipboardMetrics) {
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

fn bench_display(c: &mut Criterion) {
    let history_commits = env_usize("GITCOMET_BENCH_DISPLAY_HISTORY_COMMITS", 10_000);
    let local_branches = env_usize("GITCOMET_BENCH_DISPLAY_LOCAL_BRANCHES", 100);
    let remote_branches = env_usize("GITCOMET_BENCH_DISPLAY_REMOTE_BRANCHES", 400);
    let diff_lines = env_usize("GITCOMET_BENCH_DISPLAY_DIFF_LINES", 5_000);
    let history_window = env_usize("GITCOMET_BENCH_DISPLAY_HISTORY_WINDOW", 120);
    let diff_window = env_usize("GITCOMET_BENCH_DISPLAY_DIFF_WINDOW", 200);
    let base_width = 1920.0f32;
    let sidebar_w = 280.0f32;
    let details_w = 420.0f32;

    let scale_fixture = DisplayFixture::render_cost_by_scale(
        history_commits,
        local_branches,
        remote_branches,
        diff_lines,
        history_window,
        diff_window,
        base_width,
        sidebar_w,
        details_w,
    );
    let two_win_fixture = DisplayFixture::two_windows_same_repo(
        history_commits,
        local_branches,
        remote_branches,
        diff_lines,
        history_window,
        diff_window,
        base_width,
        sidebar_w,
        details_w,
    );
    let dpi_move_fixture = DisplayFixture::window_move_between_dpis(
        history_commits,
        local_branches,
        remote_branches,
        diff_lines,
        history_window,
        diff_window,
        base_width,
        sidebar_w,
        details_w,
    );

    let mut group = c.benchmark_group("display");
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));

    group.bench_function(
        BenchmarkId::from_parameter("render_cost_1x_vs_2x_vs_3x_scale"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = scale_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) = measure_sidecar_allocations(|| scale_fixture.run_with_metrics());
                emit_display_sidecar("render_cost_1x_vs_2x_vs_3x_scale", &metrics);
                elapsed
            });
        },
    );

    group.bench_function(BenchmarkId::from_parameter("two_windows_same_repo"), |b| {
        b.iter_custom(|iters| {
            let mut elapsed = Duration::ZERO;
            for _ in 0..iters {
                let started_at = Instant::now();
                let _ = two_win_fixture.run_with_metrics();
                elapsed += started_at.elapsed();
            }
            let (_, metrics) = measure_sidecar_allocations(|| two_win_fixture.run_with_metrics());
            emit_display_sidecar("two_windows_same_repo", &metrics);
            elapsed
        });
    });

    group.bench_function(
        BenchmarkId::from_parameter("window_move_between_dpis"),
        |b| {
            b.iter_custom(|iters| {
                let mut elapsed = Duration::ZERO;
                for _ in 0..iters {
                    let started_at = Instant::now();
                    let _ = dpi_move_fixture.run_with_metrics();
                    elapsed += started_at.elapsed();
                }
                let (_, metrics) =
                    measure_sidecar_allocations(|| dpi_move_fixture.run_with_metrics());
                emit_display_sidecar("window_move_between_dpis", &metrics);
                elapsed
            });
        },
    );

    group.finish();
}

fn emit_display_sidecar(case_name: &str, metrics: &DisplayMetrics) {
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

fn bench_real_repo(c: &mut Criterion) {
    let Some(snapshot_root) = env_string("GITCOMET_PERF_REAL_REPO_ROOT") else {
        if !env_flag(SUPPRESS_MISSING_REAL_REPO_NOTICE_ENV) {
            eprintln!("skipping real_repo benchmarks: GITCOMET_PERF_REAL_REPO_ROOT is not set");
        }
        return;
    };

    let mut group = c.benchmark_group("real_repo");
    group.sample_size(10);

    let monorepo = RealRepoFixture::from_snapshot_root(
        &snapshot_root,
        RealRepoScenario::MonorepoOpenAndHistoryLoad,
    )
    .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("monorepo_open_and_history_load", |b| {
        b.iter(|| monorepo.run())
    });
    let (_, monorepo_metrics) = measure_sidecar_allocations(|| monorepo.run_with_metrics());
    emit_real_repo_sidecar("monorepo_open_and_history_load", &monorepo_metrics);

    let deep_history = RealRepoFixture::from_snapshot_root(
        &snapshot_root,
        RealRepoScenario::DeepHistoryOpenAndScroll,
    )
    .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("deep_history_open_and_scroll", |b| {
        b.iter(|| deep_history.run())
    });
    let (_, deep_history_metrics) = measure_sidecar_allocations(|| deep_history.run_with_metrics());
    emit_real_repo_sidecar("deep_history_open_and_scroll", &deep_history_metrics);

    let conflict = RealRepoFixture::from_snapshot_root(
        &snapshot_root,
        RealRepoScenario::MidMergeConflictListAndOpen,
    )
    .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("mid_merge_conflict_list_and_open", |b| {
        b.iter(|| conflict.run())
    });
    let (_, conflict_metrics) = measure_sidecar_allocations(|| conflict.run_with_metrics());
    emit_real_repo_sidecar("mid_merge_conflict_list_and_open", &conflict_metrics);

    let large_diff =
        RealRepoFixture::from_snapshot_root(&snapshot_root, RealRepoScenario::LargeFileDiffOpen)
            .unwrap_or_else(|err| panic!("{err}"));
    group.bench_function("large_file_diff_open", |b| b.iter(|| large_diff.run()));
    let (_, large_diff_metrics) = measure_sidecar_allocations(|| large_diff.run_with_metrics());
    emit_real_repo_sidecar("large_file_diff_open", &large_diff_metrics);

    group.finish();
}

fn emit_real_repo_sidecar(case_name: &str, metrics: &RealRepoMetrics) {
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

criterion_group! {
    name = benches;
    config = benchmark_criterion();
    targets =
        bench_open_repo,
        bench_branch_sidebar,
        bench_branch_sidebar_extreme_scale,
        bench_branch_sidebar_cache,
        bench_history_graph,
        bench_history_cache_build,
        bench_history_cache_build_extreme_scale,
        bench_history_load_more_append,
        bench_history_scope_switch,
        bench_repo_switch,
        bench_commit_details,
        bench_status_list,
        bench_status_multi_select,
        bench_status_select_diff_open,
        bench_merge_open_bootstrap,
        bench_frame_timing,
        bench_keyboard,
        bench_staging,
        bench_undo_redo,
        bench_git_ops,
        bench_large_file_diff_scroll,
        bench_file_diff_replacement_alignment,
        bench_text_input_prepaint_windowed,
        bench_text_input_runs_streamed_highlight,
        bench_text_input_long_line_cap,
        bench_text_input_wrap_incremental_tabs,
        bench_text_input_wrap_incremental_burst_edits,
        bench_text_model_snapshot_clone_cost,
        bench_text_model_bulk_load_large,
        bench_text_model_fragmented_edits,
        bench_file_diff_syntax_prepare,
        bench_file_diff_syntax_query_stress,
        bench_file_diff_syntax_reparse,
        bench_file_diff_inline_syntax_projection,
        bench_file_diff_syntax_cache_drop,
        bench_prepared_syntax_multidoc_cache_hit_rate,
        bench_prepared_syntax_chunk_miss_cost,
        bench_large_html_syntax,
        bench_worktree_preview_render,
        bench_markdown_preview_parse_build,
        bench_markdown_preview_render,
        bench_diff_open_markdown_preview_first_window,
        bench_diff_open_image_preview_first_paint,
        bench_diff_open_svg_dual_path_first_window,
        bench_conflict_three_way_scroll,
        bench_conflict_three_way_prepared_syntax_scroll,
        bench_conflict_three_way_visible_map_build,
        bench_conflict_two_way_split_scroll,
        bench_conflict_load_duplication,
        bench_conflict_two_way_diff_build,
        bench_conflict_two_way_word_highlights,
        bench_conflict_resolved_output_gutter_scroll,
        bench_conflict_search_query_update,
        bench_patch_diff_search_query_update,
        bench_patch_diff_paged_rows,
        bench_diff_open_patch_first_window,
        bench_diff_open_file_split_first_window,
        bench_diff_open_file_inline_first_window,
        bench_diff_open_patch_deep_window,
        bench_diff_open_patch_100k_lines_first_window,
        bench_diff_open_conflict_compare_first_window,
        bench_diff_refresh_rev_only_same_content,
        bench_conflict_split_resize_step,
        bench_conflict_streamed_provider,
        bench_conflict_streamed_resolved_output,
        bench_resolved_output_recompute_incremental,
        bench_pane_resize_drag_step,
        bench_diff_split_resize_drag_step,
        bench_window_resize_layout,
        bench_window_resize_layout_extreme_scale,
        bench_history_column_resize_drag_step,
        bench_repo_tab_drag,
        bench_scrollbar_drag_step,
        bench_search,
        bench_fs_event,
        bench_network,
        bench_clipboard,
        bench_display,
        bench_real_repo
}
criterion_main!(benches);
