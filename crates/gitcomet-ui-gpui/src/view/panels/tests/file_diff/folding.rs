//! Collapsed diffs: hunk headers, context reveal, gaps, and reset rules.

use super::*;

fn push_collapsed_diff_loading_fixture_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    file_ready: bool,
) -> gitcomet_core::domain::DiffTarget {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{}_collapsed_loading_root",
        std::process::id(),
        fixture_name
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::commit(
        gitcomet_core::domain::CommitId("deadbeef".into()),
        path.clone(),
    );
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);
    let file_diff = gitcomet_core::domain::FileDiffText::new(path, Some(old_text), Some(new_text));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file_rev = 1;
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

fn assert_collapsed_diff_hunk_height(
    cx: &mut gpui::VisualTestContext,
    selector: &'static str,
    expected: gpui::Pixels,
) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("expected `{selector}` bounds"));
    let actual_height: f32 = bounds.size.height.into();
    let expected_height: f32 = expected.into();
    assert!(
        (actual_height - expected_height).abs() < 0.01,
        "expected `{selector}` height {expected_height}, got {actual_height}"
    );
}

fn assert_collapsed_diff_loading_does_not_render_patch_rows(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &'static str,
    diff_view: DiffViewMode,
) {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_content_mode = DiffContentMode::Collapsed;
            pane.diff_view = diff_view;
            cx.notify();
        });
    });

    let target = push_collapsed_diff_loading_fixture_state(cx, view, repo_id, fixture_name, false);
    cx.run_until_parked();

    let paint_log = cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        let _ = window.draw(app);
        rows::diff_paint_log_for_tests()
    });
    assert!(
        paint_log.is_empty(),
        "collapsed loading should not render raw patch rows, got {paint_log:?}"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_content_mode, DiffContentMode::Collapsed);
        assert!(
            !pane.is_collapsed_diff_projection_active(),
            "loading file contents must not activate the collapsed projection"
        );
        assert!(
            pane.patch_diff_row_len() > 0,
            "patch rows should be cached but not rendered while collapsed file contents load"
        );
    });

    push_collapsed_diff_loading_fixture_state(cx, view, repo_id, fixture_name, true);
    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff loading fixture activates collapsed projection",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
                && !pane.collapsed_diff_hunk_visible_indices.is_empty()
        },
        |pane| {
            format!(
                "mode={:?} view={:?} collapsed_active={} inflight={:?} cache_target={:?} patch_rows={} file_rows={} collapsed_rows={} hunk_rows={:?}",
                pane.diff_content_mode,
                pane.diff_view,
                pane.is_collapsed_diff_projection_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target,
                pane.patch_diff_split_row_len(),
                pane.file_diff_split_row_len(),
                pane.collapsed_diff_visible_rows.len(),
                pane.collapsed_diff_hunk_visible_indices,
            )
        },
    );
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            let hunk_visible_ix = pane.collapsed_diff_hunk_visible_indices[0];
            pane.scroll_diff_to_item_strict(hunk_visible_ix, gpui::ScrollStrategy::Top);
            cx.notify();
        });
    });
    draw_and_drain_test_window(cx);

    let expected = cx.update(|_window, app| {
        AppTheme::gitcomet_dark()
            .editor_row_height(crate::ui_scale::UiScale::current(app).percent())
    });
    match diff_view {
        DiffViewMode::Inline => {
            assert_collapsed_diff_hunk_height(cx, "collapsed_diff_inline_hunk_shell", expected);
        }
        DiffViewMode::Split => {
            assert_collapsed_diff_hunk_height(cx, "collapsed_diff_split_left_hunk_shell", expected);
            assert_collapsed_diff_hunk_height(
                cx,
                "collapsed_diff_split_right_hunk_shell",
                expected,
            );
        }
    }
}

#[gpui::test]
fn collapsed_diff_inline_loading_does_not_render_patch_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_loading_does_not_render_patch_rows(
        cx,
        &view,
        gitcomet_state::model::RepoId(260),
        "collapsed_inline_loading",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_loading_does_not_render_patch_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_loading_does_not_render_patch_rows(
        cx,
        &view,
        gitcomet_state::model::RepoId(261),
        "collapsed_split_loading",
        DiffViewMode::Split,
    );
}

