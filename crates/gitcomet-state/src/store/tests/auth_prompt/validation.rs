use super::*;
use crate::model::AuthPromptStatus;
use crate::msg::InternalMsg;
use gitcomet_core::auth::{
    AuthChallenge, AuthOutcome, PassphraseValidation, clear_session_passphrase,
    load_session_passphrases,
};
use gitcomet_core::error::{GitFailure, GitFailureId};
use gitcomet_core::services::CancellationToken;

fn key_challenge(path: Option<PathBuf>) -> AuthChallenge {
    AuthChallenge::from_prompt(
        GitAuthKind::Passphrase,
        format!(
            "Enter passphrase for \"{}\":",
            path.as_deref().unwrap_or(Path::new("custom-key")).display()
        ),
    )
}

fn outcome_error(outcome: AuthOutcome, id: GitFailureId, output: &str) -> Error {
    Error::new(ErrorKind::Git(
        GitFailure::new(
            "git pull --prune",
            id,
            Some(1),
            Vec::new(),
            output.as_bytes().to_vec(),
            Some(output.into()),
        )
        .with_auth_outcome(outcome),
    ))
}

fn finish_pull(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    error: Error,
) {
    reduce(
        repos,
        &AtomicU64::new(1),
        state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Err(error),
        }),
    );
}

fn start_validation(
    repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
    state: &mut AppState,
    challenge: AuthChallenge,
) -> (u64, StagedGitAuth, CancellationToken) {
    finish_pull(
        repos,
        state,
        outcome_error(
            AuthOutcome::NeedsInput(challenge),
            GitFailureId::CommandFailed,
            "Permission denied (publickey).",
        ),
    );
    let generation = state.auth_prompt.as_ref().unwrap().generation;
    let effects = reduce(
        repos,
        &AtomicU64::new(1),
        state,
        Msg::SubmitAuthPrompt {
            generation,
            username: None,
            secret: "submitted-secret".into(),
        },
    );
    let Effect::ValidateAuthPrompt {
        generation,
        auth,
        cancellation,
        ..
    } = effects.into_iter().next().unwrap()
    else {
        panic!("known key must validate before retrying Git");
    };
    assert_eq!(
        state.auth_prompt.as_ref().unwrap().status,
        AuthPromptStatus::Validating
    );
    assert_eq!(state.repos[0].pull_in_flight, 0);
    assert!(!format!("{:?}", state.auth_prompt).contains("submitted-secret"));
    (generation, auth, cancellation)
}

#[test]
fn structured_non_auth_outcomes_ignore_historical_prompts() {
    for outcome in [
        AuthOutcome::NotRequested,
        AuthOutcome::Supplied,
        AuthOutcome::Verified,
        AuthOutcome::Unknown,
    ] {
        let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
        finish_pull(
            &mut repos,
            &mut state,
            outcome_error(
                outcome,
                GitFailureId::CommandFailed,
                // Without the structured outcome, this text alone asks for a
                // passphrase. The outcome must take precedence over it.
                "Load key \"/tmp/key\": incorrect passphrase supplied to decrypt private key\n\
                 git@example.invalid: Permission denied (publickey).\n\
                 SSH key passphrase prompt:\nEnter passphrase for key '/tmp/key':",
            ),
        );
        assert_terminal_failure(&state);
    }
}

#[test]
fn local_failure_categories_override_earlier_authentication_rejections() {
    for id in [
        GitFailureId::WorktreeWouldBeOverwritten,
        GitFailureId::StoppedAtConflicts,
        GitFailureId::Timeout,
    ] {
        let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
        finish_pull(
            &mut repos,
            &mut state,
            outcome_error(
                AuthOutcome::Rejected(key_challenge(None)),
                id,
                "Merge could not proceed.\nSSH key passphrase prompt:\nEnter passphrase for key '/tmp/key':",
            ),
        );
        assert_terminal_failure(&state);
        assert_eq!(
            state.repos[0]
                .feedback
                .command_log
                .last()
                .unwrap()
                .failure_id,
            Some(id)
        );
    }
}

