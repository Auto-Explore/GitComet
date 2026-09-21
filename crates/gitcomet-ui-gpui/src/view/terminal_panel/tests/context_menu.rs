use super::super::*;
use super::support::*;
use crate::test_support::refresh_and_draw;
use alacritty_terminal::event_loop::Msg as PtyMsg;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Point as GridPoint;
use alacritty_terminal::vte::ansi::{Handler, Processor};

struct TerminalMenuFixture {
    root: Entity<GitCometView>,
    repo_id: RepoId,
    viewport: Entity<TerminalViewportView>,
    term: AlacrittyTermLock,
    messages: smol::channel::Receiver<PtyMsg>,
}

fn recording_instance(
    session_seq: u64,
    cx: &mut gpui::Context<GitCometView>,
) -> (
    TerminalInstance,
    AlacrittyTermLock,
    smol::channel::Receiver<PtyMsg>,
) {
    let (events, _) = smol::channel::unbounded();
    let term = new_term(
        &terminal_config(TEST_SCROLLBACK),
        &TerminalDims {
            columns: TEST_COLS,
            screen_lines: TEST_ROWS,
            total_lines: TEST_SCROLLBACK + TEST_ROWS,
        },
        events,
    );
    let (sender, messages) = PtySender::recording();
    let mut instance = test_terminal_instance(session_seq, None, cx);
    instance.pty_sender = Some(sender.clone());
    instance.viewport.update(cx, |viewport, _| {
        viewport.term_lock = Some(term.clone());
        viewport.pty_sender = Some(sender);
    });
    (instance, term, messages)
}

fn fixture(cx: &mut gpui::TestAppContext) -> (TerminalMenuFixture, &mut gpui::VisualTestContext) {
    let (root, repo_id, cx) = test_root_view_with_active_repo(cx);
    let (viewport, term, messages) = cx.update(|window, app| {
        root.update(app, |root, cx| {
            crate::view::test_support::push_test_state(root, root.state.clone(), cx);
            let (instance, term, messages) = recording_instance(10, cx);
            let viewport = instance.viewport.clone();
            root.terminal_sessions.insert(
                repo_id,
                RepoTerminalSession {
                    workdir: PathBuf::from("/tmp/terminal-menu-test"),
                    repo_name: "terminal-menu-test".to_string(),
                    instances: vec![instance],
                    active_index: 0,
                },
            );
            root.focus_terminal_view(repo_id, window, cx);
            cx.notify();
            (viewport, term, messages)
        })
    });
    cx.run_until_parked();
    refresh_and_draw(cx);
    {
        let mut term = term.lock();
        for i in 0..50 {
            for c in format!("line{i:03}\r\n").chars() {
                match c {
                    '\r' => term.carriage_return(),
                    '\n' => term.linefeed(),
                    c => term.input(c),
                }
            }
        }
        let mut parser: Processor = Processor::new();
        parser.advance(&mut *term, b"\x1b[?2004h");
        term.scroll_display(Scroll::Delta(3));
    }
    cx.update(|window, app| {
        app.write_to_clipboard(gpui::ClipboardItem::new_string("paste\npayload".into()));
        viewport.update(app, |viewport, cx| {
            viewport.select_all(window, cx);
            viewport.select_all_active = false;
            viewport.selection_start = Some(TerminalGridPoint::new(-3, 0));
            viewport.selection_end = Some(TerminalGridPoint::new(-3, 6));
        });
    });
    refresh_and_draw(cx);
    let fixture = TerminalMenuFixture {
        root,
        repo_id,
        viewport,
        term,
        messages,
    };
    assert!(fixture.take_input().is_empty());
    (fixture, cx)
}

impl TerminalMenuFixture {
    fn take_input(&self) -> Vec<u8> {
        let mut input = Vec::new();
        while let Ok(message) = self.messages.try_recv() {
            if let PtyMsg::Input(bytes) = message {
                input.extend_from_slice(&bytes);
            }
        }
        input
    }

    fn menu_open(&self, cx: &mut gpui::VisualTestContext) -> bool {
        cx.update(|_, app| self.root.read(app).popover_host.read(app).is_open())
    }

