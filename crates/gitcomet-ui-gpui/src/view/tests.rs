use super::*;
use crate::view::test_support::TestBackend;
use chrome::{cursor_style_for_resize_edge, resize_edge};
use gitcomet_core::domain::{
    Branch, CommitId, FileEntry, FileEntryKind, Remote, RemoteBranch, RepoSpec, StashEntry,
    Submodule, SubmoduleStatus, Upstream, Worktree,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use gitcomet_state::model::{AppState, AuthPromptState, AuthRetryOperation, RepoId, RepoState};
use gitcomet_state::store::AppStore;
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct RecordingFailingBackend {
    opened: Arc<Mutex<Vec<PathBuf>>>,
}

#[test]
fn selected_sidebar_branch_colors_come_from_theme_interaction_tokens() {
    let mut theme = AppTheme::gitcomet_dark();
    let selected_background = gpui::rgba(0x12345678);
    let selected_foreground = gpui::rgba(0xabcdefee);
    theme.colors.interaction.selected_background = selected_background;
    theme.colors.interaction.selected_foreground = selected_foreground;

    assert_eq!(selected_branch_row_bg(theme), selected_background);
    assert_eq!(selected_branch_label_color(theme), selected_foreground);
}

#[test]
fn recent_repository_shortcut_is_not_a_diff_select_all_candidate() {
    let recent = gpui::Keystroke::parse("secondary-shift-a").expect("valid shortcut");
    let select_all = gpui::Keystroke::parse("secondary-a").expect("valid shortcut");

    assert!(
        !is_diff_shortcut_candidate(&recent),
        "the app-level recent-repositories chord must not reach diff text selection"
    );
    assert!(
        is_diff_shortcut_candidate(&select_all),
        "unshifted Ctrl/Cmd+A must still reach diff text selection"
    );
}

impl GitBackend for RecordingFailingBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        self.opened
            .lock()
            .expect("recording backend lock")
            .push(workdir.to_path_buf());
        Err(Error::new(ErrorKind::Unsupported(
            "Recording backend does not open repositories",
        )))
    }
}

fn pump_for(cx: &mut gpui::VisualTestContext, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(16));
    }
}

/// Like [`wait_until`], but keeps drawing and draining the test executor while
/// it waits.
///
/// Required whenever the awaited work is a GPUI task — a `cx.spawn(..).detach()`
/// — rather than something the store's own worker thread advances: those tasks
/// only run when the test driver pumps them, so a sleeping wait would spin out
/// its whole deadline without ever letting the task complete.
fn pump_until(cx: &mut gpui::VisualTestContext, description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn wait_until(description: &str, ready: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if ready() {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for {description}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
}

fn dispatch_file_drop(cx: &mut gpui::VisualTestContext, event: gpui::FileDropEvent) {
    cx.update(|window, app| {
        let _ = window.dispatch_event(gpui::PlatformInput::FileDrop(event), app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn install_repo_tab_test_state(
    store: &AppStore,
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    active_repo: RepoId,
) {
    install_repo_tab_test_state_with_count(store, view, cx, active_repo, 3);
}

fn install_repo_tab_test_state_with_count(
    store: &AppStore,
    view: &gpui::Entity<GitCometView>,
    cx: &mut gpui::VisualTestContext,
    active_repo: RepoId,
    repo_count: u64,
) {
    let mut state = AppState {
        active_repo: Some(active_repo),
        git_runtime: available_git_runtime_state(),
        ..AppState::default()
    };
    for ix in 1..=repo_count {
        state.repos.push(RepoState::new_opening(
            RepoId(ix),
            RepoSpec {
                workdir: PathBuf::from(format!("/tmp/repo-tab-menu-{ix}")),
            },
        ));
    }
    store.replace_snapshot_for_test(Arc::new(state));
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
}

fn open_repo_tab_context_menu(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} to be rendered"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);
}

fn install_app_shortcuts_for_test(cx: &mut gpui::VisualTestContext, backend: Arc<dyn GitBackend>) {
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, backend);
        let _ = window.draw(app);
        window.activate_window();
    });
}

fn sync_view_snapshot(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
}

fn focus_detached_window_focus(cx: &mut gpui::VisualTestContext) {
    cx.update(|window, app| {
        let focus = app.focus_handle();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    test_support::redraw(cx);
}

fn command_palette_input_focus(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Option<gpui::FocusHandle> {
    cx.update(|_window, app| {
        Some(
            view.read(app)
                .command_palette
                .read(app)
                .query_input
                .read(app)
                .focus_handle(),
        )
    })
}

fn command_palette_is_open(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).command_palette_open)
}

fn available_git_runtime_state() -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::SystemPath,
        availability: GitExecutableAvailability::Available {
            version_output: "git version 2.51.0".to_string(),
        },
    }
}

fn unavailable_git_runtime_state() -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::Custom(PathBuf::new()),
        availability: GitExecutableAvailability::Unavailable {
            detail: "Custom Git executable is not configured. Choose an executable or switch back to System PATH.".to_string(),
        },
    }
}

fn view_state_with_active_ready_repo(repo_id: RepoId) -> AppState {
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/repo"),
        },
    );
    repo.open = Loadable::Ready(());
    AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    }
}

fn repo_with_push_state(
    upstream: Option<Upstream>,
    remotes: Loadable<Arc<Vec<Remote>>>,
) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("/tmp/push-request"),
        },
    );
    repo.head_branch = Loadable::Ready("feature".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature".to_string(),
        target: CommitId("deadbeef".into()),
        upstream,
        divergence: None,
    }]));
    repo.remotes = remotes;
    repo
}

#[test]
fn push_request_uses_configured_upstream_without_claiming_it_is_live() {
    let repo = repo_with_push_state(
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        Loadable::Loading,
    );

    assert_eq!(push_request(&repo), PushRequest::Push);
    assert!(!head_branch_has_live_upstream(&repo));
}

#[test]
fn live_upstream_requires_the_exact_loaded_remote_tracking_ref() {
    let mut repo = repo_with_push_state(
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "feature".to_string(),
        target: CommitId("remote-feature".into()),
    }]));

    assert!(head_branch_has_live_upstream(&repo));
    assert_eq!(pull_request(&repo), PullRequest::Pull);
}

#[test]
fn configured_but_unpushed_upstream_is_a_push_target_without_being_live() {
    let mut repo = repo_with_push_state(
        Some(Upstream {
            remote: "origin".to_string(),
            branch: "review/feature".to_string(),
        }),
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));

    assert_eq!(push_request(&repo), PushRequest::Push);
    assert!(!head_branch_has_live_upstream(&repo));
    assert_eq!(
        pull_request(&repo),
        PullRequest::NotReady,
        "Pull must stay disabled until the configured branch exists remotely"
    );
}

#[test]
fn push_request_offers_standard_remote_for_untracked_branch() {
    let repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![
            Remote {
                name: "backup".to_string(),
                url: None,
            },
            Remote {
                name: "origin".to_string(),
                url: None,
            },
        ])),
    );

    assert_eq!(
        push_request(&repo),
        PushRequest::SetUpstream {
            remote: "origin".to_string()
        }
    );
    assert!(!head_branch_has_live_upstream(&repo));
}

#[test]
fn push_request_uses_first_remote_when_origin_is_absent() {
    let repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![Remote {
            name: "upstream".to_string(),
            url: None,
        }])),
    );

    assert_eq!(
        push_request(&repo),
        PushRequest::SetUpstream {
            remote: "upstream".to_string()
        }
    );
}

#[test]
fn push_request_distinguishes_no_remotes_from_loading_data() {
    let no_remotes = repo_with_push_state(None, Loadable::Ready(Arc::new(Vec::new())));
    let loading = repo_with_push_state(None, Loadable::Loading);

    assert_eq!(push_request(&no_remotes), PushRequest::NoRemotes);
    assert_eq!(push_request(&loading), PushRequest::NotReady);
}

#[test]
fn a_selected_remote_branch_is_invalidated_only_after_a_ready_refresh_omits_it() {
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/selected-remote-branch"),
        },
    );
    let selected = SelectedBranch {
        repo_id,
        target: BranchMenuTarget::remote("origin", "deleted"),
    };
    let mut state = AppState {
        repos: vec![repo.clone()],
        active_repo: Some(repo_id),
        ..Default::default()
    };

    assert!(
        !selected_remote_branch_is_missing(&state, Some(&selected)),
        "loading data is not proof that the selection disappeared"
    );

    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));
    state.repos[0] = repo;
    assert!(selected_remote_branch_is_missing(&state, Some(&selected)));
}

#[test]
fn pull_request_offers_a_pull_for_a_branch_that_was_never_pushed() {
    let repo = repo_with_push_state(
        None,
        Loadable::Ready(Arc::new(vec![Remote {
            name: "origin".to_string(),
            url: None,
        }])),
    );

    assert!(!head_branch_has_live_upstream(&repo));
    assert_eq!(
        pull_request(&repo),
        PullRequest::Pull,
        "the backend pulls from the preferred remote and sets the upstream"
    );
}

#[test]
fn pull_request_allows_a_detached_head_and_reports_a_repo_without_remotes() {
    let mut detached = repo_with_push_state(None, Loadable::Loading);
    detached.head_branch = Loadable::Ready("HEAD".to_string());
    assert!(head_is_detached(&detached));
    assert_eq!(pull_request(&detached), PullRequest::Pull);

    let no_remotes = repo_with_push_state(None, Loadable::Ready(Arc::new(Vec::new())));
    assert_eq!(pull_request(&no_remotes), PullRequest::NoRemotes);
}

#[test]
fn a_selected_remote_branch_survives_a_remote_name_containing_a_slash() {
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/nested-remote"),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "forks/alice".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    let state = AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    };
    let selected = SelectedBranch {
        repo_id,
        target: BranchMenuTarget::remote("forks/alice", "main"),
    };

    assert!(!selected_remote_branch_is_missing(&state, Some(&selected)));
}