#[test]
fn verified_passphrase_is_cached_before_retry_and_survives_merge_failure() {
    let _lock = super::super::staged_auth_test_lock();
    clear_session_passphrase();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, "key bytes validated by the executor").unwrap();
    let challenge = key_challenge(Some(path));
    let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
    let (generation, auth, _) = start_validation(&mut repos, &mut state, challenge.clone());
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::Internal(InternalMsg::AuthPromptValidated {
            generation,
            result: PassphraseValidation::Verified,
            auth,
        }),
    );
    assert!(state.auth_prompt.is_none());
    let retry_auth = effects.iter().find_map(Effect::git_auth).unwrap();
    assert_eq!(retry_auth.challenge.as_ref(), Some(&challenge));
    assert_eq!(load_session_passphrases().len(), 1);
    assert_eq!(state.repos[0].pull_in_flight, 1);
    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::Supplied,
            GitFailureId::WorktreeWouldBeOverwritten,
            "error: Your local changes to README.md would be overwritten by merge.",
        ),
    );
    assert_terminal_failure(&state);
    assert_eq!(load_session_passphrases().len(), 1);
    assert!(
        state.repos[0]
            .feedback
            .command_log
            .last()
            .unwrap()
            .summary
            .starts_with("Pull blocked by local changes")
    );
    clear_session_passphrase();
}

#[test]
fn rejected_validation_stays_inline_without_replaying_or_logging_a_failure() {
    let _lock = super::super::staged_auth_test_lock();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, "key fixture").unwrap();
    let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
    let (generation, auth, _) = start_validation(&mut repos, &mut state, key_challenge(Some(path)));
    let duplicate = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::SubmitAuthPrompt {
            generation,
            username: None,
            secret: "another-secret".into(),
        },
    );
    assert!(duplicate.is_empty());
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::Internal(InternalMsg::AuthPromptValidated {
            generation,
            result: PassphraseValidation::Rejected,
            auth: auth.clone(),
        }),
    );
    assert!(effects.is_empty());
    let prompt = state.auth_prompt.as_ref().unwrap();
    assert_eq!(prompt.status, AuthPromptStatus::Rejected);
    assert_ne!(prompt.generation, generation);
    assert!(prompt.reason.contains("could not unlock"));
    let rejected_prompt = prompt.clone();
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::SubmitAuthPrompt {
            generation: rejected_prompt.generation,
            username: None,
            secret: "another-wrong-secret".into(),
        },
    );
    let Effect::ValidateAuthPrompt {
        generation: retry_generation,
        auth: retry_auth,
        ..
    } = effects.into_iter().next().unwrap()
    else {
        panic!("another input attempt must validate independently");
    };
    let validating_prompt = state.auth_prompt.clone();
    let stale = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::Internal(InternalMsg::AuthPromptValidated {
            generation,
            result: PassphraseValidation::Verified,
            auth,
        }),
    );
    assert!(stale.is_empty());
    assert_eq!(state.auth_prompt, validating_prompt);
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::Internal(InternalMsg::AuthPromptValidated {
            generation: retry_generation,
            result: PassphraseValidation::Rejected,
            auth: retry_auth,
        }),
    );
    assert!(effects.is_empty());
    let prompt = state.auth_prompt.as_ref().unwrap();
    assert_eq!(prompt.status, rejected_prompt.status);
    assert_eq!(prompt.reason, rejected_prompt.reason);
    assert_eq!(prompt.challenge, rejected_prompt.challenge);
    assert_ne!(prompt.generation, rejected_prompt.generation);
    assert_eq!(state.repos[0].feedback.command_log.len(), 1);
    assert_eq!(state.repos[0].pull_in_flight, 0);
    assert_auth_attempt_is_recoverable(&state);
}

