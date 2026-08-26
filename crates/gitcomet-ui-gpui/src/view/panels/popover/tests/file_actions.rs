use super::*;
use crate::view::panels::tests::wait_for_main_pane_condition;
use crate::view::panels::tests::{
    app_state_with_repo, opening_repo_state, push_test_state, set_test_file_status,
};

fn context_menu_entry_disabled(model: &ContextMenuModel, label: &str) -> bool {
    model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label: entry_label,
                disabled,
                ..
            } if entry_label.as_ref() == label => Some(*disabled),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected `{label}` context menu entry"))
}

fn context_menu_has_entry(model: &ContextMenuModel, label: &str) -> bool {
    model.items.iter().any(|item| {
        matches!(
            item,
            ContextMenuItem::Entry {
                label: entry_label,
                ..
            } if entry_label.as_ref() == label
        )
    })
}

fn commit_menu_test_repo(repo_id: RepoId, commit_id: &CommitId) -> RepoState {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_menu",
        std::process::id()
    ));
    let mut repo = RepoState::new_opening(repo_id, gitcomet_core::domain::RepoSpec { workdir });
    repo.log = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits: vec![gitcomet_core::domain::Commit {
                id: commit_id.clone(),
                parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                summary: "Hello".into(),
                author: "Alice".into(),
                time: SystemTime::UNIX_EPOCH,
            }],
            next_cursor: None,
        }
        .into(),
    );
    repo.tags = Loadable::Ready(Arc::new(vec![]));
    repo.rebase_in_progress = Loadable::Ready(false);
    repo.sequencer_state = Loadable::Ready(gitcomet_core::services::SequencerState::None);
    repo.merge_commit_message = Loadable::Ready(None);
    repo
}

#[gpui::test]
fn commit_menu_has_add_tag_entry(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_menu_test_repo(repo_id, &commit_id);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
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
fn commit_menu_cherry_pick_action_opens_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_menu_test_repo(repo_id, &commit_id);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.cherry_pick_mainline = Some(2);
                host.context_menu_activate_action(
                    ContextMenuAction::CherryPickCommit {
                        repo_id,
                        commit_id: commit_id.clone(),
                    },
                    window,
                    cx,
                );
                assert_eq!(
                    host.popover_kind_for_tests(),
                    Some(PopoverKind::CherryPickCommitConfirm {
                        repo_id,
                        commit_id: commit_id.clone()
                    })
                );
                assert_eq!(
                    host.cherry_pick_mainline, None,
                    "opening a single cherry-pick must not reuse an earlier parent choice"
                );
            });
        });
    });
}

#[gpui::test]
fn commit_menu_hides_cherry_pick_for_current_head(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.detached_head_commit = Some(commit_id.clone());
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
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

        assert!(!context_menu_has_entry(&model, "Cherry-pick"));
    });
}

#[gpui::test]
fn commit_menu_disables_cherry_pick_when_local_operation_in_progress(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.local_actions_in_flight = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
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

        assert!(context_menu_entry_disabled(&model, "Cherry-pick"));
    });
}

#[gpui::test]
fn commit_menu_disables_merge_when_repository_is_busy(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(11);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.head_branch = Loadable::Ready("main".to_string());
            repo.local_actions_in_flight = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
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

        assert!(context_menu_entry_disabled(
            &model,
            "Merge deadbeef into main"
        ));
    });
}

