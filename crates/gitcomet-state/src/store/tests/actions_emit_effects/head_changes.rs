use super::*;

#[test]
fn commit_emits_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Commit {
            repo_id: RepoId(1),
            message: "hello".to_string(),
            push_after_commit: false,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::Commit { repo_id: RepoId(1), message, .. } ] if message == "hello"
    ));
}

#[test]
fn reset_emits_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Reset {
            repo_id: RepoId(1),
            target: "HEAD~1".to_string(),
            mode: gitcomet_core::services::ResetMode::Hard,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::Reset { repo_id: RepoId(1), target, mode: gitcomet_core::services::ResetMode::Hard })]
            if target == "HEAD~1"
    ));
}

#[test]
fn revert_commit_emits_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevertCommit {
            repo_id: RepoId(1),
            commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
            commit: false,
            mainline: Some(2),
            summary: "revert me".into(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RevertCommit {
            repo_id: RepoId(1),
            commit_id: _,
            commit: false,
            mainline: Some(2),
            summary,
            auth: None,
        })] if summary == "revert me"
    ));
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
}

#[test]
fn commit_amend_emits_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CommitAmend {
            repo_id: RepoId(1),
            message: "amended".to_string(),
            push_after_commit: false,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CommitAmend { repo_id: RepoId(1), message, .. }] if message == "amended"
    ));
}

#[test]
fn create_rename_and_delete_branch_emit_effects() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CreateBranch {
            repo_id: RepoId(1),
            name: "feature".to_string(),
            target: "HEAD".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateBranch {
            repo_id: RepoId(1),
            name,
            target,
        }] if name == "feature" && target == "HEAD"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RenameBranch {
            repo_id: RepoId(1),
            old_name: "feature".to_string(),
            new_name: "renamed-feature".to_string(),
            force: false,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RenameBranch {
            repo_id: RepoId(1),
            old_name,
            new_name,
            force: false,
        }] if old_name == "feature" && new_name == "renamed-feature"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::DeleteBranch {
            repo_id: RepoId(1),
            name: "feature".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::DeleteBranch { repo_id: RepoId(1), name }] if name == "feature"
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ForceDeleteBranch {
            repo_id: RepoId(1),
            name: "feature".to_string(),
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::ForceDeleteBranch { repo_id: RepoId(1), name }] if name == "feature"
    ));
}

#[test]
fn checkout_commit_emits_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let commit_id = gitcomet_core::domain::CommitId("deadbeef".into());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutCommit {
            repo_id: RepoId(1),
            commit_id: commit_id.clone(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutCommit {
            repo_id: RepoId(1),
            commit_id: _
        }]
    ));

    let repo = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("repo should exist");
    assert_eq!(repo.detached_head_commit, Some(commit_id));
}

#[test]
fn commit_bumps_ops_rev() {
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
        Msg::Commit {
            repo_id,
            message: "test commit".to_string(),
            push_after_commit: false,
        },
    );
    assert!(
        state.repos[0].ops_rev > ops_before,
        "ops_rev should bump after Commit"
    );
}

#[test]
fn commit_finished_clears_commit_state_and_requests_primary_refreshes() {
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
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("README.md"),
        DiffArea::Unstaged,
    ));
    state.repos[0].diff_state.diff = Loadable::Loading;
    state.repos[0].diff_state.diff_file = Loadable::Loading;
    state.repos[0].diff_state.diff_file_image = Loadable::Loading;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    let expected_scope = state.repos[0].history_state.history_scope;

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert_eq!(state.repos[0].commit_in_flight, 0);
    assert_eq!(state.repos[0].diff_state.diff_target, None);
    assert!(matches!(
        state.repos[0].diff_state.diff,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        state.repos[0].diff_state.diff_file,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        state.repos[0].diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::References(ReferenceEffect::LoadHeadBranch { repo_id: id }) if *id == repo_id))
    );
    assert!(effects.iter().any(|e| matches!(
        e,
        Effect::LoadLog {
            repo_id: id,
            scope,
            ..
        } if *id == repo_id && *scope == expected_scope
    )));
}

