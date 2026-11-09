//! Skipping Git hooks (`--no-verify`) for a commit and the push after it.
//! Every hook here fails, so an operation that ran one fails too.

use crate::git_cli::{git_cmd, git_command, run_checked, stdout_text};
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
    let mut cmd = git_cmd(repo);
    // Local bare remotes require file protocol to be permitted.
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    stdout_text(run_checked(&mut cmd, args)).trim().to_string()
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
    let mut cmd = git_command();
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
    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Run)
        .expect_err("the pre-push hook must prevent publication");
    assert_eq!(remote_branch(&fixture.remote, "topic"), None);
    let config = git(&fixture.work, &["config", "--local", "--list"]);
    assert!(!config.contains("branch.topic.remote="));
    assert!(!config.contains("branch.topic.merge="));

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

/// Commits `topic.txt` on a new `topic` branch and returns its set-upstream
/// push target.
fn new_topic_branch_target(
    fixture: &Fixture,
    repo: &Arc<dyn GitRepository>,
) -> (String, SafePushAfterCommitTarget) {
    git(&fixture.work, &["checkout", "-q", "-b", "topic"]);
    stage_change(&fixture.work, "topic.txt", "topic\n");
    repo.commit_with_hooks("topic", None, GitHooks::Skip)
        .unwrap();
    let committed = head(&fixture.work);
    let (target, set_upstream) = push_target(repo, "topic", None, &committed, GitHooks::Skip);
    assert!(set_upstream, "topic has no upstream yet");
    (committed, target)
}

/// `git push --set-upstream` honours `branch.autoSetupRebase`; the upstream
/// written after a push of the captured commit must too.
#[test]
fn push_after_commit_set_upstream_honours_auto_setup_rebase() {
    for (setting, rebase) in [
        ("always", Some("true")),
        ("remote", Some("true")),
        ("local", None),
        ("never", None),
    ] {
        let fixture = fixture();
        git(
            &fixture.work,
            &["config", "branch.autoSetupRebase", setting],
        );
        let repo = GixBackend.open(&fixture.work).unwrap();
        let (_committed, target) = new_topic_branch_target(&fixture, &repo);

        repo.push_after_commit_with_hooks(&target, true, GitHooks::Skip)
            .unwrap();

        let config = git(&fixture.work, &["config", "--local", "--list"]);
        let actual = config
            .lines()
            .find_map(|line| line.strip_prefix("branch.topic.rebase="));
        assert_eq!(actual, rebase, "{setting}");
    }
}

/// The push already published the commit when the upstream is written, so a
/// locked config must not turn it into a failed push. Like `git push -u`, the
/// output reports the missing upstream.
#[cfg(unix)]
#[test]
fn push_after_commit_set_upstream_succeeds_when_the_config_is_locked_after_the_push() {
    let fixture = fixture();
    let repo = GixBackend.open(&fixture.work).unwrap();
    let (committed, target) = new_topic_branch_target(&fixture, &repo);
    let hooks = PathBuf::from(git(&fixture.work, &["config", "core.hooksPath"]));
    let config_lock = fixture.work.join(".git/config.lock");
    let hook = hooks.join("pre-push");
    fs::write(
        &hook,
        format!("#!/bin/sh\n: > '{}'\nexit 0\n", config_lock.display()),
    )
    .unwrap();

    let result = repo.push_after_commit_with_hooks(&target, true, GitHooks::Run);
    assert!(config_lock.is_file(), "the pre-push hook took the lock");
    fs::remove_file(&config_lock).unwrap();

    let output = result.expect("the push itself succeeded");
    assert_eq!(output.exit_code, Some(0));
    assert!(
        output
            .stderr
            .contains("error: unable to write upstream branch configuration"),
        "the output says the upstream still needs setting: {}",
        output.stderr
    );
    let config = git(&fixture.work, &["config", "--local", "--list"]);
    assert!(!config.contains("branch.topic."), "no upstream: {config}");
    assert_eq!(
        remote_branch(&fixture.remote, "topic").as_deref(),
        Some(committed.as_str())
    );
}

