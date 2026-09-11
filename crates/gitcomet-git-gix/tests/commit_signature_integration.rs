//! End-to-end coverage for commit signature verification.
//!
//! Uses SSH signing rather than GPG: generating a throwaway ed25519 key is fast
//! and hermetic, while a GPG keypair needs entropy and an agent.

use gitcomet_core::domain::{CommitId, SignatureFormat, SignatureStatus};
use gitcomet_core::services::{GitBackend, GitRepository};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;

#[path = "support/test_git_env.rs"]
mod test_git_env;

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

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(repo).args(args);
    test_git_env::apply(&mut cmd);
    let output = cmd.output().expect("run git command");
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
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

struct SigningRepo {
    repo: std::path::PathBuf,
    allowed_signers: std::path::PathBuf,
}

/// A repository that signs every commit with a throwaway SSH key. The allowed
/// signers file is written but not yet configured, so the caller chooses whether
/// signatures are verifiable.
fn init_signing_repo(dir: &Path) -> SigningRepo {
    let repo = dir.join("repo");
    fs::create_dir_all(&repo).expect("create repository directory");
    let key = dir.join("signing_key");

    let status = Command::new("ssh-keygen")
        .args(["-t", "ed25519", "-q", "-C", "gitcomet-test", "-N", ""])
        .arg("-f")
        .arg(&key)
        .status()
        .expect("generate SSH signing key");
    assert!(status.success(), "ssh-keygen key generation failed");

    let public_key = fs::read_to_string(key.with_extension("pub")).expect("read public key");
    let allowed_signers = dir.join("allowed_signers");
    fs::write(
        &allowed_signers,
        format!("test@example.com {}", public_key.trim()),
    )
    .expect("write allowed signers");

    run_git(&repo, &["init"]);
    run_git(&repo, &["config", "user.name", "Test"]);
    run_git(&repo, &["config", "user.email", "test@example.com"]);
    run_git(&repo, &["config", "gpg.format", "ssh"]);
    run_git(
        &repo,
        &["config", "user.signingkey", &key.display().to_string()],
    );
    run_git(&repo, &["config", "commit.gpgsign", "true"]);

    SigningRepo {
        repo,
        allowed_signers,
    }
}

fn trust_signatures(fixture: &SigningRepo) {
    run_git(
        &fixture.repo,
        &[
            "config",
            "gpg.ssh.allowedSignersFile",
            &fixture.allowed_signers.display().to_string(),
        ],
    );
}

fn commit(repo: &Path, name: &str, signed: bool) -> CommitId {
    fs::write(repo.join(name), name).expect("write file");
    run_git(repo, &["add", "."]);
    if signed {
        run_git(repo, &["commit", "-m", name]);
    } else {
        run_git(repo, &["commit", "--no-gpg-sign", "-m", name]);
    }
    CommitId(git_stdout(repo, &["rev-parse", "HEAD"]).into())
}

fn open(repo: &Path) -> std::sync::Arc<dyn GitRepository> {
    GixBackend.open(repo).expect("open repository")
}

#[test]
fn a_trusted_ssh_signature_verifies_and_an_unsigned_commit_earns_no_entry() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y verify` is unavailable");
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);

    let signed = commit(&fixture.repo, "signed.txt", true);
    let unsigned = commit(&fixture.repo, "unsigned.txt", false);

    let repo = open(&fixture.repo);
    let results = repo
        .verify_commit_signatures(&[signed.clone(), unsigned.clone()])
        .expect("verify signatures");

    assert_eq!(
        results.len(),
        1,
        "only the signed commit should earn an entry, got {results:?}"
    );
    let (id, signature) = &results[0];
    assert_eq!(id, &signed);
    assert_eq!(signature.format, SignatureFormat::Ssh);
    assert!(
        signature.status.is_verified(),
        "expected a verified status, got {:?}",
        signature.status
    );
}

#[test]
fn an_untrusted_ssh_signature_earns_no_entry() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y verify` is unavailable");
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    // No allowed-signers file: git can see the signature but cannot judge it.

    let signed = commit(&fixture.repo, "signed.txt", true);

    let repo = open(&fixture.repo);
    let results = repo
        .verify_commit_signatures(&[signed])
        .expect("verify signatures");

    assert!(
        results.is_empty(),
        "an unverifiable signature must earn no badge, got {results:?}"
    );
}

