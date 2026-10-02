use super::*;
use crate::model::{AuthPromptKind, AuthPromptState, AuthRetryOperation, PendingCommitRetry};
use gitcomet_core::auth::{
    GitAuthKind, StagedGitAuth, clear_staged_git_auth, stage_git_auth, take_staged_git_auth,
};
use gitcomet_core::domain::Upstream;
use gitcomet_core::services::{ConflictSide, RemoteUrlKind, ResetMode};

fn auth_error(message: &str) -> Error {
    Error::new(ErrorKind::Backend(message.to_string()))
}

fn setup_open_repo(
    repo_id: RepoId,
    workdir: &str,
) -> (FxHashMap<RepoId, Arc<dyn GitRepository>>, AppState) {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    repos.insert(repo_id, Arc::new(DummyRepo::new(workdir)));

    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from(workdir),
        },
    ));
    state.active_repo = Some(repo_id);
    (repos, state)
}

#[test]
fn review_busy_annex_auth_retry_keeps_credentials_until_push_runs() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".into(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::LargeFile {
                command: C::AnnexPush { content: true },
            },
        },
    });
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: Some("alice".into()),
            secret: "test-token".into(),
        },
    );
    assert!(effects.is_empty(), "wait until the push finishes");
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: Ok(gitcomet_core::services::CommandOutput::default()),
        }),
    );
    let auth = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::RunLargeFileCommand {
                command: C::AnnexPush { content: true },
                auth,
                ..
            } => auth.as_ref(),
            _ => None,
        })
        .expect("the queued retry must keep its credentials");
    assert_eq!(auth.username.as_deref(), Some("alice"));
    assert_eq!(auth.secret, "test-token");
    clear_staged_git_auth();
}

/// With nothing busy to queue behind, a large-file retry runs at once; the
/// immediate effect must carry the credentials the user just typed.
#[test]
fn large_file_auth_retry_runs_with_the_submitted_credentials() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    let _lock = super::staged_auth_test_lock();
    let context = gitcomet_core::services::SafePushAfterCommitContext {
        amend: false,
        local_branch: Some("adjusted/main(unlocked)".into()),
        pre_head: None,
        post_head: None,
    };
    for (operation, expected) in [
        (
            AuthRetryOperation::RepoCommand {
                repo_id: RepoId(1),
                command: RepoCommandKind::LargeFile {
                    command: C::LfsFetchAll,
                },
            },
            C::LfsFetchAll,
        ),
        (
            AuthRetryOperation::SafePushAfterCommit {
                repo_id: RepoId(1),
                context,
            },
            C::AnnexPush { content: false },
        ),
    ] {
        clear_staged_git_auth();
        let (mut repos, mut state) = setup_open_repo(RepoId(1), "/tmp/repo");
        let mut support = gitcomet_core::large_files::LargeFileSupport::default();
        support.annex.uuid = Some("here".into());
        state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
        state.repos[0].head_branch = Loadable::Ready("adjusted/main(unlocked)".into());
        state.auth_prompt = Some(AuthPromptState {
            kind: AuthPromptKind::UsernamePassword,
            reason: "auth required".into(),
            operation,
        });
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(1),
            &mut state,
            Msg::SubmitAuthPrompt {
                username: Some("alice".into()),
                secret: "test-token".into(),
            },
        );
        let auth = effects
            .iter()
            .find(|effect| matches!(effect, Effect::RunLargeFileCommand { command, .. } if *command == expected))
            .unwrap_or_else(|| panic!("expected {expected:?}: {effects:?}"))
            .git_auth()
            .unwrap_or_else(|| panic!("{expected:?} must carry the credentials"));
        assert_eq!(auth.username.as_deref(), Some("alice"));
        assert_eq!(auth.secret, "test-token");
    }
    clear_staged_git_auth();
}

