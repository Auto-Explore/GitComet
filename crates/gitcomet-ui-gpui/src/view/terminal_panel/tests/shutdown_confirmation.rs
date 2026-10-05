use super::super::*;
use super::support::*;

fn show_confirmation(
    view: &Entity<GitCometView>,
    action: TerminalShutdownAction,
    cx: &mut gpui::VisualTestContext,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            // The terminal backend is simulated; exercise the real dialog and
            // click handler after the running-command check has queued it.
            this.queue_terminal_shutdown_prompt(
                action,
                TerminalShutdownSummary {
                    terminal_count: 1,
                    running_command_count: 1,
                    repo_names: vec!["terminal-exit-test".to_string()],
                },
                cx,
            );
        });
    });
    cx.run_until_parked();
    crate::test_support::refresh_and_draw(cx);
}

fn click_dialog_button(selector: &'static str, cx: &mut gpui::VisualTestContext) {
    let button = cx.debug_bounds(selector).expect("terminal shutdown button");
    cx.simulate_click(button.center(), gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn terminal_shutdown_confirm_click_closes_only_the_selected_tab(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(10, None), (20, None)], 0, cx),
            );
        });
    });
    show_confirmation(
        &view,
        TerminalShutdownAction::CloseTerminalTab {
            repo_id,
            session_seq: 10,
        },
        cx,
    );

    click_dialog_button("terminal_shutdown_confirm", cx);

    cx.update(|_window, app| {
        let root = view.read(app);
        let session = root.terminal_sessions.get(&repo_id).unwrap();
        assert_eq!(session.instances.len(), 1);
        assert_eq!(session.instances[0].session_seq, 20);
        assert_eq!(app.windows().len(), 1);
        assert!(!root.popover_host.read(app).is_open());
    });
}

#[gpui::test]
fn terminal_shutdown_confirm_click_closes_the_panel(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
        });
    });
    show_confirmation(
        &view,
        TerminalShutdownAction::CloseTerminalForRepo { repo_id },
        cx,
    );

    click_dialog_button("terminal_shutdown_confirm", cx);

    cx.update(|_window, app| {
        let root = view.read(app);
        assert!(!root.terminal_sessions.contains_key(&repo_id));
        assert_eq!(app.windows().len(), 1);
        assert!(!root.popover_host.read(app).is_open());
    });
}

#[gpui::test]
fn terminal_shutdown_confirm_cancel_keeps_the_panel(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
        });
    });
    show_confirmation(
        &view,
        TerminalShutdownAction::CloseTerminalForRepo { repo_id },
        cx,
    );

    click_dialog_button("terminal_shutdown_cancel", cx);

    cx.update(|_window, app| {
        let root = view.read(app);
        assert_eq!(root.terminal_sessions[&repo_id].instances.len(), 1);
        assert!(root.pending_terminal_shutdown_prompt.is_none());
        assert!(!root.popover_host.read(app).is_open());
    });
}

#[gpui::test]
fn terminal_shutdown_confirm_click_closes_the_window(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
        });
    });
    show_confirmation(&view, TerminalShutdownAction::CloseWindow, cx);

    click_dialog_button("terminal_shutdown_confirm", cx);

    cx.cx.update(|app| assert!(app.windows().is_empty()));
}

#[gpui::test]
fn terminal_shutdown_confirm_click_preserves_later_close_guards(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            Arc::make_mut(&mut this.state).repos[0].push_in_flight = 1;
            this.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
        });
    });
    show_confirmation(&view, TerminalShutdownAction::CloseWindow, cx);

    click_dialog_button("terminal_shutdown_confirm", cx);
    crate::test_support::refresh_and_draw(cx);

    cx.update(|_window, app| {
        let root = view.read(app);
        assert_eq!(app.windows().len(), 1);
        assert_eq!(root.terminal_sessions[&repo_id].instances.len(), 1);
        assert_eq!(
            root.popover_host.read(app).close_guard_action(),
            Some(&TerminalShutdownAction::CloseWindow),
        );
    });
    assert!(cx.debug_bounds("close_guard_reasons").is_some());
}
