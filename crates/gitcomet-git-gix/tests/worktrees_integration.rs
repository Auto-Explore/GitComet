use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;

fn git_command() -> Command {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd
}

fn run_git(repo: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn same_path(lhs: &Path, rhs: &Path) -> bool {
    canonicalize_or_original(lhs.to_path_buf()) == canonicalize_or_original(rhs.to_path_buf())
}

#[test]
fn worktree_add_list_remove_round_trip() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();
    let repo = root.join("repo");
    fs::create_dir_all(&repo).expect("create repo directory");

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);
    run_git(&repo, &["config", "core.autocrlf", "false"]);
    run_git(&repo, &["config", "core.eol", "lf"]);

    fs::write(repo.join("seed.txt"), "seed\n").expect("write seed file");
    run_git(&repo, &["add", "seed.txt"]);
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "seed"],
    );

    let backend = GixBackend;
    let opened = backend.open(&repo).expect("open repository");

    let before = opened.list_worktrees().expect("list initial worktrees");
    let primary = before
        .iter()
        .find(|worktree| same_path(&worktree.path, &repo))
        .expect("primary worktree should be listed");
    assert!(primary.head.is_some());
    assert!(!primary.detached);
    if let Some(branch) = primary.branch.as_deref() {
        assert!(
            !branch.starts_with("refs/heads/"),
            "branch name should be normalized: {branch}"
        );
    }

    #[cfg(unix)]
    let linked_path = root.join("linked\ntree");
    #[cfg(not(unix))]
    let linked_path = root.join("linked tree");
    let add_output = opened
        .add_worktree_with_output(&linked_path, Some("--detach"))
        .expect("add linked worktree");
    assert_eq!(add_output.exit_code, Some(0));

    let listed = opened.list_worktrees().expect("list worktrees after add");
    let linked = listed
        .iter()
        .find(|worktree| same_path(&worktree.path, &linked_path))
        .expect("linked worktree should be listed");
    assert!(linked.detached);
    assert!(linked.branch.is_none());
    assert!(linked.head.is_some());

    let remove_output = opened
        .remove_worktree_with_output(&linked_path)
        .expect("remove linked worktree");
    assert_eq!(remove_output.exit_code, Some(0));

    let after = opened
        .list_worktrees()
        .expect("list worktrees after remove");
    assert!(
        after
            .iter()
            .all(|worktree| !same_path(&worktree.path, &linked_path)),
        "linked worktree should be removed"
    );
}
