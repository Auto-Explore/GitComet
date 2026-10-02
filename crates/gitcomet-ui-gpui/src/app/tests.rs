use super::*;
use gpui::point;
use gpui::{
    Action, Context, FocusHandle, InteractiveElement, IntoElement, Render, Styled, Window, div,
};

use crate::test_support::lock_visual_test;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitRepository, Result};
use gitcomet_state::msg::Msg;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "test backend does not open repositories",
        )))
    }
}

struct NotARepositoryBackend;

impl GitBackend for NotARepositoryBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::NotARepository))
    }
}

struct RecordingOpenBackend {
    opened: mpsc::Sender<PathBuf>,
}

impl GitBackend for RecordingOpenBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        let _ = self.opened.send(workdir.to_path_buf());
        Err(Error::new(ErrorKind::Unsupported(
            "recording backend does not open repositories",
        )))
    }
}

struct ControlledOpenBackend {
    opened: mpsc::Sender<PathBuf>,
    result: Mutex<mpsc::Receiver<bool>>,
}

impl GitBackend for ControlledOpenBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        let _ = self.opened.send(workdir.to_path_buf());
        // Dropping the sender also unblocks the worker if an assertion fails.
        if self.result.lock().unwrap().recv().unwrap_or(false) {
            Ok(Arc::new(
                gitcomet_core::test_support::UnconfiguredRepository::new(workdir),
            ))
        } else {
            Err(Error::new(ErrorKind::NotARepository))
        }
    }
}

#[test]
fn manual_update_menu_is_disabled_without_a_feedback_window() {
    assert!(manual_update_check_menu_disabled(false, false));
    assert!(manual_update_check_menu_disabled(true, true));
    assert!(!manual_update_check_menu_disabled(false, true));
}

#[gpui::test]
fn manual_update_check_activates_its_feedback_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let main_window_id = cx.update(|window, app| {
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });

    cx.cx.update(crate::view::open_settings_window);
    cx.run_until_parked();
    let settings_window = cx.cx.update(|app| {
        app.windows()
            .into_iter()
            .find(|window| window.window_id() != main_window_id)
            .expect("Settings window should open")
    });
    cx.cx.update(|app| {
        let _ = settings_window.update(app, |_view, window, _cx| window.activate());
        assert_ne!(
            app.active_window().map(|window| window.window_id()),
            Some(main_window_id),
            "Settings should own focus before the manual update check"
        );
        assert!(check_for_updates_in_active_or_existing_normal_window(app));
        assert_eq!(
            app.active_window().map(|window| window.window_id()),
            Some(main_window_id),
            "manual feedback must be shown in the window brought to the foreground"
        );
    });
}

#[cfg(target_os = "macos")]
fn menu_action_entries(menu: &Menu) -> Vec<(String, String)> {
    menu.items
        .iter()
        .filter_map(|item| match item {
            MenuItem::Action { name, action, .. } => {
                Some((name.to_string(), action.name().to_string()))
            }
            _ => None,
        })
        .collect()
}

#[cfg(target_os = "macos")]
#[test]
fn macos_app_menus_use_gitcomet_terminology_and_actions() {
    let menus = macos_app_menus_with_external_editor(true);
    let app_menu = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "GitComet")
        .expect("GitComet menu");
    let file_menu = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "File")
        .expect("File menu");

    let app_entries = menu_action_entries(app_menu);
    assert_eq!(
        &app_entries[..2],
        &[
            (
                crate::menu_labels::COMMAND_PALETTE.to_string(),
                ToggleCommandPalette.name().to_string(),
            ),
            (
                crate::menu_labels::SETTINGS.to_string(),
                OpenSettings.name().to_string(),
            ),
        ]
    );

    let file_entries = menu_action_entries(file_menu);
    assert_eq!(
        file_entries,
        vec![
            ("New Window".to_string(), NewWindow.name().to_string()),
            (
                crate::menu_labels::OPEN_REPOSITORY.to_string(),
                OpenRepository.name().to_string(),
            ),
            (
                crate::menu_labels::CLONE_REPOSITORY.to_string(),
                CloneRepository.name().to_string(),
            ),
            (
                crate::menu_labels::INITIALIZE_REPOSITORY.to_string(),
                InitializeRepository.name().to_string(),
            ),
            (
                "Switch Repository…".to_string(),
                SwitchRepository.name().to_string(),
            ),
            (
                "Open Workspace…".to_string(),
                OpenWorkspace.name().to_string(),
            ),
            (
                crate::menu_labels::OPEN_IN_CODE_EDITOR.to_string(),
                OpenInCodeEditor.name().to_string(),
            ),
            (
                crate::menu_labels::OPEN_IN_FILE_EXPLORER.to_string(),
                LocateFileInExplorer.name().to_string(),
            ),
            (
                crate::menu_labels::OPEN_REMOTE_IN_BROWSER.to_string(),
                OpenRemoteInBrowser.name().to_string(),
            ),
            (
                crate::menu_labels::APPLY_PATCH.to_string(),
                ApplyPatch.name().to_string(),
            ),
            (
                crate::menu_labels::CHECK_FOR_UPDATES.to_string(),
                CheckForUpdates.name().to_string(),
            ),
            ("Close".to_string(), Close.name().to_string()),
            ("Close Window".to_string(), CloseWindow.name().to_string(),),
        ]
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_file_menu_hides_unconfigured_external_editor_action() {
    let menus = macos_app_menus_with_external_editor(false);
    let file_menu = menus
        .iter()
        .find(|menu| menu.name.as_ref() == "File")
        .expect("File menu");
    let entries = menu_action_entries(file_menu);

    assert!(
        entries
            .iter()
            .all(|(_, action)| action != OpenInCodeEditor.name())
    );
    assert!(entries.iter().any(|(label, action)| {
        label == crate::menu_labels::OPEN_IN_FILE_EXPLORER && action == LocateFileInExplorer.name()
    }));
}

fn seed_worktree_repo(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    view: gpui::Entity<GitCometView>,
) {
    store.dispatch(Msg::OpenRepo(
        std::env::temp_dir().join("gitcomet-app-test-repo"),
    ));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| !view.read(app).blocks_non_repository_actions());
        if ready {
            return;
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for the window to leave the Home screen");
        }

        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui::test]
fn review_regression_cold_start_honors_new_window_browser_target_when_workspaces_restore(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let restored_path = std::env::temp_dir().join("gitcomet-cold-start-restored");
    let requested_path = restored_path.clone();
    let mut saved = session::Workspace::new(vec![restored_path]);
    saved.restore_on_launch = true;
    saved.last_activation_order = 7;
    let workspaces = vec![saved];
    let mut launch = normal_launch_config(Some(requested_path), None);
    launch.browser_open_target = BrowserOpenTarget::NewWindow;

    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, workspaces.clone());
        open_initial_gitcomet_windows_after_workspace_initialization(
            app,
            Arc::clone(&backend),
            &launch,
            workspaces,
        );
    });

    assert_eq!(
        cx.update(|app| app.windows().len()),
        2,
        "the requested repository needs its own window beside the restored group"
    );
}

#[gpui::test]
fn review_regression_startup_path_uses_the_last_activated_restored_workspace(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let mut older = session::Workspace::new(vec![
        std::env::temp_dir().join("gitcomet-startup-older-workspace"),
    ]);
    older.restore_on_launch = true;
    older.last_activation_order = 1;
    let mut latest = session::Workspace::new(vec![
        std::env::temp_dir().join("gitcomet-startup-latest-workspace"),
    ]);
    latest.restore_on_launch = true;
    latest.last_activation_order = 9;
    let latest_id = latest.id;
    // Saved order differs from activation order.
    let workspaces = vec![latest, older];
    let requested = std::env::temp_dir().join("gitcomet-startup-requested-repository");
    let mut launch = normal_launch_config(Some(requested.clone()), None);
    launch.browser_open_target = BrowserOpenTarget::ExistingWindow;

    crate::ui_runtime::with_override(
        crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
        || {
            cx.update(|app| {
                crate::workspaces::initialize_for_test(app, workspaces.clone());
                open_initial_gitcomet_windows_after_workspace_initialization(
                    app, backend, &launch, workspaces,
                );
                assert_eq!(app.windows().len(), 2);
                let owner = find_normal_gitcomet_window_for_repo(app, &requested)
                    .expect("the startup request must have a window owner before focus events run");
                assert_eq!(owner.workspace_id, Some(latest_id));
            });
        },
    );
}

#[gpui::test]
fn review_regression_startup_routing_does_not_depend_on_native_focus(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let launch = normal_empty_launch_config(None);
    let first = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
    let second = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
    let mut window_cx = gpui::VisualTestContext::from_window(first.into(), cx);
    window_cx.deactivate_window();

    cx.update(|app| {
        // Reproduce startup before Linux delivers either a native active
        // window or the view's focus notification. Pick the window the
        // arbitrary fallback would not choose, so the test is deterministic.
        app.update_default_global::<GitCometWindowRegistry, _>(|registry, _| {
            registry.last_focused_normal_window = None;
        });
        assert!(app.active_window().is_none());
        let fallback = find_normal_gitcomet_window(app)
            .expect("normal window")
            .handle;
        let preferred = if fallback.window_id() == first.window_id() {
            second
        } else {
            first
        };

        for stale_focus in [false, true] {
            if stale_focus {
                // A compositor can also still report the previous window
                // as active while the requested activation is pending.
                activate_gitcomet_window(app, fallback);
                mark_gitcomet_window_focused(app, fallback.window_id());
            }
            let path =
                std::env::temp_dir().join(format!("gitcomet-startup-pending-focus-{stale_focus}"));
            handle_browser_open_request_with_window(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: Some(path.clone()),
                    target: BrowserOpenTarget::ExistingWindow,
                },
                Some(preferred.window_id()),
            );
            let owner = find_normal_gitcomet_window_for_repo(app, &path).expect("repository owner");
            assert_eq!(owner.handle.window_id(), preferred.window_id());

            // The destination wins even when another window has the path.
            handle_browser_open_request_with_window(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: Some(path.clone()),
                    target: BrowserOpenTarget::ExistingWindow,
                },
                Some(fallback.window_id()),
            );
            let owners: Vec<_> = gitcomet_window_entries(app)
                .into_iter()
                .filter(|entry| entry_contains_repo_path(entry, &path))
                .map(|entry| entry.handle.window_id())
                .collect();
            assert_eq!(owners.len(), 2);
            assert!(owners.contains(&preferred.window_id()));
            assert!(owners.contains(&fallback.window_id()));
        }
        assert_eq!(app.windows().len(), 2);
    });
}

#[gpui::test]
fn review_regression_followup_startup_path_is_routed_when_no_workspace_restores(
    cx: &mut gpui::TestAppContext,
) {
    let (opened_tx, opened_rx) = mpsc::channel();
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingOpenBackend { opened: opened_tx });
    let requested = std::env::temp_dir().join("gitcomet-fresh-start-requested-repository");
    let launch = normal_launch_config(Some(requested.clone()), None);

    crate::ui_runtime::with_override(
        crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
        || {
            cx.update(|app| {
                crate::workspaces::initialize_for_test(app, Vec::new());
                open_initial_gitcomet_windows_after_workspace_initialization(
                    app,
                    Arc::clone(&backend),
                    &launch,
                    Vec::new(),
                );
            });
        },
    );

    let opened = opened_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("a fresh launch must dispatch its requested repository");
    assert_eq!(opened, requested);
}

#[gpui::test]
fn review_regression_manual_and_picker_opens_use_the_initiating_window(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (opened_tx, opened_rx) = mpsc::channel();
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingOpenBackend { opened: opened_tx });
    let path = std::env::temp_dir().join("gitcomet-app-wide-owned-repository");
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

    let (first_store, first_events) = AppStore::new_test(Arc::clone(&backend));
    let first =
        cx.add_window(|window, cx| GitCometView::new(first_store, first_events, None, window, cx));
    let (second_store, second_events) = AppStore::new_test(Arc::clone(&backend));
    let second_store_for_assertions = second_store.clone();
    let second = cx
        .add_window(|window, cx| GitCometView::new(second_store, second_events, None, window, cx));

    let repo_id = gitcomet_state::model::RepoId(41);
    let owner_state = gitcomet_state::model::AppState {
        repos: vec![gitcomet_state::model::RepoState::new_opening(
            repo_id,
            gitcomet_core::domain::RepoSpec {
                workdir: path.clone(),
            },
        )],
        active_repo: Some(repo_id),
        ..gitcomet_state::model::AppState::test_default()
    };
    first
        .update(cx, |view, _window, cx| {
            crate::view::test_support::apply_state_snapshot_for_test(
                view,
                Arc::new(owner_state),
                cx,
            );
        })
        .expect("install the first window's live repository state");

    second
        .update(cx, |view, _window, cx| {
            view.open_repo_path(path.clone(), cx);
        })
        .expect("manually open from the second window");
    second
        .update(cx, |view, _window, cx| {
            crate::view::test_support::activate_closed_repo_picker_entry_for_test(
                view,
                path.clone(),
                cx,
            );
        })
        .expect("activate a closed repository-picker row");
    cx.background_executor.run_until_parked();

    assert_eq!(
        opened_rx
            .recv_timeout(Duration::from_secs(3))
            .expect("open in the second window"),
        path
    );
    let owners = cx.update(|app| {
        gitcomet_window_entries(app)
            .into_iter()
            .filter(|entry| entry_contains_repo_path(entry, &path))
            .map(|entry| entry.handle.window_id())
            .collect::<Vec<_>>()
    });
    assert_eq!(owners.len(), 2);
    assert!(owners.contains(&first.window_id()));
    assert!(owners.contains(&second.window_id()));
    assert_eq!(second_store_for_assertions.snapshot().repos.len(), 1);
    assert!(
        opened_rx.try_recv().is_err(),
        "reopening in the same window must reuse its tab"
    );
}

