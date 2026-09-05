use super::*;

fn running_hook_activity(operation_id: u64) -> GitHookOperation {
    let output = (0..80)
        .map(|line| format!("checking file {line:02}\n"))
        .collect::<String>();
    GitHookOperation {
        id: GitOperationId(operation_id),
        label: "Commit".to_string(),
        context: Some("Exercise hook reporting".to_string()),
        time: std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(operation_id),
        duration: None,
        status: GitHookOperationStatus::Running,
        hooks: vec![gitcomet_state::model::GitHookRun {
            id: gitcomet_core::git_operation::HookExecutionId {
                sid: Arc::from("ui-test"),
                child_id: 1,
            },
            name: "pre-commit".to_string(),
            status: GitHookRunStatus::Running,
            exit_code: None,
            duration: None,
        }],
        output: Arc::new(std::collections::VecDeque::from([
            gitcomet_state::model::GitHookOutputChunk {
                stream: gitcomet_core::git_operation::GitOutputStream::Stderr,
                text: Arc::from(output.clone()),
            },
        ])),
        output_bytes: output.len(),
        output_truncated: false,
        latest_line: "checking file 79".to_string(),
    }
}

fn hook_activity_state_for_two_repos(
    active_repo: RepoId,
    first: RepoState,
    second: RepoState,
) -> Arc<AppState> {
    Arc::new(AppState {
        repos: vec![first, second],
        active_repo: Some(active_repo),
        ..Default::default()
    })
}

#[gpui::test]
fn hook_activity_dialog_only_hides_progress_for_its_own_repository(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let first_id = RepoId(718);
    let second_id = RepoId(719);
    let first_commit = CommitId("7187187187187187".into());
    let second_commit = CommitId("7197197197197197".into());
    let temp = std::env::temp_dir();
    let mut first = shortcut_fixture_repo(
        first_id,
        &temp.join(format!(
            "gitcomet_ui_test_{}_hook_repo_a",
            std::process::id()
        )),
        &first_commit,
    );
    let mut second = shortcut_fixture_repo(
        second_id,
        &temp.join(format!(
            "gitcomet_ui_test_{}_hook_repo_b",
            std::process::id()
        )),
        &second_commit,
    );

    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first.clone(), second.clone()),
    );
    first
        .feedback
        .hook_activity
        .push(running_hook_activity(718));
    first.feedback.hook_activity_rev = 1;
    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first.clone(), second.clone()),
    );
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("hook_activity_panel").is_some(),
        "the first repository's running hook should open Activity"
    );

    second
        .feedback
        .hook_activity
        .push(running_hook_activity(719));
    second.feedback.hook_activity_rev = 1;
    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first, second),
    );
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("hook_progress_toast").is_some(),
        "Activity for one repository must not hide another repository's running hooks"
    );

    let open = cx
        .debug_bounds("hook_progress_open")
        .expect("expected the second repository's compact progress Open action");
    cx.simulate_click(open.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("hook_activity_repository_719").is_some(),
        "Activity opened from another repository's toast must identify that repository in its header"
    );
    assert!(
        cx.debug_bounds("hook_activity_repository_718").is_none(),
        "the header must not keep identifying the active repository after a cross-repository toast is opened"
    );
}

