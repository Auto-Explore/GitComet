//! End-to-end coverage for committing with a passphrase-protected SSH signing
//! key (`gpg.format = ssh`).
//!
//! Git signs such commits by shelling out to `ssh-keygen -Y sign`, which asks for
//! the key passphrase through `SSH_ASKPASS`. When nothing is staged to answer it,
//! ssh-keygen fails with `Load key "<path>": incorrect passphrase supplied to
//! decrypt private key` — wording that names neither a prompt nor a remote. These
//! tests pin down that the failure still carries enough for the reducer to open a
//! passphrase prompt, and that replaying the commit with a staged passphrase
//! succeeds.

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

/// `ssh-keygen` is not guaranteed on every build machine, and Git needs to be new
/// enough to support SSH signing at all.
fn ssh_signing_available() -> bool {
    Command::new("ssh-keygen")
        .arg("-?")
        .output()
        .is_ok_and(|out| {
            let text = String::from_utf8_lossy(&out.stderr);
            // `-Y sign` is the signing subcommand; older builds lack it.
            text.contains("-Y") || String::from_utf8_lossy(&out.stdout).contains("-Y")
        })
}

/// A repo configured to sign every commit with a passphrase-protected key.
fn init_signing_repo(dir: &Path) -> std::path::PathBuf {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).expect("create repo directory");

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
        .expect("generate ssh signing key");
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
fn ssh_signed_commit_without_staged_passphrase_reports_a_passphrase_failure() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` not available");
        return;
    }
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();

    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = init_signing_repo(dir.path());

    let err = open(&repo)
        .commit("signed work")
        .expect_err("commit should fail without the signing key passphrase");

    let ErrorKind::Git(failure) = err.kind() else {
        panic!("expected a structured git failure, got {:?}", err.kind());
    };
    let stderr = String::from_utf8_lossy(failure.stderr());

    // The askpass helper saw the prompt; the git layer replays it so the failure
    // is classifiable no matter how OpenSSH worded the error.
    assert!(
        stderr.contains(SSH_PASSPHRASE_PROMPT_MARKER),
        "expected the replayed passphrase prompt in stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("Enter passphrase for"),
        "expected the real ssh-keygen prompt text, got: {stderr}"
    );
}

#[test]
fn ssh_signed_commit_succeeds_with_a_staged_passphrase() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` not available");
        return;
    }
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();

    let dir = tempfile::tempdir().expect("create tempdir");
    let repo = init_signing_repo(dir.path());

    // Pinned to this thread so the sibling test, which must find nothing staged,
    // cannot consume it when the two run in parallel.
    stage_git_auth_for_current_thread(StagedGitAuth {
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.to_string(),
    });

    open(&repo)
        .commit("signed work")
        .expect("commit should succeed once the passphrase is staged");
    clear_staged_git_auth();

    // Read the raw commit object rather than `%G?`: verifying an SSH signature
    // additionally needs `gpg.ssh.allowedSignersFile`, and without it git reports
    // an otherwise perfectly signed commit as unsigned.
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(&repo)
        .args(["cat-file", "commit", "HEAD"]);
    test_git_env::apply(&mut cmd);
    let out = cmd.output().expect("read commit object");
    let object = String::from_utf8_lossy(&out.stdout);
    assert!(
        object.contains("gpgsig -----BEGIN SSH SIGNATURE-----"),
        "commit should carry an SSH signature, got: {object}"
    );

    // A successful command caches the passphrase against the exact prompt text,
    // so the next signed commit in this session must not ask again.
    let cached = load_session_passphrases();
    assert!(
        cached.iter().any(|e| e.prompt.contains("Enter passphrase")),
        "expected the ssh-keygen prompt to be cached, got: {cached:?}"
    );

    fs::write(repo.join("file.txt"), "more contents").expect("write file");
    run_git(&repo, &["add", "."]);
    open(&repo)
        .commit("second signed commit")
        .expect("second commit should reuse the cached passphrase without prompting");

    clear_session_passphrase();
}
