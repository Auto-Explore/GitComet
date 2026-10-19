use super::*;

#[test]
fn pull_and_push_mark_in_flight_until_command_finished() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    let repo_id = RepoId(1);
    let workdir = PathBuf::from("/tmp/repo");
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.clone(),
        },
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Fetch(crate::msg::FetchMsg::All { repo_id }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneMergedBranches { repo_id },
    );
    assert_eq!(state.repos[0].pull_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PruneLocalTags { repo_id },
    );
    assert_eq!(state.repos[0].pull_in_flight, 4);

    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    assert_eq!(state.repos[0].push_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushTag {
            repo_id,
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteRemoteTag {
            repo_id,
            remote: "origin".to_string(),
            name: "v1.0.0".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 4);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "main".to_string(),
        },
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::FetchAll,
            result: Ok(CommandOutput::empty_success("git fetch --all")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Ok(CommandOutput::empty_success("git pull")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PruneMergedBranches,
            result: Ok(CommandOutput::empty_success("git prune merged branches")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PruneLocalTags,
            result: Ok(CommandOutput::empty_success("git prune local tags")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 0);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Push,
            result: Ok(CommandOutput::empty_success("git push")),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 3);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteRemoteBranch {
                remote: "origin".to_string(),
                branch: "feature".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git push origin --delete feature",
            )),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 2);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PushTag {
                remote: "origin".to_string(),
                name: "v1.0.0".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git push origin refs/tags/v1.0.0",
            )),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::DeleteRemoteTag {
                remote: "origin".to_string(),
                name: "v1.0.0".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git push origin --delete refs/tags/v1.0.0",
            )),
        }),
    );
    assert_eq!(state.repos[0].push_in_flight, 0);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::UnsetUpstreamBranch {
                branch: "main".to_string(),
            },
            result: Ok(CommandOutput::empty_success(
                "git branch --unset-upstream main",
            )),
        }),
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
}

#[test]
fn pull_and_push_do_not_mark_in_flight_before_repo_is_opened() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

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
        Msg::Fetch(crate::msg::FetchMsg::All { repo_id }),
    );
    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });

    assert_eq!(state.repos[0].pull_in_flight, 0);
    assert_eq!(state.repos[0].push_in_flight, 0);
}

#[test]
fn pull_stopped_at_conflicts_is_summarized_as_a_conflict_stop() {
    use gitcomet_core::error::{GitFailure, GitFailureId};

    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let error = Error::new(ErrorKind::Git(GitFailure::new(
        "git pull --no-rebase --ff --no-prune . dev",
        GitFailureId::StoppedAtConflicts,
        Some(1),
        b"CONFLICT (content): Merge conflict in a.txt\n".to_vec(),
        b"From .\n * branch dev -> FETCH_HEAD\n".to_vec(),
        Some(
            "Merge conflict in a.txt. Resolve it, then commit, or abort the merge.\n\nCONFLICT (content): Merge conflict in a.txt"
                .to_string(),
        ),
    )));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PullBranch {
                remote: ".".to_string(),
                branch: "dev".to_string(),
            },
            result: Err(error),
        }),
    );

    let summary = &state.repos[0].feedback.command_log[0].summary;
    assert!(
        summary.starts_with(
            "Pull stopped at conflicts:\n\n    git pull --no-rebase --ff --no-prune . dev\n\n    Merge conflict in a.txt."
        ),
        "{summary}"
    );
}

#[test]
fn pull_error_is_formatted_as_command_and_output() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let message = "git pull --no-rebase origin main failed: From https://example.com\n * branch main -> FETCH_HEAD\nfatal: refusing to merge unrelated histories".to_string();
    let error = Error::new(ErrorKind::Backend(message));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::PullBranch {
                remote: "origin".to_string(),
                branch: "main".to_string(),
            },
            result: Err(error),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.feedback.diagnostics.is_empty());
    assert_eq!(repo_state.feedback.command_log.len(), 1);

    let summary = &repo_state.feedback.command_log[0].summary;
    assert!(summary.starts_with("Pull failed:\n\n    git pull --no-rebase origin main"));
    assert!(summary.contains(
        "\n\n    From https://example.com\n     * branch main -> FETCH_HEAD\n    fatal: refusing to merge unrelated histories"
    ));
    assert!(!summary.contains("\\n"));
    assert_eq!(
        repo_state.feedback.last_error.as_deref(),
        Some(summary.as_str())
    );
}