#[gpui::test]
fn hook_activity_dialog_only_suppresses_completion_for_its_own_repository(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let first_id = RepoId(720);
    let second_id = RepoId(721);
    let first_commit = CommitId("7207207207207207".into());
    let second_commit = CommitId("7217217217217217".into());
    let temp = std::env::temp_dir();
    let mut first = shortcut_fixture_repo(
        first_id,
        &temp.join(format!(
            "gitcomet_ui_test_{}_notice_repo_a",
            std::process::id()
        )),
        &first_commit,
    );
    let mut second = shortcut_fixture_repo(
        second_id,
        &temp.join(format!(
            "gitcomet_ui_test_{}_notice_repo_b",
            std::process::id()
        )),
        &second_commit,
    );

    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first.clone(), second.clone()),
    );
    first
        .feedback
        .hook_activity
        .push(running_hook_activity(720));
    first.feedback.hook_activity_rev = 1;
    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first.clone(), second.clone()),
    );
    draw_and_drain_test_window(cx);

    second
        .feedback
        .hook_activity
        .push(running_hook_activity(721));
    second.feedback.hook_activity_rev = 1;
    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first.clone(), second.clone()),
    );
    second.feedback.hook_activity[0].status = GitHookOperationStatus::Succeeded;
    second.feedback.hook_activity[0].duration = Some(Duration::from_secs(1));
    second.feedback.hook_activity[0].hooks[0].status = GitHookRunStatus::Succeeded;
    second.feedback.hook_activity[0].hooks[0].duration = Some(Duration::from_secs(1));
    second.feedback.hook_activity_rev = 2;
    apply_state(
        cx,
        &view,
        hook_activity_state_for_two_repos(first_id, first, second),
    );

    let second_repo_notices = cx.update(|_window, app| {
        view.read(app)
            .toast_host
            .read(app)
            .hook_activity_notice_count_for_test(second_id)
    });
    assert_eq!(
        second_repo_notices, 1,
        "Activity for one repository must not suppress another repository's completion notice"
    );
}