/// `git lfs install` configures the LFS filter globally. A repository that
/// does not use LFS must not need git-lfs to push without hooks, since a
/// push that runs hooks would not run it either. Empty files and size-0
/// pointers have no content for LFS to upload.
#[cfg(unix)]
#[test]
fn push_after_commit_without_hooks_does_not_run_git_lfs_in_a_repository_without_lfs() {
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "push_after_commit_without_hooks_does_not_run_git_lfs_in_a_repository_without_lfs",
        r#"#!/bin/sh
for argument in "$@"; do
  if [ "$argument" = lfs ]; then
    echo "git: 'lfs' is not a git command. See 'git --help'." >&2
    exit 1
  fi
done
exec git "$@"
"#,
        || {
            for branch in ["main", "topic"] {
                let fixture = fixture();
                for (key, value) in [
                    ("filter.lfs.process", "git-lfs filter-process"),
                    ("filter.lfs.clean", "git-lfs clean -- %f"),
                ] {
                    git(&fixture.work, &["config", key, value]);
                }
                let repo = GixBackend.open(&fixture.work).unwrap();
                let base = head(&fixture.work);
                let pre_head = if branch == "main" {
                    Some(base.as_str())
                } else {
                    git(&fixture.work, &["checkout", "-q", "-b", branch]);
                    None
                };
                stage_change(&fixture.work, "file.txt", "change\n");
                stage_change(&fixture.work, "empty.txt", "");
                stage_change(
                    &fixture.work,
                    "empty.ptr",
                    "version https://git-lfs.github.com/spec/v1\n\
                     oid sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855\n\
                     size 0\n",
                );
                repo.commit_with_hooks("change", None, GitHooks::Skip)
                    .unwrap();
                let committed = head(&fixture.work);
                let (target, set_upstream) =
                    push_target(&repo, branch, pre_head, &committed, GitHooks::Skip);

                let output = repo
                    .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
                    .unwrap_or_else(|error| panic!("{branch}: {error}"));

                assert!(!output.command.contains("git lfs"), "{}", output.command);
                assert_eq!(
                    remote_branch(&fixture.remote, branch).as_deref(),
                    Some(committed.as_str()),
                    "{branch}"
                );
            }
        },
    );
}

/// `--no-verify` changes nothing when Git would run no `pre-push` hook, so
/// a pointer-shaped test fixture in a repository without LFS must not need
/// git-lfs then. A hook configured for the event still counts.
#[cfg(unix)]
#[test]
fn push_after_commit_without_hooks_runs_git_lfs_only_when_a_pre_push_hook_would_run() {
    use std::os::unix::fs::PermissionsExt as _;

    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "push_after_commit_without_hooks_runs_git_lfs_only_when_a_pre_push_hook_would_run",
        r#"#!/bin/sh
for argument in "$@"; do
  if [ "$argument" = lfs ]; then
    echo "git: 'lfs' is not a git command. See 'git --help'." >&2
    exit 1
  fi
done
exec git "$@"
"#,
        || {
            for case in ["no hook", "hook not executable", "configured hook"] {
                let fixture = fixture();
                let hook = PathBuf::from(git(&fixture.work, &["config", "core.hooksPath"]))
                    .join("pre-push");
                match case {
                    "hook not executable" => {
                        fs::set_permissions(&hook, fs::Permissions::from_mode(0o644)).unwrap()
                    }
                    _ => fs::remove_file(&hook).unwrap(),
                }
                if case == "configured hook" {
                    git(&fixture.work, &["config", "hook.refuse.event", "pre-push"]);
                    git(&fixture.work, &["config", "hook.refuse.command", "false"]);
                }
                let repo = GixBackend.open(&fixture.work).unwrap();
                let base = head(&fixture.work);
                stage_change(
                    &fixture.work,
                    "fixture.ptr",
                    "version https://git-lfs.github.com/spec/v1\n\
                     oid sha256:6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72\n\
                     size 12\n",
                );
                repo.commit_with_hooks("pointer fixture", None, GitHooks::Skip)
                    .unwrap();
                let committed = head(&fixture.work);
                let (target, set_upstream) =
                    push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);

                let result =
                    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip);

                if case == "configured hook" {
                    result.expect_err("the skipped hook could have uploaded the pointer");
                    assert_eq!(
                        remote_branch(&fixture.remote, "main").as_deref(),
                        Some(base.as_str())
                    );
                    continue;
                }
                let output = result.unwrap_or_else(|error| panic!("{case}: {error}"));
                assert!(!output.command.contains("git lfs"), "{}", output.command);
                assert_eq!(
                    remote_branch(&fixture.remote, "main").as_deref(),
                    Some(committed.as_str()),
                    "{case}"
                );
            }
        },
    );
}

