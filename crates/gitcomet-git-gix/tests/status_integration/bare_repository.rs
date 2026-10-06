use super::*;
use gitcomet_core::domain::{HistoryMode, RepoStatus};

fn init_bare(repo: &Path, populated: bool) {
    run_git(repo, &["init", "--bare", "-b", "main"]);
    if populated {
        import_linear_history(
            git_command().arg("-C").arg(repo),
            "main",
            [("first", "one\n"), ("second", "two\n")]
                .into_iter()
                .enumerate()
                .map(|(index, (message, contents))| LinearCommit {
                    author: "You <you@example.com>",
                    timestamp: 1_600_000_000 + index as i64,
                    message,
                    path: "file.txt",
                    contents,
                }),
        );
    }
}

#[test]
fn bare_repository_status_refreshes_are_empty_and_history_remains_readable() {
    for populated in [false, true] {
        // No .git suffix: detect the repository's actual worktree capability.
        let dir = tempfile::tempdir().unwrap();
        init_bare(dir.path(), populated);
        let repo = GixBackend.open(dir.path()).unwrap();
        let cancel = CancellationToken::new();

        for _ in 0..2 {
            assert_eq!(repo.status().unwrap(), RepoStatus::default());
            assert_eq!(
                repo.status_cancellable(&cancel).unwrap(),
                RepoStatus::default()
            );
            assert!(repo.worktree_status().unwrap().is_empty());
            assert!(
                repo.worktree_status_cancellable(&cancel)
                    .unwrap()
                    .is_empty()
            );
            assert!(repo.staged_status().unwrap().is_empty());
            assert!(repo.staged_status_cancellable(&cancel).unwrap().is_empty());
        }

        let page = repo.log_all_branches_page(20, None).unwrap();
        let index = repo
            .build_history_index(HistoryMode::AllBranches, None, &cancel, &mut |_| {})
            .unwrap()
            .unwrap();
        assert_eq!(page.commits.len(), if populated { 2 } else { 0 });
        assert_eq!(index.len(), page.commits.len());
        assert_eq!(
            repo.read_history_range(&index, 0..index.len(), &cancel)
                .unwrap()
                .commits,
            page.commits
        );
        assert!(
            !dir.path().join("index").exists(),
            "reading a bare repo must not create an index"
        );
        assert!(
            !dir.path().join("file.txt").exists(),
            "reading history must not check out files"
        );

        cancel.cancel();
        assert!(matches!(
            repo.status_cancellable(&cancel).unwrap_err().kind(),
            ErrorKind::Cancelled
        ));
        assert!(matches!(
            repo.worktree_status_cancellable(&cancel)
                .unwrap_err()
                .kind(),
            ErrorKind::Cancelled
        ));
        assert!(matches!(
            repo.staged_status_cancellable(&cancel).unwrap_err().kind(),
            ErrorKind::Cancelled
        ));
    }
}

#[test]
fn linked_worktree_of_bare_repository_still_reports_staged_and_unstaged_changes() {
    let dir = tempfile::tempdir().unwrap();
    let bare = dir.path().join("origin.git");
    let worktree = dir.path().join("checkout");
    fs::create_dir(&bare).unwrap();
    init_bare(&bare, true);
    run_git(
        &bare,
        &["worktree", "add", "-b", "feature", &git_path_arg(&worktree)],
    );
    assert_eq!(run_git_output(&bare, &["config", "core.bare"]), "true");

    write(&worktree, "file.txt", "staged\n");
    run_git(&worktree, &["add", "file.txt"]);
    write(&worktree, "file.txt", "staged\nunstaged\n");
    write(&worktree, "untracked.txt", "new\n");

    let repo = GixBackend.open(&worktree).unwrap();
    let status = repo.status().unwrap();
    assert_eq!(status.staged.len(), 1);
    assert_eq!(status.staged[0].path, Path::new("file.txt"));
    assert_eq!(status.staged[0].kind, FileStatusKind::Modified);
    assert_eq!(status.unstaged.len(), 2);
    assert!(
        status
            .unstaged
            .iter()
            .any(|entry| entry.path == Path::new("file.txt")
                && entry.kind == FileStatusKind::Modified)
    );
    assert!(
        status
            .unstaged
            .iter()
            .any(|entry| entry.path == Path::new("untracked.txt")
                && entry.kind == FileStatusKind::Untracked)
    );
    assert_eq!(
        repo.staged_status().unwrap().as_slice(),
        status.staged.as_slice()
    );
    assert_eq!(
        repo.worktree_status().unwrap().as_slice(),
        status.unstaged.as_slice()
    );
    assert_eq!(
        GixBackend.open(&bare).unwrap().status().unwrap(),
        RepoStatus::default()
    );
}