#[gpui::test]
fn review_regression_followup_restoring_multiple_workspaces_runs_startup_hooks_once(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let workspaces = (0..3)
        .map(|index| {
            let mut workspace = session::Workspace::new(vec![
                std::env::temp_dir().join(format!("gitcomet-startup-hook-group-{index}")),
            ]);
            // Test runtime disables automatic repo restore. Keep the named
            // workspace mapped even while its window is empty.
            workspace.custom_name = Some(format!("Startup {index}"));
            workspace.restore_on_launch = true;
            workspace.last_activation_order = index;
            workspace
        })
        .collect::<Vec<_>>();
    let expected_workspace = workspaces.last().unwrap().id;
    let launch = normal_empty_launch_config(None);

    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, workspaces.clone());
        open_initial_gitcomet_windows_after_workspace_initialization(
            app,
            Arc::clone(&backend),
            &launch,
            workspaces,
        );
    });

    assert_eq!(
        cx.update(startup_hook_invocation_count_for_test),
        1,
        "survey and update startup hooks are process work, not per-window work"
    );
    cx.update(|app| {
        let window = app.global::<StartupHookInvocationCount>().1.unwrap();
        assert_eq!(
            crate::workspaces::workspace_for_window(app, window)
                .unwrap()
                .id,
            expected_workspace,
            "startup hooks belong to the most recently active restored workspace"
        );
    });
}

#[gpui::test]
fn pr530_browser_request_handler_closes_its_receiver_on_shutdown(cx: &mut gpui::TestAppContext) {
    let app = cx.new_app();
    let (requests, received) = smol::channel::unbounded();
    app.update(|app| register_browser_open_request_handler(app, Arc::new(TestBackend), received));
    app.background_executor.run_until_parked();
    assert!(!requests.is_closed());
    app.quit();
    assert!(
        requests.is_closed(),
        "shutdown must stop accepting forwarded requests"
    );
}

#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn pr530_closing_last_main_window_with_settings_keeps_workspace_restorable(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, Vec::new());
        let main = open_gitcomet_window(
            app,
            Arc::new(TestBackend),
            &normal_empty_launch_config(None),
        );
        let workspace = crate::workspaces::sync_window(
            app,
            main.window_id(),
            None,
            vec![PathBuf::from("/tmp/pr530-last-main")],
            None,
        )
        .unwrap();
        crate::view::open_settings_window(app);
        assert_eq!(app.windows().len(), 2);
        mark_window_closing(app, main.window_id());
        assert!(
            crate::workspaces::workspace(app, workspace)
                .unwrap()
                .restore_on_launch,
            "Settings must not turn last-main-window close into workspace removal"
        );
    });
}

#[gpui::test]
fn review_regression_restoring_overlapping_workspaces_keeps_both_windows(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let shared = std::env::temp_dir().join("gitcomet-restored-shared");
    let other = std::env::temp_dir().join("gitcomet-restored-other");
    let mut first = session::Workspace::new(vec![shared.clone(), other.clone()]);
    first.active_repository = Some(shared.clone());
    first.last_activation_order = 1;
    let mut second = session::Workspace::new(vec![shared.clone(), other.clone()]);
    second.active_repository = Some(other);
    second.last_activation_order = 2;
    let workspaces = vec![first, second];
    crate::ui_runtime::with_override(
        crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
        || {
            cx.update(|app| {
                crate::workspaces::initialize_for_test(app, workspaces.clone());
                open_initial_gitcomet_windows_after_workspace_initialization(
                    app,
                    backend,
                    &normal_empty_launch_config(None),
                    workspaces.clone(),
                );
            })
        },
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.run_until_parked();
        let restored = cx.update(|app| {
            let windows = gitcomet_window_entries(app);
            assert_eq!(windows.len(), 2);
            windows.iter().all(|entry| {
                let saved = workspaces
                    .iter()
                    .find(|workspace| Some(workspace.id) == entry.workspace_id)
                    .unwrap();
                entry
                    .view
                    .update(app, |view, cx| {
                        crate::view::test_support::sync_store_snapshot(view, cx);
                        let (pending, count, active) =
                            crate::view::test_support::startup_repository_state_for_test(view);
                        !pending && count == 2 && active == saved.active_repository
                    })
                    .unwrap()
            })
        });
        if restored {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "restore each workspace's independent active tab"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|app| {
        assert_eq!(windows_owning_repo_for_test(app, &shared).len(), 2);
        for saved in workspaces {
            let restored = crate::workspaces::workspace(app, saved.id).unwrap();
            assert_eq!(restored.repositories, saved.repositories);
            assert_eq!(restored.active_repository, saved.active_repository);
        }
    });
}

#[gpui::test]
fn review_regression_native_picker_keeps_its_initiating_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let launch = normal_empty_launch_config(None);
    let first = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
    let second = cx.update(|app| open_gitcomet_window(app, Arc::clone(&backend), &launch));
    let path = std::env::temp_dir().join("gitcomet-picker-shared");
    cx.update(|app| {
        let first_entry = normal_gitcomet_window_by_id(app, first.window_id()).unwrap();
        open_repository_in_window(app, &first_entry, path.clone());
        activate_gitcomet_window(app, second.into());
        prompt_open_repository(app, Arc::clone(&backend));
        // Focus can change while the native dialog is open.
        activate_gitcomet_window(app, first.into());
    });
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![path.clone()]));
    cx.run_until_parked();
    cx.update(|app| {
        let owners = windows_owning_repo_for_test(app, &path);
        assert_eq!(owners.len(), 2);
        assert!(owners.contains(&first.window_id()));
        assert!(owners.contains(&second.window_id()));
        assert_eq!(app.active_window().unwrap().window_id(), second.window_id());
    });
}

#[gpui::test]
fn review_regression_folder_drop_reuses_only_the_destination_windows_tab(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (opened_tx, opened_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let backend: Arc<dyn GitBackend> = Arc::new(ControlledOpenBackend {
        opened: opened_tx,
        result: Mutex::new(result_rx),
    });
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let folder = tempfile::tempdir().unwrap();
    let path = normalize_repository_open_path(folder.path().to_path_buf());
    let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
    let first =
        cx.add_window(|window, cx| GitCometView::new(first_store, first_events, None, window, cx));
    first
        .update(cx, |view, _, cx| view.open_repo_path(path.clone(), cx))
        .unwrap();
    cx.run_until_parked();
    let (store, events) = AppStore::new_test(backend);
    let store_for_view = store.clone();
    let second =
        cx.add_window(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    for dropped_path in [path.clone(), path.join(".")] {
        second
            .update(cx, |_, _, cx| {
                open_dropped_repository_from_view(cx, second.window_id(), dropped_path);
            })
            .unwrap();
        cx.run_until_parked();
    }
    assert_eq!(
        opened_rx.recv_timeout(Duration::from_secs(3)).unwrap(),
        path
    );
    assert_eq!(store.snapshot().repos.len(), 1);
    cx.update(|app| {
        let windows = windows_owning_repo_for_test(app, &path);
        assert_eq!(windows.len(), 2);
        assert!(windows.contains(&first.window_id()));
        assert!(windows.contains(&second.window_id()));
        assert!(
            crate::workspaces::workspace_for_window(app, second.window_id()).is_none(),
            "the dropped copy must validate before it is saved"
        );
    });
    result_tx.send(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        second
            .update(cx, |view, _, cx| {
                crate::view::test_support::sync_store_snapshot(view, cx)
            })
            .unwrap();
        cx.run_until_parked();
        if cx.update(|app| {
            crate::workspaces::workspace_for_window(app, second.window_id()).is_some()
        }) {
            break;
        }
        assert!(Instant::now() < deadline, "save the validated drop");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        opened_rx.try_recv().is_err(),
        "same-window drops must share one validation"
    );
    first
        .update(cx, |_, window, _| window.remove_window())
        .unwrap();
    cx.run_until_parked();
    cx.update(|app| {
        assert_eq!(
            windows_owning_repo_for_test(app, &path),
            vec![second.window_id()]
        );
        assert_eq!(
            crate::workspaces::workspace_for_window(app, second.window_id())
                .unwrap()
                .repositories,
            vec![path]
        );
    });
    assert_eq!(store.snapshot().repos.len(), 1);
}

#[gpui::test]
fn review_regression_forwarded_open_falls_back_to_the_last_focused_normal_window(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

    let (first_store, first_events) = AppStore::new_test(Arc::clone(&backend));
    let first =
        cx.add_window(|window, cx| GitCometView::new(first_store, first_events, None, window, cx));
    let (second_store, second_events) = AppStore::new_test(Arc::clone(&backend));
    let second = cx
        .add_window(|window, cx| GitCometView::new(second_store, second_events, None, window, cx));

    first
        .update(cx, |_view, window, _cx| window.activate())
        .expect("activate a window before simulating app blur");
    cx.background_executor.run_until_parked();
    let mut first_context = gpui::VisualTestContext::from_window(first.into(), cx);
    first_context.deactivate_window();
    let arbitrary_fallback = cx
        .update(find_normal_gitcomet_window)
        .expect("normal window")
        .handle
        .window_id();
    let expected = if arbitrary_fallback == first.window_id() {
        gpui::AnyWindowHandle::from(second)
    } else {
        gpui::AnyWindowHandle::from(first)
    };
    expected
        .update(cx, |_view, window, _cx| window.activate())
        .expect("activate the non-fallback window");
    cx.background_executor.run_until_parked();
    let mut expected_context = gpui::VisualTestContext::from_window(expected, cx);
    expected_context.deactivate_window();

    let selected = cx
        .update(find_normal_gitcomet_window)
        .expect("normal window after app blur");
    assert_eq!(
        selected.handle.window_id(),
        expected.window_id(),
        "an external terminal leaves no active GPUI window, so the remembered focus must win"
    );
}

#[gpui::test]
fn review_regression_recovering_a_saved_workspace_keeps_overlapping_repositories(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let duplicate_path = std::env::temp_dir().join("gitcomet-app-test-repo");
    let mut saved = session::Workspace::new(vec![duplicate_path.clone()]);
    saved.restore_on_launch = false;
    let saved_id = saved.id;
    cx.update(|app| crate::workspaces::initialize_for_test(app, vec![saved]));

    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });
    seed_worktree_repo(cx, &store, view);
    assert!(
        cx.cx
            .update(|app| { find_normal_gitcomet_window_for_repo(app, &duplicate_path).is_some() })
    );

    let recovered = crate::ui_runtime::with_override(
        crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
        || {
            cx.cx.update(|app| {
                activate_or_open_workspace(app, saved_id).expect("open the saved workspace")
            })
        },
    );
    cx.run_until_parked();

    assert_eq!(
        cx.cx.update(|app| app.windows().len()),
        2,
        "each workspace needs its own window even when its repositories overlap"
    );
    cx.cx.update(|app| {
        let saved = crate::workspaces::workspace(app, saved_id).expect("retain saved workspace");
        assert_eq!(saved.repositories, vec![duplicate_path.clone()]);
        assert_eq!(recovered.workspace_id, Some(saved_id));
        let again = activate_or_open_workspace(app, saved_id).expect("focus the workspace");
        assert_eq!(again.handle.window_id(), recovered.handle.window_id());
        assert_eq!(app.windows().len(), 2);
    });
}

#[gpui::test]
fn review_regression_move_to_a_saved_workspace_with_the_same_repository(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let path = std::env::temp_dir().join("gitcomet-app-test-repo");
    let mut saved = session::Workspace::new(vec![path.clone()]);
    saved.restore_on_launch = false;
    let saved_id = saved.id;
    cx.update(|app| crate::workspaces::initialize_for_test(app, vec![saved]));

    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let source_window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view);
    let repo_id = store.snapshot().repos[0].id;

    crate::ui_runtime::with_override(
        crate::ui_runtime::UiRuntime::deterministic_auto_restore(),
        || {
            cx.cx.update(|app| {
                move_repository_to_workspace(
                    app,
                    source_window_id,
                    repo_id,
                    path.clone(),
                    Some(saved_id),
                );
            })
        },
    );
    cx.run_until_parked();

    let (window_count, owners) = cx.cx.update(|app| {
        (
            app.windows().len(),
            gitcomet_window_entries(app)
                .into_iter()
                .filter(|entry| entry_contains_repo_path(entry, &path))
                .map(|entry| entry.handle.window_id())
                .collect::<Vec<_>>(),
        )
    });
    assert_eq!(window_count, 1, "moving the only tab closes the source");
    assert_eq!(owners.len(), 1);
    assert_ne!(owners[0], source_window_id);
    cx.cx.update(|app| {
        let destination = normal_gitcomet_window_by_id(app, owners[0]).unwrap();
        assert_eq!(destination.workspace_id, Some(saved_id));
        assert_eq!(destination.repo_paths.as_ref(), &[path]);
    });
}