#[gpui::test]
fn hook_activity_auto_opens_centered_and_minimizes_to_compact_progress(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(714);
    let commit_id = CommitId("7147147147147147".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_bottom_hook_activity",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo.clone()));
    assert!(
        cx.debug_bounds("bottom_hook_activity_running").is_none(),
        "the idle Activity button must not show a running indicator"
    );
    assert!(
        cx.debug_bounds("bottom_hook_activity_lightning").is_some(),
        "the Activity button must always use the lightning-bolt icon"
    );
    let activity_bounds = cx
        .debug_bounds("bottom_hook_activity")
        .expect("expected Git hook Activity button");
    let zoom_bounds = cx
        .debug_bounds("bottom_status_bar_zoom")
        .expect("expected bottom status bar zoom button");
    assert!(
        activity_bounds.right() <= zoom_bounds.left(),
        "the Activity button must sit immediately before zoom"
    );

    let mut repo_with_hook = repo.clone();
    repo_with_hook
        .feedback
        .hook_activity
        .push(running_hook_activity(714));
    let hook_template = repo_with_hook.feedback.hook_activity[0].hooks[0].clone();
    repo_with_hook.feedback.hook_activity[0]
        .hooks
        .extend((2..=18).map(|child_id| {
            let mut hook = hook_template.clone();
            hook.id.child_id = child_id;
            hook.name = if child_id % 2 == 0 {
                format!("post-index-change-{child_id:02}")
            } else {
                format!("pre-commit-{child_id:02}")
            };
            hook
        }));
    repo_with_hook.feedback.hook_activity_rev = 1;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("bottom_hook_activity_running").is_some(),
        "the cached bottom bar must repaint when a hook starts"
    );
    assert!(
        cx.debug_bounds("hook_progress_toast").is_none(),
        "the compact hook toast must stay hidden while the Activity dialog is open"
    );

    let panel = cx
        .debug_bounds("hook_activity_panel")
        .expect("expected hook Activity to open automatically");
    let window_size = cx.update(|window, _app| window.window_bounds().get_bounds().size);
    let window_width: f32 = window_size.width.into();
    let window_height: f32 = window_size.height.into();
    let ui_scale_percent = cx.update(|_window, app| view.read(app).ui_scale_percent);
    let scaled = |value: f32| -> f32 {
        crate::ui_scale::design_px_from_percent(value, ui_scale_percent).into()
    };
    let panel_width: f32 = panel.size.width.into();
    let panel_height: f32 = panel.size.height.into();
    let expected_width = scaled(900.0).min((window_width - scaled(32.0)).max(0.0));
    let expected_height = scaled(680.0).min((window_height - scaled(32.0)).max(0.0));
    assert!(
        (panel_width - expected_width).abs() < 1.0,
        "expected the dialog width to be 900px or viewport-clamped (actual={panel_width}, expected={expected_width})"
    );
    assert!(
        (panel_height - expected_height).abs() < 1.0,
        "expected the dialog height to be 680px or viewport-clamped (actual={panel_height}, expected={expected_height})"
    );
    let panel_center = panel.center();
    let panel_center_x: f32 = panel_center.x.into();
    let panel_center_y: f32 = panel_center.y.into();
    assert!(
        (panel_center_x - window_width / 2.0).abs() < 2.0
            && (panel_center_y - window_height / 2.0).abs() < 2.0,
        "expected hook Activity to be centered in the app window"
    );
    let history_rail = cx
        .debug_bounds("hook_activity_history_rail")
        .expect("expected compact run history rail");
    let detail = cx
        .debug_bounds("hook_activity_detail")
        .expect("expected hook run detail pane");
    assert!(
        detail.size.width > history_rail.size.width,
        "the output detail pane should receive more space than run history"
    );
    let run_row = cx
        .debug_bounds("hook_activity_run_714")
        .expect("expected active run in history");
    let run_row_height: f32 = run_row.size.height.into();
    assert!(
        run_row_height >= scaled(47.0) && run_row_height <= scaled(49.0),
        "run history should use compact two-line rows (height={run_row_height})"
    );
    assert!(
        cx.debug_bounds("hook_activity_run_timestamp_714").is_some(),
        "the run history row should show its start timestamp"
    );
    assert!(
        cx.debug_bounds("hook_activity_operation_context").is_some(),
        "the selected run should show its operation context"
    );
    assert!(
        cx.debug_bounds("hook_activity_output_scrollbar").is_some(),
        "the output log must render a visible scrollbar"
    );
    assert!(
        cx.debug_bounds("hook_activity_hooks_scrollbar").is_some(),
        "the hook line list must render a visible scrollbar"
    );
    let main_area = cx
        .debug_bounds("hook_activity_main_area")
        .expect("expected hook Activity main area");
    let hooks_area = cx
        .debug_bounds("hook_activity_hooks_container")
        .expect("expected scrollable hook line list");
    let output_area = cx
        .debug_bounds("hook_activity_output_container")
        .expect("expected output log area");
    let main_height: f32 = main_area.size.height.into();
    let hooks_height: f32 = hooks_area.size.height.into();
    let output_height: f32 = output_area.size.height.into();
    let section_gap: f32 = (output_area.top() - hooks_area.bottom()).into();
    assert!(
        hooks_height <= main_height * 0.34,
        "hook lines must use no more than one third of the main area (hooks={hooks_height}, main={main_height})"
    );
    assert!(
        output_height >= main_height * 0.66,
        "the output log must receive at least two thirds of the main area (output={output_height}, main={main_height})"
    );
    assert!(
        section_gap >= scaled(7.0),
        "the hook lines and terminal output should have visible breathing room (gap={section_gap})"
    );
    assert!(
        cx.debug_bounds("hook_activity_output_terminal_header")
            .is_some(),
        "the output log should render terminal-style chrome"
    );

    let hooks_are_near_bottom = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .hook_activity_hooks_are_near_bottom_for_test()
        })
    };
    assert!(
        hooks_are_near_bottom(cx),
        "the hook line list should initially follow the newest hook"
    );
    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| {
            host.scroll_hook_activity_hooks_to_top_for_test(cx)
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        !hooks_are_near_bottom(cx),
        "the hook line list must remain manually scrollable"
    );

    let minimize = cx
        .debug_bounds("hook_activity_minimize")
        .expect("expected hook Activity minimize button");
    cx.simulate_mouse_move(minimize.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Minimize to a toast and keep future hook activity minimized".into())
    );
    let close = cx
        .debug_bounds("hook_activity_close")
        .expect("expected hook Activity close button beside minimize");
    cx.simulate_mouse_move(close.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Close and automatically reopen when new hook activity starts".into())
    );

    let output_is_near_bottom = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .hook_activity_output_is_near_bottom_for_test()
        })
    };
    assert!(
        output_is_near_bottom(cx),
        "the output log should initially stick to its newest output"
    );

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| {
            host.scroll_hook_activity_output_to_top_for_test(cx)
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        !output_is_near_bottom(cx),
        "scrolling up should pause automatic output following"
    );
    let additional_output = "one more check completed\n";
    Arc::make_mut(&mut repo_with_hook.feedback.hook_activity[0].output).push_back(
        gitcomet_state::model::GitHookOutputChunk {
            stream: gitcomet_core::git_operation::GitOutputStream::Stdout,
            text: Arc::from(additional_output),
        },
    );
    repo_with_hook.feedback.hook_activity[0].output_bytes += additional_output.len();
    repo_with_hook.feedback.hook_activity[0].latest_line = "one more check completed".to_string();
    repo_with_hook.feedback.hook_activity_rev = 2;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    assert!(
        !output_is_near_bottom(cx),
        "new output must not pull a user away from an earlier log section"
    );

    cx.update(|_window, app| {
        let popover_host = view.read(app).popover_host.clone();
        popover_host.update(app, |host, cx| {
            host.scroll_hook_activity_output_to_bottom_for_test(cx)
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        output_is_near_bottom(cx),
        "reaching the bottom should resume output following"
    );

    let stop = cx
        .debug_bounds("hook_activity_stop_714")
        .expect("expected danger-colored Stop button for the active operation");
    cx.simulate_click(stop.center(), Modifiers::default());
    sync_store_snapshot(cx, &view);
    assert!(
        cx.debug_bounds("hook_activity_panel").is_some(),
        "Stop should keep Activity open instead of replacing it with a confirmation dialog"
    );
    let status_after_stop = cx.update(|_window, app| {
        view.read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| {
                repo.feedback
                    .hook_activity
                    .iter()
                    .find(|operation| operation.id.0 == 714)
            })
            .map(|operation| operation.status)
    });
    assert_eq!(
        status_after_stop,
        Some(GitHookOperationStatus::Cancelling),
        "Stop should request cancellation immediately"
    );

    let minimize = cx
        .debug_bounds("hook_activity_minimize")
        .expect("expected hook Activity minimize button");
    cx.simulate_click(minimize.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("hook_activity_panel").is_none(),
        "minimizing should close the Activity dialog"
    );
    let compact_toast = cx
        .debug_bounds("hook_progress_toast")
        .expect("minimizing an active run should reveal compact progress");
    let compact_width: f32 = compact_toast.size.width.into();
    assert!(
        (compact_width - scaled(300.0)).abs() < 1.0,
        "the minimized hook progress toast must have a fixed 300px width (actual={compact_width})"
    );

    repo_with_hook.feedback.hook_activity_rev = 3;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    assert!(
        cx.debug_bounds("hook_activity_panel").is_none(),
        "updates in the same Git operation chain must remain minimized"
    );

    repo_with_hook.feedback.hook_activity[0].status = GitHookOperationStatus::Succeeded;
    repo_with_hook.feedback.hook_activity[0].duration = Some(Duration::from_secs(1));
    repo_with_hook.feedback.hook_activity[0].hooks[0].status = GitHookRunStatus::Succeeded;
    repo_with_hook.feedback.hook_activity[0].hooks[0].duration = Some(Duration::from_secs(1));
    repo_with_hook.feedback.hook_activity_rev = 4;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    assert!(
        cx.debug_bounds("hook_progress_toast").is_none(),
        "compact hook progress should disappear when the minimized run finishes"
    );

    repo_with_hook
        .feedback
        .hook_activity
        .push(running_hook_activity(715));
    repo_with_hook.feedback.hook_activity_rev = 5;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("hook_activity_panel").is_none(),
        "explicit minimization must persist across separate Git operations"
    );
    assert!(
        cx.debug_bounds("hook_progress_toast").is_some(),
        "future operations should remain represented by the compact toast"
    );

    let open = cx
        .debug_bounds("hook_progress_open")
        .expect("expected compact progress Open action");
    cx.simulate_click(open.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("hook_progress_toast").is_none(),
        "opening Activity from compact progress must hide the toast"
    );
    let selected = cx
        .debug_bounds("hook_activity_selected_run")
        .expect("expected selected latest run");
    let run_715 = cx
        .debug_bounds("hook_activity_run_715")
        .expect("expected newest run");
    assert!(
        run_715.contains(&selected.center()),
        "opening Activity should select the latest run by default"
    );

    let close = cx
        .debug_bounds("hook_activity_close")
        .expect("expected X button while a hook is active");
    cx.simulate_click(close.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("hook_activity_panel").is_none(),
        "X should close the Activity dialog"
    );
    assert!(
        cx.debug_bounds("hook_progress_toast").is_some(),
        "the active hook should return to compact progress after X closes Activity"
    );

    repo_with_hook.feedback.hook_activity[1].status = GitHookOperationStatus::Succeeded;
    repo_with_hook.feedback.hook_activity[1].duration = Some(Duration::from_secs(1));
    repo_with_hook.feedback.hook_activity[1].hooks[0].status = GitHookRunStatus::Succeeded;
    repo_with_hook.feedback.hook_activity[1].hooks[0].duration = Some(Duration::from_secs(1));
    repo_with_hook
        .feedback
        .hook_activity
        .push(running_hook_activity(716));
    repo_with_hook.feedback.hook_activity_rev = 6;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    draw_and_drain_test_window(cx);
    let selected = cx
        .debug_bounds("hook_activity_selected_run")
        .expect("expected selected run marker after X restored auto-open");
    let run_716 = cx
        .debug_bounds("hook_activity_run_716")
        .expect("expected newly auto-opened run");
    assert!(
        run_716.contains(&selected.center()),
        "closing with X should make the next hook activity auto-open and select its run"
    );

    repo_with_hook
        .feedback
        .hook_activity
        .push(running_hook_activity(717));
    repo_with_hook.feedback.hook_activity_rev = 7;
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(repo_with_hook.clone()),
    );
    let selected = cx
        .debug_bounds("hook_activity_selected_run")
        .expect("expected selected run marker after another run starts");
    let run_716 = cx
        .debug_bounds("hook_activity_run_716")
        .expect("expected previously selected run");
    assert!(
        run_716.contains(&selected.center()),
        "a new run must not replace the user's current selection while Activity is open"
    );
}

