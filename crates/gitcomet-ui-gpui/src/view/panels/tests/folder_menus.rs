use super::*;
use crate::view::rows::FileListId;
use gitcomet_state::model::{Loadable, RepoId};

fn leaked(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn repo_with_unstaged(repo_id: RepoId, paths: &[&str]) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-folder-menus"));
    repo.worktree_status = Loadable::Ready(Arc::new(
        paths
            .iter()
            .map(|path| gitcomet_core::domain::FileStatus {
                path: (*path).into(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
    ));
    repo.worktree_status_rev = 1;
    repo
}

fn status_tree_view<'a>(
    cx: &'a mut gpui::TestAppContext,
    repo_id: RepoId,
    paths: &[&str],
) -> (
    gpui::Entity<super::super::GitCometView>,
    &'a mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo = repo_with_unstaged(repo_id, paths);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    (view, cx)
}

fn popover_kind(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<PopoverKind> {
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

fn menu_entries(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    kind: &PopoverKind,
) -> Vec<(String, ContextMenuAction)> {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_model(kind, cx)
                    .expect("a context menu model")
                    .items
                    .into_iter()
                    .filter_map(|item| match item {
                        ContextMenuItem::Entry { label, action, .. } => {
                            Some((label.to_string(), *action))
                        }
                        _ => None,
                    })
                    .collect()
            })
        })
    })
}

fn activate(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    action: ContextMenuAction,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_activate_action(action, window, cx);
            });
        });
    });
    draw_and_drain_test_window(cx);
}