#[test]
fn verification_rechecks_a_key_added_to_the_trust_configuration() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    let signed = commit(&fixture.repo, "signed.txt", true);
    let repo = open(&fixture.repo);
    let ids = [signed];
    assert!(repo.verify_commit_signatures(&ids).unwrap().is_empty());

    trust_signatures(&fixture);
    assert_eq!(
        git_stdout(&fixture.repo, &["log", "-1", "--format=%G?"]),
        "G"
    );
    let results = repo.verify_commit_signatures(&ids).unwrap();
    assert_eq!(results.len(), 1, "a missing key must not be cached forever");
    assert_eq!(results[0].1.status, SignatureStatus::Good);
}

#[test]
fn verification_rechecks_changes_to_an_existing_revocation_file() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let revoked = dir.path().join("revoked_keys");
    fs::write(&revoked, "").unwrap();
    run_git(
        &fixture.repo,
        &[
            "config",
            "gpg.ssh.revocationFile",
            revoked.to_str().unwrap(),
        ],
    );
    let signed = commit(&fixture.repo, "signed.txt", true);
    let repo = open(&fixture.repo);
    let ids = [signed];
    assert_eq!(
        repo.verify_commit_signatures(&ids).unwrap()[0].1.status,
        SignatureStatus::Good
    );

    // The config and commit stay identical; only the trust file's contents change.
    fs::copy(dir.path().join("signing_key.pub"), &revoked).unwrap();
    assert_eq!(
        git_stdout(&fixture.repo, &["log", "-1", "--format=%G?"]),
        "B"
    );
    assert_eq!(
        repo.verify_commit_signatures(&ids).unwrap()[0].1.status,
        SignatureStatus::Bad
    );

    fs::write(&revoked, "").unwrap();
    assert_eq!(
        repo.verify_commit_signatures(&ids).unwrap()[0].1.status,
        SignatureStatus::Good
    );
}

#[test]
fn verification_drops_a_badge_when_verification_becomes_unavailable() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let signed = commit(&fixture.repo, "signed.txt", true);
    let repo = open(&fixture.repo);
    let ids = [signed];
    assert_eq!(
        repo.verify_commit_signatures(&ids).unwrap()[0].1.status,
        SignatureStatus::Good
    );

    run_git(
        &fixture.repo,
        &["config", "--unset", "gpg.ssh.allowedSignersFile"],
    );
    assert!(matches!(
        git_stdout(&fixture.repo, &["log", "-1", "--format=%G?"]).as_str(),
        "E" | "N"
    ));
    assert!(repo.verify_commit_signatures(&ids).unwrap().is_empty());
}

#[test]
fn results_preserve_input_order_across_repeat_verification() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y verify` is unavailable");
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);

    let first = commit(&fixture.repo, "a.txt", true);
    let second = commit(&fixture.repo, "b.txt", true);
    let third = commit(&fixture.repo, "c.txt", true);

    let repo = open(&fixture.repo);
    let ids = [third.clone(), first.clone(), second.clone()];
    let results = repo
        .verify_commit_signatures(&ids)
        .expect("verify signatures");

    assert_eq!(
        results.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        vec![&third, &first, &second],
        "git log reorders by date; the backend must restore input order"
    );

    // Rechecking signed commits must preserve the same order and verdicts.
    let cached = repo
        .verify_commit_signatures(&ids)
        .expect("verify signatures again");
    assert_eq!(cached, results);
}

#[test]
fn a_tampered_commit_reports_a_bad_signature() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y verify` is unavailable");
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);

    let signed = commit(&fixture.repo, "signed.txt", true);

    // Re-wrap the signed commit around a different message, keeping the original
    // signature header: the payload no longer matches what was signed.
    let raw = git_stdout(&fixture.repo, &["cat-file", "commit", signed.as_ref()]);
    let tampered_raw = format!("{raw}\ntampered\n");
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(&fixture.repo)
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped());
    test_git_env::apply(&mut cmd);
    let mut child = cmd.spawn().expect("spawn git hash-object");
    {
        use std::io::Write as _;
        child
            .stdin
            .as_mut()
            .expect("stdin")
            .write_all(tampered_raw.as_bytes())
            .expect("write commit object");
    }
    let output = child.wait_with_output().expect("hash-object output");
    assert!(output.status.success(), "git hash-object failed");
    let tampered = CommitId(
        String::from_utf8_lossy(&output.stdout)
            .trim()
            .to_string()
            .into(),
    );

    let repo = open(&fixture.repo);
    let results = repo
        .verify_commit_signatures(&[tampered])
        .expect("verify signatures");

    assert_eq!(results.len(), 1, "a bad signature still earns a badge");
    assert_eq!(results[0].1.status, SignatureStatus::Bad);
}