/// Closing the last window quits on Linux/Windows, so it must keep the
/// workspace restorable; closing one of several windows still hides it.
#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn review_regression_closing_the_last_window_keeps_its_workspace_restorable(
    cx: &mut gpui::TestAppContext,
) {
    let first = cx.add_window(|_, _| gpui::Empty);
    let last = cx.add_window(|_, _| gpui::Empty);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (first_id, last_id) = cx.update(|app| {
        let sync = |app: &mut App, window: gpui::WindowId, repo: &str| {
            crate::workspaces::sync_window(app, window, None, vec![PathBuf::from(repo)], None)
                .expect("durable workspace")
        };
        (
            sync(app, first.window_id(), "/repos/a"),
            sync(app, last.window_id(), "/repos/b"),
        )
    });
    let restores = |cx: &mut gpui::TestAppContext, id| {
        cx.update(|app| crate::workspaces::workspace(app, id))
            .expect("workspace kept")
            .restore_on_launch
    };

    first
        .update(cx, |_, window, app| close_window_or_warn(window, app))
        .unwrap();
    assert!(
        !restores(cx, first_id),
        "a closed non-final window is hidden"
    );

    last.update(cx, |_, window, app| close_window_or_warn(window, app))
        .unwrap();
    assert!(
        restores(cx, last_id),
        "closing the final window quits, so its workspace must restore"
    );
}

/// A stale registry entry must not prevent reopening a distinct workspace,
/// even when that workspace contains the same repository as the source.
#[gpui::test]
fn review_regression_move_noop_check_ignores_closed_windows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let path = std::env::temp_dir().join("gitcomet-app-test-repo");
    let mut saved = session::Workspace::new(vec![path.clone()]);
    saved.restore_on_launch = false;
    let saved_id = saved.id;
    cx.update(|app| crate::workspaces::initialize_for_test(app, vec![saved]));
    let closed = cx.add_window(|_, _| gpui::Empty);
    let closed_handle: gpui::AnyWindowHandle = closed.into();

    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let source_window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view);

    closed
        .update(cx, |_, window, _| window.remove_window())
        .expect("close the stale window");
    cx.run_until_parked();
    cx.cx.update(|app| {
        app.update_default_global::<GitCometWindowRegistry, _>(|registry, _cx| {
            registry.windows.insert(
                closed_handle.window_id(),
                GitCometWindowEntry {
                    diff_fallback_enabled: true,
                    handle: closed_handle,
                    view: gpui::WeakEntity::new_invalid(),
                    main_pane: gpui::WeakEntity::new_invalid(),
                    documents: gpui::WeakEntity::new_invalid(),
                    view_mode: GitCometViewMode::Normal,
                    workspace_id: Some(saved_id),
                    repo_paths: Arc::from(Vec::new()),
                },
            );
        });
    });

    assert!(
        !cx.cx
            .update(|app| repository_move_target_is_noop(app, source_window_id, Some(saved_id))),
        "a closed workspace with the same repository is a valid move destination"
    );
}

#[gpui::test]
fn review_regression_forwarded_opens_activate_the_application_after_routing(
    cx: &mut gpui::TestAppContext,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, Vec::new());
        let window =
            open_gitcomet_window(app, Arc::clone(&backend), &normal_empty_launch_config(None));
        let existing = std::env::temp_dir().join("gitcomet-forwarded-existing");
        let preferred = std::env::temp_dir().join("gitcomet-forwarded-preferred");
        let new_window = std::env::temp_dir().join("gitcomet-forwarded-new");
        let scenarios = [
            (None, BrowserOpenTarget::ExistingWindow, None),
            (
                Some(existing.clone()),
                BrowserOpenTarget::ExistingWindow,
                None,
            ),
            (
                Some(preferred),
                BrowserOpenTarget::ExistingWindow,
                Some(window.window_id()),
            ),
            (Some(existing), BrowserOpenTarget::NewWindow, None),
            (Some(new_window), BrowserOpenTarget::NewWindow, None),
        ];
        for (path, target, preferred_window) in scenarios {
            let activated = Cell::new(false);
            handle_browser_open_request_and_activate(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: path.clone(),
                    target,
                },
                preferred_window,
                |app, ignoring_other_apps| {
                    assert!(
                        ignoring_other_apps,
                        "a forwarded open must foreground a backgrounded app"
                    );
                    let active = app
                        .active_window()
                        .expect("route to a window before activating the app");
                    if let Some(path) = path.as_ref() {
                        let registry = app.global::<GitCometWindowRegistry>();
                        let owner = registry
                            .windows
                            .get(&active.window_id())
                            .expect("active repository window");
                        assert!(
                            entry_contains_repo_path(owner, path),
                            "open the repository before activating the app"
                        );
                    }
                    activated.set(true);
                },
            );
            assert!(
                activated.get(),
                "every forwarded-open route must activate the application"
            );
        }
        assert_eq!(
            app.windows().len(),
            3,
            "every request targeting a new window creates one"
        );
    });
}

fn check_provisional_drop_window_locality(cx: &mut gpui::TestAppContext, succeeds: bool) {
    let _visual_guard = lock_visual_test();
    let (opened_tx, opened_rx) = mpsc::channel();
    let (result_tx, result_rx) = mpsc::channel();
    let backend: Arc<dyn GitBackend> = Arc::new(ControlledOpenBackend {
        opened: opened_tx,
        result: Mutex::new(result_rx),
    });
    let mut move_target = session::Workspace::new(Vec::new());
    move_target.custom_name = Some("Move target".to_string());
    let move_target_id = move_target.id;
    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, vec![move_target]);
        app.set_global(GitCometBackendGlobal(Arc::clone(&backend)));
    });
    let directory = tempfile::tempdir().expect("create repository directories");
    let root = normalize_repository_open_path(directory.path().to_path_buf());
    let base = root.join("base");
    let dropped = root.join("dropped");
    std::fs::create_dir(&base).unwrap();
    std::fs::create_dir(&dropped).unwrap();
    let repo_id = gitcomet_state::model::RepoId(51);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let mut repo = gitcomet_state::model::RepoState::new_opening(
        repo_id,
        gitcomet_core::domain::RepoSpec {
            workdir: base.clone(),
        },
    );
    repo.open = gitcomet_state::model::Loadable::Ready(());
    store.insert_repo_for_test(
        repo_id,
        Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
            base.clone(),
        )),
    );
    store.replace_snapshot_for_test(Arc::new(gitcomet_state::model::AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..gitcomet_state::model::AppState::test_default()
    }));
    let store_for_view = store.clone();
    let source =
        cx.add_window(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let (other_store, other_events) = AppStore::new_test(Arc::new(TestBackend));
    let other =
        cx.add_window(|window, cx| GitCometView::new(other_store, other_events, None, window, cx));

    // Check routing before the view receives the store's provisional tab,
    // then check it again while backend validation is deliberately blocked.
    for validating in [false, true] {
        if validating {
            assert_eq!(
                opened_rx
                    .recv_timeout(Duration::from_secs(3))
                    .expect("start validation"),
                dropped
            );
            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let snapshot = store.snapshot();
                if snapshot
                    .repos
                    .iter()
                    .any(|repo| repo.spec.workdir == dropped)
                {
                    let dropped_id = snapshot
                        .repos
                        .iter()
                        .find(|repo| repo.spec.workdir == dropped)
                        .unwrap()
                        .id;
                    assert!(
                        !session::snapshot_repos_from_state(&snapshot)
                            .open_repos
                            .iter()
                            .any(|path| session::path_from_storage_key(path) == dropped)
                    );
                    source
                        .update(cx, |view, _, cx| {
                            crate::view::test_support::apply_state_snapshot_for_test(
                                view, snapshot, cx,
                            );
                        })
                        .unwrap();
                    for target in [None, Some(move_target_id)] {
                        source
                            .update(cx, |view, _, cx| {
                                view.request_move_repo_to_workspace(
                                    dropped_id,
                                    dropped.clone(),
                                    target,
                                    cx,
                                );
                            })
                            .unwrap();
                        cx.run_until_parked();
                        cx.update(|app| {
                            // The deferred execution boundary also rejects a
                            // stale menu/confirmation trying to bypass the UI.
                            move_repository_to_workspace(
                                app,
                                source.window_id(),
                                dropped_id,
                                dropped.clone(),
                                target,
                            );
                            assert_eq!(
                                app.windows().len(),
                                2,
                                "validation must finish before moving a drop"
                            );
                            assert!(
                                crate::workspaces::workspace(app, move_target_id)
                                    .unwrap()
                                    .repositories
                                    .is_empty()
                            );
                        });
                    }
                    break;
                }
                assert!(Instant::now() < deadline, "publish the provisional tab");
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            source
                .update(cx, |view, _, cx| {
                    view.open_dropped_repo_locally(dropped.clone(), cx)
                })
                .unwrap();
        }

        other
            .update(cx, |view, _, cx| view.open_repo_path(dropped.clone(), cx))
            .unwrap();
        source
            .update(cx, |view, _, cx| {
                view.open_repo_path(dropped.clone(), cx);
                open_dropped_repository_from_view(cx, source.window_id(), dropped.clone());
            })
            .unwrap();
        cx.run_until_parked();
        cx.update(|app| {
            for window in [source.window_id(), other.window_id()] {
                handle_browser_open_request_with_window(
                    app,
                    Arc::clone(&backend),
                    BrowserOpenRequest {
                        path: Some(dropped.clone()),
                        target: BrowserOpenTarget::ExistingWindow,
                    },
                    Some(window),
                );
            }
            let owners: Vec<_> = gitcomet_window_entries(app)
                .into_iter()
                .filter(|entry| entry_contains_repo_path(entry, &dropped))
                .map(|entry| entry.handle.window_id())
                .collect();
            assert_eq!(owners.len(), 2);
            assert!(owners.contains(&source.window_id()));
            assert!(owners.contains(&other.window_id()));
            assert_eq!(
                app.windows().len(),
                2,
                "existing-window requests must reuse their destination window"
            );
            let workspace = crate::workspaces::workspace_for_window(app, source.window_id())
                .expect("original workspace");
            assert_eq!(
                workspace.repositories,
                vec![base.clone()],
                "unvalidated paths must stay out of saved workspaces"
            );
            assert_eq!(workspace.active_repository, Some(base.clone()));
            assert_eq!(
                crate::workspaces::workspace_for_window(app, other.window_id())
                    .expect("independent explicit open")
                    .repositories,
                vec![dropped.clone()]
            );
        });
    }

    result_tx.send(succeeds).expect("finish validation");
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let snapshot = store.snapshot();
        let finished = if succeeds {
            snapshot.repos.iter().any(|repo| {
                repo.spec.workdir == dropped
                    && matches!(repo.open, gitcomet_state::model::Loadable::Ready(()))
            })
        } else {
            snapshot
                .repo_open_failures
                .get(&dropped)
                .copied()
                .unwrap_or_default()
                > 0
        };
        if finished {
            source
                .update(cx, |view, _, cx| {
                    crate::view::test_support::apply_state_snapshot_for_test(view, snapshot, cx);
                })
                .unwrap();
            break;
        }
        assert!(Instant::now() < deadline, "finish the provisional open");
        std::thread::sleep(Duration::from_millis(10));
    }
    cx.update(|app| {
        let source_entry = normal_gitcomet_window_by_id(app, source.window_id()).unwrap();
        assert_eq!(entry_contains_repo_path(&source_entry, &dropped), succeeds);
        let other_entry = normal_gitcomet_window_by_id(app, other.window_id()).unwrap();
        assert!(entry_contains_repo_path(&other_entry, &dropped));
        let workspace = crate::workspaces::workspace_for_window(app, source.window_id())
            .expect("original workspace");
        assert_eq!(
            workspace.repositories,
            if succeeds {
                vec![base.clone(), dropped.clone()]
            } else {
                vec![base.clone()]
            }
        );
        assert_eq!(app.windows().len(), 2);
    });
    assert!(
        opened_rx.try_recv().is_err(),
        "reopening a provisional tab in the same window must not restart validation"
    );
}

#[gpui::test]
fn review_regression_provisional_drop_stays_window_local_until_validation_succeeds(
    cx: &mut gpui::TestAppContext,
) {
    check_provisional_drop_window_locality(cx, true);
}

