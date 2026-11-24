use super::*;

#[test]
fn load_more_history_emits_paginated_load_log_effect() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.history_state.log = Loadable::Ready(Arc::new(LogPage {
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
            walk,
            author: None,
            limit: 200,
            cursor: Some(_),
            ..
        }] if walk.mode() == LogScope::CurrentBranch
    ));
}

#[test]
fn refresh_preserves_selected_root_after_new_commit() {
    let old = commits_named("old", 600);
    let mut state = paginated_repo_state(&old);
    state.repos[0].set_log(Loadable::Ready(Arc::new(LogPage {
        commits: old.clone(),
        next_cursor: None,
    })));
    let selected = old.last().unwrap().id.clone();
    state.repos[0].set_selected_commit(Some(selected.clone()));
    state.repos[0].set_commit_multi_selection(crate::model::CommitMultiSelection {
        commits: vec![selected.clone()].into(),
        anchor: Some(selected.clone()),
        ..Default::default()
    });
    let history: Vec<_> = commits_named("new", 1).into_iter().chain(old).collect();
    let effects = refresh_history(&mut state);
    answer_log(&mut state, &effects, &history);
    assert_eq!(
        state.repos[0].history_state.selected_commit.as_ref(),
        Some(&selected)
    );
}

#[test]
fn refresh_does_not_ratchet_on_repeated_refreshes() {
    let history = commits_named("c", 2000);
    let mut state = paginated_repo_state(&history[..600]);
    for _ in 0..3 {
        let effects = refresh_history(&mut state);
        assert_eq!(log_effect(&effects).1, 600);
        answer_log(&mut state, &effects, &history);
    }
}

#[test]
fn refresh_depth_preserves_direct_and_promoted_loads() {
    for count in [4_999, 5_000, 5_001, 10_000, 50_000] {
        for queued in [false, true] {
            let history = commits_named("c", count + 200);
            let mut state = paginated_repo_state(&history[..count]);
            state.repos[0].history_state.history_author_filter = Some("alice".into());
            let pending = queued
                .then(|| history_message(&mut state, Msg::LoadMoreHistory { repo_id: RepoId(1) }));
            let mut effects = refresh_history(&mut state);
            if let Some(pending) = pending {
                assert!(!effects.iter().any(|e| matches!(e, Effect::LoadLog { .. })));
                effects = answer_log(&mut state, &pending, &history);
            }
            let retained = count + if queued { 200 } else { 0 };
            assert_eq!(log_effect(&effects).1, retained);
            assert!(effects.iter().any(
                |e| matches!(e, Effect::LoadLog { author: Some(author), .. } if author == "alice")
            ));
            answer_log(&mut state, &effects, &history);
            assert!(
                matches!(&state.repos[0].history_state.log, Loadable::Ready(page) if page.commits == history[..retained])
            );
        }
    }
}

#[test]
fn full_refresh_of_ready_log_keeps_depth() {
    let mut state = paginated_repo_state(&commits_named("c", 600));
    state.active_repo = None;
    let effects = history_message(&mut state, Msg::SetActiveRepo { repo_id: RepoId(1) });
    assert_eq!(log_effect(&effects).1, 600);
}

#[test]
fn load_more_during_refresh_does_not_queue_a_stale_cursor_or_drop_later_refresh() {
    let history = commits_named("c", 1200);
    let mut state = paginated_repo_state(&history[..600]);
    let first = refresh_history(&mut state);
    let pagination = history_message(&mut state, Msg::LoadMoreHistory { repo_id: RepoId(1) });
    assert!(pagination.is_empty());
    refresh_history(&mut state);
    let next = answer_log(&mut state, &first, &history);
    assert!(
        log_effect(&next).2.is_none(),
        "the later refresh must run from the tips"
    );
    answer_log(&mut state, &next, &history);
    let pagination = history_message(&mut state, Msg::LoadMoreHistory { repo_id: RepoId(1) });
    assert_eq!(
        log_effect(&pagination).2,
        Some(cursor_after(&history[..600]))
    );
    answer_log(&mut state, &pagination, &history);
    assert!(
        matches!(&state.repos[0].history_state.log, Loadable::Ready(page) if page.commits == history[..800])
    );
}

