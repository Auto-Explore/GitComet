use super::*;

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
                        parent_ids: vec![],
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