#[test]
fn commit_finished_invalidates_loaded_blame() {
    // Regression: committing changes which lines are committed, so a stale blame
    // would mislabel them. Blame must be dropped along with the diff.
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
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    state.repos[0].diff_state.blame_path = Some(PathBuf::from("src/lib.rs"));
    state.repos[0].diff_state.blame_source = Some(gitcomet_core::domain::BlameSource::WorkingTree(
        DiffArea::Staged,
    ));
    state.repos[0].diff_state.blame = ready_working_tree_blame();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );

    assert!(
        matches!(state.repos[0].diff_state.blame, Loadable::NotLoaded),
        "blame must be invalidated after a commit so the annotation column reloads"
    );
}

#[test]
fn branch_collision_prompt_choices_emit_exact_follow_up_actions() {
    fn seeded_state(repo_id: RepoId) -> AppState {
        let mut state = AppState::test_default();
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(repo_id);
        state
    }

    let repo_id = RepoId(1);
    let prompt = crate::model::BranchExistsPromptState {
        repo_id,
        name: "feature".to_string(),
        target: "origin/feature-one".to_string(),
        operation: crate::model::BranchExistsPromptOperation::CreateBranch,
    };
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);

    let mut state = seeded_state(repo_id);
    state.branch_exists_prompt = Some(prompt.clone());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveBranchExistsPrompt {
            prompt: prompt.clone(),
            choice: crate::msg::BranchExistsChoice::Cancel,
        },
    );
    assert!(effects.is_empty());
    assert!(state.branch_exists_prompt.is_none());
    assert_eq!(state.repos[0].local_actions_in_flight, 0);

    let mut state = seeded_state(repo_id);
    state.branch_exists_prompt = Some(prompt.clone());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveBranchExistsPrompt {
            prompt: prompt.clone(),
            choice: crate::msg::BranchExistsChoice::CheckoutExisting,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutBranch { repo_id: id, name }]
            if *id == repo_id && name == "feature"
    ));
    assert!(state.branch_exists_prompt.is_none());
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    let remote_prompt = crate::model::BranchExistsPromptState {
        repo_id,
        name: "feature".to_string(),
        target: "upstream/feature-two".to_string(),
        operation: crate::model::BranchExistsPromptOperation::CheckoutRemoteBranch {
            remote: "upstream".to_string(),
            branch: "feature-two".to_string(),
        },
    };
    let mut state = seeded_state(repo_id);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ShowBranchExistsPrompt {
            prompt: remote_prompt.clone(),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.branch_exists_prompt, Some(remote_prompt.clone()));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveBranchExistsPrompt {
            prompt: remote_prompt,
            choice: crate::msg::BranchExistsChoice::OverwriteAndCheckout,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CheckoutRemoteBranch {
            repo_id: id,
            remote,
            branch,
            local_branch,
            mode: gitcomet_core::services::CheckoutRemoteBranchMode::Overwrite,
        }] if *id == repo_id
            && remote == "upstream"
            && branch == "feature-two"
            && local_branch == "feature"
    ));
    assert!(state.branch_exists_prompt.is_none());
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    let mut state = seeded_state(repo_id);
    state.branch_exists_prompt = Some(prompt.clone());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveBranchExistsPrompt {
            prompt: prompt.clone(),
            choice: crate::msg::BranchExistsChoice::OverwriteAndCheckout,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::CreateBranchAndCheckout {
            repo_id: id,
            name,
            target,
            force: true,
        }] if *id == repo_id
            && name == "feature"
            && target == "origin/feature-one"
    ));
    assert!(state.branch_exists_prompt.is_none());
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    let rename_prompt = crate::model::BranchExistsPromptState {
        repo_id,
        name: "feature".to_string(),
        target: "old".to_string(),
        operation: crate::model::BranchExistsPromptOperation::RenameBranch {
            old_name: "old".to_string(),
        },
    };
    let mut state = seeded_state(repo_id);
    state.branch_exists_prompt = Some(rename_prompt.clone());
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveBranchExistsPrompt {
            prompt: rename_prompt,
            choice: crate::msg::BranchExistsChoice::OverwriteAndCheckout,
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::RenameBranch {
            repo_id: id,
            old_name,
            new_name,
            force: true,
        }] if *id == repo_id && old_name == "old" && new_name == "feature"
    ));
    assert!(state.branch_exists_prompt.is_none());
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    let mut state = seeded_state(repo_id);
    state.branch_exists_prompt = Some(prompt.clone());
    let stale_prompt = crate::model::BranchExistsPromptState {
        name: "different".to_string(),
        ..prompt.clone()
    };
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveBranchExistsPrompt {
            prompt: stale_prompt,
            choice: crate::msg::BranchExistsChoice::OverwriteAndCheckout,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(state.branch_exists_prompt, Some(prompt));
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
}

