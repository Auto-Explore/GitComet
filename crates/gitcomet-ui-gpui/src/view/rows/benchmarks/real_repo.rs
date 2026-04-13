use super::*;
use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};
use gitcomet_core::domain::FileConflictKind;
use serde::Deserialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

const DEFAULT_MONOREPO_HISTORY_LIMIT: usize = 10_000;
const DEFAULT_DEEP_HISTORY_LIMIT: usize = 50_000;
const DEFAULT_HISTORY_PAGE_SIZE: usize = 1_000;
const DEFAULT_HISTORY_WINDOW: usize = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RealRepoScenario {
    MonorepoOpenAndHistoryLoad,
    DeepHistoryOpenAndScroll,
    MidMergeConflictListAndOpen,
    LargeFileDiffOpen,
}

impl RealRepoScenario {
    pub fn case_name(self) -> &'static str {
        match self {
            Self::MonorepoOpenAndHistoryLoad => "monorepo_open_and_history_load",
            Self::DeepHistoryOpenAndScroll => "deep_history_open_and_scroll",
            Self::MidMergeConflictListAndOpen => "mid_merge_conflict_list_and_open",
            Self::LargeFileDiffOpen => "large_file_diff_open",
        }
    }

    fn default_history_limit(self) -> usize {
        match self {
            Self::MonorepoOpenAndHistoryLoad => DEFAULT_MONOREPO_HISTORY_LIMIT,
            Self::DeepHistoryOpenAndScroll => DEFAULT_DEEP_HISTORY_LIMIT,
            Self::MidMergeConflictListAndOpen | Self::LargeFileDiffOpen => 0,
        }
    }

    fn needs_history(self) -> bool {
        matches!(
            self,
            Self::MonorepoOpenAndHistoryLoad | Self::DeepHistoryOpenAndScroll
        )
    }
}

#[cfg(any(test, feature = "benchmarks"))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RealRepoMetrics {
    pub worktree_file_count: u64,
    pub status_entries: u64,
    pub local_branches: u64,
    pub remote_branches: u64,
    pub remotes: u64,
    pub commits_loaded: u64,
    pub log_pages_loaded: u64,
    pub next_cursor_present: u64,
    pub sidebar_rows: u64,
    pub graph_rows: u64,
    pub max_graph_lanes: u64,
    pub history_windows_scanned: u64,
    pub history_rows_scanned: u64,
    pub conflict_files: u64,
    pub conflict_regions: u64,
    pub selected_conflict_bytes: u64,
    pub diff_lines: u64,
    pub file_old_bytes: u64,
    pub file_new_bytes: u64,
    pub split_rows_painted: u64,
    pub inline_rows_painted: u64,
    pub status_calls: u64,
    pub log_walk_calls: u64,
    pub diff_calls: u64,
    pub ref_enumerate_calls: u64,
    pub status_ms: f64,
    pub log_walk_ms: f64,
    pub diff_ms: f64,
    pub ref_enumerate_ms: f64,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct RealRepoSnapshotMetadata {
    source: String,
    #[serde(default)]
    checkout_ref: Option<String>,
    #[serde(default)]
    merge_ref: Option<String>,
    #[serde(default)]
    conflict_path: Option<PathBuf>,
    #[serde(default)]
    diff_path: Option<PathBuf>,
    #[serde(default)]
    diff_commitish: Option<String>,
    #[serde(default)]
    history_limit: Option<usize>,
    #[serde(default)]
    history_page_size: Option<usize>,
    #[serde(default)]
    history_window: Option<usize>,
}

pub struct RealRepoFixture {
    _repo_root: TempDir,
    repo: Arc<dyn GitRepository>,
    scenario: RealRepoScenario,
    worktree_file_count: usize,
    history_limit: usize,
    history_page_size: usize,
    history_window: usize,
    selected_conflict_path: Option<PathBuf>,
    selected_diff_target: Option<DiffTarget>,
}