#[test]
fn log_show_signature_does_not_corrupt_the_batch_parse() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y verify` is unavailable");
        return;
    }
    let dir = tempfile::tempdir().expect("create tempdir");
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    // With this set, `git log` prints gpg's prose to stdout before the records.
    run_git(&fixture.repo, &["config", "log.showSignature", "true"]);

    let signed = commit(&fixture.repo, "signed.txt", true);

    let repo = open(&fixture.repo);
    let results = repo
        .verify_commit_signatures(&[signed.clone()])
        .expect("verify signatures");

    assert_eq!(
        results.len(),
        1,
        "log.showSignature must not swallow the record, got {results:?}"
    );
    assert!(results[0].1.status.is_verified());
}

#[test]
fn verification_forces_utf8_despite_log_output_encoding() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let signed = commit(&fixture.repo, "signed.txt", true);
    run_git(
        &fixture.repo,
        &["config", "i18n.logOutputEncoding", "UTF-16"],
    );
    let result = open(&fixture.repo)
        .verify_commit_signatures(&[signed.clone()])
        .unwrap();
    assert_eq!(
        result.len(),
        1,
        "output encoding must not swallow the record"
    );
    assert_eq!(result[0].0, signed);
    assert_eq!(result[0].1.status, SignatureStatus::Good);
}

#[cfg(unix)]
fn write_program(path: &Path, contents: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, contents).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
#[test]
fn unavailable_ssh_verifiers_do_not_claim_the_signature_is_bad() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let signed = commit(&fixture.repo, "signed.txt", true);
    let signing_only = dir.path().join("signing-only");
    write_program(
        &signing_only,
        "#!/bin/sh\necho 'This helper only supports signing' >&2\nexit 1\n",
    );
    let repo = open(&fixture.repo);
    for program in [
        dir.path().join("missing-verifier"),
        "/bin/false".into(),
        signing_only,
    ] {
        run_git(
            &fixture.repo,
            &["config", "gpg.ssh.program", program.to_str().unwrap()],
        );
        let result = repo.verify_commit_signatures(&[signed.clone()]).unwrap();
        assert!(
            result.is_empty(),
            "an unavailable verifier {program:?} must omit the badge, got {result:?}"
        );
    }
}

#[test]
fn an_ssh_key_absent_from_allowed_signers_is_untrusted() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let signed = commit(&fixture.repo, "signed.txt", true);
    fs::write(&fixture.allowed_signers, "").unwrap();
    assert_eq!(
        git_stdout(&fixture.repo, &["log", "-1", "--format=%G?"]),
        "U"
    );
    let result = open(&fixture.repo)
        .verify_commit_signatures(&[signed])
        .unwrap();
    assert_eq!(result[0].1.status, SignatureStatus::GoodUncertified);
    assert!(!result[0].1.status.is_verified());
}

#[cfg(unix)]
#[test]
fn slow_signature_batches_make_progress_in_bounded_chunks() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let ids: Vec<_> = (0..80)
        .map(|ix| commit(&fixture.repo, &format!("signed-{ix}"), true))
        .collect();
    let verifier = dir.path().join("slow-verifier");
    write_program(&verifier, "#!/bin/sh\nsleep 0.08\nexec ssh-keygen \"$@\"\n");
    run_git(
        &fixture.repo,
        &["config", "gpg.ssh.program", verifier.to_str().unwrap()],
    );
    let result = open(&fixture.repo)
        .verify_commit_signatures(&ids)
        .expect("bounded verification must make progress");
    assert_eq!(
        result.iter().map(|(id, _)| id).collect::<Vec<_>>(),
        ids.iter().collect::<Vec<_>>()
    );
}

#[cfg(unix)]
#[test]
fn cancellation_stops_a_running_signature_verifier() {
    if !ssh_signing_available() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let fixture = init_signing_repo(dir.path());
    trust_signatures(&fixture);
    let id = commit(&fixture.repo, "signed.txt", true);
    let verifier = dir.path().join("blocked-verifier");
    write_program(
        &verifier,
        "#!/bin/sh\ntouch verifier-started\nexec sleep 60\n",
    );
    run_git(
        &fixture.repo,
        &["config", "gpg.ssh.program", verifier.to_str().unwrap()],
    );
    let repo = open(&fixture.repo);
    let cancellation = gitcomet_core::services::CancellationToken::new();
    let worker_cancellation = cancellation.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        tx.send(repo.verify_commit_signatures_cancellable(&[id], &worker_cancellation))
            .unwrap();
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !fixture.repo.join("verifier-started").exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    cancellation.cancel();
    assert!(
        fixture.repo.join("verifier-started").exists(),
        "verifier must have started"
    );
    let error = rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .expect("cancellation must stop the child promptly")
        .unwrap_err();
    assert!(matches!(
        error.kind(),
        gitcomet_core::error::ErrorKind::Cancelled
    ));
    worker.join().unwrap();
}
