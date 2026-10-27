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

use crate::{auth_test_lock, test_git_env};

const PASSPHRASE: &str = "correct horse battery staple";

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

#[test]
fn reword_head_retries_signing_without_committing_staged_changes() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_signing_repo(dir.path());
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "Original"],
    );
    let target = CommitId(git_stdout(&repo, &["rev-parse", "HEAD"]).into());
    let tree = git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]);
    let author = git_stdout(&repo, &["log", "-1", "--format=%an|%ae|%aI"]);
    fs::write(repo.join("staged.txt"), "staged\n").unwrap();
    run_git(&repo, &["add", "staged.txt"]);
    fs::write(repo.join("file.txt"), "dirty\n").unwrap();
    let status = git_stdout(&repo, &["status", "--porcelain"]);
    let backend = open(&repo);
    let error = backend
        .reword_commit_with_output(&target, "Signed message")
        .unwrap_err();
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected a signing failure: {error}");
    };
    assert!(matches!(
        failure.auth_outcome(),
        Some(AuthOutcome::NeedsInput(_))
    ));
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), target.as_ref());
    assert!(
        !backend
            .reword_commit_in_progress(&target, "Signed message")
            .unwrap()
    );

    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.into(),
    });
    let outcome = backend
        .reword_commit_with_output(&target, "Signed message")
        .unwrap();
    clear_staged_git_auth();
    assert_eq!(
        outcome.rewritten_id,
        Some(CommitId(git_stdout(&repo, &["rev-parse", "HEAD"]).into()))
    );
    assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD^{tree}"]), tree);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%an|%ae|%aI"]),
        author
    );
    assert_eq!(git_stdout(&repo, &["status", "--porcelain"]), status);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Signed message"
    );
    assert!(git_stdout(&repo, &["cat-file", "-p", "HEAD"]).contains("gpgsig"));
    clear_session_passphrase();
}

#[test]
fn reword_signing_continuation_keeps_its_target_through_repeated_failures() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_signing_repo(dir.path());
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "Base"],
    );
    fs::write(repo.join("file.txt"), "target\n").unwrap();
    run_git(
        &repo,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-am",
            "Original target",
        ],
    );
    let target = CommitId(git_stdout(&repo, &["rev-parse", "HEAD"]).into());
    fs::write(repo.join("file.txt"), "descendant\n").unwrap();
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "Descendant"],
    );
    let original_head = CommitId(git_stdout(&repo, &["rev-parse", "HEAD"]).into());
    let before = git_stdout(&repo, &["log", "--format=%T|%an|%ae|%aI"]);
    let message = "Signed target\n\n# Message preserved";
    let backend = open(&repo);
    let error = backend
        .reword_commit_with_output(&target, message)
        .unwrap_err();
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected a signing failure: {error}");
    };
    assert!(matches!(
        failure.auth_outcome(),
        Some(AuthOutcome::NeedsInput(_))
    ));
    assert!(backend.reword_commit_in_progress(&target, message).unwrap());

    let paused_head = git_stdout(&repo, &["rev-parse", "HEAD"]);
    for (other_target, other_message) in [(&original_head, message), (&target, "Different message")]
    {
        assert!(
            !backend
                .reword_commit_in_progress(other_target, other_message)
                .unwrap()
        );
        let error = backend
            .continue_reword_commit_with_output(other_target, other_message)
            .unwrap_err();
        assert!(error.to_string().contains("does not belong"), "{error}");
        assert_eq!(git_stdout(&repo, &["rev-parse", "HEAD"]), paused_head);
    }

    fs::write(
        repo.join("file.txt"),
        "edit made while the signing prompt is open\n",
    )
    .unwrap();
    let error = backend
        .continue_reword_commit_with_output(&target, message)
        .unwrap_err();
    assert!(
        error.to_string().contains("stash your worktree changes"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(repo.join("file.txt")).unwrap(),
        "edit made while the signing prompt is open\n"
    );
    assert!(backend.reword_commit_in_progress(&target, message).unwrap());
    fs::write(repo.join("file.txt"), "target\n").unwrap();

    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: "wrong passphrase".into(),
    });
    let error = backend
        .continue_reword_commit_with_output(&target, message)
        .unwrap_err();
    clear_staged_git_auth();
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("expected the repeated signing failure: {error}");
    };
    assert!(matches!(
        failure.auth_outcome(),
        Some(AuthOutcome::Rejected(_))
    ));
    assert!(backend.reword_commit_in_progress(&target, message).unwrap());

    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.into(),
    });
    let outcome = backend
        .continue_reword_commit_with_output(&target, message)
        .unwrap();
    clear_staged_git_auth();
    let rewritten = CommitId(git_stdout(&repo, &["rev-parse", "HEAD^"]).into());
    assert_ne!(rewritten, target);
    assert_eq!(outcome.rewritten_id, Some(rewritten.clone()));
    assert_eq!(
        git_stdout(&repo, &["log", "--format=%T|%an|%ae|%aI"]),
        before
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD^"]),
        message
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Descendant"
    );
    assert!(git_stdout(&repo, &["cat-file", "-p", "HEAD^"]).contains("gpgsig"));
    assert!(!backend.reword_commit_in_progress(&target, message).unwrap());
    assert!(git_stdout(&repo, &["status", "--porcelain"]).is_empty());

    // Initially supplied credentials must survive the ancestry/listing
    // commands that run before the first signing subprocess too.
    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.into(),
    });
    let second = backend
        .reword_commit_with_output(&rewritten, "Signed again")
        .expect("preflight commands must not consume the staged signing answer");
    clear_staged_git_auth();
    assert_eq!(
        second.rewritten_id,
        Some(CommitId(git_stdout(&repo, &["rev-parse", "HEAD^"]).into()))
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD^"]),
        "Signed again"
    );
    clear_session_passphrase();
}