#[gpui::test]
fn folder_drag_marks_repository_bar_available_and_tracks_hover_emphasis(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
        assert!(!test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
        assert!(!test_support::repo_external_folder_drag_hovered(
            view.read(app),
            app
        ));
    });
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
    let (store, events) = AppStore::new(backend);
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
    pump_until(cx, "folder drop to dispatch a repository open", || {
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
fn repository_bar_ignores_files_multiple_paths_and_drops_outside_the_bar(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let opened = Arc::new(Mutex::new(Vec::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(RecordingFailingBackend {
        opened: Arc::clone(&opened),
    });
    let (store, events) = AppStore::new(backend);
    let store_for_state = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    install_repo_tab_test_state_with_count(&store_for_state, &view, cx, RepoId(1), 1);

    let folder_a = tempfile::tempdir().expect("create first dropped folder");
    let folder_b = tempfile::tempdir().expect("create second dropped folder");
    let file = tempfile::NamedTempFile::new().expect("create dropped file");
    let bar_point = cx
        .debug_bounds("repo_external_folder_drop_target")
        .expect("repository bar drop target should be rendered")
        .center();
    let outside_bar = cx.update(|window, _app| {
        let viewport = window.viewport_size();
        gpui::point(viewport.width / 2.0, viewport.height / 2.0)
    });

    for paths in [
        vec![file.path().to_path_buf()],
        vec![folder_a.path().to_path_buf(), folder_b.path().to_path_buf()],
    ] {
        dispatch_file_drop(
            cx,
            gpui::FileDropEvent::Entered {
                position: bar_point,
                paths: gpui::ExternalPaths(paths.into_iter().collect()),
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
    }

    dispatch_file_drop(
        cx,
        gpui::FileDropEvent::Entered {
            position: outside_bar,
            paths: gpui::ExternalPaths([folder_a.path().to_path_buf()].into_iter().collect()),
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
        "unsupported payloads and drops outside the repository bar must remain unhandled"
    );
    cx.update(|_window, app| {
        assert!(!test_support::repo_external_folder_drag_active(
            view.read(app),
            app
        ));
    });
}

#[gpui::test]
fn startup_crash_report_is_visible_after_relaunch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(backend);
    let config = GitCometViewConfig::normal(Some(StartupCrashReport {
        issue_url: "https://example.invalid/crash-report".to_string(),
        summary: "WSLg clipboard copy terminated unexpectedly".to_string(),
        crash_log_path: PathBuf::from("/tmp/gitcomet-crash.log"),
    }));
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    });

    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("startup_crash_report").is_some(),
        "a recovered crash must render the report notification"
    );
    cx.update(|_window, app| {
        assert!(
            view.read(app).startup_crash_report.is_some(),
            "the recovered report must remain available until ignored"
        );
    });
}

#[gpui::test]
fn ignoring_startup_crash_report_deletes_it_and_hides_notification(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let recovery_dir = tempfile::tempdir().expect("create recovery state directory");
    let crash_log_path = recovery_dir.path().join("pending-startup-report.log");
    std::fs::write(&crash_log_path, "message=previous crash\n").expect("write crash report");

    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(backend);
    let config = GitCometViewConfig::normal(Some(StartupCrashReport {
        issue_url: "https://example.invalid/crash-report".to_string(),
        summary: "WSLg clipboard copy terminated unexpectedly".to_string(),
        crash_log_path: crash_log_path.clone(),
    }));
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    });

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.ignore_startup_crash_report()
                .expect("ignore startup crash report");
        });
    });

    assert!(
        !crash_log_path.exists(),
        "ignoring the crash must delete its persisted report"
    );
    cx.update(|_window, app| {
        assert!(
            view.read(app).startup_crash_report.is_none(),
            "ignoring the crash must hide its notification"
        );
    });
}

#[gpui::test]
fn reporting_startup_crash_keeps_report_and_notification(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let report_dir = tempfile::tempdir().expect("create report directory");
    let crash_log_path = report_dir.path().join("pending-startup-report.log");
    std::fs::write(&crash_log_path, "message=previous crash\n").expect("write crash report");

    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(backend);
    let config = GitCometViewConfig::normal(Some(StartupCrashReport {
        issue_url: "https://example.invalid/crash-report".to_string(),
        summary: "previous crash".to_string(),
        crash_log_path: crash_log_path.clone(),
    }));
    let (view, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new_with_config(store, events, config, window, cx)
    });

    // Drive the button's real handler with a stub launcher standing in for the
    // browser, so the assertions below describe a report page that was actually
    // opened rather than a getter that was read.
    let opened = Arc::new(std::sync::Mutex::new(None::<String>));
    let opened_in_launch = Arc::clone(&opened);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.report_startup_crash_report_with(cx, move |url| {
                *opened_in_launch
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(url);
                Ok(())
            });
        });
    });
    cx.run_until_parked();

    assert_eq!(
        opened
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_deref(),
        Some("https://example.invalid/crash-report"),
        "the button must open the URL recorded for the crash"
    );

    assert!(
        crash_log_path.exists(),
        "opening the report page must retain the persisted crash report"
    );
    cx.update(|_window, app| {
        assert!(
            view.read(app).startup_crash_report.is_some(),
            "opening the report page must keep the notification visible"
        );
    });
}

#[gpui::test]
fn command_palette_opens_from_detached_focus_on_loading_repo_tabs(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    focus_detached_window_focus(cx);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);

    assert!(
        command_palette_is_open(cx, &view),
        "expected secondary-p from detached focus to open the command palette"
    );
    assert!(
        cx.debug_bounds("modal_scrim").is_some(),
        "expected command palette to use the shared modal scrim"
    );
    let input_focus = command_palette_input_focus(cx, &view)
        .expect("expected command palette input to exist after opening");
    cx.update(|window, app| {
        assert_eq!(
            window.focused(app),
            Some(input_focus),
            "expected command palette input to own window focus after opening"
        );
    });
}

#[gpui::test]
fn command_palette_reopens_after_tab_switch_and_close_cycles(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    install_repo_tab_test_state(&store, &view, cx, RepoId(1));
    store.dispatch(Msg::SetActiveRepo { repo_id: RepoId(2) });
    sync_view_snapshot(cx, &view);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "expected command palette to open after switching repository tabs"
    );

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        !command_palette_is_open(cx, &view),
        "expected secondary-p to close the command palette"
    );

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "expected command palette to reopen after a toggle-close cycle"
    );

    cx.simulate_keystrokes("escape");
    test_support::redraw(cx);
    assert!(
        !command_palette_is_open(cx, &view),
        "expected escape to close the command palette"
    );

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    assert!(
        command_palette_is_open(cx, &view),
        "expected command palette to reopen after closing with escape"
    );
}

#[gpui::test]
fn command_palette_opens_commit_prompt_for_clean_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].staged_status = Loadable::Ready(Arc::new(Vec::new()));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    cx.simulate_keystrokes("enter");
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert!(
            matches!(
                test_support::popover_kind(view.read(app), app),
                Some(PopoverKind::CommitPrompt { repo_id: RepoId(1) })
            ),
            "expected Commit Changes to remain selectable for a clean repo"
        );
    });
    assert!(
        cx.debug_bounds("modal_scrim").is_some(),
        "expected command-palette dialogs to use the shared modal scrim"
    );
}

#[gpui::test]
fn command_palette_rename_branch_opens_prompt_for_current_branch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].head_branch = Loadable::Ready("feature/current".to_string());
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("rename-branch", Some(window), cx)
        });
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert!(matches!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::RenameBranchPrompt {
                repo_id: RepoId(1),
                name,
                is_current_branch: true,
            }) if name == "feature/current"
        ));
    });
}

/// Staging is what marks a conflict resolved, so every stage entry point has to
/// warn about markers left in the worktree — including the command palette's
/// "Stage all", which reaches conflicted files just as the buttons do.
#[gpui::test]
fn command_palette_stage_all_asks_before_staging_unresolved_conflicts(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_command_stage_all_conflict",
        std::process::id()
    ));
    let conflicted = PathBuf::from("conflicted.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join(&conflicted),
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n",
    )
    .unwrap();

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].spec.workdir = workdir.clone();
    state.repos[0].status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: vec![],
            unstaged: vec![gitcomet_core::domain::FileStatus {
                path: conflicted.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
            }],
        }
        .into(),
    );
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("stage-all", Some(window), cx)
        });
    });
    test_support::redraw(cx);

    cx.update(|_window, app| {
        let kind = test_support::popover_kind(view.read(app), app);
        assert!(
            matches!(
                kind,
                Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                    if unresolved == &vec![conflicted.clone()]
            ),
            "expected the unresolved-conflict confirmation, got {kind:?}"
        );
    });

    // The stage itself must wait for the user's answer.
    assert!(
        cx.update(|_window, app| {
            view.read(app)
                .store
                .snapshot()
                .repos
                .iter()
                .find(|repo| repo.id == RepoId(1))
                .is_some_and(|repo| repo.local_actions_in_flight == 0)
        }),
        "nothing may be staged until the confirmation is answered"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn command_palette_close_falls_back_to_diff_panel_when_saved_focus_is_stale(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new(Arc::clone(&backend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_app_shortcuts_for_test(cx, Arc::clone(&backend));
    let state = view_state_with_active_ready_repo(RepoId(1));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|window, app| {
        let focus = view
            .read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    test_support::redraw(cx);

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let stale_focus = this
                .command_palette
                .read(cx)
                .query_input
                .read(cx)
                .focus_handle();
            this.command_palette.update(cx, |palette, _cx| {
                palette.restore_focus = Some(stale_focus);
            });
        });
    });

    cx.simulate_keystrokes("secondary-p");
    test_support::redraw(cx);
    pump_for(cx, Duration::from_millis(16));

    let diff_focus = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone()
    });
    cx.update(|window, app| {
        assert_eq!(
            window.focused(app),
            Some(diff_focus),
            "expected stale command-palette restore focus to fall back to the diff panel"
        );
    });
}

#[test]
fn window_activation_dispatches_repo_activated_message() {
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut last_activation_dispatch = FxHashMap::default();
    let now = Instant::now();

    let msg = repo_activation_msg(&state, &mut last_activation_dispatch, now)
        .expect("ready active repo should produce activation message");

    assert!(matches!(msg, Msg::RepoActivated { repo_id: got } if got == repo_id));
    assert!(!matches!(msg, Msg::RepoExternallyChanged { .. }));
}

#[test]
fn window_activation_dispatch_is_throttled_per_repo() {
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut last_activation_dispatch = FxHashMap::default();
    let now = Instant::now();

    assert!(repo_activation_msg(&state, &mut last_activation_dispatch, now).is_some());
    assert!(
        repo_activation_msg(
            &state,
            &mut last_activation_dispatch,
            now + Duration::from_secs(1),
        )
        .is_none()
    );
    assert!(matches!(
        repo_activation_msg(
            &state,
            &mut last_activation_dispatch,
            now + REPO_ACTIVATION_THROTTLE,
        ),
        Some(Msg::RepoActivated { repo_id: got }) if got == repo_id
    ));
}

#[test]
fn window_grab_suppresses_the_activation_it_caused() {
    // Dragging the title bar or a resize edge hands focus to the compositor for
    // the duration of the grab, which GPUI reports as a deactivate → activate
    // pair. Treating that as a return to the app refreshed the whole repo on
    // every window move/resize.
    let now = Instant::now();
    crate::app::note_window_grab_started();

    let armed = crate::app::take_window_grab_started_within(now, WINDOW_GRAB_DEACTIVATE_GRACE);
    assert!(armed, "a fresh grab must claim the deactivation it caused");

    let mut suppressed_at = Some(now);
    assert!(consume_window_grab_activation(
        &mut suppressed_at,
        now + Duration::from_secs(5)
    ));
    assert!(
        suppressed_at.is_none(),
        "the marker must be consumed so it cannot suppress twice"
    );
}

#[test]
fn stale_window_grab_does_not_suppress_a_later_activation() {
    // A grab the compositor ignored (bad serial, unsupported protocol) must not
    // leave suppression armed for an unrelated alt-tab minutes later.
    let now = Instant::now();
    crate::app::note_window_grab_started();

    assert!(!crate::app::take_window_grab_started_within(
        now + WINDOW_GRAB_DEACTIVATE_GRACE + Duration::from_millis(1),
        WINDOW_GRAB_DEACTIVATE_GRACE,
    ));
    assert!(
        !crate::app::take_window_grab_started_within(now, WINDOW_GRAB_DEACTIVATE_GRACE),
        "the stale marker must have been cleared, not left armed"
    );
}

#[test]
fn window_grab_suppression_expires_for_a_very_late_activation() {
    let now = Instant::now();
    let mut suppressed_at = Some(now);
    assert!(!consume_window_grab_activation(
        &mut suppressed_at,
        now + WINDOW_GRAB_REACTIVATE_GRACE + Duration::from_secs(1),
    ));
}

