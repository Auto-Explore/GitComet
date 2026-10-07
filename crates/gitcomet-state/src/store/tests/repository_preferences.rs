use super::worktree_redirect::{open_repo_and_wait, repo_with_linked_worktree, wait_until};
use super::*;
use crate::model::{
    RepositoryFileSort, RepositoryKey, RepositoryListKind, RepositoryPreferenceUpdate,
    RepositoryPreferencesSnapshot, SharedRepositoryPreferences,
};
use gitcomet_core::domain::HistoryMode;
use std::collections::{BTreeMap, BTreeSet};

fn preferences(store: &AppStore, id: RepoId) -> RepositoryPreferencesSnapshot {
    store
        .snapshot()
        .repos
        .iter()
        .find(|repo| repo.id == id)
        .unwrap()
        .shared_preferences
        .clone()
        .expect("shared preferences initialized")
}

fn update(store: &AppStore, id: RepoId, update: RepositoryPreferenceUpdate) {
    store.dispatch(Msg::UpdateRepositoryPreference {
        repo_id: id,
        update,
    });
}

#[cfg(target_os = "linux")]
#[test]
fn failed_legacy_migration_keeps_preferences_for_later_updates_and_windows() {
    use std::os::fd::AsRawFd;

    let (dir, main, linked) = repo_with_linked_worktree();
    let path = dir.path().join("session.json");
    let main_key = crate::session::path_storage_key(&main);
    let original = serde_json::to_vec(&serde_json::json!({
        "version": 5,
        "open_repos": [],
        "repo_history_modes": {main_key.clone(): "first_parent"},
        "repo_sidebar_pinned_branches": {main_key.clone(): ["local:main"]},
        "repo_sidebar_collapsed_items": {main_key: ["group:local:existing"]}
    }))
    .unwrap();
    fs::write(&path, &original).unwrap();
    let session = fs::File::open(&path).unwrap();
    // procfs allows reading the legacy session but cannot create its backup
    // or atomically replace it, even when the test process runs as root.
    let read_only_path = PathBuf::from(format!("/proc/self/fd/{}", session.as_raw_fd()));
    let _path_guard = crate::session::push_test_session_file_path_override(Some(read_only_path));
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (first, _first_events) = AppStore::new_test(backend.clone());
    let first_id = open_repo_and_wait(&first, &main);
    let mut expected = SharedRepositoryPreferences {
        history_mode: HistoryMode::FirstParent,
        pinned_items: BTreeSet::from(["local:main".into()]),
        collapsed_items: BTreeSet::from(["group:local:existing".into()]),
        ..Default::default()
    };
    assert_eq!(*preferences(&first, first_id).preferences, expected);
    assert!(
        first.snapshot().repos[0]
            .feedback
            .diagnostics
            .iter()
            .any(|entry| {
                entry.kind == DiagnosticKind::Error
                    && entry
                        .message
                        .contains("initializing repository preferences")
            })
    );

    update(
        &first,
        first_id,
        RepositoryPreferenceUpdate::Pin {
            key: "local:dev".into(),
            pinned: true,
        },
    );
    expected.pinned_items.insert("local:dev".into());
    wait_until("pin preserves migrated preferences", || {
        *preferences(&first, first_id).preferences == expected
    });
    let (second, _second_events) = AppStore::new_test_sharing_preferences(backend, &first);
    let second_id = open_repo_and_wait(&second, &linked);
    assert_eq!(*preferences(&second, second_id).preferences, expected);
    update(
        &second,
        second_id,
        RepositoryPreferenceUpdate::CollapseItems {
            added: BTreeSet::from(["group:local:added".into()]),
            removed: BTreeSet::new(),
        },
    );
    expected.collapsed_items.insert("group:local:added".into());
    wait_until(
        "collapse preserves migrated preferences in both windows",
        || {
            *preferences(&first, first_id).preferences == expected
                && *preferences(&second, second_id).preferences == expected
        },
    );
    assert_eq!(
        first.snapshot().repos[0].history_state.history_scope,
        HistoryMode::FirstParent
    );
    assert_eq!(fs::read(&path).unwrap(), original);
}