/// Merging the remote's default branch, or starting a new branch from it,
/// brings in commits the remote already has, pointer fixtures included. LFS's
/// hook would upload none of them, so a push without hooks must not run
/// git-lfs for them either.
#[cfg(unix)]
#[test]
fn push_after_commit_without_hooks_does_not_rescan_what_the_remote_has() {
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "push_after_commit_without_hooks_does_not_rescan_what_the_remote_has",
        r#"#!/bin/sh
for argument in "$@"; do
  if [ "$argument" = lfs ]; then
    echo "git: 'lfs' is not a git command. See 'git --help'." >&2
    exit 1
  fi
done
exec git "$@"
"#,
        || {
            let fixture = fixture();
            git(&fixture.work, &["checkout", "-q", "-b", "topic"]);
            stage_change(&fixture.work, "topic.txt", "topic\n");
            git(
                &fixture.work,
                &["commit", "-q", "--no-verify", "-m", "topic"],
            );
            git(
                &fixture.work,
                &["push", "-q", "--no-verify", "-u", "origin", "topic"],
            );
            let topic = head(&fixture.work);
            git(&fixture.work, &["checkout", "-q", "main"]);
            stage_change(
                &fixture.work,
                "fixture.ptr",
                "version https://git-lfs.github.com/spec/v1\n\
                 oid sha256:6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72\n\
                 size 12\n",
            );
            git(
                &fixture.work,
                &["commit", "-q", "--no-verify", "-m", "fixture"],
            );
            git(
                &fixture.work,
                &["push", "-q", "--no-verify", "origin", "main"],
            );
            git(&fixture.work, &["remote", "set-head", "origin", "main"]);
            git(&fixture.work, &["checkout", "-q", "topic"]);
            git(
                &fixture.work,
                &["merge", "-q", "--no-ff", "--no-commit", "main"],
            );
            let repo = GixBackend.open(&fixture.work).unwrap();
            repo.commit_with_hooks("merge main", None, GitHooks::Skip)
                .unwrap();
            let merged = head(&fixture.work);
            let (target, set_upstream) =
                push_target(&repo, "topic", Some(&topic), &merged, GitHooks::Skip);

            let output = repo
                .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
                .unwrap();

            assert!(!output.command.contains("git lfs"), "{}", output.command);
            assert_eq!(
                remote_branch(&fixture.remote, "topic").as_deref(),
                Some(merged.as_str())
            );

            git(&fixture.work, &["checkout", "-q", "-b", "fresh", "main"]);
            stage_change(&fixture.work, "fresh.txt", "fresh\n");
            repo.commit_with_hooks("fresh", None, GitHooks::Skip)
                .unwrap();
            let fresh = head(&fixture.work);
            let (target, set_upstream) = push_target(&repo, "fresh", None, &fresh, GitHooks::Skip);

            let output = repo
                .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
                .unwrap();

            assert!(!output.command.contains("git lfs"), "{}", output.command);
            assert_eq!(
                remote_branch(&fixture.remote, "fresh").as_deref(),
                Some(fresh.as_str())
            );
        },
    );
}

