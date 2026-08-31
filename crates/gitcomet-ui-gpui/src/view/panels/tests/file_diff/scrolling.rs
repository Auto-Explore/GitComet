use super::*;

fn push_raw_patch_diff_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    unified: String,
) -> gitcomet_core::domain::DiffTarget {
    push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified, 1, true)
}

#[gpui::test]
fn split_file_diff_multiline_search_preserves_blank_side_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(9140);
    let path = PathBuf::from("src/split_search.rs");
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "split_search_blank_side_rows",
        path,
        "\
diff --git a/src/split_search.rs b/src/split_search.rs
index 1111111..2222222 100644
--- a/src/split_search.rs
+++ b/src/split_search.rs
@@ -1,2 +1,3 @@
 foo
+inserted
 bar
"
        .to_string(),
        "foo\nbar\n".to_string(),
        "foo\ninserted\nbar\n".to_string(),
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "split search file diff fixture activates",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.file_diff_split_row_len() == 3
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target.clone(),
                pane.file_diff_split_row_len(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_view = DiffViewMode::Split;
            pane.diff_search_active = true;

            pane.diff_search_query = "foo\nbar".into();
            pane.diff_search_recompute_matches();
            assert!(
                pane.diff_search_matches.is_empty(),
                "split search must not collapse a visible blank left cell between foo and bar"
            );

            pane.diff_search_query = "foo\n\nbar".into();
            pane.diff_search_recompute_matches();
            assert_eq!(
                pane.diff_search_matches.len(),
                1,
                "split search should match the visible left stream including the blank row"
            );
            let match_row = pane.diff_search_matches[0];
            assert_eq!(
                pane.diff_text_line_for_region(match_row, DiffTextRegion::SplitLeft)
                    .as_ref(),
                "foo"
            );
        });
    });
}

#[gpui::test]
fn short_split_diff_has_region_confined_below_eof_surfaces(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(9141);
    let path = PathBuf::from("src/split_below_eof.rs");
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "split_below_eof_surfaces",
        path,
        "\
diff --git a/src/split_below_eof.rs b/src/split_below_eof.rs
index 1111111..2222222 100644
--- a/src/split_below_eof.rs
+++ b/src/split_below_eof.rs
@@ -1,2 +1,2 @@
-old
+new
 tail
"
        .to_string(),
        "old\ntail\n".to_string(),
        "new\ntail\n".to_string(),
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "short split diff fixture activates",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            (
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target.clone(),
            )
        },
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Split;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let left = cx
        .debug_bounds("diff_text_empty_space_SplitLeft")
        .expect("short split diff left below-EOF surface");
    let right = cx
        .debug_bounds("diff_text_empty_space_SplitRight")
        .expect("short split diff right below-EOF surface");
    assert!(left.right() <= right.left());
    let left_row = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        0,
        DiffTextRegion::SplitLeft,
        0..3,
        "split left row drag target",
    );

    cx.simulate_mouse_down(right.center(), MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(left_row, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(left_row, MouseButton::Left, Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_anchor.map(|pos| pos.region),
            Some(DiffTextRegion::SplitRight)
        );
        assert_eq!(
            pane.diff_text_head.map(|pos| pos.region),
            Some(DiffTextRegion::SplitRight),
            "dragging into the other column must stay in the initiating region"
        );
    });
}

#[gpui::test]
fn diff_search_f3_continues_from_previous_location_after_patch_refresh(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(9138);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_search_refresh_position",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = std::path::PathBuf::from("src/search_refresh.rs");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let push_patch = |cx: &mut gpui::VisualTestContext, diff_rev: u64, unified: &str| {
        let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), unified);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_target = Some(target.clone());
                repo.diff_state.diff_state_rev = diff_rev;
                repo.diff_state.diff_rev = diff_rev;
                repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
    };

    let initial_unified = "\
diff --git a/src/search_refresh.rs b/src/search_refresh.rs
index 1111111..2222222 100644
--- a/src/search_refresh.rs
+++ b/src/search_refresh.rs
@@ -1,9 +1,9 @@
 context 0
-old first
+needle first
 context 1
-old second
+needle second
 context 2
-old current
+needle current
 context 3
-old next
+needle next
 context 4
";
    push_patch(cx, 1, initial_unified);
    wait_for_main_pane_condition(
        cx,
        &view,
        "initial patch diff for search refresh regression",
        |pane| pane.diff_cache_rev == 1 && pane.patch_diff_row_len() > 0,
        |pane| (pane.diff_cache_rev, pane.patch_diff_row_len()),
    );

    let previous_visible_ix = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle".into();
            pane.diff_search_recompute_matches();
            assert_eq!(
                pane.diff_search_matches.len(),
                4,
                "initial fixture should expose four search matches"
            );
            pane.diff_search_match_ix = Some(2);
            pane.diff_search_matches[2]
        })
    });

    let refreshed_unified = "\
diff --git a/src/search_refresh.rs b/src/search_refresh.rs
index 1111111..3333333 100644
--- a/src/search_refresh.rs
+++ b/src/search_refresh.rs
@@ -1,8 +1,8 @@
 context 0
-old first
+needle first
 context 1
-old second
+needle second
 context 2
 context 3
-old next
+needle next
 context 4
";
    push_patch(cx, 2, refreshed_unified);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.ensure_diff_visible_indices();
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "refreshed patch diff preserves search cursor before F3",
        |pane| pane.diff_cache_rev == 2 && pane.diff_search_matches.len() == 3,
        |pane| {
            (
                pane.diff_cache_rev,
                pane.diff_visible_len(),
                pane.diff_search_matches.clone(),
                pane.diff_search_match_ix,
            )
        },
    );

    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let match_ix = pane
            .diff_search_match_ix
            .expect("F3 should leave an active search match");
        let active_visible_ix = pane.diff_search_matches[match_ix];
        let active_text = pane
            .diff_text_line_for_region(active_visible_ix, DiffTextRegion::Inline)
            .to_string();

        assert!(
            active_visible_ix > previous_visible_ix,
            "F3 should continue after the pre-refresh match row, got previous={previous_visible_ix}, active={active_visible_ix}, matches={:?}",
            pane.diff_search_matches
        );
        assert!(
            active_text.contains("needle next"),
            "F3 should land on the next later remaining match, got {active_text:?}"
        );
    });
}