#[test]
fn commit_and_amend_finished_cover_success_error_and_unknown_repo_paths() {
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
    {
        let repo = &mut state.repos[0];
        repo.local_actions_in_flight = 1;
        repo.commit_in_flight = 1;
        repo.diff_state.diff_target = Some(DiffTarget::working_tree(
            PathBuf::from("a.txt"),
            DiffArea::Unstaged,
        ));
        repo.diff_state.diff = Loadable::Loading;
        repo.diff_state.diff_file = Loadable::Loading;
        repo.diff_state.diff_file_image = Loadable::Loading;
    }

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert!(!effects.is_empty());
    let repo = &state.repos[0];
    assert_eq!(repo.local_actions_in_flight, 0);
    assert_eq!(repo.commit_in_flight, 0);
    assert!(repo.feedback.last_error.is_none());
    assert!(repo.diff_state.diff_target.is_none());
    assert!(matches!(repo.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(matches!(
        repo.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));
    assert_eq!(
        repo.feedback
            .command_log
            .last()
            .map(|entry| entry.summary.as_str()),
        Some("Commit: Completed")
    );

    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend("commit boom".to_string()))),
        }),
    );
    assert!(
        state.repos[0]
            .feedback
            .last_error
            .as_deref()
            .unwrap_or_default()
            .starts_with("Commit failed:")
    );

    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id,
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert_eq!(
        state.repos[0]
            .feedback
            .command_log
            .last()
            .map(|entry| entry.summary.as_str()),
        Some("Amend: Completed")
    );
    state.repos[0].local_actions_in_flight = 1;
    state.repos[0].commit_in_flight = 1;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id,
            result: Err(Error::new(ErrorKind::Backend("amend boom".to_string()))),
        }),
    );
    assert!(
        state.repos[0]
            .feedback
            .last_error
            .as_deref()
            .unwrap_or_default()
            .starts_with("Amend failed:")
    );

    let missing_commit = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitFinished {
            repo_id: RepoId(999),
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert!(missing_commit.is_empty());
    let missing_amend = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitAmendFinished {
            repo_id: RepoId(999),
            result: Ok(gitcomet_core::services::CommitOperationOutcome::default()),
        }),
    );
    assert!(missing_amend.is_empty());
}

#[test]
fn checkout_branch_clears_stale_force_push_lease_and_recent_messages() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state
        .repos
        .push(repo_with_head_dependent_cached_state(repo_id));
    let recent_rev = state.repos[0].recent_commit_messages_rev;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CheckoutBranch {
            repo_id,
            name: "feature".to_string(),
        },
    );

    assert!(effects.iter().any(|effect| {
        matches!(
            effect,
            Effect::CheckoutBranch { repo_id: id, name }
                if *id == repo_id && name == "feature"
        )
    }));
    assert_eq!(state.repos[0].pending.force_push_lease, None);
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert!(state.repos[0].recent_commit_messages_rev > recent_rev);
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
}

#[test]
fn revert_clears_recent_messages_from_previous_head() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state
        .repos
        .push(repo_with_head_dependent_cached_state(repo_id));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevertCommit {
            repo_id,
            commit_id: CommitId("3333333333333333333333333333333333333333".into()),
            commit: true,
            mainline: None,
            summary: "revert me".into(),
        },
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert_eq!(state.repos[0].pending.force_push_lease, None);
}

