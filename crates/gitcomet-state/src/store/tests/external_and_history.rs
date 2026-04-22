use super::*;

#[test]
fn external_worktree_change_refreshes_status_and_selected_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Unstaged,
            },
        },
    );

    // Complete the initial open-repo refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );

    assert!(
        has_worktree_status_effect(&effects, RepoId(1)),
        "expected status refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id, .. } if *repo_id == RepoId(1))),
        "expected diff refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(e, Effect::LoadDiffFile { repo_id, .. } if *repo_id == RepoId(1))
        }),
        "expected diff-file refresh"
    );
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::LoadLog { .. })),
        "did not expect history refresh on pure worktree changes"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { .. })),
        "did not expect head-branch refresh on pure worktree changes"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadUpstreamDivergence { .. })),
        "did not expect upstream divergence refresh on pure worktree changes"
    );
    assert!(
        !effects.iter().any(|e| matches!(
            e,
            Effect::LoadBranches { .. } | Effect::LoadRemoteBranches { .. }
        )),
        "did not expect branch refresh on pure worktree changes"
    );
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::LoadRebaseState { .. })),
        "did not expect rebase state refresh on pure worktree changes"
    );
}

#[test]
fn external_git_state_change_refreshes_history_and_selected_diff() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
            repo: Arc::new(DummyRepo::new("/tmp/repo")),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("a.txt"),
                area: DiffArea::Unstaged,
            },
        },
    );

    // Complete the initial open-repo refresh so the external-change refresh isn't coalesced away.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id: RepoId(1),
            result: Ok("main".to_string()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
            repo_id: RepoId(1),
            result: Ok(false),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    let history_scope = state.repos[0].history_state.history_scope;
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert!(
        effects
            .iter()
            .any(|e| { matches!(e, Effect::LoadLog { repo_id, .. } if *repo_id == RepoId(1)) }),
        "expected history refresh"
    );
    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { repo_id } if *repo_id == RepoId(1))),
        "expected head-branch refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(e, Effect::LoadUpstreamDivergence { repo_id } if *repo_id == RepoId(1))
        }),
        "expected upstream divergence refresh"
    );
    assert!(
        effects.iter().any(|e| matches!(
            e,
            Effect::LoadRebaseAndMergeState { repo_id } if *repo_id == RepoId(1)
        )),
        "expected rebase state refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadBranches { repo_id } if *repo_id == RepoId(1))),
        "expected local branches refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(e, Effect::LoadRemoteBranches { repo_id } if *repo_id == RepoId(1))
        }),
        "expected remote branches refresh"
    );
    assert!(
        effects.iter().any(|e| {
            matches!(
                e,
                Effect::LoadRebaseAndMergeState { repo_id } if *repo_id == RepoId(1)
            )
        }),
        "expected merge commit message refresh"
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::LoadDiff { repo_id, .. } if *repo_id == RepoId(1))),
        "expected diff refresh"
    );
}

#[test]
fn external_git_state_refresh_is_coalesced_and_replayed_once() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let effects1 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );

    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadHeadBranch { .. }))
    );
    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadUpstreamDivergence { .. }))
    );
    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadRebaseAndMergeState { .. }))
    );
    assert!(
        effects1
            .iter()
            .any(|e| matches!(e, Effect::LoadRebaseAndMergeState { .. }))
    );
    assert!(has_status_refresh_effects(&effects1, RepoId(1)));
    assert!(effects1.iter().any(|e| matches!(e, Effect::LoadLog { .. })));

    // Second refresh request while the first one is in flight is coalesced into a single pending
    // refresh per load kind (no immediate duplicate effects).
    let effects2 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::GitState,
        },
    );
    assert!(
        effects2.is_empty(),
        "expected coalescing/backpressure, got {effects2:?}"
    );

    // Completing each in-flight load replays exactly one more load for that kind.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id: RepoId(1),
            result: Ok("main".to_string()),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadHeadBranch { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::UpstreamDivergenceLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadUpstreamDivergence { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RebaseStateLoaded {
            repo_id: RepoId(1),
            result: Ok(false),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadRebaseState { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::MergeCommitMessageLoaded {
            repo_id: RepoId(1),
            result: Ok(None),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadMergeCommitMessage { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadWorktreeStatus { repo_id: RepoId(1) }]
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StagedStatusLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadStagedStatus { repo_id: RepoId(1) }]
    ));

    let history_scope = state.repos[0].history_state.history_scope;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }),
        }),
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadLog {
            repo_id: RepoId(1),
            scope,
            limit: 200,
            cursor: None
        }] if *scope == history_scope
    ));
}