#[gpui::test]
fn diff_search_refresh_scrolls_to_first_match_after_previous_zero_match_query(
    cx: &mut gpui::TestAppContext,
) {
    fn unified_with_replacement(replacement: &str) -> String {
        let mut unified = "\
diff --git a/src/search_refresh_scroll.rs b/src/search_refresh_scroll.rs
index 1111111..2222222 100644
--- a/src/search_refresh_scroll.rs
+++ b/src/search_refresh_scroll.rs
@@ -1,72 +1,72 @@
"
        .to_string();

        for ix in 0..72 {
            if ix == 60 {
                unified.push_str("-old focus line\n");
                unified.push('+');
                unified.push_str(replacement);
                unified.push('\n');
            } else {
                unified.push_str(&format!(" context {ix}\n"));
            }
        }

        unified
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(9139);

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    push_raw_patch_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        "search_refresh_zero_previous_match",
        unified_with_replacement("fresh focus line"),
        1,
        true,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "initial no-match patch diff for search refresh regression",
        |pane| pane.diff_cache_rev == 1 && pane.patch_diff_row_len() > 0,
        |pane| (pane.diff_cache_rev, pane.patch_diff_row_len()),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle".into();
            pane.diff_search_recompute_matches();
            assert!(
                pane.diff_search_matches.is_empty(),
                "initial fixture should have no matches for the active query"
            );
            assert_eq!(pane.diff_search_match_ix, None);
        });
    });

    push_raw_patch_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        "search_refresh_zero_previous_match",
        unified_with_replacement("needle focus line"),
        2,
        true,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "refreshed patch diff scrolls to first new search match",
        |pane| {
            let first_match = pane.diff_search_matches.first().copied();
            pane.diff_cache_rev == 2
                && pane.diff_search_matches.len() == 1
                && pane.diff_search_match_ix == Some(0)
                && pane.diff_selection_anchor == first_match
                && pane.diff_selection_range == first_match.map(|ix| (ix, ix))
                && pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0)
                && pane.diff_scroll.0.borrow().base_handle.offset().y < px(-1.0)
        },
        |pane| {
            (
                pane.diff_cache_rev,
                pane.diff_visible_len(),
                pane.diff_search_matches.clone(),
                pane.diff_search_match_ix,
                pane.diff_selection_anchor,
                pane.diff_selection_range,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );
}

pub(super) fn push_raw_patch_diff_state_with_rev(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    unified: String,
    diff_rev: u64,
    ready: bool,
) -> gitcomet_core::domain::DiffTarget {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_raw_patch_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: gitcomet_core::domain::CommitId("feedface".into()),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = diff_rev;
            repo.diff_state.diff_rev = diff_rev;
            repo.diff_state.diff = if ready {
                gitcomet_state::model::Loadable::Ready(Arc::new(diff))
            } else {
                gitcomet_state::model::Loadable::Loading
            };
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    target
}

fn activate_full_file_diff_horizontal_scroll_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = if diff_view == DiffViewMode::Inline {
        build_full_file_inline_horizontal_scroll_fixture_texts()
    } else {
        build_collapsed_diff_horizontal_scroll_fixture_texts()
    };
    let target = push_regular_diff_content_mode_state(
        cx,
        view,
        repo_id,
        fixture_name,
        PathBuf::from("src/lib.rs"),
        unified,
        old_text,
        new_text,
    );

    set_diff_content_mode_for_test(cx, view, DiffContentMode::Full);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "full file diff horizontal overflow becomes available",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && match diff_view {
                    DiffViewMode::Inline => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    }
                    DiffViewMode::Split => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                            && pane
                                .diff_split_right_scroll
                                .0
                                .borrow()
                                .base_handle
                                .max_offset()
                                .x
                                > px(0.0)
                    }
                }
        },
        |pane| {
            format!(
                "file_diff_active={} target={:?} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );
}

fn push_working_tree_full_file_horizontal_scroll_fixture_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    area: gitcomet_core::domain::DiffArea,
    diff_view: DiffViewMode,
    diff_rev: u64,
    diff_file_rev: u64,
    patch_ready: bool,
    file_ready: bool,
) -> gitcomet_core::domain::DiffTarget {
    let (unified, old_text, new_text) = if diff_view == DiffViewMode::Inline {
        build_full_file_inline_horizontal_scroll_fixture_texts()
    } else {
        build_collapsed_diff_horizontal_scroll_fixture_texts()
    };
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_unstaged_full_file_hscroll_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);
    let file_diff =
        gitcomet_core::domain::FileDiffText::new(path.clone(), Some(old_text), Some(new_text));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                area,
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = diff_rev;
            repo.diff_state.diff_rev = diff_rev;
            repo.diff_state.diff = if patch_ready {
                gitcomet_state::model::Loadable::Ready(Arc::new(diff))
            } else {
                gitcomet_state::model::Loadable::Loading
            };
            repo.diff_state.diff_file_rev = diff_file_rev;
            repo.diff_state.diff_file = if file_ready {
                gitcomet_state::model::Loadable::Ready(Some(Arc::new(file_diff)))
            } else {
                gitcomet_state::model::Loadable::Loading
            };
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    target
}

fn activate_raw_patch_horizontal_scroll_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, _, _) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    let target = push_raw_patch_diff_state(cx, view, repo_id, fixture_name, unified);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "raw patch diff horizontal overflow becomes available",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && match diff_view {
                    DiffViewMode::Inline => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    }
                    DiffViewMode::Split => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                            && pane
                                .diff_split_right_scroll
                                .0
                                .borrow()
                                .base_handle
                                .max_offset()
                                .x
                                > px(0.0)
                    }
                }
        },
        |pane| {
            format!(
                "target={:?} file_diff_active={} patch_rows={} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.rendered_diff_target(),
                pane.is_file_diff_view_active(),
                pane.patch_diff_row_len(),
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );
}

fn assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(540.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(920.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });
    assert!(
        left_before_x < 0.0,
        "test setup should scroll the left/inline diff horizontally, got {left_before_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_x < 0.0,
            "test setup should scroll the split-right diff horizontally, got {right_before_x}"
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.invalidate_font_metrics(cx);
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                let mut state = handle.0.borrow_mut();
                state.last_item_size = None;
                let base_handle = state.base_handle.clone();
                drop(state);
                let offset = base_handle.offset();
                base_handle.set_offset(point(px(0.0), offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_after_x, right_after_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });

    assert!(
        left_after_x.abs() < 0.01,
        "unmeasured render should not force saved left/inline horizontal scroll back after the handle moves to zero (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_after_x.abs() < 0.01,
            "unmeasured render should not force saved split-right horizontal scroll back after the handle moves to zero (before={right_before_x}, after={right_after_x})"
        );
    }
}

fn assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(540.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(920.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });
    assert!(
        left_before_x < 0.0,
        "test setup should scroll the left/inline diff horizontally, got {left_before_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_x < 0.0,
            "test setup should scroll the split-right diff horizontally, got {right_before_x}"
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                handle.0.borrow_mut().last_item_size = None;
            }
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let (left_after_x, right_after_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });

    assert!(
        (left_after_x - left_before_x).abs() < 0.01,
        "first unmeasured render should not zero left/inline horizontal scroll (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_x - right_before_x).abs() < 0.01,
            "first unmeasured render should not zero split-right horizontal scroll (before={right_before_x}, after={right_after_x})"
        );
    }
}

fn assert_diff_horizontal_scroll_to_start_persists(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(540.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(920.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_scrolled_x, right_scrolled_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });
    assert!(
        left_scrolled_x < 0.0,
        "test setup should scroll the left/inline diff horizontally, got {left_scrolled_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_scrolled_x < 0.0,
            "test setup should scroll the split-right diff horizontally, got {right_scrolled_x}"
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                let base_handle = handle.0.borrow().base_handle.clone();
                let offset = base_handle.offset();
                base_handle.set_offset(point(px(0.0), offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |_pane, cx| cx.notify());
    });
    draw_and_drain_test_window(cx);

    let (left_after_x, right_after_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_x: f32 = pane.diff_scroll.0.borrow().base_handle.offset().x.into();
        let right_x: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .x
            .into();
        (left_x, right_x)
    });

    assert!(
        left_after_x.abs() < 0.01,
        "left/inline horizontal scroll should stay at start, got {left_after_x}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_after_x.abs() < 0.01,
            "split-right horizontal scroll should stay at start, got {right_after_x}"
        );
    }
}