#[test]
fn unknown_validation_replays_bound_credentials_without_caching_or_reprompting() {
    let _lock = super::super::staged_auth_test_lock();
    clear_session_passphrase();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, "key fixture").unwrap();
    let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
    let challenge = key_challenge(Some(path));
    let (generation, auth, _) = start_validation(&mut repos, &mut state, challenge.clone());
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::Internal(InternalMsg::AuthPromptValidated {
            generation,
            result: PassphraseValidation::Unknown,
            auth,
        }),
    );
    assert!(state.auth_prompt.is_none());
    assert_eq!(
        effects
            .iter()
            .find_map(Effect::git_auth)
            .unwrap()
            .challenge
            .as_ref(),
        Some(&challenge)
    );
    assert!(load_session_passphrases().is_empty());
    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::Supplied,
            GitFailureId::CommandFailed,
            "Permission denied (publickey).",
        ),
    );
    assert_terminal_failure(&state);
}

#[test]
fn cancel_or_replacement_invalidates_validation_and_late_cache_writes() {
    let _lock = super::super::staged_auth_test_lock();
    clear_session_passphrase();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("key");
    std::fs::write(&path, "key fixture").unwrap();
    for replace in [false, true] {
        let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
        let (generation, auth, cancellation) =
            start_validation(&mut repos, &mut state, key_challenge(Some(path.clone())));
        if replace {
            finish_pull(
                &mut repos,
                &mut state,
                outcome_error(
                    AuthOutcome::NeedsInput(key_challenge(None)),
                    GitFailureId::CommandFailed,
                    "new key requires input",
                ),
            );
        } else {
            reduce(
                &mut repos,
                &AtomicU64::new(1),
                &mut state,
                Msg::CancelAuthPrompt,
            );
        }
        assert!(cancellation.is_cancelled());
        let current_prompt = state.auth_prompt.clone();
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(1),
            &mut state,
            Msg::Internal(InternalMsg::AuthPromptValidated {
                generation,
                result: PassphraseValidation::Verified,
                auth,
            }),
        );
        assert!(effects.is_empty());
        assert_eq!(state.auth_prompt, current_prompt);
        assert!(load_session_passphrases().is_empty());
        assert_eq!(state.repos[0].pull_in_flight, 0);
        let stale_submit = reduce(
            &mut repos,
            &AtomicU64::new(1),
            &mut state,
            Msg::SubmitAuthPrompt {
                generation,
                username: None,
                secret: "stale-secret".into(),
            },
        );
        assert!(stale_submit.is_empty());
        assert_eq!(state.auth_prompt, current_prompt);
    }
}

#[test]
fn username_password_retries_keep_rejections_inline_and_later_git_failures_terminal() {
    let _lock = super::super::staged_auth_test_lock();
    let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
    let challenge = AuthChallenge::from_prompt(
        GitAuthKind::UsernamePassword,
        "Password for 'https://alice@example.com':",
    );
    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::NeedsInput(challenge.clone()),
            GitFailureId::CommandFailed,
            "fatal: could not read Password for 'https://alice@example.com': terminal prompts disabled",
        ),
    );
    assert_auth_attempt_is_recoverable(&state);

    for (attempt, secret) in ["wrong-password", "correct-password"]
        .into_iter()
        .enumerate()
    {
        let prompt = state.auth_prompt.clone().expect("credential prompt");
        assert_eq!(prompt.kind, AuthPromptKind::UsernamePassword);
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(1),
            &mut state,
            Msg::SubmitAuthPrompt {
                generation: prompt.generation,
                username: Some(" alice ".into()),
                secret: secret.into(),
            },
        );
        assert!(state.auth_prompt.is_none());
        assert_eq!(state.repos[0].pull_in_flight, 1);
        let auth = effects
            .iter()
            .find_map(Effect::git_auth)
            .expect("HTTPS retry carries credentials");
        assert_eq!(auth.kind, GitAuthKind::UsernamePassword);
        assert_eq!(auth.username.as_deref(), Some("alice"));
        assert_eq!(auth.secret, secret);
        assert_eq!(auth.challenge.as_ref(), Some(&challenge));

        if attempt == 0 {
            finish_pull(
                &mut repos,
                &mut state,
                outcome_error(
                    AuthOutcome::Rejected(challenge.clone()),
                    GitFailureId::CommandFailed,
                    "fatal: Authentication failed for 'https://example.com/repo.git/'",
                ),
            );
            assert_auth_attempt_is_recoverable(&state);
            let rejected = state.auth_prompt.as_ref().unwrap();
            assert_eq!(rejected.status, AuthPromptStatus::Rejected);
            assert_ne!(rejected.generation, prompt.generation);
            assert_eq!(rejected.challenge.as_ref(), Some(&challenge));
            assert!(rejected.reason.contains("credentials were rejected"));
            assert_eq!(state.repos[0].feedback.command_log.len(), 2);
        }
    }

    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::Supplied,
            GitFailureId::WorktreeWouldBeOverwritten,
            "error: Your local changes to README.md would be overwritten by merge.\nPassword for 'https://alice@example.com':",
        ),
    );
    assert_terminal_failure(&state);
    assert_eq!(state.repos[0].pull_in_flight, 0);
    let log = &state.repos[0].feedback.command_log;
    assert_eq!(log.len(), 3);
    assert!(log.iter().all(|entry| {
        [
            entry.command.as_str(),
            entry.summary.as_str(),
            entry.stdout.as_ref(),
            entry.stderr.as_ref(),
        ]
        .iter()
        .all(|text| !text.contains("wrong-password") && !text.contains("correct-password"))
    }));
}

