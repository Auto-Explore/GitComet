use super::*;

#[test]
fn merge_ref_emits_effect() {
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
        Msg::MergeRef {
            repo_id: RepoId(1),
            reference: "feature".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::MergeRef { repo_id: RepoId(1), reference })] if reference == "feature"
    ));
}

#[test]
fn squash_ref_emits_effect() {
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
        Msg::SquashRef {
            repo_id: RepoId(1),
            reference: "feature".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::SquashRef { repo_id: RepoId(1), reference })] if reference == "feature"
    ));
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
}

#[test]
fn rebase_emits_effect() {
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
        Msg::Rebase {
            repo_id: RepoId(1),
            onto: "master".to_string(),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::Rebase { repo_id: RepoId(1), onto })] if onto == "master"
    ));
}

#[test]
fn cherry_pick_clears_recent_messages_from_previous_head() {
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
        Msg::CherryPickCommit {
            repo_id,
            commit_id: CommitId("3333333333333333333333333333333333333333".into()),
            commit: true,
            mainline: None,
            summary: "pick me".into(),
        },
    );

    assert!(matches!(
        &state.repos[0].recent_commit_messages,
        Loadable::NotLoaded
    ));
    assert_eq!(state.repos[0].pending.force_push_lease, None);
}

#[test]
fn continue_and_abort_wait_only_for_sequencer_commands() {
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
    state.active_repo = Some(repo_id);
    let commit_id = CommitId("3333333333333333333333333333333333333333".into());
    // A revert's commit step still running while REVERT_HEAD is on disk.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevertCommit {
            repo_id,
            commit_id: commit_id.clone(),
            commit: true,
            mainline: None,
            summary: "revert me".into(),
        },
    );
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 1);

    for msg in [
        Msg::RebaseAbort { repo_id },
        Msg::RebaseContinue { repo_id },
        Msg::MergeAbort { repo_id },
    ] {
        let effects = reduce(&mut repos, &id_alloc, &mut state, msg);
        assert!(effects.is_empty(), "{effects:?}");
    }
    assert!(
        state
            .notifications
            .iter()
            .any(|notification| notification.message.contains("Wait for the running Git")),
        "a blocked Continue/Abort should say why"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::Revert {
                commit_id,
                commit: true,
                mainline: None,
                summary: "revert me".into(),
            },
            result: Ok(CommandOutput::empty_success("git revert 3333333")),
        }),
    );
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 0);

    // A merge tool blocks for as long as the user keeps it open, but touches
    // no sequencer state, so it must not lock Continue/Abort out.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LaunchMergetool {
            repo_id,
            path: PathBuf::from("conflicted.txt"),
        },
    );
    assert!(state.repos[0].local_actions_in_flight > 0);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RebaseAbort { repo_id },
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RepoCommand(RepoCommandEffect::RebaseAbort { .. })]
        ),
        "a running merge tool must not block Abort: {effects:?}"
    );
}

#[test]
fn a_suggested_commit_message_is_stored_for_the_commit_box() {
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
    let before = state.repos[0].suggested_commit_message_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitMessageSuggested {
            repo_id,
            message: "Revert \"change\"".to_string(),
        }),
    );

    assert_eq!(
        state.repos[0].suggested_commit_message.as_deref(),
        Some("Revert \"change\"")
    );
    assert_ne!(state.repos[0].suggested_commit_message_rev, before);

    for (consumed, expected) in [
        ("some older message", Some("Revert \"change\"")),
        ("Revert \"change\"", None),
    ] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::CommitMessageSuggestionConsumed {
                repo_id,
                message: consumed.to_owned(),
            }),
        );
        assert_eq!(state.repos[0].suggested_commit_message.as_deref(), expected);
    }
}

