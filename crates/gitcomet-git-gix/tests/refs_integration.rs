use gitcomet_core::domain::{Upstream, UpstreamDivergence};
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

#[test]
fn list_branches_reports_upstream_and_divergence() {
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
            remote_repo.to_str().expect("remote path"),
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
    run_git(
        &work_repo,
        &[
            "remote",
            "add",
            "origin",
            remote_repo.to_str().expect("remote path"),
        ],
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
