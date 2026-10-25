use super::super::*;
use super::support::*;
use crate::view::{extension_host, test_support};
use gitcomet_extension_api::{HostError, Registry, TerminalId, TerminalTab, WindowHost};

/// Pids past every platform's pid limit: the view's teardown signals each
/// tab's process group, which must never reach a real process.
const SHELL_PIDS: [u32; 2] = [999_999_001, 999_999_002];

fn window_with_two_repositories(
    cx: &mut gpui::TestAppContext,
) -> (
    Entity<GitCometView>,
    WindowHost,
    &mut gpui::VisualTestContext,
) {
    cx.update(|app| {
        let registry = Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .expect("valid registration");
        extension_host::install(registry, app);
    });
    let (store, events) = AppStore::new_test(Arc::new(TerminalTestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    let mut state = AppState {
        active_repo: Some(RepoId(2)),
        ..AppState::test_default()
    };
    for id in [1, 2] {
        state.repos.push(RepoState::new_opening(
            RepoId(id),
            gitcomet_core::domain::RepoSpec {
                workdir: PathBuf::from(format!("/tmp/terminal-host-{id}")),
            },
        ));
    }
    store.replace_snapshot_for_test(Arc::new(state));
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
    let host = cx.update(|_window, app| view.read(app).extension_window.as_ref().unwrap().host());
    (view, host, cx)
}

#[gpui::test]
fn extensions_list_terminal_tabs_and_bring_one_to_the_front(cx: &mut gpui::TestAppContext) {
    let (view, host, cx) = window_with_two_repositories(cx);
    let repo_id = RepoId(1);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut session = test_terminal_session(vec![(10, None), (11, None)], 0, cx);
            for (instance, pid) in session.instances.iter_mut().zip(SHELL_PIDS) {
                instance.child_pid = Some(pid);
            }
            this.terminal_sessions.insert(repo_id, session);
            this.terminal_sessions_changed(cx);
            this.open_reflog_panel(repo_id, cx);
        })
    });

    let tabs = cx.update(|_window, app| host.terminals(app));
    let workdirs: Vec<_> = tabs
        .iter()
        .map(|tab| tab.repository.workdir().to_path_buf())
        .collect();
    assert_eq!(
        workdirs,
        vec![PathBuf::from("/tmp/terminal-host-1"); 2],
        "both tabs belong to the first repository"
    );
    assert_eq!(
        tabs.iter()
            .map(|tab| (tab.id, tab.shell_pid))
            .collect::<Vec<_>>(),
        vec![
            (TerminalId::new(10), Some(SHELL_PIDS[0])),
            (TerminalId::new(11), Some(SHELL_PIDS[1])),
        ]
    );

    cx.update(|_window, app| host.focus_terminal(&tabs[1], app))
        .expect("an open tab can be focused");
    test_support::drain_store_worker(&view, cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);

    assert!(cx.debug_bounds("terminal_tab-1").is_some());
    cx.update(|window, app| {
        let this = view.read(app);
        assert_eq!(this.state.active_repo, Some(repo_id));
        assert_eq!(
            this.active_bottom_panel.get(&repo_id),
            Some(&crate::view::BottomPanelTab::Terminal)
        );
        let session = &this.terminal_sessions[&repo_id];
        assert_eq!(session.active_index, 1);
        assert!(session.instances[1].focus_handle.is_focused(window));
    });
}

/// Tabs follow the window's repository order as it is now, not as it was
/// when they opened.
#[gpui::test]
fn terminal_tabs_follow_reordered_repository_tabs(cx: &mut gpui::TestAppContext) {
    let (view, host, cx) = window_with_two_repositories(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions
                .insert(RepoId(1), test_terminal_session(vec![(10, None)], 0, cx));
            this.terminal_sessions
                .insert(RepoId(2), test_terminal_session(vec![(20, None)], 0, cx));
            this.terminal_sessions_changed(cx);
        })
    });
    let ids = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| host.terminals(app))
            .iter()
            .map(|tab| tab.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(ids(cx), [TerminalId::new(10), TerminalId::new(20)]);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut reordered = (*this.state).clone();
            reordered.repos.reverse();
            test_support::push_test_state(this, Arc::new(reordered), cx);
        })
    });
    cx.run_until_parked();

    assert_eq!(ids(cx), [TerminalId::new(20), TerminalId::new(10)]);
}

#[gpui::test]
fn a_closed_terminal_tab_leaves_the_list_and_cannot_be_focused(cx: &mut gpui::TestAppContext) {
    let (view, host, cx) = window_with_two_repositories(cx);
    let repo_id = RepoId(1);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(10, None), (11, None)], 0, cx),
            );
            this.terminal_sessions_changed(cx);
        })
    });
    let closed: TerminalTab = cx.update(|_window, app| host.terminals(app))[1].clone();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.close_terminal_tab(repo_id, 1, window, cx)
        })
    });

    let open = cx.update(|_window, app| host.terminals(app));
    assert_eq!(
        open.iter().map(|tab| tab.id).collect::<Vec<_>>(),
        vec![TerminalId::new(10)]
    );
    assert_eq!(
        cx.update(|_window, app| host.focus_terminal(&closed, app)),
        Err(HostError::TerminalClosed)
    );
    cx.run_until_parked();
    cx.update(|_window, app| {
        let this = view.read(app);
        assert_eq!(this.state.active_repo, Some(RepoId(2)), "nothing switched");
        assert_eq!(this.terminal_sessions[&repo_id].active_index, 0);
    });
}