fn collapsed_diff_cache_rebuild_snapshot(pane: &crate::view::panes::main::MainPaneView) -> String {
    let active = pane.active_repo();
    format!(
        "mode={:?} rev={} active_rev={:?} target={:?} active_target={:?} signature={:?} file_rev={} active_file_rev={:?} file_target={:?} file_path={:?} collapsed_active={} hunks={:?}",
        pane.diff_content_mode,
        pane.diff_cache_rev,
        active.map(|repo| repo.diff_state.diff_rev),
        pane.diff_cache_target,
        active.and_then(|repo| repo.diff_state.diff_target.clone()),
        pane.diff_cache_content_signature,
        pane.file_diff_cache_rev,
        active.map(|repo| repo.diff_state.diff_file_rev),
        pane.file_diff_cache_target,
        pane.file_diff_cache_path,
        pane.is_collapsed_diff_projection_active(),
        pane.collapsed_diff_hunks,
    )
}

fn assert_collapsed_hunk_header_hides_after_full_reveal(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
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

    let hunk_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hunks
            .first()
            .map(|hunk| hunk.src_ix)
            .expect("expected collapsed diff fixture to expose one hunk")
    });

    reveal_collapsed_diff_hunk_side_fully(cx, view, hunk_src_ix, true);

    let hidden_down_after_up = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            0,
            "fully revealing the upper side should consume the hidden-up budget"
        );
        let anchor_visible_ix = pane
            .collapsed_diff_hunk_visible_indices
            .first()
            .copied()
            .expect("expected collapsed diff hunk anchor after revealing upward");
        assert!(
            matches!(
                pane.collapsed_visible_row(anchor_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { .. })
            ),
            "once the upper gap is fully consumed, the hunk anchor should move to the first visible file row"
        );
        // Line 35 is the only change: row 34 in both layouts.
        assert_eq!(
            pane.diff_nav_entries(),
            vec![collapsed_file_row_visible_ix(pane, 34)],
            "diff navigation should stop on the changed row, not the hunk anchor, once the top expansion row disappears"
        );
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_up > 0,
        "fixture should still keep hidden rows below the hunk after revealing only upward context"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            matches!(
                pane.collapsed_visible_row(pane.diff_visible_len().saturating_sub(1)),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                    display_src_ix: None,
                    ..
                })
            ),
            "expected the trailing down-expansion row to remain in the collapsed projection while hidden rows still exist below the merged hunk"
        );
    });

    reveal_collapsed_diff_hunk_side_fully(cx, view, hunk_src_ix, false);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.collapsed_diff_hidden_up_rows(hunk_src_ix), 0);
        assert_eq!(pane.collapsed_diff_hidden_down_rows(hunk_src_ix), 0);

        let anchor_visible_ix = pane
            .collapsed_diff_hunk_visible_indices
            .first()
            .copied()
            .expect("expected collapsed diff hunk anchor");
        assert!(
            matches!(
                pane.collapsed_visible_row(anchor_visible_ix),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { .. })
            ),
            "fully revealed collapsed hunks should anchor to the first file row instead of a synthetic header"
        );
        assert_eq!(
            pane.diff_nav_entries(),
            vec![collapsed_file_row_visible_ix(pane, 34)],
            "diff navigation should keep stopping on the changed row once the hunk is fully revealed"
        );
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !matches!(
                pane.collapsed_visible_row(pane.diff_visible_len().saturating_sub(1)),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                    display_src_ix: None,
                    ..
                })
            ),
            "expected the trailing down-expansion row to disappear once the remaining hidden rows are fully revealed"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_survives_projection_reset(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(188);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_reset",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });
    assert!(
        hidden_down_before > 0,
        "fixture should start with hidden rows below the hunk"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let hidden_down_after_reveal = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.reset_collapsed_diff_projection(false);
            pane.ensure_diff_visible_indices();
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_after_reveal,
            "non-clearing projection resets should preserve revealed collapsed diff context"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.reset_collapsed_diff_projection(true);
            pane.ensure_diff_visible_indices();
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before,
            "clearing projection resets should restore the collapsed default"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_survives_window_resize(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(189);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_resize",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let (hidden_down_after_reveal, visible_len_after_reveal) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            pane.diff_visible_len(),
        )
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    cx.simulate_resize(gpui::size(px(900.0), px(620.0)));
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_after_reveal,
            "window resize should not reset revealed collapsed diff context"
        );
        assert_eq!(
            pane.diff_visible_len(),
            visible_len_after_reveal,
            "window resize should preserve the collapsed projection row count"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_survives_same_content_diff_cache_rebuild(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(286);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_same_content_rebuild",
        DiffViewMode::Inline,
        unified.clone(),
        old_text.clone(),
        new_text.clone(),
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let hidden_down_after_reveal = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_survives_same_content_rebuild",
        PathBuf::from("src/lib.rs"),
        2,
        unified,
        old_text,
        new_text,
    );
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content patch diff cache rebuild completes",
        |pane| {
            pane.diff_cache_rev == 2
                && pane.diff_cache_content_signature.is_some()
                && pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunks.is_empty()
        },
        collapsed_diff_cache_rebuild_snapshot,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_after_reveal,
            "same-content patch diff cache rebuilds should preserve revealed collapsed diff context"
        );
    });
}

