use super::*;
use crate::auth::{
    clear_session_passphrase, remember_session_passphrase, remember_verified_passphrase,
};
use crate::services::CancellationToken;
use std::process::Stdio;

fn auth_test_lock() -> std::sync::MutexGuard<'static, ()> {
    super::super::tests::auth_test_lock()
}

fn challenge(prompt: &str) -> AuthChallenge {
    AuthChallenge::from_prompt(GitAuthKind::Passphrase, normalized_prompt(prompt))
}

fn explicit_auth(challenge: &AuthChallenge, secret: &str) -> PromptAuth {
    PromptAuth::Explicit(StagedGitAuth {
        kind: challenge.kind,
        username: None,
        secret: secret.to_string(),
        challenge: Some(challenge.clone()),
    })
}

/// Runs the freshly written helper. A test forking on another thread while
/// `create_askpass_script` writes the script keeps that writable fd until its
/// child execs, and exec'ing the script meanwhile fails with ETXTBSY.
/// https://github.com/rust-lang/rust/issues/114554
fn run_helper(command: &mut Command) -> std::process::Output {
    for _ in 0..500 {
        match command.output() {
            Err(error) if error.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result.expect("run real askpass helper"),
        }
    }
    panic!("askpass helper stayed busy");
}

fn answer(askpass: &AskPassScript, auth: Option<&PromptAuth>, prompt: &str) -> String {
    let mut command = Command::new(askpass.path());
    command.arg(prompt).stdin(Stdio::null());
    respond(command, askpass, auth)
}

/// [`answer`] through [`raw_prompt_command`], as Git for Windows runs the
/// helper. `Command` turns `%` into `%%cd:~,%` for a batch file, which the
/// helper reads back verbatim, so a percent-encoded username never matched.
fn answer_raw(askpass: &AskPassScript, auth: Option<&PromptAuth>, prompt: &str) -> String {
    respond(raw_prompt_command(askpass, prompt), askpass, auth)
}

fn respond(mut command: Command, askpass: &AskPassScript, auth: Option<&PromptAuth>) -> String {
    configure_git_auth_prompt(&mut command, auth, askpass);
    let output = run_helper(&mut command);
    assert!(output.status.success(), "askpass helper did not complete");
    assert!(output.stderr.is_empty(), "askpass helper wrote an error");
    String::from_utf8(output.stdout)
        .expect("UTF-8 response")
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

#[test]
fn missing_helper_answer_requests_input_but_unused_credentials_do_not() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let askpass = create_askpass_script().expect("askpass helper");
    let request = challenge("Enter passphrase for key 'missing-test-key':");
    assert_eq!(
        resolve_auth_outcome(None, &askpass, b"Permission denied (publickey).", false),
        AuthOutcome::NotRequested,
    );
    let unused_auth = explicit_auth(&request, "unused-secret");
    assert_eq!(
        resolve_auth_outcome(Some(&unused_auth), &askpass, b"local failure", false),
        AuthOutcome::NotRequested,
        "configured credentials have not necessarily been consumed",
    );
    assert_eq!(answer(&askpass, None, &request.prompt), "");
    assert_eq!(
        resolve_auth_outcome(None, &askpass, b"Permission denied (publickey).", false),
        AuthOutcome::NeedsInput(request),
    );
    assert_eq!(
        resolve_auth_outcome(None, &askpass, b"", true),
        AuthOutcome::NotRequested,
        "an unused identity does not turn a successful operation into an auth challenge",
    );
    clear_session_passphrase();
}

#[test]
fn supplied_answer_is_not_rejected_by_a_later_merge_or_remote_access_failure() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let askpass = create_askpass_script().expect("askpass helper");
    let request = challenge("Enter passphrase for key 'supplied-test-key':");
    let auth = explicit_auth(&request, "test-only!&^secret");
    assert_eq!(
        answer(&askpass, Some(&auth), &request.prompt),
        "test-only!&^secret"
    );
    // OpenSSH can ask again without explaining whether this key was rejected.
    assert_eq!(
        answer(&askpass, Some(&auth), &request.prompt),
        "test-only!&^secret"
    );
    for stderr in [
        "error: Your local changes to the following files would be overwritten by merge:",
        "git@example.invalid: Permission denied (publickey).",
        "fatal: Could not read from remote repository.",
    ] {
        assert_eq!(
            resolve_auth_outcome(Some(&auth), &askpass, stderr.as_bytes(), false),
            AuthOutcome::Supplied,
        );
    }
    assert_eq!(
        resolve_auth_outcome(Some(&auth), &askpass, b"", true),
        AuthOutcome::Supplied,
        "overall Git success alone is not proof of key unlock",
    );
    assert_eq!(
        resolve_auth_outcome(
            Some(&auth),
            &askpass,
            b"Load key: incorrect passphrase supplied to decrypt private key",
            false,
        ),
        AuthOutcome::Rejected(request),
    );
    assert!(load_session_passphrases().is_empty());
    let log = read_response_log(&askpass);
    assert!(log.contains("S:"), "the helper's records were decoded");
    assert!(!log.contains("test-only!&^secret"));
    clear_session_passphrase();
}

