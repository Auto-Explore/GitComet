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
            handled_by_auth_prompt: false,
            failure_id: None,
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

const WINDOWS_SSH_PASSPHRASE_CHALLENGE: &str = "git@github.com: Permission denied (publickey).\n\
fatal: Could not read from remote repository.\n\
Please make sure you have the correct access rights\n\
and the repository exists.\n\
SSH key passphrase prompt:\n\
Enter passphrase for key 'C:\\Users\\sampo/.ssh/id_ed25519':";

const AUTH_PROMPT_KINDS: [AuthPromptKind; 2] =
    [AuthPromptKind::Passphrase, AuthPromptKind::UsernamePassword];

fn announced_command(summary: &str) -> gitcomet_state::model::CommandLogEntry {
    gitcomet_state::model::CommandLogEntry {
        time: std::time::SystemTime::now(),
        ok: true,
        command: "git push".into(),
        summary: summary.into(),
        stdout: "".into(),
        stderr: "".into(),
        announce_success: true,
        handled_by_auth_prompt: false,
        failure_id: None,
        hook_operation_id: None,
    }
}

fn warning(message: &str) -> gitcomet_state::model::AppNotification {
    gitcomet_state::model::AppNotification {
        time: std::time::SystemTime::now(),
        kind: gitcomet_state::model::AppNotificationKind::Warning,
        message: message.into(),
    }
}

#[gpui::test]
fn toasts_follow_feedback_sequences_after_shown_entries_are_dropped(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut repo = open_repo_state_with_workdir("/tmp/acknowledged-feedback");
    for ix in 1..=3 {
        repo.feedback.command_log_seq += 1;
        repo.feedback
            .command_log
            .push(announced_command(&format!("Pushed {ix}")));
    }
    let mut state = AppState {
        active_repo: Some(repo.id),
        repos: vec![repo],
        notifications: vec![warning("Warning 1")],
        notifications_seq: 1,
        ..AppState::test_default()
    };
    apply_notification_snapshot(cx, &view, &state);

    // The store dropped what the window acknowledged and one more of each
    // arrived meanwhile: both queues got shorter, yet each holds news.
    state.notifications = vec![warning("Warning 2")];
    state.notifications_seq = 2;
    let feedback = &mut state.repos[0].feedback;
    feedback.command_log = vec![announced_command("Pushed 4")];
    feedback.command_log_seq = 4;
    apply_notification_snapshot(cx, &view, &state);
    apply_notification_snapshot(cx, &view, &state);

    let toasts = all_toasts(cx, &view)
        .into_iter()
        .map(|(_, message)| message)
        .collect::<Vec<_>>();
    assert_eq!(
        toasts,
        [
            "Warning 1",
            "Pushed 1",
            "Pushed 2",
            "Pushed 3",
            "Warning 2",
            "Pushed 4"
        ]
    );
}

#[gpui::test]
fn shown_feedback_is_released_from_the_store(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let store = cx.update(|_window, app| view.read(app).store.clone());
    // Still opening, so the view requests no loads that would add feedback.
    let repo_id = RepoId(1);
    let mut repo = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/released-feedback"),
        },
    );
    repo.feedback.command_log_seq = 1;
    repo.feedback.command_log.push(announced_command("Pushed"));
    repo.feedback.diagnostics_seq = 1;
    repo.feedback
        .diagnostics
        .push(gitcomet_state::model::DiagnosticEntry {
            time: std::time::SystemTime::now(),
            kind: DiagnosticKind::Error,
            message: "Disk full".into(),
        });
    store.replace_snapshot_for_test(Arc::new(AppState {
        active_repo: Some(repo_id),
        repos: vec![repo],
        notifications: vec![warning("Low on space")],
        notifications_seq: 1,
        ..AppState::test_default()
    }));
    let show_store_state = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
        });
        test_support::drain_store_worker(&view, cx);
    };
    let queued = |store: &AppStore| {
        let state = store.snapshot();
        let feedback = &state.repos[0].feedback;
        (
            state.notifications.len(),
            feedback.diagnostics.len(),
            feedback.command_log.len(),
        )
    };

    show_store_state(cx);
    assert_eq!(
        all_toasts(cx, &view)
            .into_iter()
            .map(|(_, message)| message)
            .collect::<Vec<_>>(),
        ["Low on space", "Pushed", "Disk full"]
    );
    assert_eq!(queued(&store), (0, 0, 0), "the window acknowledged them");

    store.dispatch(Msg::ReportError {
        repo_id: Some(repo_id),
        message: "Disk still full".into(),
    });
    test_support::drain_store_worker(&view, cx);
    assert_eq!(queued(&store), (0, 1, 0));
    show_store_state(cx);
    assert_eq!(
        error_toasts(cx, &view),
        vec![
            (Some(repo_id), "Disk still full".to_string()),
            (Some(repo_id), "Disk full".to_string()),
        ]
    );
    assert_eq!(queued(&store), (0, 0, 0));
}

