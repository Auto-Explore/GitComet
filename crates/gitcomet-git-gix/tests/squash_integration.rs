use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

fn run_git(repo: &Path, args: &[&str]) {
    run_git_with_env(repo, args, &[]);
}

fn run_git_with_env(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let cmd = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true");
    for (key, value) in envs {
        cmd.env(key, value);
    }
    let status = cmd.status().expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .expect("git command to run");
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

fn init_repo(repo: &Path) {
    fs::create_dir_all(repo).expect("create repo directory");
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
}

fn commit_file(repo: &Path, name: &str, content: &str, message: &str) {
    commit_file_with_env(repo, name, content, message, &[]);
}

fn commit_file_with_env(
    repo: &Path,
    name: &str,
    content: &str,
    message: &str,
    envs: &[(&str, &str)],
) {
    fs::write(repo.join(name), content).expect("write file");
    run_git(repo, &["add", "."]);
    run_git_with_env(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
        envs,
    );
}

fn rev_parse(repo: &Path, spec: &str) -> String {
    git_stdout(repo, &["rev-parse", spec])
}

fn commit_id(sha: &str) -> CommitId {
    CommitId(sha.into())
}

fn open_backend(repo: &Path) -> Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

/// root <- a <- b <- c <- d(HEAD); returns (root, a, b, c, d) shas.
fn linear_repo(repo: &Path) -> (String, String, String, String, String) {
    init_repo(repo);
    commit_file(repo, "file.txt", "root\n", "Root commit");
    let root = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "a\n", "Commit A");
    let a = rev_parse(repo, "HEAD");
    commit_file_with_env(
        repo,
        "file.txt",
        "b\n",
        "Commit B\n\nBody of B",
        &[
            ("GIT_AUTHOR_NAME", "Oldest Author"),
            ("GIT_AUTHOR_EMAIL", "oldest@example.com"),
            ("GIT_AUTHOR_DATE", "1600000000 +0230"),
        ],
    );
    let b = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "c\n", "Commit C");
    let c = rev_parse(repo, "HEAD");
    commit_file(repo, "file.txt", "d\n", "Commit D");
    let d = rev_parse(repo, "HEAD");
    (root, a, b, c, d)
}

#[test]
fn squash_replaces_linear_range_with_single_commit() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, a, b, _c, d) = linear_repo(&repo);
    let tree_before = rev_parse(&repo, "HEAD^{tree}");

    let backend = open_backend(&repo);
    backend
        .squash_commits_with_output(
            &commit_id(&b),
            &commit_id(&d),
            "Squashed message\n\nDetails",
        )
        .expect("squash commits");

    let head = rev_parse(&repo, "HEAD");
    assert_ne!(head, d, "HEAD must move to the squash commit");
    assert_eq!(rev_parse(&repo, "HEAD^{tree}"), tree_before);
    assert_eq!(rev_parse(&repo, "HEAD^"), a);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Squashed message\n\nDetails"
    );
    // Author is preserved from the oldest squashed commit; committer is fresh.
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%an|%ae|%ad", "--date=raw"]),
        "Oldest Author|oldest@example.com|1600000000 +0230"
    );
    assert_eq!(git_stdout(&repo, &["log", "-1", "--format=%cn"]), "You");
    // Exactly root, a, squash remain.
    assert_eq!(
        git_stdout(&repo, &["rev-list", "--count", "HEAD"]),
        "3".to_string()
    );
    // The reflog records the squash.
    assert!(
        git_stdout(&repo, &["reflog", "-1"]).contains("squash: 3 commits"),
        "reflog should mention the squash"
    );
}

#[test]
fn squash_leaves_dirty_worktree_and_index_untouched() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);

    fs::write(repo.join("staged.txt"), "staged\n").expect("write staged file");
    run_git(&repo, &["add", "staged.txt"]);
    fs::write(repo.join("file.txt"), "dirty\n").expect("dirty the worktree");

    let backend = open_backend(&repo);
    backend
        .squash_commits_with_output(&commit_id(&b), &commit_id(&d), "Squash")
        .expect("squash commits");

    // Note: git_stdout trims the output, which strips porcelain's leading
    // space from the first line; match without the column prefix.
    let status = git_stdout(&repo, &["status", "--porcelain"]);
    assert!(
        status.contains("A  staged.txt"),
        "staged file kept: {status}"
    );
    assert!(status.contains("M file.txt"), "dirty file kept: {status}");
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "dirty\n"
    );
}