#[test]
fn fetch_all_emits_effect_with_global_prune_setting() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();

    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    let repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    state.repos.push(repo_state);
    state.remote_settings.prune_deleted_remote_branches_on_fetch = false;

    let fetch_without_prune = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Fetch(crate::msg::FetchMsg::All { repo_id }),
    );
    assert!(matches!(
        fetch_without_prune.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::FetchAll {
            repo_id: RepoId(1),
            prune: false,
            ..
        })]
    ));
    assert_eq!(state.repos[0].pull_in_flight, 1);

    state.remote_settings.prune_deleted_remote_branches_on_fetch = true;
    let fetch_with_prune = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Fetch(crate::msg::FetchMsg::All { repo_id }),
    );
    assert!(matches!(
        fetch_with_prune.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::FetchAll {
            repo_id: RepoId(1),
            prune: true,
            ..
        })]
    ));
    assert_eq!(state.repos[0].pull_in_flight, 2);
}

#[test]
fn a_refspec_fetch_is_a_fetch_in_flight_and_refreshes_remote_branches_when_done() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let refspecs = vec!["refs/heads/topic:refs/remotes/origin/topic".to_string()];

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Fetch(crate::msg::FetchMsg::Refspecs {
            repo_id,
            remote: "origin".to_string(),
            refspecs: refspecs.clone(),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::FetchRefspecs { remote, refspecs: sent, auth: None, .. })]
            if remote == "origin" && sent == &refspecs
    ));
    assert_eq!(state.repos[0].pull_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::FetchRefspecs {
                remote: "origin".to_string(),
                refspecs,
            },
            result: Ok(CommandOutput::empty_success("git fetch origin")),
        }),
    );
    assert_eq!(state.repos[0].pull_in_flight, 0);
    assert!(
        matches!(state.repos[0].remote_branches, Loadable::Loading),
        "a fetch reloads the remote branches"
    );
}

#[test]
fn pull_variants_snapshot_the_global_remote_prune_setting() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    state.remote_settings.prune_deleted_remote_branches_on_fetch = false;
    let pull = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Rebase,
        },
    );
    assert!(matches!(
        pull.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::Pull {
            repo_id: RepoId(1),
            mode: PullMode::Rebase,
            prune: false,
            ..
        })]
    ));

    state.remote_settings.prune_deleted_remote_branches_on_fetch = true;
    let pull_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PullBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        },
    );
    assert!(matches!(
        pull_branch.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::PullBranch {
            repo_id: RepoId(1),
            remote,
            branch,
            prune: true,
            ..
        })] if remote == "origin" && branch == "feature"
    ));
}

#[test]
fn pull_push_bump_ops_rev() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let ops_before = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );
    assert!(
        state.repos[0].ops_rev > ops_before,
        "ops_rev should bump after Pull"
    );
    let ops_after_pull = state.repos[0].ops_rev;

    reduce(&mut repos, &id_alloc, &mut state, Msg::Push { repo_id });
    assert!(
        state.repos[0].ops_rev > ops_after_pull,
        "ops_rev should bump after Push"
    );
    let ops_after_push = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Pull {
                mode: PullMode::Default,
            },
            result: Ok(CommandOutput::empty_success("git pull")),
        }),
    );
    assert!(
        state.repos[0].ops_rev > ops_after_push,
        "ops_rev should bump when command finishes"
    );
}

