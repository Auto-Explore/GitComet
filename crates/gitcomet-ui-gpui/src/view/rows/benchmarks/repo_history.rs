use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct BranchSidebarFingerprint {
    head_branch_rev: u64,
    branches_rev: u64,
    remotes_rev: u64,
    remote_branches_rev: u64,
    worktrees_rev: u64,
    submodules_rev: u64,
    stashes_rev: u64,
}

impl BranchSidebarFingerprint {
    fn from_repo(repo: &RepoState) -> Self {
        Self {
            head_branch_rev: repo.head_branch_rev,
            branches_rev: repo.branches_rev,
            remotes_rev: repo.remotes_rev,
            remote_branches_rev: repo.remote_branches_rev,
            worktrees_rev: repo.worktrees_rev,
            submodules_rev: repo.submodules_rev,
            stashes_rev: repo.stashes_rev,
        }
    }
}

#[derive(Clone, Debug)]
struct BranchSidebarCache {
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
    source_fingerprint: u64,
    source_parts: BranchSidebarSourceParts,
    rows: Rc<[BranchSidebarRow]>,
}

type BranchSidebarSourceParts = ();

fn branch_sidebar_branch_label(name: &str) -> SharedString {
    SharedString::from(name.rsplit('/').next().unwrap_or(name).to_string())
}

fn branch_sidebar_source_fingerprint(
    repo: &RepoState,
    _cached_source_parts: Option<&BranchSidebarSourceParts>,
) -> (u64, BranchSidebarSourceParts) {
    let mut hasher = FxHasher::default();
    repo.head_branch_rev.hash(&mut hasher);
    repo.branches_rev.hash(&mut hasher);
    repo.remotes_rev.hash(&mut hasher);
    repo.remote_branches_rev.hash(&mut hasher);
    repo.worktrees_rev.hash(&mut hasher);
    repo.submodules_rev.hash(&mut hasher);
    repo.stashes_rev.hash(&mut hasher);
    (hasher.finish(), ())
}

fn branch_sidebar_cache_lookup(
    cache: &mut Option<BranchSidebarCache>,
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
) -> Option<Rc<[BranchSidebarRow]>> {
    cache.as_ref().and_then(|cached| {
        (cached.repo_id == repo_id && cached.fingerprint == fingerprint)
            .then(|| Rc::clone(&cached.rows))
    })
}

fn branch_sidebar_cache_lookup_by_cached_source(
    _cache: &mut Option<BranchSidebarCache>,
    _repo: &RepoState,
    _fingerprint: BranchSidebarFingerprint,
) -> Option<Rc<[BranchSidebarRow]>> {
    None
}

fn branch_sidebar_cache_lookup_by_source(
    cache: &mut Option<BranchSidebarCache>,
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
    source_fingerprint: u64,
    _source_parts: &BranchSidebarSourceParts,
) -> Option<Rc<[BranchSidebarRow]>> {
    cache.as_ref().and_then(|cached| {
        (cached.repo_id == repo_id
            && cached.fingerprint == fingerprint
            && cached.source_fingerprint == source_fingerprint)
            .then(|| Rc::clone(&cached.rows))
    })
}

fn branch_sidebar_cache_store(
    cache: &mut Option<BranchSidebarCache>,
    repo_id: RepoId,
    fingerprint: BranchSidebarFingerprint,
    source_fingerprint: u64,
    source_parts: BranchSidebarSourceParts,
    rows: Rc<[BranchSidebarRow]>,
) {
    *cache = Some(BranchSidebarCache {
        repo_id,
        fingerprint,
        source_fingerprint,
        source_parts,
        rows,
    });
}

#[derive(Clone, Debug)]
struct HistoryWhenVm(SharedString);

impl HistoryWhenVm {
    fn deferred(time: Option<SystemTime>) -> Self {
        Self(
            time.map(|time| {
                time.duration_since(SystemTime::UNIX_EPOCH)
                    .ok()
                    .map(|elapsed| elapsed.as_secs().to_string())
                    .unwrap_or_default()
                    .into()
            })
            .unwrap_or_default(),
        )
    }
}

#[derive(Clone, Debug)]
struct HistoryShortShaVm(SharedString);

impl HistoryShortShaVm {
    fn new(commit_id: &str) -> Self {
        Self(commit_id.chars().take(7).collect::<String>().into())
    }

    fn as_str(&self) -> &str {
        self.0.as_ref()
    }
}

#[derive(Clone, Debug)]
struct BenchHistoryStashTip {
    commit_ix: usize,
    message: Option<Arc<str>>,
}