fn assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    let (left_before_max, right_before_max) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_max: f32 = pane
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        let right_max: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        (left_max, right_max)
    });
    assert!(
        left_before_max > 0.0,
        "test setup should expose left/inline horizontal range, got {left_before_max}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_max > 0.0,
            "test setup should expose split-right horizontal range, got {right_before_max}"
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            for handle in [&pane.diff_scroll, &pane.diff_split_right_scroll] {
                handle.0.borrow_mut().last_item_size = None;
            }
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let (left_after_max, right_after_max) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_max: f32 = pane
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        let right_max: f32 = pane
            .diff_split_right_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        (left_max, right_max)
    });

    assert!(
        (left_after_max - left_before_max).abs() < 1.0,
        "left/inline horizontal range should not flicker across unmeasured render (before={left_before_max}, after={left_after_max})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_max - right_before_max).abs() < 1.0,
            "split-right horizontal range should not flicker across unmeasured render (before={right_before_max}, after={right_after_max})"
        );
    }
}

#[derive(Clone, Copy, Debug)]
struct DiffHorizontalScrollbarGeometry {
    label: &'static str,
    scrollbar_bounds: gpui::Bounds<Pixels>,
    viewport_bounds: gpui::Bounds<Pixels>,
    offset_x: f32,
}

fn pixel_delta(a: Pixels, b: Pixels) -> f32 {
    let delta: f32 = (a - b).into();
    delta.abs()
}

fn capture_diff_horizontal_scrollbar_geometry(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) -> Vec<DiffHorizontalScrollbarGeometry> {
    match diff_view {
        DiffViewMode::Inline => {
            let scrollbar_bounds = debug_selector_bounds(cx, "diff_hscrollbar");
            let (viewport_bounds, offset_x) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let state = pane.diff_scroll.0.borrow();
                let offset_x: f32 = state.base_handle.offset().x.into();
                (state.base_handle.bounds(), offset_x)
            });
            vec![DiffHorizontalScrollbarGeometry {
                label: "inline",
                scrollbar_bounds,
                viewport_bounds,
                offset_x,
            }]
        }
        DiffViewMode::Split => {
            let left_scrollbar_bounds = debug_selector_bounds(cx, "diff_split_left_hscrollbar");
            let right_scrollbar_bounds = debug_selector_bounds(cx, "diff_split_right_hscrollbar");
            let ((left_viewport_bounds, left_offset_x), (right_viewport_bounds, right_offset_x)) =
                cx.update(|_window, app| {
                    let pane = view.read(app).main_pane.read(app);
                    let left_state = pane.diff_scroll.0.borrow();
                    let left_offset_x: f32 = left_state.base_handle.offset().x.into();
                    let left = (left_state.base_handle.bounds(), left_offset_x);
                    drop(left_state);
                    let right_state = pane.diff_split_right_scroll.0.borrow();
                    let right_offset_x: f32 = right_state.base_handle.offset().x.into();
                    let right = (right_state.base_handle.bounds(), right_offset_x);
                    (left, right)
                });
            vec![
                DiffHorizontalScrollbarGeometry {
                    label: "split left",
                    scrollbar_bounds: left_scrollbar_bounds,
                    viewport_bounds: left_viewport_bounds,
                    offset_x: left_offset_x,
                },
                DiffHorizontalScrollbarGeometry {
                    label: "split right",
                    scrollbar_bounds: right_scrollbar_bounds,
                    viewport_bounds: right_viewport_bounds,
                    offset_x: right_offset_x,
                },
            ]
        }
    }
}

fn assert_scrollbar_geometry_matches_viewport(geometry: &[DiffHorizontalScrollbarGeometry]) {
    for sample in geometry {
        assert!(
            pixel_delta(
                sample.scrollbar_bounds.size.width,
                sample.viewport_bounds.size.width
            ) < 1.0,
            "{} horizontal scrollbar width should match viewport width (scrollbar={:?}, viewport={:?})",
            sample.label,
            sample.scrollbar_bounds,
            sample.viewport_bounds
        );
        assert!(
            pixel_delta(
                sample.scrollbar_bounds.left(),
                sample.viewport_bounds.left()
            ) < 1.0,
            "{} horizontal scrollbar left edge should match viewport left edge (scrollbar={:?}, viewport={:?})",
            sample.label,
            sample.scrollbar_bounds,
            sample.viewport_bounds
        );
        assert!(
            pixel_delta(
                sample.scrollbar_bounds.right(),
                sample.viewport_bounds.right()
            ) < 1.0,
            "{} horizontal scrollbar right edge should match viewport right edge (scrollbar={:?}, viewport={:?})",
            sample.label,
            sample.scrollbar_bounds,
            sample.viewport_bounds
        );
    }
}

fn assert_scrollbar_geometry_stays_stable(
    before: &[DiffHorizontalScrollbarGeometry],
    after: &[DiffHorizontalScrollbarGeometry],
) {
    assert_eq!(
        before.len(),
        after.len(),
        "geometry capture should keep the same number of scrollbars"
    );
    for (before, after) in before.iter().zip(after.iter()) {
        assert_eq!(before.label, after.label);
        assert!(
            pixel_delta(
                before.scrollbar_bounds.left(),
                after.scrollbar_bounds.left()
            ) < 1.0,
            "{} horizontal scrollbar left edge should not move across unmeasured render (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.right(),
                after.scrollbar_bounds.right()
            ) < 1.0,
            "{} horizontal scrollbar right edge should not move across unmeasured render (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.size.width,
                after.scrollbar_bounds.size.width
            ) < 1.0,
            "{} horizontal scrollbar width should not change across unmeasured render (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            (after.offset_x - before.offset_x).abs() < 0.01,
            "{} horizontal offset should not change across unmeasured render (before={}, after={})",
            before.label,
            before.offset_x,
            after.offset_x
        );
    }
}

fn assert_scrollbar_bounds_stay_stable(
    before: &[DiffHorizontalScrollbarGeometry],
    after: &[DiffHorizontalScrollbarGeometry],
) {
    assert_eq!(
        before.len(),
        after.len(),
        "geometry capture should keep the same number of scrollbars"
    );
    for (before, after) in before.iter().zip(after.iter()) {
        assert_eq!(before.label, after.label);
        assert!(
            pixel_delta(
                before.scrollbar_bounds.left(),
                after.scrollbar_bounds.left()
            ) < 1.0,
            "{} horizontal scrollbar left edge should not move (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.right(),
                after.scrollbar_bounds.right()
            ) < 1.0,
            "{} horizontal scrollbar right edge should not move (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
        assert!(
            pixel_delta(
                before.scrollbar_bounds.size.width,
                after.scrollbar_bounds.size.width
            ) < 1.0,
            "{} horizontal scrollbar width should not change (before={:?}, after={:?})",
            before.label,
            before.scrollbar_bounds,
            after.scrollbar_bounds
        );
    }
}

