//! Skipping Git hooks (`--no-verify`) for a commit and the push after it.
//! Every hook here fails, so an operation that ran one fails too.

use crate::test_git_env;
use gitcomet_core::services::{
    GitBackend, GitHooks, GitRepository, SafePushAfterCommitContext, SafePushAfterCommitDecision,
    SafePushAfterCommitTarget,
};
use gitcomet_core::test_support::git_fixture::append_config;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn git(repo: &Path, args: &[&str]) -> String {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut cmd);
    // Local bare remotes require file protocol to be permitted.
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    let output = cmd
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn write_failing_hook(hooks: &Path, name: &str) {
    fs::create_dir_all(hooks).expect("create hooks directory");
    let path = hooks.join(name);
    fs::write(
        &path,
        format!("#!/bin/sh\necho \"{name} refused\" >&2\nexit 1\n"),
    )
    .expect("write hook");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut permissions = fs::metadata(&path).expect("hook metadata").permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&path, permissions).expect("make hook executable");
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    work: PathBuf,
    remote: PathBuf,
}

/// A clone whose `pre-commit`, `commit-msg` and `pre-push` hooks all refuse,
/// with one pushed commit on `main` tracking `origin/main`.
fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let work = dir.path().join("work");
    let remote = dir.path().join("remote.git");
    let hooks = dir.path().join("hooks");
    fs::create_dir_all(&work).unwrap();
    git(
        dir.path(),
        &["init", "-q", "--bare", remote.to_str().unwrap()],
    );
    git(&work, &["init", "-q", "-b", "main"]);
    append_config(
        &work,
        &[
            ("user.name", "Test"),
            ("user.email", "test@example.com"),
            ("commit.gpgsign", "false"),
            ("core.autocrlf", "false"),
        ],
    );
    git(
        &work,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    fs::write(work.join("file.txt"), "base\n").unwrap();
    git(&work, &["add", "file.txt"]);
    git(&work, &["commit", "-qm", "base"]);
    git(&work, &["push", "-q", "-u", "origin", "main"]);
    for name in ["pre-commit", "commit-msg", "pre-push"] {
        write_failing_hook(&hooks, name);
    }
    git(
        &work,
        &["config", "core.hooksPath", hooks.to_str().unwrap()],
    );
    Fixture {
        _dir: dir,
        work,
        remote,
    }
}

fn stage_change(work: &Path, name: &str, contents: &str) {
    fs::write(work.join(name), contents).unwrap();
    git(work, &["add", name]);
}

fn head(repo: &Path) -> String {
    git(repo, &["rev-parse", "HEAD"])
}

fn remote_branch(remote: &Path, branch: &str) -> Option<String> {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut cmd);
    let output = cmd
        .arg("--git-dir")
        .arg(remote)
        .args(["rev-parse", "--verify", "--quiet"])
        .arg(format!("refs/heads/{branch}"))
        .output()
        .expect("run git");
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn push_target(
    repo: &Arc<dyn GitRepository>,
    branch: &str,
    pre_head: Option<&str>,
    post_head: &str,
    hooks: GitHooks,
) -> (SafePushAfterCommitTarget, bool) {
    let decision = repo
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: false,
            local_branch: Some(branch.to_string()),
            pre_head: pre_head.map(|id| gitcomet_core::domain::CommitId(id.into())),
            post_head: Some(gitcomet_core::domain::CommitId(post_head.into())),
            hooks,
        })
        .unwrap();
    match decision {
        SafePushAfterCommitDecision::Push { target } => (target, false),
        SafePushAfterCommitDecision::PushSetUpstream { target } => (target, true),
        SafePushAfterCommitDecision::Blocked { summary, .. } => {
            panic!("push after commit blocked: {summary}")
        }
    }
}

#[test]
fn commit_and_amend_skip_pre_commit_and_commit_msg_hooks_only_when_asked() {
    let fixture = fixture();
    let repo = GixBackend.open(&fixture.work).unwrap();
    let base = head(&fixture.work);

    stage_change(&fixture.work, "file.txt", "change\n");
    let refused = repo
        .commit_with_hooks("checked", None, GitHooks::Run)
        .unwrap_err();
    assert!(
        format!("{refused}").contains("pre-commit refused"),
        "the hook ran: {refused}"
    );
    assert_eq!(head(&fixture.work), base);

    let outcome = repo
        .commit_with_hooks("unchecked", None, GitHooks::Skip)
        .unwrap();
    let committed = head(&fixture.work);
    assert_ne!(committed, base);
    assert_eq!(
        outcome.post_head.as_ref().map(|id| id.as_ref()),
        Some(committed.as_str())
    );
    assert_eq!(
        git(&fixture.work, &["log", "-1", "--format=%s"]),
        "unchecked"
    );

    assert!(
        repo.commit_amend_with_hooks("amend checked", None, GitHooks::Run)
            .is_err()
    );
    assert_eq!(head(&fixture.work), committed);
    repo.commit_amend_with_hooks("amend unchecked", None, GitHooks::Skip)
        .unwrap();
    assert_ne!(head(&fixture.work), committed);
    assert_eq!(
        git(&fixture.work, &["log", "-1", "--format=%s"]),
        "amend unchecked"
    );
    assert_eq!(
        git(&fixture.work, &["rev-parse", "HEAD~1"]),
        base,
        "the amend replaced the commit"
    );
}