#[test]
fn external_worktree_refresh_with_unchanged_status_settles_without_replay_loop() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].set_status(Loadable::Ready(Arc::new(RepoStatus::default())));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        has_worktree_status_effect(&effects, repo_id),
        "expected first worktree event to request status refresh"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        effects.is_empty(),
        "expected in-flight coalescing while status load is running, got {effects:?}"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::WorktreeStatusLoaded {
            repo_id,
            result: Ok(Vec::new()),
        }),
    );
    assert!(
        effects
            .iter()
            .all(|e| !matches!(e, Effect::LoadWorktreeStatus { repo_id: rid } if *rid == repo_id)),
        "unchanged status payload should not replay another status load, got {effects:?}"
    );
    assert!(
        !state.repos[0].loads_in_flight.any_in_flight(),
        "in-flight flags should settle after unchanged status load"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id,
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        has_worktree_status_effect(&effects, repo_id),
        "subsequent real worktree events should still trigger status refresh"
    );
}

#[test]
fn external_worktree_refresh_coalesces_status_while_status_is_in_flight() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SelectDiff {
            repo_id: RepoId(1),
            target: DiffTarget::WorkingTree {
                path: PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs"),
                area: DiffArea::Unstaged,
            },
        },
    );

    let effects1 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        has_worktree_status_effect(&effects1, RepoId(1)),
        "expected first refresh to request status"
    );
    assert!(
        effects1.iter().any(|e| matches!(
            e,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                ..
            }
        )),
        "expected first refresh to request diff reload"
    );

    let effects2 = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::Worktree,
        },
    );
    assert!(
        !has_worktree_status_effect(&effects2, RepoId(1)),
        "coalesced worktree refresh should not emit duplicate status effects, got {effects2:?}"
    );
    assert!(
        effects2.iter().any(|e| matches!(
            e,
            Effect::LoadDiff {
                repo_id: RepoId(1),
                ..
            }
        )),
        "selected diff should still refresh on subsequent worktree changes"
    );
    assert!(
        effects2.iter().any(|e| matches!(
            e,
            Effect::LoadDiffFile {
                repo_id: RepoId(1),
                ..
            }
        )),
        "selected diff file should still refresh on subsequent worktree changes"
    );
}

#[test]
fn reload_repo_sets_sections_loading_and_emits_refresh_effects() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
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
        Msg::ReloadRepo { repo_id: RepoId(1) },
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.head_branch.is_loading());
    assert!(repo_state.branches.is_loading());
    assert!(repo_state.tags.is_loading());
    assert!(matches!(repo_state.remote_tags, Loadable::NotLoaded));
    assert!(repo_state.remotes.is_loading());
    assert!(repo_state.remote_branches.is_loading());
    assert!(repo_state.status.is_loading());
    assert!(repo_state.worktree_status_is_loading());
    assert!(repo_state.staged_status_is_loading());
    assert!(repo_state.log.is_loading());
    assert!(!repo_state.history_state.log_loading_more);
    assert!(repo_state.merge_commit_message.is_loading());
    assert!(has_status_refresh_effects(&effects, RepoId(1)));
    assert!(
        !effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadRemoteTags { repo_id } if *repo_id == RepoId(1)
        )),
        "remote tags should lazy-load from tag UI, not repo reload"
    );
}

#[test]
fn load_more_history_emits_paginated_load_log_effect() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    }));
    repo_state.history_state.log_loading_more = false;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadMoreHistory { repo_id: RepoId(1) },
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.history_state.log_loading_more);
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadLog {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            limit: 200,
            cursor: Some(_)
        }]
    ));
}

#[test]
fn set_history_scope_emits_load_log_effect_for_every_history_mode() {
    for target_scope in [
        LogScope::FullReachable,
        LogScope::FirstParent,
        LogScope::NoMerges,
        LogScope::MergesOnly,
        LogScope::AllBranches,
    ] {
        let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
        let id_alloc = AtomicU64::new(1);
        let mut state = AppState::default();
        state.repos.push(RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(RepoId(1));

        let repo_state = &mut state.repos[0];
        repo_state.history_state.history_scope = if target_scope == LogScope::FullReachable {
            LogScope::FirstParent
        } else {
            LogScope::FullReachable
        };
        repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
            commits: vec![Commit {
                id: CommitId("old".into()),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "old".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        })));

        let effects = reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::SetHistoryScope {
                repo_id: RepoId(1),
                scope: target_scope,
            },
        );

        let repo_state = &state.repos[0];
        assert_eq!(repo_state.history_state.history_scope, target_scope);
        assert!(repo_state.log.is_loading());
        assert!(
            repo_state
                .history_state
                .retained_log_while_loading
                .is_some(),
            "expected retained history page while switching to {target_scope:?}"
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::LoadLog {
                    repo_id: RepoId(1),
                    scope,
                    cursor: None,
                    ..
                } if *scope == target_scope
            )),
            "expected LoadLog({target_scope:?}) effect, got {effects:?}"
        );
    }
}