fn auth_challenge(kind: AuthPromptKind) -> &'static str {
    match kind {
        AuthPromptKind::Passphrase => WINDOWS_SSH_PASSPHRASE_CHALLENGE,
        AuthPromptKind::UsernamePassword => {
            "fatal: Authentication failed for 'https://example.com/private/repo.git/'"
        }
        AuthPromptKind::HostVerification => unreachable!("these tests require credential input"),
    }
}

fn auth_rejection(kind: AuthPromptKind) -> &'static str {
    match kind {
        AuthPromptKind::Passphrase => "The passphrase could not unlock this key. Try again.",
        AuthPromptKind::UsernamePassword => "The username or password was not accepted. Try again.",
        AuthPromptKind::HostVerification => unreachable!("these tests require credential input"),
    }
}

fn auth_pull_attempt(kind: AuthPromptKind) -> gitcomet_state::model::CommandLogEntry {
    let challenge = auth_challenge(kind);
    gitcomet_state::model::CommandLogEntry {
        time: std::time::SystemTime::UNIX_EPOCH,
        ok: false,
        command: "git pull --prune".into(),
        summary: format!("Pull failed:\n\n{challenge}"),
        stdout: "".into(),
        stderr: challenge.into(),
        announce_success: true,
        handled_by_auth_prompt: true,
        failure_id: None,
        hook_operation_id: None,
    }
}

fn auth_pull_state(kind: AuthPromptKind) -> AppState {
    let mut repo = open_repo_state_with_workdir("/tmp/auth-pull-notifications");
    repo.feedback.command_log.push(auth_pull_attempt(kind));
    AppState {
        active_repo: Some(repo.id),
        auth_prompt: Some(AuthPromptState {
            generation: 1,
            challenge: None,
            status: AuthPromptStatus::AwaitingInput,
            kind,
            reason: auth_challenge(kind).into(),
            operation: AuthRetryOperation::RepoCommand {
                repo_id: repo.id,
                command: gitcomet_state::msg::RepoCommandKind::Pull {
                    mode: PullMode::Default,
                },
            },
        }),
        repos: vec![repo],
        ..AppState::test_default()
    }
}

fn successful_pull_attempt() -> gitcomet_state::model::CommandLogEntry {
    gitcomet_state::model::CommandLogEntry {
        time: std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1),
        ok: true,
        summary: "Pull: Fast-forwarded".into(),
        stdout: "Updating 1111111..2222222\nFast-forward\n".into(),
        stderr: "".into(),
        handled_by_auth_prompt: false,
        ..auth_pull_attempt(AuthPromptKind::Passphrase)
    }
}

fn apply_notification_snapshot(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    state: &AppState,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.apply_state_snapshot(Arc::new(state.clone()), cx);
        });
    });
}

fn all_toasts(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
) -> Vec<(components::ToastKind, String)> {
    cx.update(|_window, app| view.read(app).toast_host.read(app).toasts_for_tests(app))
}

fn first_auth_input(view: &GitCometView, kind: AuthPromptKind) -> Entity<components::TextInput> {
    if kind == AuthPromptKind::UsernamePassword {
        view.auth_prompt_username_input.clone()
    } else {
        view.auth_prompt_secret_input.clone()
    }
}

/// Moves the keyboard to the diff panel, somewhere that is not a text field.
fn focus_diff_panel(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
) -> gpui::FocusHandle {
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .main_pane
            .read(app)
            .diff_panel_focus_handle
            .clone();
        window.focus(&focus, app);
        focus
    })
}

