use super::*;

#[test]
fn open_repo_sets_opening_and_emits_effect() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.active_repo, Some(RepoId(1)));
    let repo_state = state.repos.first().expect("repo state to be set");
    assert_eq!(repo_state.id.0, 1);
    assert!(repo_state.open.is_loading());
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(1)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
}

#[test]
fn dropped_repo_stays_provisional_until_the_backend_opens_it() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/dropped-repo");

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(path),
    );

    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert_eq!(state.repos.len(), 1);
    assert!(state.repos[0].open.is_loading());
    assert!(state.repos[0].is_provisional_external_drop_open());
    assert!(matches!(
        effects.as_slice(),
        [Effect::OpenRepo {
            repo_id: RepoId(1),
            ..
        }]
    ));
    assert!(!effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistRecentRepo { .. }
            | Effect::PersistSession { .. }
            | Effect::PersistRepoHistoryMode { .. }
    )));
}

#[test]
fn repeating_or_closing_a_provisional_drop_never_records_it_as_recent() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/dropped-repo");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(path.clone()),
    );
    let repeated_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(path),
    );
    assert_eq!(state.repos.len(), 1);
    assert!(state.repos[0].is_provisional_external_drop_open());
    assert!(
        !repeated_effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistRecentRepo { .. }))
    );

    let close_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id: RepoId(1) },
    );
    assert!(state.repos.is_empty());
    assert!(
        !close_effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistRecentRepo { .. }))
    );
    assert!(
        close_effects
            .iter()
            .any(|effect| matches!(effect, Effect::PersistSession { .. }))
    );
}

#[test]
fn bulk_close_skips_provisional_drop_when_recording_recents() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/committed-repo")),
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(PathBuf::from("/tmp/dropped-repo")),
    );

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepos {
            repo_ids: vec![RepoId(1), RepoId(2)],
            activate_after: None,
        },
    );

    let recent_repo_ids: Vec<_> = effects
        .iter()
        .filter_map(|effect| match effect {
            Effect::PersistRecentRepo { repo_id, .. } => *repo_id,
            _ => None,
        })
        .collect();
    assert_eq!(recent_repo_ids, vec![RepoId(1)]);
}

#[test]
fn successful_dropped_repo_commits_and_emits_deferred_persistence() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let path = PathBuf::from("/tmp/dropped-repo");

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(path.clone()),
    );
    let spec = state.repos[0].spec.clone();
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedOk {
            repo_id: RepoId(1),
            spec,
            repo: Arc::new(DummyRepo::new(path.to_string_lossy().as_ref())),
        }),
    );

    assert!(matches!(state.repos[0].open, Loadable::Ready(())));
    assert!(!state.repos[0].is_provisional_external_drop_open());
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistRecentRepo {
            repo_id: Some(RepoId(1)),
            ..
        }
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistSession {
            repo_id: Some(RepoId(1)),
            ..
        }
    )));
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::PersistRepoHistoryMode {
            repo_id: Some(RepoId(1)),
            ..
        }
    )));
}

#[test]
fn operational_error_discards_dropped_tab_and_preserves_existing_recents() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let dir = tempfile::tempdir().expect("tempdir");
    let existing = dir.path().join("existing");
    let dropped = dir.path().join("dropped");
    let unrelated = dir.path().join("unrelated");
    let session_file = dir.path().join("session.json");
    for path in [&existing, &dropped, &unrelated] {
        std::fs::create_dir(path).expect("create repository candidate directory");
    }
    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));
    crate::session::persist_recent_repo_to_path(&dropped, &session_file)
        .expect("seed dropped recent");
    crate::session::persist_recent_repo_to_path(&unrelated, &session_file)
        .expect("seed unrelated recent");

    let existing_id = open_repo_ready(&mut repos, &id_alloc, &mut state, existing);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(dropped.clone()),
    );
    let dropped_id = state.active_repo.expect("dropped tab becomes active");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: dropped_id,
            spec: RepoSpec {
                workdir: dropped.clone(),
            },
            error: Error::new(ErrorKind::Backend("permission denied".to_string())),
        }),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(existing_id));
    assert_eq!(state.repos[0].id, existing_id);
    assert!(state.notifications.iter().any(|notification| {
        notification.kind == AppNotificationKind::Warning
            && notification.message
                == format!(
                    "Could not open repository at {}: permission denied",
                    super::reducer::normalize_repo_path(dropped.clone()).display()
                )
    }));

    let session = crate::session::load_from_path(&session_file);
    assert_eq!(
        session.recent_repos,
        vec![
            super::reducer::normalize_repo_path(unrelated),
            super::reducer::normalize_repo_path(dropped),
        ]
    );
}

