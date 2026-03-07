use gitcomet_core::services::GitBackend;
use gitcomet_git_gix::GixBackend;
use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

#[cfg(windows)]
const NULL_DEVICE: &str = "NUL";
#[cfg(not(windows))]
const NULL_DEVICE: &str = "/dev/null";

fn run_git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", NULL_DEVICE)
        .env("GIT_CONFIG_SYSTEM", NULL_DEVICE)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true")
        .status()
        .expect("git command to run");
    assert!(status.success(), "git {:?} failed", args);
}

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_integration_tests() -> bool {
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

fn require_git_shell_for_remote_tracking_test() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_integration_tests() {
            eprintln!(
                "skipping remote-tracking integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

fn git_remote_url(path: &Path) -> String {
    if cfg!(windows) {
        // Use a file:// URL so drive-letter paths are never treated as
        // scp-style host:path remotes.
        let normalized = path.to_string_lossy().replace('\\', "/");
        format!("file:///{normalized}")
    } else {
        path.to_string_lossy().into_owned()
    }
}

#[test]
fn log_all_branches_includes_remote_tracking_branches() {
    if !require_git_shell_for_remote_tracking_test() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path().join("repo");
    let origin = dir.path().join("origin.git");

    std::fs::create_dir_all(&repo).unwrap();
    run_git(&repo, &["init", "-b", "main"]);
    run_git(&repo, &["config", "user.email", "you@example.com"]);
    run_git(&repo, &["config", "user.name", "You"]);
    run_git(&repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(&repo, &["add", "a.txt"]);
    run_git(&repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(&repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(&repo, &["add", "b.txt"]);
    run_git(&repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);
    let feature_tip = {
        let out = Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git rev-parse to run");
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    run_git(
        dir.path(),
        &["init", "--bare", "-b", "main", origin.to_str().unwrap()],
    );
    let origin_url = git_remote_url(&origin);
    run_git(&repo, &["remote", "add", "origin", origin_url.as_str()]);
    run_git(&repo, &["push", "-u", "origin", "feature"]);

    run_git(&repo, &["checkout", "main"]);
    run_git(&repo, &["branch", "-D", "feature"]);
    run_git(&repo, &["fetch", "origin"]);

    let backend = GixBackend;
    let opened = backend.open(&repo).unwrap();

    let head = opened.log_head_page(200, None).unwrap();
    assert!(
        !head.commits.iter().any(|c| c.id.0 == feature_tip),
        "head log unexpectedly contains feature commit"
    );

    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        all.commits.iter().any(|c| c.id.0 == feature_tip),
        "all-branches log should include remote-tracking branch commit"
    );
}

#[test]
fn log_all_branches_includes_nonstandard_ref_namespaces() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(repo, &["checkout", "-b", "feature"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "C"]);
    let feature_tip = {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git rev-parse to run");
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["branch", "-D", "feature"]);
    run_git(
        repo,
        &[
            "update-ref",
            "refs/branch-heads/feature",
            feature_tip.as_str(),
        ],
    );

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        all.commits.iter().any(|c| c.id.0 == feature_tip),
        "all-branches log should include commits reachable from refs outside refs/heads and refs/remotes"
    );
}

#[test]
fn log_all_branches_does_not_include_tag_only_tips() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);
    run_git(repo, &["config", "user.email", "you@example.com"]);
    run_git(repo, &["config", "user.name", "You"]);
    run_git(repo, &["config", "commit.gpgsign", "false"]);

    std::fs::write(repo.join("a.txt"), "one\n").unwrap();
    run_git(repo, &["add", "a.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "A"]);

    run_git(repo, &["checkout", "-b", "tag-only"]);
    std::fs::write(repo.join("b.txt"), "two\n").unwrap();
    run_git(repo, &["add", "b.txt"]);
    run_git(repo, &["-c", "commit.gpgsign=false", "commit", "-m", "B"]);
    let tag_only_tip = {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["rev-parse", "HEAD"])
            .output()
            .expect("git rev-parse to run");
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };

    run_git(
        repo,
        &[
            "-c",
            "tag.gpgSign=false",
            "tag",
            "-a",
            "-m",
            "tag",
            "v0.0",
            tag_only_tip.as_str(),
        ],
    );
    run_git(repo, &["checkout", "main"]);
    run_git(repo, &["branch", "-D", "tag-only"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();
    let all = opened.log_all_branches_page(200, None).unwrap();
    assert!(
        !all.commits.iter().any(|c| c.id.0 == tag_only_tip),
        "all-branches log should not be expanded by tag-only tips"
    );
}

#[test]
fn empty_repo_log_and_head_branch_do_not_error() {
    let dir = tempfile::tempdir().unwrap();
    let repo = dir.path();

    run_git(repo, &["init", "-b", "main"]);

    let backend = GixBackend;
    let opened = backend.open(repo).unwrap();

    assert_eq!(opened.current_branch().unwrap(), "main");
    assert!(opened.log_head_page(200, None).unwrap().commits.is_empty());
    assert!(opened
        .log_all_branches_page(200, None)
        .unwrap()
        .commits
        .is_empty());
}