#[gpui::test]
fn detached_head_commit_menu_names_head_as_merge_destination(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(12);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.head_branch = Loadable::Ready("HEAD".to_string());
            repo.detached_head_commit = Some(CommitId("cafebabecafebabe".into()));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
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

        assert!(context_menu_has_entry(&model, "Merge deadbeef into HEAD"));
        assert!(!context_menu_has_entry(
            &model,
            "Merge deadbeef into deadbeef"
        ));
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
fn unopened_submodule_menus_disable_open_in_code_editor(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    crate::external_editor::set_configured_setting_override(Some(
        gitcomet_state::session::ExternalCodeEditorSetting::Custom {
            executable: std::path::PathBuf::from("/usr/bin/editor"),
            arguments: None,
        },
    ));
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(4);
    let commit_id = CommitId("baadf00dbaadf00d".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unopened_submodule_editor_menu",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("vendor/lib");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.history_state.commit_details = Loadable::Ready(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "Submodule update".into(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: String::new(),
                    committed_at_unix: 0,
                    parent_ids: Vec::new(),
                    files: vec![gitcomet_core::domain::CommitFileChange {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        is_submodule: true,
                        additions: None,
                        deletions: None,
                    }],
                }
                .into(),
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
            repo.submodules = Loadable::Ready(
                vec![gitcomet_core::domain::Submodule {
                    path: path.clone(),
                    recorded_head: CommitId("1111111111111111".into()),
                    checked_out_head: None,
                    status: gitcomet_core::domain::SubmoduleStatus::NotInitialized,
                }]
                .into(),
            );

            let state = app_state_with_repo(repo, repo_id);
            this.state = Arc::clone(&state);
            push_test_state(this, state, cx);
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let commit_model = view
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
        assert!(context_menu_entry_disabled(&commit_model, "Open submodule"));
        assert!(context_menu_entry_disabled(
            &commit_model,
            "Open in code editor"
        ));

        let status_model = view
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
        assert!(context_menu_entry_disabled(&status_model, "Open submodule"));
        assert!(context_menu_entry_disabled(
            &status_model,
            "Open in code editor"
        ));
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
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Copy absolute path" =>
            {
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
            _ => panic!("expected Copy absolute path entry with CopyText action"),
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
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Copy absolute path" =>
            {
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
            _ => panic!("expected Copy absolute path entry with CopyText action"),
        }
    });
}

#[gpui::test]
fn commit_file_menu_copy_path_supports_right_button_release(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

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
        .debug_bounds("context_menu_copy_absolute_path")
        .expect("expected Copy absolute path context menu row");
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
        .debug_bounds("context_menu_copy_absolute_path")
        .expect("expected Copy absolute path context menu row");
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
                            copy_target: None,
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
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                DiffArea::Staged,
            );
            repo.diff_state.diff_file = Loadable::Error(
                "materialized diff_file should not be consulted for file preview".into(),
            );
            repo.diff_state.diff_preview_text_file =
                Loadable::Ready(Some(Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: workdir.join(&path),
                    side: gitcomet_core::domain::DiffPreviewTextSide::New,
                })));
            repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file preview ready before opening preview context menu",
        |pane| matches!(pane.worktree_preview, Loadable::Ready(3)),
        |pane| {
            format!(
                "preview={:?} preview_path={:?} source_path={:?}",
                pane.worktree_preview,
                pane.worktree_preview_path,
                pane.worktree_preview_source_path
            )
        },
    );

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
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
                    Some(ContextMenuAction::CopyDiffSelection { text }) => {
                        assert_eq!(text, "beta");
                    }
                    _ => panic!("expected Copy action in preview context menu"),
                }
            });
        });
    });
}

fn context_menu_action_for(model: &ContextMenuModel, label: &str) -> ContextMenuAction {
    model
        .items
        .iter()
        .find_map(|item| match item {
            ContextMenuItem::Entry {
                label: entry_label,
                action,
                ..
            } if entry_label.as_ref() == label => Some((**action).clone()),
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected `{label}` context menu entry"))
}

/// Builds a repo whose file browser holds `src/` (with `src/a.rs` inside it) and
/// returns the folder menu's model for `src`.
fn file_browser_folder_menu_model(
    cx: &mut gpui::TestAppContext,
    configure: impl FnOnce(&mut RepoState),
) -> ContextMenuModel {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(71);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_browser_folder_menu",
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
            repo.file_browser.entries = Loadable::Ready(Arc::new(vec![
                gitcomet_core::domain::FileEntry {
                    name: "src".to_string(),
                    path: Arc::new(std::path::PathBuf::from("src")),
                    kind: gitcomet_core::domain::FileEntryKind::Directory,
                    depth: 0,
                },
                gitcomet_core::domain::FileEntry {
                    name: "a.rs".to_string(),
                    path: Arc::new(std::path::PathBuf::from("src/a.rs")),
                    kind: gitcomet_core::domain::FileEntryKind::File,
                    depth: 1,
                },
            ]));
            configure(&mut repo);

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
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_model(
                    &PopoverKind::FileBrowserFolderMenu {
                        repo_id,
                        path: std::path::PathBuf::from("src"),
                    },
                    cx,
                )
            })
        })
        .expect("expected file browser folder context menu model")
    })
}