#[test]
fn reword_continue_keeps_a_later_signing_challenge_after_making_progress() {
    if !ssh_signing_available() {
        eprintln!("skipping: ssh-keygen with `-Y sign` is unavailable");
        return;
    }
    let _auth_guard = auth_test_lock();
    test_git_env::ensure_initialized();
    clear_staged_git_auth();
    clear_session_passphrase();
    let dir = tempfile::tempdir().unwrap();
    let repo = init_signing_repo(dir.path());
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-m", "Base"],
    );
    fs::write(repo.join("file.txt"), "target\n").unwrap();
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "Target"],
    );
    let target = CommitId(git_stdout(&repo, &["rev-parse", "HEAD"]).into());
    fs::write(repo.join("file.txt"), "descendant\n").unwrap();
    run_git(
        &repo,
        &["-c", "commit.gpgsign=false", "commit", "-am", "Descendant"],
    );
    let before = git_stdout(&repo, &["log", "--format=%T"]);

    const SECOND_PASSPHRASE: &str = "another fixture signing passphrase";
    let second_key = dir.path().join("second-key");
    assert!(
        Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", SECOND_PASSPHRASE, "-f"])
            .arg(&second_key)
            .status()
            .unwrap()
            .success()
    );
    let signer = dir.path().join("signer.sh");
    let quoted_key = gitcomet_core::text_utils::shell_single_quote(
        &second_key.to_string_lossy().replace('\\', "/"),
    );
    fs::write(&signer, format!(
        "#!/bin/sh\ncount_file=$(git rev-parse --git-path signing-attempt)\ncount=0\nif [ -f \"$count_file\" ]; then count=$(cat \"$count_file\"); fi\ncount=$((count + 1))\nprintf '%s' \"$count\" >\"$count_file\"\nif [ \"$count\" -lt 3 ]; then exec ssh-keygen \"$@\"; fi\nfor arg in \"$@\"; do input=$arg; done\nexec ssh-keygen -Y sign -n git -f {quoted_key} \"$input\"\n"
    )).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&signer, fs::Permissions::from_mode(0o755)).unwrap();
    }
    run_git(
        &repo,
        &[
            "config",
            "gpg.ssh.program",
            &signer.to_string_lossy().replace('\\', "/"),
        ],
    );
    let backend = open(&repo);
    let message = "Reworded target";
    backend
        .reword_commit_with_output(&target, message)
        .expect_err("target needs the first passphrase");
    assert!(backend.reword_commit_in_progress(&target, message).unwrap());
    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: PASSPHRASE.into(),
    });
    let error = backend
        .continue_reword_commit_with_output(&target, message)
        .expect_err("the descendant requests the second key after the target was committed");
    clear_staged_git_auth();
    let ErrorKind::Git(failure) = error.kind() else {
        panic!("{error}")
    };
    assert!(matches!(
        failure.auth_outcome(),
        Some(AuthOutcome::NeedsInput(_) | AuthOutcome::Rejected(_))
    ));
    assert!(
        backend.reword_commit_in_progress(&target, message).unwrap(),
        "the later signing failure must not roll back"
    );
    assert_eq!(git_stdout(&repo, &["log", "-1", "--format=%B"]), message);

    stage_git_auth_for_current_thread(StagedGitAuth {
        challenge: None,
        kind: GitAuthKind::Passphrase,
        username: None,
        secret: SECOND_PASSPHRASE.into(),
    });
    let outcome = backend
        .continue_reword_commit_with_output(&target, message)
        .unwrap();
    clear_staged_git_auth();
    assert_eq!(
        outcome.rewritten_id,
        Some(CommitId(git_stdout(&repo, &["rev-parse", "HEAD^"]).into()))
    );
    assert_eq!(git_stdout(&repo, &["log", "--format=%T"]), before);
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B", "HEAD^"]),
        message
    );
    assert_eq!(
        git_stdout(&repo, &["log", "-1", "--format=%B"]),
        "Descendant"
    );
    clear_session_passphrase();
}