#[test]
fn unsuppressed_activation_still_dispatches_repo_activated() {
    // Suppression is opt-in, and a suppressed activation must not stamp the
    // throttle map — a genuine alt-tab right after a drag still refreshes.
    let repo_id = RepoId(1);
    let state = view_state_with_active_ready_repo(repo_id);
    let mut last_activation_dispatch = FxHashMap::default();
    let now = Instant::now();

    let mut suppressed_at = None;
    assert!(!consume_window_grab_activation(&mut suppressed_at, now));
    assert!(matches!(
        repo_activation_msg(&state, &mut last_activation_dispatch, now),
        Some(Msg::RepoActivated { repo_id: got }) if got == repo_id
    ));
}

#[test]
fn toast_total_lifetime_includes_fade_in_and_out() {
    let ttl = Duration::from_secs(6);
    assert_eq!(
        toast_total_lifetime(ttl),
        ttl + Duration::from_millis(TOAST_FADE_IN_MS + TOAST_FADE_OUT_MS)
    );
}

#[test]
fn next_pane_resize_drag_width_recomputes_bounds_when_window_changes() {
    let state = PaneResizeState::new(
        PaneResizeHandle::Sidebar,
        px(0.0),
        px(280.0),
        px(420.0),
        px(1280.0),
        false,
        false,
    );
    let current_x = px(320.0);
    let total_w = px(900.0);
    let width = next_pane_resize_drag_width(&state, current_x, total_w, false, false);
    let (min_width, max_width) = pane_resize_drag_width_bounds(
        PaneResizeHandle::Sidebar,
        px(280.0),
        px(420.0),
        total_w,
        false,
        false,
    );
    let expected = (px(280.0) + current_x).max(min_width).min(max_width);

    assert_eq!(width, expected);
}

#[test]
fn diff_split_column_widths_from_available_clamps_to_min_widths() {
    let (left, right) = diff_split_column_widths_from_available(px(556.0), px(160.0), 0.95);

    assert_eq!(left, px(396.0));
    assert_eq!(right, px(160.0));
}

#[test]
fn diff_split_column_widths_from_available_falls_back_to_even_split_when_narrow() {
    let (left, right) = diff_split_column_widths_from_available(px(300.0), px(160.0), 0.95);

    assert_eq!(left, px(150.0));
    assert_eq!(right, px(150.0));
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

#[test]
fn splash_backdrop_embedded_png_decodes() {
    assert_eq!(
        super::splash::load_splash_backdrop_image().format(),
        gpui::ImageFormat::Png,
        "expected splash backdrop image to decode from embedded PNG bytes"
    );
}

#[test]
fn reconcile_status_multi_selection_prunes_missing_paths_and_anchors() {
    let a = PathBuf::from("a.txt");
    let b = PathBuf::from("b.txt");
    let c = PathBuf::from("c.txt");

    let status = RepoStatus {
        staged: vec![],
        unstaged: vec![FileStatus {
            path: a.clone(),
            kind: FileStatusKind::Modified,
            conflict: None,
        }],
    };

    let mut selection = StatusMultiSelection {
        untracked: vec![],
        untracked_anchor: None,
        unstaged: vec![a.clone(), b.clone()],
        unstaged_anchor: Some(b),
        unstaged_anchor_index: None,
        unstaged_anchor_status_rev: None,
        staged: vec![c.clone()],
        staged_anchor: Some(c),
        staged_anchor_index: None,
        staged_anchor_status_rev: None,
    };

    reconcile_status_multi_selection(&mut selection, &status);

    assert_eq!(selection.unstaged, vec![a]);
    assert!(selection.unstaged_anchor.is_none());
    assert!(selection.staged.is_empty());
    assert!(selection.staged_anchor.is_none());
}

#[test]
fn remote_rows_groups_and_sorts() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "b".to_string(),
            target: CommitId("b0".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "a".to_string(),
            target: CommitId("a0".into()),
        },
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "main".to_string(),
            target: CommitId("c0".into()),
        },
    ]));

    let rows = GitCometView::remote_rows(&repo);
    assert_eq!(
        rows,
        vec![
            RemoteRow::Header("origin".to_string()),
            RemoteRow::Branch {
                remote: "origin".to_string(),
                name: "a".to_string()
            },
            RemoteRow::Branch {
                remote: "origin".to_string(),
                name: "b".to_string()
            },
            RemoteRow::Header("upstream".to_string()),
            RemoteRow::Branch {
                remote: "upstream".to_string(),
                name: "main".to_string()
            },
        ]
    );
}

#[test]
fn remote_headers_include_remotes_with_no_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.remotes = Loadable::Ready(Arc::new(vec![
        Remote {
            name: "origin".to_string(),
            url: Some("https://example.com/origin.git".to_string()),
        },
        Remote {
            name: "upstream".to_string(),
            url: Some("https://example.com/upstream.git".to_string()),
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let mut headers = rows
        .iter()
        .filter_map(|r| match r {
            BranchSidebarRow::RemoteHeader { name, .. } => Some(name.as_ref().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    headers.sort_unstable();
    headers.dedup();

    assert!(
        headers.contains(&"origin".to_string()),
        "expected origin remote header"
    );
    assert!(
        headers.contains(&"upstream".to_string()),
        "expected upstream remote header"
    );
}

#[test]
fn remote_upstream_branch_is_marked() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.head_branch = Loadable::Ready("main".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "main".to_string(),
        }),
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let upstream_row = rows.iter().find(|r| {
        matches!(
            r,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                is_upstream: true,
                ..
            } if name.as_ref() == "origin/main"
        )
    });
    assert!(
        upstream_row.is_some(),
        "expected origin/main to be marked as upstream"
    );
}

#[test]
fn branch_sidebar_branch_label_uses_leaf_segment() {
    assert_eq!(
        branch_sidebar::branch_sidebar_branch_label("origin/feature/topic"),
        "topic"
    );
    assert_eq!(
        branch_sidebar::branch_sidebar_branch_label("feature"),
        "feature"
    );
}

#[test]
fn branch_sidebar_keeps_leaf_before_children_when_branch_is_also_group() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "feature".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature/topic".to_string(),
            target: CommitId("feedface".into()),
            upstream: None,
            divergence: None,
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let feature_group_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                BranchSidebarRow::GroupHeader { label, depth, .. }
                    if label.as_ref() == "feature/" && *depth == 0
            )
        })
        .expect("expected feature group header");
    let feature_leaf_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch { name, depth, .. }
                    if name.as_ref() == "feature" && *depth == 1
            )
        })
        .expect("expected feature branch row");
    let feature_child_index = rows
        .iter()
        .position(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch { name, depth, .. }
                    if name.as_ref() == "feature/topic" && *depth == 1
            )
        })
        .expect("expected feature/topic branch row");

    assert!(feature_group_index < feature_leaf_index);
    assert!(feature_leaf_index < feature_child_index);
}

#[test]
fn branch_sidebar_sorts_unsorted_local_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "feature/topic".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "zeta".to_string(),
            target: CommitId("feedface".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature".to_string(),
            target: CommitId("cafebabe".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "alpha".to_string(),
            target: CommitId("8badf00d".into()),
            upstream: None,
            divergence: None,
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch { name, .. } => Some(name.as_ref().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(names, vec!["feature", "feature/topic", "alpha", "zeta"]);
}

#[test]
fn branch_sidebar_sorts_unsorted_remote_branches() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "zeta/topic".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "feature/topic".to_string(),
            target: CommitId("feedface".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "alpha".to_string(),
            target: CommitId("cafebabe".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "feature".to_string(),
            target: CommitId("8badf00d".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "alpha".to_string(),
            target: CommitId("decafbad".into()),
        },
        RemoteBranch {
            remote: "upstream".to_string(),
            name: "main".to_string(),
            target: CommitId("facefeed".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } => Some(name.as_ref().to_string()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            "origin/feature",
            "origin/feature/topic",
            "origin/alpha",
            "upstream/zeta/topic",
            "upstream/main",
        ]
    );
}

#[test]
fn remote_section_excludes_upstream_without_remote_tracking_ref() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    repo.head_branch = Loadable::Ready("feature".to_string());
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: Some(Upstream {
            remote: "origin".to_string(),
            branch: "feature".to_string(),
        }),
        divergence: None,
    }]));
    repo.remotes = Loadable::Ready(Arc::new(vec![Remote {
        name: "origin".to_string(),
        url: Some("https://example.com/origin.git".to_string()),
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(Vec::new()));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    assert!(
        rows.iter().all(|row| {
            !matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Remote,
                    name,
                    ..
                } if name.as_ref() == "origin/feature"
            )
        }),
        "a configured upstream must not synthesize a missing remote branch"
    );
}

#[test]
fn worktree_tooltip_includes_branch_name() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("main-worktree"),
        },
    );

    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: PathBuf::from("linked-worktree"),
        head: None,
        branch: Some("feature/tooltip".to_string()),
        detached: false,
    }]));

    let expanded_key = branch_sidebar::expanded_default_section_storage_key(
        branch_sidebar::worktrees_section_storage_key(),
    )
    .expect("worktrees should support explicit expansion");
    let rows = GitCometView::branch_sidebar_rows_with_collapsed(&repo, &[expanded_key.as_str()]);
    let row = rows
        .iter()
        .find_map(|row| match row {
            BranchSidebarRow::WorktreeItem {
                path,
                branch,
                detached,
                ..
            } => Some(
                branch_sidebar::branch_sidebar_worktree_label(
                    branch.as_ref().map(SharedString::as_ref),
                    *detached,
                    &path.to_string_lossy(),
                )
                .as_ref()
                .to_owned(),
            ),
            _ => None,
        })
        .expect("expected worktree row");

    assert_eq!(row, "feature/tooltip  linked-worktree");
}

#[test]
fn branch_sidebar_defaults_secondary_sections_to_collapsed() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: PathBuf::from("linked-worktree"),
        head: None,
        branch: Some("main".to_string()),
        detached: false,
    }]));
    repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
        path: PathBuf::from("vendor/lib"),
        recorded_head: CommitId("beadfeed".into()),
        checked_out_head: Some(CommitId("beadfeed".into())),
        status: SubmoduleStatus::UpToDate,
    }]));
    repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
        index: 0,
        id: CommitId("c0ffee".into()),
        message: "stash message".into(),
        created_at: None,
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);

    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::WorktreesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected Worktrees to start collapsed"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SubmodulesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected Submodules to start collapsed"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::StashHeader {
                collapsed: true,
                ..
            }
        )),
        "expected Stash to start collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::WorktreeItem { .. })),
        "expected Worktrees rows to stay hidden until expanded"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::SubmoduleItem { .. })),
        "expected Submodules rows to stay hidden until expanded"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::StashItem { .. })),
        "expected Stash rows to stay hidden until expanded"
    );
}

#[test]
fn branch_sidebar_starts_with_local_and_remote_branch_sections() {
    let repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );

    let rows = GitCometView::branch_sidebar_rows(&repo);
    assert!(
        matches!(
            rows.first(),
            Some(BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                ..
            })
        ),
        "expected Local Branches header to be the first sidebar row"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                ..
            }
        )),
        "expected Remote branches header to be present"
    );
}

