use gitcomet_core::services::GitBackend;
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

#[cfg(windows)]
fn is_git_shell_startup_failure(text: &str) -> bool {
    text.contains("sh.exe: *** fatal error -")
        && (text.contains("couldn't create signal pipe") || text.contains("CreateFileMapping"))
}

#[cfg(windows)]
fn git_shell_available_for_submodule_tests() -> bool {
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

fn require_git_shell_for_submodule_tests() -> bool {
    #[cfg(windows)]
    {
        if !git_shell_available_for_submodule_tests() {
            eprintln!(
                "skipping submodule integration test: Git-for-Windows shell startup failed in this environment"
            );
            return false;
        }
    }
    true
}

#[test]
fn list_submodules_ignores_missing_gitmodules_mapping() {
    if !require_git_shell_for_submodule_tests() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    let sub_repo = root.join("sub");
    let parent_repo = root.join("parent");
    fs::create_dir_all(&sub_repo).unwrap();
    fs::create_dir_all(&parent_repo).unwrap();

    run_git(&sub_repo, &["init"]);
    run_git(&sub_repo, &["config", "user.email", "you@example.com"]);
    run_git(&sub_repo, &["config", "user.name", "You"]);
    run_git(&sub_repo, &["config", "commit.gpgsign", "false"]);
    fs::write(sub_repo.join("file.txt"), "hi\n").unwrap();
    run_git(&sub_repo, &["add", "file.txt"]);
    run_git(
        &sub_repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

    run_git(&parent_repo, &["init"]);
    run_git(&parent_repo, &["config", "user.email", "you@example.com"]);
    run_git(&parent_repo, &["config", "user.name", "You"]);
    run_git(&parent_repo, &["config", "commit.gpgsign", "false"]);

    let status = Command::new("git")
        .arg("-C")
        .arg(&parent_repo)
        .arg("-c")
        .arg("protocol.file.allow=always")
        .arg("submodule")
        .arg("add")
        .arg(&sub_repo)
        .arg("submod")
        .status()
        .expect("git submodule add to run");
    assert!(status.success(), "git submodule add failed");

    run_git(
        &parent_repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "add submodule",
        ],
    );

    fs::write(parent_repo.join(".gitmodules"), "").unwrap();
    run_git(&parent_repo, &["add", ".gitmodules"]);

    let output = Command::new("git")
        .arg("-C")
        .arg(&parent_repo)
        .args(["submodule", "status", "--recursive"])
        .output()
        .expect("git submodule status to run");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("no submodule mapping found in .gitmodules for path"),
        "unexpected stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let backend = GixBackend;
    let opened = backend.open(&parent_repo).unwrap();

    let submodules = opened.list_submodules().unwrap();
    assert!(submodules.is_empty());
}