    fn open_menu(&self, padding: bool, cx: &mut gpui::VisualTestContext) {
        let surface = cx
            .debug_bounds("terminal_context_surface")
            .expect("terminal surface");
        let position = if padding {
            point(surface.left() + px(1.0), surface.center().y)
        } else {
            surface.center()
        };
        cx.simulate_mouse_down(position, MouseButton::Right, gpui::Modifiers::default());
        assert!(!self.menu_open(cx), "a press alone must not open the menu");
        cx.simulate_mouse_up(position, MouseButton::Right, gpui::Modifiers::default());
        cx.run_until_parked();
        refresh_and_draw(cx);
        assert!(self.menu_open(cx));
    }

    fn activate(
        &self,
        selector: &'static str,
        index: usize,
        keyboard: bool,
        cx: &mut gpui::VisualTestContext,
    ) {
        if keyboard {
            // Moving outside prevents pointer hover from choosing a different row.
            cx.simulate_mouse_move(point(px(1.0), px(1.0)), None, gpui::Modifiers::default());
            cx.simulate_keystrokes("home");
            for _ in 0..index {
                cx.simulate_keystrokes("down");
            }
            cx.simulate_keystrokes("enter");
        } else {
            let row = cx
                .debug_bounds(selector)
                .expect("rendered terminal menu action");
            cx.simulate_click(row.center(), gpui::Modifiers::default());
        }
    }

    fn assert_focus_and_type(&self, cx: &mut gpui::VisualTestContext) {
        assert!(!self.menu_open(cx));
        cx.update(|window, app| assert!(self.viewport.read(app).focus_handle.is_focused(window)));
        cx.simulate_keystrokes("x");
        cx.run_until_parked();
        refresh_and_draw(cx);
        cx.update(|window, app| assert!(self.viewport.read(app).focus_handle.is_focused(window)));
        cx.simulate_keystrokes("y");
        cx.run_until_parked();
        refresh_and_draw(cx);
        assert_eq!(
            self.take_input(),
            b"xy",
            "input must reach the originating terminal exactly once"
        );
    }

    fn assert_cleared(&self, cx: &mut gpui::VisualTestContext) {
        let term = self.term.lock();
        assert_eq!(term.grid().history_size(), 0);
        assert_eq!(term.grid().display_offset(), 0);
        assert_eq!(term.grid().cursor.point, GridPoint::default());
        assert!(!term.grid().cursor.input_needs_wrap);
        assert!(term.grid().display_iter().all(|cell| cell.c == ' '));
        drop(term);
        cx.update(|_, app| {
            let viewport = self.viewport.read(app);
            assert!(!viewport.has_selection());
            assert!(!viewport.select_all_active);
            assert!(!viewport.selecting);
            assert_eq!(viewport.selected_text(), None);
            assert_eq!(viewport.copy_entire_buffer(), "");
            assert!(
                viewport
                    .last_content
                    .as_ref()
                    .unwrap()
                    .cells
                    .iter()
                    .all(|cell| cell.cell.c == ' ')
            );
        });
    }
}

#[gpui::test]
fn terminal_menu_actions_restore_focus_and_share_clipboard_commands(cx: &mut gpui::TestAppContext) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    for keyboard in [false, true] {
        for (index, (selector, command)) in [
            ("context_menu_copy", TerminalCommand::Copy),
            ("context_menu_paste", TerminalCommand::Paste),
            ("context_menu_select_all", TerminalCommand::SelectAll),
            (
                "context_menu_clear_screen_and_scrollback",
                TerminalCommand::ClearScreenAndScrollback,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let (fixture, cx) = fixture(cx);
            let selected = cx.update(|_, app| fixture.viewport.read(app).selected_text().unwrap());
            fixture.open_menu(false, cx);
            fixture.activate(selector, index, keyboard, cx);
            match command {
                TerminalCommand::Copy => {
                    cx.update(|_, app| {
                        assert_eq!(
                            app.read_from_clipboard()
                                .and_then(|item| item.text())
                                .as_deref(),
                            Some(selected.as_str())
                        )
                    });
                    assert!(fixture.take_input().is_empty());
                }
                TerminalCommand::Paste => {
                    assert_eq!(fixture.take_input(), b"\x1b[200~paste\npayload\x1b[201~")
                }
                TerminalCommand::SelectAll => cx.update(|_, app| {
                    let viewport = fixture.viewport.read(app);
                    assert!(viewport.select_all_active);
                    let geometry = viewport.grid_geometry().unwrap();
                    assert_eq!(
                        viewport.selection_start,
                        Some(TerminalGridPoint::new(-(geometry.history_size as i32), 0))
                    );
                    assert_eq!(
                        viewport.selection_end,
                        Some(TerminalGridPoint::new(
                            geometry.screen_lines as i32 - 1,
                            geometry.columns as u16 - 1
                        ))
                    );
                    let text = viewport.selected_text().unwrap();
                    assert!(text.starts_with("line000\n"));
                    assert!(text.ends_with("line049"));
                }),
                TerminalCommand::ClearScreenAndScrollback => {
                    fixture.assert_cleared(cx);
                    assert!(
                        fixture.take_input().is_empty(),
                        "clear must send no process input"
                    );
                }
            }
            fixture.assert_focus_and_type(cx);
        }
    }
}