#[gpui::test]
fn authentication_prompt_focuses_first_input_before_typing(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        cx.update(|window, _app| window.activate());
        cx.run_until_parked();

        apply_notification_snapshot(cx, &view, &auth_pull_state(kind));
        test_support::redraw(cx);
        let input = cx.update(|_window, app| first_auth_input(view.read(app), kind));
        cx.update(|window, app| {
            assert!(
                window.is_window_active(),
                "typing requires an active window"
            );
            assert!(
                input.read(app).focus_handle().is_focused(window),
                "a new {kind:?} prompt must be ready to type into"
            );
        });

        cx.simulate_input("credential-input");
        cx.update(|_window, app| {
            assert_eq!(
                input.read(app).text(),
                "credential-input",
                "typing without clicking must enter the first {kind:?} field"
            );
        });
    }
}

#[gpui::test]
fn authentication_prompt_preserves_user_focus_on_redraw_and_unrelated_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        cx.update(|window, _app| window.activate());
        cx.run_until_parked();
        let mut state = auth_pull_state(kind);
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        let input = cx.update(|_window, app| first_auth_input(view.read(app), kind));
        cx.simulate_input("partly-entered-credential");

        let other_focus = focus_diff_panel(cx, &view);
        test_support::redraw(cx);
        assert!(
            cx.update(|window, _app| other_focus.is_focused(window)),
            "redrawing an existing {kind:?} prompt must preserve the user's focus"
        );

        state.repos[0]
            .feedback
            .command_log
            .push(successful_pull_attempt());
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(
            cx.update(|window, _app| other_focus.is_focused(window)),
            "an unrelated repository update must not refocus the {kind:?} prompt"
        );
        cx.update(|_window, app| {
            assert_eq!(
                input.read(app).text(),
                "partly-entered-credential",
                "the existing {kind:?} prompt must retain its input across updates"
            );
        });
    }
}

#[gpui::test]
fn validating_authentication_prompt_preserves_focus_and_rejection_refocuses_input(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        cx.update(|window, _app| window.activate());
        cx.run_until_parked();
        let mut state = auth_pull_state(kind);
        let mut prompt = state.auth_prompt.take().unwrap();
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        let input = cx.update(|_window, app| first_auth_input(view.read(app), kind));
        let other_focus = focus_diff_panel(cx, &view);

        // The first delivered snapshot can already be checking a submitted secret.
        prompt.status = AuthPromptStatus::Validating;
        state.auth_prompt = Some(prompt);
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(
            cx.update(|window, _app| other_focus.is_focused(window)),
            "a validating {kind:?} prompt must not steal focus"
        );
        cx.update(|_window, app| {
            assert!(
                view.read(app)
                    .auth_prompt_secret_input
                    .read(app)
                    .is_read_only()
            );
            assert!(input.read(app).is_read_only());
        });

        let prompt = state.auth_prompt.as_mut().unwrap();
        prompt.generation += 1;
        prompt.status = AuthPromptStatus::Rejected;
        prompt.reason = auth_rejection(kind).into();
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        cx.update(|window, app| {
            let input = input.read(app);
            assert!(!input.is_read_only());
            assert!(input.text().is_empty());
            assert!(
                input.focus_handle().is_focused(window),
                "a rejected {kind:?} attempt with a new generation must focus the retry input"
            );
        });
        cx.simulate_input("corrected-credential");
        cx.update(|_window, app| {
            assert_eq!(input.read(app).text(), "corrected-credential");
        });
    }
}

#[gpui::test]
fn dismissed_authentication_prompt_returns_focus_it_took(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        cx.update(|window, _app| window.activate());
        cx.run_until_parked();
        let mut state = auth_pull_state(kind);
        let prompt = state.auth_prompt.take();
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        let diff_focus = focus_diff_panel(cx, &view);

        state.auth_prompt = prompt;
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        let input = cx.update(|_window, app| first_auth_input(view.read(app), kind));
        assert!(cx.update(|window, app| input.read(app).focus_handle().is_focused(window)));

        // Escape dismisses the prompt; the store then publishes it as gone.
        cx.update(|_window, app| view.update(app, |view, cx| view.cancel_auth_prompt(cx)));
        state.auth_prompt = None;
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(
            cx.update(|window, _app| diff_focus.is_focused(window)),
            "dismissing the {kind:?} prompt must return focus to where it was"
        );
    }
}

