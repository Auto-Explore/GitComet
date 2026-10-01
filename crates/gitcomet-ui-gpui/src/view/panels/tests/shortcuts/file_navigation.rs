//! File and change navigation from every focus context.

use super::*;

fn set_change_tracking_view_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    next: ChangeTrackingView,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| this.set_change_tracking_view(next, cx));
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn active_commit_diff_target_path(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> Option<std::path::PathBuf> {
    cx.update(|_window, app| {
        let root = view.read(app);
        let repo_id = root.state.active_repo?;
        let repo = root.state.repos.iter().find(|repo| repo.id == repo_id)?;
        match repo.diff_state.diff_target.clone()? {
            DiffTarget::Commit { path, .. } => Some(path),
            _ => None,
        }
    })
}

fn set_diff_selection_area(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    anchor: Option<usize>,
    range: Option<(usize, usize)>,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_selection_anchor = anchor;
                pane.diff_selection_range = range;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
}

fn three_hunk_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    let mut diff = two_hunk_diff(target);
    diff.lines.extend([
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Context,
            text: " unchanged again".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Hunk,
            text: "@@ -20 +20 @@".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Remove,
            text: "-old three".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Add,
            text: "+new three".into(),
        },
    ]);
    diff
}

#[gpui::test]
fn split_untracked_file_navigation_stays_within_untracked_section(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(703);
    let commit_id = CommitId("cafebabecafebabe".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_untracked_nav",
        std::process::id()
    ));
    let untracked_a = std::path::PathBuf::from("new-a.txt");
    let tracked = std::path::PathBuf::from("src/lib.rs");
    let untracked_b = std::path::PathBuf::from("new-b.txt");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: untracked_a.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: untracked_b.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
            ]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        untracked_a.clone(),
        DiffArea::Unstaged,
    ));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    set_change_tracking_view_for_test(cx, &view, ChangeTrackingView::SplitUntracked);

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, 1, window, cx)
        })
    });
    assert!(
        moved,
        "expected adjacent navigation to move to the next untracked row"
    );
}

#[gpui::test]
fn split_tracked_file_navigation_does_not_cross_into_untracked_section(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(704);
    let commit_id = CommitId("deadc0dedeadc0de".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_tracked_nav",
        std::process::id()
    ));
    let untracked = std::path::PathBuf::from("new-a.txt");
    let tracked_a = std::path::PathBuf::from("src/lib.rs");
    let tracked_b = std::path::PathBuf::from("src/main.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: untracked.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked_a.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked_b.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
            ]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        tracked_a.clone(),
        DiffArea::Unstaged,
    ));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    set_change_tracking_view_for_test(cx, &view, ChangeTrackingView::SplitUntracked);

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, -1, window, cx)
        })
    });
    assert!(
        !moved,
        "tracked-section navigation should not jump into the split untracked section"
    );
}

#[gpui::test]
fn commit_details_file_navigation_scrolls_selected_row_into_view(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7051);
    let commit_id = CommitId("fedcba0987654321".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_details_file_nav_scroll",
        std::process::id()
    ));
    let files = (0..64)
        .map(|ix| {
            CommitFileChange::new(
                std::path::PathBuf::from(format!("src/commit_nav/file_{ix:02}.rs")),
                FileStatusKind::Modified,
            )
        })
        .collect::<Vec<_>>();
    let start_ix = 40usize;
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id.clone(),
        message: "subject".into(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "2026-04-14 12:00:00 +0300".into(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: files.clone(),
    }));
    repo.diff_state.diff_target = Some(DiffTarget::commit(
        commit_id.clone(),
        files[start_ix].path.clone(),
    ));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1024.0), px(420.0)));
    draw_and_drain_test_window(cx);

    let initial_offset_y = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        uniform_list_offset(&pane.commit_files_scroll).y
    });
    assert_eq!(
        initial_offset_y,
        px(0.0),
        "expected the commit-details file list to start at the top"
    );

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, 1, window, cx)
        })
    });
    assert!(
        moved,
        "expected commit-details adjacent navigation to succeed"
    );
    draw_and_drain_test_window(cx);

    let offset_y = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        uniform_list_offset(&pane.commit_files_scroll).y
    });
    assert!(
        offset_y < px(0.0),
        "expected commit-details file navigation to scroll the selected row into view (offset_y={offset_y:?})",
    );
}