impl RealRepoFixture {
    pub fn from_snapshot_root(
        snapshot_root: impl AsRef<Path>,
        scenario: RealRepoScenario,
    ) -> Result<Self, String> {
        let case_dir = snapshot_root.as_ref().join(scenario.case_name());
        let metadata_path = case_dir.join("metadata.json");
        let metadata_json = fs::read_to_string(&metadata_path).map_err(|err| {
            format!(
                "failed to read real_repo metadata {}: {err}",
                metadata_path.display()
            )
        })?;
        let metadata: RealRepoSnapshotMetadata =
            serde_json::from_str(&metadata_json).map_err(|err| {
                format!(
                    "failed to parse real_repo metadata {}: {err}",
                    metadata_path.display()
                )
            })?;
        let source_path = resolve_snapshot_source(&case_dir, metadata.source.as_str());
        if !source_path.exists() {
            return Err(format!(
                "real_repo snapshot source for {} does not exist: {}",
                scenario.case_name(),
                source_path.display()
            ));
        }

        let repo_root = tempfile::tempdir().map_err(|err| {
            format!(
                "failed to create tempdir for real_repo {}: {err}",
                scenario.case_name()
            )
        })?;
        let worktree = repo_root.path().join("repo");
        clone_snapshot_source(&source_path, &worktree)?;
        configure_git_identity(&worktree);

        if let Some(checkout_ref) = metadata.checkout_ref.as_deref() {
            let checkout_ref = resolve_cloned_ref(&worktree, checkout_ref);
            run_git(&worktree, &["checkout", "--quiet", checkout_ref.as_str()]);
        }

        if scenario == RealRepoScenario::MidMergeConflictListAndOpen {
            let merge_ref = metadata.merge_ref.as_deref().ok_or_else(|| {
                format!(
                    "real_repo {} metadata must provide merge_ref",
                    scenario.case_name()
                )
            })?;
            let merge_ref = resolve_cloned_ref(&worktree, merge_ref);
            run_git_merge_allow_conflict(&worktree, merge_ref.as_str())?;
        }

        let backend = GixBackend;
        let repo = backend.open(&worktree).map_err(|err| {
            format!(
                "open real_repo {} fixture repo: {err}",
                scenario.case_name()
            )
        })?;

        let history_limit = if scenario.needs_history() {
            metadata
                .history_limit
                .unwrap_or_else(|| scenario.default_history_limit())
                .max(1)
        } else {
            0
        };
        let history_page_size = metadata
            .history_page_size
            .unwrap_or(DEFAULT_HISTORY_PAGE_SIZE)
            .max(1);
        let history_window = metadata
            .history_window
            .unwrap_or(DEFAULT_HISTORY_WINDOW)
            .max(1);

        let selected_conflict_path = metadata.conflict_path;
        let selected_diff_target = if scenario == RealRepoScenario::LargeFileDiffOpen {
            let diff_path = metadata.diff_path.ok_or_else(|| {
                format!(
                    "real_repo {} metadata must provide diff_path",
                    scenario.case_name()
                )
            })?;
            let commitish = metadata
                .diff_commitish
                .as_deref()
                .unwrap_or("HEAD")
                .to_string();
            Some(DiffTarget::Commit {
                commit_id: CommitId(resolve_commitish(&worktree, &commitish).into()),
                path: Some(diff_path),
            })
        } else {
            None
        };

        Ok(Self {
            _repo_root: repo_root,
            repo,
            scenario,
            worktree_file_count: count_worktree_files(&worktree)?,
            history_limit,
            history_page_size,
            history_window,
            selected_conflict_path,
            selected_diff_target,
        })
    }

    pub fn run(&self) -> u64 {
        self.run_with_metrics().0
    }

    #[cfg(any(test, feature = "benchmarks"))]
    pub fn run_with_metrics(&self) -> (u64, RealRepoMetrics) {
        let _capture = git_ops_trace::capture();
        let (hash, mut metrics) = match self.scenario {
            RealRepoScenario::MonorepoOpenAndHistoryLoad => self.run_monorepo_open_and_history(),
            RealRepoScenario::DeepHistoryOpenAndScroll => self.run_deep_history_open_and_scroll(),
            RealRepoScenario::MidMergeConflictListAndOpen => {
                self.run_mid_merge_conflict_list_and_open()
            }
            RealRepoScenario::LargeFileDiffOpen => self.run_large_file_diff_open(),
        };
        let trace = git_ops_trace::snapshot();
        metrics.status_calls = trace.status.calls;
        metrics.log_walk_calls = trace.log_walk.calls;
        metrics.diff_calls = trace.diff.calls;
        metrics.ref_enumerate_calls = trace.ref_enumerate.calls;
        metrics.status_ms = trace.status.total_millis();
        metrics.log_walk_ms = trace.log_walk.total_millis();
        metrics.diff_ms = trace.diff.total_millis();
        metrics.ref_enumerate_ms = trace.ref_enumerate.total_millis();
        (hash, metrics)
    }

