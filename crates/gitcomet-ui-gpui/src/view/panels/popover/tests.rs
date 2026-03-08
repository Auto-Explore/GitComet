use super::*;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use std::path::Path;
use std::sync::Arc;
use std::time::SystemTime;

struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

#[gpui::test]
fn commit_menu_has_add_tag_entry(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(1);
    let commit_id = CommitId("deadbeefdeadbeef".to_string());
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
                        summary: "Hello".to_string(),
                        author: "Alice".to_string(),
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
    let commit_id = CommitId("deadbeefdeadbeef".to_string());
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
fn tag_menu_lists_delete_entries_for_commit_tags(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(2);
    let commit_id = CommitId("0123456789abcdef".to_string());
    let other_commit = CommitId("aaaaaaaaaaaaaaaa".to_string());
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
                        summary: "Hello".to_string(),
                        author: "Alice".to_string(),
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
    let commit_id = CommitId("fedcba9876543210".to_string());
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
                        &PopoverKind::RemoteMenu {
                            repo_id,
                            name: remote_name.clone(),
                        },
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
                        unstaged: vec![a.clone(), b.clone()],
                        unstaged_anchor: Some(a.clone()),
                        staged: vec![],
                        staged_anchor: None,
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
                        unstaged: vec![],
                        unstaged_anchor: None,
                        staged: vec![a.clone(), b.clone()],
                        staged_anchor: Some(a.clone()),
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
                    ContextMenuAction::SelectDiff {
                        repo_id: rid,
                        target: DiffTarget::WorkingTree { path: p, area: DiffArea::Unstaged }
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

#[gpui::test]
fn stash_menu_has_apply_pop_and_drop_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(8);
    let index = 3usize;
    let message = "WIP".to_string();

    cx.update(|_window, app| {
        let model = view
            .update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_model(
                        &PopoverKind::StashMenu {
                            repo_id,
                            index,
                            message: message.clone(),
                        },
                        cx,
                    )
                })
            })
            .expect("expected stash context menu model");

        let apply_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Apply stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            apply_action,
            Some(ContextMenuAction::ApplyStash {
                repo_id: rid,
                index: ix
            }) if rid == repo_id && ix == index
        ));

        let pop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Pop stash" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            pop_action,
            Some(ContextMenuAction::PopStash {
                repo_id: rid,
                index: ix
            }) if rid == repo_id && ix == index
        ));

        let drop_action = model.items.iter().find_map(|item| match item {
            ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Drop stash…" => {
                Some((**action).clone())
            }
            _ => None,
        });
        assert!(matches!(
            drop_action,
            Some(ContextMenuAction::DropStashConfirm {
                repo_id: rid,
                index: ix,
                message: ref msg
            }) if rid == repo_id && ix == index && msg == &message
        ));
    });
}

#[gpui::test]
fn stash_menu_drop_action_opens_drop_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    let repo_id = RepoId(9);
    let index = 1usize;
    let message = "Drop me".to_string();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.popover_anchor = Some(PopoverAnchor::Point(point(px(32.0), px(48.0))));
                host.context_menu_activate_action(
                    ContextMenuAction::DropStashConfirm {
                        repo_id,
                        index,
                        message: message.clone(),
                    },
                    window,
                    cx,
                );

                match host.popover.as_ref() {
                    Some(PopoverKind::StashDropConfirm {
                        repo_id: rid,
                        index: ix,
                        message: msg,
                    }) => {
                        assert_eq!(*rid, repo_id);
                        assert_eq!(*ix, index);
                        assert_eq!(msg, &message);
                    }
                    _ => panic!("expected stash drop confirm popover to open"),
                }
            });
        });
    });
}