#[gpui::test]
fn commit_details_text_input_f4_navigates_files_without_stealing_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7052);
    let commit_id = CommitId("1122334455667788".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_details_input_nav",
        std::process::id()
    ));
    let files = vec![
        CommitFileChange::new(
            std::path::PathBuf::from("src/commit_details/first.rs"),
            FileStatusKind::Modified,
        ),
        CommitFileChange::new(
            std::path::PathBuf::from("src/commit_details/second.rs"),
            FileStatusKind::Modified,
        ),
    ];

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details = Loadable::Ready(Arc::new(CommitDetails {
        id: commit_id.clone(),
        message: "subject".into(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: "2026-04-14 12:00:00 +0300".into(),
        committed_at_unix: 0,
        parent_ids: vec![],
        files: files.clone(),
    }));
    repo.diff_state.diff_target =
        Some(DiffTarget::commit(commit_id.clone(), files[0].path.clone()));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_details_sha_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, files[1].path.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_commit_diff_target_path(cx, &view),
        Some(files[1].path.clone()),
        "expected F4 from commit-details text input to select the next commit file"
    );
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-details SHA input to keep focus after F4 navigation"
        );
    });
}

/// The comparison view's file list navigates like commit details: the diff
/// toolbar offers prev/next arrows, and F4/F1 step through the drawn rows.
#[gpui::test]
fn comparison_diff_steps_through_range_files_with_arrows_and_f1_f4(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70521);
    let tip = CommitId("aabbccddeeff0011".into());
    let base = CommitId("1100ffeeddccbbaa".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_comparison_file_nav",
        std::process::id()
    ));
    let files: Vec<CommitFileChange> = ["src/range/a.rs", "src/range/b.rs", "src/range/c.rs"]
        .into_iter()
        .map(|path| CommitFileChange::new(std::path::PathBuf::from(path), FileStatusKind::Modified))
        .collect();
    let range_target = |path: &std::path::Path| {
        DiffTarget::commit_range(base.clone(), Some(tip.clone()), Some(path.to_path_buf()))
    };

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &tip);
    repo.history_state.range_selection = Some(gitcomet_state::model::RangeSelection::new(
        base.clone(),
        Some(tip.clone()),
        "base".into(),
        "tip".into(),
    ));
    repo.history_state.range_files = Loadable::Ready(Arc::new(files.clone()));
    repo.history_state.range_files_rev = 1;
    repo.diff_state.diff_target = Some(range_target(&files[1].path));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_for_test(cx);
    // Where a row click leaves focus.
    focus_diff_panel(cx, &view);

    assert!(cx.debug_bounds("diff_prev_file").is_some());
    assert!(cx.debug_bounds("diff_next_file").is_some());

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, files[2].path.as_path());
    sync_store_snapshot(cx, &view);
    assert_eq!(
        cx.update(|_window, app| {
            let root = view.read(app);
            root.state.repos[0].diff_state.diff_target.clone()
        }),
        Some(range_target(&files[2].path)),
        "expected F4 to open the next comparison file"
    );
    assert!(
        cx.debug_bounds("diff_next_file").is_none(),
        "the last comparison file has no next file"
    );

    cx.simulate_keystrokes("f1");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, files[1].path.as_path());
    sync_store_snapshot(cx, &view);
    assert!(cx.debug_bounds("diff_next_file").is_some());
}