    fn run_monorepo_open_and_history(&self) -> (u64, RealRepoMetrics) {
        let status = self
            .repo
            .status()
            .expect("real_repo monorepo status benchmark");
        let branches = self
            .repo
            .list_branches()
            .expect("real_repo monorepo branches benchmark");
        let remotes = self
            .repo
            .list_remotes()
            .expect("real_repo monorepo remotes benchmark");
        let remote_branches = self
            .repo
            .list_remote_branches()
            .expect("real_repo monorepo remote branches benchmark");
        let head_branch = self.repo.current_branch().ok();
        let (commits, next_cursor_present, log_pages_loaded) = load_log_commits(
            self.repo.as_ref(),
            self.history_limit,
            self.history_page_size,
        );
        let repo_state = build_real_repo_state(
            self.repo.spec().clone(),
            head_branch,
            branches.clone(),
            remotes.clone(),
            remote_branches.clone(),
            status.clone(),
            commits.clone(),
            next_cursor_present,
        );
        let sidebar_rows = GitCometView::branch_sidebar_rows(&repo_state);
        let graph = history_graph::compute_graph(
            commits.as_slice(),
            AppTheme::gitcomet_dark(),
            std::iter::empty(),
            None,
        );

        let mut hasher = FxHasher::default();
        self.worktree_file_count.hash(&mut hasher);
        hash_repo_status(&status).hash(&mut hasher);
        hash_branch_sidebar_rows(sidebar_rows.as_slice()).hash(&mut hasher);
        hash_history_graph_window(graph.as_slice(), commits.as_slice(), 0, 256, &mut hasher);
        let max_graph_lanes = graph
            .iter()
            .map(|row| row.lanes_now.len().max(row.lanes_next.len()))
            .max()
            .unwrap_or_default();

        (
            hasher.finish(),
            RealRepoMetrics {
                worktree_file_count: bench_counter_u64(self.worktree_file_count),
                status_entries: bench_counter_u64(status.staged.len() + status.unstaged.len()),
                local_branches: bench_counter_u64(branches.len()),
                remote_branches: bench_counter_u64(remote_branches.len()),
                remotes: bench_counter_u64(remotes.len()),
                commits_loaded: bench_counter_u64(commits.len()),
                log_pages_loaded: bench_counter_u64(log_pages_loaded),
                next_cursor_present: u64::from(next_cursor_present),
                sidebar_rows: bench_counter_u64(sidebar_rows.len()),
                graph_rows: bench_counter_u64(graph.len()),
                max_graph_lanes: bench_counter_u64(max_graph_lanes),
                ..RealRepoMetrics::default()
            },
        )
    }

    fn run_deep_history_open_and_scroll(&self) -> (u64, RealRepoMetrics) {
        let (commits, next_cursor_present, log_pages_loaded) = load_log_commits(
            self.repo.as_ref(),
            self.history_limit,
            self.history_page_size,
        );
        let graph = history_graph::compute_graph(
            commits.as_slice(),
            AppTheme::gitcomet_dark(),
            std::iter::empty(),
            None,
        );
        let window = self.history_window.min(graph.len().max(1));
        let positions = [
            0,
            graph.len().saturating_sub(window) / 2,
            graph.len().saturating_sub(window),
        ];

        let mut hasher = FxHasher::default();
        self.worktree_file_count.hash(&mut hasher);
        for start in positions {
            hash_history_graph_window(
                graph.as_slice(),
                commits.as_slice(),
                start,
                window,
                &mut hasher,
            );
        }

        let max_graph_lanes = graph
            .iter()
            .map(|row| row.lanes_now.len().max(row.lanes_next.len()))
            .max()
            .unwrap_or_default();

        (
            hasher.finish(),
            RealRepoMetrics {
                worktree_file_count: bench_counter_u64(self.worktree_file_count),
                commits_loaded: bench_counter_u64(commits.len()),
                log_pages_loaded: bench_counter_u64(log_pages_loaded),
                next_cursor_present: u64::from(next_cursor_present),
                graph_rows: bench_counter_u64(graph.len()),
                max_graph_lanes: bench_counter_u64(max_graph_lanes),
                history_windows_scanned: bench_counter_u64(positions.len()),
                history_rows_scanned: bench_counter_u64(positions.len().saturating_mul(window)),
                ..RealRepoMetrics::default()
            },
        )
    }

    fn run_mid_merge_conflict_list_and_open(&self) -> (u64, RealRepoMetrics) {
        let status = self
            .repo
            .status()
            .expect("real_repo mid-merge status benchmark");
        let conflict_paths =
            conflict_paths_from_status_or_git(&status, self.repo.spec().workdir.as_path());
        let selected_path = self
            .selected_conflict_path
            .clone()
            .or_else(|| conflict_paths.first().cloned())
            .expect("real_repo mid-merge conflict path");
        let session = self.conflict_session_from_repo(&status, &selected_path);

        let mut hasher = FxHasher::default();
        self.worktree_file_count.hash(&mut hasher);
        hash_repo_status(&status).hash(&mut hasher);
        selected_path.hash(&mut hasher);
        hash_conflict_session(&session, &mut hasher);

        (
            hasher.finish(),
            RealRepoMetrics {
                worktree_file_count: bench_counter_u64(self.worktree_file_count),
                status_entries: bench_counter_u64(status.staged.len() + status.unstaged.len()),
                conflict_files: bench_counter_u64(conflict_paths.len().max(1)),
                conflict_regions: bench_counter_u64(session.regions.len()),
                selected_conflict_bytes: bench_counter_u64(conflict_session_bytes(&session)),
                ..RealRepoMetrics::default()
            },
        )
    }