fn assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
    sync_mode: DiffScrollSync,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, sync_mode);
    }

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "diff horizontal overflow is available before geometry capture",
        |pane| {
            let left_max = pane.diff_scroll.0.borrow().base_handle.max_offset();
            let left_overflows = left_max.x > px(0.0);
            if diff_view == DiffViewMode::Inline {
                return left_overflows;
            }
            left_overflows
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x
                    > px(0.0)
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(240.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(360.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let before = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    assert_scrollbar_geometry_matches_viewport(&before);
    for sample in &before {
        assert!(
            sample.offset_x < 0.0,
            "{} test setup should start with a horizontal offset, got {}",
            sample.label,
            sample.offset_x
        );
    }

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.invalidate_font_metrics(cx);
            pane.diff_scroll.0.borrow_mut().last_item_size = None;
            pane.diff_split_right_scroll.0.borrow_mut().last_item_size = None;
        });
    });
    draw_and_drain_test_window(cx);

    let after = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    assert_scrollbar_geometry_matches_viewport(&after);
    assert_scrollbar_geometry_stays_stable(&before, &after);
}

fn assert_diff_horizontal_scrollbar_drag_keeps_range_stable(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    diff_view: DiffViewMode,
) {
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "diff horizontal overflow is available before scrollbar drag",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                && (diff_view == DiffViewMode::Inline
                    || pane
                        .diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x
                        > px(0.0))
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let before = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    let (before_content_width, before_max_offset) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let column = crate::view::panes::main::DiffHorizontalScrollColumn::Primary;
        let content_width: f32 = pane.diff_horizontal_content_width_for_column(column).into();
        let max_offset: f32 = pane
            .diff_scroll
            .0
            .borrow()
            .base_handle
            .max_offset()
            .x
            .into();
        (content_width, max_offset)
    });
    assert!(
        before_max_offset > 0.0,
        "test setup should expose horizontal range before drag, got {before_max_offset}"
    );

    let scrollbar_bounds = before[0].scrollbar_bounds;
    let start = point(
        scrollbar_bounds.left() + px(12.0),
        scrollbar_bounds.center().y,
    );
    let end = point(
        (start.x + px(80.0)).min(scrollbar_bounds.right() - px(12.0)),
        start.y,
    );
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
    draw_and_drain_test_window(cx);
    cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
    draw_and_drain_test_window(cx);

    let after = capture_diff_horizontal_scrollbar_geometry(cx, view, diff_view);
    assert_scrollbar_bounds_stay_stable(&before, &after);

    let (after_content_width, after_max_offset, after_offset_x) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let column = crate::view::panes::main::DiffHorizontalScrollColumn::Primary;
        let content_width: f32 = pane.diff_horizontal_content_width_for_column(column).into();
        let state = pane.diff_scroll.0.borrow();
        let max_offset: f32 = state.base_handle.max_offset().x.into();
        let offset_x: f32 = state.base_handle.offset().x.into();
        (content_width, max_offset, offset_x)
    });

    assert!(
        (after_content_width - before_content_width).abs() < 1.0,
        "dragging the horizontal scrollbar should not change measured content width (before={before_content_width}, after={after_content_width})"
    );
    assert!(
        (after_max_offset - before_max_offset).abs() < 1.0,
        "dragging the horizontal scrollbar should not change horizontal range (before={before_max_offset}, after={after_max_offset})"
    );
    assert!(
        after_offset_x < 0.0,
        "dragging the horizontal scrollbar should move horizontally, got {after_offset_x}"
    );
}

#[gpui::test]
fn diff_vertical_scrollbar_gutter_stays_reserved_when_unmeasured(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(287),
        "diff_vertical_scrollbar_gutter_stays_reserved_when_unmeasured",
        DiffViewMode::Split,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
            pane.diff_scroll.0.borrow_mut().last_item_size = None;
            pane.diff_split_right_scroll.0.borrow_mut().last_item_size = None;

            let left_gutter = pane.diff_vertical_scrollbar_gutter_for_column(
                crate::view::panes::main::DiffHorizontalScrollColumn::Primary,
                pane.diff_scroll.clone(),
            );
            let right_gutter = pane.diff_vertical_scrollbar_gutter_for_column(
                crate::view::panes::main::DiffHorizontalScrollColumn::SplitRight,
                pane.diff_split_right_scroll.clone(),
            );

            assert_eq!(
                left_gutter, gutter,
                "unmeasured primary diff should keep reserved vertical gutter"
            );
            assert_eq!(
                right_gutter, gutter,
                "unmeasured split-right diff should keep reserved vertical gutter"
            );
        });
    });
}

fn assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    area: gitcomet_core::domain::DiffArea,
    diff_view: DiffViewMode,
    sync_mode: DiffScrollSync,
) {
    let target = push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        view,
        repo_id,
        fixture_name,
        area,
        diff_view,
        1,
        1,
        true,
        true,
    );
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, sync_mode);
    }
    set_diff_content_mode_for_test(cx, view, DiffContentMode::Full);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "unstaged full-file diff horizontal overflow becomes available",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.file_diff_cache_inflight.is_none()
                && match diff_view {
                    DiffViewMode::Inline => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    }
                    DiffViewMode::Split => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                            && pane
                                .diff_split_right_scroll
                                .0
                                .borrow()
                                .base_handle
                                .max_offset()
                                .x
                                > px(0.0)
                    }
                }
        },
        |pane| {
            format!(
                "active={} target={:?} inflight={:?} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
                pane.file_diff_cache_inflight,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(360.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(540.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x, left_before_max, right_before_max, seq_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
                pane.file_diff_cache_seq,
            )
        });
    assert!(
        left_before_x < px(0.0),
        "test setup should scroll the left/inline diff horizontally, got {left_before_x:?}"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            right_before_x < px(0.0),
            "test setup should scroll split-right horizontally, got {right_before_x:?}"
        );
    }

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        view,
        repo_id,
        fixture_name,
        area,
        diff_view,
        2,
        2,
        false,
        false,
    );
    draw_and_drain_test_window(cx);

    let assert_stable = |cx: &mut gpui::VisualTestContext, label: &str, expected_rev: u64| {
        let (left_x, right_x, left_max, right_max, seq, cache_rev, active) =
            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.diff_scroll.0.borrow().base_handle.offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .offset()
                        .x,
                    pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x,
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_rev,
                    pane.is_file_diff_view_active(),
                )
            });
        assert!(active, "{label}: file diff view should remain active");
        assert_eq!(
            cache_rev, expected_rev,
            "{label}: same-target cache rev should track the active refresh"
        );
        assert_eq!(
            seq, seq_before,
            "{label}: same-content refresh should not rebuild the file-diff cache"
        );
        assert!(
            (left_x - left_before_x).abs() < px(0.01),
            "{label}: left/inline horizontal offset should stay stable (before={left_before_x:?}, after={left_x:?})"
        );
        assert!(
            (left_max - left_before_max).abs() < px(1.0),
            "{label}: left/inline horizontal range should stay stable (before={left_before_max:?}, after={left_max:?})"
        );
        if diff_view == DiffViewMode::Split {
            assert!(
                (right_x - right_before_x).abs() < px(0.01),
                "{label}: split-right horizontal offset should stay stable (before={right_before_x:?}, after={right_x:?})"
            );
            assert!(
                (right_max - right_before_max).abs() < px(1.0),
                "{label}: split-right horizontal range should stay stable (before={right_before_max:?}, after={right_max:?})"
            );
        }
    };
    assert_stable(cx, "same-target loading redraw", 2);

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        view,
        repo_id,
        fixture_name,
        area,
        diff_view,
        2,
        2,
        true,
        true,
    );
    draw_and_drain_test_window(cx);
    assert_stable(cx, "same-target ready redraw", 2);
}