#[test]
fn safe_push_auth_retry_preserves_credentials_when_annex_takeover_is_queued() {
    use gitcomet_core::large_files::LargeFileCommand as C;
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.uuid = Some("here".into());
    state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
    state.repos[0].head_branch = Loadable::Ready("adjusted/main(unlocked)".into());
    let id_alloc = AtomicU64::new(1);
    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".into(),
        operation: AuthRetryOperation::SafePushAfterCommit {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: false,
                local_branch: Some("adjusted/main(unlocked)".into()),
                pre_head: None,
                post_head: None,
            },
        },
    });
    assert!(
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SubmitAuthPrompt {
                username: Some("alice".into()),
                secret: "test-token".into(),
            }
        )
        .is_empty()
    );
    assert_eq!(state.repos[0].pending.large_file_commands.len(), 1);
    assert_eq!(
        state.repos[0].pending.large_file_commands[0]
            .auth
            .as_ref()
            .map(|auth| auth.secret.as_str()),
        Some("test-token")
    );
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::LargeFile {
                command: C::AnnexPush { content: false },
            },
            result: Ok(gitcomet_core::services::CommandOutput::default()),
        }),
    );
    assert!(effects.iter().any(|effect| matches!(effect, Effect::RunLargeFileCommand { auth: Some(auth), .. } if auth.secret == "test-token")));
    clear_staged_git_auth();
}

#[test]
fn repo_command_finished_auth_error_sets_username_password_prompt() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: Err(auth_error(
                "git push failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::UsernamePassword);
    assert!(prompt.reason.contains("could not read Username"));
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::Push,
        }
    );
}

#[test]
fn repo_command_finished_auth_error_sets_passphrase_prompt() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Err(auth_error(
                "git pull failed: Enter passphrase for key '/home/user/.ssh/id_ed25519': terminal prompts disabled",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    assert!(prompt.reason.contains("passphrase"));
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
        }
    );
}

#[test]
fn repo_command_finished_host_key_error_sets_host_verification_prompt() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Err(auth_error(
                "git pull --no-rebase origin main failed: Host key verification failed.\nfatal: Could not read from remote repository.",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::HostVerification);
    assert!(prompt.reason.contains("Host key verification failed"));
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
        }
    );
}

#[test]
fn repo_command_finished_non_auth_error_does_not_set_prompt() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: Err(auth_error(
                "git push failed: remote rejected because branch is protected",
            )),
        }),
    );

    assert!(state.auth_prompt.is_none());
}

#[test]
fn repo_command_finished_auth_error_does_not_prompt_for_non_replayable_commands() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    let commands = vec![
        RepoCommandKind::SaveWorktreeFile {
            path: PathBuf::from("README.md"),
            stage: true,
        },
        RepoCommandKind::StageHunk,
        RepoCommandKind::UnstageHunk,
        RepoCommandKind::ApplyWorktreePatch { reverse: false },
    ];

    for command in commands {
        state.auth_prompt = None;
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Err(auth_error(
                    "git command failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled",
                )),
            }),
        );
        assert!(state.auth_prompt.is_none());
    }
}

#[test]
fn commit_finished_auth_error_uses_pending_retry_and_clears_it() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.repos[0].pending.commit_retry = Some(PendingCommitRetry {
        message: "ship it".to_string(),
        amend: false,
        push_after_commit: false,
    });

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Err(auth_error(
                "git commit failed: fatal: could not read Password for 'https://example.com': terminal prompts disabled",
            )),
        }),
    );

    assert!(state.repos[0].pending.commit_retry.is_none());
    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::UsernamePassword);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::Commit {
            repo_id,
            message: "ship it".to_string(),
            amend: false,
            push_after_commit: false,
        }
    );
}

#[test]
fn commit_finished_auth_error_without_pending_retry_does_not_prompt() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Err(auth_error(
                "git commit failed: fatal: could not read Password for 'https://example.com': terminal prompts disabled",
            )),
        }),
    );

    assert!(state.auth_prompt.is_none());
}

#[test]
fn commit_amend_finished_auth_error_uses_pending_retry_with_amend() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.repos[0].pending.commit_retry = Some(PendingCommitRetry {
        message: "fixup".to_string(),
        amend: true,
        push_after_commit: false,
    });

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id,
            result: Err(auth_error(
                "git commit --amend failed: Enter passphrase for key '/home/user/.ssh/id_ed25519': terminal prompts disabled",
            )),
        }),
    );

    assert!(state.repos[0].pending.commit_retry.is_none());
    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::Commit {
            repo_id,
            message: "fixup".to_string(),
            amend: true,
            push_after_commit: false,
        }
    );
}

