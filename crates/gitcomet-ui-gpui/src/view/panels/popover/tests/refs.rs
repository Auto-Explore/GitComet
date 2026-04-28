use super::branch::{create_tracking_store, wait_until};
use super::*;

fn click_debug_selector(cx: &mut gpui::VisualTestContext, selector: &'static str) {
    let center = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected {selector} in debug bounds"))
        .center();
    cx.simulate_mouse_move(center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn tag_menu_lists_delete_entries_for_commit_tags(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(2);
    let commit_id = CommitId("0123456789abcdef".into());
    let other_commit = CommitId("aaaaaaaaaaaaaaaa".into());
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_tag_menu", std::process::id()));

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
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "Hello".into(),
                        author: "Alice".into(),
                        time: SystemTime::UNIX_EPOCH,
                    }],
                    next_cursor: None,
                }
                .into(),
            );
            repo.tags = Loadable::Ready(Arc::new(vec![
                gitcomet_core::domain::Tag {
                    name: "release".to_string(),
                    target: commit_id.clone(),
                },
                gitcomet_core::domain::Tag {
                    name: "v1.0.0".to_string(),
                    target: commit_id.clone(),
                },
                gitcomet_core::domain::Tag {
                    name: "other".to_string(),
                    target: other_commit,
                },
            ]));

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
                        &PopoverKind::TagMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected tag context menu model");

        for name in ["release", "v1.0.0"] {
            let expected_label = format!("Delete tag {name}");
            let delete_action = model.items.iter().find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == expected_label.as_str() =>
                {
                    Some((**action).clone())
                }
                _ => None,
            });
            match delete_action {
                Some(ContextMenuAction::DeleteTag {
                    repo_id: rid,
                    name: n,
                }) => {
                    assert_eq!(rid, repo_id);
                    assert_eq!(n, name);
                }
                _ => panic!("expected Delete tag {name} action"),
            }
        }

        let has_other = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Delete tag other",
            _ => false,
        });
        assert!(
            !has_other,
            "tag menu should only show tags on the clicked commit"
        );
    });
}

#[gpui::test]
fn tag_menu_lists_remote_push_and_delete_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(20);
    let commit_id = CommitId("fedcba9876543210".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_tag_menu_remote",
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
            repo.tags = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Tag {
                name: "v2.0.0".to_string(),
                target: commit_id.clone(),
            }]));
            repo.remotes = Loadable::Ready(Arc::new(vec![
                gitcomet_core::domain::Remote {
                    name: "upstream".to_string(),
                    url: None,
                },
                gitcomet_core::domain::Remote {
                    name: "origin".to_string(),
                    url: None,
                },
            ]));
            repo.remote_tags = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::RemoteTag {
                remote: "origin".to_string(),
                name: "v2.0.0".to_string(),
                target: commit_id.clone(),
            }]));

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
                        &PopoverKind::TagMenu {
                            repo_id,
                            commit_id: commit_id.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected tag context menu model");

        for remote in ["origin", "upstream"] {
            let push_label = format!("Push tag v2.0.0 to {remote}");
            let push_action = model.items.iter().find_map(|item| match item {
                ContextMenuItem::Entry { label, action, .. }
                    if label.as_ref() == push_label.as_str() =>
                {
                    Some((**action).clone())
                }
                _ => None,
            });
            match push_action {
                Some(ContextMenuAction::PushTag {
                    repo_id: rid,
                    remote: r,
                    name,
                }) => {
                    assert_eq!(rid, repo_id);
                    assert_eq!(r, remote);
                    assert_eq!(name, "v2.0.0");
                }
                _ => panic!("expected Push tag action for remote {remote}"),
            }
        }

        let delete_label = "Delete tag v2.0.0 from origin";
        let delete_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == delete_label => {
                Some((**action).clone())
            }
            _ => None,
        });
        match delete_action {
            Some(ContextMenuAction::DeleteRemoteTag {
                repo_id: rid,
                remote: r,
                name,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(r, "origin");
                assert_eq!(name, "v2.0.0");
            }
            _ => panic!("expected Delete remote tag action for origin"),
        }

        let has_upstream_delete = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => {
                label.as_ref() == "Delete tag v2.0.0 from upstream"
            }
            _ => false,
        });
        assert!(
            !has_upstream_delete,
            "did not expect delete remote tag action for upstream without tag"
        );
    });
}

