use super::*;

const SECTIONS: [(StatusSection, &[&str]); 4] = [
    (
        StatusSection::CombinedUnstaged,
        &["a.rs", "b.rs", "c.rs", "new/a.rs", "new/b.rs"],
    ),
    (StatusSection::Unstaged, &["a.rs", "b.rs", "c.rs"]),
    (StatusSection::Untracked, &["new/a.rs", "new/b.rs"]),
    (StatusSection::Staged, &["staged/a.rs", "staged/b.rs"]),
];

fn reset_section(cx: &mut gpui::VisualTestContext, view: &View, section: StatusSection) {
    apply_state(cx, view, app_state_with_active_repo(status_repo()));
    cx.update(|_window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.clear_status_multi_selection(REPO);
            cx.notify();
        });
    });
    set_change_tracking_view_for_test(
        cx,
        view,
        if matches!(section, StatusSection::Unstaged | StatusSection::Untracked) {
            ChangeTrackingView::SplitUntracked
        } else {
            ChangeTrackingView::Combined
        },
    );
    draw_and_drain_test_window(cx);
}

fn assert_destination(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    section: StatusSection,
    next: Option<&str>,
) {
    let expected = next.map(|path| DiffTarget::WorkingTree {
        path: path.into(),
        area: section.diff_area(),
    });
    wait_until(
        cx,
        &format!("{section:?} Space destination {expected:?}"),
        |cx| preview(cx, view) == expected,
    );
    sync_store_snapshot(cx, view);
    assert_eq!(preview(cx, view), expected);
    assert!(selected(cx, view, section.diff_area()).is_empty());
    cx.update(|window, app| {
        assert!(
            view.read(app)
                .main_pane
                .read(app)
                .diff_panel_focus_handle
                .is_focused(window),
            "Space must leave the diff ready for the next review keystroke"
        );
    });
}

fn refresh_after_action(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    section: StatusSection,
    path: &str,
) {
    let mut repo = cx.update(|_window, app| view.read(app).store.snapshot().repos[0].clone());
    let entries = match section.diff_area() {
        DiffArea::Unstaged => &mut repo.worktree_status,
        DiffArea::Staged => &mut repo.staged_status,
    };
    let Loadable::Ready(entries) = entries else {
        unreachable!()
    };
    Arc::make_mut(entries).retain(|entry| entry.path != Path::new(path));
    repo.worktree_status_rev += 1;
    repo.staged_status_rev += 1;
    repo.local_actions_in_flight = 0;
    apply_state(cx, view, app_state_with_active_repo(repo));
}

#[gpui::test]
fn single_file_space_reviews_each_section_from_list_and_diff(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    for (section, paths) in SECTIONS {
        for diff_focus in [false, true] {
            reset_section(cx, &view, section);
            click_row(cx, section, 0, gpui::Modifiers::default());
            wait_until_store_diff_target_path(cx, &view, Path::new(paths[0]));
            sync_store_snapshot(cx, &view);
            if diff_focus {
                focus_diff_panel(cx, &view);
            }
            // Subsequent presses use the focus and selection left by Space,
            // including the final press when only one file remains.
            for (ix, path) in paths.iter().enumerate() {
                cx.simulate_keystrokes("space");
                let next = paths.get(ix + 1).copied();
                assert_destination(cx, &view, section, next);
                refresh_after_action(cx, &view, section, path);
                assert_destination(cx, &view, section, next);
            }
        }
    }
}

#[gpui::test]
fn single_file_space_at_end_clears_even_with_earlier_files(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    for (section, paths) in SECTIONS {
        for diff_focus in [false, true] {
            reset_section(cx, &view, section);
            click_row(cx, section, paths.len() - 1, gpui::Modifiers::default());
            wait_until_store_diff_target_path(cx, &view, Path::new(paths.last().unwrap()));
            sync_store_snapshot(cx, &view);
            if diff_focus {
                focus_diff_panel(cx, &view);
            }
            cx.simulate_keystrokes("space");
            assert_destination(cx, &view, section, None);
            refresh_after_action(cx, &view, section, paths.last().unwrap());
            assert_destination(cx, &view, section, None);
            cx.simulate_keystrokes("space");
            draw_and_drain_test_window(cx);
            assert_eq!(preview(cx, &view), None);
            cx.update(|_window, app| {
                assert_eq!(
                    view.read(app).store.snapshot().repos[0].local_actions_in_flight,
                    0
                );
            });
        }
    }
}

