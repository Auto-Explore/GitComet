use super::*;
use crate::view::panels::popover::context_menu::context_menu_shortcut_entry_ix;
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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

        let add_tag_ix = model
            .items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ContextMenuItem::Entry { label, .. } if label.as_ref() == "Add tag…"
                )
            })
            .expect("expected Add tag… context menu entry");
        assert!(!context_menu_entry_disabled(&model, "Add tag…"));
        assert!(!model.entry_tooltips.contains_key(&add_tag_ix));

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
fn commit_menu_disables_add_tag_when_history_tags_are_hidden(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_menu_test_repo(repo_id, &commit_id);
            let mut state = app_state_with_repo(repo, repo_id);
            Arc::make_mut(&mut state).git_log_settings.show_history_tags = false;
            push_test_state(this, state, cx);
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

        let add_tag_ix = model
            .items
            .iter()
            .position(|item| {
                matches!(
                    item,
                    ContextMenuItem::Entry { label, .. } if label.as_ref() == "Add tag…"
                )
            })
            .expect("expected Add tag… context menu entry");

        assert!(context_menu_entry_disabled(&model, "Add tag…"));
        assert_eq!(context_menu_shortcut_entry_ix(&model, "T"), None);
        assert_eq!(
            model.entry_tooltips.get(&add_tag_ix).map(|t| t.as_ref()),
            Some("Enable “Show tags in history view” in Settings > Git log to add tags.")
        );
    });
}

#[gpui::test]
fn commit_menu_cherry_pick_action_opens_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                host.commit_mainline = Some(2);
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
                    host.commit_mainline, None,
                    "opening a single cherry-pick must not reuse an earlier parent choice"
                );
            });
        });
    });
}

#[gpui::test]
fn commit_menu_revert_action_opens_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                host.commit_mainline = Some(2);
                host.context_menu_activate_action(
                    ContextMenuAction::RevertCommit {
                        repo_id,
                        commit_id: commit_id.clone(),
                    },
                    window,
                    cx,
                );
                assert_eq!(
                    host.popover_kind_for_tests(),
                    Some(PopoverKind::RevertCommitConfirm {
                        repo_id,
                        commit_id: commit_id.clone()
                    }),
                    "revert confirms first instead of dispatching immediately"
                );
                assert_eq!(
                    host.commit_mainline, None,
                    "opening a revert must not reuse an earlier parent choice"
                );
            });
        });
    });
}

#[gpui::test]
fn revert_confirm_keeps_and_restores_the_commit_menu_invoker_focus(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
        let invoker = app.focus_handle();
        window.focus(&invoker, app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CommitMenu {
                        repo_id,
                        commit_id: commit_id.clone(),
                    },
                    gpui::point(gpui::px(0.0), gpui::px(0.0)),
                    window,
                    cx,
                );
                host.context_menu_activate_action(
                    ContextMenuAction::RevertCommit {
                        repo_id,
                        commit_id: commit_id.clone(),
                    },
                    window,
                    cx,
                );
                assert_eq!(
                    host.menu_invoker_focus.as_ref(),
                    Some(&invoker),
                    "the dialog must remember what the commit menu was opened from"
                );
                host.dismiss_prompt_popover(window, cx);
            });
        });
        assert!(
            invoker.is_focused(window),
            "closing the dialog restores focus"
        );
    });
}

#[gpui::test]
fn commit_menu_hides_cherry_pick_for_current_head(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(2);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    let path = std::path::PathBuf::from("src/main.rs");
    let old_path = std::path::PathBuf::from("src/old.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "Rename".into(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: String::new(),
                    committed_at_unix: 0,
                    parent_ids: Vec::new(),
                    files: vec![
                        gitcomet_core::domain::CommitFileChange::new(
                            path.clone(),
                            gitcomet_core::domain::FileStatusKind::Renamed,
                        )
                        .with_old_path(Some(old_path.clone())),
                    ],
                }));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.run_until_parked();
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

        // The row's click opens the diff, with its rename source; the menu
        // does not repeat it.
        assert!(!model.items.iter().any(|item| matches!(
            item,
            ContextMenuItem::Entry { action, .. }
                if matches!(action.as_ref(), ContextMenuAction::SelectDiff { .. })
        )));

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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    files: vec![
                        gitcomet_core::domain::CommitFileChange::new(
                            path.clone(),
                            gitcomet_core::domain::FileStatusKind::Modified,
                        )
                        .with_submodule(true),
                    ],
                }
                .into(),
            );
            repo.status = Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                ..AppState::test_default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
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
fn commit_file_menu_copy_path_requires_a_completed_right_click(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                ..AppState::test_default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
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

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("initial".into()),
        "a release without an entry press must not copy"
    );
    cx.simulate_mouse_down(
        copy_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        copy_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );

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
fn status_file_menu_copy_path_requires_a_completed_right_click(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
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

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("initial".into()),
        "a release without an entry press must not copy"
    );
    cx.simulate_mouse_down(
        copy_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        copy_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );

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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    large_file: None,
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
    file_browser_menu_model(cx, "src", true, |state| configure(&mut state.repos[0]))
}