#[derive(Clone, Debug, Default)]
struct BenchHistoryStashAnalysis {
    stash_tips: Vec<BenchHistoryStashTip>,
    stash_helper_ids: HashSet<CommitId>,
}

fn analyze_history_stashes(
    commits: &[Commit],
    stashes: &[StashEntry],
) -> BenchHistoryStashAnalysis {
    let commit_ix_by_id = commits
        .iter()
        .enumerate()
        .map(|(ix, commit)| (commit.id.clone(), ix))
        .collect::<HashMap<_, _>>();
    let mut stash_tips = stashes
        .iter()
        .filter_map(|stash| {
            commit_ix_by_id
                .get(&stash.id)
                .copied()
                .map(|commit_ix| BenchHistoryStashTip {
                    commit_ix,
                    message: Some(Arc::clone(&stash.message)),
                })
        })
        .collect::<Vec<_>>();
    stash_tips.sort_by_key(|tip| tip.commit_ix);
    BenchHistoryStashAnalysis {
        stash_tips,
        stash_helper_ids: HashSet::default(),
    }
}

fn build_history_visible_indices(
    commits: &[Commit],
    _stash_helper_ids: &HashSet<CommitId>,
) -> Vec<usize> {
    (0..commits.len()).collect()
}

fn build_history_branch_text_by_target<'a>(
    branches: &'a [Branch],
    remote_branches: &'a [RemoteBranch],
    head_branch: Option<&'a str>,
    head_target: Option<&'a str>,
) -> (HashMap<&'a str, SharedString>, Option<SharedString>) {
    let mut by_target: HashMap<&'a str, Vec<String>> = HashMap::default();
    for branch in branches {
        by_target
            .entry(branch.target.as_ref())
            .or_default()
            .push(branch.name.clone());
    }
    for branch in remote_branches {
        by_target
            .entry(branch.target.as_ref())
            .or_default()
            .push(format!("{}/{}", branch.remote, branch.name));
    }
    let head_label = head_branch.map(|name| branch_sidebar_branch_label(name).to_string());
    let mut out = HashMap::default();
    let mut head_branches_text = None;
    for (target, names) in by_target {
        let joined: SharedString = names.join(", ").into();
        if Some(target) == head_target {
            head_branches_text = Some(
                head_label
                    .clone()
                    .map(SharedString::from)
                    .unwrap_or_else(|| joined.clone()),
            );
        }
        out.insert(target, joined);
    }
    (out, head_branches_text)
}

fn build_history_tag_names_by_target<'a>(tags: &'a [Tag]) -> HashMap<&'a str, Arc<[SharedString]>> {
    let mut by_target: HashMap<&'a str, Vec<SharedString>> = HashMap::default();
    for tag in tags {
        by_target
            .entry(tag.target.as_ref())
            .or_default()
            .push(tag.name.clone().into());
    }
    by_target
        .into_iter()
        .map(|(target, names)| (target, Arc::<[SharedString]>::from(names)))
        .collect()
}

fn next_history_stash_tip_for_commit_ix<'a>(
    stash_tips: &'a [BenchHistoryStashTip],
    next_stash_tip_ix: &mut usize,
    commit_ix: &usize,
) -> Option<&'a BenchHistoryStashTip> {
    let Some(tip) = stash_tips.get(*next_stash_tip_ix) else {
        return None;
    };
    if tip.commit_ix == *commit_ix {
        *next_stash_tip_ix = next_stash_tip_ix.saturating_add(1);
        return Some(tip);
    }
    None
}

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
            let branch_heads = empty_history_graph_heads();
            let graph =
                history_graph::compute_graph(&self.commits, self.theme, &branch_heads, None);

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
        let branch_heads = empty_history_graph_heads();
        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads, None);

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

