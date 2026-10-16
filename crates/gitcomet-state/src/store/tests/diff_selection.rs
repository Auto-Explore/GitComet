use super::*;
use crate::msg::RepoCommandEffect;
use crate::store::repository_runtime::RepositoryRuntimes;
use gitcomet_core::domain::{
    CommitDetails, CommitFileChange, CommitId, FileConflictKind, FileDiffImage, FileSource,
    FileStatus, FileStatusKind, Submodule, SubmoduleDiffSummary, SubmoduleDiffSummaryMode,
    SubmoduleInnerChange, SubmoduleStatus,
};

fn fake_gitlink_id(repo: &Path) -> String {
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--show-object-format"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let width = match output.stdout.trim_ascii() {
        b"sha1" => 40,
        b"sha256" => 64,
        other => panic!("unknown format: {other:?}"),
    };
    "1".repeat(width)
}

fn staged_deleted_gitlink_fixture() -> (
    tempfile::TempDir,
    RepositoryRuntimes,
    AppState,
    gitcomet_core::domain::DiffTarget,
) {
    let dir = tempfile::tempdir().expect("tempdir");
    init_test_repository(dir.path());
    run_git(
        dir.path(),
        &[
            "update-index",
            "--add",
            "--cacheinfo",
            &format!("160000,{},vendor/submodule", fake_gitlink_id(dir.path())),
        ],
    );
    run_git(dir.path(), &["commit", "-q", "-m", "add submodule gitlink"]);

    let mut repos: RepositoryRuntimes = Default::default();
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: dir.path().to_path_buf(),
        },
    );
    let submodule_path = PathBuf::from("vendor/submodule");
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        submodule_path.clone(),
        gitcomet_core::domain::DiffArea::Staged,
    );
    repo_state.set_open(Loadable::Ready(()));
    repo_state.set_status(Loadable::Ready(Arc::new(RepoStatus {
        staged: std::sync::Arc::new(vec![FileStatus {
            path: submodule_path,
            kind: FileStatusKind::Deleted,
            conflict: None,
        }]),
        unstaged: std::sync::Arc::new(vec![]),
    })));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));

    let backend = gitcomet_git_gix::GixBackend;
    repos.attach_repository_for_test(
        RepoId(1),
        backend
            .open(dir.path())
            .expect("open repository through backend"),
    );

    (dir, repos, state, target)
}

fn init_test_repository(path: &Path) {
    run_git(path, &["init", "-q"]);
    run_git(path, &["config", "commit.gpgsign", "false"]);
    run_git(path, &["config", "user.name", "Test User"]);
    run_git(path, &["config", "user.email", "test@example.com"]);
}

fn status_action_state(target: DiffTarget) -> AppState {
    let mut state = AppState::test_default();
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: "/tmp/repo".into(),
        },
    );
    repo.open = Loadable::Ready(());
    repo.set_diff_target(Some(target.clone()));
    repo.diff_state.diff = Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
        target,
        lines: vec![],
    }));
    state.repos.push(repo);
    state.active_repo = Some(RepoId(1));
    state
}

fn finish_status_action(
    state: &mut AppState,
    repo_id: RepoId,
    action: RepoActionKind,
    paths: Vec<PathBuf>,
    succeeds: bool,
) {
    reduce(
        &mut Default::default(),
        &AtomicU64::new(3),
        state,
        Msg::Internal(crate::msg::InternalMsg::RepoPathsActionFinished {
            repo_id,
            action,
            paths: paths.into(),
            result: if succeeds {
                Ok(())
            } else {
                Err(Error::new(ErrorKind::Unsupported("index locked")))
            },
        }),
    );
}

// --- Revision counter regression tests ---

/// A repo with a working-tree diff and blame both loaded, ready to be fed a
/// `DiffLoaded` / `DiffFileLoaded` result.
fn state_with_loaded_diff_and_blame(
    diff: gitcomet_core::domain::Diff,
) -> (
    AppState,
    DiffTarget,
    Arc<Vec<gitcomet_core::services::BlameLine>>,
) {
    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    let target = diff.target.clone();
    repo_state.diff_state.diff_target = Some(target.clone());
    repo_state.diff_state.diff = Loadable::Ready(Arc::new(diff));
    let blame = Arc::new(vec![gitcomet_core::services::BlameLine {
        commit_id: Arc::from("1111111111111111111111111111111111111111"),
        author: Arc::from("Ada"),
        author_time_unix: Some(1_700_000_000),
        summary: Arc::from("initial"),
        body: None,
        line: "let x = 1;".to_string(),
        prior_exists: true,
        source_path: None,
        prior_commit: None,
    }]);
    repo_state.diff_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    repo_state.diff_state.blame_source = Some(gitcomet_core::domain::BlameSource::WorkingTree(
        gitcomet_core::domain::DiffArea::Unstaged,
    ));
    repo_state.diff_state.blame = Loadable::Ready(Arc::clone(&blame));
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    (state, target, blame)
}

