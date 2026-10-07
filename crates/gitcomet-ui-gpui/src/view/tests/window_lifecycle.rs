//! Window and workspace lifecycle: drops, tabs, splash, close and quit.

use super::*;

fn assert_external_drag_cleared(view: &GitCometView, app: &gpui::App) {
    assert!(
        view.external_drag_paths.is_none(),
        "clear the dragged paths"
    );
    assert!(
        view.external_drag_payload.is_none(),
        "clear the classified payload"
    );
    assert!(!view.external_drag_drop_pending, "clear the pending drop");
    assert!(!test_support::repo_external_folder_drag_active(view, app));
    assert!(!test_support::repo_external_folder_drag_hovered(view, app));
}

#[gpui::test]
fn folder_drag_marks_repository_bar_available_and_tracks_hover_emphasis(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_state = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    install_repo_tab_test_state_with_count(&store_for_state, &view, cx, RepoId(1), 1);

    let folder = tempfile::tempdir().expect("create dropped folder");
    let outside_bar = cx.update(|window, _app| {
        let viewport = window.viewport_size();
        gpui::point(viewport.width / 2.0, viewport.height / 2.0)
    });
    cx.update(|window, app| {
        let _ = window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Entered {
                position: outside_bar,
                paths: gpui::ExternalPaths([folder.path().to_path_buf()].into_iter().collect()),
            }),
            app,
        );
        assert!(test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
        assert!(!test_support::repo_external_folder_drag_hovered(
            view.read(app),
            app
        ));
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert!(test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
        assert!(!test_support::repo_external_folder_drag_hovered(
            view.read(app),
            app
        ));
    });

    let bar_point = cx
        .debug_bounds("repo_external_folder_drop_target")
        .expect("repository bar drop target should be rendered")
        .center();
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Pending {
            position: bar_point,
        },
    );
    cx.update(|_window, app| {
        assert!(test_support::repo_external_folder_drag_hovered(
            view.read(app),
            app
        ));
    });

    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Pending {
            position: outside_bar,
        },
    );
    cx.update(|_window, app| {
        assert!(!test_support::repo_external_folder_drag_hovered(
            view.read(app),
            app
        ));
    });

    let classification_seq =
        cx.update(|_window, app| test_support::external_drag_classification_seq(view.read(app)));
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position: outside_bar,
            paths: gpui::ExternalPaths([folder.path().to_path_buf()].into_iter().collect()),
        },
    );
    cx.update(|_window, app| {
        assert_eq!(
            test_support::external_drag_classification_seq(view.read(app)),
            classification_seq,
            "repeated move events for one payload must reuse its background classification"
        );
    });

    dispatch_file_drop(cx, gpui::FileDropEvent::Exited);
    test_support::redraw(cx);
    cx.update(|_window, app| {
        assert_external_drag_cleared(view.read(app), app);
    });
}

fn check_folder_drop_clears_highlight(
    cx: &mut gpui::TestAppContext,
    on_home: bool,
    classify_before_drop: bool,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let initial = tempfile::tempdir().expect("create the initial repository");
    if !on_home {
        let repo_id = RepoId(100);
        let path = initial.path().canonicalize().unwrap();
        let mut repo = RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: path.clone(),
            },
        );
        repo.open = Loadable::Ready(());
        store.insert_repo_for_test(
            repo_id,
            Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                path,
            )),
        );
        store.replace_snapshot_for_test(Arc::new(AppState {
            repos: vec![repo],
            active_repo: Some(repo_id),
            ..AppState::test_default()
        }));
    }
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    test_support::redraw(cx);
    let folder = tempfile::tempdir().expect("create dropped folder");
    let path = folder.path().canonicalize().unwrap();
    let target = if on_home {
        "repository_entry_screen"
    } else {
        "repo_external_folder_drop_target"
    };
    let position = cx
        .debug_bounds(target)
        .expect("render drop target")
        .center();
    let entered = gpui::FileDropEvent::Entered {
        position,
        paths: gpui::ExternalPaths(vec![path.clone()].into()),
    };
    if classify_before_drop {
        dispatch_file_drop(cx, entered.clone());
        pump_until(cx, "classify the folder before dropping", |cx| {
            cx.update(|_, app| view.read(app).external_drag_payload.is_some())
        });
    }
    cx.update(|window, app| {
        if !classify_before_drop {
            let _ = window.dispatch_event(gpui::PlatformInput::FileDrop(entered), app);
            assert!(view.read(app).external_drag_payload.is_none());
        }
        assert!(test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
        let _ = window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Submit { position }),
            app,
        );
        assert!(
            !test_support::repo_external_folder_drag_active(view.read(app), app),
            "clear the highlight immediately, before repository loading or classification completes"
        );
        assert!(!test_support::repo_external_folder_drag_hovered(
            view.read(app),
            app
        ));
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    pump_until(cx, "finish the dropped folder's classification", |cx| {
        cx.update(|_, app| view.read(app).external_drag_paths.is_none())
    });
    // Apply the loaded snapshot deterministically: this visual transition does
    // not need to wait for the process-wide repository-load worker pool.
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            assert_external_drag_cleared(view, cx);
            let repo_id = RepoId(1);
            let mut repo = RepoState::new_opening(
                repo_id,
                RepoSpec {
                    workdir: path.clone(),
                },
            );
            repo.open = Loadable::Ready(());
            let mut snapshot = view.state.as_ref().clone();
            snapshot.repos.retain(|repo| repo.id != repo_id);
            snapshot.repos.push(repo);
            snapshot.active_repo = Some(repo_id);
            test_support::apply_state_snapshot_for_test(view, Arc::new(snapshot), cx);
        })
    });
    test_support::redraw(cx);
    cx.update(|_, app| {
        assert_eq!(
            view.read(app).state.repos.len(),
            if on_home { 1 } else { 2 }
        )
    });
    assert!(
        cx.debug_bounds("repo_external_folder_drop_target")
            .is_some()
    );
    cx.update(|_, app| assert_external_drag_cleared(view.read(app), app));

    // A fresh drag of the same folder must highlight again and clear on exit.
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position,
            paths: gpui::ExternalPaths(vec![path].into()),
        },
    );
    cx.update(|_, app| {
        assert!(test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ))
    });
    dispatch_file_drop(cx, gpui::FileDropEvent::Exited);
    test_support::redraw(cx);
    cx.update(|_, app| assert_external_drag_cleared(view.read(app), app));
}

#[gpui::test]
fn home_folder_drop_clears_highlight_before_and_after_classification(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for classified in [true, false] {
        check_folder_drop_clears_highlight(cx, true, classified);
    }
}

#[gpui::test]
fn repository_bar_folder_drop_clears_highlight_before_and_after_classification(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for classified in [true, false] {
        check_folder_drop_clears_highlight(cx, false, classified);
    }
}