/// The file-browser menu for `path` (`""` is the listing's root) over the
/// `src/` + `src/a.rs` fixture; `configure` adjusts the state before it is built.
fn file_browser_menu_model(
    cx: &mut gpui::TestAppContext,
    path: &str,
    is_dir: bool,
    configure: impl FnOnce(&mut AppState),
) -> ContextMenuModel {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    ignored: false,
                },
                gitcomet_core::domain::FileEntry {
                    name: "a.rs".to_string(),
                    path: Arc::new(std::path::PathBuf::from("src/a.rs")),
                    kind: gitcomet_core::domain::FileEntryKind::File,
                    depth: 1,
                    ignored: false,
                },
            ]));

            let mut state = AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
            };
            configure(&mut state);
            let state = Arc::new(state);
            this.state = Arc::clone(&state);
            this.ui_model
                .update(cx, |model, cx| model.set_state(state, cx));
            cx.notify();
        });
    });

    let path = std::path::PathBuf::from(path);
    let kind = if is_dir {
        PopoverKind::FileBrowserFolderMenu { repo_id, path }
    } else {
        PopoverKind::FileBrowserFileMenu { repo_id, path }
    };
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, cx| host.context_menu_model(&kind, cx))
        })
        .expect("expected a file browser context menu model")
    })
}

/// Entry labels in menu order, with `—` for each separator.
fn menu_outline(model: &ContextMenuModel) -> Vec<String> {
    model
        .items
        .iter()
        .filter_map(|item| match item {
            ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
            ContextMenuItem::Separator => Some("—".to_string()),
            _ => None,
        })
        .collect()
}

#[gpui::test]
fn file_browser_file_menu_groups_working_tree_actions_in_order(cx: &mut gpui::TestAppContext) {
    let model = file_browser_menu_model(cx, "src/a.rs", false, |_state| {});

    assert_eq!(
        menu_outline(&model),
        [
            "Open",
            "Edit file",
            "—",
            "New file",
            "New folder",
            "—",
            "Cut",
            "Copy",
            "Paste",
            "Duplicate",
            "—",
            "Rename…",
            "Add to .gitignore",
            "—",
            "Open file",
            "Open file location",
            "File history",
            "Copy absolute path",
            "Copy relative path",
            "—",
            "Trash",
            "Delete permanently…",
            "—",
            "Undo",
            "Redo",
        ]
    );
}

#[gpui::test]
fn file_browser_folder_menu_groups_working_tree_actions_in_order(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    crate::external_editor::set_configured_setting_override(None);
    let model = file_browser_menu_model(cx, "src", true, |_state| {});

    assert_eq!(
        menu_outline(&model),
        [
            "Expand",
            "Expand all under here",
            "Collapse all under here",
            "—",
            "New file",
            "New folder",
            "—",
            "Cut",
            "Copy",
            "Paste",
            "Duplicate",
            "—",
            "Rename…",
            "Add to .gitignore",
            "—",
            "Open folder location",
            "Copy absolute path",
            "Copy relative path",
            "—",
            "Trash",
            "Delete permanently…",
            "—",
            "Undo",
            "Redo",
        ]
    );
}

