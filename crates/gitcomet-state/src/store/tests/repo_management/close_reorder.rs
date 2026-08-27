use super::*;

#[test]
fn close_repo_removes_and_moves_active() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(10);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(11)));
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(11))
        .expect("repo 11 exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: RepoId(11),
        },
    );

    assert!(has_cancel_repo_loads_effect(
        &effects,
        RepoId(11),
        old_epoch
    ));
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(10))
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(10)));
}

fn recent_repo_effect_workdirs(effects: &[Effect]) -> Vec<PathBuf> {
    effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PersistRecentRepo { workdir, .. } => Some(workdir.clone()),
            _ => None,
        })
        .collect()
}

/// Recording the close here rather than at the affordance that asked for it is
/// what keeps the Recently Closed order the same whichever way a repository was
/// closed — the repo tab's `x`, its menu, or the picker's row menu.
#[test]
fn close_repo_records_the_closed_repository_as_recent() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for name in ["repo1", "repo2"] {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/{name}"))),
        );
    }
    let closed_workdir = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(2))
        .expect("repo 2 exists")
        .spec
        .workdir
        .clone();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(2) },
    );

    assert_eq!(recent_repo_effect_workdirs(&effects), vec![closed_workdir]);

    // Closing something that is not open leaves the recents alone: there is no
    // workdir to name, and re-running a close must not reorder the list.
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(2) },
    );
    assert!(recent_repo_effect_workdirs(&effects).is_empty());
}

/// Bulk closes walk the tab strip left to right rather than the `FxHashSet` of
/// ids, so the Recently Closed order they leave behind is the same on every run.
#[test]
fn close_repos_records_recents_in_tab_order() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=3 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    let workdir_of = |state: &AppState, repo_id: RepoId| {
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .expect("repo exists")
            .spec
            .workdir
            .clone()
    };
    let first = workdir_of(&state, RepoId(1));
    let third = workdir_of(&state, RepoId(3));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(3), RepoId(999), RepoId(1)],
            activate_after: None,
        },
    );

    assert_eq!(recent_repo_effect_workdirs(&effects), vec![first, third]);
}

#[test]
fn close_repo_selects_right_neighbor_when_closing_first_active_tab() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(20);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: RepoId(20),
        },
    );
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(20))
        .expect("repo 20 exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo {
            repo_id: RepoId(20),
        },
    );

    assert!(has_cancel_repo_loads_effect(
        &effects,
        RepoId(20),
        old_epoch
    ));
    assert!(effects.iter().any(
        |effect| matches!(effect, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(21))
    ));
    assert!(
        effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(21)));
}

#[test]
fn close_repos_ignores_unknown_ids_and_persists_once() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    let old_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == RepoId(1))
        .expect("repo 1 exists")
        .load_epoch;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(999), RepoId(1), RepoId(1)],
            activate_after: None,
        },
    );

    assert!(has_cancel_repo_loads_effect(&effects, RepoId(1), old_epoch));
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistSession { .. }))
            .count(),
        1
    );
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));
}

#[test]
fn close_repos_selects_left_neighbor_when_active_repo_is_closed() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=3 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: RepoId(2) },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(2)],
            activate_after: None,
        },
    );

    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(1)));
}

#[test]
fn close_repos_uses_requested_active_repo_after_batch_close() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=3 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: RepoId(1) },
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(1), RepoId(3)],
            activate_after: Some(RepoId(2)),
        },
    );

    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![RepoId(2)]
    );
    assert_eq!(state.active_repo, Some(RepoId(2)));
}

#[test]
fn close_repos_noops_when_no_existing_repos_match() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=2 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }
    let original_repo_ids = state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>();
    let original_active = state.active_repo;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(999)],
            activate_after: Some(RepoId(1)),
        },
    );

    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        original_repo_ids
    );
    assert_eq!(state.active_repo, original_active);
}

#[test]
fn close_repos_closing_all_repos_clears_active_and_persists_once() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    for ix in 1..=2 {
        reduce(
            &mut repos,
            &id_alloc,
            &mut state,
            Msg::OpenRepo(PathBuf::from(format!("/tmp/repo{ix}"))),
        );
    }

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(1), RepoId(2)],
            activate_after: Some(RepoId(1)),
        },
    );

    assert!(state.repos.is_empty());
    assert_eq!(state.active_repo, None);
    assert_eq!(
        effects
            .iter()
            .filter(|effect| matches!(effect, Effect::PersistSession { .. }))
            .count(),
        1
    );
}

#[test]
fn reorder_repo_tabs_moves_repo_and_keeps_active() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );

    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: Some(RepoId(1)),
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(3), RepoId(1), RepoId(2)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: None,
        },
    );

    assert!(matches!(
        effects.as_slice(),
        [Effect::PersistSession { .. }]
    ));
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(state.active_repo, Some(RepoId(3)));
}

#[test]
fn reorder_repo_tabs_noops_for_invalid_or_already_stable_ordering() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );
    let original = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(1),
            insert_before: None,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo2")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo3")),
    );
    let original = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(999),
            insert_before: Some(RepoId(1)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(2),
            insert_before: Some(RepoId(2)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(1),
            insert_before: Some(RepoId(2)),
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReorderRepoTabs {
            repo_id: RepoId(3),
            insert_before: None,
        },
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos.iter().map(|r| r.id).collect::<Vec<_>>(),
        original
    );
}