#[gpui::test]
fn hook_activity_stays_minimized_when_another_overlay_blocks_auto_open(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(717);
    let commit_id = CommitId("7177177177177177".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_hook_activity_blocked",
        std::process::id()
    ));
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    apply_state(cx, &view, app_state_with_active_repo(repo.clone()));

    let zoom = cx
        .debug_bounds("bottom_status_bar_zoom")
        .expect("expected bottom status bar zoom button");
    cx.simulate_click(zoom.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        popover_is_open(cx, &view),
        "expected a pre-existing overlay before the hook starts"
    );

    repo.feedback.hook_activity.push(running_hook_activity(717));
    repo.feedback.hook_activity_rev = 1;
    apply_state(cx, &view, app_state_with_active_repo(repo.clone()));
    assert!(
        cx.debug_bounds("hook_activity_panel").is_none(),
        "hook Activity must not displace an overlay that is already open"
    );
    assert!(
        cx.debug_bounds("hook_progress_toast").is_some(),
        "a blocked auto-open should fall back to compact progress"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(
        !popover_is_open(cx, &view),
        "expected the blocking overlay to close"
    );

    repo.feedback.hook_activity_rev = 2;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    assert!(
        cx.debug_bounds("hook_activity_panel").is_none(),
        "closing the blocking overlay must not auto-open the already-minimized chain later"
    );
    assert!(
        cx.debug_bounds("hook_progress_toast").is_some(),
        "compact progress should remain available after the blocking overlay closes"
    );
}