/// The root (empty space below the rows) can only receive new items and
/// pastes; its tree toggles and relative path would do nothing.
#[gpui::test]
fn file_browser_root_menu_offers_create_paste_locations_and_history_only(
    cx: &mut gpui::TestAppContext,
) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    crate::external_editor::set_configured_setting_override(None);
    let model = file_browser_menu_model(cx, "", true, |_state| {});

    assert_eq!(
        menu_outline(&model),
        [
            "New file",
            "New folder",
            "—",
            "Paste",
            "—",
            "Open folder location",
            "Copy absolute path",
            "—",
            "Undo",
            "Redo",
        ]
    );
}

#[gpui::test]
fn file_browser_file_menu_for_a_commit_source_has_no_explorer_groups(
    cx: &mut gpui::TestAppContext,
) {
    let model = file_browser_menu_model(cx, "src/a.rs", false, |state| {
        state.repos[0].file_browser.source =
            gitcomet_core::domain::FileSource::Commit(CommitId("abc123".into()));
    });

    assert_eq!(
        menu_outline(&model),
        [
            "Open",
            "Edit file",
            "—",
            "File history",
            "Copy absolute path",
            "Copy relative path",
        ]
    );
}

#[gpui::test]
fn explorer_entries_carry_icons_and_platform_shortcuts(cx: &mut gpui::TestAppContext) {
    use crate::view::shortcut_labels::{Shortcut, secondary_shortcut};

    let model = file_browser_menu_model(cx, "src/a.rs", false, |_state| {});
    let entry = |label: &str| {
        model
            .items
            .iter()
            .find_map(|item| match item {
                ContextMenuItem::Entry {
                    label: entry_label,
                    icon,
                    shortcut,
                    ..
                } if entry_label.as_ref() == label => Some((icon.clone(), shortcut.clone())),
                _ => None,
            })
            .unwrap_or_else(|| panic!("expected `{label}` context menu entry"))
    };
    let redo = Shortcut::Platform {
        macos: "Cmd+Shift+Z",
        other: "Ctrl+Y",
    }
    .label();
    let expected: [(&str, &str, Option<String>); 13] = [
        ("New file", "icons/file_plus.svg", None),
        ("New folder", "icons/folder_plus.svg", None),
        ("Cut", "icons/scissors.svg", Some(secondary_shortcut("X"))),
        ("Copy", "icons/copy.svg", Some(secondary_shortcut("C"))),
        (
            "Paste",
            "icons/clipboard_paste.svg",
            Some(secondary_shortcut("V")),
        ),
        (
            "Duplicate",
            "icons/copy_plus.svg",
            Some(secondary_shortcut("D")),
        ),
        ("Rename…", "icons/pencil.svg", Some("F2".to_string())),
        ("Add to .gitignore", "icons/eye_off.svg", None),
        ("Trash", "icons/trash.svg", Some("Delete".to_string())),
        (
            "Delete permanently…",
            "icons/trash.svg",
            Some("Shift+Delete".to_string()),
        ),
        ("Undo", "icons/undo.svg", Some(secondary_shortcut("Z"))),
        ("Redo", "icons/redo.svg", redo),
        ("File history", "icons/refresh.svg", None),
    ];
    for (label, icon, shortcut) in expected {
        let (actual_icon, actual_shortcut) = entry(label);
        assert_eq!(actual_icon.as_deref(), Some(icon), "{label} icon");
        assert_eq!(
            actual_shortcut.as_ref().map(|s| s.to_string()),
            shortcut,
            "{label} shortcut"
        );
    }

    // The menu's single-key mnemonic is the shortcut's last part, so `c` copies
    // the row here rather than its relative path, and Undo keeps `z`.
    let ix_of = |label: &str| {
        model.items.iter().position(|item| {
            matches!(item, ContextMenuItem::Entry { label: entry_label, .. }
                if entry_label.as_ref() == label)
        })
    };
    assert_eq!(context_menu_shortcut_entry_ix(&model, "c"), ix_of("Copy"));
    assert_eq!(context_menu_shortcut_entry_ix(&model, "x"), ix_of("Cut"));
}

#[gpui::test]
fn explorer_undo_redo_disabled_states_follow_filesystem_availability(
    cx: &mut gpui::TestAppContext,
) {
    let unavailable = file_browser_menu_model(cx, "src/a.rs", false, |_state| {});
    assert!(context_menu_entry_disabled(&unavailable, "Undo"));
    assert!(context_menu_entry_disabled(&unavailable, "Redo"));

    let available = file_browser_menu_model(cx, "src/a.rs", false, |state| {
        state.filesystem.undo_available = true;
        state.filesystem.redo_available = true;
    });
    assert!(!context_menu_entry_disabled(&available, "Undo"));
    assert!(!context_menu_entry_disabled(&available, "Redo"));
}