#[test]
fn safe_push_after_commit_auth_error_uses_safe_push_retry() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let context = gitcomet_core::services::SafePushAfterCommitContext {
        amend: true,
        local_branch: Some("main".to_string()),
        pre_head: Some(CommitId("1111111111111111111111111111111111111111".into())),
        post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: context.clone(),
            auth: None,
            result: Err(auth_error(
                "git fetch origin refs/heads/main failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::UsernamePassword);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::SafePushAfterCommit { repo_id, context }
    );
}

#[test]
fn clone_finished_auth_error_sets_clone_retry_prompt() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let url = "https://example.com/private/repo.git".to_string();
    let dest = PathBuf::from("/tmp/private-repo");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: url.clone(),
            dest: dest.clone(),
            result: Err(auth_error(
                "git clone failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::UsernamePassword);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::Clone {
            url,
            dest: dest.clone(),
        }
    );
    assert!(prompt.reason.contains("could not read Username"));
}

#[test]
fn clone_finished_ssh_publickey_error_sets_passphrase_prompt() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let url = "git@github.com:private/repo.git".to_string();
    let dest = PathBuf::from("/tmp/private-repo");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
            url: url.clone(),
            dest: dest.clone(),
            result: Err(auth_error(
                "git clone failed: git@github.com: Permission denied (publickey).\nfatal: Could not read from remote repository.",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::Clone {
            url,
            dest: dest.clone(),
        }
    );
    assert!(prompt.reason.contains("Permission denied (publickey)"));
}

#[test]
fn submit_auth_prompt_replays_repo_command_and_stages_trimmed_credentials() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::PushSetUpstream {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            },
        },
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: Some(" alice ".to_string()),
            secret: "token-123".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PushSetUpstream {
            repo_id: RepoId(1),
            remote,
            branch,
            ..
        }] if remote == "origin" && branch == "main"
    ));
    assert!(state.auth_prompt.is_none());
    assert_eq!(state.repos[0].push_in_flight, 1);

    let staged = effects[0]
        .git_auth()
        .expect("staged auth should be present");
    assert_eq!(staged.kind, GitAuthKind::UsernamePassword);
    assert_eq!(staged.username.as_deref(), Some("alice"));
    assert_eq!(staged.secret, "token-123");
}

/// The batch remote delete is one effect carrying many branches, so it needs its
/// own auth slot filled on retry — a credential prompt answered here must not
/// replay the delete unauthenticated.
#[test]
fn submit_auth_prompt_stages_credentials_for_a_batch_remote_branch_delete() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::DeleteRemoteBranches {
                remote: "origin".to_string(),
                branches: vec!["feat/a".to_string(), "feat/b".to_string()],
            },
        },
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: Some("alice".to_string()),
            secret: "token-123".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::DeleteRemoteBranches {
            repo_id: RepoId(1),
            remote,
            branches,
            ..
        }] if remote == "origin" && branches.len() == 2
    ));

    let staged = effects[0]
        .git_auth()
        .expect("staged auth should be present");
    assert_eq!(staged.kind, GitAuthKind::UsernamePassword);
    assert_eq!(staged.username.as_deref(), Some("alice"));
    assert_eq!(staged.secret, "token-123");
}

#[test]
fn submit_auth_prompt_replays_commit_and_commit_amend() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: "first".to_string(),
            amend: false,
            push_after_commit: false,
        },
    });
    let commit_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: None,
            secret: "passphrase".to_string(),
        },
    );
    assert!(matches!(
        commit_effects.as_slice(),
        [Effect::Commit {
            repo_id: RepoId(1),
            message,
            ..
        }] if message == "first"
    ));
    let commit_auth = commit_effects[0]
        .git_auth()
        .expect("commit auth should be present");
    assert_eq!(commit_auth.kind, GitAuthKind::Passphrase);
    assert_eq!(commit_auth.secret, "passphrase");
    assert_eq!(
        state.repos[0].pending.commit_retry,
        Some(PendingCommitRetry {
            message: "first".to_string(),
            amend: false,
            push_after_commit: false,
        })
    );

    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: "second".to_string(),
            amend: true,
            push_after_commit: false,
        },
    });
    let amend_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: None,
            secret: "passphrase".to_string(),
        },
    );
    assert!(matches!(
        amend_effects.as_slice(),
        [Effect::CommitAmend {
            repo_id: RepoId(1),
            message,
            ..
        }] if message == "second"
    ));
    let amend_auth = amend_effects[0]
        .git_auth()
        .expect("amend auth should be present");
    assert_eq!(amend_auth.kind, GitAuthKind::Passphrase);
    assert_eq!(amend_auth.secret, "passphrase");
    assert_eq!(
        state.repos[0].pending.commit_retry,
        Some(PendingCommitRetry {
            message: "second".to_string(),
            amend: true,
            push_after_commit: false,
        })
    );
}

