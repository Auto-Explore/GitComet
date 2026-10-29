use super::super::*;
use super::support::*;

#[gpui::test]
fn terminal_toolbar_fill_tracks_panel_lifetime_and_target(cx: &mut gpui::TestAppContext) {
    assert_terminal_toolbar_fill_tracks_panel_lifetime_and_target(cx);
}

struct StatusBarTerminal;

impl gitcomet_extension_api::Extension for StatusBarTerminal {
    fn id(&self) -> gitcomet_extension_api::ExtensionId {
        gitcomet_extension_api::ExtensionId::new("com.example.terminal-layout").unwrap()
    }

    fn register(&self, registrar: &mut gitcomet_extension_api::Registrar) {
        registrar.place_bar_item(
            gitcomet_extension_api::BuiltinBarItem::Terminal,
            gitcomet_extension_api::BarItemLocation::StatusBarEnd,
        );
    }
}

#[gpui::test]
fn relocated_terminal_toolbar_fill_tracks_panel_lifetime_and_target(cx: &mut gpui::TestAppContext) {
    cx.update(|app| {
        crate::view::extension_host::install(
            gitcomet_extension_api::Registry::build(vec![Box::new(StatusBarTerminal)]).unwrap(),
            app,
        );
    });
    assert_terminal_toolbar_fill_tracks_panel_lifetime_and_target(cx);
}

fn assert_terminal_toolbar_fill_tracks_panel_lifetime_and_target(cx: &mut gpui::TestAppContext) {
    use crate::test_support::painted_control_quads as paint;
    use crate::view::test_support::redraw;

    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            let state = Arc::clone(&view.state);
            crate::view::test_support::push_test_state(view, state, cx);
            view.terminal_preferences.terminal_button_target = TerminalButtonTarget::Embedded;
            view.sync_terminal_button_target(cx);
        })
    });
    cx.run_until_parked();
    redraw(cx);
    let closed = paint(cx, "terminal");
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
            view.terminal_sessions_changed(cx);
            cx.notify();
        })
    });
    cx.run_until_parked();
    redraw(cx);
    assert_ne!(
        paint(cx, "terminal"),
        closed,
        "an open embedded terminal must highlight its toggle"
    );
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.terminal_preferences.terminal_button_target = TerminalButtonTarget::External;
            view.sync_terminal_button_target(cx);
        })
    });
    cx.run_until_parked();
    redraw(cx);
    assert_eq!(
        paint(cx, "terminal"),
        closed,
        "the external terminal launcher is momentary"
    );
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.terminal_preferences.terminal_button_target = TerminalButtonTarget::Embedded;
            view.sync_terminal_button_target(cx);
            view.close_terminal_tab(repo_id, 0, window, cx);
        })
    });
    cx.run_until_parked();
    redraw(cx);
    assert_eq!(
        paint(cx, "terminal"),
        closed,
        "closing the last tab must clear the toggle"
    );
}

#[gpui::test]
fn the_terminal_toggle_defaults_to_the_action_bar(cx: &mut gpui::TestAppContext) {
    let (view, _repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            let state = Arc::clone(&view.state);
            crate::view::test_support::push_test_state(view, state, cx);
        })
    });
    cx.run_until_parked();
    crate::test_support::refresh_and_draw(cx);

    let terminal = cx.debug_bounds("terminal").expect("the terminal toggle");
    let branch = cx.debug_bounds("create_branch").expect("the branch action");
    let stash = cx.debug_bounds("stash").expect("the stash action");
    let actions = cx
        .debug_bounds("right_action_group")
        .expect("the action bar's right group");
    assert!(
        actions.contains(&terminal.center()),
        "Terminal belongs in the action bar: {terminal:?} vs {actions:?}"
    );
    assert!(
        terminal.right() <= branch.left() && branch.right() <= stash.left(),
        "Terminal, Branch and Stash retain their original order"
    );
    assert!(
        (terminal.center().y - branch.center().y).abs() < gpui::px(1.0),
        "on the action bar's row: {terminal:?} vs {branch:?}"
    );
}

#[gpui::test]
fn shell_exit_closes_the_matching_tab_after_indices_shift(cx: &mut gpui::TestAppContext) {
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
        });
    });
}