#[test]
fn linked_worktree_first_migrates_closed_siblings_once_and_preserves_v5_backup() {
    let (dir, main, linked) = repo_with_linked_worktree();
    let path = dir.path().join("session.json");
    let main_key = crate::session::path_storage_key(&main);
    let linked_key = crate::session::path_storage_key(&linked);
    let legacy = serde_json::json!({
        "version": 5,
        "open_repos": [],
        "repo_history_modes": {main_key.clone(): "all_branches", linked_key.clone(): "first_parent"},
        "repo_sidebar_pinned_branches": {main_key.clone(): ["local:main"], linked_key.clone(): ["local:feature"]},
        "repo_sidebar_collapsed_items": {main_key: ["group:local:main"], linked_key: ["group:local:linked"]}
    });
    let original = serde_json::to_vec(&legacy).unwrap();
    fs::write(&path, &original).unwrap();
    let _path_guard = crate::session::push_test_session_file_path_override(Some(path.clone()));
    let (store, _events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let id = open_repo_and_wait(&store, &linked);
    let initial = preferences(&store, id);
    assert_eq!(initial.preferences.history_mode, HistoryMode::AllBranches);
    assert_eq!(
        initial.preferences.pinned_items,
        BTreeSet::from(["local:main".into(), "local:feature".into()])
    );
    assert_eq!(
        initial.preferences.collapsed_items,
        BTreeSet::from(["group:local:main".into()])
    );
    assert_eq!(
        store.snapshot().repos[0].history_state.history_scope,
        HistoryMode::AllBranches
    );
    assert_eq!(
        fs::read(path.with_file_name("session.json.v5.bak")).unwrap(),
        original
    );

    update(
        &store,
        id,
        RepositoryPreferenceUpdate::Pin {
            key: "local:main".into(),
            pinned: false,
        },
    );
    wait_until("unpin on disk", || {
        let file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        file["repository_preferences"][initial.key.storage_key()]["pinned_items"]
            == serde_json::json!(["local:feature"])
    });
    // A new authority simulates restarting the process. Old legacy pins remain
    // available for recovery but cannot resurrect a pin that was removed.
    let (reopened, _reopened_events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let main_id = open_repo_and_wait(&reopened, &main);
    assert_eq!(
        preferences(&reopened, main_id).preferences.pinned_items,
        BTreeSet::from(["local:feature".into()])
    );
    assert_eq!(
        fs::read(path.with_file_name("session.json.v5.bak")).unwrap(),
        original
    );
    drop(store);
    drop(reopened);
}

#[test]
fn stale_explorer_menus_preserve_independent_toggles_across_windows() {
    let (dir, main, linked) = repo_with_linked_worktree();
    let path = dir.path().join("session.json");
    let _path_guard = crate::session::push_test_session_file_path_override(Some(path.clone()));
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (first, _first_events) = AppStore::new_test(backend.clone());
    let first_id = open_repo_and_wait(&first, &main);
    let (second, _second_events) = AppStore::new_test_sharing_preferences(backend, &first);
    let second_id = open_repo_and_wait(&second, &linked);
    for (hidden, ignored) in [(true, false), (false, true), (false, false), (true, true)] {
        for hidden_first in [true, false] {
            first.dispatch(Msg::SetExplorerVisibility {
                repo_id: first_id,
                hidden: Some(hidden),
                ignored: Some(ignored),
            });
            wait_until("explorer menus share their initial flags", || {
                let first = preferences(&first, first_id);
                let second = preferences(&second, second_id);
                first.revision == second.revision
                    && first.preferences.show_hidden_files == hidden
                    && first.preferences.show_ignored_files == ignored
            });
            let initial_revision = preferences(&first, first_id).revision;
            // Both actions come from menus built before either toggle.
            let toggle_hidden = Msg::SetExplorerVisibility {
                repo_id: first_id,
                hidden: Some(!hidden),
                ignored: None,
            };
            let toggle_ignored = Msg::SetExplorerVisibility {
                repo_id: second_id,
                hidden: None,
                ignored: Some(!ignored),
            };
            let (earlier_store, earlier, later_store, later) = if hidden_first {
                (&first, toggle_hidden, &second, toggle_ignored)
            } else {
                (&second, toggle_ignored, &first, toggle_hidden)
            };
            earlier_store.dispatch(earlier);
            wait_until("first explorer toggle broadcast", || {
                preferences(&first, first_id).revision == initial_revision + 1
            });
            later_store.dispatch(later);
            wait_until("both explorer toggles broadcast", || {
                preferences(&first, first_id).revision == initial_revision + 2
                    && preferences(&second, second_id).revision == initial_revision + 2
            });
            for (store, id) in [(&first, first_id), (&second, second_id)] {
                let prefs = preferences(store, id);
                assert_eq!(prefs.preferences.show_hidden_files, !hidden);
                assert_eq!(prefs.preferences.show_ignored_files, !ignored);
                let state = store.snapshot();
                let browser = &state
                    .repos
                    .iter()
                    .find(|repo| repo.id == id)
                    .unwrap()
                    .file_browser;
                assert_eq!(browser.show_hidden, !hidden);
                assert_eq!(browser.show_ignored, !ignored);
            }
            wait_until("independent explorer toggles persisted", || {
                let file: serde_json::Value =
                    serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
                let prefs = &file["repository_preferences"]
                    [preferences(&first, first_id).key.storage_key()];
                prefs["show_hidden_files"] == !hidden && prefs["show_ignored_files"] == !ignored
            });
        }
    }
}

#[test]
fn preferences_sync_across_tabs_and_windows_without_lost_fields_or_clone_leakage() {
    let (dir, main, linked) = repo_with_linked_worktree();
    let path = dir.path().join("session.json");
    let _path_guard = crate::session::push_test_session_file_path_override(Some(path.clone()));
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let (first, _first_events) = AppStore::new_test(backend.clone());
    let main_id = open_repo_and_wait(&first, &main);
    // Switching tabs cancels unfinished loads, so wait for this reply while
    // the main worktree is still active.
    wait_until("main worktree HEAD before switching tabs", || {
        matches!(
            &first.snapshot().repos.iter().find(|repo| repo.id == main_id).unwrap().head_branch,
            Loadable::Ready(name) if name == "main"
        )
    });
    let linked_id = open_repo_and_wait(&first, &linked);
    let (second, _second_events) = AppStore::new_test_sharing_preferences(backend, &first);
    let second_id = open_repo_and_wait(&second, &linked);
    let clone = dir.path().join("clone");
    run_git(
        dir.path(),
        &[
            "clone",
            "-q",
            "--local",
            main.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let clone_id = open_repo_and_wait(&second, &clone);
    assert_ne!(
        preferences(&first, main_id).key,
        preferences(&second, clone_id).key
    );

    // Independent windows issue changes without waiting for each other's
    // snapshots. Field updates must preserve both pins and all list sorts.
    update(
        &first,
        main_id,
        RepositoryPreferenceUpdate::Pin {
            key: "local:dev".into(),
            pinned: true,
        },
    );
    update(
        &second,
        second_id,
        RepositoryPreferenceUpdate::Pin {
            key: "group:local:topic".into(),
            pinned: true,
        },
    );
    first.dispatch(Msg::SetHistoryScope {
        repo_id: linked_id,
        scope: HistoryMode::FirstParent,
    });
    update(
        &second,
        second_id,
        RepositoryPreferenceUpdate::CollapseItems {
            added: BTreeSet::from(["group:local:topic".into()]),
            removed: BTreeSet::new(),
        },
    );
    update(
        &first,
        linked_id,
        RepositoryPreferenceUpdate::ExplorerVisibility {
            hidden: Some(false),
            ignored: Some(true),
        },
    );
    let sorts = BTreeMap::from([
        (RepositoryListKind::CommitFiles, RepositoryFileSort::Edits),
        (
            RepositoryListKind::RangeFiles,
            RepositoryFileSort::PathDescending,
        ),
        (
            RepositoryListKind::WorktreeFiles,
            RepositoryFileSort::FileTypeAscending,
        ),
        (
            RepositoryListKind::CombinedUnstaged,
            RepositoryFileSort::EditSizeAscending,
        ),
        (
            RepositoryListKind::Untracked,
            RepositoryFileSort::PathDescending,
        ),
        (
            RepositoryListKind::Unstaged,
            RepositoryFileSort::FileTypeDescending,
        ),
        (
            RepositoryListKind::Staged,
            RepositoryFileSort::EditSizeDescending,
        ),
    ]);
    for (&list, &sort) in &sorts {
        update(
            &second,
            second_id,
            RepositoryPreferenceUpdate::FileSort { list, sort },
        );
    }
    let expected = SharedRepositoryPreferences {
        history_mode: HistoryMode::FirstParent,
        pinned_items: BTreeSet::from(["local:dev".into(), "group:local:topic".into()]),
        collapsed_items: BTreeSet::from(["group:local:topic".into()]),
        file_sorts: sorts,
        show_hidden_files: false,
        show_ignored_files: true,
    };
    wait_until("all tabs and windows share all preferences", || {
        [
            &preferences(&first, main_id),
            &preferences(&first, linked_id),
            &preferences(&second, second_id),
        ]
        .iter()
        .all(|snapshot| *snapshot.preferences == expected)
    });
    assert_eq!(
        *preferences(&second, clone_id).preferences,
        SharedRepositoryPreferences::default()
    );
    let snapshot = first.snapshot();
    let main_state = snapshot
        .repos
        .iter()
        .find(|repo| repo.id == main_id)
        .unwrap();
    let linked_state = snapshot
        .repos
        .iter()
        .find(|repo| repo.id == linked_id)
        .unwrap();
    assert_eq!(
        main_state.history_state.history_scope,
        HistoryMode::FirstParent
    );
    assert_eq!(
        linked_state.history_state.history_scope,
        HistoryMode::FirstParent
    );
    assert!(!main_state.file_browser.show_hidden && main_state.file_browser.show_ignored);
    wait_until("each worktree keeps its own HEAD", || {
        let state = first.snapshot();
        matches!(&state.repos.iter().find(|repo| repo.id == main_id).unwrap().head_branch, Loadable::Ready(name) if name == "main")
            && matches!(&state.repos.iter().find(|repo| repo.id == linked_id).unwrap().head_branch, Loadable::Ready(name) if name == "feature")
    });
    first.dispatch(Msg::SetHistoryAuthorFilter {
        repo_id: linked_id,
        author: Some("local filter".into()),
    });
    wait_until("worktree filter applied", || {
        first
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == linked_id)
            .unwrap()
            .history_state
            .history_author_filter
            .as_deref()
            == Some("local filter")
    });
    assert_eq!(
        second
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == second_id)
            .unwrap()
            .history_state
            .history_author_filter,
        None
    );
    assert_eq!(
        first
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == main_id)
            .unwrap()
            .history_state
            .history_author_filter,
        None
    );

    wait_until("shared preferences on disk", || {
        let file: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        serde_json::from_value::<SharedRepositoryPreferences>(
            file["repository_preferences"][preferences(&first, main_id).key.storage_key()].clone(),
        )
        .ok()
        .as_ref()
            == Some(&expected)
    });
    let (reopened, _reopened_events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    let id = open_repo_and_wait(&reopened, &linked);
    assert_eq!(*preferences(&reopened, id).preferences, expected);
    drop(first);
    drop(second);
    drop(reopened);
}

#[test]
fn same_history_selection_overrides_a_queued_worktree_choice_without_reloading() {
    let hub = crate::store::repository_preferences::PreferenceHub::new(None);
    let key = RepositoryKey::CommonDir(PathBuf::from("/repo/.git"));
    let backend_repo = gitcomet_core::test_support::UnconfiguredRepository::new("/repo");
    let (initial, persistence) = hub.initialize(key.clone(), Path::new("/repo"), &backend_repo);
    persistence.unwrap();
    let mut state = AppState::test_default();
    for (id, path) in [(RepoId(1), "/repo"), (RepoId(2), "/linked")] {
        let mut repo = RepoState::new_opening(
            id,
            RepoSpec {
                workdir: path.into(),
            },
        );
        repo.open = Loadable::Ready(());
        repo.shared_preferences = Some(initial.clone());
        repo.set_log(Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        })));
        state.repos.push(repo);
    }
    state.active_repo = Some(RepoId(1));
    let mut repos = FxHashMap::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(3);
    let first = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(1),
            scope: HistoryMode::FirstParent,
        },
    );
    let linked_log_rev = state.repos[1].log_rev;
    // Neither queued change has been published yet. The linked tab still
    // displays FullReachable, and explicitly choosing it must win last.
    let second = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetHistoryScope {
            repo_id: RepoId(2),
            scope: HistoryMode::FullReachable,
        },
    );
    assert!(matches!(
        second.as_slice(),
        [Effect::UpdateRepositoryPreferences {
            repo_id: RepoId(2),
            update: RepositoryPreferenceUpdate::HistoryMode(HistoryMode::FullReachable),
            ..
        }]
    ));
    assert_eq!(state.repos[1].log_rev, linked_log_rev);
    assert!(matches!(state.repos[1].log, Loadable::Ready(_)));
    for effect in first.iter().chain(&second) {
        if let Effect::UpdateRepositoryPreferences { key, update, .. } = effect {
            hub.update(key.clone(), update).unwrap();
        }
    }
    let latest = hub.latest(&key).unwrap();
    assert_eq!(latest.preferences.history_mode, HistoryMode::FullReachable);
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyRepositoryPreferences(latest),
    );
    assert!(state.repos.iter().all(|repo| {
        repo.history_state.history_scope == HistoryMode::FullReachable
            && repo
                .shared_preferences
                .as_ref()
                .unwrap()
                .preferences
                .history_mode
                == HistoryMode::FullReachable
    }));
}