#[gpui::test]
fn home_folder_drag_exit_ignores_late_classification(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);
    let folder = tempfile::tempdir().unwrap();
    let position = cx.debug_bounds("repository_entry_screen").unwrap().center();
    cx.update(|window, app| {
        let _ = window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Entered {
                position,
                paths: gpui::ExternalPaths(vec![folder.path().to_path_buf()].into()),
            }),
            app,
        );
        let _ = window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Exited),
            app,
        );
        let _ = window.draw(app);
        assert_external_drag_cleared(view.read(app), app);
    });
    cx.run_until_parked();
    test_support::redraw(cx);
    cx.update(|_, app| assert_external_drag_cleared(view.read(app), app));
}

#[gpui::test]
fn dropping_one_folder_on_repository_bar_dispatches_external_repo_open(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingFailingBackend {
        opened: Arc::clone(&opened),
    });
    let (store, events) = AppStore::new_test(backend);
    let store_for_state = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    install_repo_tab_test_state_with_count(&store_for_state, &view, cx, RepoId(1), 1);

    let folder = tempfile::tempdir().expect("create dropped folder");
    let drop_point = cx
        .debug_bounds("repo_external_folder_drop_target")
        .expect("repository bar drop target should be rendered")
        .center();
    // Submit in the same UI turn as Entered. The background metadata probe
    // cannot apply its result until this update completes, so this exercises
    // the pending-drop path rather than relying on a fast local filesystem.
    cx.update(|window, app| {
        let _ = window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Entered {
                position: drop_point,
                paths: gpui::ExternalPaths([folder.path().to_path_buf()].into_iter().collect()),
            }),
            app,
        );
        let _ = window.dispatch_event(
            gpui::PlatformInput::FileDrop(gpui::FileDropEvent::Submit {
                position: drop_point,
            }),
            app,
        );
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    // The store canonicalizes every workdir it opens, so compare against the
    // resolved path: on macOS the temp dir arrives as `/var/...` and comes back
    // as `/private/var/...`.
    let dropped = canonicalize_or_original(folder.path().to_path_buf());
    pump_until(cx, "folder drop to dispatch a repository open", |_| {
        store_for_state
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == dropped)
            || !opened.lock().expect("recording backend lock").is_empty()
    });
    let opened = opened.lock().expect("recording backend lock");
    assert!(
        opened.is_empty() || opened.as_slice() == [dropped.clone()],
        "the repository-load effect must receive the dropped folder, got {opened:?}"
    );
    cx.update(|_window, app| {
        assert!(!test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
    });
}

#[gpui::test]
fn review_regression_lifecycle_provisional_external_drop_is_not_added_to_a_workspace(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(BlockingFailingBackend {
        release: Arc::clone(&release),
    });
    let (store, events) = AppStore::new_test(backend);
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.cx
        .update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

    let dropped = std::env::temp_dir().join("gitcomet-provisional-invalid-drop");
    store.dispatch(Msg::OpenRepoFromExternalDrop(dropped.clone()));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = store.snapshot();
        if snapshot
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == dropped)
            && gitcomet_state::session::snapshot_repos_from_state(snapshot.as_ref())
                .open_repos
                .is_empty()
        {
            break;
        }
        if Instant::now() >= deadline {
            let (released, wake) = release.as_ref();
            *released.lock().expect("release blocking backend") = true;
            wake.notify_all();
            panic!("timed out waiting for the provisional external-drop tab");
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    let provisional_snapshot = store.snapshot();
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            test_support::apply_state_snapshot_for_test(view, provisional_snapshot, cx);
        });
    });
    let persisted_paths = cx.cx.update(|app| {
        crate::workspaces::workspaces(app)
            .into_iter()
            .flat_map(|group| group.repositories)
            .collect::<Vec<_>>()
    });
    let (released, wake) = release.as_ref();
    *released.lock().expect("release blocking backend") = true;
    wake.notify_all();

    assert!(
        !persisted_paths.contains(&dropped),
        "an unvalidated external drop must not become durable group membership"
    );
}

#[gpui::test]
fn review_regression_followup_window_bounds_do_not_schedule_global_settings_persistence(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let before = cx.update(|_window, app| view.read(app).ui_settings_persist_requests_for_test);

    cx.simulate_resize(gpui::size(gpui::px(913.0), gpui::px(677.0)));

    let after = cx.update(|_window, app| view.read(app).ui_settings_persist_requests_for_test);
    assert_eq!(
        after, before,
        "a window-local bounds update must not enqueue a stale full UiSettings snapshot"
    );
}

#[gpui::test]
fn review_regression_confirmed_focused_mergetool_bounds_persist_legacy_size(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let config = GitCometViewConfig {
        view_mode: GitCometViewMode::FocusedMergetool,
        focused_mergetool: Some(FocusedMergetoolViewConfig {
            repo_path: PathBuf::from("/tmp/gitcomet-focused-bounds-repo"),
            conflicted_file_path: PathBuf::from("conflicted.txt"),
            labels: FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            },
        }),
        ..GitCometViewConfig::default()
    };
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    });
    let before = cx.update(|_window, app| view.read(app).ui_settings_persist_requests_for_test);

    cx.simulate_resize(gpui::size(gpui::px(911.0), gpui::px(673.0)));

    let after = cx.update(|_window, app| view.read(app).ui_settings_persist_requests_for_test);
    assert!(
        after > before,
        "focused mergetool bounds must persist through the legacy UiSettings dimensions"
    );
}

#[gpui::test]
fn repository_bar_ignores_files_and_drops_outside_the_bar(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingFailingBackend {
        opened: Arc::clone(&opened),
    });
    let (store, events) = AppStore::new_test(backend);
    let store_for_state = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    install_repo_tab_test_state_with_count(&store_for_state, &view, cx, RepoId(1), 1);

    let folder = tempfile::tempdir().expect("create dropped folder");
    let file = tempfile::NamedTempFile::new().expect("create dropped file");
    let bar_point = cx
        .debug_bounds("repo_external_folder_drop_target")
        .expect("repository bar drop target should be rendered")
        .center();
    let outside_bar = cx.update(|window, _app| {
        let viewport = window.viewport_size();
        gpui::point(viewport.width / 2.0, viewport.height / 2.0)
    });

    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position: bar_point,
            paths: gpui::ExternalPaths([file.path().to_path_buf()].into_iter().collect()),
        },
    );
    cx.update(|_window, app| {
        assert!(!test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
    });
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Submit {
            position: bar_point,
        },
    );

    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position: outside_bar,
            paths: gpui::ExternalPaths([folder.path().to_path_buf()].into_iter().collect()),
        },
    );
    cx.update(|_window, app| {
        assert!(test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
    });
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Submit {
            position: outside_bar,
        },
    );
    pump_for(cx, Duration::from_millis(100));

    assert!(
        opened.lock().expect("recording backend lock").is_empty(),
        "file-only payloads and drops outside the repository bar must remain unhandled"
    );
    cx.update(|_window, app| {
        assert!(!test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
    });
}