#[gpui::test]
fn full_file_diff_inline_unstaged_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(912),
        "full_file_inline_unstaged_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn full_file_diff_split_unstaged_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(913),
        "full_file_split_unstaged_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn full_file_diff_split_staged_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(914),
        "full_file_split_staged_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Staged,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn full_file_diff_split_vertical_sync_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_working_tree_full_file_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(915),
        "full_file_split_vertical_sync_same_target_loading_hscroll",
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

fn assert_raw_patch_diff_horizontal_scroll_stable_across_same_target_loading(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
    sync_mode: DiffScrollSync,
) {
    let (unified, _, _) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    let target = push_raw_patch_diff_state_with_rev(
        cx,
        view,
        repo_id,
        fixture_name,
        unified.clone(),
        1,
        true,
    );
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, sync_mode);
    }
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "raw patch horizontal overflow becomes available before same-target loading",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && !pane.is_file_diff_view_active()
                && pane.diff_cache_rev == 1
                && pane.patch_diff_row_len() > 0
                && match diff_view {
                    DiffViewMode::Inline => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    }
                    DiffViewMode::Split => {
                        pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                            && pane
                                .diff_split_right_scroll
                                .0
                                .borrow()
                                .base_handle
                                .max_offset()
                                .x
                                > px(0.0)
                    }
                }
        },
        |pane| {
            format!(
                "target={:?} cache_rev={} rows={} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.rendered_diff_target(),
                pane.diff_cache_rev,
                pane.patch_diff_row_len(),
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(360.0)), left_offset.y));

            if diff_view == DiffViewMode::Split {
                let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
                let right_offset = right_handle.offset();
                let right_max = right_handle.max_offset();
                right_handle.set_offset(point(-right_max.x.min(px(540.0)), right_offset.y));
            }
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x, left_before_max, right_before_max) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
            )
        });
    assert!(left_before_x < px(0.0));
    if diff_view == DiffViewMode::Split {
        assert!(right_before_x < px(0.0));
    }

    let assert_stable = |cx: &mut gpui::VisualTestContext, label: &str| {
        let (left_x, right_x, left_max, right_max, cache_rev, rows) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
                pane.diff_cache_rev,
                pane.patch_diff_row_len(),
            )
        });
        assert_eq!(cache_rev, 2, "{label}: raw patch cache rev should advance");
        assert!(rows > 0, "{label}: raw patch rows should remain cached");
        assert!(
            (left_x - left_before_x).abs() < px(0.01),
            "{label}: left/inline offset should remain stable"
        );
        assert!(
            (left_max - left_before_max).abs() < px(1.0),
            "{label}: left/inline range should remain stable"
        );
        if diff_view == DiffViewMode::Split {
            assert!(
                (right_x - right_before_x).abs() < px(0.01),
                "{label}: split-right offset should remain stable"
            );
            assert!(
                (right_max - right_before_max).abs() < px(1.0),
                "{label}: split-right range should remain stable"
            );
        }
    };

    push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified.clone(), 2, false);
    draw_and_drain_test_window(cx);
    assert_stable(cx, "raw patch same-target loading redraw");

    push_raw_patch_diff_state_with_rev(cx, view, repo_id, fixture_name, unified, 2, true);
    draw_and_drain_test_window(cx);
    assert_stable(cx, "raw patch same-target ready redraw");
}

#[gpui::test]
fn raw_patch_inline_same_target_loading_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_raw_patch_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(916),
        "raw_patch_inline_same_target_loading_hscroll",
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn raw_patch_split_vertical_sync_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_raw_patch_diff_horizontal_scroll_stable_across_same_target_loading(
        cx,
        &view,
        gitcomet_state::model::RepoId(917),
        "raw_patch_split_vertical_sync_same_target_loading_hscroll",
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

#[gpui::test]
fn collapsed_diff_split_same_target_loading_preserves_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(918);
    let fixture_name = "collapsed_split_same_target_loading_hscroll";
    let target = push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        &view,
        repo_id,
        fixture_name,
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        1,
        1,
        true,
        true,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Split;
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split horizontal overflow becomes available before loading refresh",
        |pane| {
            pane.rendered_diff_target() == Some(&target)
                && pane.is_collapsed_diff_projection_active()
                && pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x
                    > px(0.0)
        },
        |pane| {
            format!(
                "collapsed_active={} diff_rev={} file_rev={} left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_cache_rev,
                pane.file_diff_cache_rev,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let left_handle = pane.diff_scroll.0.borrow().base_handle.clone();
            let left_offset = left_handle.offset();
            let left_max = left_handle.max_offset();
            left_handle.set_offset(point(-left_max.x.min(px(360.0)), left_offset.y));

            let right_handle = pane.diff_split_right_scroll.0.borrow().base_handle.clone();
            let right_offset = right_handle.offset();
            let right_max = right_handle.max_offset();
            right_handle.set_offset(point(-right_max.x.min(px(540.0)), right_offset.y));
        });
    });
    draw_and_drain_test_window(cx);

    let (left_before_x, right_before_x, left_before_max, right_before_max) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_scroll.0.borrow().base_handle.offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .offset()
                    .x,
                pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x,
            )
        });
    assert!(left_before_x < px(0.0));
    assert!(right_before_x < px(0.0));

    let assert_stable = |cx: &mut gpui::VisualTestContext, label: &str| {
        let (left_x, right_x, left_max, right_max, diff_rev, file_rev, active) =
            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.diff_scroll.0.borrow().base_handle.offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .offset()
                        .x,
                    pane.diff_scroll.0.borrow().base_handle.max_offset().x,
                    pane.diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x,
                    pane.diff_cache_rev,
                    pane.file_diff_cache_rev,
                    pane.is_collapsed_diff_projection_active(),
                )
            });
        assert!(
            active,
            "{label}: collapsed projection should remain cache-active"
        );
        assert_eq!(diff_rev, 2, "{label}: patch cache rev should advance");
        assert_eq!(file_rev, 2, "{label}: file cache rev should advance");
        assert!(
            (left_x - left_before_x).abs() < px(0.01),
            "{label}: split-left offset should remain stable"
        );
        assert!(
            (right_x - right_before_x).abs() < px(0.01),
            "{label}: split-right offset should remain stable"
        );
        assert!(
            (left_max - left_before_max).abs() < px(1.0),
            "{label}: split-left range should remain stable"
        );
        assert!(
            (right_max - right_before_max).abs() < px(1.0),
            "{label}: split-right range should remain stable"
        );
    };

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        &view,
        repo_id,
        fixture_name,
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        2,
        2,
        false,
        false,
    );
    draw_and_drain_test_window(cx);
    assert_stable(cx, "collapsed same-target loading redraw");

    push_working_tree_full_file_horizontal_scroll_fixture_state(
        cx,
        &view,
        repo_id,
        fixture_name,
        gitcomet_core::domain::DiffArea::Unstaged,
        DiffViewMode::Split,
        2,
        2,
        true,
        true,
    );
    draw_and_drain_test_window(cx);
    assert_stable(cx, "collapsed same-target ready redraw");
}