#[gpui::test]
fn collapsed_diff_reveal_state_resets_when_diff_content_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(287);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_resets_on_content_change",
        DiffViewMode::Inline,
        unified.clone(),
        old_text.clone(),
        new_text.clone(),
    );

    let (hunk_src_ix, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let hidden_down_after_reveal = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.collapsed_diff_hidden_down_rows(hunk_src_ix)
    });
    assert!(
        hidden_down_after_reveal < hidden_down_before,
        "revealing below the hunk should reduce the hidden row budget"
    );

    let changed_unified = unified.replace("new value 35", "new value 35 updated");
    let changed_new_text = new_text.replace("new value 35", "new value 35 updated");
    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "collapsed_reveal_resets_on_content_change",
        PathBuf::from("src/lib.rs"),
        2,
        changed_unified,
        old_text,
        changed_new_text,
    );
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "changed-content patch diff cache rebuild completes",
        |pane| {
            pane.diff_cache_rev == 2
                && pane.diff_cache_content_signature.is_some()
                && pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunks.is_empty()
        },
        collapsed_diff_cache_rebuild_snapshot,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before,
            "changed patch diff content should reset revealed collapsed diff context"
        );
    });
}

fn assert_collapsed_diff_file_switch_resets_expanded_context(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
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

    let (hunk_src_ix, hidden_up_before, hidden_down_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected initial collapsed fixture to expose one hunk");
        (
            hunk.src_ix,
            pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
            pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
        )
    });
    assert!(hidden_up_before >= 20 && hidden_down_before >= 20);

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_up(hunk_src_ix, cx);
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.collapsed_diff_reveals.is_empty(),
            "fixture should have persisted expanded collapsed-diff context before switching files"
        );
        assert!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix) < hidden_up_before,
            "upward reveal should reduce the hidden-up budget before switching files"
        );
        assert!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix) < hidden_down_before,
            "downward reveal should reduce the hidden-down budget before switching files"
        );
    });

    let next_path = PathBuf::from("src/other.rs");
    let (next_unified, next_old_text, next_new_text) =
        build_collapsed_diff_multi_hunk_fixture_texts(&[(
            60,
            "old other value 60",
            "new other value 60",
        )]);
    let next_path_for_patch = next_path.to_string_lossy().replace('\\', "/");
    let next_unified = next_unified.replace("src/lib.rs", &next_path_for_patch);
    let next_target = push_regular_diff_content_mode_state_with_rev(
        cx,
        view,
        repo_id,
        fixture_name,
        next_path.clone(),
        2,
        next_unified,
        next_old_text,
        next_new_text,
    );
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff projection switches to the second file",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && pane.file_diff_cache_target == Some(next_target.clone())
                && !pane.collapsed_diff_hunks.is_empty()
        },
        collapsed_diff_cache_rebuild_snapshot,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected second collapsed fixture to expose one hunk");
        assert_eq!(
            pane.collapsed_diff_projection_identity
                .as_ref()
                .map(|identity| &identity.diff_target),
            Some(&next_target),
            "collapsed projection identity should follow the newly selected file target"
        );
        assert!(
            pane.collapsed_diff_reveals.is_empty(),
            "expanded collapsed-diff context from the previous file must not leak into the next file"
        );
        assert!(
            hunk.base_row_start > 50,
            "expected the rebuilt collapsed hunk to map to the second file's line-60 change, got {hunk:?}"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
            hunk.base_row_start,
            "the second file should start with default hidden context above its hunk"
        );
    });
}