#[gpui::test]
fn stale_shell_exit_after_manual_close_does_not_close_another_tab(cx: &mut gpui::TestAppContext) {
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

/// Both bottom-panel strips and the terminal's own tabs: click targets that
/// used to sit at a fixed pixel size whatever the density.
#[gpui::test]
fn comfortable_bottom_panel_tabs_grow_with_the_density(cx: &mut gpui::TestAppContext) {
    use crate::appearance::{Appearance, UiDensity};
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
            this.open_reflog_panel(repo_id, cx);
            this.active_bottom_panel
                .insert(repo_id, crate::view::BottomPanelTab::Terminal);
        })
    });

    let selectors = [
        "terminal_tab-0",
        "terminal_tab_close-0",
        "bottom_panel_tab_terminal",
        "bottom_panel_tab_terminal_close",
        "bottom_panel_tab_reflog",
    ];
    let mut per_density: Vec<Vec<gpui::Pixels>> = Vec::new();

    for density in UiDensity::ALL {
        cx.update(|_window, app| {
            app.set_global(Appearance {
                density,
                ..Appearance::default()
            });
            view.update(app, |this, cx| this.notify_font_preferences_changed(cx));
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let mut height = |selector: &'static str| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("missing {selector} at {density:?} density"))
                .size
                .height
        };
        per_density.push(selectors.into_iter().map(&mut height).collect());

        // Both strips are built from `panel_tab`, so they must agree.
        assert_eq!(
            height("terminal_tab-0"),
            height("bottom_panel_tab_terminal")
        );
        assert_eq!(
            height("terminal_tab_close-0"),
            height("bottom_panel_tab_terminal_close")
        );
    }

    for (step, pair) in per_density.windows(2).enumerate() {
        for (ix, selector) in selectors.into_iter().enumerate() {
            assert!(
                pair[1][ix] > pair[0][ix],
                "{selector} must grow from {:?} to {:?}, stayed {:?}",
                UiDensity::ALL[step],
                UiDensity::ALL[step + 1],
                pair[1][ix]
            );
        }
    }
}

#[gpui::test]
fn reflog_rows_are_visible_with_the_terminal_open(cx: &mut gpui::TestAppContext) {
    use crate::test_support::{painted_control_quads, refresh_and_draw};
    use crate::view::test_support::push_test_state;
    use gitcomet_core::domain::ReflogEntry;

    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = (*this.state).clone();
            state.repos[0].reflog = Loadable::Ready(Arc::new(
                (0..20)
                    .map(|index| ReflogEntry {
                        index,
                        new_id: CommitId(Arc::from(format!("{index:040x}"))),
                        selector: Arc::from(format!("HEAD@{{{index}}}")),
                        message: Arc::from(format!("commit: entry {index}")),
                        time: None,
                        author: Arc::from("Jane Doe"),
                    })
                    .collect(),
            ));
            state.repos[0].reflog_rev = 1;
            push_test_state(this, Arc::new(state), cx);
            this.open_reflog_panel(repo_id, cx);
        });
    });
    refresh_and_draw(cx);
    let row = cx.debug_bounds("reflog_row-0").unwrap();
    assert!(row.size.height > px(0.0));
    assert!(
        cx.debug_bounds("reflog_message-0").unwrap().size.width >= px(70.0),
        "the message must remain readable in the app's narrower center pane"
    );
    assert!(
        cx.debug_bounds("reflog_header_message")
            .unwrap()
            .size
            .height
            <= row.size.height
    );
    assert!(!painted_control_quads(cx, "reflog_row-0").is_empty());
    let original_width = cx.debug_bounds("reflog_header_sha").unwrap().size.width;
    let start = cx.debug_bounds("reflog_resize_sha").unwrap().center();
    cx.simulate_mouse_down(start, MouseButton::Left, gpui::Modifiers::default());
    for delta in [-8.0, -24.0] {
        cx.simulate_mouse_move(
            point(start.x + px(delta), start.y),
            Some(MouseButton::Left),
            gpui::Modifiers::default(),
        );
        refresh_and_draw(cx);
    }
    cx.simulate_mouse_up(
        point(start.x - px(24.0), start.y),
        MouseButton::Left,
        gpui::Modifiers::default(),
    );
    refresh_and_draw(cx);
    let resized_width = cx.debug_bounds("reflog_header_sha").unwrap().size.width;
    assert!(resized_width < original_width);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
            cx.notify();
        });
    });
    refresh_and_draw(cx);
    assert!(cx.debug_bounds("bottom_panel_tab_terminal").is_some());
    assert!(cx.debug_bounds("bottom_panel_tab_reflog").is_some());
    assert!(
        !painted_control_quads(cx, "reflog_row-0").is_empty(),
        "the active reflog must paint its rows while the terminal is also open"
    );

    for selector in ["bottom_panel_tab_terminal", "bottom_panel_tab_reflog"] {
        let tab = cx.debug_bounds(selector).unwrap();
        cx.simulate_click(tab.center(), gpui::Modifiers::default());
        refresh_and_draw(cx);
    }
    assert!(!painted_control_quads(cx, "reflog_row-0").is_empty());
    assert_eq!(
        cx.debug_bounds("reflog_header_sha").unwrap().size.width,
        resized_width
    );
}