#[gpui::test]
fn terminal_padding_menu_returns_focus_from_another_input_on_actions_and_escape(
    cx: &mut gpui::TestAppContext,
) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    for action in [
        None,
        Some(("context_menu_copy", 0)),
        Some(("context_menu_paste", 1)),
        Some(("context_menu_select_all", 2)),
        Some(("context_menu_clear_screen_and_scrollback", 3)),
    ] {
        let (fixture, cx) = fixture(cx);
        let input = cx.update(|window, app| {
            let input = fixture
                .root
                .read(app)
                .main_pane
                .read(app)
                .diff_search_input
                .clone();
            window.focus(&input.read(app).focus_handle(), app);
            input
        });
        fixture.open_menu(true, cx);
        if let Some((selector, index)) = action {
            fixture.activate(selector, index, false, cx);
        } else {
            cx.simulate_keystrokes("escape");
        }
        fixture.take_input();
        fixture.assert_focus_and_type(cx);
        cx.update(|_, app| assert!(input.read(app).text().is_empty()));
    }
}

#[gpui::test]
fn terminal_header_and_menu_clear_identically_without_a_connected_process(
    cx: &mut gpui::TestAppContext,
) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    for header in [false, true] {
        let (fixture, cx) = fixture(cx);
        cx.update(|_, app| {
            fixture.root.update(app, |root, cx| {
                let instance = root
                    .terminal_sessions
                    .get_mut(&fixture.repo_id)
                    .unwrap()
                    .instance_by_seq_mut(10)
                    .unwrap();
                instance.connected = false;
                instance.pty_sender = None;
                instance.viewport.update(cx, |v, cx| {
                    v.pty_sender = None;
                    cx.notify();
                });
                cx.notify();
            });
        });
        refresh_and_draw(cx);
        if header {
            let clear = cx.debug_bounds("terminal_clear").unwrap().center();
            cx.simulate_click(clear, gpui::Modifiers::default());
        } else {
            fixture.open_menu(false, cx);
            fixture.activate("context_menu_clear_screen_and_scrollback", 3, false, cx);
        }
        fixture.assert_cleared(cx);
        cx.run_until_parked();
        refresh_and_draw(cx);
        fixture.assert_cleared(cx);
        cx.update(|window, app| {
            assert!(fixture.viewport.read(app).focus_handle.is_focused(window))
        });
        assert!(fixture.take_input().is_empty());
        {
            let mut term = fixture.term.lock();
            for c in "next output".chars() {
                term.input(c);
            }
        }
        refresh_and_draw(cx);
        cx.update(|_, app| {
            assert_eq!(
                fixture.viewport.read(app).copy_entire_buffer(),
                "next output"
            )
        });
    }
}

#[gpui::test]
fn terminal_menu_mismatched_pointer_targets_do_not_activate(cx: &mut gpui::TestAppContext) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    let (fixture, cx) = fixture(cx);
    fixture.open_menu(false, cx);
    let copy = cx.debug_bounds("context_menu_copy").unwrap().center();
    let clear = cx
        .debug_bounds("context_menu_clear_screen_and_scrollback")
        .unwrap()
        .center();
    for (press, release, button) in [
        (copy, clear, MouseButton::Left),
        (clear, copy, MouseButton::Left),
        (copy, copy, MouseButton::Right),
    ] {
        cx.simulate_mouse_down(press, MouseButton::Left, gpui::Modifiers::default());
        refresh_and_draw(cx);
        cx.simulate_mouse_move(release, Some(MouseButton::Left), gpui::Modifiers::default());
        cx.simulate_mouse_up(release, button, gpui::Modifiers::default());
        cx.run_until_parked();
        refresh_and_draw(cx);
        assert!(fixture.menu_open(cx));
        cx.update(|_, app| {
            assert_eq!(
                app.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("paste\npayload")
            );
            assert!(!fixture.viewport.read(app).copy_entire_buffer().is_empty());
        });
        assert!(fixture.take_input().is_empty());
    }
    cx.simulate_keystrokes("escape");
    fixture.assert_focus_and_type(cx);
}

