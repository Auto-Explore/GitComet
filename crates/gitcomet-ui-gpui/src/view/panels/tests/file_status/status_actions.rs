//! Status actions: staging, folder actions, and the file context menu.

use super::*;

#[gpui::test]
fn status_file_right_click_opens_menu_without_opening_diff_or_changing_selection(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store_for_view, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_right_click_menu_only",
        std::process::id()
    ));

    let a = std::path::PathBuf::from("a.txt");
    let b = std::path::PathBuf::from("b.txt");
    let untracked = std::path::PathBuf::from("untracked.txt");
    let staged = std::path::PathBuf::from("staged.txt");

    // The diff panel is parked on a file that none of the right-clicks touch.
    let initial_target = gitcomet_core::domain::DiffTarget::working_tree(
        std::path::PathBuf::from("parked.txt"),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    let selection = vec![a.clone(), b.clone()];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.open = gitcomet_state::model::Loadable::Ready(());
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: staged.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
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
                        gitcomet_core::domain::FileStatus {
                            path: untracked.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Untracked,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(initial_target.clone());

            // Seed the store too, so `Msg::SelectDiff` would really land in
            // `diff_state.diff_target` if the right-click still dispatched one.
            let next_state = app_state_with_repo(repo, repo_id);
            store.replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);

            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        unstaged: selection.clone(),
                        unstaged_anchor: Some(a.clone()),
                        ..Default::default()
                    },
                );
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    // `a` sits inside the left-click multi-selection; the untracked and staged rows sit
    // outside it. All three must behave the same on right-click.
    let cases = [
        (
            "unstaged",
            0usize,
            gitcomet_core::domain::DiffArea::Unstaged,
            a.clone(),
        ),
        (
            "unstaged",
            2,
            gitcomet_core::domain::DiffArea::Unstaged,
            untracked.clone(),
        ),
        (
            "staged",
            0,
            gitcomet_core::domain::DiffArea::Staged,
            staged.clone(),
        ),
    ];

    for (section_label, ix, area, path) in cases {
        let row_selector = format!("status_row_{}_{}_{}", repo_id.0, section_label, ix);
        let row_bounds = cx
            .debug_bounds(Box::leak(row_selector.clone().into_boxed_str()))
            .unwrap_or_else(|| panic!("expected status row {row_selector} to be rendered"));
        let row_center = row_bounds.center();
        cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
        test_support::redraw(cx);
        let action_selector = format!("status_row_action_{}_{}_{}", repo_id.0, section_label, ix);
        let action_bounds = cx
            .debug_bounds(Box::leak(action_selector.into_boxed_str()))
            .expect("expected the hovered row action to be rendered");
        assert_eq!(
            action_bounds.right(),
            row_bounds.right(),
            "the per-file stage action must be flush with the row's right edge"
        );
        cx.simulate_mouse_down(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );

        cx.run_until_parked();

        let diff_target = store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.diff_state.diff_target.clone());
        let (popover_kind, multi_selection) = cx.update(|_window, app| {
            let view = view.read(app);
            let popover_kind = view.popover_host.read(app).popover_kind_for_tests();
            let multi_selection = view
                .details_pane
                .read(app)
                .status_multi_selection
                .get(&repo_id)
                .map(|sel| {
                    sel.selected_paths_for_area(gitcomet_core::domain::DiffArea::Unstaged)
                        .to_vec()
                });
            (popover_kind, multi_selection)
        });

        assert_eq!(
            popover_kind,
            Some(PopoverKind::StatusFileMenu {
                repo_id,
                area,
                path: path.clone(),
            }),
            "right-clicking {row_selector} must open that row's file context menu"
        );
        assert_eq!(
            diff_target,
            Some(initial_target.clone()),
            "right-clicking {row_selector} must not open the file diff"
        );
        assert_eq!(
            multi_selection,
            Some(selection.clone()),
            "right-clicking {row_selector} must not change the left-click selection"
        );

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }
}

/// Both "Stage all" buttons go through the same helper, so the confirmation
/// cannot be present on one and missing on the other. The split view's button
/// names its section's paths; the combined one passes an empty set meaning
/// "everything". Either way a conflicted file with markers left in it has to
/// stop the stage.
#[gpui::test]
fn stage_all_asks_before_staging_unresolved_conflicts(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(70615);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_stage_all_conflict_confirm",
        std::process::id()
    ));
    let conflicted = std::path::PathBuf::from("conflicted.rs");
    let clean = std::path::PathBuf::from("clean.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join(&conflicted),
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n",
    )
    .unwrap();
    std::fs::write(workdir.join(&clean), "resolved\n").unwrap();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: conflicted.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                        },
                        gitcomet_core::domain::FileStatus {
                            path: clean.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            let state = app_state_with_repo(repo, repo_id);
            // Both sides: the pane renders from the UI model, while the
            // "nothing was staged" assertion reads the store's own snapshot.
            this.store.replace_snapshot_for_test(Arc::clone(&state));
            push_test_state(this, state, cx);
        });
    });
    draw_and_drain_test_window(cx);

    // The split view's "Stage all": the tracked-changes section by name.
    let ops_rev_before = crate::view::test_support::repo_ops_rev(&view, cx, repo_id);
    let split_paths = vec![conflicted.clone(), clean.clone()];
    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.stage_all_with_conflict_confirmation(repo_id, split_paths, window, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(
            kind,
            Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                if unresolved == &vec![conflicted.clone()]
        ),
        "the split view's Stage all must warn about the conflicted file, got {kind:?}"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        });
    });
    draw_and_drain_test_window(cx);

    // The combined view's "Stage all": everything, expressed as an empty set.
    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.stage_all_with_conflict_confirmation(repo_id, Vec::new(), window, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(
            kind,
            Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                if unresolved == &vec![conflicted.clone()]
        ),
        "the combined view's Stage all must warn about the conflicted file, got {kind:?}"
    );

    crate::view::test_support::drain_store_worker(&view, cx);
    assert_eq!(
        crate::view::test_support::repo_ops_rev(&view, cx, repo_id),
        ops_rev_before,
        "nothing may be staged until the confirmation is answered"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The folder covers its whole subtree, nested collapsed folders included.
#[gpui::test]
fn status_folder_stage_covers_the_subtree_and_ignores_the_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(640);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(
                repo_id,
                &["src/nested/deep.rs", "src/shallow.rs", "docs/other.md"],
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    // No selection input at all, so `docs/other.md` cannot leak in.
    let paths = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_folder_subtree_paths(
                repo_id,
                StatusSection::CombinedUnstaged,
                std::path::Path::new("src"),
            )
    });

    let mut names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "src/nested/deep.rs".to_string(),
            "src/shallow.rs".to_string()
        ],
        "the folder covers its whole subtree and nothing outside it"
    );
}