#[test]
fn explicit_key_b_overlays_cached_key_a_without_answering_key_c() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let request_a = challenge("Enter passphrase for key 'overlay-key-a':");
    let request_b = challenge("Enter passphrase for key 'overlay-key-b':");
    let request_c = challenge("Enter passphrase for key 'overlay-key-c':");
    remember_session_passphrase(&request_a.prompt, "cached-a");
    remember_session_passphrase(&request_b.prompt, "obsolete-b");
    let auth = explicit_auth(&request_b, "submitted-b");
    let askpass = create_askpass_script().expect("askpass helper");
    assert_eq!(answer(&askpass, Some(&auth), &request_a.prompt), "cached-a");
    assert_eq!(
        answer(&askpass, Some(&auth), &request_b.prompt),
        "submitted-b"
    );
    assert_eq!(answer(&askpass, Some(&auth), &request_c.prompt), "");
    assert_eq!(
        observed_responses(&askpass),
        vec![
            (true, request_a),
            (true, request_b),
            (false, request_c.clone())
        ],
    );
    assert_eq!(
        resolve_auth_outcome(
            Some(&auth),
            &askpass,
            b"Permission denied (publickey).",
            false
        ),
        AuthOutcome::NeedsInput(request_c),
    );
    assert_eq!(
        resolve_auth_outcome(
            Some(&auth),
            &askpass,
            b"error: Your local changes would be overwritten by merge",
            false,
        ),
        AuthOutcome::Supplied,
        "an earlier unanswered unused key must not obscure a later local failure",
    );
    clear_session_passphrase();
}

#[test]
fn a_later_supplied_key_does_not_reopen_an_earlier_unanswered_identity() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("key fixture directory");
    let key_b = dir.path().join("key-b");
    fs::write(&key_b, b"key B private key fixture").expect("key fixture");
    let request_a = challenge("Enter passphrase for key 'unused-key-a':");
    let request_b = challenge(&format!("Enter passphrase for \"{}\":", key_b.display()));
    let auth = explicit_auth(&request_b, "key-b-secret");
    let askpass = create_askpass_script().expect("askpass helper");
    assert_eq!(answer(&askpass, Some(&auth), &request_a.prompt), "");
    assert_eq!(
        answer(&askpass, Some(&auth), &request_b.prompt),
        "key-b-secret"
    );
    assert_eq!(
        observed_responses(&askpass),
        vec![(false, request_a), (true, request_b.clone())],
    );
    let failures = [
        "Connection reset by peer",
        "remote: error: GH006: Protected branch update failed for refs/heads/main.",
        "! [rejected] main -> main (non-fast-forward)",
        "remote: Repository not found.\nfatal: Could not read from remote repository.",
    ];
    for stderr in failures {
        assert_eq!(
            resolve_auth_outcome(Some(&auth), &askpass, stderr.as_bytes(), false),
            AuthOutcome::Supplied,
            "a terminal remote failure must not reopen an older identity: {stderr}",
        );
    }

    // Supply the state that independent key validation establishes. Neither
    // the failed operation nor text from another identity may erase it.
    let PromptAuth::Explicit(staged) = &auth else {
        unreachable!()
    };
    assert!(remember_verified_passphrase(&request_b, staged));
    for stderr in failures {
        assert_eq!(
            resolve_auth_outcome(Some(&auth), &askpass, stderr.as_bytes(), false),
            AuthOutcome::Verified,
        );
        let earlier_rejection = format!(
            "Load key unused-key-a: incorrect passphrase supplied to decrypt private key\n{stderr}",
        );
        assert_eq!(
            resolve_auth_outcome(Some(&auth), &askpass, earlier_rejection.as_bytes(), false),
            AuthOutcome::Verified,
            "key A's historical rejection is not evidence against verified key B",
        );
    }
    assert_eq!(load_session_passphrases().len(), 1);
    clear_session_passphrase();
}

