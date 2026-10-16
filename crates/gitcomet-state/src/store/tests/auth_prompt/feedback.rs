use super::*;
use crate::model::{GitHookOperationStatus, GitOperationOuterOutcome};
use crate::msg::InternalMsg;
use gitcomet_core::error::{GitFailure, GitFailureId};
use gitcomet_core::git_operation::{GitOperationEvent, GitOperationId, HookExecutionId};

const WINDOWS_SSH_STDERR: &str = "git@github.com: Permission denied (publickey).\n\
fatal: Could not read from remote repository.\n\
Please make sure you have the correct access rights\n\
and the repository exists.\n\
SSH key passphrase prompt:\n\
Enter passphrase for key 'C:\\Users\\sampo/.ssh/id_ed25519':\n";

fn windows_ssh_error(command: &str) -> Error {
    Error::new(ErrorKind::Git(
        GitFailure::new(
            command,
            GitFailureId::CommandFailed,
            Some(128),
            Vec::new(),
            WINDOWS_SSH_STDERR.as_bytes().to_vec(),
            Some(WINDOWS_SSH_STDERR.trim_end().into()),
        )
        .with_auth_outcome(gitcomet_core::auth::AuthOutcome::NeedsInput(
            gitcomet_core::auth::AuthChallenge {
                kind: GitAuthKind::Passphrase,
                prompt: "Enter passphrase for key 'C:\\Users\\sampo/.ssh/id_ed25519':".into(),
                key_path: None,
                key_fingerprint: None,
            },
        )),
    ))
}

#[test]
fn windows_ssh_pull_retry_keeps_auth_attempts_out_of_terminal_failure_feedback() {
    let _lock = super::super::staged_auth_test_lock();
    for outcome in ["success", "failure", "cancel"] {
        let repo_id = RepoId(1);
        let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
        let id_alloc = AtomicU64::new(1);
        let command = RepoCommandKind::Pull {
            mode: PullMode::Default,
        };
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Pull {
                repo_id,
                mode: PullMode::Default,
            },
        );

        // Missing credentials and a wrong passphrase both stay recoverable.
        for attempt in 0..2 {
            reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                Msg::Internal(InternalMsg::RepoCommandFinished {
                    repo_id,
                    command: command.clone(),
                    result: Err(windows_ssh_error("git pull --prune")),
                }),
            );
            assert_auth_attempt_is_recoverable(&state);
            assert_eq!(state.repos[0].pull_in_flight, 0);
            assert_eq!(state.repos[0].worktree_pull_in_flight, 0);
            let prompt = state.auth_prompt.as_ref().expect("passphrase prompt");
            assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
            assert!(prompt.reason.contains("Enter passphrase for key"));
            let log = &state.repos[0].feedback.command_log;
            assert_eq!(log.len(), attempt + 1);
            assert_eq!(log.last().unwrap().command, "git pull --prune");
            assert!(
                log.last()
                    .unwrap()
                    .stderr
                    .ends_with(WINDOWS_SSH_STDERR.trim_end())
            );

            if attempt == 1 && outcome == "cancel" {
                let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::CancelAuthPrompt);
                assert!(effects.is_empty());
                assert!(state.auth_prompt.is_none());
                assert_auth_attempt_is_recoverable(&state);
                break;
            }
            let secret = if attempt == 0 {
                "wrong secret"
            } else {
                "correct secret"
            };
            let effects = reduce(
                &mut repos,
                &id_alloc,
                &mut state,
                Msg::SubmitAuthPrompt {
                    generation: (attempt + 1) as u64,
                    username: None,
                    secret: secret.into(),
                },
            );
            assert!(state.auth_prompt.is_none());
            assert_eq!(state.repos[0].pull_in_flight, 1);
            assert_eq!(state.repos[0].worktree_pull_in_flight, 1);
            let auth = effects
                .iter()
                .find_map(Effect::git_auth)
                .expect("retry auth");
            assert_eq!(auth.kind, GitAuthKind::Passphrase);
            assert_eq!(auth.secret, secret);
        }

        if outcome == "cancel" {
            assert_eq!(state.repos[0].feedback.command_log.len(), 2);
            continue;
        }
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: if outcome == "success" {
                    Ok(CommandOutput {
                        command: "git pull --prune".into(),
                        stdout: "Updating 1234567..abcdef0\nFast-forward\n".into(),
                        stderr: String::new(),
                        exit_code: Some(0),
                    })
                } else {
                    Err(auth_error(
                        "git pull failed: unable to resolve host github.com",
                    ))
                },
            }),
        );
        assert!(state.auth_prompt.is_none());
        assert_eq!(state.repos[0].pull_in_flight, 0);
        assert_eq!(state.repos[0].worktree_pull_in_flight, 0);
        let feedback = &state.repos[0].feedback;
        assert_eq!(feedback.command_log.len(), 3);
        let entry = feedback.command_log.last().unwrap();
        assert!(!entry.handled_by_auth_prompt);
        assert_eq!(entry.ok, outcome == "success");
        assert_eq!(feedback.last_error.is_some(), outcome == "failure");
        assert!(feedback.command_log.iter().all(|entry| {
            [
                entry.command.as_str(),
                entry.summary.as_str(),
                entry.stdout.as_ref(),
                entry.stderr.as_ref(),
            ]
            .iter()
            .all(|text| !text.contains("wrong secret") && !text.contains("correct secret"))
        }));
    }
}