#[test]
fn submit_auth_prompt_replays_safe_push_after_commit() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let context = gitcomet_core::services::SafePushAfterCommitContext {
        amend: true,
        local_branch: Some("main".to_string()),
        pre_head: Some(CommitId("1111111111111111111111111111111111111111".into())),
        post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
    };

    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::SafePushAfterCommit {
            repo_id,
            context: context.clone(),
        },
    });
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: None,
            secret: "passphrase".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::SafePushAfterCommit {
            repo_id: RepoId(1),
            context: emitted,
            ..
        }] if emitted == &context
    ));
    let auth = effects[0]
        .git_auth()
        .expect("safe push auth should be present");
    assert_eq!(auth.kind, GitAuthKind::Passphrase);
    assert_eq!(auth.secret, "passphrase");
}

#[test]
fn submit_auth_prompt_replays_clone_operation() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let url = "ssh://git@example.com/private/repo.git".to_string();
    let dest = PathBuf::from("/tmp/retry-clone");
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::Clone {
            url: url.clone(),
            dest: dest.clone(),
        },
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: None,
            secret: "passphrase".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CloneRepo {
            url: effect_url,
            dest: effect_dest,
            ..
        }] if effect_url == &url && effect_dest == &dest
    ));
    assert!(state.auth_prompt.is_none());
    let staged = effects[0].git_auth().expect("clone auth should be present");
    assert_eq!(staged.kind, GitAuthKind::Passphrase);
    assert_eq!(staged.secret, "passphrase");
}

#[test]
fn submit_auth_prompt_host_verification_replays_repo_command_and_stages_confirmation() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::HostVerification,
        reason: "Host key verification failed".to_string(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::FetchAll,
        },
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: None,
            secret: " YES ".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::FetchAll {
            repo_id: RepoId(1),
            ..
        }]
    ));
    assert!(state.auth_prompt.is_none());

    let staged = effects[0]
        .git_auth()
        .expect("staged auth should be present");
    assert_eq!(staged.kind, GitAuthKind::HostVerification);
    assert_eq!(staged.secret, "yes");
}

#[test]
fn submit_auth_prompt_validation_failure_keeps_prompt_and_sets_diagnostic() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::Push,
        },
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: Some("   ".to_string()),
            secret: "token".to_string(),
        },
    );

    assert!(effects.is_empty());
    assert!(state.auth_prompt.is_some());
    let repo_state = &state.repos[0];
    let diagnostic = repo_state
        .feedback
        .diagnostics
        .last()
        .expect("expected validation diagnostic");
    assert_eq!(diagnostic.kind, DiagnosticKind::Error);
    assert!(diagnostic.message.contains("username cannot be empty"));

    clear_staged_git_auth();
}

#[test]
fn submit_auth_prompt_without_prompt_is_noop() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: Some("alice".to_string()),
            secret: "secret".to_string(),
        },
    );

    assert!(effects.is_empty());
    assert!(state.auth_prompt.is_none());
    assert!(take_staged_git_auth().is_none());
}

#[test]
fn cancel_auth_prompt_clears_prompt_and_staged_auth() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    stage_git_auth(StagedGitAuth {
        kind: GitAuthKind::UsernamePassword,
        username: Some("alice".to_string()),
        secret: "token".to_string(),
    });

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::Push,
        },
    });

    let effects = reduce(&mut repos, &id_alloc, &mut state, Msg::CancelAuthPrompt);

    assert!(effects.is_empty());
    assert!(state.auth_prompt.is_none());
    assert!(take_staged_git_auth().is_none());
}