#[test]
fn branch_sidebar_sorts_groups_before_branches_case_insensitively() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "zeta".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic/zeta".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "Alpha".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic/beta".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "topic/Alpha".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
    ]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/zeta".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "Main".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/beta".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/Alpha".to_string(),
            target: CommitId("deadbeef".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows(&repo);
    let local_names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Local,
                name,
                ..
            } => Some(name.as_ref().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let remote_names = rows
        .iter()
        .filter_map(|row| match row {
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } => Some(name.as_ref().to_owned()),
            _ => None,
        })
        .collect::<Vec<_>>();

    assert_eq!(
        local_names,
        vec![
            "topic/Alpha".to_string(),
            "topic/beta".to_string(),
            "topic/zeta".to_string(),
            "Alpha".to_string(),
            "zeta".to_string(),
        ]
    );
    assert_eq!(
        remote_names,
        vec![
            "origin/release/Alpha".to_string(),
            "origin/release/beta".to_string(),
            "origin/release/zeta".to_string(),
            "origin/Main".to_string(),
        ]
    );
}

#[test]
fn branch_sidebar_collapses_branch_sections_without_hiding_other_sections() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    repo.worktrees = Loadable::Ready(Arc::new(vec![Worktree {
        path: PathBuf::from("linked-worktree"),
        head: None,
        branch: Some("main".to_string()),
        detached: false,
    }]));
    repo.submodules = Loadable::Ready(Arc::new(vec![Submodule {
        path: PathBuf::from("vendor/lib"),
        recorded_head: CommitId("beadfeed".into()),
        checked_out_head: Some(CommitId("beadfeed".into())),
        status: SubmoduleStatus::UpToDate,
    }]));
    repo.stashes = Loadable::Ready(Arc::new(vec![StashEntry {
        index: 0,
        id: CommitId("c0ffee".into()),
        message: "stash message".into(),
        created_at: None,
    }]));

    let rows = GitCometView::branch_sidebar_rows_with_collapsed(
        &repo,
        &[
            branch_sidebar::local_section_storage_key(),
            branch_sidebar::remote_section_storage_key(),
            branch_sidebar::worktrees_section_storage_key(),
            branch_sidebar::submodules_section_storage_key(),
            branch_sidebar::stash_section_storage_key(),
        ],
    );

    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Local Branches header"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Remote branches header"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::Branch { .. })),
        "expected branch rows to be hidden when Local and Remote sections are collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::RemoteHeader { .. })),
        "expected remote headers to be hidden when Remote branches is collapsed"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::WorktreesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Worktrees header"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::SubmodulesHeader {
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Submodules header"
    );
    assert!(
        rows.iter().any(|row| matches!(
            row,
            BranchSidebarRow::StashHeader {
                collapsed: true,
                ..
            }
        )),
        "expected collapsed Stash header"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::WorktreeItem { .. })),
        "expected worktree rows to be hidden when Worktrees is collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::SubmoduleItem { .. })),
        "expected submodule rows to be hidden when Submodules is collapsed"
    );
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::StashItem { .. })),
        "expected stash rows to be hidden when Stash is collapsed"
    );
}

#[test]
fn branch_sidebar_collapses_local_branch_groups() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![
        Branch {
            name: "feature".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature/one".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "feature/two".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
        Branch {
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
            upstream: None,
            divergence: None,
        },
    ]));

    let feature_group_key = branch_sidebar::local_group_storage_key("feature");
    let rows =
        GitCometView::branch_sidebar_rows_with_collapsed(&repo, &[feature_group_key.as_str()]);

    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::GroupHeader {
                label,
                collapsed: true,
                ..
            } if label.as_ref() == "feature/"
        )
    }));
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::Branch { name, .. } if name.as_ref() == "main"
        )
    }));
    for hidden in ["feature", "feature/one", "feature/two"] {
        assert!(
            !rows.iter().any(|row| {
                matches!(
                    row,
                    BranchSidebarRow::Branch { name, .. } if name.as_ref() == hidden
                )
            }),
            "expected {hidden} to be hidden by collapsed feature/ group"
        );
    }
}

#[test]
fn branch_sidebar_collapses_local_section_without_hiding_remote_rows() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "main".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows_with_collapsed(
        &repo,
        &[branch_sidebar::local_section_storage_key()],
    );

    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Local,
                collapsed: true,
                ..
            }
        )
    }));
    assert!(
        !rows.iter().any(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Local,
                    ..
                }
            )
        }),
        "expected local branches to be hidden when Local section is collapsed"
    );
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::RemoteHeader { name, .. } if name.as_ref() == "origin"
        )
    }));
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::Branch {
                section: BranchSection::Remote,
                name,
                ..
            } if name.as_ref() == "origin/main"
        )
    }));
}

#[test]
fn branch_sidebar_collapses_remote_section_and_remote_groups() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.remote_branches = Loadable::Ready(Arc::new(vec![
        RemoteBranch {
            remote: "origin".to_string(),
            name: "main".to_string(),
            target: CommitId("deadbeef".into()),
        },
        RemoteBranch {
            remote: "origin".to_string(),
            name: "release/one".to_string(),
            target: CommitId("deadbeef".into()),
        },
    ]));

    let rows = GitCometView::branch_sidebar_rows_with_collapsed(
        &repo,
        &[branch_sidebar::remote_section_storage_key()],
    );
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::SectionHeader {
                section: BranchSection::Remote,
                collapsed: true,
                ..
            }
        )
    }));
    assert!(
        !rows
            .iter()
            .any(|row| matches!(row, BranchSidebarRow::RemoteHeader { .. })),
        "expected remote rows to be hidden when Remote section is collapsed"
    );

    let origin_key = branch_sidebar::remote_header_storage_key("origin");
    let rows = GitCometView::branch_sidebar_rows_with_collapsed(&repo, &[origin_key.as_str()]);
    assert!(rows.iter().any(|row| {
        matches!(
            row,
            BranchSidebarRow::RemoteHeader {
                name,
                collapsed: true,
                ..
            } if name.as_ref() == "origin"
        )
    }));
    assert!(
        !rows.iter().any(|row| {
            matches!(
                row,
                BranchSidebarRow::Branch {
                    section: BranchSection::Remote,
                    ..
                }
            )
        }),
        "expected origin branches to be hidden when the remote group is collapsed"
    );
}

#[test]
fn branch_sidebar_exposes_stable_collapse_keys_for_persistence() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/one".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    repo.remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "release/one".to_string(),
        target: CommitId("deadbeef".into()),
    }]));

    let rows = GitCometView::branch_sidebar_rows(&repo);

    let local_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::SectionHeader {
            section: BranchSection::Local,
            collapse_key,
            ..
        } => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(local_key, Some(branch_sidebar::local_section_storage_key()));

    let remote_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::SectionHeader {
            section: BranchSection::Remote,
            collapse_key,
            ..
        } => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        remote_key,
        Some(branch_sidebar::remote_section_storage_key())
    );

    let origin_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::RemoteHeader {
            name, collapse_key, ..
        } if name.as_ref() == "origin" => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        origin_key,
        Some(branch_sidebar::remote_header_storage_key("origin").as_str())
    );

    let local_group_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::GroupHeader {
            label,
            collapse_key,
            ..
        } if label.as_ref() == "feature/" => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        local_group_key,
        Some(branch_sidebar::local_group_storage_key("feature").as_str())
    );

    let remote_group_key = rows.iter().find_map(|row| match row {
        BranchSidebarRow::GroupHeader {
            label,
            collapse_key,
            ..
        } if label.as_ref() == "release/" => Some(collapse_key.as_ref()),
        _ => None,
    });
    assert_eq!(
        remote_group_key,
        Some(branch_sidebar::remote_group_storage_key("origin", "release").as_str())
    );
}

#[test]
fn resize_edge_detects_edges_and_corners() {
    let window_size = size(px(100.0), px(100.0));
    let tiling = Tiling::default();
    let inset = px(10.0);

    assert_eq!(
        resize_edge(point(px(0.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::TopLeft)
    );
    assert_eq!(
        resize_edge(point(px(99.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::TopRight)
    );
    assert_eq!(
        resize_edge(point(px(0.0), px(99.0)), inset, window_size, tiling),
        Some(ResizeEdge::BottomLeft)
    );
    assert_eq!(
        resize_edge(point(px(99.0), px(99.0)), inset, window_size, tiling),
        Some(ResizeEdge::BottomRight)
    );

    assert_eq!(
        resize_edge(point(px(50.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::Top)
    );
    assert_eq!(
        resize_edge(point(px(50.0), px(99.0)), inset, window_size, tiling),
        Some(ResizeEdge::Bottom)
    );
    assert_eq!(
        resize_edge(point(px(0.0), px(50.0)), inset, window_size, tiling),
        Some(ResizeEdge::Left)
    );
    assert_eq!(
        resize_edge(point(px(99.0), px(50.0)), inset, window_size, tiling),
        Some(ResizeEdge::Right)
    );

    assert_eq!(
        resize_edge(point(px(50.0), px(50.0)), inset, window_size, tiling),
        None
    );
}

#[test]
fn resize_edge_respects_tiling() {
    let window_size = size(px(100.0), px(100.0));
    let inset = px(10.0);
    let tiling = Tiling {
        top: true,
        left: false,
        right: false,
        bottom: false,
    };

    assert_eq!(
        resize_edge(point(px(0.0), px(0.0)), inset, window_size, tiling),
        Some(ResizeEdge::Left)
    );
    assert_eq!(
        resize_edge(point(px(50.0), px(0.0)), inset, window_size, tiling),
        None
    );
    assert_eq!(
        resize_edge(point(px(0.0), px(50.0)), inset, window_size, tiling),
        Some(ResizeEdge::Left)
    );
}

#[test]
fn cursor_style_matches_resize_edge() {
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::Left),
        CursorStyle::ResizeLeftRight
    );
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::Top),
        CursorStyle::ResizeUpDown
    );
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::TopLeft),
        CursorStyle::ResizeUpLeftDownRight
    );
    assert_eq!(
        cursor_style_for_resize_edge(ResizeEdge::TopRight),
        CursorStyle::ResizeUpRightDownLeft
    );
}

#[test]
fn is_markdown_path_detects_common_extensions() {
    use std::path::Path;
    assert!(is_markdown_path(Path::new("README.md")));
    assert!(is_markdown_path(Path::new("doc.markdown")));
    assert!(is_markdown_path(Path::new("notes.mdown")));
    assert!(is_markdown_path(Path::new("CHANGES.mkd")));
    assert!(is_markdown_path(Path::new("file.mkdn")));
    assert!(is_markdown_path(Path::new("file.mdwn")));
    assert!(is_markdown_path(Path::new("UPPER.MD")));
}

#[test]
fn is_markdown_path_rejects_non_markdown() {
    use std::path::Path;
    assert!(!is_markdown_path(Path::new("file.txt")));
    assert!(!is_markdown_path(Path::new("file.rs")));
    assert!(!is_markdown_path(Path::new("file")));
}

#[test]
fn should_bypass_text_file_preview_for_path_detects_supported_image_types() {
    use std::path::Path;

    for path in [
        "image.png",
        "image.JPEG",
        "image.gif",
        "image.webp",
        "image.bmp",
        "image.ico",
        "image.svg",
        "image.tif",
        "image.tiff",
    ] {
        assert!(
            should_bypass_text_file_preview_for_path(Path::new(path)),
            "expected {path} to bypass text file preview"
        );
    }

    for path in ["image.heic", "README.md", "notes.txt", "image"] {
        assert!(
            !should_bypass_text_file_preview_for_path(Path::new(path)),
            "did not expect {path} to bypass text file preview"
        );
    }
}

