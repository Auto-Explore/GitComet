use super::*;

fn repo_state_for_focus_tests() -> AppState {
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    state.active_repo = Some(RepoId(1));
    state
}

fn branch_named(name: &str) -> gitcomet_core::domain::Branch {
    gitcomet_core::domain::Branch {
        name: name.to_string(),
        target: CommitId("a".repeat(40).into()),
        upstream: None,
        divergence: None,
    }
}

/// Focusing restarts the walk with the focus attached and writes it to the
/// session, the same contract the author filter has.
#[test]
fn toggling_a_focus_restarts_the_walk_and_persists_it() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let scope = state.repos[0].history_state.history_scope;
    let unfocused = expect_log_reply(&mut state.repos[0], scope, None, None);
    let target = gitcomet_core::domain::HistoryFocus::local_branch("feature");
    let expected = HistoryFocusSet::from_iter([target.clone()]);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ToggleHistoryFocus {
            repo_id: RepoId(1),
            target: target.clone(),
        },
    );

    assert_eq!(state.repos[0].history_state.history_focus, expected);
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog { focus, cursor: None, .. } if *focus == expected
        )),
        "expected the focus to start its own first-page walk, got {effects:?}"
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::Persistence(PersistenceEffect::PersistRepoHistoryFocus { focus, .. }) if *focus == expected
        )),
        "expected the focus to be persisted, got {effects:?}"
    );
    assert!(
        !state.repos[0]
            .loads_in_flight
            .is_active_log_reply(unfocused),
        "the unfocused walk it replaced is no longer the active one"
    );
}

/// Toggling the same ref again takes it back out, which is the only way the
/// per-ref menu entry can clear what it set.
#[test]
fn toggling_a_focused_ref_again_removes_just_that_ref() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let feature = gitcomet_core::domain::HistoryFocus::local_branch("feature");
    let main = gitcomet_core::domain::HistoryFocus::local_branch("main");
    state.repos[0].history_state.history_focus =
        HistoryFocusSet::from_iter([feature.clone(), main.clone()]);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ToggleHistoryFocus {
            repo_id: RepoId(1),
            target: feature,
        },
    );

    assert_eq!(
        state.repos[0].history_state.history_focus,
        HistoryFocusSet::from_iter([main]),
        "the other focused ref must survive"
    );
}

/// A branch group's menu entry and eye focus all of its members in one message,
/// which restarts the walk and persists the set once rather than per member.
#[test]
fn focusing_a_groups_refs_adds_them_all_in_one_step() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let main = gitcomet_core::domain::HistoryFocus::local_branch("main");
    let feat_a = gitcomet_core::domain::HistoryFocus::local_branch("feat/a");
    let feat_b = gitcomet_core::domain::HistoryFocus::local_branch("feat/b");
    state.repos[0].history_state.history_focus = HistoryFocusSet::from_iter([main.clone()]);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::FocusHistoryRefs {
            repo_id: RepoId(1),
            targets: vec![feat_a.clone(), feat_b.clone()],
            focused: true,
        },
    );
    let expected = HistoryFocusSet::from_iter([main.clone(), feat_a.clone(), feat_b.clone()]);
    assert_eq!(state.repos[0].history_state.history_focus, expected);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(
                effect,
                Effect::Persistence(PersistenceEffect::PersistRepoHistoryFocus { .. })
            ))
            .count(),
        1,
        "expected one persist for the whole group, got {effects:?}"
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::FocusHistoryRefs {
            repo_id: RepoId(1),
            targets: vec![feat_a, feat_b],
            focused: false,
        },
    );
    assert_eq!(
        state.repos[0].history_state.history_focus,
        HistoryFocusSet::from_iter([main]),
        "unfocusing the group leaves refs outside it focused"
    );
}

/// A branch under a focused remote reads as focused, so toggling it takes it
/// out — and the rest of the remote stays focused through the branch list.
#[test]
fn toggling_a_branch_its_focused_remote_covers_unfocuses_just_that_branch() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let remote_branch = |name: &str| gitcomet_core::domain::RemoteBranch {
        remote: "origin".to_string(),
        name: name.to_string(),
        target: CommitId("a".repeat(40).into()),
    };
    state.repos[0].set_remote_branches(Loadable::Ready(vec![
        remote_branch("main"),
        remote_branch("feature"),
    ]));
    state.repos[0].history_state.history_focus =
        HistoryFocusSet::from_iter([gitcomet_core::domain::HistoryFocus::remote("origin")]);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ToggleHistoryFocus {
            repo_id: RepoId(1),
            target: gitcomet_core::domain::HistoryFocus::remote_branch("origin", "feature"),
        },
    );

    assert_eq!(
        state.repos[0].history_state.history_focus,
        HistoryFocusSet::from_iter([gitcomet_core::domain::HistoryFocus::remote_branch(
            "origin", "main"
        )]),
    );
}