#[test]
fn reload_supersedes_refresh_and_pagination_and_restarts_at_first_page() {
    for paginating in [false, true] {
        let history = commits_named("c", 1200);
        let mut state = paginated_repo_state(&history[..600]);
        let old = if paginating {
            history_message(&mut state, Msg::LoadMoreHistory { repo_id: RepoId(1) })
        } else {
            refresh_history(&mut state)
        };
        let reload = history_message(&mut state, Msg::ReloadRepo { repo_id: RepoId(1) });
        assert_eq!(log_effect(&reload).1, 200);
        assert!(answer_log(&mut state, &old, &history).is_empty());
        assert!(state.repos[0].history_state.log.is_loading());
        answer_log(&mut state, &reload, &history);
        assert!(
            matches!(&state.repos[0].history_state.log, Loadable::Ready(page) if page.commits == history[..200])
        );
    }
}

#[test]
fn cancelled_operation_keeps_ready_history_and_selection() {
    let history = commits_named("c", 600);
    let mut state = paginated_repo_state(&history);
    let selected = history[450].id.clone();
    state.repos[0].set_selected_commit(Some(selected.clone()));
    let effects = history_message(
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::GitOperationFinished {
            repo_id: RepoId(1),
            operation_id: gitcomet_core::git_operation::GitOperationId(1),
            outer_outcome: crate::model::GitOperationOuterOutcome::Cancelled,
            duration: std::time::Duration::ZERO,
            message: Box::new(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: RepoId(1),
                action: RepoActionKind::CheckoutBranch,
                result: Err(Error::new(gitcomet_core::error::ErrorKind::Cancelled)),
            }),
        }),
    );
    assert!(
        matches!(&state.repos[0].history_state.log, Loadable::Ready(page) if page.commits == history)
    );
    assert_eq!(
        state.repos[0].history_state.selected_commit.as_ref(),
        Some(&selected)
    );
    assert_eq!(log_effect(&effects).1, 600);
}

/// Alt-tabbing back to the window refreshes the log through
/// `RepoExternallyChanged`. With three "load more" pages on screen and the
/// selection on the last of them, a refresh that walked one default page
/// would replace 600 rows with 200: the selected commit would vanish and the
/// list, now shorter than its scroll offset, would clamp back toward the top.
#[test]
fn activation_refresh_reloads_the_depth_the_user_paginated_to() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let commits = commits_named("c", 600);
    let selected = commits[450].id.clone();
    let mut state = paginated_repo_state(&commits);
    let repo_state = &mut state.repos[0];
    repo_state.set_selected_commit(Some(selected.clone()));
    repo_state.set_commit_multi_selection(crate::model::CommitMultiSelection {
        commits: vec![selected.clone()].into(),
        anchor: Some(selected.clone()),
        anchor_index: Some(450),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    });

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RepoExternallyChanged {
            repo_id: RepoId(1),
            change: crate::msg::RepoExternalChange::all(),
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                repo_id: RepoId(1),
                walk,
                limit: 600,
                cursor: None,
                ..
            } if walk.mode() == LogScope::CurrentBranch
        )),
        "expected the refresh to walk as deep as the loaded log, got {effects:?}"
    );

    answer_log(&mut state, &effects, &commits);

    let repo_state = &state.repos[0];
    assert!(
        matches!(&repo_state.history_state.log, Loadable::Ready(page) if page.commits.len() == 600),
        "the reloaded page must hold every row that was on screen"
    );
    assert_eq!(
        repo_state.history_state.selected_commit.as_ref(),
        Some(&selected),
        "the selected commit must survive the refresh"
    );
    assert_eq!(
        *repo_state.history_state.multi_selection.commits,
        vec![selected]
    );
}

#[test]
fn refresh_of_a_log_shorter_than_a_page_keeps_the_default_page_size() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = paginated_repo_state(&commits_named("c", 3));

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
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                limit: 200,
                cursor: None,
                ..
            }
        )),
        "a short log refreshes with the default page, got {effects:?}"
    );
}