#[gpui::test]
fn pull_authentication_announces_only_the_successful_retry(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        assert!(all_toasts(cx, &view).is_empty());

        let mut state = auth_pull_state(kind);
        apply_notification_snapshot(cx, &view, &state);
        assert!(
            all_toasts(cx, &view).is_empty(),
            "the challenge is recoverable"
        );

        state.auth_prompt = None;
        apply_notification_snapshot(cx, &view, &state);
        assert!(
            all_toasts(cx, &view).is_empty(),
            "submitting must not reveal an old error"
        );

        state.repos[0]
            .feedback
            .command_log
            .push(successful_pull_attempt());
        apply_notification_snapshot(cx, &view, &state);
        apply_notification_snapshot(cx, &view, &state);
        assert_eq!(
            all_toasts(cx, &view),
            vec![(
                components::ToastKind::Success,
                "Pull: Fast-forwarded".into()
            )]
        );
        assert!(error_toasts(cx, &view).is_empty());
        cx.update(|_window, app| {
            let state = &view.read(app).state;
            assert_eq!(state.repos[0].feedback.command_log.len(), 2);
            assert_eq!(
                state.repos[0].feedback.command_log[0].stderr.as_ref(),
                auth_challenge(kind)
            );
            assert!(
                !state.repos[0].feedback.command_log[0].ok,
                "keep the real exit result in the log"
            );
        });
    }
}

#[gpui::test]
fn coalesced_authentication_and_success_snapshots_do_not_revive_the_challenge(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let mut state = auth_pull_state(kind);
        state.auth_prompt = None;
        state.repos[0]
            .feedback
            .command_log
            .push(successful_pull_attempt());

        // The UI never received the prompt or retry-in-flight snapshots.
        apply_notification_snapshot(cx, &view, &state);
        assert_eq!(
            all_toasts(cx, &view),
            vec![(
                components::ToastKind::Success,
                "Pull: Fast-forwarded".into()
            )]
        );
        assert!(error_toasts(cx, &view).is_empty());
    }
}

#[gpui::test]
fn rejected_credentials_and_prompt_cancellation_do_not_accumulate_error_toasts(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let mut state = auth_pull_state(kind);
        apply_notification_snapshot(cx, &view, &state);
        let mut prompt = state.auth_prompt.take().unwrap();
        apply_notification_snapshot(cx, &view, &state);

        let mut rejected = auth_pull_attempt(kind);
        rejected.time += Duration::from_secs(1);
        state.repos[0].feedback.command_log.push(rejected);
        prompt.generation += 1;
        prompt.status = AuthPromptStatus::Rejected;
        prompt.reason = auth_rejection(kind).into();
        state.auth_prompt = Some(prompt);
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(
            cx.debug_bounds("auth_prompt_error").is_some(),
            "rejected {kind:?} credentials must show an inline error"
        );
        assert!(
            all_toasts(cx, &view).is_empty(),
            "rejected credentials belong in the prompt"
        );

        state.auth_prompt = None;
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(cx.debug_bounds("auth_prompt_confirm").is_none());
        assert!(
            all_toasts(cx, &view).is_empty(),
            "cancelling does not expose either failed attempt"
        );
    }
}

#[gpui::test]
fn authentication_validation_rejection_and_new_generation_clear_stale_input(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let mut state = auth_pull_state(kind);
        apply_notification_snapshot(cx, &view, &state);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.auth_prompt_username_input
                    .update(cx, |input, cx| input.set_text("alice", cx));
                this.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("wrong-secret", cx));
                this.try_auth_prompt_submit(cx);
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
                assert_eq!(this.auth_prompt_submitted_generation, Some(1));
            });
        });
        state.auth_prompt.as_mut().unwrap().status = AuthPromptStatus::Validating;
        apply_notification_snapshot(cx, &view, &state);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.try_auth_prompt_submit(cx);
                assert!(
                    this.auth_prompt_input_error.is_none(),
                    "validation cannot be submitted twice"
                );
            });
        });

        let prompt = state.auth_prompt.as_mut().unwrap();
        prompt.status = AuthPromptStatus::Rejected;
        prompt.reason = auth_rejection(kind).into();
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(cx.debug_bounds("auth_prompt_error").is_some());
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                assert!(this.auth_prompt_submitted_generation.is_none());
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
                this.auth_prompt_username_input
                    .update(cx, |input, cx| input.set_text("alice", cx));
                this.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("corrected-secret", cx));
                this.try_auth_prompt_submit(cx);
                assert_eq!(this.auth_prompt_submitted_generation, Some(1));
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
            });
        });

        // The same operation and prompt text can represent another challenge.
        state.auth_prompt.as_mut().unwrap().generation = 2;
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                assert_eq!(this.auth_prompt_key, Some(2));
                assert!(this.auth_prompt_submitted_generation.is_none());
                this.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("cancelled-secret", cx));
                this.cancel_auth_prompt(cx);
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
            });
        });
        assert!(all_toasts(cx, &view).is_empty());
    }
}