#[test]
fn set_history_scope_retains_ready_log_while_loading() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let retained_page = Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    });
    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::clone(&retained_page)));

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.log.is_loading());
    let retained = repo_state
        .history_state
        .retained_log_while_loading
        .as_ref()
        .expect("scope switch should retain the previous ready log while loading");
    assert!(Arc::ptr_eq(retained, &retained_page));
}

#[test]
fn stale_log_loaded_result_replays_latest_pending_scope_switch() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::FullReachable;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("old".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "old".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    })));
    assert!(
        repo_state
            .loads_in_flight
            .request_log(LogScope::FullReachable, 200, None)
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );
    assert!(effects.is_empty());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::NoMerges,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::NoMerges
    );
    assert!(state.repos[0].log.is_loading());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::FullReachable,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![],
                next_cursor: None,
            }),
        }),
    );

    assert!(state.repos[0].log.is_loading());
    assert!(!state.repos[0].history_state.log_loading_more);
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::LoadLog {
                repo_id: RepoId(1),
                scope: LogScope::NoMerges,
                limit: 200,
                cursor: None,
            }]
        ),
        "expected stale result to replay the latest pending scope switch, got {effects:?}"
    );
}

#[test]
fn load_more_history_noops_when_no_next_cursor() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let repo_state = &mut state.repos[0];
    repo_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    }));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadMoreHistory { repo_id: RepoId(1) },
    );

    let repo_state = &state.repos[0];
    assert!(!repo_state.history_state.log_loading_more);
    assert!(effects.is_empty());
}

#[test]
fn log_loaded_appends_when_loading_more() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.log = Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    }));
    repo_state.history_state.log_loading_more = true;
    let log_before = (repo_state.log_rev, repo_state.history_state.log_rev);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("c2".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s2".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(!repo_state.history_state.log_loading_more);
    assert!(repo_state.log_rev > log_before.0);
    assert!(repo_state.history_state.log_rev > log_before.1);
    let Loadable::Ready(page) = &repo_state.log else {
        panic!("expected log ready");
    };
    assert_eq!(page.commits.len(), 2);
    assert_eq!(page.commits[0].id.as_ref(), "c1");
    assert_eq!(page.commits[1].id.as_ref(), "c2");
    assert_eq!(page.next_cursor, None);
}

#[test]
fn log_loaded_appends_when_loading_more_re_shares_history_log_arc() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("c1".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "s1".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    })));
    repo_state.history_state.log_loading_more = true;

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("c2".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s2".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: Some(LogCursor {
                    last_seen: CommitId("c2".into()),
                    resume_from: None,
                    resume_token: None,
                }),
            }),
        }),
    );

    let repo_state = &state.repos[0];
    let Loadable::Ready(repo_log) = &repo_state.log else {
        panic!("expected repo log ready");
    };
    let Loadable::Ready(history_log) = &repo_state.history_state.log else {
        panic!("expected history log ready");
    };

    assert!(Arc::ptr_eq(repo_log, history_log));
    assert_eq!(repo_log.commits.len(), 2);
    assert_eq!(repo_log.commits[1].id.as_ref(), "c2");
    assert_eq!(
        repo_log
            .next_cursor
            .as_ref()
            .and_then(|cursor| cursor.last_seen.as_ref().strip_prefix('c')),
        Some("2")
    );
}

#[test]
fn log_loaded_clears_retained_scope_switch_log() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![Commit {
            id: CommitId("old".into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: "old".into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        }],
        next_cursor: None,
    })));

    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        },
    );

    assert!(
        state.repos[0]
            .history_state
            .retained_log_while_loading
            .is_some()
    );

    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("new".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "new".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.log, Loadable::Ready(_)));
    assert!(
        repo_state
            .history_state
            .retained_log_while_loading
            .is_none()
    );
}

