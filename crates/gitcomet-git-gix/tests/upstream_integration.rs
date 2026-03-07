use gitcomet_core::services::{GitBackend, PullMode};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;
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

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_upstream_tests() -> bool {
    static AVAILABLE: OnceLock<bool> = OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        let output = match Command::new("git").args(["difftool", "--tool-help"]).output() {
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
