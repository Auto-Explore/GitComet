use gitcomet_core::domain::UpstreamDivergence;
use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

fn run_git(repo: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git command to run");
    assert!(
        output.status.success(),
        "git {:?} failed:\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

#[test]
fn upstream_divergence_reports_ahead_and_behind_counts() {
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

    fs::write(work_repo.join("file.txt"), "base\nlocal ahead\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "local ahead"],
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
    fs::write(peer_repo.join("peer.txt"), "remote ahead\n").unwrap();
    run_git(&peer_repo, &["add", "peer.txt"]);
    run_git(
        &peer_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "remote ahead"],
    );
    run_git(&peer_repo, &["push", "origin", "main"]);

    run_git(&work_repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open repository");
    let divergence = opened.upstream_divergence().expect("read divergence");

    assert_eq!(
        divergence,
        Some(UpstreamDivergence {
            ahead: 1,
            behind: 1
        })
    );
}

#[test]
fn upstream_divergence_returns_none_when_branch_has_no_upstream() {
    let dir = tempfile::tempdir().unwrap();
    let work_repo = dir.path().join("work");
    fs::create_dir_all(&work_repo).unwrap();

    run_git(&work_repo, &["init", "-b", "main"]);
    run_git(&work_repo, &["config", "user.email", "you@example.com"]);
    run_git(&work_repo, &["config", "user.name", "You"]);
    run_git(&work_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(work_repo.join("file.txt"), "base\n").unwrap();
    run_git(&work_repo, &["add", "file.txt"]);
    run_git(
        &work_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open repository");
    let divergence = opened.upstream_divergence().expect("read divergence");

    assert_eq!(divergence, None);
}

#[test]
fn upstream_divergence_reflects_new_upstream_without_reopen() {
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
    let opened = backend.open(&work_repo).expect("open repository");

    assert_eq!(opened.upstream_divergence().expect("read divergence"), None);

    opened
        .push_set_upstream("origin", "feature")
        .expect("push branch and configure upstream");

    assert_eq!(
        opened
            .upstream_divergence()
            .expect("read divergence after push -u"),
        Some(UpstreamDivergence {
            ahead: 0,
            behind: 0,
        })
    );
}

#[test]
fn upstream_divergence_returns_none_on_detached_head() {
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
    run_git(&work_repo, &["checkout", "--detach", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open repository");
    let divergence = opened.upstream_divergence().expect("read divergence");

    assert_eq!(divergence, None);
}