#[test]
fn push_after_commit_skips_the_pre_push_hook_only_when_asked() {
    let fixture = fixture();
    let repo = GixBackend.open(&fixture.work).unwrap();
    let base = head(&fixture.work);

    stage_change(&fixture.work, "file.txt", "change\n");
    repo.commit_with_hooks("change", None, GitHooks::Skip)
        .unwrap();
    let committed = head(&fixture.work);

    let (target, set_upstream) =
        push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);
    assert!(!set_upstream, "main already tracks origin/main");

    let refused = repo
        .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Run)
        .unwrap_err();
    assert!(
        format!("{refused}").contains("pre-push refused"),
        "the hook ran: {refused}"
    );
    assert_eq!(
        remote_branch(&fixture.remote, "main").as_deref(),
        Some(base.as_str())
    );

    let output = repo
        .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .unwrap();
    assert!(output.command.contains("--no-verify"), "{}", output.command);
    assert!(!output.command.contains("git lfs"), "{}", output.command);
    assert_eq!(
        remote_branch(&fixture.remote, "main").as_deref(),
        Some(committed.as_str())
    );
}

#[test]
fn push_after_commit_sets_upstream_without_running_the_pre_push_hook() {
    let fixture = fixture();
    git(&fixture.work, &["checkout", "-q", "-b", "topic"]);
    let repo = GixBackend.open(&fixture.work).unwrap();

    stage_change(&fixture.work, "topic.txt", "topic\n");
    repo.commit_with_hooks("topic", None, GitHooks::Skip)
        .unwrap();
    let committed = head(&fixture.work);

    let (target, set_upstream) = push_target(&repo, "topic", None, &committed, GitHooks::Skip);
    assert!(set_upstream, "topic has no upstream yet");
    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .unwrap();

    assert_eq!(
        remote_branch(&fixture.remote, "topic").as_deref(),
        Some(committed.as_str())
    );
    assert_eq!(
        git(
            &fixture.work,
            &["rev-parse", "--abbrev-ref", "topic@{upstream}"]
        ),
        "origin/topic"
    );
}

fn git_lfs_available() -> bool {
    let available = gitcomet_core::test_support::git_fixture::git_command()
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|output| output.status.success());
    if !available {
        assert!(
            std::env::var("GITCOMET_REQUIRE_GIT_LFS").as_deref() != Ok("1"),
            "git-lfs is required for this test run",
        );
        eprintln!("skipping: git-lfs is not installed");
    }
    available
}

fn file_url(path: &Path) -> String {
    // git-lfs rejects `file://C:\...`; drive paths need `file:///C:/...`.
    let path = path.to_string_lossy().replace('\\', "/");
    if path.starts_with('/') {
        format!("file://{path}")
    } else {
        format!("file:///{path}")
    }
}

/// LFS uploads from its `pre-push` hook. A push that skips hooks uploads the
/// objects itself, so the remote never receives a pointer without content.
#[test]
fn push_after_commit_without_hooks_still_uploads_lfs_objects() {
    if !git_lfs_available() {
        return;
    }
    let fixture = fixture();
    git(
        &fixture.work,
        &["remote", "set-url", "origin", &file_url(&fixture.remote)],
    );
    for (key, value) in [
        ("filter.lfs.process", "git-lfs filter-process"),
        ("filter.lfs.clean", "git-lfs clean -- %f"),
        ("filter.lfs.smudge", "git-lfs smudge -- %f"),
        ("filter.lfs.required", "true"),
    ] {
        git(&fixture.work, &["config", key, value]);
    }
    let repo = GixBackend.open(&fixture.work).unwrap();
    let base = head(&fixture.work);

    fs::write(
        fixture.work.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    fs::write(fixture.work.join("asset.bin"), b"large file contents\n").unwrap();
    git(&fixture.work, &["add", ".gitattributes", "asset.bin"]);
    repo.commit_with_hooks("asset", None, GitHooks::Skip)
        .unwrap();
    let committed = head(&fixture.work);
    let pointer = git(&fixture.work, &["show", "HEAD:asset.bin"]);
    let oid = pointer
        .lines()
        .find_map(|line| line.strip_prefix("oid sha256:"))
        .expect("asset.bin is an LFS pointer")
        .to_string();
    let remote_object = fixture
        .remote
        .join("lfs/objects")
        .join(&oid[0..2])
        .join(&oid[2..4])
        .join(&oid);
    assert!(!remote_object.exists());

    let (target, set_upstream) =
        push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);
    let output = repo
        .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .unwrap();

    assert!(
        output.command.starts_with("git lfs push origin "),
        "{}",
        output.command
    );
    assert!(output.command.contains("--no-verify"), "{}", output.command);
    assert!(remote_object.is_file(), "the LFS object was uploaded");
    assert_eq!(
        remote_branch(&fixture.remote, "main").as_deref(),
        Some(committed.as_str())
    );
}
