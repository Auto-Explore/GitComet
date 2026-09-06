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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![
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
                    ]),
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
                    staged: std::sync::Arc::new(vec![
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
                    ]),
                    unstaged: std::sync::Arc::new(vec![]),
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                        conflict: None,
                    }]),
                }
                .into(),
            );
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![]),
                }
                .into(),
            );
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
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
fn status_file_menu_hides_permalink_for_local_only_branch(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(8);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_local_only_permalink",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("a.txt");

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
            // `permalink-copy` exists only locally: a `blob/<branch>` link
            // would point at a nonexistent source on the forge.
            repo.head_branch = Loadable::Ready("permalink-copy".to_string());
            repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
                name: "origin".to_string(),
                url: Some("git@github.com:Auto-Explore/GitComet.git".to_string()),
            }]));
            repo.remote_branches =
                Loadable::Ready(Arc::new(vec![gitcomet_core::domain::RemoteBranch {
                    remote: "origin".to_string(),
                    name: "main".to_string(),
                    target: gitcomet_core::domain::CommitId("deadbeef".into()),
                }]));

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
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

        let has_permalink = model.items.iter().any(|item| match item {
            ContextMenuItem::Entry { label, .. } => label.as_ref() == "Copy file permalink",
            _ => false,
        });
        assert!(!has_permalink);
    });
}

#[gpui::test]
fn status_file_menu_offers_permalink_for_pushed_branch(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(9);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_menu_pushed_permalink",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("a.txt");

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
            // `main` has a remote counterpart, so the permalink resolves.
            repo.head_branch = Loadable::Ready("main".to_string());
            repo.remotes = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Remote {
                name: "origin".to_string(),
                url: Some("git@github.com:Auto-Explore/GitComet.git".to_string()),
            }]));
            repo.remote_branches =
                Loadable::Ready(Arc::new(vec![gitcomet_core::domain::RemoteBranch {
                    remote: "origin".to_string(),
                    name: "main".to_string(),
                    target: gitcomet_core::domain::CommitId("deadbeef".into()),
                }]));

            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
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

        let permalink = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. }
                if label.as_ref() == "Copy file permalink" =>
            {
                Some((**action).clone())
            }
            _ => None,
        });
        match permalink {
            Some(ContextMenuAction::CopyText { text }) => {
                assert_eq!(
                    text,
                    "https://github.com/Auto-Explore/GitComet/blob/main/a.txt"
                );
            }
            _ => panic!("expected Copy file permalink to copy the branch permalink"),
        }
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Conflicted,
                        conflict: None,
                    }]),
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

/// Seed a repo whose unstaged lane holds `entries`, optionally with `selection`
/// marked in the unstaged bucket, and return the status-file menu for `clicked`.
fn status_menu_for(
    cx: &mut gpui::TestAppContext,
    entries: &[(&str, gitcomet_core::domain::FileStatusKind)],
    selection: &[&str],
    clicked: &str,
) -> ContextMenuModel {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(7);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_gitignore_menu",
        std::process::id()
    ));

    let unstaged: Vec<_> = entries
        .iter()
        .map(|(path, kind)| gitcomet_core::domain::FileStatus {
            path: std::path::PathBuf::from(path),
            kind: *kind,
            conflict: None,
        })
        .collect();
    let selection: Vec<std::path::PathBuf> =
        selection.iter().map(std::path::PathBuf::from).collect();

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
                    unstaged: std::sync::Arc::new(unstaged),
                }
                .into(),
            );
            let state = Arc::new(AppState {
                repos: vec![repo],
                active_repo: Some(repo_id),
                ..Default::default()
            });
            this.state = Arc::clone(&state);
            // In the running app the host mirrors the UI model through an
            // observer; seed it directly so the menu model sees the repo.
            this.popover_host.update(cx, |host, _cx| {
                host.state = Arc::clone(&state);
            });
            if selection.len() > 1 {
                this.details_pane.update(cx, |pane, cx| {
                    pane.status_multi_selection.insert(
                        repo_id,
                        StatusMultiSelection {
                            untracked: vec![],
                            untracked_anchor: None,
                            unstaged: selection.clone(),
                            unstaged_anchor: selection.first().cloned(),
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
            }
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_model(
                    &PopoverKind::StatusFileMenu {
                        repo_id,
                        area: DiffArea::Unstaged,
                        path: std::path::PathBuf::from(clicked),
                    },
                    cx,
                )
            })
        })
        .expect("expected status file context menu model")
    })
}

fn gitignore_entry_label(model: &ContextMenuModel) -> Option<String> {
    model.items.iter().find_map(|item| match item {
        ContextMenuItem::Entry { label, .. } if label.contains(".gitignore") => {
            Some(label.to_string())
        }
        _ => None,
    })
}

#[gpui::test]
fn status_file_menu_offers_gitignore_only_for_untracked(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::FileStatusKind;

    let entries = &[
        ("build/out.log", FileStatusKind::Untracked),
        ("src/lib.rs", FileStatusKind::Modified),
    ];

    let untracked = status_menu_for(cx, entries, &[], "build/out.log");
    assert_eq!(
        gitignore_entry_label(&untracked).as_deref(),
        Some("Add to .gitignore…")
    );

    let tracked = status_menu_for(cx, entries, &[], "src/lib.rs");
    assert_eq!(
        gitignore_entry_label(&tracked),
        None,
        "a tracked file keeps being tracked no matter what .gitignore says, so \
         the entry must not be offered at all"
    );
}

#[gpui::test]
fn status_file_menu_gitignore_entry_counts_the_selection(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::FileStatusKind;

    let model = status_menu_for(
        cx,
        &[
            ("build/a.log", FileStatusKind::Untracked),
            ("build/b.log", FileStatusKind::Untracked),
            ("build/c.log", FileStatusKind::Untracked),
        ],
        &["build/a.log", "build/b.log", "build/c.log"],
        "build/a.log",
    );

    assert_eq!(
        gitignore_entry_label(&model).as_deref(),
        Some("Add 3 files to .gitignore…")
    );

    let action = model.items.iter().find_map(|item| match item {
        ContextMenuItem::Entry { label, action, .. } if label.contains(".gitignore") => {
            Some((**action).clone())
        }
        _ => None,
    });
    match action {
        Some(ContextMenuAction::AddToGitignoreSelectionOrPath {
            repo_id: _,
            area,
            path,
        }) => {
            assert_eq!(area, DiffArea::Unstaged);
            assert_eq!(
                path,
                std::path::PathBuf::from("build/a.log"),
                "the payload carries the clicked row only; the selection is \
                 re-derived so cancelling the dialog cannot lose it"
            );
        }
        _ => panic!("expected AddToGitignoreSelectionOrPath"),
    }
}

#[gpui::test]
fn status_file_menu_hides_gitignore_for_a_mixed_selection(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::domain::FileStatusKind;

    let model = status_menu_for(
        cx,
        &[
            ("build/a.log", FileStatusKind::Untracked),
            ("src/lib.rs", FileStatusKind::Modified),
        ],
        &["build/a.log", "src/lib.rs"],
        "build/a.log",
    );

    assert_eq!(
        gitignore_entry_label(&model),
        None,
        "one tracked path in the selection would get a pattern that does nothing"
    );
}