#[gpui::test]
fn review_regression_failed_provisional_drop_does_not_remove_another_windows_copy(
    cx: &mut gpui::TestAppContext,
) {
    check_provisional_drop_window_locality(cx, false);
}

#[gpui::test]
fn review_regression_pending_new_window_opens_allow_the_same_repository(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let path = std::env::temp_dir().join("gitcomet-pending-new-window-owner");
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));

    cx.update(|app| {
        for _ in 0..2 {
            handle_browser_open_request(
                app,
                Arc::clone(&backend),
                BrowserOpenRequest {
                    path: Some(path.clone()),
                    target: BrowserOpenTarget::NewWindow,
                },
            );
        }

        assert_eq!(
            app.windows().len(),
            2,
            "new-window requests must open separate windows even while loading"
        );
        let owners = gitcomet_window_entries(app)
            .into_iter()
            .filter(|entry| entry_contains_repo_path(entry, &path))
            .count();
        assert_eq!(owners, 2, "each destination reserves its own pending tab");
    });
}

#[gpui::test]
fn review_regression_confirmed_failed_open_releases_pending_path(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(NotARepositoryBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let path = std::env::temp_dir().join("gitcomet-failed-pending-open");
    cx.cx.update(|app| {
        handle_browser_open_request(
            app,
            Arc::clone(&backend),
            BrowserOpenRequest {
                path: Some(path.clone()),
                target: BrowserOpenTarget::ExistingWindow,
            },
        );
        assert_eq!(
            gitcomet_window_entries(app)
                .iter()
                .filter(|entry| entry_contains_repo_path(entry, &path))
                .count(),
            1,
            "the path must be reserved while its open attempt is pending"
        );
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx);
            });
        });
        cx.run_until_parked();
        let released = cx.cx.update(|app| {
            gitcomet_window_entries(app)
                .iter()
                .all(|entry| !entry_contains_repo_path(entry, &path))
                && crate::workspaces::workspaces(app)
                    .iter()
                    .all(|workspace| !workspace.repositories.contains(&path))
        });
        if released && store.snapshot().repo_open_failure_revision > 0 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "a failed open must release its pending app-wide ownership"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui::test]
fn review_regression_confirmed_existing_window_open_waits_for_git_recovery(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::process::{
        GitExecutableAvailability, GitExecutablePreference, GitRuntimeState,
    };
    use gitcomet_state::model::AppState;

    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    store.replace_snapshot_for_test(Arc::new(AppState {
        git_runtime: GitRuntimeState {
            preference: GitExecutablePreference::Custom(PathBuf::new()),
            availability: GitExecutableAvailability::Unavailable {
                detail: "Git unavailable for broker routing test".to_string(),
            },
        },
        ..AppState::test_default()
    }));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let path = std::env::temp_dir().join("gitcomet-broker-open-after-git-recovery");
    cx.cx.update(|app| {
        handle_browser_open_request(
            app,
            Arc::clone(&backend),
            BrowserOpenRequest {
                path: Some(path.clone()),
                target: BrowserOpenTarget::ExistingWindow,
            },
        );
    });
    cx.run_until_parked();
    assert!(
        store.snapshot().repos.is_empty(),
        "the unavailable reducer must not start the repository open yet"
    );

    store.dispatch(Msg::SetGitRuntimeState(GitRuntimeState {
        preference: GitExecutablePreference::SystemPath,
        availability: GitExecutableAvailability::Available {
            version_output: "git version test".to_string(),
        },
    }));

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx);
            });
        });
        cx.run_until_parked();
        if store
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == path)
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the acknowledged broker request must resume once Git is available"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        store
            .snapshot()
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == path),
        "the acknowledged broker request must resume once Git is available"
    );
}

#[gpui::test]
fn review_regression_lifecycle_cold_start_activates_a_requested_inactive_saved_tab(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let initially_active = std::env::temp_dir().join("gitcomet-cold-start-initially-active");
    let requested = std::env::temp_dir().join("gitcomet-cold-start-requested-inactive");
    let mut saved = session::Workspace::new(vec![initially_active.clone(), requested.clone()]);
    saved.active_repository = Some(initially_active.clone());
    saved.restore_on_launch = true;
    saved.last_activation_order = 9;
    let saved_id = saved.id;
    let saved_paths = saved.repositories.clone();
    let launch = normal_empty_launch_config(None);

    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, vec![saved]);
        let window = open_gitcomet_window(app, Arc::clone(&backend), &launch);
        let owner = normal_gitcomet_window_by_id(app, window.window_id())
            .expect("normal window registry entry");
        sync_gitcomet_window_registry(
            app,
            owner.handle,
            owner.view.clone(),
            owner.main_pane.clone(),
            owner.documents.clone(),
            GitCometViewMode::Normal,
            Some(saved_id),
            saved_paths.into(),
        );
        owner
            .view
            .update(app, |view, _cx| {
                crate::view::test_support::dispatch_restore_session_for_test(
                    view,
                    vec![initially_active.clone(), requested.clone()],
                    Some(initially_active.clone()),
                );
            })
            .expect("queue saved tab restoration");
        handle_browser_open_request(
            app,
            Arc::clone(&backend),
            BrowserOpenRequest {
                path: Some(requested.clone()),
                target: BrowserOpenTarget::ExistingWindow,
            },
        );
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    let active = loop {
        cx.update(|app| {
            for entry in gitcomet_window_entries(app) {
                let _ = entry.view.update(app, |view, cx| {
                    crate::view::test_support::sync_store_snapshot(view, cx);
                });
            }
        });
        cx.run_until_parked();

        let startup = cx.update(|app| {
            find_normal_gitcomet_window_for_repo(app, &requested).and_then(|entry| {
                entry
                    .view
                    .read_with(app, |view, _cx| {
                        crate::view::test_support::startup_repository_state_for_test(view)
                    })
                    .ok()
            })
        });
        if let Some((false, 2, active)) = startup {
            break active;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for the saved repository tabs to restore");
        }
        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(
        active,
        Some(requested),
        "the startup request must win over the group's previously active tab"
    );
}

#[gpui::test]
fn review_regression_closing_immediately_flushes_the_current_workspace_layout(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| {
        crate::workspaces::initialize_for_test(app, Vec::new());
        ui_scale::set_default(app, 100);
    });
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view.clone());
    let workspace_id = cx
        .cx
        .update(|app| crate::workspaces::workspace_for_window(app, window_id))
        .expect("durable live group")
        .id;
    let width = px(517.0);
    let expected_width = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::set_sidebar_width_for_test(this, width, cx);
            ui_scale::stored_design_units(Some(
                ui_scale::UiScale::current(cx).design_units_from_pixels(width),
            ))
        })
    });

    cx.cx
        .update(|app| close_window_by_id_or_warn(app, window_id));
    cx.run_until_parked();

    let saved = cx
        .cx
        .update(|app| crate::workspaces::workspace(app, workspace_id))
        .expect("closed group remains recoverable");
    assert_eq!(saved.layout.sidebar_width, expected_width);
}

#[gpui::test]
fn review_regression_colliding_restored_window_frames_are_cascaded(cx: &mut gpui::TestAppContext) {
    let placement = session::PortableWindowPlacement {
        normal_frame: Some(session::SavedWindowFrame {
            x: 120,
            y: 90,
            width: 900,
            height: 650,
        }),
        captured_visible_frame: None,
        display_id: None,
        state: session::SavedWindowState::Windowed,
        tiled: None,
    };
    let fallback_size = size(px(900.0), px(650.0));
    let min_size = size(px(820.0), px(560.0));

    let first = cx
        .update(|app| restored_workspace_window_bounds(&placement, fallback_size, min_size, app).0);
    let _first_window = cx.update(|app| {
        app.open_window(
            WindowOptions {
                window_bounds: Some(first),
                ..Default::default()
            },
            |_window, cx| cx.new(|_| gpui::Empty),
        )
        .expect("open the first restored frame")
    });
    let second = cx
        .update(|app| restored_workspace_window_bounds(&placement, fallback_size, min_size, app).0);

    assert_ne!(
        first.get_bounds(),
        second.get_bounds(),
        "identical saved frames must not restore directly on top of one another"
    );
}

#[test]
fn window_zoom_action_restores_only_on_windows_when_already_maximized() {
    assert_eq!(window_zoom_action(false), WindowZoomAction::Zoom);

    let expected = if cfg!(target_os = "windows") {
        WindowZoomAction::Restore
    } else {
        WindowZoomAction::Zoom
    };
    assert_eq!(window_zoom_action(true), expected);
}

#[test]
fn window_menu_position_scales_logical_pixels_to_device_pixels() {
    assert_eq!(
        window_menu_position(point(px(12.4), px(7.6)), 1.25),
        (16, 10)
    );
}

#[test]
fn maximized_and_fullscreen_captures_keep_the_last_windowed_frame() {
    let previous_frame = session::SavedWindowFrame {
        x: 100,
        y: 80,
        width: 1100,
        height: 720,
    };
    let reported_screen_frame = session::SavedWindowFrame {
        x: 0,
        y: 0,
        width: 1920,
        height: 1080,
    };
    let previous = session::PortableWindowPlacement {
        normal_frame: Some(previous_frame),
        ..Default::default()
    };

    for state in [
        session::SavedWindowState::Maximized,
        session::SavedWindowState::Fullscreen,
    ] {
        assert_eq!(
            captured_normal_frame(state, reported_screen_frame, Some(&previous)),
            previous_frame
        );
    }
    assert_eq!(
        captured_normal_frame(
            session::SavedWindowState::Windowed,
            reported_screen_frame,
            Some(&previous)
        ),
        reported_screen_frame
    );
}

struct KeyBindingProbe {
    focus_handle: FocusHandle,
    key_context: Option<&'static str>,
    /// A context on an ancestor of the focused element, for bindings
    /// scoped like "Outer > Inner".
    outer_key_context: Option<&'static str>,
    observed_actions: Arc<Mutex<Vec<String>>>,
}

impl KeyBindingProbe {
    fn new(
        key_context: Option<&'static str>,
        observed_actions: Arc<Mutex<Vec<String>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            focus_handle: cx.focus_handle().tab_index(0).tab_stop(true),
            key_context,
            outer_key_context: None,
            observed_actions,
        }
    }

    fn nested(
        outer_key_context: &'static str,
        key_context: &'static str,
        observed_actions: Arc<Mutex<Vec<String>>>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            outer_key_context: Some(outer_key_context),
            ..Self::new(Some(key_context), observed_actions, cx)
        }
    }

    fn focus_handle(&self) -> FocusHandle {
        self.focus_handle.clone()
    }

    fn record_action(&self, action_name: &str) {
        self.observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(action_name.to_string());
    }
}

impl Render for KeyBindingProbe {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        macro_rules! record_action_listener {
            ($action:path) => {
                cx.listener(|this, _: &$action, _window, _cx| {
                    this.record_action($action.name());
                })
            };
        }