#[gpui::test]
fn terminal_clear_header_restores_focus_and_ctrl_l_stays_process_input(
    cx: &mut gpui::TestAppContext,
) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    let (fixture, cx) = fixture(cx);
    let before = cx.update(|_, app| fixture.viewport.read(app).copy_entire_buffer());
    cx.simulate_keystrokes("ctrl-l");
    assert_eq!(fixture.take_input(), b"\x0c");
    cx.update(|_, app| assert_eq!(fixture.viewport.read(app).copy_entire_buffer(), before));
    let clear = cx.debug_bounds("terminal_clear").unwrap().center();
    cx.simulate_click(clear, gpui::Modifiers::default());
    fixture.assert_cleared(cx);
    fixture.assert_focus_and_type(cx);
}

#[gpui::test]
fn terminal_menu_keeps_its_origin_when_the_active_session_changes(cx: &mut gpui::TestAppContext) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    for switch_repository in [false, true] {
        for (index, selector) in [
            "context_menu_copy",
            "context_menu_paste",
            "context_menu_select_all",
            "context_menu_clear_screen_and_scrollback",
        ]
        .into_iter()
        .enumerate()
        {
            let (fixture, cx) = fixture(cx);
            let expected_copy =
                cx.update(|_, app| fixture.viewport.read(app).selected_text().unwrap());
            fixture.open_menu(false, cx);
            let (other, other_messages) = cx.update(|_, app| {
                fixture.root.update(app, |root, cx| {
                    // Reuse the sequence in another repository to exercise both
                    // parts of the menu's identity independently.
                    let (instance, term, messages) =
                        recording_instance(if switch_repository { 10 } else { 20 }, cx);
                    term.lock().input('B');
                    let viewport = instance.viewport.clone();
                    if switch_repository {
                        let repo_id = RepoId(2);
                        let workdir = PathBuf::from("/tmp/terminal-menu-other");
                        let state = Arc::make_mut(&mut root.state);
                        state.repos.push(RepoState::new_opening(
                            repo_id,
                            gitcomet_core::domain::RepoSpec {
                                workdir: workdir.clone(),
                            },
                        ));
                        state.active_repo = Some(repo_id);
                        root.terminal_sessions.insert(
                            repo_id,
                            RepoTerminalSession {
                                workdir,
                                repo_name: "other".into(),
                                instances: vec![instance],
                                active_index: 0,
                            },
                        );
                    } else {
                        let session = root.terminal_sessions.get_mut(&fixture.repo_id).unwrap();
                        session.instances.push(instance);
                        session.active_index = 1;
                    }
                    cx.notify();
                    (viewport, messages)
                })
            });
            refresh_and_draw(cx);
            fixture.activate(selector, index, false, cx);
            assert!(!fixture.menu_open(cx));
            match index {
                0 => cx.update(|_, app| {
                    assert_eq!(
                        app.read_from_clipboard()
                            .and_then(|item| item.text())
                            .as_deref(),
                        Some(expected_copy.as_str())
                    )
                }),
                1 => assert_eq!(fixture.take_input(), b"\x1b[200~paste\npayload\x1b[201~"),
                2 => cx.update(|_, app| assert!(fixture.viewport.read(app).select_all_active)),
                3 => fixture.assert_cleared(cx),
                _ => unreachable!(),
            }
            cx.update(|_, app| {
                let other = other.read(app);
                assert_eq!(other.copy_entire_buffer(), "B");
                assert!(!other.has_selection());
            });
            while let Ok(message) = other_messages.try_recv() {
                assert!(
                    !matches!(message, PtyMsg::Input(_)),
                    "the newly active terminal must receive no menu input"
                );
            }
        }
    }
}