#[gpui::test]
fn create_tag_prompt_escape_cancels(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-tag-escape");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        crate::app::bind_text_input_keys_for_test(app);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Escape to close create-tag popover");
    assert!(
        repo.actions().is_empty(),
        "expected Escape to cancel without creating a tag"
    );
}

#[gpui::test]
fn create_tag_prompt_renders_shortcut_hints_and_separators(cx: &mut gpui::TestAppContext) {
    let (store, events, _repo, _workdir) = create_tracking_store("create-tag-shortcuts");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    cx.debug_bounds("create_tag_cancel_hint")
        .expect("expected create-tag Cancel shortcut hint");
    cx.debug_bounds("create_tag_go_hint")
        .expect("expected create-tag Create shortcut hint");
    cx.debug_bounds("create_tag_cancel_end_slot_separator")
        .expect("expected create-tag Cancel shortcut separator");
    cx.debug_bounds("create_tag_go_end_slot_separator")
        .expect("expected create-tag Create shortcut separator");
}

#[gpui::test]
fn create_tag_prompt_cancel_button_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-tag-cancel-click");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    click_debug_selector(cx, "create_tag_cancel_hint");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        !is_open,
        "expected clicking Cancel to close create-tag popover"
    );
    assert!(
        repo.actions().is_empty(),
        "expected clicking Cancel to avoid tag creation"
    );
}

#[gpui::test]
fn create_tag_prompt_create_button_click_creates_and_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-tag-create-click");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.create_tag_input
                    .update(cx, |input, cx| input.set_text("v2.0.0", cx));
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    click_debug_selector(cx, "create_tag_go_hint");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        !is_open,
        "expected clicking Create to close create-tag popover"
    );

    wait_until("create-tag click repo actions", || {
        repo.actions() == vec!["tag:v2.0.0:HEAD".to_string()]
    });
}

#[gpui::test]
fn create_tag_prompt_create_button_click_with_empty_input_does_not_close_or_create(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events, repo, _workdir) = create_tracking_store("create-tag-empty-click");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    click_debug_selector(cx, "create_tag_go_hint");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        is_open,
        "expected clicking disabled Create to keep create-tag popover open"
    );

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        repo.actions().is_empty(),
        "expected clicking disabled Create to avoid tag creation"
    );
}

#[gpui::test]
fn create_tag_prompt_enter_creates_and_closes(cx: &mut gpui::TestAppContext) {
    let (store, events, repo, _workdir) = create_tracking_store("create-tag-enter");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                assert!(
                    !host.can_submit_create_tag(cx),
                    "expected empty create-tag input to disable Create"
                );
                host.create_tag_input
                    .update(cx, |input, cx| input.set_text("v1.0.0", cx));
                assert!(
                    host.can_submit_create_tag(cx),
                    "expected non-empty create-tag input to enable Create"
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(!is_open, "expected Enter to close create-tag popover");

    wait_until("create-tag repo actions", || {
        repo.actions() == vec!["tag:v1.0.0:HEAD".to_string()]
    });
}

#[gpui::test]
fn create_tag_prompt_enter_with_empty_input_does_not_close_or_create(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events, repo, _workdir) = create_tracking_store("create-tag-empty-enter");
    let repo_id = store.snapshot().active_repo.expect("expected active repo");
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    cx.update(|window, app| {
        app.bind_keys([gpui::KeyBinding::new(
            "enter",
            crate::kit::Enter,
            Some("TextInput"),
        )]);
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateTagPrompt {
                        repo_id,
                        target: "HEAD".to_string(),
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

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                assert!(
                    !host.can_submit_create_tag(cx),
                    "expected empty create-tag input to disable Create"
                );
            });
        });
    });

    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let is_open = cx.update(|_window, app| view.read(app).popover_host.read(app).is_open());
    assert!(
        is_open,
        "expected Enter to respect the disabled Create action when the tag name is empty"
    );

    std::thread::sleep(std::time::Duration::from_millis(100));
    assert!(
        repo.actions().is_empty(),
        "expected empty create-tag input to avoid tag creation"
    );
}

