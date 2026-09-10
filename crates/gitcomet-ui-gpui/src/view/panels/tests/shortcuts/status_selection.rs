use super::*;

const REPO: RepoId = RepoId(731);
type View = gpui::Entity<crate::view::GitCometView>;

fn status_repo() -> RepoState {
    let mut repo = simple_worktree_repo(
        REPO,
        Path::new("/tmp/status-selection"),
        &CommitId("7317317317317317".into()),
        &["a.rs".into(), "b.rs".into(), "c.rs".into()],
        Path::new("a.rs"),
    );
    let file = |path: &str, kind| gitcomet_core::domain::FileStatus {
        path: path.into(),
        kind,
        conflict: None,
    };
    repo.worktree_status = Loadable::Ready(Arc::new(vec![
        file("a.rs", FileStatusKind::Modified),
        file("b.rs", FileStatusKind::Modified),
        file("c.rs", FileStatusKind::Modified),
        file("new/a.rs", FileStatusKind::Untracked),
        file("new/b.rs", FileStatusKind::Untracked),
    ]));
    repo.staged_status = Loadable::Ready(Arc::new(vec![
        file("staged/a.rs", FileStatusKind::Modified),
        file("staged/b.rs", FileStatusKind::Modified),
    ]));
    repo.worktree_status_rev = 1;
    repo.staged_status_rev = 1;
    repo
}

fn fixture(cx: &mut gpui::TestAppContext) -> (View, &mut gpui::VisualTestContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    apply_state(cx, &view, app_state_with_active_repo(status_repo()));
    cx.update(|_window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.set_file_list_layout(crate::view::FileListLayout::Flat, cx);
        });
    });
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    cx.update(|_window, app| crate::app::bind_text_input_keys_for_test(app));
    draw_and_drain_test_window(cx);
    (view, cx)
}

fn click(cx: &mut gpui::VisualTestContext, selector: String, modifiers: gpui::Modifiers) {
    let header = selector.ends_with("_header");
    let bounds = cx
        .debug_bounds(Box::leak(selector.into_boxed_str()))
        .expect("rendered target");
    // Header padding avoids both the pane resize edge and the title's dropdown;
    // rows avoid the trailing action.
    cx.simulate_click(
        gpui::point(
            bounds.left()
                + if header {
                    px(6.0)
                } else {
                    px(40.0).min(bounds.size.width * 0.5)
                },
            bounds.center().y,
        ),
        modifiers,
    );
    draw_and_drain_test_window(cx);
}

fn click_row(
    cx: &mut gpui::VisualTestContext,
    section: StatusSection,
    ix: usize,
    modifiers: gpui::Modifiers,
) {
    click(
        cx,
        format!("status_row_{}_{}_{ix}", REPO.0, section.id_label()),
        modifiers,
    );
}

fn selected(
    cx: &mut gpui::VisualTestContext,
    view: &View,
    area: DiffArea,
) -> Vec<std::path::PathBuf> {
    cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_selected_paths_for_area(REPO, area)
            .to_vec()
    })
}

fn preview(cx: &mut gpui::VisualTestContext, view: &View) -> Option<DiffTarget> {
    cx.update(|_window, app| {
        view.read(app).store.snapshot().repos[0]
            .diff_state
            .diff_target
            .clone()
    })
}

#[gpui::test]
fn status_select_all_is_scoped_to_each_focused_section(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let before = preview(cx, &view);
    for (section, header, expected) in [
        (
            StatusSection::CombinedUnstaged,
            "unstaged_header",
            vec!["a.rs", "b.rs", "c.rs", "new/a.rs", "new/b.rs"],
        ),
        (
            StatusSection::Staged,
            "staged_header",
            vec!["staged/a.rs", "staged/b.rs"],
        ),
        (
            StatusSection::Untracked,
            "untracked_header",
            vec!["new/a.rs", "new/b.rs"],
        ),
        (
            StatusSection::Unstaged,
            "split_unstaged_header",
            vec!["a.rs", "b.rs", "c.rs"],
        ),
    ] {
        if section == StatusSection::Untracked {
            cx.update(|_window, app| {
                view.read(app).details_pane.clone().update(app, |pane, cx| {
                    pane.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
                });
            });
            draw_and_drain_test_window(cx);
        }
        click(cx, header.into(), gpui::Modifiers::default());
        cx.update(|window, app| {
            assert!(
                view.read(app)
                    .details_pane
                    .read(app)
                    .status_section_focus_handle(section)
                    .is_focused(window),
                "clicking {header} should focus its section"
            );
        });
        cx.simulate_keystrokes("secondary-a");
        draw_and_drain_test_window(cx);
        assert_eq!(
            selected(cx, &view, section.diff_area()),
            expected
                .iter()
                .map(std::path::PathBuf::from)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            preview(cx, &view),
            before,
            "select all must not change the preview"
        );
        cx.update(|window, app| {
            let pane = view.read(app).details_pane.read(app);
            assert!(pane.status_section_focus_handle(section).is_focused(window));
            let selection = &pane.status_multi_selection[&REPO];
            assert_eq!(selection.explicit_section, Some(section));
            assert_eq!(
                selection.untracked.len() + selection.unstaged.len() + selection.staged.len(),
                expected.len()
            );
            assert!(!view.read(app).main_pane.read(app).diff_text_has_selection());
        });
    }
}