/// A blob missing locally, as in a partial clone, could be a pointer, so a
/// push without hooks leaves it to git-lfs rather than skipping the upload.
#[cfg(unix)]
#[test]
fn push_after_commit_without_hooks_leaves_an_unreadable_blob_to_git_lfs() {
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "push_after_commit_without_hooks_leaves_an_unreadable_blob_to_git_lfs",
        r#"#!/bin/sh
for argument in "$@"; do
  if [ "$argument" = lfs ]; then
    echo "git: 'lfs' is not a git command. See 'git --help'." >&2
    exit 1
  fi
done
exec git "$@"
"#,
        || {
            let fixture = fixture();
            let base = head(&fixture.work);
            let mut mktree = git_command();
            let mut child = mktree
                .args(["-C", fixture.work.to_str().unwrap(), "mktree", "--missing"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap();
            {
                use std::io::Write as _;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(
                        b"100644 blob 0123456789abcdef0123456789abcdef01234567\tmissing.ptr\n",
                    )
                    .unwrap();
            }
            let tree = child.wait_with_output().unwrap();
            assert!(tree.status.success());
            let tree = String::from_utf8(tree.stdout).unwrap();
            let commit = git(
                &fixture.work,
                &[
                    "commit-tree",
                    tree.trim(),
                    "-p",
                    &base,
                    "-m",
                    "missing blob",
                ],
            );
            git(&fixture.work, &["update-ref", "refs/heads/main", &commit]);
            let repo = GixBackend.open(&fixture.work).unwrap();
            let (target, set_upstream) =
                push_target(&repo, "main", Some(&base), &commit, GitHooks::Skip);

            let error = repo
                .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
                .expect_err("git-lfs is missing here");

            assert!(error.to_string().contains("git lfs push"), "{error}");
            assert_eq!(
                remote_branch(&fixture.remote, "main").as_deref(),
                Some(base.as_str())
            );
        },
    );
}

/// `git fetch --prune` can leave `origin/HEAD` naming a deleted branch; the
/// check before a push without hooks ignores it, as Git does.
#[test]
fn push_after_commit_without_hooks_ignores_a_dangling_remote_head() {
    let fixture = fixture();
    git(
        &fixture.work,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/deleted",
        ],
    );
    let repo = GixBackend.open(&fixture.work).unwrap();
    let base = head(&fixture.work);
    stage_change(&fixture.work, "file.txt", "change\n");
    repo.commit_with_hooks("change", None, GitHooks::Skip)
        .unwrap();
    let committed = head(&fixture.work);
    let (target, set_upstream) =
        push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);

    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .unwrap();

    assert_eq!(
        remote_branch(&fixture.remote, "main").as_deref(),
        Some(committed.as_str())
    );
}