#[test]
fn auth_prompt_does_not_clear_an_unrelated_existing_error() {
    let _lock = super::super::staged_auth_test_lock();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.repos[0].feedback_mut().last_error = Some("previous command failed".into());
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Err(windows_ssh_error("git pull --prune")),
        }),
    );
    assert!(state.auth_prompt.is_some());
    assert_eq!(
        state.repos[0].feedback.last_error.as_deref(),
        Some("previous command failed")
    );
    assert!(
        state.repos[0]
            .feedback
            .command_log
            .last()
            .unwrap()
            .handled_by_auth_prompt
    );
}

#[test]
fn auth_recovery_marks_only_its_own_hook_operation() {
    let _lock = super::super::staged_auth_test_lock();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let auth_id = GitOperationId(41);
    let other_id = GitOperationId(42);
    for operation_id in [auth_id, other_id] {
        let hook_id = HookExecutionId {
            sid: Arc::from("auth-feedback"),
            child_id: operation_id.0,
        };
        for message in [
            InternalMsg::GitOperationStarted {
                repo_id,
                operation_id,
                label: "Push".into(),
                context: None,
                time: SystemTime::UNIX_EPOCH,
                progress_lane: false,
            },
            InternalMsg::GitOperationEvent {
                repo_id,
                operation_id,
                event: GitOperationEvent::HookStarted {
                    id: hook_id.clone(),
                    name: "pre-push".into(),
                },
            },
            InternalMsg::GitOperationEvent {
                repo_id,
                operation_id,
                event: GitOperationEvent::HookFinished {
                    id: hook_id,
                    name: "pre-push".into(),
                    exit_code: Some(0),
                    duration: Duration::from_millis(1),
                },
            },
        ] {
            reduce(&mut repos, &id_alloc, &mut state, Msg::Internal(message));
        }
    }
    for operation_id in [auth_id, other_id] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(InternalMsg::GitOperationFinished {
                repo_id,
                operation_id,
                outer_outcome: GitOperationOuterOutcome::Failed,
                duration: Duration::from_millis(10),
                message: Box::new(InternalMsg::RepoCommandFinished {
                    repo_id,
                    command: RepoCommandKind::Push,
                    result: Err(if operation_id == auth_id {
                        windows_ssh_error("git push")
                    } else {
                        auth_error("remote rejected: branch is protected")
                    }),
                }),
            }),
        );
        let feedback = &state.repos[0].feedback;
        let entry = feedback.command_log.last().unwrap();
        assert_eq!(entry.hook_operation_id, Some(operation_id));
        assert_eq!(entry.handled_by_auth_prompt, operation_id == auth_id);
        let activity = feedback
            .hook_activity
            .iter()
            .find(|op| op.id == operation_id)
            .unwrap();
        assert_eq!(activity.status, GitHookOperationStatus::Failed);
        assert_eq!(activity.handled_by_auth_prompt, operation_id == auth_id);
        assert_eq!(feedback.last_error.is_none(), operation_id == auth_id);
    }
    assert!(
        state.auth_prompt.is_some(),
        "unrelated failure keeps the pending prompt"
    );
}