/// A refresh that arrives while a "load more" walk is running queues behind
/// it. By the time it starts the log is a page deeper, so the depth it walks
/// is settled when it is promoted, not when it was queued.
#[test]
fn queued_refresh_promoted_after_load_more_covers_the_grown_log() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(1);
    let first_pages = commits_named("c", 400);
    let mut state = paginated_repo_state(&first_pages);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::LoadMoreHistory { repo_id: RepoId(1) },
    );
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadLog {
            limit: 200,
            cursor: Some(_),
            ..
        }]
    ));
    let load_more_seq = active_log_seq(&state.repos[0]);

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
        !effects
            .iter()
            .any(|effect| matches!(effect, Effect::LoadLog { .. })),
        "the refresh must queue behind the running load-more, got {effects:?}"
    );

    let third_page = commits_named("d", 200);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq: load_more_seq,
            scope: LogScope::CurrentBranch,
            cursor: Some(cursor_after(&first_pages)),
            result: Ok(LogPage {
                commits: third_page.clone(),
                next_cursor: Some(cursor_after(&third_page)),
            }
            .into()),
        }),
    );
    assert!(
        matches!(&state.repos[0].history_state.log, Loadable::Ready(page) if page.commits.len() == 600)
    );
    assert!(
        matches!(
            effects.as_slice(),
            [Effect::LoadLog {
                limit: 600,
                cursor: None,
                ..
            },]
        ),
        "unexpected effects: {effects:?}"
    );
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
            Msg::HistoryFilter(HistoryFilterMsg::SetScope {
                repo_id: RepoId(1),
                scope: target_scope,
            }),
        );

        let repo_state = &state.repos[0];
        assert_eq!(repo_state.history_state.history_scope, target_scope);
        assert!(repo_state.history_state.log.is_loading());
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
                    walk,
                    cursor: None,
                    ..
                } if walk.mode() == target_scope
            )),
            "expected LoadLog({target_scope:?}) effect, got {effects:?}"
        );
        assert!(
            effects.iter().any(|effect| matches!(
                effect,
                Effect::Persistence(PersistenceEffect::PersistRepoHistoryMode {
                    repo_id: Some(RepoId(1)),
                    mode,
                    ..
                }) if *mode == target_scope
            )),
            "expected async history mode persist effect for {target_scope:?}, got {effects:?}"
        );
    }
}

#[test]
fn set_history_scope_retains_ready_log_while_loading() {
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
        Msg::HistoryFilter(HistoryFilterMsg::SetScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        }),
    );

    let repo_state = &state.repos[0];
    assert!(repo_state.history_state.log.is_loading());
    let retained = repo_state
        .history_state
        .retained_log_while_loading
        .as_ref()
        .expect("scope switch should retain the previous ready log while loading");
    assert!(Arc::ptr_eq(retained, &retained_page));
}

#[test]
fn stale_log_loaded_result_replays_latest_pending_scope_switch() {
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
    let seq = repo_state
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            walk: LogScope::FullReachable.into(),
            author: None,
            focus: Default::default(),
            limit: 200,
            cursor: None,
        })
        .expect("the first walk starts immediately");

    // Each switch supersedes the walk in flight and is dispatched at once,
    // rather than queueing behind a walk that may run for tens of seconds.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::HistoryFilter(HistoryFilterMsg::SetScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        }),
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                walk,
                cursor: None,
                ..
            } if walk.mode() == LogScope::AllBranches
        )),
        "expected the scope switch to start its load immediately, got {effects:?}"
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::HistoryFilter(HistoryFilterMsg::SetScope {
            repo_id: RepoId(1),
            scope: LogScope::NoMerges,
        }),
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog {
                walk,
                cursor: None,
                ..
            } if walk.mode() == LogScope::NoMerges
        )),
        "expected the second scope switch to start immediately too, got {effects:?}"
    );
    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::NoMerges
    );
    assert!(state.repos[0].history_state.log.is_loading());

    // The first walk finally answers. It is superseded, so it must neither land
    // in the log nor disturb the walk that replaced it.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::FullReachable,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![],
                next_cursor: None,
            })
            .into()),
        }),
    );

    assert!(state.repos[0].history_state.log.is_loading());
    assert!(!state.repos[0].history_state.log_loading_more);
    assert!(
        effects.is_empty(),
        "a superseded reply must not schedule anything, got {effects:?}"
    );
    assert!(
        !state.repos[0].loads_in_flight.is_active_log_reply(seq),
        "the superseded walk must no longer be the active one"
    );
}