fn git_lfs_available() -> bool {
    let available = git_command()
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

/// The fixture with Git LFS configured and `*.bin` tracked by LFS.
fn lfs_fixture() -> (Fixture, Arc<dyn GitRepository>) {
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
    fs::write(
        fixture.work.join(".gitattributes"),
        "*.bin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    let repo = GixBackend.open(&fixture.work).unwrap();
    (fixture, repo)
}

/// Commits `asset.bin` as an LFS pointer, skipping hooks, and returns the
/// commit and where the remote keeps its object.
fn commit_lfs_asset(fixture: &Fixture, repo: &Arc<dyn GitRepository>) -> (String, PathBuf) {
    fs::write(fixture.work.join("asset.bin"), b"large file contents\n").unwrap();
    git(&fixture.work, &["add", ".gitattributes", "asset.bin"]);
    repo.commit_with_hooks("asset", None, GitHooks::Skip)
        .unwrap();
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
    (head(&fixture.work), remote_object)
}

/// LFS uploads from its `pre-push` hook. A push that skips hooks uploads the
/// objects itself, so the remote never receives a pointer without content,
/// on a tracked branch and on one the push sets the upstream of.
#[test]
fn push_after_commit_without_hooks_still_uploads_lfs_objects() {
    if !git_lfs_available() {
        return;
    }
    for branch in ["main", "topic"] {
        let (fixture, repo) = lfs_fixture();
        let base = head(&fixture.work);
        let pre_head = if branch == "main" {
            Some(base.as_str())
        } else {
            git(&fixture.work, &["checkout", "-q", "-b", branch]);
            None
        };
        let (committed, remote_object) = commit_lfs_asset(&fixture, &repo);

        let (target, set_upstream) =
            push_target(&repo, branch, pre_head, &committed, GitHooks::Skip);
        assert_eq!(set_upstream, branch == "topic", "{branch}");
        let output = repo
            .push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
            .unwrap();

        assert!(
            output.command.starts_with("git lfs push origin "),
            "{branch}: {}",
            output.command
        );
        assert!(
            output.command.contains("--no-verify"),
            "{branch}: {}",
            output.command
        );
        assert!(
            remote_object.is_file(),
            "{branch}: the LFS object was uploaded"
        );
        assert_eq!(
            remote_branch(&fixture.remote, branch).as_deref(),
            Some(committed.as_str()),
            "{branch}"
        );
    }
}

#[test]
fn push_after_commit_without_hooks_refuses_missing_lfs_objects_without_a_local_store() {
    if !git_lfs_available() {
        return;
    }
    for branch in ["main", "topic"] {
        let (fixture, repo) = lfs_fixture();
        let base = head(&fixture.work);
        let pre_head = if branch == "main" {
            Some(base.as_str())
        } else {
            git(&fixture.work, &["checkout", "-q", "-b", branch]);
            None
        };
        let (committed, remote_object) = commit_lfs_asset(&fixture, &repo);
        let pointer = git(&fixture.work, &["show", "HEAD:asset.bin"]);
        fs::write(fixture.work.join("asset.bin"), pointer).unwrap();
        let local_store = fixture.work.join(".git/lfs/objects");
        fs::remove_dir_all(&local_store).unwrap();
        assert!(!local_store.exists());

        let (target, set_upstream) =
            push_target(&repo, branch, pre_head, &committed, GitHooks::Skip);
        repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
            .expect_err("missing LFS content must prevent publishing its pointer");

        assert!(!remote_object.exists(), "{branch}");
        assert_eq!(
            remote_branch(&fixture.remote, branch).as_deref(),
            pre_head,
            "{branch}: the remote ref must not advance"
        );
    }
}

/// The outgoing commit holds an LFS pointer whose content is missing, but the
/// current attribute files do not show `filter=lfs`: a macro, a global
/// attributes file, or an unstaged edit dropping the rule. LFS's own hook
/// refuses this push, so a push that skips hooks must too.
#[test]
fn push_after_commit_without_hooks_finds_outgoing_pointers_the_attribute_files_hide() {
    if !git_lfs_available() {
        return;
    }
    for case in ["macro", "global attributes", "unstaged edit"] {
        let (fixture, repo) = lfs_fixture();
        let base = head(&fixture.work);
        let attributes = fixture.work.join(".gitattributes");
        match case {
            "macro" => fs::write(
                &attributes,
                "[attr]big filter=lfs diff=lfs merge=lfs -text\n*.bin big\n",
            )
            .unwrap(),
            "global attributes" => {
                let global = fixture.work.parent().unwrap().join("global-attributes");
                fs::rename(&attributes, &global).unwrap();
                fs::write(&attributes, "").unwrap();
                git(
                    &fixture.work,
                    &["config", "core.attributesFile", global.to_str().unwrap()],
                );
            }
            _ => {}
        }
        let (committed, remote_object) = commit_lfs_asset(&fixture, &repo);
        if case == "unstaged edit" {
            fs::write(&attributes, "").unwrap();
        }
        let pointer = git(&fixture.work, &["show", "HEAD:asset.bin"]);
        fs::write(fixture.work.join("asset.bin"), pointer).unwrap();
        fs::remove_dir_all(fixture.work.join(".git/lfs/objects")).unwrap();

        let (target, set_upstream) =
            push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);
        repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
            .expect_err(case);

        assert!(!remote_object.exists(), "{case}");
        assert_eq!(
            remote_branch(&fixture.remote, "main").as_deref(),
            Some(base.as_str()),
            "{case}: the remote ref must not advance"
        );
    }
}

/// Changing an LFS file modifies its pointer blob instead of adding one.
#[test]
fn push_after_commit_without_hooks_refuses_a_modified_pointer_with_missing_content() {
    if !git_lfs_available() {
        return;
    }
    let (fixture, repo) = lfs_fixture();
    let base = head(&fixture.work);
    let (first, _remote_object) = commit_lfs_asset(&fixture, &repo);
    let (target, set_upstream) = push_target(&repo, "main", Some(&base), &first, GitHooks::Skip);
    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .unwrap();
    stage_change(&fixture.work, "asset.bin", "changed large file contents\n");
    repo.commit_with_hooks("change asset", None, GitHooks::Skip)
        .unwrap();
    let changed = head(&fixture.work);
    fs::remove_dir_all(fixture.work.join(".git/lfs/objects")).unwrap();

    let (target, set_upstream) = push_target(&repo, "main", Some(&first), &changed, GitHooks::Skip);
    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .expect_err("the changed content is missing");

    assert_eq!(
        remote_branch(&fixture.remote, "main").as_deref(),
        Some(first.as_str())
    );
}

/// git-lfs reads pointers more loosely than the spec it writes them by, and
/// its hook refuses each of these when the content is missing.
#[test]
fn push_after_commit_without_hooks_refuses_noncanonical_pointers_with_missing_content() {
    if !git_lfs_available() {
        return;
    }
    let oid = "oid sha256:6ae8a75555209fd6c44157c0aed8016e763ff435a19cf186f76863140143ff72";
    for (case, pointer) in [
        (
            "CRLF line endings",
            format!("version https://git-lfs.github.com/spec/v1\r\n{oid}\r\nsize 12\r\n"),
        ),
        (
            "zero-padded size",
            format!("version https://git-lfs.github.com/spec/v1\n{oid}\nsize 0012\n"),
        ),
        (
            "signed size",
            format!("version https://git-lfs.github.com/spec/v1\n{oid}\nsize +12\n"),
        ),
        (
            "surrounding blank lines",
            format!("\nversion https://git-lfs.github.com/spec/v1\n\n{oid}\nsize 12\n\n"),
        ),
        (
            "alpha spec version",
            format!("version http://git-media.io/v/2\n{oid}\nsize 12\n"),
        ),
    ] {
        let (fixture, repo) = lfs_fixture();
        let base = head(&fixture.work);
        stage_change(&fixture.work, "fixture.ptr", &pointer);
        repo.commit_with_hooks("pointer", None, GitHooks::Skip)
            .unwrap();
        let committed = head(&fixture.work);

        let (target, set_upstream) =
            push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);
        repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
            .expect_err(case);

        assert_eq!(
            remote_branch(&fixture.remote, "main").as_deref(),
            Some(base.as_str()),
            "{case}: the remote ref must not advance"
        );
    }
}

