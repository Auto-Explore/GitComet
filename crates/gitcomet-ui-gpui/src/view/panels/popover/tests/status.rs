use super::*;

#[gpui::test]
fn status_file_menu_uses_multi_selection_for_stage(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(3);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu",
        std::process::id()
    ));

    let a = std::path::PathBuf::from("a.txt");
    let b = std::path::PathBuf::from("b.txt");

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
                    unstaged: vec![
                        gitcomet_core::domain::FileStatus {
                            path: a.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: b.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ],
                }
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        untracked: vec![],
                        untracked_anchor: None,
                        unstaged: vec![a.clone(), b.clone()],
                        unstaged_anchor: Some(a.clone()),
                        unstaged_anchor_index: None,
                        unstaged_anchor_status_rev: None,
                        staged: vec![],
                        staged_anchor: None,
                        staged_anchor_index: None,
                        staged_anchor_status_rev: None,
                    },
                );
                cx.notify();
            });
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
                            path: a.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected status file context menu model");

        let stage_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Stage (2)" => {
                Some((**action).clone())
            }
            _ => None,
        });

        match stage_action {
            Some(ContextMenuAction::StageSelectionOrPath {
                repo_id: rid,
                area,
                path,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(area, DiffArea::Unstaged);
                assert_eq!(path, a);
            }
            _ => panic!("expected Stage (2) to stage selected paths"),
        }
    });
}

#[gpui::test]
fn status_file_menu_uses_multi_selection_for_unstage(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(4);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_staged",
        std::process::id()
    ));

    let a = std::path::PathBuf::from("a.txt");
    let b = std::path::PathBuf::from("b.txt");

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
                    staged: vec![
                        gitcomet_core::domain::FileStatus {
                            path: a.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: b.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ],
                    unstaged: vec![],
                }
                .into(),
            );

            this.state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        untracked: vec![],
                        untracked_anchor: None,
                        unstaged: vec![],
                        unstaged_anchor: None,
                        unstaged_anchor_index: None,
                        unstaged_anchor_status_rev: None,
                        staged: vec![a.clone(), b.clone()],
                        staged_anchor: Some(a.clone()),
                        staged_anchor_index: None,
                        staged_anchor_status_rev: None,
                    },
                );
                cx.notify();
            });
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
                            area: DiffArea::Staged,
                            path: a.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected status file context menu model");

        let unstage_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Unstage (2)" => {
                Some((**action).clone())
            }
            _ => None,
        });

        match unstage_action {
            Some(ContextMenuAction::UnstageSelectionOrPath {
                repo_id: rid,
                area,
                path,
            }) => {
                assert_eq!(rid, repo_id);
                assert_eq!(area, DiffArea::Staged);
                assert_eq!(path, a);
            }
            _ => panic!("expected Unstage (2) to unstage selected paths"),
        }
    });
}

#[gpui::test]
fn status_file_menu_offers_resolve_actions_for_conflicts(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(5);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_conflict",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("conflict.txt");

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
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
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

        let has_ours = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Resolve using ours" =>
            {
                matches!(
                    action.as_ref(),
                    ContextMenuAction::CheckoutConflictSideSelectionOrPath {
                        repo_id: rid,
                        area: DiffArea::Unstaged,
                        path: p,
                        side: gitcomet_core::services::ConflictSide::Ours
                    } if *rid == repo_id && p.as_path() == path.as_path()
                )
            }
            _ => false,
        });
        let has_theirs = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Resolve using theirs" =>
            {
                matches!(
                    action.as_ref(),
                    ContextMenuAction::CheckoutConflictSideSelectionOrPath {
                        repo_id: rid,
                        area: DiffArea::Unstaged,
                        path: p,
                        side: gitcomet_core::services::ConflictSide::Theirs
                    } if *rid == repo_id && p.as_path() == path.as_path()
                )
            }
            _ => false,
        });
        let has_manual = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Resolve manually…" =>
            {
                matches!(
                    action.as_ref(),
                    ContextMenuAction::SelectConflictDiff {
                        repo_id: rid,
                        path: p
                    } if *rid == repo_id && p.as_path() == path.as_path()
                )
            }
            _ => false,
        });
        let has_external_mergetool = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Open external mergetool" =>
            {
                matches!(
                    action.as_ref(),
                    ContextMenuAction::LaunchMergetool {
                        repo_id: rid,
                        path: p
                    } if *rid == repo_id && p.as_path() == path.as_path()
                )
            }
            _ => false,
        });

        assert!(has_ours);
        assert!(has_theirs);
        assert!(has_manual);
        assert!(has_external_mergetool);
    });
}

#[gpui::test]
fn status_file_menu_hides_external_mergetool_for_staged_conflicts(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(7);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_staged_conflict",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("conflict.txt");

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
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                        conflict: None,
                    }],
                    unstaged: vec![],
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
                            area: DiffArea::Staged,
                            path: path.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected status file context menu model");

        let has_external_mergetool = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => {
                label.as_ref().starts_with("Open external mergetool")
            }
            _ => false,
        });
        let has_discard_changes = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Discard changes",
            _ => false,
        });
        assert!(!has_external_mergetool);
        assert!(!has_discard_changes);
    });
}

#[gpui::test]
fn status_file_menu_open_from_details_pane_does_not_double_lease_panic(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(6);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_reentrant",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("conflict.txt");

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
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
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
            cx.notify();
        });
    });

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let anchor = point(px(0.0), px(0.0));
        details_pane.update(app, |pane, cx| {
            pane.open_popover_at(
                PopoverKind::StatusFileMenu {
                    repo_id,
                    area: DiffArea::Unstaged,
                    path: path.clone(),
                },
                anchor,
                window,
                cx,
            );
        });
    });
}