#[test]
fn preview_path_rendered_kind_detects_supported_preview_kinds() {
    use std::path::Path;

    assert_eq!(
        preview_path_rendered_kind(Path::new("diagram.svg")),
        Some(RenderedPreviewKind::Svg)
    );
    assert_eq!(
        preview_path_rendered_kind(Path::new("README.md")),
        Some(RenderedPreviewKind::Markdown)
    );
    assert_eq!(preview_path_rendered_kind(Path::new("notes.txt")), None);
}

#[test]
fn diff_target_rendered_preview_kind_reads_diff_target_paths() {
    let svg_target = DiffTarget::WorkingTree {
        path: PathBuf::from("diagram.svg"),
        area: DiffArea::Unstaged,
    };
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&svg_target)),
        Some(RenderedPreviewKind::Svg)
    );

    let markdown_target = DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: Some(PathBuf::from("README.md")),
    };
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&markdown_target)),
        Some(RenderedPreviewKind::Markdown)
    );

    let no_path_target = DiffTarget::Commit {
        commit_id: CommitId("deadbeef".into()),
        path: None,
    };
    assert_eq!(
        diff_target_rendered_preview_kind(Some(&no_path_target)),
        None
    );
}

#[test]
fn main_diff_rendered_preview_toggle_kind_matches_supported_modes() {
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(true, false, false, Some(RenderedPreviewKind::Svg),),
        Some(RenderedPreviewKind::Svg)
    );
    // The SVG Image/Code toggle is independent of the Full/Collapsed diff mode.
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(false, true, false, Some(RenderedPreviewKind::Svg),),
        Some(RenderedPreviewKind::Svg)
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(false, false, false, Some(RenderedPreviewKind::Svg),),
        None
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(
            true,
            false,
            false,
            Some(RenderedPreviewKind::Markdown),
        ),
        Some(RenderedPreviewKind::Markdown)
    );
    assert_eq!(
        main_diff_rendered_preview_toggle_kind(
            false,
            false,
            true,
            Some(RenderedPreviewKind::Markdown),
        ),
        Some(RenderedPreviewKind::Markdown)
    );
}

#[test]
fn rendered_preview_modes_track_each_kind_independently() {
    let mut modes = RenderedPreviewModes::default();

    assert_eq!(
        modes.get(RenderedPreviewKind::Svg),
        RenderedPreviewMode::Rendered
    );
    assert_eq!(
        modes.get(RenderedPreviewKind::Markdown),
        RenderedPreviewMode::Rendered
    );

    modes.set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
    modes.set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);

    assert_eq!(
        modes.get(RenderedPreviewKind::Svg),
        RenderedPreviewMode::Source
    );
    assert_eq!(
        modes.get(RenderedPreviewKind::Markdown),
        RenderedPreviewMode::Source
    );
}

#[test]
fn conflict_resolver_preview_mode_defaults_to_text() {
    assert_eq!(
        ConflictResolverPreviewMode::default(),
        ConflictResolverPreviewMode::Text
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

fn open_repo_state_with_workdir(workdir: &str) -> RepoState {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: normalize_bootstrap_repo_path(PathBuf::from(workdir)),
        },
    );
    repo.open = Loadable::Ready(());
    repo
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
    let state = AppState::default();

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
        ..AppState::default()
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
        ..AppState::default()
    };
    let mut repo_state =
        open_repo_state_with_workdir(repo.to_str().expect("test path should be unicode"));
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        area: DiffArea::Unstaged,
        path: PathBuf::from("src/conflict.txt"),
    });
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
        ..AppState::default()
    };
    let mut repo_state =
        open_repo_state_with_workdir(repo.to_str().expect("test path should be unicode"));
    repo_state.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        area: DiffArea::Unstaged,
        path: PathBuf::from("src/conflict.txt"),
    });
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

fn state_with_active_diff(path: &str, kind: FileStatusKind) -> AppState {
    let repo_id = RepoId(1);
    let path = PathBuf::from(path);
    let mut repo = open_repo_state_with_workdir("/repo");
    repo.worktree_status = Loadable::Ready(Arc::new(vec![FileStatus {
        path: path.clone(),
        kind,
        conflict: (kind == FileStatusKind::Conflicted)
            .then_some(gitcomet_core::domain::FileConflictKind::BothModified),
    }]));
    repo.diff_state.diff_target = Some(DiffTarget::WorkingTree {
        path,
        area: DiffArea::Unstaged,
    });
    AppState {
        active_repo: Some(repo_id),
        repos: vec![repo],
        ..AppState::default()
    }
}

#[test]
fn merge_view_target_requires_an_unstaged_conflict() {
    let normal = state_with_active_diff("src/normal.rs", FileStatusKind::Modified);
    let merge = state_with_active_diff("src/conflict.rs", FileStatusKind::Conflicted);

    assert!(active_merge_view_target(&normal).is_none());
    assert!(active_merge_view_target(&merge).is_some());
}

#[gpui::test]
fn merge_view_temporarily_collapses_and_restores_sidebar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/conflict.rs",
        FileStatusKind::Conflicted,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(view.read(app).sidebar_collapsed));

    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/normal.rs",
        FileStatusKind::Modified,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(!view.read(app).sidebar_collapsed));

    cx.update(|_window, app| {
        view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx));
    });
    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/conflict.rs",
        FileStatusKind::Conflicted,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(view.read(app).sidebar_collapsed));

    cx.update(|_window, app| {
        view.update(app, |this, cx| this.set_sidebar_collapsed(false, cx));
    });
    store.replace_snapshot_for_test(Arc::new(state_with_active_diff(
        "src/normal.rs",
        FileStatusKind::Modified,
    )));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| assert!(view.read(app).sidebar_collapsed));
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
    assert!(!should_show_splash_screen(
        GitCometViewMode::Normal,
        false,
        true
    ));
    assert!(should_show_splash_screen(
        GitCometViewMode::Normal,
        false,
        false
    ));
    assert!(!repository_entry_interstitial_active(
        GitCometViewMode::Normal,
        true
    ));
    assert!(titlebar_workspace_actions_enabled(
        GitCometViewMode::FocusedMergetool,
        false
    ));
    assert!(!titlebar_workspace_actions_enabled(
        GitCometViewMode::Normal,
        false
    ));
}

#[test]
fn focused_mergetool_keeps_titlebar_actions_without_repo_tabs_or_command_palette() {
    assert!(titlebar_workspace_actions_enabled(
        GitCometViewMode::FocusedMergetool,
        true
    ));
    assert!(!show_titlebar_repo_tabs(GitCometViewMode::FocusedMergetool));
    assert!(!command_palette_available(
        GitCometViewMode::FocusedMergetool
    ));

    assert!(show_titlebar_repo_tabs(GitCometViewMode::Normal));
    assert!(command_palette_available(GitCometViewMode::Normal));
}

#[gpui::test]
fn sidebar_resize_handle_straddles_the_content_card_edge(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(1))));
    sync_view_snapshot(cx, &view);

    let sidebar = cx
        .debug_bounds("sidebar_pane")
        .expect("expected the sidebar pane");
    let handle = cx
        .debug_bounds("pane_resize_sidebar")
        .expect("expected the sidebar resize handle");

    // The same rule the details handle follows: the grab strip is centered on
    // the boundary it drags, so its grip lands on the rule rather than beside
    // it. Without this the strip hangs entirely inside the content card.
    assert_eq!(
        handle.center().x,
        sidebar.right(),
        "sidebar resize handle must straddle the sidebar/card boundary"
    );
}

#[gpui::test]
fn sidebar_expand_after_collapse_does_not_reenter_root_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(false, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|_window, app| {
        assert!(!view.read(app).sidebar_collapsed);
    });
}

#[gpui::test]
fn collapsed_files_popover_uses_branch_style_rows_and_scrolls(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        (0..40)
            .map(|ix| FileEntry {
                name: format!("file_{ix}.txt"),
                path: Arc::new(PathBuf::from(format!("file_{ix}.txt"))),
                kind: FileEntryKind::File,
                depth: 0,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Files, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected collapsed Files popover");
    assert!(
        cx.debug_bounds("collapsed_file_browser_rows").is_some(),
        "collapsed Files should eagerly render intrinsic rows like branch popovers"
    );
    assert!(
        cx.debug_bounds("file_browser_scroll_container").is_none(),
        "collapsed Files must not use the full-sidebar virtualized viewport"
    );
    let scroll = cx.update(|_window, app| {
        view.read(app)
            .sidebar_pane
            .read(app)
            .collapsed_popover_scroll
            .clone()
    });
    assert!(
        scroll.max_offset().y > px(0.0),
        "collapsed popover scrollbar must observe overflowing rows"
    );
    assert!(
        components::Scrollbar::thumb_visible_for_test(&scroll, panel.size.height),
        "collapsed popover must render a scrollbar thumb for overflowing rows"
    );
    let surface = cx
        .debug_bounds("collapsed_sidebar_popover_content")
        .expect("expected collapsed popover scroll surface");
    let scrollbar_before = cx
        .debug_bounds("collapsed_sidebar_popover_scrollbar")
        .expect("expected collapsed popover scrollbar");
    assert_eq!(
        (scrollbar_before.top(), scrollbar_before.bottom()),
        (surface.top(), surface.bottom()),
        "scrollbar track must be anchored to the visible surface"
    );

    let before = cx
        .debug_bounds("file_browser_row_0")
        .expect("expected first file row")
        .top();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: panel.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    test_support::redraw(cx);
    let after = cx
        .debug_bounds("file_browser_row_0")
        .expect("expected first file row after scroll")
        .top();
    let scrollbar_after = cx
        .debug_bounds("collapsed_sidebar_popover_scrollbar")
        .expect("expected collapsed popover scrollbar after scroll");
    assert!(
        after < before - px(1.0),
        "mouse wheel must move collapsed file rows (before={before:?}, after={after:?})"
    );
    assert_eq!(
        scrollbar_after, scrollbar_before,
        "scrollbar track must stay fixed while its content scrolls"
    );
}

#[gpui::test]
fn collapsed_branch_popover_filter_spans_local_and_remote(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![Branch {
        name: "feature/alpha".to_string(),
        target: CommitId("deadbeef".into()),
        upstream: None,
        divergence: None,
    }]));
    state.repos[0].remote_branches = Loadable::Ready(Arc::new(vec![RemoteBranch {
        remote: "origin".to_string(),
        name: "feature/beta".to_string(),
        target: CommitId("deadbeef".into()),
    }]));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Local, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    assert!(
        cx.debug_bounds("collapsed_popover_filter_bar").is_none(),
        "the popover filter must stay hidden until its header toggle is used"
    );
    let toggle = cx
        .debug_bounds("collapsed_popover_filter_toggle")
        .expect("expected a filter toggle in the branch popover header");
    let section_menu = cx
        .debug_bounds("collapsed_popover_section_menu")
        .expect("expected a section menu button in the branch popover header");
    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected the collapsed branch popover");
    assert!(
        section_menu.left() >= toggle.right() && section_menu.right() <= panel.right(),
        "the header's two buttons must sit side by side inside the panel \
         (filter={toggle:?}, menu={section_menu:?}, panel={panel:?})"
    );

    cx.simulate_mouse_move(toggle.center(), None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        toggle.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        toggle.center(),
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    let filter_bar = cx
        .debug_bounds("collapsed_popover_filter_bar")
        .expect("expected the toggle to reveal the popover filter");
    // The branch sits under a `feature/` group header, so it is not row zero.
    let first_row = ["branch_row_1_0", "branch_row_1_1", "branch_row_1_2"]
        .into_iter()
        .find_map(|selector| cx.debug_bounds(selector))
        .expect("expected the popover to render branch rows");
    assert!(
        filter_bar.bottom() <= first_row.top(),
        "the filter box must sit above every branch row \
         (filter={filter_bar:?}, first row={first_row:?})"
    );
    assert!(
        filter_bar.top() > toggle.top(),
        "the filter box must sit below the popover header"
    );

    cx.simulate_keystrokes("b e t a");
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("branch_filter_group_remote").is_some(),
        "a Local popover filter must also surface Remote matches, under a Remote label"
    );
    let query = cx.update(|_window, app| {
        view.read(app)
            .sidebar_pane
            .read(app)
            .collapsed_popover_filter_query
            .clone()
    });
    assert_eq!(
        query, "beta",
        "keystrokes must reach the popover filter box"
    );
}

