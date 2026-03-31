use super::*;

#[gpui::test]
fn commit_menu_has_add_tag_entry(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_menu_tag",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.log = Loadable::Ready(
                gitcomet_core::domain::LogPage {
                    commits: vec![gitcomet_core::domain::Commit {
                        id: commit_id.clone(),
                        parent_ids: vec![],
                        summary: "Hello".into(),
                        author: "Alice".into(),
                        time: SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                }
                .into(),
            );
            repo.tags = Loadable::Ready(Arc::new(vec![]));

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected commit context menu model");

        let add_tag_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Add tag…" => {
                Some((**action).clone())
            }
            _ => None,
        });

        let Some(ContextMenuAction::OpenPopover { kind }) = add_tag_action else {
            panic!("expected Add tag… to open a popover");
        };

        let PopoverKind::CreateTagPrompt {
            repo_id: rid,
            target,
        } = kind
        else {
            panic!("expected Add tag… to open CreateTagPrompt");
        };

        assert_eq!(rid, repo_id);
        assert_eq!(target, commit_id.as_ref().to_string());
    });
}

#[gpui::test]
fn commit_file_menu_has_open_file_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(2);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let path = std::path::PathBuf::from("src/main.rs");

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitFileMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected commit file context menu model");

        let open_file_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Open file" => {
                Some((**action).clone())
            }
            _ => None,
        });
        match open_file_action {
            Some(ContextMenuAction::OpenFile {
                repo_id: rid,
                path: p,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(p, path);
            }
            _ => panic!("expected Open file entry with OpenFile action"),
        }

        let open_location_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Open file location" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        match open_location_action {
            Some(ContextMenuAction::OpenFileLocation {
                repo_id: rid,
                path: p,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(p, path);
            }
            _ => panic!("expected Open file location entry with OpenFileLocation action"),
        }
    });
}

#[gpui::test]
fn status_file_menu_has_open_file_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(3);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_open_file",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("a.txt");

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
        });
    });

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::StatusFileMenu {
                            repo_id,
                            area: DiffArea::Unstaged,
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected status file context menu model");

        let open_file_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Open file" => {
                Some((**action).clone())
            }
            _ => None,
        });
        match open_file_action {
            Some(ContextMenuAction::OpenFile {
                repo_id: rid,
                path: p,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(p, path);
            }
            _ => panic!("expected Open file entry with OpenFile action"),
        }

        let open_location_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Open file location" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        match open_location_action {
            Some(ContextMenuAction::OpenFileLocation {
                repo_id: rid,
                path: p,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(p, path);
            }
            _ => panic!("expected Open file location entry with OpenFileLocation action"),
        }
    });
}

#[gpui::test]
fn status_file_menu_copy_path_uses_os_native_separators(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(33);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_copy_path_native",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this._ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::StatusFileMenu {
                            repo_id,
                            area: DiffArea::Unstaged,
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected status file context menu model");

        let copy_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Copy path" => {
                Some((**action).clone())
            }
            _ => None,
        });

        let mut expected = workdir.clone();
        expected.push("crates");
        expected.push("gitcomet-ui-gpui");
        expected.push("src");
        expected.push("smoke_tests.rs");

        match copy_action {
            Some(ContextMenuAction::CopyText { text }) => {
                assert_eq!(text, expected.display().to_string());
                #[cfg(target_os = "windows")]
                assert!(
                    !text.contains('/'),
                    "copy-path text should use Windows separators only: {text}"
                );
            }
            _ => panic!("expected Copy path entry with CopyText action"),
        }
    });
}

#[gpui::test]
fn commit_file_menu_copy_path_uses_os_native_separators(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(34);
    let commit_id = CommitId("beadbeadbeadbead".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_menu_copy_path_native",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this._ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::CommitFileMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected commit file context menu model");

        let copy_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Copy path" => {
                Some((**action).clone())
            }
            _ => None,
        });

        let mut expected = workdir.clone();
        expected.push("crates");
        expected.push("gitcomet-ui-gpui");
        expected.push("src");
        expected.push("smoke_tests.rs");

        match copy_action {
            Some(ContextMenuAction::CopyText { text }) => {
                assert_eq!(text, expected.display().to_string());
                #[cfg(target_os = "windows")]
                assert!(
                    !text.contains('/'),
                    "copy-path text should use Windows separators only: {text}"
                );
            }
            _ => panic!("expected Copy path entry with CopyText action"),
        }
    });
}

#[gpui::test]
fn commit_file_menu_copy_path_supports_right_button_release(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
    });

    let repo_id = RepoId(35);
    let commit_id = CommitId("feedfacefeedface".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_menu_copy_path_right_release",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this._ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    cx.write_to_clipboard(gpui::ClipboardItem::new_string("initial".to_string()));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CommitFileMenu {
                        repo_id,
                        commit_id: commit_id.clone(),
                        path: path.clone(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let copy_bounds = cx
        .debug_bounds("context_menu_copy_path")
        .expect("expected Copy path context menu row");
    let copy_center = copy_bounds.center();

    cx.simulate_mouse_move(
        copy_center,
        Some(gpui::MouseButton::Right),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: copy_center,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
    });

    let mut expected = workdir.clone();
    expected.push("crates");
    expected.push("gitcomet-ui-gpui");
    expected.push("src");
    expected.push("smoke_tests.rs");

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected.display().to_string())
    );
}