#[gpui::test]
fn remote_menu_lists_fetch_and_prune_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(21);
    let remote_name = "origin".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_remote_menu_prune",
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
            repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
                name: remote_name.clone(),
                url: None,
            }]));

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
                        &PopoverKind::remote(
                            repo_id,
                            RemotePopoverKind::Menu {
                                name: remote_name.clone(),
                            },
                        ),
                        cx,
                    )
                })
            })
            .expect("expected remote context menu model");

        let fetch = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Fetch all" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            fetch,
            Some(ContextMenuAction::FetchAll { repo_id: rid }) if rid == repo_id
        ));

        let prune_branches = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Prune merged branches" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            prune_branches,
            Some(ContextMenuAction::PruneMergedBranches { repo_id: rid }) if rid == repo_id
        ));

        let prune_tags = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Prune local tags" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            prune_tags,
            Some(ContextMenuAction::PruneLocalTags { repo_id: rid }) if rid == repo_id
        ));
    });
}

#[gpui::test]
fn local_branch_menu_has_pull_merge_and_squash_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(22);
    let branch_name = "feature/awesome".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_local_branch_menu_merge",
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
            repo.head_branch = Loadable::Ready("main".to_string());

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
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Local,
                            name: branch_name.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected branch context menu model");

        let pull_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Pull into current" => Some(((**action).clone(), *disabled)),
            _ => None,
        });

        match pull_entry {
            Some((
                ContextMenuAction::PullBranch {
                    repo_id: rid,
                    remote,
                    branch,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(remote, ".");
                assert_eq!(branch, branch_name);
                assert!(!disabled);
            }
            _ => panic!("expected Pull into current entry with PullBranch action"),
        }

        let merge_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Merge into current" => Some(((**action).clone(), *disabled)),
            _ => None,
        });

        match merge_entry {
            Some((
                ContextMenuAction::MergeRef {
                    repo_id: rid,
                    reference,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(reference, branch_name);
                assert!(!disabled);
            }
            _ => panic!("expected Merge into current entry with MergeRef action"),
        }

        let squash_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Squash into current" => Some(((**action).clone(), *disabled)),
            _ => None,
        });

        match squash_entry {
            Some((
                ContextMenuAction::SquashRef {
                    repo_id: rid,
                    reference,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(reference, branch_name);
                assert!(!disabled);
            }
            _ => panic!("expected Squash into current entry with SquashRef action"),
        }

        let create_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Create branch" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            create_entry,
            Some(ContextMenuAction::OpenPopover {
                kind: PopoverKind::CreateBranchFromRefPrompt {
                    repo_id: rid,
                    target
                }
            }) if rid == repo_id && target == branch_name
        ));

        let has_pull_into_current = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Pull into current",
            _ => false,
        });
        assert!(has_pull_into_current);
    });
}