#[test]
fn revert_finished_releases_local_action_and_clears_stale_force_push_lease() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.local_actions_in_flight = 1;
    state.repos.push(repo_state);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Revert {
                commit_id: CommitId("3333333333333333333333333333333333333333".into()),
                commit: true,
                mainline: None,
                summary: "revert me".into(),
            },
            result: Ok(CommandOutput::empty_success("git revert 3333333")),
        }),
    );

    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert_eq!(state.repos[0].pending.force_push_lease, None);
}

#[test]
fn head_changing_repo_action_finish_invalidates_data_loaded_while_in_flight() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.local_actions_in_flight = 1;
    state.repos.push(repo_state);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
            repo_id,
            action: RepoActionKind::CherryPickCommit,
            result: Ok(()),
        }),
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert_eq!(state.repos[0].pending.force_push_lease, None);
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
}

#[test]
fn stale_recent_commit_messages_loaded_after_head_change_is_ignored() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let mut repo_state = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo_state.open = Loadable::Ready(());
    state.repos.push(repo_state);

    let first_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRecentCommitMessages { repo_id, limit: 10 },
    );
    let first_request_rev = match first_effects.as_slice() {
        [
            Effect::LoadRecentCommitMessages {
                repo_id: effect_repo_id,
                limit,
                request_rev,
            },
        ] if *effect_repo_id == repo_id && *limit == 10 => *request_rev,
        effects => panic!("expected recent commit message load effect, got {effects:?}"),
    };
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::Loading
    ));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));

    let second_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadRecentCommitMessages { repo_id, limit: 10 },
    );
    let second_request_rev = match second_effects.as_slice() {
        [
            Effect::LoadRecentCommitMessages {
                repo_id: effect_repo_id,
                limit,
                request_rev,
            },
        ] if *effect_repo_id == repo_id && *limit == 10 => *request_rev,
        effects => panic!("expected second recent commit message load effect, got {effects:?}"),
    };
    assert!(second_request_rev > first_request_rev);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev: first_request_rev,
            result: Ok(vec![test_recent_commit_message_with_summary(
                "2222222222222222222222222222222222222222",
                "stale message",
            )]),
        }),
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::Loading
    ));
    assert_eq!(
        state.repos[0].recent_commit_messages_rev,
        second_request_rev
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RecentCommitMessagesLoaded {
            repo_id,
            request_rev: second_request_rev,
            result: Ok(vec![test_recent_commit_message_with_summary(
                "3333333333333333333333333333333333333333",
                "current message",
            )]),
        }),
    );

    match &state.repos[0].recent_commit_messages {
        Loadable::Ready(messages) => {
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].summary.as_ref(), "current message");
        }
        other => panic!("expected current recent messages, got {other:?}"),
    }
}

#[test]
fn repo_command_finished_reset_clears_diff_state_and_unknown_repo_is_noop() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("a.txt"),
        DiffArea::Staged,
    ));
    repo_state.diff_state.diff = Loadable::Loading;
    repo_state.diff_state.diff_file = Loadable::Loading;
    repo_state.diff_state.diff_file_image = Loadable::Loading;
    state.repos.push(repo_state);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Reset {
                mode: gitcomet_core::services::ResetMode::Mixed,
                target: "HEAD~1".to_string(),
            },
            result: Ok(CommandOutput::empty_success("git reset --mixed HEAD~1")),
        }),
    );
    assert!(!effects.is_empty());
    let repo = &state.repos[0];
    assert!(repo.diff_state.diff_target.is_none());
    assert_eq!(repo.pending.force_push_lease, None);
    assert!(matches!(&repo.recent_commit_messages, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff, Loadable::NotLoaded));
    assert!(matches!(repo.diff_state.diff_file, Loadable::NotLoaded));
    assert!(matches!(
        repo.diff_state.diff_file_image,
        Loadable::NotLoaded
    ));

    let no_repo_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(999),
            command: RepoCommandKind::Reset {
                mode: gitcomet_core::services::ResetMode::Hard,
                target: "HEAD".to_string(),
            },
            result: Ok(CommandOutput::empty_success("git reset --hard HEAD")),
        }),
    );
    assert!(no_repo_effects.is_empty());
}
