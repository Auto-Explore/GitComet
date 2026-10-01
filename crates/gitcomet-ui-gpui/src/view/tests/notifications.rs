//! Notifications: toasts, error routing, crash reports, auth prompts, tooltips.

use super::*;

#[gpui::test]
fn startup_crash_report_is_visible_after_relaunch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let backend: Arc<dyn GitBackend> = Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(backend);
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
    let (store, events) = AppStore::new_test(backend);
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
    let (store, events) = AppStore::new_test(backend);
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

#[test]
fn toast_total_lifetime_includes_fade_in_and_out() {
    let ttl = Duration::from_secs(6);
    assert_eq!(
        toast_total_lifetime(ttl),
        ttl + Duration::from_millis(TOAST_FADE_IN_MS + TOAST_FADE_OUT_MS)
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

#[gpui::test]
fn git_unavailable_open_settings_button_publishes_expected_tooltip(cx: &mut gpui::TestAppContext) {
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
fn home_screen_buttons_publish_expected_tooltips(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let open_center = cx
        .debug_bounds("home_open_repo_action")
        .expect("expected Home open repository button")
        .center();
    cx.simulate_mouse_move(open_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Open an existing repository".to_string())
    );

    let clone_center = cx
        .debug_bounds("home_clone_repo_action")
        .expect("expected Home clone repository button")
        .center();
    cx.simulate_mouse_move(clone_center, None, gpui::Modifiers::default());
    test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some("Clone a repository from a URL".to_string())
    );
}

#[gpui::test]
fn closing_popover_clears_truncated_text_tooltip(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    pump_until(cx, "repository tab to render", |cx| {
        cx.debug_bounds("repo_tab_1").is_some()
    });

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
    pump_until(cx, "removed tab and tooltip to disappear", |cx| {
        cx.debug_bounds("repo_tab_1").is_none() && test_support::tooltip_text(cx, &view).is_none()
    });

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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    pump_until(cx, "repository tab to render", |cx| {
        cx.debug_bounds("repo_tab_1").is_some()
    });

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
    pump_until(cx, "removed tab and close tooltip to disappear", |cx| {
        cx.debug_bounds("repo_tab_1").is_none() && test_support::tooltip_text(cx, &view).is_none()
    });

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

/// The errors on screen as `(repo, message)`, newest first.
fn error_toasts(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
) -> Vec<(Option<RepoId>, String)> {
    cx.update(|_window, app| {
        view.read(app)
            .toast_host
            .read(app)
            .error_notices()
            .into_iter()
            .map(|(_, notice)| (notice.repo_id, notice.message.clone()))
            .collect()
    })
}

#[test]
fn auth_prompt_banner_colors_use_accent_palette() {
    let theme = AppTheme::gitcomet_light();
    let (bg, border) = GitCometView::auth_prompt_banner_colors(theme);

    assert_eq!(bg, with_alpha(theme.colors.accent.foreground, 0.15));
    assert_eq!(border, with_alpha(theme.colors.accent.foreground, 0.3));
}

#[gpui::test]
fn apply_state_snapshot_routes_command_errors_into_error_toasts(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let error = "Fetch failed".to_string();
    let mut next = AppState::test_default();
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
            stdout: "".into(),
            stderr: "fatal: test".into(),
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

    assert_eq!(
        error_toasts(cx, &view),
        vec![(Some(repo_id), error.clone())]
    );
    // A snapshot that repeats nothing new adds nothing.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::clone(&next), cx);
        });
    });
    assert_eq!(error_toasts(cx, &view).len(), 1);
}

#[gpui::test]
fn one_failure_recorded_twice_is_one_error(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let message = "Failed to persist session state while opening: disk full".to_string();
    let mut next = AppState::test_default();
    let mut repo = open_repo_state_with_workdir("/tmp/persist-repo");
    repo.feedback
        .diagnostics
        .push(gitcomet_state::model::DiagnosticEntry {
            time: std::time::SystemTime::now(),
            kind: DiagnosticKind::Error,
            message: message.clone(),
        });
    next.notifications
        .push(gitcomet_state::model::AppNotification {
            time: std::time::SystemTime::now(),
            kind: gitcomet_state::model::AppNotificationKind::Error,
            message: message.clone(),
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

    let counts = cx.update(|_window, app| {
        view.read(app)
            .toast_host
            .read(app)
            .error_notices()
            .into_iter()
            .map(|(_, notice)| (notice.message.clone(), notice.count))
            .collect::<Vec<_>>()
    });
    assert_eq!(counts, vec![(message, 1)]);
}

#[gpui::test]
fn apply_state_snapshot_routes_clone_progress_errors_into_global_error_toasts(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut next = AppState {
        active_repo: Some(RepoId(1)),
        ..AppState::test_default()
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

    assert_eq!(
        error_toasts(cx, &view),
        vec![(
            None,
            "Clone failed:\n\ngit@github.com: Permission denied (publickey).".to_string()
        )]
    );
}

#[gpui::test]
fn try_auth_prompt_submit_passphrase_without_secret_shows_error(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::test_default();
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

    assert!(
        error_toasts(cx, &view)
            .iter()
            .any(|(_, message)| message.contains("Passphrase is required")),
        "an empty passphrase shows an error, visible above the auth prompt"
    );
}

#[gpui::test]
fn try_auth_prompt_submit_passphrase_dispatches_submit(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::test_default();
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::test_default();
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

    assert!(
        error_toasts(cx, &view)
            .iter()
            .any(|(_, message)| message.contains("Username is required"))
    );
}

#[gpui::test]
fn try_auth_prompt_submit_username_password_dispatches_submit(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_assert = store.clone();
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::test_default();
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

#[gpui::test]
fn outdated_git_shows_one_sticky_update_notice(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    crate::view::git_version_notice::reset_outdated_git_notice_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let with_git = |version_output: &str| {
        Arc::new(AppState {
            git_runtime: GitRuntimeState {
                preference: GitExecutablePreference::SystemPath,
                availability: GitExecutableAvailability::Available {
                    version_output: version_output.to_string(),
                },
            },
            ..AppState::test_default()
        })
    };

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(with_git("git version 2.45.1.windows.1"), cx);
            this.apply_state_snapshot(with_git("git version 2.55.0"), cx);
            // Back to the same old git (another window, a re-probe): no repeat.
            this.apply_state_snapshot(with_git("git version 2.45.1.windows.1"), cx);
        });
        let _ = window.draw(app);
    });

    let notices = cx.update(|_window, app| {
        view.read(app)
            .toast_host
            .read(app)
            .toasts_for_tests(app)
            .into_iter()
            .filter(|(_, message)| message.contains("older than 2.53"))
            .collect::<Vec<_>>()
    });
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert_eq!(notices[0].0, components::ToastKind::Warning);
    assert!(notices[0].1.starts_with("Git 2.45 is older"), "{notices:?}");
}