#[gpui::test]
fn collapsed_worktrees_popover_offers_its_section_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    // An empty section is the worst case: it has no rows to right-click, so
    // without the panel's own handler the click falls through to the history
    // canvas underneath (whose listener is window-level, not hitbox-gated).
    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![]));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_sidebar_collapsed(true, cx);
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Worktrees, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    let panel = cx
        .debug_bounds("collapsed_sidebar_popover")
        .expect("expected the collapsed Worktrees popover");
    assert!(
        cx.debug_bounds("collapsed_popover_section_menu").is_some(),
        "the popover header must expose the section's menu button"
    );

    // Low in the panel, below the header and the empty state.
    let empty_point = gpui::point(panel.center().x, panel.bottom() - px(24.0));
    cx.simulate_mouse_move(empty_point, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        empty_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        empty_point,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    test_support::redraw(cx);

    cx.update(|_window, app| {
        assert_eq!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::worktree(
                RepoId(1),
                WorktreePopoverKind::SectionMenu
            )),
            "right-clicking the popover must open the worktrees section menu"
        );
        assert_eq!(
            view.read(app).sidebar_collapsed_popover,
            Some(CollapsedSidebarSection::Worktrees),
            "the popover must stay open behind its own context menu"
        );
    });
}

#[gpui::test]
fn details_expand_after_collapse_does_not_reenter_root_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_details_collapsed(true, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_details_collapsed(false, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|_window, app| {
        assert!(!view.read(app).details_collapsed);
    });
}

/// The full-chrome layout keeps every large pane behind a `stable_cached_*`
/// boundary so a frame requested by one view (a spinner tick in the title bar,
/// a store update in the main pane) does not re-render the others. The bottom
/// status bar and the overlay hosts stay uncached: their paint ranges are
/// recorded after a focused TextInput registers its platform input handler,
/// and replaying them during a Wayland text-input redraw has panicked before.
#[test]
fn full_chrome_layout_caches_the_pane_subviews() {
    let splash_source = include_str!("splash.rs");
    let normalized: String = splash_source
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    // The repo tabs bar lives inside the title bar since the browser-style
    // chrome merge, so its cache boundary is the title bar mount in the render
    // implementation.
    let root_source = include_str!("gitcomet_view_render.rs");
    let normalized_root: String = root_source.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        normalized_root.contains(
            "stable_cached_fixed_height_view(self.title_bar.clone(),chrome::title_bar_height("
        ),
        "expected the title bar (hosting the repo tabs bar) to stay behind the stable cache boundary"
    );
    assert!(
        normalized.contains(
            "stable_cached_fixed_height_view(self.action_bar.clone(),action_bar_height(cx)"
        ),
        "expected action bar to stay behind the stable cache boundary"
    );
    assert!(
        normalized.contains("self.bottom_status_bar.clone(),"),
        "expected bottom status bar to mount directly"
    );
    assert!(
        normalized
            .matches("stable_cached_fill_view(self.main_pane.clone())")
            .count()
            >= 2,
        "expected both full-chrome main pane mount sites to stay cached"
    );
    assert!(
        normalized.contains("d.child(stable_cached_fill_view(self.sidebar_pane.clone()"),
        "expected the expanded sidebar pane to mount behind the stable cache boundary"
    );
    assert!(
        normalized.contains(".child(stable_cached_fill_view(self.details_pane.clone()"),
        "expected the expanded details pane to mount behind the stable cache boundary"
    );
    assert!(
        !normalized.contains(
            "stable_cached_fixed_height_view(self.bottom_status_bar.clone(),components::Tab::container_height("
        ),
        "bottom status bar must stay outside the stable cache boundary"
    );
}

#[gpui::test]
fn cached_sidebar_rerenders_when_the_mode_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let _cache_guard = enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    let renders_before =
        cx.update(|_window, app| view.read(app).sidebar_pane.read(app).render_count);

    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    let renders_after =
        cx.update(|_window, app| view.read(app).sidebar_pane.read(app).render_count);

    assert!(
        renders_after > renders_before,
        "the cached sidebar must be dirtied by a Branches/Files mode change"
    );
}

#[gpui::test]
fn splash_screen_renders_when_no_repositories_are_open(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("repository_entry_screen")
        .expect("expected repository entry splash screen");
    cx.debug_bounds("splash_headline")
        .expect("expected splash headline");
    cx.debug_bounds("splash_open_repo_action")
        .expect("expected splash open repository button");
    cx.debug_bounds("splash_clone_repo_action")
        .expect("expected splash clone repository button");

    #[cfg(not(target_os = "macos"))]
    assert!(
        cx.debug_bounds("app_menu").is_none(),
        "expected app menu button to be hidden on the splash screen"
    );

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(splash_active, "expected splash screen to be active");
}

#[gpui::test]
fn git_unavailable_splash_renders_open_settings_call_to_action(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let next = Arc::new(AppState {
        git_runtime: unavailable_git_runtime_state(),
        ..AppState::default()
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
        assert!(view.read(app).is_splash_screen_active());
        assert!(view.read(app).blocks_non_repository_actions());
    });
}

#[gpui::test]
fn git_unavailable_open_settings_button_publishes_expected_tooltip(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let next = Arc::new(AppState {
        git_runtime: unavailable_git_runtime_state(),
        ..AppState::default()
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
        let _ = window.draw(app);
    });

    let button_center = cx
        .debug_bounds("git_unavailable_open_settings")
        .expect("expected open settings call to action")
        .center();
    cx.simulate_mouse_move(button_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);

    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Open settings".to_string())
    );

    let icon_center = cx
        .debug_bounds("git_unavailable_status_icon")
        .expect("expected git unavailable status icon")
        .center();
    cx.simulate_mouse_move(icon_center, None, gpui::Modifiers::default());

    assert_eq!(
        test_support::tooltip_text(cx, &view),
        None,
        "expected the open settings tooltip to clear after leaving the button"
    );
}

#[gpui::test]
fn git_unavailable_overlay_blocks_open_repositories(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut next = AppState {
        git_runtime: unavailable_git_runtime_state(),
        active_repo: Some(RepoId(1)),
        ..AppState::default()
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
        assert!(!view.read(app).is_splash_screen_active());
        assert!(view.read(app).blocks_non_repository_actions());
    });
}

#[gpui::test]
fn git_unavailable_overlay_clears_after_runtime_recovery(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut unavailable = AppState {
        git_runtime: unavailable_git_runtime_state(),
        active_repo: Some(RepoId(1)),
        ..AppState::default()
    };
    unavailable.repos.push(open_repo_state_with_workdir(
        "/tmp/git-unavailable-recovery-test",
    ));
    let unavailable = Arc::new(unavailable);

    let mut recovered = AppState {
        git_runtime: available_git_runtime_state(),
        active_repo: Some(RepoId(1)),
        ..AppState::default()
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
fn splash_backdrop_renders_native_layers(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("splash_backdrop_native")
        .expect("expected native splash backdrop root");
    cx.debug_bounds("splash_backdrop_image")
        .expect("expected SVG-backed splash image layer");
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).splash_backdrop_image.format(),
            gpui::ImageFormat::Png,
            "expected splash backdrop to be preloaded before the first draw"
        );
    });
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

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(splash_active, "expected splash screen to remain active");
}

#[gpui::test]
fn splash_screen_buttons_publish_expected_tooltips(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let open_center = cx
        .debug_bounds("splash_open_repo_action")
        .expect("expected splash open repository button")
        .center();
    cx.simulate_mouse_move(open_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Open repository".to_string())
    );

    let clone_center = cx
        .debug_bounds("splash_clone_repo_action")
        .expect("expected splash clone repository button")
        .center();
    cx.simulate_mouse_move(clone_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Clone repository".to_string())
    );
}

#[gpui::test]
fn closing_last_repository_tab_returns_to_splash_screen(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    pump_for(cx, Duration::from_millis(120));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
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
    pump_for(cx, Duration::from_millis(120));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("repository_entry_screen")
        .expect("expected splash screen after closing the last repo");

    let splash_active = cx.update(|_window, app| view.read(app).is_splash_screen_active());
    assert!(
        splash_active,
        "expected splash screen to return after closing the last repo"
    );
}

#[gpui::test]
fn request_quit_or_warn_queues_terminal_shutdown_prompt(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

#[gpui::test]
fn cancel_pending_terminal_shutdown_clears_prompt(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

#[gpui::test]
fn closing_popover_clears_truncated_text_tooltip(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| {
            host.open_popover_at(
                PopoverKind::BranchPicker {
                    purpose: BranchPickerPurpose::Checkout,
                },
                point(px(72.0), px(72.0)),
                window,
                cx,
            );
        });

        let tooltip_host = view.read(app).tooltip_host.clone();
        tooltip_host.update(app, |host, cx| {
            host.set_tooltip_text_if_changed(Some("stale popover label".into()), cx);
        });

        popover_host.update(app, |host, cx| host.close_popover(cx));
    });

    assert_eq!(test_support::tooltip_text(cx, &view), None);
}