/// Each folder in a drop opens on its own; files riding along are skipped.
#[gpui::test]
fn dropping_multiple_folders_on_repository_bar_opens_each(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingFailingBackend {
        opened: Arc::clone(&opened),
    });
    let (store, events) = AppStore::new_test(backend);
    // Seeded clear of the store's id allocator, which starts at 1: a shared id
    // would make the drops' tabs alias the seeded one.
    let seeded_id = RepoId(100);
    let seeded_path = PathBuf::from("/tmp/multi-folder-drop-seeded");
    let mut seeded = RepoState::new_opening(
        seeded_id,
        RepoSpec {
            workdir: seeded_path.clone(),
        },
    );
    seeded.open = Loadable::Ready(());
    store.insert_repo_for_test(
        seeded_id,
        Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
            seeded_path,
        )),
    );
    store.replace_snapshot_for_test(Arc::new(AppState {
        repos: vec![seeded],
        active_repo: Some(seeded_id),
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    }));
    let store_for_state = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);

    let folder_a = tempfile::tempdir().expect("create first dropped folder");
    let folder_b = tempfile::tempdir().expect("create second dropped folder");
    let file = tempfile::NamedTempFile::new().expect("create dropped file");
    let bar_point = cx
        .debug_bounds("repo_external_folder_drop_target")
        .expect("repository bar drop target should be rendered")
        .center();
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position: bar_point,
            paths: gpui::ExternalPaths(
                [
                    folder_a.path().to_path_buf(),
                    file.path().to_path_buf(),
                    folder_b.path().to_path_buf(),
                ]
                .into_iter()
                .collect(),
            ),
        },
    );
    pump_until(cx, "classify the mixed payload", |cx| {
        cx.update(|_, app| view.read(app).external_drag_payload.is_some())
    });
    cx.update(|_window, app| {
        assert!(
            test_support::repo_external_folder_drag_active(view.read(app), app),
            "a payload with a folder in it highlights the bar"
        );
    });
    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Submit {
            position: bar_point,
        },
    );

    // The shared repository-load pool can be saturated by other tests, so a
    // folder counts as handled once the store has it in any form: a pending
    // tab, a backend open, or the warning its failed open left behind.
    let reached_store = |path: &Path| {
        let snapshot = store_for_state.snapshot();
        let shown = path.display().to_string();
        snapshot.repos.iter().any(|repo| repo.spec.workdir == path)
            || opened
                .lock()
                .expect("recording backend lock")
                .iter()
                .any(|opened| opened == path)
            || snapshot
                .notifications
                .iter()
                .any(|notification| notification.message.contains(&shown))
    };
    let folders =
        [folder_a.path(), folder_b.path()].map(|path| canonicalize_or_original(path.to_path_buf()));
    pump_until(cx, "both dropped folders to reach the store", |_| {
        folders.iter().all(|folder| reached_store(folder))
    });
    let file_path = canonicalize_or_original(file.path().to_path_buf());
    assert!(!reached_store(&file_path), "a dropped file is ignored");
    cx.update(|_window, app| {
        assert_external_drag_cleared(view.read(app), app);
    });
}

#[test]
fn restore_session_mode_does_not_seed_empty_session_from_initial_repository() {
    assert!(!should_seed_initial_repository_from_session(
        GitCometViewMode::Normal,
        Some(Path::new("/repo")),
        InitialRepositoryLaunchMode::RestoreSession,
        false,
    ));
}

#[test]
fn restore_session_mode_keeps_initial_repository_when_session_has_saved_repos() {
    assert!(should_seed_initial_repository_from_session(
        GitCometViewMode::Normal,
        Some(Path::new("/repo")),
        InitialRepositoryLaunchMode::RestoreSession,
        true,
    ));
}

#[test]
fn explicit_initial_repository_mode_seeds_empty_session() {
    assert!(should_seed_initial_repository_from_session(
        GitCometViewMode::Normal,
        Some(Path::new("/repo")),
        InitialRepositoryLaunchMode::OpenExplicitly,
        false,
    ));
}

#[gpui::test]
fn restored_active_repository_is_not_reopened_before_view_snapshot_arrives(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_state = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.run_until_parked();

    let path = PathBuf::from("/repos/restored-before-view-sync");
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: path.clone(),
        },
    );
    repo.open = Loadable::Ready(());
    let restored = Arc::new(AppState {
        repos: vec![repo],
        active_repo: Some(RepoId(1)),
        git_runtime: available_git_runtime_state(),
        ..AppState::test_default()
    });

    cx.update(|_window, app| {
        store_for_state.replace_snapshot_for_test(Arc::clone(&restored));
        view.update(app, |this, cx| {
            assert!(this.state.repos.is_empty(), "view is still stale");
            this.activate_or_open_repo_path(path, cx);
            assert!(
                this.pending_repo_open_reservations.is_empty(),
                "selecting the restored active tab must not reserve another open"
            );
            assert!(this.deferred_repo_bootstrap.is_none());
        });
    });
    assert!(Arc::ptr_eq(&store_for_state.snapshot(), &restored));
}

#[test]
fn splash_backdrop_embedded_png_decodes() {
    for is_dark in [true, false] {
        let backdrop = super::super::splash::load_splash_backdrop_image(is_dark);
        let decoded = image::load_from_memory_with_format(&backdrop.bytes, image::ImageFormat::Png)
            .expect("expected splash backdrop to decode from embedded PNG bytes");
        assert!(decoded.width() > 0 && decoded.height() > 0);
    }
    assert_ne!(
        super::super::splash::load_splash_backdrop_image(true).id(),
        super::super::splash::load_splash_backdrop_image(false).id(),
        "dark and light themes must have different backdrop artwork"
    );
}

fn focused_bootstrap(
    repo_path: PathBuf,
    conflicted_file_path: PathBuf,
) -> FocusedMergetoolBootstrap {
    FocusedMergetoolBootstrap::from_view_config(FocusedMergetoolViewConfig {
        repo_path,
        conflicted_file_path,
        labels: FocusedMergetoolLabels {
            local: "LOCAL".to_string(),
            remote: "REMOTE".to_string(),
            base: "BASE".to_string(),
        },
    })
}

#[test]
fn focused_mergetool_target_path_prefers_repo_relative_path() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let target = focused_mergetool_target_path(&repo, &repo.join("src/conflict.txt"));
    assert_eq!(target, PathBuf::from("src/conflict.txt"));
}

#[test]
fn focused_mergetool_bootstrap_requests_open_repo_when_missing() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let state = AppState::test_default();

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::OpenRepo(repo))
    );
}

#[test]
fn focused_mergetool_bootstrap_selects_worktree_diff_target() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    state.repos.push(open_repo_state_with_workdir(
        repo.to_str().expect("test path should be unicode"),
    ));

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::SelectConflictDiff {
            repo_id: RepoId(1),
            path: PathBuf::from("src/conflict.txt"),
        })
    );
}