        let root = div()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_action(record_action_listener!(crate::kit::Backspace))
            .on_action(record_action_listener!(crate::kit::Delete))
            .on_action(record_action_listener!(crate::kit::DeleteWordLeft))
            .on_action(record_action_listener!(crate::kit::DeleteWordRight))
            .on_action(record_action_listener!(crate::kit::DeleteToLineStart))
            .on_action(record_action_listener!(crate::kit::DeleteToLineEnd))
            .on_action(record_action_listener!(crate::kit::Enter))
            .on_action(record_action_listener!(crate::kit::ShiftEnter))
            .on_action(record_action_listener!(crate::kit::Left))
            .on_action(record_action_listener!(crate::kit::Right))
            .on_action(record_action_listener!(crate::kit::Up))
            .on_action(record_action_listener!(crate::kit::Down))
            .on_action(record_action_listener!(crate::kit::WordLeft))
            .on_action(record_action_listener!(crate::kit::WordRight))
            .on_action(record_action_listener!(crate::kit::SelectLeft))
            .on_action(record_action_listener!(crate::kit::SelectRight))
            .on_action(record_action_listener!(crate::kit::SelectUp))
            .on_action(record_action_listener!(crate::kit::SelectDown))
            .on_action(record_action_listener!(crate::kit::SelectWordLeft))
            .on_action(record_action_listener!(crate::kit::SelectWordRight))
            .on_action(record_action_listener!(crate::kit::SelectAll))
            .on_action(record_action_listener!(crate::kit::Home))
            .on_action(record_action_listener!(crate::kit::DocumentHome))
            .on_action(record_action_listener!(crate::kit::DocumentEnd))
            .on_action(record_action_listener!(crate::kit::SelectHome))
            .on_action(record_action_listener!(crate::kit::End))
            .on_action(record_action_listener!(crate::kit::SelectEnd))
            .on_action(record_action_listener!(crate::kit::PageUp))
            .on_action(record_action_listener!(crate::kit::SelectPageUp))
            .on_action(record_action_listener!(crate::kit::PageDown))
            .on_action(record_action_listener!(crate::kit::SelectPageDown))
            .on_action(record_action_listener!(crate::kit::Paste))
            .on_action(record_action_listener!(crate::kit::Cut))
            .on_action(record_action_listener!(crate::kit::Copy))
            .on_action(record_action_listener!(crate::kit::Undo))
            .on_action(record_action_listener!(crate::kit::Redo))
            .on_action(record_action_listener!(crate::view::DiffPrevFile))
            .on_action(record_action_listener!(crate::view::DiffNextFile))
            .on_action(record_action_listener!(
                crate::view::DiffPrevSearchMatchOrChange
            ))
            .on_action(record_action_listener!(
                crate::view::DiffNextSearchMatchOrChange
            ))
            .on_action(record_action_listener!(crate::view::TextInputCommitSubmit))
            .on_action(record_action_listener!(crate::view::TextInputDiffPrevFile))
            .on_action(record_action_listener!(crate::view::TextInputDiffNextFile))
            .on_action(record_action_listener!(
                crate::view::TextInputDiffPrevSearchMatchOrChange
            ))
            .on_action(record_action_listener!(
                crate::view::TextInputDiffNextSearchMatchOrChange
            ))
            .on_action(record_action_listener!(
                crate::view::TextInputDiffPrevChange
            ))
            .on_action(record_action_listener!(
                crate::view::TextInputDiffNextChange
            ))
            .on_action(record_action_listener!(crate::view::OpenActiveViewSearch))
            .on_action(record_action_listener!(crate::view::HistoryFindPrevious))
            .on_action(record_action_listener!(crate::view::ToggleCommandPalette))
            .on_action(record_action_listener!(crate::view::ToggleRevealCommit))
            .on_action(record_action_listener!(crate::view::LocateFileInExplorer))
            .on_action(record_action_listener!(crate::view::OpenRemoteInBrowser))
            .on_action(record_action_listener!(NewWindow))
            .on_action(record_action_listener!(OpenSettings))
            .on_action(record_action_listener!(OpenInCodeEditor))
            .on_action(record_action_listener!(OpenRepository))
            .on_action(record_action_listener!(SwitchRepository))
            .on_action(record_action_listener!(ShowReflog))
            .on_action(record_action_listener!(Close))
            .on_action(record_action_listener!(CloseWindow))
            .on_action(record_action_listener!(PreviousRepository))
            .on_action(record_action_listener!(NextRepository))
            .on_action(record_action_listener!(TerminalCopy))
            .on_action(record_action_listener!(TerminalPaste))
            .on_action(record_action_listener!(TerminalSelectAll))
            .on_action(record_action_listener!(MinimizeWindow))
            .on_action(record_action_listener!(ZoomWindow))
            .on_action(record_action_listener!(ToggleFullScreen))
            .on_action(record_action_listener!(IncreaseUiScale))
            .on_action(record_action_listener!(DecreaseUiScale))
            .on_action(record_action_listener!(ResetUiScale))
            .on_action(record_action_listener!(Hide))
            .on_action(record_action_listener!(HideOthers))
            .on_action(record_action_listener!(ShowAll))
            .on_action(record_action_listener!(Quit));

        #[cfg(target_os = "macos")]
        let root = root.on_action(record_action_listener!(crate::kit::ShowCharacterPalette));

        let root = if let Some(key_context) = self.key_context {
            root.key_context(key_context)
        } else {
            root
        };
        match self.outer_key_context {
            Some(outer) => gpui::ParentElement::child(div().size_full().key_context(outer), root),
            None => root,
        }
    }
}

#[test]
fn focused_mergetool_title_uses_file_name_when_available() {
    let title = focused_mergetool_window_title(Path::new("/repo/src/conflict.txt"));
    assert_eq!(title, "GitComet - Mergetool (conflict.txt)");
}

#[test]
fn focused_mergetool_launch_config_sets_focused_view_mode_and_repo() {
    let config = FocusedMergetoolConfig {
        repo_path: PathBuf::from("/repo"),
        conflicted_file_path: PathBuf::from("/repo/src/conflict.txt"),
        label_local: "LOCAL".to_string(),
        label_remote: "REMOTE".to_string(),
        label_base: "BASE".to_string(),
    };

    let launch = focused_mergetool_launch_config(&config, None);
    assert_eq!(launch.app_id, "gitcomet-mergetool");
    assert_eq!(launch.title, "GitComet - Mergetool (conflict.txt)");
    assert_eq!(launch.view_config.initial_path, Some(config.repo_path));
    assert_eq!(
        launch.view_config.view_mode,
        GitCometViewMode::FocusedMergetool
    );
    assert_eq!(
        launch.view_config.focused_mergetool,
        Some(FocusedMergetoolViewConfig {
            repo_path: PathBuf::from("/repo"),
            conflicted_file_path: PathBuf::from("/repo/src/conflict.txt"),
            labels: FocusedMergetoolLabels {
                local: "LOCAL".to_string(),
                remote: "REMOTE".to_string(),
                base: "BASE".to_string(),
            },
        })
    );
    assert!(launch.view_config.focused_mergetool_exit_code.is_none());
}

#[test]
fn focused_mergetool_exit_codes_match_mergetool_contract() {
    assert_eq!(FOCUSED_MERGETOOL_EXIT_SUCCESS, 0);
    assert_eq!(FOCUSED_MERGETOOL_EXIT_CANCELED, 1);
    assert_eq!(FOCUSED_MERGETOOL_EXIT_ERROR, 2);
}