#[test]
#[ignore = "renders reflog previews using the native text backend and a GPU"]
fn reflog_panel_visual_capture() {
    let text_system = gpui_platform::current_platform(true).text_system();
    let mut cx = gpui::HeadlessAppContext::with_platform(
        text_system,
        Arc::new(crate::assets::GitCometAssets::default()),
        || Some(Box::new(gpui_wgpu::WgpuHeadlessRenderer::new().unwrap())),
    );
    let repo_id = RepoId(1);
    let (store, events) = AppStore::new_test(Arc::new(TerminalTestBackend));
    let handle = cx
        .open_window(gpui::size(px(1200.0), px(720.0)), |window, app| {
            app.new(|cx| GitCometView::new(store, events, None, window, cx))
        })
        .unwrap();
    handle
        .update(&mut cx, |view, _, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: PathBuf::from("/tmp/reflog-preview"),
                },
            );
            repo.reflog = Loadable::Ready(Arc::new(
                (0..20)
                    .map(|index| gitcomet_core::domain::ReflogEntry {
                        index,
                        new_id: CommitId(Arc::from("abcdef1234567890abcdef1234567890abcdef12")),
                        selector: Arc::from(format!("HEAD@{{{index}}}")),
                        message: Arc::from(
                            "checkout: moving from main to feature/reflog-selection-and-resizing",
                        ),
                        author: Arc::from("Jane Doe"),
                        time: Some(
                            std::time::UNIX_EPOCH
                                + Duration::from_secs(1_700_000_000 + index as u64),
                        ),
                    })
                    .collect(),
            ));
            repo.reflog_rev = 1;
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
            });
            crate::view::test_support::push_test_state(view, state, cx);
            view.terminal_sessions
                .insert(repo_id, test_terminal_session(vec![(10, None)], 0, cx));
            view.terminal_panel_height = px(320.0);
            view.open_reflog_panel(repo_id, cx);
        })
        .unwrap();
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/reflog-preview");
    std::fs::create_dir_all(&directory).unwrap();
    for (name, theme, font_size) in [
        ("dark", AppTheme::gitcomet_dark(), 14),
        ("light", AppTheme::gitcomet_light(), 14),
        ("large-text", AppTheme::gitcomet_dark(), 20),
    ] {
        cx.update(|app| {
            app.set_global(crate::appearance::Appearance {
                ui_font_size_px: font_size,
                ..Default::default()
            })
        });
        handle
            .update(&mut cx, |view, _, cx| {
                view.set_theme(theme, cx);
                view.notify_font_preferences_changed(cx);
            })
            .unwrap();
        cx.run_until_parked();
        cx.update_window(handle.into(), |_, window, app| {
            window.refresh();
            let _ = window.draw(app);
        })
        .unwrap();
        cx.capture_screenshot(handle.into())
            .unwrap()
            .save(directory.join(format!("{name}.png")))
            .unwrap();
    }
}

/// Selecting a terminal tab behind the reflog shows the terminal, so focus
/// never lands in a terminal that is not drawn.
#[gpui::test]
fn selecting_a_terminal_tab_brings_the_terminal_in_front_of_the_reflog(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::test_support::redraw;
    let (view, repo_id, cx) = test_root_view_with_active_repo(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.terminal_sessions.insert(
                repo_id,
                test_terminal_session(vec![(10, None), (11, None)], 0, cx),
            );
            this.open_reflog_panel(repo_id, cx);
        })
    });
    redraw(cx);
    assert!(cx.debug_bounds("bottom_panel_tab_reflog").is_some());
    assert!(cx.debug_bounds("terminal_tab-1").is_none());

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.select_terminal_tab(repo_id, 1, window, cx)
        })
    });
    redraw(cx);

    assert!(cx.debug_bounds("terminal_tab-1").is_some());
    cx.update(|window, app| {
        let this = view.read(app);
        assert_eq!(
            this.active_bottom_panel.get(&repo_id),
            Some(&crate::view::BottomPanelTab::Terminal)
        );
        let session = &this.terminal_sessions[&repo_id];
        assert_eq!(session.active_index, 1);
        assert!(session.instances[1].focus_handle.is_focused(window));
    });
}