#[test]
fn focused_mergetool_bootstrap_loads_conflict_file_after_diff_target() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    let mut repo_state =
        open_repo_state_with_workdir(repo.to_str().expect("test path should be unicode"));
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("src/conflict.txt"),
        DiffArea::Unstaged,
    ));
    state.repos.push(repo_state);

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::LoadConflictFile {
            repo_id: RepoId(1),
            path: PathBuf::from("src/conflict.txt"),
        })
    );
}

#[test]
fn focused_mergetool_bootstrap_completes_after_conflict_file_target_set() {
    let repo = normalize_bootstrap_repo_path(PathBuf::from("/repo"));
    let bootstrap = focused_bootstrap(repo.clone(), repo.join("src/conflict.txt"));
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    let mut repo_state =
        open_repo_state_with_workdir(repo.to_str().expect("test path should be unicode"));
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        PathBuf::from("src/conflict.txt"),
        DiffArea::Unstaged,
    ));
    repo_state.conflict_state.conflict_file_path = Some(PathBuf::from("src/conflict.txt"));
    repo_state.conflict_state.conflict_file = Loadable::Loading;
    state.repos.push(repo_state);

    assert_eq!(
        focused_mergetool_bootstrap_action(&state, &bootstrap),
        Some(FocusedMergetoolBootstrapAction::Complete)
    );
}

#[test]
fn focused_mergetool_mode_hides_full_chrome() {
    assert!(renders_full_chrome(GitCometViewMode::Normal));
    assert!(!renders_full_chrome(GitCometViewMode::FocusedMergetool));
}

#[test]
fn merge_view_target_requires_an_unstaged_conflict() {
    let normal = state_with_active_diff("src/normal.rs", FileStatusKind::Modified);
    let merge = state_with_active_diff("src/conflict.rs", FileStatusKind::Conflicted);

    assert!(active_merge_view_target(&normal).is_none());
    assert!(active_merge_view_target(&merge).is_some());
}

#[test]
fn repository_entry_interstitial_helpers_distinguish_loading_and_splash() {
    assert!(repository_entry_interstitial_active(
        GitCometViewMode::Normal,
        false
    ));
    assert!(should_show_startup_repository_loading_screen(
        GitCometViewMode::Normal,
        false,
        true
    ));
    assert!(!should_show_home_screen(
        GitCometViewMode::Normal,
        false,
        true
    ));
    assert!(should_show_home_screen(
        GitCometViewMode::Normal,
        false,
        false
    ));
    assert!(!repository_entry_interstitial_active(
        GitCometViewMode::Normal,
        true
    ));
    assert!(titlebar_repo_tab_actions_enabled(
        GitCometViewMode::FocusedMergetool,
        false
    ));
    assert!(!titlebar_repo_tab_actions_enabled(
        GitCometViewMode::Normal,
        false
    ));
}

#[gpui::test]
fn home_screen_renders_when_no_repositories_are_open(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for selector in [
        "repository_entry_screen",
        "home_title",
        "home_tagline",
        "home_open_repo_action",
        "home_clone_repo_action",
        "home_init_repo_action",
        "home_search",
        "home_workspaces_list",
        "home_recent_list",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "expected {selector} on the Home page"
        );
    }
    assert!(
        cx.debug_bounds("splash_headline").is_none(),
        "the marketing headline is gone"
    );
    let workspaces = cx
        .debug_bounds("home_workspaces_list")
        .expect("workspaces list");
    let recent = cx.debug_bounds("home_recent_list").expect("recent list");
    assert!(
        workspaces.right() <= recent.left() && (workspaces.top() - recent.top()).abs() < px(1.0),
        "the two lists sit side by side: {workspaces:?} {recent:?}"
    );

    #[cfg(not(target_os = "macos"))]
    assert!(
        cx.debug_bounds("app_menu").is_some(),
        "settings and quit stay reachable from Home through the app menu"
    );

    let home_active = cx.update(|_window, app| view.read(app).is_home_screen_active());
    assert!(home_active, "expected the Home page to be active");
}

#[gpui::test]
fn git_unavailable_splash_renders_open_settings_call_to_action(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let next = Arc::new(AppState {
        git_runtime: unavailable_git_runtime_state(),
        ..AppState::test_default()
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
        let _ = window.draw(app);
    });

    cx.debug_bounds("git_unavailable_screen")
        .expect("expected git unavailable splash screen");
    cx.debug_bounds("git_unavailable_status_icon")
        .expect("expected git unavailable status icon");
    cx.debug_bounds("git_unavailable_open_settings")
        .expect("expected open settings call to action");
    assert!(
        cx.debug_bounds("splash_open_repo_action").is_none(),
        "expected repository entry actions to be hidden while Git is unavailable"
    );

    cx.update(|_window, app| {
        assert!(view.read(app).is_home_screen_active());
        assert!(view.read(app).blocks_non_repository_actions());
    });
}

/// Repositories deferred until Git recovers keep bootstrap pending (so the
/// workspace membership survives), but must not hide the unavailable screen.
#[gpui::test]
fn review_regression_deferred_restore_shows_git_unavailable_screen(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let next = Arc::new(AppState {
        git_runtime: unavailable_git_runtime_state(),
        ..AppState::test_default()
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
            this.adopt_workspace(
                session::Workspace::new(vec![PathBuf::from("/repos/deferred")]),
                cx,
            );
        });
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("repository_loading_screen").is_none(),
        "a deferred restore must not spin while Git is unavailable"
    );
    cx.debug_bounds("git_unavailable_screen")
        .expect("expected the git unavailable screen");
}

#[gpui::test]
fn git_unavailable_overlay_blocks_open_repositories(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut next = AppState {
        git_runtime: unavailable_git_runtime_state(),
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    next.repos.push(open_repo_state_with_workdir(
        "/tmp/git-unavailable-overlay-test",
    ));
    let next = Arc::new(next);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
        let _ = window.draw(app);
    });

    cx.debug_bounds("git_unavailable_overlay")
        .expect("expected blocking git unavailable overlay");

    cx.update(|_window, app| {
        assert!(!view.read(app).is_home_screen_active());
        assert!(view.read(app).blocks_non_repository_actions());
    });
}

#[gpui::test]
fn git_unavailable_overlay_clears_after_runtime_recovery(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut unavailable = AppState {
        git_runtime: unavailable_git_runtime_state(),
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    unavailable.repos.push(open_repo_state_with_workdir(
        "/tmp/git-unavailable-recovery-test",
    ));
    let unavailable = Arc::new(unavailable);

    let mut recovered = AppState {
        git_runtime: available_git_runtime_state(),
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
    };
    recovered.repos.push(open_repo_state_with_workdir(
        "/tmp/git-unavailable-recovery-test",
    ));
    let recovered = Arc::new(recovered);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&unavailable), cx);
        });
        let _ = window.draw(app);
    });
    cx.debug_bounds("git_unavailable_overlay")
        .expect("expected overlay before runtime recovery");

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&recovered), cx);
        });
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("git_unavailable_overlay").is_none(),
        "expected overlay to disappear after runtime recovery"
    );
    cx.update(|_window, app| {
        assert!(!view.read(app).blocks_non_repository_actions());
    });
}

