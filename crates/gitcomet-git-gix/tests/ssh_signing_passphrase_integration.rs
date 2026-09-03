//! End-to-end coverage for session reuse of an SSH signing-key passphrase.

use gitcomet_core::auth::{
    GitAuthKind, SSH_PASSPHRASE_PROMPT_MARKER, StagedGitAuth, clear_session_passphrase,
    clear_staged_git_auth, load_session_passphrases, stage_git_auth_for_current_thread,
};
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "support/test_git_env.rs"]
mod test_git_env;

const PASSPHRASE: &str = "correct horse battery staple";

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    test_git_env::apply(&mut cmd);
    let output = cmd.output().expect("run git command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn ssh_signing_available() -> bool {
    Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_ok_and(|output| {
            String::from_utf8_lossy(&output.stderr).contains("-Y")
                || String::from_utf8_lossy(&output.stdout).contains("-Y")
        })
}

fn init_signing_repo(dir: &Path) -> std::path::PathBuf {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).expect("create repository directory");
    let key = dir.join("signing_key");

    let status = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-q",
            "-C",
            "gitcomet-test",
            "-N",
            PASSPHRASE,
        ])
        .arg("-f")
        .arg(&key)
        .status()
        .expect("generate SSH signing key");
    assert!(status.success(), "ssh-keygen key generation failed");

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.name", "Test"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "gpg.format", "ssh"]);
    run_git(
        &repo,
        &["config", "user.signingkey", &key.display().to_string()],
    );
    run_git(&repo, &["config", "commit.gpgsign", "true"]);
    fs::write(repo.join("file.txt"), "contents").expect("write file");
    run_git(&repo, &["add", "."]);
    repo
}

fn open(repo: &Path) -> std::sync::Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

#[test]
fn missing_ssh_signing_passphrase_preserves_the_observed_prompt() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("create temp directory");
    let repo = init_signing_repo(dir.path());

    let error = open(&repo)
        .commit("signed work")
        .expect_err("commit should need a passphrase");
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected structured Git failure, got {:?}", error.kind());
    };
    let stderr = String::from_utf8_lossy(failure.stderr());
    assert!(stderr.contains(SSH_PASSPHRASE_PROMPT_MARKER));
    assert!(
        stderr.contains("Enter passphrase"),
        "expected the observed passphrase prompt in stderr, got:\n{stderr}"
    );
}

#[test]
fn successful_ssh_signing_passphrase_is_reused_for_the_session() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("create temp directory");
    let repo = init_signing_repo(dir.path());

    stage_git_auth_for_current_thread(StagedGitAuth {
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.to_string(),
    });
    open(&repo)
        .commit("first signed commit")
        .expect("first signed commit should accept the staged passphrase");
    clear_staged_git_auth();
    let cached = load_session_passphrases();
    let cached_prompt = cached
        .iter()
        .find(|entry| entry.prompt.contains("Enter passphrase"))
        .expect("successful signing should cache its passphrase prompt");
    assert!(!cached_prompt.prompt.contains("cmd.exe"));
    assert!(!cached_prompt.prompt.contains("gitcomet-askpass.cmd"));

    fs::write(repo.join("file.txt"), "more contents").expect("update file");
    run_git(&repo, &["add", "."]);
    open(&repo)
        .commit("second signed commit")
        .expect("second signed commit should reuse the session passphrase");

    clear_session_passphrase();
}