#[gpui::test]
fn terminal_session_disappearance_dismisses_menu_and_rejects_late_actions(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::panels::ContextMenuAction;
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    for remove_repo in [false, true] {
        let (fixture, cx) = fixture(cx);
        fixture.open_menu(false, cx);
        let (replacement, messages) = cx.update(|window, app| {
            fixture.root.update(app, |root, cx| {
                let (replacement, term, messages) = recording_instance(20, cx);
                term.lock().input('B');
                let viewport = replacement.viewport.clone();
                if remove_repo {
                    root.close_terminal_for_repo(fixture.repo_id, cx);
                    root.terminal_sessions.insert(
                        fixture.repo_id,
                        RepoTerminalSession {
                            workdir: PathBuf::from("/tmp/terminal-menu-replacement"),
                            repo_name: "replacement".into(),
                            instances: vec![replacement],
                            active_index: 0,
                        },
                    );
                } else {
                    root.terminal_sessions
                        .get_mut(&fixture.repo_id)
                        .unwrap()
                        .instances
                        .push(replacement);
                    root.close_terminal_tab(fixture.repo_id, 0, window, cx);
                }
                (viewport, messages)
            })
        });
        cx.run_until_parked();
        refresh_and_draw(cx);
        assert!(
            !fixture.menu_open(cx),
            "closing the origin must dismiss its menu"
        );
        let other_focus = cx.update(|window, app| {
            let focus = app.focus_handle();
            window.focus(&focus, app);
            focus
        });
        for command in [
            TerminalCommand::Copy,
            TerminalCommand::Paste,
            TerminalCommand::SelectAll,
            TerminalCommand::ClearScreenAndScrollback,
        ] {
            cx.update(|window, app| {
                let host = fixture.root.read(app).popover_host.clone();
                host.update(app, |host, cx| {
                    host.context_menu_activate_action(
                        ContextMenuAction::TerminalCommand {
                            repo_id: fixture.repo_id,
                            session_seq: 10,
                            command,
                        },
                        window,
                        cx,
                    )
                });
            });
        }
        cx.run_until_parked();
        refresh_and_draw(cx);
        cx.update(|window, app| {
            assert!(
                other_focus.is_focused(window),
                "a removed session must not reclaim focus"
            );
            assert_eq!(
                app.read_from_clipboard()
                    .and_then(|item| item.text())
                    .as_deref(),
                Some("paste\npayload")
            );
            assert_eq!(replacement.read(app).copy_entire_buffer(), "B");
            assert!(!replacement.read(app).has_selection());
        });
        assert!(fixture.take_input().is_empty());
        while let Ok(message) = messages.try_recv() {
            assert!(!matches!(message, PtyMsg::Input(_)));
        }
    }
}

#[gpui::test]
fn terminal_clipboard_shortcuts_keep_full_buffer_copy_and_bracketed_paste(
    cx: &mut gpui::TestAppContext,
) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    let (fixture, cx) = fixture(cx);
    let (select_all, copy, paste) = if cfg!(target_os = "macos") {
        ("cmd-a", "cmd-c", "cmd-v")
    } else {
        ("ctrl-shift-a", "ctrl-shift-c", "ctrl-shift-v")
    };
    cx.simulate_keystrokes(select_all);
    cx.simulate_keystrokes(copy);
    let copied = cx.update(|_, app| {
        let text = app
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap();
        assert_eq!(
            fixture.viewport.read(app).selected_text().as_deref(),
            Some(text.as_str())
        );
        assert!(text.starts_with("line000\n"));
        assert!(text.ends_with("line049"));
        text
    });
    cx.simulate_keystrokes(paste);
    assert_eq!(
        fixture.take_input(),
        [b"\x1b[200~", copied.as_bytes(), b"\x1b[201~"].concat()
    );
}

#[gpui::test]
fn terminal_clearing_is_disabled_without_a_buffer(cx: &mut gpui::TestAppContext) {
    let _visual = crate::test_support::lock_visual_test();
    let _clipboard = crate::test_support::lock_clipboard_test();
    let (fixture, cx) = fixture(cx);
    cx.update(|_, app| {
        fixture.viewport.update(app, |v, cx| {
            v.term_lock = None;
            cx.notify();
        })
    });
    refresh_and_draw(cx);
    let clear = cx.debug_bounds("terminal_clear").unwrap().center();
    cx.simulate_click(clear, gpui::Modifiers::default());
    assert!(fixture.take_input().is_empty());
    fixture.open_menu(false, cx);
    fixture.activate("context_menu_clear_screen_and_scrollback", 3, false, cx);
    assert!(
        fixture.menu_open(cx),
        "the disabled clearing entry must not activate"
    );
    assert!(fixture.take_input().is_empty());
}