#[gpui::test]
fn status_select_all_can_be_reduced_to_a_range_and_then_nothing(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    // Start with the preview in a different section from the selection being edited.
    let before = preview(cx, &view);
    let section = StatusSection::Staged;
    let toggle = gpui::Modifiers {
        control: true,
        ..Default::default()
    };
    click_row(cx, section, 0, toggle);
    assert_eq!(preview(cx, &view), before);
    cx.simulate_keystrokes("ctrl-a");
    draw_and_drain_test_window(cx);
    assert_eq!(selected(cx, &view, DiffArea::Staged).len(), 2);
    click_row(cx, section, 1, toggle);
    assert_eq!(
        selected(cx, &view, DiffArea::Staged),
        vec![std::path::PathBuf::from("staged/a.rs")]
    );
    click_row(cx, section, 1, toggle);
    assert_eq!(selected(cx, &view, DiffArea::Staged).len(), 2);
    click_row(
        cx,
        section,
        1,
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(
        selected(cx, &view, DiffArea::Staged),
        vec![std::path::PathBuf::from("staged/b.rs")]
    );
    click_row(cx, section, 1, toggle);
    assert!(selected(cx, &view, DiffArea::Staged).is_empty());
    assert_eq!(preview(cx, &view), before);

    // A later snapshot must not remove the explicit empty selection.
    apply_state(cx, &view, app_state_with_active_repo(status_repo()));
    cx.simulate_keystrokes("space ctrl-u");
    draw_and_drain_test_window(cx);
    assert_eq!(preview(cx, &view), before);
    assert!(
        cx.debug_bounds("stage_selected_button").is_none(),
        "the preview must not contribute a selected file in another section"
    );
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.details_pane.read(app);
        assert_eq!(
            pane.status_multi_selection[&REPO].explicit_section,
            Some(section)
        );
        assert_eq!(root.store.snapshot().repos[0].local_actions_in_flight, 0);
    });
}

#[gpui::test]
fn status_select_all_includes_collapsed_and_offscreen_files(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let mut repo = status_repo();
    let paths: Vec<_> = (0..100)
        .map(|ix| std::path::PathBuf::from(format!("src/file_{ix:03}.rs")))
        .collect();
    repo.worktree_status = Loadable::Ready(Arc::new(
        paths
            .iter()
            .map(|path| gitcomet_core::domain::FileStatus {
                path: path.clone(),
                kind: FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
    ));
    repo.worktree_status_rev += 1;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|_window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.set_file_list_layout(crate::view::FileListLayout::Tree, cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("status_row_731_unstaged_99").is_none(),
        "list should be virtualized"
    );
    click(
        cx,
        "status_dir_731_unstaged_0".into(),
        gpui::Modifiers::default(),
    );
    assert!(
        cx.debug_bounds("status_row_731_unstaged_1").is_none(),
        "folder should be collapsed"
    );
    cx.simulate_keystrokes("ctrl-a");
    draw_and_drain_test_window(cx);
    assert_eq!(selected(cx, &view, DiffArea::Unstaged), paths);
    assert!(
        cx.debug_bounds("status_row_731_unstaged_1").is_none(),
        "select all must not expand folders"
    );
}

#[gpui::test]
fn status_select_all_leaves_text_inputs_and_other_shortcuts_intact(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let text = "a commit message";
    cx.update(|window, app| {
        let input = view
            .read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .clone();
        input.update(app, |input, cx| input.set_text(text, cx));
        window.focus(&input.read(app).focus_handle(), app);
    });
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("secondary-a");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).selected_range(),
            0..text.len()
        );
        assert!(!pane.status_multi_selection.contains_key(&REPO));
    });
    click_row(
        cx,
        StatusSection::CombinedUnstaged,
        1,
        gpui::Modifiers::default(),
    );
    wait_until_store_diff_target_path(cx, &view, Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("ctrl-a");
    draw_and_drain_test_window(cx);
    assert_eq!(selected(cx, &view, DiffArea::Unstaged).len(), 5);
    cx.simulate_keystrokes("f4");
    wait_until_store_diff_target_path(cx, &view, Path::new("c.rs"));
}