#[test]
fn the_mainline_lookup_does_not_disturb_the_reveal_dialogs() {
    use crate::model::CommitLookupPurpose;

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
    let typed = CommitId("aaaaaaaa".into());
    let merge = CommitId("bbbbbbbb".into());

    // The Go-to dialog is resolving what the user typed.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveCommitLookup {
            repo_id,
            reference: typed.clone(),
            purpose: CommitLookupPurpose::RevealDialog,
        },
    );
    // Opening a revert/cherry-pick confirmation asks for a merge's parents.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveCommitLookup {
            repo_id,
            reference: merge.clone(),
            purpose: CommitLookupPurpose::MainlineParents,
        },
    );
    let Some(Effect::ResolveCommitLookup { request, .. }) = effects.first().cloned() else {
        panic!("expected a lookup effect: {effects:?}");
    };
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitLookupResolved {
            repo_id,
            reference: merge.clone(),
            request,
            purpose: CommitLookupPurpose::MainlineParents,
            result: Err(Error::new(ErrorKind::Backend("nope".to_string()))),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.commit_lookup.reference.as_ref(), Some(&typed));
    assert!(
        history.commit_lookup.result.is_loading(),
        "the Go-to dialog's own lookup must still be in flight"
    );
    assert_eq!(history.mainline_lookup.reference.as_ref(), Some(&merge));
    assert!(matches!(history.mainline_lookup.result, Loadable::Error(_)));
}

#[test]
fn sequencer_commands_release_their_in_flight_count() {
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
    state.active_repo = Some(repo_id);
    let commit_id = CommitId("3333333333333333333333333333333333333333".into());

    // Every counted effect must be released by the command it schedules.
    let cases: Vec<(Msg, RepoCommandKind)> = vec![
        (
            Msg::RevertCommit {
                repo_id,
                commit_id: commit_id.clone(),
                commit: true,
                mainline: None,
                summary: "revert me".into(),
            },
            RepoCommandKind::Revert {
                commit_id: commit_id.clone(),
                commit: true,
                mainline: None,
                summary: "revert me".into(),
            },
        ),
        (
            Msg::CherryPickCommit {
                repo_id,
                commit_id: commit_id.clone(),
                commit: true,
                mainline: None,
                summary: "pick me".into(),
            },
            RepoCommandKind::CherryPick {
                commit_id: commit_id.clone(),
                commit: true,
                mainline: None,
                summary: "pick me".into(),
            },
        ),
        (
            Msg::ApplyFileChange {
                commit_retry: None,
                repo_id,
                target: gitcomet_core::domain::ApplyChangeTarget::commit(
                    commit_id.clone(),
                    PathBuf::from("a.txt"),
                ),
                commit: true,
            },
            RepoCommandKind::ApplyFileChange {
                commit_retry: None,
                target: gitcomet_core::domain::ApplyChangeTarget::commit(
                    commit_id.clone(),
                    PathBuf::from("a.txt"),
                ),
                commit: true,
            },
        ),
        (
            Msg::RebaseContinue { repo_id },
            RepoCommandKind::RebaseContinue,
        ),
        (Msg::RebaseAbort { repo_id }, RepoCommandKind::RebaseAbort),
        (Msg::MergeAbort { repo_id }, RepoCommandKind::MergeAbort),
        (
            Msg::Rebase {
                repo_id,
                onto: "main".into(),
            },
            RepoCommandKind::Rebase {
                onto: "main".into(),
            },
        ),
        (
            Msg::Reset {
                repo_id,
                target: "HEAD~1".into(),
                mode: gitcomet_core::services::ResetMode::Mixed,
            },
            RepoCommandKind::Reset {
                mode: gitcomet_core::services::ResetMode::Mixed,
                target: "HEAD~1".into(),
            },
        ),
        (
            Msg::MergeRef {
                repo_id,
                reference: "topic".into(),
            },
            RepoCommandKind::MergeRef {
                reference: "topic".into(),
            },
        ),
    ];

    for (msg, command) in cases {
        let label = format!("{command:?}");
        reduce(&mut repos, &id_alloc, &mut state, msg);
        assert_eq!(
            state.repos[0].sequencer_actions_in_flight, 1,
            "{label} should count while it runs"
        );
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result: Ok(CommandOutput::empty_success("git")),
            }),
        );
        assert_eq!(
            state.repos[0].sequencer_actions_in_flight, 0,
            "{label} should release its count"
        );
    }
}