#[test]
fn clone_authentication_waits_then_retries_or_cancels_quietly() {
    let _lock = super::super::staged_auth_test_lock();
    for outcome in ["success", "failure", "cancel"] {
        let mut repos: RepositoryRuntimes = Default::default();
        let mut state = AppState::test_default();
        let id_alloc = AtomicU64::new(1);
        let url = "git@github.com:private/repo.git".to_string();
        let dest = PathBuf::from("/tmp/private-repo");
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::CloneRepo {
                url: url.clone(),
                dest: dest.clone(),
            },
        );
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(InternalMsg::CloneRepoFinished {
                url: url.clone(),
                dest: dest.clone(),
                result: Err(windows_ssh_error("git clone")),
            }),
        );
        assert!(matches!(
            state.clone.as_ref().unwrap().status,
            CloneOpStatus::WaitingForAuthentication
        ));
        assert!(state.auth_prompt.is_some());
        if outcome == "cancel" {
            assert!(reduce(&mut repos, &id_alloc, &mut state, Msg::CancelAuthPrompt).is_empty());
            assert!(state.auth_prompt.is_none());
            assert!(state.clone.is_none());
            continue;
        }
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SubmitAuthPrompt {
                generation: 1,
                username: None,
                secret: "correct secret".into(),
            },
        );
        assert!(matches!(
            state.clone.as_ref().unwrap().status,
            CloneOpStatus::Running
        ));
        assert!(state.auth_prompt.is_none());
        assert_eq!(
            effects.iter().find_map(Effect::git_auth).unwrap().secret,
            "correct secret"
        );
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(InternalMsg::CloneRepoFinished {
                url,
                dest,
                result: if outcome == "success" {
                    Ok(CommandOutput {
                        command: "git clone".into(),
                        stdout: String::new(),
                        stderr: String::new(),
                        exit_code: Some(0),
                    })
                } else {
                    Err(auth_error("git clone failed: repository does not exist"))
                },
            }),
        );
        let status = &state.clone.as_ref().unwrap().status;
        assert_eq!(
            matches!(status, CloneOpStatus::FinishedOk),
            outcome == "success"
        );
        assert_eq!(
            matches!(status, CloneOpStatus::FinishedErr(_)),
            outcome == "failure"
        );
    }
}

#[test]
fn cancelling_auth_does_not_remove_an_unrelated_clone() {
    let _lock = super::super::staged_auth_test_lock();
    let mut repos: RepositoryRuntimes = Default::default();
    let mut state = AppState::test_default();
    let id_alloc = AtomicU64::new(1);
    state.auth_prompt = Some(AuthPromptState {
        generation: 1,
        challenge: None,
        status: crate::model::AuthPromptStatus::AwaitingInput,
        kind: AuthPromptKind::Passphrase,
        reason: "passphrase required".into(),
        operation: AuthRetryOperation::Clone {
            url: "git@github.com:first/repo.git".into(),
            dest: PathBuf::from("/tmp/first"),
        },
    });
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: "git@github.com:second/repo.git".into(),
            dest: PathBuf::from("/tmp/second"),
        },
    );
    reduce(&mut repos, &id_alloc, &mut state, Msg::CancelAuthPrompt);
    let clone = state.clone.as_ref().expect("unrelated clone retained");
    assert_eq!(clone.dest.as_ref(), &PathBuf::from("/tmp/second"));
    assert_eq!(clone.status, CloneOpStatus::Running);
}