/// Rename acts on one item; a multi-selection disables it rather than hiding it.
#[gpui::test]
fn explorer_rename_is_disabled_for_a_multi_selection(cx: &mut gpui::TestAppContext) {
    let model = file_browser_menu_model(cx, "src/a.rs", false, |state| {
        let selection = &mut state.repos[0].file_browser.selection.paths;
        selection.insert(std::path::PathBuf::from("src/a.rs"));
        selection.insert(std::path::PathBuf::from("src"));
    });

    assert!(context_menu_entry_disabled(&model, "Rename…"));
    assert!(!context_menu_entry_disabled(&model, "Cut"));
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

    assert!(matches!(
        model.items.first(),
        Some(ContextMenuItem::Entry { .. })
    ));
    assert!(
        !model
            .items
            .iter()
            .any(|item| matches!(item, ContextMenuItem::Header(_) | ContextMenuItem::Label(_)))
    );

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

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..AppState::test_default()
            });
            this.state = Arc::clone(&state);
            this.ui_model
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

fn lfs_status_menu_labels(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    repo_id: RepoId,
    path: &std::path::Path,
    configure: impl FnOnce(&mut RepoState, &mut AppState),
) -> Vec<(String, bool)> {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_lfs_status_menu_{}",
        std::process::id(),
        repo_id.0
    ));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.to_path_buf(),
                gitcomet_core::domain::FileStatusKind::Modified,
                DiffArea::Unstaged,
            );
            let mut state = (*app_state_with_repo(repo.clone(), repo_id)).clone();
            let mut repo = state.repos.remove(0);
            configure(&mut repo, &mut state);
            state.repos.insert(0, repo);
            push_test_state(this, Arc::new(state), cx);
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
                            path: path.to_path_buf(),
                        },
                        cx,
                    )
                })
            })
            .expect("status file menu");
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry {
                    label, disabled, ..
                } => Some((label.to_string(), *disabled)),
                _ => None,
            })
            .collect()
    })
}

fn lfs_support(lockable: bool) -> gitcomet_core::large_files::LargeFileSupport {
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.lfs.filter_configured = true;
    support.lfs.has_lockable_patterns = lockable;
    support
        .lfs
        .tracked_patterns
        .push(gitcomet_core::large_files::LfsTrackedPattern {
            pattern: "*.psd".into(),
            source: std::path::PathBuf::from(".gitattributes"),
        });
    support
}

fn lfs_row(missing: bool, lockable: bool) -> gitcomet_core::large_files::UncommittedLargeFiles {
    let mut files = gitcomet_core::large_files::UncommittedLargeFiles::default();
    files.unstaged.insert(
        std::path::PathBuf::from("art/hero.psd"),
        gitcomet_core::large_files::LargeFileState {
            pointer: gitcomet_core::large_files::LargeFilePointer::Lfs(
                gitcomet_core::lfs::LfsPointer {
                    oid: gitcomet_core::lfs::LfsOid([4; 32]),
                    size: 10,
                },
            ),
            in_local_store: Some(!missing),
            worktree: Some(if missing {
                gitcomet_core::large_files::LargeFileWorktree::Pointer
            } else {
                gitcomet_core::large_files::LargeFileWorktree::Content
            }),
            lockable,
        },
    );
    files
}