/// A status folder's right-click menu opens on the row, keeps it lit, and
/// stages the folder's whole subtree.
#[gpui::test]
fn status_folder_right_click_opens_a_menu_that_stages_the_folder(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = RepoId(690);
    let (view, cx) = status_tree_view(cx, repo_id, &["src/a.rs", "src/b.rs"]);

    let folder = cx
        .debug_bounds(leaked(format!("status_dir_{}_unstaged_0", repo_id.0)))
        .expect("expected a folder row");
    cx.simulate_mouse_down(
        folder.center(),
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        folder.center(),
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    draw_and_drain_test_window(cx);

    let list = FileListId::Status(StatusSection::CombinedUnstaged);
    let kind = popover_kind(cx, &view).expect("the folder menu opens");
    let PopoverKind::FileListFolderMenu {
        list: menu_list,
        key,
        collapsed,
        apply_source,
        ..
    } = &kind
    else {
        panic!("expected the folder menu, got {kind:?}");
    };
    assert_eq!(*menu_list, list);
    assert_eq!(key.as_ref(), Path::new("src"));
    assert!(!collapsed);
    assert_eq!(*apply_source, None);
    let invoker = cx.update(|_window, app| view.read(app).active_context_menu_invoker.clone());
    assert_eq!(
        invoker,
        Some(crate::view::rows::file_list_folder_menu_invoker(
            repo_id.0,
            list,
            Path::new("src")
        )),
        "the folder row stays lit while its menu is open"
    );

    let entries = menu_entries(cx, &view, &kind);
    let labels: Vec<&str> = entries.iter().map(|(label, _)| label.as_str()).collect();
    for expected in [
        "Collapse",
        "Expand all under here",
        "Collapse all under here",
        "Stage folder (2)",
        "Discard changes…",
        "Open folder location",
        "Copy relative path",
    ] {
        assert!(labels.contains(&expected), "{expected} in {labels:?}");
    }
    assert!(!labels.iter().any(|label| label.starts_with("Apply")));

    assert!(
        cx.update(|_window, app| view.read(app).store.snapshot().git_runtime.is_available()),
        "this test needs a git executable on PATH"
    );
    let before = crate::view::test_support::repo_ops_rev(&view, cx, repo_id);
    let stage = entries
        .into_iter()
        .find(|(label, _)| label == "Stage folder (2)")
        .map(|(_, action)| action)
        .expect("stage entry");
    activate(cx, &view, stage);
    crate::view::test_support::drain_store_worker(&view, cx);
    assert!(
        crate::view::test_support::repo_ops_rev(&view, cx, repo_id) > before,
        "the folder was staged"
    );
}

/// The discard confirmation counts the folder's files and discards them all.
#[gpui::test]
fn discard_folder_confirm_discards_every_file_under_it(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = RepoId(691);
    let (view, cx) = status_tree_view(cx, repo_id, &["src/a.rs", "src/b.rs", "docs/c.md"]);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::DiscardFolderChangesConfirm {
                        repo_id,
                        section: StatusSection::CombinedUnstaged,
                        folder: Arc::from(Path::new("src")),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    let paths = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_folder_subtree_paths(repo_id, StatusSection::CombinedUnstaged, Path::new("src"))
    });
    assert_eq!(paths.len(), 2);

    assert!(
        cx.update(|_window, app| view.read(app).store.snapshot().git_runtime.is_available()),
        "this test needs a git executable on PATH"
    );
    let before = crate::view::test_support::repo_ops_rev(&view, cx, repo_id);
    let go = cx
        .debug_bounds("discard_folder_changes_go")
        .expect("the Discard button renders")
        .center();
    cx.simulate_mouse_move(go, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(go, gpui::MouseButton::Left, gpui::Modifiers::default());
    cx.simulate_mouse_up(go, gpui::MouseButton::Left, gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    crate::view::test_support::drain_store_worker(&view, cx);
    assert!(
        crate::view::test_support::repo_ops_rev(&view, cx, repo_id) > before,
        "the folder's changes were discarded"
    );
    assert_eq!(popover_kind(cx, &view), None);
}

/// A linked worktree's paths are relative to that worktree, so its folder
/// menu offers no absolute path and nothing to apply.
#[gpui::test]
fn worktree_folder_menu_copies_only_the_relative_path(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = RepoId(692);
    let (view, cx) = status_tree_view(cx, repo_id, &["src/a.rs"]);
    let entries = menu_entries(
        cx,
        &view,
        &PopoverKind::FileListFolderMenu {
            repo_id,
            list: FileListId::WorktreeFiles,
            key: Arc::from(Path::new("src")),
            chain: Arc::from(vec![Arc::from(Path::new("src"))]),
            collapsed: true,
            apply_source: None,
        },
    );
    let labels: Vec<&str> = entries.iter().map(|(label, _)| label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Expand",
            "Expand all under here",
            "Collapse all under here",
            "Copy relative path"
        ]
    );
}

#[gpui::test]
fn deleted_linked_worktree_files_keep_diff_and_path_actions_without_file_opening(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let linked = tempfile::tempdir().unwrap();
    let path = std::path::PathBuf::from("deleted.txt");
    let file = gitcomet_core::domain::FileStatus {
        path: path.clone(),
        kind: gitcomet_core::domain::FileStatusKind::Deleted,
        conflict: None,
    };
    let repo_id = RepoId(1);
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/origin-checkout"));
    repo.open = Loadable::Ready(());
    repo.history_state.worktree_selection = Some(linked.path().to_path_buf());
    repo.worktree_dirty = Loadable::Ready(Arc::new(vec![
        gitcomet_core::domain::WorktreeDirtySummary {
            path: linked.path().to_path_buf(),
            head: None,
            branch: Some("linked".into()),
            detached: false,
            added: 0,
            modified: 0,
            deleted: 1,
            staged: vec![file.clone()],
            unstaged: vec![file],
            line_stats: Default::default(),
        },
    ]));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            push_test_state(this, app_state_with_repo(repo, repo_id), cx)
        })
    });
    for area in [DiffArea::Staged, DiffArea::Unstaged] {
        let kind = PopoverKind::WorktreeFileMenu {
            repo_id,
            worktree_path: linked.path().to_path_buf(),
            target: DiffTarget::working_tree(path.clone(), area),
        };
        let model = cx.update(|_, app| {
            view.read(app)
                .popover_host
                .clone()
                .update(app, |host, cx| host.context_menu_model(&kind, cx).unwrap())
        });
        for item in model.items {
            if let ContextMenuItem::Entry {
                label,
                disabled,
                action,
                ..
            } = item
            {
                match label.as_ref() {
                    "Open file" | "Open in code editor" => {
                        assert!(disabled, "a deleted file cannot be opened")
                    }
                    "View diff"
                    | "Open in worktree tab"
                    | "Open file location"
                    | "Copy absolute path"
                    | "Copy relative path" => assert!(!disabled, "deleted files retain {label}"),
                    unexpected => panic!("unexpected foreign-file action: {unexpected}"),
                }
                match *action {
                    ContextMenuAction::OpenWorktreeFile {
                        worktree_path,
                        path: action_path,
                    }
                    | ContextMenuAction::OpenWorktreeFileLocation {
                        worktree_path,
                        path: action_path,
                    } => {
                        assert_eq!(worktree_path, linked.path());
                        assert_eq!(action_path, path);
                    }
                    ContextMenuAction::OpenWorktreeDiff { target, .. } => {
                        assert_eq!(target, DiffTarget::working_tree(path.clone(), area))
                    }
                    ContextMenuAction::CopyText { text }
                        if label.as_ref() == "Copy absolute path" =>
                    {
                        assert_eq!(text, linked.path().join(&path).to_string_lossy())
                    }
                    _ => {}
                }
            }
        }
    }
}