#[gpui::test]
fn raw_patch_inline_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(902),
        "raw_patch_inline_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn raw_patch_split_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(903),
        "raw_patch_split_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn raw_patch_split_vertical_sync_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(904),
        "raw_patch_split_vertical_sync_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

#[gpui::test]
fn raw_patch_inline_horizontal_scrollbar_drag_keeps_range_stable(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(909),
        "raw_patch_inline_hscrollbar_drag_range_stable",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scrollbar_drag_keeps_range_stable(cx, &view, DiffViewMode::Inline);
}

#[gpui::test]
fn collapsed_diff_split_horizontal_scrollbar_drag_keeps_range_stable(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(910),
        "collapsed_split_hscrollbar_drag_range_stable",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_drag_keeps_range_stable(cx, &view, DiffViewMode::Split);
}

#[gpui::test]
fn collapsed_diff_inline_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(905),
        "collapsed_inline_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn collapsed_diff_split_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(906),
        "collapsed_split_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::None,
    );
}

#[gpui::test]
fn collapsed_diff_split_vertical_sync_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(907),
        "collapsed_split_vertical_sync_hscrollbar_bounds_stable_unmeasured",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    assert_diff_horizontal_scrollbar_bounds_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
        DiffScrollSync::Vertical,
    );
}

#[gpui::test]
fn collapsed_diff_split_scroll_sync_setting_controls_each_axis(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let (unified, old_text, new_text) = build_collapsed_diff_scroll_sync_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(908),
        "collapsed_split_scroll_sync_setting_controls_each_axis",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split diff exposes horizontal and vertical scroll ranges",
        |pane| {
            let left_max = uniform_list_max_offset(&pane.diff_scroll);
            let right_max = uniform_list_max_offset(&pane.diff_split_right_scroll);
            left_max.width > px(40.0)
                && right_max.width > px(40.0)
                && left_max.height > px(120.0)
                && right_max.height > px(120.0)
        },
        |pane| {
            format!(
                "left_offset={:?} right_offset={:?} left_max={:?} right_max={:?}",
                uniform_list_offset(&pane.diff_scroll),
                uniform_list_offset(&pane.diff_split_right_scroll),
                uniform_list_max_offset(&pane.diff_scroll),
                uniform_list_max_offset(&pane.diff_split_right_scroll),
            )
        },
    );

    for mode in ALL_DIFF_SCROLL_SYNC_MODES {
        set_diff_scroll_sync_for_test(cx, &view, mode);

        for axis in ScrollSyncAxis::ALL {
            let scrolled = axis.offset(px(40.0));
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    reset_uniform_list_offsets(&[&pane.diff_scroll, &pane.diff_split_right_scroll]);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    set_uniform_list_offset(&pane.diff_scroll, scrolled);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let expected_right = if axis.includes(mode) {
                    axis.component(scrolled)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_scroll)),
                    axis.component(scrolled),
                    "collapsed split left should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_split_right_scroll)),
                    expected_right,
                    "collapsed split right should {} {} scrolling from the left column in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });

            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    reset_uniform_list_offsets(&[&pane.diff_scroll, &pane.diff_split_right_scroll]);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);
            cx.update(|_window, app| {
                let main_pane = view.read(app).main_pane.clone();
                main_pane.update(app, |pane, cx| {
                    set_uniform_list_offset(&pane.diff_split_right_scroll, scrolled);
                    cx.notify();
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let expected_left = if axis.includes(mode) {
                    axis.component(scrolled)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_split_right_scroll)),
                    axis.component(scrolled),
                    "collapsed split right should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.diff_scroll)),
                    expected_left,
                    "collapsed split left should {} {} scrolling from the right column in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });
        }
    }
}

#[gpui::test]
fn full_file_diff_inline_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(271),
        "full_file_inline_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_file_diff_split_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(272),
        "full_file_split_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn raw_patch_inline_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(273),
        "raw_patch_inline_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn raw_patch_split_unmeasured_render_does_not_force_horizontal_scroll_restore(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(274),
        "raw_patch_split_unmeasured_render_no_forced_hscroll_restore",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_does_not_force_horizontal_scroll_restore(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_file_diff_inline_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(275),
        "full_file_inline_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_file_diff_split_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(276),
        "full_file_split_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn raw_patch_inline_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(277),
        "raw_patch_inline_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Inline,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn raw_patch_split_unmeasured_render_does_not_zero_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(278),
        "raw_patch_split_unmeasured_render_does_not_zero_hscroll",
        DiffViewMode::Split,
    );
    assert_diff_unmeasured_render_keeps_horizontal_scroll_without_zero_frame(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_file_diff_inline_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(279),
        "full_file_inline_scroll_to_start_persists",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Inline);
}

#[gpui::test]
fn full_file_diff_split_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(280),
        "full_file_split_scroll_to_start_persists",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Split);
}

#[gpui::test]
fn raw_patch_inline_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(281),
        "raw_patch_inline_scroll_to_start_persists",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Inline);
}

#[gpui::test]
fn raw_patch_split_scroll_to_start_persists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(282),
        "raw_patch_split_scroll_to_start_persists",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_to_start_persists(cx, &view, DiffViewMode::Split);
}

#[gpui::test]
fn full_file_diff_inline_horizontal_range_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(283),
        "full_file_inline_horizontal_range_stable_unmeasured",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_file_diff_split_horizontal_range_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_full_file_diff_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(284),
        "full_file_split_horizontal_range_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn raw_patch_inline_horizontal_range_stable_across_unmeasured_render(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(285),
        "raw_patch_inline_horizontal_range_stable_unmeasured",
        DiffViewMode::Inline,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn raw_patch_split_horizontal_range_stable_across_unmeasured_render(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    activate_raw_patch_horizontal_scroll_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(286),
        "raw_patch_split_horizontal_range_stable_unmeasured",
        DiffViewMode::Split,
    );
    assert_diff_horizontal_scroll_range_stable_across_unmeasured_render(
        cx,
        &view,
        DiffViewMode::Split,
    );
}

fn assert_collapsed_diff_trailing_down_button_clickable_above_hscrollbar(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_trailing_hscroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        unified,
        old_text,
        new_text,
    );
    if diff_view == DiffViewMode::Split {
        set_diff_scroll_sync_for_test(cx, view, DiffScrollSync::None);
    }

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed trailing hunk horizontal overflow becomes available",
        |pane| match diff_view {
            DiffViewMode::Inline => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
            }
            DiffViewMode::Split => {
                pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
                    && pane
                        .diff_split_right_scroll
                        .0
                        .borrow()
                        .base_handle
                        .max_offset()
                        .x
                        > px(0.0)
            }
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let (hunk_src_ix, trailing_visible_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let trailing_visible_ix = pane
            .diff_visible_len()
            .checked_sub(1)
            .expect("expected at least one collapsed diff row");
        match pane.collapsed_visible_row(trailing_visible_ix) {
            Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                src_ix,
                expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                display_src_ix: None,
                hidden_rows,
            }) => (src_ix, trailing_visible_ix, hidden_rows),
            row => panic!("expected trailing collapsed down hunk header, got {row:?}"),
        }
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.scroll_diff_to_item_strict(trailing_visible_ix, gpui::ScrollStrategy::Bottom);
        });
    });
    draw_and_drain_test_window(cx);

    let down_selector = match diff_view {
        DiffViewMode::Inline => "collapsed_diff_inline_hunk_down",
        DiffViewMode::Split => "collapsed_diff_split_left_hunk_down",
    };
    let down_bounds = cx
        .debug_bounds(down_selector)
        .unwrap_or_else(|| panic!("expected `{down_selector}` bounds"));
    let scrollbar_top = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_scroll.0.borrow().base_handle.bounds().bottom()
            - components::Scrollbar::gutter(components::ScrollbarAxis::Horizontal)
    });
    assert!(
        down_bounds.center().y < scrollbar_top,
        "collapsed trailing down button center should be above the horizontal scrollbar (button={down_bounds:?}, scrollbar_top={scrollbar_top:?})"
    );

    simulate_counted_click(cx, down_bounds.center(), 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix) < hidden_down_before,
            "clicking the trailing collapsed hunk down button should reveal context"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
    });
}