/// `Path::starts_with` is component-wise, so a sibling sharing a name prefix
/// must not be swept in.
#[gpui::test]
fn status_folder_paths_do_not_capture_a_name_prefixed_sibling(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(641);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["src/a.rs", "src2/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    let paths = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_folder_subtree_paths(
                repo_id,
                StatusSection::CombinedUnstaged,
                std::path::Path::new("src"),
            )
    });
    assert_eq!(
        paths,
        vec![std::path::PathBuf::from("src/a.rs")],
        "src must not capture src2"
    );
}

/// Folder actions must use the original path even when display labels collide.
#[cfg(unix)]
#[gpui::test]
fn status_folder_actions_keep_non_utf8_directory_paths(cx: &mut gpui::TestAppContext) {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(642);
    let paths = [
        std::path::PathBuf::from(OsStr::from_bytes(b"bad\xff/nested/a.rs")),
        std::path::PathBuf::from(OsStr::from_bytes(b"bad\xfe/nested/b.rs")),
    ];
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = repo_with_unstaged_paths(repo_id, &[]);
            let entries = Arc::new(
                paths
                    .iter()
                    .map(|path| gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    })
                    .collect(),
            );
            repo.worktree_status = Loadable::Ready(Arc::clone(&entries));
            repo.staged_status = Loadable::Ready(entries);
            repo.staged_status_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.set_file_list_layout(crate::view::FileListLayout::Tree, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo = pane.active_repo().expect("repo");
        for section in [StatusSection::CombinedUnstaged, StatusSection::Staged] {
            let plan = pane.status_file_plan(repo, section);
            let keys: Vec<_> = (0..plan.row_len())
                .filter_map(|ix| match plan.row_at(crate::view::rows::RowIx(ix)) {
                    Some(crate::view::rows::FileListRow::Directory { key, .. }) => Some(key),
                    _ => None,
                })
                .collect();
            assert_eq!(keys.len(), 2, "each raw directory needs its own action");
            for path in &paths {
                let key = keys
                    .iter()
                    .find(|key| key.as_ref() == path.parent().unwrap())
                    .expect("folder action retains the raw path");
                assert_eq!(
                    pane.status_folder_subtree_paths(repo_id, section, key),
                    vec![path.clone()],
                );
            }
        }
    });
}

/// Drives the folder Stage button on a tree-layout Unstaged section and reports
/// whether the click dispatched a repo action to the store, read through
/// `drain_store_worker` so the answer does not race the worker thread.
fn folder_stage_button_stages(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    button: gpui::MouseButton,
) -> bool {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["src/a.rs", "src/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let folder = cx
        .debug_bounds(leaked(format!("status_dir_{}_unstaged_0", repo_id.0)))
        .expect("expected a folder row");
    // The action is revealed on hover, so the pointer has to be on the row.
    cx.simulate_mouse_move(folder.center(), None, gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    let action = cx
        .debug_bounds(leaked(format!(
            "status_dir_action_{}_unstaged_0",
            repo_id.0
        )))
        .expect("expected the folder action");
    // The reducer drops stage messages outright without git, which would read
    // as "not dispatched" and pass the negative control for the wrong reason.
    assert!(
        cx.update(|_window, app| view.read(app).store.snapshot().git_runtime.is_available()),
        "this test needs a git executable on PATH"
    );
    let before = crate::view::test_support::repo_ops_rev(&view, cx, repo_id);
    let at = action.center();
    cx.simulate_mouse_down(at, button, gpui::Modifiers::default());
    cx.simulate_mouse_up(at, button, gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    crate::view::test_support::drain_store_worker(&view, cx);
    crate::view::test_support::repo_ops_rev(&view, cx, repo_id) > before
}

/// A left click stages the folder — the positive control, so the right-click
/// assertion below cannot pass just because the button was never hit.
#[gpui::test]
fn folder_stage_button_stages_on_a_left_click(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    assert!(folder_stage_button_stages(
        cx,
        gitcomet_state::model::RepoId(680),
        gpui::MouseButton::Left
    ));
}

/// A right click opens the context menu; it must not stage the subtree.
#[gpui::test]
fn folder_stage_button_ignores_a_right_click(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    assert!(!folder_stage_button_stages(
        cx,
        gitcomet_state::model::RepoId(681),
        gpui::MouseButton::Right
    ));
}