#[test]
fn interactive_cherry_pick_carries_the_commit_choice_to_its_effect() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    state
        .repos
        .push(repo_with_head_dependent_cached_state(repo_id));
    let entries = vec![gitcomet_core::services::InteractiveRebaseEntry {
        action: gitcomet_core::services::InteractiveRebaseAction::Pick,
        commit_id: "3333333333333333333333333333333333333333".to_string(),
        summary: "pick me".to_string(),
        message: "pick me".to_string(),
        new_message: None,
    }];

    for commit in [true, false] {
        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::InteractiveCherryPick {
                repo_id,
                entries: entries.clone(),
                commit,
            },
        );
        assert!(
            matches!(
                effects.as_slice(),
                [Effect::RepoCommand(RepoCommandEffect::InteractiveCherryPick {
                    repo_id: RepoId(1),
                    entries: sent,
                    commit: sent_commit,
                })] if sent == &entries && *sent_commit == commit
            ),
            "commit={commit}: {effects:?}"
        );
    }
}

#[test]
fn interactive_cherry_pick_finished_clears_stale_force_push_lease() {
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
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::InteractiveCherryPick {
                entries: vec![gitcomet_core::services::InteractiveRebaseEntry {
                    action: gitcomet_core::services::InteractiveRebaseAction::Pick,
                    commit_id: "3333333333333333333333333333333333333333".to_string(),
                    summary: "pick me".to_string(),
                    message: "pick me".to_string(),
                    new_message: None,
                }],
                commit: true,
            },
            result: Ok(CommandOutput::empty_success("git cherry-pick")),
        }),
    );

    assert_eq!(state.repos[0].pending.force_push_lease, None);
}

#[test]
fn cherry_pick_setup_loads_full_messages_and_patches_entries() {
    const SHA: &str = "1111111111111111111111111111111111111111";
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));

    // Opening the setup with subject-only seeds schedules the full-message
    // load for the selected commits.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![gitcomet_core::services::InteractiveRebaseEntry {
                action: gitcomet_core::services::InteractiveRebaseAction::Pick,
                commit_id: SHA.to_string(),
                summary: "subject".to_string(),
                message: "subject".to_string(),
                new_message: None,
            }],
            source_colors: vec![],
        },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadInteractiveCherryPickMessages { repo_id: RepoId(1), ids }]
            if ids.as_slice() == [SHA.to_string()]
    ));
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup opens in a loading state");
    assert!(matches!(setup.full_messages, Loadable::Loading));

    // Only after every full message arrives does the setup become editable;
    // the full body replaces the subject-only seed while the summary stays.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![SHA.to_string()],
                result: Ok(vec![(SHA.to_string(), "subject\n\nfull body".to_string())]),
            },
        ),
    );
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup stays open");
    assert_eq!(setup.entries[0].message, "subject\n\nfull body");
    assert_eq!(setup.entries[0].summary, "subject");
    assert!(matches!(setup.full_messages, Loadable::Ready(())));
}

#[test]
fn cherry_pick_setup_applies_repository_topological_order() {
    const DESCENDANT: &str = "3333333333333333333333333333333333333333";
    const ANCESTOR: &str = "1111111111111111111111111111111111111111";
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let entry = |commit_id: &str, summary: &str| gitcomet_core::services::InteractiveRebaseEntry {
        action: gitcomet_core::services::InteractiveRebaseAction::Pick,
        commit_id: commit_id.to_string(),
        summary: summary.to_string(),
        message: summary.to_string(),
        new_message: None,
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![entry(DESCENDANT, "descendant"), entry(ANCESTOR, "ancestor")],
            source_colors: vec![],
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![DESCENDANT.to_string(), ANCESTOR.to_string()],
                result: Ok(vec![
                    (ANCESTOR.to_string(), "ancestor full".to_string()),
                    (DESCENDANT.to_string(), "descendant full".to_string()),
                ]),
            },
        ),
    );

    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup stays open");
    assert_eq!(
        setup
            .entries
            .iter()
            .map(|entry| entry.commit_id.as_str())
            .collect::<Vec<_>>(),
        [ANCESTOR, DESCENDANT]
    );
    assert_eq!(setup.entries[0].message, "ancestor full");
    assert_eq!(setup.entries[1].message, "descendant full");
    assert!(matches!(setup.full_messages, Loadable::Ready(())));
}

