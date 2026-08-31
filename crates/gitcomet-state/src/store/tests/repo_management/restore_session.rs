use super::*;

#[test]
fn remote_branches_loaded_sets_state() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(2);
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
        Msg::Internal(crate::msg::InternalMsg::RemoteBranchesLoaded {
            repo_id: RepoId(1),
            result: Ok(vec![RemoteBranch {
                remote: "origin".to_string(),
                name: "main".to_string(),
                target: CommitId("deadbeef".into()),
            }]),
        }),
    );

    let repo = state.repos.iter().find(|r| r.id == RepoId(1)).unwrap();
    match &repo.remote_branches {
        Loadable::Ready(branches) => {
            assert_eq!(branches.len(), 1);
            assert_eq!(branches[0].remote, "origin");
            assert_eq!(branches[0].name, "main");
        }
        other => panic!("expected Ready remote_branches, got {other:?}"),
    }
}

#[test]
fn restore_session_opens_only_active_repo_and_selects_active_repo() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = std::env::temp_dir().join(format!(
        "gitcomet-restore-session-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::create_dir_all(&dir);

    let repo_a = dir.join("repo-a");
    let repo_b = dir.join("repo-b");
    let _ = std::fs::create_dir_all(&repo_a);
    let _ = std::fs::create_dir_all(&repo_b);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a.clone(), repo_b],
            active_repo: Some(repo_a.clone()),
        },
    );

    assert_eq!(state.repos.len(), 2);
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::OpenRepo { .. }))
            .count(),
        1
    );
    assert_eq!(
        effects
            .iter()
            .filter(|e| matches!(e, Effect::PersistSession { .. }))
            .count(),
        1
    );

    let active_repo_id = state.active_repo.expect("active repo is set");
    let active_workdir = state
        .repos
        .iter()
        .find(|r| r.id == active_repo_id)
        .expect("active repo exists")
        .spec
        .workdir
        .clone();

    assert_eq!(active_workdir, super::reducer::normalize_repo_path(repo_a));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == active_repo_id)
            .expect("active repo exists")
            .open,
        Loadable::Loading
    ));
    assert!(
        state
            .repos
            .iter()
            .filter(|repo| repo.id != active_repo_id)
            .all(|repo| matches!(repo.open, Loadable::NotLoaded))
    );
}

#[test]
fn selecting_inactive_restored_repo_cancels_previous_load_and_starts_open() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a, repo_b],
            active_repo: None,
        },
    );

    let previous_active = state.active_repo.expect("active repo exists");
    let previous_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == previous_active)
        .expect("previous active repo exists")
        .load_epoch;
    let inactive_repo = state
        .repos
        .iter()
        .find(|repo| repo.id != previous_active)
        .expect("inactive repo exists")
        .id;
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == inactive_repo)
            .expect("inactive repo exists")
            .open,
        Loadable::NotLoaded
    ));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: inactive_repo,
        },
    );

    assert_eq!(state.active_repo, Some(inactive_repo));
    assert!(has_cancel_repo_loads_effect(
        &effects,
        previous_active,
        previous_epoch
    ));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == inactive_repo
    )));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == inactive_repo)
            .expect("inactive repo exists")
            .open,
        Loadable::Loading
    ));
}

#[test]
fn selecting_third_restored_repo_while_second_is_opening_cancels_second_open() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");
    let repo_c = dir.path().join("repo-c");
    std::fs::create_dir_all(&repo_a).expect("create repo-a");
    std::fs::create_dir_all(&repo_b).expect("create repo-b");
    std::fs::create_dir_all(&repo_c).expect("create repo-c");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_a.clone(), repo_b, repo_c],
            active_repo: Some(repo_a),
        },
    );

    let repo1 = RepoId(1);
    let repo2 = RepoId(2);
    let repo3 = RepoId(3);

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo2 },
    );
    assert_eq!(state.active_repo, Some(repo2));
    assert!(has_cancel_repo_loads_effect(&effects, repo1, 0));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo2
    )));

    let repo2_epoch = state
        .repos
        .iter()
        .find(|repo| repo.id == repo2)
        .expect("repo2 exists")
        .load_epoch;
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: repo3 },
    );

    assert_eq!(state.active_repo, Some(repo3));
    assert!(has_cancel_repo_loads_effect(&effects, repo2, repo2_epoch));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::OpenRepo { repo_id, .. } if *repo_id == repo3
    )));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo2)
            .expect("repo2 exists")
            .open,
        Loadable::NotLoaded
    ));
    assert!(matches!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == repo3)
            .expect("repo3 exists")
            .open,
        Loadable::Loading
    ));
}

#[test]
fn restore_session_resolves_history_mode_precedence_per_repository() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");
    let repo_mode = dir.path().join("repo-mode");
    let repo_legacy = dir.path().join("repo-legacy");
    let repo_default = dir.path().join("repo-default");
    std::fs::create_dir_all(&repo_mode).expect("create repo-mode");
    std::fs::create_dir_all(&repo_legacy).expect("create repo-legacy");
    std::fs::create_dir_all(&repo_default).expect("create repo-default");
    let normalized_repo_mode = super::reducer::normalize_repo_path(repo_mode.clone());
    let normalized_repo_legacy = super::reducer::normalize_repo_path(repo_legacy.clone());
    let normalized_repo_default = super::reducer::normalize_repo_path(repo_default.clone());

    crate::session::persist_ui_settings_to_path(
        crate::session::UiSettings {
            default_history_mode: Some(LogScope::MergesOnly),
            ..Default::default()
        },
        &session_file,
    )
    .expect("persist default history mode");
    crate::session::persist_repo_history_mode_to_path(
        &normalized_repo_mode,
        LogScope::NoMerges,
        &session_file,
    )
    .expect("persist repo mode");
    crate::session::persist_repo_history_scope_to_path(
        &normalized_repo_legacy,
        LogScope::CurrentBranch,
        &session_file,
    )
    .expect("persist legacy scope");

    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::RestoreSession {
            open_repos: vec![repo_mode.clone(), repo_legacy.clone(), repo_default.clone()],
            active_repo: Some(repo_default.clone()),
        },
    );

    let by_workdir = state
        .repos
        .iter()
        .map(|repo| (repo.spec.workdir.clone(), repo.history_state.history_scope))
        .collect::<FxHashMap<_, _>>();

    assert_eq!(
        by_workdir.get(&normalized_repo_mode),
        Some(&LogScope::NoMerges)
    );
    assert_eq!(
        by_workdir.get(&normalized_repo_legacy),
        Some(&LogScope::FirstParent)
    );
    assert_eq!(
        by_workdir.get(&normalized_repo_default),
        Some(&LogScope::MergesOnly)
    );
    assert_eq!(
        state.active_repo.and_then(|repo_id| state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.spec.workdir.clone())),
        Some(normalized_repo_default.clone())
    );
    let updates = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistRepoHistoryModesBatch { updates, .. } => Some(updates),
            _ => None,
        })
        .expect("expected async history mode batch persist effect");
    assert!(updates.contains(&(normalized_repo_legacy.clone(), LogScope::FirstParent)));
    assert!(updates.contains(&(normalized_repo_default.clone(), LogScope::MergesOnly)));
    crate::session::persist_repo_history_modes_batch_to_path(updates, &session_file)
        .expect("apply async history mode batch persist effect");
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_mode, &session_file),
        Some(LogScope::NoMerges)
    );
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_legacy, &session_file),
        Some(LogScope::FirstParent)
    );
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_default, &session_file),
        Some(LogScope::MergesOnly)
    );
}