#[test]
fn load_more_history_noops_when_no_next_cursor() {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.log = Loadable::Ready(Arc::new(LogPage {
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

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.history_state.log = Loadable::Ready(Arc::new(LogPage {
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
    let log_before = repo_state.history_state.log_rev;

    let seq = expect_log_reply(
        &mut state.repos[0],
        LogScope::CurrentBranch,
        None,
        Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    );

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![Commit {
                    id: CommitId("c2".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s2".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            })
            .into()),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(!repo_state.history_state.log_loading_more);
    assert!(repo_state.history_state.log_rev > log_before);
    let Loadable::Ready(page) = &repo_state.history_state.log else {
        panic!("expected log ready");
    };
    assert_eq!(page.commits.len(), 2);
    assert_eq!(page.commits[0].id.as_ref(), "c1");
    assert_eq!(page.commits[1].id.as_ref(), "c2");
    assert_eq!(page.next_cursor, None);
}

#[test]
fn log_loaded_reconciles_commit_multi_selection() {
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

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![CommitId("kept".into()), CommitId("gone".into())].into(),
        anchor: Some(CommitId("gone".into())),
        anchor_index: Some(1),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    };

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![commit("kept"), commit("other")],
                next_cursor: None,
            })
            .into()),
        }),
    );

    let sel = &state.repos[0].history_state.multi_selection;
    assert_eq!(*sel.commits, vec![CommitId("kept".into())]);
    assert_eq!(sel.anchor, None);
    assert_eq!(sel.anchor_index, None);
    assert_eq!(sel.anchor_log_rev, None);
}

/// Reloads the log without `gone` while `kept`/`gone` are compared as `source`
/// with a file of it open; returns the comparison left, the reload's effects
/// and the open diff.
fn reload_dropping_a_compared_commit(
    source: crate::model::ComparisonSource,
) -> (
    Option<crate::model::RangeSelection>,
    Vec<Effect>,
    Option<gitcomet_core::domain::DiffTarget>,
) {
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
    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };
    let kept = CommitId("kept".into());
    let gone = CommitId("gone".into());

    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![kept.clone(), gone.clone()].into(),
        ..Default::default()
    };
    repo_state.history_state.selected_commit = Some(kept.clone());
    repo_state.history_state.range_selection = Some(crate::model::RangeSelection {
        source,
        ..crate::model::RangeSelection::new(
            gone.clone(),
            Some(kept.clone()),
            "gone".into(),
            "kept".into(),
        )
    });
    repo_state.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::commit_range(
        gone,
        Some(kept),
        Some(PathBuf::from("a.txt")),
    ));

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![commit("kept"), commit("other")],
                next_cursor: None,
            })
            .into()),
        }),
    );
    (
        state.repos[0].history_state.range_selection.clone(),
        effects,
        state.repos[0].diff_state.diff_target.clone(),
    )
}

#[test]
fn log_reload_that_drops_a_compared_commit_leaves_the_selection_comparison() {
    use crate::model::ComparisonSource;
    for source in [
        ComparisonSource::SelectedPair,
        ComparisonSource::MergedSelection,
    ] {
        let (range, effects, diff_target) = reload_dropping_a_compared_commit(source);
        assert_eq!(range, None, "{source:?}");
        assert_eq!(
            diff_target, None,
            "the comparison's open diff goes too ({source:?})"
        );
        assert!(
            effects.iter().any(|e| matches!(
                e,
                Effect::LoadCommitDetails { commit_id, .. } if commit_id.as_ref() == "kept"
            )),
            "the details pane goes back to the still-selected commit ({source:?})"
        );
    }
    let (range, _, diff_target) = reload_dropping_a_compared_commit(ComparisonSource::Explicit);
    assert!(
        range.is_some(),
        "an explicit comparison does not come from the selection"
    );
    assert!(
        diff_target.is_some(),
        "an explicit comparison keeps its open diff"
    );
}

fn lookup_commit(id: &CommitId, summary: &str) -> Commit {
    Commit {
        id: id.clone(),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: summary.into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    }
}