#[test]
fn stale_preference_broadcasts_cannot_reset_navigation_or_newer_values() {
    let id = RepoId(1);
    let mut repo = RepoState::new_opening(
        id,
        RepoSpec {
            workdir: PathBuf::from("/repo"),
        },
    );
    let key = RepositoryKey::CommonDir(PathBuf::from("/repo/.git"));
    repo.common_dir = Some(PathBuf::from("/repo/.git").into());
    repo.history_state.history_author_filter = Some("mine".into());
    repo.history_state.selected_commit = Some(CommitId("selected".into()));
    let current = RepositoryPreferencesSnapshot {
        key,
        revision: 9,
        preferences: Arc::new(SharedRepositoryPreferences {
            history_mode: HistoryMode::FirstParent,
            ..Default::default()
        }),
    };
    repo.set_log_scope(HistoryMode::FirstParent);
    repo.shared_preferences = Some(current.clone());
    let mut state = AppState::test_default();
    state.repos.push(repo);
    state.active_repo = Some(id);
    let mut stale = current;
    stale.revision = 8;
    stale.preferences = Arc::new(SharedRepositoryPreferences::default());
    let effects = crate::store::reducer::reduce(
        &mut FxHashMap::default(),
        &std::sync::atomic::AtomicU64::new(2),
        &mut state,
        Msg::ApplyRepositoryPreferences(stale),
    );
    assert!(effects.is_empty());
    assert_eq!(
        state.repos[0].history_state.history_scope,
        HistoryMode::FirstParent
    );
    assert_eq!(
        state.repos[0]
            .history_state
            .history_author_filter
            .as_deref(),
        Some("mine")
    );
    assert_eq!(
        state.repos[0].history_state.selected_commit,
        Some(CommitId("selected".into()))
    );
}