/// Removing the LFS filter config leaves the repository's pointers, objects
/// and `pre-push` hook. A push that skips that hook still uploads the objects.
#[test]
fn push_after_commit_without_hooks_uploads_lfs_objects_when_the_filter_is_unconfigured() {
    if !git_lfs_available() {
        return;
    }
    let (fixture, repo) = lfs_fixture();
    let base = head(&fixture.work);
    let (committed, remote_object) = commit_lfs_asset(&fixture, &repo);
    // Blank values override a global `git lfs install` as well.
    for key in ["filter.lfs.process", "filter.lfs.clean"] {
        git(&fixture.work, &["config", key, ""]);
    }

    let (target, set_upstream) =
        push_target(&repo, "main", Some(&base), &committed, GitHooks::Skip);
    repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
        .unwrap();

    assert!(remote_object.is_file(), "the LFS object was uploaded");
    assert_eq!(
        remote_branch(&fixture.remote, "main").as_deref(),
        Some(committed.as_str())
    );
}

#[cfg(unix)]
#[test]
fn push_after_commit_sets_upstream_for_the_captured_commit_when_the_branch_moves_at_push_launch() {
    if !git_lfs_available() {
        return;
    }
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "push_after_commit_sets_upstream_for_the_captured_commit_when_the_branch_moves_at_push_launch",
        r#"#!/bin/sh
previous=
for argument in "$@"; do
  if [ "$previous" = -C ]; then
    cd "$argument" || exit $?
  fi
  case "$argument" in
    lfs) break ;;
    push)
      moved=$(git config --get test.concurrentCommit) || exit $?
      git update-ref refs/heads/topic "$moved" || exit $?
      break ;;
  esac
  previous=$argument