#[gpui::test]
fn splash_backdrop_renders_native_layers_and_tracks_theme(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let initial = view.read(app);
        assert!(
            Arc::ptr_eq(
                &initial.splash_backdrop_image,
                &super::super::splash::load_splash_backdrop_image(initial.theme.is_dark),
            ),
            "expected the resolved theme's backdrop before the first draw"
        );
        let _ = window.draw(app);
    });

    cx.debug_bounds("splash_backdrop_native")
        .expect("expected native splash backdrop root");
    cx.debug_bounds("splash_backdrop_image")
        .expect("expected SVG-backed splash image layer");
    for theme in [
        AppTheme::gitcomet_dark(),
        AppTheme::gitcomet_light(),
        AppTheme::gitcomet_dark(),
    ] {
        cx.update(|window, app| {
            view.update(app, |this, cx| this.set_theme(theme, cx));
            assert!(
                Arc::ptr_eq(
                    &view.read(app).splash_backdrop_image,
                    &super::super::splash::load_splash_backdrop_image(theme.is_dark),
                ),
                "expected theme changes to select the matching cached backdrop"
            );
            let _ = window.draw(app);
        });
        cx.debug_bounds("splash_backdrop_image")
            .expect("expected backdrop after switching themes");
        cx.debug_bounds("home_open_repo_action")
            .expect("expected Home controls after switching themes");
    }
    assert!(
        cx.debug_bounds("splash_backdrop_glow_layer").is_none(),
        "expected legacy procedural glow layer to be removed"
    );
    assert!(
        cx.debug_bounds("splash_backdrop_star_layer").is_none(),
        "expected animated star overlay to be removed"
    );
    assert!(
        cx.debug_bounds("splash_backdrop_center").is_none(),
        "expected legacy centered backdrop container to be removed"
    );

    let splash_active = cx.update(|_window, app| view.read(app).is_home_screen_active());
    assert!(splash_active, "expected splash screen to remain active");
}

#[gpui::test]
fn closing_last_repository_tab_returns_to_splash_screen(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from(
        "/tmp/repository-entry-screen-test",
    )));
    wait_until("repository tab to be added", || {
        !store_for_assert.snapshot().repos.is_empty()
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    pump_until(cx, "repository tab to render", |cx| {
        cx.debug_bounds("repo_tab_1").is_some()
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let splash_active = cx.update(|_window, app| view.read(app).is_home_screen_active());
    assert!(
        !splash_active,
        "expected splash screen to disappear after opening a repo"
    );

    #[cfg(not(target_os = "macos"))]
    assert!(
        cx.debug_bounds("app_menu").is_some(),
        "expected app menu button to be visible once a repo tab exists"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(
                this.close_active_repo_tab(cx),
                "expected the active repo tab to close"
            );
        });
    });

    wait_until("last repository tab to close", || {
        store_for_assert.snapshot().repos.is_empty()
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    pump_until(
        cx,
        "splash screen to render after closing the last tab",
        |cx| {
            cx.debug_bounds("repository_entry_screen").is_some()
                && cx.debug_bounds("repo_tab_1").is_none()
        },
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("repository_entry_screen")
        .expect("expected splash screen after closing the last repo");

    let splash_active = cx.update(|_window, app| view.read(app).is_home_screen_active());
    assert!(
        splash_active,
        "expected splash screen to return after closing the last repo"
    );
}

#[gpui::test]
fn request_quit_or_warn_queues_terminal_shutdown_prompt(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(this.request_quit_or_warn(2, 1, vec![], vec![], cx));
            let prompt = this
                .pending_terminal_shutdown_prompt
                .as_ref()
                .expect("expected a queued terminal shutdown prompt");
            assert!(matches!(prompt.action, TerminalShutdownAction::QuitApp));
            assert_eq!(prompt.summary.terminal_count, 2);
            assert_eq!(prompt.summary.running_command_count, 1);
        });
    });
}

#[gpui::test]
fn confirm_terminal_shutdown_close_window_removes_the_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    assert_eq!(cx.update(|_window, app| app.windows().len()), 1);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.confirm_terminal_shutdown(
                TerminalShutdownPrompt {
                    action: TerminalShutdownAction::CloseWindow,
                    summary: TerminalShutdownSummary {
                        terminal_count: 1,
                        running_command_count: 1,
                        repo_names: vec![],
                    },
                },
                window,
                cx,
            );
        });
    });

    assert_eq!(cx.cx.update(|app| app.windows().len()), 0);
}

/// Terminating a workspace's terminals to delete it, in the only window, takes
/// that window back to Home instead of closing it.
#[gpui::test]
fn confirm_terminal_shutdown_delete_workspace_keeps_the_last_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut workspace = gitcomet_state::session::Workspace::new(Vec::new());
    workspace.custom_name = Some("Alpha".into());
    let workspace_id = workspace.id;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace.clone()]);
        let _ = window.draw(app);
    });
    cx.update(|_window, app| view.update(app, |view, cx| view.adopt_workspace(workspace, cx)));
    cx.run_until_parked();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.confirm_terminal_shutdown(
                TerminalShutdownPrompt {
                    action: TerminalShutdownAction::DeleteWorkspace { workspace_id },
                    summary: TerminalShutdownSummary {
                        terminal_count: 1,
                        running_command_count: 1,
                        repo_names: vec![],
                    },
                },
                window,
                cx,
            );
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(app.windows().len(), 1, "the last window stays open");
        assert!(crate::workspaces::workspace(app, workspace_id).is_none());
        assert_eq!(view.read(app).workspace_id, None);
    });
}

#[gpui::test]
fn cancel_pending_terminal_shutdown_clears_prompt(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(this.request_quit_or_warn(2, 1, vec![], vec![], cx));
            assert!(this.pending_terminal_shutdown_prompt.is_some());
            this.clear_pending_terminal_shutdown_prompt(cx);
            assert!(this.pending_terminal_shutdown_prompt.is_none());
        });
    });
}

#[gpui::test]
fn request_close_window_or_warn_returns_false_without_terminals(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let window_id = window.window_handle().window_id();
        view.update(app, |this, cx| {
            assert!(!this.request_close_window_or_warn(window_id, cx));
        });
    });
}

#[gpui::test]
fn request_quit_or_warn_returns_false_when_no_running_commands(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(!this.request_quit_or_warn(1, 0, vec![], vec![], cx));
            assert!(this.pending_terminal_shutdown_prompt.is_none());
        });
    });
}

#[gpui::test]
fn quit_or_warn_stores_other_window_views(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let fake_views: Vec<gpui::WeakEntity<GitCometView>> = vec![
        gpui::WeakEntity::new_invalid(),
        gpui::WeakEntity::new_invalid(),
    ];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            assert!(this.request_quit_or_warn(1, 2, vec![], fake_views, cx));
            assert_eq!(this.pending_quit_other_views.len(), 2);
        });
    });
}