fn lookup_state() -> (RepositoryRuntimes, AtomicU64, AppState) {
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    (Default::default(), AtomicU64::new(1), state)
}

/// The Reveal Commit dialog previews a reference without committing to it, so a
/// lookup must resolve and report without touching the selection.
#[test]
fn commit_lookup_resolves_without_selecting_anything() {
    let (mut repos, id_alloc, mut state) = lookup_state();
    let reference = CommitId("deadbee".into());
    let full = CommitId("deadbeef0123456789abcdef0123456789abcdef".into());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveCommitLookup {
            repo_id: RepoId(1),
            reference: reference.clone(),
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
        },
    );
    let request = match effects.as_slice() {
        [
            Effect::ResolveCommitLookup {
                reference: r,
                request,
                ..
            },
        ] if *r == reference => *request,
        other => panic!("expected a single lookup effect, got {other:?}"),
    };
    let lookup = &state.repos[0].history_state.commit_lookup;
    assert_eq!(lookup.request, request);
    assert!(lookup.result.is_loading());

    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitLookupResolved {
            repo_id: RepoId(1),
            reference,
            request,
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
            result: Ok(lookup_commit(&full, "the reland")),
        }),
    );

    let history = &state.repos[0].history_state;
    assert!(
        matches!(&history.commit_lookup.result, Loadable::Ready(commit) if commit.id == full),
        "the lookup should hold the resolved commit, got {:?}",
        history.commit_lookup.result
    );
    assert_eq!(
        history.selected_commit, None,
        "a preview must not move the history selection"
    );
    assert_eq!(history.reveal_target, None);
}

/// Every keystroke issues a lookup, so replies arrive out of order. The newest
/// request wins; an overtaken one must not repaint the row with a stale answer.
#[test]
fn commit_lookup_drops_a_reply_a_newer_lookup_overtook() {
    let (mut repos, id_alloc, mut state) = lookup_state();
    let first = CommitId("deadb".into());
    let second = CommitId("deadbee".into());

    let first_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveCommitLookup {
            repo_id: RepoId(1),
            reference: first.clone(),
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
        },
    );
    let first_request = match first_effects.as_slice() {
        [Effect::ResolveCommitLookup { request, .. }] => *request,
        other => panic!("expected a lookup effect, got {other:?}"),
    };
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveCommitLookup {
            repo_id: RepoId(1),
            reference: second.clone(),
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
        },
    );

    // The slower first lookup answers last.
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitLookupResolved {
            repo_id: RepoId(1),
            reference: first,
            request: first_request,
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
            result: Ok(lookup_commit(&CommitId("stale".into()), "stale")),
        }),
    );

    let lookup = &state.repos[0].history_state.commit_lookup;
    assert_eq!(lookup.reference.as_ref(), Some(&second));
    assert!(
        lookup.result.is_loading(),
        "the overtaken reply must not land, got {:?}",
        lookup.result
    );
}

/// An unresolvable reference is the normal state of a half-typed one, so it is
/// left in the lookup for the dialog to render rather than raised as a toast.
#[test]
fn commit_lookup_failure_stays_inline_without_a_notification() {
    let (mut repos, id_alloc, mut state) = lookup_state();
    let reference = CommitId("nosuchref".into());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ResolveCommitLookup {
            repo_id: RepoId(1),
            reference: reference.clone(),
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
        },
    );
    let request = match effects.as_slice() {
        [Effect::ResolveCommitLookup { request, .. }] => *request,
        other => panic!("expected a lookup effect, got {other:?}"),
    };

    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitLookupResolved {
            repo_id: RepoId(1),
            reference,
            request,
            purpose: crate::model::CommitLookupPurpose::RevealDialog,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("gix rev-parse nosuchref".into()),
            )),
        }),
    );

    assert!(matches!(
        state.repos[0].history_state.commit_lookup.result,
        Loadable::Error(_)
    ));
    assert!(
        state.notifications.is_empty(),
        "a failed preview must not toast, got {:?}",
        state.notifications
    );
}