#[test]
fn pull_branch_and_extended_push_commands_bump_in_flight_and_ops_rev() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let ops_before = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PullBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "main".to_string(),
        },
    );
    assert_eq!(state.repos[0].pull_in_flight, 1);
    assert!(
        state.repos[0].ops_rev > ops_before,
        "ops_rev should bump after PullBranch"
    );
    let ops_after_pull_branch = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForcePush { repo_id },
    );
    assert_eq!(state.repos[0].push_in_flight, 1);
    assert!(
        state.repos[0].ops_rev > ops_after_pull_branch,
        "ops_rev should bump after ForcePush"
    );
    let ops_after_force_push = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature/test".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);
    assert!(
        state.repos[0].ops_rev > ops_after_force_push,
        "ops_rev should bump after PushSetUpstream"
    );
    let ops_after_push_set_upstream = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/test".to_string(),
            upstream: upstream_target("origin", "feature/test"),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);
    assert!(
        state.repos[0].ops_rev > ops_after_push_set_upstream,
        "ops_rev should bump after SetUpstreamBranch"
    );
    let ops_after_set_upstream_branch = state.repos[0].ops_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::UnsetUpstreamBranch {
            repo_id,
            branch: "feature/test".to_string(),
        },
    );
    assert_eq!(state.repos[0].push_in_flight, 2);
    assert!(
        state.repos[0].ops_rev > ops_after_set_upstream_branch,
        "ops_rev should bump after UnsetUpstreamBranch"
    );
}

#[test]
fn pull_push_do_not_bump_unrelated_revs() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let status_before = state.repos[0].status_rev;
    let log_before = state.repos[0].history_state.log_rev;
    let selected_before = state.repos[0].history_state.selected_commit_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Pull {
            repo_id,
            mode: PullMode::Default,
        },
    );

    assert_eq!(state.repos[0].status_rev, status_before);
    assert_eq!(state.repos[0].history_state.log_rev, log_before);
    assert_eq!(
        state.repos[0].history_state.selected_commit_rev,
        selected_before
    );
}

#[test]
fn pull_branch_and_push_variants_mark_in_flight_when_repo_is_opened() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let pull_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PullBranch {
            repo_id,
            remote: "origin".to_string(),
            branch: "main".to_string(),
        },
    );
    assert!(matches!(
        pull_branch.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::PullBranch {
            repo_id: RepoId(1),
            remote,
            branch,
            ..
        })] if remote == "origin" && branch == "main"
    ));
    assert_eq!(state.repos[0].pull_in_flight, 1);

    let force_push = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForcePush { repo_id },
    );
    assert!(matches!(
        force_push.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::ForcePush {
            repo_id: RepoId(1),
            ..
        })]
    ));
    assert_eq!(state.repos[0].push_in_flight, 1);

    let push_set_upstream = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::PushSetUpstream {
            repo_id,
            remote: "origin".to_string(),
            branch: "feature/xyz".to_string(),
        },
    );
    assert!(matches!(
        push_set_upstream.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::PushSetUpstream {
            repo_id: RepoId(1),
            remote,
            branch,
            ..
        })] if remote == "origin" && branch == "feature/xyz"
    ));
    assert_eq!(state.repos[0].push_in_flight, 2);

    let set_upstream_branch = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetUpstreamBranch {
            repo_id,
            branch: "feature/local".to_string(),
            upstream: upstream_target("origin", "feature/xyz"),
        },
    );
    assert!(matches!(
        set_upstream_branch.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SetUpstreamBranch {
            repo_id: RepoId(1),
            branch,
            upstream
        })] if branch == "feature/local" && upstream == &upstream_target("origin", "feature/xyz")
    ));
    assert_eq!(state.repos[0].push_in_flight, 2);
}

#[test]
fn commit_finished_push_after_commit_enqueues_safe_push_only_on_success() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].pending.commit_retry = Some(crate::model::PendingCommitRetry {
        message: "ship".to_string(),
        amend: false,
        push_after_commit: true,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome {
                local_branch: Some("main".to_string()),
                pre_head: Some(CommitId("1111111111111111111111111111111111111111".into())),
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            }),
        }),
    );

    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::RepoCommand(RepoCommandEffect::SafePushAfterCommit { repo_id: id, context, .. }) if *id == repo_id
                && !context.amend
                && context.local_branch.as_deref() == Some("main")
                && context.pre_head.as_ref().is_some_and(|id| id.as_ref() == "1111111111111111111111111111111111111111")
                && context.post_head.as_ref().is_some_and(|id| id.as_ref() == "2222222222222222222222222222222222222222")))
    );
    assert_eq!(state.repos[0].push_in_flight, 0);
    assert!(state.repos[0].pending.commit_retry.is_none());

    state.repos[0].push_in_flight = 0;
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].pending.commit_retry = Some(crate::model::PendingCommitRetry {
        message: "ship".to_string(),
        amend: false,
        push_after_commit: true,
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend("commit boom".to_string()))),
        }),
    );

    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::RepoCommand(RepoCommandEffect::SafePushAfterCommit { .. })
    )));
    assert_eq!(state.repos[0].push_in_flight, 0);
}