/// Row entries follow the row: download for missing content, lock for
/// lockable files, track for plain files; nothing in a repo without LFS.
#[gpui::test]
fn status_file_menu_offers_git_lfs_entries_by_row_state(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let psd = std::path::Path::new("art/hero.psd");
    let has =
        |labels: &[(String, bool)], wanted: &str| labels.iter().any(|(label, _)| label == wanted);

    let plain_repo = lfs_status_menu_labels(cx, &view, RepoId(81), psd, |_, _| {});
    assert!(
        !plain_repo.iter().any(|(label, _)| label.contains("LFS")),
        "{plain_repo:?}"
    );

    let missing = lfs_status_menu_labels(cx, &view, RepoId(82), psd, |repo, _| {
        repo.large_file_support = Loadable::Ready(Arc::new(lfs_support(true)));
        repo.uncommitted_large_files = Arc::new(lfs_row(true, true));
    });
    assert!(has(&missing, "Download LFS content"), "{missing:?}");
    assert!(has(&missing, "Lock file"), "{missing:?}");

    let locked = lfs_status_menu_labels(cx, &view, RepoId(83), psd, |repo, _| {
        repo.large_file_support = Loadable::Ready(Arc::new(lfs_support(true)));
        repo.uncommitted_large_files = Arc::new(lfs_row(false, true));
        repo.lfs_locks = Loadable::Ready(Arc::new(vec![gitcomet_core::large_files::LfsLock {
            id: "1".into(),
            path: psd.to_path_buf(),
            owner: Some("alice".into()),
            locked_at: None,
        }]));
    });
    assert!(has(&locked, "Unlock (locked by alice)"), "{locked:?}");
    assert!(!has(&locked, "Download LFS content"));

    let untracked_type = lfs_status_menu_labels(
        cx,
        &view,
        RepoId(84),
        std::path::Path::new("scene.blend"),
        |repo, state| {
            repo.large_file_support = Loadable::Ready(Arc::new(lfs_support(false)));
            state.large_file_tools.git_lfs =
                gitcomet_core::large_file_tools::ToolAvailability::NotFound {
                    detail: "Git cannot run `git lfs`.".into(),
                };
        },
    );
    assert!(
        untracked_type.iter().any(|(label, disabled)| label
            == "Track *.blend in Git LFS (install git-lfs)"
            && *disabled),
        "missing tool lists the entry disabled: {untracked_type:?}"
    );
}

fn annex_support(initialized: bool) -> gitcomet_core::large_files::LargeFileSupport {
    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.has_annex_dir = true;
    support.annex.uuid = initialized.then(|| "u-here".to_string());
    support.annex.repositories = vec![
        gitcomet_core::large_files::AnnexRepository {
            uuid: "u-here".into(),
            description: "laptop".into(),
            remote_name: None,
            special_type: None,
            special_name: None,
            trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
            here: true,
        },
        gitcomet_core::large_files::AnnexRepository {
            uuid: "u-backup".into(),
            description: String::new(),
            remote_name: Some("backup".into()),
            special_type: Some("directory".into()),
            special_name: None,
            trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
            here: false,
        },
        gitcomet_core::large_files::AnnexRepository {
            uuid: "u-nas".into(),
            // Another clone's special remote, not enabled here.
            description: "nas".into(),
            remote_name: None,
            special_type: Some("rsync".into()),
            special_name: Some("nas".into()),
            trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
            here: false,
        },
    ];
    support
}

#[gpui::test]
fn review_annex_sync_menu_disables_only_for_a_running_pull_or_push(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(991);
    for (pull, worktree_pull, push, disabled) in
        [(1, 0, 0, false), (1, 1, 0, true), (0, 0, 1, true)]
    {
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &std::env::temp_dir());
                repo.large_file_support = Loadable::Ready(Arc::new(annex_support(true)));
                repo.pull_in_flight = pull;
                repo.worktree_pull_in_flight = worktree_pull;
                repo.push_in_flight = push;
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
        cx.update(|_, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    let model = host
                        .context_menu_model(
                            &PopoverKind::annex(repo_id, AnnexPopoverKind::SectionMenu),
                            cx,
                        )
                        .unwrap();
                    assert_eq!(
                        context_menu_entry_disabled(&model, "Sync with remotes"),
                        disabled
                    );
                });
            });
        });
    }
}

#[gpui::test]
fn review_adjusted_branch_menus_disable_merge_and_squash(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(992);
    let commit_id = CommitId("deadbeefdeadbeef".into());
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = commit_menu_test_repo(repo_id, &commit_id);
            repo.head_branch = Loadable::Ready("adjusted/main(unlocked)".into());
            repo.large_file_support = Loadable::Ready(Arc::new(annex_support(true)));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                for target in [
                    BranchMenuTarget::local("feature".to_string()),
                    BranchMenuTarget::remote("origin".to_string(), "feature".to_string()),
                ] {
                    let model = host
                        .context_menu_model(&PopoverKind::BranchMenu { repo_id, target }, cx)
                        .unwrap();
                    assert!(context_menu_entry_disabled(&model, "Merge into current"));
                    assert!(context_menu_entry_disabled(&model, "Squash into current"));
                }
                let model = host
                    .context_menu_model(
                        &PopoverKind::CommitMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                        cx,
                    )
                    .unwrap();
                assert!(context_menu_entry_disabled(
                    &model,
                    "Merge deadbeef into adjusted/main(unlocked)"
                ));
            });
        });
    });
}