#[test]
fn submit_auth_prompt_replays_expected_repo_command_mappings() {
    let _lock = super::staged_auth_test_lock();

    let replay_case = |command: RepoCommandKind| {
        clear_staged_git_auth();
        let repo_id = RepoId(1);
        let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
        let id_alloc = AtomicU64::new(1);
        state.auth_prompt = Some(AuthPromptState {
            kind: AuthPromptKind::UsernamePassword,
            reason: "auth required".to_string(),
            operation: AuthRetryOperation::RepoCommand { repo_id, command },
        });
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SubmitAuthPrompt {
                username: Some("alice".to_string()),
                secret: "secret".to_string(),
            },
        );
        clear_staged_git_auth();
        effects
    };

    let fetch_effects = replay_case(RepoCommandKind::FetchAll);
    assert!(matches!(
        fetch_effects.as_slice(),
        [Effect::FetchAll {
            repo_id: RepoId(1),
            ..
        }]
    ));

    let pull_branch_effects = replay_case(RepoCommandKind::PullBranch {
        remote: "origin".to_string(),
        branch: "main".to_string(),
    });
    assert!(matches!(
        pull_branch_effects.as_slice(),
        [Effect::PullBranch {
            repo_id: RepoId(1),
            remote,
            branch,
            ..
        }] if remote == "origin" && branch == "main"
    ));

    let force_with_lease_effects = replay_case(RepoCommandKind::ForcePushWithLease {
        lease: gitcomet_core::services::ForcePushLease {
            remote: "origin".to_string(),
            branch: "main".to_string(),
            expected: CommitId("1111111111111111111111111111111111111111".into()),
            local_branch: "main".to_string(),
            local_head: CommitId("2222222222222222222222222222222222222222".into()),
        },
    });
    assert!(matches!(
        force_with_lease_effects.as_slice(),
        [Effect::ForcePushWithLease {
            repo_id: RepoId(1),
            lease,
            ..
        }] if lease.remote == "origin" && lease.branch == "main"
    ));

    let push_after_commit_target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };
    let push_after_commit_effects = replay_case(RepoCommandKind::PushAfterCommit {
        target: push_after_commit_target.clone(),
        set_upstream: true,
    });
    assert!(matches!(
        push_after_commit_effects.as_slice(),
        [Effect::PushAfterCommit {
            repo_id: RepoId(1),
            target,
            set_upstream: true,
            ..
        }] if target == &push_after_commit_target
    ));

    let unset_upstream_effects = replay_case(RepoCommandKind::UnsetUpstreamBranch {
        branch: "feature/current".to_string(),
    });
    assert!(matches!(
        unset_upstream_effects.as_slice(),
        [Effect::UnsetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
        }] if branch == "feature/current"
    ));

    let set_upstream_effects = replay_case(RepoCommandKind::SetUpstreamBranch {
        branch: "feature/current".to_string(),
        upstream: Upstream {
            remote: "origin".to_string(),
            branch: "feature/current".to_string(),
        },
    });
    assert!(matches!(
        set_upstream_effects.as_slice(),
        [Effect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        }] if branch == "feature/current"
            && upstream.remote == "origin"
            && upstream.branch == "feature/current"
    ));

    let reset_effects = replay_case(RepoCommandKind::Reset {
        mode: ResetMode::Mixed,
        target: "HEAD~1".to_string(),
    });
    assert!(matches!(
        reset_effects.as_slice(),
        [Effect::Reset {
            repo_id: RepoId(1),
            target,
            mode: ResetMode::Mixed,
        }] if target == "HEAD~1"
    ));

    let set_remote_url_effects = replay_case(RepoCommandKind::SetRemoteUrl {
        name: "origin".to_string(),
        url: "https://example.com/repo.git".to_string(),
        kind: RemoteUrlKind::Push,
    });
    assert!(matches!(
        set_remote_url_effects.as_slice(),
        [Effect::SetRemoteUrl {
            repo_id: RepoId(1),
            name,
            url,
            kind: RemoteUrlKind::Push,
            ..
        }] if name == "origin" && url == "https://example.com/repo.git"
    ));

    let checkout_conflict_effects = replay_case(RepoCommandKind::CheckoutConflict {
        path: PathBuf::from("conflicted.txt"),
        side: ConflictSide::Ours,
    });
    assert!(matches!(
        checkout_conflict_effects.as_slice(),
        [Effect::CheckoutConflictSide {
            repo_id: RepoId(1),
            path,
            side: ConflictSide::Ours,
        }] if path == &PathBuf::from("conflicted.txt")
    ));

    let remove_submodule_effects = replay_case(RepoCommandKind::RemoveSubmodule {
        path: PathBuf::from("vendor/lib"),
    });
    assert!(matches!(
        remove_submodule_effects.as_slice(),
        [Effect::RemoveSubmodule {
            repo_id: RepoId(1),
            path,
        }] if path == &PathBuf::from("vendor/lib")
    ));

    // A revert is replayed whole with the staged auth: the backend resumes one
    // stopped at its commit step, and a `--no-commit` fetch failure reruns.
    for commit in [true, false] {
        let revert_effects = replay_case(RepoCommandKind::Revert {
            commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
            commit,
            mainline: Some(1),
            summary: "revert me".to_string(),
        });
        assert!(
            matches!(
                revert_effects.as_slice(),
                [Effect::RevertCommit {
                    repo_id: RepoId(1),
                    commit: replayed,
                    mainline: Some(1),
                    auth: Some(_),
                    ..
                }] if *replayed == commit
            ),
            "commit={commit}: {revert_effects:?}"
        );
    }

    // A single pick is replayed whole too: one beside staged work rolled its
    // failed commit step back, and one stopped at the commit step resumes.
    for commit in [true, false] {
        let pick_effects = replay_case(RepoCommandKind::CherryPick {
            commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
            commit,
            mainline: None,
            summary: "pick me".to_string(),
        });
        assert!(
            matches!(
                pick_effects.as_slice(),
                [Effect::CherryPickCommit {
                    repo_id: RepoId(1),
                    commit: replayed,
                    mainline: None,
                    auth: Some(_),
                    ..
                }] if *replayed == commit
            ),
            "commit={commit}: {pick_effects:?}"
        );
    }

    // A committing multi-pick continues git's paused sequencer; an
    // uncommitted one keeps no sequencer and is replayed whole.
    let multi_entries = vec![gitcomet_core::services::InteractiveRebaseEntry {
        action: gitcomet_core::services::InteractiveRebaseAction::Pick,
        commit_id: "deadbeef".to_string(),
        summary: "pick me".to_string(),
        message: "pick me".to_string(),
        new_message: None,
    }];
    let committing_multi_effects = replay_case(RepoCommandKind::InteractiveCherryPick {
        entries: multi_entries.clone(),
        commit: true,
    });
    assert!(
        matches!(
            committing_multi_effects.as_slice(),
            [Effect::RebaseContinue {
                repo_id: RepoId(1),
                auth: Some(_),
            }]
        ),
        "{committing_multi_effects:?}"
    );
    let uncommitted_multi_effects = replay_case(RepoCommandKind::InteractiveCherryPick {
        entries: multi_entries.clone(),
        commit: false,
    });
    assert!(
        matches!(
            uncommitted_multi_effects.as_slice(),
            [Effect::InteractiveCherryPick {
                repo_id: RepoId(1),
                entries,
                commit: false,
            }] if entries == &multi_entries
        ),
        "{uncommitted_multi_effects:?}"
    );

    // A failure before staging replays the apply; a commit failure carries
    // the checkpoint so the authenticated retry commits only that result.
    let apply_target = gitcomet_core::domain::ApplyChangeTarget::commit(
        gitcomet_core::domain::CommitId("deadbeef".into()),
        PathBuf::from("a.txt"),
    );
    let retry = gitcomet_core::domain::ApplyFileChangeRetry {
        target: apply_target.clone(),
        head: Some(CommitId("12345678".into())),
        index: Vec::new(),
    };
    for commit_retry in [None, Some(retry)] {
        let apply_effects = replay_case(RepoCommandKind::ApplyFileChange {
            commit_retry: commit_retry.clone(),
            target: apply_target.clone(),
            commit: true,
        });
        assert!(
            matches!(apply_effects.as_slice(), [Effect::ApplyFileChange {
            commit_retry: replayed,
            repo_id: RepoId(1), target, commit: true, auth: Some(_),
        }] if target == &apply_target && replayed == &commit_retry),
            "{apply_effects:?}"
        );
    }

    let non_replayable_effects = replay_case(RepoCommandKind::StageHunk);
    assert!(non_replayable_effects.is_empty());
}