#[gpui::test]
fn confirm_quit_app_terminates_other_window_terminals(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let fake_views: Vec<gpui::WeakEntity<GitCometView>> = vec![gpui::WeakEntity::new_invalid()];

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.pending_quit_other_views = fake_views;
            this.confirm_terminal_shutdown(
                TerminalShutdownPrompt {
                    action: TerminalShutdownAction::QuitApp,
                    summary: TerminalShutdownSummary {
                        terminal_count: 1,
                        running_command_count: 1,
                        repo_names: vec![],
                    },
                },
                window,
                cx,
            );
            assert!(
                this.pending_quit_other_views.is_empty(),
                "other views must be drained after confirm"
            );
        });
    });
}

/// Every save runs a background refresh that is busy for a few milliseconds.
/// Only the tab spinner shows busy, and only after its delay, so a refresh
/// shorter than that must not redraw the tab strip or the action bar.
#[gpui::test]
fn refreshes_shorter_than_the_spinner_delay_leave_the_chrome_alone(cx: &mut gpui::TestAppContext) {
    use gitcomet_state::model::RepoLoadsInFlight;
    use std::cell::Cell;
    use std::rc::Rc;

    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/busy-chrome"),
        },
    );
    repo.open = Loadable::Ready(());
    let idle = Arc::new(AppState {
        active_repo: Some(repo_id),
        repos: vec![repo],
        ..AppState::test_default()
    });
    let mut busy = (*idle).clone();
    busy.repos[0]
        .loads_in_flight
        .request(RepoLoadsInFlight::WORKTREE_STATUS);
    let busy = Arc::new(busy);
    let publish = |state: &Arc<AppState>, cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                test_support::push_test_state(this, Arc::clone(state), cx)
            })
        });
    };
    let spinner = |cx: &mut gpui::VisualTestContext| {
        test_support::redraw(cx);
        cx.debug_bounds("repo_tab_busy_spinner_1").is_some()
    };

    cx.update(|_window, app| {
        let tabs = view.read(app).repo_tabs_bar.clone();
        tabs.update(app, |tabs, _| tabs.use_spinner_delay_for_tests());
    });
    publish(&idle, cx);
    test_support::redraw(cx);
    let tabs_notified = Rc::new(Cell::new(0usize));
    let actions_notified = Rc::new(Cell::new(0usize));
    let _subscriptions = cx.update(|_window, app| {
        let tabs = view.read(app).repo_tabs_bar.clone();
        let actions = view.read(app).action_bar.clone();
        let (tabs_count, actions_count) = (tabs_notified.clone(), actions_notified.clone());
        [
            app.observe(&tabs, move |_, _| tabs_count.set(tabs_count.get() + 1)),
            app.observe(&actions, move |_, _| {
                actions_count.set(actions_count.get() + 1)
            }),
        ]
    });

    publish(&busy, cx);
    publish(&idle, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert_eq!((tabs_notified.get(), actions_notified.get()), (0, 0));
    assert!(!spinner(cx));

    // Control: a refresh outlasting the delay shows the spinner, then hides it.
    publish(&busy, cx);
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    assert!(spinner(cx));
    publish(&idle, cx);
    assert!(!spinner(cx));
    assert_eq!((tabs_notified.get(), actions_notified.get()), (2, 0));
}

#[gpui::test]
fn loading_repo_tab_close_button_closes_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let mut state = AppState {
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    ));
    let ready_repo_id = RepoId(2);
    let mut ready_repo = RepoState::new_opening(
        ready_repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/GitComet"),
        },
    );
    ready_repo.open = Loadable::Ready(());
    state.repos.push(ready_repo);
    store_for_assert.replace_snapshot_for_test(Arc::new(state));
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
        let repo_tabs_bar = view.read(app).repo_tabs_bar.clone();
        repo_tabs_bar.update(app, |bar, cx| {
            let mut open_terminal_repo_ids = FxHashSet::default();
            open_terminal_repo_ids.insert(ready_repo_id);
            bar.set_open_terminal_repo_ids(open_terminal_repo_ids, cx);
        });
    });
    test_support::redraw(cx);

    let repo_tab_center = cx
        .debug_bounds("repo_tab_1")
        .expect("expected loading repo tab to be rendered")
        .center();
    let repo_tab_bounds = cx
        .debug_bounds("repo_tab_1")
        .expect("expected loading repo tab bounds");
    let label_before_hover = cx
        .debug_bounds("repo_tab_label_1")
        .expect("expected loading repo tab label before hover");
    assert_eq!(
        cx.debug_bounds("repo_tab_close_1"),
        None,
        "close action should stay hidden until the repository tab is hovered"
    );
    assert_eq!(
        cx.debug_bounds("repo_tab_close_fade_1"),
        None,
        "close fade should only exist together with the close action"
    );
    assert_eq!(
        repo_tab_bounds.size.width,
        px(components::Tab::MIN_WIDTH_PX),
        "expected a short repository label to fit the 18px status mark at the compact width"
    );
    cx.simulate_mouse_move(repo_tab_center, None, gpui::Modifiers::default());
    test_support::redraw(cx);

    let label_bounds = cx
        .debug_bounds("repo_tab_label_1")
        .expect("expected loading repo tab label bounds");
    let label_center_y = label_bounds.center().y;
    let spinner_bounds = cx
        .debug_bounds("repo_tab_busy_spinner_1")
        .expect("expected loading repo tab spinner bounds");
    let initials_bounds = cx
        .debug_bounds("repo_tab_initials_2")
        .expect("expected ready repo tab initials bounds");
    let ready_label_bounds = cx
        .debug_bounds("repo_tab_label_2")
        .expect("expected ready repo tab label bounds");
    let ready_label_center_y = ready_label_bounds.center().y;
    let terminal_bounds = cx
        .debug_bounds("repo_tab_terminal_2")
        .expect("expected ready repo tab terminal icon bounds");
    let close_center = cx
        .debug_bounds("repo_tab_close_1")
        .expect("expected loading repo tab close button to be rendered")
        .center();
    let close_bounds = cx
        .debug_bounds("repo_tab_close_1")
        .expect("expected loading repo tab close button bounds");
    let close_fade_bounds = cx
        .debug_bounds("repo_tab_close_fade_1")
        .expect("expected a fade before the overlaid close button");
    let close_trailing_inset = repo_tab_bounds.right() - close_bounds.right();
    // The tab's own side padding plus its border; tracked from the constant so
    // padding tweaks do not need this number re-derived by hand.
    let tab_side_padding = px(crate::view::panels::REPO_TAB_SIDE_PADDING_PX);
    assert!(
        close_trailing_inset >= tab_side_padding
            && close_trailing_inset <= tab_side_padding + px(2.0),
        "expected close button at the end of the tab inside its trailing padding, got \
         {close_trailing_inset:?}"
    );
    assert_eq!(
        label_bounds.size.width, label_before_hover.size.width,
        "showing the close action must not reserve or remove repository-label space"
    );
    assert!(
        label_bounds.right() > close_bounds.left(),
        "the close action should overlay the repository text instead of taking a flex slot"
    );
    assert_eq!(
        close_fade_bounds.size.width,
        px(16.0),
        "expected the shared 16px fade ramp before the close action"
    );
    assert_eq!(
        close_fade_bounds.right(),
        close_bounds.left(),
        "the fade ramp should meet the close button without a hard edge"
    );
    assert_eq!(
        spinner_bounds.size, initials_bounds.size,
        "expected loading spinner and repository initials to have identical dimensions"
    );
    assert_eq!(
        spinner_bounds.size,
        gpui::size(px(18.0), px(18.0)),
        "expected repository status marks to match the shared 18px text line box"
    );
    assert_eq!(
        close_bounds.size, spinner_bounds.size,
        "expected the repository close button to use the shared 18px geometry"
    );
    assert_eq!(
        terminal_bounds.size, spinner_bounds.size,
        "expected the embedded terminal icon to use the shared 18px geometry"
    );
    assert_eq!(
        label_bounds.left() - spinner_bounds.right(),
        px(6.0),
        "expected a 6px gap between the loading spinner and repository name"
    );
    assert_eq!(
        ready_label_bounds.left() - initials_bounds.right(),
        px(6.0),
        "expected a 6px gap between the initials badge and repository name"
    );
    assert_eq!(
        cx.debug_bounds("repo_tab_initials_1"),
        None,
        "expected loading repository initials to be replaced by the spinner"
    );
    assert_eq!(
        cx.debug_bounds("repo_tab_busy_spinner_2"),
        None,
        "expected a ready repository to show initials instead of a spinner"
    );
    assert_eq!(
        label_center_y,
        spinner_bounds.center().y,
        "expected repository label and loading spinner to share a centerline"
    );
    assert_eq!(
        label_center_y, close_center.y,
        "expected repository label and close button to share a centerline"
    );
    assert_eq!(
        ready_label_center_y,
        initials_bounds.center().y,
        "expected repository label and initials badge to share a centerline"
    );
    assert_eq!(
        ready_label_center_y,
        terminal_bounds.center().y,
        "expected repository label and terminal icon to share a centerline"
    );
    cx.simulate_mouse_move(close_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        close_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        close_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );

    wait_until("loading repo tab to close", || {
        !store_for_assert
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.id == repo_id)
    });
}

