use super::*;
use gitcomet_core::domain::{CommitDetails, CommitFileChange, CommitId, FileStatusKind};
use gitcomet_state::model::{Loadable, RepoId};

const SHA_A: &str = "0123456789abcdef0123456789abcdef01234567";
const SHA_B: &str = "89abcdef0123456789abcdef0123456789abcdef";

fn file(path: &str) -> CommitFileChange {
    CommitFileChange::new(std::path::PathBuf::from(path), FileStatusKind::Modified)
}

fn push_commit(
    view: &gpui::Entity<super::super::GitCometView>,
    cx: &mut gpui::VisualTestContext,
    repo_id: RepoId,
    sha: &str,
    files: Vec<CommitFileChange>,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-select"));
            repo.history_state.selected_commit = Some(CommitId(sha.into()));
            // Loading another commit's details always moves this rev.
            repo.history_state.commit_details_rev = u64::from(sha.as_bytes()[0]);
            repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
                id: CommitId(sha.into()),
                message: "subject".to_string(),
                author_name: String::new(),
                author_email: String::new(),
                authored_at_unix: 0,
                committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                committed_at_unix: 0,
                parent_ids: vec![],
                files,
            }));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
}

fn click_row(cx: &mut gpui::VisualTestContext, selector: String, modifiers: gpui::Modifiers) {
    let center = cx
        .debug_bounds(Box::leak(selector.clone().into_boxed_str()))
        .unwrap_or_else(|| panic!("{selector} should render"))
        .center();
    cx.simulate_mouse_move(center, None, modifiers);
    cx.simulate_mouse_down(center, gpui::MouseButton::Left, modifiers);
    cx.simulate_mouse_up(center, gpui::MouseButton::Left, modifiers);
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

/// Drawn row indices of the file rows (folder rows excluded).
fn file_rows(cx: &mut gpui::VisualTestContext, repo_id: RepoId) -> Vec<usize> {
    (0..16)
        .filter(|ix| {
            cx.debug_bounds(Box::leak(
                format!("commit_file_{}_{ix}", repo_id.0).into_boxed_str(),
            ))
            .is_some()
        })
        .collect()
}

fn selection(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: RepoId,
) -> Vec<String> {
    cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_list_selected_paths(repo_id, crate::view::rows::FileListId::CommitFiles)
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    })
}

/// In a tree, Shift-click spans the drawn file rows, stepping over folder
/// rows, and a different commit starts with no selection.
#[gpui::test]
fn commit_file_tree_shift_click_spans_drawn_files(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(162);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, _cx| {
                pane.file_list_layout = crate::view::FileListLayout::Tree;
            });
        });
    });
    push_commit(
        &view,
        cx,
        repo_id,
        SHA_A,
        vec![file("src/a.rs"), file("src/nested/b.rs"), file("web/c.rs")],
    );

    // Drawn: src/, nested/, b.rs, a.rs, web/, c.rs.
    let rows = file_rows(cx, repo_id);
    assert_eq!(rows.len(), 3, "three file rows: {rows:?}");
    assert!(
        rows[2] > rows[1] + 1,
        "a folder row sits between the last two files: {rows:?}"
    );
    let selector = |ix: usize| format!("commit_file_{}_{ix}", repo_id.0);
    let shift = gpui::Modifiers {
        shift: true,
        ..Default::default()
    };

    // `src/a.rs` to `web/c.rs` is two drawn files, though `src/nested/b.rs`
    // sorts between them.
    click_row(cx, selector(rows[1]), gpui::Modifiers::default());
    click_row(cx, selector(rows[2]), shift);
    assert_eq!(selection(cx, &view, repo_id), ["src/a.rs", "web/c.rs"]);

    click_row(cx, selector(rows[0]), shift);
    assert_eq!(
        selection(cx, &view, repo_id),
        ["src/nested/b.rs", "src/a.rs"],
        "the anchor stays on the plain-clicked row"
    );

    push_commit(&view, cx, repo_id, SHA_B, vec![file("other.rs")]);
    assert!(selection(cx, &view, repo_id).is_empty());
}

/// Submodule rows carry no file change, so the selection's apply skips them.
#[gpui::test]
fn commit_file_selection_apply_skips_submodules(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(163);
    let mut submodule = file("vendor/lib");
    submodule.is_submodule = true;
    push_commit(
        &view,
        cx,
        repo_id,
        SHA_A,
        vec![file("a.rs"), file("b.rs"), submodule],
    );
    let ctrl = gpui::Modifiers {
        control: true,
        ..Default::default()
    };
    click_row(
        cx,
        format!("commit_file_{}_0", repo_id.0),
        gpui::Modifiers::default(),
    );
    click_row(cx, format!("commit_file_{}_1", repo_id.0), ctrl);
    click_row(cx, format!("commit_file_{}_2", repo_id.0), ctrl);
    assert_eq!(selection(cx, &view, repo_id).len(), 3);

    let paths = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_list_paths_for_action(
                repo_id,
                crate::view::rows::FileListId::CommitFiles,
                Path::new("b.rs"),
            )
    });
    assert_eq!(
        paths,
        [
            std::path::PathBuf::from("a.rs"),
            std::path::PathBuf::from("b.rs")
        ]
    );
}