#[gpui::test]
fn file_browser_folder_menu_offers_tree_os_and_copy_actions(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    crate::external_editor::set_configured_setting_override(Some(
        gitcomet_state::session::ExternalCodeEditorSetting::Custom {
            executable: std::path::PathBuf::from("/usr/bin/editor"),
            arguments: None,
        },
    ));
    let model = file_browser_folder_menu_model(cx, |_repo| {});

    // A collapsed folder's toggle says what activating it will do.
    assert!(context_menu_has_entry(&model, "Expand"));
    assert!(!context_menu_has_entry(&model, "Collapse"));
    assert!(context_menu_has_entry(&model, "Expand all under here"));
    assert!(context_menu_has_entry(&model, "Collapse all under here"));
    assert!(context_menu_has_entry(&model, "Open folder location"));
    assert!(context_menu_has_entry(&model, "Open in code editor"));
    assert!(context_menu_has_entry(&model, "Copy absolute path"));
    assert!(context_menu_has_entry(&model, "Copy relative path"));

    match context_menu_action_for(&model, "Expand") {
        ContextMenuAction::ToggleFileBrowserDir { path, .. } => {
            assert_eq!(path, std::path::PathBuf::from("src"));
        }
        _ => panic!("expected a ToggleFileBrowserDir action"),
    }
    match context_menu_action_for(&model, "Expand all under here") {
        ContextMenuAction::SetFileBrowserDirExpandedRecursive { expanded, .. } => {
            assert!(expanded);
        }
        _ => panic!("expected a recursive expand action"),
    }
    match context_menu_action_for(&model, "Collapse all under here") {
        ContextMenuAction::SetFileBrowserDirExpandedRecursive { expanded, .. } => {
            assert!(!expanded);
        }
        _ => panic!("expected a recursive collapse action"),
    }
}

#[gpui::test]
fn file_browser_folder_menu_toggle_label_follows_expanded_state(cx: &mut gpui::TestAppContext) {
    let model = file_browser_folder_menu_model(cx, |repo| {
        repo.file_browser
            .expanded_dirs
            .insert(Arc::new(std::path::PathBuf::from("src")));
    });

    assert!(context_menu_has_entry(&model, "Collapse"));
    assert!(
        !context_menu_has_entry(&model, "Expand"),
        "an open folder must not offer to open again"
    );
}

/// With a search filtering the tree, every directory renders force-expanded and
/// `expanded_dirs` is ignored — so a toggle would change state that nothing
/// reads. The entries have to say they are unavailable rather than no-op.
#[gpui::test]
fn file_browser_folder_menu_disables_expand_entries_while_searching(cx: &mut gpui::TestAppContext) {
    let model = file_browser_folder_menu_model(cx, |repo| {
        repo.file_browser.search_query = "a.rs".to_string();
    });

    assert!(context_menu_entry_disabled(&model, "Expand"));
    assert!(context_menu_entry_disabled(&model, "Expand all under here"));
    assert!(context_menu_entry_disabled(
        &model,
        "Collapse all under here"
    ));
    // The copy entries do not depend on the tree's shape, so they stay live.
    assert!(!context_menu_entry_disabled(&model, "Copy absolute path"));
    assert!(!context_menu_entry_disabled(&model, "Copy relative path"));
}

/// A folder listed from a commit has no guaranteed counterpart on disk, so the
/// OS actions drop out — the same line the file menu draws.
#[gpui::test]
fn file_browser_folder_menu_hides_os_actions_for_a_commit_source(cx: &mut gpui::TestAppContext) {
    let model = file_browser_folder_menu_model(cx, |repo| {
        repo.file_browser.source =
            gitcomet_core::domain::FileSource::Commit(CommitId("abc123".into()));
    });

    assert!(!context_menu_has_entry(&model, "Open folder location"));
    assert!(!context_menu_has_entry(&model, "Open in code editor"));
    assert!(context_menu_has_entry(&model, "Expand"));
    assert!(context_menu_has_entry(&model, "Copy absolute path"));
    assert!(context_menu_has_entry(&model, "Copy relative path"));
}