#[gpui::test]
fn status_file_menu_copy_path_supports_right_button_release(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        view.update(app, |this, _cx| this.disable_poller_for_tests());
    });

    let repo_id = RepoId(36);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_copy_path_right_release",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            this._ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    cx.write_to_clipboard(gpui::ClipboardItem::new_string("initial".to_string()));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::StatusFileMenu {
                        repo_id,
                        area: DiffArea::Unstaged,
                        path: path.clone(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let copy_bounds = cx
        .debug_bounds("context_menu_copy_path")
        .expect("expected Copy path context menu row");
    let copy_center = copy_bounds.center();

    cx.simulate_mouse_move(
        copy_center,
        Some(gpui::MouseButton::Right),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: copy_center,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Right,
        click_count: 1,
    });

    let mut expected = workdir.clone();
    expected.push("crates");
    expected.push("gitcomet-ui-gpui");
    expected.push("src");
    expected.push("smoke_tests.rs");

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected.display().to_string())
    );
}

#[gpui::test]
fn diff_editor_menu_has_open_file_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(4);
    let path = std::path::PathBuf::from("a.txt");

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::DiffEditorMenu {
                            repo_id,
                            area: DiffArea::Unstaged,
                            path: Some(path.clone()),
                            hunk_patch: None,
                            hunks_count: 0,
                            lines_patch: None,
                            discard_lines_patch: None,
                            lines_count: 0,
                            copy_text: Some("x".to_string()),
                        },
                        cx,
                    )
                })
            })
            .expect("expected diff editor context menu model");

        let open_file_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Open file" => {
                Some((**action).clone())
            }
            _ => None,
        });
        match open_file_action {
            Some(ContextMenuAction::OpenFile {
                repo_id: rid,
                path: p,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(p, path);
            }
            _ => panic!("expected Open file entry with OpenFile action"),
        }

        let open_location_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Open file location" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        match open_location_action {
            Some(ContextMenuAction::OpenFileLocation {
                repo_id: rid,
                path: p,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(p, path);
            }
            _ => panic!("expected Open file location entry with OpenFileLocation action"),
        }
    });
}

#[gpui::test]
fn file_preview_context_menu_matches_diff_editor_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(44);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_context_menu",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("added.txt");
    std::fs::create_dir_all(&workdir).expect("create preview test workdir");
    std::fs::write(workdir.join(&path), "alpha\nbeta\n").expect("write preview test file");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.status = Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }],
                    unstaged: vec![],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: path.clone(),
                area: DiffArea::Staged,
            });
            repo.diff_state.diff_file =
                Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText {
                    path: path.clone(),
                    old: None,
                    new: Some("alpha\nbeta\n".to_string()),
                })));

            let next_state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this._ui_model.update(cx, |model, cx| {
                model.set_state(next_state, cx);
            });
        });
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_populate_worktree_preview_from_diff_file(cx);
            pane.open_diff_editor_context_menu(
                1,
                DiffTextRegion::Inline,
                point(px(24.0), px(24.0)),
                window,
                cx,
            );
        });
    });

    // Flush deferred popover open from MainPaneView::open_popover_at.
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let Some(popover_kind) = host.popover.clone() else {
                    panic!("expected file preview right-click to open a context menu");
                };

                match &popover_kind {
                    PopoverKind::DiffEditorMenu {
                        repo_id: rid,
                        area,
                        path: menu_path,
                        copy_text,
                        ..
                    } => {
                        assert_eq!(*rid, repo_id);
                        assert_eq!(*area, DiffArea::Staged);
                        assert_eq!(menu_path, &Some(path.clone()));
                        assert_eq!(copy_text, &Some("beta".to_string()));
                    }
                    _ => panic!("expected DiffEditorMenu popover for file preview"),
                }

                let model = host
                    .context_menu_model(&popover_kind, cx)
                    .expect("expected diff editor menu model");

                let labels: Vec<String> = model
                    .items
                    .iter()
                    .filter_map(|item| match item {
                        ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                        _ => None,
                    })
                    .collect();
                for expected in [
                    "Unstage line",
                    "Unstage hunk",
                    "Open file",
                    "Open file location",
                    "Copy",
                ] {
                    assert!(
                        labels.iter().any(|label| label == expected),
                        "expected {expected} entry in preview context menu"
                    );
                }

                let open_file_action = model.items.iter().find_map(|item| match item {
                    ContextMenuItem::Entry { label, action, .. }
                        if label.as_ref() == "Open file" =>
                    {
                        Some((**action).clone())
                    }
                    _ => None,
                });
                match open_file_action {
                    Some(ContextMenuAction::OpenFile {
                        repo_id: rid,
                        path: p,
                    }) => {
                        assert_eq!(rid, repo_id);
                        assert_eq!(p, path);
                    }
                    _ => panic!("expected Open file action in preview context menu"),
                }

                let copy_action = model.items.iter().find_map(|item| match item {
                    ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Copy" => {
                        Some((**action).clone())
                    }
                    _ => None,
                });
                match copy_action {
                    Some(ContextMenuAction::CopyText { text }) => {
                        assert_eq!(text, "beta");
                    }
                    _ => panic!("expected Copy action in preview context menu"),
                }
            });
        });
    });
}
