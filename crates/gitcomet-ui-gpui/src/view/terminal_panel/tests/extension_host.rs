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

/// A tab an extension opens joins the repository's terminal, starting it
/// when it has none; the terminal keeps the repository's own workdir for
/// its "+" and for the external terminal.
#[gpui::test]
fn an_extensions_tab_joins_the_repositorys_terminal_in_front(cx: &mut gpui::TestAppContext) {
    let (view, _host, cx) = window_with_two_repositories(cx);
    let repo_id = RepoId(1);
    for seq in [30, 31] {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                let instance = test_terminal_instance(seq, None, cx);
                this.add_terminal_tab(repo_id, instance, window, cx);
            })
        });
    }
    test_support::drain_store_worker(&view, cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| test_support::sync_store_snapshot(this, cx));
    });
    test_support::redraw(cx);
    cx.update(|window, app| {
        let this = view.read(app);
        assert_eq!(this.state.active_repo, Some(repo_id));
        assert_eq!(
            this.active_bottom_panel.get(&repo_id),
            Some(&crate::view::BottomPanelTab::Terminal)
        );
        let session = &this.terminal_sessions[&repo_id];
        assert_eq!(session.workdir, PathBuf::from("/tmp/terminal-host-1"));
        assert_eq!(
            session
                .instances
                .iter()
                .map(|instance| instance.session_seq)
                .collect::<Vec<_>>(),
            [30, 31]
        );
        assert_eq!(session.active_index, 1);
        assert!(session.instances[1].focus_handle.is_focused(window));
    });
}

/// What an extension asked to type goes in once, when the program first
/// prints: not before, and not on a repaint the terminal asks for itself.
#[gpui::test]
fn an_extensions_command_is_typed_once_the_shell_prints(cx: &mut gpui::TestAppContext) {
    use alacritty_terminal::event_loop::Msg as PtyMsg;
    let (view, _host, cx) = window_with_two_repositories(cx);
    let (events, events_rx) = smol::channel::unbounded();
    let (sender, typed) = gitcomet_ui_terminal::PtySender::recording();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            let mut instance = test_terminal_instance(40, Some(events_rx), cx);
            instance.pty_sender = Some(sender);
            instance.pending_input = Some(typed_line("claude --resume abc"));
            this.add_terminal_tab(RepoId(1), instance, window, cx);
        })
    });
    cx.run_until_parked();
    let inputs = |typed: &smol::channel::Receiver<PtyMsg>| {
        let mut inputs = Vec::new();
        while let Ok(message) = typed.try_recv() {
            if let PtyMsg::Input(bytes) = message {
                inputs.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        inputs
    };
    let send = |event: TerminalBackendEvent, cx: &mut gpui::VisualTestContext| {
        events.try_send(event).expect("the tab listens");
        cx.run_until_parked();
    };
    assert!(inputs(&typed).is_empty(), "nothing before the shell prints");
    send(TerminalBackendEvent::Wakeup, cx);
    send(TerminalBackendEvent::CursorBlinkingChange, cx);
    assert!(
        inputs(&typed).is_empty(),
        "a repaint is not the shell printing"
    );

    send(TerminalBackendEvent::Output, cx);
    send(TerminalBackendEvent::Output, cx);
    assert_eq!(
        inputs(&typed),
        ["claude --resume abc\r"],
        "the line and Enter, once"
    );
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).terminal_sessions[&RepoId(1)].instances[0].pending_input,
            None
        );
    });
}