fn unstaged_diff(line: &str) -> gitcomet_core::domain::Diff {
    gitcomet_core::domain::Diff {
        target: DiffTarget::working_tree(
            PathBuf::from("src/lib.rs"),
            gitcomet_core::domain::DiffArea::Unstaged,
        ),
        lines: vec![gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Context,
            text: line.into(),
        }],
    }
}

fn file_diff_text(line: &str) -> gitcomet_core::domain::FileDiffText {
    gitcomet_core::domain::FileDiffText::new(
        PathBuf::from("icon.svg"),
        None,
        Some(line.to_string()),
    )
}

/// A repo showing a file-text view (`load_file_text`, not `load_patch_diff`)
/// with blame loaded — the shape that makes `diff_file` the only content signal.
fn state_with_loaded_file_text_and_blame(
    text: gitcomet_core::domain::FileDiffText,
) -> (
    AppState,
    DiffTarget,
    Arc<Vec<gitcomet_core::services::BlameLine>>,
) {
    let target = DiffTarget::working_tree(PathBuf::from("icon.svg"), DiffArea::Unstaged);
    let (mut state, _, blame) = state_with_loaded_diff_and_blame(gitcomet_core::domain::Diff {
        target: target.clone(),
        lines: vec![],
    });
    state.repos[0].diff_state.diff = Loadable::NotLoaded;
    state.repos[0].diff_state.diff_file = Loadable::Ready(Some(Arc::new(text)));
    (state, target, blame)
}

fn worktree_dirty_summary(
    files: &[&str],
    branch: &str,
) -> gitcomet_core::domain::WorktreeDirtySummary {
    use gitcomet_core::domain::{FileStatus, WorktreeDirtySummary};

    WorktreeDirtySummary {
        path: PathBuf::from("/tmp/wt/a"),
        head: Some(CommitId("tip".into())),
        branch: Some(branch.to_string()),
        detached: false,
        added: 0,
        modified: files.len(),
        deleted: 0,
        staged: Vec::new(),
        unstaged: files
            .iter()
            .map(|name| FileStatus {
                path: PathBuf::from(name),
                kind: FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
        line_stats: Default::default(),
    }
}

/// A repo with one linked worktree, its row selected and one of its files open
/// in the inline diff.
fn worktree_inline_diff_fixture(
    files: &[&str],
    selected_ix: usize,
    branch: &str,
) -> (AppState, PathBuf) {
    let worktree = PathBuf::from("/tmp/wt/a");
    let summary = worktree_dirty_summary(files, branch);
    let entries = crate::model::worktree_inline_diff_entries(&summary);
    let target = entries[selected_ix].target.clone();

    let mut state = AppState::test_default();
    let mut repo_state = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.set_worktree_dirty(Loadable::Ready(vec![summary]));
    repo_state.history_state.worktree_selection = Some(worktree.clone());
    repo_state.diff_state.inline_submodule_diff = Some(crate::model::InlineSubmoduleDiffState {
        origin: crate::model::ForeignDiffOrigin::Worktree {
            branch: Some(branch.to_string()),
            detached: false,
        },
        submodule_repo_path: worktree.clone(),
        parent_submodule_path: worktree.clone(),
        entries: entries.into(),
        selected_ix,
        target,
        rev: 1,
        diff_rev: 1,
        diff: Loadable::NotLoaded,
        diff_file_rev: 1,
        diff_file: Loadable::NotLoaded,
        diff_file_image: Loadable::NotLoaded,
    });
    state.repos.push(repo_state);
    state.active_repo = Some(RepoId(1));
    (state, worktree)
}

mod text_override;

mod completion_identities;
mod editor_navigation;
mod gitlinks;
mod preview_loading;
mod submodules;
mod worktree_navigation;