#[test]
fn custom_prompt_whitespace_preserves_binding_during_the_one_replay() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let prompt = "  Custom passphrase request:   ";
    let missing = create_askpass_script().expect("askpass helper");
    assert_eq!(answer(&missing, None, prompt), "");
    let AuthOutcome::NeedsInput(request) =
        resolve_auth_outcome(None, &missing, b"Permission denied (publickey).", false)
    else {
        panic!("the helper must record the custom challenge");
    };
    assert!(request.key_path.is_none());
    let auth = explicit_auth(&request, "custom-secret");
    let replay = create_askpass_script().expect("askpass helper");
    assert_eq!(answer(&replay, Some(&auth), prompt), "custom-secret");
    assert_eq!(
        resolve_auth_outcome(
            Some(&auth),
            &replay,
            b"Permission denied (publickey).",
            false
        ),
        AuthOutcome::Supplied,
        "an unknown key identity permits one supplied replay without reopening auth",
    );
    assert!(load_session_passphrases().is_empty());
    clear_session_passphrase();
}

#[test]
fn credential_and_host_requests_keep_their_typed_challenges() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    for (kind, prompt, secret) in [
        (
            GitAuthKind::UsernamePassword,
            "Password for 'https://example.invalid':",
            "test-password",
        ),
        (
            GitAuthKind::HostVerification,
            "Are you sure you want to continue connecting (yes/no/[fingerprint])?",
            "yes",
        ),
    ] {
        let request = AuthChallenge::from_prompt(kind, normalized_prompt(prompt));
        let missing = create_askpass_script().expect("askpass helper");
        assert_eq!(answer(&missing, None, prompt), "");
        assert_eq!(
            resolve_auth_outcome(None, &missing, b"connection failed", false),
            AuthOutcome::NeedsInput(request.clone()),
        );
        let supplied = create_askpass_script().expect("askpass helper");
        let auth = explicit_auth(&request, secret);
        assert_eq!(answer(&supplied, Some(&auth), prompt), secret);
        let expected = if kind == GitAuthKind::UsernamePassword {
            AuthOutcome::Rejected(request)
        } else {
            AuthOutcome::Supplied
        };
        assert_eq!(
            resolve_auth_outcome(Some(&auth), &supplied, b"Authentication failed", false),
            expected,
        );
    }
    clear_session_passphrase();
}

/// Typed credentials answered every username and password prompt of the
/// replayed command, so a `fetch --all` or a submodule on another host was
/// sent the username and password typed for the first one. Git spells the
/// username in its password prompt percent-encoded, and the challenge may
/// carry the username the user just replaced.
#[test]
fn typed_credentials_answer_only_their_own_host() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    for (challenge_prompt, login, password_prompt) in [
        (
            "Username for 'https://git.example.com': ",
            "alice",
            "Password for 'https://alice@git.example.com': ",
        ),
        (
            "Password for 'https://git.example.com': ",
            "alice",
            "Password for 'https://alice@git.example.com': ",
        ),
        (
            "Username for 'https://git.example.com': ",
            "alice@example.com",
            "Password for 'https://alice%40example.com@git.example.com': ",
        ),
        (
            "Password for 'https://alice@git.example.com': ",
            "bob",
            "Password for 'https://bob@git.example.com': ",
        ),
        (
            "Password for 'https://git.example.com': ",
            "username",
            "Password for 'https://username@git.example.com': ",
        ),
    ] {
        let request = AuthChallenge::from_prompt(
            GitAuthKind::UsernamePassword,
            normalized_prompt(challenge_prompt),
        );
        let auth = PromptAuth::Explicit(StagedGitAuth {
            kind: GitAuthKind::UsernamePassword,
            username: Some(login.to_string()),
            secret: "test-password".to_string(),
            challenge: Some(request),
        });
        let askpass = create_askpass_script().expect("askpass helper");
        let case = format!("{challenge_prompt:?} typed as {login:?}");

        assert_eq!(
            answer_raw(
                &askpass,
                Some(&auth),
                "Username for 'https://git.example.com': "
            ),
            login,
            "{case}"
        );
        assert_eq!(
            answer_raw(&askpass, Some(&auth), password_prompt),
            "test-password",
            "{case}"
        );
        assert_eq!(
            answer_raw(
                &askpass,
                Some(&auth),
                "Username for 'https://other.example.net': "
            ),
            "",
            "another host's username prompt after {case}"
        );
        assert_eq!(
            answer_raw(
                &askpass,
                Some(&auth),
                "Password for 'https://alice@other.example.net': "
            ),
            "",
            "another host's password prompt after {case}"
        );
    }
    clear_session_passphrase();
}