/// A reveal asks git to resolve the reference before touching the selection, so
/// an abbreviation lands on the full id and the details pane fills in without
/// the log having paged anywhere near the commit.
#[test]
fn reveal_commit_resolves_an_abbreviation_and_shows_it_immediately() {
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

    let reference = CommitId("deadbee".into());
    let full = CommitId("deadbeef0123456789abcdef0123456789abcdef".into());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevealCommit {
            repo_id: RepoId(1),
            reference: reference.clone(),
        },
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::ResolveCommitForReveal { reference: r, .. } if *r == reference
        )),
        "asking to reveal should resolve the reference, got {effects:?}"
    );
    // Nothing is selected until it is known to exist.
    assert_eq!(state.repos[0].history_state.selected_commit, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
            repo_id: RepoId(1),
            reference,
            result: Ok(commit_details_for(&full, "the reland")),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit.as_ref(), Some(&full));
    assert_eq!(history.reveal_target.as_ref(), Some(&full));
    assert!(
        matches!(&history.commit_details, Loadable::Ready(details) if details.id == full),
        "the details fetched to resolve the reference should be the ones shown"
    );
}

/// A hex-looking run that is not a commit — a Gerrit change id, say — must fail
/// loudly and cheaply instead of sending the log walking to the root.
#[test]
fn reveal_commit_reports_an_unresolvable_reference_without_selecting() {
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

    let reference = CommitId("7a5d480873e839444e4e188ffa87f9c635e2fb81".into());
    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RevealCommit {
            repo_id: RepoId(1),
            reference: reference.clone(),
        },
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::CommitRevealResolved {
            repo_id: RepoId(1),
            reference,
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("unknown revision".to_string()),
            )),
        }),
    );

    assert!(effects.is_empty(), "a dead reference starts no work");
    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit, None);
    assert_eq!(history.reveal_target, None);
    assert!(
        state
            .notifications
            .iter()
            .any(|note| note.message.contains("Could not find commit")),
        "the user has to be told the reference went nowhere"
    );
}

/// A "load more" only ever grows the page, so a selection it does not contain
/// has not been paged in yet — it has not vanished. Clearing it there used to
/// make the details pane flip to the working tree once per batch for the whole
/// length of a reveal.
#[test]
fn log_loaded_keeps_a_not_yet_paged_selection_when_loading_more() {
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

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };

    let deep = CommitId("deep".into());
    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_log(Loadable::Ready(Arc::new(LogPage {
        commits: vec![commit("c1")],
        next_cursor: Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    })));
    repo_state.set_selected_commit(Some(deep.clone()));
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![deep.clone()].into(),
        anchor: Some(deep.clone()),
        anchor_index: Some(0),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    };

    let seq = expect_log_reply(
        &mut state.repos[0],
        LogScope::CurrentBranch,
        None,
        Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    );

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![commit("c2")],
                next_cursor: Some(LogCursor {
                    last_seen: CommitId("c2".into()),
                    resume_from: None,
                    resume_token: None,
                }),
            })
            .into()),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit.as_ref(), Some(&deep));
    assert_eq!(*history.multi_selection.commits, vec![deep]);
}

/// A first page *replaces* the log, so it can genuinely retire a selection —
/// unless a reveal is still walking toward exactly that commit, which is what a
/// mid-reveal scope switch does.
#[test]
fn log_loaded_first_page_keeps_the_commit_a_reveal_is_walking_toward() {
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

    let commit = |id: &str| Commit {
        id: CommitId(id.into()),
        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
        summary: "s".into(),
        author: "a".into(),
        time: SystemTime::UNIX_EPOCH,
    };

    let target = CommitId("target".into());
    let repo_state = &mut state.repos[0];
    repo_state.history_state.history_scope = LogScope::CurrentBranch;
    repo_state.set_reveal_target(Some(target.clone()));
    repo_state.set_selected_commit(Some(target.clone()));
    repo_state.history_state.multi_selection = crate::model::CommitMultiSelection {
        commits: vec![target.clone()].into(),
        anchor: Some(target.clone()),
        anchor_index: Some(0),
        anchor_log_rev: Some(repo_state.history_state.log_rev),
    };

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![commit("other")],
                next_cursor: None,
            })
            .into()),
        }),
    );

    let history = &state.repos[0].history_state;
    assert_eq!(history.selected_commit.as_ref(), Some(&target));
    assert_eq!(*history.multi_selection.commits, vec![target]);
}