pub(crate) fn hash_branch_sidebar_rows(rows: &[BranchSidebarRow]) -> u64 {
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
                label,
                tooltip,
                is_active,
                ..
            } => {
                let path_len = path
                    .to_str()
                    .map_or_else(|| path.to_string_lossy().len(), str::len);
                let label_len = label.len();
                label_len.hash(&mut h);
                tooltip.len().hash(&mut h);
                path_len.hash(&mut h);
                is_active.hash(&mut h);
            }
            BranchSidebarRow::SubmoduleItem {
                path,
                label,
                tooltip,
                ..
            } => {
                let path_len = path
                    .to_str()
                    .map_or_else(|| path.to_string_lossy().len(), str::len);
                path_len.hash(&mut h);
                label.len().hash(&mut h);
                tooltip.len().hash(&mut h);
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
    pub(crate) fn row_count(&self) -> usize {
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
    pub(crate) repo: RepoState,
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
    /// row slice (cache hit).  On mismatch or cold cache → rebuilds rows (cache
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

        if let Some(cached_rows) =
            branch_sidebar_cache_lookup_by_cached_source(&mut self.cache, &self.repo, fingerprint)
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
        let rows: Rc<[BranchSidebarRow]> = GitCometView::branch_sidebar_rows(&self.repo).into();
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
        self.metrics.invalidations += 1;
        self.run_cached()
    }

    /// Invalidate the cache by bumping `worktrees_rev` (simulating worktree
    /// placeholders completing their async load) and then rebuild.  Returns
    /// row-count hash.
    pub fn run_invalidate_worktrees_ready(&mut self) -> u64 {
        self.repo.worktrees_rev = self.repo.worktrees_rev.wrapping_add(1);
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

fn hash_repo_switch_outcome(state: &AppState, effects: &[Effect]) -> u64 {
    fn hash_diff_target(target: &DiffTarget, h: &mut FxHasher) {
        match target {
            DiffTarget::WorkingTree { path, area } => {
                path.hash(h);
                (*area as u8).hash(h);
            }
            DiffTarget::Commit { commit_id, path } => {
                commit_id.hash(h);
                path.hash(h);
            }
        }
    }

    fn hash_repo_selected_diff_target(state: &AppState, repo_id: RepoId, h: &mut FxHasher) {
        if let Some(target) = state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.diff_state.diff_target.as_ref())
        {
            hash_diff_target(target, h);
        }
    }

    let mut h = FxHasher::default();
    state.active_repo.hash(&mut h);
    state.repos.len().hash(&mut h);
    effects.len().hash(&mut h);

    for effect in effects.iter().take(32) {
        std::mem::discriminant(effect).hash(&mut h);
        match effect {
            Effect::LoadDiff { repo_id, target }
            | Effect::LoadDiffFile { repo_id, target }
            | Effect::LoadDiffFileImage { repo_id, target } => {
                repo_id.0.hash(&mut h);
                hash_diff_target(target, &mut h);
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

    h.finish()
}

pub(crate) fn reset_repo_switch_bench_state(state: &mut AppState, baseline: &AppState) {
    debug_assert_eq!(state.repos.len(), baseline.repos.len());
    state.active_repo = baseline.active_repo;

    for (repo_state, baseline_repo) in state.repos.iter_mut().zip(baseline.repos.iter()) {
        repo_state.loads_in_flight = baseline_repo.loads_in_flight.clone();
        repo_state.log_loading_more = baseline_repo.log_loading_more;
        repo_state.history_state.log_loading_more = baseline_repo.history_state.log_loading_more;
    }
}

pub struct RepoSwitchFixture {
    pub(crate) baseline: AppState,
    pub(crate) target_repo_id: RepoId,
}

impl RepoSwitchFixture {
    pub(crate) fn flipped_direction(&self) -> Self {
        let active_repo_id = self.baseline.active_repo.unwrap_or(self.target_repo_id);
        debug_assert_ne!(active_repo_id, self.target_repo_id);

        let mut baseline = self.baseline.clone();
        baseline.active_repo = Some(self.target_repo_id);

        Self {
            baseline,
            target_repo_id: active_repo_id,
        }
    }

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
        self.baseline.clone()
    }

    pub fn run_with_state(&self, state: &mut AppState) -> (u64, RepoSwitchMetrics) {
        with_set_active_repo_sync(state, self.target_repo_id, |state, effects| {
            (
                hash_repo_switch_outcome(state, effects),
                RepoSwitchMetrics::from_state_and_effects(state, effects),
            )
        })
    }

    pub fn run_with_state_hash_only(&self, state: &mut AppState) -> u64 {
        with_set_active_repo_sync(state, self.target_repo_id, |state, effects| {
            hash_repo_switch_outcome(state, effects)
        })
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
        let branch_heads =
            history_graph_heads_from_indices(&self.commits, &self.branch_head_indices);
        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads, None);
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
        let branch_heads =
            history_graph_heads_from_indices(&self.commits, &self.branch_head_indices);
        let graph = history_graph::compute_graph(&self.commits, self.theme, &branch_heads, None);

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
    pub(crate) fn commit_count(&self) -> usize {
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
            theme: AppTheme::gitcomet_dark(),
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
            theme: AppTheme::gitcomet_dark(),
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
            theme: AppTheme::gitcomet_dark(),
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
            theme: AppTheme::gitcomet_dark(),
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
            theme: AppTheme::gitcomet_dark(),
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

        // 1. stash tip analysis
        let stash_analysis = analyze_history_stashes(commits, stashes);
        let stash_tips = stash_analysis.stash_tips;
        let stash_helper_ids = stash_analysis.stash_helper_ids;

        let visible_indices = build_history_visible_indices(commits, &stash_helper_ids);
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
        let branch_heads = history_graph_heads_from_branches(branches, remote_branches);
        let graph_rows: Arc<[history_graph::GraphRow]> = if stash_helper_ids.is_empty() {
            history_graph::compute_graph(commits, theme, &branch_heads, head_target).into()
        } else {
            let visible_commits = visible_indices
                .iter()
                .map(|&ix| commits[ix].clone())
                .collect::<Vec<_>>();
            history_graph::compute_graph(&visible_commits, theme, &branch_heads, head_target).into()
        };
        let max_lanes = graph_rows
            .iter()
            .map(|r| r.lanes_now.len().max(r.lanes_next.len()))
            .max()
            .unwrap_or(1);

        // 8. branch/tag decorations precomputed once per target
        let (mut branch_text_by_target, head_branches_text) = build_history_branch_text_by_target(
            branches,
            remote_branches,
            head_branch.as_deref(),
            head_target,
        );
        let mut tag_names_by_target = build_history_tag_names_by_target(tags);

        // 9. commit_row_vms — replicate the VM construction from ensure_history_cache
        let mut decorated_count = 0usize;
        let has_stash_tips = !stash_tips.is_empty();
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
        if has_stash_tips {
            let mut next_stash_tip_ix = 0usize;
            for ix in visible_indices.iter() {
                let Some(commit) = commits.get(*ix) else {
                    continue;
                };
                let commit_id = commit.id.as_ref();
                let is_head = head_target == Some(commit_id);

                let branches_text = if is_head {
                    head_branches_text.clone().unwrap_or_default()
                } else {
                    branch_text_by_target.remove(commit_id).unwrap_or_default()
                };

                let tag_names = tag_names_by_target.remove(commit_id).unwrap_or_default();

                if is_head || !branches_text.is_empty() || !tag_names.is_empty() {
                    decorated_count += 1;
                }

                let author: SharedString = author_cache
                    .entry(commit.author.as_ref())
                    .or_insert_with(|| commit.author.clone().into())
                    .clone();
                let (is_stash, summary): (bool, SharedString) =
                    match next_history_stash_tip_for_commit_ix(
                        &stash_tips,
                        &mut next_stash_tip_ix,
                        ix,
                    ) {
                        Some(stash_tip) => (
                            true,
                            stash_tip
                                .message
                                .as_ref()
                                .map(|message| SharedString::from(message.to_string()))
                                .or_else(|| {
                                    Self::stash_summary_from_log_summary(&commit.summary)
                                        .map(SharedString::new)
                                })
                                .unwrap_or_else(|| commit.summary.clone().into()),
                        ),
                        None => (false, commit.summary.clone().into()),
                    };

                commit_row_vms.push((
                    branches_text,
                    tag_names,
                    author,
                    summary,
                    HistoryWhenVm::deferred(Some(commit.time)),
                    HistoryShortShaVm::new(commit.id.as_ref()),
                    is_head,
                    is_stash,
                ));
            }
        } else {
            for ix in visible_indices.iter() {
                let Some(commit) = commits.get(*ix) else {
                    continue;
                };
                let commit_id = commit.id.as_ref();
                let is_head = head_target == Some(commit_id);

                let branches_text = if is_head {
                    head_branches_text.clone().unwrap_or_default()
                } else {
                    branch_text_by_target.remove(commit_id).unwrap_or_default()
                };

                let tag_names = tag_names_by_target.remove(commit_id).unwrap_or_default();

                if is_head || !branches_text.is_empty() || !tag_names.is_empty() {
                    decorated_count += 1;
                }

                let author: SharedString = author_cache
                    .entry(commit.author.as_ref())
                    .or_insert_with(|| commit.author.clone().into())
                    .clone();

                commit_row_vms.push((
                    branches_text,
                    tag_names,
                    author,
                    commit.summary.clone().into(),
                    HistoryWhenVm::deferred(Some(commit.time)),
                    HistoryShortShaVm::new(commit.id.as_ref()),
                    is_head,
                    false,
                ));
            }
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
        })
    }

    fn response_cursor(&self) -> Option<LogCursor> {
        self.appended_commits.last().map(|commit| LogCursor {
            last_seen: commit.id.clone(),
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
        state.repos.push(repo_state);
        state.active_repo = Some(self.repo_id);

        // Seed the initial page through the reducer so benchmark setup matches
        // the production initial-load path, including any pagination slack.
        let _ = dispatch_sync(
            &mut state,
            Msg::Internal(InternalMsg::LogLoaded {
                repo_id: self.repo_id,
                scope: self.scope,
                cursor: None,
                result: Ok(LogPage {
                    commits: self.existing_commits.clone(),
                    next_cursor: self.request_cursor(),
                }),
            }),
        );

        let repo_state = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == self.repo_id)
            .expect("history load-more fixture should keep its repo");
        repo_state.history_state.log_loading_more = true;
        repo_state.log_loading_more = true;
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
    message_render: Option<CommitDetailsMessageRenderState>,
    file_rows: RefCell<CommitFileRowPresentationCache<CommitId>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommitDetailsMessageRenderConfig {
    pub(crate) visible_lines: usize,
    pub(crate) wrap_width_px: usize,
    pub(crate) max_shape_bytes: usize,
}

#[derive(Clone, Debug)]
pub(crate) struct CommitDetailsMessageRenderState {
    pub(crate) message_len: usize,
    pub(crate) line_count: usize,
    pub(crate) shaped_bytes: usize,
    pub(crate) visible_lines: Vec<CommitDetailsVisibleMessageLine>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommitDetailsVisibleMessageLine {
    pub(crate) shaping_hash: u64,
    pub(crate) capped_len: usize,
    pub(crate) wrap_rows: usize,
}

impl CommitDetailsFixture {
    pub fn new(files: usize, depth: usize) -> Self {
        Self {
            details: build_synthetic_commit_details(files, depth),
            message_render: None,
            file_rows: RefCell::new(CommitFileRowPresentationCache::default()),
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
        let message_render = CommitDetailsMessageRenderConfig {
            visible_lines: visible_lines.max(1),
            wrap_width_px: wrap_width_px.max(1),
            max_shape_bytes: 4 * 1024,
        };
        let details = build_synthetic_commit_details_with_message(
            files,
            depth,
            build_synthetic_commit_message(message_bytes, line_bytes),
        );
        Self {
            message_render: Some(build_commit_details_message_render_state(
                details.message.as_str(),
                message_render,
            )),
            details,
            file_rows: RefCell::new(CommitFileRowPresentationCache::default()),
        }
    }

    pub fn prewarm_runtime_state(&self) {
        let mut file_rows = self.file_rows.borrow_mut();
        let _ = file_rows.rows_for(&self.details.id, &self.details.files);
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn reset_runtime_state(&self) {
        self.file_rows.borrow_mut().clear();
    }

    pub fn run(&self) -> u64 {
        let file_rows = {
            let mut file_rows = self.file_rows.borrow_mut();
            commit_details_cached_row_hash(
                &self.details,
                self.message_render.as_ref(),
                &mut file_rows,
            )
        };
        file_rows
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
        let (message_shaped_lines, message_shaped_bytes) =
            measure_commit_message_visible_window(self.message_render.as_ref());

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
    prewarmed_file_rows: CommitFileRowPresentationCache<CommitId>,
}

impl CommitSelectReplaceFixture {
    pub fn new(files: usize, depth: usize) -> Self {
        let commit_a = build_synthetic_commit_details(files, depth);
        let commit_b = build_synthetic_commit_details_with_id(files, depth, "e");
        let mut prewarmed_file_rows = CommitFileRowPresentationCache::default();
        let _ = prewarmed_file_rows.rows_for(&commit_a.id, &commit_a.files);
        Self {
            commit_a,
            commit_b,
            prewarmed_file_rows,
        }
    }

    /// Run the replacement starting from the first commit's already-rendered
    /// file rows, then switch to commit_b and hash the replacement work only.
    pub fn run(&self) -> u64 {
        let mut file_rows = self.prewarmed_file_rows.clone();
        commit_details_cached_row_hash(&self.commit_b, None, &mut file_rows)
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, CommitSelectReplaceMetrics) {
        let mut file_rows_a = self.prewarmed_file_rows.clone();
        let hash_a = commit_details_cached_row_hash(&self.commit_a, None, &mut file_rows_a);
        let hash_b = self.run();
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
            hash_shared_string_identity(&display, &mut h);
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