#[gpui::test]
fn collapsed_diff_inline_trailing_down_button_stays_above_hscrollbar(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_trailing_down_button_clickable_above_hscrollbar(
        cx,
        &view,
        gitcomet_state::model::RepoId(262),
        "collapsed_inline_trailing_hscroll",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_trailing_down_button_stays_above_hscrollbar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_trailing_down_button_clickable_above_hscrollbar(
        cx,
        &view,
        gitcomet_state::model::RepoId(263),
        "collapsed_split_trailing_hscroll",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn collapsed_diff_inline_reveal_buttons_expand_context_without_creating_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(193);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_inline_buttons",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, visible_before, hidden_up_before, hidden_down_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let hunk = pane
                .collapsed_diff_hunks
                .first()
                .copied()
                .expect("expected collapsed inline fixture to expose one hunk");
            (
                hunk.src_ix,
                pane.diff_visible_len(),
                pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
                pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
            )
        });

    assert!(
        hidden_up_before >= 20 && hidden_down_before >= 20,
        "fixture should expose enough hidden context for inline reveal buttons"
    );

    let up_click = debug_selector_center(cx, "collapsed_diff_inline_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 20,
            "clicking the inline reveal-up gutter button should add 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            hidden_up_before - 20,
            "clicking the inline reveal-up gutter button should reduce the hidden-up budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    let down_click = debug_selector_center(cx, "collapsed_diff_inline_hunk_down");
    simulate_counted_click(cx, down_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 40,
            "clicking the inline reveal-down gutter button should add another 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before - 20,
            "clicking the inline reveal-down gutter button should reduce the hidden-down budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });
}

#[gpui::test]
fn collapsed_diff_split_reveal_buttons_expand_context_without_creating_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(194);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_buttons",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, visible_before, hidden_up_before, hidden_down_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let hunk = pane
                .collapsed_diff_hunks
                .first()
                .copied()
                .expect("expected collapsed split fixture to expose one hunk");
            (
                hunk.src_ix,
                pane.diff_visible_len(),
                pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
                pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
            )
        });

    assert!(
        hidden_up_before >= 20 && hidden_down_before >= 20,
        "fixture should expose enough hidden context for split reveal buttons"
    );

    let up_click = debug_selector_center(cx, "collapsed_diff_split_left_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 20,
            "clicking the split reveal-up gutter button should add 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            hidden_up_before - 20,
            "clicking the split reveal-up gutter button should reduce the hidden-up budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    let down_click = debug_selector_center(cx, "collapsed_diff_split_left_hunk_down");
    simulate_counted_click(cx, down_click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 40,
            "clicking the split reveal-down gutter button should add another 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before - 20,
            "clicking the split reveal-down gutter button should reduce the hidden-down budget by 20 rows"
        );
        assert_eq!(pane.diff_selection_anchor, None);
        assert_eq!(pane.diff_selection_range, None);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });
}

#[gpui::test]
fn collapsed_diff_split_reveal_arrows_show_directional_tooltips(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(203);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_reveal_arrow_tooltips",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );

    let up_hover = debug_selector_center(cx, "collapsed_diff_split_left_hunk_up");
    cx.simulate_mouse_move(up_hover, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Show hidden lines above".into())
    );

    let down_hover = debug_selector_center(cx, "collapsed_diff_split_left_hunk_down");
    cx.simulate_mouse_move(down_hover, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Show hidden lines below".into())
    );
}

#[gpui::test]
fn collapsed_diff_inline_up_reveal_keeps_header_above_revealed_context(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(199);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_inline_anchor_up",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);

    let (hunk_src_ix, hunk_base_row_start) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .first()
            .map(|hunk| (hunk.src_ix, hunk.base_row_start))
            .expect("expected collapsed inline fixture to expose one hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed inline reveal-up anchor becomes scrollable",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    let hunk_visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, hunk_visible_ix);

    let scroll_y_before = diff_scroll_offset_y(cx, &view);
    let header_top_before =
        diff_text_hitbox_top_for_visible_ix(cx, &view, hunk_visible_ix, DiffTextRegion::Inline);
    let hunk_first_visible_ix_before = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });

    let up_click = debug_selector_center(cx, "collapsed_diff_inline_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    let hunk_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix);
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader { .. })
            ),
            "expected the hunk header to remain visible after a partial upward reveal"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix + 1),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix })
                    if row_ix < hunk_base_row_start
            ),
            "expected newly revealed upward context to appear below the collapsed hunk header"
        );
        visible_ix
    });
    let hunk_first_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });
    let header_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        hunk_visible_ix_after,
        DiffTextRegion::Inline,
    );
    let scroll_y_after = diff_scroll_offset_y(cx, &view);

    assert!(
        (scroll_y_after - scroll_y_before).abs() < 0.01,
        "expected reveal-up to keep the inline diff scroll offset unchanged (before={scroll_y_before}, after={scroll_y_after})"
    );
    assert_eq!(
        hunk_visible_ix_after, hunk_visible_ix,
        "expected the collapsed inline hunk header to stay at the hidden-context boundary"
    );
    assert!(
        (header_top_after - header_top_before).abs() < 0.01,
        "expected the collapsed inline hunk header to remain visually fixed while revealed context is inserted below it (before={header_top_before}, after={header_top_after})"
    );
    assert!(
        hunk_first_visible_ix_after > hunk_first_visible_ix_before,
        "expected the hunk body to move down below newly revealed upward context (before={hunk_first_visible_ix_before}, after={hunk_first_visible_ix_after})"
    );
}