#[gpui::test]
fn single_file_space_uses_selected_row_when_preview_is_elsewhere(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let before = preview(cx, &view);
    click_row(
        cx,
        StatusSection::Staged,
        0,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    );
    assert_eq!(preview(cx, &view), before);
    cx.simulate_keystrokes("space");
    assert_destination(cx, &view, StatusSection::Staged, Some("staged/b.rs"));
}

#[gpui::test]
fn single_file_space_opens_the_next_conflict_like_f4(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let mut repo = status_repo();
    let Loadable::Ready(entries) = &mut repo.worktree_status else {
        unreachable!()
    };
    Arc::make_mut(entries)[1].kind = FileStatusKind::Conflicted;
    Arc::make_mut(entries)[1].conflict =
        Some(gitcomet_core::domain::FileConflictKind::BothModified);
    repo.worktree_status_rev += 1;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("space");
    assert_destination(cx, &view, StatusSection::CombinedUnstaged, Some("b.rs"));
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).store.snapshot().repos[0]
                .conflict_state
                .conflict_file_path
                .as_deref(),
            Some(Path::new("b.rs")),
            "the next conflict must open through SelectConflictDiff"
        );
    });
}

#[gpui::test]
fn single_file_space_follows_reverse_sort(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let section = StatusSection::Unstaged;
    reset_section(cx, &view, section);
    cx.update(|_window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.set_status_file_sort(
                section,
                crate::view::rows::CommitFileSort::PathDescending,
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);
    click_row(cx, section, 0, gpui::Modifiers::default());
    wait_until_store_diff_target_path(cx, &view, Path::new("c.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("space");
    assert_destination(cx, &view, section, Some("b.rs"));
}

#[gpui::test]
fn single_file_space_reveals_next_file_in_collapsed_tree_folder(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let section = StatusSection::CombinedUnstaged;
    let repo = simple_worktree_repo(
        REPO,
        Path::new("/tmp/status-space-tree"),
        &CommitId("7317317317317317".into()),
        &[
            "root.rs".into(),
            "src/a.rs".into(),
            "src/b.rs".into(),
            "z/c.rs".into(),
        ],
        Path::new("src/b.rs"),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|_window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.set_file_list_layout(crate::view::FileListLayout::Tree, cx);
        });
    });
    draw_and_drain_test_window(cx);
    // Tree rows: src/, a, b, z/, c, root. Hide c without changing the
    // navigation order, then advance from b into its collapsed sibling folder.
    click(
        cx,
        format!("status_dir_{}_unstaged_3", REPO.0),
        gpui::Modifiers::default(),
    );
    assert!(cx.debug_bounds("status_row_731_unstaged_5").is_none());
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("space");
    assert_destination(cx, &view, section, Some("z/c.rs"));
    assert!(
        cx.debug_bounds("status_row_731_unstaged_5").is_some(),
        "navigation must expand z/"
    );
    assert!(
        cx.debug_bounds("status_row_731_unstaged_4").is_some(),
        "the next file must be visible"
    );
}