#[test]
fn cherry_pick_setup_never_enables_rewording_after_partial_or_stale_message_load() {
    const OLD_SHA: &str = "1111111111111111111111111111111111111111";
    const NEW_SHA: &str = "2222222222222222222222222222222222222222";
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let entry = |commit_id: &str| gitcomet_core::services::InteractiveRebaseEntry {
        action: gitcomet_core::services::InteractiveRebaseAction::Pick,
        commit_id: commit_id.to_string(),
        summary: "subject".to_string(),
        message: "subject".to_string(),
        new_message: None,
    };

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![entry(OLD_SHA)],
            source_colors: vec![],
        },
    );
    // Replace the selection before the detached response for the first one
    // arrives. That stale body must not unlock or alter the new setup.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenInteractiveCherryPickSetup {
            repo_id: RepoId(1),
            entries: vec![entry(NEW_SHA)],
            source_colors: vec![],
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![OLD_SHA.to_string()],
                result: Ok(vec![(OLD_SHA.to_string(), "old full body".to_string())]),
            },
        ),
    );
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("new setup stays open");
    assert_eq!(setup.entries[0].commit_id, NEW_SHA);
    assert_eq!(setup.entries[0].message, "subject");
    assert!(matches!(setup.full_messages, Loadable::Loading));

    // A response missing even one requested full message becomes an error,
    // never a subject-only Ready state.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(
            crate::msg::InternalMsg::InteractiveCherryPickMessagesLoaded {
                repo_id: RepoId(1),
                requested_ids: vec![NEW_SHA.to_string()],
                result: Ok(vec![]),
            },
        ),
    );
    let setup = state.repos[0]
        .interactive_cherry_pick_setup
        .as_ref()
        .expect("setup stays open with its load error");
    assert!(matches!(setup.full_messages, Loadable::Error(_)));
    assert_eq!(setup.entries[0].message, "subject");
}

#[test]
fn reword_commit_emits_effect_and_counts_as_a_sequencer_command() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let target = CommitId("4444444444444444444444444444444444444444".into());
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let request_id = crate::model::RewordRequestId::new();
    let reword = |message: &str| Msg::RewordCommit {
        repo_id,
        request_id,
        commit_id: target.clone(),
        message: message.into(),
    };
    let effects = reduce(&mut repos, &id_alloc, &mut state, reword("New message"));
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::RepoCommand(RepoCommandEffect::RewordCommit {
                repo_id: RepoId(1),
                commit_id,
                message,
                resume: false,
                auth: None,
                request_id: actual_request,
            })] if *commit_id == target && message == "New message" && *actual_request == request_id
        ),
        "{effects:?}"
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 1);

    // A second submit while the first runs is refused.
    let effects = reduce(&mut repos, &id_alloc, &mut state, reword("New message"));
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(state.repos[0].local_actions_in_flight, 1);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command: RepoCommandKind::RewordCommit {
                request_id,
                commit_id: target.clone(),
                message: "New message".into(),
                rebase_paused: false,
                rewritten_id: None,
            },
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("reword: hook failed".into()),
            )),
        }),
    );
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 0);
    let last = state.repos[0].feedback.command_log.last().unwrap();
    assert!(!last.ok, "the failure must be recorded as such");
    assert!(
        last.summary.starts_with("Edit commit message failed"),
        "{}",
        last.summary
    );

    // A blank message never reaches the backend.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RewordCommit {
            repo_id,
            request_id: crate::model::RewordRequestId::new(),
            commit_id: target,
            message: "  \n".into(),
        },
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
}

#[test]
fn reword_commit_waits_for_a_running_history_operation() {
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
    repo_state.rebase_in_progress = Loadable::Ready(true);
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RewordCommit {
            repo_id,
            request_id: crate::model::RewordRequestId::new(),
            commit_id: CommitId("4444444444444444444444444444444444444444".into()),
            message: "New message".into(),
        },
    );
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(state.repos[0].local_actions_in_flight, 0);
    assert_eq!(state.repos[0].sequencer_actions_in_flight, 0);
}