done
exec git "$@"
"#,
        || {
            let (fixture, repo) = lfs_fixture();
            git(&fixture.work, &["checkout", "-q", "-b", "topic"]);
            let (committed, remote_object) = commit_lfs_asset(&fixture, &repo);
            let (target, set_upstream) =
                push_target(&repo, "topic", None, &committed, GitHooks::Skip);
            assert!(set_upstream);

            stage_change(&fixture.work, "newer.bin", "newer large file contents\n");
            repo.commit_with_hooks("newer asset", None, GitHooks::Skip)
                .unwrap();
            let moved = head(&fixture.work);
            let newer_pointer = git(&fixture.work, &["show", "HEAD:newer.bin"]);
            let newer_oid = newer_pointer
                .lines()
                .find_map(|line| line.strip_prefix("oid sha256:"))
                .expect("newer.bin is an LFS pointer");
            let newer_remote_object = fixture
                .remote
                .join("lfs/objects")
                .join(&newer_oid[0..2])
                .join(&newer_oid[2..4])
                .join(newer_oid);
            git(&fixture.work, &["reset", "--hard", &committed]);
            git(&fixture.work, &["config", "test.concurrentCommit", &moved]);

            repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
                .unwrap();

            assert_eq!(
                head(&fixture.work),
                moved,
                "the branch moved at push launch"
            );
            assert!(
                remote_object.is_file(),
                "the captured commit's LFS content arrived"
            );
            assert!(!newer_remote_object.exists());
            assert_eq!(
                remote_branch(&fixture.remote, "topic").as_deref(),
                Some(committed.as_str()),
                "the remote must receive the commit whose LFS content was uploaded"
            );
            assert_eq!(
                git(
                    &fixture.work,
                    &["rev-parse", "--abbrev-ref", "topic@{upstream}"]
                ),
                "origin/topic"
            );
        },
    );
}

/// A commit made while `git lfs push` uploads stays local: both push modes
/// publish the captured commit whose objects were uploaded.
#[cfg(unix)]
#[test]
fn push_after_commit_without_hooks_publishes_the_uploaded_commit_when_the_branch_moves() {
    use std::os::unix::fs::PermissionsExt as _;

    if !git_lfs_available() {
        return;
    }
    for branch in ["main", "topic"] {
        let (fixture, repo) = lfs_fixture();
        let base = head(&fixture.work);
        let pre_head = if branch == "main" {
            Some(base.as_str())
        } else {
            git(&fixture.work, &["checkout", "-q", "-b", branch]);
            None
        };
        let (committed, _remote_object) = commit_lfs_asset(&fixture, &repo);
        // A transfer agent that commits on the branch while it "uploads".
        let agent = fixture.work.parent().unwrap().join("moving-agent.sh");
        fs::write(
            &agent,
            format!(
                r#"#!/bin/sh
while IFS= read -r line; do
  case "$line" in
    *'"event":"init"'*) echo '{{}}' ;;
    *'"event":"upload"'*)
      oid=$(printf '%s' "$line" | sed 's/.*"oid":"\([0-9a-f]*\)".*/\1/')
      git -C '{work}' commit -q --no-verify --allow-empty -m moved >/dev/null 2>&1
      printf '{{"event":"complete","oid":"%s"}}\n' "$oid" ;;
    *'"event":"terminate"'*) exit 0 ;;
  esac
done
"#,
                work = fixture.work.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&agent, fs::Permissions::from_mode(0o755)).unwrap();
        git(
            &fixture.work,
            &[
                "config",
                "lfs.customtransfer.moving.path",
                agent.to_str().unwrap(),
            ],
        );
        git(
            &fixture.work,
            &["config", "lfs.standalonetransferagent", "moving"],
        );

        let (target, set_upstream) =
            push_target(&repo, branch, pre_head, &committed, GitHooks::Skip);
        assert_eq!(set_upstream, branch == "topic", "{branch}");
        repo.push_after_commit_with_hooks(&target, set_upstream, GitHooks::Skip)
            .unwrap_or_else(|error| panic!("{branch}: {error}"));

        assert_ne!(
            head(&fixture.work),
            committed,
            "{branch}: the agent committed"
        );
        assert_eq!(
            remote_branch(&fixture.remote, branch).as_deref(),
            Some(committed.as_str()),
            "{branch}: the remote must receive the commit whose upload ran"
        );
    }
}