#[test]
fn structured_rejected_credentials_and_new_key_challenges_preserve_their_identity() {
    let _lock = super::super::staged_auth_test_lock();
    let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
    let challenge = AuthChallenge::from_prompt(
        GitAuthKind::UsernamePassword,
        "Password for 'https://example.com':",
    );
    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::Rejected(challenge.clone()),
            GitFailureId::CommandFailed,
            "HTTP 401",
        ),
    );
    let first = state.auth_prompt.clone().unwrap();
    assert_eq!(first.kind, AuthPromptKind::UsernamePassword);
    assert_eq!(first.status, AuthPromptStatus::Rejected);
    assert_eq!(first.challenge, Some(challenge));
    assert_auth_attempt_is_recoverable(&state);
    let second = key_challenge(Some(PathBuf::from("/tmp/other-key")));
    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::NeedsInput(second.clone()),
            GitFailureId::CommandFailed,
            "other key",
        ),
    );
    let prompt = state.auth_prompt.as_ref().unwrap();
    assert_ne!(prompt.generation, first.generation);
    assert_eq!(prompt.challenge.as_ref(), Some(&second));
    assert_eq!(prompt.status, AuthPromptStatus::AwaitingInput);
}

#[test]
fn security_key_pin_is_replayed_directly_and_its_rejection_says_pin() {
    let _lock = super::super::staged_auth_test_lock();
    clear_session_passphrase();
    let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
    let pin = AuthChallenge::from_prompt(
        GitAuthKind::Passphrase,
        "Enter PIN for ED25519-SK key SHA256:0123456789abcdefghijklmnopqrstuvwxyzABCDEFG: ",
    );
    assert!(pin.is_pin() && pin.key_path.is_none());
    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::NeedsInput(pin.clone()),
            GitFailureId::CommandFailed,
            "git@github.com: Permission denied (publickey).",
        ),
    );
    let prompt = state.auth_prompt.clone().expect("PIN prompt");
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    assert_eq!(prompt.status, AuthPromptStatus::AwaitingInput);

    // No local key file to check a PIN against, so it goes straight to Git.
    let effects = reduce(
        &mut repos,
        &AtomicU64::new(1),
        &mut state,
        Msg::SubmitAuthPrompt {
            generation: prompt.generation,
            username: None,
            secret: "123456".into(),
        },
    );
    assert!(effects.iter().any(|effect| effect.git_auth().is_some()));

    finish_pull(
        &mut repos,
        &mut state,
        outcome_error(
            AuthOutcome::Rejected(pin),
            GitFailureId::CommandFailed,
            "git@github.com: Permission denied (publickey).",
        ),
    );
    let prompt = state.auth_prompt.as_ref().expect("PIN retry prompt");
    assert_eq!(prompt.status, AuthPromptStatus::Rejected);
    assert_eq!(
        prompt.reason,
        "The security key PIN was not accepted. Try again."
    );
}