#[gpui::test]
fn commit_message_text_input_change_navigation_shortcuts_move_diff_without_stealing_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70532);
    let commit_id = CommitId("8899aabbccddef11".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_change_nav",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        three_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    wait_for_main_pane_condition(
        cx,
        &view,
        "diff rows for text-input change navigation",
        |pane| pane.diff_visible_len() > 0,
        |pane| {
            format!(
                "diff_visible_len={} diff_target={:?}",
                pane.diff_visible_len(),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone())
            )
        },
    );

    set_diff_selection_anchor(cx, &view, None);
    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let first_change = diff_selection_anchor(cx, &view)
        .expect("expected F7 from commit-message input to navigate to the first diff change");

    set_diff_selection_anchor(cx, &view, Some(first_change));
    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let second_change = diff_selection_anchor(cx, &view)
        .expect("expected F7 from commit-message input to reach the second diff change");
    assert!(
        second_change > first_change,
        "expected a later diff change target after the second F7 navigation"
    );

    set_diff_selection_area(
        cx,
        &view,
        Some(first_change),
        Some((first_change, second_change)),
    );
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let third_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 from a selected diff area to reach the third diff change");
    assert!(
        third_change > second_change,
        "expected F3 to continue after the selected diff area"
    );
    assert_eq!(
        diff_selection_range(cx, &view),
        None,
        "expected F3 to replace the selected diff area with an anchor on the target change"
    );

    set_diff_selection_area(
        cx,
        &view,
        Some(third_change),
        Some((second_change, third_change)),
    );
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 to continue before the selected diff area"
    );
    assert_eq!(
        diff_selection_range(cx, &view),
        None,
        "expected F2 to replace the selected diff area with an anchor on the target change"
    );

    set_diff_text_selection_on_row(cx, &view, second_change);
    assert!(
        diff_text_has_selection(cx, &view),
        "expected test setup to create a diff text selection"
    );
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 from commit-message input to fall back to the previous diff change when search is inactive"
    );
    assert!(
        !diff_text_has_selection(cx, &view),
        "expected F2 to clear the active diff text selection"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after F2 change navigation"
    );

    set_diff_text_selection_on_row(cx, &view, second_change);
    assert!(
        diff_text_has_selection(cx, &view),
        "expected test setup to create a diff text selection"
    );
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(third_change),
        "expected F3 from commit-message input to continue after the selected diff text"
    );
    assert!(
        !diff_text_has_selection(cx, &view),
        "expected F3 to clear the active diff text selection"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after F3 change navigation"
    );

    set_diff_selection_anchor(cx, &view, Some(second_change));
    cx.simulate_keystrokes("shift-f7");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected Shift-F7 from commit-message input to navigate to the previous diff change"
    );

    set_diff_selection_anchor(cx, &view, Some(second_change));
    cx.simulate_keystrokes("alt-up");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected Alt-Up from commit-message input to navigate to the previous diff change"
    );

    set_diff_selection_anchor(cx, &view, None);
    cx.simulate_keystrokes("alt-down");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected Alt-Down from commit-message input to navigate to the next diff change"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after change-navigation shortcuts"
    );
}

#[gpui::test]
fn create_branch_popover_text_input_f4_navigates_diff_without_closing_popover(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7054);
    let commit_id = CommitId("0102030405060708".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_create_branch_f4",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: first.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: second.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
            ]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(first.clone(), DiffArea::Unstaged));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id: RepoId(1),
                        target: "HEAD".to_string(),
                        source_selectable: false,
                        name_prefix: String::new(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to hold focus before navigation"
        );
    });

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected create-branch popover to remain open after F4 diff navigation"
    );
    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 from create-branch input to select the next diff target"
    );
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to keep focus after F4 navigation"
        );
    });
}

#[gpui::test]
fn create_branch_popover_text_input_f1_navigates_previous_diff_without_closing_popover(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70541);
    let commit_id = CommitId("0102030405060718".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_create_branch_f1",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &second,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::CreateBranchFromRefPrompt {
                        repo_id: RepoId(1),
                        target: "HEAD".to_string(),
                        source_selectable: false,
                        name_prefix: String::new(),
                    },
                    gpui::point(gpui::px(120.0), gpui::px(72.0)),
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to hold focus before previous-file navigation"
        );
    });

    cx.simulate_keystrokes("f1");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, first.as_path());
    sync_store_snapshot(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected create-branch popover to remain open after F1 diff navigation"
    );
    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected F1 from create-branch input to select the previous diff target"
    );
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .popover_host
            .read(app)
            .create_branch_input_focus_handle_for_test(app);
        assert!(
            focus.is_focused(window),
            "expected create-branch input to keep focus after F1 navigation"
        );
    });
}