#[gpui::test]
fn removed_repo_tab_tooltip_does_not_reappear_after_hover_target_disappears(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);

    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from(
        "/tmp/splash-tooltip-clear-test",
    )));
    wait_until("repository tab to be added", || {
        !store_for_assert.snapshot().repos.is_empty()
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    pump_for(cx, Duration::from_millis(120));

    let repo_tab_center = cx
        .debug_bounds("repo_tab_1")
        .expect("expected repo tab to be rendered")
        .center();
    cx.simulate_mouse_move(repo_tab_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);

    let expected_tooltip = {
        let snapshot = store_for_assert.snapshot();
        let workdir = snapshot
            .repos
            .first()
            .map(|r| r.spec.workdir.clone())
            .unwrap_or_else(|| PathBuf::from("/tmp/splash-tooltip-clear-test"));
        path_display::path_display_string(&workdir)
    };
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some(expected_tooltip)
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
    pump_for(cx, Duration::from_millis(120));

    assert_eq!(
        test_support::tooltip_text(cx, &view),
        None,
        "expected repo tab tooltip to clear once its source view is removed"
    );

    let neutral_point = gpui::point(px(700.0), px(500.0));
    cx.simulate_mouse_move(neutral_point, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);

    assert_eq!(
        test_support::tooltip_text(cx, &view),
        None,
        "expected removed repo tab tooltip not to reappear after the mouse stops elsewhere"
    );
}

#[gpui::test]
fn removed_repo_tab_close_tooltip_does_not_reappear_after_hover_target_disappears(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    test_support::redraw(cx);

    store_for_assert.dispatch(Msg::OpenRepo(PathBuf::from(
        "/tmp/splash-close-tooltip-clear-test",
    )));
    wait_until("repository tab to be added", || {
        !store_for_assert.snapshot().repos.is_empty()
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    pump_for(cx, Duration::from_millis(120));

    let repo_tab_center = cx
        .debug_bounds("repo_tab_1")
        .expect("expected repo tab to be rendered")
        .center();
    cx.simulate_mouse_move(repo_tab_center, None, gpui::Modifiers::default());
    test_support::redraw(cx);

    let close_center = cx
        .debug_bounds("repo_tab_close_1")
        .expect("expected repo tab close button to be rendered while hovering the tab")
        .center();
    cx.simulate_mouse_move(close_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);

    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Close repository".to_string())
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
    pump_for(cx, Duration::from_millis(120));

    assert_eq!(
        test_support::tooltip_text(cx, &view),
        None,
        "expected repo tab close tooltip to clear once its source view is removed"
    );

    let neutral_point = gpui::point(px(700.0), px(500.0));
    cx.simulate_mouse_move(neutral_point, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);

    assert_eq!(
        test_support::tooltip_text(cx, &view),
        None,
        "expected removed repo tab close tooltip not to reappear after the mouse stops elsewhere"
    );
}

#[gpui::test]
fn loading_repo_tab_close_button_closes_repo(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let mut state = AppState {
        active_repo: Some(repo_id),
        ..AppState::default()
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

#[test]
fn generic_error_banner_is_hidden_when_auth_prompt_is_active() {
    assert!(GitCometView::should_render_generic_error_banner(false));
    assert!(!GitCometView::should_render_generic_error_banner(true));
}

#[test]
fn error_banner_overflow_hint_is_hidden_for_short_errors() {
    assert!(!GitCometView::should_show_error_banner_overflow_hint(
        "Submodule failed:\n\nfatal: branch not found"
    ));
}

#[test]
fn error_banner_overflow_hint_is_shown_for_long_command_failures() {
    let error = [
        "Submodule failed:",
        "",
        "    git submodule add --branch git-subtree /tmp/src comet2",
        "",
        "    Cloning into '/tmp/comet2'...",
        "    done.",
        "    fatal: 'origin/git-subtree' is not a commit and a branch 'git-subtree' cannot be created from it",
        "    fatal: unable to checkout submodule 'comet2'",
    ]
    .join("\n");
    assert!(GitCometView::should_show_error_banner_overflow_hint(&error));
}

#[test]
fn auth_prompt_banner_colors_use_accent_palette() {
    let theme = AppTheme::gitcomet_light();
    let (bg, border) = GitCometView::auth_prompt_banner_colors(theme);

    assert_eq!(bg, with_alpha(theme.colors.accent.foreground, 0.15));
    assert_eq!(border, with_alpha(theme.colors.accent.foreground, 0.3));
}

#[gpui::test]
fn apply_state_snapshot_routes_command_errors_into_store_backed_banner(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let error = "Fetch failed".to_string();
    let mut next = AppState::default();
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("repo"),
        },
    );
    repo.feedback.last_error = Some(error.clone());
    repo.feedback
        .command_log
        .push(gitcomet_state::model::CommandLogEntry {
            time: std::time::SystemTime::now(),
            ok: false,
            command: "git fetch".to_string(),
            summary: error.clone(),
            stdout: String::new(),
            stderr: "fatal: test".to_string(),
            announce_success: true,
            hook_operation_id: None,
        });
    next.active_repo = Some(repo_id);
    next.repos.push(repo);
    let next = Arc::new(next);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
    });

    wait_until("store-backed banner error", || {
        let snapshot = store_for_assert.snapshot();
        snapshot
            .banner_error
            .as_ref()
            .is_some_and(|banner| banner.repo_id == Some(repo_id) && banner.message == error)
    });
}

#[gpui::test]
fn apply_state_snapshot_routes_clone_progress_errors_into_global_banner(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut next = AppState {
        active_repo: Some(RepoId(1)),
        ..AppState::default()
    };
    next.repos
        .push(open_repo_state_with_workdir("/tmp/existing-active-repo"));
    next.clone = Some(gitcomet_state::model::CloneOpState {
        url: Arc::<str>::from("git@github.com:private/repo.git"),
        dest: Arc::new(PathBuf::from("/tmp/private-repo")),
        status: gitcomet_state::model::CloneOpStatus::FinishedErr(
            "Clone failed:\n\ngit@github.com: Permission denied (publickey).".to_string(),
        ),
        progress: gitcomet_state::model::CloneProgressMeter::default(),
        seq: 1,
        output_tail: std::collections::VecDeque::new(),
    });
    let next = Arc::new(next);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
    });
    cx.run_until_parked();

    wait_until("global clone banner error", || {
        let snapshot = store_for_assert.snapshot();
        snapshot.banner_error.as_ref().is_some_and(|banner| {
            banner.repo_id.is_none()
                && banner.message
                    == "Clone failed:\n\ngit@github.com: Permission denied (publickey)."
        })
    });
}

#[gpui::test]
fn try_auth_prompt_submit_passphrase_without_secret_shows_error(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::default();
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "Enter passphrase".to_string(),
        operation: AuthRetryOperation::Clone {
            url: "git@example.com:repo.git".to_string(),
            dest: PathBuf::from("/tmp/repo"),
        },
    });
    let state = Arc::new(state);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&state), cx);
            this.try_auth_prompt_submit(cx);
        });
    });

    wait_until("empty passphrase should show banner error", || {
        store_for_assert
            .snapshot()
            .banner_error
            .as_ref()
            .is_some_and(|b| b.message.contains("Passphrase is required"))
    });
}

#[gpui::test]
fn try_auth_prompt_submit_passphrase_dispatches_submit(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::default();
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::Passphrase,
        reason: "Enter passphrase".to_string(),
        operation: AuthRetryOperation::Clone {
            url: "git@example.com:repo.git".to_string(),
            dest: PathBuf::from("/tmp/repo"),
        },
    });
    let state = Arc::new(state);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&state), cx);
            this.auth_prompt_secret_input
                .update(cx, |input, cx| input.set_text("my-passphrase", cx));
            this.try_auth_prompt_submit(cx);
        });
    });

    wait_until(
        "auth prompt should be cleared after successful submit",
        || store_for_assert.snapshot().auth_prompt.is_none(),
    );
}

#[gpui::test]
fn try_auth_prompt_submit_username_password_empty_username_shows_error(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::default();
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::Clone {
            url: "https://example.com/repo.git".to_string(),
            dest: PathBuf::from("/tmp/repo"),
        },
    });
    let state = Arc::new(state);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&state), cx);
            this.auth_prompt_secret_input
                .update(cx, |input, cx| input.set_text("token-123", cx));
            this.try_auth_prompt_submit(cx);
        });
    });

    wait_until("empty username should show banner error", || {
        store_for_assert
            .snapshot()
            .banner_error
            .as_ref()
            .is_some_and(|b| b.message.contains("Username is required"))
    });
}

#[gpui::test]
fn try_auth_prompt_submit_username_password_dispatches_submit(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::default();
    state.auth_prompt = Some(AuthPromptState {
        kind: AuthPromptKind::UsernamePassword,
        reason: "auth required".to_string(),
        operation: AuthRetryOperation::Clone {
            url: "https://example.com/repo.git".to_string(),
            dest: PathBuf::from("/tmp/repo"),
        },
    });
    let state = Arc::new(state);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&state), cx);
            this.auth_prompt_username_input
                .update(cx, |input, cx| input.set_text("alice", cx));
            this.auth_prompt_secret_input
                .update(cx, |input, cx| input.set_text("token-123", cx));
            this.try_auth_prompt_submit(cx);
        });
    });

    wait_until(
        "auth prompt should be cleared after successful submit with credentials",
        || store_for_assert.snapshot().auth_prompt.is_none(),
    );
}

#[test]
fn pane_collapse_ease_is_a_well_formed_easing_curve() {
    // Endpoints are pinned.
    assert_eq!(GitCometView::pane_collapse_ease(0.0), 0.0);
    assert_eq!(GitCometView::pane_collapse_ease(1.0), 1.0);

    // Out-of-range inputs clamp to the endpoints.
    assert_eq!(GitCometView::pane_collapse_ease(-0.5), 0.0);
    assert_eq!(GitCometView::pane_collapse_ease(1.5), 1.0);

    // Monotonically non-decreasing across the domain.
    let mut prev = 0.0;
    for i in 0..=100 {
        let t = i as f32 / 100.0;
        let y = GitCometView::pane_collapse_ease(t);
        assert!(
            y >= prev - 1e-4,
            "easing should be monotonic: y({t}) = {y} < previous {prev}"
        );
        assert!(
            (0.0..=1.0).contains(&y),
            "easing stays in [0, 1]: y({t}) = {y}"
        );
        prev = y;
    }

    // Fast-out, slow-in: past the halfway mark well before the halfway time.
    assert!(GitCometView::pane_collapse_ease(0.5) > 0.5);
}

#[test]
fn cubic_bezier_matches_a_linear_curve_for_the_identity_control_points() {
    // cubic-bezier(1/3, 1/3, 2/3, 2/3) is the straight line y = x.
    for i in 0..=20 {
        let t = i as f32 / 20.0;
        let y = GitCometView::cubic_bezier(1.0 / 3.0, 1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0, t);
        assert!((y - t).abs() < 1e-3, "linear bezier: y({t}) = {y}");
    }
}

#[gpui::test]
fn locate_open_file_switches_to_files_and_expands_its_folders(cx: &mut gpui::TestAppContext) {
    // The action is reachable from a shortcut, the app menu and the palette, so
    // it has to work with the sidebar on Branches and the folders collapsed.
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let nested = PathBuf::from("src/inner/deep.rs");
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: FileEntryKind::Directory,
            depth: 0,
        },
        FileEntry {
            name: "inner".to_string(),
            path: Arc::new(PathBuf::from("src/inner")),
            kind: FileEntryKind::Directory,
            depth: 1,
        },
        FileEntry {
            name: "deep.rs".to_string(),
            path: Arc::new(nested.clone()),
            kind: FileEntryKind::File,
            depth: 2,
        },
    ]));
    state.repos[0].file_browser.bump_rev();
    state.repos[0].diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: nested.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    state.repos[0].diff_state.content_preview = true;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    cx.update(|_window, app| {
        view.update(app, |this, cx| this.locate_open_file_in_explorer(cx));
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let state = view.read(app).store.snapshot();
        assert_eq!(
            state.sidebar_mode,
            gitcomet_state::model::SidebarMode::Files,
            "locating has to bring the tree it scrolls into view"
        );
        let expanded = &state.repos[0].file_browser.expanded_dirs;
        assert!(expanded.contains(&Arc::new(PathBuf::from("src"))));
        assert!(expanded.contains(&Arc::new(PathBuf::from("src/inner"))));
    });
}