#[test]
fn verified_cache_is_independent_of_git_result_and_invalidates_replaced_keys() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("key fixture directory");
    let key = dir.path().join("key");
    fs::write(&key, b"original private key fixture").expect("key fixture");
    let request = challenge(&format!("Enter passphrase for \"{}\":", key.display()));
    assert!(request.key_path.is_some());
    let auth = explicit_auth(&request, "verified-secret");
    let PromptAuth::Explicit(staged) = &auth else {
        unreachable!()
    };
    // This unit fixture supplies the post-validation cache state. The next
    // test exercises actual cryptographic validation with a generated key.
    assert!(remember_verified_passphrase(&request, staged));
    let cached = PromptAuth::CachedPassphrases(load_session_passphrases());
    let askpass = create_askpass_script().expect("askpass helper");
    assert_eq!(
        answer(&askpass, Some(&cached), &request.prompt),
        "verified-secret"
    );
    for stderr in [
        "Your local changes would be overwritten by merge",
        "Permission denied (publickey).",
    ] {
        assert_eq!(
            resolve_auth_outcome(Some(&cached), &askpass, stderr.as_bytes(), false),
            AuthOutcome::Verified,
        );
    }
    assert_eq!(load_session_passphrases().len(), 1);
    let changed_answer = explicit_auth(&request, "different-unvalidated-secret");
    let changed_askpass = create_askpass_script().expect("askpass helper");
    assert_eq!(
        answer(&changed_askpass, Some(&changed_answer), &request.prompt),
        "different-unvalidated-secret",
    );
    assert_eq!(
        resolve_auth_outcome(
            Some(&changed_answer),
            &changed_askpass,
            b"Permission denied (publickey).",
            false,
        ),
        AuthOutcome::Supplied,
        "a verified key does not prove a newly submitted different secret",
    );
    remember_session_passphrase("Enter passphrase for key 'unrelated':", "other");
    fs::write(&key, b"replacement private key fixture").expect("replace key fixture");
    assert!(!remember_verified_passphrase(&request, staged));
    let remaining = load_session_passphrases();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0].secret, "other");
    assert_eq!(
        resolve_auth_outcome(
            Some(&cached),
            &askpass,
            b"Permission denied (publickey).",
            false
        ),
        AuthOutcome::Supplied,
    );
    clear_session_passphrase();
}

#[test]
fn independent_key_validation_proves_unlock_and_handles_rejection_cancel_and_unknown() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let dir = tempfile::tempdir().expect("temporary SSH key directory");
    let key = dir.path().join("encrypted test key");
    let generated = Command::new("ssh-keygen")
        .args([
            "-t",
            "ed25519",
            "-q",
            "-C",
            "gitcomet-test",
            "-N",
            "test!&^passphrase",
            "-f",
        ])
        .arg(&key)
        .stdin(Stdio::null())
        .output();
    let generated = match generated {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("ssh-keygen unavailable; skipping cryptographic validation test");
            return;
        }
        result => result.expect("generate disposable encrypted SSH key"),
    };
    assert!(
        generated.status.success(),
        "generate disposable encrypted SSH key"
    );
    let original = fs::read(&key).expect("original private key bytes");
    let request = challenge(&format!("Enter passphrase for \"{}\":", key.display()));
    assert!(request.key_path.is_some());
    let PromptAuth::Explicit(correct) = explicit_auth(&request, "test!&^passphrase") else {
        unreachable!()
    };
    let mut wrong = correct.clone();
    wrong.secret = "incorrect-test-passphrase".into();
    let cancellation = CancellationToken::new();
    assert_eq!(
        validate_passphrase(&request, &wrong, &cancellation),
        PassphraseValidation::Rejected
    );
    assert_eq!(
        validate_passphrase(&request, &correct, &cancellation),
        PassphraseValidation::Verified
    );
    assert!(
        load_session_passphrases().is_empty(),
        "validation does not itself cache a secret"
    );
    assert!(remember_verified_passphrase(&request, &correct));
    assert_eq!(load_session_passphrases().len(), 1);
    // A cached correct answer must not substitute for the newly submitted one.
    assert_eq!(
        validate_passphrase(&request, &wrong, &cancellation),
        PassphraseValidation::Rejected
    );
    assert!(load_session_passphrases().is_empty());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert_eq!(
        validate_passphrase(&request, &correct, &cancelled),
        PassphraseValidation::Unknown
    );
    let unsupported = challenge("Custom passphrase request:");
    assert_eq!(
        validate_passphrase(&unsupported, &correct, &cancellation),
        PassphraseValidation::Unknown
    );
    assert_eq!(
        fs::read(&key).expect("private key after validation"),
        original
    );
    fs::write(&key, b"not an SSH private key").expect("replace disposable key");
    assert_eq!(
        validate_passphrase(&request, &correct, &cancellation),
        PassphraseValidation::Unknown
    );
    assert!(load_session_passphrases().is_empty());
    clear_session_passphrase();
}