#[test]
fn squash_works_on_detached_head_and_moves_branch_when_attached() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);
    let branch = git_stdout(&repo, &["branch", "--show-current"]);

    // Attached: the branch ref moves.
    let backend = open_backend(&repo);
    backend
        .squash_commits_with_output(&commit_id(&b), &commit_id(&d), "Attached squash")
        .expect("squash commits");
    let squashed_head = rev_parse(&repo, "HEAD");
    assert_eq!(
        rev_parse(&repo, &format!("refs/heads/{branch}")),
        squashed_head
    );

    // Detached: HEAD itself moves, the branch stays.
    commit_file(&repo, "file.txt", "e\n", "Commit E");
    let e = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "f\n", "Commit F");
    let f = rev_parse(&repo, "HEAD");
    let branch_tip = rev_parse(&repo, &format!("refs/heads/{branch}"));
    run_git(&repo, &["checkout", "--detach"]);
    backend
        .squash_commits_with_output(&commit_id(&e), &commit_id(&f), "Detached squash")
        .expect("squash on detached HEAD");
    assert_eq!(
        rev_parse(&repo, "HEAD^"),
        squashed_head,
        "squash parent is the pre-range tip"
    );
    assert_eq!(
        rev_parse(&repo, &format!("refs/heads/{branch}")),
        branch_tip,
        "branch must not move while detached"
    );
}

#[test]
fn squash_with_stale_expected_head_fails_without_changing_refs() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, c, d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    // Pretend the squash was prepared when `c` was HEAD.
    let err = backend
        .squash_commits_with_output(&commit_id(&b), &commit_id(&c), "Stale")
        .expect_err("stale expected head must fail");
    assert!(
        err.to_string().contains("HEAD moved"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), d, "refs must be unchanged");
}

#[test]
fn squash_rejects_merge_commit_in_range() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "base.txt", "base\n", "Base");
    let base = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-b", "feature"]);
    commit_file(&repo, "feature.txt", "feature\n", "Feature");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "main.txt", "main\n", "Main work");
    let main_work = rev_parse(&repo, "HEAD");
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-ff",
            "-m",
            "Merge feature",
            "feature",
        ],
    );
    let merge = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .squash_commits_with_output(&commit_id(&main_work), &commit_id(&merge), "Nope")
        .expect_err("merge commit in range must fail");
    assert!(
        err.to_string().contains("exactly one parent"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), merge);
    let _ = base;
}

#[test]
fn squash_rejects_root_commit_as_oldest() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    let root = rev_parse(&repo, "HEAD");
    commit_file(&repo, "file.txt", "next\n", "Next");
    let next = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .squash_commits_with_output(&commit_id(&root), &commit_id(&next), "Nope")
        .expect_err("root commit as oldest must fail");
    assert!(
        err.to_string().contains("exactly one parent"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), next);
}

#[test]
fn squash_rejects_non_ancestor_oldest() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    init_repo(&repo);
    commit_file(&repo, "file.txt", "root\n", "Root");
    run_git(&repo, &["checkout", "-b", "side"]);
    commit_file(&repo, "side.txt", "side\n", "Side");
    let side = rev_parse(&repo, "HEAD");
    run_git(&repo, &["checkout", "-"]);
    commit_file(&repo, "file.txt", "main\n", "Main");
    let head = rev_parse(&repo, "HEAD");

    let backend = open_backend(&repo);
    let err = backend
        .squash_commits_with_output(&commit_id(&side), &commit_id(&head), "Nope")
        .expect_err("non-ancestor oldest must fail");
    assert!(
        err.to_string().contains("exactly one parent"),
        "unexpected error: {err}"
    );
    assert_eq!(rev_parse(&repo, "HEAD"), head);
}

#[test]
fn squash_message_preview_combines_messages_oldest_first() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = dir.path().join("repo");
    let (_root, _a, b, _c, d) = linear_repo(&repo);

    let backend = open_backend(&repo);
    let preview = backend
        .squash_message_preview(&commit_id(&b), &commit_id(&d))
        .expect("build message preview");
    assert_eq!(preview, "Commit B\n\nBody of B\n\nCommit C\n\nCommit D");
}
