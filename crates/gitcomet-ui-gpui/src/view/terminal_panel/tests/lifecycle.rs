use super::super::*;
use super::support::*;

#[gpui::test]
fn shell_exit_closes_the_matching_tab_after_indices_shift(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);

    let events_tx = cx.update(|window, app| {
        view.update(app, |this, cx| {
            let (events_tx, events_rx) = smol::channel::unbounded();
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(10, None), (20, Some(events_rx)), (30, None)], 2, cx),
            );
            this.spawn_terminal_event_task(repo_id, 20, cx);

            // Shift the target from index 1 to index 0 before its delayed
            // exit arrives. An index captured at spawn would close seq 30.
            this.close_terminal_tab(repo_id, 0, window, cx);
            events_tx
        })
    });

    events_tx
        .try_send(TerminalBackendEvent::Exit)
        .expect("send terminal exit event");
    cx.run_until_parked();

    cx.update(|_window, app| {
        let session = view
            .read(app)
            .terminal_sessions
            .get(&repo_id)
            .expect("surviving terminal session");
        assert_eq!(
            session
                .instances
                .iter()
                .map(|instance| instance.session_seq)
                .collect::<Vec<_>>(),
            vec![30]
        );
        assert_eq!(session.active_index, 0);
    });
}

#[gpui::test]
fn shell_exit_of_the_last_tab_closes_the_terminal_panel(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);

    let events_tx = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let (events_tx, events_rx) = smol::channel::unbounded();
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(40, Some(events_rx))], 0, cx),
            );
            this.spawn_terminal_event_task(repo_id, 40, cx);
            events_tx
        })
    });

    events_tx
        .try_send(TerminalBackendEvent::Exit)
        .expect("send terminal exit event");
    cx.run_until_parked();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            assert!(!this.terminal_sessions.contains_key(&repo_id));
            let theme = this.theme;
            assert!(this.render_terminal_panel(theme, window, cx).is_none());
            assert!(!this.terminal_cursor_blink_active);
        });
    });
}

#[gpui::test]
fn stale_shell_exit_after_manual_close_does_not_close_another_tab(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);

    let events_tx = cx.update(|window, app| {
        view.update(app, |this, cx| {
            let (events_tx, events_rx) = smol::channel::unbounded();
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(50, Some(events_rx)), (60, None)], 1, cx),
            );
            this.spawn_terminal_event_task(repo_id, 50, cx);
            this.close_terminal_tab(repo_id, 0, window, cx);
            events_tx
        })
    });

    events_tx
        .try_send(TerminalBackendEvent::Exit)
        .expect("send stale terminal exit event");
    cx.run_until_parked();

    cx.update(|_window, app| {
        let session = view
            .read(app)
            .terminal_sessions
            .get(&repo_id)
            .expect("surviving terminal session");
        assert_eq!(session.instances.len(), 1);
        assert_eq!(session.instances[0].session_seq, 60);
        assert_eq!(session.active_index, 0);
    });
}

#[gpui::test]
fn shutdown_confirmation_for_an_exited_tab_does_not_close_its_sibling(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);

    let (events_tx, prompt) = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let (events_tx, events_rx) = smol::channel::unbounded();
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(70, Some(events_rx)), (80, None)], 0, cx),
            );
            this.spawn_terminal_event_task(repo_id, 70, cx);
            let prompt = TerminalShutdownPrompt {
                action: TerminalShutdownAction::CloseTerminalTab {
                    repo_id,
                    session_seq: 70,
                },
                summary: TerminalShutdownSummary {
                    terminal_count: 1,
                    running_command_count: 1,
                    repo_names: vec!["terminal-exit-test".to_string()],
                },
            };
            (events_tx, prompt)
        })
    });

    // The target exits while its terminate-and-close prompt is still open.
    // Its sibling shifts into index 0, which an index-keyed prompt would
    // incorrectly terminate and remove when confirmed.
    events_tx
        .try_send(TerminalBackendEvent::Exit)
        .expect("send terminal exit event");
    cx.run_until_parked();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.confirm_terminal_shutdown(prompt, window, cx);
        });
    });

    cx.update(|_window, app| {
        let session = view
            .read(app)
            .terminal_sessions
            .get(&repo_id)
            .expect("sibling terminal session");
        assert_eq!(session.instances.len(), 1);
        assert_eq!(session.instances[0].session_seq, 80);
        assert_eq!(session.active_index, 0);
    });
}