#[gpui::test]
fn file_browser_folder_menu_copy_entries_carry_different_paths(cx: &mut gpui::TestAppContext) {
    let model = file_browser_folder_menu_model(cx, |_repo| {});

    let absolute = match context_menu_action_for(&model, "Copy absolute path") {
        ContextMenuAction::CopyText { text } => text,
        _ => panic!("expected a CopyText action"),
    };
    let relative = match context_menu_action_for(&model, "Copy relative path") {
        ContextMenuAction::CopyText { text } => text,
        _ => panic!("expected a CopyText action"),
    };

    assert_eq!(relative, "src");
    assert!(
        absolute.ends_with("src") && absolute != relative,
        "the absolute entry has to be workdir-joined, got {absolute}"
    );
    #[cfg(target_os = "windows")]
    assert!(
        !absolute.contains('/'),
        "copy-path text should use Windows separators only: {absolute}"
    );
}

/// Without a configured editor the entry can only produce a "not configured"
/// error toast, so it must not be offered at all — the gate every other menu
/// carrying this entry applies.
#[gpui::test]
fn file_browser_folder_menu_hides_code_editor_entry_without_a_configured_editor(
    cx: &mut gpui::TestAppContext,
) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    crate::external_editor::set_configured_setting_override(None);

    let model = file_browser_folder_menu_model(cx, |_repo| {});

    assert!(!context_menu_has_entry(&model, "Open in code editor"));
    // The rest of the OS block does not depend on the editor setting.
    assert!(context_menu_has_entry(&model, "Open folder location"));
}

/// The search input is multiline and stores what was typed verbatim, so a lone
/// space is a non-empty query that filters nothing. Keying the disabled state
/// on `!is_empty()` would grey out working controls.
#[gpui::test]
fn file_browser_folder_menu_keeps_expand_live_for_a_whitespace_only_query(
    cx: &mut gpui::TestAppContext,
) {
    let model = file_browser_folder_menu_model(cx, |repo| {
        repo.file_browser.search_query = "   \n".to_string();
    });

    assert!(!context_menu_entry_disabled(&model, "Expand"));
    assert!(!context_menu_entry_disabled(
        &model,
        "Expand all under here"
    ));
}

/// `c` is matched on the key alone, so it has to mean one thing in every menu.
/// Before the split it selected the absolute path everywhere; it now selects
/// the relative one everywhere, and this pins the two menus together.
#[gpui::test]
fn copy_path_mnemonic_selects_the_relative_entry_in_every_menu(cx: &mut gpui::TestAppContext) {
    fn relative_entry_owns_the_mnemonic(model: &ContextMenuModel) {
        for item in &model.items {
            let ContextMenuItem::Entry {
                label, shortcut, ..
            } = item
            else {
                continue;
            };
            let mnemonic = shortcut
                .as_ref()
                .and_then(|s| s.as_ref().rsplit('+').next().map(str::to_ascii_lowercase));
            if label.as_ref() == "Copy relative path" {
                assert_eq!(
                    mnemonic.as_deref(),
                    Some("c"),
                    "the relative entry must own the copy mnemonic"
                );
            } else if label.as_ref() == "Copy absolute path" {
                assert_ne!(
                    mnemonic.as_deref(),
                    Some("c"),
                    "the absolute entry must not shadow it"
                );
            }
        }
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(72);
    let commit_id = CommitId("aaaaaaaaaaaa".into());
    let path = std::path::PathBuf::from("src/lib.rs");
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_copy_mnemonic",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.spec.workdir = workdir.clone();
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
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let status = host
                    .context_menu_model(
                        &PopoverKind::StatusFileMenu {
                            repo_id,
                            area: DiffArea::Unstaged,
                            path: path.clone(),
                        },
                        cx,
                    )
                    .expect("expected the status file menu");
                relative_entry_owns_the_mnemonic(&status);

                let commit = host
                    .context_menu_model(
                        &PopoverKind::CommitFileMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                            path: path.clone(),
                        },
                        cx,
                    )
                    .expect("expected the commit file menu");
                relative_entry_owns_the_mnemonic(&commit);
            });
        });
    });
}