#[gpui::test]
fn remote_branch_menu_has_pull_merge_and_squash_actions(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(23);
    let branch_name = "origin/feature/awesome".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_remote_branch_menu_merge",
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
            repo.head_branch = Loadable::Ready("main".to_string());

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
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Remote,
                            name: branch_name.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected branch context menu model");

        let pull_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Pull into current" => Some(((**action).clone(), *disabled)),
            _ => None,
        });

        match pull_entry {
            Some((
                ContextMenuAction::PullBranch {
                    repo_id: rid,
                    remote,
                    branch,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(remote, "origin");
                assert_eq!(branch, "feature/awesome");
                assert!(!disabled);
            }
            _ => panic!("expected Pull into current entry with PullBranch action"),
        }

        let merge_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Merge into current" => Some(((**action).clone(), *disabled)),
            _ => None,
        });

        match merge_entry {
            Some((
                ContextMenuAction::MergeRef {
                    repo_id: rid,
                    reference,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(reference, branch_name);
                assert!(!disabled);
            }
            _ => panic!("expected Merge into current entry with MergeRef action"),
        }

        let squash_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Squash into current" => Some(((**action).clone(), *disabled)),
            _ => None,
        });

        match squash_entry {
            Some((
                ContextMenuAction::SquashRef {
                    repo_id: rid,
                    reference,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(reference, branch_name);
                assert!(!disabled);
            }
            _ => panic!("expected Squash into current entry with SquashRef action"),
        }

        let create_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Create branch" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            create_entry,
            Some(ContextMenuAction::OpenPopover {
                kind: PopoverKind::CreateBranchFromRefPrompt {
                    repo_id: rid,
                    target
                }
            }) if rid == repo_id && target == branch_name
        ));
    });
}

#[gpui::test]
fn remote_branch_menu_renders_squash_entry_without_panic(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(232);
    let branch_name = "origin/feature/awesome".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_remote_branch_menu_render",
        std::process::id()
    ));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.head_branch = Loadable::Ready("main".to_string());

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
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::BranchMenu {
                        repo_id,
                        section: BranchSection::Remote,
                        name: branch_name.clone(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.debug_bounds("context_menu_squash_into_current")
        .expect("expected remote branch squash entry to render");
}

#[gpui::test]
fn remote_branch_menu_only_enables_unlink_for_active_branch_upstream(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(230);
    let enabled_name = "origin/feature/awesome".to_string();
    let disabled_name = "origin/main".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_remote_branch_menu_unlink",
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
            repo.head_branch = Loadable::Ready("feature/local".to_string());
            repo.branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
                name: "feature/local".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: Some(gitcomet_core::domain::Upstream {
                    remote: "origin".to_string(),
                    branch: "feature/awesome".to_string(),
                }),
                divergence: None,
            }]));

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
        let enabled_model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Remote,
                            name: enabled_name.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected enabled remote branch context menu model");

        let enabled_entry = enabled_model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Unlink upstream branch" => {
                Some(((**action).clone(), *disabled))
            }
            _ => None,
        });

        match enabled_entry {
            Some((
                ContextMenuAction::UnsetUpstreamBranch {
                    repo_id: rid,
                    branch,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(branch, "feature/local");
                assert!(!disabled);
            }
            _ => panic!("expected enabled UnsetUpstreamBranch entry"),
        }

        let disabled_model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Remote,
                            name: disabled_name.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected disabled remote branch context menu model");

        let disabled_entry = disabled_model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Unlink upstream branch" => {
                Some(((**action).clone(), *disabled))
            }
            _ => None,
        });

        match disabled_entry {
            Some((
                ContextMenuAction::UnsetUpstreamBranch {
                    repo_id: rid,
                    branch,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(branch, "feature/local");
                assert!(disabled);
            }
            _ => panic!("expected disabled UnsetUpstreamBranch entry"),
        }
    });
}