#[test]
fn history_update_retires_inactive_walks_until_activation() {
    let id = RepoId(1);
    let key = RepositoryKey::CommonDir(PathBuf::from("/repo/.git"));
    let mut repo = RepoState::new_opening(
        id,
        RepoSpec {
            workdir: PathBuf::from("/repo"),
        },
    );
    repo.open = Loadable::Ready(());
    repo.shared_preferences = Some(RepositoryPreferencesSnapshot {
        key: key.clone(),
        revision: 1,
        preferences: Arc::new(SharedRepositoryPreferences::default()),
    });
    let seq = repo
        .loads_in_flight
        .request_log(crate::model::PendingLogLoad {
            scope: HistoryMode::default(),
            author: None,
            limit: 100,
            cursor: None,
        })
        .unwrap();
    repo.set_log(Loadable::Loading);
    let mut state = AppState::test_default();
    state.repos.push(repo);
    let mut repos = FxHashMap::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(2);
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyRepositoryPreferences(RepositoryPreferencesSnapshot {
            key,
            revision: 2,
            preferences: Arc::new(SharedRepositoryPreferences {
                history_mode: HistoryMode::FirstParent,
                ..Default::default()
            }),
        }),
    );
    assert!(effects.is_empty(), "inactive history reload is deferred");
    assert!(!state.repos[0].loads_in_flight.is_active_log_reply(seq));
    assert!(matches!(state.repos[0].log, Loadable::NotLoaded));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::Internal(crate::msg::InternalMsg::LogLoaded {
            repo_id: id,
            seq,
            scope: HistoryMode::default(),
            cursor: None,
            result: Ok(gitcomet_core::services::HistoryReadResult::Unchanged),
        }),
    );
    assert!(effects.is_empty());
    assert!(matches!(state.repos[0].log, Loadable::NotLoaded));
    let effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::SetActiveRepo { repo_id: id },
    );
    assert!(effects.iter().any(|effect| matches!(
        effect,
        Effect::LoadLog {
            scope: HistoryMode::FirstParent,
            ..
        }
    )));
}