fn folder_menu_entries(
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

fn commit_tree_view(
    cx: &mut gpui::TestAppContext,
    repo_id: RepoId,
) -> (
    gpui::Entity<super::super::GitCometView>,
    &mut gpui::VisualTestContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, _cx| {
                pane.file_list_layout = crate::view::FileListLayout::Tree;
            });
        });
    });
    let mut added = file("src/a.rs");
    added.kind = FileStatusKind::Added;
    push_commit(
        &view,
        cx,
        repo_id,
        SHA_A,
        vec![added, file("src/nested/b.rs"), file("web/c.rs")],
    );
    (view, cx)
}

/// A commit tree's folder menu applies the folder's files, as the active
/// filter shows them.
#[gpui::test]
fn commit_folder_menu_applies_the_folders_shown_files(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = RepoId(164);
    let (view, cx) = commit_tree_view(cx, repo_id);

    let folder = cx
        .debug_bounds(Box::leak(
            format!("commit_file_dir_{}_0", repo_id.0).into_boxed_str(),
        ))
        .expect("the src folder row renders")
        .center();
    cx.simulate_mouse_down(folder, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.simulate_mouse_up(folder, gpui::MouseButton::Right, gpui::Modifiers::default());
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let kind = cx
        .update(|_window, app| {
            view.read(app)
                .popover_host
                .read(app)
                .popover_kind_for_tests()
        })
        .expect("the folder menu opens");
    assert!(
        matches!(
            &kind,
            PopoverKind::FileListFolderMenu {
                list: crate::view::rows::FileListId::CommitFiles,
                key,
                apply_source: Some(gitcomet_core::domain::ApplyChangeSource::Commit(id)),
                ..
            } if key.as_ref() == Path::new("src") && id.as_ref() == SHA_A
        ),
        "{kind:?}"
    );

    let apply = folder_menu_entries(cx, &view, &kind)
        .into_iter()
        .find(|(label, _)| label.starts_with("Apply"))
        .expect("an apply entry");
    assert_eq!(apply.0, "Apply changes (2)");
    let ContextMenuAction::ApplyFileChange { target, .. } = apply.1 else {
        panic!("expected the apply action");
    };
    assert_eq!(
        target.paths,
        [
            std::path::PathBuf::from("src/nested/b.rs"),
            std::path::PathBuf::from("src/a.rs")
        ],
        "the folder's files in drawn order"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, _cx| {
                pane.commit_file_filter = crate::view::rows::CommitFileFilter::Added;
            });
        });
    });
    let filtered = folder_menu_entries(cx, &view, &kind)
        .into_iter()
        .find(|(label, _)| label.starts_with("Apply"))
        .expect("an apply entry");
    assert_eq!(filtered.0, "Apply change", "only the shown file is applied");
}

/// "Collapse all under here" folds the folder and every folder inside it;
/// "Expand all under here" opens them all again.
#[gpui::test]
fn folder_menu_collapses_and_expands_everything_under_it(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let repo_id = RepoId(165);
    let (view, cx) = commit_tree_view(cx, repo_id);
    assert_eq!(file_rows(cx, repo_id).len(), 3);
    let set_collapsed = |cx: &mut gpui::VisualTestContext, collapsed: bool| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.context_menu_activate_action(
                        ContextMenuAction::SetFileListFolderCollapsed {
                            repo_id,
                            list: crate::view::rows::FileListId::CommitFiles,
                            key: Arc::from(Path::new("src")),
                            chain: Arc::from(vec![Arc::from(Path::new("src"))]),
                            collapsed,
                            recursive: true,
                        },
                        window,
                        cx,
                    );
                });
            });
        });
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    };
    let collapsed_dirs = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let mut dirs: Vec<String> = view
                .read(app)
                .details_pane
                .read(app)
                .file_list_collapsed_for(repo_id, crate::view::rows::FileListId::CommitFiles)
                .set()
                .iter()
                .map(|dir| dir.display().to_string())
                .collect();
            dirs.sort();
            dirs
        })
    };

    set_collapsed(cx, true);
    assert_eq!(collapsed_dirs(cx), ["src", "src/nested"]);
    assert_eq!(file_rows(cx, repo_id).len(), 1, "only web/c.rs is shown");

    set_collapsed(cx, false);
    assert!(collapsed_dirs(cx).is_empty());
    assert_eq!(file_rows(cx, repo_id).len(), 3);
}

/// The file menu has no "Open diff"; the row's click opens the diff, and for a
/// renamed file that diff reads its old side from the rename source.
#[gpui::test]
fn commit_file_click_opens_a_renamed_file_from_its_old_path(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(164);
    push_commit(
        &view,
        cx,
        repo_id,
        SHA_A,
        vec![
            CommitFileChange::new(
                std::path::PathBuf::from("src/new.rs"),
                FileStatusKind::Renamed,
            )
            .with_old_path(Some(std::path::PathBuf::from("src/old.rs"))),
        ],
    );
    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.store
                .replace_snapshot_for_test(Arc::clone(&this.state));
        });
    });

    click_row(
        cx,
        format!("commit_file_{}_0", repo_id.0),
        gpui::Modifiers::default(),
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let target = loop {
        let target = cx.update(|_window, app| {
            view.read(app).store.snapshot().repos[0]
                .diff_state
                .diff_target
                .clone()
        });
        if let Some(target) = target {
            break target;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the row click should open the file's diff"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(target.file_path(), Some(Path::new("src/new.rs")));
    assert_eq!(target.old_file_path(), Some(Path::new("src/old.rs")));
}