#[gpui::test]
fn remote_branch_menu_offers_set_tracking_upstream_only_without_current_upstream(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(231);
    let branch_name = "origin/feature/awesome".to_string();
    let branch_name_with_upstream = "origin/main".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_remote_branch_menu_set_tracking_upstream",
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
            repo.head_branch = Loadable::Ready("feature/local".to_string());
            repo.branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
                name: "feature/local".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: None,
                divergence: None,
            }]));

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
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Remote,
                            name: branch_name.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected remote branch context menu model");

        let set_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Set as tracking upstream" => {
                Some(((**action).clone(), *disabled))
            }
            _ => None,
        });

        match set_entry {
            Some((
                ContextMenuAction::SetUpstreamBranch {
                    repo_id: rid,
                    branch,
                    upstream,
                },
                disabled,
            )) => {
                assert_eq!(rid, repo_id);
                assert_eq!(branch, "feature/local");
                assert_eq!(upstream, "origin/feature/awesome");
                assert!(!disabled);
            }
            _ => panic!("expected enabled SetUpstreamBranch entry"),
        }

        let unlink_entry = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry {
                label, disabled, ..
            } if label.as_ref() == "Unlink upstream branch" => Some(*disabled),
            _ => None,
        });
        assert_eq!(unlink_entry, None);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = RepoState::new_opening(
                repo_id,
                gitcomet_core::domain::RepoSpec {
                    workdir: workdir.clone(),
                },
            );
            repo.head_branch = Loadable::Ready("feature/local".to_string());
            repo.branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
                name: "feature/local".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: Some(gitcomet_core::domain::Upstream {
                    remote: "origin".to_string(),
                    branch: "main".to_string(),
                }),
                divergence: None,
            }]));
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
        let model_with_upstream = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Remote,
                            name: branch_name_with_upstream.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected remote branch context menu model after upstream update");

        let has_set_entry = model_with_upstream.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Set as tracking upstream",
            _ => false,
        });
        assert!(
            !has_set_entry,
            "did not expect Set as tracking upstream when current branch already tracks a remote"
        );
    });
}

#[gpui::test]
fn pull_and_push_picker_headers_include_tracking_branch_name(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(401);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_pull_push_picker_header_branch_name",
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
            repo.head_branch = Loadable::Ready("feature/local".to_string());
            repo.branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
                name: "feature/local".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: Some(gitcomet_core::domain::Upstream {
                    remote: "origin".to_string(),
                    branch: "feature/awesome".to_string(),
                }),
                divergence: None,
            }]));

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
        let pull_model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(&PopoverKind::PullPicker, cx)
                })
            })
            .expect("expected pull context menu model");
        let push_model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(&PopoverKind::PushPicker, cx)
                })
            })
            .expect("expected push context menu model");

        assert!(matches!(
            pull_model.items.first(),
            Some(ContextMenuItem::Header(title))
                if title.as_ref() == "Pull origin/feature/awesome"
        ));
        assert!(matches!(
            push_model.items.first(),
            Some(ContextMenuItem::Header(title))
                if title.as_ref() == "Push origin/feature/awesome"
        ));
    });
}

#[gpui::test]
fn local_branch_menu_excludes_pull_merge_and_squash_for_current_branch(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(24);
    let branch_name = "main".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_local_branch_menu_current_branch",
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
            repo.head_branch = Loadable::Ready(branch_name.clone());

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
                        &PopoverKind::BranchMenu {
                            repo_id,
                            section: BranchSection::Local,
                            name: branch_name.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected branch context menu model");

        let has_merge = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Merge into current",
            _ => false,
        });
        let has_pull = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Pull into current",
            _ => false,
        });
        let has_squash = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Squash into current",
            _ => false,
        });

        let delete_disabled = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry {
                label,
                action,
                disabled,
                ..
            } if label.as_ref() == "Delete branch" => {
                *disabled
                    && matches!(
                        action.as_ref(),
                        ContextMenuAction::DeleteBranch { repo_id: rid, name }
                            if *rid == repo_id && name == &branch_name
                    )
            }
            _ => false,
        });

        assert!(!has_pull, "expected pull entry to be excluded");
        assert!(!has_merge, "expected merge entry to be excluded");
        assert!(!has_squash, "expected squash entry to be excluded");
        assert!(delete_disabled, "expected delete entry to be disabled");
    });
}
