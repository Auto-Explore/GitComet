//! End-to-end coverage for session reuse of an SSH signing-key passphrase.

use gitcomet_core::auth::{
    AuthOutcome, GitAuthKind, SSH_PASSPHRASE_PROMPT_MARKER, StagedGitAuth,
    clear_session_passphrase, clear_staged_git_auth, load_session_passphrases,
    stage_git_auth_for_current_thread,
};
use gitcomet_core::domain::CommitId;
use gitcomet_core::error::ErrorKind;
use gitcomet_core::services::{GitBackend, GitRepository, SequencerState};
use gitcomet_git_gix::GixBackend;
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::{Mutex, MutexGuard};

use crate::test_git_env;

const PASSPHRASE: &str = "correct horse battery staple";

// These tests reset the process-wide staged-auth and session-passphrase slots.
fn auth_test_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

fn run_git(repo: &Path, args: &[&str]) {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
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
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    cmd.arg("-C").arg(repo).args(args);
    test_git_env::apply(&mut cmd);
    let output = cmd.output().expect("run git command");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_string()
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
    let _auth_guard = auth_test_lock();
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
    assert!(matches!(
        failure.auth_outcome(),
        Some(AuthOutcome::NeedsInput(_))
    ));
    assert!(stderr.contains(SSH_PASSPHRASE_PROMPT_MARKER));
    assert!(
        stderr.contains("Enter passphrase"),
        "expected the observed passphrase prompt in stderr, got:\n{stderr}"
    );
}

#[test]
fn independently_validated_ssh_signing_passphrase_is_reused_for_the_session() {
    use gitcomet_core::auth::{
        AuthChallenge, PassphraseValidation, remember_verified_passphrase, validate_passphrase,
    };
    use gitcomet_core::services::CancellationToken;

    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("create temp directory");
    let repo = init_signing_repo(dir.path());

    let missing = open(&repo)
        .commit("first signed commit")
        .expect_err("the signer must request its passphrase");
    let ErrorKind::Git(missing) = missing.kind() else {
        panic!("expected structured signing failure: {missing:?}");
    };
    let Some(AuthOutcome::NeedsInput(observed)) = missing.auth_outcome() else {
        panic!("expected the observed signing challenge: {missing:?}");
    };
    // This fixture knows the exact configured signer key. Preserve the
    // signer's actual prompt while binding validation to that known key;
    // some ssh-keygen versions use only a generic signing prompt.
    let mut challenge = AuthChallenge::from_prompt(
        GitAuthKind::Passphrase,
        format!(
            "Enter passphrase for \"{}\":",
            dir.path().join("signing_key").display()
        ),
    );
    challenge.prompt = observed.prompt.clone();
    let auth = StagedGitAuth {
        challenge: Some(challenge.clone()),
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.to_string(),
    };
    assert_eq!(
        validate_passphrase(&challenge, &auth, &CancellationToken::new()),
        PassphraseValidation::Verified,
    );
    assert!(remember_verified_passphrase(&challenge, &auth));
    stage_git_auth_for_current_thread(auth);
    open(&repo)
        .commit("first signed commit")
        .expect("first signed commit should accept the staged passphrase");
    clear_staged_git_auth();
    let cached = load_session_passphrases();
    let cached_prompt = cached
        .iter()
        .find(|entry| entry.prompt.contains("Enter passphrase"))
        .expect("independent validation should cache the signer's passphrase prompt");
    assert!(!cached_prompt.prompt.contains("cmd.exe"));
    assert!(!cached_prompt.prompt.contains("gitcomet-askpass.cmd"));

    fs::write(repo.join("file.txt"), "more contents").expect("update file");
    run_git(&repo, &["add", "."]);
    open(&repo)
        .commit("second signed commit")
        .expect("second signed commit should reuse the session passphrase");

    clear_session_passphrase();
}

#[test]
fn successful_signing_does_not_promote_an_unvalidated_staged_answer_to_cache() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("create temp directory");
    let repo = init_signing_repo(dir.path());
    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.into(),
    });
    open(&repo)
        .commit("signed commit")
        .expect("staged answer signs");
    clear_staged_git_auth();
    assert!(load_session_passphrases().is_empty());

    fs::write(repo.join("file.txt"), "another change").unwrap();
    run_git(&repo, &["add", "."]);
    let missing = open(&repo).commit("another signed commit").unwrap_err();
    let ErrorKind::Git(missing) = missing.kind() else {
        panic!("expected structured signing failure: {missing:?}");
    };
    assert!(matches!(
        missing.auth_outcome(),
        Some(AuthOutcome::NeedsInput(_))
    ));
    clear_session_passphrase();
}