#[gpui::test]
fn text_input_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
    let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_window, cx| {
        KeyBindingProbe::new(Some("TextInput"), Arc::clone(&observed_actions), cx)
    });

    cx.update(|window, app| {
        app.clear_key_bindings();
        bind_text_input_keys(app);
        let focus = view.update(app, |view, _cx| view.focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    let cases: Vec<(&str, &'static str)> = vec![
        ("backspace", crate::kit::Backspace.name()),
        ("shift-backspace", crate::kit::Backspace.name()),
        ("delete", crate::kit::Delete.name()),
        ("ctrl-backspace", crate::kit::DeleteWordLeft.name()),
        ("ctrl-delete", crate::kit::DeleteWordRight.name()),
        ("alt-backspace", crate::kit::DeleteWordLeft.name()),
        ("alt-delete", crate::kit::DeleteWordRight.name()),
        ("cmd-backspace", crate::kit::DeleteToLineStart.name()),
        ("cmd-delete", crate::kit::DeleteToLineEnd.name()),
        ("ctrl-shift-backspace", crate::kit::DeleteToLineStart.name()),
        ("ctrl-shift-delete", crate::kit::DeleteToLineEnd.name()),
        ("enter", crate::kit::Enter.name()),
        ("shift-enter", crate::kit::ShiftEnter.name()),
        ("secondary-enter", crate::view::TextInputCommitSubmit.name()),
        ("f1", crate::view::TextInputDiffPrevFile.name()),
        ("f4", crate::view::TextInputDiffNextFile.name()),
        (
            "f2",
            crate::view::TextInputDiffPrevSearchMatchOrChange.name(),
        ),
        (
            "f3",
            crate::view::TextInputDiffNextSearchMatchOrChange.name(),
        ),
        ("shift-f7", crate::view::TextInputDiffPrevChange.name()),
        ("f7", crate::view::TextInputDiffNextChange.name()),
        ("alt-up", crate::view::TextInputDiffPrevChange.name()),
        ("alt-down", crate::view::TextInputDiffNextChange.name()),
        ("left", crate::kit::Left.name()),
        ("right", crate::kit::Right.name()),
        ("up", crate::kit::Up.name()),
        ("down", crate::kit::Down.name()),
        ("ctrl-left", crate::kit::WordLeft.name()),
        ("ctrl-right", crate::kit::WordRight.name()),
        ("ctrl-shift-left", crate::kit::SelectWordLeft.name()),
        ("ctrl-shift-right", crate::kit::SelectWordRight.name()),
        ("alt-left", crate::kit::WordLeft.name()),
        ("alt-right", crate::kit::WordRight.name()),
        ("alt-shift-left", crate::kit::SelectWordLeft.name()),
        ("alt-shift-right", crate::kit::SelectWordRight.name()),
        ("shift-left", crate::kit::SelectLeft.name()),
        ("shift-right", crate::kit::SelectRight.name()),
        ("shift-up", crate::kit::SelectUp.name()),
        ("shift-down", crate::kit::SelectDown.name()),
        ("home", crate::kit::Home.name()),
        ("ctrl-home", crate::kit::DocumentHome.name()),
        ("ctrl-end", crate::kit::DocumentEnd.name()),
        ("cmd-home", crate::kit::DocumentHome.name()),
        ("cmd-end", crate::kit::DocumentEnd.name()),
        ("shift-home", crate::kit::SelectHome.name()),
        ("end", crate::kit::End.name()),
        ("shift-end", crate::kit::SelectEnd.name()),
        ("cmd-left", crate::kit::Home.name()),
        ("cmd-shift-left", crate::kit::SelectHome.name()),
        ("cmd-right", crate::kit::End.name()),
        ("cmd-shift-right", crate::kit::SelectEnd.name()),
        ("pageup", crate::kit::PageUp.name()),
        ("shift-pageup", crate::kit::SelectPageUp.name()),
        ("pagedown", crate::kit::PageDown.name()),
        ("shift-pagedown", crate::kit::SelectPageDown.name()),
        ("cmd-a", crate::kit::SelectAll.name()),
        ("ctrl-a", crate::kit::SelectAll.name()),
        ("cmd-v", crate::kit::Paste.name()),
        ("ctrl-v", crate::kit::Paste.name()),
        ("cmd-c", crate::kit::Copy.name()),
        ("ctrl-c", crate::kit::Copy.name()),
        ("cmd-x", crate::kit::Cut.name()),
        ("ctrl-x", crate::kit::Cut.name()),
        ("cmd-z", crate::kit::Undo.name()),
        ("ctrl-z", crate::kit::Undo.name()),
        ("cmd-shift-z", crate::kit::Redo.name()),
        ("ctrl-shift-z", crate::kit::Redo.name()),
    ];

    #[cfg(target_os = "macos")]
    let cases = {
        let mut cases = cases;
        cases.push(("ctrl-cmd-space", crate::kit::ShowCharacterPalette.name()));
        cases
    };

    for (keystroke, expected_action) in cases {
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.simulate_keystrokes(keystroke);
        let actual_action = observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned();
        assert_eq!(
            actual_action.as_deref(),
            Some(expected_action),
            "expected `{keystroke}` to resolve to `{expected_action}`"
        );
    }
}

/// The history find bar's input steps back through matches on
/// Shift-Enter, independently of binding order. Other text inputs keep
/// `ShiftEnter`.
#[gpui::test]
fn history_find_text_input_shift_enter_resolves_to_previous_match(cx: &mut gpui::TestAppContext) {
    for (outer, expected) in [
        (Some("HistoryFind"), crate::view::HistoryFindPrevious.name()),
        (None, crate::kit::ShiftEnter.name()),
        // An unrelated ancestor context does not pick up the find binding.
        (Some("DiffSearch"), crate::kit::ShiftEnter.name()),
    ] {
        let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (view, cx) = cx.add_window_view(|_window, cx| match outer {
            Some(outer) => {
                KeyBindingProbe::nested(outer, "TextInput", Arc::clone(&observed_actions), cx)
            }
            None => KeyBindingProbe::new(Some("TextInput"), Arc::clone(&observed_actions), cx),
        });

        cx.update(|window, app| {
            app.clear_key_bindings();
            bind_app_keys(app);
            bind_text_input_keys(app);
            // The scopes must work independently of registration order.
            let reversed = app
                .key_bindings()
                .borrow()
                .bindings()
                .rev()
                .cloned()
                .collect::<Vec<_>>();
            app.clear_key_bindings();
            app.bind_keys(reversed);
            let focus = view.update(app, |view, _cx| view.focus_handle());
            window.focus(&focus, app);
            let _ = window.draw(app);
        });

        for (keystroke, expected_action) in [
            ("shift-enter", expected),
            // Plain Enter stays the text input's own action everywhere.
            ("enter", crate::kit::Enter.name()),
        ] {
            observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clear();
            cx.simulate_keystrokes(keystroke);
            let actual_action = observed_actions
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .last()
                .cloned();
            assert_eq!(
                actual_action.as_deref(),
                Some(expected_action),
                "expected `{keystroke}` under {outer:?} > TextInput to resolve to `{expected_action}`"
            );
        }
    }
}

#[gpui::test]
fn text_input_diff_keybindings_stay_scoped_when_app_keys_are_installed(
    cx: &mut gpui::TestAppContext,
) {
    let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_window, cx| {
        KeyBindingProbe::new(Some("TextInput"), Arc::clone(&observed_actions), cx)
    });

    cx.update(|window, app| {
        app.clear_key_bindings();
        bind_app_keys(app);
        bind_text_input_keys(app);
        let focus = view.update(app, |view, _cx| view.focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    let cases = [
        ("f1", crate::view::TextInputDiffPrevFile.name()),
        ("f4", crate::view::TextInputDiffNextFile.name()),
        (
            "f2",
            crate::view::TextInputDiffPrevSearchMatchOrChange.name(),
        ),
        (
            "f3",
            crate::view::TextInputDiffNextSearchMatchOrChange.name(),
        ),
        ("secondary-shift-a", SwitchRepository.name()),
        // No text-input binding (an Emacs kill-line, say) may shadow it.
        ("secondary-k", crate::view::OpenRemoteInBrowser.name()),
    ];

    for (keystroke, expected_action) in cases {
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.simulate_keystrokes(keystroke);
        let actual_actions = observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        assert_eq!(
            actual_actions,
            vec![expected_action.to_string()],
            "expected `{keystroke}` to resolve only to the TextInput-scoped diff action"
        );
    }
}

#[gpui::test]
fn terminal_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
    let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_window, cx| {
        KeyBindingProbe::new(Some("Terminal"), Arc::clone(&observed_actions), cx)
    });

    cx.update(|window, app| {
        app.clear_key_bindings();
        bind_app_keys(app);
        bind_terminal_keys_for_test(app);
        let focus = view.update(app, |view, _cx| view.focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    #[cfg(target_os = "macos")]
    let cases = [
        ("cmd-c", TerminalCopy.name()),
        ("cmd-v", TerminalPaste.name()),
        ("cmd-a", TerminalSelectAll.name()),
    ];

    #[cfg(not(target_os = "macos"))]
    let cases = [
        ("ctrl-shift-c", TerminalCopy.name()),
        ("ctrl-shift-v", TerminalPaste.name()),
        ("secondary-shift-a", TerminalSelectAll.name()),
    ];

    for (keystroke, expected_action) in cases {
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.simulate_keystrokes(keystroke);
        let actual_action = observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned();
        assert_eq!(
            actual_action.as_deref(),
            Some(expected_action),
            "expected `{keystroke}` to resolve to `{expected_action}`"
        );
    }

    for keystroke in ["ctrl-c", "ctrl-v", "ctrl-a"] {
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.simulate_keystrokes(keystroke);
        let actual_actions = observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        assert!(
            actual_actions.is_empty(),
            "expected `{keystroke}` to remain shell input, got {actual_actions:?}"
        );
    }
}

#[gpui::test]
fn text_input_command_shortcuts_trigger_undo_and_redo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        crate::kit::TextInput::new(crate::kit::TextInputOptions::default(), window, cx)
    });

    cx.update(|window, app| {
        app.clear_key_bindings();
        bind_text_input_keys(app);
        let focus = input.read(app).focus_handle();
        window.focus(&focus, app);

        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("cmd-z");
    assert_eq!(
        cx.update(|_window, app| input.read(app).text().to_string()),
        "alpha"
    );

    cx.simulate_keystrokes("cmd-shift-z");
    assert_eq!(
        cx.update(|_window, app| input.read(app).text().to_string()),
        "beta"
    );
}

#[gpui::test]
fn text_input_control_redo_shortcut_triggers_redo(cx: &mut gpui::TestAppContext) {
    let (input, cx) = cx.add_window_view(|window, cx| {
        crate::kit::TextInput::new(crate::kit::TextInputOptions::default(), window, cx)
    });

    cx.update(|window, app| {
        app.clear_key_bindings();
        bind_text_input_keys(app);
        let focus = input.read(app).focus_handle();
        window.focus(&focus, app);

        input.update(app, |input, cx| {
            input.set_text("alpha", cx);
            let inserted = input.replace_utf8_range(0..5, "beta", cx);
            assert_eq!(inserted, 0..4);
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("ctrl-z");
    assert_eq!(
        cx.update(|_window, app| input.read(app).text().to_string()),
        "alpha"
    );

    cx.simulate_keystrokes("ctrl-shift-z");
    assert_eq!(
        cx.update(|_window, app| input.read(app).text().to_string()),
        "beta"
    );
}

#[test]
fn should_quit_when_all_windows_closed_depends_on_launch_mode() {
    let normal = normal_launch_config(None, None);
    let focused = focused_mergetool_launch_config(
        &FocusedMergetoolConfig {
            repo_path: PathBuf::from("/repo"),
            conflicted_file_path: PathBuf::from("/repo/conflict.txt"),
            label_local: "LOCAL".to_string(),
            label_remote: "REMOTE".to_string(),
            label_base: "BASE".to_string(),
        },
        None,
    );

    #[cfg(target_os = "macos")]
    assert!(!should_quit_when_all_windows_closed(&normal));
    #[cfg(not(target_os = "macos"))]
    assert!(should_quit_when_all_windows_closed(&normal));
    assert!(should_quit_when_all_windows_closed(&focused));
}

#[test]
fn normal_launch_config_keeps_startup_paths_in_restore_session_mode() {
    let launch = normal_launch_config(Some(PathBuf::from("/repo")), None);

    assert_eq!(
        launch.view_config.initial_path,
        Some(PathBuf::from("/repo"))
    );
    assert_eq!(
        launch.view_config.initial_repository_launch_mode,
        InitialRepositoryLaunchMode::RestoreSession
    );
}

#[test]
fn explicit_repository_launch_config_marks_initial_path_as_explicit() {
    let launch = normal_launch_config_with_initial_repository(PathBuf::from("/repo"), None);

    assert_eq!(
        launch.view_config.initial_path,
        Some(PathBuf::from("/repo"))
    );
    assert_eq!(
        launch.view_config.initial_repository_launch_mode,
        InitialRepositoryLaunchMode::OpenExplicitly
    );
}

#[test]
fn recent_repository_label_formats_repo_name_and_parent() {
    let label = recent_repository_label(Path::new("/Users/sampo/projects/gitcomet"));
    assert_eq!(label, "gitcomet - /Users/sampo/projects");
}

#[test]
fn recent_repository_label_falls_back_to_display_when_file_name_is_missing() {
    let path = PathBuf::from(std::path::MAIN_SEPARATOR.to_string());
    assert_eq!(recent_repository_label(&path), path.display().to_string());
}

#[gpui::test]
fn app_keybindings_resolve_expected_actions(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_window, cx| {
        KeyBindingProbe::new(None, Arc::clone(&observed_actions), cx)
    });

    cx.update(|window, app| {
        app.clear_key_bindings();
        bind_app_keys(app);
        let focus = view.update(app, |view, _cx| view.focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    let mut cases = vec![
        ("secondary-n", NewWindow.name()),
        ("secondary-shift-n", NewWindow.name()),
        ("secondary-,", OpenSettings.name()),
        ("secondary-o", OpenRepository.name()),
        ("secondary-shift-o", SwitchRepository.name()),
        ("secondary-shift-a", SwitchRepository.name()),
        ("secondary-f", crate::view::OpenActiveViewSearch.name()),
        ("secondary-p", crate::view::ToggleCommandPalette.name()),
        ("secondary-g", crate::view::ToggleRevealCommit.name()),
        (
            "secondary-shift-l",
            crate::view::LocateFileInExplorer.name(),
        ),
        ("secondary-k", crate::view::OpenRemoteInBrowser.name()),
        ("secondary-w", Close.name()),
        ("secondary-shift-w", CloseWindow.name()),
        ("secondary-pageup", PreviousRepository.name()),
        ("secondary-pagedown", NextRepository.name()),
        ("secondary-+", IncreaseUiScale.name()),
        ("secondary-=", IncreaseUiScale.name()),
        ("secondary--", DecreaseUiScale.name()),
        ("secondary-0", ResetUiScale.name()),
        ("secondary-q", Quit.name()),
        ("f1", crate::view::DiffPrevFile.name()),
        ("f4", crate::view::DiffNextFile.name()),
        ("f2", crate::view::DiffPrevSearchMatchOrChange.name()),
        ("f3", crate::view::DiffNextSearchMatchOrChange.name()),
    ];

    #[cfg(target_os = "macos")]
    cases.extend([
        ("alt-cmd-o", SwitchRepository.name()),
        ("cmd-{", PreviousRepository.name()),
        ("alt-cmd-left", PreviousRepository.name()),
        ("cmd-}", NextRepository.name()),
        ("alt-cmd-right", NextRepository.name()),
        ("cmd-m", MinimizeWindow.name()),
        ("ctrl-cmd-f", ToggleFullScreen.name()),
        ("cmd-h", Hide.name()),
        ("alt-cmd-h", HideOthers.name()),
    ]);

    #[cfg(not(target_os = "macos"))]
    cases.extend([
        ("ctrl-shift-tab", PreviousRepository.name()),
        ("ctrl-tab", NextRepository.name()),
        ("f11", ToggleFullScreen.name()),
    ]);

    for (keystroke, expected_action) in cases {
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        cx.simulate_keystrokes(keystroke);
        let actual_action = observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last()
            .cloned();
        assert_eq!(
            actual_action.as_deref(),
            Some(expected_action),
            "expected `{keystroke}` to resolve to `{expected_action}`"
        );
    }

    observed_actions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    cx.simulate_keystrokes("secondary-shift-e");
    assert!(
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty(),
        "expected external editor shortcut to be unbound by default"
    );
}

#[gpui::test]
fn external_editor_shortcut_respects_configured_setting_override(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    let observed_actions: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let (view, cx) = cx.add_window_view(|_window, cx| {
        KeyBindingProbe::new(None, Arc::clone(&observed_actions), cx)
    });

    cx.update(|window, app| {
        let focus = view.update(app, |view, _cx| view.focus_handle());
        window.focus(&focus, app);
        let _ = window.draw(app);
    });

    crate::external_editor::set_configured_setting_override(Some(
        session::ExternalCodeEditorSetting::Custom {
            executable: PathBuf::from("/usr/bin/editor"),
            arguments: None,
        },
    ));
    cx.update(|_window, app| {
        app.clear_key_bindings();
        bind_app_keys(app);
    });

    cx.simulate_keystrokes("secondary-shift-e");
    let actual_action = observed_actions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .last()
        .cloned();
    assert_eq!(actual_action.as_deref(), Some(OpenInCodeEditor.name()));

    crate::external_editor::set_configured_setting_override(None);
    observed_actions
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clear();
    cx.update(|_window, app| {
        bind_app_keys(app);
    });

    cx.simulate_keystrokes("secondary-shift-e");
    assert!(
        observed_actions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .is_empty(),
        "expected external editor shortcut to be unbound after the configured override is cleared"
    );
}

#[gpui::test]
fn settings_shortcut_opens_a_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });
    seed_worktree_repo(cx, &store, view);

    assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
    cx.simulate_keystrokes("secondary-,");
    cx.run_until_parked();
    assert_eq!(cx.update(|_window, app| app.windows().len()), 2);
}

#[gpui::test]
fn settings_shortcut_reuses_existing_window_and_activates_it(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let main_window = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle()
    });
    let main_window_id = main_window.window_id();

    cx.simulate_keystrokes("secondary-,");
    cx.run_until_parked();

    let settings_window_id = cx.cx.update(|app| {
        assert_eq!(app.windows().len(), 2);
        app.windows()
            .into_iter()
            .map(|window| window.window_id())
            .find(|window_id| *window_id != main_window_id)
            .expect("expected the settings window to be open")
    });

    cx.cx.update(|app| {
        let _ = main_window.update(app, |_, window, _cx| {
            window.activate();
        });
        assert_eq!(
            app.active_window().map(|window| window.window_id()),
            Some(main_window_id),
            "expected the main window to become active before reopening settings"
        );
    });

    cx.simulate_keystrokes("secondary-,");
    cx.run_until_parked();

    cx.cx.update(|app| {
        assert_eq!(app.windows().len(), 2);
        assert_eq!(
            app.active_window().map(|window| window.window_id()),
            Some(settings_window_id),
            "expected reopening settings to activate the existing settings window"
        );
    });
}

#[gpui::test]
fn recent_picker_shortcut_toggles_the_popover(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });
    seed_worktree_repo(cx, &store, view);

    cx.simulate_keystrokes("secondary-shift-a");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(cx.debug_bounds("app_popover").is_some());

    cx.simulate_keystrokes("secondary-shift-a");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("app_popover").is_none(),
        "pressing the shortcut again should close the repository picker"
    );
}

#[gpui::test]
fn new_window_shortcuts_open_new_windows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });
    seed_worktree_repo(cx, &store, view);

    assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();
    assert_eq!(cx.update(|_window, app| app.windows().len()), 2);
    let mut repo_counts = cx.cx.update(|app| {
        gitcomet_window_entries(app)
            .into_iter()
            .map(|entry| entry.repo_paths.len())
            .collect::<Vec<_>>()
    });
    repo_counts.sort_unstable();
    assert_eq!(
        repo_counts,
        vec![0, 1],
        "a new window must not copy repositories from the active window"
    );

    cx.simulate_keystrokes("secondary-shift-n");
    cx.run_until_parked();
    assert_eq!(cx.update(|_window, app| app.windows().len()), 3);
    let mut repo_counts = cx.cx.update(|app| {
        gitcomet_window_entries(app)
            .into_iter()
            .map(|entry| entry.repo_paths.len())
            .collect::<Vec<_>>()
    });
    repo_counts.sort_unstable();
    assert_eq!(repo_counts, vec![0, 0, 1]);
}

