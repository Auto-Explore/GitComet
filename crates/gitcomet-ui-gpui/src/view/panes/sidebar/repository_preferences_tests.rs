use super::*;
use crate::view::rows::{CommitFileSort, FileListId};
use crate::view::test_support::{self, TestBackend};
use gitcomet_state::model::{
    RepositoryFileSort, RepositoryKey, RepositoryListKind, RepositoryPreferencesSnapshot,
    SharedRepositoryPreferences,
};

fn wait_for_preferences(store: &AppStore, ready: impl Fn(&SharedRepositoryPreferences) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let state = store.snapshot();
        if state.repos[0]
            .shared_preferences
            .as_ref()
            .is_some_and(|snapshot| ready(&snapshot.preferences))
        {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "preferences did not update"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[gpui::test]
fn final_sort_selection_survives_an_unchanged_pane_snapshot(cx: &mut gpui::TestAppContext) {
    use gitcomet_state::model::RepositoryPreferenceUpdate;

    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let state = long_list_tests::branch_fixture(1);
    let repo_id = state.repos[0].id;
    store.replace_snapshot_for_test(state);
    store.dispatch(Msg::UpdateRepositoryPreference {
        repo_id,
        update: RepositoryPreferenceUpdate::FileSort {
            list: RepositoryListKind::CommitFiles,
            sort: RepositoryFileSort::PathAscending,
        },
    });
    wait_for_preferences(&store, |prefs| {
        prefs.file_sorts.get(&RepositoryListKind::CommitFiles)
            == Some(&RepositoryFileSort::PathAscending)
    });
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    cx.update(|_, app| {
        view.update(app, |view, cx| test_support::sync_store_snapshot(view, cx));
    });
    test_support::redraw(cx);

    let lists = [
        FileListId::CommitFiles,
        FileListId::RangeFiles,
        FileListId::WorktreeFiles,
        FileListId::Status(StatusSection::CombinedUnstaged),
        FileListId::Status(StatusSection::Untracked),
        FileListId::Status(StatusSection::Unstaged),
        FileListId::Status(StatusSection::Staged),
    ];
    cx.update(|_, app| {
        let details = view.read(app).details_pane.clone();
        details.update(app, |pane, cx| {
            for list in lists {
                assert_eq!(pane.file_list_sort_for(list), CommitFileSort::PathAscending);
                // Both choices happen in one UI update, before a preference
                // broadcast can replace this pane's ascending snapshot.
                for sort in [
                    CommitFileSort::PathDescending,
                    CommitFileSort::PathAscending,
                ] {
                    match list {
                        FileListId::CommitFiles => pane.set_commit_file_sort(sort, cx),
                        FileListId::Status(section) => pane.set_status_file_sort(section, sort, cx),
                        _ => pane.set_file_list_sort(list, sort, cx),
                    }
                }
                assert_eq!(pane.file_list_sort_for(list), CommitFileSort::PathAscending);
            }
        });
    });
    // This marker follows all sort effects on the shared persistence queue.
    // Wait for it so an already-ascending snapshot cannot pass the assertion.
    store.dispatch(Msg::UpdateRepositoryPreference {
        repo_id,
        update: RepositoryPreferenceUpdate::Pin {
            key: "local:sorts-complete".into(),
            pinned: true,
        },
    });
    wait_for_preferences(&store, |prefs| {
        prefs.pinned_items.contains("local:sorts-complete")
    });
    let snapshot = store.snapshot();
    let preferences = &snapshot.repos[0]
        .shared_preferences
        .as_ref()
        .unwrap()
        .preferences;
    for list in lists {
        assert_eq!(
            preferences.file_sorts.get(&list.preference_key()),
            Some(&RepositoryFileSort::PathAscending),
            "the final choice for {list:?} was discarded"
        );
    }
}

#[gpui::test]
fn shared_snapshots_update_sidebar_menus_and_every_file_list(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = long_list_tests::branch_fixture(10);
    let first_id = state.repos[0].id;
    let second_id = RepoId(first_id.0 + 1);
    let key = RepositoryKey::CommonDir(PathBuf::from("/tmp/shared-repo/.git"));
    let mut second = state.repos[0].clone();
    second.id = second_id;
    second.spec.workdir = PathBuf::from("/tmp/shared-repo-linked");
    Arc::make_mut(&mut state).repos.push(second);
    for repo in &mut Arc::make_mut(&mut state).repos {
        repo.shared_preferences = Some(RepositoryPreferencesSnapshot {
            key: key.clone(),
            revision: 1,
            preferences: Arc::new(SharedRepositoryPreferences::default()),
        });
    }
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state.clone(), cx);
            view.set_sidebar_collapsed(false, cx);
        })
    });
    test_support::redraw(cx);
    let default_layout = cx.update(|_, app| {
        let details = view.read(app).details_pane.clone();
        details.update(app, |pane, cx| {
            let layout = pane.file_list_layout_for(first_id, FileListId::CommitFiles);
            pane.toggle_file_list_layout(first_id, FileListId::CommitFiles, cx);
            layout
        })
    });
    let pin = branch_sidebar::branch_pin_storage_key(BranchSection::Local, "shared/topic-000001");
    let collapsed = branch_sidebar::remote_header_storage_key("origin");
    let lists = [
        FileListId::CommitFiles,
        FileListId::RangeFiles,
        FileListId::WorktreeFiles,
        FileListId::Status(StatusSection::CombinedUnstaged),
        FileListId::Status(StatusSection::Untracked),
        FileListId::Status(StatusSection::Unstaged),
        FileListId::Status(StatusSection::Staged),
    ];
    let prefs = Arc::new(SharedRepositoryPreferences {
        pinned_items: BTreeSet::from([pin.clone()]),
        collapsed_items: BTreeSet::from([collapsed.clone()]),
        file_sorts: lists
            .iter()
            .map(|list| (list.preference_key(), RepositoryFileSort::PathDescending))
            .collect(),
        ..Default::default()
    });
    for repo in &mut Arc::make_mut(&mut state).repos {
        repo.shared_preferences = Some(RepositoryPreferencesSnapshot {
            key: key.clone(),
            revision: 2,
            preferences: prefs.clone(),
        });
        repo.branch_sidebar_rev += 1;
    }
    for active in [first_id, second_id] {
        Arc::make_mut(&mut state).active_repo = Some(active);
        cx.update(|_, app| {
            view.update(app, |view, cx| {
                view.store.replace_snapshot_for_test(state.clone());
                test_support::push_test_state(view, state.clone(), cx);
            })
        });
        test_support::redraw(cx);
        cx.update(|_, app| {
            let sidebar = view.read(app).sidebar_pane.clone();
            sidebar.update(app, |pane, _| {
                let repo = pane.active_repo().unwrap();
                assert!(pane.sidebar_pinned_branches_by_repo[&repo.spec.workdir].contains(&pin));
                assert!(
                    pane.sidebar_collapsed_items_by_repo[&repo.spec.workdir].contains(&collapsed)
                );
                assert!(
                    !pane
                        .branch_sidebar_presentation_cached()
                        .unwrap()
                        .pins
                        .is_empty()
                );
            });
            let root = view.read(app);
            assert!(
                root.popover_host
                    .read(app)
                    .is_sidebar_item_pinned(active, &pin)
            );
            assert!(
                root.popover_host
                    .read(app)
                    .sidebar_collapse_key_is_collapsed(active, &collapsed)
            );
            for list in lists {
                assert_eq!(
                    root.details_pane.read(app).file_list_sort_for(list),
                    CommitFileSort::PathDescending
                );
            }
            assert_eq!(
                root.details_pane
                    .read(app)
                    .file_list_layout_for(active, FileListId::CommitFiles),
                if active == first_id {
                    default_layout.next()
                } else {
                    default_layout
                }
            );
        });
    }

    // A later shared update must also remove pins from mirrored maps, and a
    // sort for one list must leave all other list choices intact.
    let mut next = (*prefs).clone();
    next.pinned_items.clear();
    next.file_sorts
        .insert(RepositoryListKind::CommitFiles, RepositoryFileSort::Edits);
    for repo in &mut Arc::make_mut(&mut state).repos {
        repo.shared_preferences = Some(RepositoryPreferencesSnapshot {
            key: key.clone(),
            revision: 3,
            preferences: Arc::new(next.clone()),
        });
        repo.branch_sidebar_rev += 1;
    }
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.store.replace_snapshot_for_test(state.clone());
            test_support::push_test_state(view, state, cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        let root = view.read(app);
        assert!(
            !root
                .popover_host
                .read(app)
                .is_sidebar_item_pinned(second_id, &pin)
        );
        assert_eq!(
            root.details_pane
                .read(app)
                .file_list_sort_for(FileListId::CommitFiles),
            CommitFileSort::Edits
        );
        assert_eq!(
            root.details_pane
                .read(app)
                .file_list_sort_for(FileListId::RangeFiles),
            CommitFileSort::PathDescending
        );
        let sidebar = root.sidebar_pane.clone();
        sidebar.update(app, |pane, _| {
            assert!(
                pane.branch_sidebar_presentation_cached()
                    .unwrap()
                    .pins
                    .is_empty()
            )
        });
    });
}