#[gpui::test]
fn repeated_auth_rejection_reenables_input_when_validation_snapshot_is_skipped(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let mut state = auth_pull_state(kind);
        let prompt = state.auth_prompt.as_mut().unwrap();
        prompt.generation = 2;
        prompt.status = AuthPromptStatus::Rejected;
        prompt.reason = auth_rejection(kind).into();
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.auth_prompt_username_input
                    .update(cx, |input, cx| input.set_text("alice", cx));
                this.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("another-wrong-secret", cx));
                this.try_auth_prompt_submit(cx);
                assert_eq!(this.auth_prompt_submitted_generation, Some(2));
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
            });
        });
        test_support::redraw(cx);
        cx.update(|_window, app| {
            assert!(
                view.read(app)
                    .auth_prompt_secret_input
                    .read(app)
                    .is_read_only()
            );
        });

        // Only the attempt identity changes: the UI never receives Validating,
        // and the repeated rejection has exactly the same reason and challenge.
        state.auth_prompt.as_mut().unwrap().generation = 3;
        apply_notification_snapshot(cx, &view, &state);
        test_support::redraw(cx);
        assert!(cx.debug_bounds("auth_prompt_error").is_some());
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                assert_eq!(this.auth_prompt_key, Some(3));
                assert!(this.auth_prompt_submitted_generation.is_none());
                assert!(!this.auth_prompt_secret_input.read(cx).is_read_only());
                assert!(!this.auth_prompt_username_input.read(cx).is_read_only());
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
                this.auth_prompt_username_input
                    .update(cx, |input, cx| input.set_text("alice", cx));
                this.auth_prompt_secret_input
                    .update(cx, |input, cx| input.set_text("corrected-secret", cx));
                this.try_auth_prompt_submit(cx);
                assert_eq!(this.auth_prompt_submitted_generation, Some(3));
                assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
            });
        });
        assert!(all_toasts(cx, &view).is_empty());
    }
}

#[gpui::test]
fn genuine_retry_failures_still_produce_error_toasts(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    for kind in AUTH_PROMPT_KINDS {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        let mut state = auth_pull_state(kind);
        apply_notification_snapshot(cx, &view, &state);
        state.auth_prompt = None;

        for (command, summary) in [
            ("git pull --prune", "Pull failed: Connection timed out"),
            (
                "git push",
                "Push failed: ! [rejected] main -> main (fetch first)",
            ),
        ] {
            state.repos[0]
                .feedback
                .command_log
                .push(gitcomet_state::model::CommandLogEntry {
                    command: command.into(),
                    summary: summary.into(),
                    stderr: summary.into(),
                    handled_by_auth_prompt: false,
                    ..auth_pull_attempt(kind)
                });
            apply_notification_snapshot(cx, &view, &state);
            assert_eq!(
                error_toasts(cx, &view)[0],
                (Some(RepoId(1)), summary.into())
            );
        }
        assert_eq!(error_toasts(cx, &view).len(), 2);
    }
}