#[test]
fn log_loaded_appends_when_loading_more_re_shares_history_log_arc() {
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

    let seq = expect_log_reply(
        &mut state.repos[0],
        LogScope::CurrentBranch,
        None,
        Some(LogCursor {
            last_seen: CommitId("c1".into()),
            resume_from: None,
            resume_token: None,
        }),
    );

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::CurrentBranch,
            cursor: Some(LogCursor {
                last_seen: CommitId("c1".into()),
                resume_from: None,
                resume_token: None,
            }),
            result: Ok(std::sync::Arc::new(LogPage {
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
            })
            .into()),
        }),
    );

    let repo_state = &state.repos[0];
    let Loadable::Ready(repo_log) = &repo_state.history_state.log else {
        panic!("expected repo log ready");
    };

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
        Msg::HistoryFilter(HistoryFilterMsg::SetScope {
            repo_id: RepoId(1),
            scope: LogScope::AllBranches,
        }),
    );

    assert!(
        state.repos[0]
            .history_state
            .retained_log_while_loading
            .is_some()
    );

    let seq = expect_log_reply(&mut state.repos[0], LogScope::AllBranches, None, None);
    let _ = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: LogScope::AllBranches,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![Commit {
                    id: CommitId("new".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "new".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            })
            .into()),
        }),
    );

    let repo_state = &state.repos[0];
    assert!(matches!(repo_state.history_state.log, Loadable::Ready(_)));
    assert!(
        repo_state
            .history_state
            .retained_log_while_loading
            .is_none()
    );
}

#[test]
fn log_loaded_initial_paginated_page_keeps_append_slack() {
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
    let seq = expect_log_reply(&mut state.repos[0], history_scope, None, None);

    let _effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: RepoId(1),
            seq,
            scope: history_scope,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits,
                next_cursor: Some(LogCursor {
                    last_seen,
                    resume_from: None,
                    resume_token: None,
                }),
            })
            .into()),
        }),
    );

    let Loadable::Ready(page) = &state.repos[0].history_state.log else {
        panic!("expected log ready");
    };
    assert!(page.commits.capacity() >= page.commits.len() + 512);
}

// --- Revision counter regression tests ---

#[test]
fn log_loaded_bumps_log_rev() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let log_before = state.repos[0].history_state.log_rev;
    let history_scope = state.repos[0].history_state.history_scope;
    let seq = expect_log_reply(&mut state.repos[0], history_scope, None, None);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            seq,
            scope: history_scope,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
                commits: vec![Commit {
                    id: CommitId("c1".into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "s1".into(),
                    author: "a".into(),
                    time: SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            })
            .into()),
        }),
    );

    assert!(
        state.repos[0].history_state.log_rev > log_before,
        "log_rev should bump after LogLoaded"
    );
}

#[test]
fn detached_head_target_tracks_current_branch_log_head() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
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

    let seq = expect_log_reply(&mut state.repos[0], LogScope::CurrentBranch, None, None);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id,
            seq,
            scope: LogScope::CurrentBranch,
            cursor: None,
            result: Ok(std::sync::Arc::new(LogPage {
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
            })
            .into()),
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
        let mut repos: RepositoryRuntimes = Default::default();
        let id_alloc = AtomicU64::new(2);
        let mut state = AppState::test_default();
        let repo_id = RepoId(1);
        repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
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

        let seq = expect_log_reply(&mut state.repos[0], scope, None, None);
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::Internal(crate::msg::InternalMsg::LogLoaded {
                repo_id,
                seq,
                scope,
                cursor: None,
                result: Ok(std::sync::Arc::new(LogPage {
                    commits,
                    next_cursor: None,
                })
                .into()),
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
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = AtomicU64::new(2);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, Arc::new(DummyRepo::new("/tmp/repo")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(repo_id);

    let log_before = state.repos[0].history_state.log_rev;

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::HistoryFilter(HistoryFilterMsg::SetScope {
            repo_id,
            scope: LogScope::AllBranches,
        }),
    );

    assert!(
        state.repos[0].history_state.log_rev > log_before,
        "log_rev should bump after SetHistoryScope"
    );
}