#[gpui::test]
fn collapsed_diff_inline_file_switch_resets_expanded_context(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_file_switch_resets_expanded_context(
        cx,
        &view,
        gitcomet_state::model::RepoId(289),
        "collapsed_inline_file_switch_resets_context",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_file_switch_resets_expanded_context(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_file_switch_resets_expanded_context(
        cx,
        &view,
        gitcomet_state::model::RepoId(290),
        "collapsed_split_file_switch_resets_context",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn collapsed_diff_split_header_shows_stats_without_file_header(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(288);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_split_header_stats",
        DiffViewMode::Split,
        unified,
        old_text,
        new_text,
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunk_visible_indices.first().copied(),
            Some(0),
            "collapsed mode should start at the hunk expansion row, not a file-path header"
        );
        assert_eq!(
            pane.collapsed_diff_total_file_stat(),
            Some((1, 1)),
            "fixture should expose one added and one removed row for the split header counters"
        );
    });
    assert!(
        cx.debug_bounds("diff_split_header_removed_stat").is_some(),
        "expected the removed counter to be rendered in the left (before) pane header"
    );
    assert!(
        cx.debug_bounds("diff_split_header_added_stat").is_some(),
        "expected the added counter to be rendered in the right (after) pane header"
    );
}

#[gpui::test]
fn collapsed_diff_revealed_hunk_header_hides_context_and_updates_ranges(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(289);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let unified = unified.replace("@@ -32,7 +32,7 @@", "@@ -32,7 +32,7 @@ impl MainPaneView {");
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_header_dynamic_range",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let (hunk_src_ix, hunk_visible_ix, header_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        let visible_ix = collapsed_hunk_visible_ix_for_src_ix(pane, hunk.src_ix);
        let header = pane
            .diff_text_line_for_region(visible_ix, DiffTextRegion::Inline)
            .to_string();
        (hunk.src_ix, visible_ix, header)
    });
    assert_eq!(header_before, "-32,7 +32,7  impl MainPaneView {");

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_up(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_line_for_region(hunk_visible_ix, DiffTextRegion::Inline)
                .as_ref(),
            "-12,27 +12,27",
            "revealing context above should hide the static context label and expand the displayed old/new ranges"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_line_for_region(hunk_visible_ix, DiffTextRegion::Inline)
                .as_ref(),
            "-12,47 +12,47",
            "revealing context below should also expand the displayed old/new ranges"
        );
    });
}

#[gpui::test]
fn collapsed_diff_first_open_keeps_hunk_header_visible_after_scrolled_file(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(620.0)));

    let repo_id = gitcomet_state::model::RepoId(70633);
    let changes = [
        (20, "old value 20", "new value 20"),
        (40, "old value 40", "new value 40"),
        (60, "old value 60", "new value 60"),
        (80, "old value 80", "new value 80"),
    ];
    let (unified, old_text, new_text) = build_collapsed_diff_multi_hunk_fixture_texts(&changes);
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_stale_offset",
        DiffViewMode::Inline,
        unified.clone(),
        old_text.clone(),
        new_text.clone(),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            let last = pane.diff_visible_len().saturating_sub(1);
            pane.scroll_diff_to_item_strict(last, gpui::ScrollStrategy::Top);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        diff_scroll_offset_y(cx, &view) < 0.0,
        "fixture should leave the first file scrolled down"
    );

    // Same content under another path: a new target that inherits the offset.
    let other_path = PathBuf::from("src/other.rs");
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_stale_offset",
        other_path,
        unified.replace("src/lib.rs", "src/other.rs"),
        old_text,
        new_text,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "second collapsed file opens and autoscrolls",
        |pane| {
            pane.file_diff_cache_target == Some(target.clone())
                && pane.is_collapsed_diff_projection_active()
                && !pane.diff_autoscroll_pending
        },
        |pane| {
            (
                pane.file_diff_cache_target.clone(),
                pane.is_collapsed_diff_projection_active(),
                pane.diff_autoscroll_pending,
            )
        },
    );
    draw_and_drain_test_window(cx);

    assert_eq!(
        diff_scroll_offset_y(cx, &view),
        0.0,
        "opening a collapsed diff should keep its first hunk header in view, not scroll to the \
         first changed line"
    );
}

