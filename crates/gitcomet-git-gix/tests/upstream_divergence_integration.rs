use gitcomet_core::domain::UpstreamDivergence;
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
fn git_local_push_available_for_upstream_divergence_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let dir = match tempfile::tempdir() {
            Ok(dir) => dir,
            Err(_) => return true,
        };
        let remote_repo = dir.path().join("probe-remote.git");
        let work_repo = dir.path().join("probe-work");
        if fs::create_dir_all(&remote_repo).is_err() || fs::create_dir_all(&work_repo).is_err() {
            return true;
        }

        for (repo, args) in [
            (&remote_repo, ["init", "--bare", "-b", "main"].as_slice()),
            (&work_repo, ["init", "-b", "main"].as_slice()),
            (
                &work_repo,
                ["config", "user.email", "you@example.com"].as_slice(),
            ),
            (&work_repo, ["config", "user.name", "You"].as_slice()),
            (&work_repo, ["config", "commit.gpgsign", "false"].as_slice()),
        ] {
            let status = match Command::new("git").arg("-C").arg(repo).args(args).status() {
                Ok(status) => status,
                Err(_) => return true,
            };
            if !status.success() {
                return true;
            }
        }

        if fs::write(work_repo.join("probe.txt"), "probe\n").is_err() {
            return true;
        }

        for args in [
            ["add", "probe.txt"].as_slice(),
            ["-c", "commit.gpgsign=false", "commit", "-m", "probe"].as_slice(),
        ] {
            let status = match Command::new("git")
                .arg("-C")
                .arg(&work_repo)
                .args(args)
                .status()
            {
                Ok(status) => status,
                Err(_) => return true,
            };
            if !status.success() {
                return true;
            }
        }

        let remote_url = git_remote_url(&remote_repo);
        let add_remote = match Command::new("git")
            .arg("-C")
            .arg(&work_repo)
            .args(["remote", "add", "origin", remote_url.as_str()])
            .status()
        {
            Ok(status) => status,
            Err(_) => return true,
        };
        if !add_remote.success() {
            return true;
        }

        let push_output = match Command::new("git")
            .arg("-C")
            .arg(&work_repo)
            .args(["push", "-u", "origin", "main"])
            .output()
        {
            Ok(output) => output,
            Err(_) => return true,
        };
        if push_output.status.success() {
            return true;
        }

        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&push_output.stdout),
            String::from_utf8_lossy(&push_output.stderr)
        );
        !is_git_shell_startup_failure(&text)
    })
}

fn require_git_local_push_for_upstream_divergence_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_local_push_available_for_upstream_divergence_tests() {
            eprintln!(
                "skipping upstream-divergence integration test: Git-for-Windows local push shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

#[test]
fn upstream_divergence_reports_ahead_and_behind_counts() {
    if !require_git_local_push_for_upstream_divergence_tests() {
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
    if !require_git_local_push_for_upstream_divergence_tests() {
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
    if !require_git_local_push_for_upstream_divergence_tests() {
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
    run_git(&work_repo, &["checkout", "--detach", "HEAD"]);

    let backend = GixBackend;
    let opened = backend.open(&work_repo).expect("open repository");
    let divergence = opened.upstream_divergence().expect("read divergence");

    assert_eq!(divergence, None);
}