#[test]
fn revert_signing_passphrase_failure_sets_passphrase_prompt() {
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let command = RepoCommandKind::Revert {
        commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
        commit: true,
        mainline: None,
        summary: "revert me".to_string(),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: command.clone(),
            result: Err(auth_error(
                "git commit --no-verify -F MERGE_MSG failed: Enter passphrase for key '/home/user/.ssh/id_ed25519': terminal prompts disabled",
            )),
        }),
    );

    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::RepoCommand { repo_id, command }
    );
}

#[test]
fn tag_push_auth_retry_preserves_mode_destination_and_upstream_choice() {
    use gitcomet_core::tag_push::{TagPushMode, TagPushRequest};
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();
    for mode in TagPushMode::ALL {
        let repo_id = RepoId(1);
        let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/tag-push-auth");
        let request = TagPushRequest {
            mode,
            remote: "publish".into(),
            branch: "releases/stable".into(),
            local_branch: "main".into(),
            head: gitcomet_core::domain::CommitId("1234567".into()),
            set_upstream: true,
        };
        state.auth_prompt = Some(AuthPromptState {
            kind: AuthPromptKind::UsernamePassword,
            reason: "auth required".into(),
            operation: AuthRetryOperation::RepoCommand {
                repo_id,
                command: RepoCommandKind::PushWithTags {
                    request: request.clone(),
                },
            },
        });
        let effects = reduce(
            &mut repos,
            &AtomicU64::new(1),
            &mut state,
            Msg::SubmitAuthPrompt {
                username: Some("alice".into()),
                secret: "test-token".into(),
            },
        );
        assert!(
            matches!(effects.as_slice(), [Effect::PushWithTags { request: retry, .. }] if retry == &request)
        );
        assert_eq!(effects[0].git_auth().unwrap().secret, "test-token");
        assert_eq!(state.repos[0].push_in_flight, 1);
        assert!(state.auth_prompt.is_none());
    }
    clear_staged_git_auth();
}

