use gitcomet_core::domain::CommitId;
use gitcomet_core::services::{
    GitBackend, PullMode, SafePushAfterCommitContext, SafePushAfterCommitDecision,
};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;
#[cfg(windows)]
use std::sync::OnceLock;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

fn run_git_capture(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).to_string()
}

fn commit_id(repo: &Path, rev: &str) -> CommitId {
    CommitId(run_git_capture(repo, &["rev-parse", rev]).trim().into())
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_upstream_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git")
            .args(["difftool", "--tool-help"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if output.status.success() {
            return true;
        }
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_shell_for_upstream_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_upstream_tests() {
            eprintln!(
                "skipping upstream integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

#[test]
fn push_without_upstream_sets_upstream() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);

    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "hi\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&work_repo, &["checkout", "-b", "ai_report_issue"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    opened.push().unwrap();

    let upstream = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/ai_report_issue",
        ],
    )
    .trim()
    .to_string();
    assert_eq!(upstream, "origin/ai_report_issue");
}

#[test]
fn pull_without_upstream_sets_upstream() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);

    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "hi\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&work_repo, &["checkout", "-b", "ai_report_issue"]);
    fs::write(work_repo.join("file.txt"), "hi\nmore\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "change"],
    );

    // Push the branch without setting upstream tracking (matches the reported scenario).
    run_git(
        &work_repo,
        &["push", "origin", "HEAD:refs/heads/ai_report_issue"],
    );

    let upstream_before = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/ai_report_issue",
        ],
    );
    assert!(upstream_before.trim().is_empty());

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    opened.pull(PullMode::Default).unwrap();

    let upstream_after = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/ai_report_issue",
        ],
    )
    .trim()
    .to_string();
    assert_eq!(upstream_after, "origin/ai_report_issue");
}

#[test]
fn safe_push_after_published_amend_blocks_and_offers_lease() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);
    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let branch = run_git_capture(&work_repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    fs::write(work_repo.join("file.txt"), "published\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "published"],
    );
    run_git(&work_repo, &["push", "origin", "HEAD"]);
    let pre_head = commit_id(&work_repo, "HEAD");

    fs::write(work_repo.join("file.txt"), "amended\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--amend",
            "-m",
            "amended",
        ],
    );
    let post_head = commit_id(&work_repo, "HEAD");

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let decision = opened
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: true,
            local_branch: Some(branch.to_string()),
            pre_head: Some(pre_head.clone()),
            post_head: Some(post_head.clone()),
        })
        .unwrap();

    let SafePushAfterCommitDecision::Blocked {
        summary: _,
        lease: Some(lease),
    } = decision
    else {
        panic!("published amend should produce a lease offer");
    };
    assert_eq!(lease.remote, "origin");
    assert_eq!(lease.branch, branch);
    assert_eq!(lease.expected, pre_head);
    assert_eq!(lease.local_branch, branch);
    assert_eq!(lease.local_head, post_head);

    run_git(&work_repo, &["checkout", "-b", "other"]);
    let stale_branch_error = opened
        .push_force_with_lease_with_output(&lease)
        .unwrap_err();
    assert!(format!("{stale_branch_error:?}").contains("stale force-push lease"));
    assert_eq!(
        commit_id(&remote_repo, &format!("refs/heads/{}", lease.branch)),
        lease.expected
    );

    run_git(&work_repo, &["checkout", &lease.local_branch]);
    fs::write(work_repo.join("file.txt"), "later\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "later"],
    );
    let stale_head_error = opened
        .push_force_with_lease_with_output(&lease)
        .unwrap_err();
    assert!(format!("{stale_head_error:?}").contains("stale force-push lease"));
    assert_eq!(
        commit_id(&remote_repo, &format!("refs/heads/{}", lease.branch)),
        lease.expected
    );

    run_git(&work_repo, &["reset", "--hard", lease.local_head.as_ref()]);

    opened.push_force_with_lease_with_output(&lease).unwrap();
    assert_eq!(
        commit_id(&work_repo, &format!("origin/{}", lease.branch)),
        post_head
    );
}

#[test]
fn safe_push_after_commit_without_upstream_sets_upstream_for_new_branch() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);
    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["checkout", "-b", "new-topic"]);
    fs::write(work_repo.join("file.txt"), "topic\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "topic"],
    );
    let post_head = commit_id(&work_repo, "HEAD");

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let decision = opened
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: false,
            local_branch: Some("new-topic".to_string()),
            pre_head: None,
            post_head: Some(post_head.clone()),
        })
        .unwrap();

    assert_eq!(
        decision,
        SafePushAfterCommitDecision::PushSetUpstream {
            target: gitcomet_core::services::SafePushAfterCommitTarget {
                remote: "origin".to_string(),
                branch: "new-topic".to_string(),
                local_branch: "new-topic".to_string(),
                local_head: post_head,
            },
        }
    );
}