#[gpui::test]
fn single_file_space_scrolls_to_an_offscreen_destination(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let paths: Vec<_> = (0..80)
        .map(|ix| std::path::PathBuf::from(format!("file{ix:02}.rs")))
        .collect();
    let repo = simple_worktree_repo(
        REPO,
        Path::new("/tmp/status-space-scroll"),
        &CommitId("7317317317317317".into()),
        &paths,
        &paths[45],
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    assert!(cx.debug_bounds("status_row_731_unstaged_46").is_none());
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("space");
    assert_destination(
        cx,
        &view,
        StatusSection::CombinedUnstaged,
        Some("file46.rs"),
    );
    assert!(cx.debug_bounds("status_row_731_unstaged_46").is_some());
}

#[gpui::test]
fn single_file_space_resumes_navigation_only_after_conflict_confirmation(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let dir = tempfile::tempdir().unwrap();
    for (source_ix, path, next) in [(1, "b.rs", Some("c.rs")), (4, "new/b.rs", None)] {
        std::fs::create_dir_all(dir.path().join("new")).unwrap();
        std::fs::write(
            dir.path().join(path),
            "<<<<<<< ours\nleft\n=======\nright\n>>>>>>> theirs\n",
        )
        .unwrap();
        for diff_focus in [false, true] {
            reset_section(cx, &view, StatusSection::CombinedUnstaged);
            let mut repo = status_repo();
            repo.spec.workdir = dir.path().to_path_buf();
            let Loadable::Ready(entries) = &mut repo.worktree_status else {
                unreachable!()
            };
            Arc::make_mut(entries)[source_ix].conflict =
                Some(gitcomet_core::domain::FileConflictKind::BothModified);
            repo.status = Loadable::Ready(Arc::new(gitcomet_core::domain::RepoStatus {
                staged: repo.staged_status.ready().unwrap().clone(),
                unstaged: entries.clone(),
            }));
            repo.worktree_status_rev += 1;
            apply_state(cx, &view, app_state_with_active_repo(repo));
            click_row(
                cx,
                StatusSection::CombinedUnstaged,
                source_ix,
                gpui::Modifiers::default(),
            );
            wait_until_store_diff_target_path(cx, &view, Path::new(path));
            sync_store_snapshot(cx, &view);
            if diff_focus {
                focus_diff_panel(cx, &view);
            }
            let before = preview(cx, &view);
            for confirm in [false, true] {
                cx.simulate_keystrokes("space");
                draw_and_drain_test_window(cx);
                assert!(popover_is_open(cx, &view));
                assert_eq!(preview(cx, &view), before);
                assert_eq!(
                    selected(cx, &view, DiffArea::Unstaged),
                    vec![std::path::PathBuf::from(path)]
                );
                cx.update(|_window, app| {
                    assert_eq!(
                        view.read(app).store.snapshot().repos[0].local_actions_in_flight,
                        0
                    );
                });
                if confirm {
                    click(
                        cx,
                        "stage_conflict_markers_go".into(),
                        gpui::Modifiers::default(),
                    );
                    assert_destination(cx, &view, StatusSection::CombinedUnstaged, next);
                } else {
                    cx.simulate_keystrokes("escape");
                    draw_and_drain_test_window(cx);
                    assert_eq!(preview(cx, &view), before);
                    assert_eq!(
                        selected(cx, &view, DiffArea::Unstaged),
                        vec![std::path::PathBuf::from(path)]
                    );
                }
                assert!(!popover_is_open(cx, &view));
            }
        }
    }
}

#[gpui::test]
fn single_file_space_confirmation_clears_without_canceling_focused_mergetool(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let exit_code = Arc::new(std::sync::atomic::AtomicI32::new(-1));
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new_with_config(
            store,
            events,
            crate::view::GitCometViewConfig {
                view_mode: GitCometViewMode::FocusedMergetool,
                focused_mergetool_exit_code: Some(Arc::clone(&exit_code)),
                ..Default::default()
            },
            window,
            cx,
        )
    });
    let dir = tempfile::tempdir().unwrap();
    let path = std::path::PathBuf::from("conflicted.rs");
    std::fs::write(
        dir.path().join(&path),
        "<<<<<<< ours\nleft\n=======\nright\n>>>>>>> theirs\n",
    )
    .unwrap();
    let mut repo = simple_worktree_repo(
        REPO,
        dir.path(),
        &CommitId("7317317317317317".into()),
        std::slice::from_ref(&path),
        &path,
    );
    set_test_conflict_status(&mut repo, path.clone(), DiffArea::Unstaged);
    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);
    let before = preview(cx, &view);

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(
            kind,
            Some(PopoverKind::StageConflictMarkersConfirm {
                ref paths,
                navigation: Some(StatusStageNavigation::Clear),
                ..
            }) if paths == &vec![path]
        ),
        "staging the last file must await confirmation before clearing: {kind:?}"
    );
    assert_eq!(preview(cx, &view), before);
    assert_eq!(exit_code.load(Ordering::SeqCst), -1);

    click(
        cx,
        "stage_conflict_markers_go".into(),
        gpui::Modifiers::default(),
    );
    assert_eq!(
        exit_code.load(Ordering::SeqCst),
        -1,
        "accepting staging must not cancel the focused mergetool"
    );
    assert!(!popover_is_open(cx, &view));
    assert_destination(cx, &view, StatusSection::CombinedUnstaged, None);
}

#[gpui::test]
fn multiple_file_space_in_status_list_keeps_clearing_the_diff(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    for (section, _) in SECTIONS {
        reset_section(cx, &view, section);
        click_row(cx, section, 0, gpui::Modifiers::default());
        click_row(
            cx,
            section,
            1,
            gpui::Modifiers {
                control: true,
                ..Default::default()
            },
        );
        assert_eq!(selected(cx, &view, section.diff_area()).len(), 2);
        cx.simulate_keystrokes("space");
        wait_until(cx, "multi-file Space to clear the diff", |cx| {
            preview(cx, &view).is_none()
        });
        assert!(selected(cx, &view, section.diff_area()).is_empty());
        cx.update(|window, app| {
            assert!(
                view.read(app)
                    .details_pane
                    .read(app)
                    .status_section_focus_handle(section)
                    .is_focused(window)
            );
        });
    }
}
