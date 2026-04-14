use super::*;

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
                let status = load_split_repo_status(self.repo.as_ref(), "git_ops status benchmark");
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

pub(crate) fn hash_repo_status(status: &RepoStatus) -> u64 {
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

pub(crate) fn hash_parsed_diff(diff: &Diff) -> u64 {
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

pub(crate) fn build_git_ops_status_repo(tracked_files: usize, dirty_files: usize) -> TempDir {
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

pub(crate) fn git_ops_status_relative_path(index: usize) -> std::path::PathBuf {
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

pub(crate) fn run_git(repo: &Path, args: &[&str]) {
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

pub(crate) fn git_stdout(repo: &Path, args: &[&str]) -> String {
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

pub(crate) fn git_command(repo: &Path) -> Command {
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