fn finish_reword(
    repos: &mut RepositoryRuntimes,
    id_alloc: &AtomicU64,
    state: &mut AppState,
    commit_id: &CommitId,
    rewritten_id: Option<CommitId>,
) -> Vec<Effect> {
    let request_id = crate::model::RewordRequestId::new();
    let started = reduce(
        repos,
        id_alloc,
        state,
        Msg::RewordCommit {
            repo_id: RepoId(1),
            request_id,
            commit_id: commit_id.clone(),
            message: "New message".into(),
        },
    );
    assert!(matches!(
        started.as_slice(),
        [Effect::RepoCommand(RepoCommandEffect::RewordCommit { .. })]
    ));
    reduce(
        repos,
        id_alloc,
        state,
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id: RepoId(1),
            command: RepoCommandKind::RewordCommit {
                request_id,
                commit_id: commit_id.clone(),
                message: "New message".into(),
                rebase_paused: false,
                rewritten_id,
            },
            result: Ok(CommandOutput::empty_success("git commit --amend --only")),
        }),
    )
}

#[test]
fn reword_commit_finished_follows_the_rewritten_commit() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let old = CommitId("3333333333333333333333333333333333333333".into());
    let rewritten = CommitId("5555555555555555555555555555555555555555".into());
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.set_selected_commit(Some(old.clone()));
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);

    let effects = finish_reword(
        &mut repos,
        &id_alloc,
        &mut state,
        &old,
        Some(rewritten.clone()),
    );

    let repo_state = &state.repos[0];
    assert_eq!(
        repo_state.history_state.selected_commit.as_ref(),
        Some(&rewritten)
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadCommitDetails { repo_id: RepoId(1), commit_id } if *commit_id == rewritten
        )),
        "the rewritten commit's details must load: {effects:?}"
    );
    assert_eq!(repo_state.pending.force_push_lease, None);
    let last = repo_state.feedback.command_log.last().unwrap();
    assert!(last.ok);
    assert_eq!(last.summary, "Edit commit message: Completed");
}

#[test]
fn reword_commit_finished_leaves_another_selection_alone() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let reworded = CommitId("3333333333333333333333333333333333333333".into());
    let other = CommitId("6666666666666666666666666666666666666666".into());
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.set_selected_commit(Some(other.clone()));
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);
    let details_rev = state.repos[0].history_state.commit_details_rev;

    let effects = finish_reword(
        &mut repos,
        &id_alloc,
        &mut state,
        &reworded,
        Some(CommitId("5555555555555555555555555555555555555555".into())),
    );

    let repo_state = &state.repos[0];
    assert_eq!(
        repo_state.history_state.selected_commit.as_ref(),
        Some(&other)
    );
    assert_eq!(repo_state.history_state.commit_details_rev, details_rev);
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadCommitDetails { .. })),
        "{effects:?}"
    );
}

#[test]
fn reword_commit_finished_without_a_new_id_clears_the_reworded_selection() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    let old = CommitId("3333333333333333333333333333333333333333".into());
    let mut repo_state = repo_with_head_dependent_cached_state(repo_id);
    repo_state.set_selected_commit(Some(old.clone()));
    state.repos.push(repo_state);
    state.active_repo = Some(repo_id);

    let effects = finish_reword(&mut repos, &id_alloc, &mut state, &old, None);

    let repo_state = &state.repos[0];
    assert_eq!(repo_state.history_state.selected_commit, None);
    assert!(matches!(
        repo_state.history_state.commit_details,
        Loadable::NotLoaded
    ));
    assert!(
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadCommitDetails { .. })),
        "{effects:?}"
    );
    assert!(repo_state.feedback.command_log.last().unwrap().ok);
}

#[test]
fn reword_commit_retry_continues_a_paused_rebase_or_replays() {
    let commit_id = CommitId("3333333333333333333333333333333333333333".into());
    let request_id = crate::model::RewordRequestId::new();
    let kind = |rebase_paused| RepoCommandKind::RewordCommit {
        request_id,
        commit_id: commit_id.clone(),
        message: "New message".into(),
        rebase_paused,
        rewritten_id: None,
    };

    let replay = crate::store::reducer::repo_command_replay_msg_for_test(RepoId(3), kind(false));
    assert!(
        matches!(
            &replay,
            Some(Msg::RewordCommit { repo_id: RepoId(3), request_id: actual, commit_id: id, message })
                if *id == commit_id && message == "New message" && *actual == request_id
        ),
        "{replay:?}"
    );
    let replay = crate::store::reducer::repo_command_replay_msg_for_test(RepoId(3), kind(true));
    assert!(
        matches!(&replay, Some(Msg::ContinueRewordCommit { repo_id: RepoId(3), request_id: actual }) if *actual == request_id),
        "{replay:?}"
    );
}