#[test]
fn tag_push_preview_discards_stale_results_and_does_not_start_auth_or_mark_push_in_flight() {
    use gitcomet_core::services::CancellationToken;
    use gitcomet_core::tag_push::{TagPushMode, TagPushPreview, TagPushRequest};
    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/tag-preview");
    let id_alloc = AtomicU64::new(1);
    let mut request = TagPushRequest {
        mode: TagPushMode::All,
        remote: "origin".into(),
        branch: "main".into(),
        local_branch: "main".into(),
        head: gitcomet_core::domain::CommitId("1234567".into()),
        set_upstream: true,
    };
    let old_token = CancellationToken::new();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PreviewTagPush {
            repo_id,
            request: request.clone(),
            cancellation: old_token.clone(),
        },
    );
    let [
        Effect::PreviewTagPush {
            generation: old_generation,
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("preview expected")
    };
    let old_generation = *old_generation;
    request.remote = "publish".into();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PreviewTagPush {
            repo_id,
            request,
            cancellation: CancellationToken::new(),
        },
    );
    assert!(old_token.is_cancelled());
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::TagPushPreviewLoaded {
            repo_id,
            mode: TagPushMode::All,
            generation: old_generation,
            result: Ok(TagPushPreview::default()),
        }),
    );
    assert!(
        state.repos[0].tag_push_previews[1]
            .as_ref()
            .unwrap()
            .result
            .is_loading()
    );
    let generation = state.repos[0].tag_push_previews[1]
        .as_ref()
        .unwrap()
        .generation;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::TagPushPreviewLoaded {
            repo_id,
            mode: TagPushMode::All,
            generation,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("authentication required".into()),
            )),
        }),
    );
    assert!(matches!(
        &state.repos[0].tag_push_previews[1].as_ref().unwrap().result,
        crate::model::Loadable::Error(_)
    ));
    assert!(state.auth_prompt.is_none());
    assert_eq!(state.repos[0].push_in_flight, 0);
}

/// A refspec fetch that needs credentials replays with exactly its remote
/// and refspecs, authenticated, like a full fetch.
#[test]
fn submit_auth_prompt_replays_a_refspec_fetch() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    let command = RepoCommandKind::FetchRefspecs {
        remote: "origin".to_string(),
        refspecs: vec!["+refs/pull/7/head:refs/remotes/origin/pr/7".to_string()],
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: command.clone(),
            result: Err(auth_error(
                "git fetch failed: fatal: could not read Username for 'https://example.com': terminal prompts disabled",
            )),
        }),
    );
    assert_eq!(
        state.auth_prompt.as_ref().map(|prompt| &prompt.operation),
        Some(&AuthRetryOperation::RepoCommand { repo_id, command })
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SubmitAuthPrompt {
            username: Some("alice".to_string()),
            secret: "token".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::FetchRefspecs { remote, refspecs, .. }]
            if remote == "origin" && refspecs == &["+refs/pull/7/head:refs/remotes/origin/pr/7"]
    ));
    assert!(effects[0].git_auth().is_some());
    assert_eq!(state.repos[0].pull_in_flight, 1);
}