#[gpui::test]
fn inactive_repo_tab_tracks_pressed_state_for_its_label_fade(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(move |window, cx| {
        GitCometView::new(store_for_view, events, None, window, cx)
    });
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));

    let inactive_tab_center = cx
        .debug_bounds("repo_tab_2")
        .expect("expected inactive repository tab bounds")
        .center();
    cx.simulate_mouse_move(inactive_tab_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        inactive_tab_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert_eq!(
            test_support::pressed_repo_tab(view.read(app), app),
            Some(RepoId(2)),
            "expected the label fade to resolve against the held tab's active background"
        );
    });

    cx.simulate_mouse_up(
        inactive_tab_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);
    cx.update(|_window, app| {
        assert_eq!(test_support::pressed_repo_tab(view.read(app), app), None);
    });
}

#[gpui::test]
fn repo_tab_context_menu_renders_requested_actions(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_repo_tab_context_menu(cx, "repo_tab_2");

    assert_eq!(store.snapshot().active_repo, Some(RepoId(1)));
    cx.debug_bounds("context_menu_activate")
        .expect("expected Activate menu item");
    cx.debug_bounds("context_menu_open_repository_location")
        .expect("expected Open repository location menu item");
    cx.debug_bounds("context_menu_move_to_new_window")
        .expect("expected Move to new window menu item");
    cx.debug_bounds("context_menu_close")
        .expect("expected Close menu item");
    cx.debug_bounds("context_menu_close_repositories_to_the_right")
        .expect("expected Close repositories to the right menu item");
    cx.debug_bounds("context_menu_close_other_repositories")
        .expect("expected Close other repositories menu item");
    assert!(
        cx.debug_bounds("app_popover")
            .expect("expected repository tab context menu bounds")
            .size
            .width
            >= px(360.0),
        "expected repository tab context menu to use its wider layout"
    );
}

#[gpui::test]
fn repo_tab_context_menu_activate_activates_selected_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_repo_tab_context_menu(cx, "repo_tab_2");
    click_debug_selector(cx, "context_menu_activate");

    wait_until("repo tab menu activate action", || {
        store.snapshot().active_repo == Some(RepoId(2))
    });
}

#[gpui::test]
fn repo_tab_context_menu_close_closes_selected_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_repo_tab_context_menu(cx, "repo_tab_2");
    click_debug_selector(cx, "context_menu_close");

    wait_until("repo tab menu close action", || {
        store
            .snapshot()
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<Vec<_>>()
            == vec![RepoId(1), RepoId(3)]
    });
}

#[gpui::test]
fn repo_tab_context_menu_close_to_right_closes_right_repos(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(3));
    open_repo_tab_context_menu(cx, "repo_tab_2");
    click_debug_selector(cx, "context_menu_close_repositories_to_the_right");

    wait_until("repo tab menu close right action", || {
        let snapshot = store.snapshot();
        snapshot
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<Vec<_>>()
            == vec![RepoId(1), RepoId(2)]
            && snapshot.active_repo == Some(RepoId(2))
    });
}

#[gpui::test]
fn repo_tab_context_menu_close_other_repos_keeps_selected_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    open_repo_tab_context_menu(cx, "repo_tab_2");
    click_debug_selector(cx, "context_menu_close_other_repositories");

    wait_until("repo tab menu close other action", || {
        let snapshot = store.snapshot();
        snapshot
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<Vec<_>>()
            == vec![RepoId(2)]
            && snapshot.active_repo == Some(RepoId(2))
    });
}

#[gpui::test]
fn repo_tab_context_menu_activate_is_disabled_for_active_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(2));
    open_repo_tab_context_menu(cx, "repo_tab_2");
    click_debug_selector(cx, "context_menu_activate");

    assert_eq!(store.snapshot().active_repo, Some(RepoId(2)));
    cx.debug_bounds("context_menu_activate")
        .expect("expected disabled Activate item to leave the menu open");
}

#[gpui::test]
fn repo_tab_context_menu_close_right_is_disabled_for_last_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state(&store, &view, cx, RepoId(2));
    open_repo_tab_context_menu(cx, "repo_tab_3");
    click_debug_selector(cx, "context_menu_close_repositories_to_the_right");

    let snapshot = store.snapshot();
    assert_eq!(
        snapshot
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<Vec<_>>(),
        vec![RepoId(1), RepoId(2), RepoId(3)]
    );
    assert_eq!(snapshot.active_repo, Some(RepoId(2)));
    cx.debug_bounds("context_menu_close_repositories_to_the_right")
        .expect("expected disabled close-right item to leave the menu open");
}