    fn run_large_file_diff_open(&self) -> (u64, RealRepoMetrics) {
        use gitcomet_core::domain::DiffRowProvider;

        let target = self
            .selected_diff_target
            .as_ref()
            .expect("real_repo large diff target");
        let diff = self
            .repo
            .diff_parsed(target)
            .expect("real_repo large diff parsed benchmark");
        let file = self
            .repo
            .diff_file_text(target)
            .expect("real_repo large diff file text benchmark")
            .unwrap_or_else(|| panic!("real_repo large diff benchmark requires file text"));

        let old_text = file.old.as_deref().unwrap_or("");
        let new_text = file.new.as_deref().unwrap_or("");
        let rebuild = crate::view::panes::main::diff_cache::build_file_diff_cache_rebuild(
            &file,
            self._repo_root.path(),
        );
        let split = rebuild.row_provider;
        let inline = rebuild.inline_row_provider;
        let split_rows_painted = split
            .slice(0, DEFAULT_HISTORY_WINDOW)
            .take(DEFAULT_HISTORY_WINDOW)
            .count();
        let inline_rows_painted = inline
            .slice(0, DEFAULT_HISTORY_WINDOW)
            .take(DEFAULT_HISTORY_WINDOW)
            .count();

        let mut hasher = FxHasher::default();
        self.worktree_file_count.hash(&mut hasher);
        hash_parsed_diff(&diff).hash(&mut hasher);
        hash_file_diff_window(
            split.as_ref(),
            inline.as_ref(),
            DEFAULT_HISTORY_WINDOW,
            &mut hasher,
        );

        (
            hasher.finish(),
            RealRepoMetrics {
                worktree_file_count: bench_counter_u64(self.worktree_file_count),
                diff_lines: bench_counter_u64(diff.lines.len()),
                file_old_bytes: bench_counter_u64(old_text.len()),
                file_new_bytes: bench_counter_u64(new_text.len()),
                split_rows_painted: bench_counter_u64(split_rows_painted),
                inline_rows_painted: bench_counter_u64(inline_rows_painted),
                ..RealRepoMetrics::default()
            },
        )
    }

    fn conflict_session_from_repo(&self, status: &RepoStatus, path: &Path) -> ConflictSession {
        if let Some(session) = self
            .repo
            .conflict_session(path)
            .expect("real_repo conflict session benchmark")
        {
            return session;
        }

        let stages = self
            .repo
            .conflict_file_stages(path)
            .expect("real_repo conflict file stages benchmark")
            .unwrap_or_else(|| {
                panic!(
                    "real_repo conflict benchmark did not produce stages for {}",
                    path.display()
                )
            });
        let conflict_kind =
            conflict_kind_for_path(status, path).unwrap_or(FileConflictKind::BothModified);
        let base = ConflictPayload::from_stage_parts(stages.base_bytes, stages.base);
        let ours = ConflictPayload::from_stage_parts(stages.ours_bytes, stages.ours);
        let theirs = ConflictPayload::from_stage_parts(stages.theirs_bytes, stages.theirs);
        let current_bytes = fs::read(self.repo.spec().workdir.join(path))
            .ok()
            .map(Arc::<[u8]>::from);
        let current_text = gitcomet_core::services::decode_utf8_optional(current_bytes.as_deref())
            .map(Arc::<str>::from);

        if let Some(text) = current_text {
            ConflictSession::from_merged_shared_text(
                path.to_path_buf(),
                conflict_kind,
                base,
                ours,
                theirs,
                text,
            )
        } else {
            let current = ConflictPayload::from_stage_parts(current_bytes, None);
            ConflictSession::new_with_current(
                path.to_path_buf(),
                conflict_kind,
                base,
                ours,
                theirs,
                current,
            )
        }
    }
}

fn resolve_snapshot_source(case_dir: &Path, source: &str) -> PathBuf {
    let path = PathBuf::from(source);
    if path.is_absolute() {
        path
    } else {
        case_dir.join(path)
    }
}