/// Only the embedded terminal can be handed a command, and only one line of
/// it; asked from inside the window's own update, the host answers without
/// reading the view. A closed repository gets nothing.
#[gpui::test]
fn a_command_needs_the_embedded_terminal_and_one_line(cx: &mut gpui::TestAppContext) {
    let (view, host, cx) = window_with_two_repositories(cx);
    let repository = cx.update(|_window, app| {
        let this = view.read(app);
        let repo = &this.state.repos[0];
        gitcomet_extension_api::RepositoryHandle::new(
            host.id(),
            repo.id,
            repo.lifetime(),
            repo.spec.workdir.clone(),
        )
    });
    let asked = |command: &'static str, cx: &mut gpui::VisualTestContext| {
        let (host, repository, view) = (host.clone(), repository.clone(), view.clone());
        cx.update(move |_window, app| {
            view.update(app, |_, cx| {
                host.open_terminal_with_command(
                    &repository,
                    std::path::Path::new("src"),
                    command,
                    cx,
                )
            })
        })
    };
    assert_eq!(
        asked("claude --resume abc\nrm -rf ~", cx),
        Err(HostError::InvalidRequest(
            "A terminal command is one line without control characters".into()
        ))
    );
    assert!(matches!(
        asked("claude\u{1b}[A", cx),
        Err(HostError::InvalidRequest(_))
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_preferences.terminal_button_target = TerminalButtonTarget::External;
            this.sync_terminal_button_target(cx);
        })
    });
    assert_eq!(
        asked("claude --resume abc", cx),
        Err(HostError::Unsupported)
    );

    let closed = gitcomet_extension_api::RepositoryHandle::new(
        host.id(),
        RepoId(9),
        1,
        PathBuf::from("/tmp/terminal-host-9"),
    );
    assert_eq!(
        cx.update(|_window, app| host.open_terminal(&closed, std::path::Path::new("/tmp"), app)),
        Err(HostError::RepositoryClosed)
    );
    cx.run_until_parked();
    cx.update(|_window, app| assert!(view.read(app).terminal_sessions.is_empty()));
}

/// A folder that is not there opens no terminal, which would start in
/// GitComet's own folder instead, and the window says why.
#[gpui::test]
fn a_terminal_is_not_opened_in_a_missing_folder(cx: &mut gpui::TestAppContext) {
    let (view, host, cx) = window_with_two_repositories(cx);
    let missing = PathBuf::from("/tmp/terminal-host-1/no-such-folder-here");
    cx.update(|_window, app| {
        let repository = {
            let this = view.read(app);
            let repo = &this.state.repos[0];
            gitcomet_extension_api::RepositoryHandle::new(
                host.id(),
                repo.id,
                repo.lifetime(),
                repo.spec.workdir.clone(),
            )
        };
        view.update(app, |this, cx| {
            this.open_terminal_at(&repository, missing.clone(), Some("claude".into()), cx)
        })
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let this = view.read(app);
        assert!(this.terminal_sessions.is_empty());
        let toasts = this.toast_host.read(app).toasts_for_tests(app);
        assert!(
            toasts
                .iter()
                .any(|(_, text)| text.contains("no-such-folder-here is not a folder")),
            "{toasts:?}"
        );
    });
}

/// A relative folder is the repository's: the host looks for it below the
/// workdir, and says which folder it was when it is not there.
#[gpui::test]
fn a_relative_folder_is_joined_to_the_workdir(cx: &mut gpui::TestAppContext) {
    let (view, host, cx) = window_with_two_repositories(cx);
    let repository = cx.update(|_window, app| {
        let this = view.read(app);
        let repo = &this.state.repos[0];
        gitcomet_extension_api::RepositoryHandle::new(
            host.id(),
            repo.id,
            repo.lifetime(),
            repo.spec.workdir.clone(),
        )
    });
    cx.update(|_window, app| {
        host.open_terminal(
            &repository,
            std::path::Path::new("no-such-folder-here"),
            app,
        )
    })
    .expect("the request is taken");
    cx.run_until_parked();
    let expected_error = format!(
        "{} is not a folder",
        repository.workdir().join("no-such-folder-here").display()
    );
    cx.update(|_window, app| {
        let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
        assert!(
            toasts
                .iter()
                .any(|(_, text)| text.contains(&expected_error)),
            "{toasts:?}"
        );
    });
}