#[gpui::test]
fn repo_tab_context_menu_close_others_is_disabled_for_single_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state_with_count(&store, &view, cx, RepoId(1), 1);
    open_repo_tab_context_menu(cx, "repo_tab_1");
    click_debug_selector(cx, "context_menu_close_other_repositories");

    let snapshot = store.snapshot();
    assert_eq!(
        snapshot
            .repos
            .iter()
            .map(|repo| repo.id)
            .collect::<Vec<_>>(),
        vec![RepoId(1)]
    );
    assert_eq!(snapshot.active_repo, Some(RepoId(1)));
    cx.debug_bounds("context_menu_close_other_repositories")
        .expect("expected disabled close-others item to leave the menu open");
}

fn theme_panel_color(key: &str) -> gpui::Rgba {
    crate::theme::AppTheme::from_key(key)
        .unwrap_or_else(|| panic!("embedded theme `{key}`"))
        .colors
        .surface
        .panel
}

/// A view restored into a customized, empty workspace with `theme_key` as its override.
fn view_in_themed_workspace<'a>(
    cx: &'a mut gpui::TestAppContext,
    theme_key: &str,
) -> (
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    gitcomet_state::session::WorkspaceId,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let mut workspace = gitcomet_state::session::Workspace::new(Vec::new());
    workspace.custom_name = Some("Themed".to_string());
    workspace.theme_mode = Some(theme_key.to_string());
    let workspace_id = workspace.id;
    cx.update(|app| crate::workspaces::initialize_for_test(app, vec![workspace.clone()]));
    let config = GitCometViewConfig {
        workspace: WorkspaceBootstrap::Saved(Box::new(workspace)),
        ..GitCometViewConfig::normal(None)
    };
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (view, cx, workspace_id)
}

#[gpui::test]
fn workspace_theme_override_beats_the_global_preference(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx, _workspace_id) = view_in_themed_workspace(cx, "tokyo_night");
    assert_eq!(
        cx.update(|_window, app| view.read(app).theme.colors.surface.panel),
        theme_panel_color("tokyo_night"),
        "the window starts in its workspace theme"
    );

    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.set_theme_mode(
                ThemeMode::Named("sunset_veil".to_string()),
                window.appearance(),
                cx,
            );
        });
    });
    cx.update(|_window, app| {
        view.update(app, |view, cx| test_support::sync_store_snapshot(view, cx));
    });

    let (global, panel) = cx.update(|_window, app| {
        let view = view.read(app);
        (view.theme_mode.clone(), view.theme.colors.surface.panel)
    });
    assert_eq!(global, ThemeMode::Named("sunset_veil".to_string()));
    assert_eq!(
        panel,
        theme_panel_color("tokyo_night"),
        "a global theme change must not repaint an overridden workspace"
    );
}

#[gpui::test]
fn clearing_the_workspace_theme_override_falls_back_to_the_global_preference(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, cx, workspace_id) = view_in_themed_workspace(cx, "tokyo_night");
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.set_theme_mode(
                ThemeMode::Named("sunset_veil".to_string()),
                window.appearance(),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        assert!(crate::workspaces::set_workspace_theme_mode(
            app,
            workspace_id,
            None
        ));
        view.update(app, |view, cx| view.sync_workspace_theme_override(cx));
    });

    assert_eq!(
        cx.update(|_window, app| view.read(app).theme.colors.surface.panel),
        theme_panel_color("sunset_veil")
    );
}

#[gpui::test]
fn opening_a_workspace_from_home_adopts_it_into_this_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let mut workspace = gitcomet_state::session::Workspace::new(Vec::new());
    workspace.custom_name = Some("Later".to_string());
    workspace.restore_on_launch = false;
    let id = workspace.id;
    let row: &'static str = format!("home_workspace_{id}").leak();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let window_id = cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace]);
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    assert!(cx.debug_bounds("repo_picker_toggle").is_none());

    let center = cx.debug_bounds(row).expect("workspace row").center();
    cx.simulate_click(center, gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let adopted = cx.update(|_window, app| crate::workspaces::workspace_for_window(app, window_id));
    assert_eq!(adopted.map(|workspace| workspace.id), Some(id));
    assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
    assert!(
        cx.debug_bounds("repo_picker_toggle").is_some(),
        "the title bar shows the adopted workspace's chip on Home"
    );
    assert!(
        cx.debug_bounds(row).is_none(),
        "Home no longer lists its own workspace"
    );
}

#[gpui::test]
fn linked_worktree_scans_do_not_keep_the_active_tab_spinner_busy(cx: &mut gpui::TestAppContext) {
    use gitcomet_state::model::RepoLoadsInFlight;
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repos = [1, 2].map(|id| {
        let mut repo = RepoState::new_opening(
            RepoId(id),
            RepoSpec {
                workdir: PathBuf::from(format!("/tmp/linked-spinner-{id}")),
            },
        );
        repo.open = Loadable::Ready(());
        repo.loads_in_flight
            .request(RepoLoadsInFlight::WORKTREE_DIRTY);
        repo
    });
    let mut state = AppState {
        active_repo: Some(RepoId(1)),
        repos: repos.into(),
        ..AppState::test_default()
    };
    cx.update(|_, app| {
        view.read(app)
            .repo_tabs_bar
            .clone()
            .update(app, |tabs, _| tabs.use_spinner_delay_for_tests());
    });
    let publish = |state: &AppState, cx: &mut gpui::VisualTestContext| {
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                test_support::push_test_state(this, Arc::new(state.clone()), cx)
            })
        });
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        test_support::redraw(cx);
    };
    for id in [1, 2, 1] {
        state.active_repo = Some(RepoId(id));
        publish(&state, cx);
        assert!(
            cx.debug_bounds(if id == 1 {
                "repo_tab_busy_spinner_1"
            } else {
                "repo_tab_busy_spinner_2"
            })
            .is_none(),
            "scanning another checkout is not loading this tab"
        );
        let repo = state
            .repos
            .iter_mut()
            .find(|repo| repo.id == RepoId(id))
            .unwrap();
        repo.loads_in_flight
            .request(RepoLoadsInFlight::WORKTREE_STATUS);
        publish(&state, cx);
        assert!(
            cx.debug_bounds(if id == 1 {
                "repo_tab_busy_spinner_1"
            } else {
                "repo_tab_busy_spinner_2"
            })
            .is_some(),
            "foreground status still shows busy"
        );
        state
            .repos
            .iter_mut()
            .find(|repo| repo.id == RepoId(id))
            .unwrap()
            .loads_in_flight
            .finish(RepoLoadsInFlight::WORKTREE_STATUS);
        publish(&state, cx);
        assert!(
            cx.debug_bounds(if id == 1 {
                "repo_tab_busy_spinner_1"
            } else {
                "repo_tab_busy_spinner_2"
            })
            .is_none(),
            "foreground completion hides busy while the linked scan continues"
        );
    }
}