#[gpui::test]
fn the_locate_button_is_present_whenever_the_files_tab_is(cx: &mut gpui::TestAppContext) {
    // It used to be gated on a file being open as well, so the strip's contents
    // changed as files were opened and closed — and it was simply absent for
    // anyone who had not opened one yet.
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    assert!(
        cx.debug_bounds("sidebar_locate_open_file").is_none(),
        "the Branches tab has no tree to locate anything in"
    );

    // Files, still with no file open: present, and disabled rather than absent.
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    assert!(
        cx.debug_bounds("sidebar_locate_open_file").is_some(),
        "the locate button belongs to the Files tab, open file or not"
    );

    state.repos[0].diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
        path: PathBuf::from("src/main.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    });
    state.repos[0].diff_state.content_preview = true;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    assert!(cx.debug_bounds("sidebar_locate_open_file").is_some());
}

#[gpui::test]
fn file_explorer_pins_and_marks_files_with_unsaved_editor_buffers(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        ["a.rs", "b.rs", "c.rs"]
            .into_iter()
            .map(|name| FileEntry {
                name: name.to_string(),
                path: Arc::new(PathBuf::from(name)),
                kind: FileEntryKind::File,
                depth: 0,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    assert!(
        cx.debug_bounds("file_browser_unsaved_header").is_none(),
        "with nothing unsaved the section must take no space at all"
    );

    // Stash a dirty buffer for `b.rs` -- the case the section exists for, since
    // a file edited and navigated away from is the one hardest to find again.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_stash.insert(
                    (RepoId(1), PathBuf::from("b.rs")),
                    crate::view::panes::main::StashedFileEdit {
                        text: SharedString::from("edited\n"),
                        cursor: 0,
                        text_fingerprint: 1,
                        saved_fingerprint: 2,
                        first_dirty_line: Some(0),
                    },
                );
                pane.sync_unsaved_file_edits_rev(cx);
            });
        });
    });
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("file_browser_unsaved_header").is_some(),
        "an unsaved buffer must pin a section at the top of the explorer"
    );
    let pinned = cx
        .debug_bounds("file_browser_unsaved_1")
        .expect("the unsaved file gets a pinned row");
    assert!(
        cx.debug_bounds("file_browser_unsaved_discard_1").is_some(),
        "the pinned row carries its own discard control"
    );
    // Row 0 is the header and row 1 the file, so the tree starts at row 2: the
    // pinned rows sit above the tree rather than replacing it.
    let first_tree_row = cx
        .debug_bounds("file_browser_row_2")
        .expect("the tree is still listed below the pinned section");
    assert!(
        pinned.top() < first_tree_row.top(),
        "pinned rows come first: pinned at {:?}, tree at {:?}",
        pinned.top(),
        first_tree_row.top()
    );

    // Discarding through the same entry point the row's button uses clears it.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.discard_file_edits_for(RepoId(1), &PathBuf::from("b.rs"), cx);
            });
        });
    });
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("file_browser_unsaved_header").is_none(),
        "discarding the last unsaved buffer removes the section again"
    );
    assert!(
        cx.debug_bounds("file_browser_row_0").is_some(),
        "and the tree closes back up to the top"
    );
}

/// Folder rows carried no context-menu invoker at all until this menu existed,
/// so the right-click handler had nothing to light up and was simply never
/// attached. This drives the real row to catch a regression back to that.
#[gpui::test]
fn right_clicking_a_folder_row_opens_the_folder_context_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(vec![
        FileEntry {
            name: "src".to_string(),
            path: Arc::new(PathBuf::from("src")),
            kind: FileEntryKind::Directory,
            depth: 0,
        },
        FileEntry {
            name: "a.rs".to_string(),
            path: Arc::new(PathBuf::from("a.rs")),
            kind: FileEntryKind::File,
            depth: 0,
        },
    ]));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let folder_row = cx
        .debug_bounds("file_browser_row_0")
        .expect("the folder is the first tree row");
    let center = folder_row.center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "right-clicking a folder must open a context menu"
    );
    // A folder-only entry: proof this is the folder menu rather than the file
    // menu firing on the wrong row.
    assert!(
        cx.debug_bounds("context_menu_expand_all_under_here")
            .is_some(),
        "expected the folder menu's recursive expand entry"
    );
    assert!(
        cx.debug_bounds("context_menu_copy_absolute_path").is_some(),
        "expected the folder menu's copy entries"
    );
    // The folder row is the only row that pairs a state-mutating `on_click`
    // with a right-button handler, so opening the menu must not also toggle it
    // — otherwise every right-click would collapse the folder under the menu.
    assert!(
        store
            .snapshot()
            .repos
            .iter()
            .all(|repo| repo.file_browser.expanded_dirs.is_empty()),
        "right-clicking a folder must not toggle it"
    );
}

/// Clicking a file the editor is holding unsaved text for must land back in the
/// editor, not in the read-only view -- which would show the copy on disk and
/// look like the edits were lost.
#[gpui::test]
fn clicking_a_file_with_unsaved_edits_opens_the_editor(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    state.repos[0].file_browser.entries = Loadable::Ready(Arc::new(
        ["a.rs", "b.rs"]
            .into_iter()
            .map(|name| FileEntry {
                name: name.to_string(),
                path: Arc::new(PathBuf::from(name)),
                kind: FileEntryKind::File,
                depth: 0,
            })
            .collect(),
    ));
    state.repos[0].file_browser.bump_rev();
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    // A clean tree: clicking a file opens the read-only content view.
    click_debug_selector(cx, "file_browser_row_0");
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
    cx.update(|_window, app| {
        let repo = &view.read(app).state.repos[0];
        assert!(
            repo.diff_state.content_preview && !repo.diff_state.edit_mode,
            "a file with nothing unsaved opens read-only"
        );
    });

    // Now give `b.rs` an unsaved buffer and click it in the tree.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_editor_stash.insert(
                    (RepoId(1), PathBuf::from("b.rs")),
                    crate::view::panes::main::StashedFileEdit {
                        text: SharedString::from("edited\n"),
                        cursor: 0,
                        text_fingerprint: 1,
                        saved_fingerprint: 2,
                        first_dirty_line: Some(0),
                    },
                );
                pane.sync_unsaved_file_edits_rev(cx);
            });
        });
    });
    test_support::redraw(cx);

    // Rows 0 and 1 are now the pinned section, so `b.rs` sits at tree row 3.
    click_debug_selector(cx, "file_browser_row_3");
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    cx.update(|_window, app| {
        let repo = &view.read(app).state.repos[0];
        assert!(
            repo.diff_state.edit_mode,
            "a file with unsaved edits opens straight into the editor"
        );
    });

    // And the pinned row itself does the same, from a read-only starting point.
    cx.update(|_window, app| {
        let mut state = (*view.read(app).state).clone();
        state.repos[0].diff_state.edit_mode = false;
        state.repos[0].diff_state.content_preview = true;
        store.replace_snapshot_for_test(Arc::new(state));
    });
    sync_view_snapshot(cx, &view);
    click_debug_selector(cx, "file_browser_unsaved_1");
    test_support::redraw(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    cx.update(|_window, app| {
        assert!(
            view.read(app).state.repos[0].diff_state.edit_mode,
            "the pinned row opens the editor too"
        );
    });
}

#[gpui::test]
fn sidebar_worktree_badges_share_one_right_edge_near_the_pane_edge(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    let branch = |name: &str| gitcomet_core::domain::Branch {
        name: name.to_string(),
        target: CommitId("1111111111111111".into()),
        upstream: None,
        divergence: None,
    };
    let worktree = |path: &str, branch: &str| gitcomet_core::domain::Worktree {
        path: PathBuf::from(path),
        head: None,
        branch: Some(branch.to_string()),
        detached: false,
    };
    // Names and badge labels of deliberately different widths: the badges are
    // pushed against the trailing edge, so none of that may reach their right
    // edge.
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![
        branch("alpha"),
        branch("beta"),
        branch("gamma-with-a-much-longer-name"),
    ]));
    state.repos[0].branches_rev = 1;
    state.repos[0].worktrees = Loadable::Ready(Arc::new(vec![
        worktree("/tmp/wt-alpha", "alpha"),
        worktree("/tmp/wt-beta-considerably-longer", "beta"),
        worktree("/tmp/g", "gamma-with-a-much-longer-name"),
    ]));
    state.repos[0].worktrees_rev = 1;
    state.repos[0].branch_sidebar_rev = 1;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let sidebar = cx
        .debug_bounds("sidebar_pane")
        .expect("expected the sidebar pane");
    let badges: Vec<_> = (0..12usize)
        .filter_map(|ix| {
            let selector: &'static str =
                Box::leak(format!("branch_workspace_badge_{ix}").into_boxed_str());
            cx.debug_bounds(selector)
        })
        .collect();
    assert!(
        badges.len() >= 3,
        "expected a worktree badge on each branch that has one, got {}",
        badges.len()
    );

    let first_right = badges[0].right();
    for badge in &badges {
        assert_eq!(
            badge.right(),
            first_right,
            "worktree badges must share one right edge regardless of label width"
        );
    }

    // What is left between the badges and the pane edge is the reserved `⋮`
    // slot, the gap before it, and the row-highlight inset — nothing else.
    let trailing_gap = sidebar.right() - first_right;
    assert!(
        trailing_gap <= px(30.0),
        "worktree badges should sit close to the pane's right edge, got {trailing_gap:?}"
    );
}

/// Branch group rows carried no context-menu invoker and no right-click handler
/// at all until this menu existed. This drives the real row to catch a
/// regression back to that.
#[gpui::test]
fn right_clicking_a_branch_group_row_opens_the_group_context_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    let branch = |name: &str| gitcomet_core::domain::Branch {
        name: name.to_string(),
        target: gitcomet_core::domain::CommitId("aaaaaaaaaaaa".into()),
        upstream: None,
        divergence: None,
    };
    state.repos[0].head_branch = Loadable::Ready("main".to_string());
    state.repos[0].branches = Loadable::Ready(Arc::new(vec![
        branch("main"),
        branch("feat/a"),
        branch("feat/b"),
    ]));
    state.repos[0].branches_rev = 1;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);

    let group_row = cx
        .debug_bounds("branch_group_0")
        .or_else(|| cx.debug_bounds("branch_group_1"))
        .or_else(|| cx.debug_bounds("branch_group_2"))
        .expect("the feat/ group renders a row");
    assert_eq!(
        group_row.size.height,
        px(24.0),
        "branch hierarchy rows must remain 24 px tall"
    );
    let center = group_row.center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Right, gpui::Modifiers::default());
    test_support::redraw(cx);

    assert!(
        cx.debug_bounds("app_popover").is_some(),
        "right-clicking a branch group must open a context menu"
    );
    // A group-only entry: proof this is the group menu rather than the section
    // or branch menu firing on the wrong row.
    assert!(
        cx.debug_bounds("context_menu_expand_all_under_here")
            .is_some(),
        "expected the group menu's recursive expand entry"
    );

    // The group row pairs a collapse-toggling `on_click` with the new
    // right-button handler, so opening the menu must not also collapse the
    // group under it.
    let collapsed_after = cx.update(|_window, app| {
        view.read(app)
            .sidebar_pane
            .read(app)
            .collapsed_items_for_test()
    });
    assert!(
        collapsed_after.is_empty(),
        "right-clicking a branch group must not toggle it, got {collapsed_after:?}"
    );
}
