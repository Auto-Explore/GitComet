use super::*;
use crate::model::{AuthPromptKind, AuthPromptState, AuthRetryOperation, PendingCommitRetry};
use gitcomet_core::auth::{
    GitAuthKind, StagedGitAuth, clear_staged_git_auth, stage_git_auth, take_staged_git_auth,
};
use gitcomet_core::services::{ConflictSide, RemoteUrlKind, ResetMode};

fn auth_error(message: &str) -> Error {
    Error::new(ErrorKind::Backend(message.to_string()))
}

fn setup_open_repo(
    repo_id: RepoId,
    workdir: &str,
) -> (HashMap<RepoId, Arc<dyn GitRepository>>, AppState) {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    repos.insert(repo_id, Arc::new(DummyRepo::new(workdir)));

    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from(workdir),
        },
    ));
    state.active_repo = Some(repo_id);
    (repos, state)
}

fn effect_git_auth(effect: &Effect) -> Option<&StagedGitAuth> {
    match effect {
        Effect::CloneRepo { auth, .. }
        | Effect::AddSubmodule { auth, .. }
        | Effect::UpdateSubmodules { auth, .. }
        | Effect::AddSubtree { auth, .. }
        | Effect::PullSubtree { auth, .. }
        | Effect::PushSubtree { auth, .. }
        | Effect::Commit { auth, .. }
        | Effect::CommitAmend { auth, .. }
        | Effect::FetchAll { auth, .. }
        | Effect::Pull { auth, .. }
        | Effect::PullBranch { auth, .. }
        | Effect::Push { auth, .. }
        | Effect::ForcePush { auth, .. }
        | Effect::PushSetUpstream { auth, .. }
        | Effect::DeleteRemoteBranch { auth, .. }
        | Effect::PushTag { auth, .. }
        | Effect::DeleteRemoteTag { auth, .. } => auth.as_ref(),
        _ => None,
    }
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
    state.repos[0].pending_commit_retry = Some(PendingCommitRetry {
        message: "ship it".to_string(),
        amend: false,
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

    assert!(state.repos[0].pending_commit_retry.is_none());
    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::UsernamePassword);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::Commit {
            repo_id,
            message: "ship it".to_string(),
            amend: false,
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
    state.repos[0].pending_commit_retry = Some(PendingCommitRetry {
        message: "fixup".to_string(),
        amend: true,
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

    assert!(state.repos[0].pending_commit_retry.is_none());
    let prompt = state.auth_prompt.expect("expected auth prompt");
    assert_eq!(prompt.kind, AuthPromptKind::Passphrase);
    assert_eq!(
        prompt.operation,
        AuthRetryOperation::Commit {
            repo_id,
            message: "fixup".to_string(),
            amend: true,
        }
    );
}

#[test]
fn clone_finished_auth_error_sets_clone_retry_prompt() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
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
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
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
fn submit_auth_prompt_clears_stale_repo_banner_before_retry() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let repo_id = RepoId(1);
    let (mut repos, mut state) = setup_open_repo(repo_id, "/tmp/repo");
    let id_alloc = AtomicU64::new(1);
    state.banner_error = Some(crate::model::BannerErrorState {
        repo_id: Some(repo_id),
        message: "git pull failed: Enter passphrase for key '/home/user/.ssh/id_ed25519'"
            .to_string(),
    });
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::RepoCommand {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
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
        [Effect::Pull {
            repo_id: RepoId(1),
            mode: PullMode::Default,
            ..
        }]
    ));
    assert!(state.banner_error.is_none());
    assert!(state.auth_prompt.is_none());
    clear_staged_git_auth();
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

    let staged = effect_git_auth(&effects[0]).expect("staged auth should be present");
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
    let commit_auth = effect_git_auth(&commit_effects[0]).expect("commit auth should be present");
    assert_eq!(commit_auth.kind, GitAuthKind::Passphrase);
    assert_eq!(commit_auth.secret, "passphrase");
    assert_eq!(
        state.repos[0].pending_commit_retry,
        Some(PendingCommitRetry {
            message: "first".to_string(),
            amend: false,
        })
    );

    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::Commit {
            repo_id,
            message: "second".to_string(),
            amend: true,
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
    let amend_auth = effect_git_auth(&amend_effects[0]).expect("amend auth should be present");
    assert_eq!(amend_auth.kind, GitAuthKind::Passphrase);
    assert_eq!(amend_auth.secret, "passphrase");
    assert_eq!(
        state.repos[0].pending_commit_retry,
        Some(PendingCommitRetry {
            message: "second".to_string(),
            amend: true,
        })
    );
}

#[test]
fn submit_auth_prompt_replays_clone_operation() {
    let _lock = super::staged_auth_test_lock();
    clear_staged_git_auth();

    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
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
    let staged = effect_git_auth(&effects[0]).expect("clone auth should be present");
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
            prune: _,
            ..
        }]
    ));
    assert!(state.auth_prompt.is_none());

    let staged = effect_git_auth(&effects[0]).expect("staged auth should be present");
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
            prune: _,
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
        upstream: "origin/feature/current".to_string(),
    });
    assert!(matches!(
        set_upstream_effects.as_slice(),
        [Effect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream,
        }] if branch == "feature/current" && upstream == "origin/feature/current"
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

    let add_subtree_effects = replay_case(RepoCommandKind::AddSubtree {
        repository: "https://example.com/sub.git".to_string(),
        reference: "main".to_string(),
        path: PathBuf::from("vendor/lib"),
        squash: true,
    });
    assert!(matches!(
        add_subtree_effects.as_slice(),
        [Effect::AddSubtree {
            repo_id: RepoId(1),
            repository,
            reference,
            path,
            squash: true,
            auth: Some(_),
        }] if repository == "https://example.com/sub.git"
            && reference == "main"
            && path == &PathBuf::from("vendor/lib")
    ));

    let pull_subtree_effects = replay_case(RepoCommandKind::PullSubtree {
        repository: "https://example.com/sub.git".to_string(),
        reference: "main".to_string(),
        path: PathBuf::from("vendor/lib"),
        squash: false,
    });
    assert!(matches!(
        pull_subtree_effects.as_slice(),
        [Effect::PullSubtree {
            repo_id: RepoId(1),
            repository,
            reference,
            path,
            squash: false,
            auth: Some(_),
        }] if repository == "https://example.com/sub.git"
            && reference == "main"
            && path == &PathBuf::from("vendor/lib")
    ));

    let push_subtree_effects = replay_case(RepoCommandKind::PushSubtree {
        repository: "https://example.com/sub.git".to_string(),
        refspec: "main".to_string(),
        path: PathBuf::from("vendor/lib"),
    });
    assert!(matches!(
        push_subtree_effects.as_slice(),
        [Effect::PushSubtree {
            repo_id: RepoId(1),
            repository,
            refspec,
            path,
            auth: Some(_),
        }] if repository == "https://example.com/sub.git"
            && refspec == "main"
            && path == &PathBuf::from("vendor/lib")
    ));

    let split_subtree_effects = replay_case(RepoCommandKind::SplitSubtree {
        path: PathBuf::from("vendor/lib"),
        branch: Some("subtree-split".to_string()),
    });
    assert!(matches!(
        split_subtree_effects.as_slice(),
        [Effect::SplitSubtree {
            repo_id: RepoId(1),
            path,
            branch: Some(branch),
        }] if path == &PathBuf::from("vendor/lib") && branch == "subtree-split"
    ));

    let remove_subtree_effects = replay_case(RepoCommandKind::RemoveSubtree {
        path: PathBuf::from("vendor/lib"),
    });
    assert!(matches!(
        remove_subtree_effects.as_slice(),
        [Effect::RemoveSubtree {
            repo_id: RepoId(1),
            path,
        }] if path == &PathBuf::from("vendor/lib")
    ));

    let non_replayable_effects = replay_case(RepoCommandKind::StageHunk);
    assert!(non_replayable_effects.is_empty());
}