#[test]
fn log_loaded_initial_paginated_page_keeps_append_slack() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));

    let commits: Vec<Commit> = (0..600)
        .map(|ix| Commit {
            id: CommitId(format!("{ix:040x}").into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: format!("s{ix}").into(),
            author: "a".into(),
            time: SystemTime::UNIX_EPOCH,
        })
        .collect();
    let last_seen = commits.last().expect("last commit").id.clone();
    let history_scope = state.repos[0].history_state.history_scope;

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits,
                next_cursor: Some(LogCursor {
                    last_seen,
                    resume_from: None,
                    resume_token: None,
                }),
            }),
        }),
    );

    let Loadable::Ready(page) = &state.repos[0].log else {
        panic!("expected log ready");
    };
    assert!(page.commits.capacity() >= page.commits.len() + 512);
}

// --- Revision counter regression tests ---

#[test]
fn log_loaded_bumps_log_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let log_before = (state.repos[0].log_rev, state.repos[0].history_state.log_rev);
    let history_scope = state.repos[0].history_state.history_scope;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            scope: history_scope,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![Commit {
                    id: CommitId("c1".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s1".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }),
        }),
    );

    assert!(
        state.repos[0].log_rev > log_before.0,
        "repo log_rev should bump after LogLoaded"
    );
    assert!(
        state.repos[0].history_state.log_rev > log_before.1,
        "log_rev should bump after LogLoaded"
    );
}

#[test]
fn detached_head_target_tracks_current_branch_log_head() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);
    state.repos[0].history_state.history_scope = LogScope::CurrentBranch;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
            repo_id,
            result: Ok("HEAD".to_string()),
        }),
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(LogPage {
                commits: vec![
                    Commit {
                        id: CommitId("c1".into()),
                        parent_ids: smallvec::smallvec![CommitId("c0".into())],
                        summary: "s1".into(),
                        author: "a".into(),
                        time: SystemTime::UNIX_EPOCH,
                    },
                    Commit {
                        id: CommitId("c0".into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "s0".into(),
                        author: "a".into(),
                        time: SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }),
        }),
    );

    assert_eq!(
        state.repos[0].detached_head_commit,
        Some(CommitId("c1".into()))
    );
}

#[test]
fn filtered_current_branch_logs_do_not_backfill_detached_head_target() {
    for (scope, commits, expected_first_visible) in [
        (
            LogScope::NoMerges,
            vec![Commit {
                id: CommitId("visible-non-merge".into()),
                parent_ids: smallvec::smallvec![CommitId("hidden-head".into())],
                summary: "visible".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            CommitId("visible-non-merge".into()),
        ),
        (
            LogScope::MergesOnly,
            vec![Commit {
                id: CommitId("visible-merge".into()),
                parent_ids: smallvec::smallvec![CommitId("p0".into()), CommitId("p1".into())],
                summary: "merge".into(),
                author: "a".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            CommitId("visible-merge".into()),
        ),
    ] {
        let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
        let id_alloc = AtomicU64::new(2);
        let mut state = AppState::default();
        let repo_id = RepoId(1);
        repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
        state.repos.push(RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        ));
        state.active_repo = Some(repo_id);
        state.repos[0].history_state.history_scope = scope;

        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::HeadBranchLoaded {
                repo_id,
                result: Ok("HEAD".to_string()),
            }),
        );

        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                repo_id,
                scope,
                cursor: None,
                result: Ok(LogPage {
                    commits,
                    next_cursor: None,
                }),
            }),
        );

        assert!(
            state.repos[0].detached_head_commit.is_none(),
            "{scope:?} should not infer detached HEAD from first visible commit {expected_first_visible}"
        );
    }
}

#[test]
fn set_history_scope_bumps_log_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let log_before = (state.repos[0].log_rev, state.repos[0].history_state.log_rev);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id,
            scope: LogScope::AllBranches,
        },
    );

    assert!(
        state.repos[0].log_rev > log_before.0,
        "repo log_rev should bump after SetHistoryScope"
    );
    assert!(
        state.repos[0].history_state.log_rev > log_before.1,
        "log_rev should bump after SetHistoryScope"
    );
}

#[test]
fn status_loaded_bumps_status_rev() {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::default();
    let repo_id = RepoId(1);
    repos.insert(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let status_before = state.repos[0].status_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::StatusLoaded {
            repo_id,
            result: Ok(RepoStatus::default()),
        }),
    );

    assert!(
        state.repos[0].status_rev > status_before,
        "status_rev should bump after StatusLoaded"
    );
}