#[test]
fn failed_dropped_repo_restores_the_repo_active_before_the_drop() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    let session_dir = tempfile::tempdir().expect("session tempdir");
    let _session_file_override = crate::session::push_test_session_file_path_override(Some(
        session_dir.path().join("session.json"),
    ));

    let original_active = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");
    let adjacent_repo = open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo3");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo {
            repo_id: original_active,
        },
    );

    let dropped_path = PathBuf::from("/tmp/not-a-repository");
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepoFromExternalDrop(dropped_path.clone()),
    );
    let dropped_id = state.active_repo.expect("dropped tab becomes active");
    assert_ne!(dropped_id, original_active);
    assert_eq!(
        state
            .repos
            .iter()
            .find(|repo| repo.id == dropped_id)
            .and_then(RepoState::external_drop_previous_active_repo),
        Some(original_active)
    );

    let failure_effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::RepoOpenedErr {
            repo_id: dropped_id,
            spec: RepoSpec {
                workdir: dropped_path,
            },
            error: Error::new(ErrorKind::Backend("open failed".to_string())),
        }),
    );

    assert_eq!(state.repos.len(), 3);
    assert_eq!(state.active_repo, Some(original_active));
    assert_ne!(state.active_repo, Some(adjacent_repo));
    assert!(
        has_status_refresh_effects(&failure_effects, original_active),
        "restoring the active repository must restart loads canceled by the dropped tab"
    );
    assert!(failure_effects.iter().any(
        |effect| matches!(effect, Effect::LoadLog { repo_id, .. } if *repo_id == original_active)
    ));
    assert!(failure_effects.iter().any(
        |effect| matches!(effect, Effect::LoadBranches { repo_id } if *repo_id == original_active)
    ));
}

#[test]
fn open_repo_focuses_existing_repo_instead_of_opening_duplicate() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo1");
    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo2");

    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(2)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo1")),
    );

    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh when focusing an already open repo"
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    let repo1 = super::reducer::normalize_repo_path(PathBuf::from("/tmp/repo1"));
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == repo1)
            .count(),
        1
    );
}

#[test]
fn open_repo_allows_same_basename_in_different_folders() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = std::env::temp_dir().join(format!(
        "gitcomet-open-repo-same-basename-test-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let repo_a = dir.join("a").join("repo");
    let repo_b = dir.join("b").join("repo");
    let _ = std::fs::create_dir_all(&repo_a);
    let _ = std::fs::create_dir_all(&repo_b);

    open_repo_ready(&mut repos, &id_alloc, &mut state, repo_a.clone());

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_b.clone()),
    );
    mark_repo_open_ready(&mut repos, &mut state, RepoId(2));
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::OpenRepo { repo_id, .. } if *repo_id == RepoId(2)))
    );
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::PersistSession { .. }))
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(2)));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_a.clone()),
    );
    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh when re-focusing repo by path"
    );
    assert_eq!(state.repos.len(), 2);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == super::reducer::normalize_repo_path(repo_a.clone()))
            .count(),
        1
    );
    assert_eq!(
        state
            .repos
            .iter()
            .filter(|r| r.spec.workdir == super::reducer::normalize_repo_path(repo_b.clone()))
            .count(),
        1
    );
}

#[test]
fn open_repo_refreshes_when_repo_is_already_active() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    open_repo_ready(&mut repos, &id_alloc, &mut state, "/tmp/repo");
    state.repos[0].missing_on_disk = true;

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(PathBuf::from("/tmp/repo")),
    );

    assert_eq!(state.repos.len(), 1);
    assert_eq!(state.active_repo, Some(RepoId(1)));
    assert!(
        has_status_refresh_effects(&effects, RepoId(1)),
        "expected status refresh when re-opening active repo"
    );
}