#[gpui::test]
fn review_annex_repository_menu_shows_description_with_local_remote_name(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(993);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &std::env::temp_dir());
            let mut support = annex_support(true);
            support.annex.repositories[1].description = "Archive".into();
            repo.large_file_support = Loadable::Ready(Arc::new(support));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let model = host.context_menu_model(&PopoverKind::annex(repo_id, AnnexPopoverKind::RepositoryMenu { uuid: "u-backup".into() }), cx).unwrap();
                assert!(matches!(&model.items[0], ContextMenuItem::Header(label) if label.as_ref() == "Archive [backup]"));
            });
        });
    });
}

#[gpui::test]
fn review_hunk_menu_uses_rendered_large_file_target(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::large_files::{LargeFileContent, LargeFilePointer, LargeFileSide};
    use gitcomet_state::model::{ForeignDiffOrigin, InlineSubmoduleDiffState};
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(994);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &std::env::temp_dir());
            let target = DiffTarget::working_tree("asset.txt".into(), DiffArea::Unstaged);
            let diff = Arc::new(gitcomet_core::domain::Diff::from_unified(target.clone(), "diff --git a/asset.txt b/asset.txt\n--- a/asset.txt\n+++ b/asset.txt\n@@ -1 +1 @@\n-before\n+after\n"));
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff = Loadable::Ready(diff.clone());
            repo.diff_state.diff_file = Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new("asset.txt".into(), Some("before\n".into()), Some("after\n".into())))));
            let large = LargeFileSide {
                pointer: LargeFilePointer::Lfs(gitcomet_core::lfs::LfsPointer { oid: gitcomet_core::lfs::LfsOid([9; 32]), size: 12 }),
                content: LargeFileContent::Available,
            };
            repo.diff_state.inline_submodule_diff = Some(InlineSubmoduleDiffState {
                origin: ForeignDiffOrigin::Submodule, submodule_repo_path: "/tmp/sub".into(), parent_submodule_path: "sub".into(),
                entries: Arc::from([]), selected_ix: 0, target, rev: 1, diff_rev: 1,
                diff: Loadable::Ready(diff), diff_file_rev: 1,
                diff_file: Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new("asset.txt".into(), Some("before\n".into()), Some("after\n".into())).with_large_sides(Some(large.clone()), Some(large))))),
                diff_file_image: Loadable::NotLoaded,
            });
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            assert!(this.main_pane.read(cx).has_large_file_text_diff());
            this.popover_host.update(cx, |host, cx| {
                let model = host
                    .context_menu_model(&PopoverKind::DiffHunkMenu { repo_id, src_ix: 3 }, cx)
                    .unwrap();
                assert!(context_menu_entry_disabled(&model, "Stage hunk"));
            });
        });
    });
}

fn annex_row(present: Option<bool>) -> gitcomet_core::large_files::UncommittedLargeFiles {
    let mut files = gitcomet_core::large_files::UncommittedLargeFiles::default();
    files.unstaged.insert(
        std::path::PathBuf::from("art/hero.psd"),
        gitcomet_core::large_files::LargeFileState {
            pointer: gitcomet_core::large_files::LargeFilePointer::Annex(
                gitcomet_core::annex::parse_key("SHA256E-s10--abc.psd").unwrap(),
            ),
            in_local_store: present,
            worktree: None,
            lockable: false,
        },
    );
    files
}

