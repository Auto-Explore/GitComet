use super::*;

const FILE_TEXT: &str = "alpha\nbeta\ngamma\n";

fn head_commit(workdir: &Path) -> CommitId {
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(workdir)
        .args(["rev-parse", "HEAD"])
        .output()
        .expect("git rev-parse runs");
    assert!(output.status.success(), "git rev-parse HEAD failed");
    CommitId(String::from_utf8_lossy(&output.stdout).trim().into())
}

fn repository_with_a_rename_only_commit() -> (tempfile::TempDir, CommitId) {
    let dir = tempfile::tempdir().expect("tempdir");
    let workdir = dir.path();
    run_git(workdir, &["init", "-q"]);
    run_git(workdir, &["config", "commit.gpgsign", "false"]);
    run_git(workdir, &["config", "user.name", "Test User"]);
    run_git(workdir, &["config", "user.email", "test@example.com"]);
    std::fs::write(workdir.join("old_name.txt"), FILE_TEXT).expect("write the file");
    run_git(workdir, &["add", "old_name.txt"]);
    run_git(workdir, &["commit", "-q", "-m", "base"]);
    run_git(workdir, &["mv", "old_name.txt", "new_name.txt"]);
    run_git(workdir, &["commit", "-q", "-m", "rename"]);
    let rename = head_commit(workdir);
    (dir, rename)
}

fn show_file_changes_at_commit(repo: Arc<dyn GitRepository>, commit_id: CommitId) -> Msg {
    let repo_id = RepoId(1);
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let mut repos = RepositoryRuntimes::default();
    repos.attach_repository_for_test(repo_id, repo);
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &mut repos,
        msg_tx,
        Effect::OpenFileAtCommit {
            repo_id,
            commit_id,
            path: PathBuf::from("new_name.txt"),
            content_preview: false,
        },
    );
    recv_effect_message(&msg_rx, Duration::from_secs(10))
        .expect("the effect answers with the view to open")
}

#[test]
fn showing_the_changes_of_a_rename_commit_compares_against_the_old_name() {
    let (dir, rename) = repository_with_a_rename_only_commit();

    let repo = gitcomet_git_gix::GixBackend
        .open(dir.path())
        .expect("open the repository");

    let message = show_file_changes_at_commit(repo, rename.clone());

    let Msg::SelectDiff { target, .. } = message else {
        panic!("expected SelectDiff, got {message:?}");
    };
    assert_eq!(
        target,
        DiffTarget::commit(rename, PathBuf::from("new_name.txt"))
    );
    assert_eq!(target.old_file_path(), Some(Path::new("old_name.txt")));
}

#[test]
fn a_failed_rename_lookup_still_shows_the_changes_under_the_given_name() {
    let commit = CommitId("abcdef".into());
    let repo = Arc::new(UnsupportedRepo {
        spec: RepoSpec {
            workdir: PathBuf::from("/tmp/open-file-at-commit"),
        },
        delete_branch_calls: None,
        cancel_delete_branch: None,
    });

    let message = show_file_changes_at_commit(repo, commit.clone());

    let Msg::SelectDiff { target, .. } = message else {
        panic!("expected SelectDiff, got {message:?}");
    };
    assert_eq!(
        target,
        DiffTarget::commit(commit, PathBuf::from("new_name.txt"))
    );
    assert_eq!(target.old_file_path(), None);
}