#[test]
fn open_repo_prefers_saved_history_mode_over_legacy_scope_and_default() {
    assert_open_repo_history_mode_resolution(
        |repo_path, session_file| {
            crate::session::persist_ui_settings_to_path(
                crate::session::UiSettings {
                    default_history_mode: Some(LogScope::MergesOnly),
                    ..Default::default()
                },
                session_file,
            )
            .expect("persist default history mode");
            crate::session::persist_repo_history_scope_to_path(
                repo_path,
                LogScope::AllBranches,
                session_file,
            )
            .expect("persist legacy history scope");
            crate::session::persist_repo_history_mode_to_path(
                repo_path,
                LogScope::NoMerges,
                session_file,
            )
            .expect("persist repo history mode");
        },
        LogScope::NoMerges,
    );
}

#[test]
fn open_repo_falls_back_to_legacy_history_scope_when_saved_mode_is_missing() {
    assert_open_repo_history_mode_resolution(
        |repo_path, session_file| {
            crate::session::persist_ui_settings_to_path(
                crate::session::UiSettings {
                    default_history_mode: Some(LogScope::MergesOnly),
                    ..Default::default()
                },
                session_file,
            )
            .expect("persist default history mode");
            crate::session::persist_repo_history_scope_to_path(
                repo_path,
                LogScope::CurrentBranch,
                session_file,
            )
            .expect("persist legacy history scope");
        },
        LogScope::FirstParent,
    );
}

#[test]
fn open_repo_falls_back_to_default_history_mode_when_repo_settings_are_missing() {
    assert_open_repo_history_mode_resolution(
        |_repo_path, session_file| {
            crate::session::persist_ui_settings_to_path(
                crate::session::UiSettings {
                    default_history_mode: Some(LogScope::AllBranches),
                    ..Default::default()
                },
                session_file,
            )
            .expect("persist default history mode");
        },
        LogScope::AllBranches,
    );
}

#[test]
fn open_repo_uses_builtin_default_history_mode_without_saved_preferences() {
    assert_open_repo_history_mode_resolution(|_, _| {}, LogScope::default());
}

#[test]
fn open_repo_persists_resolved_history_mode_and_keeps_it_sticky() {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();

    let dir = tempfile::tempdir().expect("tempdir");
    let repo_path = dir.path().join("repo");
    let session_file = dir.path().join("session.json");
    std::fs::create_dir_all(&repo_path).expect("create repo path");
    let normalized_repo_path = super::reducer::normalize_repo_path(repo_path.clone());

    crate::session::persist_ui_settings_to_path(
        crate::session::UiSettings {
            default_history_mode: Some(LogScope::AllBranches),
            ..Default::default()
        },
        &session_file,
    )
    .expect("persist initial default history mode");

    let _session_file_override =
        crate::session::push_test_session_file_path_override(Some(session_file.clone()));

    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::OpenRepo(repo_path.clone()),
    );

    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::AllBranches
    );
    let persist_history = effects
        .iter()
        .find_map(|effect| match effect {
            Effect::PersistRepoHistoryMode { workdir, mode, .. } => Some((workdir, mode)),
            _ => None,
        })
        .expect("expected async history mode persist effect");
    assert_eq!(persist_history.0, &normalized_repo_path);
    assert_eq!(*persist_history.1, LogScope::AllBranches);
    crate::session::persist_repo_history_mode_to_path(
        persist_history.0,
        *persist_history.1,
        &session_file,
    )
    .expect("apply async history mode persist effect");
    assert_eq!(
        crate::session::load_repo_history_mode_from_path(&normalized_repo_path, &session_file),
        Some(LogScope::AllBranches)
    );

    crate::session::persist_ui_settings_to_path(
        crate::session::UiSettings {
            default_history_mode: Some(LogScope::NoMerges),
            ..Default::default()
        },
        &session_file,
    )
    .expect("persist updated default history mode");

    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let mut state = AppState::default();
    reduce(&mut repos, &id_alloc, &mut state, Msg::OpenRepo(repo_path));

    assert_eq!(
        state.repos[0].history_state.history_scope,
        LogScope::AllBranches
    );
}