#[test]
fn push_after_commit_checked_push_rejects_stale_branch_and_head() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);
    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["checkout", "-b", "safe-main"]);
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);
    let remote_base = commit_id(&remote_repo, "refs/heads/safe-main");

    fs::write(work_repo.join("file.txt"), "safe\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "safe"],
    );
    let post_head = commit_id(&work_repo, "HEAD");

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let decision = opened
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: false,
            local_branch: Some("safe-main".to_string()),
            pre_head: None,
            post_head: Some(post_head.clone()),
        })
        .unwrap();
    let SafePushAfterCommitDecision::Push { target } = decision else {
        panic!("safe push should produce a checked push target");
    };

    run_git(&work_repo, &["checkout", "-b", "other"]);
    let stale_branch_error = opened.push_after_commit_with_output(&target).unwrap_err();
    assert!(format!("{stale_branch_error:?}").contains("stale push-after-commit target"));
    assert_eq!(commit_id(&remote_repo, "refs/heads/safe-main"), remote_base);

    run_git(&work_repo, &["checkout", &target.local_branch]);
    fs::write(work_repo.join("file.txt"), "later\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "later"],
    );
    let stale_head_error = opened.push_after_commit_with_output(&target).unwrap_err();
    assert!(format!("{stale_head_error:?}").contains("stale push-after-commit target"));
    assert_eq!(commit_id(&remote_repo, "refs/heads/safe-main"), remote_base);

    run_git(&work_repo, &["reset", "--hard", target.local_head.as_ref()]);
    opened.push_after_commit_with_output(&target).unwrap();
    assert_eq!(commit_id(&remote_repo, "refs/heads/safe-main"), post_head);
}

#[test]
fn safe_push_after_commit_rejects_branch_change_before_decision() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);
    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["checkout", "-b", "safe-main"]);
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);
    let remote_base = commit_id(&remote_repo, "refs/heads/safe-main");

    fs::write(work_repo.join("file.txt"), "safe\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "safe"],
    );
    let post_head = commit_id(&work_repo, "HEAD");

    run_git(&work_repo, &["checkout", "-b", "other"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let decision = opened
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: false,
            local_branch: Some("safe-main".to_string()),
            pre_head: None,
            post_head: Some(post_head),
        })
        .unwrap();

    let SafePushAfterCommitDecision::Blocked {
        summary,
        lease: None,
    } = decision
    else {
        panic!("branch changes before safe-push decision should block");
    };
    assert!(summary.contains("Current branch changed from safe-main to other"));
    assert_eq!(commit_id(&remote_repo, "refs/heads/safe-main"), remote_base);
}

#[test]
fn safe_push_after_amend_blocks_when_remote_advanced_without_conflicting_worktree() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    let peer_repo = root.join("peer");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);
    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let branch = run_git_capture(&work_repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    fs::write(work_repo.join("file.txt"), "published\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "published"],
    );
    run_git(&work_repo, &["push", "origin", "HEAD"]);
    let pre_head = commit_id(&work_repo, "HEAD");

    run_git(root, &["clone", remote_repo.to_str().unwrap(), "peer"]);
    run_git(&peer_repo, &["config", "user.email", "peer@example.com"]);
    run_git(&peer_repo, &["config", "user.name", "Peer"]);
    run_git(&peer_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(peer_repo.join("file.txt"), "remote advanced\n").unwrap();
    run_git(&peer_repo, &["add", "file.txt"]);
    run_git(
        &peer_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "remote advanced",
        ],
    );
    run_git(&peer_repo, &["push", "origin", "HEAD"]);

    fs::write(work_repo.join("file.txt"), "local amended\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--amend",
            "-m",
            "local amended",
        ],
    );
    let post_head = commit_id(&work_repo, "HEAD");

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let decision = opened
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: true,
            local_branch: Some(branch),
            pre_head: Some(pre_head),
            post_head: Some(post_head),
        })
        .unwrap();

    let SafePushAfterCommitDecision::Blocked {
        summary,
        lease: None,
    } = decision
    else {
        panic!("remote-advanced amend should block without lease");
    };
    assert!(summary.contains("changed while committing"));
    assert!(!run_git_capture(&work_repo, &["status", "--short"]).contains("UU "));
}

#[test]
fn safe_push_after_commit_remote_advance_does_not_refresh_force_lease_tracking_ref() {
    if !require_git_shell_for_upstream_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    let peer_repo = root.join("peer");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare"]);
    run_git(&work_repo, &["init"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let branch = run_git_capture(&work_repo, &["branch", "--show-current"])
        .trim()
        .to_string();
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);
    let tracking_ref = format!("refs/remotes/origin/{branch}");
    let tracking_before = commit_id(&work_repo, &tracking_ref);

    run_git(root, &["clone", remote_repo.to_str().unwrap(), "peer"]);
    run_git(&peer_repo, &["config", "user.email", "peer@example.com"]);
    run_git(&peer_repo, &["config", "user.name", "Peer"]);
    run_git(&peer_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(peer_repo.join("file.txt"), "remote advanced\n").unwrap();
    run_git(&peer_repo, &["add", "file.txt"]);
    run_git(
        &peer_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "remote advanced",
        ],
    );
    run_git(&peer_repo, &["push", "origin", "HEAD"]);
    let remote_advanced = commit_id(&remote_repo, &format!("refs/heads/{branch}"));

    fs::write(work_repo.join("file.txt"), "local commit\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "local commit"],
    );
    let post_head = commit_id(&work_repo, "HEAD");

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let decision = opened
        .safe_push_after_commit(&SafePushAfterCommitContext {
            amend: false,
            local_branch: Some(branch.clone()),
            pre_head: None,
            post_head: Some(post_head),
        })
        .unwrap();

    let SafePushAfterCommitDecision::Blocked {
        summary,
        lease: None,
    } = decision
    else {
        panic!("remote-advanced commit should block without lease");
    };
    assert!(summary.contains("changed while committing"));
    assert_eq!(commit_id(&work_repo, &tracking_ref), tracking_before);
    assert_eq!(commit_id(&work_repo, "FETCH_HEAD"), remote_advanced);

    assert!(
        opened.push_force_with_output().is_err(),
        "generic force-with-lease should reject the stale tracking ref"
    );
    assert_eq!(
        commit_id(&remote_repo, &format!("refs/heads/{branch}")),
        remote_advanced
    );
}