#[test]
fn safe_push_after_commit_decision_push_enqueues_checked_push() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let auth = gitcomet_core::auth::StagedGitAuth {
        challenge: None,
        kind: gitcomet_core::auth::GitAuthKind::UsernamePassword,
        username: Some("alice".to_string()),
        secret: "token".to_string(),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: false,
                local_branch: Some("main".to_string()),
                pre_head: None,
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: Some(auth.clone()),
            result: Ok(gitcomet_core::services::SafePushAfterCommitDecision::Push {
                target: target.clone(),
            }),
        }),
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RepoCommand(RepoCommandEffect::PushAfterCommit {
            repo_id: id,
            target: effect_target,
            set_upstream: false,
            auth: Some(effect_auth),
        }) if *id == repo_id && effect_target == &target && effect_auth == &auth
    )));
    assert_eq!(state.repos[0].push_in_flight, 1);
}

#[test]
fn safe_push_after_commit_decision_push_set_upstream_enqueues_checked_push() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let target = gitcomet_core::services::SafePushAfterCommitTarget {
        remote: "origin".to_string(),
        branch: "feature".to_string(),
        local_branch: "feature".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: false,
                local_branch: Some("feature".to_string()),
                pre_head: None,
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: None,
            result: Ok(
                gitcomet_core::services::SafePushAfterCommitDecision::PushSetUpstream {
                    target: target.clone(),
                },
            ),
        }),
    );

    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::RepoCommand(RepoCommandEffect::PushAfterCommit {
            repo_id: id,
            target: effect_target,
            set_upstream: true,
            ..
        }) if *id == repo_id && effect_target == &target
    )));
    assert_eq!(state.repos[0].push_in_flight, 1);
}

#[test]
fn safe_push_after_commit_published_amend_block_stores_lease_offer() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let expected = CommitId("1111111111111111111111111111111111111111".into());
    let lease = gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: expected.clone(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: true,
                local_branch: Some("main".to_string()),
                pre_head: Some(expected.clone()),
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: None,
            result: Ok(
                gitcomet_core::services::SafePushAfterCommitDecision::Blocked {
                    summary: "published amend".to_string(),
                    lease: Some(lease.clone()),
                },
            ),
        }),
    );

    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::RepoCommand(RepoCommandEffect::Push { .. })
            | Effect::RepoCommand(RepoCommandEffect::PushAfterCommit { .. })
            | Effect::RepoCommand(RepoCommandEffect::PushSetUpstream { .. })
    )));
    assert_eq!(state.repos[0].pending.force_push_lease, Some(lease));
    assert!(
        state.repos[0]
            .feedback
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("Push after commit blocked")
    );
}

#[test]
fn safe_push_after_commit_published_amend_lease_survives_followup_git_state_refresh() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let expected = CommitId("1111111111111111111111111111111111111111".into());
    let lease = gitcomet_core::services::ForcePushLease {
        remote: "origin".to_string(),
        branch: "main".to_string(),
        expected: expected.clone(),
        local_branch: "main".to_string(),
        local_head: CommitId("2222222222222222222222222222222222222222".into()),
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::SafePushAfterCommitFinished {
            repo_id,
            context: gitcomet_core::services::SafePushAfterCommitContext {
                amend: true,
                local_branch: Some("main".to_string()),
                pre_head: Some(expected),
                post_head: Some(CommitId("2222222222222222222222222222222222222222".into())),
            },
            auth: None,
            result: Ok(
                gitcomet_core::services::SafePushAfterCommitDecision::Blocked {
                    summary: "published amend".to_string(),
                    lease: Some(lease.clone()),
                },
            ),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert_eq!(state.repos[0].pending.force_push_lease, Some(lease));
}
