use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
#[path = "support/test_git_env.rs"]
mod test_git_env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn run_git_allow_file(repo: &Path, args: &[&str]) {
    let status = git_command()
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command with file protocol to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn git_output(repo: &Path, args: &[&str]) -> Output {
    git_command()
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run")
}

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let output = git_output(repo, args);
    assert!(output.status.success(), "git {:?} failed", args);
    String::from_utf8(output.stdout)
        .expect("git stdout is utf-8")
        .trim()
        .to_string()
}

fn init_repo_with_seed(repo: &Path, file: &str, contents: &str, message: &str) {
    run_git(repo, &["init"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);
    run_git(repo, &["config", "core.autocrlf", "false"]);
    run_git(repo, &["config", "core.eol", "lf"]);

    write_and_commit(repo, file, contents, message);
    run_git(repo, &["branch", "-M", "main"]);
}

fn init_bare_repo(repo: &Path) {
    let status = git_command()
        .arg("init")
        .arg("--bare")
        .arg(repo)
        .status()
        .expect("git init --bare to run");
    assert!(status.success(), "git init --bare failed");
}

fn write_and_commit(repo: &Path, file: &str, contents: &str, message: &str) {
    fs::write(repo.join(file), contents).expect("write file");
    run_git(repo, &["add", file]);
    run_git(
        repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", message],
    );
}

#[test]
fn list_subtrees_discovers_squash_and_full_history_without_metadata() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let upstream_full = root.join("upstream-full");
    let upstream_squash = root.join("upstream-squash");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&upstream_full).expect("create upstream-full");
    fs::create_dir_all(&upstream_squash).expect("create upstream-squash");
    fs::create_dir_all(&parent_repo).expect("create parent");

    init_repo_with_seed(&upstream_full, "lib.txt", "full\n", "seed full history subtree");
    init_repo_with_seed(
        &upstream_squash,
        "lib.txt",
        "squash\n",
        "seed squash subtree",
    );
    init_repo_with_seed(&parent_repo, "README.md", "parent\n", "seed parent");

    let upstream_full_text = upstream_full.to_string_lossy().to_string();
    let upstream_squash_text = upstream_squash.to_string_lossy().to_string();

    run_git_allow_file(
        &parent_repo,
        &[
            "subtree",
            "add",
            "--prefix",
            "vendor/full",
            &upstream_full_text,
            "main",
        ],
    );
    run_git_allow_file(
        &parent_repo,
        &[
            "subtree",
            "add",
            "--prefix",
            "vendor/squash",
            "--squash",
            &upstream_squash_text,
            "main",
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");
    let subtrees = opened.list_subtrees().expect("list subtrees");

    assert_eq!(subtrees.len(), 2);
    assert_eq!(subtrees[0].path, PathBuf::from("vendor/full"));
    assert!(subtrees[0].source.is_none());
    assert_eq!(subtrees[1].path, PathBuf::from("vendor/squash"));
    assert!(subtrees[1].source.is_none());
}

#[test]
fn subtree_commands_round_trip_and_persist_source_metadata() {
    let dir = tempfile::tempdir().expect("create tempdir");
    let root = dir.path();

    let upstream_repo = root.join("upstream");
    let push_repo = root.join("push-target.git");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&upstream_repo).expect("create upstream");
    fs::create_dir_all(&parent_repo).expect("create parent");

    init_repo_with_seed(&upstream_repo, "lib.txt", "v1\n", "seed subtree source");
    init_bare_repo(&push_repo);
    init_repo_with_seed(&parent_repo, "README.md", "parent\n", "seed parent");

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).expect("open parent repository");

    let upstream_text = upstream_repo.to_string_lossy().to_string();
    let push_text = push_repo.to_string_lossy().to_string();
    let subtree_path = Path::new("vendor/lib");

    opened
        .add_subtree_with_output(&upstream_text, "main", subtree_path, true)
        .expect("add subtree");
    assert_eq!(
        fs::read_to_string(parent_repo.join("vendor/lib/lib.txt")).expect("read subtree file"),
        "v1\n"
    );

    let added = opened.list_subtrees().expect("list added subtree");
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].path, subtree_path);
    let source = added[0].source.as_ref().expect("stored source config");
    assert_eq!(source.repository, upstream_text);
    assert_eq!(source.reference, "main");
    assert_eq!(source.push_refspec.as_deref(), None);
    assert!(source.squash);

    write_and_commit(&upstream_repo, "lib.txt", "v2\n", "update subtree source");
    opened
        .pull_subtree_with_output(&upstream_text, "main", subtree_path, true)
        .expect("pull subtree");
    assert_eq!(
        fs::read_to_string(parent_repo.join("vendor/lib/lib.txt")).expect("read updated subtree"),
        "v2\n"
    );

    opened
        .split_subtree_with_output(subtree_path, Some("subtree-split"))
        .expect("split subtree");
    assert!(
        !git_stdout(&parent_repo, &["rev-parse", "subtree-split"]).is_empty(),
        "split branch should exist"
    );

    opened
        .push_subtree_with_output(&push_text, "refs/heads/published", subtree_path)
        .expect("push subtree");
    assert!(
        !git_stdout(&push_repo, &["rev-parse", "refs/heads/published"]).is_empty(),
        "published ref should exist in push target"
    );

    let reopened = backend.open(&parent_repo).expect("reopen parent repository");
    let persisted = reopened.list_subtrees().expect("list persisted subtree");
    assert_eq!(persisted.len(), 1);
    let source = persisted[0].source.as_ref().expect("persisted source config");
    assert_eq!(source.repository, upstream_text);
    assert_eq!(source.reference, "main");
    assert_eq!(source.push_refspec.as_deref(), Some("refs/heads/published"));
    assert!(source.squash);

    reopened
        .remove_subtree_with_output(subtree_path)
        .expect("remove subtree");
    assert!(!parent_repo.join(subtree_path).exists());
    assert!(reopened.list_subtrees().expect("list after removal").is_empty());

    let config_output = git_output(&parent_repo, &["config", "--local", "--get-regexp", r"^gitcomet\.subtree\."]);
    assert_eq!(
        config_output.status.code(),
        Some(1),
        "subtree metadata should be cleared, stderr: {}",
        String::from_utf8_lossy(&config_output.stderr)
    );
}