#[gpui::test]
fn status_select_all_range_tracks_line_stats_and_refresh_prunes_only_missing_files(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    click_row(
        cx,
        StatusSection::CombinedUnstaged,
        1,
        gpui::Modifiers::default(),
    );
    wait_until_store_diff_target_path(cx, &view, Path::new("b.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("ctrl-a");
    draw_and_drain_test_window(cx);
    let mut repo = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .active_repo()
            .unwrap()
            .clone()
    });
    repo.uncommitted_line_stats =
        Loadable::Ready(Arc::new(gitcomet_core::domain::UncommittedLineStats {
            staged: Default::default(),
            unstaged: [("a.rs", 100), ("b.rs", 1), ("c.rs", 10)]
                .into_iter()
                .map(|(path, additions)| {
                    (
                        path.into(),
                        gitcomet_core::domain::LineStats {
                            additions: Some(additions),
                            deletions: Some(0),
                        },
                    )
                })
                .collect(),
        }));
    repo.unstaged_line_stats_rev += 1;
    apply_state(cx, &view, app_state_with_active_repo(repo.clone()));
    cx.update(|_window, app| {
        view.read(app).details_pane.clone().update(app, |pane, cx| {
            pane.set_status_file_sort(
                StatusSection::CombinedUnstaged,
                crate::view::rows::CommitFileSort::EditSizeDescending,
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);
    click_row(
        cx,
        StatusSection::CombinedUnstaged,
        0,
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    assert_eq!(
        selected(cx, &view, DiffArea::Unstaged),
        ["a.rs", "c.rs", "b.rs"].map(std::path::PathBuf::from)
    );
    assert_eq!(preview(cx, &view), repo.diff_state.diff_target);

    let Loadable::Ready(entries) = &mut repo.worktree_status else {
        unreachable!()
    };
    Arc::make_mut(entries).retain(|entry| entry.path != Path::new("c.rs"));
    Arc::make_mut(entries).push(gitcomet_core::domain::FileStatus {
        path: "d.rs".into(),
        kind: FileStatusKind::Modified,
        conflict: None,
    });
    repo.worktree_status_rev += 1;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    assert_eq!(
        selected(cx, &view, DiffArea::Unstaged),
        ["a.rs", "b.rs"].map(std::path::PathBuf::from)
    );
    cx.simulate_keystrokes("ctrl-a");
    draw_and_drain_test_window(cx);
    assert!(selected(cx, &view, DiffArea::Unstaged).contains(&std::path::PathBuf::from("d.rs")));
}

#[gpui::test]
fn status_selection_staging_uses_remaining_paths_even_when_the_preview_is_excluded(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("b.rs"),
        "<<<<<<< ours\nleft\n=======\nright\n>>>>>>> theirs\n",
    )
    .unwrap();
    let mut repo = status_repo();
    repo.spec.workdir = dir.path().to_path_buf();
    let Loadable::Ready(entries) = &mut repo.worktree_status else {
        unreachable!()
    };
    Arc::make_mut(entries)[1].conflict =
        Some(gitcomet_core::domain::FileConflictKind::BothModified);
    repo.status = Loadable::Ready(Arc::new(gitcomet_core::domain::RepoStatus {
        staged: repo.staged_status.ready().unwrap().clone(),
        unstaged: entries.clone(),
    }));
    repo.worktree_status_rev += 1;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    click(cx, "unstaged_header".into(), gpui::Modifiers::default());
    cx.simulate_keystrokes("ctrl-a");
    draw_and_drain_test_window(cx);
    click_row(
        cx,
        StatusSection::CombinedUnstaged,
        0,
        gpui::Modifiers {
            control: true,
            ..Default::default()
        },
    );
    let expected: Vec<_> = ["b.rs", "c.rs", "new/a.rs", "new/b.rs"]
        .map(std::path::PathBuf::from)
        .into();
    assert_eq!(selected(cx, &view, DiffArea::Unstaged), expected);
    for (shortcut, cancel) in [
        (Some("space"), "escape"),
        (Some("ctrl-s"), "button"),
        (None, "outside"),
        (Some("ctrl-s"), "escape"),
    ] {
        if let Some(shortcut) = shortcut {
            cx.simulate_keystrokes(shortcut);
        } else {
            click(
                cx,
                "stage_selected_button".into(),
                gpui::Modifiers::default(),
            );
        }
        draw_and_drain_test_window(cx);
        let kind =
            cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
        let Some(PopoverKind::StageConflictMarkersConfirm { paths, .. }) = kind else {
            panic!("expected selected-path confirmation, got {kind:?}");
        };
        assert_eq!(paths, expected);
        assert_eq!(selected(cx, &view, DiffArea::Unstaged), expected);
        match cancel {
            "escape" => cx.simulate_keystrokes("escape"),
            "button" => click(
                cx,
                "stage_conflict_markers_cancel_hint".into(),
                gpui::Modifiers::default(),
            ),
            "outside" => {
                let scrim = cx
                    .debug_bounds("repo_popover_close")
                    .expect("dismiss scrim");
                cx.simulate_click(
                    scrim.origin + point(px(8.0), px(8.0)),
                    gpui::Modifiers::default(),
                );
            }
            _ => unreachable!(),
        }
        draw_and_drain_test_window(cx);
        assert!(!popover_is_open(cx, &view));
        cx.update(|window, app| {
            assert!(
                view.read(app)
                    .details_pane
                    .read(app)
                    .status_section_focus_handle(StatusSection::CombinedUnstaged)
                    .is_focused(window),
                "{cancel} must restore section focus before retrying"
            );
            assert_eq!(
                view.read(app).store.snapshot().repos[0].local_actions_in_flight,
                0
            );
        });
        assert_eq!(
            selected(cx, &view, DiffArea::Unstaged),
            expected,
            "cancelling must preserve the edited selection"
        );
    }
}

#[gpui::test]
fn diff_stage_and_advance_clears_only_the_consumed_singleton_selection(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    for (section, shortcut, first, next) in [
        (StatusSection::CombinedUnstaged, "space", "a.rs", "b.rs"),
        (StatusSection::CombinedUnstaged, "ctrl-s", "a.rs", "b.rs"),
        (StatusSection::Staged, "space", "staged/a.rs", "staged/b.rs"),
        (
            StatusSection::Staged,
            "ctrl-u",
            "staged/a.rs",
            "staged/b.rs",
        ),
    ] {
        for leave_empty in [false, true] {
            cx.update(|_window, app| {
                view.read(app)
                    .details_pane
                    .clone()
                    .update(app, |pane, _cx| {
                        pane.clear_status_multi_selection(REPO);
                    });
            });
            apply_state(cx, &view, app_state_with_active_repo(status_repo()));
            click_row(cx, section, 0, gpui::Modifiers::default());
            wait_until_store_diff_target_path(cx, &view, Path::new(first));
            sync_store_snapshot(cx, &view);
            if leave_empty {
                click_row(
                    cx,
                    section,
                    0,
                    gpui::Modifiers {
                        control: true,
                        ..Default::default()
                    },
                );
                assert!(selected(cx, &view, section.diff_area()).is_empty());
            }
            focus_diff_panel(cx, &view);
            cx.simulate_keystrokes(shortcut);
            wait_until_store_diff_target_path(cx, &view, Path::new(next));
            sync_store_snapshot(cx, &view);
            // Model the status refresh that removes the file just acted on.
            let mut repo =
                cx.update(|_window, app| view.read(app).store.snapshot().repos[0].clone());
            let status = match section.diff_area() {
                DiffArea::Unstaged => &mut repo.worktree_status,
                DiffArea::Staged => &mut repo.staged_status,
            };
            let Loadable::Ready(entries) = status else {
                unreachable!()
            };
            Arc::make_mut(entries).retain(|entry| entry.path != Path::new(first));
            repo.worktree_status_rev += 1;
            repo.staged_status_rev += 1;
            apply_state(cx, &view, app_state_with_active_repo(repo));
            cx.update(|_window, app| {
                let pane = view.read(app).details_pane.read(app);
                assert_eq!(
                    pane.status_multi_selection.contains_key(&REPO),
                    leave_empty,
                    "{shortcut} must clear a consumed selection and retain a user-emptied selection"
                );
            });
            if section == StatusSection::CombinedUnstaged {
                assert_eq!(
                    cx.debug_bounds("stage_selected_button").is_some(),
                    !leave_empty,
                    "the next preview must become the fallback selection"
                );
            }
        }
    }
}

#[gpui::test]
fn status_select_all_on_empty_or_loading_sections_never_selects_diff_text(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (view, cx) = fixture(cx);
    for status in [Loadable::Ready(Arc::new(vec![])), Loadable::Loading] {
        let mut repo = status_repo();
        repo.staged_status = status;
        repo.staged_status_rev += 1;
        apply_state(cx, &view, app_state_with_active_repo(repo));
        click(cx, "staged_header".into(), gpui::Modifiers::default());
        cx.simulate_keystrokes("ctrl-a space ctrl-u");
        draw_and_drain_test_window(cx);
        assert!(selected(cx, &view, DiffArea::Staged).is_empty());
        cx.update(|_window, app| {
            assert!(!view.read(app).main_pane.read(app).diff_text_has_selection());
            assert_eq!(
                view.read(app).store.snapshot().repos[0].local_actions_in_flight,
                0
            );
        });
    }
}