/// Builds the helper's command so Windows gets it on a raw `cmd.exe` command
/// line, where Git's and OpenSSH's requests land. `Command` refuses a batch
/// argument spanning several lines, as OpenSSH's host-key prompts do, and
/// rewrites `%`.
fn raw_prompt_command(askpass: &AskPassScript, prompt: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        let mut command = Command::new("cmd.exe");
        command.raw_arg(format!(
            "/d /c \"\"{}\" \"{prompt}\"\"",
            askpass.path().display()
        ));
        command.stdin(Stdio::null());
        command
    }
    #[cfg(not(windows))]
    {
        let mut command = Command::new(askpass.path());
        command.arg(prompt).stdin(Stdio::null());
        command
    }
}

const REFUSED_PUBLIC_KEY: &[u8] = b"git@github.com: Permission denied (publickey).\r\n\
fatal: Could not read from remote repository.\n";

#[test]
fn multiline_host_prompt_keeps_its_fingerprint() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    // Verbatim from Windows OpenSSH 9.5 through the real helper.
    let prompt = "The authenticity of host 'github.com (140.82.121.4)' can't be established.\n\
ED25519 key fingerprint is SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU.\n\
This key is not known by any other names.\n\
Are you sure you want to continue connecting (yes/no/[fingerprint])?";
    let askpass = create_askpass_script().expect("askpass helper");
    let mut command = raw_prompt_command(&askpass, prompt);
    configure_git_auth_prompt(&mut command, None, &askpass);
    let output = run_helper(&mut command);
    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "");
    let AuthOutcome::NeedsInput(request) =
        resolve_auth_outcome(None, &askpass, b"Host key verification failed.", false)
    else {
        panic!("an unanswered host prompt requests input");
    };
    assert_eq!(request.kind, GitAuthKind::HostVerification);
    assert_eq!(
        request.prompt, prompt,
        "the fingerprint must reach the user"
    );
}

/// Confirming one host's key answered "yes" to every host-key prompt of the
/// replayed command, so a second unknown host reached by `fetch --all` or a
/// submodule was trusted without the user ever seeing its fingerprint.
#[test]
fn host_key_confirmation_answers_only_the_host_the_user_saw() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let host_prompt = |host: &str, address: &str, fingerprint: &str| {
        format!(
            "The authenticity of host '{host} ({address})' can't be established.\n\
             ED25519 key fingerprint is SHA256:{fingerprint}.\n\
             This key is not known by any other names.\n\
             Are you sure you want to continue connecting (yes/no/[fingerprint])?"
        )
    };
    let confirmed = host_prompt(
        "git.example.com",
        "192.0.2.1",
        "confirmedFingerprintForTestOnly",
    );
    let other = host_prompt(
        "other.example.net",
        "192.0.2.1",
        "otherFingerprintForTestOnly",
    );
    let other_key = host_prompt(
        "git.example.com",
        "192.0.2.1",
        "otherFingerprintForTestOnly",
    );
    let request =
        AuthChallenge::from_prompt(GitAuthKind::HostVerification, normalized_prompt(&confirmed));
    let auth = explicit_auth(&request, "yes");
    let askpass = create_askpass_script().expect("askpass helper");
    let answer_multiline = |prompt: &str| {
        let mut command = raw_prompt_command(&askpass, prompt);
        configure_git_auth_prompt(&mut command, Some(&auth), &askpass);
        let output = run_helper(&mut command);
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    };

    assert_eq!(answer_multiline(&confirmed), "yes");
    assert_eq!(answer_multiline(&other), "");
    assert_eq!(answer_multiline(&other_key), "");
    // A round-robin host answers the replay from another address; only the
    // POSIX helper drops it (the Windows one still asks once more).
    #[cfg(not(windows))]
    assert_eq!(
        answer_multiline(&host_prompt(
            "git.example.com",
            "192.0.2.77",
            "confirmedFingerprintForTestOnly"
        )),
        "yes"
    );

    let unbound = PromptAuth::Explicit(StagedGitAuth {
        kind: GitAuthKind::HostVerification,
        username: None,
        secret: "yes".to_string(),
        challenge: None,
    });
    let mut command = raw_prompt_command(&askpass, &confirmed);
    configure_git_auth_prompt(&mut command, Some(&unbound), &askpass);
    let output = run_helper(&mut command);
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "",
        "a confirmation for no prompt in particular confirms nothing"
    );
    clear_session_passphrase();
}