fn clone_snapshot_source(source: &Path, worktree: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .arg("clone")
        .arg("--quiet")
        .arg(source)
        .arg(worktree)
        .output()
        .map_err(|err| {
            format!(
                "failed to clone real_repo snapshot {}: {err}",
                source.display()
            )
        })?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "git clone failed for {}:\nstdout:\n{}\nstderr:\n{}",
        source.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn configure_git_identity(repo: &Path) {
    run_git(repo, &["config", "user.email", "bench@example.com"]);
    run_git(repo, &["config", "user.name", "Bench"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
}

fn resolve_commitish(repo: &Path, commitish: &str) -> String {
    git_stdout(repo, &["rev-parse", commitish])
}

fn resolve_cloned_ref(repo: &Path, reference: &str) -> String {
    if git_ref_exists(repo, reference) {
        return reference.to_string();
    }

    let remote_tracking = format!("origin/{reference}");
    if git_ref_exists(repo, remote_tracking.as_str()) {
        return remote_tracking;
    }

    reference.to_string()
}

fn git_ref_exists(repo: &Path, reference: &str) -> bool {
    git_command(repo)
        .args(["rev-parse", "--verify", "--quiet", reference])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn run_git_merge_allow_conflict(repo: &Path, reference: &str) -> Result<(), String> {
    let output = git_command(repo)
        .args(["merge", "--no-commit", "--no-ff", "--quiet", reference])
        .output()
        .map_err(|err| format!("run git merge {reference:?} in {}: {err}", repo.display()))?;
    if output.status.success() {
        return Ok(());
    }

    let merge_head_exists = repo.join(".git").join("MERGE_HEAD").exists();
    let has_unmerged_entries = git_command(repo)
        .args(["ls-files", "--unmerged"])
        .output()
        .is_ok_and(|stdout| stdout.status.success() && !stdout.stdout.is_empty());
    if output.status.code() == Some(1) && (merge_head_exists || has_unmerged_entries) {
        return Ok(());
    }

    Err(format!(
        "git merge {:?} failed in {} with code {}:\nstdout:\n{}\nstderr:\n{}",
        reference,
        repo.display(),
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn count_worktree_files(root: &Path) -> Result<usize, String> {
    fn walk(path: &Path) -> Result<usize, String> {
        let mut total = 0usize;
        for entry in fs::read_dir(path)
            .map_err(|err| format!("read_dir {} for real_repo count: {err}", path.display()))?
        {
            let entry = entry.map_err(|err| {
                format!(
                    "read_dir entry {} for real_repo count: {err}",
                    path.display()
                )
            })?;
            let file_type = entry.file_type().map_err(|err| {
                format!(
                    "file_type {} for real_repo count: {err}",
                    entry.path().display()
                )
            })?;
            let name = entry.file_name();
            if name == ".git" {
                continue;
            }
            if file_type.is_dir() {
                total = total.saturating_add(walk(&entry.path())?);
            } else if file_type.is_file() {
                total = total.saturating_add(1);
            }
        }
        Ok(total)
    }

    walk(root)
}

fn load_log_commits(
    repo: &dyn GitRepository,
    limit: usize,
    page_size: usize,
) -> (Vec<Commit>, bool, usize) {
    let limit = limit.max(1);
    let page_size = page_size.max(1);
    let mut commits = Vec::with_capacity(limit);
    let mut cursor = None;
    let mut pages = 0usize;
    let mut next_cursor_present = false;

    while commits.len() < limit {
        let remaining = limit.saturating_sub(commits.len()).max(1);
        let page = repo
            .log_head_page(page_size.min(remaining), cursor.as_ref())
            .expect("real_repo log load benchmark");
        pages = pages.saturating_add(1);
        let exhausted = page.commits.is_empty();
        commits.extend(page.commits);
        cursor = page.next_cursor;
        next_cursor_present = cursor.is_some();
        if exhausted || cursor.is_none() {
            break;
        }
    }

    (commits, next_cursor_present, pages)
}

fn build_real_repo_state(
    spec: RepoSpec,
    head_branch: Option<String>,
    branches: Vec<Branch>,
    remotes: Vec<Remote>,
    remote_branches: Vec<RemoteBranch>,
    status: RepoStatus,
    commits: Vec<Commit>,
    next_cursor_present: bool,
) -> RepoState {
    let mut repo = RepoState::new_opening(RepoId(1), spec);
    repo.open = Loadable::Ready(());
    repo.open_rev = 1;
    repo.head_branch = match head_branch {
        Some(branch) => Loadable::Ready(branch),
        None => Loadable::NotLoaded,
    };
    repo.head_branch_rev = 1;
    repo.branches = Loadable::Ready(Arc::new(branches));
    repo.branches_rev = 1;
    repo.tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.tags_rev = 1;
    repo.remote_tags = Loadable::Ready(Arc::new(Vec::new()));
    repo.remote_tags_rev = 1;
    repo.remotes = Loadable::Ready(Arc::new(remotes));
    repo.remotes_rev = 1;
    repo.remote_branches = Loadable::Ready(Arc::new(remote_branches));
    repo.remote_branches_rev = 1;
    repo.status = Loadable::Ready(Arc::new(status));
    repo.status_rev = 1;
    repo.stashes = Loadable::Ready(Arc::new(Vec::new()));
    repo.stashes_rev = 1;
    repo.worktrees = Loadable::Ready(Arc::new(Vec::new()));
    repo.worktrees_rev = 1;
    repo.submodules = Loadable::Ready(Arc::new(Vec::new()));
    repo.submodules_rev = 1;
    repo.branch_sidebar_rev = 1;

    let log_page = Arc::new(LogPage {
        commits,
        next_cursor: next_cursor_present.then(|| LogCursor {
            last_seen: CommitId("next".repeat(10).into()),
            resume_from: None,
        }),
    });
    repo.log = Loadable::Ready(Arc::clone(&log_page));
    repo.history_state.log = Loadable::Ready(log_page);
    repo.log_rev = 1;
    repo.history_state.log_rev = 1;
    repo
}

fn hash_history_graph_window(
    graph: &[history_graph::GraphRow],
    commits: &[Commit],
    start: usize,
    window: usize,
    hasher: &mut FxHasher,
) {
    let end = start
        .saturating_add(window)
        .min(graph.len())
        .min(commits.len());
    start.hash(hasher);
    end.saturating_sub(start).hash(hasher);
    for (row, commit) in graph[start..end].iter().zip(commits[start..end].iter()) {
        commit.id.hash(hasher);
        commit.parent_ids.len().hash(hasher);
        row.is_merge.hash(hasher);
        row.lanes_now.len().hash(hasher);
        row.lanes_next.len().hash(hasher);
        row.node_col.hash(hasher);
    }
}

fn conflict_paths_from_status(status: &RepoStatus) -> Vec<PathBuf> {
    let mut paths = BTreeSet::new();
    for entry in status.staged.iter().chain(status.unstaged.iter()) {
        if entry.conflict.is_some() || entry.kind == FileStatusKind::Conflicted {
            paths.insert(entry.path.clone());
        }
    }
    paths.into_iter().collect()
}

fn conflict_paths_from_status_or_git(status: &RepoStatus, workdir: &Path) -> Vec<PathBuf> {
    let from_status = conflict_paths_from_status(status);
    if !from_status.is_empty() {
        return from_status;
    }

    let stdout = git_stdout(workdir, &["ls-files", "--unmerged"]);
    let mut paths = BTreeSet::new();
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(path) = trimmed.split_whitespace().last()
            && !path.is_empty()
        {
            paths.insert(PathBuf::from(path));
        }
    }
    paths.into_iter().collect()
}

fn conflict_kind_for_path(status: &RepoStatus, path: &Path) -> Option<FileConflictKind> {
    status
        .staged
        .iter()
        .chain(status.unstaged.iter())
        .find_map(|entry| (entry.path == path).then_some(entry.conflict))
        .flatten()
}

fn hash_conflict_session(session: &ConflictSession, hasher: &mut FxHasher) {
    session.path.hash(hasher);
    std::mem::discriminant(&session.conflict_kind).hash(hasher);
    session.strategy.label().hash(hasher);
    session.regions.len().hash(hasher);
    for region in session.regions.iter().take(64) {
        region.base.as_ref().map(|text| text.len()).hash(hasher);
        region.ours.len().hash(hasher);
        region.theirs.len().hash(hasher);
    }
}

fn conflict_session_bytes(session: &ConflictSession) -> usize {
    session
        .base
        .byte_len()
        .unwrap_or_default()
        .saturating_add(session.ours.byte_len().unwrap_or_default())
        .saturating_add(session.theirs.byte_len().unwrap_or_default())
        .saturating_add(
            session
                .current
                .as_ref()
                .and_then(|payload| payload.byte_len())
                .unwrap_or_default(),
        )
}

fn hash_file_diff_window(
    split: &crate::view::panes::main::diff_cache::PagedFileDiffRows,
    inline: &crate::view::panes::main::diff_cache::PagedFileDiffInlineRows,
    window: usize,
    hasher: &mut FxHasher,
) {
    use gitcomet_core::domain::DiffRowProvider;

    let window = window.max(1);
    split.len_hint().hash(hasher);
    inline.len_hint().hash(hasher);
    for row in split.slice(0, window).take(window) {
        let kind_key: u8 = match row.kind {
            gitcomet_core::file_diff::FileDiffRowKind::Context => 0,
            gitcomet_core::file_diff::FileDiffRowKind::Add => 1,
            gitcomet_core::file_diff::FileDiffRowKind::Remove => 2,
            gitcomet_core::file_diff::FileDiffRowKind::Modify => 3,
        };
        kind_key.hash(hasher);
        row.old_line.hash(hasher);
        row.new_line.hash(hasher);
        row.old.as_ref().map(|text| text.len()).hash(hasher);
        row.new.as_ref().map(|text| text.len()).hash(hasher);
    }
    for line in inline.slice(0, window).take(window) {
        let kind_key: u8 = match line.kind {
            DiffLineKind::Header => 0,
            DiffLineKind::Hunk => 1,
            DiffLineKind::Add => 2,
            DiffLineKind::Remove => 3,
            DiffLineKind::Context => 4,
        };
        kind_key.hash(hasher);
        line.text.len().hash(hasher);
        line.old_line.hash(hasher);
        line.new_line.hash(hasher);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_repo_monorepo_fixture_reports_expected_metrics() {
        let snapshot_root = build_monorepo_snapshot_root();
        let fixture = RealRepoFixture::from_snapshot_root(
            snapshot_root.path(),
            RealRepoScenario::MonorepoOpenAndHistoryLoad,
        )
        .expect("real_repo monorepo fixture");

        let (hash, metrics) = fixture.run_with_metrics();
        assert_ne!(hash, 0);
        assert!(metrics.worktree_file_count >= 24);
        assert!(metrics.commits_loaded >= 8);
        assert!(metrics.graph_rows >= 8);
        assert!(metrics.status_calls >= 1);
        assert!(metrics.log_walk_calls >= 1);
    }

    #[test]
    fn real_repo_deep_history_fixture_scrolls_multiple_windows() {
        let snapshot_root = build_deep_history_snapshot_root();
        let fixture = RealRepoFixture::from_snapshot_root(
            snapshot_root.path(),
            RealRepoScenario::DeepHistoryOpenAndScroll,
        )
        .expect("real_repo deep history fixture");

        let (hash, metrics) = fixture.run_with_metrics();
        assert_ne!(hash, 0);
        assert_eq!(metrics.history_windows_scanned, 3);
        assert!(metrics.commits_loaded >= 40);
        assert!(metrics.graph_rows >= 40);
        assert!(metrics.log_walk_calls >= 1);
    }

    #[test]
    fn real_repo_conflict_fixture_loads_conflict_session() {
        let snapshot_root = build_conflict_snapshot_root();
        let fixture = RealRepoFixture::from_snapshot_root(
            snapshot_root.path(),
            RealRepoScenario::MidMergeConflictListAndOpen,
        )
        .expect("real_repo conflict fixture");

        let (hash, metrics) = fixture.run_with_metrics();
        assert_ne!(hash, 0);
        assert!(metrics.status_entries >= 1);
        assert!(metrics.conflict_files >= 1);
        assert!(metrics.conflict_regions >= 1);
        assert!(metrics.selected_conflict_bytes >= 1);
        assert!(metrics.status_calls >= 1);
    }

    #[test]
    fn real_repo_large_diff_fixture_paints_first_window() {
        let snapshot_root = build_large_diff_snapshot_root();
        let fixture = RealRepoFixture::from_snapshot_root(
            snapshot_root.path(),
            RealRepoScenario::LargeFileDiffOpen,
        )
        .expect("real_repo large diff fixture");

        let (hash, metrics) = fixture.run_with_metrics();
        assert_ne!(hash, 0);
        assert!(metrics.diff_lines >= 200);
        assert!(metrics.file_new_bytes > metrics.file_old_bytes);
        assert!(metrics.split_rows_painted >= 1);
        assert!(metrics.inline_rows_painted >= 1);
        assert!(metrics.diff_calls >= 1);
    }

    fn build_monorepo_snapshot_root() -> TempDir {
        let root = tempfile::tempdir().expect("snapshot root");
        let case_dir = root.path().join("monorepo_open_and_history_load");
        let source = build_repo_with_linear_history(root.path().join("monorepo-source"), 8, 24, 16);
        write_metadata(
            &case_dir,
            serde_json::json!({
                "source": source.display().to_string(),
                "history_limit": 8,
                "history_page_size": 4
            }),
        );
        root
    }

    fn build_deep_history_snapshot_root() -> TempDir {
        let root = tempfile::tempdir().expect("snapshot root");
        let case_dir = root.path().join("deep_history_open_and_scroll");
        let source =
            build_repo_with_linear_history(root.path().join("deep-history-source"), 48, 4, 4);
        write_metadata(
            &case_dir,
            serde_json::json!({
                "source": source.display().to_string(),
                "history_limit": 40,
                "history_page_size": 10,
                "history_window": 12
            }),
        );
        root
    }

    fn build_conflict_snapshot_root() -> TempDir {
        let root = tempfile::tempdir().expect("snapshot root");
        let case_dir = root.path().join("mid_merge_conflict_list_and_open");
        let source = root.path().join("conflict-source");
        fs::create_dir_all(&source).expect("create conflict repo");
        run_git(source.as_path(), &["init", "--quiet"]);
        configure_git_identity(&source);
        fs::write(source.join("src.txt"), "line 1\nshared\nline 3\n")
            .expect("write base conflict file");
        run_git(&source, &["add", "src.txt"]);
        run_git(&source, &["commit", "--quiet", "-m", "base"]);
        run_git(&source, &["checkout", "--quiet", "-b", "main"]);
        run_git(&source, &["checkout", "--quiet", "-b", "feature/conflict"]);
        fs::write(source.join("src.txt"), "line 1\nfeature change\nline 3\n")
            .expect("write feature conflict file");
        run_git(&source, &["commit", "--quiet", "-am", "feature change"]);
        run_git(&source, &["checkout", "--quiet", "main"]);
        fs::write(source.join("src.txt"), "line 1\nmain change\nline 3\n")
            .expect("write main conflict file");
        run_git(&source, &["commit", "--quiet", "-am", "main change"]);

        write_metadata(
            &case_dir,
            serde_json::json!({
                "source": source.display().to_string(),
                "checkout_ref": "main",
                "merge_ref": "feature/conflict",
                "conflict_path": "src.txt"
            }),
        );
        root
    }

    fn build_large_diff_snapshot_root() -> TempDir {
        let root = tempfile::tempdir().expect("snapshot root");
        let case_dir = root.path().join("large_file_diff_open");
        let source = root.path().join("large-diff-source");
        fs::create_dir_all(&source).expect("create large diff repo");
        run_git(source.as_path(), &["init", "--quiet"]);
        configure_git_identity(&source);
        let file_path = source.join("generated.txt");
        fs::write(&file_path, synthetic_text_lines(256, "old")).expect("write old diff file");
        run_git(&source, &["add", "generated.txt"]);
        run_git(&source, &["commit", "--quiet", "-m", "base large file"]);
        fs::write(&file_path, synthetic_text_lines(320, "new")).expect("write new diff file");
        run_git(&source, &["commit", "--quiet", "-am", "update large file"]);

        write_metadata(
            &case_dir,
            serde_json::json!({
                "source": source.display().to_string(),
                "diff_commitish": "HEAD",
                "diff_path": "generated.txt"
            }),
        );
        root
    }

    fn build_repo_with_linear_history(
        path: PathBuf,
        commits: usize,
        files: usize,
        line_count: usize,
    ) -> PathBuf {
        fs::create_dir_all(&path).expect("create repo");
        run_git(path.as_path(), &["init", "--quiet"]);
        configure_git_identity(&path);

        for file_ix in 0..files {
            let rel_path = path.join(format!("src/module_{file_ix}/file_{file_ix}.txt"));
            fs::create_dir_all(rel_path.parent().expect("parent")).expect("mkdirs");
            fs::write(&rel_path, synthetic_text_lines(line_count, "seed"))
                .expect("write seed file");
        }
        run_git(&path, &["add", "."]);
        run_git(&path, &["commit", "--quiet", "-m", "initial"]);

        for commit_ix in 0..commits.saturating_sub(1) {
            let rel_path = path.join(format!(
                "src/module_{}/file_{}.txt",
                commit_ix % files,
                commit_ix % files
            ));
            fs::write(
                &rel_path,
                synthetic_text_lines(
                    line_count.saturating_add(commit_ix % 8),
                    &format!("commit-{commit_ix}"),
                ),
            )
            .expect("update commit file");
            run_git(
                &path,
                &["commit", "--quiet", "-am", &format!("commit {commit_ix}")],
            );
        }

        path
    }

    fn synthetic_text_lines(lines: usize, label: &str) -> String {
        let mut out = String::with_capacity(lines.saturating_mul(32));
        for ix in 0..lines {
            out.push_str(&format!("{label} line {ix}\n"));
        }
        out
    }

    fn write_metadata(case_dir: &Path, json: serde_json::Value) {
        fs::create_dir_all(case_dir).expect("create case dir");
        fs::write(
            case_dir.join("metadata.json"),
            serde_json::to_vec_pretty(&json).expect("metadata json"),
        )
        .expect("write metadata");
    }
}