#[gpui::test]
fn status_file_menu_offers_git_annex_entries_by_presence(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let psd = std::path::Path::new("art/hero.psd");
    let has = |labels: &[(String, bool)], wanted: &str| labels.iter().any(|(l, _)| l == wanted);

    let absent = lfs_status_menu_labels(cx, &view, RepoId(91), psd, |repo, _| {
        repo.large_file_support = Loadable::Ready(Arc::new(annex_support(true)));
        repo.uncommitted_large_files = Arc::new(annex_row(Some(false)));
    });
    assert!(has(&absent, "Get content"), "{absent:?}");
    assert!(!has(&absent, "Drop local content"), "{absent:?}");
    assert!(has(&absent, "Copy to backup") && has(&absent, "Move to backup"));
    assert!(
        !has(&absent, "Copy to nas"),
        "unreachable remotes are not targets"
    );

    let present = lfs_status_menu_labels(cx, &view, RepoId(92), psd, |repo, _| {
        repo.large_file_support = Loadable::Ready(Arc::new(annex_support(true)));
        repo.uncommitted_large_files = Arc::new(annex_row(Some(true)));
    });
    assert!(has(&present, "Drop local content") && !has(&present, "Get content"));
    assert!(has(&present, "Drop even without other copies…"));

    let missing_tool = lfs_status_menu_labels(cx, &view, RepoId(93), psd, |repo, state| {
        repo.large_file_support = Loadable::Ready(Arc::new(annex_support(true)));
        repo.uncommitted_large_files = Arc::new(annex_row(None));
        state.large_file_tools.git_annex =
            gitcomet_core::large_file_tools::ToolAvailability::NotFound {
                detail: "Git cannot run `git annex`.".into(),
            };
    });
    assert!(
        missing_tool
            .iter()
            .any(|(label, disabled)| label == "Get content (install git-annex)" && *disabled),
        "{missing_tool:?}"
    );
}

fn annex_menu_labels(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    repo_id: RepoId,
    support: gitcomet_core::large_files::LargeFileSupport,
    kind: AnnexPopoverKind,
) -> Vec<String> {
    annex_menu_labels_on(cx, view, repo_id, support, kind, "main")
}

fn annex_menu_labels_on(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    repo_id: RepoId,
    support: gitcomet_core::large_files::LargeFileSupport,
    kind: AnnexPopoverKind,
    head: &str,
) -> Vec<String> {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_annex_menu_{}",
        std::process::id(),
        repo_id.0
    ));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.large_file_support = Loadable::Ready(Arc::new(support));
            repo.head_branch = Loadable::Ready(head.to_string());
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(&PopoverKind::annex(repo_id, kind), cx)
                })
            })
            .expect("annex menu");
        model
            .items
            .iter()
            .filter_map(|item| match item {
                ContextMenuItem::Entry { label, .. } => Some(label.to_string()),
                _ => None,
            })
            .collect()
    })
}

#[gpui::test]
fn annex_section_and_repository_menus_follow_repo_state(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let fresh = annex_menu_labels(
        cx,
        &view,
        RepoId(101),
        annex_support(false),
        AnnexPopoverKind::SectionMenu,
    );
    assert_eq!(fresh, ["Initialize git-annex in this clone"]);

    let section = annex_menu_labels(
        cx,
        &view,
        RepoId(102),
        annex_support(true),
        AnnexPopoverKind::SectionMenu,
    );
    for wanted in [
        "Sync with remotes",
        "Adjusted branch: unlock all files",
        "Add special remote…",
        "Set number of copies…",
    ] {
        assert!(
            section.iter().any(|l| l == wanted),
            "{wanted} in {section:?}"
        );
    }
    for wanted in ["Find unused content…", "Open git-annex webapp…"] {
        assert!(
            section.iter().any(|l| l == wanted),
            "{wanted} in {section:?}"
        );
    }
    assert!(
        !section.iter().any(|l| l == "Stop git-annex assistant"),
        "no assistant runs: {section:?}"
    );
    let mut running = annex_support(true);
    running.annex.assistant_running = true;
    let running = annex_menu_labels(
        cx,
        &view,
        RepoId(106),
        running,
        AnnexPopoverKind::SectionMenu,
    );
    assert!(running.iter().any(|l| l == "Stop git-annex assistant"));
    let adjusted = annex_menu_labels_on(
        cx,
        &view,
        RepoId(103),
        annex_support(true),
        AnnexPopoverKind::SectionMenu,
        "adjusted/main(unlocked)",
    );
    assert!(
        adjusted
            .iter()
            .any(|l| l == "Leave adjusted branch (back to main)")
    );

    let backup = annex_menu_labels(
        cx,
        &view,
        RepoId(104),
        annex_support(true),
        AnnexPopoverKind::RepositoryMenu {
            uuid: "u-backup".into(),
        },
    );
    assert!(
        backup.iter().any(|l| l == "Copy all content to backup"),
        "{backup:?}"
    );
    assert!(backup.iter().any(|l| l == "Mark as untrusted"));
    assert!(
        !backup.iter().any(|l| l == "Mark as semitrusted"),
        "current level is omitted"
    );

    let nas = annex_menu_labels(
        cx,
        &view,
        RepoId(105),
        annex_support(true),
        AnnexPopoverKind::RepositoryMenu {
            uuid: "u-nas".into(),
        },
    );
    assert!(
        nas.iter().any(|l| l == "Enable nas in this clone…"),
        "{nas:?}"
    );
}