/// The first run of a two-host `fetch --all` asks about the last host only.
/// Its replay answers that host and leaves the first one unanswered, which
/// used to end in a bare failure instead of asking about the first host.
#[test]
fn a_replay_asks_about_an_earlier_host_left_unanswered() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let host_prompt = |host: &str| {
        format!(
            "The authenticity of host '{host} (192.0.2.1)' can't be established.\n\
             ED25519 key fingerprint is SHA256:fingerprintFor{host}.\n\
             Are you sure you want to continue connecting (yes/no/[fingerprint])?"
        )
    };
    for (first, second, kind, secret) in [
        (
            host_prompt("one.example.com"),
            host_prompt("two.example.net"),
            GitAuthKind::HostVerification,
            "yes",
        ),
        (
            "Username for 'https://one.example.com': ".to_string(),
            "Username for 'https://two.example.net': ".to_string(),
            GitAuthKind::UsernamePassword,
            "test-password",
        ),
    ] {
        let confirmed = AuthChallenge::from_prompt(kind, normalized_prompt(&second));
        let auth = PromptAuth::Explicit(StagedGitAuth {
            kind,
            username: Some("alice".to_string()),
            secret: secret.to_string(),
            challenge: Some(confirmed),
        });
        let askpass = create_askpass_script().expect("askpass helper");
        for prompt in [&first, &second] {
            let mut command = raw_prompt_command(&askpass, prompt);
            configure_git_auth_prompt(&mut command, Some(&auth), &askpass);
            assert!(run_helper(&mut command).status.success());
        }

        assert_eq!(
            resolve_auth_outcome(Some(&auth), &askpass, b"fatal: could not fetch one", false),
            AuthOutcome::NeedsInput(AuthChallenge::from_prompt(kind, normalized_prompt(&first))),
            "{kind:?}"
        );
    }
    clear_session_passphrase();
}

#[test]
fn security_key_pin_requests_reopen_input_and_are_answered() {
    let _lock = auth_test_lock();
    clear_session_passphrase();
    let prompt =
        "Enter PIN for ED25519-SK key SHA256:0123456789abcdefghijklmnopqrstuvwxyzABCDEFG: ";
    let askpass = create_askpass_script().expect("askpass helper");
    assert_eq!(answer(&askpass, None, prompt), "");
    let AuthOutcome::NeedsInput(request) =
        resolve_auth_outcome(None, &askpass, REFUSED_PUBLIC_KEY, false)
    else {
        panic!("an unanswered PIN request needs input");
    };
    assert_eq!(request.kind, GitAuthKind::Passphrase);
    assert!(request.is_pin());
    let auth = explicit_auth(&request, "123456");
    let askpass = create_askpass_script().expect("askpass helper");
    assert_eq!(answer(&askpass, Some(&auth), prompt), "123456");
}

#[cfg(windows)]
#[test]
fn non_ascii_prompts_survive_a_fresh_console_code_page() {
    use std::os::windows::process::CommandExt as _;
    let _lock = auth_test_lock();
    clear_session_passphrase();
    // The app runs Git without a console window, so the helper gets a new
    // console using the system OEM code page rather than this test's.
    let prompt = "Enter passphrase for key 'C:\\Users\\J\u{e4}rvinen\\.ssh\\id_ed25519':";
    let askpass = create_askpass_script().expect("askpass helper");
    let mut command = Command::new(askpass.path());
    command
        .arg(prompt)
        .stdin(Stdio::null())
        .creation_flags(0x0800_0000);
    configure_git_auth_prompt(&mut command, None, &askpass);
    assert!(run_helper(&mut command).status.success());
    assert_eq!(
        resolve_auth_outcome(None, &askpass, REFUSED_PUBLIC_KEY, false),
        AuthOutcome::NeedsInput(challenge(prompt)),
    );
}