fn empty_window_for_adoption(
    cx: &mut gpui::TestAppContext,
    workspaces: Vec<session::Workspace>,
) -> (
    gpui::Entity<GitCometView>,
    &mut gpui::VisualTestContext,
    gpui::WindowId,
) {
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let window_id = cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, workspaces);
        install_app_shortcuts_for_test(app, backend);
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            crate::view::test_support::sync_store_snapshot(view, cx)
        });
    });
    (view, cx, window_id)
}

#[gpui::test]
fn adopting_an_overlapping_workspace_reuses_the_empty_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repositories = vec![PathBuf::from("/tmp/adopt-a"), PathBuf::from("/tmp/adopt-b")];
    let mut workspace = session::Workspace::new(repositories.clone());
    workspace.restore_on_launch = false;
    let id = workspace.id;
    let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace]);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let other = cx
        .cx
        .add_window(|window, cx| GitCometView::new(store, events, None, window, cx));
    other
        .update(&mut cx.cx, |view, _, cx| {
            view.open_repo_path_locally(repositories[0].clone(), cx)
        })
        .unwrap();

    cx.update(|_window, app| open_workspace_in_window(app, window_id, id));

    cx.cx.update(|app| {
        assert_eq!(
            app.windows().len(),
            2,
            "adoption must reuse the empty window"
        );
        let adopted = crate::workspaces::workspace_for_window(app, window_id)
            .expect("the window now belongs to the workspace");
        assert_eq!(adopted.id, id);
        // The store has not reduced the restore yet; the snapshot taken
        // during adoption must not have emptied (and so deleted) it.
        assert_eq!(adopted.repositories, repositories);
        assert!(adopted.restore_on_launch);
        assert_eq!(windows_owning_repo_for_test(app, &repositories[0]).len(), 2);
    });
}

#[gpui::test]
fn opening_a_workspace_owned_by_another_window_leaves_this_window_empty(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let mut workspace = session::Workspace::new(vec![PathBuf::from("/tmp/adopt-owned")]);
    // Test views do not auto-restore repositories; a name keeps the owner's
    // workspace alive while its window stays empty.
    workspace.custom_name = Some("Owned".to_string());
    let id = workspace.id;
    let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace.clone()]);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let config = crate::view::GitCometViewConfig {
        workspace: WorkspaceBootstrap::Saved(Box::new(workspace)),
        ..crate::view::GitCometViewConfig::normal(None)
    };
    let owner = cx
        .cx
        .add_window(|window, cx| GitCometView::new_with_config(store, events, config, window, cx));
    cx.cx.update(|app| {
        let any_owner: gpui::AnyWindowHandle = owner.into();
        let _ = any_owner.update(app, |_root, window, cx| {
            let _ = window.draw(cx);
        });
        let _ = owner.update(app, |view, _window, cx| {
            crate::view::test_support::sync_store_snapshot(view, cx);
        });
    });
    cx.run_until_parked();
    assert_eq!(
        cx.cx.update(|app| {
            normal_gitcomet_window_by_id(app, owner.window_id()).and_then(|e| e.workspace_id)
        }),
        Some(id),
        "the owner window registers the workspace"
    );

    cx.update(|_window, app| open_workspace_in_window(app, window_id, id));

    cx.cx.update(|app| {
        assert!(crate::workspaces::workspace_for_window(app, window_id).is_none());
        assert_eq!(
            crate::workspaces::workspace_for_window(app, owner.window_id()).map(|w| w.id),
            Some(id)
        );
    });
}

#[gpui::test]
fn adopting_an_empty_customized_workspace_keeps_the_window_on_home(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let mut workspace = session::Workspace::new(Vec::new());
    workspace.custom_name = Some("Later".to_string());
    workspace.restore_on_launch = false;
    let id = workspace.id;
    let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace]);

    cx.update(|_window, app| open_workspace_in_window(app, window_id, id));

    cx.cx.update(|app| {
        assert_eq!(app.windows().len(), 1);
        let adopted = crate::workspaces::workspace_for_window(app, window_id).expect("adopted");
        assert_eq!(adopted.id, id);
        assert!(adopted.repositories.is_empty());
    });
}

#[gpui::test]
fn closing_the_active_window_saves_a_layout_change_the_debounce_has_not_written(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let mut workspace = session::Workspace::new(Vec::new());
    workspace.custom_name = Some("Layout".to_string());
    let id = workspace.id;
    let (view, cx, window_id) = empty_window_for_adoption(cx, vec![workspace]);
    cx.update(|window, app| {
        open_workspace_in_window(app, window_id, id);
        window.activate();
    });
    cx.update(|_window, app| {
        view.update(app, |view, cx| {
            crate::view::test_support::set_sidebar_width_for_test(view, px(333.0), cx);
        });
    });

    cx.update(|_window, app| close_active_window(app));

    let saved = cx
        .cx
        .update(|app| crate::workspaces::workspace(app, id))
        .expect("a customized workspace outlives its window");
    assert_eq!(saved.layout.sidebar_width, Some(333));
    assert_eq!(
        saved.restore_on_launch,
        !cfg!(target_os = "macos"),
        "closing the only window quits off macOS, so only there is it restored"
    );
}

#[gpui::test]
fn deleting_a_workspace_closes_its_window_when_another_window_remains(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let mut alpha = session::Workspace::new(Vec::new());
    alpha.custom_name = Some("Alpha".into());
    let alpha_id = alpha.id;
    let (view, cx, window_id) = empty_window_for_adoption(cx, vec![alpha.clone()]);
    cx.update(|_window, app| view.update(app, |view, cx| view.adopt_workspace(alpha, cx)));
    cx.run_until_parked();
    cx.cx.update(open_new_empty_window);
    cx.run_until_parked();
    let other = cx.cx.update(|app| {
        app.windows()
            .into_iter()
            .map(|window| window.window_id())
            .find(|id| *id != window_id)
            .expect("a second window")
    });

    cx.cx.update(|app| delete_workspace(app, alpha_id));
    cx.run_until_parked();

    cx.cx.update(|app| {
        let open: Vec<_> = app
            .windows()
            .iter()
            .map(|window| window.window_id())
            .collect();
        assert_eq!(
            open,
            vec![other],
            "only the deleted workspace's window closes"
        );
        assert!(crate::workspaces::workspace(app, alpha_id).is_none());
    });
}

#[gpui::test]
fn deleting_the_last_windows_workspace_returns_it_to_home(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let window_id = cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, Vec::new());
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view.clone());
    let workspace_id = cx.cx.update(|app| {
        crate::workspaces::workspace_for_window(app, window_id)
            .expect("the seeded repository makes the window durable")
            .id
    });
    cx.cx
        .update(|app| crate::workspaces::set_workspace_name(app, workspace_id, "Alpha"));

    cx.cx.update(|app| delete_workspace(app, workspace_id));
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.update(|window, app| {
            view.update(app, |view, cx| {
                crate::view::test_support::sync_store_snapshot(view, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        let emptied = cx.cx.update(|app| {
            normal_gitcomet_window_by_id(app, window_id)
                .is_some_and(|entry| entry.repo_paths.is_empty())
        });
        if emptied {
            break;
        }
        assert!(Instant::now() < deadline, "the window's repositories close");
        std::thread::sleep(Duration::from_millis(10));
    }

    cx.cx.update(|app| {
        assert!(
            app.windows()
                .iter()
                .any(|window| window.window_id() == window_id),
            "the last window stays open on Home"
        );
        // Still gone once the repositories have closed, even customized.
        assert!(crate::workspaces::workspace(app, workspace_id).is_none());
        assert!(crate::workspaces::workspace_for_window(app, window_id).is_none());
        assert!(crate::workspaces::workspaces(app).is_empty());
        assert_eq!(
            normal_gitcomet_window_by_id(app, window_id).and_then(|entry| entry.workspace_id),
            None
        );
    });
}

#[gpui::test]
fn deleting_a_saved_workspace_only_forgets_it(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let mut saved = session::Workspace::new(vec![PathBuf::from("/tmp/saved-workspace")]);
    saved.restore_on_launch = false;
    let saved_id = saved.id;
    let (_view, cx, window_id) = empty_window_for_adoption(cx, vec![saved]);

    cx.cx.update(|app| delete_workspace(app, saved_id));
    cx.run_until_parked();

    cx.cx.update(|app| {
        assert!(crate::workspaces::workspace(app, saved_id).is_none());
        let open: Vec<_> = app
            .windows()
            .iter()
            .map(|window| window.window_id())
            .collect();
        assert_eq!(open, vec![window_id], "no window owned it");
    });
}

#[gpui::test]
fn moving_the_last_repository_out_of_a_customized_workspace_keeps_the_window(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let source_window_id = cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, Vec::new());
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view.clone());
    let snapshot = store.snapshot();
    let repo = snapshot.repos.first().expect("source repo");
    let (repo_id, path) = (repo.id, repo.spec.workdir.clone());
    let source_workspace = cx.cx.update(|app| {
        crate::workspaces::workspace_for_window(app, source_window_id)
            .expect("the seeded repository makes the window durable")
            .id
    });
    cx.cx
        .update(|app| crate::workspaces::set_workspace_name(app, source_workspace, "Alpha"));

    cx.update(|_window, app| {
        view.update(app, |_view, cx| {
            move_repository_to_workspace_from_view(
                cx,
                source_window_id,
                repo_id,
                path.clone(),
                None,
            );
        });
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.cx.update(|app| {
            for entry in gitcomet_window_entries(app) {
                let _ = entry.view.update(app, |view, cx| {
                    crate::view::test_support::sync_store_snapshot(view, cx);
                });
            }
        });
        cx.run_until_parked();
        let settled = cx.cx.update(|app| {
            let entries = gitcomet_window_entries(app);
            entries.iter().any(|entry| {
                entry.handle.window_id() != source_window_id
                    && entry.repo_paths.as_ref() == [path.clone()]
            }) && entries.iter().any(|entry| {
                entry.handle.window_id() == source_window_id && entry.repo_paths.is_empty()
            })
        });
        if settled {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for the repository to leave the customized workspace"
        );
        std::thread::sleep(Duration::from_millis(10));
    }

    let (source_open, kept) = cx.cx.update(|app| {
        (
            app.windows()
                .iter()
                .any(|window| window.window_id() == source_window_id),
            crate::workspaces::workspace_for_window(app, source_window_id),
        )
    });
    assert!(
        source_open,
        "a customized workspace keeps its window on Home"
    );
    let kept = kept.expect("the window still belongs to its workspace");
    assert_eq!(kept.id, source_workspace);
    assert_eq!(kept.custom_name.as_deref(), Some("Alpha"));
    assert!(kept.repositories.is_empty());
}

#[gpui::test]
fn moving_the_only_repository_to_a_new_window_closes_the_source(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let source_window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view.clone());
    let snapshot = store.snapshot();
    let repo = snapshot.repos.first().expect("source repo");
    let (repo_id, path) = (repo.id, repo.spec.workdir.clone());

    cx.update(|_window, app| {
        view.update(app, |_view, cx| {
            move_repository_to_workspace_from_view(
                cx,
                source_window_id,
                repo_id,
                path.clone(),
                None,
            );
        });
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        cx.cx.update(|app| {
            for entry in gitcomet_window_entries(app) {
                let _ = entry.view.update(app, |view, cx| {
                    crate::view::test_support::sync_store_snapshot(view, cx);
                });
            }
        });
        cx.run_until_parked();
        let moved = cx.cx.update(|app| {
            app.windows().len() == 1
                && gitcomet_window_entries(app)
                    .iter()
                    .any(|entry| entry.repo_paths.as_ref() == [path.clone()])
        });
        if moved {
            break;
        }
        if Instant::now() >= deadline {
            let diagnostics = cx.cx.update(|app| {
                let entries = gitcomet_window_entries(app)
                    .into_iter()
                    .map(|entry| format!("{:?}: {:?}", entry.handle.window_id(), entry.repo_paths))
                    .collect::<Vec<_>>();
                format!("{} windows; registry: {entries:?}", app.windows().len())
            });
            panic!(
                "timed out waiting for the repository to move to its new window ({diagnostics})"
            );
        }
        std::thread::sleep(Duration::from_millis(10));
    }

    assert!(
        cx.cx.update(|app| {
            app.windows()
                .iter()
                .all(|window| window.window_id() != source_window_id)
        }),
        "moving the final tab should remove the now-empty source window"
    );
}

#[gpui::test]
fn review_regression_confirmed_final_tab_move_reserves_durable_target_first(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let source_window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    seed_worktree_repo(cx, &store, view);
    let snapshot = store.snapshot();
    let repo = snapshot.repos.first().expect("source repo");
    let (repo_id, path) = (repo.id, repo.spec.workdir.clone());

    cx.cx.update(|app| {
        move_repository_to_workspace(app, source_window_id, repo_id, path.clone(), None);

        assert!(
            crate::workspaces::workspaces(app)
                .iter()
                .any(|workspace| workspace.repositories.contains(&path)),
            "durable target ownership must exist before the source group is discarded"
        );
        assert_eq!(
            gitcomet_window_entries(app)
                .into_iter()
                .filter(|entry| entry_contains_repo_path(entry, &path))
                .count(),
            1,
            "the target reservation must replace the source as the sole owner"
        );
    });
}

#[gpui::test]
fn close_button_closes_only_the_clicked_window_after_opening_a_new_window(
    cx: &mut gpui::TestAppContext,
) {
    if cfg!(target_os = "macos") {
        // The custom Min/Max/Close controls are only rendered on non-macOS.
        return;
    }

    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let first_window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });

    cx.simulate_keystrokes("secondary-n");
    cx.run_until_parked();

    let second_window_id = cx.cx.update(|app| {
        assert_eq!(app.windows().len(), 2, "expected two GitComet windows");
        app.windows()
            .into_iter()
            .map(|window| window.window_id())
            .find(|window_id| *window_id != first_window_id)
            .expect("expected the new window to remain open")
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let close_bounds = cx
        .debug_bounds("titlebar_win_close")
        .expect("expected titlebar close control bounds");
    cx.simulate_mouse_move(close_bounds.center(), None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        close_bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        close_bounds.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.run_until_parked();

    cx.cx.update(|app| {
        let remaining_windows = app.windows();
        assert_eq!(
            remaining_windows.len(),
            1,
            "expected the close control to remove only the clicked window"
        );
        assert_eq!(
            remaining_windows[0].window_id(),
            second_window_id,
            "expected the new window to remain open after closing the original window"
        );
    });
}

#[gpui::test]
fn close_shortcut_closes_the_active_window_when_no_repo_tab_can_close(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });

    assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert_eq!(cx.cx.update(|app| app.windows().len()), 0);
}

#[gpui::test]
fn close_window_shortcut_closes_the_active_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });

    assert_eq!(cx.update(|_window, app| app.windows().len()), 1);
    cx.simulate_keystrokes("secondary-shift-w");
    cx.run_until_parked();
    assert_eq!(cx.cx.update(|app| app.windows().len()), 0);
}