#[test]
fn aborting_a_clone_that_exits_for_credentials_cancels_without_a_prompt() {
    let _lock = super::super::staged_auth_test_lock();
    let mut repos: RepositoryRuntimes = Default::default();
    let mut state = AppState::test_default();
    let id_alloc = AtomicU64::new(1);
    let url = "git@github.com:private/repo.git".to_string();
    let dest = PathBuf::from("/tmp/private-repo");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: url.clone(),
            dest: dest.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );
    // Git had already exited to ask for credentials when the abort arrived.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::CloneRepoFinished {
            url,
            dest,
            result: Err(windows_ssh_error("git clone")),
        }),
    );
    assert!(state.auth_prompt.is_none());
    assert_eq!(
        state.clone.as_ref().unwrap().status,
        CloneOpStatus::Cancelled
    );
}

#[test]
fn aborting_a_clone_that_exits_for_credentials_stays_quiet_behind_another_prompt() {
    let _lock = super::super::staged_auth_test_lock();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let fetch = AuthPromptState {
        generation: 1,
        challenge: None,
        status: crate::model::AuthPromptStatus::AwaitingInput,
        kind: AuthPromptKind::Passphrase,
        reason: "passphrase required".into(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::FetchAll,
        },
    };
    state.auth_prompt = Some(fetch.clone());
    let url = "git@github.com:private/repo.git".to_string();
    let dest = PathBuf::from("/tmp/private-repo");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloneRepo {
            url: url.clone(),
            dest: dest.clone(),
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AbortCloneRepo { dest: dest.clone() },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::CloneRepoFinished {
            url,
            dest,
            result: Err(windows_ssh_error("git clone")),
        }),
    );
    assert_eq!(state.auth_prompt, Some(fetch));
    assert_eq!(
        state.clone.as_ref().unwrap().status,
        CloneOpStatus::Cancelled
    );
}

#[test]
fn closing_a_repository_discards_its_pending_auth_prompt() {
    let _lock = super::super::staged_auth_test_lock();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let pull = RepoCommandKind::Pull {
        mode: PullMode::Default,
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: pull,
            result: Err(windows_ssh_error("git pull")),
        }),
    );
    assert!(state.auth_prompt.is_some());
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id },
    );
    assert!(state.auth_prompt.is_none());
}

#[test]
fn a_command_that_succeeds_another_way_discards_only_its_own_prompt() {
    let _lock = super::super::staged_auth_test_lock();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let pull = RepoCommandKind::Pull {
        mode: PullMode::Default,
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: pull.clone(),
            result: Err(windows_ssh_error("git pull")),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::FetchAll,
            result: Ok(CommandOutput::default()),
        }),
    );
    assert!(state.auth_prompt.is_some(), "another command's success");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: pull,
            result: Ok(CommandOutput::default()),
        }),
    );
    assert!(state.auth_prompt.is_none());
}

#[test]
fn a_second_auth_failure_does_not_silently_discard_the_pending_prompt() {
    let _lock = super::super::staged_auth_test_lock();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let pull = RepoCommandKind::Pull {
        mode: PullMode::Default,
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: pull.clone(),
            result: Err(windows_ssh_error("git pull --prune")),
        }),
    );
    let pending = state.auth_prompt.clone().expect("pull prompt");

    // Another operation needs credentials while the pull's prompt is open.
    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: Err(windows_ssh_error("git push")),
        }),
    );

    assert_eq!(
        state.auth_prompt.as_ref(),
        Some(&pending),
        "the pull's prompt stays answerable"
    );
    // The push could not take over the prompt, so it is a visible failure.
    assert_terminal_failure_keeps_prompt(&state);
}

fn assert_terminal_failure_keeps_prompt(state: &AppState) {
    let feedback = &state.repos[0].feedback;
    assert!(feedback.last_error.is_some());
    let entry = feedback.command_log.last().expect("failed attempt logged");
    assert!(!entry.ok);
    assert!(!entry.handled_by_auth_prompt);
}