#[test]
fn unlocked_ssh_key_survives_pull_refused_for_local_changes() {
    use gitcomet_core::auth::{
        PassphraseValidation, remember_verified_passphrase, validate_passphrase,
    };
    use gitcomet_core::error::GitFailureId;
    use gitcomet_core::services::{CancellationToken, PullMode};

    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::Builder::new().prefix("gc-").tempdir().unwrap();
    let source = init_signing_repo(dir.path());
    run_git(
        &source,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    let work = dir.path().join("work");
    run_git(
        dir.path(),
        &["clone", source.to_str().unwrap(), work.to_str().unwrap()],
    );
    fs::write(source.join("file.txt"), "remote update\n").unwrap();
    run_git(
        &source,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-am",
            "remote update",
        ],
    );
    fs::write(work.join("file.txt"), "local changes\n").unwrap();

    // The transport is local, but the app's real askpass helper must answer
    // before git-upload-pack can supply the objects to the real git pull.
    let shell_quote = |text: &str| format!("'{}'", text.replace('\'', "'\\''"));
    let key = dir.path().join("signing_key");
    let key_text = key.to_string_lossy().replace('\\', "/");
    let prompt = format!("Enter passphrase for key '{key_text}':");
    let shim = dir.path().join("ssh-transport.sh");
    fs::write(
        &shim,
        format!(
            "#!/bin/sh\nsecret=$(\"$SSH_ASKPASS\" {})\nif [ \"$secret\" != {} ]; then\n  echo 'fixture: Permission denied (publickey).' >&2\n  exit 255\nfi\nexec git-upload-pack {}\n",
            shell_quote(&prompt),
            shell_quote(PASSPHRASE),
            shell_quote(&source.to_string_lossy().replace('\\', "/")),
        ),
    )
    .unwrap();
    run_git(
        &work,
        &["config", "remote.origin.url", "git@fixture:repository"],
    );
    run_git(&work, &["config", "ssh.variant", "ssh"]);
    run_git(&work, &["config", "pull.rebase", "false"]);
    run_git(
        &work,
        &[
            "config",
            "core.sshCommand",
            &format!(
                "sh {}",
                shell_quote(&shim.to_string_lossy().replace('\\', "/"))
            ),
        ],
    );

    let repo = open(&work);
    let missing = repo.pull(PullMode::Default, true).unwrap_err();
    let ErrorKind::Git(missing) = missing.kind() else {
        panic!("expected structured failure: {missing:?}");
    };
    let Some(AuthOutcome::NeedsInput(challenge)) = missing.auth_outcome() else {
        panic!("expected a missing passphrase: {missing:?}");
    };
    let auth = StagedGitAuth {
        challenge: Some(challenge.clone()),
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.into(),
    };
    assert_eq!(
        validate_passphrase(challenge, &auth, &CancellationToken::new()),
        PassphraseValidation::Verified,
    );
    assert!(remember_verified_passphrase(challenge, &auth));
    stage_git_auth_for_current_thread(auth);

    let refused = repo.pull(PullMode::Default, true).unwrap_err();
    let ErrorKind::Git(refused) = refused.kind() else {
        panic!("expected structured failure: {refused:?}");
    };
    assert_eq!(refused.id(), GitFailureId::WorktreeWouldBeOverwritten);
    assert_eq!(refused.auth_outcome(), Some(&AuthOutcome::Verified));
    assert!(String::from_utf8_lossy(refused.stderr()).contains("would be overwritten"));
    assert!(!String::from_utf8_lossy(refused.stderr()).contains(PASSPHRASE));
    assert!(
        !String::from_utf8_lossy(refused.stderr()).contains(SSH_PASSPHRASE_PROMPT_MARKER),
        "a local refusal must not read as a passphrase request"
    );
    assert!(
        load_session_passphrases()
            .iter()
            .any(|entry| entry.prompt == prompt)
    );
    clear_staged_git_auth();

    fs::write(work.join("file.txt"), "contents").unwrap();
    repo.pull(PullMode::Default, true)
        .expect("a later pull reuses the unlocked key");
    assert_eq!(
        fs::read_to_string(work.join("file.txt")).unwrap(),
        "remote update\n"
    );

    // A merge conflict is reclassified by the repository layer; that
    // reconstruction must preserve the independent authentication result.
    run_git(&work, &["config", "user.name", "Test"]);
    run_git(&work, &["config", "user.email", "test@example.com"]);
    fs::write(work.join("file.txt"), "committed local edit\n").unwrap();
    run_git(&work, &["commit", "-am", "local edit"]);
    fs::write(source.join("file.txt"), "conflicting remote edit\n").unwrap();
    run_git(
        &source,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-am",
            "conflicting edit",
        ],
    );
    let conflict = repo.pull(PullMode::Default, true).unwrap_err();
    let ErrorKind::Git(conflict) = conflict.kind() else {
        panic!("expected structured conflict: {conflict:?}");
    };
    assert_eq!(conflict.id(), GitFailureId::StoppedAtConflicts);
    assert_eq!(conflict.auth_outcome(), Some(&AuthOutcome::Verified));
    clear_session_passphrase();
}

#[test]
fn revert_needing_a_signing_passphrase_resumes_when_replayed() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("create temp directory");
    let repo = init_signing_repo(dir.path());
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "base"],
    );
    fs::write(repo.join("file.txt"), "changed").expect("update file");
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "change"],
    );
    let change = git_stdout(&repo, &["rev-parse", "HEAD"]);

    let error = open(&repo)
        .revert_with_output(&CommitId(change.clone().into()), true, None)
        .expect_err("revert commit should need a passphrase");
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected structured Git failure, got {:?}", error.kind());
    };
    assert!(String::from_utf8_lossy(failure.stderr()).contains(SSH_PASSPHRASE_PROMPT_MARKER));
    assert_eq!(
        open(&repo)
            .sequencer_state(&gitcomet_core::services::CancellationToken::new())
            .unwrap(),
        SequencerState::Revert
    );

    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.to_string(),
    });
    // The auth retry replays the revert, which resumes at its commit step.
    let output = open(&repo)
        .revert_with_output(&CommitId(change.clone().into()), true, None)
        .expect("the replay should sign with the staged passphrase");
    clear_staged_git_auth();

    assert_eq!(output.command, format!("git revert {change}"));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD~1"]), change);
    assert!(git_stdout(&repo, &["cat-file", "-p", "HEAD"]).contains("gpgsig"));
    assert_eq!(
        open(&repo)
            .sequencer_state(&gitcomet_core::services::CancellationToken::new())
            .unwrap(),
        SequencerState::None
    );

    clear_session_passphrase();
}