#[gpui::test]
fn non_text_context_menu_focus_f4_uses_app_level_diff_navigation(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70561);
    let commit_id = CommitId("abcdef0011223355".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_menu_f4",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_for_test(cx);
    open_change_tracking_settings_popover(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking context menu to remain open before F4"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected context-menu focus to exercise the app-level shortcut fallback"
    );

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 from non-text context-menu focus to select the next diff target"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected app-level F4 navigation not to dismiss an unrelated context menu"
    );
}

#[gpui::test]
fn switching_diff_content_mode_restores_diff_panel_focus_for_change_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70566);
    let commit_id = CommitId("abcdef00112233aa".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_content_focus_switch",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    repo.diff_state.diff_rev = repo.diff_state.diff_rev.wrapping_add(1);
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    focus_diff_panel(cx, &view);
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected the diff panel to be focused before opening diff mode settings"
    );

    open_popover_for_test(cx, &view, PopoverKind::DiffContentModeSettings);
    assert!(
        popover_is_open(cx, &view),
        "expected the diff mode settings popover to open"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected the diff mode settings popover to move focus away from the diff panel"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.context_menu_activate_action(
                    ContextMenuAction::SetDiffContentMode {
                        mode: DiffContentMode::Collapsed,
                    },
                    window,
                    cx,
                );
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed diff content mode with navigable changes",
        |pane| {
            pane.diff_content_mode == DiffContentMode::Collapsed
                && pane.diff_nav_entries().len() >= 2
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_visible_len(),
                pane.diff_nav_entries(),
            )
        },
    );

    assert!(
        !popover_is_open(cx, &view),
        "expected selecting a diff mode to close the popover"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected selecting a diff mode to restore diff-panel focus"
    );
    assert_eq!(
        cx.update(|_window, app| crate::view::test_support::diff_content_mode(view.read(app))),
        DiffContentMode::Collapsed,
        "expected selecting the collapsed entry to update the global diff content mode"
    );

    // Nothing is focused after the mode switch, so the first F3 lands on the
    // first change and the second on the next one.
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let first_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 after closing diff mode settings to navigate to a change");
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let next_change = diff_selection_anchor(cx, &view)
        .expect("expected a second F3 to navigate to the next change");
    assert!(
        next_change > first_change,
        "expected each F3 to move one change forward"
    );

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    let previous_change = diff_selection_anchor(cx, &view)
        .expect("expected F2 after closing diff mode settings to navigate to a change");
    assert_eq!(
        previous_change, first_change,
        "expected F2 after closing diff mode settings to refresh and move to the previous change"
    );
}

#[gpui::test]
fn switching_change_tracking_view_restores_diff_panel_focus_for_adjacent_navigation(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(705);
    let commit_id = CommitId("1234567812345678".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_change_tracking_focus_switch",
        std::process::id()
    ));
    let untracked_a = std::path::PathBuf::from("new-a.txt");
    let tracked = std::path::PathBuf::from("src/lib.rs");
    let untracked_b = std::path::PathBuf::from("new-b.txt");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: untracked_a.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: tracked,
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: untracked_b.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Untracked,
                    conflict: None,
                },
            ]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        untracked_a.clone(),
        DiffArea::Unstaged,
    ));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected the diff panel to be focused before opening change-tracking settings"
    );

    open_change_tracking_settings_popover(cx, &view);
    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings popover to open"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected opening the change-tracking settings popover to move focus away from the diff panel"
    );

    cx.simulate_keystrokes("s");
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.update(|_window, app| {
            crate::view::test_support::change_tracking_view(view.read(app))
        }),
        ChangeTrackingView::SplitUntracked,
        "expected selecting the split view menu entry to update the change-tracking layout"
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the change-tracking settings popover to close after selecting split view"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected closing the change-tracking settings popover to restore diff-panel focus"
    );
    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(untracked_a),
        "expected the active diff target to stay selected after switching to split view"
    );

    let moved = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.try_select_adjacent_diff_file(repo_id, 1, window, cx)
        })
    });
    assert!(
        moved,
        "expected adjacent navigation to keep working immediately after switching to split view"
    );
}