#[gpui::test]
fn collapsed_diff_split_up_reveal_keeps_header_above_revealed_context(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(202);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_anchor_up",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    draw_and_drain_test_window(cx);
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    let (hunk_src_ix, hunk_base_row_start) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .first()
            .map(|hunk| (hunk.src_ix, hunk.base_row_start))
            .expect("expected collapsed split fixture to expose one hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split reveal-up anchor becomes scrollable",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0)
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .y
                    > px(0.0)
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let hunk_visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, hunk_visible_ix);

    let left_scroll_y_before = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_before = diff_split_right_scroll_offset_y(cx, &view);
    let left_top_before =
        diff_text_hitbox_top_for_visible_ix(cx, &view, hunk_visible_ix, DiffTextRegion::SplitLeft);
    let right_top_before =
        diff_text_hitbox_top_for_visible_ix(cx, &view, hunk_visible_ix, DiffTextRegion::SplitRight);
    let hunk_first_visible_ix_before = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });

    let up_click = debug_selector_center(cx, "collapsed_diff_split_left_hunk_up");
    simulate_counted_click(cx, up_click, 1);
    draw_and_drain_test_window(cx);

    let hunk_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, hunk_src_ix);
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader { .. })
            ),
            "expected the split hunk header to remain visible after a partial upward reveal"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix + 1),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix })
                    if row_ix < hunk_base_row_start
            ),
            "expected newly revealed split upward context to appear below the collapsed hunk header"
        );
        visible_ix
    });
    let hunk_first_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, hunk_base_row_start)
    });
    let left_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        hunk_visible_ix_after,
        DiffTextRegion::SplitLeft,
    );
    let right_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        hunk_visible_ix_after,
        DiffTextRegion::SplitRight,
    );
    let left_scroll_y_after = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_after = diff_split_right_scroll_offset_y(cx, &view);

    assert!(
        (left_scroll_y_after - left_scroll_y_before).abs() < 0.01,
        "expected reveal-up to keep the split-left scroll offset unchanged (before={left_scroll_y_before}, after={left_scroll_y_after})"
    );
    assert!(
        (right_scroll_y_after - right_scroll_y_before).abs() < 0.01,
        "expected reveal-up to keep the split-right scroll offset unchanged (before={right_scroll_y_before}, after={right_scroll_y_after})"
    );
    assert_eq!(
        hunk_visible_ix_after, hunk_visible_ix,
        "expected the collapsed split hunk header to stay at the hidden-context boundary"
    );
    assert!(
        (left_top_after - left_top_before).abs() < 0.01,
        "expected the split-left collapsed hunk header to remain visually fixed while revealed context is inserted below it (before={left_top_before}, after={left_top_after})"
    );
    assert!(
        (right_top_after - right_top_before).abs() < 0.01,
        "expected the split-right collapsed hunk header to remain visually fixed while revealed context is inserted below it (before={right_top_before}, after={right_top_after})"
    );
    assert!(
        hunk_first_visible_ix_after > hunk_first_visible_ix_before,
        "expected the split hunk body to move down below newly revealed upward context (before={hunk_first_visible_ix_before}, after={hunk_first_visible_ix_after})"
    );
}

#[gpui::test]
fn collapsed_diff_split_down_before_reveal_moves_both_columns_without_vertical_sync(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(200);
    let (unified, old_text, new_text) = build_collapsed_diff_long_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_anchor_down_before",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    set_diff_scroll_sync_for_test(cx, &view, DiffScrollSync::None);

    let second_hunk_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .get(1)
            .map(|hunk| hunk.src_ix)
            .expect("expected long-gap fixture to expose a second collapsed hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, second_hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed split down-before anchor becomes scrollable",
        |pane| {
            pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0)
                && pane
                    .diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .y
                    > px(0.0)
        },
        |pane| {
            format!(
                "left_offset={:?} left_max={:?} right_offset={:?} right_max={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
                pane.diff_split_right_scroll.0.borrow().base_handle.offset(),
                pane.diff_split_right_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset(),
            )
        },
    );

    let target_visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_hunk_visible_ix_for_src_ix(pane, second_hunk_src_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, target_visible_ix);

    let left_scroll_y_before = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_before = diff_split_right_scroll_offset_y(cx, &view);
    let left_top_before = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix,
        DiffTextRegion::SplitLeft,
    );
    let right_top_before = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix,
        DiffTextRegion::SplitRight,
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down_before(second_hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let target_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, second_hunk_src_ix);
        assert!(
            matches!(
                pane.collapsed_visible_row(visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader { .. })
            ),
            "expected the second collapsed hunk header to remain visible after a partial down-before reveal"
        );
        visible_ix
    });
    let left_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix_after,
        DiffTextRegion::SplitLeft,
    );
    let right_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        target_visible_ix_after,
        DiffTextRegion::SplitRight,
    );
    let left_scroll_y_after = diff_scroll_offset_y(cx, &view);
    let right_scroll_y_after = diff_split_right_scroll_offset_y(cx, &view);

    assert!(
        (left_scroll_y_after - left_scroll_y_before).abs() < 0.01,
        "expected down-before reveal to keep the split-left scroll offset unchanged (before={left_scroll_y_before}, after={left_scroll_y_after})"
    );
    assert!(
        (right_scroll_y_after - right_scroll_y_before).abs() < 0.01,
        "expected down-before reveal to keep the split-right scroll offset unchanged (before={right_scroll_y_before}, after={right_scroll_y_after})"
    );
    assert!(
        left_top_after > left_top_before,
        "expected the split-left collapsed hunk header to move down during down-before reveal (before={left_top_before}, after={left_top_after})"
    );
    assert!(
        right_top_after > right_top_before,
        "expected the split-right collapsed hunk header to move down during down-before reveal (before={right_top_before}, after={right_top_after})"
    );
}

#[gpui::test]
fn collapsed_diff_short_gap_merge_moves_following_file_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(201);
    let (unified, old_text, new_text) = build_collapsed_diff_short_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_short_gap_anchor",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let second_hunk_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .get(1)
            .map(|hunk| hunk.src_ix)
            .expect("expected short-gap fixture to expose a second collapsed hunk")
    });
    reveal_collapsed_diff_hunk_side_fully(cx, &view, second_hunk_src_ix, false);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed short-gap merge becomes scrollable",
        |pane| pane.diff_scroll.0.borrow().base_handle.max_offset().y > px(0.0),
        |pane| {
            format!(
                "offset={:?} max_offset={:?}",
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    let (target_visible_ix, tracked_row_ix) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, second_hunk_src_ix);
        let row_ix = match pane.collapsed_visible_row(visible_ix + 1) {
            Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix }) => row_ix,
            other => panic!("expected a file row after the short-gap header, got {other:?}"),
        };
        (visible_ix, row_ix)
    });
    scroll_collapsed_visible_ix_to_center(cx, &view, target_visible_ix);

    let tracked_visible_ix_before = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, tracked_row_ix)
    });
    let scroll_y_before = diff_scroll_offset_y(cx, &view);
    let row_top_before = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        tracked_visible_ix_before,
        DiffTextRegion::Inline,
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_short(second_hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let tracked_visible_ix_after = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        collapsed_file_row_visible_ix(pane, tracked_row_ix)
    });
    let row_top_after = diff_text_hitbox_top_for_visible_ix(
        cx,
        &view,
        tracked_visible_ix_after,
        DiffTextRegion::Inline,
    );
    let scroll_y_after = diff_scroll_offset_y(cx, &view);

    assert!(
        (scroll_y_after - scroll_y_before).abs() < 0.01,
        "expected short-gap merge to keep the inline diff scroll offset unchanged (before={scroll_y_before}, after={scroll_y_after})"
    );
    assert!(
        row_top_after > row_top_before,
        "expected the first visible row after a short-gap merge to move down when rows are inserted before it (before={row_top_before}, after={row_top_after})"
    );
}