/// Setting the set the repository already has must not restart the walk.
#[test]
fn setting_the_focus_already_in_effect_changes_nothing() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let focus = HistoryFocusSet::from_iter([gitcomet_core::domain::HistoryFocus::remote_branch(
        "origin", "main",
    )]);
    state.repos[0].history_state.history_focus = focus.clone();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryFocus {
            repo_id: RepoId(1),
            focus,
        },
    );

    assert!(effects.is_empty(), "expected no effects, got {effects:?}");
}

/// "Stop focus" empties the set in one message and returns the normal walk.
#[test]
fn clearing_the_focus_set_restarts_the_unfocused_walk() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    state.repos[0].history_state.history_focus = HistoryFocusSet::from_iter([
        gitcomet_core::domain::HistoryFocus::local_branch("feature"),
        gitcomet_core::domain::HistoryFocus::remote("origin"),
    ]);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryFocus {
            repo_id: RepoId(1),
            focus: HistoryFocusSet::default(),
        },
    );

    assert!(state.repos[0].history_state.history_focus.is_empty());
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog { focus, cursor: None, .. } if focus.is_empty()
        )),
        "expected the normal walk to be restarted, got {effects:?}"
    );
}

/// A focused branch that is deleted would otherwise leave the history empty with
/// nothing on screen to explain it, so the branch list clears the focus.
#[test]
fn a_branch_list_without_the_focused_branch_clears_the_focus() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    state.repos[0].history_state.history_focus = HistoryFocusSet::from_iter([
        gitcomet_core::domain::HistoryFocus::local_branch("feature"),
        gitcomet_core::domain::HistoryFocus::local_branch("main"),
    ]);

    // Still there: the focus survives.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(vec![branch_named("main"), branch_named("feature")]),
        }),
    );
    assert_eq!(state.repos[0].history_state.history_focus.len(), 2);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(vec![branch_named("main")]),
        }),
    );

    // Only the deleted branch drops out; `main` is still focused.
    assert_eq!(
        state.repos[0].history_state.history_focus,
        HistoryFocusSet::from_iter([gitcomet_core::domain::HistoryFocus::local_branch("main")]),
    );
    assert!(
        effects.iter().any(|effect| matches!(
            effect,
            Effect::LoadLog { focus, cursor: None, .. } if focus.len() == 1
        )),
        "expected the surviving focus to be reloaded, got {effects:?}"
    );
}

/// A failed branch load says nothing about whether the branch exists, so it
/// must not be read as a deletion.
#[test]
fn a_failed_branch_load_leaves_the_focus_alone() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let focus =
        HistoryFocusSet::from_iter([gitcomet_core::domain::HistoryFocus::local_branch("feature")]);
    state.repos[0].history_state.history_focus = focus.clone();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::BranchesLoaded {
            repo_id: RepoId(1),
            result: Err(gitcomet_core::error::Error::new(
                gitcomet_core::error::ErrorKind::Backend("no".into()),
            )),
        }),
    );

    assert_eq!(state.repos[0].history_state.history_focus, focus);
}

/// The bug this guards: a repository finishing its startup loads dropped the
/// user's focus on its own. Only the list that governs a target may retire it.
#[test]
fn other_ref_lists_loading_never_clear_a_focus() {
    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let focus =
        HistoryFocusSet::from_iter([gitcomet_core::domain::HistoryFocus::local_branch("feature")]);
    state.repos[0].history_state.history_focus = focus.clone();
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![branch_named("feature")]));

    // A remote list arriving says nothing about a local branch.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemotesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert_eq!(state.repos[0].history_state.history_focus, focus);

    // Nor does a remote-branch list.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert_eq!(state.repos[0].history_state.history_focus, focus);
}

/// A successful empty list is a real answer: removing the last remote, or
/// pruning the last remote-tracking branch, leaves exactly that. Focus on what
/// it lost would otherwise hold the history empty until cleared by hand.
#[test]
fn a_successful_empty_list_clears_the_focus_it_governs() {
    use gitcomet_core::domain::HistoryFocus;

    let mut repos = RepositoryRuntimes::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = repo_state_for_focus_tests();
    let feature = HistoryFocus::local_branch("feature");
    state.repos[0].history_state.history_focus = HistoryFocusSet::from_iter([
        feature.clone(),
        HistoryFocus::remote("origin"),
        HistoryFocus::remote_branch("upstream", "main"),
    ]);

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RemotesLoaded {
            repo_id: RepoId(1),
            result: Ok(Vec::new()),
        }),
    );
    assert_eq!(
        state.repos[0].history_state.history_focus,
        HistoryFocusSet::from_iter([
            feature.clone(),
            HistoryFocus::remote_branch("upstream", "main"),
        ]),
        "an empty remote list retires the focused remote, and only that"
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
    assert_eq!(
        state.repos[0].history_state.history_focus,
        HistoryFocusSet::from_iter([feature]),
        "an empty remote-branch list retires the focused remote branch"
    );
}
