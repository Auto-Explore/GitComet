use super::diff_text::{
    DiffSyntaxBudget, DiffSyntaxLanguage, DiffSyntaxMode, PrepareDiffSyntaxDocumentResult,
    diff_syntax_language_for_path, inject_background_prepared_diff_syntax_document,
    prepare_diff_syntax_document_with_budget_reuse_text,
};
use super::*;
use crate::kit::text_model::TextModel;
use crate::kit::{
    benchmark_text_input_runs_legacy_visible_window,
    benchmark_text_input_runs_streamed_visible_window,
};
use crate::theme::AppTheme;
use crate::view::branch_sidebar::{branch_sidebar_branch_label, branch_sidebar_source_fingerprint};
use crate::view::caches::{
    BranchSidebarCache, BranchSidebarFingerprint, HistoryShortShaVm, HistoryWhenVm,
    branch_sidebar_cache_lookup, branch_sidebar_cache_lookup_by_source, branch_sidebar_cache_store,
    build_history_branch_text_by_target, build_history_tag_names_by_target,
};
use crate::view::history_graph;
use crate::view::mod_helpers::{
    PaneResizeHandle, PaneResizeState, StatusMultiSelection, StatusSection,
};
use crate::view::panes::main::{
    AsciiCaseInsensitiveNeedle, DiffSearchQueryReuse,
    diff_cache::{
        PagedFileDiffRows, PagedPatchDiffRows, PagedPatchSplitRows, PatchInlineVisibleMap,
    },
    diff_search_query_reuse,
};
use crate::view::path_display;
use crate::view::rows::status::{
    apply_status_multi_selection_click, bench_reset_status_selection,
    bench_snapshot_status_selection,
};
use crate::view::{next_pane_resize_drag_width, pane_resize_drag_width_bounds};
use gitcomet_core::domain::DiffLineKind;
use gitcomet_core::domain::{
    Branch, Commit, CommitDetails, CommitFileChange, CommitId, Diff, DiffArea, DiffLine,
    DiffRowProvider, DiffTarget, FileDiffText, FileStatus, FileStatusKind, LogCursor, LogPage,
    LogScope, Remote, RemoteBranch, RepoSpec, RepoStatus, StashEntry, Submodule, SubmoduleStatus,
    Tag, Upstream, UpstreamDivergence, Worktree,
};
use gitcomet_core::git_ops_trace::{self, GitOpTraceSnapshot};
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use gitcomet_state::benchmarks::{dispatch_sync, set_active_repo_sync};
use gitcomet_state::model::{AppState, ConflictFile, Loadable, RepoId, RepoState};
use gitcomet_state::msg::{Effect, InternalMsg, Msg};
use rustc_hash::FxHasher;
use std::cell::RefCell;
use std::fmt::Write as _;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::ops::Range;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tempfile::TempDir;

mod conflict;
mod real_repo;
mod syntax;

pub use conflict::*;
pub use real_repo::*;
pub use syntax::*;

// Re-export frame timing capture from view::perf for use in benchmark harnesses.
#[cfg(feature = "benchmarks")]
pub use crate::view::perf::{FrameTimingCapture, FrameTimingStats};

#[cfg(test)]
mod tests;

pub struct OpenRepoFixture {
    repo: RepoState,
    commits: Vec<Commit>,
    theme: AppTheme,
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    worktrees: usize,
    submodules: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct OpenRepoMetrics {
    pub commit_count: u64,
    pub local_branches: u64,
    pub remote_branches: u64,
    pub remotes: u64,
    pub worktrees: u64,
    pub submodules: u64,
    pub sidebar_rows: u64,
    pub graph_rows: u64,
    pub max_graph_lanes: u64,
}

impl OpenRepoFixture {
    pub fn new(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        Self::with_sidebar_fanout(commits, local_branches, remote_branches, remotes, 0, 0)
    }

    pub fn with_sidebar_fanout(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
        worktrees: usize,
        submodules: usize,
    ) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let commits_vec = build_synthetic_commits(commits);
        let repo = build_synthetic_repo_state(
            local_branches,
            remote_branches,
            remotes,
            worktrees,
            submodules,
            0,
            &commits_vec,
        );
        Self {
            repo,
            commits: commits_vec,
            theme,
            local_branches,
            remote_branches,
            remotes,
            worktrees,
            submodules,
        }
    }

    pub fn run(&self) -> u64 {
        #[cfg(any(test, feature = "benchmarks"))]
        {
            self.run_with_metrics().0
        }

        #[cfg(not(any(test, feature = "benchmarks")))]
        {
            // Branch sidebar is the main "many branches" transformation.
            let rows = GitCometView::branch_sidebar_rows(&self.repo);

            // History graph is the main "long history" transformation.
            let graph =
                history_graph::compute_graph(&self.commits, self.theme, std::iter::empty(), None);

            let mut h = FxHasher::default();
            rows.len().hash(&mut h);
            graph.len().hash(&mut h);
            graph
                .iter()
                .take(128)
                .map(|r| (r.lanes_now.len(), r.lanes_next.len(), r.is_merge))
                .collect::<Vec<_>>()
                .hash(&mut h);
            h.finish()
        }
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, OpenRepoMetrics) {
        // Branch sidebar is the main "many branches" transformation.
        let rows = GitCometView::branch_sidebar_rows(&self.repo);

        // History graph is the main "long history" transformation.
        let graph =
            history_graph::compute_graph(&self.commits, self.theme, std::iter::empty(), None);

        let mut h = FxHasher::default();
        rows.len().hash(&mut h);
        graph.len().hash(&mut h);
        graph
            .iter()
            .take(128)
            .map(|r| (r.lanes_now.len(), r.lanes_next.len(), r.is_merge))
            .collect::<Vec<_>>()
            .hash(&mut h);

        let max_graph_lanes = graph
            .iter()
            .map(|row| row.lanes_now.len().max(row.lanes_next.len()))
            .max()
            .unwrap_or_default();

        (
            h.finish(),
            OpenRepoMetrics {
                commit_count: u64::try_from(self.commits.len()).unwrap_or(u64::MAX),
                local_branches: u64::try_from(self.local_branches).unwrap_or(u64::MAX),
                remote_branches: u64::try_from(self.remote_branches).unwrap_or(u64::MAX),
                remotes: u64::try_from(self.remotes).unwrap_or(u64::MAX),
                worktrees: u64::try_from(self.worktrees).unwrap_or(u64::MAX),
                submodules: u64::try_from(self.submodules).unwrap_or(u64::MAX),
                sidebar_rows: u64::try_from(rows.len()).unwrap_or(u64::MAX),
                graph_rows: u64::try_from(graph.len()).unwrap_or(u64::MAX),
                max_graph_lanes: u64::try_from(max_graph_lanes).unwrap_or(u64::MAX),
            },
        )
    }
}

pub struct BranchSidebarFixture {
    repo: RepoState,
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    worktrees: usize,
    submodules: usize,
    stashes: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct BranchSidebarMetrics {
    pub local_branches: u64,
    pub remote_branches: u64,
    pub remotes: u64,
    pub worktrees: u64,
    pub submodules: u64,
    pub stashes: u64,
    pub sidebar_rows: u64,
    pub branch_rows: u64,
    pub remote_headers: u64,
    pub group_headers: u64,
    pub max_branch_depth: u64,
}

fn hash_branch_sidebar_rows(rows: &[BranchSidebarRow]) -> u64 {
    let mut h = FxHasher::default();
    rows.len().hash(&mut h);
    for row in rows.iter().take(256) {
        std::mem::discriminant(row).hash(&mut h);
        match row {
            BranchSidebarRow::SectionHeader {
                section,
                top_border,
                collapsed,
                ..
            } => {
                match section {
                    BranchSection::Local => 0u8,
                    BranchSection::Remote => 1u8,
                }
                .hash(&mut h);
                top_border.hash(&mut h);
                collapsed.hash(&mut h);
            }
            BranchSidebarRow::Placeholder { section, message } => {
                match section {
                    BranchSection::Local => 0u8,
                    BranchSection::Remote => 1u8,
                }
                .hash(&mut h);
                message.len().hash(&mut h);
            }
            BranchSidebarRow::RemoteHeader {
                name, collapsed, ..
            } => {
                name.len().hash(&mut h);
                collapsed.hash(&mut h);
            }
            BranchSidebarRow::GroupHeader {
                label,
                section,
                depth,
                collapsed,
                ..
            } => {
                match section {
                    BranchSection::Local => 0u8,
                    BranchSection::Remote => 1u8,
                }
                .hash(&mut h);
                label.len().hash(&mut h);
                depth.hash(&mut h);
                collapsed.hash(&mut h);
            }
            BranchSidebarRow::Branch {
                name,
                depth,
                muted,
                is_head,
                is_upstream,
                ..
            } => {
                branch_sidebar_branch_label(name.as_ref())
                    .len()
                    .hash(&mut h);
                name.len().hash(&mut h);
                depth.hash(&mut h);
                muted.hash(&mut h);
                is_head.hash(&mut h);
                is_upstream.hash(&mut h);
            }
            BranchSidebarRow::WorktreeItem {
                path,
                branch,
                detached,
                is_active,
            } => {
                let path_len = path
                    .to_str()
                    .map_or_else(|| path.to_string_lossy().len(), str::len);
                let label_len = branch.as_ref().map_or_else(
                    || {
                        if *detached {
                            "(detached)  ".len() + path_len
                        } else {
                            path_len
                        }
                    },
                    |branch| branch.len() + "  ".len() + path_len,
                );
                label_len.hash(&mut h);
                label_len.hash(&mut h);
                is_active.hash(&mut h);
            }
            BranchSidebarRow::SubmoduleItem { path } => {
                let path_len = path
                    .to_str()
                    .map_or_else(|| path.to_string_lossy().len(), str::len);
                path_len.hash(&mut h);
                path_len.hash(&mut h);
            }
            BranchSidebarRow::StashItem {
                index,
                message,
                tooltip,
                ..
            } => {
                index.hash(&mut h);
                message.len().hash(&mut h);
                tooltip.len().hash(&mut h);
            }
            BranchSidebarRow::SectionSpacer
            | BranchSidebarRow::WorktreesHeader { .. }
            | BranchSidebarRow::WorktreePlaceholder { .. }
            | BranchSidebarRow::SubmodulesHeader { .. }
            | BranchSidebarRow::SubmodulePlaceholder { .. }
            | BranchSidebarRow::StashHeader { .. }
            | BranchSidebarRow::StashPlaceholder { .. } => {}
        }
    }
    h.finish()
}

impl BranchSidebarFixture {
    pub fn new(
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
        worktrees: usize,
        submodules: usize,
        stashes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(1);
        let repo = build_synthetic_repo_state(
            local_branches,
            remote_branches,
            remotes,
            worktrees,
            submodules,
            stashes,
            &commits,
        );
        Self {
            repo,
            local_branches,
            remote_branches,
            remotes,
            worktrees,
            submodules,
            stashes,
        }
    }

    pub fn twenty_thousand_branches_hundred_remotes() -> Self {
        Self::new(1, 20_000, 100, 0, 0, 0)
    }

    pub fn run(&self) -> u64 {
        let rows = GitCometView::branch_sidebar_rows(&self.repo);
        hash_branch_sidebar_rows(&rows)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, BranchSidebarMetrics) {
        let rows = GitCometView::branch_sidebar_rows(&self.repo);
        let mut branch_rows = 0u64;
        let mut remote_headers = 0u64;
        let mut group_headers = 0u64;
        let mut max_branch_depth = 0usize;

        for row in &rows {
            match row {
                BranchSidebarRow::RemoteHeader { .. } => {
                    remote_headers = remote_headers.saturating_add(1);
                }
                BranchSidebarRow::GroupHeader { .. } => {
                    group_headers = group_headers.saturating_add(1);
                }
                BranchSidebarRow::Branch { depth, .. } => {
                    branch_rows = branch_rows.saturating_add(1);
                    max_branch_depth = max_branch_depth.max(usize::from(*depth));
                }
                _ => {}
            }
        }

        (
            hash_branch_sidebar_rows(&rows),
            BranchSidebarMetrics {
                local_branches: u64::try_from(self.local_branches).unwrap_or(u64::MAX),
                remote_branches: u64::try_from(self.remote_branches).unwrap_or(u64::MAX),
                remotes: u64::try_from(self.remotes).unwrap_or(u64::MAX),
                worktrees: u64::try_from(self.worktrees).unwrap_or(u64::MAX),
                submodules: u64::try_from(self.submodules).unwrap_or(u64::MAX),
                stashes: u64::try_from(self.stashes).unwrap_or(u64::MAX),
                sidebar_rows: u64::try_from(rows.len()).unwrap_or(u64::MAX),
                branch_rows,
                remote_headers,
                group_headers,
                max_branch_depth: u64::try_from(max_branch_depth).unwrap_or(u64::MAX),
            },
        )
    }

    #[cfg(test)]
    fn row_count(&self) -> usize {
        GitCometView::branch_sidebar_rows(&self.repo).len()
    }
}

// ---------------------------------------------------------------------------
// Branch sidebar cache simulation benchmarks (Phase 1)
// ---------------------------------------------------------------------------

/// Metrics emitted as sidecar JSON for branch sidebar cache benchmarks.
#[derive(Clone, Copy, Debug, Default)]
pub struct BranchSidebarCacheMetrics {
    pub cache_hits: usize,
    pub cache_misses: usize,
    pub rows_count: usize,
    pub invalidations: usize,
}

/// Simulates the `branch_sidebar_rows_cached()` path from `SidebarPaneView`
/// without requiring the full GPUI view context. This lets benchmarks measure
/// the direct fingerprint hit, the row-source-equal reuse path after
/// invalidation, and the full-rebuild path separately.
pub struct BranchSidebarCacheFixture {
    repo: RepoState,
    cache: Option<BranchSidebarCache>,
    metrics: BranchSidebarCacheMetrics,
}

impl BranchSidebarCacheFixture {
    /// Balanced fixture: moderate branch/remote/worktree/stash counts.
    pub fn balanced(
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
        worktrees: usize,
        submodules: usize,
        stashes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(1);
        let repo = build_synthetic_repo_state(
            local_branches,
            remote_branches,
            remotes,
            worktrees,
            submodules,
            stashes,
            &commits,
        );
        Self {
            repo,
            cache: None,
            metrics: BranchSidebarCacheMetrics::default(),
        }
    }

    /// Remote-fanout-heavy fixture for cache miss measurements.
    pub fn remote_fanout(local_branches: usize, remote_branches: usize, remotes: usize) -> Self {
        let commits = build_synthetic_commits(1);
        let repo =
            build_synthetic_repo_state(local_branches, remote_branches, remotes, 0, 0, 0, &commits);
        Self {
            repo,
            cache: None,
            metrics: BranchSidebarCacheMetrics::default(),
        }
    }

    /// Execute the cached path.  On fingerprint match → returns the cached
    /// `Arc` (cache hit).  On mismatch or cold cache → rebuilds rows (cache
    /// miss).  Returns a hash of the row slice for black-boxing.
    pub fn run_cached(&mut self) -> u64 {
        let repo_id = self.repo.id;
        let fingerprint = BranchSidebarFingerprint::from_repo(&self.repo);
        if let Some(cached_rows) =
            branch_sidebar_cache_lookup(&mut self.cache, repo_id, fingerprint)
        {
            self.metrics.cache_hits += 1;
            self.metrics.rows_count = cached_rows.len();
            let mut h = FxHasher::default();
            cached_rows.len().hash(&mut h);
            return h.finish();
        }

        let cached_source_parts = self
            .cache
            .as_ref()
            .filter(|cached| cached.repo_id == repo_id)
            .map(|cached| &cached.source_parts);
        let (source_fingerprint, source_parts) =
            branch_sidebar_source_fingerprint(&self.repo, cached_source_parts);

        if let Some(cached_rows) = branch_sidebar_cache_lookup_by_source(
            &mut self.cache,
            repo_id,
            fingerprint,
            source_fingerprint,
            &source_parts,
        ) {
            self.metrics.cache_hits += 1;
            self.metrics.rows_count = cached_rows.len();
            let mut h = FxHasher::default();
            cached_rows.len().hash(&mut h);
            return h.finish();
        }

        // Cache miss — full rebuild.
        self.metrics.cache_misses += 1;
        let rows: Arc<[BranchSidebarRow]> = GitCometView::branch_sidebar_rows(&self.repo).into();
        self.metrics.rows_count = rows.len();

        let mut h = FxHasher::default();
        rows.len().hash(&mut h);
        for row in rows.iter().take(256) {
            std::mem::discriminant(row).hash(&mut h);
        }
        let hash = h.finish();

        branch_sidebar_cache_store(
            &mut self.cache,
            repo_id,
            fingerprint,
            source_fingerprint,
            source_parts,
            rows,
        );
        hash
    }

    /// Invalidate the cache by bumping one rev counter (simulating a single
    /// ref change) and then rebuild.  Returns row-count hash.
    pub fn run_invalidate_single_ref(&mut self) -> u64 {
        self.repo.branches_rev = self.repo.branches_rev.wrapping_add(1);
        self.repo.branch_sidebar_rev = self.repo.branch_sidebar_rev.wrapping_add(1);
        self.metrics.invalidations += 1;
        self.run_cached()
    }

    /// Invalidate the cache by bumping `worktrees_rev` (simulating worktree
    /// placeholders completing their async load) and then rebuild.  Returns
    /// row-count hash.
    pub fn run_invalidate_worktrees_ready(&mut self) -> u64 {
        self.repo.worktrees_rev = self.repo.worktrees_rev.wrapping_add(1);
        self.repo.branch_sidebar_rev = self.repo.branch_sidebar_rev.wrapping_add(1);
        self.metrics.invalidations += 1;
        self.run_cached()
    }

    /// Reset metrics for a fresh measurement interval.
    pub fn reset_metrics(&mut self) {
        self.metrics = BranchSidebarCacheMetrics::default();
    }

    /// Snapshot the accumulated metrics.
    pub fn metrics(&self) -> BranchSidebarCacheMetrics {
        self.metrics
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RepoSwitchMetrics {
    pub effect_count: usize,
    pub refresh_effect_count: usize,
    pub selected_diff_reload_effect_count: usize,
    pub persist_session_effect_count: usize,
    pub repo_count: usize,
    pub hydrated_repo_count: usize,
    pub selected_commit_repo_count: usize,
    pub selected_diff_repo_count: usize,
}

impl RepoSwitchMetrics {
    fn from_state_and_effects(state: &AppState, effects: &[Effect]) -> Self {
        let mut metrics = Self {
            effect_count: effects.len(),
            repo_count: state.repos.len(),
            hydrated_repo_count: state
                .repos
                .iter()
                .filter(|repo| repo_switch_repo_is_hydrated(repo))
                .count(),
            selected_commit_repo_count: state
                .repos
                .iter()
                .filter(|repo| repo_switch_repo_has_selected_commit(repo))
                .count(),
            selected_diff_repo_count: state
                .repos
                .iter()
                .filter(|repo| repo.diff_state.diff_target.is_some())
                .count(),
            ..Self::default()
        };

        for effect in effects {
            match effect {
                Effect::PersistSession { .. } => {
                    metrics.persist_session_effect_count =
                        metrics.persist_session_effect_count.saturating_add(1);
                }
                Effect::LoadDiff { .. }
                | Effect::LoadDiffFile { .. }
                | Effect::LoadDiffFileImage { .. }
                | Effect::LoadConflictFile { .. } => {
                    metrics.selected_diff_reload_effect_count =
                        metrics.selected_diff_reload_effect_count.saturating_add(1);
                }
                Effect::LoadSelectedDiff {
                    load_file_text,
                    load_file_image,
                    ..
                } => {
                    let extra = usize::from(*load_file_text) + usize::from(*load_file_image);
                    metrics.selected_diff_reload_effect_count = metrics
                        .selected_diff_reload_effect_count
                        .saturating_add(1 + extra);
                }
                Effect::LoadBranches { .. }
                | Effect::LoadRemotes { .. }
                | Effect::LoadRemoteBranches { .. }
                | Effect::LoadStatus { .. }
                | Effect::LoadHeadBranch { .. }
                | Effect::LoadUpstreamDivergence { .. }
                | Effect::LoadLog { .. }
                | Effect::LoadTags { .. }
                | Effect::LoadRemoteTags { .. }
                | Effect::LoadStashes { .. }
                | Effect::LoadRebaseState { .. }
                | Effect::LoadMergeCommitMessage { .. } => {
                    metrics.refresh_effect_count = metrics.refresh_effect_count.saturating_add(1);
                }
                _ => {}
            }
        }

        metrics
    }
}

pub struct RepoSwitchFixture {
    baseline: AppState,
    target_repo_id: RepoId,
}

impl RepoSwitchFixture {
    pub fn refocus_same_repo(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(commits.max(1));
        let repo = build_repo_switch_repo_state(
            RepoId(1),
            "/tmp/bench-repo-switch-refocus",
            &commits,
            local_branches,
            remote_branches,
            remotes,
            1_024,
            Some("src/lib.rs"),
        );

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(1),
        }
    }

    pub fn two_hot_repos(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(commits.max(2));
        let repo1 = build_repo_switch_repo_state(
            RepoId(1),
            "/tmp/bench-repo-switch-alpha",
            &commits,
            local_branches,
            remote_branches,
            remotes,
            1_024,
            Some("src/main.rs"),
        );
        let repo2 = build_repo_switch_repo_state(
            RepoId(2),
            "/tmp/bench-repo-switch-beta",
            &commits,
            local_branches.saturating_add(24),
            remote_branches.saturating_add(96),
            remotes.max(2),
            1_536,
            Some("src/lib.rs"),
        );

        Self {
            baseline: AppState {
                repos: vec![repo1, repo2],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(2),
        }
    }

    pub fn selected_commit_and_details(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(commits.max(2));
        let repo1 = build_repo_switch_repo_state(
            RepoId(1),
            "/tmp/bench-repo-switch-details-alpha",
            &commits,
            local_branches,
            remote_branches,
            remotes,
            1_024,
            None,
        );
        let repo2 = build_repo_switch_repo_state(
            RepoId(2),
            "/tmp/bench-repo-switch-details-beta",
            &commits,
            local_branches.saturating_add(24),
            remote_branches.saturating_add(96),
            remotes.max(2),
            1_536,
            None,
        );

        Self {
            baseline: AppState {
                repos: vec![repo1, repo2],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(2),
        }
    }

    pub fn twenty_tabs(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        const TAB_COUNT: usize = 20;

        let commits = build_synthetic_commits(commits.max(2));
        let mut repos = Vec::with_capacity(TAB_COUNT);
        for ix in 0..TAB_COUNT {
            let repo_id = RepoId(u64::try_from(ix + 1).unwrap_or(u64::MAX));
            let workdir = format!("/tmp/bench-repo-switch-tab-{ix:02}");
            let repo = if ix == 0 || ix + 1 == TAB_COUNT {
                build_repo_switch_repo_state(
                    repo_id,
                    &workdir,
                    &commits,
                    local_branches.saturating_add(ix.saturating_mul(4)),
                    remote_branches.saturating_add(ix.saturating_mul(16)),
                    remotes.max(2),
                    1_024usize.saturating_add(ix.saturating_mul(64)),
                    Some("src/main.rs"),
                )
            } else {
                build_repo_switch_minimal_repo_state(repo_id, &workdir)
            };
            repos.push(repo);
        }

        Self {
            baseline: AppState {
                repos,
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(u64::try_from(TAB_COUNT).unwrap_or(u64::MAX)),
        }
    }

    pub fn twenty_repos_all_hot(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        const REPO_COUNT: usize = 20;

        let commits = build_synthetic_commits(commits.max(2));
        let mut repos = Vec::with_capacity(REPO_COUNT);
        for ix in 0..REPO_COUNT {
            let repo_id = RepoId(u64::try_from(ix + 1).unwrap_or(u64::MAX));
            let workdir = format!("/tmp/bench-repo-switch-hot-{ix:02}");
            let diff_path = match ix % 3 {
                0 => Some("src/main.rs"),
                1 => Some("src/lib.rs"),
                _ => Some("README.md"),
            };
            repos.push(build_repo_switch_repo_state(
                repo_id,
                &workdir,
                &commits,
                local_branches.saturating_add(ix.saturating_mul(3)),
                remote_branches.saturating_add(ix.saturating_mul(24)),
                remotes.max(2).saturating_add(ix / 5),
                1_024usize.saturating_add(ix.saturating_mul(128)),
                diff_path,
            ));
        }

        Self {
            baseline: AppState {
                repos,
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(u64::try_from(REPO_COUNT).unwrap_or(u64::MAX)),
        }
    }

    /// Two repos with fully loaded diff state (diff content + file text cached).
    /// Measures repo-switch cost when a file diff is actively being viewed.
    pub fn selected_diff_file(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(commits.max(2));
        let mut repo1 = build_repo_switch_repo_state(
            RepoId(1),
            "/tmp/bench-repo-switch-diff-alpha",
            &commits,
            local_branches,
            remote_branches,
            remotes,
            1_024,
            Some("src/main.rs"),
        );
        populate_loaded_diff_state(&mut repo1, "src/main.rs", 500);

        let mut repo2 = build_repo_switch_repo_state(
            RepoId(2),
            "/tmp/bench-repo-switch-diff-beta",
            &commits,
            local_branches.saturating_add(24),
            remote_branches.saturating_add(96),
            remotes.max(2),
            1_536,
            Some("src/lib.rs"),
        );
        populate_loaded_diff_state(&mut repo2, "src/lib.rs", 500);

        Self {
            baseline: AppState {
                repos: vec![repo1, repo2],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(2),
        }
    }

    /// Two repos where the diff target points to a conflicted file. The
    /// reducer dispatches `LoadConflictFile` instead of `LoadDiff`+`LoadDiffFile`.
    pub fn selected_conflict_target(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(commits.max(2));
        let mut repo1 = build_repo_switch_repo_state(
            RepoId(1),
            "/tmp/bench-repo-switch-conflict-alpha",
            &commits,
            local_branches,
            remote_branches,
            remotes,
            1_024,
            Some("src/conflict_a.rs"),
        );
        populate_conflict_state(&mut repo1, "src/conflict_a.rs", 200);

        let mut repo2 = build_repo_switch_repo_state(
            RepoId(2),
            "/tmp/bench-repo-switch-conflict-beta",
            &commits,
            local_branches.saturating_add(24),
            remote_branches.saturating_add(96),
            remotes.max(2),
            1_536,
            Some("src/conflict_b.rs"),
        );
        populate_conflict_state(&mut repo2, "src/conflict_b.rs", 200);

        Self {
            baseline: AppState {
                repos: vec![repo1, repo2],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(2),
        }
    }

    /// Two repos where the target has a loaded merge commit message (draft).
    /// Measures the state-transition cost when switching to a repo mid-merge.
    pub fn merge_active_with_draft_restore(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        remotes: usize,
    ) -> Self {
        let commits = build_synthetic_commits(commits.max(2));
        let repo1 = build_repo_switch_repo_state(
            RepoId(1),
            "/tmp/bench-repo-switch-merge-alpha",
            &commits,
            local_branches,
            remote_branches,
            remotes,
            1_024,
            Some("src/main.rs"),
        );

        let mut repo2 = build_repo_switch_repo_state(
            RepoId(2),
            "/tmp/bench-repo-switch-merge-beta",
            &commits,
            local_branches.saturating_add(24),
            remote_branches.saturating_add(96),
            remotes.max(2),
            1_536,
            Some("src/lib.rs"),
        );
        repo2.merge_commit_message = Loadable::Ready(Some(
            "Merge branch 'feature/large-refactor' into main\n\n\
             This merge brings in the large-refactor feature branch which includes:\n\
             - Restructured module hierarchy\n\
             - Updated dependency graph\n\
             - New integration test suite\n\
             - Migrated configuration format"
                .to_string(),
        ));
        repo2.merge_message_rev = 1;

        Self {
            baseline: AppState {
                repos: vec![repo1, repo2],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            target_repo_id: RepoId(2),
        }
    }

    pub fn fresh_state(&self) -> AppState {
        let mut state = self.baseline.clone();
        let now = SystemTime::now();
        for repo in &mut state.repos {
            if repo.last_active_at.is_some() {
                repo.last_active_at = Some(now);
            }
        }
        state
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, RepoSwitchMetrics) {
        let effects = set_active_repo_sync(state, self.target_repo_id);
        let metrics = RepoSwitchMetrics::from_state_and_effects(state, &effects);

        let mut h = FxHasher::default();
        state.active_repo.hash(&mut h);
        effects.len().hash(&mut h);
        metrics.effect_count.hash(&mut h);
        metrics.refresh_effect_count.hash(&mut h);
        metrics.selected_diff_reload_effect_count.hash(&mut h);
        metrics.persist_session_effect_count.hash(&mut h);
        metrics.repo_count.hash(&mut h);
        metrics.hydrated_repo_count.hash(&mut h);
        metrics.selected_commit_repo_count.hash(&mut h);
        metrics.selected_diff_repo_count.hash(&mut h);

        for effect in effects.iter().take(32) {
            std::mem::discriminant(effect).hash(&mut h);
            match effect {
                Effect::LoadDiff { repo_id, target }
                | Effect::LoadDiffFile { repo_id, target }
                | Effect::LoadDiffFileImage { repo_id, target } => {
                    repo_id.0.hash(&mut h);
                    match target {
                        DiffTarget::WorkingTree { path, area } => {
                            path.hash(&mut h);
                            (*area as u8).hash(&mut h);
                        }
                        DiffTarget::Commit { commit_id, path } => {
                            commit_id.hash(&mut h);
                            path.hash(&mut h);
                        }
                    }
                }
                Effect::LoadSelectedDiff {
                    repo_id,
                    target,
                    load_file_text,
                    load_file_image,
                } => {
                    repo_id.0.hash(&mut h);
                    load_file_text.hash(&mut h);
                    load_file_image.hash(&mut h);
                    match target {
                        DiffTarget::WorkingTree { path, area } => {
                            path.hash(&mut h);
                            (*area as u8).hash(&mut h);
                        }
                        DiffTarget::Commit { commit_id, path } => {
                            commit_id.hash(&mut h);
                            path.hash(&mut h);
                        }
                    }
                }
                Effect::LoadLog {
                    repo_id,
                    scope,
                    limit,
                    cursor,
                } => {
                    repo_id.0.hash(&mut h);
                    std::mem::discriminant(scope).hash(&mut h);
                    limit.hash(&mut h);
                    cursor.is_some().hash(&mut h);
                }
                Effect::PersistSession {
                    repo_id, action, ..
                } => {
                    repo_id.hash(&mut h);
                    action.hash(&mut h);
                }
                Effect::LoadStashes { repo_id, limit } => {
                    repo_id.0.hash(&mut h);
                    limit.hash(&mut h);
                }
                Effect::LoadBranches { repo_id }
                | Effect::LoadRemotes { repo_id }
                | Effect::LoadRemoteBranches { repo_id }
                | Effect::LoadStatus { repo_id }
                | Effect::LoadHeadBranch { repo_id }
                | Effect::LoadUpstreamDivergence { repo_id }
                | Effect::LoadTags { repo_id }
                | Effect::LoadRemoteTags { repo_id }
                | Effect::LoadRebaseState { repo_id }
                | Effect::LoadMergeCommitMessage { repo_id } => {
                    repo_id.0.hash(&mut h);
                }
                Effect::LoadConflictFile { repo_id, path, .. } => {
                    repo_id.0.hash(&mut h);
                    path.hash(&mut h);
                }
                _ => {}
            }
        }

        (h.finish(), metrics)
    }

    pub fn run(&self) -> (u64, RepoSwitchMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }
}

pub struct HistoryGraphFixture {
    commits: Vec<Commit>,
    branch_head_indices: Vec<usize>,
    theme: AppTheme,
}

fn repo_switch_repo_is_hydrated(repo: &RepoState) -> bool {
    matches!(repo.open, Loadable::Ready(()))
        && matches!(repo.status, Loadable::Ready(_))
        && matches!(repo.log, Loadable::Ready(_))
        && matches!(repo.history_state.log, Loadable::Ready(_))
        && matches!(repo.branches, Loadable::Ready(_))
        && matches!(repo.remote_tags, Loadable::Ready(_))
        && matches!(repo.remote_branches, Loadable::Ready(_))
        && matches!(repo.remotes, Loadable::Ready(_))
        && matches!(repo.tags, Loadable::Ready(_))
        && matches!(repo.stashes, Loadable::Ready(_))
        && matches!(repo.rebase_in_progress, Loadable::Ready(_))
        && matches!(repo.merge_commit_message, Loadable::Ready(_))
}

fn repo_switch_repo_has_selected_commit(repo: &RepoState) -> bool {
    repo.history_state.selected_commit.is_some()
        && matches!(repo.history_state.commit_details, Loadable::Ready(_))
}

impl HistoryGraphFixture {
    pub fn new(commits: usize, merge_every: usize, branch_head_every: usize) -> Self {
        let commits_vec = build_synthetic_commits_with_merge_stride(commits, merge_every, 40);
        let mut branch_head_indices = Vec::new();
        if branch_head_every > 0 {
            for ix in (0..commits_vec.len()).step_by(branch_head_every) {
                branch_head_indices.push(ix);
            }
        }
        Self {
            commits: commits_vec,
            branch_head_indices,
            theme: AppTheme::gitcomet_dark(),
        }
    }

    pub fn run(&self) -> u64 {
        let graph = history_graph::compute_graph(
            &self.commits,
            self.theme,
            self.branch_head_indices
                .iter()
                .map(|&ix| self.commits[ix].id.as_ref()),
            None,
        );
        let mut h = FxHasher::default();
        graph.len().hash(&mut h);
        graph
            .iter()
            .take(256)
            .map(|r| {
                (
                    r.lanes_now.len(),
                    r.lanes_next.len(),
                    r.joins_in.len(),
                    r.edges_out.len(),
                    r.is_merge,
                )
            })
            .collect::<Vec<_>>()
            .hash(&mut h);
        h.finish()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, HistoryGraphMetrics) {
        let graph = history_graph::compute_graph(
            &self.commits,
            self.theme,
            self.branch_head_indices
                .iter()
                .map(|&ix| self.commits[ix].id.as_ref()),
            None,
        );

        let graph_rows = graph.len();
        let max_lanes = graph.iter().map(|r| r.lanes_now.len()).max().unwrap_or(0);
        let merge_count = graph.iter().filter(|r| r.is_merge).count();

        let mut h = FxHasher::default();
        graph_rows.hash(&mut h);
        graph
            .iter()
            .take(256)
            .map(|r| {
                (
                    r.lanes_now.len(),
                    r.lanes_next.len(),
                    r.joins_in.len(),
                    r.edges_out.len(),
                    r.is_merge,
                )
            })
            .collect::<Vec<_>>()
            .hash(&mut h);

        let metrics = HistoryGraphMetrics {
            commit_count: self.commits.len(),
            graph_rows,
            max_lanes,
            merge_count,
            branch_heads: self.branch_head_indices.len(),
        };
        (h.finish(), metrics)
    }

    #[cfg(test)]
    fn commit_count(&self) -> usize {
        self.commits.len()
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct HistoryGraphMetrics {
    pub commit_count: usize,
    pub graph_rows: usize,
    pub max_lanes: usize,
    pub merge_count: usize,
    pub branch_heads: usize,
}

pub struct HistoryCacheBuildMetrics {
    pub visible_commits: usize,
    pub graph_rows: usize,
    pub max_lanes: usize,
    pub commit_vms: usize,
    pub stash_helpers_filtered: usize,
    pub decorated_commits: usize,
}

pub struct HistoryCacheBuildFixture {
    commits: Vec<Commit>,
    branches: Vec<Branch>,
    remote_branches: Vec<RemoteBranch>,
    tags: Vec<Tag>,
    stashes: Vec<StashEntry>,
    head_branch: Option<String>,
    theme: AppTheme,
}

impl HistoryCacheBuildFixture {
    pub const EXTREME_SCALE_COMMITS: usize = 50_000;
    pub const EXTREME_SCALE_LOCAL_BRANCHES: usize = 500;
    pub const EXTREME_SCALE_REMOTE_BRANCHES: usize = 1_000;
    pub const EXTREME_SCALE_TAGS: usize = 500;
    pub const EXTREME_SCALE_STASHES: usize = 200;

    /// Moderate mix of commits, branches, tags, and stashes.
    pub fn balanced(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        tags: usize,
        stashes: usize,
    ) -> Self {
        let commits_vec = build_synthetic_commits(commits);
        let (branches, remote_branches_vec) =
            build_branches_targeting_commits(&commits_vec, local_branches, remote_branches);
        let tags_vec = build_tags_targeting_commits(&commits_vec, tags);
        let (stash_entries, _) = build_simple_stash_entries(stashes);
        Self {
            commits: commits_vec,
            branches,
            remote_branches: remote_branches_vec,
            tags: tags_vec,
            stashes: stash_entries,
            head_branch: Some("main".to_string()),
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    /// Dense merge topology stressing graph lane computation.
    pub fn merge_dense(commits: usize) -> Self {
        let commits_vec = build_synthetic_commits_with_merge_stride(commits, 5, 3);
        let (branches, remote_branches) = build_branches_targeting_commits(&commits_vec, 10, 20);
        let tags_vec = build_tags_targeting_commits(&commits_vec, 10);
        Self {
            commits: commits_vec,
            branches,
            remote_branches,
            tags: tags_vec,
            stashes: Vec::new(),
            head_branch: Some("main".to_string()),
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    /// Many branches and tags decorating commits, stressing decoration map build.
    pub fn decorated_refs_heavy(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        tags: usize,
    ) -> Self {
        let commits_vec = build_synthetic_commits(commits);
        let (branches, remote_branches_vec) =
            build_branches_targeting_commits(&commits_vec, local_branches, remote_branches);
        let tags_vec = build_tags_targeting_commits(&commits_vec, tags);
        Self {
            commits: commits_vec,
            branches,
            remote_branches: remote_branches_vec,
            tags: tags_vec,
            stashes: Vec::new(),
            head_branch: Some("main".to_string()),
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    /// Many stash entries with stash-like commits injected into the log,
    /// stressing stash detection, helper filtering, and stash summary extraction.
    pub fn stash_heavy(commits: usize, stash_count: usize) -> Self {
        let base_count = commits.saturating_sub(stash_count * 2);
        let mut commits_vec = build_synthetic_commits(base_count);
        let start_ix = commits_vec.len();
        let (stash_entries, extra_commits) =
            build_stash_fixture_commits(&commits_vec, stash_count, start_ix);
        commits_vec.extend(extra_commits);
        let (branches, remote_branches) = build_branches_targeting_commits(&commits_vec, 50, 100);
        Self {
            commits: commits_vec,
            branches,
            remote_branches,
            tags: Vec::new(),
            stashes: stash_entries,
            head_branch: Some("main".to_string()),
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    /// Extreme-scale history-cache build with a 50k-commit log, 2k refs, and
    /// 200 matching stash tips/helpers so all synchronous cache-build phases
    /// execute under large but deterministic inputs.
    pub fn extreme_scale_50k_2k_refs_200_stashes() -> Self {
        let base_count =
            Self::EXTREME_SCALE_COMMITS.saturating_sub(Self::EXTREME_SCALE_STASHES * 2);
        let mut commits_vec = build_synthetic_commits(base_count);
        let start_ix = commits_vec.len();
        let (stash_entries, extra_commits) =
            build_stash_fixture_commits(&commits_vec, Self::EXTREME_SCALE_STASHES, start_ix);
        commits_vec.extend(extra_commits);

        let (branches, remote_branches) = build_branches_targeting_commits(
            &commits_vec,
            Self::EXTREME_SCALE_LOCAL_BRANCHES,
            Self::EXTREME_SCALE_REMOTE_BRANCHES,
        );
        let tags = build_tags_targeting_commits(&commits_vec, Self::EXTREME_SCALE_TAGS);

        Self {
            commits: commits_vec,
            branches,
            remote_branches,
            tags,
            stashes: stash_entries,
            head_branch: Some("main".to_string()),
            theme: AppTheme::zed_ayu_dark(),
        }
    }

    /// Replicates the synchronous computation from `ensure_history_cache`'s
    /// `smol::unblock` closure: commit index map, stash detection, visible
    /// commit filtering, graph computation, decoration maps, and row VM
    /// construction.
    pub fn run(&self) -> (u64, HistoryCacheBuildMetrics) {
        let commits = &self.commits;
        let branches = &self.branches;
        let remote_branches = &self.remote_branches;
        let tags = &self.tags;
        let stashes = &self.stashes;
        let head_branch = &self.head_branch;
        let theme = self.theme;

        // 1. stash_messages_by_id
        let mut stash_messages_by_id: HashMap<&str, Arc<str>> =
            HashMap::with_capacity_and_hasher(stashes.len(), Default::default());
        for stash in stashes.iter() {
            stash_messages_by_id.insert(stash.id.as_ref(), stash.message.clone());
        }

        // 2. stash tip detection
        let stash_tip_ids_from_list: HashSet<&str> = stash_messages_by_id.keys().copied().collect();
        let mut stash_tip_ids = stash_tip_ids_from_list.clone();
        let derive_helper_ids_from_detected_tips = stash_tip_ids_from_list.is_empty();
        let mut stash_helper_ids: HashSet<&str> = HashSet::with_capacity_and_hasher(
            stash_tip_ids_from_list.len().max(4),
            Default::default(),
        );
        for commit in commits.iter() {
            let commit_id = commit.id.as_ref();
            let is_probable_stash = Self::is_probable_stash_tip(commit);
            if is_probable_stash {
                stash_tip_ids.insert(commit_id);
            }

            let should_collect_helpers = if derive_helper_ids_from_detected_tips {
                is_probable_stash
            } else {
                stash_tip_ids_from_list.contains(commit_id)
            };
            if !should_collect_helpers {
                continue;
            }

            for parent_id in commit.parent_ids.iter().skip(1).map(|p| p.as_ref()) {
                stash_helper_ids.insert(parent_id);
            }
        }

        // 4. visible_indices — short-circuit when no stash helpers to avoid
        // N hash lookups against the empty set.
        let visible_indices: Vec<usize> = if stash_helper_ids.is_empty() {
            (0..commits.len()).collect()
        } else {
            let mut visible_indices =
                Vec::with_capacity(commits.len().saturating_sub(stash_helper_ids.len()));
            for (ix, commit) in commits.iter().enumerate() {
                if !stash_helper_ids.contains(commit.id.as_ref()) {
                    visible_indices.push(ix);
                }
            }
            visible_indices
        };
        let stash_helpers_filtered = commits.len() - visible_indices.len();

        // 7. head target resolution + branch_heads + compute_graph
        let head_target = match head_branch.as_deref() {
            Some("HEAD") => None,
            Some(head) => branches
                .iter()
                .find(|b| b.name == head)
                .map(|b| b.target.as_ref()),
            None => None,
        };
        let graph_rows: Arc<[history_graph::GraphRow]> = if stash_helper_ids.is_empty() {
            history_graph::compute_graph(
                commits,
                theme,
                branches
                    .iter()
                    .map(|b| b.target.as_ref())
                    .chain(remote_branches.iter().map(|b| b.target.as_ref())),
                head_target,
            )
            .into()
        } else {
            let visible_commit_refs = visible_indices
                .iter()
                .map(|&ix| &commits[ix])
                .collect::<Vec<_>>();
            history_graph::compute_graph_refs(
                &visible_commit_refs,
                theme,
                branches
                    .iter()
                    .map(|b| b.target.as_ref())
                    .chain(remote_branches.iter().map(|b| b.target.as_ref())),
                head_target,
            )
            .into()
        };
        let max_lanes = graph_rows
            .iter()
            .map(|r| r.lanes_now.len().max(r.lanes_next.len()))
            .max()
            .unwrap_or(1);

        // 8. branch/tag decorations precomputed once per target
        let (branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
            branches,
            remote_branches,
            head_branch.as_deref(),
            head_target,
        );
        let tag_names_by_target = build_history_tag_names_by_target(tags);

        // 9. commit_row_vms — replicate the VM construction from ensure_history_cache
        let empty_tags: Arc<[SharedString]> = Vec::new().into();
        let mut decorated_count = 0usize;
        let mut author_cache: HashMap<&str, SharedString> =
            HashMap::with_capacity_and_hasher(64, Default::default());
        let mut commit_row_vms: Vec<(
            SharedString,
            Arc<[SharedString]>,
            SharedString,
            SharedString,
            HistoryWhenVm,
            HistoryShortShaVm,
            bool,
            bool,
        )> = Vec::with_capacity(visible_indices.len());
        for &ix in &visible_indices {
            let Some(commit) = commits.get(ix) else {
                continue;
            };
            let commit_id = commit.id.as_ref();
            let is_head = head_target == Some(commit_id);
            let cached_branch_text = branch_text_by_target.get(commit_id);

            let branches_text = if is_head {
                head_branches_text.clone().unwrap_or_default()
            } else {
                cached_branch_text.cloned().unwrap_or_default()
            };

            let tag_names = tag_names_by_target
                .get(commit_id)
                .cloned()
                .unwrap_or_else(|| Arc::clone(&empty_tags));

            if is_head || cached_branch_text.is_some() || !tag_names.is_empty() {
                decorated_count += 1;
            }

            let author: SharedString = author_cache
                .entry(commit.author.as_ref())
                .or_insert_with(|| commit.author.clone().into())
                .clone();
            let is_stash = stash_tip_ids.contains(commit_id);
            let summary: SharedString = if is_stash {
                stash_messages_by_id
                    .get(commit_id)
                    .filter(|message| !message.trim().is_empty())
                    .cloned()
                    .map(Into::into)
                    .or_else(|| {
                        Self::stash_summary_from_log_summary(&commit.summary).map(SharedString::new)
                    })
                    .unwrap_or_else(|| commit.summary.clone().into())
            } else {
                commit.summary.clone().into()
            };

            commit_row_vms.push((
                branches_text,
                tag_names,
                author,
                summary,
                HistoryWhenVm::deferred(commit.time),
                HistoryShortShaVm::new(commit.id.as_ref()),
                is_head,
                is_stash,
            ));
        }

        // Hash output to prevent dead-code elimination
        let mut h = FxHasher::default();
        visible_indices.len().hash(&mut h);
        graph_rows.len().hash(&mut h);
        max_lanes.hash(&mut h);
        commit_row_vms.len().hash(&mut h);
        for vm in commit_row_vms.iter().take(256) {
            let bt: &str = vm.0.as_ref();
            let sha = vm.5.as_str();
            bt.hash(&mut h);
            sha.hash(&mut h);
            vm.6.hash(&mut h);
            vm.7.hash(&mut h);
        }

        let metrics = HistoryCacheBuildMetrics {
            visible_commits: visible_indices.len(),
            graph_rows: graph_rows.len(),
            max_lanes,
            commit_vms: commit_row_vms.len(),
            stash_helpers_filtered,
            decorated_commits: decorated_count,
        };

        (h.finish(), metrics)
    }

    fn is_probable_stash_tip(commit: &Commit) -> bool {
        if !(2..=3).contains(&commit.parent_ids.len()) {
            return false;
        }
        let summary: &str = &commit.summary;
        (summary.starts_with("WIP on ") || summary.starts_with("On ")) && summary.contains(": ")
    }

    fn stash_summary_from_log_summary(summary: &str) -> Option<&str> {
        let (_, tail) = summary.split_once(": ")?;
        let trimmed = tail.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistoryLoadMoreAppendMetrics {
    pub existing_commits: usize,
    pub appended_commits: usize,
    pub total_commits_after_append: usize,
    pub next_cursor_present: u64,
    pub follow_up_effect_count: usize,
    pub log_rev_delta: u64,
    pub log_loading_more_cleared: u64,
}

pub struct HistoryLoadMoreAppendFixture {
    repo_id: RepoId,
    scope: LogScope,
    workdir: std::path::PathBuf,
    existing_commits: Vec<Commit>,
    appended_commits: Vec<Commit>,
}

impl HistoryLoadMoreAppendFixture {
    pub fn new(existing_commits: usize, page_size: usize) -> Self {
        let existing_commits = existing_commits.max(1);
        let page_size = page_size.max(1);
        let commits = build_synthetic_commits(existing_commits.saturating_add(page_size));
        let (existing_commits, appended_commits) = commits.split_at(existing_commits);

        Self {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            workdir: std::path::PathBuf::from("/tmp/bench-history-load-more-append"),
            existing_commits: existing_commits.to_vec(),
            appended_commits: appended_commits.to_vec(),
        }
    }

    pub fn request_cursor(&self) -> Option<LogCursor> {
        self.existing_commits.last().map(|commit| LogCursor {
            last_seen: commit.id.clone(),
            resume_from: None,
        })
    }

    fn response_cursor(&self) -> Option<LogCursor> {
        self.appended_commits.last().map(|commit| LogCursor {
            last_seen: commit.id.clone(),
            resume_from: None,
        })
    }

    pub fn fresh_state(&self) -> AppState {
        let mut state = AppState::default();
        let mut repo_state = RepoState::new_opening(
            self.repo_id,
            RepoSpec {
                workdir: self.workdir.clone(),
            },
        );
        repo_state.history_state.history_scope = self.scope;
        repo_state.history_state.log_loading_more = true;
        repo_state.log_loading_more = true;
        let log = Loadable::Ready(Arc::new(LogPage {
            commits: self.existing_commits.clone(),
            next_cursor: self.request_cursor(),
        }));
        repo_state.history_state.log = log.clone();
        repo_state.log = log;
        state.repos.push(repo_state);
        state.active_repo = Some(self.repo_id);
        state
    }

    pub fn append_page(&self) -> LogPage {
        LogPage {
            commits: self.appended_commits.clone(),
            next_cursor: self.response_cursor(),
        }
    }

    pub fn run_with_state_and_page(
        &self,
        state: &mut AppState,
        cursor: Option<LogCursor>,
        page: LogPage,
    ) -> (u64, HistoryLoadMoreAppendMetrics) {
        let log_rev_before = state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .map(|repo| repo.history_state.log_rev)
            .unwrap_or_default();

        let effects = dispatch_sync(
            state,
            Msg::Internal(InternalMsg::LogLoaded {
                repo_id: self.repo_id,
                scope: self.scope,
                cursor,
                result: Ok(page),
            }),
        );

        let repo_state = state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .expect("history load-more fixture should keep its repo");
        let Loadable::Ready(page) = &repo_state.log else {
            panic!("history load-more fixture expected ready log after append");
        };

        let total_commits_after_append = page.commits.len();
        let log_rev_delta = repo_state
            .history_state
            .log_rev
            .saturating_sub(log_rev_before);
        let next_cursor_present = u64::from(page.next_cursor.is_some());
        let log_loading_more_cleared = u64::from(!repo_state.history_state.log_loading_more);

        let mut h = FxHasher::default();
        total_commits_after_append.hash(&mut h);
        self.existing_commits.len().hash(&mut h);
        self.appended_commits.len().hash(&mut h);
        next_cursor_present.hash(&mut h);
        log_rev_delta.hash(&mut h);
        log_loading_more_cleared.hash(&mut h);
        effects.len().hash(&mut h);
        page.commits.first().map(|commit| &commit.id).hash(&mut h);
        page.commits.last().map(|commit| &commit.id).hash(&mut h);

        let metrics = HistoryLoadMoreAppendMetrics {
            existing_commits: self.existing_commits.len(),
            appended_commits: self.appended_commits.len(),
            total_commits_after_append,
            next_cursor_present,
            follow_up_effect_count: effects.len(),
            log_rev_delta,
            log_loading_more_cleared,
        };

        (h.finish(), metrics)
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, HistoryLoadMoreAppendMetrics) {
        self.run_with_state_and_page(state, self.request_cursor(), self.append_page())
    }

    pub fn run(&self) -> (u64, HistoryLoadMoreAppendMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistoryScopeSwitchMetrics {
    pub existing_commits: usize,
    pub scope_changed: u64,
    pub log_rev_delta: u64,
    pub log_set_to_loading: u64,
    pub load_log_effect_count: usize,
    pub persist_session_effect_count: usize,
}

pub struct HistoryScopeSwitchFixture {
    repo_id: RepoId,
    from_scope: LogScope,
    to_scope: LogScope,
    workdir: std::path::PathBuf,
    existing_commits: Vec<Commit>,
}

impl HistoryScopeSwitchFixture {
    pub fn new(existing_commits: usize, from: LogScope, to: LogScope) -> Self {
        let existing_commits = existing_commits.max(1);
        let commits = build_synthetic_commits(existing_commits);

        Self {
            repo_id: RepoId(1),
            from_scope: from,
            to_scope: to,
            workdir: std::path::PathBuf::from("/tmp/bench-history-scope-switch"),
            existing_commits: commits,
        }
    }

    pub fn current_branch_to_all_refs(existing_commits: usize) -> Self {
        Self::new(
            existing_commits,
            LogScope::CurrentBranch,
            LogScope::AllBranches,
        )
    }

    pub fn fresh_state(&self) -> AppState {
        let mut state = AppState::default();
        let mut repo_state = RepoState::new_opening(
            self.repo_id,
            RepoSpec {
                workdir: self.workdir.clone(),
            },
        );
        repo_state.history_state.history_scope = self.from_scope;
        repo_state.history_state.log_loading_more = false;
        repo_state.log = Loadable::Ready(Arc::new(LogPage {
            commits: self.existing_commits.clone(),
            next_cursor: self.existing_commits.last().map(|c| LogCursor {
                last_seen: c.id.clone(),
                resume_from: None,
            }),
        }));
        state.repos.push(repo_state);
        state.active_repo = Some(self.repo_id);
        state
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, HistoryScopeSwitchMetrics) {
        let log_rev_before = state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .map(|repo| repo.history_state.log_rev)
            .unwrap_or_default();

        let effects = dispatch_sync(
            state,
            Msg::SetHistoryScope {
                repo_id: self.repo_id,
                scope: self.to_scope,
            },
        );

        let repo_state = state
            .repos
            .iter()
            .find(|repo| repo.id == self.repo_id)
            .expect("history scope switch fixture should keep its repo");

        let log_rev_delta = repo_state
            .history_state
            .log_rev
            .saturating_sub(log_rev_before);
        let scope_changed = u64::from(repo_state.history_state.history_scope == self.to_scope);
        let log_set_to_loading = u64::from(matches!(repo_state.log, Loadable::Loading));
        let load_log_effect_count = effects
            .iter()
            .filter(|e| matches!(e, Effect::LoadLog { .. }))
            .count();
        let persist_session_effect_count = effects.len() - load_log_effect_count;

        let mut h = FxHasher::default();
        self.existing_commits.len().hash(&mut h);
        log_rev_delta.hash(&mut h);
        scope_changed.hash(&mut h);
        log_set_to_loading.hash(&mut h);
        load_log_effect_count.hash(&mut h);
        persist_session_effect_count.hash(&mut h);

        let metrics = HistoryScopeSwitchMetrics {
            existing_commits: self.existing_commits.len(),
            scope_changed,
            log_rev_delta,
            log_set_to_loading,
            load_log_effect_count,
            persist_session_effect_count,
        };

        (h.finish(), metrics)
    }

    pub fn run(&self) -> (u64, HistoryScopeSwitchMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }
}

pub struct CommitDetailsFixture {
    details: CommitDetails,
    message_render: Option<CommitDetailsMessageRenderConfig>,
    path_labels: RefCell<CommitFilePathLabelsCache<CommitId>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CommitDetailsMessageRenderConfig {
    visible_lines: usize,
    wrap_width_px: usize,
    max_shape_bytes: usize,
}

impl CommitDetailsFixture {
    pub fn new(files: usize, depth: usize) -> Self {
        Self {
            details: build_synthetic_commit_details(files, depth),
            message_render: None,
            path_labels: RefCell::new(CommitFilePathLabelsCache::default()),
        }
    }

    pub fn large_message_body(
        files: usize,
        depth: usize,
        message_bytes: usize,
        line_bytes: usize,
        visible_lines: usize,
        wrap_width_px: usize,
    ) -> Self {
        Self {
            details: build_synthetic_commit_details_with_message(
                files,
                depth,
                build_synthetic_commit_message(message_bytes, line_bytes),
            ),
            message_render: Some(CommitDetailsMessageRenderConfig {
                visible_lines: visible_lines.max(1),
                wrap_width_px: wrap_width_px.max(1),
                max_shape_bytes: 4 * 1024,
            }),
            path_labels: RefCell::new(CommitFilePathLabelsCache::default()),
        }
    }

    pub fn prewarm_runtime_state(&self) {
        let mut path_labels = self.path_labels.borrow_mut();
        let _ = path_labels.labels_for(&self.details.id, &self.details.files);
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn reset_runtime_state(&self) {
        self.path_labels.borrow_mut().clear();
    }

    pub fn run(&self) -> u64 {
        // Approximation of the per-row work done by the commit files list:
        // kind->icon mapping and reusing the current commit's cached path labels.
        let mut h = FxHasher::default();
        self.details.id.as_ref().hash(&mut h);
        commit_details_message_hash(self.details.message.as_str(), self.message_render, &mut h);

        let path_labels = {
            let mut path_labels = self.path_labels.borrow_mut();
            path_labels.labels_for(&self.details.id, &self.details.files)
        };
        let mut counts = [0usize; 6];
        for (f, path_label) in self.details.files.iter().zip(path_labels.iter()) {
            let visuals = commit_file_kind_visuals(f.kind);
            Some(visuals.icon).hash(&mut h);
            visuals.kind_key.hash(&mut h);
            path_label.as_ref().hash(&mut h);

            counts[visuals.kind_key as usize] = counts[visuals.kind_key as usize].saturating_add(1);
        }
        counts.hash(&mut h);
        h.finish()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, CommitDetailsMetrics) {
        let hash = self.run();

        let file_count = self.details.files.len();
        let max_depth = self
            .details
            .files
            .iter()
            .map(|f| f.path.components().count())
            .max()
            .unwrap_or(0);
        let message_bytes = self.details.message.len();
        let message_lines = count_commit_message_lines(self.details.message.as_str());
        let (message_shaped_lines, message_shaped_bytes) = measure_commit_message_visible_window(
            self.details.message.as_str(),
            self.message_render,
        );

        let mut kind_counts = [0usize; 6];
        for f in &self.details.files {
            let ix = commit_file_kind_visuals(f.kind).kind_key as usize;
            kind_counts[ix] = kind_counts[ix].saturating_add(1);
        }

        let metrics = CommitDetailsMetrics {
            file_count,
            max_path_depth: max_depth,
            message_bytes,
            message_lines,
            message_shaped_lines,
            message_shaped_bytes,
            added_files: kind_counts[0],
            modified_files: kind_counts[1],
            deleted_files: kind_counts[2],
            renamed_files: kind_counts[3],
        };
        (hash, metrics)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct CommitDetailsMetrics {
    pub file_count: usize,
    pub max_path_depth: usize,
    pub message_bytes: usize,
    pub message_lines: usize,
    pub message_shaped_lines: usize,
    pub message_shaped_bytes: usize,
    pub added_files: usize,
    pub modified_files: usize,
    pub deleted_files: usize,
    pub renamed_files: usize,
}

/// Simulates switching from one selected commit to another, measuring the cost
/// of replacing commit details (resetting scroll state and rebuilding the file
/// list for a different commit). This captures the select_commit_replace workflow.
pub struct CommitSelectReplaceFixture {
    commit_a: CommitDetails,
    commit_b: CommitDetails,
}

impl CommitSelectReplaceFixture {
    pub fn new(files: usize, depth: usize) -> Self {
        let commit_a = build_synthetic_commit_details(files, depth);
        let commit_b = build_synthetic_commit_details_with_id(files, depth, "e");
        Self { commit_a, commit_b }
    }

    /// Run the replacement: process commit_a, then switch to commit_b.
    /// Returns the hash of the second commit's processing (the replacement cost).
    pub fn run(&self) -> u64 {
        // First commit is already rendered (warm cache of the first commit's rows).
        let _ = commit_details_row_hash(&self.commit_a);
        // Second commit replaces it — this is the measured cost.
        commit_details_row_hash(&self.commit_b)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, CommitSelectReplaceMetrics) {
        let hash_a = commit_details_row_hash(&self.commit_a);
        let hash_b = commit_details_row_hash(&self.commit_b);
        let metrics = CommitSelectReplaceMetrics {
            files_a: self.commit_a.files.len(),
            files_b: self.commit_b.files.len(),
            commit_ids_differ: self.commit_a.id != self.commit_b.id,
            hash_a,
            hash_b,
        };
        (hash_b, metrics)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct CommitSelectReplaceMetrics {
    pub files_a: usize,
    pub files_b: usize,
    pub commit_ids_differ: bool,
    pub hash_a: u64,
    pub hash_b: u64,
}

/// Simulates commit details rendering with enough unique file paths to overflow
/// the bounded path-display cache, exercising the generation-rotation path that
/// `cached_path_display` uses. This catches regressions where large file lists
/// trigger repeated cache resets within a single interaction.
pub struct PathDisplayCacheChurnFixture {
    details: CommitDetails,
    path_display_cache: path_display::PathDisplayCache,
}

impl PathDisplayCacheChurnFixture {
    /// Creates a fixture with `files` unique paths at `depth` directory levels.
    /// Set `files` > 8192 to trigger at least one generation rotation during a
    /// single rendering pass.
    pub fn new(files: usize, depth: usize) -> Self {
        Self {
            details: build_synthetic_commit_details_unique_paths(files, depth),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.path_display_cache.clear();
    }

    /// Processes all file paths through `cached_path_display`, simulating a
    /// full-list render pass. Returns the FxHash of all formatted paths.
    pub fn run(&mut self) -> u64 {
        let mut h = FxHasher::default();
        for f in &self.details.files {
            let display = path_display::cached_path_display(&mut self.path_display_cache, &f.path);
            display.as_ref().hash(&mut h);
        }
        h.finish()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&mut self) -> (u64, PathDisplayCacheChurnMetrics) {
        self.reset_runtime_state();
        path_display::bench_reset();
        let hash = self.run();
        let counters = path_display::bench_snapshot();
        path_display::bench_reset();
        let metrics = PathDisplayCacheChurnMetrics {
            file_count: self.details.files.len(),
            path_display_cache_hits: counters.cache_hits,
            path_display_cache_misses: counters.cache_misses,
            path_display_cache_clears: counters.cache_clears,
        };
        (hash, metrics)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PathDisplayCacheChurnMetrics {
    pub file_count: usize,
    pub path_display_cache_hits: u64,
    pub path_display_cache_misses: u64,
    pub path_display_cache_clears: u64,
}

#[cfg(windows)]
const GIT_OPS_NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const GIT_OPS_NULL_DEVICE: &str = "/dev/null";

enum GitOpsScenario {
    StatusDirty {
        tracked_files: usize,
        dirty_files: usize,
    },
    StatusClean {
        tracked_files: usize,
    },
    LogWalk {
        total_commits: usize,
        requested_commits: usize,
    },
    DiffCommit {
        target: DiffTarget,
        changed_files: usize,
        renamed_files: usize,
        binary_files: usize,
        line_count: usize,
    },
    BlameFile {
        path: std::path::PathBuf,
        total_lines: usize,
        total_commits: usize,
    },
    FileHistory {
        path: std::path::PathBuf,
        total_commits: usize,
        file_history_commits: usize,
        requested_commits: usize,
    },
    RefEnumerate {
        total_refs: usize,
    },
}

enum GitOpsOutcome {
    Status {
        dirty_files: usize,
    },
    LogWalk {
        commits_returned: usize,
    },
    Diff {
        diff_lines: usize,
    },
    Blame {
        blame_lines: usize,
        distinct_commits: usize,
    },
    FileHistory {
        commits_returned: usize,
    },
    RefEnumerate {
        branches_returned: usize,
    },
}

pub struct GitOpsFixture {
    _repo_root: TempDir,
    repo: Arc<dyn GitRepository>,
    scenario: GitOpsScenario,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GitOpsMetrics {
    pub tracked_files: u64,
    pub dirty_files: u64,
    pub total_commits: u64,
    pub requested_commits: u64,
    pub commits_returned: u64,
    pub changed_files: u64,
    pub renamed_files: u64,
    pub binary_files: u64,
    pub line_count: u64,
    pub diff_lines: u64,
    pub blame_lines: u64,
    pub blame_distinct_commits: u64,
    pub file_history_commits: u64,
    pub total_refs: u64,
    pub branches_returned: u64,
    pub status_calls: u64,
    pub log_walk_calls: u64,
    pub diff_calls: u64,
    pub blame_calls: u64,
    pub ref_enumerate_calls: u64,
    pub status_ms: f64,
    pub log_walk_ms: f64,
    pub diff_ms: f64,
    pub blame_ms: f64,
    pub ref_enumerate_ms: f64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl GitOpsMetrics {
    fn from_snapshot(snapshot: GitOpTraceSnapshot) -> Self {
        Self {
            status_calls: snapshot.status.calls,
            log_walk_calls: snapshot.log_walk.calls,
            diff_calls: snapshot.diff.calls,
            blame_calls: snapshot.blame.calls,
            ref_enumerate_calls: snapshot.ref_enumerate.calls,
            status_ms: snapshot.status.total_millis(),
            log_walk_ms: snapshot.log_walk.total_millis(),
            diff_ms: snapshot.diff.total_millis(),
            blame_ms: snapshot.blame.total_millis(),
            ref_enumerate_ms: snapshot.ref_enumerate.total_millis(),
            ..Self::default()
        }
    }
}

impl GitOpsFixture {
    pub fn status_dirty(tracked_files: usize, dirty_files: usize) -> Self {
        let tracked_files = tracked_files.max(1);
        let dirty_files = dirty_files.min(tracked_files);
        let repo_root = build_git_ops_status_repo(tracked_files, dirty_files);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops status benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::StatusDirty {
                tracked_files,
                dirty_files,
            },
        }
    }

    pub fn status_dirty_500_files() -> Self {
        Self::status_dirty(1_000, 500)
    }

    pub fn log_walk(total_commits: usize, requested_commits: usize) -> Self {
        let total_commits = total_commits.max(1);
        let requested_commits = requested_commits.max(1).min(total_commits);
        let repo_root = build_git_ops_log_repo(total_commits);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops log benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::LogWalk {
                total_commits,
                requested_commits,
            },
        }
    }

    pub fn log_walk_10k_commits() -> Self {
        Self::log_walk(10_000, 10_000)
    }

    pub fn diff_rename_heavy(renamed_files: usize) -> Self {
        let renamed_files = renamed_files.max(1);
        let (repo_root, commit_id) = build_git_ops_diff_rename_repo(renamed_files);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops diff_rename_heavy benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::DiffCommit {
                target: DiffTarget::Commit {
                    commit_id,
                    path: None,
                },
                changed_files: renamed_files,
                renamed_files,
                binary_files: 0,
                line_count: 0,
            },
        }
    }

    pub fn diff_binary_heavy(binary_files: usize, bytes_per_file: usize) -> Self {
        let binary_files = binary_files.max(1);
        let bytes_per_file = bytes_per_file.max(1);
        let (repo_root, commit_id) = build_git_ops_binary_diff_repo(binary_files, bytes_per_file);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops diff_binary_heavy benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::DiffCommit {
                target: DiffTarget::Commit {
                    commit_id,
                    path: None,
                },
                changed_files: binary_files,
                renamed_files: 0,
                binary_files,
                line_count: 0,
            },
        }
    }

    pub fn diff_large_single_file(line_count: usize, line_bytes: usize) -> Self {
        let line_count = line_count.max(1);
        let line_bytes = line_bytes.max(16);
        let (repo_root, commit_id) = build_git_ops_large_diff_repo(line_count, line_bytes);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops diff_large_single_file benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::DiffCommit {
                target: DiffTarget::Commit {
                    commit_id,
                    path: None,
                },
                changed_files: 1,
                renamed_files: 0,
                binary_files: 0,
                line_count,
            },
        }
    }

    pub fn blame_large_file(total_lines: usize, total_commits: usize) -> Self {
        let total_lines = total_lines.max(1);
        let total_commits = total_commits.max(1);
        let (repo_root, path, total_commits) = build_git_ops_blame_repo(total_lines, total_commits);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops blame_large_file benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::BlameFile {
                path,
                total_lines,
                total_commits,
            },
        }
    }

    pub fn file_history(
        total_commits: usize,
        requested_commits: usize,
        touch_every: usize,
    ) -> Self {
        let total_commits = total_commits.max(1);
        let touch_every = touch_every.max(1).min(total_commits);
        let (repo_root, path, file_history_commits) =
            build_git_ops_file_history_repo(total_commits, touch_every);
        let requested_commits = requested_commits.max(1).min(file_history_commits.max(1));
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops file_history benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::FileHistory {
                path,
                total_commits,
                file_history_commits,
                requested_commits,
            },
        }
    }

    pub fn status_clean(tracked_files: usize) -> Self {
        let tracked_files = tracked_files.max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops status_clean benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::StatusClean { tracked_files },
        }
    }

    pub fn ref_enumerate(total_refs: usize) -> Self {
        let total_refs = total_refs.max(1);
        let repo_root = build_git_ops_ref_repo(total_refs);
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open git_ops ref_enumerate benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            scenario: GitOpsScenario::RefEnumerate { total_refs },
        }
    }

    pub fn run(&self) -> u64 {
        self.execute().0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, GitOpsMetrics) {
        let _capture = git_ops_trace::capture();
        let (hash, outcome) = self.execute();
        let mut metrics = GitOpsMetrics::from_snapshot(git_ops_trace::snapshot());

        match (&self.scenario, outcome) {
            (
                GitOpsScenario::StatusDirty {
                    tracked_files,
                    dirty_files: configured_dirty_files,
                },
                GitOpsOutcome::Status { dirty_files },
            ) => {
                metrics.tracked_files = u64::try_from(*tracked_files).unwrap_or(u64::MAX);
                metrics.dirty_files = u64::try_from(dirty_files).unwrap_or(u64::MAX);
                debug_assert_eq!(dirty_files, *configured_dirty_files);
            }
            (
                GitOpsScenario::StatusClean { tracked_files },
                GitOpsOutcome::Status { dirty_files },
            ) => {
                metrics.tracked_files = u64::try_from(*tracked_files).unwrap_or(u64::MAX);
                metrics.dirty_files = u64::try_from(dirty_files).unwrap_or(u64::MAX);
                debug_assert_eq!(dirty_files, 0);
            }
            (
                GitOpsScenario::LogWalk {
                    total_commits,
                    requested_commits,
                },
                GitOpsOutcome::LogWalk { commits_returned },
            ) => {
                metrics.total_commits = u64::try_from(*total_commits).unwrap_or(u64::MAX);
                metrics.requested_commits = u64::try_from(*requested_commits).unwrap_or(u64::MAX);
                metrics.commits_returned = u64::try_from(commits_returned).unwrap_or(u64::MAX);
            }
            (
                GitOpsScenario::DiffCommit {
                    changed_files,
                    renamed_files,
                    binary_files,
                    line_count,
                    ..
                },
                GitOpsOutcome::Diff { diff_lines },
            ) => {
                metrics.changed_files = u64::try_from(*changed_files).unwrap_or(u64::MAX);
                metrics.renamed_files = u64::try_from(*renamed_files).unwrap_or(u64::MAX);
                metrics.binary_files = u64::try_from(*binary_files).unwrap_or(u64::MAX);
                metrics.line_count = u64::try_from(*line_count).unwrap_or(u64::MAX);
                metrics.diff_lines = u64::try_from(diff_lines).unwrap_or(u64::MAX);
            }
            (
                GitOpsScenario::BlameFile {
                    total_lines,
                    total_commits,
                    ..
                },
                GitOpsOutcome::Blame {
                    blame_lines,
                    distinct_commits,
                },
            ) => {
                metrics.line_count = u64::try_from(*total_lines).unwrap_or(u64::MAX);
                metrics.total_commits = u64::try_from(*total_commits).unwrap_or(u64::MAX);
                metrics.blame_lines = u64::try_from(blame_lines).unwrap_or(u64::MAX);
                metrics.blame_distinct_commits =
                    u64::try_from(distinct_commits).unwrap_or(u64::MAX);
            }
            (
                GitOpsScenario::FileHistory {
                    total_commits,
                    file_history_commits,
                    requested_commits,
                    ..
                },
                GitOpsOutcome::FileHistory { commits_returned },
            ) => {
                metrics.total_commits = u64::try_from(*total_commits).unwrap_or(u64::MAX);
                metrics.file_history_commits =
                    u64::try_from(*file_history_commits).unwrap_or(u64::MAX);
                metrics.requested_commits = u64::try_from(*requested_commits).unwrap_or(u64::MAX);
                metrics.commits_returned = u64::try_from(commits_returned).unwrap_or(u64::MAX);
            }
            (
                GitOpsScenario::RefEnumerate { total_refs },
                GitOpsOutcome::RefEnumerate { branches_returned },
            ) => {
                metrics.total_refs = u64::try_from(*total_refs).unwrap_or(u64::MAX);
                metrics.branches_returned = u64::try_from(branches_returned).unwrap_or(u64::MAX);
            }
            _ => panic!("git_ops fixture outcome did not match configured scenario"),
        }

        (hash, metrics)
    }

    fn execute(&self) -> (u64, GitOpsOutcome) {
        match &self.scenario {
            GitOpsScenario::StatusDirty { .. } | GitOpsScenario::StatusClean { .. } => {
                let status = self.repo.status().expect("git_ops status benchmark");
                let dirty_files = status.staged.len().saturating_add(status.unstaged.len());
                (
                    hash_repo_status(&status),
                    GitOpsOutcome::Status { dirty_files },
                )
            }
            GitOpsScenario::LogWalk {
                requested_commits, ..
            } => {
                let page = self
                    .repo
                    .log_head_page(*requested_commits, None)
                    .expect("git_ops log benchmark");
                let commits_returned = page.commits.len();
                (
                    hash_log_page(&page),
                    GitOpsOutcome::LogWalk { commits_returned },
                )
            }
            GitOpsScenario::DiffCommit { target, .. } => {
                let diff = self
                    .repo
                    .diff_parsed(target)
                    .expect("git_ops diff benchmark");
                let diff_lines = diff.lines.len();
                (hash_parsed_diff(&diff), GitOpsOutcome::Diff { diff_lines })
            }
            GitOpsScenario::BlameFile { path, .. } => {
                let blame = self
                    .repo
                    .blame_file(path, None)
                    .expect("git_ops blame benchmark");
                let blame_lines = blame.len();
                let distinct_commits = blame
                    .iter()
                    .map(|line| line.commit_id.clone())
                    .collect::<HashSet<_>>()
                    .len();
                (
                    hash_blame_lines(&blame),
                    GitOpsOutcome::Blame {
                        blame_lines,
                        distinct_commits,
                    },
                )
            }
            GitOpsScenario::FileHistory {
                path,
                requested_commits,
                ..
            } => {
                let page = self
                    .repo
                    .log_file_page(path, *requested_commits, None)
                    .expect("git_ops file_history benchmark");
                let commits_returned = page.commits.len();
                (
                    hash_log_page(&page),
                    GitOpsOutcome::FileHistory { commits_returned },
                )
            }
            GitOpsScenario::RefEnumerate { .. } => {
                let branches = self
                    .repo
                    .list_branches()
                    .expect("git_ops ref_enumerate benchmark");
                let branches_returned = branches.len();
                (
                    hash_branch_list(&branches),
                    GitOpsOutcome::RefEnumerate { branches_returned },
                )
            }
        }
    }
}

fn hash_repo_status(status: &RepoStatus) -> u64 {
    let mut h = FxHasher::default();
    status.staged.len().hash(&mut h);
    status.unstaged.len().hash(&mut h);
    for entry in status.staged.iter().chain(status.unstaged.iter()).take(128) {
        entry.path.hash(&mut h);
        file_status_kind_code(entry.kind).hash(&mut h);
        entry.conflict.is_some().hash(&mut h);
    }
    h.finish()
}

fn hash_log_page(page: &LogPage) -> u64 {
    let mut h = FxHasher::default();
    page.commits.len().hash(&mut h);
    page.next_cursor.is_some().hash(&mut h);
    for commit in page.commits.iter().take(128) {
        commit.id.hash(&mut h);
        commit.parent_ids.len().hash(&mut h);
        commit.summary.len().hash(&mut h);
        commit.author.len().hash(&mut h);
    }
    h.finish()
}

fn hash_branch_list(branches: &[Branch]) -> u64 {
    let mut h = FxHasher::default();
    branches.len().hash(&mut h);
    for branch in branches.iter().take(128) {
        branch.name.hash(&mut h);
        branch.target.hash(&mut h);
    }
    h.finish()
}

fn hash_parsed_diff(diff: &Diff) -> u64 {
    let mut h = FxHasher::default();
    diff.lines.len().hash(&mut h);
    std::mem::discriminant(&diff.target).hash(&mut h);
    for line in diff.lines.iter().take(128) {
        diff_line_kind_code(line.kind).hash(&mut h);
        line.text.len().hash(&mut h);
    }
    h.finish()
}

fn hash_blame_lines(lines: &[gitcomet_core::services::BlameLine]) -> u64 {
    let mut h = FxHasher::default();
    lines.len().hash(&mut h);
    for line in lines.iter().take(128) {
        line.commit_id.hash(&mut h);
        line.author.hash(&mut h);
        line.summary.hash(&mut h);
        line.line.len().hash(&mut h);
    }
    h.finish()
}

fn file_status_kind_code(kind: FileStatusKind) -> u8 {
    match kind {
        FileStatusKind::Untracked => 0,
        FileStatusKind::Modified => 1,
        FileStatusKind::Added => 2,
        FileStatusKind::Deleted => 3,
        FileStatusKind::Renamed => 4,
        FileStatusKind::Conflicted => 5,
    }
}

fn diff_line_kind_code(kind: DiffLineKind) -> u8 {
    match kind {
        DiffLineKind::Header => 0,
        DiffLineKind::Hunk => 1,
        DiffLineKind::Add => 2,
        DiffLineKind::Remove => 3,
        DiffLineKind::Context => 4,
    }
}

fn build_git_ops_status_repo(tracked_files: usize, dirty_files: usize) -> TempDir {
    let repo_root = tempfile::tempdir().expect("create git_ops status tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    for index in 0..tracked_files {
        let relative = git_ops_status_relative_path(index);
        let path = repo.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create git_ops status parent directory");
        }
        fs::write(
            &path,
            format!("tracked-{index:05}\nmodule-{:02}\n", index % 32),
        )
        .expect("write git_ops tracked file");
    }

    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "seed"],
    );

    for index in 0..dirty_files {
        let relative = git_ops_status_relative_path(index);
        let path = repo.join(&relative);
        fs::write(
            &path,
            format!(
                "tracked-{index:05}\nmodule-{:02}\ndirty-{index:05}\n",
                index % 32
            ),
        )
        .expect("write git_ops dirty file");
    }

    repo_root
}

fn build_git_ops_log_repo(total_commits: usize) -> TempDir {
    let repo_root = tempfile::tempdir().expect("create git_ops log tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    let mut import = String::with_capacity(total_commits.saturating_mul(192));
    for index in 1..=total_commits {
        let blob_mark = index;
        let commit_mark = 100_000usize.saturating_add(index);
        let previous_commit_mark = commit_mark.saturating_sub(1);
        let payload = format!("seed-{index:05}");
        let message = format!("c{index:05}");
        let timestamp = 1_700_000_000usize.saturating_add(index);

        import.push_str("blob\n");
        import.push_str(&format!("mark :{blob_mark}\n"));
        import.push_str(&format!("data {}\n", payload.len()));
        import.push_str(&payload);
        import.push('\n');
        import.push_str("commit refs/heads/main\n");
        import.push_str(&format!("mark :{commit_mark}\n"));
        import.push_str(&format!(
            "author Bench <bench@example.com> {timestamp} +0000\n"
        ));
        import.push_str(&format!(
            "committer Bench <bench@example.com> {timestamp} +0000\n"
        ));
        import.push_str(&format!("data {}\n", message.len()));
        import.push_str(&message);
        import.push('\n');
        if index > 1 {
            import.push_str(&format!("from :{previous_commit_mark}\n"));
        }
        import.push_str(&format!("M 100644 :{blob_mark} history.txt\n"));
    }

    run_git_with_input(repo, &["fast-import", "--quiet"], &import);
    repo_root
}

fn build_git_ops_file_history_repo(
    total_commits: usize,
    touch_every: usize,
) -> (TempDir, std::path::PathBuf, usize) {
    let repo_root = tempfile::tempdir().expect("create git_ops file_history tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    let target_path = std::path::PathBuf::from("src/history/target.txt");
    let target_path_str = target_path.to_string_lossy();
    let mut import = String::with_capacity(total_commits.saturating_mul(224));
    let mut file_history_commits = 0usize;
    let noise_file_count = 64usize;

    for index in 1..=total_commits {
        let blob_mark = index;
        let commit_mark = 200_000usize.saturating_add(index);
        let previous_commit_mark = commit_mark.saturating_sub(1);
        let timestamp = 1_700_000_000usize.saturating_add(index);
        let touches_target = index % touch_every == 0;
        let (path, payload, message): (String, String, String) = if touches_target {
            file_history_commits = file_history_commits.saturating_add(1);
            (
                target_path_str.as_ref().to_string(),
                format!(
                    "history-commit-{index:06}\nrender_cache_hot_path_{index} = keep({index});\n"
                ),
                format!("history-{index:06}"),
            )
        } else {
            let noise_slot = index % noise_file_count;
            (
                format!("src/noise/module_{noise_slot:02}.txt"),
                format!("noise-commit-{index:06}\nmodule_slot_{noise_slot}\n"),
                format!("noise-{index:06}"),
            )
        };

        import.push_str("blob\n");
        import.push_str(&format!("mark :{blob_mark}\n"));
        import.push_str(&format!("data {}\n", payload.len()));
        import.push_str(&payload);
        import.push('\n');
        import.push_str("commit refs/heads/main\n");
        import.push_str(&format!("mark :{commit_mark}\n"));
        import.push_str(&format!(
            "author Bench <bench@example.com> {timestamp} +0000\n"
        ));
        import.push_str(&format!(
            "committer Bench <bench@example.com> {timestamp} +0000\n"
        ));
        import.push_str(&format!("data {}\n", message.len()));
        import.push_str(&message);
        import.push('\n');
        if index > 1 {
            import.push_str(&format!("from :{previous_commit_mark}\n"));
        }
        import.push_str(&format!("M 100644 :{blob_mark} {path}\n"));
    }

    run_git_with_input(repo, &["fast-import", "--quiet"], &import);
    (repo_root, target_path, file_history_commits)
}

fn build_git_ops_ref_repo(total_refs: usize) -> TempDir {
    let repo_root = tempfile::tempdir().expect("create git_ops ref tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    // Create a single seed commit, then point `total_refs` branches at it.
    let mut import = String::with_capacity(total_refs.saturating_mul(64).saturating_add(256));
    import.push_str("blob\nmark :1\ndata 4\nseed\n");
    import.push_str("commit refs/heads/main\nmark :100001\n");
    import.push_str("author Bench <bench@example.com> 1700000001 +0000\n");
    import.push_str("committer Bench <bench@example.com> 1700000001 +0000\n");
    import.push_str("data 4\nseed\nM 100644 :1 file.txt\n");

    // Create branches pointing to the same commit.
    for index in 0..total_refs {
        import.push_str(&format!(
            "reset refs/heads/branch_{index:05}\nfrom :100001\n\n"
        ));
    }

    run_git_with_input(repo, &["fast-import", "--quiet"], &import);
    repo_root
}

fn build_git_ops_diff_rename_repo(renamed_files: usize) -> (TempDir, CommitId) {
    let repo_root = tempfile::tempdir().expect("create git_ops diff_rename_heavy tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);
    run_git(repo, &["config", "diff.renames", "true"]);

    for index in 0..renamed_files {
        let relative = git_ops_rename_source_path(index);
        let path = repo.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create git_ops rename parent directory");
        }
        fs::write(&path, git_ops_rename_file_contents(index))
            .expect("write git_ops rename seed file");
    }

    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "seed"],
    );

    for index in 0..renamed_files {
        let from = repo.join(git_ops_rename_source_path(index));
        let to = repo.join(git_ops_rename_target_path(index));
        if let Some(parent) = to.parent() {
            fs::create_dir_all(parent).expect("create git_ops rename target directory");
        }
        fs::rename(&from, &to).expect("rename git_ops benchmark file");
        let mut content = fs::read_to_string(&to).expect("read renamed git_ops file");
        let _ = writeln!(&mut content, "renamed-{index:05}");
        fs::write(&to, content).expect("rewrite renamed git_ops file");
    }

    run_git(repo, &["add", "-A"]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "rename-heavy",
        ],
    );

    let head_commit_id = git_ops_head_commit_id(repo);
    (repo_root, head_commit_id)
}

fn build_git_ops_binary_diff_repo(
    binary_files: usize,
    bytes_per_file: usize,
) -> (TempDir, CommitId) {
    let repo_root = tempfile::tempdir().expect("create git_ops diff_binary_heavy tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    for index in 0..binary_files {
        let relative = git_ops_binary_relative_path(index);
        let path = repo.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create git_ops binary parent directory");
        }
        fs::write(&path, git_ops_binary_bytes(index, bytes_per_file, 17))
            .expect("write git_ops binary seed file");
    }

    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "seed"],
    );

    for index in 0..binary_files {
        let path = repo.join(git_ops_binary_relative_path(index));
        fs::write(&path, git_ops_binary_bytes(index, bytes_per_file, 53))
            .expect("rewrite git_ops binary file");
    }

    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "binary-heavy",
        ],
    );

    let head_commit_id = git_ops_head_commit_id(repo);
    (repo_root, head_commit_id)
}

fn build_git_ops_large_diff_repo(line_count: usize, line_bytes: usize) -> (TempDir, CommitId) {
    let repo_root = tempfile::tempdir().expect("create git_ops large_diff tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    let relative = std::path::PathBuf::from("src/large_diff/story.txt");
    let path = repo.join(&relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create git_ops large diff parent directory");
    }

    fs::write(&path, git_ops_large_text(line_count, line_bytes, 'a'))
        .expect("write git_ops large diff seed file");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-q", "-m", "seed"],
    );

    fs::write(&path, git_ops_large_text(line_count, line_bytes, 'b'))
        .expect("rewrite git_ops large diff file");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "rewrite",
        ],
    );

    let head_commit_id = git_ops_head_commit_id(repo);
    (repo_root, head_commit_id)
}

fn build_git_ops_blame_repo(
    total_lines: usize,
    total_commits: usize,
) -> (TempDir, std::path::PathBuf, usize) {
    let repo_root = tempfile::tempdir().expect("create git_ops blame tempdir");
    let repo = repo_root.path();
    init_git_ops_repo(repo);

    let path_rel = std::path::PathBuf::from("src/blame/story.txt");
    let path = repo.join(&path_rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create git_ops blame parent directory");
    }

    let effective_commits = total_commits.min(total_lines).max(1);
    let mut owners = vec![0usize; total_lines];

    fs::write(&path, git_ops_blame_text(&owners)).expect("write git_ops blame seed file");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "-m",
            "blame-00",
        ],
    );

    let chunk = total_lines.div_ceil(effective_commits);
    for commit_ix in 1..effective_commits {
        let start = commit_ix.saturating_mul(chunk).min(total_lines);
        let end = start.saturating_add(chunk).min(total_lines);
        for owner in &mut owners[start..end] {
            *owner = commit_ix;
        }
        fs::write(&path, git_ops_blame_text(&owners)).expect("rewrite git_ops blame file");
        run_git(repo, &["add", "."]);
        let message = format!("blame-{commit_ix:02}");
        run_git(
            repo,
            &["-c", "commit.gpgsign=false", "commit", "-q", "-m", &message],
        );
    }

    (repo_root, path_rel, effective_commits)
}

fn git_ops_status_relative_path(index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("src/module_{:02}/file_{index:05}.txt", index % 32))
}

fn git_ops_rename_source_path(index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "src/rename/from_{:02}/file_{index:05}.txt",
        index % 32
    ))
}

fn git_ops_rename_target_path(index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!(
        "src/rename/to_{:02}/renamed_{index:05}.txt",
        index % 32
    ))
}

fn git_ops_rename_file_contents(index: usize) -> String {
    let mut out = String::new();
    for line_ix in 0..8 {
        let _ = writeln!(
            &mut out,
            "rename-{index:05}-line-{line_ix:02}-module-{:02}",
            index % 32
        );
    }
    out
}

fn git_ops_binary_relative_path(index: usize) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("assets/blob_{:02}/file_{index:05}.bin", index % 16))
}

fn git_ops_binary_bytes(index: usize, bytes_per_file: usize, salt: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(bytes_per_file.max(1));
    for offset in 0..bytes_per_file.max(1) {
        bytes.push(((index.saturating_mul(31) + offset.saturating_mul(salt)) % 256) as u8);
    }
    if let Some(first) = bytes.first_mut() {
        *first = 0;
    }
    bytes
}

fn git_ops_large_text(line_count: usize, line_bytes: usize, marker: char) -> String {
    let line_bytes = line_bytes.max(16);
    let mut out = String::with_capacity(line_count.saturating_mul(line_bytes.saturating_add(1)));
    for index in 0..line_count {
        let prefix = format!("{marker}-{index:06}-");
        out.push_str(&prefix);
        let remaining = line_bytes.saturating_sub(prefix.len());
        for fill_ix in 0..remaining {
            out.push((b'a' + ((index + fill_ix) % 26) as u8) as char);
        }
        out.push('\n');
    }
    out
}

fn git_ops_blame_text(owners: &[usize]) -> String {
    let mut out = String::with_capacity(owners.len().saturating_mul(40));
    for (index, owner) in owners.iter().enumerate() {
        let _ = writeln!(
            &mut out,
            "line-{index:06}-owner-{owner:02}-payload-{:02}",
            (index + *owner) % 97
        );
    }
    out
}

fn init_git_ops_repo(repo: &Path) {
    run_git(repo, &["init", "-q", "-b", "main"]);
    run_git(repo, &["config", "user.email", "bench@example.com"]);
    run_git(repo, &["config", "user.name", "Bench"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
}

fn git_ops_head_commit_id(repo: &Path) -> CommitId {
    CommitId(git_stdout(repo, &["rev-parse", "HEAD"]).into())
}

fn run_git(repo: &Path, args: &[&str]) {
    let output = git_command(repo)
        .args(args)
        .output()
        .expect("run git benchmark helper");
    assert!(
        output.status.success(),
        "git {:?} failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_command(repo)
        .args(args)
        .output()
        .expect("run git benchmark helper for stdout");
    assert!(
        output.status.success(),
        "git {:?} failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git benchmark helper stdout utf8")
        .trim()
        .to_string()
}

fn run_git_with_input(repo: &Path, args: &[&str], input: &str) {
    let mut child = git_command(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn git benchmark helper");

    let mut stdin = child
        .stdin
        .take()
        .expect("git benchmark helper stdin available");
    stdin
        .write_all(input.as_bytes())
        .expect("write git benchmark helper stdin");
    drop(stdin);

    let output = child
        .wait_with_output()
        .expect("wait for git benchmark helper");
    assert!(
        output.status.success(),
        "git {:?} failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_command(repo: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", GIT_OPS_NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", GIT_OPS_NULL_DEVICE)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true");
    command
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusMultiSelectMetrics {
    pub entries_total: u64,
    pub selected_paths: u64,
    pub anchor_index: u64,
    pub clicked_index: u64,
    pub anchor_preserved: u64,
    pub position_scan_steps: u64,
}

pub struct StatusMultiSelectFixture {
    entries: Vec<std::path::PathBuf>,
    anchor_index: usize,
    clicked_index: usize,
    baseline_selection: StatusMultiSelection,
}

impl StatusMultiSelectFixture {
    pub fn range_select(entries: usize, anchor_index: usize, selected_paths: usize) -> Self {
        let entries = build_synthetic_status_entries(entries.max(1), DiffArea::Unstaged)
            .into_iter()
            .map(|entry| entry.path)
            .collect::<Vec<_>>();
        let max_index = entries.len().saturating_sub(1);
        let anchor_index = anchor_index.min(max_index);
        let selected_paths = selected_paths.max(1);
        let clicked_index = anchor_index
            .saturating_add(selected_paths.saturating_sub(1))
            .min(max_index);

        let mut baseline_selection = StatusMultiSelection::default();
        apply_status_multi_selection_click(
            &mut baseline_selection,
            StatusSection::CombinedUnstaged,
            entries[anchor_index].clone(),
            Some(anchor_index),
            gpui::Modifiers::default(),
            Some(&entries),
        );

        Self {
            entries,
            anchor_index,
            clicked_index,
            baseline_selection,
        }
    }

    pub fn run(&self) -> u64 {
        let selection = self.run_selection();
        hash_status_multi_selection(&selection)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, StatusMultiSelectMetrics) {
        bench_reset_status_selection();
        let selection = self.run_selection();
        let hash = hash_status_multi_selection(&selection);
        let counters = bench_snapshot_status_selection();
        let anchor_path = &self.entries[self.anchor_index];
        let selected_paths = selection.unstaged.as_slice();

        (
            hash,
            StatusMultiSelectMetrics {
                entries_total: self.entries.len() as u64,
                selected_paths: selected_paths.len() as u64,
                anchor_index: self.anchor_index as u64,
                clicked_index: self.clicked_index as u64,
                anchor_preserved: u64::from(
                    selection.unstaged_anchor.as_ref() == Some(anchor_path)
                        && selected_paths.iter().any(|path| path == anchor_path),
                ),
                position_scan_steps: counters.position_scan_steps,
            },
        )
    }

    fn run_selection(&self) -> StatusMultiSelection {
        let mut selection = self.baseline_selection.clone();
        apply_status_multi_selection_click(
            &mut selection,
            StatusSection::CombinedUnstaged,
            self.entries[self.clicked_index].clone(),
            Some(self.clicked_index),
            gpui::Modifiers {
                shift: true,
                ..Default::default()
            },
            Some(&self.entries),
        );
        selection
    }
}

fn hash_status_multi_selection(selection: &StatusMultiSelection) -> u64 {
    let mut h = FxHasher::default();
    selection.unstaged.len().hash(&mut h);
    selection.unstaged_anchor.hash(&mut h);
    for path in selection.unstaged.iter().take(64) {
        path.hash(&mut h);
    }
    selection.staged.len().hash(&mut h);
    selection.staged_anchor.hash(&mut h);
    h.finish()
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusSelectDiffOpenMetrics {
    pub effect_count: usize,
    pub load_diff_effect_count: usize,
    pub load_diff_file_effect_count: usize,
    pub load_diff_file_image_effect_count: usize,
    pub diff_state_rev_delta: u64,
}

impl StatusSelectDiffOpenMetrics {
    fn from_effects_and_rev(effects: &[Effect], rev_before: u64, rev_after: u64) -> Self {
        let mut metrics = Self {
            effect_count: effects.len(),
            diff_state_rev_delta: rev_after.wrapping_sub(rev_before),
            ..Self::default()
        };
        for effect in effects {
            match effect {
                Effect::LoadDiff { .. } => metrics.load_diff_effect_count += 1,
                Effect::LoadDiffFile { .. } => metrics.load_diff_file_effect_count += 1,
                Effect::LoadDiffFileImage { .. } => metrics.load_diff_file_image_effect_count += 1,
                _ => {}
            }
        }
        metrics
    }
}

pub struct StatusSelectDiffOpenFixture {
    baseline: AppState,
    diff_target: DiffTarget,
}

impl StatusSelectDiffOpenFixture {
    pub fn unstaged(status_entries: usize) -> Self {
        let entries = build_synthetic_status_entries(status_entries, DiffArea::Unstaged);
        let target_path = entries[entries.len() / 2].path.clone();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            diff_target: DiffTarget::WorkingTree {
                path: target_path,
                area: DiffArea::Unstaged,
            },
        }
    }

    pub fn staged(status_entries: usize) -> Self {
        let entries = build_synthetic_status_entries(status_entries, DiffArea::Staged);
        let target_path = entries[entries.len() / 2].path.clone();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            staged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            diff_target: DiffTarget::WorkingTree {
                path: target_path,
                area: DiffArea::Staged,
            },
        }
    }

    pub fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, StatusSelectDiffOpenMetrics) {
        let rev_before = state.repos[0].diff_state.diff_state_rev;
        let effects = dispatch_sync(
            state,
            Msg::SelectDiff {
                repo_id: RepoId(1),
                target: self.diff_target.clone(),
            },
        );
        let rev_after = state.repos[0].diff_state.diff_state_rev;
        let metrics =
            StatusSelectDiffOpenMetrics::from_effects_and_rev(&effects, rev_before, rev_after);

        let mut h = FxHasher::default();
        state.repos[0].diff_state.diff_state_rev.hash(&mut h);
        effects.len().hash(&mut h);
        for effect in effects.iter() {
            std::mem::discriminant(effect).hash(&mut h);
        }
        metrics.load_diff_effect_count.hash(&mut h);
        metrics.load_diff_file_effect_count.hash(&mut h);

        (h.finish(), metrics)
    }

    pub fn run(&self) -> (u64, StatusSelectDiffOpenMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StatusListMetrics {
    pub rows_requested: u64,
    pub rows_painted: u64,
    pub entries_total: u64,
    pub path_display_cache_hits: u64,
    pub path_display_cache_misses: u64,
    pub path_display_cache_clears: u64,
    pub max_path_depth: u64,
    pub prewarmed_entries: u64,
}

pub struct StatusListFixture {
    entries: Vec<FileStatus>,
    path_display_cache: path_display::PathDisplayCache,
}

impl StatusListFixture {
    pub fn unstaged_large(entries: usize) -> Self {
        Self {
            entries: build_synthetic_status_entries(entries, DiffArea::Unstaged),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn staged_large(entries: usize) -> Self {
        Self {
            entries: build_synthetic_status_entries(entries, DiffArea::Staged),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn mixed_depth(entries: usize) -> Self {
        Self {
            entries: build_synthetic_status_entries_mixed_depth(entries),
            path_display_cache: path_display::PathDisplayCache::default(),
        }
    }

    pub fn reset_runtime_state(&mut self) {
        self.path_display_cache.clear();
    }

    pub fn run_window_step(&mut self, start: usize, window: usize) -> u64 {
        let range = self.visible_range(start, window);
        self.hash_visible_range(range)
    }

    pub fn measure_window_step(&mut self, start: usize, window: usize) -> StatusListMetrics {
        self.measure_window_step_with_prewarm(start, window, 0)
    }

    pub fn prewarm_cache(&mut self, entries: usize) {
        let count = entries.min(self.entries.len());
        for entry in self.entries.iter().take(count) {
            let _ = path_display::cached_path_display(&mut self.path_display_cache, &entry.path);
        }
    }

    pub fn measure_window_step_with_prewarm(
        &mut self,
        start: usize,
        window: usize,
        prewarm_entries: usize,
    ) -> StatusListMetrics {
        let range = self.visible_range(start, window);
        self.reset_runtime_state();
        path_display::bench_reset();
        self.prewarm_cache(prewarm_entries);
        path_display::bench_reset();
        let _ = self.hash_visible_range(range.clone());
        let counters = path_display::bench_snapshot();
        path_display::bench_reset();

        StatusListMetrics {
            rows_requested: range.len() as u64,
            rows_painted: range.len() as u64,
            entries_total: self.entries.len() as u64,
            path_display_cache_hits: counters.cache_hits,
            path_display_cache_misses: counters.cache_misses,
            path_display_cache_clears: counters.cache_clears,
            max_path_depth: self.max_path_depth_for_range(range.clone()) as u64,
            prewarmed_entries: prewarm_entries.min(self.entries.len()) as u64,
        }
    }

    fn visible_range(&self, start: usize, window: usize) -> Range<usize> {
        let window = window.max(1).min(self.entries.len());
        if window == 0 {
            return 0..0;
        }

        let max_start = self.entries.len().saturating_sub(window);
        let start = if max_start == 0 {
            0
        } else {
            start % (max_start + 1)
        };
        start..start + window
    }

    fn hash_visible_range(&mut self, range: Range<usize>) -> u64 {
        let mut h = FxHasher::default();
        range.start.hash(&mut h);
        range.end.hash(&mut h);

        for (row_ix, entry) in self.entries[range].iter().enumerate() {
            status_row_kind_key(entry.kind).hash(&mut h);
            row_ix.hash(&mut h);
            entry.path.hash(&mut h);

            let path_display =
                path_display::cached_path_display(&mut self.path_display_cache, &entry.path);
            path_display.as_ref().hash(&mut h);
            path_display.len().hash(&mut h);
        }

        self.path_display_cache.len().hash(&mut h);
        h.finish()
    }

    fn max_path_depth_for_range(&self, range: Range<usize>) -> usize {
        self.entries[range]
            .iter()
            .map(|entry| entry.path.components().count())
            .max()
            .unwrap_or_default()
    }
}

fn status_row_kind_key(kind: FileStatusKind) -> u8 {
    match kind {
        FileStatusKind::Untracked => 0,
        FileStatusKind::Modified => 1,
        FileStatusKind::Added => 2,
        FileStatusKind::Deleted => 3,
        FileStatusKind::Renamed => 4,
        FileStatusKind::Conflicted => 5,
    }
}

pub struct LargeFileDiffScrollFixture {
    lines: Vec<String>,
    language: Option<super::diff_text::DiffSyntaxLanguage>,
    theme: AppTheme,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LargeFileDiffScrollMetrics {
    pub total_lines: u64,
    pub window_size: u64,
    pub start_line: u64,
    pub visible_text_bytes: u64,
    pub min_line_bytes: u64,
    pub language_detected: u64,
    pub syntax_mode_auto: u64,
}

impl LargeFileDiffScrollFixture {
    pub fn new(lines: usize) -> Self {
        Self::new_with_line_bytes(lines, 96)
    }

    pub fn new_with_line_bytes(lines: usize, line_bytes: usize) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        Self {
            lines: build_synthetic_source_lines(lines, line_bytes),
            language,
            theme,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        self.run_scroll_step_with_metrics(start, window).0
    }

    pub fn run_scroll_step_with_metrics(
        &self,
        start: usize,
        window: usize,
    ) -> (u64, LargeFileDiffScrollMetrics) {
        // Approximate "a scroll step": style the newly visible rows in a window.
        let actual_start = if self.lines.is_empty() {
            0
        } else {
            start % self.lines.len()
        };
        let end = (actual_start + window).min(self.lines.len());
        let visible_lines = &self.lines[actual_start..end];
        let highlight_palette = super::diff_text::syntax_highlight_palette(self.theme);
        let mut h = FxHasher::default();
        let mut visible_text_bytes = 0usize;
        let mut min_line_bytes = usize::MAX;
        for line in visible_lines {
            let styled = super::diff_text::build_cached_diff_styled_text_with_palette(
                self.theme,
                &highlight_palette,
                super::diff_text::DiffTextBuildRequest {
                    text: line,
                    word_ranges: &[],
                    query: "",
                    syntax: super::diff_text::DiffSyntaxConfig {
                        language: self.language,
                        mode: DiffSyntaxMode::Auto,
                    },
                    word_color: None,
                },
            );
            styled.text.len().hash(&mut h);
            styled.highlights.len().hash(&mut h);
            visible_text_bytes = visible_text_bytes.saturating_add(line.len());
            min_line_bytes = min_line_bytes.min(line.len());
        }
        (
            h.finish(),
            LargeFileDiffScrollMetrics {
                total_lines: bench_counter_u64(self.lines.len()),
                window_size: bench_counter_u64(visible_lines.len()),
                start_line: bench_counter_u64(actual_start),
                visible_text_bytes: bench_counter_u64(visible_text_bytes),
                min_line_bytes: bench_counter_u64(if visible_lines.is_empty() {
                    0
                } else {
                    min_line_bytes
                }),
                language_detected: u64::from(self.language.is_some()),
                syntax_mode_auto: 1,
            },
        )
    }
}

/// Synthetic visible-window scrolling fixture for the history list.
///
/// This keeps the expensive history-cache build outside the measured loop and
/// approximates the per-frame work of painting visible commit rows plus graph
/// lane state during sustained scrolling.
pub struct HistoryListScrollFixture {
    commits: Vec<Commit>,
    graph_rows: Vec<Arc<history_graph::GraphRow>>,
}

impl HistoryListScrollFixture {
    pub fn new(commits: usize, local_branches: usize, remote_branches: usize) -> Self {
        let commits = build_synthetic_commits_with_merge_stride(commits.max(1), 11, 5);
        let (branches, remote_branches) =
            build_branches_targeting_commits(&commits, local_branches, remote_branches);
        let graph_rows = history_graph::compute_graph(
            &commits,
            AppTheme::zed_ayu_dark(),
            branches
                .iter()
                .map(|branch| branch.target.as_ref())
                .chain(remote_branches.iter().map(|branch| branch.target.as_ref())),
            None,
        )
        .into_iter()
        .map(Arc::new)
        .collect();

        Self {
            commits,
            graph_rows,
        }
    }

    pub fn run_scroll_step(&self, start: usize, window: usize) -> u64 {
        let range = self.visible_range(start, window);
        let mut h = FxHasher::default();
        range.len().hash(&mut h);

        for ix in range {
            let commit = &self.commits[ix];
            let graph_row = &self.graph_rows[ix];
            commit.id.as_ref().hash(&mut h);
            commit.summary.hash(&mut h);
            commit.author.hash(&mut h);
            commit.parent_ids.len().hash(&mut h);
            (
                graph_row.lanes_now.len(),
                graph_row.lanes_next.len(),
                graph_row.joins_in.len(),
                graph_row.edges_out.len(),
                graph_row.is_merge,
            )
                .hash(&mut h);
        }

        h.finish()
    }

    fn visible_range(&self, start: usize, window: usize) -> Range<usize> {
        let window = window.max(1).min(self.commits.len());
        let max_start = self.commits.len().saturating_sub(window);
        let start = start.min(max_start);
        start..start + window
    }

    #[cfg(any(test, feature = "benchmarks"))]
    fn total_rows(&self) -> usize {
        self.commits.len()
    }
}

enum KeyboardArrowScrollScenario {
    History(HistoryListScrollFixture),
    Diff(LargeFileDiffScrollFixture),
}

pub struct KeyboardArrowScrollFixture {
    scenario: KeyboardArrowScrollScenario,
    total_rows: usize,
    window_rows: usize,
    scroll_step_rows: usize,
    repeat_events: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardArrowScrollMetrics {
    pub total_rows: u64,
    pub window_rows: u64,
    pub scroll_step_rows: u64,
    pub repeat_events: u64,
    pub rows_requested_total: u64,
    pub unique_windows_visited: u64,
    pub wrap_count: u64,
    pub final_start_row: u64,
}

impl KeyboardArrowScrollFixture {
    pub fn history(
        commits: usize,
        local_branches: usize,
        remote_branches: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        repeat_events: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let fixture = HistoryListScrollFixture::new(commits, local_branches, remote_branches);
        let total_rows = fixture.total_rows();
        Self {
            scenario: KeyboardArrowScrollScenario::History(fixture),
            total_rows,
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            repeat_events: repeat_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn diff(
        lines: usize,
        line_bytes: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        repeat_events: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let total_rows = lines.max(1);
        Self {
            scenario: KeyboardArrowScrollScenario::Diff(
                LargeFileDiffScrollFixture::new_with_line_bytes(total_rows, line_bytes.max(1)),
            ),
            total_rows,
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            repeat_events: repeat_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        KeyboardArrowScrollMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_step(&self, start: usize, window_rows: usize) -> u64 {
        match &self.scenario {
            KeyboardArrowScrollScenario::History(fixture) => {
                fixture.run_scroll_step(start, window_rows)
            }
            KeyboardArrowScrollScenario::Diff(fixture) => {
                fixture.run_scroll_step(start, window_rows)
            }
        }
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, KeyboardArrowScrollMetrics) {
        let window_rows = self.window_rows.max(1).min(self.total_rows.max(1));
        let scroll_step_rows = self.scroll_step_rows.max(1);
        let repeat_events = self.repeat_events.max(1);
        let max_start = self.total_rows.saturating_sub(window_rows);
        let mut seen_starts: HashSet<usize> = HashSet::default();
        let mut hash = 0u64;
        let mut start = 0usize;
        let mut wrap_count = 0u64;

        for _ in 0..repeat_events {
            seen_starts.insert(start);

            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= self.run_step(start, window_rows);
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= self.run_step(start, window_rows);
            }

            if max_start > 0 {
                let next = start.saturating_add(scroll_step_rows);
                if next > max_start {
                    wrap_count = wrap_count.saturating_add(1);
                    start = next % (max_start + 1);
                } else {
                    start = next;
                }
            }
        }

        (
            hash,
            KeyboardArrowScrollMetrics {
                total_rows: u64::try_from(self.total_rows).unwrap_or(u64::MAX),
                window_rows: u64::try_from(window_rows).unwrap_or(u64::MAX),
                scroll_step_rows: u64::try_from(scroll_step_rows).unwrap_or(u64::MAX),
                repeat_events: u64::try_from(repeat_events).unwrap_or(u64::MAX),
                rows_requested_total: u64::try_from(window_rows)
                    .unwrap_or(u64::MAX)
                    .saturating_mul(u64::try_from(repeat_events).unwrap_or(u64::MAX)),
                unique_windows_visited: u64::try_from(seen_starts.len()).unwrap_or(u64::MAX),
                wrap_count,
                final_start_row: u64::try_from(start).unwrap_or(u64::MAX),
            },
        )
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
enum KeyboardFocusNodeKind {
    RepoTab,
    TabBarSpacer,
    HistoryPanel,
    SidebarResizeHandle,
    DiffPanel,
    DetailsResizeHandle,
    CommitMessageInput,
    CommitShaInput,
    CommitDateInput,
    CommitParentInput,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug)]
struct KeyboardFocusNode {
    kind: KeyboardFocusNodeKind,
    label_len: usize,
    focusable: bool,
}

/// Fixture for `keyboard/tab_focus_cycle_all_panes`.
///
/// Models the tab-order traversal across the major focusable chrome in a
/// typical open-repo view: repo tabs, the history panel, the diff panel,
/// and the commit-details text inputs. Two structural nodes (the sidebar and
/// details split handles) are present but skipped because they are not tab
/// stops, so the benchmark measures both focus-target switching and the scan
/// needed to find the next focusable node.
pub struct KeyboardTabFocusCycleFixture {
    nodes: Vec<KeyboardFocusNode>,
    focusable_node_indices: Vec<usize>,
    repo_tab_count: usize,
    detail_input_count: usize,
    cycle_events: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardTabFocusCycleMetrics {
    pub focus_target_count: u64,
    pub repo_tab_count: u64,
    pub detail_input_count: u64,
    pub cycle_events: u64,
    pub unique_targets_visited: u64,
    pub wrap_count: u64,
    pub max_scan_len: u64,
    pub final_target_index: u64,
}

impl KeyboardTabFocusCycleFixture {
    pub fn all_panes(repo_tab_count: usize, cycle_events: usize, frame_budget_ns: u64) -> Self {
        let repo_tab_count = repo_tab_count.max(1);
        let detail_input_count = 4usize;
        let mut nodes = Vec::with_capacity(repo_tab_count + detail_input_count + 4);

        for ix in 0..repo_tab_count {
            nodes.push(KeyboardFocusNode {
                kind: KeyboardFocusNodeKind::RepoTab,
                label_len: format!("repo-tab-{ix:02}").len(),
                focusable: true,
            });
        }

        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::TabBarSpacer,
            label_len: 0,
            focusable: false,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::HistoryPanel,
            label_len: "History".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::SidebarResizeHandle,
            label_len: 0,
            focusable: false,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::DiffPanel,
            label_len: "Diff".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::DetailsResizeHandle,
            label_len: 0,
            focusable: false,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitMessageInput,
            label_len: "Commit message".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitShaInput,
            label_len: "Commit SHA".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitDateInput,
            label_len: "Commit date".len(),
            focusable: true,
        });
        nodes.push(KeyboardFocusNode {
            kind: KeyboardFocusNodeKind::CommitParentInput,
            label_len: "Parent commit".len(),
            focusable: true,
        });

        let focusable_node_indices = nodes
            .iter()
            .enumerate()
            .filter_map(|(ix, node)| node.focusable.then_some(ix))
            .collect::<Vec<_>>();

        Self {
            nodes,
            focusable_node_indices,
            repo_tab_count,
            detail_input_count,
            cycle_events: cycle_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        KeyboardTabFocusCycleMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn focus_node_hash(&self, focus_ix: usize) -> u64 {
        let node_ix = self.focusable_node_indices[focus_ix];
        let node = &self.nodes[node_ix];
        let mut hasher = FxHasher::default();
        focus_ix.hash(&mut hasher);
        node_ix.hash(&mut hasher);
        std::mem::discriminant(&node.kind).hash(&mut hasher);
        node.label_len.hash(&mut hasher);
        hasher.finish()
    }

    fn next_focus_target(&self, current_focus_ix: usize) -> (usize, usize, bool) {
        let current_node_ix = self.focusable_node_indices[current_focus_ix];
        let next_focus_ix = (current_focus_ix + 1) % self.focusable_node_indices.len();
        let next_node_ix = self.focusable_node_indices[next_focus_ix];
        let scan_len = if next_node_ix > current_node_ix {
            next_node_ix - current_node_ix
        } else {
            self.nodes.len() - current_node_ix + next_node_ix
        };
        (next_focus_ix, scan_len.max(1), next_focus_ix == 0)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, KeyboardTabFocusCycleMetrics) {
        let mut seen_targets: HashSet<usize> = HashSet::default();
        let mut hash = 0u64;
        let mut current_focus_ix = 0usize;
        let mut wrap_count = 0u64;
        let mut max_scan_len = 0usize;

        for _ in 0..self.cycle_events {
            seen_targets.insert(current_focus_ix);

            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= self.focus_node_hash(current_focus_ix);
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= self.focus_node_hash(current_focus_ix);
            }

            let (next_focus_ix, scan_len, wrapped) = self.next_focus_target(current_focus_ix);
            max_scan_len = max_scan_len.max(scan_len);
            if wrapped {
                wrap_count = wrap_count.saturating_add(1);
            }
            current_focus_ix = next_focus_ix;
        }

        (
            hash,
            KeyboardTabFocusCycleMetrics {
                focus_target_count: u64::try_from(self.focusable_node_indices.len())
                    .unwrap_or(u64::MAX),
                repo_tab_count: u64::try_from(self.repo_tab_count).unwrap_or(u64::MAX),
                detail_input_count: u64::try_from(self.detail_input_count).unwrap_or(u64::MAX),
                cycle_events: u64::try_from(self.cycle_events).unwrap_or(u64::MAX),
                unique_targets_visited: u64::try_from(seen_targets.len()).unwrap_or(u64::MAX),
                wrap_count,
                max_scan_len: u64::try_from(max_scan_len).unwrap_or(u64::MAX),
                final_target_index: u64::try_from(current_focus_ix).unwrap_or(u64::MAX),
            },
        )
    }
}

/// Fixture for `keyboard/stage_unstage_toggle_rapid`.
///
/// Uses a partially staged synthetic status list so the same path corpus exists
/// in both the unstaged and staged areas. Each keyboard event dispatches either
/// `StagePath` or `UnstagePath`, immediately followed by `SelectDiff` for the
/// same path in the opposite area to model rapid toggling between the two
/// keyboard actions while keeping the diff view active.
pub struct KeyboardStageUnstageToggleFixture {
    baseline: AppState,
    paths: Vec<std::path::PathBuf>,
    toggle_events: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct KeyboardStageUnstageToggleMetrics {
    pub path_count: u64,
    pub toggle_events: u64,
    pub effect_count: u64,
    pub stage_effect_count: u64,
    pub unstage_effect_count: u64,
    pub select_diff_effect_count: u64,
    pub ops_rev_delta: u64,
    pub diff_state_rev_delta: u64,
    pub area_flip_count: u64,
    pub path_wrap_count: u64,
}

impl KeyboardStageUnstageToggleFixture {
    pub fn rapid_toggle(path_count: usize, toggle_events: usize, frame_budget_ns: u64) -> Self {
        let path_count = path_count.max(1);
        let entries = build_synthetic_partially_staged_entries(path_count);
        let paths = entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<Vec<_>>();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged: entries.clone(),
            staged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());
        repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: paths[0].clone(),
            area: DiffArea::Unstaged,
        });
        repo.diff_state.diff_state_rev = 1;

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            toggle_events: toggle_events.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        KeyboardStageUnstageToggleMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, KeyboardStageUnstageToggleMetrics) {
        let mut state = self.fresh_state();
        let repo = &state.repos[0];
        let ops_rev_before = repo.ops_rev;
        let diff_state_rev_before = repo.diff_state.diff_state_rev;

        let mut hash = 0u64;
        let mut total_effects = 0u64;
        let mut stage_effect_count = 0u64;
        let mut unstage_effect_count = 0u64;
        let mut select_diff_effect_count = 0u64;
        let mut area = DiffArea::Unstaged;
        let mut path_ix = 0usize;
        let mut path_wrap_count = 0u64;

        for _ in 0..self.toggle_events {
            let path = self.paths[path_ix].clone();

            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= self.run_toggle_step(
                    &mut state,
                    area,
                    path.clone(),
                    &mut total_effects,
                    &mut stage_effect_count,
                    &mut unstage_effect_count,
                    &mut select_diff_effect_count,
                );
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= self.run_toggle_step(
                    &mut state,
                    area,
                    path.clone(),
                    &mut total_effects,
                    &mut stage_effect_count,
                    &mut unstage_effect_count,
                    &mut select_diff_effect_count,
                );
            }

            area = match area {
                DiffArea::Unstaged => DiffArea::Staged,
                DiffArea::Staged => DiffArea::Unstaged,
            };
            path_ix = (path_ix + 1) % self.paths.len();
            if path_ix == 0 {
                path_wrap_count = path_wrap_count.saturating_add(1);
            }
        }

        let repo = &state.repos[0];
        (
            hash,
            KeyboardStageUnstageToggleMetrics {
                path_count: u64::try_from(self.paths.len()).unwrap_or(u64::MAX),
                toggle_events: u64::try_from(self.toggle_events).unwrap_or(u64::MAX),
                effect_count: total_effects,
                stage_effect_count,
                unstage_effect_count,
                select_diff_effect_count,
                ops_rev_delta: repo.ops_rev.wrapping_sub(ops_rev_before),
                diff_state_rev_delta: repo
                    .diff_state
                    .diff_state_rev
                    .wrapping_sub(diff_state_rev_before),
                area_flip_count: u64::try_from(self.toggle_events).unwrap_or(u64::MAX),
                path_wrap_count,
            },
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn run_toggle_step(
        &self,
        state: &mut AppState,
        area: DiffArea,
        path: std::path::PathBuf,
        total_effects: &mut u64,
        stage_effect_count: &mut u64,
        unstage_effect_count: &mut u64,
        select_diff_effect_count: &mut u64,
    ) -> u64 {
        let repo_id = RepoId(1);
        let toggle_effects = match area {
            DiffArea::Unstaged => dispatch_sync(
                state,
                Msg::StagePath {
                    repo_id,
                    path: path.clone(),
                },
            ),
            DiffArea::Staged => dispatch_sync(
                state,
                Msg::UnstagePath {
                    repo_id,
                    path: path.clone(),
                },
            ),
        };

        let next_area = match area {
            DiffArea::Unstaged => DiffArea::Staged,
            DiffArea::Staged => DiffArea::Unstaged,
        };
        let select_effects = dispatch_sync(
            state,
            Msg::SelectDiff {
                repo_id,
                target: DiffTarget::WorkingTree {
                    path,
                    area: next_area,
                },
            },
        );

        let mut hasher = FxHasher::default();
        for effect in &toggle_effects {
            *total_effects = total_effects.saturating_add(1);
            match effect {
                Effect::StagePath { .. } | Effect::StagePaths { .. } => {
                    *stage_effect_count = stage_effect_count.saturating_add(1);
                }
                Effect::UnstagePath { .. } | Effect::UnstagePaths { .. } => {
                    *unstage_effect_count = unstage_effect_count.saturating_add(1);
                }
                _ => {}
            }
            std::mem::discriminant(effect).hash(&mut hasher);
        }
        for effect in &select_effects {
            *total_effects = total_effects.saturating_add(1);
            match effect {
                Effect::LoadDiff { .. }
                | Effect::LoadDiffFile { .. }
                | Effect::LoadDiffFileImage { .. } => {
                    *select_diff_effect_count = select_diff_effect_count.saturating_add(1);
                }
                _ => {}
            }
            std::mem::discriminant(effect).hash(&mut hasher);
        }

        state.repos[0].ops_rev.hash(&mut hasher);
        state.repos[0].diff_state.diff_state_rev.hash(&mut hasher);
        hasher.finish()
    }
}

// ---------------------------------------------------------------------------
// Frame-timing: sidebar resize drag sustained
// ---------------------------------------------------------------------------

/// Fixture for `frame_timing/sidebar_resize_drag_sustained`.
///
/// Runs `frames` drag-step updates on the sidebar pane boundary inside a
/// per-frame timing capture, measuring both the pane-clamp math cost and
/// the layout recomputation cost under sustained interaction. Each frame
/// performs one drag step (same work as `PaneResizeDragStepFixture::run`)
/// and records frame duration via `FrameTimingCapture`.
pub struct SidebarResizeDragSustainedFixture {
    inner: PaneResizeDragStepFixture,
    frames: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct SidebarResizeDragSustainedMetrics {
    pub frames: u64,
    pub steps_per_frame: u64,
    pub total_clamp_at_min: u64,
    pub total_clamp_at_max: u64,
}

impl SidebarResizeDragSustainedFixture {
    pub fn new(frames: usize, frame_budget_ns: u64) -> Self {
        Self {
            inner: PaneResizeDragStepFixture::new(PaneResizeTarget::Sidebar),
            frames: frames.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &mut self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        SidebarResizeDragSustainedMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &mut self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, SidebarResizeDragSustainedMetrics) {
        let mut combined_hash = 0u64;
        let mut total_clamp_at_min = 0u64;
        let mut total_clamp_at_max = 0u64;

        // Reset the inner fixture to starting state each invocation so the
        // benchmark is deterministic across iterations.
        self.inner = PaneResizeDragStepFixture::new(PaneResizeTarget::Sidebar);

        for _ in 0..self.frames {
            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                let (hash, metrics) = self.inner.run_with_metrics();
                capture.record_frame(frame_started.elapsed());
                combined_hash ^= hash;
                total_clamp_at_min = total_clamp_at_min.saturating_add(metrics.clamp_at_min_count);
                total_clamp_at_max = total_clamp_at_max.saturating_add(metrics.clamp_at_max_count);
            } else {
                let (hash, metrics) = self.inner.run_with_metrics();
                combined_hash ^= hash;
                total_clamp_at_min = total_clamp_at_min.saturating_add(metrics.clamp_at_min_count);
                total_clamp_at_max = total_clamp_at_max.saturating_add(metrics.clamp_at_max_count);
            }
        }

        (
            combined_hash,
            SidebarResizeDragSustainedMetrics {
                frames: u64::try_from(self.frames).unwrap_or(u64::MAX),
                steps_per_frame: 200, // PaneResizeDragStepFixture default
                total_clamp_at_min,
                total_clamp_at_max,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Frame-timing: rapid commit selection changes
// ---------------------------------------------------------------------------

/// Fixture for `frame_timing/rapid_commit_selection_changes`.
///
/// Builds `commit_count` synthetic commit details and cycles through them
/// in a round-robin pattern, measuring per-frame cost of replacing the
/// selected commit details. This captures the interactive cost of rapidly
/// arrowing through the history list where each selection triggers a full
/// commit-details replacement render.
pub struct RapidCommitSelectionFixture {
    commits: Vec<CommitDetails>,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct RapidCommitSelectionMetrics {
    pub commit_count: u64,
    pub files_per_commit: u64,
    pub selections: u64,
}

impl RapidCommitSelectionFixture {
    pub fn new(commit_count: usize, files_per_commit: usize, frame_budget_ns: u64) -> Self {
        let commits: Vec<CommitDetails> = (0..commit_count.max(2))
            .map(|ix| {
                // Each commit gets a unique 40-char hex ID by zero-padding the index.
                let mut details = build_synthetic_commit_details(files_per_commit, 4);
                details.id = CommitId(format!("{ix:040x}").into());
                details
            })
            .collect();

        Self {
            commits,
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        RapidCommitSelectionMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, RapidCommitSelectionMetrics) {
        let mut hash = 0u64;
        let count = self.commits.len();

        // Walk through all commits, simulating rapid selection change.
        // For each pair (prev, next), we hash prev (warm/discard) then hash next
        // (the replacement cost — the measured work).
        for ix in 0..count {
            let current = &self.commits[ix];
            if let Some(capture) = capture.as_deref_mut() {
                let frame_started = std::time::Instant::now();
                hash ^= commit_details_row_hash(current);
                capture.record_frame(frame_started.elapsed());
            } else {
                hash ^= commit_details_row_hash(current);
            }
        }

        (
            hash,
            RapidCommitSelectionMetrics {
                commit_count: u64::try_from(count).unwrap_or(u64::MAX),
                files_per_commit: self
                    .commits
                    .first()
                    .map(|c| u64::try_from(c.files.len()).unwrap_or(u64::MAX))
                    .unwrap_or(0),
                selections: u64::try_from(count).unwrap_or(u64::MAX),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Frame-timing: repo switch during scroll
// ---------------------------------------------------------------------------

/// Fixture for `frame_timing/repo_switch_during_scroll`.
///
/// Interleaves history-list scroll steps with periodic repo switches,
/// measuring per-frame timing for the combined interaction. Every
/// `switch_every_n_frames` frames, a repo switch is performed (via
/// `RepoSwitchFixture::run_with_state`) instead of a scroll step. This
/// captures the jank risk of switching repos while scrolling through
/// the history list.
pub struct RepoSwitchDuringScrollFixture {
    history_fixture: HistoryListScrollFixture,
    repo_switch_fixture: RepoSwitchFixture,
    frames: usize,
    window_rows: usize,
    scroll_step_rows: usize,
    switch_every_n_frames: usize,
    frame_budget_ns: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
pub struct RepoSwitchDuringScrollMetrics {
    pub total_frames: u64,
    pub scroll_frames: u64,
    pub switch_frames: u64,
    pub total_rows: u64,
    pub window_rows: u64,
}

impl RepoSwitchDuringScrollFixture {
    pub fn new(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        frames: usize,
        switch_every_n_frames: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let history_fixture =
            HistoryListScrollFixture::new(history_commits, local_branches, remote_branches);

        // Two-hot-repos switch: models the common case of switching between
        // two active repositories.
        let repo_switch_fixture = RepoSwitchFixture::two_hot_repos(
            history_commits.min(1_000),
            local_branches.min(20),
            remote_branches.min(60),
            4,
        );

        Self {
            history_fixture,
            repo_switch_fixture,
            frames: frames.max(1),
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            switch_every_n_frames: switch_every_n_frames.max(1),
            frame_budget_ns: frame_budget_ns.max(1),
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(
        &self,
    ) -> (
        u64,
        crate::view::perf::FrameTimingStats,
        RepoSwitchDuringScrollMetrics,
    ) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, RepoSwitchDuringScrollMetrics) {
        let total_rows = self.history_fixture.total_rows();
        let window_rows = self.window_rows.min(total_rows.max(1));
        let max_start = total_rows.saturating_sub(window_rows);
        let mut hash = 0u64;
        let mut start = 0usize;
        let mut scroll_frames = 0u64;
        let mut switch_frames = 0u64;
        let mut repo_state = self.repo_switch_fixture.fresh_state();

        for frame_ix in 0..self.frames {
            let is_switch_frame = frame_ix > 0 && frame_ix % self.switch_every_n_frames == 0;

            if is_switch_frame {
                // Repo switch frame
                if let Some(capture) = capture.as_deref_mut() {
                    let frame_started = std::time::Instant::now();
                    let (switch_hash, _metrics) =
                        self.repo_switch_fixture.run_with_state(&mut repo_state);
                    capture.record_frame(frame_started.elapsed());
                    hash ^= switch_hash;
                } else {
                    let (switch_hash, _metrics) =
                        self.repo_switch_fixture.run_with_state(&mut repo_state);
                    hash ^= switch_hash;
                }
                switch_frames += 1;
                // Reset repo state for next switch
                repo_state = self.repo_switch_fixture.fresh_state();
            } else {
                // Scroll frame
                if let Some(capture) = capture.as_deref_mut() {
                    let frame_started = std::time::Instant::now();
                    hash ^= self.history_fixture.run_scroll_step(start, window_rows);
                    capture.record_frame(frame_started.elapsed());
                } else {
                    hash ^= self.history_fixture.run_scroll_step(start, window_rows);
                }
                scroll_frames += 1;

                if max_start > 0 {
                    start = start.saturating_add(self.scroll_step_rows);
                    if start > max_start {
                        start %= max_start + 1;
                    }
                }
            }
        }

        (
            hash,
            RepoSwitchDuringScrollMetrics {
                total_frames: u64::try_from(self.frames).unwrap_or(u64::MAX),
                scroll_frames,
                switch_frames,
                total_rows: u64::try_from(total_rows).unwrap_or(u64::MAX),
                window_rows: u64::try_from(window_rows).unwrap_or(u64::MAX),
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Staging benchmarks — reducer dispatch cost of stage / unstage operations
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StagingScenario {
    /// Dispatch `Msg::StagePaths` with all paths in one batch.
    StageAll,
    /// Dispatch `Msg::UnstagePaths` with all paths in one batch.
    UnstageAll,
    /// Alternate `Msg::StagePath` / `Msg::UnstagePath` for each path.
    Interleaved,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StagingMetrics {
    pub file_count: u64,
    pub effect_count: u64,
    pub ops_rev_delta: u64,
    pub local_actions_delta: u64,
    pub stage_effect_count: u64,
    pub unstage_effect_count: u64,
}

pub struct StagingFixture {
    baseline: AppState,
    paths: Vec<std::path::PathBuf>,
    scenario: StagingScenario,
}

impl StagingFixture {
    pub fn stage_all(file_count: usize) -> Self {
        let entries = build_synthetic_status_entries(file_count.max(1), DiffArea::Unstaged);
        let paths: Vec<std::path::PathBuf> = entries.iter().map(|e| e.path.clone()).collect();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            scenario: StagingScenario::StageAll,
        }
    }

    pub fn unstage_all(file_count: usize) -> Self {
        let entries = build_synthetic_status_entries(file_count.max(1), DiffArea::Staged);
        let paths: Vec<std::path::PathBuf> = entries.iter().map(|e| e.path.clone()).collect();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            staged: entries,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            scenario: StagingScenario::UnstageAll,
        }
    }

    pub fn interleaved(file_count: usize) -> Self {
        // Start with half unstaged, half staged — toggle operations will alternate.
        let half = file_count.max(2) / 2;
        let unstaged = build_synthetic_status_entries(half, DiffArea::Unstaged);
        let staged = build_synthetic_status_entries(half, DiffArea::Staged);
        let paths: Vec<std::path::PathBuf> = unstaged
            .iter()
            .map(|e| e.path.clone())
            .chain(staged.iter().map(|e| e.path.clone()))
            .collect();

        let commits = build_synthetic_commits(100);
        let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
        repo.status = Loadable::Ready(Arc::new(RepoStatus {
            unstaged,
            staged,
            ..RepoStatus::default()
        }));
        repo.status_rev = 1;
        repo.open = Loadable::Ready(());

        Self {
            baseline: AppState {
                repos: vec![repo],
                active_repo: Some(RepoId(1)),
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
            paths,
            scenario: StagingScenario::Interleaved,
        }
    }

    pub fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&self) -> (u64, StagingMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, StagingMetrics) {
        let ops_rev_before = state.repos[0].ops_rev;
        let actions_before = state.repos[0].local_actions_in_flight;

        let mut total_effects = 0u64;
        let mut stage_effect_count = 0u64;
        let mut unstage_effect_count = 0u64;
        let mut h = FxHasher::default();

        match self.scenario {
            StagingScenario::StageAll => {
                let effects = dispatch_sync(
                    state,
                    Msg::StagePaths {
                        repo_id: RepoId(1),
                        paths: self.paths.clone(),
                    },
                );
                total_effects += effects.len() as u64;
                for effect in &effects {
                    match effect {
                        Effect::StagePaths { .. } | Effect::StagePath { .. } => {
                            stage_effect_count += 1;
                        }
                        _ => {}
                    }
                    std::mem::discriminant(effect).hash(&mut h);
                }
            }
            StagingScenario::UnstageAll => {
                let effects = dispatch_sync(
                    state,
                    Msg::UnstagePaths {
                        repo_id: RepoId(1),
                        paths: self.paths.clone(),
                    },
                );
                total_effects += effects.len() as u64;
                for effect in &effects {
                    match effect {
                        Effect::UnstagePaths { .. } | Effect::UnstagePath { .. } => {
                            unstage_effect_count += 1;
                        }
                        _ => {}
                    }
                    std::mem::discriminant(effect).hash(&mut h);
                }
            }
            StagingScenario::Interleaved => {
                for (ix, path) in self.paths.iter().enumerate() {
                    let effects = if ix % 2 == 0 {
                        dispatch_sync(
                            state,
                            Msg::StagePath {
                                repo_id: RepoId(1),
                                path: path.clone(),
                            },
                        )
                    } else {
                        dispatch_sync(
                            state,
                            Msg::UnstagePath {
                                repo_id: RepoId(1),
                                path: path.clone(),
                            },
                        )
                    };
                    total_effects += effects.len() as u64;
                    for effect in &effects {
                        match effect {
                            Effect::StagePaths { .. } | Effect::StagePath { .. } => {
                                stage_effect_count += 1;
                            }
                            Effect::UnstagePaths { .. } | Effect::UnstagePath { .. } => {
                                unstage_effect_count += 1;
                            }
                            _ => {}
                        }
                        std::mem::discriminant(effect).hash(&mut h);
                    }
                }
            }
        }

        let ops_rev_after = state.repos[0].ops_rev;
        let actions_after = state.repos[0].local_actions_in_flight;

        state.repos[0].ops_rev.hash(&mut h);
        state.repos[0].local_actions_in_flight.hash(&mut h);
        total_effects.hash(&mut h);

        let metrics = StagingMetrics {
            file_count: self.paths.len() as u64,
            effect_count: total_effects,
            ops_rev_delta: ops_rev_after.wrapping_sub(ops_rev_before),
            local_actions_delta: actions_after.wrapping_sub(actions_before) as u64,
            stage_effect_count,
            unstage_effect_count,
        };

        (h.finish(), metrics)
    }
}

// ---------------------------------------------------------------------------
// Undo/redo — conflict resolution deep stack and undo-replay benchmarks
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
pub enum UndoRedoScenario {
    /// Apply a `ConflictSetRegionChoice` to every region in a deep session.
    DeepStack,
    /// Apply N region choices, reset all, then replay the same N choices.
    UndoReplay,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct UndoRedoMetrics {
    pub region_count: u64,
    pub apply_dispatches: u64,
    pub reset_dispatches: u64,
    pub replay_dispatches: u64,
    pub conflict_rev_delta: u64,
    pub total_effects: u64,
}

pub struct UndoRedoFixture {
    baseline: AppState,
    conflict_path: std::path::PathBuf,
    region_count: usize,
    scenario: UndoRedoScenario,
}

impl UndoRedoFixture {
    /// Deep stack: apply a choice to every region in a session with `region_count`
    /// conflict regions. Measures the cost of N sequential `ConflictSetRegionChoice`
    /// dispatches building up resolver state.
    pub fn deep_stack(region_count: usize) -> Self {
        let (baseline, conflict_path) = build_undo_redo_baseline(region_count);
        Self {
            baseline,
            conflict_path,
            region_count,
            scenario: UndoRedoScenario::DeepStack,
        }
    }

    /// Undo-replay: apply `region_count` choices, reset all via
    /// `ConflictResetResolutions`, then replay the same choices.
    /// Measures the full undo + replay cycle cost.
    pub fn undo_replay(region_count: usize) -> Self {
        let (baseline, conflict_path) = build_undo_redo_baseline(region_count);
        Self {
            baseline,
            conflict_path,
            region_count,
            scenario: UndoRedoScenario::UndoReplay,
        }
    }

    pub fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&self) -> (u64, UndoRedoMetrics) {
        let mut state = self.fresh_state();
        self.run_with_state(&mut state)
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, UndoRedoMetrics) {
        let conflict_rev_before = state.repos[0].conflict_state.conflict_rev;

        let mut total_effects = 0u64;
        let mut apply_dispatches = 0u64;
        let mut reset_dispatches = 0u64;
        let mut replay_dispatches = 0u64;
        let mut h = FxHasher::default();

        let choices = [
            gitcomet_state::msg::ConflictRegionChoice::Ours,
            gitcomet_state::msg::ConflictRegionChoice::Theirs,
            gitcomet_state::msg::ConflictRegionChoice::Both,
            gitcomet_state::msg::ConflictRegionChoice::Base,
        ];

        match self.scenario {
            UndoRedoScenario::DeepStack => {
                // Apply one choice per region, cycling through choice variants.
                for i in 0..self.region_count {
                    let effects = dispatch_sync(
                        state,
                        Msg::ConflictSetRegionChoice {
                            repo_id: RepoId(1),
                            path: self.conflict_path.clone(),
                            region_index: i,
                            choice: choices[i % choices.len()],
                        },
                    );
                    total_effects += effects.len() as u64;
                    apply_dispatches += 1;
                    for effect in &effects {
                        std::mem::discriminant(effect).hash(&mut h);
                    }
                }
            }
            UndoRedoScenario::UndoReplay => {
                // Phase 1: Apply choices.
                for i in 0..self.region_count {
                    let effects = dispatch_sync(
                        state,
                        Msg::ConflictSetRegionChoice {
                            repo_id: RepoId(1),
                            path: self.conflict_path.clone(),
                            region_index: i,
                            choice: choices[i % choices.len()],
                        },
                    );
                    total_effects += effects.len() as u64;
                    apply_dispatches += 1;
                    for effect in &effects {
                        std::mem::discriminant(effect).hash(&mut h);
                    }
                }

                // Phase 2: Reset all resolutions (undo).
                let effects = dispatch_sync(
                    state,
                    Msg::ConflictResetResolutions {
                        repo_id: RepoId(1),
                        path: self.conflict_path.clone(),
                    },
                );
                total_effects += effects.len() as u64;
                reset_dispatches += 1;
                for effect in &effects {
                    std::mem::discriminant(effect).hash(&mut h);
                }

                // Phase 3: Replay the same choices.
                for i in 0..self.region_count {
                    let effects = dispatch_sync(
                        state,
                        Msg::ConflictSetRegionChoice {
                            repo_id: RepoId(1),
                            path: self.conflict_path.clone(),
                            region_index: i,
                            choice: choices[i % choices.len()],
                        },
                    );
                    total_effects += effects.len() as u64;
                    replay_dispatches += 1;
                    for effect in &effects {
                        std::mem::discriminant(effect).hash(&mut h);
                    }
                }
            }
        }

        let conflict_rev_after = state.repos[0].conflict_state.conflict_rev;
        conflict_rev_after.hash(&mut h);
        total_effects.hash(&mut h);

        let metrics = UndoRedoMetrics {
            region_count: self.region_count as u64,
            apply_dispatches,
            reset_dispatches,
            replay_dispatches,
            conflict_rev_delta: conflict_rev_after.wrapping_sub(conflict_rev_before),
            total_effects,
        };

        (h.finish(), metrics)
    }
}

/// Build an `AppState` with a conflict session containing `region_count` unresolved regions.
fn build_undo_redo_baseline(region_count: usize) -> (AppState, std::path::PathBuf) {
    use gitcomet_core::conflict_session::{
        ConflictPayload, ConflictRegion, ConflictRegionResolution, ConflictRegionText,
        ConflictSession,
    };
    use gitcomet_core::domain::FileConflictKind;

    let conflict_path = std::path::PathBuf::from("src/conflict_undo_redo.rs");

    // Build a full-text-resolver session with N synthetic conflict regions.
    let base_text: Arc<str> = Arc::from("base content\n");
    let ours_text: Arc<str> = Arc::from("ours content\n");
    let theirs_text: Arc<str> = Arc::from("theirs content\n");

    let mut session = ConflictSession::new(
        conflict_path.clone(),
        FileConflictKind::BothModified,
        ConflictPayload::Text(Arc::clone(&base_text)),
        ConflictPayload::Text(Arc::clone(&ours_text)),
        ConflictPayload::Text(Arc::clone(&theirs_text)),
    );

    // Populate with N synthetic conflict regions.
    session.regions.clear();
    for i in 0..region_count {
        session.regions.push(ConflictRegion {
            base: Some(ConflictRegionText::from(format!(
                "base region {i} content line\n"
            ))),
            ours: ConflictRegionText::from(format!("ours region {i} modified line\n")),
            theirs: ConflictRegionText::from(format!("theirs region {i} modified line\n")),
            resolution: ConflictRegionResolution::Unresolved,
        });
    }

    let commits = build_synthetic_commits(100);
    let mut repo = build_synthetic_repo_state(20, 40, 2, 0, 0, 0, &commits);
    repo.conflict_state.conflict_file_path = Some(conflict_path.clone());
    repo.conflict_state.conflict_session = Some(session);
    repo.conflict_state.conflict_rev = 1;
    repo.open = Loadable::Ready(());

    let baseline = AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        clone: None,
        notifications: Vec::new(),
        banner_error: None,
        auth_prompt: None,
    };

    (baseline, conflict_path)
}

pub struct ReplacementAlignmentFixture {
    old_text: String,
    new_text: String,
}

impl ReplacementAlignmentFixture {
    pub fn new(
        blocks: usize,
        old_block_lines: usize,
        new_block_lines: usize,
        context_lines: usize,
        line_bytes: usize,
    ) -> Self {
        let (old_text, new_text) = build_synthetic_replacement_alignment_documents(
            blocks,
            old_block_lines,
            new_block_lines,
            context_lines,
            line_bytes,
        );
        Self { old_text, new_text }
    }

    pub fn run_plan_step(&self) -> u64 {
        let plan = gitcomet_core::file_diff::side_by_side_plan(&self.old_text, &self.new_text);
        hash_file_diff_plan(&plan)
    }

    pub fn run_plan_step_with_backend(
        &self,
        backend: gitcomet_core::file_diff::BenchmarkReplacementDistanceBackend,
    ) -> u64 {
        let plan = gitcomet_core::file_diff::benchmark_side_by_side_plan_with_replacement_backend(
            &self.old_text,
            &self.new_text,
            backend,
        );
        hash_file_diff_plan(&plan)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputPrepaintWindowedMetrics {
    pub total_lines: u64,
    pub viewport_rows: u64,
    pub guard_rows: u64,
    pub max_shape_bytes: u64,
    pub cache_entries_after: u64,
    pub cache_hits: u64,
    pub cache_misses: u64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TextInputShapeCacheKey {
    line_ix: usize,
    wrap_width_key: i32,
    style_epoch: u64,
    text_hash_slice: u64,
}

pub struct TextInputPrepaintWindowedFixture {
    lines: Vec<String>,
    wrap_width_key: i32,
    style_epoch: u64,
    guard_rows: usize,
    max_shape_bytes: usize,
    shape_cache: HashMap<TextInputShapeCacheKey, u64>,
}

impl TextInputPrepaintWindowedFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        Self {
            lines: build_synthetic_source_lines(lines.max(1), line_bytes),
            wrap_width_key: wrap_width_px.max(1) as i32,
            style_epoch: 1,
            guard_rows: 2,
            max_shape_bytes: 4 * 1024,
            shape_cache: HashMap::default(),
        }
    }

    pub fn run_windowed_step(&mut self, start_row: usize, viewport_rows: usize) -> u64 {
        if self.lines.is_empty() || viewport_rows == 0 {
            return 0;
        }

        let line_count = self.lines.len();
        let total_rows = viewport_rows
            .saturating_add(self.guard_rows.saturating_mul(2))
            .max(1);
        let mut h = FxHasher::default();

        for row in 0..total_rows {
            let line_ix = start_row.wrapping_add(row) % line_count;
            let (slice_hash, capped_len) = hash_text_input_shaping_slice(
                self.lines.get(line_ix).map(String::as_str).unwrap_or(""),
                self.max_shape_bytes,
            );
            let key = TextInputShapeCacheKey {
                line_ix,
                wrap_width_key: self.wrap_width_key,
                style_epoch: self.style_epoch,
                text_hash_slice: slice_hash,
            };
            let shaped = *self.shape_cache.entry(key).or_insert_with(|| {
                let mut shaped_hash = FxHasher::default();
                line_ix.hash(&mut shaped_hash);
                capped_len.hash(&mut shaped_hash);
                slice_hash.hash(&mut shaped_hash);
                shaped_hash.finish()
            });
            shaped.hash(&mut h);
        }

        self.shape_cache.len().hash(&mut h);
        h.finish()
    }

    pub fn run_windowed_step_with_metrics(
        &mut self,
        start_row: usize,
        viewport_rows: usize,
    ) -> (u64, TextInputPrepaintWindowedMetrics) {
        if self.lines.is_empty() || viewport_rows == 0 {
            return (0, TextInputPrepaintWindowedMetrics::default());
        }

        let line_count = self.lines.len();
        let total_rows = viewport_rows
            .saturating_add(self.guard_rows.saturating_mul(2))
            .max(1);
        let mut h = FxHasher::default();
        let cache_before = self.shape_cache.len();

        for row in 0..total_rows {
            let line_ix = start_row.wrapping_add(row) % line_count;
            let (slice_hash, capped_len) = hash_text_input_shaping_slice(
                self.lines.get(line_ix).map(String::as_str).unwrap_or(""),
                self.max_shape_bytes,
            );
            let key = TextInputShapeCacheKey {
                line_ix,
                wrap_width_key: self.wrap_width_key,
                style_epoch: self.style_epoch,
                text_hash_slice: slice_hash,
            };
            let shaped = *self.shape_cache.entry(key).or_insert_with(|| {
                let mut shaped_hash = FxHasher::default();
                line_ix.hash(&mut shaped_hash);
                capped_len.hash(&mut shaped_hash);
                slice_hash.hash(&mut shaped_hash);
                shaped_hash.finish()
            });
            shaped.hash(&mut h);
        }

        self.shape_cache.len().hash(&mut h);
        let cache_after = self.shape_cache.len();
        let cache_misses = cache_after.saturating_sub(cache_before);
        let cache_hits = total_rows.saturating_sub(cache_misses);

        (
            h.finish(),
            TextInputPrepaintWindowedMetrics {
                total_lines: bench_counter_u64(line_count),
                viewport_rows: bench_counter_u64(viewport_rows),
                guard_rows: bench_counter_u64(self.guard_rows),
                max_shape_bytes: bench_counter_u64(self.max_shape_bytes),
                cache_entries_after: bench_counter_u64(cache_after),
                cache_hits: bench_counter_u64(cache_hits),
                cache_misses: bench_counter_u64(cache_misses),
            },
        )
    }

    pub fn run_full_document_step(&mut self) -> u64 {
        self.run_windowed_step(0, self.lines.len())
    }

    pub fn run_full_document_step_with_metrics(
        &mut self,
    ) -> (u64, TextInputPrepaintWindowedMetrics) {
        let len = self.lines.len();
        self.run_windowed_step_with_metrics(0, len)
    }

    pub fn total_rows(&self) -> usize {
        self.lines.len()
    }

    #[cfg(test)]
    fn cache_entries(&self) -> usize {
        self.shape_cache.len()
    }
}

pub struct TextInputLongLineCapFixture {
    line: String,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputLongLineCapMetrics {
    pub line_bytes: u64,
    pub max_shape_bytes: u64,
    pub capped_len: u64,
    pub iterations: u64,
    pub cap_active: u64,
}

impl TextInputLongLineCapFixture {
    pub fn new(bytes: usize) -> Self {
        let bytes = bytes.max(1);
        let mut line = String::with_capacity(bytes.saturating_add(16));
        let token = "let very_long_identifier = \"token\"; ";
        while line.len() < bytes {
            line.push_str(token);
        }
        line.truncate(bytes);
        Self { line }
    }

    pub fn run_with_cap(&self, max_bytes: usize) -> u64 {
        let mut h = FxHasher::default();
        for nonce in 0..64usize {
            let (slice_hash, capped_len) =
                hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
            nonce.hash(&mut h);
            slice_hash.hash(&mut h);
            capped_len.hash(&mut h);
        }
        h.finish()
    }

    pub fn run_with_cap_with_metrics(
        &self,
        max_bytes: usize,
    ) -> (u64, TextInputLongLineCapMetrics) {
        let hash = self.run_with_cap(max_bytes);
        let (_h, capped_len) = hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
        let cap_active = if capped_len < self.line.len() { 1 } else { 0 };
        (
            hash,
            TextInputLongLineCapMetrics {
                line_bytes: self.line.len() as u64,
                max_shape_bytes: max_bytes as u64,
                capped_len: capped_len as u64,
                iterations: 64,
                cap_active,
            },
        )
    }

    pub fn run_without_cap_with_metrics(&self) -> (u64, TextInputLongLineCapMetrics) {
        self.run_with_cap_with_metrics(self.line.len().saturating_add(8))
    }

    pub fn run_without_cap(&self) -> u64 {
        self.run_with_cap(self.line.len().saturating_add(8))
    }

    #[cfg(test)]
    fn capped_len(&self, max_bytes: usize) -> usize {
        let (_hash, len) = hash_text_input_shaping_slice(self.line.as_str(), max_bytes.max(1));
        len
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextInputHighlightDensity {
    Dense,
    Sparse,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputRunsStreamedHighlightMetrics {
    pub total_lines: u64,
    pub visible_rows: u64,
    pub scroll_step: u64,
    pub total_highlights: u64,
    pub visible_highlights: u64,
    pub visible_lines_with_highlights: u64,
    pub density_dense: u64,
    pub algorithm_streamed: u64,
}

pub struct TextInputRunsStreamedHighlightFixture {
    text: String,
    line_starts: Vec<usize>,
    highlights: Vec<(Range<usize>, gpui::HighlightStyle)>,
    density: TextInputHighlightDensity,
    visible_rows: usize,
    scroll_step: usize,
}

impl TextInputRunsStreamedHighlightFixture {
    pub fn new(
        lines: usize,
        line_bytes: usize,
        visible_rows: usize,
        density: TextInputHighlightDensity,
    ) -> Self {
        let source_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(24));
        let text = source_lines.join("\n");
        let line_starts = line_starts_for_text(text.as_str());
        let highlights =
            build_text_input_streamed_highlights(text.as_str(), line_starts.as_slice(), density);
        let visible_rows = visible_rows.max(1).min(line_starts.len().max(1));
        let scroll_step = (visible_rows / 2).max(1);
        Self {
            text,
            line_starts,
            highlights,
            density,
            visible_rows,
            scroll_step,
        }
    }

    fn max_start_row(&self) -> usize {
        self.line_starts.len().saturating_sub(self.visible_rows)
    }

    fn visible_range(&self, start_row: usize) -> Range<usize> {
        if self.line_starts.is_empty() {
            return 0..0;
        }
        let max_start = self.max_start_row();
        let start = if max_start == 0 {
            0
        } else {
            start_row % (max_start + 1)
        };
        start..start.saturating_add(self.visible_rows)
    }

    fn line_byte_range(&self, line_ix: usize) -> Range<usize> {
        let start = self
            .line_starts
            .get(line_ix)
            .copied()
            .unwrap_or(self.text.len());
        let mut end = self
            .line_starts
            .get(line_ix + 1)
            .copied()
            .unwrap_or(self.text.len());
        if end > start && self.text.as_bytes().get(end - 1) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        start..end
    }

    fn metrics_for_visible_range(
        &self,
        visible_range: Range<usize>,
        algorithm_streamed: bool,
    ) -> TextInputRunsStreamedHighlightMetrics {
        // Highlights are generated line-by-line and sorted by start offset, so a
        // monotonic scan preserves deterministic counts without rescanning the
        // whole highlight list for every visible line.
        let mut highlight_ix = 0usize;
        let mut visible_highlights = 0usize;
        let mut visible_lines_with_highlights = 0usize;

        for line_ix in visible_range.clone() {
            let line_range = self.line_byte_range(line_ix);

            while self
                .highlights
                .get(highlight_ix)
                .map(|(range, _)| range.end <= line_range.start)
                .unwrap_or(false)
            {
                highlight_ix += 1;
            }

            let mut scan_ix = highlight_ix;
            let mut line_has_highlight = false;
            while let Some((range, _)) = self.highlights.get(scan_ix) {
                if range.start >= line_range.end {
                    break;
                }
                if range.end > line_range.start {
                    visible_highlights += 1;
                    line_has_highlight = true;
                }
                scan_ix += 1;
            }

            if line_has_highlight {
                visible_lines_with_highlights += 1;
            }
            highlight_ix = scan_ix;
        }

        TextInputRunsStreamedHighlightMetrics {
            total_lines: bench_counter_u64(self.line_starts.len()),
            visible_rows: bench_counter_u64(visible_range.len()),
            scroll_step: bench_counter_u64(self.scroll_step),
            total_highlights: bench_counter_u64(self.highlights.len()),
            visible_highlights: bench_counter_u64(visible_highlights),
            visible_lines_with_highlights: bench_counter_u64(visible_lines_with_highlights),
            density_dense: if matches!(self.density, TextInputHighlightDensity::Dense) {
                1
            } else {
                0
            },
            algorithm_streamed: if algorithm_streamed { 1 } else { 0 },
        }
    }

    pub fn run_legacy_step(&self, start_row: usize) -> u64 {
        benchmark_text_input_runs_legacy_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            self.visible_range(start_row),
            self.highlights.as_slice(),
        )
    }

    pub fn run_legacy_step_with_metrics(
        &self,
        start_row: usize,
    ) -> (u64, TextInputRunsStreamedHighlightMetrics) {
        let visible_range = self.visible_range(start_row);
        let hash = benchmark_text_input_runs_legacy_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            visible_range.clone(),
            self.highlights.as_slice(),
        );
        (hash, self.metrics_for_visible_range(visible_range, false))
    }

    pub fn run_streamed_step(&self, start_row: usize) -> u64 {
        benchmark_text_input_runs_streamed_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            self.visible_range(start_row),
            self.highlights.as_slice(),
        )
    }

    pub fn run_streamed_step_with_metrics(
        &self,
        start_row: usize,
    ) -> (u64, TextInputRunsStreamedHighlightMetrics) {
        let visible_range = self.visible_range(start_row);
        let hash = benchmark_text_input_runs_streamed_visible_window(
            self.text.as_str(),
            self.line_starts.as_slice(),
            visible_range.clone(),
            self.highlights.as_slice(),
        );
        (hash, self.metrics_for_visible_range(visible_range, true))
    }

    pub fn next_start_row(&self, start_row: usize) -> usize {
        let max_start = self.max_start_row().max(1);
        start_row.wrapping_add(self.scroll_step) % (max_start + 1)
    }

    #[cfg(test)]
    fn highlights_len(&self) -> usize {
        self.highlights.len()
    }
}

pub struct TextInputWrapIncrementalTabsFixture {
    lines: Vec<String>,
    row_counts: Vec<usize>,
    wrap_columns: usize,
    edit_nonce: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputWrapIncrementalTabsMetrics {
    pub total_lines: u64,
    pub line_bytes: u64,
    pub wrap_columns: u64,
    pub edit_line_ix: u64,
    pub dirty_lines: u64,
    pub total_rows_after: u64,
    pub recomputed_lines: u64,
    pub incremental_patch: u64,
}

impl TextInputWrapIncrementalTabsFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        let lines = build_synthetic_tabbed_source_lines(lines.max(1), line_bytes.max(8));
        let wrap_columns = wrap_columns_for_benchmark_width(wrap_width_px.max(1));
        let row_counts = lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), wrap_columns))
            .collect::<Vec<_>>();
        Self {
            lines,
            row_counts,
            wrap_columns,
            edit_nonce: 0,
        }
    }

    fn normalized_line_ix(&self, edit_line_ix: usize) -> usize {
        if self.lines.is_empty() {
            0
        } else {
            edit_line_ix % self.lines.len()
        }
    }

    fn apply_edit(&mut self, edit_line_ix: usize) -> (usize, usize, Range<usize>) {
        if self.lines.is_empty() {
            return (0, 0, 0..0);
        }

        let line_ix = self.normalized_line_ix(edit_line_ix);
        let edit_col = mutate_tabbed_line_for_wrap_patch(
            self.lines.get_mut(line_ix).expect("line index must exist"),
            self.edit_nonce,
        );
        self.edit_nonce = self.edit_nonce.wrapping_add(1);
        let line_bytes = self.lines.get(line_ix).map(String::len).unwrap_or(0);
        let dirty = expand_tabbed_dirty_line_range(
            self.lines.as_slice(),
            line_ix,
            edit_col,
            self.wrap_columns,
        );
        (line_ix, line_bytes, dirty)
    }

    fn metrics_for_step(
        &self,
        line_ix: usize,
        line_bytes: usize,
        dirty: &Range<usize>,
        recomputed_lines: usize,
        incremental_patch: bool,
    ) -> TextInputWrapIncrementalTabsMetrics {
        let total_rows_after = self.row_counts.iter().copied().sum::<usize>();
        TextInputWrapIncrementalTabsMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            line_bytes: bench_counter_u64(line_bytes),
            wrap_columns: bench_counter_u64(self.wrap_columns),
            edit_line_ix: bench_counter_u64(line_ix),
            dirty_lines: bench_counter_u64(dirty.end.saturating_sub(dirty.start)),
            total_rows_after: bench_counter_u64(total_rows_after),
            recomputed_lines: bench_counter_u64(recomputed_lines),
            incremental_patch: if incremental_patch { 1 } else { 0 },
        }
    }

    pub fn run_full_recompute_step(&mut self, edit_line_ix: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let (_line_ix, _line_bytes, _dirty) = self.apply_edit(edit_line_ix);
        self.row_counts = self
            .lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), self.wrap_columns))
            .collect();
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_full_recompute_step_with_metrics(
        &mut self,
        edit_line_ix: usize,
    ) -> (u64, TextInputWrapIncrementalTabsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalTabsMetrics::default());
        }
        let (line_ix, line_bytes, dirty) = self.apply_edit(edit_line_ix);
        self.row_counts = self
            .lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), self.wrap_columns))
            .collect();
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let metrics =
            self.metrics_for_step(line_ix, line_bytes, &dirty, self.row_counts.len(), false);
        (hash, metrics)
    }

    pub fn run_incremental_step(&mut self, edit_line_ix: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let (_line_ix, _line_bytes, dirty) = self.apply_edit(edit_line_ix);
        for ix in dirty {
            if let Some(slot) = self.row_counts.get_mut(ix) {
                *slot = estimate_tabbed_wrap_rows(
                    self.lines.get(ix).map(String::as_str).unwrap_or(""),
                    self.wrap_columns,
                );
            }
        }
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_incremental_step_with_metrics(
        &mut self,
        edit_line_ix: usize,
    ) -> (u64, TextInputWrapIncrementalTabsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalTabsMetrics::default());
        }
        let (line_ix, line_bytes, dirty) = self.apply_edit(edit_line_ix);
        let recomputed_lines = dirty.end.saturating_sub(dirty.start);
        for ix in dirty.clone() {
            if let Some(slot) = self.row_counts.get_mut(ix) {
                *slot = estimate_tabbed_wrap_rows(
                    self.lines.get(ix).map(String::as_str).unwrap_or(""),
                    self.wrap_columns,
                );
            }
        }
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let metrics = self.metrics_for_step(line_ix, line_bytes, &dirty, recomputed_lines, true);
        (hash, metrics)
    }

    #[cfg(test)]
    fn row_counts(&self) -> &[usize] {
        self.row_counts.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextInputWrapIncrementalBurstEditsMetrics {
    pub total_lines: u64,
    pub edits_per_burst: u64,
    pub wrap_columns: u64,
    pub total_dirty_lines: u64,
    pub total_rows_after: u64,
    pub recomputed_lines: u64,
    pub incremental_patch: u64,
}

pub struct TextInputWrapIncrementalBurstEditsFixture {
    lines: Vec<String>,
    row_counts: Vec<usize>,
    wrap_columns: usize,
    edit_nonce: usize,
}

impl TextInputWrapIncrementalBurstEditsFixture {
    pub fn new(lines: usize, line_bytes: usize, wrap_width_px: usize) -> Self {
        let lines = build_synthetic_tabbed_source_lines(lines.max(1), line_bytes.max(8));
        let wrap_columns = wrap_columns_for_benchmark_width(wrap_width_px.max(1));
        let row_counts = lines
            .iter()
            .map(|line| estimate_tabbed_wrap_rows(line.as_str(), wrap_columns))
            .collect::<Vec<_>>();
        Self {
            lines,
            row_counts,
            wrap_columns,
            edit_nonce: 0,
        }
    }

    pub fn run_full_recompute_burst_step(&mut self, edits_per_burst: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let edits_per_burst = edits_per_burst.max(1);
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            let _ = mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            self.row_counts = self
                .lines
                .iter()
                .map(|line| estimate_tabbed_wrap_rows(line.as_str(), self.wrap_columns))
                .collect();
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_incremental_burst_step(&mut self, edits_per_burst: usize) -> u64 {
        if self.lines.is_empty() {
            return 0;
        }
        let edits_per_burst = edits_per_burst.max(1);
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            let edit_col = mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = expand_tabbed_dirty_line_range(
                self.lines.as_slice(),
                line_ix,
                edit_col,
                self.wrap_columns,
            );
            for ix in dirty {
                if let Some(slot) = self.row_counts.get_mut(ix) {
                    *slot = estimate_tabbed_wrap_rows(
                        self.lines.get(ix).map(String::as_str).unwrap_or(""),
                        self.wrap_columns,
                    );
                }
            }
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        hash_wrap_rows(self.row_counts.as_slice())
    }

    pub fn run_full_recompute_burst_step_with_metrics(
        &mut self,
        edits_per_burst: usize,
    ) -> (u64, TextInputWrapIncrementalBurstEditsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalBurstEditsMetrics::default());
        }
        let edits_per_burst = edits_per_burst.max(1);
        let mut total_dirty_lines: usize = 0;
        let mut recomputed_lines: usize = 0;
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            let edit_col = mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = expand_tabbed_dirty_line_range(
                self.lines.as_slice(),
                line_ix,
                edit_col,
                self.wrap_columns,
            );
            total_dirty_lines += dirty.end.saturating_sub(dirty.start);
            self.row_counts = self
                .lines
                .iter()
                .map(|line| estimate_tabbed_wrap_rows(line.as_str(), self.wrap_columns))
                .collect();
            recomputed_lines += self.lines.len();
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let total_rows_after = self.row_counts.iter().copied().sum::<usize>();
        let metrics = TextInputWrapIncrementalBurstEditsMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            edits_per_burst: bench_counter_u64(edits_per_burst),
            wrap_columns: bench_counter_u64(self.wrap_columns),
            total_dirty_lines: bench_counter_u64(total_dirty_lines),
            total_rows_after: bench_counter_u64(total_rows_after),
            recomputed_lines: bench_counter_u64(recomputed_lines),
            incremental_patch: 0,
        };
        (hash, metrics)
    }

    pub fn run_incremental_burst_step_with_metrics(
        &mut self,
        edits_per_burst: usize,
    ) -> (u64, TextInputWrapIncrementalBurstEditsMetrics) {
        if self.lines.is_empty() {
            return (0, TextInputWrapIncrementalBurstEditsMetrics::default());
        }
        let edits_per_burst = edits_per_burst.max(1);
        let mut total_dirty_lines: usize = 0;
        let mut recomputed_lines: usize = 0;
        for step in 0..edits_per_burst {
            let line_ix = self.edit_nonce.wrapping_add(step).wrapping_mul(17) % self.lines.len();
            let edit_col = mutate_tabbed_line_for_wrap_patch(
                self.lines.get_mut(line_ix).expect("line index must exist"),
                self.edit_nonce.wrapping_add(step),
            );
            let dirty = expand_tabbed_dirty_line_range(
                self.lines.as_slice(),
                line_ix,
                edit_col,
                self.wrap_columns,
            );
            let dirty_count = dirty.end.saturating_sub(dirty.start);
            total_dirty_lines += dirty_count;
            recomputed_lines += dirty_count;
            for ix in dirty {
                if let Some(slot) = self.row_counts.get_mut(ix) {
                    *slot = estimate_tabbed_wrap_rows(
                        self.lines.get(ix).map(String::as_str).unwrap_or(""),
                        self.wrap_columns,
                    );
                }
            }
        }
        self.edit_nonce = self.edit_nonce.wrapping_add(edits_per_burst);
        let hash = hash_wrap_rows(self.row_counts.as_slice());
        let total_rows_after = self.row_counts.iter().copied().sum::<usize>();
        let metrics = TextInputWrapIncrementalBurstEditsMetrics {
            total_lines: bench_counter_u64(self.lines.len()),
            edits_per_burst: bench_counter_u64(edits_per_burst),
            wrap_columns: bench_counter_u64(self.wrap_columns),
            total_dirty_lines: bench_counter_u64(total_dirty_lines),
            total_rows_after: bench_counter_u64(total_rows_after),
            recomputed_lines: bench_counter_u64(recomputed_lines),
            incremental_patch: 1,
        };
        (hash, metrics)
    }

    #[cfg(test)]
    fn row_counts(&self) -> &[usize] {
        self.row_counts.as_slice()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextModelSnapshotCloneCostMetrics {
    pub document_bytes: u64,
    pub line_starts: u64,
    pub clone_count: u64,
    pub sampled_prefix_bytes: u64,
    pub snapshot_path: u64,
}

pub struct TextModelSnapshotCloneCostFixture {
    model: TextModel,
    string_control: SharedString,
}

impl TextModelSnapshotCloneCostFixture {
    pub fn new(min_bytes: usize) -> Self {
        let text = build_text_model_document(min_bytes.max(1));
        let model = TextModel::from_large_text(text.as_str());
        let string_control = model.as_shared_string();
        Self {
            model,
            string_control,
        }
    }

    fn metrics(
        &self,
        clones: usize,
        sampled_prefix_bytes: usize,
        snapshot_path: bool,
    ) -> TextModelSnapshotCloneCostMetrics {
        TextModelSnapshotCloneCostMetrics {
            document_bytes: bench_counter_u64(self.model.len()),
            line_starts: bench_counter_u64(self.model.line_starts().len()),
            clone_count: bench_counter_u64(clones),
            sampled_prefix_bytes: bench_counter_u64(sampled_prefix_bytes),
            snapshot_path: if snapshot_path { 1 } else { 0 },
        }
    }

    pub fn run_snapshot_clone_step(&self, clones: usize) -> u64 {
        self.run_snapshot_clone_step_with_metrics(clones).0
    }

    pub fn run_snapshot_clone_step_with_metrics(
        &self,
        clones: usize,
    ) -> (u64, TextModelSnapshotCloneCostMetrics) {
        let clones = clones.max(1);
        let snapshot = self.model.snapshot();
        let mut h = FxHasher::default();
        self.model.model_id().hash(&mut h);
        self.model.revision().hash(&mut h);
        let mut sampled_prefix_bytes = 0usize;

        for nonce in 0..clones {
            let cloned = snapshot.clone();
            nonce.hash(&mut h);
            cloned.len().hash(&mut h);
            cloned.line_starts().len().hash(&mut h);
            let prefix_end = cloned.clamp_to_char_boundary(cloned.len().min(96));
            let prefix = cloned.slice_to_string(0..prefix_end);
            sampled_prefix_bytes = prefix.len();
            prefix.len().hash(&mut h);
        }
        let metrics = self.metrics(clones, sampled_prefix_bytes, true);
        (h.finish(), metrics)
    }

    pub fn run_string_clone_control_step(&self, clones: usize) -> u64 {
        self.run_string_clone_control_step_with_metrics(clones).0
    }

    pub fn run_string_clone_control_step_with_metrics(
        &self,
        clones: usize,
    ) -> (u64, TextModelSnapshotCloneCostMetrics) {
        let clones = clones.max(1);
        let mut h = FxHasher::default();
        let mut sampled_prefix_bytes = 0usize;
        for nonce in 0..clones {
            let cloned = self.string_control.clone();
            nonce.hash(&mut h);
            cloned.len().hash(&mut h);
            sampled_prefix_bytes = cloned.as_ref().bytes().take(96).count();
            sampled_prefix_bytes.hash(&mut h);
        }
        let metrics = self.metrics(clones, sampled_prefix_bytes, false);
        (h.finish(), metrics)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextModelBulkLoadLargeMetrics {
    pub source_bytes: u64,
    pub document_bytes_after: u64,
    pub line_starts_after: u64,
    pub chunk_count: u64,
    pub load_variant: u64,
}

pub struct TextModelBulkLoadLargeFixture {
    pub text: String,
}

impl TextModelBulkLoadLargeFixture {
    pub fn new(lines: usize, line_bytes: usize) -> Self {
        let mut text = String::new();
        let synthetic_lines = build_synthetic_source_lines(lines.max(1), line_bytes.max(32));
        for line in synthetic_lines {
            text.push_str(line.as_str());
            text.push('\n');
        }
        Self { text }
    }

    pub fn run_piece_table_bulk_load_step(&self) -> u64 {
        self.run_piece_table_bulk_load_step_with_metrics().0
    }

    pub fn run_piece_table_bulk_load_step_with_metrics(
        &self,
    ) -> (u64, TextModelBulkLoadLargeMetrics) {
        if self.text.is_empty() {
            return (0, TextModelBulkLoadLargeMetrics::default());
        }

        let mut model = TextModel::new();
        let mut split = self.text.len() / 2;
        while split > 0 && !self.text.is_char_boundary(split) {
            split = split.saturating_sub(1);
        }

        let _ = model.append_large(&self.text[..split]);
        let _ = model.append_large(&self.text[split..]);
        let snapshot = model.snapshot();

        let mut h = FxHasher::default();
        snapshot.len().hash(&mut h);
        snapshot.line_starts().len().hash(&mut h);
        let suffix_start = snapshot.clamp_to_char_boundary(snapshot.len().saturating_sub(96));
        let suffix = snapshot.slice_to_string(suffix_start..snapshot.len());
        suffix.len().hash(&mut h);

        let metrics = TextModelBulkLoadLargeMetrics {
            source_bytes: bench_counter_u64(self.text.len()),
            document_bytes_after: bench_counter_u64(snapshot.len()),
            line_starts_after: bench_counter_u64(snapshot.line_starts().len()),
            chunk_count: 2,
            load_variant: 0,
        };
        (h.finish(), metrics)
    }

    pub fn run_piece_table_from_large_text_step(&self) -> u64 {
        self.run_piece_table_from_large_text_step_with_metrics().0
    }

    pub fn run_piece_table_from_large_text_step_with_metrics(
        &self,
    ) -> (u64, TextModelBulkLoadLargeMetrics) {
        if self.text.is_empty() {
            return (0, TextModelBulkLoadLargeMetrics::default());
        }

        let model = TextModel::from_large_text(self.text.as_str());
        let snapshot = model.snapshot();
        let mut h = FxHasher::default();
        snapshot.len().hash(&mut h);
        snapshot.line_starts().len().hash(&mut h);
        let prefix_end = snapshot.clamp_to_char_boundary(snapshot.len().min(96));
        let prefix = snapshot.slice_to_string(0..prefix_end);
        prefix.len().hash(&mut h);

        let metrics = TextModelBulkLoadLargeMetrics {
            source_bytes: bench_counter_u64(self.text.len()),
            document_bytes_after: bench_counter_u64(snapshot.len()),
            line_starts_after: bench_counter_u64(snapshot.line_starts().len()),
            chunk_count: 1,
            load_variant: 1,
        };
        (h.finish(), metrics)
    }

    pub fn run_string_bulk_load_control_step(&self) -> u64 {
        self.run_string_bulk_load_control_step_with_metrics().0
    }

    pub fn run_string_bulk_load_control_step_with_metrics(
        &self,
    ) -> (u64, TextModelBulkLoadLargeMetrics) {
        if self.text.is_empty() {
            return (0, TextModelBulkLoadLargeMetrics::default());
        }

        let mut loaded = String::with_capacity(self.text.len());
        let mut chunk_count = 0u64;
        for chunk in self.text.as_bytes().chunks(32 * 1024) {
            if let Ok(chunk_text) = std::str::from_utf8(chunk) {
                loaded.push_str(chunk_text);
            }
            chunk_count += 1;
        }
        let mut h = FxHasher::default();
        loaded.len().hash(&mut h);
        loaded.bytes().take(96).count().hash(&mut h);

        let metrics = TextModelBulkLoadLargeMetrics {
            source_bytes: bench_counter_u64(self.text.len()),
            document_bytes_after: bench_counter_u64(loaded.len()),
            line_starts_after: 0,
            chunk_count,
            load_variant: 2,
        };
        (h.finish(), metrics)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TextModelFragmentedEditsMetrics {
    pub initial_bytes: u64,
    pub edit_count: u64,
    pub deleted_bytes: u64,
    pub inserted_bytes: u64,
    pub final_bytes: u64,
    pub line_starts_after: u64,
    pub readback_operations: u64,
    pub string_control: u64,
}

pub struct TextModelFragmentedEditFixture {
    /// The initial document text, used to build fresh models per iteration.
    initial_text: String,
    /// Pre-computed edit sequence: (byte_offset, delete_len, insert_text).
    edits: Vec<(usize, usize, String)>,
}

impl TextModelFragmentedEditFixture {
    pub fn new(min_bytes: usize, edit_count: usize) -> Self {
        let initial_text = build_text_model_document(min_bytes.max(1024));
        let doc_len = initial_text.len();
        let edits = build_deterministic_edits(&initial_text, doc_len, edit_count.max(1));
        Self {
            initial_text,
            edits,
        }
    }

    fn metrics(
        &self,
        deleted_bytes: usize,
        inserted_bytes: usize,
        final_bytes: usize,
        line_starts_after: usize,
        readback_operations: usize,
        string_control: bool,
    ) -> TextModelFragmentedEditsMetrics {
        TextModelFragmentedEditsMetrics {
            initial_bytes: bench_counter_u64(self.initial_text.len()),
            edit_count: bench_counter_u64(self.edits.len()),
            deleted_bytes: bench_counter_u64(deleted_bytes),
            inserted_bytes: bench_counter_u64(inserted_bytes),
            final_bytes: bench_counter_u64(final_bytes),
            line_starts_after: bench_counter_u64(line_starts_after),
            readback_operations: bench_counter_u64(readback_operations),
            string_control: if string_control { 1 } else { 0 },
        }
    }

    fn apply_edits_to_model(&self) -> (TextModel, usize, usize) {
        let mut model = TextModel::from_large_text(&self.initial_text);
        let mut deleted_bytes = 0usize;
        let mut inserted_bytes = 0usize;
        for (offset, delete_len, insert) in &self.edits {
            let end = offset.saturating_add(*delete_len).min(model.len());
            let start = (*offset).min(model.len());
            deleted_bytes = deleted_bytes.saturating_add(end.saturating_sub(start));
            inserted_bytes = inserted_bytes.saturating_add(insert.len());
            let _ = model.replace_range(start..end, insert);
        }
        (model, deleted_bytes, inserted_bytes)
    }

    fn apply_edits_to_string(&self) -> (String, usize, usize) {
        let mut text = self.initial_text.clone();
        let mut deleted_bytes = 0usize;
        let mut inserted_bytes = 0usize;
        for (offset, delete_len, insert) in &self.edits {
            let start = (*offset).min(text.len());
            let end = offset.saturating_add(*delete_len).min(text.len());
            deleted_bytes = deleted_bytes.saturating_add(end.saturating_sub(start));
            inserted_bytes = inserted_bytes.saturating_add(insert.len());
            text.replace_range(start..end, insert);
        }
        (text, deleted_bytes, inserted_bytes)
    }

    /// Benchmark: apply all edits to a fresh piece-table model.
    pub fn run_fragmented_edit_step(&self) -> u64 {
        self.run_fragmented_edit_step_with_metrics().0
    }

    pub fn run_fragmented_edit_step_with_metrics(&self) -> (u64, TextModelFragmentedEditsMetrics) {
        let (model, deleted_bytes, inserted_bytes) = self.apply_edits_to_model();
        let mut h = FxHasher::default();
        model.len().hash(&mut h);
        model.revision().hash(&mut h);
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            model.len(),
            model.line_starts().len(),
            0,
            false,
        );
        (h.finish(), metrics)
    }

    /// Benchmark: apply all edits, then materialize via `as_str()`.
    pub fn run_materialize_after_edits_step(&self) -> u64 {
        self.run_materialize_after_edits_step_with_metrics().0
    }

    pub fn run_materialize_after_edits_step_with_metrics(
        &self,
    ) -> (u64, TextModelFragmentedEditsMetrics) {
        let (model, deleted_bytes, inserted_bytes) = self.apply_edits_to_model();
        let text = model.as_str();
        let mut h = FxHasher::default();
        text.len().hash(&mut h);
        text.bytes().take(128).count().hash(&mut h);
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            text.len(),
            model.line_starts().len(),
            1,
            false,
        );
        (h.finish(), metrics)
    }

    /// Benchmark: apply all edits, then call `as_shared_string()` repeatedly.
    pub fn run_shared_string_after_edits_step(&self, reads: usize) -> u64 {
        self.run_shared_string_after_edits_step_with_metrics(reads)
            .0
    }

    pub fn run_shared_string_after_edits_step_with_metrics(
        &self,
        reads: usize,
    ) -> (u64, TextModelFragmentedEditsMetrics) {
        let reads = reads.max(1);
        let (model, deleted_bytes, inserted_bytes) = self.apply_edits_to_model();
        let mut h = FxHasher::default();
        for nonce in 0..reads {
            let ss = model.as_shared_string();
            nonce.hash(&mut h);
            ss.len().hash(&mut h);
        }
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            model.len(),
            model.line_starts().len(),
            reads,
            false,
        );
        (h.finish(), metrics)
    }

    /// Control: apply the same edits to a plain `String`.
    pub fn run_string_edit_control_step(&self) -> u64 {
        self.run_string_edit_control_step_with_metrics().0
    }

    pub fn run_string_edit_control_step_with_metrics(
        &self,
    ) -> (u64, TextModelFragmentedEditsMetrics) {
        let (text, deleted_bytes, inserted_bytes) = self.apply_edits_to_string();
        let mut h = FxHasher::default();
        text.len().hash(&mut h);
        text.bytes().take(128).count().hash(&mut h);
        let metrics = self.metrics(
            deleted_bytes,
            inserted_bytes,
            text.len(),
            text_line_starts_for_benchmark(text.as_str()),
            0,
            true,
        );
        (h.finish(), metrics)
    }
}

/// Build a deterministic pseudo-random edit sequence that stays within document bounds.
fn build_deterministic_edits(
    text: &str,
    initial_len: usize,
    count: usize,
) -> Vec<(usize, usize, String)> {
    let mut edits = Vec::with_capacity(count);
    // Track approximate document length to keep offsets in bounds.
    let mut approx_len = initial_len;
    let mut seed = 0x517cc1b727220a95u64;
    for ix in 0..count {
        // Simple xorshift-style PRNG for determinism.
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed = seed.wrapping_add(ix as u64);

        let offset = if approx_len > 0 {
            (seed as usize) % approx_len
        } else {
            0
        };
        // Clamp offset to a char boundary in the initial text (approximate).
        let offset = clamp_byte_to_char_boundary(text, offset);

        let delete_len = ((seed >> 16) as usize) % 16;
        let insert = match ix % 5 {
            0 => format!("edit_{ix}"),
            1 => format!("fn f{ix}() {{ }}\n"),
            2 => String::new(), // pure delete
            3 => format!("/* {ix} */"),
            _ => format!("x{ix}\ny{ix}\n"),
        };
        approx_len = approx_len
            .saturating_sub(delete_len.min(approx_len.saturating_sub(offset)))
            .saturating_add(insert.len());
        edits.push((offset, delete_len, insert));
    }
    edits
}

fn clamp_byte_to_char_boundary(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while offset > 0 && !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn text_line_starts_for_benchmark(text: &str) -> usize {
    text.as_bytes()
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        .saturating_add(1)
}

fn build_text_model_document(min_bytes: usize) -> String {
    let mut out = String::with_capacity(min_bytes.saturating_add(64));
    let mut ix = 0usize;
    while out.len() < min_bytes {
        out.push_str(
            format!("line_{ix:06}: fn synthetic_{ix}() {{ let value = {ix}; }}\n").as_str(),
        );
        ix = ix.wrapping_add(1);
    }
    out
}

fn build_synthetic_tabbed_source_lines(lines: usize, min_line_bytes: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.max(1));
    let target = min_line_bytes.max(8);
    for ix in 0..lines.max(1) {
        let mut line = String::new();
        line.push('\t');
        line.push_str(&format!("section_{ix:05}\t"));
        line.push_str("value = ");
        while line.len() < target {
            line.push_str("token\t");
        }
        out.push(line);
    }
    out
}

fn wrap_columns_for_benchmark_width(wrap_width_px: usize) -> usize {
    let estimated_char_px = (13.0f32 * 0.6).max(1.0);
    ((wrap_width_px as f32) / estimated_char_px)
        .floor()
        .max(1.0) as usize
}

fn estimate_tabbed_wrap_rows(line: &str, wrap_columns: usize) -> usize {
    if line.is_empty() {
        return 1;
    }
    let wrap_columns = wrap_columns.max(1);
    let bytes = line.as_bytes();
    const TAB_STOP: usize = 4;

    // ASCII fast path: process segments between tabs in O(1) each.
    if line.is_ascii() {
        let mut rows = 1usize;
        let mut column = 0usize;
        let mut pos = 0usize;

        for tab_pos in memchr::memchr_iter(b'\t', bytes) {
            let seg = tab_pos - pos;
            if seg > 0 {
                let remaining = wrap_columns - column;
                if seg <= remaining {
                    column += seg;
                } else {
                    let after = seg - remaining;
                    rows += 1 + after / wrap_columns;
                    column = after % wrap_columns;
                }
            }
            let rem = column % TAB_STOP;
            let tw = if rem == 0 { TAB_STOP } else { TAB_STOP - rem };
            if tw >= wrap_columns {
                if column > 0 {
                    rows += 1;
                }
                rows += tw / wrap_columns;
                column = tw % wrap_columns;
                if column == 0 {
                    column = wrap_columns;
                }
            } else if column + tw > wrap_columns {
                rows += 1;
                column = tw;
            } else {
                column += tw;
            }
            pos = tab_pos + 1;
        }
        let trailing = bytes.len() - pos;
        if trailing > 0 {
            let remaining = wrap_columns - column;
            if trailing <= remaining {
                column += trailing;
            } else {
                let after = trailing - remaining;
                rows += 1 + after / wrap_columns;
                column = after % wrap_columns;
            }
        }
        let _ = column;
        return rows.max(1);
    }

    // Non-ASCII fallback
    let mut rows = 1usize;
    let mut column = 0usize;
    for ch in line.chars() {
        let width = if ch == '\t' {
            let rem = column % TAB_STOP;
            if rem == 0 { TAB_STOP } else { TAB_STOP - rem }
        } else {
            1
        };

        if width >= wrap_columns {
            if column > 0 {
                rows = rows.saturating_add(1);
            }
            rows = rows.saturating_add(width / wrap_columns);
            column = width % wrap_columns;
            if column == 0 {
                column = wrap_columns;
            }
            continue;
        }

        if column + width > wrap_columns {
            rows = rows.saturating_add(1);
            column = width;
        } else {
            column += width;
        }
    }
    rows.max(1)
}

fn mutate_tabbed_line_for_wrap_patch(line: &mut String, nonce: usize) -> usize {
    if line.is_empty() {
        line.push('\t');
    }
    let mut insert_ix = line.find('\t').unwrap_or(0);
    insert_ix = insert_ix.min(line.len());
    let ch = (b'a' + (nonce % 26) as u8) as char;
    line.insert(insert_ix, ch);

    if line.chars().count() > 1 {
        let remove_ix = line
            .char_indices()
            .next_back()
            .map(|(ix, _)| ix)
            .unwrap_or(0);
        let _ = line.remove(remove_ix);
    }
    insert_ix
}

fn expand_tabbed_dirty_line_range(
    lines: &[String],
    line_ix: usize,
    edit_column: usize,
    _wrap_columns: usize,
) -> Range<usize> {
    if lines.is_empty() {
        return 0..0;
    }
    let line_ix = line_ix.min(lines.len().saturating_sub(1));
    let mut end = (line_ix + 1).min(lines.len());
    if let Some(line) = lines.get(line_ix)
        && line
            .get(edit_column.min(line.len())..)
            .is_some_and(|suffix| suffix.contains('\t'))
    {
        end = end.max((line_ix + 1).min(lines.len()));
    }
    if end < lines.len() && lines.get(end).is_some_and(|line| line.starts_with('\t')) {
        end = (end + 1).min(lines.len());
    }
    line_ix..end
}

fn hash_wrap_rows(row_counts: &[usize]) -> u64 {
    let mut h = FxHasher::default();
    row_counts.len().hash(&mut h);
    for rows in row_counts.iter().take(512) {
        rows.hash(&mut h);
    }
    h.finish()
}

fn hash_text_input_shaping_slice(text: &str, max_bytes: usize) -> (u64, usize) {
    if text.len() <= max_bytes {
        let mut hasher = FxHasher::default();
        text.hash(&mut hasher);
        return (hasher.finish(), text.len());
    }

    let suffix = "…";
    let suffix_len = suffix.len();
    let mut end = max_bytes.saturating_sub(suffix_len).min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let capped_len = end.saturating_add(suffix_len);

    let mut hasher = FxHasher::default();
    capped_len.hash(&mut hasher);
    hasher.write(&text.as_bytes()[..end]);
    (hasher.finish(), capped_len)
}

fn build_synthetic_unified_patch(line_count: usize) -> String {
    let line_count = line_count.max(1);
    let mut out = String::new();
    out.push_str("diff --git a/src/lib.rs b/src/lib.rs\n");
    out.push_str("index 1111111..2222222 100644\n");
    out.push_str("--- a/src/lib.rs\n");
    out.push_str("+++ b/src/lib.rs\n");
    out.push_str(&format!(
        "@@ -1,{} +1,{} @@ fn synthetic() {{\n",
        line_count.saturating_mul(2),
        line_count.saturating_mul(2)
    ));

    for ix in 0..line_count {
        if ix % 7 == 0 {
            out.push_str(&format!("-let old_{ix} = old_call({ix});\n"));
            out.push_str(&format!("+let new_{ix} = new_call({ix});\n"));
        } else {
            out.push_str(&format!(" let shared_{ix} = keep({ix});\n"));
        }
    }
    out
}

fn should_hide_unified_diff_header_for_bench(kind: DiffLineKind, text: &str) -> bool {
    matches!(kind, DiffLineKind::Header)
        && (text.starts_with("index ") || text.starts_with("--- ") || text.starts_with("+++ "))
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct PatchDiffFirstWindowMetrics {
    pub rows_requested: u64,
    pub patch_rows_painted: u64,
    pub patch_rows_materialized: u64,
    pub patch_page_cache_entries: u64,
    pub split_rows_painted: u64,
    pub split_rows_materialized: u64,
    pub full_text_materializations: u64,
}

pub struct PatchDiffPagedRowsFixture {
    diff: Arc<Diff>,
}

impl PatchDiffPagedRowsFixture {
    pub fn new(lines: usize) -> Self {
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let text = build_synthetic_unified_patch(lines);
        Self {
            diff: Arc::new(Diff::from_unified(target, text.as_str())),
        }
    }

    pub fn run_eager_full_materialize_step(&self) -> u64 {
        let annotated = annotate_unified(&self.diff);
        let split = build_patch_split_rows(&annotated);
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        let mut hasher = FxHasher::default();
        annotated.len().hash(&mut hasher);
        split.len().hash(&mut hasher);
        for line in annotated.iter().take(256) {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut hasher);
            line.text.len().hash(&mut hasher);
            line.old_line.hash(&mut hasher);
            line.new_line.hash(&mut hasher);
        }
        for row in split.iter().take(256) {
            match row {
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    src_ix.hash(&mut hasher);
                    let click_kind_key: u8 = match click_kind {
                        DiffClickKind::Line => 0,
                        DiffClickKind::HunkHeader => 1,
                        DiffClickKind::FileHeader => 2,
                    };
                    click_kind_key.hash(&mut hasher);
                }
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let kind_key: u8 = match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    };
                    kind_key.hash(&mut hasher);
                    row.old_line.hash(&mut hasher);
                    row.new_line.hash(&mut hasher);
                    row.old.as_ref().map(|s| s.len()).hash(&mut hasher);
                    row.new.as_ref().map(|s| s.len()).hash(&mut hasher);
                    old_src_ix.hash(&mut hasher);
                    new_src_ix.hash(&mut hasher);
                }
            }
        }
        for line in &annotated {
            if !matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                continue;
            }
            let styled = super::diff_text::build_cached_diff_styled_text(
                theme,
                diff_content_text(line),
                &[],
                "",
                language,
                DiffSyntaxMode::HeuristicOnly,
                None,
            );
            styled.text.len().hash(&mut hasher);
            styled.highlights.len().hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn run_paged_first_window_step(&self, window: usize) -> u64 {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");

        let mut hasher = FxHasher::default();
        rows_provider.len_hint().hash(&mut hasher);
        split_provider.len_hint().hash(&mut hasher);

        for line in rows_provider.slice(0, window).take(window) {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut hasher);
            line.text.len().hash(&mut hasher);
            line.old_line.hash(&mut hasher);
            line.new_line.hash(&mut hasher);
            if matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    theme,
                    diff_content_text(&line),
                    &[],
                    "",
                    language,
                    DiffSyntaxMode::HeuristicOnly,
                    None,
                );
                styled.text.len().hash(&mut hasher);
                styled.highlights.len().hash(&mut hasher);
            }
        }
        for row in split_provider.slice(0, window).take(window) {
            match row {
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    src_ix.hash(&mut hasher);
                    let click_kind_key: u8 = match click_kind {
                        DiffClickKind::Line => 0,
                        DiffClickKind::HunkHeader => 1,
                        DiffClickKind::FileHeader => 2,
                    };
                    click_kind_key.hash(&mut hasher);
                }
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let kind_key: u8 = match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    };
                    kind_key.hash(&mut hasher);
                    row.old_line.hash(&mut hasher);
                    row.new_line.hash(&mut hasher);
                    row.old.as_ref().map(|s| s.len()).hash(&mut hasher);
                    row.new.as_ref().map(|s| s.len()).hash(&mut hasher);
                    old_src_ix.hash(&mut hasher);
                    new_src_ix.hash(&mut hasher);
                }
            }
        }

        hasher.finish()
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_paged_first_window_step(&self, window: usize) -> PatchDiffFirstWindowMetrics {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));

        let patch_rows_painted = rows_provider.slice(0, window).take(window).count();
        let split_rows_painted = split_provider.slice(0, window).take(window).count();

        PatchDiffFirstWindowMetrics {
            rows_requested: bench_counter_u64(window),
            patch_rows_painted: bench_counter_u64(patch_rows_painted),
            patch_rows_materialized: bench_counter_u64(rows_provider.materialized_row_count()),
            patch_page_cache_entries: bench_counter_u64(rows_provider.cached_page_count()),
            split_rows_painted: bench_counter_u64(split_rows_painted),
            split_rows_materialized: bench_counter_u64(split_provider.materialized_row_count()),
            full_text_materializations: 0,
        }
    }

    pub fn run_inline_visible_eager_scan_step(&self) -> u64 {
        let rows_provider = PagedPatchDiffRows::new(Arc::clone(&self.diff), 256);
        let mut visible_indices = Vec::new();
        for (src_ix, line) in rows_provider.slice(0, rows_provider.len_hint()).enumerate() {
            if !should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()) {
                visible_indices.push(src_ix);
            }
        }

        let mut hasher = FxHasher::default();
        visible_indices.len().hash(&mut hasher);
        for src_ix in visible_indices.into_iter().take(512) {
            src_ix.hash(&mut hasher);
        }
        hasher.finish()
    }

    pub fn run_inline_visible_hidden_map_step(&self) -> u64 {
        let hidden_flags = self
            .diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        let visible_map = PatchInlineVisibleMap::from_hidden_flags(hidden_flags.as_slice());

        let mut hasher = FxHasher::default();
        visible_map.visible_len().hash(&mut hasher);
        for visible_ix in 0..visible_map.visible_len().min(512) {
            visible_map
                .src_ix_for_visible_ix(visible_ix)
                .hash(&mut hasher);
        }
        hasher.finish()
    }

    #[cfg(test)]
    fn inline_visible_indices_eager(&self) -> Vec<usize> {
        self.diff
            .lines
            .iter()
            .enumerate()
            .filter_map(|(src_ix, line)| {
                (!should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
                    .then_some(src_ix)
            })
            .collect()
    }

    #[cfg(test)]
    fn inline_visible_indices_map(&self) -> Vec<usize> {
        let hidden_flags = self
            .diff
            .lines
            .iter()
            .map(|line| should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
            .collect::<Vec<_>>();
        let visible_map = PatchInlineVisibleMap::from_hidden_flags(hidden_flags.as_slice());
        (0..visible_map.visible_len())
            .filter_map(|visible_ix| visible_map.src_ix_for_visible_ix(visible_ix))
            .collect()
    }

    #[cfg(test)]
    fn total_rows(&self) -> usize {
        self.diff.lines.len()
    }

    /// Total row count hint for benchmark use (deep-window offset calculation).
    #[cfg(feature = "benchmarks")]
    pub fn total_rows_hint(&self) -> usize {
        self.diff.lines.len()
    }

    /// Like `run_paged_first_window_step` but starts at `start_row` (patch
    /// offset).  The split provider offset is scaled to 90% of its own
    /// `len_hint()` to avoid indexing past the end.  Used for deep-scroll
    /// benchmarks.
    pub fn run_paged_window_at_step(&self, start_row: usize, window: usize) -> u64 {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));
        let theme = AppTheme::zed_ayu_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");

        // Compute per-provider deep offsets clamped to valid range.
        let patch_start = start_row.min(rows_provider.len_hint().saturating_sub(window).max(0));
        let split_start = split_provider
            .len_hint()
            .saturating_mul(9)
            .checked_div(10)
            .unwrap_or(0)
            .min(split_provider.len_hint().saturating_sub(window));

        let mut hasher = FxHasher::default();
        rows_provider.len_hint().hash(&mut hasher);
        split_provider.len_hint().hash(&mut hasher);
        patch_start.hash(&mut hasher);

        for line in rows_provider
            .slice(patch_start, patch_start + window)
            .take(window)
        {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut hasher);
            line.text.len().hash(&mut hasher);
            line.old_line.hash(&mut hasher);
            line.new_line.hash(&mut hasher);
            if matches!(
                line.kind,
                DiffLineKind::Add | DiffLineKind::Remove | DiffLineKind::Context
            ) {
                let styled = super::diff_text::build_cached_diff_styled_text(
                    theme,
                    diff_content_text(&line),
                    &[],
                    "",
                    language,
                    DiffSyntaxMode::HeuristicOnly,
                    None,
                );
                styled.text.len().hash(&mut hasher);
                styled.highlights.len().hash(&mut hasher);
            }
        }
        for row in split_provider
            .slice(split_start, split_start + window)
            .take(window)
        {
            match row {
                PatchSplitRow::Raw { src_ix, click_kind } => {
                    src_ix.hash(&mut hasher);
                    let click_kind_key: u8 = match click_kind {
                        DiffClickKind::Line => 0,
                        DiffClickKind::HunkHeader => 1,
                        DiffClickKind::FileHeader => 2,
                    };
                    click_kind_key.hash(&mut hasher);
                }
                PatchSplitRow::Aligned {
                    row,
                    old_src_ix,
                    new_src_ix,
                } => {
                    let kind_key: u8 = match row.kind {
                        gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                        gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                        gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                        gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
                    };
                    kind_key.hash(&mut hasher);
                    row.old_line.hash(&mut hasher);
                    row.new_line.hash(&mut hasher);
                    row.old.as_ref().map(|s| s.len()).hash(&mut hasher);
                    row.new.as_ref().map(|s| s.len()).hash(&mut hasher);
                    old_src_ix.hash(&mut hasher);
                    new_src_ix.hash(&mut hasher);
                }
            }
        }

        hasher.finish()
    }

    /// Collect sidecar metrics for a deep-window paging run.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_paged_deep_window_step(
        &self,
        start_row: usize,
        window: usize,
    ) -> PatchDiffFirstWindowMetrics {
        let window = window.max(1);
        let rows_provider = Arc::new(PagedPatchDiffRows::new(Arc::clone(&self.diff), 256));
        let split_provider = PagedPatchSplitRows::new(Arc::clone(&rows_provider));

        let patch_start = start_row.min(rows_provider.len_hint().saturating_sub(window));
        let split_start = split_provider
            .len_hint()
            .saturating_mul(9)
            .checked_div(10)
            .unwrap_or(0)
            .min(split_provider.len_hint().saturating_sub(window));

        let patch_rows_painted = rows_provider
            .slice(patch_start, patch_start + window)
            .take(window)
            .count();
        let split_rows_painted = split_provider
            .slice(split_start, split_start + window)
            .take(window)
            .count();

        PatchDiffFirstWindowMetrics {
            rows_requested: bench_counter_u64(window),
            patch_rows_painted: bench_counter_u64(patch_rows_painted),
            patch_rows_materialized: bench_counter_u64(rows_provider.materialized_row_count()),
            patch_page_cache_entries: bench_counter_u64(rows_provider.cached_page_count()),
            split_rows_painted: bench_counter_u64(split_rows_painted),
            split_rows_materialized: bench_counter_u64(split_provider.materialized_row_count()),
            full_text_materializations: 0,
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn bench_counter_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

// ---------------------------------------------------------------------------
// diff_refresh_rev_only_same_content — rekey vs rebuild benchmark
// ---------------------------------------------------------------------------

/// Sidecar metrics emitted by `DiffRefreshFixture`.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DiffRefreshMetrics {
    /// Number of rekey-fast-path invocations (should be 1 per same-content refresh).
    pub diff_cache_rekeys: u64,
    /// Number of full rebuilds (should be 0 for same-content refresh).
    pub full_rebuilds: u64,
    /// Whether the content signature matched (1 = yes, 0 = no).
    pub content_signature_matches: u64,
    /// Row count preserved by the rekey path (same as initial row count when content unchanged).
    pub rows_preserved: u64,
    /// Row count after a full rebuild.
    pub rebuild_rows: u64,
}

/// Benchmark fixture for `diff_refresh_rev_only_same_content`.
///
/// Simulates the file diff cache fast path: when a store-side refresh bumps
/// `diff_file_rev` with an identical file payload, the cache should rekey its
/// prepared syntax documents and reuse the existing row provider instead of
/// performing an expensive `side_by_side_plan` + row provider rebuild.
///
/// Two benchmark sub-cases:
/// - **rekey**: compute content signature, compare, bump rev (the fast path).
/// - **rebuild**: full `side_by_side_plan` + plan scan (the slow path).
pub struct DiffRefreshFixture {
    old_text: String,
    new_text: String,
    path: std::path::PathBuf,
    /// Content signature from initial build (FxHasher of path + old + new).
    initial_signature: u64,
    /// Row count from the initial side-by-side plan.
    initial_plan_row_count: usize,
}

impl DiffRefreshFixture {
    /// Create a fixture with synthetic old/new file text.
    ///
    /// `old_lines` controls the file size.  Every 7th line is modified in the
    /// "new" version to produce a realistic mix of context and change runs.
    pub fn new(old_lines: usize) -> Self {
        let old_lines = old_lines.max(10);
        let mut old_text = String::with_capacity(old_lines * 40);
        let mut new_text = String::with_capacity(old_lines * 40);
        for i in 0..old_lines {
            if i % 7 == 0 {
                old_text.push_str(&format!("let old_{i} = old_call({i});\n"));
                new_text.push_str(&format!("let new_{i} = new_call({i});\n"));
            } else {
                let shared = format!("let shared_{i} = keep({i});\n");
                old_text.push_str(&shared);
                new_text.push_str(&shared);
            }
        }
        let path = std::path::PathBuf::from("src/bench_diff_refresh.rs");
        let initial_signature = Self::content_signature(&path, &old_text, &new_text);
        let plan = gitcomet_core::file_diff::side_by_side_plan(&old_text, &new_text);
        let initial_plan_row_count = plan.runs.iter().map(|r| r.row_len()).sum::<usize>();

        Self {
            old_text,
            new_text,
            path,
            initial_signature,
            initial_plan_row_count,
        }
    }

    /// Replicate `file_diff_text_signature`: FxHasher of (path, old, new).
    fn content_signature(path: &std::path::Path, old: &str, new: &str) -> u64 {
        let mut hasher = FxHasher::default();
        path.hash(&mut hasher);
        old.hash(&mut hasher);
        new.hash(&mut hasher);
        hasher.finish()
    }

    /// **Rekey path**: computes the content signature of an identical payload
    /// and verifies it matches the cached signature.  Returns a deterministic
    /// hash to prevent dead-code elimination.
    ///
    /// This mirrors the fast path in `ensure_file_diff_cache` where
    /// `file_content_signature == self.file_diff_cache_content_signature`.
    pub fn run_rekey_step(&self) -> u64 {
        // Compute signature of "incoming" payload (same content, bumped rev).
        let incoming_signature =
            Self::content_signature(&self.path, &self.old_text, &self.new_text);

        // The real code checks `same_repo_and_target && signature == cached`.
        // Simulate that comparison cost.
        let matched = incoming_signature == self.initial_signature;

        let mut hasher = FxHasher::default();
        matched.hash(&mut hasher);
        incoming_signature.hash(&mut hasher);
        // In the real code this path also increments the rev counter and
        // possibly re-resolves syntax document keys.  We simulate that by
        // hashing the plan row count (which stays unchanged).
        self.initial_plan_row_count.hash(&mut hasher);
        hasher.finish()
    }

    /// **Rebuild path**: performs the full `side_by_side_plan` + plan scan
    /// that would occur when the content actually changes.
    pub fn run_rebuild_step(&self) -> u64 {
        let plan = gitcomet_core::file_diff::side_by_side_plan(&self.old_text, &self.new_text);
        let mut hasher = FxHasher::default();
        plan.runs.len().hash(&mut hasher);
        let row_count: usize = plan.runs.iter().map(|r| r.row_len()).sum();
        row_count.hash(&mut hasher);
        for run in plan.runs.iter().take(64) {
            run.row_len().hash(&mut hasher);
            run.inline_row_len().hash(&mut hasher);
        }
        hasher.finish()
    }

    /// Collect sidecar metrics for the same-content refresh.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_rekey(&self) -> DiffRefreshMetrics {
        let incoming_signature =
            Self::content_signature(&self.path, &self.old_text, &self.new_text);
        let matched = incoming_signature == self.initial_signature;
        DiffRefreshMetrics {
            diff_cache_rekeys: if matched { 1 } else { 0 },
            full_rebuilds: 0,
            content_signature_matches: if matched { 1 } else { 0 },
            rows_preserved: bench_counter_u64(self.initial_plan_row_count),
            rebuild_rows: 0,
        }
    }

    /// Collect sidecar metrics for the full-rebuild path (content changed).
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_rebuild(&self) -> DiffRefreshMetrics {
        let plan = gitcomet_core::file_diff::side_by_side_plan(&self.old_text, &self.new_text);
        let row_count: usize = plan.runs.iter().map(|r| r.row_len()).sum();
        DiffRefreshMetrics {
            diff_cache_rekeys: 0,
            full_rebuilds: 1,
            content_signature_matches: 0,
            rows_preserved: 0,
            rebuild_rows: bench_counter_u64(row_count),
        }
    }
}

// ---------------------------------------------------------------------------
// File diff open fixtures (split / inline first window)
// ---------------------------------------------------------------------------

/// Sidecar metrics for `diff_open_file_split_first_window` and
/// `diff_open_file_inline_first_window` benchmarks.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileDiffOpenMetrics {
    pub rows_requested: u64,
    pub split_total_rows: u64,
    pub split_rows_painted: u64,
    pub inline_total_rows: u64,
    pub inline_rows_painted: u64,
}

/// Benchmark fixture for `diff_open_file_split_first_window/N` and
/// `diff_open_file_inline_first_window/N`.
///
/// Constructs synthetic old/new file text with every 7th line modified,
/// builds a `side_by_side_plan`, and creates paged row providers.  The
/// benchmark measures the cost of materializing the first visible window
/// of split (side-by-side) or inline rows — the dominant cost when a user
/// opens a file diff.
pub struct FileDiffOpenFixture {
    split: std::sync::Arc<crate::view::panes::main::diff_cache::PagedFileDiffRows>,
    inline: std::sync::Arc<crate::view::panes::main::diff_cache::PagedFileDiffInlineRows>,
}

impl FileDiffOpenFixture {
    /// Create a fixture with `old_lines` lines in the old file.
    /// Every 7th line is modified in the new version.
    pub fn new(old_lines: usize) -> Self {
        let old_lines = old_lines.max(10);
        let mut old_text = String::with_capacity(old_lines * 80);
        let mut new_text = String::with_capacity(old_lines * 80);
        for i in 0..old_lines {
            if i % 7 == 0 {
                old_text.push_str(&format!("let old_{i} = old_call({i});\n"));
                new_text.push_str(&format!("let new_{i} = new_call({i});\n"));
            } else {
                let shared = format!("let shared_{i} = keep({i});\n");
                old_text.push_str(&shared);
                new_text.push_str(&shared);
            }
        }
        #[cfg(feature = "benchmarks")]
        let (split, inline) = crate::view::panes::main::diff_cache::bench_build_file_diff_providers(
            &old_text, &new_text, 256,
        );
        #[cfg(not(feature = "benchmarks"))]
        let (split, inline) = unreachable!("FileDiffOpenFixture requires benchmarks feature");

        Self { split, inline }
    }

    /// Measure the cost of paging the first `window` split (side-by-side) rows.
    pub fn run_split_first_window(&self, window: usize) -> u64 {
        use gitcomet_core::domain::DiffRowProvider;
        let window = window.max(1);
        let mut h = FxHasher::default();
        self.split.len_hint().hash(&mut h);
        for row in self.split.slice(0, window).take(window) {
            let kind_key: u8 = match row.kind {
                gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
                gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
                gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
                gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
            };
            kind_key.hash(&mut h);
            row.old_line.hash(&mut h);
            row.new_line.hash(&mut h);
            row.old.as_ref().map(|s| s.len()).hash(&mut h);
            row.new.as_ref().map(|s| s.len()).hash(&mut h);
        }
        h.finish()
    }

    /// Measure the cost of paging the first `window` inline rows.
    pub fn run_inline_first_window(&self, window: usize) -> u64 {
        use gitcomet_core::domain::DiffRowProvider;
        let window = window.max(1);
        let mut h = FxHasher::default();
        self.inline.len_hint().hash(&mut h);
        for line in self.inline.slice(0, window).take(window) {
            let kind_key: u8 = match line.kind {
                DiffLineKind::Header => 0,
                DiffLineKind::Hunk => 1,
                DiffLineKind::Add => 2,
                DiffLineKind::Remove => 3,
                DiffLineKind::Context => 4,
            };
            kind_key.hash(&mut h);
            line.text.len().hash(&mut h);
            line.old_line.hash(&mut h);
            line.new_line.hash(&mut h);
        }
        h.finish()
    }

    /// Collect structural sidecar metrics for the first-window operation.
    #[cfg(any(test, feature = "benchmarks"))]
    pub fn measure_first_window(&self, window: usize) -> FileDiffOpenMetrics {
        use gitcomet_core::domain::DiffRowProvider;
        let window = window.max(1);
        let split_painted = self.split.slice(0, window).take(window).count();
        let inline_painted = self.inline.slice(0, window).take(window).count();
        FileDiffOpenMetrics {
            rows_requested: bench_counter_u64(window),
            split_total_rows: bench_counter_u64(self.split.len_hint()),
            split_rows_painted: bench_counter_u64(split_painted),
            inline_total_rows: bench_counter_u64(self.inline.len_hint()),
            inline_rows_painted: bench_counter_u64(inline_painted),
        }
    }
}

// ---------------------------------------------------------------------------
// Pane resize drag step fixture
// ---------------------------------------------------------------------------

/// Drag target for `pane_resize_drag_step/*`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PaneResizeTarget {
    Sidebar,
    Details,
}

/// Benchmark fixture for `pane_resize_drag_step/sidebar` and `.../details`.
///
/// Each iteration models a single drag-step update using the production pane
/// clamp math from `view/mod.rs`, then records the resulting pane width and the
/// main-pane width after layout recomputation.
pub struct PaneResizeDragStepFixture {
    target: PaneResizeTarget,
    total_width: Pixels,
    sidebar_width: Pixels,
    details_width: Pixels,
    sidebar_collapsed: bool,
    details_collapsed: bool,
    drag_step_px: f32,
    drag_direction: f32,
    steps: usize,
}

/// Sidecar metrics for pane resize drag benchmarks.
pub struct PaneResizeDragMetrics {
    pub steps: u64,
    pub width_bounds_recomputes: u64,
    pub layout_recomputes: u64,
    pub min_pane_width_px: f64,
    pub max_pane_width_px: f64,
    pub min_main_width_px: f64,
    pub max_main_width_px: f64,
    pub clamp_at_min_count: u64,
    pub clamp_at_max_count: u64,
}

impl PaneResizeDragStepFixture {
    pub fn new(target: PaneResizeTarget) -> Self {
        Self {
            target,
            total_width: px(1_280.0),
            sidebar_width: px(280.0),
            details_width: px(420.0),
            sidebar_collapsed: false,
            details_collapsed: false,
            drag_step_px: 24.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&mut self) -> (u64, PaneResizeDragMetrics) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let mut h = FxHasher::default();
        let mut min_pane_width = f32::MAX;
        let mut max_pane_width = f32::MIN;
        let mut min_main_width = f32::MAX;
        let mut max_main_width = f32::MIN;
        let mut clamp_at_min_count: u64 = 0;
        let mut clamp_at_max_count: u64 = 0;
        let mut width_bounds_recomputes: u64 = 0;
        let mut layout_recomputes: u64 = 0;

        for _ in 0..self.steps {
            let handle = self.handle();
            let state = PaneResizeState {
                handle,
                start_x: px(0.0),
                start_sidebar: self.sidebar_width,
                start_details: self.details_width,
            };
            let current_x = px(self.drag_step_px * self.drag_direction);
            let (min_width, max_width) = pane_resize_drag_width_bounds(
                handle,
                state.start_sidebar,
                state.start_details,
                self.total_width,
                self.sidebar_collapsed,
                self.details_collapsed,
            );
            width_bounds_recomputes = width_bounds_recomputes.saturating_add(1);
            let next_width = next_pane_resize_drag_width(
                state,
                current_x,
                self.total_width,
                self.sidebar_collapsed,
                self.details_collapsed,
            );

            match self.target {
                PaneResizeTarget::Sidebar => self.sidebar_width = next_width,
                PaneResizeTarget::Details => self.details_width = next_width,
            }

            let next_width_px: f32 = next_width.into();
            let min_width_px: f32 = min_width.into();
            let max_width_px: f32 = max_width.into();

            if next_width_px <= min_width_px + f32::EPSILON {
                clamp_at_min_count += 1;
                self.drag_direction = -self.drag_direction;
            } else if next_width_px >= max_width_px - f32::EPSILON {
                clamp_at_max_count += 1;
                self.drag_direction = -self.drag_direction;
            }

            min_pane_width = min_pane_width.min(next_width_px);
            max_pane_width = max_pane_width.max(next_width_px);

            let main_width = pane_content_width_for_layout(
                self.total_width,
                self.sidebar_width,
                self.details_width,
                self.sidebar_collapsed,
                self.details_collapsed,
            );
            layout_recomputes = layout_recomputes.saturating_add(1);
            let main_width_px: f32 = main_width.into();
            min_main_width = min_main_width.min(main_width_px);
            max_main_width = max_main_width.max(main_width_px);

            next_width_px.to_bits().hash(&mut h);
            main_width_px.to_bits().hash(&mut h);
            min_width_px.to_bits().hash(&mut h);
            max_width_px.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        let metrics = PaneResizeDragMetrics {
            steps: self.steps as u64,
            width_bounds_recomputes,
            layout_recomputes,
            min_pane_width_px: min_pane_width as f64,
            max_pane_width_px: max_pane_width as f64,
            min_main_width_px: min_main_width as f64,
            max_main_width_px: max_main_width as f64,
            clamp_at_min_count,
            clamp_at_max_count,
        };

        (h.finish(), metrics)
    }

    fn handle(&self) -> PaneResizeHandle {
        match self.target {
            PaneResizeTarget::Sidebar => PaneResizeHandle::Sidebar,
            PaneResizeTarget::Details => PaneResizeHandle::Details,
        }
    }

    #[cfg(test)]
    pub(super) fn pane_widths(&self) -> (f32, f32) {
        let sidebar: f32 = self.sidebar_width.into();
        let details: f32 = self.details_width.into();
        (sidebar, details)
    }
}

// ---------------------------------------------------------------------------
// Diff split resize drag step fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `diff_split_resize_drag_step/window_200`.
///
/// Simulates 200 drag-step updates on the diff-split divider using the
/// production clamp math from `view/mod.rs::next_diff_split_drag_ratio`.
/// The fixture sweeps the split ratio back and forth across the available
/// main-pane width, reversing direction when the ratio hits the column
/// minimum bounds.
pub struct DiffSplitResizeDragStepFixture {
    /// Main pane content width (the area that holds left + handle + right).
    main_pane_width: Pixels,
    /// Current diff split ratio (0.0–1.0).
    ratio: f32,
    /// Pixel step per drag event.
    drag_step_px: f32,
    /// Current direction (+1.0 = right, -1.0 = left).
    drag_direction: f32,
    /// Number of drag steps per benchmark iteration.
    steps: usize,
}

/// Sidecar metrics for diff split resize drag benchmarks.
pub struct DiffSplitResizeDragMetrics {
    pub steps: u64,
    pub ratio_recomputes: u64,
    pub column_width_recomputes: u64,
    pub min_ratio: f64,
    pub max_ratio: f64,
    pub min_left_col_px: f64,
    pub max_left_col_px: f64,
    pub min_right_col_px: f64,
    pub max_right_col_px: f64,
    pub clamp_at_min_count: u64,
    pub clamp_at_max_count: u64,
    pub narrow_fallback_count: u64,
}

impl DiffSplitResizeDragStepFixture {
    /// Create a fixture simulating a 200-row visible diff window.
    ///
    /// The `main_pane_width` is set to a realistic value for a ~1280 px window
    /// with sidebar (280) and details (420) open: 1280 - 280 - 420 - 16 = 564 px.
    pub fn window_200() -> Self {
        Self {
            main_pane_width: px(564.0),
            ratio: 0.5,
            drag_step_px: 12.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&mut self) -> (u64, DiffSplitResizeDragMetrics) {
        use crate::view::{
            diff_split_column_widths, diff_split_drag_params, next_diff_split_drag_ratio,
        };

        let (available_base, min_col_w) = diff_split_drag_params(self.main_pane_width);

        let mut h = FxHasher::default();
        let mut min_ratio = f64::MAX;
        let mut max_ratio = f64::MIN;
        let mut min_left_col = f64::MAX;
        let mut max_left_col = f64::MIN;
        let mut min_right_col = f64::MAX;
        let mut max_right_col = f64::MIN;
        let mut clamp_at_min_count: u64 = 0;
        let mut clamp_at_max_count: u64 = 0;
        let mut narrow_fallback_count: u64 = 0;
        let mut ratio_recomputes: u64 = 0;
        let mut column_width_recomputes: u64 = 0;

        for _ in 0..self.steps {
            let dx = px(self.drag_step_px * self.drag_direction);
            ratio_recomputes = ratio_recomputes.saturating_add(1);

            match next_diff_split_drag_ratio(available_base, min_col_w, self.ratio, dx) {
                None => {
                    // Too narrow — force 50/50.
                    self.ratio = 0.5;
                    narrow_fallback_count += 1;
                }
                Some(next_ratio) => {
                    // Detect clamping by checking if the ratio is at the
                    // min or max boundary.
                    let available_f: f32 = available_base.into();
                    let min_col_f: f32 = min_col_w.into();
                    let min_bound = min_col_f / available_f;
                    let max_bound = 1.0 - min_bound;

                    if next_ratio <= min_bound + f32::EPSILON {
                        clamp_at_min_count += 1;
                        self.drag_direction = -self.drag_direction;
                    } else if next_ratio >= max_bound - f32::EPSILON {
                        clamp_at_max_count += 1;
                        self.drag_direction = -self.drag_direction;
                    }

                    self.ratio = next_ratio;
                }
            }

            // Compute column widths for this ratio (exercises the layout path).
            let (left_w, right_w) = diff_split_column_widths(self.main_pane_width, self.ratio);
            column_width_recomputes = column_width_recomputes.saturating_add(1);
            let left_f: f32 = left_w.into();
            let right_f: f32 = right_w.into();

            let ratio_f64 = self.ratio as f64;
            min_ratio = min_ratio.min(ratio_f64);
            max_ratio = max_ratio.max(ratio_f64);
            min_left_col = min_left_col.min(left_f as f64);
            max_left_col = max_left_col.max(left_f as f64);
            min_right_col = min_right_col.min(right_f as f64);
            max_right_col = max_right_col.max(right_f as f64);

            self.ratio.to_bits().hash(&mut h);
            left_f.to_bits().hash(&mut h);
            right_f.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        let metrics = DiffSplitResizeDragMetrics {
            steps: self.steps as u64,
            ratio_recomputes,
            column_width_recomputes,
            min_ratio,
            max_ratio,
            min_left_col_px: min_left_col,
            max_left_col_px: max_left_col,
            min_right_col_px: min_right_col,
            max_right_col_px: max_right_col,
            clamp_at_min_count,
            clamp_at_max_count,
            narrow_fallback_count,
        };

        (h.finish(), metrics)
    }

    #[cfg(test)]
    pub(super) fn current_ratio(&self) -> f32 {
        self.ratio
    }
}

// ---------------------------------------------------------------------------
// Window resize layout fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `window_resize_layout/sidebar_main_details`.
///
/// Simulates a sustained window-resize drag by sweeping through a range of
/// total window widths and recomputing the main-pane content width at each
/// step.  This exercises `pane_content_width_for_layout`, the sidebar/details
/// collapse/expand thresholds, and the resize-handle accounting.
pub struct WindowResizeLayoutFixture {
    sidebar_w: f32,
    details_w: f32,
    sidebar_collapsed: bool,
    details_collapsed: bool,
    start_total_w: f32,
    end_total_w: f32,
    steps: usize,
}

/// Sidecar metrics for window resize layout benchmarks.
pub struct WindowResizeLayoutMetrics {
    pub steps: u64,
    pub layout_recomputes: u64,
    pub min_main_w_px: f64,
    pub max_main_w_px: f64,
    pub clamp_at_zero_count: u64,
}

impl WindowResizeLayoutFixture {
    /// Standard 3-pane layout: sidebar 280, details 420, sweep 800..1800 in 200 steps.
    pub fn sidebar_main_details() -> Self {
        Self {
            sidebar_w: 280.0,
            details_w: 420.0,
            sidebar_collapsed: false,
            details_collapsed: false,
            start_total_w: 800.0,
            end_total_w: 1800.0,
            steps: 200,
        }
    }

    pub fn run(&self) -> u64 {
        let (_, metrics) = self.run_with_metrics();
        let mut h = FxHasher::default();
        metrics.steps.hash(&mut h);
        metrics.clamp_at_zero_count.hash(&mut h);
        h.finish()
    }

    pub fn run_with_metrics(&self) -> (u64, WindowResizeLayoutMetrics) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let step_delta = (self.end_total_w - self.start_total_w) / self.steps.max(1) as f32;
        let sidebar_px = px(self.sidebar_w);
        let details_px = px(self.details_w);

        let mut min_main: f32 = f32::MAX;
        let mut max_main: f32 = f32::MIN;
        let mut clamp_zero: u64 = 0;
        let mut layout_recomputes: u64 = 0;
        let mut h = FxHasher::default();

        for i in 0..self.steps {
            let total_w = self.start_total_w + step_delta * i as f32;
            let main_w = pane_content_width_for_layout(
                px(total_w),
                sidebar_px,
                details_px,
                self.sidebar_collapsed,
                self.details_collapsed,
            );
            layout_recomputes = layout_recomputes.saturating_add(1);
            let main_f: f32 = main_w.into();
            main_f.to_bits().hash(&mut h);
            if main_f < min_main {
                min_main = main_f;
            }
            if main_f > max_main {
                max_main = main_f;
            }
            if main_f <= 0.0 {
                clamp_zero += 1;
            }
        }

        let metrics = WindowResizeLayoutMetrics {
            steps: self.steps as u64,
            layout_recomputes,
            min_main_w_px: min_main as f64,
            max_main_w_px: max_main as f64,
            clamp_at_zero_count: clamp_zero,
        };

        (h.finish(), metrics)
    }
}

/// Benchmark fixture for
/// `window_resize_layout/history_50k_commits_diff_20k_lines`.
///
/// Unlike the baseline resize-layout fixture, this keeps two large hot-path
/// workloads resident and replays them on every width change:
///
/// - a precomputed 50k-commit history list window
/// - an open 20k-line split file diff window
///
/// Each step recomputes the production main-pane layout width, diff split
/// widths, and history-column visibility before repainting stable visible
/// windows from both fixtures. This approximates resize cost once the repo is
/// already open and both heavy views are warm.
pub struct WindowResizeLayoutExtremeFixture {
    sidebar_w: f32,
    details_w: f32,
    sidebar_collapsed: bool,
    details_collapsed: bool,
    start_total_w: f32,
    end_total_w: f32,
    steps: usize,
    history: HistoryListScrollFixture,
    history_start_row: usize,
    history_window_rows: usize,
    diff: FileDiffOpenFixture,
    diff_window_rows: usize,
    diff_split_ratio: f32,
    history_commits: usize,
    diff_lines: usize,
}

/// Sidecar metrics for the extreme-scale window resize layout benchmark.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WindowResizeLayoutExtremeMetrics {
    pub steps: u64,
    pub layout_recomputes: u64,
    pub history_visibility_recomputes: u64,
    pub diff_width_recomputes: u64,
    pub history_commits: u64,
    pub history_window_rows: u64,
    pub history_rows_processed_total: u64,
    pub history_columns_hidden_steps: u64,
    pub history_all_columns_visible_steps: u64,
    pub diff_lines: u64,
    pub diff_window_rows: u64,
    pub diff_split_total_rows: u64,
    pub diff_rows_processed_total: u64,
    pub diff_narrow_fallback_steps: u64,
    pub min_main_w_px: f64,
    pub max_main_w_px: f64,
}

impl WindowResizeLayoutExtremeFixture {
    const HISTORY_COMMITS: usize = 50_000;
    const HISTORY_LOCAL_BRANCHES: usize = 200;
    const HISTORY_REMOTE_BRANCHES: usize = 800;
    const HISTORY_WINDOW_ROWS: usize = 64;
    const DIFF_LINES: usize = 20_000;
    const DIFF_WINDOW_ROWS: usize = 200;

    pub fn history_50k_commits_diff_20k_lines() -> Self {
        let history = HistoryListScrollFixture::new(
            Self::HISTORY_COMMITS,
            Self::HISTORY_LOCAL_BRANCHES,
            Self::HISTORY_REMOTE_BRANCHES,
        );
        let history_window_rows = Self::HISTORY_WINDOW_ROWS.min(Self::HISTORY_COMMITS.max(1));
        let history_start_row = Self::HISTORY_COMMITS.saturating_sub(history_window_rows) / 2;

        Self {
            sidebar_w: 280.0,
            details_w: 420.0,
            sidebar_collapsed: false,
            details_collapsed: false,
            start_total_w: 820.0,
            end_total_w: 2_200.0,
            steps: 200,
            history,
            history_start_row,
            history_window_rows,
            diff: FileDiffOpenFixture::new(Self::DIFF_LINES),
            diff_window_rows: Self::DIFF_WINDOW_ROWS,
            diff_split_ratio: 0.5,
            history_commits: Self::HISTORY_COMMITS,
            diff_lines: Self::DIFF_LINES,
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal().0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, WindowResizeLayoutExtremeMetrics) {
        let (
            hash,
            min_main_w_px,
            max_main_w_px,
            history_columns_hidden_steps,
            history_all_columns_visible_steps,
            diff_narrow_fallback_steps,
        ) = self.run_internal();

        let diff_metrics = self.diff.measure_first_window(self.diff_window_rows);
        let steps = bench_counter_u64(self.steps);
        let history_window_rows = bench_counter_u64(self.history_window_rows);
        let diff_window_rows = bench_counter_u64(self.diff_window_rows);

        let metrics = WindowResizeLayoutExtremeMetrics {
            steps,
            layout_recomputes: steps,
            history_visibility_recomputes: steps,
            diff_width_recomputes: steps,
            history_commits: bench_counter_u64(self.history_commits),
            history_window_rows,
            history_rows_processed_total: history_window_rows.saturating_mul(steps),
            history_columns_hidden_steps,
            history_all_columns_visible_steps,
            diff_lines: bench_counter_u64(self.diff_lines),
            diff_window_rows,
            diff_split_total_rows: diff_metrics.split_total_rows,
            diff_rows_processed_total: diff_metrics.split_rows_painted.saturating_mul(steps),
            diff_narrow_fallback_steps,
            min_main_w_px,
            max_main_w_px,
        };

        (hash, metrics)
    }

    fn run_internal(&self) -> (u64, f64, f64, u64, u64, u64) {
        use crate::view::panes::main::pane_content_width_for_layout;
        use crate::view::{diff_split_column_widths, diff_split_drag_params};

        let step_delta = (self.end_total_w - self.start_total_w) / self.steps.max(1) as f32;
        let sidebar_px = px(self.sidebar_w);
        let details_px = px(self.details_w);

        let mut min_main = f32::MAX;
        let mut max_main = f32::MIN;
        let mut history_columns_hidden_steps = 0u64;
        let mut history_all_columns_visible_steps = 0u64;
        let mut diff_narrow_fallback_steps = 0u64;
        let mut h = FxHasher::default();

        for i in 0..self.steps {
            let total_w = self.start_total_w + step_delta * i as f32;
            let main_w = pane_content_width_for_layout(
                px(total_w),
                sidebar_px,
                details_px,
                self.sidebar_collapsed,
                self.details_collapsed,
            );
            let main_w_px: f32 = main_w.into();
            min_main = min_main.min(main_w_px);
            max_main = max_main.max(main_w_px);

            let (show_author, show_date, show_sha) =
                Self::history_column_visibility_for_window_width(total_w);
            if !(show_author && show_date && show_sha) {
                history_columns_hidden_steps = history_columns_hidden_steps.saturating_add(1);
            }
            if show_author && show_date && show_sha {
                history_all_columns_visible_steps =
                    history_all_columns_visible_steps.saturating_add(1);
            }

            let (available, min_col_w) = diff_split_drag_params(main_w);
            if available <= min_col_w * 2.0 {
                diff_narrow_fallback_steps = diff_narrow_fallback_steps.saturating_add(1);
            }
            let (left_w, right_w) = diff_split_column_widths(main_w, self.diff_split_ratio);
            let left_w_px: f32 = left_w.into();
            let right_w_px: f32 = right_w.into();

            let history_hash = self
                .history
                .run_scroll_step(self.history_start_row, self.history_window_rows);
            let diff_hash = self.diff.run_split_first_window(self.diff_window_rows);

            total_w.to_bits().hash(&mut h);
            main_w_px.to_bits().hash(&mut h);
            left_w_px.to_bits().hash(&mut h);
            right_w_px.to_bits().hash(&mut h);
            show_author.hash(&mut h);
            show_date.hash(&mut h);
            show_sha.hash(&mut h);
            history_hash.hash(&mut h);
            diff_hash.hash(&mut h);
        }

        (
            h.finish(),
            min_main as f64,
            max_main as f64,
            history_columns_hidden_steps,
            history_all_columns_visible_steps,
            diff_narrow_fallback_steps,
        )
    }

    fn history_column_visibility_for_window_width(total_w: f32) -> (bool, bool, bool) {
        let available = (total_w - 280.0 - 420.0 - 64.0).max(0.0);
        if available <= 0.0 {
            return (false, false, false);
        }

        let min_message = HistoryColumnResizeDragStepFixture::MESSAGE_MIN_PX;
        let fixed_base = HistoryColumnResizeDragStepFixture::COL_BRANCH_PX
            + HistoryColumnResizeDragStepFixture::COL_GRAPH_PX;
        let mut fixed = fixed_base
            + HistoryColumnResizeDragStepFixture::COL_AUTHOR_PX
            + HistoryColumnResizeDragStepFixture::COL_DATE_PX
            + HistoryColumnResizeDragStepFixture::COL_SHA_PX;

        let mut show_author = true;
        let mut show_date = true;
        let mut show_sha = true;

        if available - fixed < min_message && show_sha {
            show_sha = false;
            fixed -= HistoryColumnResizeDragStepFixture::COL_SHA_PX;
        }
        if available - fixed < min_message {
            if show_date {
                show_date = false;
                fixed -= HistoryColumnResizeDragStepFixture::COL_DATE_PX;
            }
            show_sha = false;
        }
        if available - fixed < min_message && show_author {
            show_author = false;
            fixed -= HistoryColumnResizeDragStepFixture::COL_AUTHOR_PX;
        }
        if available - fixed < min_message {
            show_author = false;
            show_date = false;
            show_sha = false;
        }

        (show_author, show_date, show_sha)
    }
}

// ---------------------------------------------------------------------------
// History column resize drag step fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `history_column_resize_drag_step/*`.
///
/// Simulates a sustained column-resize drag over a history-pane column.
/// Each step applies a pixel delta, clamps the candidate width against the
/// column's static bounds and the message-area minimum, then recomputes
/// which optional columns (author, date, SHA) remain visible.
///
/// The fixture replicates the `history_column_drag_clamped_width` +
/// `history_visible_columns` math from `crate::view::panes::history`.
pub struct HistoryColumnResizeDragStepFixture {
    col_branch: f32,
    col_graph: f32,
    col_author: f32,
    col_date: f32,
    col_sha: f32,
    window_width: f32,
    drag_step_px: f32,
    drag_direction: f32,
    steps: usize,
}

/// Sidecar metrics for history column resize drag benchmarks.
pub struct HistoryColumnResizeMetrics {
    pub steps: u64,
    pub width_clamp_recomputes: u64,
    pub visible_column_recomputes: u64,
    pub columns_hidden_count: u64,
    pub clamp_at_min_count: u64,
    pub clamp_at_max_count: u64,
}

/// Column identity for the column being dragged.
#[derive(Clone, Copy)]
pub enum HistoryResizeColumn {
    Branch,
    Graph,
    Author,
    Date,
    Sha,
}

impl HistoryColumnResizeDragStepFixture {
    // Constants replicated from view/mod.rs.
    const COL_BRANCH_PX: f32 = 130.0;
    const COL_GRAPH_PX: f32 = 80.0;
    const COL_AUTHOR_PX: f32 = 140.0;
    const COL_DATE_PX: f32 = 160.0;
    const COL_SHA_PX: f32 = 88.0;
    const MESSAGE_MIN_PX: f32 = 220.0;

    const COL_BRANCH_MIN: f32 = 60.0;
    const COL_BRANCH_MAX: f32 = 320.0;
    const COL_GRAPH_MIN: f32 = 44.0;
    const COL_GRAPH_MAX: f32 = 240.0;
    const COL_AUTHOR_MIN: f32 = 80.0;
    const COL_AUTHOR_MAX: f32 = 260.0;
    const COL_DATE_MIN: f32 = 110.0;
    const COL_DATE_MAX: f32 = 240.0;
    const COL_SHA_MIN: f32 = 60.0;
    const COL_SHA_MAX: f32 = 160.0;

    pub fn new(column: HistoryResizeColumn) -> Self {
        let _ = column; // all columns start from the same defaults
        Self {
            col_branch: Self::COL_BRANCH_PX,
            col_graph: Self::COL_GRAPH_PX,
            col_author: Self::COL_AUTHOR_PX,
            col_date: Self::COL_DATE_PX,
            col_sha: Self::COL_SHA_PX,
            window_width: 1600.0,
            drag_step_px: 8.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self, column: HistoryResizeColumn) -> u64 {
        let (hash, _) = self.run_with_metrics(column);
        hash
    }

    pub fn run_with_metrics(
        &mut self,
        column: HistoryResizeColumn,
    ) -> (u64, HistoryColumnResizeMetrics) {
        // Available width for columns: window - sidebar(280) - details(420) - misc(64)
        let available = (self.window_width - 280.0 - 420.0 - 64.0).max(0.0);

        let (min_w, static_max) = Self::static_bounds(column);

        let mut h = FxHasher::default();
        let mut columns_hidden: u64 = 0;
        let mut clamp_min: u64 = 0;
        let mut clamp_max: u64 = 0;
        let mut width_clamp_recomputes: u64 = 0;
        let mut visible_column_recomputes: u64 = 0;

        for _ in 0..self.steps {
            let current = self.col_for(column);
            let candidate = current + self.drag_step_px * self.drag_direction;

            // Compute right_fixed_w (other columns excluding the one being dragged)
            let show_author = true;
            let show_date = true;
            let show_sha = true;
            let right_fixed = self.right_fixed_excluding(column, show_author, show_date, show_sha);
            let dynamic_max = (available - right_fixed - Self::MESSAGE_MIN_PX).max(min_w);
            let max_w = static_max.min(dynamic_max).max(min_w);
            width_clamp_recomputes = width_clamp_recomputes.saturating_add(1);
            let clamped = candidate.max(min_w).min(max_w);

            self.set_col(column, clamped);

            if clamped <= min_w + f32::EPSILON {
                clamp_min += 1;
                self.drag_direction = 1.0;
            } else if clamped >= max_w - f32::EPSILON {
                clamp_max += 1;
                self.drag_direction = -1.0;
            }

            // Recompute visible columns (the message-area squeeze check)
            let (vis_author, vis_date, vis_sha) = self.visible_columns(available);
            visible_column_recomputes = visible_column_recomputes.saturating_add(1);
            if !vis_author || !vis_date || !vis_sha {
                columns_hidden += 1;
            }

            clamped.to_bits().hash(&mut h);
            vis_author.hash(&mut h);
            vis_date.hash(&mut h);
            vis_sha.hash(&mut h);
        }

        let metrics = HistoryColumnResizeMetrics {
            steps: self.steps as u64,
            width_clamp_recomputes,
            visible_column_recomputes,
            columns_hidden_count: columns_hidden,
            clamp_at_min_count: clamp_min,
            clamp_at_max_count: clamp_max,
        };

        (h.finish(), metrics)
    }

    fn static_bounds(column: HistoryResizeColumn) -> (f32, f32) {
        match column {
            HistoryResizeColumn::Branch => (Self::COL_BRANCH_MIN, Self::COL_BRANCH_MAX),
            HistoryResizeColumn::Graph => (Self::COL_GRAPH_MIN, Self::COL_GRAPH_MAX),
            HistoryResizeColumn::Author => (Self::COL_AUTHOR_MIN, Self::COL_AUTHOR_MAX),
            HistoryResizeColumn::Date => (Self::COL_DATE_MIN, Self::COL_DATE_MAX),
            HistoryResizeColumn::Sha => (Self::COL_SHA_MIN, Self::COL_SHA_MAX),
        }
    }

    fn col_for(&self, column: HistoryResizeColumn) -> f32 {
        match column {
            HistoryResizeColumn::Branch => self.col_branch,
            HistoryResizeColumn::Graph => self.col_graph,
            HistoryResizeColumn::Author => self.col_author,
            HistoryResizeColumn::Date => self.col_date,
            HistoryResizeColumn::Sha => self.col_sha,
        }
    }

    fn set_col(&mut self, column: HistoryResizeColumn, value: f32) {
        match column {
            HistoryResizeColumn::Branch => self.col_branch = value,
            HistoryResizeColumn::Graph => self.col_graph = value,
            HistoryResizeColumn::Author => self.col_author = value,
            HistoryResizeColumn::Date => self.col_date = value,
            HistoryResizeColumn::Sha => self.col_sha = value,
        }
    }

    fn right_fixed_excluding(
        &self,
        column: HistoryResizeColumn,
        show_author: bool,
        show_date: bool,
        show_sha: bool,
    ) -> f32 {
        let mut sum = 0.0;
        if !matches!(column, HistoryResizeColumn::Branch) {
            sum += self.col_branch;
        }
        if !matches!(column, HistoryResizeColumn::Graph) {
            sum += self.col_graph;
        }
        if show_author && !matches!(column, HistoryResizeColumn::Author) {
            sum += self.col_author;
        }
        if show_date && !matches!(column, HistoryResizeColumn::Date) {
            sum += self.col_date;
        }
        if show_sha && !matches!(column, HistoryResizeColumn::Sha) {
            sum += self.col_sha;
        }
        sum
    }

    fn visible_columns(&self, available: f32) -> (bool, bool, bool) {
        let min_message = Self::MESSAGE_MIN_PX;
        let mut show_author = true;
        let mut show_date = true;
        let mut show_sha = true;

        let fixed_base = self.col_branch + self.col_graph;
        let mut fixed = fixed_base + self.col_author + self.col_date + self.col_sha;

        if available - fixed < min_message && show_sha {
            show_sha = false;
            fixed -= self.col_sha;
        }
        if available - fixed < min_message {
            if show_date {
                show_date = false;
                fixed -= self.col_date;
            }
            show_sha = false;
        }
        if available - fixed < min_message && show_author {
            show_author = false;
        }

        (show_author, show_date, show_sha)
    }
}

// ---------------------------------------------------------------------------
// Repo tab drag fixtures
// ---------------------------------------------------------------------------

/// Benchmark fixture for `repo_tab_drag_hit_test/*` and `repo_tab_reorder_reduce/*`.
///
/// Simulates a sustained tab drag by performing hit-test position lookups
/// across a tab bar and dispatching `Msg::ReorderRepoTabs` through the
/// reducer.  The fixture splits into two sub-benchmarks:
///
/// - **hit_test**: Pure hit-testing — determines which tab the cursor is over
///   and the insertion point.  This exercises the same logic as
///   `repo_tab_insert_before_for_drop` without going through GPUI.
///
/// - **reorder_reduce**: Full reducer dispatch of `Msg::ReorderRepoTabs`
///   through `dispatch_sync` for each drag step.
pub struct RepoTabDragFixture {
    tab_count: usize,
    tab_width_px: f32,
    baseline: AppState,
}

/// Sidecar metrics for repo tab drag benchmarks.
pub struct RepoTabDragMetrics {
    pub tab_count: u64,
    pub hit_test_steps: u64,
    pub reorder_steps: u64,
    pub effects_emitted: u64,
    pub noop_reorders: u64,
}

impl RepoTabDragFixture {
    pub fn new(tab_count: usize) -> Self {
        let commits = build_synthetic_commits(10);
        let repos: Vec<RepoState> = (0..tab_count)
            .map(|i| {
                let repo_id = RepoId(i as u64 + 1);
                let mut repo = RepoState::new_opening(
                    repo_id,
                    RepoSpec {
                        workdir: std::path::PathBuf::from(format!("/tmp/bench-tab-{i}")),
                    },
                );
                repo.open = Loadable::Ready(());
                repo.log = Loadable::Ready(Arc::new(LogPage {
                    commits: commits.clone(),
                    next_cursor: None,
                }));
                repo
            })
            .collect();

        let active = repos.first().map(|r| r.id);
        Self {
            tab_count,
            tab_width_px: 120.0,
            baseline: AppState {
                repos,
                active_repo: active,
                clone: None,
                notifications: Vec::new(),
                banner_error: None,
                auth_prompt: None,
            },
        }
    }

    /// Hit-test only — determine insert_before for each step across the tab bar.
    pub fn run_hit_test(&self) -> (u64, RepoTabDragMetrics) {
        let repos = &self.baseline.repos;
        let steps = self.tab_count * 3; // sweep across all tabs multiple times
        let total_bar_width = self.tab_count as f32 * self.tab_width_px;

        let mut h = FxHasher::default();
        let mut hit_steps: u64 = 0;

        for step in 0..steps {
            // Simulate cursor position sweeping across the tab bar.
            let frac = (step as f32) / (steps.max(1) as f32);
            let cursor_x = frac * total_bar_width;

            // Determine which tab the cursor is over.
            let tab_ix = (cursor_x / self.tab_width_px) as usize;
            let tab_ix = tab_ix.min(self.tab_count.saturating_sub(1));
            let target_repo_id = repos[tab_ix].id;

            // Replicate repo_tab_insert_before_for_drop logic.
            let tab_left = tab_ix as f32 * self.tab_width_px;
            let tab_center = tab_left + self.tab_width_px / 2.0;
            let insert_before = if cursor_x <= tab_center {
                Some(target_repo_id)
            } else {
                repos.get(tab_ix + 1).map(|r| r.id)
            };

            insert_before.hash(&mut h);
            target_repo_id.0.hash(&mut h);
            hit_steps += 1;
        }

        let metrics = RepoTabDragMetrics {
            tab_count: self.tab_count as u64,
            hit_test_steps: hit_steps,
            reorder_steps: 0,
            effects_emitted: 0,
            noop_reorders: 0,
        };

        (h.finish(), metrics)
    }

    /// Full reducer dispatch — hit-test + reorder_repo_tabs for each step.
    pub fn run_reorder(&self) -> (u64, RepoTabDragMetrics) {
        use gitcomet_state::benchmarks::dispatch_sync;
        use gitcomet_state::msg::Msg;

        let mut state = self.baseline.clone();
        let steps = self.tab_count * 2;
        let total_bar_width = self.tab_count as f32 * self.tab_width_px;

        // We'll drag the first tab across the bar.
        let dragged_repo_id = state.repos[0].id;

        let mut h = FxHasher::default();
        let mut reorder_steps: u64 = 0;
        let mut effects_emitted: u64 = 0;
        let mut noop_reorders: u64 = 0;

        for step in 0..steps {
            let frac = (step as f32) / (steps.max(1) as f32);
            let cursor_x = frac * total_bar_width;

            let tab_ix = (cursor_x / self.tab_width_px) as usize;
            let tab_ix = tab_ix.min(state.repos.len().saturating_sub(1));
            let target_repo_id = state.repos[tab_ix].id;
            let tab_left = tab_ix as f32 * self.tab_width_px;
            let tab_center = tab_left + self.tab_width_px / 2.0;
            let insert_before = if cursor_x <= tab_center {
                Some(target_repo_id)
            } else {
                state.repos.get(tab_ix + 1).map(|r| r.id)
            };

            let effects = dispatch_sync(
                &mut state,
                Msg::ReorderRepoTabs {
                    repo_id: dragged_repo_id,
                    insert_before,
                },
            );

            if effects.is_empty() {
                noop_reorders += 1;
            } else {
                effects_emitted += effects.len() as u64;
            }

            state.active_repo.hash(&mut h);
            state.repos.len().hash(&mut h);
            // Hash the current tab order to detect reorder fidelity.
            for repo in state.repos.iter().take(8) {
                repo.id.0.hash(&mut h);
            }
            reorder_steps += 1;
        }

        let metrics = RepoTabDragMetrics {
            tab_count: self.tab_count as u64,
            hit_test_steps: 0,
            reorder_steps,
            effects_emitted,
            noop_reorders,
        };

        (h.finish(), metrics)
    }
}

pub struct PatchDiffSearchQueryUpdateFixture {
    diff_rows: Vec<AnnotatedDiffLine>,
    click_kinds: Vec<DiffClickKind>,
    word_highlights: Vec<Option<Vec<Range<usize>>>>,
    language_for_src_ix: Vec<Option<DiffSyntaxLanguage>>,
    visible_row_indices: Vec<usize>,
    theme: AppTheme,
    syntax_mode: DiffSyntaxMode,
    stable_cache: Vec<Option<CachedDiffStyledText>>,
    query_cache: Vec<Option<PatchDiffSearchQueryCacheEntry>>,
    query_cache_query: SharedString,
    query_cache_generation: u64,
}

#[derive(Clone)]
struct PatchDiffSearchQueryCacheEntry {
    generation: u64,
    styled: CachedDiffStyledText,
}

impl PatchDiffSearchQueryUpdateFixture {
    pub fn new(lines: usize) -> Self {
        let theme = AppTheme::gitcomet_dark();
        let language = diff_syntax_language_for_path("src/lib.rs");
        let target_lines = lines.max(1);
        let mut diff_rows = Vec::with_capacity(target_lines);
        let mut click_kinds = Vec::with_capacity(target_lines);
        let mut word_highlights = Vec::with_capacity(target_lines);
        let mut language_for_src_ix = Vec::with_capacity(target_lines);

        let mut file_ix = 0usize;
        while diff_rows.len() < target_lines {
            diff_rows.push(AnnotatedDiffLine {
                kind: DiffLineKind::Header,
                text: format!("diff --git a/src/file_{file_ix}.rs b/src/file_{file_ix}.rs").into(),
                old_line: None,
                new_line: None,
            });
            click_kinds.push(DiffClickKind::FileHeader);
            word_highlights.push(None);
            language_for_src_ix.push(None);
            if diff_rows.len() >= target_lines {
                break;
            }

            diff_rows.push(AnnotatedDiffLine {
                kind: DiffLineKind::Hunk,
                text: format!("@@ -1,12 +1,12 @@ fn synthetic_{file_ix}() {{").into(),
                old_line: None,
                new_line: None,
            });
            click_kinds.push(DiffClickKind::HunkHeader);
            word_highlights.push(None);
            language_for_src_ix.push(None);
            if diff_rows.len() >= target_lines {
                break;
            }

            for line_in_file in 0..12 {
                if diff_rows.len() >= target_lines {
                    break;
                }

                let content = format!(
                    "let shared_{file_ix}_{line_in_file} = compute_shared({line_in_file});"
                );
                let (kind, text) = match line_in_file % 3 {
                    0 => (DiffLineKind::Add, format!("+{content}")),
                    1 => (DiffLineKind::Remove, format!("-{content}")),
                    _ => (DiffLineKind::Context, format!(" {content}")),
                };

                let word_start = content.find("shared").unwrap_or(0);
                let word_end = (word_start + "shared".len()).min(content.len());

                diff_rows.push(AnnotatedDiffLine {
                    kind,
                    text: text.into(),
                    old_line: None,
                    new_line: None,
                });
                click_kinds.push(DiffClickKind::Line);
                let ranges = std::iter::once(word_start..word_end).collect::<Vec<_>>();
                word_highlights.push(Some(ranges));
                language_for_src_ix.push(language);
            }

            file_ix = file_ix.saturating_add(1);
        }

        let syntax_mode = if diff_rows.len() > 4_000 {
            DiffSyntaxMode::HeuristicOnly
        } else {
            DiffSyntaxMode::Auto
        };
        let mut fixture = Self {
            visible_row_indices: (0..diff_rows.len()).collect(),
            stable_cache: vec![None; diff_rows.len()],
            query_cache: vec![None; diff_rows.len()],
            query_cache_query: SharedString::default(),
            query_cache_generation: 0,
            diff_rows,
            click_kinds,
            word_highlights,
            language_for_src_ix,
            theme,
            syntax_mode,
        };
        fixture.prewarm_stable_cache();
        fixture
    }

    fn prewarm_stable_cache(&mut self) {
        for src_ix in 0..self.diff_rows.len() {
            let click_kind = self
                .click_kinds
                .get(src_ix)
                .copied()
                .unwrap_or(DiffClickKind::Line);
            if !matches!(click_kind, DiffClickKind::Line) {
                continue;
            }
            let _ = self.row_styled(src_ix, "");
        }
        self.query_cache.fill(None);
        self.query_cache_query = SharedString::default();
        self.query_cache_generation = 0;
    }

    fn sync_query_cache(&mut self, query: &str) {
        if self.query_cache_query.as_ref() != query {
            self.query_cache_query = query.to_string().into();
            self.query_cache_generation = self.query_cache_generation.wrapping_add(1);
        }
    }

    fn row_styled(&mut self, src_ix: usize, query: &str) -> Option<CachedDiffStyledText> {
        let query = query.trim();
        let query_active = !query.is_empty();
        let click_kind = self
            .click_kinds
            .get(src_ix)
            .copied()
            .unwrap_or(DiffClickKind::Line);
        let should_style = matches!(click_kind, DiffClickKind::Line) || query_active;
        if !should_style {
            return None;
        }

        if self
            .stable_cache
            .get(src_ix)
            .and_then(Option::as_ref)
            .is_none()
        {
            let line = self.diff_rows.get(src_ix)?;
            let stable = if matches!(click_kind, DiffClickKind::Line) {
                let word_ranges = self
                    .word_highlights
                    .get(src_ix)
                    .and_then(|ranges| ranges.as_deref())
                    .unwrap_or(&[]);
                let language = self.language_for_src_ix.get(src_ix).copied().flatten();
                let word_color = match line.kind {
                    DiffLineKind::Add => Some(self.theme.colors.diff_add_text),
                    DiffLineKind::Remove => Some(self.theme.colors.diff_remove_text),
                    _ => None,
                };

                super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    diff_content_text(line),
                    word_ranges,
                    "",
                    language,
                    self.syntax_mode,
                    word_color,
                )
            } else {
                super::diff_text::build_cached_diff_styled_text(
                    self.theme,
                    line.text.as_ref(),
                    &[],
                    "",
                    None,
                    self.syntax_mode,
                    None,
                )
            };
            if let Some(slot) = self.stable_cache.get_mut(src_ix) {
                *slot = Some(stable);
            }
        }

        if query_active {
            let query_generation = self.query_cache_generation;
            if self
                .query_cache
                .get(src_ix)
                .and_then(Option::as_ref)
                .is_none_or(|entry| entry.generation != query_generation)
            {
                let base = self.stable_cache.get(src_ix).and_then(Option::as_ref)?;
                let overlay = super::diff_text::build_cached_diff_query_overlay_styled_text(
                    self.theme, base, query,
                );
                if let Some(slot) = self.query_cache.get_mut(src_ix) {
                    *slot = Some(PatchDiffSearchQueryCacheEntry {
                        generation: query_generation,
                        styled: overlay,
                    });
                }
            }
            return self
                .query_cache
                .get(src_ix)
                .and_then(Option::as_ref)
                .filter(|entry| entry.generation == query_generation)
                .map(|entry| entry.styled.clone());
        }

        self.stable_cache
            .get(src_ix)
            .and_then(Option::as_ref)
            .cloned()
    }

    pub fn run_query_update_step(&mut self, query: &str, start: usize, window: usize) -> u64 {
        if self.visible_row_indices.is_empty() || window == 0 {
            return 0;
        }

        self.sync_query_cache(query);
        let start = start % self.visible_row_indices.len();
        let end = (start + window).min(self.visible_row_indices.len());
        let query = self.query_cache_query.clone();

        let mut h = FxHasher::default();
        for visible_ix in start..end {
            let src_ix = self.visible_row_indices[visible_ix];
            src_ix.hash(&mut h);
            if let Some(styled) = self.row_styled(src_ix, query.as_ref()) {
                styled.text_hash.hash(&mut h);
                styled.highlights_hash.hash(&mut h);
            }
        }
        self.stable_cache_entries().hash(&mut h);
        self.query_cache_entries().hash(&mut h);
        h.finish()
    }

    pub fn visible_rows(&self) -> usize {
        self.visible_row_indices.len()
    }

    fn stable_cache_entries(&self) -> usize {
        self.stable_cache
            .iter()
            .filter(|entry| entry.is_some())
            .count()
    }

    fn query_cache_entries(&self) -> usize {
        self.query_cache
            .iter()
            .filter(|entry| {
                entry
                    .as_ref()
                    .is_some_and(|entry| entry.generation == self.query_cache_generation)
            })
            .count()
    }
}

fn prepare_bench_diff_syntax_document(
    language: DiffSyntaxLanguage,
    budget: DiffSyntaxBudget,
    text: &str,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
    let text: SharedString = text.to_owned().into();
    let line_starts: Arc<[usize]> = Arc::from(line_starts_for_text(text.as_ref()));

    prepare_bench_diff_syntax_document_from_shared(
        language,
        budget,
        text,
        line_starts,
        old_document,
    )
}

fn prepare_bench_diff_syntax_document_from_shared(
    language: DiffSyntaxLanguage,
    budget: DiffSyntaxBudget,
    text: SharedString,
    line_starts: Arc<[usize]>,
    old_document: Option<super::diff_text::PreparedDiffSyntaxDocument>,
) -> Option<super::diff_text::PreparedDiffSyntaxDocument> {
    match prepare_diff_syntax_document_with_budget_reuse_text(
        language,
        DiffSyntaxMode::Auto,
        text.clone(),
        Arc::clone(&line_starts),
        budget,
        old_document,
        None,
    ) {
        PrepareDiffSyntaxDocumentResult::Ready(document) => Some(document),
        PrepareDiffSyntaxDocumentResult::TimedOut => {
            let old_reparse_seed = old_document.and_then(prepared_diff_syntax_reparse_seed);
            prepare_diff_syntax_document_in_background_text_with_reuse(
                language,
                DiffSyntaxMode::Auto,
                text,
                line_starts,
                old_reparse_seed,
                None,
            )
            .map(inject_background_prepared_diff_syntax_document)
        }
        PrepareDiffSyntaxDocumentResult::Unsupported => None,
    }
}

fn build_synthetic_repo_state(
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    worktrees: usize,
    submodules: usize,
    stashes: usize,
    commits: &[Commit],
) -> RepoState {
    let id = RepoId(1);
    let spec = RepoSpec {
        workdir: std::path::PathBuf::from("/tmp/bench"),
    };
    let mut repo = RepoState::new_opening(id, spec);

    let head = "main".to_string();
    repo.head_branch = Loadable::Ready(head.clone());

    let target = commits
        .first()
        .map(|c| c.id.clone())
        .unwrap_or_else(|| CommitId("0".repeat(40).into()));

    let mut branches = Vec::with_capacity(local_branches.max(1));
    branches.push(Branch {
        name: head.clone(),
        target: target.clone(),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: head.clone(),
        }),
        divergence: Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        }),
    });
    for ix in 0..local_branches.saturating_sub(1) {
        branches.push(Branch {
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
            upstream: None,
            divergence: None,
        });
    }
    repo.branches = Loadable::Ready(Arc::new(branches));

    let mut remotes_vec = Vec::with_capacity(remotes.max(1));
    for r in 0..remotes.max(1) {
        remotes_vec.push(Remote {
            name: if r == 0 {
                "origin".to_string()
            } else {
                format!("remote{r}")
            },
            url: None,
        });
    }
    repo.remotes = Loadable::Ready(Arc::new(remotes_vec.clone()));

    let mut remote = Vec::with_capacity(remote_branches);
    for ix in 0..remote_branches {
        let remote_name = if remotes <= 1 || ix % remotes == 0 {
            "origin".to_string()
        } else {
            format!("remote{}", ix % remotes)
        };
        remote.push(RemoteBranch {
            remote: remote_name,
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target: target.clone(),
        });
    }
    repo.remote_branches = Loadable::Ready(Arc::new(remote));

    let mut worktrees_vec = Vec::with_capacity(worktrees);
    for ix in 0..worktrees {
        let path = if ix == 0 {
            repo.spec.workdir.clone()
        } else {
            std::path::PathBuf::from(format!("/tmp/bench-worktree-{ix}"))
        };
        worktrees_vec.push(Worktree {
            path,
            head: Some(target.clone()),
            branch: Some(format!("feature/worktree/{ix}")),
            detached: ix % 7 == 0,
        });
    }
    repo.worktrees = Loadable::Ready(Arc::new(worktrees_vec));

    let mut submodules_vec = Vec::with_capacity(submodules);
    for ix in 0..submodules {
        submodules_vec.push(Submodule {
            path: std::path::PathBuf::from(format!("deps/submodule_{ix}")),
            head: CommitId(format!("{:040x}", 200_000usize.saturating_add(ix)).into()),
            status: if ix % 5 == 0 {
                SubmoduleStatus::HeadMismatch
            } else {
                SubmoduleStatus::UpToDate
            },
        });
    }
    repo.submodules = Loadable::Ready(Arc::new(submodules_vec));

    let stash_base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_100_000);
    let mut stashes_vec = Vec::with_capacity(stashes);
    for ix in 0..stashes {
        stashes_vec.push(StashEntry {
            index: ix,
            id: CommitId(format!("{:040x}", 300_000usize.saturating_add(ix)).into()),
            message: format!("WIP synthetic stash #{ix}").into(),
            created_at: Some(stash_base + Duration::from_secs(ix as u64)),
        });
    }
    repo.stashes = Loadable::Ready(Arc::new(stashes_vec));

    // Minimal "repo is open" status.
    repo.open = Loadable::Ready(());
    repo.branch_sidebar_rev = 1;

    repo
}

fn build_repo_switch_repo_state(
    repo_id: RepoId,
    workdir: &str,
    commits: &[Commit],
    local_branches: usize,
    remote_branches: usize,
    remotes: usize,
    status_entries: usize,
    diff_path: Option<&str>,
) -> RepoState {
    let mut repo =
        build_synthetic_repo_state(local_branches, remote_branches, remotes, 0, 0, 24, commits);
    repo.id = repo_id;
    repo.spec = RepoSpec {
        workdir: std::path::PathBuf::from(workdir),
    };

    let log_page = Arc::new(LogPage {
        commits: commits.iter().take(200).cloned().collect(),
        next_cursor: commits.get(200).map(|commit| LogCursor {
            last_seen: commit.id.clone(),
            resume_from: None,
        }),
    });
    repo.history_state.log = Loadable::Ready(Arc::clone(&log_page));
    repo.history_state.log_rev = 1;
    repo.log = Loadable::Ready(log_page);
    repo.log_rev = 1;

    repo.status = Loadable::Ready(Arc::new(build_synthetic_repo_status(status_entries)));
    repo.status_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(build_tags_targeting_commits(commits, 32)));
    repo.tags_rev = 1;
    repo.remote_tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_tags_rev = 1;
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.merge_commit_message = Loadable::Ready(None);
    repo.merge_message_rev = 1;
    repo.open_rev = 1;
    repo.last_active_at = Some(SystemTime::now());

    if let Some(selected_commit) = commits.first() {
        repo.history_state.selected_commit = Some(selected_commit.id.clone());
        repo.history_state.selected_commit_rev = 1;
        repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
            id: selected_commit.id.clone(),
            message: format!(
                "Synthetic selected commit for {}",
                repo.spec.workdir.display()
            ),
            committed_at: "2023-11-14 22:13".to_string(),
            parent_ids: selected_commit.parent_ids.clone(),
            files: (0..48)
                .map(|ix| CommitFileChange {
                    path: std::path::PathBuf::from(format!("src/module_{}/file_{ix}.rs", ix % 12)),
                    kind: if ix % 5 == 0 {
                        FileStatusKind::Added
                    } else {
                        FileStatusKind::Modified
                    },
                })
                .collect(),
        }));
        repo.history_state.commit_details_rev = 1;
    }

    if let Some(path) = diff_path {
        repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
            path: std::path::PathBuf::from(path),
            area: DiffArea::Unstaged,
        });
        repo.diff_state.diff_state_rev = 1;
        repo.diff_state.diff_rev = 1;
    }

    repo
}

/// Populate a repo's diff state with fully loaded diff content + file text,
/// simulating a user who has a file diff open and visible.
fn populate_loaded_diff_state(repo: &mut RepoState, path: &str, diff_line_count: usize) {
    let target = DiffTarget::WorkingTree {
        path: std::path::PathBuf::from(path),
        area: DiffArea::Unstaged,
    };
    repo.diff_state.diff = Loadable::Ready(Arc::new(Diff {
        target: target.clone(),
        lines: build_synthetic_diff_lines(diff_line_count),
    }));
    repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(FileDiffText {
        path: std::path::PathBuf::from(path),
        old: Some(build_synthetic_file_content(diff_line_count / 2)),
        new: Some(build_synthetic_file_content(
            diff_line_count / 2 + diff_line_count / 4,
        )),
    })));
    repo.diff_state.diff_file_rev = 1;
}

/// Set the conflict state on a repo so the diff target is recognized as a
/// conflict path by the reducer.
fn populate_conflict_state(repo: &mut RepoState, path: &str, line_count: usize) {
    let path_buf = std::path::PathBuf::from(path);
    repo.conflict_state.conflict_file_path = Some(path_buf.clone());
    let content: Arc<str> = Arc::from(build_synthetic_file_content(line_count));
    repo.conflict_state.conflict_file = Loadable::Ready(Some(ConflictFile {
        path: path_buf,
        base_bytes: None,
        ours_bytes: None,
        theirs_bytes: None,
        current_bytes: None,
        base: Some(Arc::clone(&content)),
        ours: Some(Arc::clone(&content)),
        theirs: Some(Arc::clone(&content)),
        current: Some(content),
    }));
    repo.conflict_state.conflict_rev = 1;
}

fn build_synthetic_diff_lines(count: usize) -> Vec<DiffLine> {
    let mut lines = Vec::with_capacity(count);
    lines.push(DiffLine {
        kind: DiffLineKind::Header,
        text: Arc::from("diff --git a/src/main.rs b/src/main.rs"),
    });
    lines.push(DiffLine {
        kind: DiffLineKind::Header,
        text: Arc::from("index abc1234..def5678 100644"),
    });

    let remaining = count.saturating_sub(2);
    let mut ix = 0;
    while ix < remaining {
        if ix % 50 == 0 {
            lines.push(DiffLine {
                kind: DiffLineKind::Hunk,
                text: Arc::from(
                    format!(
                        "@@ -{0},{1} +{0},{1} @@ fn synthetic_function_{0}()",
                        ix + 1,
                        50.min(remaining - ix)
                    )
                    .as_str(),
                ),
            });
            ix += 1;
            if ix >= remaining {
                break;
            }
        }

        let kind = match ix % 7 {
            0 | 1 | 2 | 3 => DiffLineKind::Context,
            4 | 5 => DiffLineKind::Add,
            _ => DiffLineKind::Remove,
        };
        lines.push(DiffLine {
            kind,
            text: Arc::from(
                format!("    let synthetic_var_{ix} = compute_value({ix}); // line {ix}").as_str(),
            ),
        });
        ix += 1;
    }

    lines
}

fn build_synthetic_file_content(line_count: usize) -> String {
    let mut content = String::with_capacity(line_count * 60);
    for ix in 0..line_count {
        content.push_str(&format!("    let line_{ix} = process_data({ix});\n"));
    }
    content
}

fn build_repo_switch_minimal_repo_state(repo_id: RepoId, workdir: &str) -> RepoState {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: std::path::PathBuf::from(workdir),
        },
    );
    repo.open = Loadable::Ready(());
    repo.open_rev = 1;
    repo
}

fn build_synthetic_repo_status(entries: usize) -> RepoStatus {
    let mut status = RepoStatus::default();
    status.unstaged = build_synthetic_status_entries(entries, DiffArea::Unstaged);
    status
}

fn build_synthetic_status_entries(entries: usize, area: DiffArea) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        let (path, kind) = match area {
            DiffArea::Unstaged => (
                std::path::PathBuf::from(format!("src/{:02}/nested/path/file_{ix}.rs", ix % 24)),
                if ix % 11 == 0 {
                    FileStatusKind::Added
                } else {
                    FileStatusKind::Modified
                },
            ),
            DiffArea::Staged => (
                std::path::PathBuf::from(format!(
                    "release/{:02}/deploy/assets/staged_file_{ix}.toml",
                    ix % 32
                )),
                match ix % 13 {
                    0 => FileStatusKind::Deleted,
                    1 => FileStatusKind::Renamed,
                    2 => FileStatusKind::Added,
                    _ => FileStatusKind::Modified,
                },
            ),
        };

        items.push(FileStatus {
            path,
            kind,
            conflict: None,
        });
    }
    items
}

fn build_synthetic_partially_staged_entries(entries: usize) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        items.push(FileStatus {
            path: std::path::PathBuf::from(format!(
                "src/{:02}/partially_staged/file_{ix:04}.rs",
                ix % 24
            )),
            kind: FileStatusKind::Modified,
            conflict: None,
        });
    }
    items
}

fn build_synthetic_status_entries_mixed_depth(entries: usize) -> Vec<FileStatus> {
    let mut items = Vec::with_capacity(entries);
    for ix in 0..entries {
        let mut path = match ix % 4 {
            0 => std::path::PathBuf::from("src"),
            1 => std::path::PathBuf::from("docs"),
            2 => std::path::PathBuf::from("assets"),
            _ => std::path::PathBuf::from("crates"),
        };
        let extra_depth = 2 + (ix % 12);
        for depth_ix in 0..extra_depth {
            path.push(format!(
                "segment_{depth_ix:02}_{}_{:03}",
                ix % 23,
                (ix.wrapping_mul(17).wrapping_add(depth_ix)) % 257
            ));
        }
        let extension = match ix % 5 {
            0 => "rs",
            1 => "toml",
            2 => "md",
            3 => "json",
            _ => "yaml",
        };
        path.push(format!("file_{ix:05}.{extension}"));

        let kind = match ix % 11 {
            0 => FileStatusKind::Added,
            1 => FileStatusKind::Deleted,
            2 => FileStatusKind::Renamed,
            3 => FileStatusKind::Conflicted,
            _ => FileStatusKind::Modified,
        };
        items.push(FileStatus {
            path,
            kind,
            conflict: None,
        });
    }
    items
}

fn build_synthetic_commits(count: usize) -> Vec<Commit> {
    build_synthetic_commits_with_merge_stride(count, 50, 40)
}

fn build_synthetic_commits_with_merge_stride(
    count: usize,
    merge_every: usize,
    merge_back_distance: usize,
) -> Vec<Commit> {
    if count == 0 {
        return Vec::new();
    }

    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let mut commits = Vec::with_capacity(count);

    for ix in 0..count {
        let id = CommitId(format!("{:040x}", ix).into());

        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1).into()));
        }
        // Synthetic merge-like commits at a fixed cadence.
        if merge_every > 0
            && merge_back_distance > 0
            && ix >= merge_back_distance
            && ix % merge_every == 0
        {
            parent_ids.push(CommitId(
                format!("{:040x}", ix - merge_back_distance).into(),
            ));
        }

        commits.push(Commit {
            id,
            parent_ids,
            summary: format!("Commit {ix} - synthetic benchmark history entry").into(),
            author: format!("Author {}", ix % 10).into(),
            time: base + Duration::from_secs(ix as u64),
        });
    }

    // Most history/UI code expects log order: newest commit first, then older commits.
    // Returning the synthetic history in ascending order creates a pathological graph where
    // every commit appears to open a fresh lane before its parent is encountered.
    commits.reverse();
    commits
}

/// Build branches whose targets are spread across the commit list rather than
/// all pointing at the first commit, giving a realistic decoration-map workload.
fn build_branches_targeting_commits(
    commits: &[Commit],
    local_count: usize,
    remote_count: usize,
) -> (Vec<Branch>, Vec<RemoteBranch>) {
    let first_target = commits
        .first()
        .map(|c| c.id.clone())
        .unwrap_or_else(|| CommitId("0".repeat(40).into()));

    let mut branches = Vec::with_capacity(local_count.max(1));
    branches.push(Branch {
        name: "main".to_string(),
        target: first_target.clone(),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: Some(UpstreamDivergence {
            ahead: 1,
            behind: 2,
        }),
    });
    let commit_len = commits.len().max(1);
    for ix in 0..local_count.saturating_sub(1) {
        let target_ix = (ix.wrapping_mul(7)) % commit_len;
        let target = commits
            .get(target_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| first_target.clone());
        branches.push(Branch {
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target,
            upstream: None,
            divergence: None,
        });
    }

    let mut remote = Vec::with_capacity(remote_count);
    for ix in 0..remote_count {
        let target_ix = (ix.wrapping_mul(13)) % commit_len;
        let target = commits
            .get(target_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| first_target.clone());
        let remote_name = if ix % 4 == 0 {
            "origin".to_string()
        } else {
            format!("upstream{}", ix % 3)
        };
        remote.push(RemoteBranch {
            remote: remote_name,
            name: format!("feature/{}/topic/{ix}", ix % 100),
            target,
        });
    }

    (branches, remote)
}

/// Build tags whose targets are spread across the commit list.
fn build_tags_targeting_commits(commits: &[Commit], count: usize) -> Vec<Tag> {
    let commit_len = commits.len().max(1);
    let mut tags = Vec::with_capacity(count);
    for ix in 0..count {
        let target_ix = (ix.wrapping_mul(11)) % commit_len;
        let target = commits
            .get(target_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| CommitId("0".repeat(40).into()));
        tags.push(Tag {
            name: format!("v{}.{}.{}", ix / 100, (ix / 10) % 10, ix % 10),
            target,
        });
    }
    tags
}

/// Build simple stash entries whose IDs do NOT match any commit in the log.
/// Use this for scenarios where stash entries exist but no stash-like commits
/// appear in the commit list (balanced scenario).
fn build_simple_stash_entries(count: usize) -> (Vec<StashEntry>, Vec<Commit>) {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_200_000);
    let mut entries = Vec::with_capacity(count);
    for ix in 0..count {
        entries.push(StashEntry {
            index: ix,
            id: CommitId(format!("{:040x}", 500_000usize.saturating_add(ix)).into()),
            message: format!("On main: stash message {ix}").into(),
            created_at: Some(base + Duration::from_secs(ix as u64)),
        });
    }
    (entries, Vec::new())
}

/// Build stash entries with matching stash-like commits and their helper (index)
/// commits, injected into the log so the full stash filtering path fires.
fn build_stash_fixture_commits(
    base_commits: &[Commit],
    stash_count: usize,
    start_ix: usize,
) -> (Vec<StashEntry>, Vec<Commit>) {
    let base_time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_200_000);
    let base_len = base_commits.len().max(1);
    let mut stash_entries = Vec::with_capacity(stash_count);
    let mut extra_commits = Vec::with_capacity(stash_count * 2);

    for i in 0..stash_count {
        let parent_ix = i % base_len;
        let parent_id = base_commits
            .get(parent_ix)
            .map(|c| c.id.clone())
            .unwrap_or_else(|| CommitId(format!("{:040x}", 0).into()));

        // Stash helper (index commit) — secondary parent of the stash tip
        let helper_ix = start_ix + i * 2;
        let helper_id = CommitId(format!("{:040x}", helper_ix).into());
        extra_commits.push(Commit {
            id: helper_id.clone(),
            parent_ids: vec![parent_id.clone()],
            summary: format!("index on main: {i}").into(),
            author: "Author 0".into(),
            time: base_time + Duration::from_secs(i as u64 * 2),
        });

        // Stash tip — 2 parents, stash-like summary
        let tip_ix = start_ix + i * 2 + 1;
        let tip_id = CommitId(format!("{:040x}", tip_ix).into());
        extra_commits.push(Commit {
            id: tip_id.clone(),
            parent_ids: vec![parent_id, helper_id],
            summary: format!("WIP on main: stash message {i}").into(),
            author: "Author 0".into(),
            time: base_time + Duration::from_secs(i as u64 * 2 + 1),
        });

        stash_entries.push(StashEntry {
            index: i,
            id: tip_id,
            message: format!("On main: stash message {i}").into(),
            created_at: Some(base_time + Duration::from_secs(i as u64 * 2 + 1)),
        });
    }

    (stash_entries, extra_commits)
}

fn build_synthetic_commit_details(files: usize, depth: usize) -> CommitDetails {
    build_synthetic_commit_details_with_message(
        files,
        depth,
        "Synthetic benchmark commit details message\n\nWith body.".to_string(),
    )
}

fn build_synthetic_commit_details_with_message(
    files: usize,
    depth: usize,
    message: String,
) -> CommitDetails {
    let id = CommitId("d".repeat(40).into());
    let mut out = Vec::with_capacity(files);
    for ix in 0..files {
        let kind = match ix % 23 {
            0 => FileStatusKind::Deleted,
            1 | 2 => FileStatusKind::Renamed,
            3..=5 => FileStatusKind::Added,
            6 => FileStatusKind::Conflicted,
            7 => FileStatusKind::Untracked,
            _ => FileStatusKind::Modified,
        };

        let mut path = std::path::PathBuf::new();
        let depth = depth.max(1);
        for d in 0..depth {
            path.push(format!("dir{}_{}", d, ix % 128));
        }
        path.push(format!("file_{ix}.rs"));

        out.push(CommitFileChange { path, kind });
    }

    CommitDetails {
        id,
        message,
        committed_at: "2024-01-01T00:00:00Z".to_string(),
        parent_ids: vec![CommitId("c".repeat(40).into())],
        files: out,
    }
}

/// Like `build_synthetic_commit_details` but with a different commit ID
/// (the `id_char` is repeated 40 times to form the ID hex string).
fn build_synthetic_commit_details_with_id(
    files: usize,
    depth: usize,
    id_char: &str,
) -> CommitDetails {
    let mut details = build_synthetic_commit_details(files, depth);
    details.id = CommitId(id_char.repeat(40).into());
    details.parent_ids = vec![CommitId("d".repeat(40).into())];
    details
}

/// Like `build_synthetic_commit_details` but every file path is globally unique
/// (no `ix % 128` clamping on directory names). This produces files that all
/// miss the path-display cache, triggering cache clears for lists > 8192.
fn build_synthetic_commit_details_unique_paths(files: usize, depth: usize) -> CommitDetails {
    let id = CommitId("f".repeat(40).into());
    let depth = depth.max(1);
    let mut out = Vec::with_capacity(files);
    for ix in 0..files {
        let kind = match ix % 23 {
            0 => FileStatusKind::Deleted,
            1 | 2 => FileStatusKind::Renamed,
            3..=5 => FileStatusKind::Added,
            6 => FileStatusKind::Conflicted,
            7 => FileStatusKind::Untracked,
            _ => FileStatusKind::Modified,
        };
        let mut path = std::path::PathBuf::new();
        for d in 0..depth {
            // Use (ix / 256) and (ix % 256) to spread across unique directory names.
            path.push(format!("dir{}_{}_{}", d, ix / 256, ix % 256));
        }
        path.push(format!("file_{ix}.rs"));
        out.push(CommitFileChange { path, kind });
    }
    CommitDetails {
        id,
        message: "Synthetic commit details with unique paths for cache churn benchmark".to_string(),
        committed_at: "2024-01-01T00:00:00Z".to_string(),
        parent_ids: vec![CommitId("d".repeat(40).into())],
        files: out,
    }
}

fn build_synthetic_commit_message(min_bytes: usize, line_bytes: usize) -> String {
    let min_bytes = min_bytes.max(1);
    let line_bytes = line_bytes.max(40);
    let mut message = String::from("Synthetic benchmark commit subject\n\n");
    let line_count = min_bytes
        .saturating_div(line_bytes.max(1))
        .saturating_add(16)
        .max(16);
    let body_lines = build_synthetic_source_lines(line_count, line_bytes);
    for (ix, line) in body_lines.iter().enumerate() {
        message.push_str(line.as_str());
        message.push('\n');
        if ix % 8 == 7 {
            message.push('\n');
        }
    }
    while message.len() < min_bytes {
        message.push_str("benchmark body filler line for commit details rendering coverage\n");
    }
    message
}

fn count_commit_message_lines(message: &str) -> usize {
    if message.is_empty() {
        1
    } else {
        message.lines().count().max(1)
    }
}

fn measure_commit_message_visible_window(
    message: &str,
    render: Option<CommitDetailsMessageRenderConfig>,
) -> (usize, usize) {
    let Some(render) = render else {
        return (0, 0);
    };

    let visible_lines = render.visible_lines.max(1);
    let mut shaped_lines = 0usize;
    let mut shaped_bytes = 0usize;
    for line in message.lines().take(visible_lines) {
        let (_, capped_len) = hash_text_input_shaping_slice(line, render.max_shape_bytes.max(1));
        shaped_lines = shaped_lines.saturating_add(1);
        shaped_bytes = shaped_bytes.saturating_add(capped_len);
    }
    (shaped_lines, shaped_bytes)
}

fn commit_details_message_hash(
    message: &str,
    render: Option<CommitDetailsMessageRenderConfig>,
    hasher: &mut FxHasher,
) {
    let Some(render) = render else {
        message.len().hash(hasher);
        return;
    };

    let model = TextModel::from_large_text(message);
    let snapshot = model.snapshot();
    snapshot.len().hash(hasher);
    snapshot.line_starts().len().hash(hasher);

    let wrap_columns = wrap_columns_for_benchmark_width(render.wrap_width_px);
    let visible_lines = render.visible_lines.max(1);
    let mut shaped_lines = 0usize;
    let mut shaped_bytes = 0usize;

    for line in message.lines().take(visible_lines) {
        let (slice_hash, capped_len) =
            hash_text_input_shaping_slice(line, render.max_shape_bytes.max(1));
        slice_hash.hash(hasher);
        capped_len.hash(hasher);
        estimate_tabbed_wrap_rows(line, wrap_columns).hash(hasher);
        shaped_lines = shaped_lines.saturating_add(1);
        shaped_bytes = shaped_bytes.saturating_add(capped_len);
    }

    shaped_lines.hash(hasher);
    shaped_bytes.hash(hasher);
}

/// Hash a CommitDetails through the same per-row work loop used in the
/// existing CommitDetailsFixture::run(). Shared by select_commit_replace.
fn commit_details_row_hash(details: &CommitDetails) -> u64 {
    let mut h = FxHasher::default();
    details.id.as_ref().hash(&mut h);
    details.message.len().hash(&mut h);
    let mut counts = [0usize; 6];
    for f in &details.files {
        let visuals = commit_file_kind_visuals(f.kind);
        Some(visuals.icon).hash(&mut h);
        visuals.kind_key.hash(&mut h);
        // Production uses path.to_str() → SharedString::new() (no intermediate String).
        let path_text = f.path.to_str().unwrap_or("");
        path_text.hash(&mut h);
        counts[visuals.kind_key as usize] = counts[visuals.kind_key as usize].saturating_add(1);
    }
    counts.hash(&mut h);
    h.finish()
}

fn build_synthetic_source_lines(count: usize, target_line_bytes: usize) -> Vec<String> {
    let target_line_bytes = target_line_bytes.max(32);
    let mut lines = Vec::with_capacity(count);
    for ix in 0..count {
        let indent = " ".repeat((ix % 8) * 2);
        let mut line = match ix % 10 {
            0 => format!("{indent}fn func_{ix}(x: usize) -> usize {{ x + {ix} }}"),
            1 => format!("{indent}let value_{ix} = \"string {ix}\";"),
            2 => format!("{indent}// comment {ix} with some extra words and tokens"),
            3 => format!("{indent}if value_{ix} > 10 {{ return value_{ix}; }}"),
            4 => format!(
                "{indent}for i in 0..{r} {{ sum += i; }}",
                r = (ix % 100) + 1
            ),
            5 => format!("{indent}match tag_{ix} {{ Some(v) => v, None => 0 }}"),
            6 => format!("{indent}struct S{ix} {{ a: i32, b: String }}"),
            7 => format!(
                "{indent}impl S{ix} {{ fn new() -> Self {{ Self {{ a: 0, b: String::new() }} }} }}"
            ),
            8 => format!("{indent}const CONST_{ix}: u64 = {v};", v = ix as u64 * 31),
            _ => format!("{indent}println!(\"{ix} {{}}\", value_{ix});"),
        };
        if line.len() < target_line_bytes {
            line.push(' ');
            line.push_str("//");
            while line.len() < target_line_bytes {
                line.push_str(" token_");
                line.push_str(&(ix % 997).to_string());
            }
        }
        lines.push(line);
    }
    lines
}

fn hash_file_diff_plan(plan: &gitcomet_core::file_diff::FileDiffPlan) -> u64 {
    let mut h = FxHasher::default();
    plan.row_count.hash(&mut h);
    plan.inline_row_count.hash(&mut h);
    match plan.eof_newline {
        Some(gitcomet_core::file_diff::FileDiffEofNewline::MissingInOld) => 1u8,
        Some(gitcomet_core::file_diff::FileDiffEofNewline::MissingInNew) => 2u8,
        None => 0u8,
    }
    .hash(&mut h);
    plan.runs.len().hash(&mut h);
    for run in plan.runs.iter().take(256) {
        std::mem::discriminant(run).hash(&mut h);
        match run {
            gitcomet_core::file_diff::FileDiffPlanRun::Context {
                old_start,
                new_start,
                len,
            } => {
                old_start.hash(&mut h);
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Remove { old_start, len } => {
                old_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Add { new_start, len } => {
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
            gitcomet_core::file_diff::FileDiffPlanRun::Modify {
                old_start,
                new_start,
                len,
            } => {
                old_start.hash(&mut h);
                new_start.hash(&mut h);
                len.hash(&mut h);
            }
        }
    }
    h.finish()
}

fn build_synthetic_replacement_alignment_documents(
    blocks: usize,
    old_block_lines: usize,
    new_block_lines: usize,
    context_lines: usize,
    target_line_bytes: usize,
) -> (String, String) {
    let blocks = blocks.max(1);
    let old_block_lines = old_block_lines.max(1);
    let new_block_lines = new_block_lines.max(1);
    let context_lines = context_lines.max(1);
    let target_line_bytes = target_line_bytes.max(80);

    let mut old_lines = Vec::new();
    let mut new_lines = Vec::new();
    old_lines.push("fn replacement_alignment_fixture() {".to_string());
    new_lines.push("fn replacement_alignment_fixture() {".to_string());

    for block_ix in 0..blocks {
        for context_ix in 0..context_lines {
            let line =
                build_synthetic_replacement_context_line(block_ix, context_ix, target_line_bytes);
            old_lines.push(line.clone());
            new_lines.push(line);
        }

        for line_ix in 0..old_block_lines {
            old_lines.push(build_synthetic_replacement_change_line(
                block_ix,
                line_ix,
                old_block_lines,
                target_line_bytes,
                "before",
            ));
        }
        for line_ix in 0..new_block_lines {
            new_lines.push(build_synthetic_replacement_change_line(
                block_ix,
                line_ix,
                new_block_lines,
                target_line_bytes,
                "after",
            ));
        }
    }

    old_lines.push("}".to_string());
    new_lines.push("}".to_string());

    let mut old_text = old_lines.join("\n");
    old_text.push('\n');
    let mut new_text = new_lines.join("\n");
    new_text.push('\n');
    (old_text, new_text)
}

fn build_synthetic_replacement_context_line(
    block_ix: usize,
    context_ix: usize,
    target_line_bytes: usize,
) -> String {
    let mut line = format!(
        "    let context_{block_ix:03}_{context_ix:03} = stable_anchor(block_{block_ix:03}, {context_ix});"
    );
    if line.len() < target_line_bytes {
        line.push(' ');
        line.push_str("//");
        while line.len() < target_line_bytes {
            line.push_str(" keep_anchor");
        }
    }
    line
}

fn build_synthetic_replacement_change_line(
    block_ix: usize,
    line_ix: usize,
    block_lines: usize,
    target_line_bytes: usize,
    variant: &str,
) -> String {
    let logical_span = block_lines.max(1);
    let rotated_ix = (line_ix + (block_ix % 7) + 1) % logical_span;
    let logical_ix = if variant == "before" {
        line_ix
    } else {
        rotated_ix
    };

    let mut line = format!(
        "    let block_{block_ix:03}_slot_{logical_ix:03} = reconcile_entry(namespace::{variant}_source_{logical_ix:03}, synth_payload(block_{block_ix:03}, {logical_ix}), \"shared-payload-{block_ix:03}-{logical_ix:03}\");"
    );
    if line.len() < target_line_bytes {
        line.push(' ');
        line.push_str("//");
        while line.len() < target_line_bytes {
            if variant == "before" {
                line.push_str(" before_token");
            } else {
                line.push_str(" after_token");
            }
        }
    }
    line
}

fn line_starts_for_text(text: &str) -> Vec<usize> {
    let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
    line_starts.push(0);
    for newline_ix in memchr::memchr_iter(b'\n', text.as_bytes()) {
        line_starts.push(newline_ix.saturating_add(1));
    }
    line_starts
}

fn build_text_input_streamed_highlights(
    text: &str,
    line_starts: &[usize],
    density: TextInputHighlightDensity,
) -> Vec<(Range<usize>, gpui::HighlightStyle)> {
    let theme = AppTheme::gitcomet_dark();
    let style_primary = gpui::HighlightStyle {
        color: Some(theme.colors.accent.into()),
        ..gpui::HighlightStyle::default()
    };
    let style_secondary = gpui::HighlightStyle {
        color: Some(theme.colors.warning.into()),
        ..gpui::HighlightStyle::default()
    };
    let style_overlay = gpui::HighlightStyle {
        color: Some(theme.colors.success.into()),
        ..gpui::HighlightStyle::default()
    };

    let mut highlights = Vec::new();
    for line_ix in 0..line_starts.len() {
        let line_start = line_starts.get(line_ix).copied().unwrap_or(0);
        let mut line_end = line_starts.get(line_ix + 1).copied().unwrap_or(text.len());
        if line_end > line_start && text.as_bytes().get(line_end - 1) == Some(&b'\n') {
            line_end = line_end.saturating_sub(1);
        }
        if line_end <= line_start {
            continue;
        }
        let line_len = line_end.saturating_sub(line_start);

        match density {
            TextInputHighlightDensity::Dense => {
                let mut local = 0usize;
                while local + 2 < line_len {
                    let start = line_start + local;
                    let end = (start + 20).min(line_end);
                    if start < end {
                        let style = if local.is_multiple_of(24) {
                            style_primary
                        } else {
                            style_secondary
                        };
                        highlights.push((start..end, style));
                    }

                    let overlap_start = start.saturating_add(4).min(line_end);
                    let overlap_end = (overlap_start + 14).min(line_end);
                    if overlap_start < overlap_end {
                        highlights.push((overlap_start..overlap_end, style_overlay));
                    }

                    local = local.saturating_add(12);
                }
            }
            TextInputHighlightDensity::Sparse => {
                if line_ix % 8 == 0 {
                    let start = line_start.saturating_add(2).min(line_end);
                    let end = (start + 26).min(line_end);
                    if start < end {
                        highlights.push((start..end, style_primary));
                    }
                }
                if line_ix % 24 == 0 {
                    let start = line_start.saturating_add(10).min(line_end);
                    let end = (start + 18).min(line_end);
                    if start < end {
                        highlights.push((start..end, style_overlay));
                    }
                }
            }
        }
    }

    highlights.sort_by(|(a, _), (b, _)| a.start.cmp(&b.start).then(a.end.cmp(&b.end)));
    highlights
}

// ---------------------------------------------------------------------------
// Scrollbar drag step fixture
// ---------------------------------------------------------------------------

/// Benchmark fixture for `scrollbar_drag_step/window_200`.
///
/// Simulates 200 scrollbar-thumb drag steps along the vertical track of a
/// scrollable list (e.g., a 10,000-row history view in a 200-row viewport).
///
/// Each step:
/// 1. Computes `vertical_thumb_metrics` to get the current thumb position/size.
/// 2. Moves the simulated mouse position by a fixed pixel step.
/// 3. Calls `compute_vertical_click_offset` to translate the mouse position
///    to a new scroll offset — the same math that runs on every
///    `MouseMoveEvent` during a real scrollbar drag.
/// 4. Updates the scroll position for the next iteration.
///
/// The drag oscillates back and forth, reversing when the scroll offset
/// reaches the top or bottom of the content.
pub struct ScrollbarDragStepFixture {
    /// Height of the visible viewport in pixels.
    viewport_h: f32,
    /// Maximum scroll offset (content_height - viewport_height).
    max_offset: f32,
    /// Current vertical scroll offset.
    scroll_y: f32,
    /// Track top Y position.
    track_top: f32,
    /// Track height (viewport_h - 2*margin).
    track_h: f32,
    /// Pixel step per drag event along the track.
    drag_step_px: f32,
    /// Current direction (+1.0 = down, -1.0 = up).
    drag_direction: f32,
    /// Number of drag steps per benchmark iteration.
    steps: usize,
}

/// Sidecar metrics for scrollbar drag step benchmarks.
pub struct ScrollbarDragStepMetrics {
    pub steps: u64,
    pub thumb_metric_recomputes: u64,
    pub scroll_offset_recomputes: u64,
    pub viewport_h: f64,
    pub max_offset: f64,
    pub min_scroll_y: f64,
    pub max_scroll_y: f64,
    pub min_thumb_offset_px: f64,
    pub max_thumb_offset_px: f64,
    pub min_thumb_length_px: f64,
    pub max_thumb_length_px: f64,
    pub clamp_at_top_count: u64,
    pub clamp_at_bottom_count: u64,
}

impl ScrollbarDragStepFixture {
    /// Create a fixture simulating 200 scrollbar-drag steps in a realistic
    /// viewport over a long list.
    ///
    /// Viewport: 800 px (≈33 rows at 24 px each).
    /// Content: 10,000 rows × 24 px = 240,000 px.
    /// Track: 800 − 2×4 margin = 792 px.
    /// Drag step: 12 px → 200 steps = 2,400 px ≈ 3× track traversals,
    /// guaranteeing multiple oscillation reversals at both ends.
    pub fn window_200() -> Self {
        let row_height = 24.0_f32;
        let total_rows = 10_000;
        let viewport_h = 800.0_f32;
        let content_h = row_height * total_rows as f32;
        let max_offset = content_h - viewport_h;
        let margin = 4.0_f32;
        let track_h = viewport_h - margin * 2.0;

        Self {
            viewport_h,
            max_offset,
            scroll_y: 0.0,
            track_top: margin,
            track_h,
            drag_step_px: 12.0,
            drag_direction: 1.0,
            steps: 200,
        }
    }

    pub fn run(&mut self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&mut self) -> (u64, ScrollbarDragStepMetrics) {
        use crate::kit::{compute_vertical_click_offset, vertical_thumb_metrics};
        use gpui::{Bounds, point, px, size};

        let mut h = FxHasher::default();
        let mut min_scroll_y = f64::MAX;
        let mut max_scroll_y = f64::MIN;
        let mut min_thumb_offset = f64::MAX;
        let mut max_thumb_offset = f64::MIN;
        let mut min_thumb_length = f64::MAX;
        let mut max_thumb_length = f64::MIN;
        let mut clamp_at_top: u64 = 0;
        let mut clamp_at_bottom: u64 = 0;
        let mut thumb_metric_recomputes: u64 = 0;
        let mut scroll_offset_recomputes: u64 = 0;

        // Build a synthetic track bounds matching the scrollbar layout:
        // track starts at (0, margin) with width=16 and height=track_h.
        let track_bounds = Bounds::new(
            point(px(0.0), px(self.track_top)),
            size(px(16.0), px(self.track_h)),
        );

        // Start with the current mouse Y at the thumb centre.
        let initial_thumb =
            vertical_thumb_metrics(px(self.viewport_h), px(self.max_offset), px(self.scroll_y));
        let mut mouse_y = match initial_thumb {
            Some(tm) => {
                let off: f32 = tm.offset.into();
                let len: f32 = tm.length.into();
                off + len / 2.0
            }
            None => self.track_top + self.track_h / 2.0,
        };

        for _ in 0..self.steps {
            // 1) Compute thumb metrics at the current scroll position.
            let thumb =
                vertical_thumb_metrics(px(self.viewport_h), px(self.max_offset), px(self.scroll_y));
            thumb_metric_recomputes = thumb_metric_recomputes.saturating_add(1);

            let (thumb_size, thumb_length_f, thumb_offset_f) = match thumb {
                Some(tm) => {
                    let len: f32 = tm.length.into();
                    let off: f32 = tm.offset.into();
                    (tm.length, len, off)
                }
                None => (px(24.0), 24.0_f32, self.track_top),
            };

            min_thumb_offset = min_thumb_offset.min(thumb_offset_f as f64);
            max_thumb_offset = max_thumb_offset.max(thumb_offset_f as f64);
            min_thumb_length = min_thumb_length.min(thumb_length_f as f64);
            max_thumb_length = max_thumb_length.max(thumb_length_f as f64);

            // 2) Advance simulated mouse position.
            mouse_y += self.drag_step_px * self.drag_direction;

            // Clamp to track bounds.
            let track_bottom = self.track_top + self.track_h;
            if mouse_y <= self.track_top {
                mouse_y = self.track_top;
                clamp_at_top += 1;
                self.drag_direction = -self.drag_direction;
            } else if mouse_y >= track_bottom {
                mouse_y = track_bottom;
                clamp_at_bottom += 1;
                self.drag_direction = -self.drag_direction;
            }

            // 3) Compute new scroll offset using the production offset function.
            //    Use thumb_size/2 as the grab offset (simulating grab at thumb centre).
            let new_offset = compute_vertical_click_offset(
                px(mouse_y),
                track_bounds,
                thumb_size,
                thumb_size / 2.0,
                px(self.max_offset),
                -1, // negative sign matches the default GPUI scroll direction
            );
            scroll_offset_recomputes = scroll_offset_recomputes.saturating_add(1);

            // The function returns a negative offset for sign=-1, take abs.
            let new_scroll: f32 = (-new_offset).into();
            self.scroll_y = new_scroll.max(0.0).min(self.max_offset);

            min_scroll_y = min_scroll_y.min(self.scroll_y as f64);
            max_scroll_y = max_scroll_y.max(self.scroll_y as f64);

            // Hash to prevent dead-code elimination.
            self.scroll_y.to_bits().hash(&mut h);
            thumb_offset_f.to_bits().hash(&mut h);
            thumb_length_f.to_bits().hash(&mut h);
            self.drag_direction.to_bits().hash(&mut h);
        }

        let metrics = ScrollbarDragStepMetrics {
            steps: self.steps as u64,
            thumb_metric_recomputes,
            scroll_offset_recomputes,
            viewport_h: self.viewport_h as f64,
            max_offset: self.max_offset as f64,
            min_scroll_y,
            max_scroll_y,
            min_thumb_offset_px: min_thumb_offset,
            max_thumb_offset_px: max_thumb_offset,
            min_thumb_length_px: min_thumb_length,
            max_thumb_length_px: max_thumb_length,
            clamp_at_top_count: clamp_at_top,
            clamp_at_bottom_count: clamp_at_bottom,
        };

        (h.finish(), metrics)
    }

    #[cfg(test)]
    pub(super) fn current_scroll_y(&self) -> f32 {
        self.scroll_y
    }
}

// ---------------------------------------------------------------------------
// Search / commit filter benchmarks (Phase 4)
// ---------------------------------------------------------------------------

/// Metrics emitted as sidecar JSON for commit search/filter benchmarks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommitSearchFilterMetrics {
    pub total_commits: u64,
    pub query_len: u64,
    pub matches_found: u64,
    /// Cost of incremental refinement: filtering the already-matched subset
    /// with one additional character appended to the query.
    pub incremental_matches: u64,
}

/// Benchmark fixture for filtering a large synthetic commit list by author
/// or message substring. Exercises the core scan that any future commit
/// search UI would perform.
pub struct CommitSearchFilterFixture {
    commits: Vec<Commit>,
    /// Pre-lowercased author strings for the author-filter path.
    authors_lower: Vec<String>,
    /// Pre-lowercased summaries for the message-filter path.
    summaries_lower: Vec<String>,
}

impl CommitSearchFilterFixture {
    /// Build a fixture with `count` synthetic commits distributed across
    /// 100 distinct authors and varied commit messages.
    pub fn new(count: usize) -> Self {
        let commits = build_synthetic_commits_for_search(count);
        let authors_lower: Vec<String> = commits.iter().map(|c| c.author.to_lowercase()).collect();
        let summaries_lower: Vec<String> =
            commits.iter().map(|c| c.summary.to_lowercase()).collect();
        Self {
            commits,
            authors_lower,
            summaries_lower,
        }
    }

    /// Filter commits whose author field contains `query` (case-insensitive).
    /// Returns a hash to prevent dead-code elimination.
    pub fn run_filter_by_author(&self, query: &str) -> u64 {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut count = 0u64;
        for author in &self.authors_lower {
            if finder.find(author.as_bytes()).is_some() {
                count += 1;
            }
        }
        count.hash(&mut h);
        h.finish()
    }

    /// Filter commits whose summary field contains `query` (case-insensitive).
    /// Returns a hash to prevent dead-code elimination.
    pub fn run_filter_by_message(&self, query: &str) -> u64 {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut count = 0u64;
        for summary in &self.summaries_lower {
            if finder.find(summary.as_bytes()).is_some() {
                count += 1;
            }
        }
        count.hash(&mut h);
        h.finish()
    }

    /// Filter by author and collect full metrics including incremental
    /// refinement (appending one character to the query).
    pub fn run_filter_by_author_with_metrics(
        &self,
        query: &str,
    ) -> (u64, CommitSearchFilterMetrics) {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut matches = Vec::new();
        for (ix, author) in self.authors_lower.iter().enumerate() {
            if finder.find(author.as_bytes()).is_some() {
                matches.push(ix);
            }
        }
        let matches_found = matches.len() as u64;
        matches_found.hash(&mut h);

        // Incremental refinement: append 'x' and re-filter only the matched subset.
        let refined_query = format!("{query_lower}x");
        let refined_finder = memchr::memmem::Finder::new(refined_query.as_bytes());
        let mut incremental_matches = 0u64;
        for &ix in &matches {
            if refined_finder
                .find(self.authors_lower[ix].as_bytes())
                .is_some()
            {
                incremental_matches += 1;
            }
        }
        incremental_matches.hash(&mut h);

        (
            h.finish(),
            CommitSearchFilterMetrics {
                total_commits: self.commits.len() as u64,
                query_len: query.len() as u64,
                matches_found,
                incremental_matches,
            },
        )
    }

    /// Filter by message and collect full metrics including incremental
    /// refinement (appending one character to the query).
    pub fn run_filter_by_message_with_metrics(
        &self,
        query: &str,
    ) -> (u64, CommitSearchFilterMetrics) {
        let query_lower = query.to_lowercase();
        let finder = memchr::memmem::Finder::new(query_lower.as_bytes());
        let mut h = FxHasher::default();
        let mut matches = Vec::new();
        for (ix, summary) in self.summaries_lower.iter().enumerate() {
            if finder.find(summary.as_bytes()).is_some() {
                matches.push(ix);
            }
        }
        let matches_found = matches.len() as u64;
        matches_found.hash(&mut h);

        // Incremental refinement: append 'x' and re-filter only the matched subset.
        let refined_query = format!("{query_lower}x");
        let refined_finder = memchr::memmem::Finder::new(refined_query.as_bytes());
        let mut incremental_matches = 0u64;
        for &ix in &matches {
            if refined_finder
                .find(self.summaries_lower[ix].as_bytes())
                .is_some()
            {
                incremental_matches += 1;
            }
        }
        incremental_matches.hash(&mut h);

        (
            h.finish(),
            CommitSearchFilterMetrics {
                total_commits: self.commits.len() as u64,
                query_len: query.len() as u64,
                matches_found,
                incremental_matches,
            },
        )
    }

    /// Number of commits in the fixture.
    #[cfg(test)]
    pub fn commit_count(&self) -> usize {
        self.commits.len()
    }

    /// Number of distinct authors in the fixture.
    #[cfg(test)]
    pub fn distinct_authors(&self) -> usize {
        let mut seen = std::collections::HashSet::new();
        for author in &self.authors_lower {
            seen.insert(author.as_str());
        }
        seen.len()
    }
}

/// Metrics emitted as sidecar JSON for in-diff text search benchmarks.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InDiffTextSearchMetrics {
    pub total_lines: u64,
    pub visible_rows_scanned: u64,
    pub query_len: u64,
    pub matches_found: u64,
    /// Prior broad-query matches when measuring a refined follow-up query.
    pub prior_matches: u64,
}

/// Benchmark fixture for scanning a large synthetic unified diff with the same
/// ASCII-case-insensitive substring semantics as the production diff search.
///
/// The broad query (`render_cache`) matches both context rows and modified
/// rows, while the refined query (`render_cache_hot_path`) narrows to a smaller
/// subset of modified rows without changing the overall scan cost.
pub struct InDiffTextSearchFixture {
    diff: Arc<Diff>,
    visible_line_indices: Box<[usize]>,
    total_lines: usize,
    visible_rows: usize,
}

impl InDiffTextSearchFixture {
    pub fn new(lines: usize) -> Self {
        let total_lines = lines.max(1);
        let target = DiffTarget::WorkingTree {
            path: std::path::PathBuf::from("src/lib.rs"),
            area: DiffArea::Unstaged,
        };
        let text = build_synthetic_diff_search_unified_patch(total_lines);
        let diff = Arc::new(Diff::from_unified(target, text.as_str()));
        let visible_line_indices = diff
            .lines
            .iter()
            .enumerate()
            .filter_map(|(ix, line)| {
                (!should_hide_unified_diff_header_for_bench(line.kind, line.text.as_ref()))
                    .then_some(ix)
            })
            .collect::<Vec<_>>();
        let visible_rows = visible_line_indices.len();

        Self {
            diff,
            visible_line_indices: visible_line_indices.into_boxed_slice(),
            total_lines,
            visible_rows,
        }
    }

    pub fn run_search(&self, query: &str) -> u64 {
        self.scan_matches(query).0
    }

    pub fn prepare_matches(&self, query: &str) -> Vec<usize> {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return Vec::new();
        };

        let mut matches = Vec::with_capacity((self.visible_rows / 16).max(1));
        for (visible_ix, &line_ix) in self.visible_line_indices.iter().enumerate() {
            if query.is_match(self.diff.lines[line_ix].text.as_ref()) {
                matches.push(visible_ix);
            }
        }
        matches
    }

    pub fn run_refinement_from_matches(&self, query: &str, prior_matches: &[usize]) -> u64 {
        self.scan_candidate_matches(query, prior_matches).0
    }

    pub fn run_search_with_metrics(&self, query: &str) -> (u64, InDiffTextSearchMetrics) {
        let (hash, matches_found) = self.scan_matches(query);
        (
            hash,
            InDiffTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                visible_rows_scanned: bench_counter_u64(self.visible_rows),
                query_len: query.trim().len() as u64,
                matches_found,
                prior_matches: 0,
            },
        )
    }

    pub fn run_refinement_from_matches_with_metrics(
        &self,
        query: &str,
        prior_matches: &[usize],
    ) -> (u64, InDiffTextSearchMetrics) {
        let (hash, matches_found) = self.scan_candidate_matches(query, prior_matches);
        (
            hash,
            InDiffTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                visible_rows_scanned: bench_counter_u64(self.visible_rows),
                query_len: query.trim().len() as u64,
                matches_found,
                prior_matches: bench_counter_u64(prior_matches.len()),
            },
        )
    }

    pub fn run_refinement_with_metrics(
        &self,
        broad_query: &str,
        refined_query: &str,
    ) -> (u64, InDiffTextSearchMetrics) {
        let prior_matches = self.prepare_matches(broad_query);
        self.run_refinement_from_matches_with_metrics(refined_query, &prior_matches)
    }

    fn scan_matches(&self, query: &str) -> (u64, u64) {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return (0, 0);
        };

        let mut h = FxHasher::default();
        let mut matches_found = 0u64;

        for (visible_ix, &line_ix) in self.visible_line_indices.iter().enumerate() {
            let line = &self.diff.lines[line_ix];
            if query.is_match(line.text.as_ref()) {
                visible_ix.hash(&mut h);
                line.text.len().hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.visible_rows.hash(&mut h);
        (h.finish(), matches_found)
    }

    fn scan_candidate_matches(&self, query: &str, prior_matches: &[usize]) -> (u64, u64) {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return (0, 0);
        };

        let mut h = FxHasher::default();
        let mut matches_found = 0u64;

        for &visible_ix in prior_matches {
            let Some(&line_ix) = self.visible_line_indices.get(visible_ix) else {
                continue;
            };
            let line = self.diff.lines[line_ix].text.as_ref();
            if query.is_match(line) {
                visible_ix.hash(&mut h);
                line.len().hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.visible_rows.hash(&mut h);
        (h.finish(), matches_found)
    }

    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    #[cfg(test)]
    pub fn visible_rows(&self) -> usize {
        self.visible_rows
    }
}

/// Metrics emitted as sidecar JSON for file-preview `Ctrl+F` search
/// benchmarks. This follows the production path that scans reconstructed
/// preview source text line by line.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FilePreviewTextSearchMetrics {
    pub total_lines: u64,
    pub source_bytes: u64,
    pub query_len: u64,
    pub matches_found: u64,
    pub prior_matches: u64,
}

/// Benchmark fixture for the file-preview `Ctrl+F` search path in
/// `diff_search_recompute_matches_for_current_view()`.
pub struct FilePreviewTextSearchFixture {
    source_text: SharedString,
    line_starts: Arc<[usize]>,
    total_lines: usize,
}

impl FilePreviewTextSearchFixture {
    pub fn new(lines: usize) -> Self {
        let total_lines = lines.max(1);
        let preview_lines = build_synthetic_file_preview_search_lines(total_lines);
        let source_len = preview_lines
            .iter()
            .map(String::len)
            .sum::<usize>()
            .saturating_add(preview_lines.len().saturating_sub(1));
        let (source_text, line_starts) =
            crate::view::panes::main::preview_source_text_and_line_starts_from_lines(
                &preview_lines,
                source_len,
            );

        Self {
            source_text,
            line_starts,
            total_lines,
        }
    }

    pub fn run_search(&self, query: &str) -> u64 {
        self.scan_matches(query).0
    }

    pub fn run_search_with_metrics(&self, query: &str) -> (u64, FilePreviewTextSearchMetrics) {
        let (hash, matches_found) = self.scan_matches(query);
        (
            hash,
            FilePreviewTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                source_bytes: bench_counter_u64(self.source_text.len()),
                query_len: query.trim().len() as u64,
                matches_found,
                prior_matches: 0,
            },
        )
    }

    pub fn run_refinement_with_metrics(
        &self,
        broad_query: &str,
        refined_query: &str,
    ) -> (u64, FilePreviewTextSearchMetrics) {
        let (_, prior_matches) = self.scan_matches(broad_query);
        let (hash, matches_found) = self.scan_matches(refined_query);
        (
            hash,
            FilePreviewTextSearchMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                source_bytes: bench_counter_u64(self.source_text.len()),
                query_len: refined_query.trim().len() as u64,
                matches_found,
                prior_matches,
            },
        )
    }

    fn scan_matches(&self, query: &str) -> (u64, u64) {
        let Some(query) = AsciiCaseInsensitiveNeedle::new(query.trim()) else {
            return (0, 0);
        };

        let mut h = FxHasher::default();
        let mut matches_found = 0u64;

        for line_ix in 0..self.total_lines {
            let line = super::diff_text::resolved_output_line_text(
                self.source_text.as_ref(),
                self.line_starts.as_ref(),
                line_ix,
            );
            if query.is_match(line) {
                line_ix.hash(&mut h);
                line.len().hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.total_lines.hash(&mut h);
        self.source_text.len().hash(&mut h);
        (h.finish(), matches_found)
    }

    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    #[cfg(test)]
    pub fn source_bytes(&self) -> usize {
        self.source_text.len()
    }
}

/// Metrics emitted as sidecar JSON for the split file-diff `Ctrl+F` search path.
///
/// This models the user-visible sequence in the large file-diff view:
/// 1. open the search input with `Ctrl+F`
/// 2. type a query one character at a time
/// 3. reuse prior matches on refinements instead of rescanning every row
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileDiffCtrlFOpenTypeMetrics {
    pub total_lines: u64,
    pub total_rows: u64,
    pub visible_window_rows: u64,
    pub search_opened: u64,
    pub typed_chars: u64,
    pub query_steps: u64,
    pub final_query_len: u64,
    pub rows_scanned: u64,
    pub full_rescans: u64,
    pub refinement_steps: u64,
    pub final_matches: u64,
}

/// Benchmark fixture for the large split file-diff `Ctrl+F` search path.
///
/// It mirrors `activate_diff_search()` plus repeated
/// `diff_search_recompute_matches_for_query_change()` updates on the split
/// file-diff view. The first visible window is prewarmed to match a diff that
/// is already open before the user hits `Ctrl+F`.
pub struct FileDiffCtrlFOpenTypeFixture {
    split: Arc<PagedFileDiffRows>,
    total_lines: usize,
    visible_window_rows: usize,
}

impl FileDiffCtrlFOpenTypeFixture {
    pub fn new(lines: usize, visible_window_rows: usize) -> Self {
        let total_lines = lines.max(1);
        let visible_window_rows = visible_window_rows.max(1);
        let (old_text, new_text) = build_synthetic_file_diff_search_texts(total_lines);
        #[cfg(feature = "benchmarks")]
        let (split, _inline) =
            crate::view::panes::main::diff_cache::bench_build_file_diff_providers(
                &old_text, &new_text, 256,
            );
        #[cfg(not(feature = "benchmarks"))]
        let (split, _inline) =
            unreachable!("FileDiffCtrlFOpenTypeFixture requires benchmarks feature");

        let warm_rows = visible_window_rows.min(split.len_hint());
        let _ = split.slice(0, warm_rows).take(warm_rows).count();

        Self {
            split,
            total_lines,
            visible_window_rows: warm_rows,
        }
    }

    pub fn run_open_and_type(&self, final_query: &str) -> u64 {
        self.run_open_and_type_with_metrics(final_query).0
    }

    pub fn run_open_and_type_with_metrics(
        &self,
        final_query: &str,
    ) -> (u64, FileDiffCtrlFOpenTypeMetrics) {
        let final_query = final_query.trim();
        let total_rows = self.split.len_hint();
        let typed_chars = final_query.chars().count();
        let mut current_query = String::with_capacity(final_query.len());
        let mut previous_query = String::new();
        let mut matches: Vec<usize> = Vec::new();
        let mut rows_scanned = 0u64;
        let mut full_rescans = 0u64;
        let mut refinement_steps = 0u64;

        let mut h = FxHasher::default();
        true.hash(&mut h);
        total_rows.hash(&mut h);

        for ch in final_query.chars() {
            current_query.push(ch);
            let Some(query) = AsciiCaseInsensitiveNeedle::new(current_query.as_str()) else {
                continue;
            };

            match diff_search_query_reuse(previous_query.as_str(), current_query.as_str()) {
                DiffSearchQueryReuse::SameSemantics => {}
                DiffSearchQueryReuse::Refinement => {
                    refinement_steps = refinement_steps.saturating_add(1);
                    let mut next_matches = Vec::with_capacity(matches.len());
                    for &row_ix in &matches {
                        rows_scanned = rows_scanned.saturating_add(1);
                        if self.row_matches_query(row_ix, query) {
                            next_matches.push(row_ix);
                        }
                    }
                    matches = next_matches;
                }
                DiffSearchQueryReuse::None => {
                    full_rescans = full_rescans.saturating_add(1);
                    matches.clear();
                    matches.reserve((total_rows / 16).max(1));
                    for row_ix in 0..total_rows {
                        rows_scanned = rows_scanned.saturating_add(1);
                        if self.row_matches_query(row_ix, query) {
                            matches.push(row_ix);
                        }
                    }
                }
            }

            current_query.len().hash(&mut h);
            matches.len().hash(&mut h);
            matches.first().hash(&mut h);
            matches.last().hash(&mut h);

            previous_query.clear();
            previous_query.push_str(&current_query);
        }

        (
            h.finish(),
            FileDiffCtrlFOpenTypeMetrics {
                total_lines: bench_counter_u64(self.total_lines),
                total_rows: bench_counter_u64(total_rows),
                visible_window_rows: bench_counter_u64(self.visible_window_rows),
                search_opened: 1,
                typed_chars: bench_counter_u64(typed_chars),
                query_steps: bench_counter_u64(typed_chars),
                final_query_len: final_query.len() as u64,
                rows_scanned,
                full_rescans,
                refinement_steps,
                final_matches: bench_counter_u64(matches.len()),
            },
        )
    }

    fn row_matches_query(&self, row_ix: usize, query: AsciiCaseInsensitiveNeedle<'_>) -> bool {
        let Some(row) = self.split.row(row_ix) else {
            return false;
        };
        row.old.as_deref().is_some_and(|text| query.is_match(text))
            || row.new.as_deref().is_some_and(|text| query.is_match(text))
    }

    #[cfg(test)]
    pub fn total_lines(&self) -> usize {
        self.total_lines
    }

    #[cfg(test)]
    pub fn total_rows(&self) -> usize {
        self.split.len_hint()
    }

    #[cfg(test)]
    pub fn visible_window_rows(&self) -> usize {
        self.visible_window_rows
    }
}

// ---------------------------------------------------------------------------
// file_fuzzy_find — file-picker fuzzy search over large path corpora
// ---------------------------------------------------------------------------

/// Sidecar metrics emitted by `FileFuzzyFindFixture`.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FileFuzzyFindMetrics {
    /// Total number of file paths in the corpus.
    pub total_files: u64,
    /// Length of the query string.
    pub query_len: u64,
    /// Number of paths that matched the query.
    pub matches_found: u64,
    /// Number of matches from a prior (shorter) query — used for incremental keystroke.
    pub prior_matches: u64,
    /// Total number of candidate paths scanned across the measured search pass(es).
    pub files_scanned: u64,
}

struct FileFuzzyFindPath {
    len: usize,
    lowercase_bytes: Box<[u8]>,
    /// Bitmap of which lowercase ASCII letters appear in the path.
    /// Bit `i` is set if byte `b'a' + i` is present (0 ≤ i < 26).
    char_bitmap: u32,
}

impl FileFuzzyFindPath {
    fn new(path: String) -> Self {
        let len = path.len();
        let mut lowercase_bytes = path.into_bytes();
        lowercase_bytes.make_ascii_lowercase();
        let mut char_bitmap = 0u32;
        for &b in lowercase_bytes.iter() {
            if b >= b'a' && b <= b'z' {
                char_bitmap |= 1 << (b - b'a');
            }
        }
        Self {
            len,
            lowercase_bytes: lowercase_bytes.into_boxed_slice(),
            char_bitmap,
        }
    }
}

#[derive(Clone, Copy)]
struct FileFuzzyFindMatchCandidate {
    index: usize,
    next_start: usize,
}

struct AsciiCaseInsensitiveSubsequenceNeedle {
    lowercase_bytes: Box<[u8]>,
    /// Bitmap of required lowercase ASCII letters.
    /// Bit `i` is set if byte `b'a' + i` appears in the needle (0 ≤ i < 26).
    required_bitmap: u32,
}

impl AsciiCaseInsensitiveSubsequenceNeedle {
    #[inline]
    fn new(needle: &str) -> Option<Self> {
        let bytes = needle.as_bytes();
        let Some(_) = bytes.first() else {
            return None;
        };

        let mut lowercase_bytes = Vec::with_capacity(bytes.len());
        let mut required_bitmap = 0u32;
        for &byte in bytes {
            let lower = byte.to_ascii_lowercase();
            lowercase_bytes.push(lower);
            if lower >= b'a' && lower <= b'z' {
                required_bitmap |= 1 << (lower - b'a');
            }
        }
        Some(Self {
            lowercase_bytes: lowercase_bytes.into_boxed_slice(),
            required_bitmap,
        })
    }

    #[inline]
    fn is_match(&self, haystack: &[u8]) -> bool {
        self.match_end(haystack).is_some()
    }

    #[inline]
    fn match_end(&self, haystack: &[u8]) -> Option<usize> {
        lowercase_subsequence_match_end(haystack, &self.lowercase_bytes)
    }

    #[inline]
    fn as_bytes(&self) -> &[u8] {
        &self.lowercase_bytes
    }

    #[inline]
    fn is_strict_extension_of(&self, prefix: &[u8]) -> bool {
        self.lowercase_bytes.len() > prefix.len() && self.lowercase_bytes.starts_with(prefix)
    }
}

#[inline]
fn lowercase_subsequence_match_end(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.len() > haystack.len() {
        return None;
    }

    let mut offset = 0usize;
    for &needle_byte in needle {
        let remaining = &haystack[offset..];
        match memchr::memchr(needle_byte, remaining) {
            Some(pos) => offset += pos + 1,
            None => return None,
        }
    }

    Some(offset)
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileFuzzyFindRunResult {
    hash: u64,
    matches_found: u64,
    files_scanned: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct FileFuzzyFindIncrementalRunResult {
    hash: u64,
    matches_found: u64,
    prior_matches: u64,
    files_scanned: u64,
}

/// Benchmark fixture for fuzzy-finding file paths in a large synthetic corpus.
///
/// Simulates the production file-picker workflow: the user types a query and
/// the UI filters a flat list of file paths using subsequence matching (each
/// character of the query must appear in order in the candidate path,
/// case-insensitively). The corpus is built once with deterministic,
/// realistic-looking paths covering varied directory depths and extensions.
pub struct FileFuzzyFindFixture {
    paths: Vec<FileFuzzyFindPath>,
    match_candidates_scratch: RefCell<Vec<FileFuzzyFindMatchCandidate>>,
    total_files: usize,
}

impl FileFuzzyFindFixture {
    pub fn new(file_count: usize) -> Self {
        let total_files = file_count.max(1);
        let paths = build_synthetic_file_path_corpus(total_files)
            .into_iter()
            .map(FileFuzzyFindPath::new)
            .collect();
        Self {
            paths,
            match_candidates_scratch: RefCell::new(Vec::with_capacity((total_files / 3).max(1))),
            total_files,
        }
    }

    pub fn run_find(&self, query: &str) -> u64 {
        self.scan_matches(query).hash
    }

    pub fn run_incremental(&self, short_query: &str, long_query: &str) -> u64 {
        self.scan_incremental_matches(short_query, long_query).hash
    }

    pub fn run_find_with_metrics(&self, query: &str) -> (u64, FileFuzzyFindMetrics) {
        let query = query.trim();
        let run = self.scan_matches(query);
        (
            run.hash,
            FileFuzzyFindMetrics {
                total_files: bench_counter_u64(self.total_files),
                query_len: bench_counter_u64(query.len()),
                matches_found: run.matches_found,
                prior_matches: 0,
                files_scanned: run.files_scanned,
            },
        )
    }

    pub fn run_incremental_with_metrics(
        &self,
        short_query: &str,
        long_query: &str,
    ) -> (u64, FileFuzzyFindMetrics) {
        let long_query = long_query.trim();
        let run = self.scan_incremental_matches(short_query, long_query);
        (
            run.hash,
            FileFuzzyFindMetrics {
                total_files: bench_counter_u64(self.total_files),
                query_len: bench_counter_u64(long_query.len()),
                matches_found: run.matches_found,
                prior_matches: run.prior_matches,
                files_scanned: run.files_scanned,
            },
        )
    }

    fn scan_matches(&self, query: &str) -> FileFuzzyFindRunResult {
        let Some(query) = AsciiCaseInsensitiveSubsequenceNeedle::new(query.trim()) else {
            return FileFuzzyFindRunResult {
                hash: 0,
                matches_found: bench_counter_u64(self.paths.len()),
                files_scanned: 0,
            };
        };

        self.scan_all_matches(&query)
    }

    fn scan_incremental_matches(
        &self,
        short_query: &str,
        long_query: &str,
    ) -> FileFuzzyFindIncrementalRunResult {
        let short_query = short_query.trim();
        let long_query = long_query.trim();
        let Some(short_needle) = AsciiCaseInsensitiveSubsequenceNeedle::new(short_query) else {
            let run = self.scan_matches(long_query);
            return FileFuzzyFindIncrementalRunResult {
                hash: run.hash,
                matches_found: run.matches_found,
                prior_matches: bench_counter_u64(self.paths.len()),
                files_scanned: run.files_scanned,
            };
        };

        let mut prior_match_candidates = self.match_candidates_scratch.borrow_mut();
        let prior_matches =
            self.collect_match_candidates(&short_needle, &mut prior_match_candidates);
        let refined_run = match AsciiCaseInsensitiveSubsequenceNeedle::new(long_query) {
            Some(long_needle) if long_needle.is_strict_extension_of(short_needle.as_bytes()) => {
                self.scan_extended_candidate_matches(
                    &long_needle,
                    short_needle.as_bytes().len(),
                    prior_match_candidates.as_slice(),
                )
            }
            Some(long_needle) => self.scan_all_matches(&long_needle),
            None => FileFuzzyFindRunResult {
                hash: 0,
                matches_found: bench_counter_u64(self.paths.len()),
                files_scanned: 0,
            },
        };

        FileFuzzyFindIncrementalRunResult {
            hash: refined_run.hash,
            matches_found: refined_run.matches_found,
            prior_matches,
            files_scanned: bench_counter_u64(self.total_files)
                .saturating_add(refined_run.files_scanned),
        }
    }

    fn scan_all_matches(
        &self,
        query: &AsciiCaseInsensitiveSubsequenceNeedle,
    ) -> FileFuzzyFindRunResult {
        let mut h = FxHasher::default();
        let mut matches_found = 0u64;
        let req = query.required_bitmap;

        for (ix, path) in self.paths.iter().enumerate() {
            if path.char_bitmap & req != req {
                continue;
            }
            if query.is_match(path.lowercase_bytes.as_ref()) {
                ix.hash(&mut h);
                path.len.hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.total_files.hash(&mut h);
        FileFuzzyFindRunResult {
            hash: h.finish(),
            matches_found,
            files_scanned: bench_counter_u64(self.total_files),
        }
    }

    fn scan_extended_candidate_matches(
        &self,
        query: &AsciiCaseInsensitiveSubsequenceNeedle,
        prefix_len: usize,
        candidate_matches: &[FileFuzzyFindMatchCandidate],
    ) -> FileFuzzyFindRunResult {
        let mut h = FxHasher::default();
        let mut matches_found = 0u64;
        let suffix = &query.as_bytes()[prefix_len..];

        for candidate in candidate_matches {
            let path = &self.paths[candidate.index];
            let matches = suffix.is_empty()
                || lowercase_subsequence_match_end(
                    &path.lowercase_bytes[candidate.next_start..],
                    suffix,
                )
                .is_some();
            if matches {
                candidate.index.hash(&mut h);
                path.len.hash(&mut h);
                matches_found = matches_found.saturating_add(1);
            }
        }

        matches_found.hash(&mut h);
        self.total_files.hash(&mut h);
        FileFuzzyFindRunResult {
            hash: h.finish(),
            matches_found,
            files_scanned: bench_counter_u64(candidate_matches.len()),
        }
    }

    fn collect_match_candidates(
        &self,
        query: &AsciiCaseInsensitiveSubsequenceNeedle,
        out: &mut Vec<FileFuzzyFindMatchCandidate>,
    ) -> u64 {
        out.clear();
        let req = query.required_bitmap;
        for (ix, path) in self.paths.iter().enumerate() {
            if path.char_bitmap & req != req {
                continue;
            }
            if let Some(next_start) = query.match_end(path.lowercase_bytes.as_ref()) {
                out.push(FileFuzzyFindMatchCandidate {
                    index: ix,
                    next_start,
                });
            }
        }
        bench_counter_u64(out.len())
    }

    #[cfg(test)]
    pub fn total_files(&self) -> usize {
        self.total_files
    }
}

/// Build a deterministic corpus of `count` synthetic file paths with realistic
/// directory depths (1–6 segments), varied extensions, and reproducible names.
fn build_synthetic_file_path_corpus(count: usize) -> Vec<String> {
    let dirs_l0 = [
        "src", "crates", "lib", "tests", "benches", "docs", "tools", "scripts", "config", "assets",
    ];
    let dirs_l1 = [
        "core", "ui", "model", "view", "utils", "cache", "render", "state", "events", "layout",
    ];
    let dirs_l2 = [
        "rows",
        "panels",
        "panes",
        "widgets",
        "handlers",
        "traits",
        "builders",
        "parsers",
        "formatters",
        "providers",
    ];
    let stems = [
        "main",
        "app",
        "config",
        "history",
        "diff_cache",
        "branch",
        "commit",
        "status",
        "merge",
        "conflict",
        "search",
        "render",
        "layout",
        "sidebar",
        "toolbar",
        "popover",
        "dialog",
        "input",
        "scroll",
        "resize",
    ];
    let exts = [
        "rs", "ts", "tsx", "js", "json", "toml", "yaml", "md", "css", "html",
    ];

    let mut paths = Vec::with_capacity(count);
    for ix in 0..count {
        let d0 = dirs_l0[ix % dirs_l0.len()];
        let d1 = dirs_l1[(ix / dirs_l0.len()) % dirs_l1.len()];
        let depth = (ix % 6) + 1;
        let stem = stems[(ix / 7) % stems.len()];
        let ext = exts[(ix / 3) % exts.len()];
        let suffix = ix;

        let path = match depth {
            1 => format!("{d0}/{stem}_{suffix}.{ext}"),
            2 => format!("{d0}/{d1}/{stem}_{suffix}.{ext}"),
            3 => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                format!("{d0}/{d1}/{d2}/{stem}_{suffix}.{ext}")
            }
            4 => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                let sub = format!("sub_{}", ix % 50);
                format!("{d0}/{d1}/{d2}/{sub}/{stem}_{suffix}.{ext}")
            }
            5 => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                let sub = format!("sub_{}", ix % 50);
                let deep = format!("deep_{}", ix % 20);
                format!("{d0}/{d1}/{d2}/{sub}/{deep}/{stem}_{suffix}.{ext}")
            }
            _ => {
                let d2 = dirs_l2[(ix / 100) % dirs_l2.len()];
                let sub = format!("sub_{}", ix % 50);
                let deep = format!("deep_{}", ix % 20);
                let leaf = format!("leaf_{}", ix % 10);
                format!("{d0}/{d1}/{d2}/{sub}/{deep}/{leaf}/{stem}_{suffix}.{ext}")
            }
        };
        paths.push(path);
    }
    paths
}

/// Build synthetic commits with richer author and message diversity for
/// search benchmarks. Uses 100 distinct authors and varied message prefixes
/// so that substring queries have realistic selectivity.
fn build_synthetic_commits_for_search(count: usize) -> Vec<Commit> {
    let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    let prefixes = [
        "fix", "feat", "refactor", "chore", "docs", "test", "perf", "ci", "style", "build",
    ];
    let areas = [
        "history view",
        "diff cache",
        "branch sidebar",
        "merge tool",
        "status panel",
        "commit details",
        "repo tabs",
        "settings",
        "theme engine",
        "search",
    ];
    let mut commits = Vec::with_capacity(count);
    for ix in 0..count {
        let id = CommitId(format!("{:040x}", ix).into());
        let mut parent_ids = Vec::new();
        if ix > 0 {
            parent_ids.push(CommitId(format!("{:040x}", ix - 1).into()));
        }

        // 100 distinct authors: "Alice Anderson", "Bob Baker", ..., cycling
        // through 10 first names × 10 last names.
        let first_names = [
            "Alice", "Bob", "Carol", "Dave", "Eve", "Frank", "Grace", "Hank", "Ivy", "Jack",
        ];
        let last_names = [
            "Anderson", "Baker", "Chen", "Davis", "Evans", "Foster", "Garcia", "Hill", "Ito",
            "Jones",
        ];
        let author = format!("{} {}", first_names[ix % 10], last_names[(ix / 10) % 10]);

        let prefix = prefixes[ix % prefixes.len()];
        let area = areas[(ix / prefixes.len()) % areas.len()];
        let summary: Arc<str> = format!("{prefix}: update {area} for commit {ix}").into();

        commits.push(Commit {
            id,
            parent_ids,
            summary,
            author: author.into(),
            time: base + Duration::from_secs(ix as u64),
        });
    }
    commits
}

fn build_synthetic_diff_search_unified_patch(line_count: usize) -> String {
    let line_count = line_count.max(1);
    let mut out = String::new();
    out.push_str("diff --git a/src/lib.rs b/src/lib.rs\n");
    out.push_str("index 3333333..4444444 100644\n");
    out.push_str("--- a/src/lib.rs\n");
    out.push_str("+++ b/src/lib.rs\n");
    out.push_str(&format!(
        "@@ -1,{line_count} +1,{line_count} @@ fn synthetic_diff_search_fixture() {{\n"
    ));

    for ix in 0..line_count {
        if ix % 64 == 0 {
            out.push_str(&format!(
                "-let render_cache_old_{ix} = old_cache_lookup({ix});\n"
            ));
            out.push_str(&format!(
                "+let render_cache_hot_path_{ix} = hot_cache_lookup({ix});\n"
            ));
        } else if ix % 16 == 0 {
            out.push_str(&format!(
                " let render_cache_probe_{ix} = inspect_cache({ix});\n"
            ));
        } else if ix % 7 == 0 {
            out.push_str(&format!("-let old_{ix} = old_call({ix});\n"));
            out.push_str(&format!("+let new_{ix} = new_call({ix});\n"));
        } else {
            out.push_str(&format!(" let stable_line_{ix} = keep({ix});\n"));
        }
    }

    out
}

fn build_synthetic_file_preview_search_lines(line_count: usize) -> Vec<String> {
    let line_count = line_count.max(1);
    let mut lines = Vec::with_capacity(line_count);
    for ix in 0..line_count {
        let line = if ix % 64 == 0 {
            format!("let render_cache_hot_path_{ix} = hot_cache_lookup({ix}); // preview search")
        } else if ix % 16 == 0 {
            format!("let render_cache_probe_{ix} = inspect_cache({ix});")
        } else if ix % 7 == 0 {
            format!("let stable_line_{ix} = keep({ix}); // wrapped preview line")
        } else {
            format!("let stable_line_{ix} = keep({ix});")
        };
        lines.push(line);
    }
    lines
}

fn build_synthetic_file_diff_search_texts(line_count: usize) -> (String, String) {
    let line_count = line_count.max(1);
    let mut old_text = String::with_capacity(line_count * 64);
    let mut new_text = String::with_capacity(line_count * 64);

    for ix in 0..line_count {
        if ix % 64 == 0 {
            old_text.push_str(&format!(
                "let render_cache_old_{ix} = old_cache_lookup({ix});\n"
            ));
            new_text.push_str(&format!(
                "let render_cache_hot_path_{ix} = hot_cache_lookup({ix});\n"
            ));
        } else if ix % 16 == 0 {
            let shared = format!("let render_cache_probe_{ix} = inspect_cache({ix});\n");
            old_text.push_str(&shared);
            new_text.push_str(&shared);
        } else if ix % 7 == 0 {
            old_text.push_str(&format!("let old_{ix} = old_call({ix});\n"));
            new_text.push_str(&format!("let new_{ix} = new_call({ix});\n"));
        } else {
            let shared = format!("let stable_line_{ix} = keep({ix});\n");
            old_text.push_str(&shared);
            new_text.push_str(&shared);
        }
    }

    (old_text, new_text)
}

// ---------------------------------------------------------------------------
// FsEventFixture — filesystem event to status update benchmark harness
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
pub enum FsEventScenario {
    /// Single file save → git status → status diff.
    SingleFileSave { tracked_files: usize },
    /// Simulate `git checkout` changing many files at once → status.
    GitCheckoutBatch {
        tracked_files: usize,
        checkout_files: usize,
    },
    /// Rapidly dirty N files → single coalesced status call (debounce model).
    RapidSavesDebounceCoalesce {
        tracked_files: usize,
        save_count: usize,
    },
    /// Dirty N files then revert → status should find 0 dirty (false positive).
    FalsePositiveUnderChurn {
        tracked_files: usize,
        churn_files: usize,
    },
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct FsEventFixture {
    _repo_root: TempDir,
    repo: Arc<dyn GitRepository>,
    repo_path: std::path::PathBuf,
    scenario: FsEventScenario,
    tracked_files: usize,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FsEventMetrics {
    pub tracked_files: u64,
    pub mutation_files: u64,
    pub dirty_files_detected: u64,
    pub status_entries_total: u64,
    pub false_positives: u64,
    pub coalesced_saves: u64,
    pub status_calls: u64,
    pub status_ms: f64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl FsEventFixture {
    pub fn single_file_save(tracked_files: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event single_file_save benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::SingleFileSave { tracked_files },
            tracked_files,
        }
    }

    pub fn git_checkout_batch(tracked_files: usize, checkout_files: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let checkout_files = checkout_files.min(tracked_files).max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event git_checkout_batch benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::GitCheckoutBatch {
                tracked_files,
                checkout_files,
            },
            tracked_files,
        }
    }

    pub fn rapid_saves_debounce(tracked_files: usize, save_count: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let save_count = save_count.min(tracked_files).max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event rapid_saves_debounce benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::RapidSavesDebounceCoalesce {
                tracked_files,
                save_count,
            },
            tracked_files,
        }
    }

    pub fn false_positive_under_churn(tracked_files: usize, churn_files: usize) -> Self {
        let tracked_files = tracked_files.max(10);
        let churn_files = churn_files.min(tracked_files).max(1);
        let repo_root = build_git_ops_status_repo(tracked_files, 0);
        let repo_path = repo_root.path().to_path_buf();
        let backend = GixBackend;
        let repo = backend
            .open(repo_root.path())
            .expect("open fs_event false_positive_under_churn benchmark repo");

        Self {
            _repo_root: repo_root,
            repo,
            repo_path,
            scenario: FsEventScenario::FalsePositiveUnderChurn {
                tracked_files,
                churn_files,
            },
            tracked_files,
        }
    }

    pub fn run(&self) -> u64 {
        self.execute().0
    }

    pub fn run_with_metrics(&self) -> (u64, FsEventMetrics) {
        self.execute()
    }

    fn execute(&self) -> (u64, FsEventMetrics) {
        let mut metrics = FsEventMetrics::default();
        metrics.tracked_files = u64::try_from(self.tracked_files).unwrap_or(u64::MAX);

        match &self.scenario {
            FsEventScenario::SingleFileSave { .. } => {
                // 1. Mutate one file (simulates save).
                let target = git_ops_status_relative_path(0);
                let full_path = self.repo_path.join(&target);
                let original = fs::read(&full_path).expect("read original file");
                fs::write(&full_path, b"fs-event-mutation\n").expect("write fs_event dirty file");
                metrics.mutation_files = 1;

                // 2. Run git status.
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event single_file_save status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;

                let hash = hash_repo_status(&status);

                // 3. Restore.
                fs::write(&full_path, &original).expect("restore original file");

                (hash, metrics)
            }
            FsEventScenario::GitCheckoutBatch { checkout_files, .. } => {
                let checkout_files = *checkout_files;

                // 1. Mutate checkout_files files.
                let mut originals = Vec::with_capacity(checkout_files);
                for index in 0..checkout_files {
                    let target = git_ops_status_relative_path(index);
                    let full_path = self.repo_path.join(&target);
                    originals.push((
                        full_path.clone(),
                        fs::read(&full_path).expect("read original"),
                    ));
                    fs::write(&full_path, format!("checkout-mutation-{index:05}\n"))
                        .expect("write fs_event checkout file");
                }
                metrics.mutation_files = u64::try_from(checkout_files).unwrap_or(u64::MAX);

                // 2. Run git status.
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event git_checkout_batch status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;

                let hash = hash_repo_status(&status);

                // 3. Restore all files.
                for (path, original) in &originals {
                    fs::write(path, original).expect("restore checkout file");
                }

                (hash, metrics)
            }
            FsEventScenario::RapidSavesDebounceCoalesce { save_count, .. } => {
                let save_count = *save_count;

                // 1. Rapidly dirty save_count files (simulating rapid saves before debounce fires).
                let mut originals = Vec::with_capacity(save_count);
                for index in 0..save_count {
                    let target = git_ops_status_relative_path(index);
                    let full_path = self.repo_path.join(&target);
                    originals.push((
                        full_path.clone(),
                        fs::read(&full_path).expect("read original"),
                    ));
                    fs::write(&full_path, format!("rapid-save-{index:05}\n"))
                        .expect("write fs_event rapid save file");
                }
                metrics.mutation_files = u64::try_from(save_count).unwrap_or(u64::MAX);
                metrics.coalesced_saves = metrics.mutation_files;

                // 2. Single coalesced status call (debounce model).
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event rapid_saves_debounce status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;

                let hash = hash_repo_status(&status);

                // 3. Restore.
                for (path, original) in &originals {
                    fs::write(path, original).expect("restore rapid save file");
                }

                (hash, metrics)
            }
            FsEventScenario::FalsePositiveUnderChurn { churn_files, .. } => {
                let churn_files = *churn_files;

                // 1. Dirty churn_files files.
                let mut originals = Vec::with_capacity(churn_files);
                for index in 0..churn_files {
                    let target = git_ops_status_relative_path(index);
                    let full_path = self.repo_path.join(&target);
                    let original = fs::read(&full_path).expect("read original");
                    originals.push((full_path.clone(), original));
                    fs::write(&full_path, format!("churn-{index:05}\n"))
                        .expect("write fs_event churn file");
                }

                // 2. Revert all files to original content (simulating churn that settles).
                for (path, original) in &originals {
                    fs::write(path, original).expect("revert churn file");
                }
                metrics.mutation_files = u64::try_from(churn_files).unwrap_or(u64::MAX);

                // 3. Status should find 0 dirty files — the FS events were false positives.
                let start = std::time::Instant::now();
                let status = self
                    .repo
                    .status()
                    .expect("fs_event false_positive_under_churn status");
                metrics.status_ms = start.elapsed().as_secs_f64() * 1_000.0;
                metrics.status_calls = 1;

                let dirty = status.staged.len().saturating_add(status.unstaged.len());
                metrics.dirty_files_detected = u64::try_from(dirty).unwrap_or(u64::MAX);
                metrics.status_entries_total = metrics.dirty_files_detected;
                // Every churn file triggered an FS event but resulted in 0 dirty files.
                metrics.false_positives = if dirty == 0 {
                    metrics.mutation_files
                } else {
                    0
                };

                let hash = hash_repo_status(&status);
                (hash, metrics)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// IdleResourceFixture — long-running idle CPU/RSS sampling harness
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdleResourceScenario {
    CpuUsageSingleRepo60s,
    CpuUsageTenRepos60s,
    MemoryGrowthSingleRepo10Min,
    MemoryGrowthTenRepos10Min,
    BackgroundRefreshCostPerCycle,
    WakeFromSleepResume,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IdleResourceConfig {
    pub repo_count: usize,
    pub tracked_files_per_repo: usize,
    pub sample_window: Duration,
    pub sample_interval: Duration,
    pub refresh_cycles: usize,
    pub wake_gap: Duration,
}

#[cfg(any(test, feature = "benchmarks"))]
impl IdleResourceConfig {
    pub fn cpu_usage_single_repo() -> Self {
        Self {
            repo_count: 1,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(60),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn cpu_usage_ten_repos() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(60),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn memory_growth_single_repo() -> Self {
        Self {
            repo_count: 1,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(600),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn memory_growth_ten_repos() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::from_secs(600),
            sample_interval: Duration::from_secs(1),
            refresh_cycles: 0,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn background_refresh_cost_per_cycle() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::ZERO,
            sample_interval: Duration::from_millis(250),
            refresh_cycles: 10,
            wake_gap: Duration::ZERO,
        }
    }

    pub fn wake_from_sleep_resume() -> Self {
        Self {
            repo_count: 10,
            tracked_files_per_repo: 1_000,
            sample_window: Duration::ZERO,
            sample_interval: Duration::ZERO,
            refresh_cycles: 1,
            wake_gap: Duration::from_secs(1),
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
pub struct IdleResourceFixture {
    _repo_roots: Vec<TempDir>,
    repos: Vec<Arc<dyn GitRepository>>,
    scenario: IdleResourceScenario,
    config: IdleResourceConfig,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct IdleResourceMetrics {
    pub open_repos: u64,
    pub tracked_files_per_repo: u64,
    pub sample_duration_ms: f64,
    pub sample_count: u64,
    pub avg_cpu_pct: f64,
    pub peak_cpu_pct: f64,
    pub rss_start_kib: u64,
    pub rss_end_kib: u64,
    pub rss_delta_kib: i64,
    pub refresh_cycles: u64,
    pub repos_refreshed: u64,
    pub status_calls: u64,
    pub status_ms: f64,
    pub avg_refresh_cycle_ms: f64,
    pub max_refresh_cycle_ms: f64,
    pub wake_resume_ms: f64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default)]
struct IdleSampleSummary {
    sample_duration_ms: f64,
    sample_count: u64,
    avg_cpu_pct: f64,
    peak_cpu_pct: f64,
    rss_start_kib: u64,
    rss_end_kib: u64,
    rss_delta_kib: i64,
}

#[cfg(any(test, feature = "benchmarks"))]
struct IdleSampler {
    started_at: Instant,
    last_at: Instant,
    start_cpu_runtime_ns: Option<u64>,
    last_cpu_runtime_ns: Option<u64>,
    peak_cpu_pct: f64,
    sample_count: u64,
    rss_start_kib: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
impl IdleSampler {
    fn start() -> Self {
        let now = Instant::now();
        let cpu_runtime_ns = current_cpu_runtime_ns();
        Self {
            started_at: now,
            last_at: now,
            start_cpu_runtime_ns: cpu_runtime_ns,
            last_cpu_runtime_ns: cpu_runtime_ns,
            peak_cpu_pct: 0.0,
            sample_count: 0,
            rss_start_kib: current_rss_kib().unwrap_or(0),
        }
    }

    fn sample(&mut self) {
        let now = Instant::now();
        if let (Some(previous_cpu_ns), Some(current_cpu_ns)) =
            (self.last_cpu_runtime_ns, current_cpu_runtime_ns())
        {
            let elapsed_wall_ns = now.duration_since(self.last_at).as_nanos() as f64;
            if elapsed_wall_ns > 0.0 {
                let cpu_pct =
                    current_cpu_ns.saturating_sub(previous_cpu_ns) as f64 / elapsed_wall_ns * 100.0;
                self.peak_cpu_pct = self.peak_cpu_pct.max(cpu_pct);
            }
            self.last_cpu_runtime_ns = Some(current_cpu_ns);
        }
        self.last_at = now;
        self.sample_count = self.sample_count.saturating_add(1);
    }

    fn finish(self) -> IdleSampleSummary {
        let finished_at = Instant::now();
        let elapsed_ns = finished_at.duration_since(self.started_at).as_nanos() as f64;
        let avg_cpu_pct = match (self.start_cpu_runtime_ns, current_cpu_runtime_ns()) {
            (Some(start_cpu_ns), Some(end_cpu_ns)) if elapsed_ns > 0.0 => {
                end_cpu_ns.saturating_sub(start_cpu_ns) as f64 / elapsed_ns * 100.0
            }
            _ => 0.0,
        };
        let rss_end_kib = current_rss_kib().unwrap_or(self.rss_start_kib);

        IdleSampleSummary {
            sample_duration_ms: elapsed_ns / 1_000_000.0,
            sample_count: self.sample_count,
            avg_cpu_pct,
            peak_cpu_pct: self.peak_cpu_pct,
            rss_start_kib: self.rss_start_kib,
            rss_end_kib,
            rss_delta_kib: i64::try_from(rss_end_kib).unwrap_or(i64::MAX)
                - i64::try_from(self.rss_start_kib).unwrap_or(i64::MAX),
        }
    }
}

#[cfg(any(test, feature = "benchmarks"))]
impl IdleResourceFixture {
    pub fn cpu_usage_single_repo_60s() -> Self {
        Self::build(
            IdleResourceScenario::CpuUsageSingleRepo60s,
            IdleResourceConfig::cpu_usage_single_repo(),
        )
    }

    pub fn cpu_usage_ten_repos_60s() -> Self {
        Self::build(
            IdleResourceScenario::CpuUsageTenRepos60s,
            IdleResourceConfig::cpu_usage_ten_repos(),
        )
    }

    pub fn memory_growth_single_repo_10min() -> Self {
        Self::build(
            IdleResourceScenario::MemoryGrowthSingleRepo10Min,
            IdleResourceConfig::memory_growth_single_repo(),
        )
    }

    pub fn memory_growth_ten_repos_10min() -> Self {
        Self::build(
            IdleResourceScenario::MemoryGrowthTenRepos10Min,
            IdleResourceConfig::memory_growth_ten_repos(),
        )
    }

    pub fn background_refresh_cost_per_cycle() -> Self {
        Self::build(
            IdleResourceScenario::BackgroundRefreshCostPerCycle,
            IdleResourceConfig::background_refresh_cost_per_cycle(),
        )
    }

    pub fn wake_from_sleep_resume() -> Self {
        Self::build(
            IdleResourceScenario::WakeFromSleepResume,
            IdleResourceConfig::wake_from_sleep_resume(),
        )
    }

    pub fn with_config(scenario: IdleResourceScenario, config: IdleResourceConfig) -> Self {
        Self::build(scenario, config)
    }

    fn build(scenario: IdleResourceScenario, mut config: IdleResourceConfig) -> Self {
        config.repo_count = config.repo_count.max(1);
        config.tracked_files_per_repo = config.tracked_files_per_repo.max(1);
        config.sample_interval = if config.sample_interval.is_zero() {
            Duration::from_millis(1)
        } else {
            config.sample_interval
        };
        if matches!(
            scenario,
            IdleResourceScenario::BackgroundRefreshCostPerCycle
                | IdleResourceScenario::WakeFromSleepResume
        ) {
            config.refresh_cycles = config.refresh_cycles.max(1);
        }

        let mut repo_roots = Vec::with_capacity(config.repo_count);
        let mut repos = Vec::with_capacity(config.repo_count);
        let backend = GixBackend;
        for _ in 0..config.repo_count {
            let repo_root = build_git_ops_status_repo(config.tracked_files_per_repo, 0);
            let repo = backend
                .open(repo_root.path())
                .expect("open idle_resource benchmark repo");
            repo_roots.push(repo_root);
            repos.push(repo);
        }

        Self {
            _repo_roots: repo_roots,
            repos,
            scenario,
            config,
        }
    }

    pub fn run(&self) -> u64 {
        self.execute().0
    }

    pub fn run_with_metrics(&self) -> (u64, IdleResourceMetrics) {
        self.execute()
    }

    fn execute(&self) -> (u64, IdleResourceMetrics) {
        let mut metrics = IdleResourceMetrics {
            open_repos: u64::try_from(self.repos.len()).unwrap_or(u64::MAX),
            tracked_files_per_repo: u64::try_from(self.config.tracked_files_per_repo)
                .unwrap_or(u64::MAX),
            ..IdleResourceMetrics::default()
        };
        let mut work_hash = 0u64;

        let sample_summary = match self.scenario {
            IdleResourceScenario::CpuUsageSingleRepo60s
            | IdleResourceScenario::CpuUsageTenRepos60s
            | IdleResourceScenario::MemoryGrowthSingleRepo10Min
            | IdleResourceScenario::MemoryGrowthTenRepos10Min => {
                self.measure_passive_window(self.config.sample_window, self.config.sample_interval)
            }
            IdleResourceScenario::BackgroundRefreshCostPerCycle => {
                let mut sampler = IdleSampler::start();
                let mut total_cycle_ms = 0.0f64;
                let mut max_cycle_ms = 0.0f64;
                for cycle_index in 0..self.config.refresh_cycles {
                    let cycle_started = Instant::now();
                    let (cycle_hash, status_calls, status_ms) = self.refresh_all_repos();
                    work_hash ^= cycle_hash;
                    metrics.status_calls = metrics.status_calls.saturating_add(status_calls);
                    metrics.status_ms += status_ms;
                    let cycle_ms = cycle_started.elapsed().as_secs_f64() * 1_000.0;
                    total_cycle_ms += cycle_ms;
                    max_cycle_ms = max_cycle_ms.max(cycle_ms);
                    metrics.refresh_cycles = metrics.refresh_cycles.saturating_add(1);
                    metrics.repos_refreshed = metrics
                        .repos_refreshed
                        .saturating_add(u64::try_from(self.repos.len()).unwrap_or(u64::MAX));
                    sampler.sample();
                    if cycle_index + 1 < self.config.refresh_cycles
                        && !self.config.sample_interval.is_zero()
                    {
                        std::thread::sleep(self.config.sample_interval);
                    }
                }
                metrics.avg_refresh_cycle_ms = total_cycle_ms / self.config.refresh_cycles as f64;
                metrics.max_refresh_cycle_ms = max_cycle_ms;
                sampler.finish()
            }
            IdleResourceScenario::WakeFromSleepResume => {
                if !self.config.wake_gap.is_zero() {
                    std::thread::sleep(self.config.wake_gap);
                }
                let mut sampler = IdleSampler::start();
                let cycle_started = Instant::now();
                let (cycle_hash, status_calls, status_ms) = self.refresh_all_repos();
                work_hash = cycle_hash;
                metrics.status_calls = status_calls;
                metrics.status_ms = status_ms;
                metrics.wake_resume_ms = cycle_started.elapsed().as_secs_f64() * 1_000.0;
                metrics.refresh_cycles = 1;
                metrics.repos_refreshed = u64::try_from(self.repos.len()).unwrap_or(u64::MAX);
                metrics.avg_refresh_cycle_ms = metrics.wake_resume_ms;
                metrics.max_refresh_cycle_ms = metrics.wake_resume_ms;
                sampler.sample();
                sampler.finish()
            }
        };

        metrics.sample_duration_ms = sample_summary.sample_duration_ms;
        metrics.sample_count = sample_summary.sample_count;
        metrics.avg_cpu_pct = sample_summary.avg_cpu_pct;
        metrics.peak_cpu_pct = sample_summary.peak_cpu_pct;
        metrics.rss_start_kib = sample_summary.rss_start_kib;
        metrics.rss_end_kib = sample_summary.rss_end_kib;
        metrics.rss_delta_kib = sample_summary.rss_delta_kib;

        let mut h = FxHasher::default();
        std::mem::discriminant(&self.scenario).hash(&mut h);
        metrics.open_repos.hash(&mut h);
        metrics.tracked_files_per_repo.hash(&mut h);
        metrics.sample_count.hash(&mut h);
        metrics.refresh_cycles.hash(&mut h);
        metrics.repos_refreshed.hash(&mut h);
        metrics.status_calls.hash(&mut h);
        work_hash.hash(&mut h);
        (h.finish(), metrics)
    }

    fn measure_passive_window(&self, window: Duration, interval: Duration) -> IdleSampleSummary {
        let mut sampler = IdleSampler::start();
        let steps = idle_sample_steps(window, interval);
        let mut remaining = window;

        for step in 0..steps {
            let sleep_for = if step + 1 == steps {
                remaining
            } else {
                remaining.min(interval)
            };
            if !sleep_for.is_zero() {
                std::thread::sleep(sleep_for);
                remaining = remaining.saturating_sub(sleep_for);
            }
            sampler.sample();
        }

        sampler.finish()
    }

    fn refresh_all_repos(&self) -> (u64, u64, f64) {
        let mut h = FxHasher::default();
        let mut status_calls = 0u64;
        let mut status_ms = 0.0f64;
        for repo in &self.repos {
            let started_at = Instant::now();
            let status = repo.status().expect("idle_resource repo refresh status");
            status_ms += started_at.elapsed().as_secs_f64() * 1_000.0;
            status_calls = status_calls.saturating_add(1);
            hash_repo_status(&status).hash(&mut h);
        }
        (h.finish(), status_calls, status_ms)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn idle_sample_steps(window: Duration, interval: Duration) -> usize {
    if window.is_zero() {
        return 1;
    }

    let interval_nanos = interval.as_nanos().max(1);
    let window_nanos = window.as_nanos();
    let steps = window_nanos.saturating_add(interval_nanos.saturating_sub(1)) / interval_nanos;
    usize::try_from(steps.max(1)).unwrap_or(usize::MAX)
}

#[cfg(target_os = "linux")]
#[cfg(any(test, feature = "benchmarks"))]
fn current_cpu_runtime_ns() -> Option<u64> {
    let schedstat = fs::read_to_string("/proc/self/schedstat").ok()?;
    let runtime_ns = schedstat.split_whitespace().next()?;
    runtime_ns.parse::<u64>().ok()
}

#[cfg(not(target_os = "linux"))]
#[cfg(any(test, feature = "benchmarks"))]
fn current_cpu_runtime_ns() -> Option<u64> {
    None
}

#[cfg(target_os = "linux")]
#[cfg(any(test, feature = "benchmarks"))]
fn current_rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status.lines().find_map(|line| {
        let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
        value.parse::<u64>().ok()
    })
}

#[cfg(not(target_os = "linux"))]
#[cfg(any(test, feature = "benchmarks"))]
fn current_rss_kib() -> Option<u64> {
    None
}

// ---------------------------------------------------------------------------
// Clipboard — copy from diff, paste into commit message, selection range
// ---------------------------------------------------------------------------

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardScenario {
    /// Extract 10k diff lines into a single clipboard string.
    CopyFromDiff,
    /// Insert a large block of text into an empty commit-message TextModel.
    PasteIntoCommitMessage,
    /// Compute the extracted text across a 5k-line selection range in a diff.
    SelectRangeInDiff,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ClipboardMetrics {
    pub total_lines: u64,
    pub total_bytes: u64,
    pub line_iterations: u64,
    pub allocations_approx: u64,
}

pub struct ClipboardFixture {
    /// Pre-built diff lines for copy/select scenarios, or None for paste.
    diff_lines: Option<Vec<DiffLine>>,
    /// Pre-generated large text for the paste scenario.
    paste_text: Option<String>,
    scenario: ClipboardScenario,
    /// Number of lines to select (for SelectRangeInDiff).
    select_range_lines: usize,
}

impl ClipboardFixture {
    /// Copy 10k lines from an inline diff view — measures the string extraction
    /// cost that `selected_diff_text_string()` would pay before writing to the
    /// system clipboard.
    pub fn copy_from_diff(line_count: usize) -> Self {
        let diff_lines = build_synthetic_diff_lines(line_count.max(1));
        Self {
            diff_lines: Some(diff_lines),
            paste_text: None,
            scenario: ClipboardScenario::CopyFromDiff,
            select_range_lines: 0,
        }
    }

    /// Paste a large block of text into an empty commit-message `TextModel`.
    /// Measures the cost of `TextModel::replace_range` with a large insertion.
    pub fn paste_into_commit_message(line_count: usize, line_bytes: usize) -> Self {
        let lines = build_synthetic_source_lines(line_count.max(1), line_bytes.max(32));
        let text = lines.join("\n");
        Self {
            diff_lines: None,
            paste_text: Some(text),
            scenario: ClipboardScenario::PasteIntoCommitMessage,
            select_range_lines: 0,
        }
    }

    /// Select a range of 5k lines in a diff — measures the iteration and text
    /// extraction cost of building the selection string.
    pub fn select_range_in_diff(total_lines: usize, select_lines: usize) -> Self {
        let total = total_lines.max(1);
        let select = select_lines.min(total).max(1);
        let diff_lines = build_synthetic_diff_lines(total);
        Self {
            diff_lines: Some(diff_lines),
            paste_text: None,
            scenario: ClipboardScenario::SelectRangeInDiff,
            select_range_lines: select,
        }
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    pub fn run_with_metrics(&self) -> (u64, ClipboardMetrics) {
        match self.scenario {
            ClipboardScenario::CopyFromDiff => self.run_copy(),
            ClipboardScenario::PasteIntoCommitMessage => self.run_paste(),
            ClipboardScenario::SelectRangeInDiff => self.run_select(),
        }
    }

    /// Simulates `selected_diff_text_string()` — iterates all diff lines,
    /// extracts the text content, and concatenates into a clipboard string.
    fn run_copy(&self) -> (u64, ClipboardMetrics) {
        let lines = self.diff_lines.as_ref().expect("copy needs diff_lines");
        let mut h = FxHasher::default();
        let mut out = String::new();
        let mut line_iterations = 0u64;
        let mut allocations_approx = 0u64;

        for line in lines.iter() {
            line_iterations += 1;
            // Skip header/hunk lines (like the real copy path does — header
            // lines appear in the gutter but their text is not part of the
            // user-visible selection).
            match line.kind {
                DiffLineKind::Header | DiffLineKind::Hunk => continue,
                _ => {}
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line.text);
            allocations_approx += 1;
        }

        out.len().hash(&mut h);
        out.as_bytes().first().copied().unwrap_or(0).hash(&mut h);
        out.as_bytes().last().copied().unwrap_or(0).hash(&mut h);

        let metrics = ClipboardMetrics {
            total_lines: lines.len() as u64,
            total_bytes: out.len() as u64,
            line_iterations,
            allocations_approx,
        };

        (h.finish(), metrics)
    }

    /// Simulates pasting a large text block into the commit message editor.
    fn run_paste(&self) -> (u64, ClipboardMetrics) {
        let text = self.paste_text.as_ref().expect("paste needs paste_text");
        let mut h = FxHasher::default();

        // Create a fresh TextModel and insert the paste text at position 0.
        let mut model = TextModel::new();
        let inserted = model.replace_range(0..0, text);

        model.len().hash(&mut h);
        inserted.start.hash(&mut h);
        inserted.end.hash(&mut h);
        model.revision().hash(&mut h);

        let line_count = text.lines().count() as u64;
        let metrics = ClipboardMetrics {
            total_lines: line_count,
            total_bytes: text.len() as u64,
            line_iterations: 1, // single bulk insertion
            allocations_approx: 1,
        };

        (h.finish(), metrics)
    }

    /// Simulates computing a selection range across `select_range_lines` diff
    /// lines and extracting the text — the same work that happens when the user
    /// shift-clicks to extend a selection.
    fn run_select(&self) -> (u64, ClipboardMetrics) {
        let lines = self.diff_lines.as_ref().expect("select needs diff_lines");
        let end = self.select_range_lines.min(lines.len());
        let mut h = FxHasher::default();
        let mut out = String::new();
        let mut line_iterations = 0u64;
        let mut allocations_approx = 0u64;

        for line in lines[..end].iter() {
            line_iterations += 1;
            match line.kind {
                DiffLineKind::Header | DiffLineKind::Hunk => continue,
                _ => {}
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&line.text);
            allocations_approx += 1;
        }

        out.len().hash(&mut h);
        out.as_bytes().first().copied().unwrap_or(0).hash(&mut h);
        out.as_bytes().last().copied().unwrap_or(0).hash(&mut h);

        let metrics = ClipboardMetrics {
            total_lines: lines.len() as u64,
            total_bytes: out.len() as u64,
            line_iterations,
            allocations_approx,
        };

        (h.finish(), metrics)
    }
}

// ---------------------------------------------------------------------------
// Network-adjacent operations — mocked transport progress and cancellation
// ---------------------------------------------------------------------------

/// Synthetic network benchmark scenarios.
///
/// GitComet currently only exposes structured long-running progress state for
/// clone operations, so these fixtures reuse the real clone-progress reducer
/// path (`Msg::CloneRepo` + `InternalMsg::CloneRepoProgress`) while modeling a
/// fetch-style remote operation on top of it.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetworkScenario {
    UiResponsivenessDuringFetch,
    ProgressBarUpdateRenderCost,
    CancelOperationLatency,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct NetworkMetrics {
    pub total_frames: u64,
    pub scroll_frames: u64,
    pub progress_updates: u64,
    pub render_passes: u64,
    pub output_tail_lines: u64,
    pub tail_trim_events: u64,
    pub rendered_bytes: u64,
    pub total_rows: u64,
    pub window_rows: u64,
    pub bar_width: u64,
    pub cancel_frames_until_stopped: u64,
    pub drained_updates_after_cancel: u64,
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug)]
struct MockNetworkProgressSnapshot {
    seq: u64,
    objects_done: u64,
    objects_total: u64,
    bytes_done: u64,
    bytes_total: u64,
    progress_line: String,
}

#[cfg(any(test, feature = "benchmarks"))]
enum MockNetworkEvent {
    Progress(MockNetworkProgressSnapshot),
    Finished,
    Cancelled,
}

#[cfg(any(test, feature = "benchmarks"))]
struct MockNetworkTransport<'a> {
    snapshots: &'a [MockNetworkProgressSnapshot],
    cursor: usize,
    cancel_drain_events_remaining: Option<usize>,
    terminal_emitted: bool,
}

#[cfg(any(test, feature = "benchmarks"))]
impl<'a> MockNetworkTransport<'a> {
    fn new(snapshots: &'a [MockNetworkProgressSnapshot]) -> Self {
        Self {
            snapshots,
            cursor: 0,
            cancel_drain_events_remaining: None,
            terminal_emitted: false,
        }
    }

    fn request_cancel(&mut self, drain_events: usize) {
        self.cancel_drain_events_remaining = Some(drain_events);
    }

    fn next_event(&mut self) -> Option<MockNetworkEvent> {
        if self.terminal_emitted {
            return None;
        }

        if let Some(remaining) = self.cancel_drain_events_remaining.as_mut() {
            if *remaining == 0 {
                self.terminal_emitted = true;
                return Some(MockNetworkEvent::Cancelled);
            }
            *remaining = remaining.saturating_sub(1);
        }

        if let Some(snapshot) = self.snapshots.get(self.cursor).cloned() {
            self.cursor = self.cursor.saturating_add(1);
            return Some(MockNetworkEvent::Progress(snapshot));
        }

        self.terminal_emitted = true;
        Some(match self.cancel_drain_events_remaining {
            Some(_) => MockNetworkEvent::Cancelled,
            None => MockNetworkEvent::Finished,
        })
    }
}

pub struct NetworkFixture {
    baseline: AppState,
    transport_url: String,
    transport_dest: std::path::PathBuf,
    snapshots: Vec<MockNetworkProgressSnapshot>,
    history_fixture: Option<HistoryListScrollFixture>,
    scenario: NetworkScenario,
    window_rows: usize,
    scroll_step_rows: usize,
    bar_width: usize,
    frame_budget_ns: u64,
    cancel_after_updates: usize,
    cancel_drain_events: usize,
}

impl NetworkFixture {
    pub fn ui_responsiveness_during_fetch(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        window_rows: usize,
        scroll_step_rows: usize,
        frames: usize,
        line_bytes: usize,
        bar_width: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let transport_url = "https://example.invalid/gitcomet/network.git".to_string();
        let transport_dest = std::path::PathBuf::from("/tmp/gitcomet-network-ui-responsiveness");
        let baseline = build_network_baseline_state(&transport_url, &transport_dest);
        let total_frames = frames.max(1);

        Self {
            baseline,
            transport_url,
            transport_dest,
            snapshots: build_mock_network_progress_snapshots(total_frames, line_bytes.max(48)),
            history_fixture: Some(HistoryListScrollFixture::new(
                history_commits,
                local_branches,
                remote_branches,
            )),
            scenario: NetworkScenario::UiResponsivenessDuringFetch,
            window_rows: window_rows.max(1),
            scroll_step_rows: scroll_step_rows.max(1),
            bar_width: bar_width.max(8),
            frame_budget_ns: frame_budget_ns.max(1),
            cancel_after_updates: 0,
            cancel_drain_events: 0,
        }
    }

    pub fn progress_bar_update_render_cost(
        updates: usize,
        line_bytes: usize,
        bar_width: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let transport_url = "https://example.invalid/gitcomet/network.git".to_string();
        let transport_dest = std::path::PathBuf::from("/tmp/gitcomet-network-progress-bar");
        let baseline = build_network_baseline_state(&transport_url, &transport_dest);
        let total_frames = updates.max(1);

        Self {
            baseline,
            transport_url,
            transport_dest,
            snapshots: build_mock_network_progress_snapshots(total_frames, line_bytes.max(48)),
            history_fixture: None,
            scenario: NetworkScenario::ProgressBarUpdateRenderCost,
            window_rows: 0,
            scroll_step_rows: 0,
            bar_width: bar_width.max(8),
            frame_budget_ns: frame_budget_ns.max(1),
            cancel_after_updates: 0,
            cancel_drain_events: 0,
        }
    }

    pub fn cancel_operation_latency(
        cancel_after_updates: usize,
        cancel_drain_events: usize,
        total_updates: usize,
        line_bytes: usize,
        bar_width: usize,
        frame_budget_ns: u64,
    ) -> Self {
        let transport_url = "https://example.invalid/gitcomet/network.git".to_string();
        let transport_dest = std::path::PathBuf::from("/tmp/gitcomet-network-cancel");
        let baseline = build_network_baseline_state(&transport_url, &transport_dest);
        let cancel_after_updates = cancel_after_updates.max(1);
        let total_frames = total_updates.max(
            cancel_after_updates
                .saturating_add(cancel_drain_events)
                .saturating_add(1),
        );

        Self {
            baseline,
            transport_url,
            transport_dest,
            snapshots: build_mock_network_progress_snapshots(total_frames, line_bytes.max(48)),
            history_fixture: None,
            scenario: NetworkScenario::CancelOperationLatency,
            window_rows: 0,
            scroll_step_rows: 0,
            bar_width: bar_width.max(8),
            frame_budget_ns: frame_budget_ns.max(1),
            cancel_after_updates,
            cancel_drain_events,
        }
    }

    fn fresh_state(&self) -> AppState {
        self.baseline.clone()
    }

    pub fn run(&self) -> u64 {
        self.run_internal(None).0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, crate::view::perf::FrameTimingStats, NetworkMetrics) {
        let mut capture = crate::view::perf::FrameTimingCapture::new(self.frame_budget_ns);
        let (hash, metrics) = self.run_internal(Some(&mut capture));
        (hash, capture.finish(), metrics)
    }

    fn run_internal(
        &self,
        mut capture: Option<&mut crate::view::perf::FrameTimingCapture>,
    ) -> (u64, NetworkMetrics) {
        let mut state = self.fresh_state();
        let mut transport = MockNetworkTransport::new(&self.snapshots);
        let mut hash = 0u64;
        let mut metrics = NetworkMetrics {
            bar_width: bench_counter_u64(self.bar_width),
            ..NetworkMetrics::default()
        };

        match self.scenario {
            NetworkScenario::UiResponsivenessDuringFetch => {
                let history_fixture = self
                    .history_fixture
                    .as_ref()
                    .expect("ui responsiveness needs history fixture");
                let total_rows = history_fixture.total_rows();
                let window_rows = self.window_rows.min(total_rows.max(1));
                let max_start = total_rows.saturating_sub(window_rows);
                let mut start = 0usize;

                metrics.total_rows = bench_counter_u64(total_rows);
                metrics.window_rows = bench_counter_u64(window_rows);

                while let Some(event) = transport.next_event() {
                    let MockNetworkEvent::Progress(snapshot) = event else {
                        break;
                    };

                    let frame_started = Instant::now();
                    apply_mock_network_progress(&mut state, &self.transport_dest, &snapshot);
                    let clone_state = state.clone.as_ref().expect("clone progress state");
                    let (render_hash, rendered_bytes) = render_mock_network_progress(
                        &self.transport_url,
                        &self.transport_dest,
                        &clone_state.output_tail,
                        &snapshot,
                        self.bar_width,
                        None,
                    );
                    hash ^= render_hash;
                    hash ^= history_fixture.run_scroll_step(start, window_rows);

                    if max_start > 0 {
                        start = start.saturating_add(self.scroll_step_rows);
                        if start > max_start {
                            start %= max_start + 1;
                        }
                    }

                    metrics.total_frames = metrics.total_frames.saturating_add(1);
                    metrics.scroll_frames = metrics.scroll_frames.saturating_add(1);
                    metrics.progress_updates = metrics.progress_updates.saturating_add(1);
                    metrics.render_passes = metrics.render_passes.saturating_add(1);
                    metrics.rendered_bytes =
                        metrics.rendered_bytes.saturating_add(rendered_bytes as u64);

                    if let Some(capture) = capture.as_deref_mut() {
                        capture.record_frame(frame_started.elapsed());
                    }
                }
            }
            NetworkScenario::ProgressBarUpdateRenderCost => {
                while let Some(event) = transport.next_event() {
                    let MockNetworkEvent::Progress(snapshot) = event else {
                        break;
                    };

                    let frame_started = Instant::now();
                    apply_mock_network_progress(&mut state, &self.transport_dest, &snapshot);
                    let clone_state = state.clone.as_ref().expect("clone progress state");
                    let (render_hash, rendered_bytes) = render_mock_network_progress(
                        &self.transport_url,
                        &self.transport_dest,
                        &clone_state.output_tail,
                        &snapshot,
                        self.bar_width,
                        None,
                    );
                    hash ^= render_hash;

                    metrics.total_frames = metrics.total_frames.saturating_add(1);
                    metrics.progress_updates = metrics.progress_updates.saturating_add(1);
                    metrics.render_passes = metrics.render_passes.saturating_add(1);
                    metrics.rendered_bytes =
                        metrics.rendered_bytes.saturating_add(rendered_bytes as u64);

                    if let Some(capture) = capture.as_deref_mut() {
                        capture.record_frame(frame_started.elapsed());
                    }
                }
            }
            NetworkScenario::CancelOperationLatency => {
                let mut cancel_requested = false;
                let mut last_snapshot = self
                    .snapshots
                    .first()
                    .cloned()
                    .expect("network snapshots should not be empty");

                while let Some(event) = transport.next_event() {
                    let frame_started = Instant::now();
                    match event {
                        MockNetworkEvent::Progress(snapshot) => {
                            apply_mock_network_progress(
                                &mut state,
                                &self.transport_dest,
                                &snapshot,
                            );
                            let clone_state = state.clone.as_ref().expect("clone progress state");
                            let (render_hash, rendered_bytes) = render_mock_network_progress(
                                &self.transport_url,
                                &self.transport_dest,
                                &clone_state.output_tail,
                                &snapshot,
                                self.bar_width,
                                None,
                            );
                            hash ^= render_hash;

                            metrics.total_frames = metrics.total_frames.saturating_add(1);
                            metrics.progress_updates = metrics.progress_updates.saturating_add(1);
                            metrics.render_passes = metrics.render_passes.saturating_add(1);
                            metrics.rendered_bytes =
                                metrics.rendered_bytes.saturating_add(rendered_bytes as u64);

                            if cancel_requested {
                                metrics.cancel_frames_until_stopped =
                                    metrics.cancel_frames_until_stopped.saturating_add(1);
                                metrics.drained_updates_after_cancel =
                                    metrics.drained_updates_after_cancel.saturating_add(1);
                            }

                            last_snapshot = snapshot;
                            if !cancel_requested
                                && metrics.progress_updates
                                    >= bench_counter_u64(self.cancel_after_updates)
                            {
                                transport.request_cancel(self.cancel_drain_events);
                                cancel_requested = true;
                            }
                        }
                        MockNetworkEvent::Cancelled => {
                            let clone_state = state.clone.as_ref().expect("clone progress state");
                            let (render_hash, rendered_bytes) = render_mock_network_progress(
                                &self.transport_url,
                                &self.transport_dest,
                                &clone_state.output_tail,
                                &last_snapshot,
                                self.bar_width,
                                Some("Fetch cancelled"),
                            );
                            hash ^= render_hash;
                            metrics.total_frames = metrics.total_frames.saturating_add(1);
                            metrics.render_passes = metrics.render_passes.saturating_add(1);
                            metrics.rendered_bytes =
                                metrics.rendered_bytes.saturating_add(rendered_bytes as u64);
                            if cancel_requested {
                                metrics.cancel_frames_until_stopped =
                                    metrics.cancel_frames_until_stopped.saturating_add(1);
                            }

                            if let Some(capture) = capture.as_deref_mut() {
                                capture.record_frame(frame_started.elapsed());
                            }
                            break;
                        }
                        MockNetworkEvent::Finished => break,
                    }

                    if let Some(capture) = capture.as_deref_mut() {
                        capture.record_frame(frame_started.elapsed());
                    }
                }
            }
        }

        if let Some(clone_state) = state.clone.as_ref() {
            metrics.output_tail_lines = bench_counter_u64(clone_state.output_tail.len());
            metrics.tail_trim_events = metrics
                .progress_updates
                .saturating_sub(metrics.output_tail_lines);
        }

        hash ^= metrics.total_frames;
        hash ^= metrics.progress_updates;
        hash ^= metrics.render_passes;
        hash ^= metrics.rendered_bytes;
        hash ^= metrics.cancel_frames_until_stopped;

        (hash, metrics)
    }
}

#[cfg(any(test, feature = "benchmarks"))]
fn build_network_baseline_state(url: &str, dest: &Path) -> AppState {
    let mut state = AppState::default();
    let _ = dispatch_sync(
        &mut state,
        Msg::CloneRepo {
            url: url.to_string(),
            dest: dest.to_path_buf(),
        },
    );
    state
}

#[cfg(any(test, feature = "benchmarks"))]
fn build_mock_network_progress_snapshots(
    updates: usize,
    line_bytes: usize,
) -> Vec<MockNetworkProgressSnapshot> {
    let updates = updates.max(1);
    let line_bytes = line_bytes.max(48);
    let objects_total = u64::try_from(updates.saturating_mul(24)).unwrap_or(u64::MAX);
    let bytes_total =
        u64::try_from(updates.saturating_mul(line_bytes).saturating_mul(64)).unwrap_or(u64::MAX);
    let mut snapshots = Vec::with_capacity(updates);

    for ix in 0..updates {
        let progress_ix = ix.saturating_add(1);
        let objects_done = u64::try_from(progress_ix.saturating_mul(24)).unwrap_or(u64::MAX);
        let bytes_done = u64::try_from(progress_ix.saturating_mul(line_bytes).saturating_mul(64))
            .unwrap_or(u64::MAX);
        let percent = ((progress_ix.saturating_mul(100)) / updates).min(100);
        let phase = match ix % 3 {
            0 => "remote: Counting objects",
            1 => "Receiving objects",
            _ => "Resolving deltas",
        };
        let mut progress_line = format!(
            "{phase}: {percent:>3}% ({objects_done}/{objects_total}) bytes={bytes_done}/{bytes_total}"
        );
        if progress_line.len() < line_bytes {
            progress_line.push(' ');
            progress_line.push_str("//");
            while progress_line.len() < line_bytes {
                let _ = write!(progress_line, " net_token_{}", ix % 97);
            }
        }

        snapshots.push(MockNetworkProgressSnapshot {
            seq: u64::try_from(progress_ix).unwrap_or(u64::MAX),
            objects_done,
            objects_total,
            bytes_done,
            bytes_total,
            progress_line,
        });
    }

    snapshots
}

#[cfg(any(test, feature = "benchmarks"))]
fn apply_mock_network_progress(
    state: &mut AppState,
    dest: &Path,
    snapshot: &MockNetworkProgressSnapshot,
) {
    let _ = dispatch_sync(
        state,
        Msg::Internal(InternalMsg::CloneRepoProgress {
            dest: dest.to_path_buf(),
            line: snapshot.progress_line.clone(),
        }),
    );
}

#[cfg(any(test, feature = "benchmarks"))]
fn render_mock_network_progress(
    url: &str,
    dest: &Path,
    output_tail: &[String],
    snapshot: &MockNetworkProgressSnapshot,
    bar_width: usize,
    title: Option<&str>,
) -> (u64, usize) {
    let bar_width = bar_width.max(8);
    let fill = usize::try_from(
        ((snapshot.bytes_done.saturating_mul(bar_width as u64)) / snapshot.bytes_total.max(1))
            .min(bar_width as u64),
    )
    .unwrap_or(bar_width);
    let empty = bar_width.saturating_sub(fill);
    let percent =
        ((snapshot.bytes_done.saturating_mul(100)) / snapshot.bytes_total.max(1)).min(100);

    let mut message = String::new();
    message.push_str(title.unwrap_or("Fetching repository..."));
    message.push('\n');
    message.push_str(url);
    message.push('\n');
    message.push_str("-> ");
    let _ = write!(message, "{}", dest.display());
    let _ = write!(
        message,
        "\n[{}{}] {percent:>3}% {}/{} objects | {} / {} KiB",
        "#".repeat(fill),
        "-".repeat(empty),
        snapshot.objects_done,
        snapshot.objects_total,
        snapshot.bytes_done / 1024,
        snapshot.bytes_total / 1024
    );

    if !output_tail.is_empty() {
        message.push_str("\n\n");
        let visible_start = output_tail.len().saturating_sub(12);
        for (ix, line) in output_tail.iter().skip(visible_start).enumerate() {
            if ix > 0 {
                message.push('\n');
            }
            message.push_str(line);
        }
    }

    let mut h = FxHasher::default();
    message.hash(&mut h);
    snapshot.seq.hash(&mut h);
    (h.finish(), message.len())
}

// ---------------------------------------------------------------------------
// display — render cost at different scale factors, multi-window, DPI switch
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayScenario {
    RenderCostByScale,
    TwoWindowsSameRepo,
    WindowMoveBetweenDpis,
}

/// Sidecar metrics for display benchmark scenarios.
#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DisplayMetrics {
    pub scale_factors_tested: u64,
    pub total_layout_passes: u64,
    pub total_rows_rendered: u64,
    pub history_rows_per_pass: u64,
    pub diff_rows_per_pass: u64,
    pub layout_width_min_px: f64,
    pub layout_width_max_px: f64,
    pub windows_rendered: u64,
    pub re_layout_passes: u64,
}

pub struct DisplayFixture {
    scenario: DisplayScenario,
    history: HistoryListScrollFixture,
    diff: FileDiffOpenFixture,
    history_window_rows: usize,
    diff_window_rows: usize,
    /// Scale factors to test (e.g. [1, 2, 3] for 1x/2x/3x).
    scale_factors: Vec<u32>,
    /// Base window width at 1x in logical pixels.
    base_window_width: f32,
    /// Sidebar and details widths (for layout computation).
    sidebar_w: f32,
    details_w: f32,
}

impl DisplayFixture {
    /// `render_cost_1x_vs_2x_vs_3x_scale`: measure rendering cost at three
    /// DPI scale factors. At higher scales, the effective physical viewport is
    /// wider, so more columns of text and wider layout computations are needed.
    /// We model this by running layout at `base_width * scale` and rendering
    /// history + diff rows for a visible window at each scale.
    pub fn render_cost_by_scale(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        diff_lines: usize,
        history_window_rows: usize,
        diff_window_rows: usize,
        base_window_width: f32,
        sidebar_w: f32,
        details_w: f32,
    ) -> Self {
        Self {
            scenario: DisplayScenario::RenderCostByScale,
            history: HistoryListScrollFixture::new(
                history_commits.max(1),
                local_branches,
                remote_branches,
            ),
            diff: FileDiffOpenFixture::new(diff_lines.max(10)),
            history_window_rows: history_window_rows.max(1),
            diff_window_rows: diff_window_rows.max(1),
            scale_factors: vec![1, 2, 3],
            base_window_width: base_window_width.max(400.0),
            sidebar_w,
            details_w,
        }
    }

    /// `two_windows_same_repo`: render two viewports from the same repo state
    /// (one history, one diff) concurrently, testing cache sharing cost.
    pub fn two_windows_same_repo(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        diff_lines: usize,
        history_window_rows: usize,
        diff_window_rows: usize,
        base_window_width: f32,
        sidebar_w: f32,
        details_w: f32,
    ) -> Self {
        Self {
            scenario: DisplayScenario::TwoWindowsSameRepo,
            history: HistoryListScrollFixture::new(
                history_commits.max(1),
                local_branches,
                remote_branches,
            ),
            diff: FileDiffOpenFixture::new(diff_lines.max(10)),
            history_window_rows: history_window_rows.max(1),
            diff_window_rows: diff_window_rows.max(1),
            scale_factors: vec![1],
            base_window_width: base_window_width.max(400.0),
            sidebar_w,
            details_w,
        }
    }

    /// `window_move_between_dpis`: render at 1x, then re-render at 2x to
    /// simulate moving a window from a standard monitor to a HiDPI monitor.
    pub fn window_move_between_dpis(
        history_commits: usize,
        local_branches: usize,
        remote_branches: usize,
        diff_lines: usize,
        history_window_rows: usize,
        diff_window_rows: usize,
        base_window_width: f32,
        sidebar_w: f32,
        details_w: f32,
    ) -> Self {
        Self {
            scenario: DisplayScenario::WindowMoveBetweenDpis,
            history: HistoryListScrollFixture::new(
                history_commits.max(1),
                local_branches,
                remote_branches,
            ),
            diff: FileDiffOpenFixture::new(diff_lines.max(10)),
            history_window_rows: history_window_rows.max(1),
            diff_window_rows: diff_window_rows.max(1),
            scale_factors: vec![1, 2],
            base_window_width: base_window_width.max(400.0),
            sidebar_w,
            details_w,
        }
    }

    pub fn run(&self) -> u64 {
        self.run_internal().0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, DisplayMetrics) {
        self.run_internal()
    }

    fn run_internal(&self) -> (u64, DisplayMetrics) {
        use crate::view::panes::main::pane_content_width_for_layout;

        let mut hash = 0u64;
        let total_history = self.history.total_rows();
        let history_window = self.history_window_rows.min(total_history.max(1));
        let mut metrics = DisplayMetrics {
            history_rows_per_pass: bench_counter_u64(history_window),
            diff_rows_per_pass: bench_counter_u64(self.diff_window_rows),
            ..DisplayMetrics::default()
        };
        let mut min_width: f32 = f32::MAX;
        let mut max_width: f32 = f32::MIN;

        match self.scenario {
            DisplayScenario::RenderCostByScale => {
                // Render history + diff at each scale factor.
                for &scale in &self.scale_factors {
                    let effective_width = self.base_window_width * scale as f32;
                    let main_w = pane_content_width_for_layout(
                        px(effective_width),
                        px(self.sidebar_w * scale as f32),
                        px(self.details_w * scale as f32),
                        false,
                        false,
                    );
                    let main_f: f32 = main_w.into();
                    hash ^= main_f.to_bits() as u64;
                    if main_f < min_width {
                        min_width = main_f;
                    }
                    if main_f > max_width {
                        max_width = main_f;
                    }
                    metrics.total_layout_passes += 1;

                    // Render history window from the middle.
                    let history_start = total_history.saturating_sub(history_window) / 2;
                    hash ^= self.history.run_scroll_step(history_start, history_window);
                    metrics.total_rows_rendered += history_window as u64;

                    // Render diff window.
                    hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                    metrics.total_rows_rendered += self.diff_window_rows as u64;

                    metrics.windows_rendered += 1;
                }
                metrics.scale_factors_tested = self.scale_factors.len() as u64;
            }
            DisplayScenario::TwoWindowsSameRepo => {
                // Two concurrent viewports at the same scale.
                let effective_width = self.base_window_width;
                let main_w = pane_content_width_for_layout(
                    px(effective_width),
                    px(self.sidebar_w),
                    px(self.details_w),
                    false,
                    false,
                );
                let main_f: f32 = main_w.into();
                hash ^= main_f.to_bits() as u64;
                min_width = main_f;
                max_width = main_f;
                metrics.total_layout_passes += 1;

                // Window 1: history from top.
                let history_start_1 = 0;
                hash ^= self
                    .history
                    .run_scroll_step(history_start_1, history_window);
                metrics.total_rows_rendered += history_window as u64;
                metrics.windows_rendered += 1;

                // Window 1: diff.
                hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;

                // Window 2: history from bottom.
                let history_start_2 = total_history.saturating_sub(history_window);
                hash ^= self
                    .history
                    .run_scroll_step(history_start_2, history_window);
                metrics.total_rows_rendered += history_window as u64;
                metrics.windows_rendered += 1;

                // Window 2: diff (inline view instead of split).
                hash ^= self.diff.run_inline_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;

                metrics.scale_factors_tested = 1;
            }
            DisplayScenario::WindowMoveBetweenDpis => {
                // Initial render at 1x.
                let scale_1x = self.scale_factors.first().copied().unwrap_or(1);
                let width_1x = self.base_window_width * scale_1x as f32;
                let main_1x = pane_content_width_for_layout(
                    px(width_1x),
                    px(self.sidebar_w * scale_1x as f32),
                    px(self.details_w * scale_1x as f32),
                    false,
                    false,
                );
                let main_1x_f: f32 = main_1x.into();
                hash ^= main_1x_f.to_bits() as u64;
                min_width = main_1x_f;
                max_width = main_1x_f;
                metrics.total_layout_passes += 1;

                let history_start = total_history.saturating_sub(history_window) / 2;
                hash ^= self.history.run_scroll_step(history_start, history_window);
                metrics.total_rows_rendered += history_window as u64;
                hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;
                metrics.windows_rendered += 1;

                // Re-render at higher DPI (simulates monitor move).
                let scale_hi = self.scale_factors.last().copied().unwrap_or(2);
                let width_hi = self.base_window_width * scale_hi as f32;
                let main_hi = pane_content_width_for_layout(
                    px(width_hi),
                    px(self.sidebar_w * scale_hi as f32),
                    px(self.details_w * scale_hi as f32),
                    false,
                    false,
                );
                let main_hi_f: f32 = main_hi.into();
                hash ^= main_hi_f.to_bits() as u64;
                if main_hi_f < min_width {
                    min_width = main_hi_f;
                }
                if main_hi_f > max_width {
                    max_width = main_hi_f;
                }
                metrics.re_layout_passes += 1;
                metrics.total_layout_passes += 1;

                // Full re-render at new scale — both history and diff.
                hash ^= self.history.run_scroll_step(history_start, history_window);
                metrics.total_rows_rendered += history_window as u64;
                hash ^= self.diff.run_split_first_window(self.diff_window_rows);
                metrics.total_rows_rendered += self.diff_window_rows as u64;
                metrics.windows_rendered += 1;

                metrics.scale_factors_tested = self.scale_factors.len() as u64;
            }
        }

        metrics.layout_width_min_px = min_width as f64;
        metrics.layout_width_max_px = max_width as f64;

        (hash, metrics)
    }
}
