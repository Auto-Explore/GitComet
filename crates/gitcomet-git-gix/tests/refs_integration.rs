use gitcomet_core::domain::{Upstream, UpstreamDivergence};
use gitcomet_core::services::GitBackend;
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

fn git_remote_url(path: &Path) -> String {
    if cfg!(windows) {
        // Ensure Windows drive-letter paths are never treated as scp-style host:path.
        let normalized = path.to_string_lossy().replace('\\', "/");
        format!("file:///{normalized}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_refs_integration_tests() -> bool {
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

fn require_git_shell_for_refs_integration_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_refs_integration_tests() {
            eprintln!(
                "skipping refs integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

#[test]
fn list_branches_reports_upstream_and_divergence() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    let peer_repo = root.join("peer");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature-1\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature-1"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    fs::write(
        work_repo.join("feature.txt"),
        "feature-1\nfeature-local-ahead\n",
    )
    .unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature-local-ahead",
        ],
    );

    run_git(
        root,
        &[
            "clone",
            origin_url.as_str(),
            peer_repo.to_str().expect("peer path"),
        ],
    );
    run_git(&peer_repo, &["config", "user.email", "you@example.com"]);
    run_git(&peer_repo, &["config", "user.name", "You"]);
    run_git(&peer_repo, &["config", "commit.gpgsign", "false"]);
    run_git(&peer_repo, &["checkout", "feature"]);

    fs::write(peer_repo.join("peer.txt"), "remote-ahead\n").unwrap();
    run_git(&peer_repo, &["add", "peer.txt"]);
    run_git(
        &peer_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "feature-remote-ahead",
        ],
    );
    run_git(&peer_repo, &["push", "origin", "feature"]);

    run_git(&work_repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");

    assert_eq!(
        feature.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
    assert_eq!(
        feature.divergence,
        Some(UpstreamDivergence {
            ahead: 1,
            behind: 1,
        })
    );
}

#[test]
fn list_branches_gone_upstream_keeps_upstream_and_clears_divergence() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("base.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "base.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    run_git(&work_repo, &["push", "origin", "--delete", "feature"]);
    run_git(&work_repo, &["fetch", "--prune", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");

    assert_eq!(
        feature.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
    assert_eq!(feature.divergence, None);
}

#[test]
fn list_branches_reflects_new_upstream_without_reopen() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let before = opened.list_branches().unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_before.upstream, None);

    opened.push_set_upstream("origin", "feature").unwrap();

    let after = opened.list_branches().unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(
        feature_after.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn list_branches_reflects_tracking_upstream_set_without_push() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "origin", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let before = opened.list_branches().unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_before.upstream, None);

    let output = opened
        .set_upstream_branch_with_output("feature", "origin/feature")
        .expect("set upstream");
    assert_eq!(output.exit_code, Some(0));

    let upstream_after = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    );
    assert_eq!(upstream_after.trim(), "origin/feature");

    let after = opened.list_branches().unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(
        feature_after.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn list_branches_reflects_repeated_tracking_toggles_on_same_repo_instance() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "origin", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let feature_upstream = |branches: &[gitcomet_core::domain::Branch]| {
        branches
            .iter()
            .find(|branch| branch.name == "feature")
            .expect("feature branch present")
            .upstream
            .clone()
    };

    assert_eq!(feature_upstream(&opened.list_branches().unwrap()), None);

    let output = opened
        .set_upstream_branch_with_output("feature", "origin/feature")
        .expect("set upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(&opened.list_branches().unwrap()),
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );

    let output = opened
        .unset_upstream_branch_with_output("feature")
        .expect("unset upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(feature_upstream(&opened.list_branches().unwrap()), None);

    let output = opened
        .set_upstream_branch_with_output("feature", "origin/feature")
        .expect("restore upstream");
    assert_eq!(output.exit_code, Some(0));
    assert_eq!(
        feature_upstream(&opened.list_branches().unwrap()),
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );
}

#[test]
fn list_branches_preserves_nested_upstream_branch_names() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "main"]);

    run_git(&work_repo, &["checkout", "-b", "feature/nested/name"]);
    fs::write(work_repo.join("nested.txt"), "nested\n").unwrap();
    run_git(&work_repo, &["add", "nested.txt"]);
    run_git(
        &work_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "nested feature",
        ],
    );
    run_git(&work_repo, &["push", "-u", "origin", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();
    let branches = opened.list_branches().unwrap();
    let feature = branches
        .iter()
        .find(|branch| branch.name == "feature/nested/name")
        .expect("nested feature branch present");

    assert_eq!(
        feature.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature/nested/name".to_string(),
        })
    );
    assert_eq!(
        feature.divergence,
        Some(UpstreamDivergence {
            ahead: 0,
            behind: 0,
        })
    );
}

#[test]
fn list_branches_reflects_removed_upstream_without_reopen() {
    if !require_git_shell_for_refs_integration_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let remote_repo = root.join("remote.git");
    let work_repo = root.join("work");
    fs::create_dir_all(&remote_repo).unwrap();
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&remote_repo, &["init", "--bare", "-b", "main"]);

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    let origin_url = git_remote_url(&remote_repo);
    run_git(
        &work_repo,
        &["remote", "add", "origin", origin_url.as_str()],
    );

    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    run_git(&work_repo, &["checkout", "-b", "feature"]);
    fs::write(work_repo.join("feature.txt"), "feature\n").unwrap();
    run_git(&work_repo, &["add", "feature.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "feature"],
    );
    run_git(&work_repo, &["push", "-u", "origin", "feature"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).unwrap();

    let before = opened.list_branches().unwrap();
    let feature_before = before
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(
        feature_before.upstream,
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        })
    );

    let output = opened
        .unset_upstream_branch_with_output("feature")
        .expect("unset upstream");
    assert_eq!(output.exit_code, Some(0));

    let upstream_after = run_git_capture(
        &work_repo,
        &[
            "for-each-ref",
            "--format=%(upstream:short)",
            "refs/heads/feature",
        ],
    );
    assert!(
        upstream_after.trim().is_empty(),
        "expected feature to have no upstream after unlink: {upstream_after:?}"
    );

    let after = opened.list_branches().unwrap();
    let feature_after = after
        .iter()
        .find(|branch| branch.name == "feature")
        .expect("feature branch present");
    assert_eq!(feature_after.upstream, None);
}