#[gpui::test]
fn collapsed_diff_reveal_controls_expand_visible_context(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(190);
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_reveal",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed reveal fixture activates full file diff first",
        |pane| {
            pane.is_file_diff_view_active() && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            format!(
                "mode={:?} file_diff_active={} target={:?}",
                pane.diff_content_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_target,
            )
        },
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed reveal projection becomes active",
        |pane| pane.is_collapsed_diff_projection_active() && !pane.collapsed_diff_hunks.is_empty(),
        |pane| {
            format!(
                "collapsed_active={} visible_len={} hunks={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_visible_len(),
                pane.collapsed_diff_hunks,
            )
        },
    );

    let (hunk_src_ix, visible_before, hidden_up_before, hidden_down_before) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let hunk = pane
                .collapsed_diff_hunks
                .first()
                .copied()
                .expect("expected collapsed diff fixture to expose one hunk");
            (
                hunk.src_ix,
                pane.diff_visible_len(),
                pane.collapsed_diff_hidden_up_rows(hunk.src_ix),
                pane.collapsed_diff_hidden_down_rows(hunk.src_ix),
            )
        });

    assert!(
        hidden_up_before >= 20 && hidden_down_before >= 20,
        "fixture should expose enough hidden context for 20-line reveal steps"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_up(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 20,
            "revealing above the hunk should add 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            hidden_up_before - 20,
            "revealing above the hunk should reduce the hidden-up budget by 20 rows"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_down(hunk_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_visible_len(),
            visible_before + 40,
            "revealing below the hunk should add another 20 visible rows"
        );
        assert_eq!(
            pane.collapsed_diff_hidden_down_rows(hunk_src_ix),
            hidden_down_before - 20,
            "revealing below the hunk should reduce the hidden-down budget by 20 rows"
        );
    });
}

#[gpui::test]
fn collapsed_diff_inline_hunk_header_hides_after_full_reveal(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_hunk_header_hides_after_full_reveal(
        cx,
        &view,
        gitcomet_state::model::RepoId(195),
        "collapsed_inline_full_reveal",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_hunk_header_hides_after_full_reveal(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_hunk_header_hides_after_full_reveal(
        cx,
        &view,
        gitcomet_state::model::RepoId(196),
        "collapsed_split_full_reveal",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn collapsed_diff_long_gap_exposes_up_both_and_trailing_down_expansions(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(197);
    let (unified, old_text, new_text) = build_collapsed_diff_long_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_long_gap",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            2,
            "expected the long-gap fixture to expose two collapsed sections"
        );
        let first_anchor = pane.collapsed_diff_hunk_visible_indices[0];
        let second_anchor = pane.collapsed_diff_hunk_visible_indices[1];
        assert!(
            matches!(
                pane.collapsed_visible_row(first_anchor),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Up,
                    ..
                })
            ),
            "the first collapsed section should expose only an upward expansion row"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(second_anchor),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Both,
                    ..
                })
            ),
            "the second collapsed section should expose a both-direction expansion row for the long interior gap"
        );
        assert!(
            matches!(
                pane.collapsed_visible_row(pane.diff_visible_len().saturating_sub(1)),
                Some(crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                    expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Down,
                    display_src_ix: None,
                    ..
                })
            ),
            "a trailing dummy expansion row should remain at the bottom when there is hidden context below the last section"
        );
    });
}

#[gpui::test]
fn collapsed_diff_short_gap_uses_single_expand_all_and_merges_sections(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(198);
    let (unified, old_text, new_text) = build_collapsed_diff_short_gap_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_short_gap",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    let second_src_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            2,
            "expected the short-gap fixture to expose two collapsed sections before merging"
        );
        let second_anchor = pane.collapsed_diff_hunk_visible_indices[1];
        assert!(
            matches!(
                pane.collapsed_visible_row(second_anchor),
                Some(
                    crate::view::panes::main::CollapsedDiffVisibleRow::HunkHeader {
                        expansion_kind: crate::view::panes::main::CollapsedDiffExpansionKind::Short,
                        ..
                    }
                )
            ),
            "the second collapsed section should expose a single short-gap expansion row"
        );
        pane.collapsed_diff_hunks[1].src_ix
    });

    assert!(
        cx.debug_bounds("collapsed_diff_inline_hunk_short")
            .is_some(),
        "expected the short-gap control to be rendered before expanding it"
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.collapsed_diff_reveal_hunk_short(second_src_ix, cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            1,
            "expanding a short gap should merge the neighboring collapsed sections"
        );
        assert_eq!(
            pane.collapsed_diff_hunk_visible_indices.len(),
            1,
            "merged short gaps should leave a single collapsed-section anchor"
        );
        assert_eq!(
            pane.diff_nav_entries().len(),
            2,
            "merging sections keeps one navigation stop per change block (lines 20 and 34)"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
            pane.reset_collapsed_diff_projection(false);
            pane.ensure_diff_visible_indices();
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.collapsed_diff_hunks.len(),
            1,
            "projection rebuilds should keep a fully revealed short gap merged"
        );
        assert_eq!(
            pane.diff_nav_entries().len(),
            2,
            "projection rebuilds should keep one navigation stop per change block"
        );
    });
    assert!(
        cx.debug_bounds("collapsed_diff_inline_hunk_short")
            .is_none(),
        "expected the short-gap control to disappear after the sections merge"
    );
}