#[gpui::test]
fn pull_blocked_after_authentication_has_one_actionable_failure_and_no_stale_prompt(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = auth_pull_state(AuthPromptKind::Passphrase);
    apply_notification_snapshot(cx, &view, &state);
    test_support::redraw(cx);
    assert!(cx.debug_bounds("auth_prompt_confirm").is_some());

    state.auth_prompt = None;
    state.repos[0]
        .feedback
        .command_log
        .push(gitcomet_state::model::CommandLogEntry {
            summary: "Pull failed".into(),
            stderr: "error: Your local changes to the following files would be overwritten by merge:\n\tREADME.md\nAborting".into(),
            handled_by_auth_prompt: false,
            failure_id: Some(gitcomet_core::error::GitFailureId::WorktreeWouldBeOverwritten),
            ..auth_pull_attempt(AuthPromptKind::Passphrase)
        });
    apply_notification_snapshot(cx, &view, &state);
    test_support::redraw(cx);
    assert!(cx.debug_bounds("auth_prompt_confirm").is_none());
    cx.update(|_window, app| {
        let notices = view.read(app).toast_host.read(app).error_notices();
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].1.text.summary, "Pull blocked by local changes");
        assert_eq!(
            notices[0].1.actions,
            vec![ErrorAction::ReviewLocalChanges { repo_id: RepoId(1) }]
        );
    });
}

#[gpui::test]
fn authentication_owned_hook_attempts_are_quiet_even_without_the_prompt_snapshot(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut state = auth_pull_state(AuthPromptKind::Passphrase);
    state.auth_prompt = None;
    let operation_id = GitOperationId(1);
    state.repos[0].feedback.command_log[0].hook_operation_id = Some(operation_id);
    state.repos[0].feedback.command_log[0].command = "git push".into();
    state.repos[0].feedback.command_log[0].summary =
        format!("Push failed:\n\n{WINDOWS_SSH_PASSPHRASE_CHALLENGE}");
    state.repos[0]
        .feedback
        .hook_activity
        .push(GitHookOperation {
            id: operation_id,
            label: "Push".into(),
            context: None,
            time: std::time::SystemTime::UNIX_EPOCH,
            duration: Some(Duration::from_secs(1)),
            status: GitHookOperationStatus::Failed,
            handled_by_auth_prompt: true,
            hooks: vec![gitcomet_state::model::GitHookRun {
                id: gitcomet_core::git_operation::HookExecutionId {
                    sid: "auth-test".into(),
                    child_id: 1,
                },
                name: "pre-push".into(),
                status: GitHookRunStatus::Failed,
                exit_code: Some(1),
                duration: Some(Duration::from_secs(1)),
            }],
            output: Default::default(),
            output_bytes: 0,
            output_truncated: false,
            latest_line: String::new(),
            command_started: true,
            transfer: None,
            progress_lane: false,
            progress: None,
        });

    apply_notification_snapshot(cx, &view, &state);
    apply_notification_snapshot(cx, &view, &state);
    assert!(
        all_toasts(cx, &view).is_empty(),
        "neither completion path announces an auth challenge"
    );
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
fn try_auth_prompt_submit_passphrase_without_secret_shows_inline_error(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::test_default();
    state.auth_prompt = Some(AuthPromptState {
        generation: 1,
        challenge: None,
        status: AuthPromptStatus::AwaitingInput,
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

    assert!(error_toasts(cx, &view).is_empty());
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).auth_prompt_input_error.as_deref(),
            Some("Passphrase is required.")
        );
    });
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
        generation: 1,
        challenge: None,
        status: AuthPromptStatus::AwaitingInput,
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
            assert!(this.auth_prompt_secret_input.read(cx).text().is_empty());
            assert_eq!(this.auth_prompt_submitted_generation, Some(1));
            this.try_auth_prompt_submit(cx);
            assert!(
                this.auth_prompt_input_error.is_none(),
                "a second submit is ignored while validation starts"
            );
        });
    });

    wait_until(
        "auth prompt should be cleared after successful submit",
        || store_for_assert.snapshot().auth_prompt.is_none(),
    );
}

#[gpui::test]
fn try_auth_prompt_submit_username_password_empty_username_shows_inline_error(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let mut state = AppState::test_default();
    state.auth_prompt = Some(AuthPromptState {
        generation: 1,
        challenge: None,
        status: AuthPromptStatus::AwaitingInput,
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

    assert!(error_toasts(cx, &view).is_empty());
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).auth_prompt_input_error.as_deref(),
            Some("Username is required.")
        );
    });
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
        generation: 1,
        challenge: None,
        status: AuthPromptStatus::AwaitingInput,
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