#[cfg(not(target_os = "macos"))]
#[gpui::test]
fn ctrl_tab_shortcuts_cycle_repository_tabs_in_the_main_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let repos = vec![
        PathBuf::from("/tmp/gitcomet-app-test-repo-1"),
        PathBuf::from("/tmp/gitcomet-app-test-repo-2"),
        PathBuf::from("/tmp/gitcomet-app-test-repo-3"),
    ];

    cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });

    store.dispatch(Msg::RestoreSession {
        open_repos: repos.clone(),
        active_repo: repos.first().cloned(),
    });

    let deadline = Instant::now() + Duration::from_secs(3);
    let repo_ids = loop {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::view::test_support::sync_store_snapshot(this, cx)
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let snapshot = store.snapshot();
        if snapshot.repos.len() == repos.len()
            && !view.read_with(&cx.cx, |view, _| view.blocks_non_repository_actions())
        {
            break snapshot
                .repos
                .iter()
                .map(|repo| repo.id)
                .collect::<Vec<_>>();
        }

        if Instant::now() >= deadline {
            panic!("timed out waiting for restored repository tabs to become interactive");
        }

        std::thread::sleep(Duration::from_millis(10));
    };

    assert_eq!(store.snapshot().active_repo, Some(repo_ids[0]));

    cx.simulate_keystrokes("ctrl-tab");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    assert_eq!(store.snapshot().active_repo, Some(repo_ids[1]));

    cx.simulate_keystrokes("ctrl-shift-tab");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    assert_eq!(store.snapshot().active_repo, Some(repo_ids[0]));

    cx.simulate_keystrokes("ctrl-shift-tab");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx)
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    assert_eq!(store.snapshot().active_repo, Some(repo_ids[2]));
}

#[gpui::test]
fn repository_picker_fallback_reuses_existing_normal_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        window.activate();
    });

    assert_eq!(cx.cx.update(|app| app.windows().len()), 1);
    cx.cx.update(|app| {
        show_open_repository_manual_entry_in_existing_or_new_window(app, Arc::clone(&backend));
    });
    cx.run_until_parked();

    assert_eq!(cx.cx.update(|app| app.windows().len()), 1);
    cx.update(|_window, app| {
        assert!(crate::view::test_support::open_repo_panel_visible(
            view.read(app)
        ));
    });
}

#[gpui::test]
fn repository_picker_fallback_opens_new_normal_window_when_none_exist(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);

    assert_eq!(cx.update(|app| app.windows().len()), 0);
    cx.update(|app| {
        show_open_repository_manual_entry_in_existing_or_new_window(app, Arc::clone(&backend));
    });
    cx.run_until_parked();

    let panel_visible = cx.update(|app| {
        let entry = find_normal_gitcomet_window(app)
            .expect("expected a normal GitComet window for manual repository entry");
        entry
            .view
            .update(app, |view, _cx| {
                crate::view::test_support::open_repo_panel_visible(view)
            })
            .expect("expected to inspect the new GitComet window")
    });
    assert_eq!(cx.update(|app| app.windows().len()), 1);
    assert!(panel_visible);
}

#[gpui::test]
fn command_palette_opens_new_normal_window_when_none_exist(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);

    assert_eq!(cx.update(|app| app.windows().len()), 0);
    cx.update(|app| {
        toggle_command_palette_in_active_existing_or_new_window(app, Arc::clone(&backend));
    });
    cx.run_until_parked();

    let palette_open = cx.update(|app| {
        let entry = find_normal_gitcomet_window(app)
            .expect("expected a normal GitComet window for Command Palette");
        entry
            .view
            .read_with(app, |view, _cx| {
                crate::view::test_support::command_palette_is_open(view)
            })
            .expect("expected to inspect the new GitComet window")
    });
    assert_eq!(cx.update(|app| app.windows().len()), 1);
    assert!(palette_open);
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn macos_clone_repository_action_opens_native_clone_prompt(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        install_macos_app_menu(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });
    cx.cx.update(|app| app.dispatch_action(&CloneRepository));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("clone_repo_go_hint").is_some(),
        "Clone repository should open GitComet's native clone prompt"
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn macos_initialize_repository_action_requests_folder_picker(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        install_macos_app_menu(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
    });
    cx.cx
        .update(|app| app.dispatch_action(&InitializeRepository));
    cx.run_until_parked();

    assert!(
        cx.did_prompt_for_paths(),
        "Initialize repository should request a destination folder"
    );
    cx.simulate_path_prompt_response(|options| {
        assert!(options.directories);
        assert!(!options.files);
        assert!(!options.multiple);
        assert_eq!(options.prompt.as_deref(), Some("Initialize Git Repository"));
        None
    });
    cx.run_until_parked();
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn macos_repository_actions_validate_background_normal_window(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::process::{
        GitExecutableAvailability, GitExecutablePreference, GitRuntimeState,
    };
    use gitcomet_state::model::AppState;

    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let main_window_id = cx.update(|window, app| {
        install_macos_app_menu(app, Arc::clone(&backend));
        view.update(app, |view, cx| {
            crate::view::test_support::apply_state_snapshot_for_test(
                view,
                Arc::new(AppState {
                    git_runtime: GitRuntimeState {
                        preference: GitExecutablePreference::Custom(PathBuf::new()),
                        availability: GitExecutableAvailability::Unavailable {
                            detail: "Git unavailable for menu routing test".to_string(),
                        },
                    },
                    ..AppState::test_default()
                }),
                cx,
            );
        });
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });

    cx.cx.update(crate::view::open_settings_window);
    cx.run_until_parked();
    let settings_window = cx.cx.update(|app| {
        app.windows()
            .into_iter()
            .find(|window| window.window_id() != main_window_id)
            .expect("Settings window should open")
    });
    let settings_window_id = settings_window.window_id();
    cx.cx.update(|app| {
        let _ = settings_window.update(app, |_view, window, _cx| window.activate());
        assert_eq!(
            app.active_window().map(|window| window.window_id()),
            Some(settings_window_id),
            "Settings should become active"
        );
    });
    assert_ne!(settings_window_id, main_window_id);

    cx.cx.update(|app| app.dispatch_action(&CloneRepository));
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        cx.debug_bounds("clone_repo_go_hint").is_none(),
        "Clone repository must stay blocked by the target window's unavailable runtime"
    );

    cx.cx
        .update(|app| app.dispatch_action(&InitializeRepository));
    cx.run_until_parked();
    assert!(
        !cx.did_prompt_for_paths(),
        "Initialize repository must stay blocked by the target window's unavailable runtime"
    );
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn locate_file_action_activates_background_normal_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let main_window_id = cx.update(|window, app| {
        install_app_shortcuts_for_test(app, Arc::clone(&backend));
        let _ = window.draw(app);
        window.activate();
        window.window_handle().window_id()
    });
    cx.cx.update(crate::view::open_settings_window);
    cx.run_until_parked();
    let settings_window = cx.cx.update(|app| {
        app.windows()
            .into_iter()
            .find(|window| window.window_id() != main_window_id)
            .expect("Settings window should open")
    });
    let settings_window_id = settings_window.window_id();
    cx.cx.update(|app| {
        let _ = settings_window.update(app, |_view, window, _cx| window.activate());
        assert_eq!(
            app.active_window().map(|window| window.window_id()),
            Some(settings_window_id),
            "Settings should become active"
        );
    });
    assert_ne!(settings_window_id, main_window_id);

    cx.cx
        .update(|app| app.dispatch_action(&LocateFileInExplorer));
    cx.run_until_parked();

    cx.cx.update(|app| {
        assert_eq!(
            app.active_window().map(|window| window.window_id()),
            Some(main_window_id),
            "locating a file should bring the main GitComet window forward"
        );
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn focus_existing_repository_window_for_path_avoids_reading_the_active_window_on_stack(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let window_handle = cx.update(|window, app| {
        let _ = window.draw(app);
        window.activate();
        window.window_handle()
    });

    let repo_path = PathBuf::from("/tmp/gitcomet-not-open");
    cx.cx.update(|app| {
        let result = window_handle.update(app, |_root_view, _window, app| {
            focus_existing_repository_window_for_path(app, repo_path.as_path())
        });
        assert_eq!(result.ok(), Some(false));
    });
}

#[cfg(target_os = "macos")]
#[gpui::test]
fn update_active_normal_gitcomet_window_avoids_reading_the_active_window_on_stack(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(Arc::clone(&backend));
    let (_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let window_handle = cx.update(|window, app| {
        let _ = window.draw(app);
        window.activate();
        window.window_handle()
    });

    cx.cx.update(|app| {
        let result = window_handle.update(app, |_root_view, _window, app| {
            update_active_normal_gitcomet_window(app, |view, cx| view.close_active_repo_tab(cx))
        });
        assert_eq!(result.ok().flatten(), Some(false));
    });
}

#[cfg(target_os = "macos")]
#[test]
fn file_url_to_path_decodes_spaces() {
    let path = file_url_to_path("file:///Users/sampo/Repo%20Name").expect("valid file url");
    assert_eq!(path, PathBuf::from("/Users/sampo/Repo Name"));
}

#[cfg(target_os = "macos")]
#[test]
fn file_url_to_path_accepts_localhost_urls() {
    let path = file_url_to_path("file://localhost/Users/sampo/repo").expect("localhost file url");
    assert_eq!(path, PathBuf::from("/Users/sampo/repo"));
}

#[cfg(target_os = "macos")]
#[test]
fn repository_paths_from_open_urls_filters_non_file_urls_and_dedups() {
    let urls = vec![
        "file:///tmp/repo".to_string(),
        "https://example.com/repo".to_string(),
        "file:///tmp/repo".to_string(),
    ];

    let paths = repository_paths_from_open_urls(&urls);

    assert_eq!(
        paths,
        vec![normalize_repository_open_path(PathBuf::from("/tmp/repo"))]
    );
}

/// A root view that is neither a main nor a Settings window, like a pop-out.
struct RenderCounter(std::rc::Rc<std::cell::Cell<usize>>);

impl gpui::Render for RenderCounter {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl gpui::IntoElement {
        self.0.set(self.0.get() + 1);
        gpui::Empty
    }
}

#[gpui::test]
fn zooming_a_plain_window_redraws_it(cx: &mut gpui::TestAppContext) {
    let renders = std::rc::Rc::new(std::cell::Cell::new(0));
    let window = cx.add_window({
        let renders = renders.clone();
        move |_, _| RenderCounter(renders)
    });
    cx.run_until_parked();
    let before = renders.get();

    cx.update(|app| set_window_ui_scale_percent(app, window.window_id(), Some(150)));
    cx.run_until_parked();

    window
        .update(cx, |_, window, _| {
            assert_eq!(window.rem_size(), ui_scale::rem_size_for_percent(150));
        })
        .unwrap();
    assert!(renders.get() > before, "the zoomed window must redraw");
}