#[gpui::test]
fn annex_text_prompts_submit_on_enter(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    for (prompt, text) in [
        (
            AnnexPrompt::AddSpecialRemote,
            "backup directory directory=/tmp/backup encryption=none",
        ),
        (
            AnnexPrompt::EnableSpecialRemote {
                name: "backup".into(),
            },
            "backup directory=/tmp/backup",
        ),
        (
            AnnexPrompt::Describe {
                repository: "here".into(),
                current: String::new(),
            },
            "laptop",
        ),
        (AnnexPrompt::Numcopies { current: Some(1) }, "2"),
    ] {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        cx.update(|window, app| {
            crate::app::bind_text_input_keys_for_test(app);
            view.update(app, |view, cx| {
                view.open_popover_at(
                    PopoverKind::annex(RepoId(1), AnnexPopoverKind::Prompt(prompt)),
                    point(px(72.0), px(72.0)),
                    window,
                    cx,
                );
                view.popover_host.update(cx, |host, cx| {
                    host.submodule_ref_input
                        .update(cx, |input, cx| input.set_text(text, cx));
                });
            });
        });
        cx.run_until_parked();
        crate::test_support::refresh_and_draw(cx);
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
        cx.update(|_, app| {
            assert!(
                !view.read(app).popover_host.read(app).is_open(),
                "Enter should submit {text}"
            )
        });
    }
}

#[gpui::test]
fn annex_unused_prompt_drops_only_a_loaded_listing(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(1);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_annex_unused",
        std::process::id()
    ));
    let push = |cx: &mut gpui::VisualTestContext,
                unused: Loadable<Arc<gitcomet_core::large_files::AnnexUnused>>| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                repo.large_file_support = Loadable::Ready(Arc::new(annex_support(true)));
                repo.annex_unused = unused;
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
    };
    let prompt = PopoverKind::annex(repo_id, AnnexPopoverKind::Prompt(AnnexPrompt::Unused));
    let open_and_submit = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.open_popover_at(
                        prompt.clone(),
                        gpui::point(gpui::px(0.0), gpui::px(0.0)),
                        window,
                        cx,
                    );
                    host.submit_annex_prompt(window, cx);
                });
            });
        });
        cx.update(|_window, app| test_support::popover_kind(view.read(app), app))
    };

    push(cx, Loadable::Loading);
    assert_eq!(
        open_and_submit(cx),
        Some(prompt.clone()),
        "Enter must not drop while the listing loads"
    );
    push(
        cx,
        Loadable::Ready(Arc::new(gitcomet_core::large_files::AnnexUnused::default())),
    );
    assert_eq!(open_and_submit(cx), Some(prompt.clone()), "nothing to drop");

    push(
        cx,
        Loadable::Ready(Arc::new(gitcomet_core::large_files::AnnexUnused {
            entries: vec![gitcomet_core::large_files::AnnexUnusedEntry {
                number: 1,
                key: "SHA256E-s4--old.bin".into(),
                kind: gitcomet_core::large_files::AnnexUnusedKind::Unused,
            }],
        })),
    );
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    prompt.clone(),
                    gpui::point(gpui::px(0.0), gpui::px(0.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.run_until_parked();
    assert!(
        cx.debug_bounds("annex_prompt_force_unused").is_some(),
        "a listing offers the forced drop too"
    );
    assert_eq!(open_and_submit(cx), None, "a listed item drops and closes");
}
