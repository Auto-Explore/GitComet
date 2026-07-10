use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{
    GitBackend, GitRepository, InteractiveRebaseAction, InteractiveRebaseEntry, SequencerState,
};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let status = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(repo: &Path, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .output()
        .expect("git command to run")
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
}

fn commit_file(repo: &Path, name: &str, content: &str, message: &str) -> String {
    fs::write(repo.join(name), content).expect("write file");
    run_git(repo, &["add", "."]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn commit_id(sha: &str) -> CommitId {
    CommitId(sha.into())
}

fn open_backend(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

#[test]
fn single_cherry_pick_with_commit_creates_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "main.txt", "main\n", "main change");
    let before_count = git_stdout(&repo, &["rev-list", "--count", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&picked), true)
        .expect("cherry-pick");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        (before_count.parse::<u32>().unwrap() + 1).to_string()
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "feature change"
    );
    assert_eq!(
        fs::read_to_string(repo.join("feature.txt")).unwrap(),
        "feature\n"
    );
}

#[test]
fn single_cherry_pick_without_commit_applies_index_and_worktree_only() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&picked), false)
        .expect("cherry-pick --no-commit");

    assert_eq!(output.exit_code, Some(0));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(
        git_stdout(&repo, &["status", "--porcelain"]),
        "A  feature.txt"
    );
    assert_eq!(
        fs::read_to_string(repo.join("feature.txt")).unwrap(),
        "feature\n"
    );
}

#[test]
fn already_applied_cherry_pick_is_successful_noop_and_cleans_state() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "old\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "file.txt", "new\n", "same change");
    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "new\n", "same change independently");
    let before_head = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let output = open_backend(&repo)
        .cherry_pick_with_output(&commit_id(&picked), true)
        .expect("already-applied cherry-pick");

    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stdout
            .contains("GITCOMET_CHERRY_PICK_ALREADY_APPLIED")
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), before_head);
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "");
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert!(
        !git_output(&repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success()
    );
}

#[test]
fn interactive_reword_without_changed_message_uses_original_message() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "base");
    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "feature.txt", "feature\n", "feature change");
    run_git(&repo, &["checkout", "main"]);

    open_backend(&repo)
        .interactive_cherry_pick_with_output(&[InteractiveRebaseEntry {
            action: InteractiveRebaseAction::Reword,
            commit_id: picked,
            summary: "feature change".to_string(),
            message: "feature change".to_string(),
            new_message: None,
        }])
        .expect("reword without edited message should use original message");

    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%s"]),
        "feature change"
    );
}

fn setup_conflicting_cherry_pick_repo(repo: &Path) -> String {
    init_repo(repo);
    commit_file(repo, "file.txt", "base\n", "base");
    run_git(repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(repo, "file.txt", "feature\n", "feature change");
    run_git(repo, &["checkout", "main"]);
    picked
}

#[test]
fn conflicting_cherry_pick_returns_error_and_leaves_worktree_conflicted() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let err = open_backend(&repo)
        .cherry_pick(&commit_id(&picked))
        .expect_err("conflicting cherry-pick should fail");

    let message = err.to_string();
    assert!(
        message.contains("could not apply") || message.contains("CONFLICT"),
        "unexpected conflict error: {message}"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "UU file.txt");
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );
    assert!(
        git_output(&repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success()
    );
}

#[test]
fn dirty_worktree_rejects_cherry_pick_and_preserves_local_change() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    fs::write(repo.join("file.txt"), "dirty worktree\n").expect("write dirty worktree");

    let err = open_backend(&repo)
        .cherry_pick(&commit_id(&picked))
        .expect_err("dirty worktree should reject cherry-pick");

    let message = err.to_string();
    assert!(
        message.contains("local changes") || message.contains("would be overwritten"),
        "unexpected dirty-worktree error: {message}"
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "dirty worktree\n"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "M file.txt");
}

#[test]
fn dirty_index_rejects_cherry_pick_and_preserves_staged_change() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let picked = setup_conflicting_cherry_pick_repo(&repo);
    fs::write(repo.join("file.txt"), "staged change\n").expect("write staged change");
    run_git(&repo, &["add", "file.txt"]);

    let err = open_backend(&repo)
        .cherry_pick(&commit_id(&picked))
        .expect_err("dirty index should reject cherry-pick");

    let message = err.to_string();
    assert!(
        message.contains("local changes") || message.contains("would be overwritten"),
        "unexpected dirty-index error: {message}"
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "staged change\n"
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), "M  file.txt");
}

#[test]
fn continue_falls_back_to_cherry_pick_continue_when_cherry_pick_is_paused() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "base\n", "base");

    run_git(&repo, &["checkout", "-b", "feature"]);
    let picked = commit_file(&repo, "file.txt", "feature\n", "feature change");

    run_git(&repo, &["checkout", "main"]);
    commit_file(&repo, "file.txt", "main\n", "main change");

    let conflict = git_output(&repo, &["cherry-pick", &picked]);
    assert!(
        !conflict.status.success(),
        "cherry-pick should pause at a conflict"
    );
    assert!(open_backend(&repo).rebase_in_progress().unwrap());
    assert_eq!(
        open_backend(&repo).sequencer_state().unwrap(),
        SequencerState::CherryPick
    );

    fs::write(repo.join("file.txt"), "resolved\n").expect("resolve conflict");
    run_git(&repo, &["add", "file.txt"]);

    let output = open_backend(&repo)
        .rebase_continue_with_output()
        .expect("continue paused cherry-pick");
    assert_eq!(output.command, "git cherry-pick --continue");
    assert!(!open_backend(&repo).rebase_in_progress().unwrap());
    assert!(
        !git_output(&repo, &["rev-parse", "-q", "--verify", "CHERRY_PICK_HEAD"])
            .status
            .success()
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "resolved\n"
    );
}
