//! Change navigation: block stops, shortcuts, and the focused-block bar.

use super::*;

/// Three change blocks; the first spans two lines so navigation can start mid-block.
fn build_full_diff_multi_block_fixture_texts() -> (String, String, String) {
    let old_text =
        "alpha\nold one\nold two\nmiddle one\nold three\nmiddle two\nold four\nomega\n".to_string();
    let new_text =
        "alpha\nnew one\nnew two\nmiddle one\nnew three\nmiddle two\nnew four\nomega\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,8 +1,8 @@
 alpha
-old one
-old two
+new one
+new two
 middle one
-old three
+new three
 middle two
-old four
+new four
 omega
"
    .to_string();
    (unified, old_text, new_text)
}

/// The first line is changed, so the first change block starts at row 0.
fn build_full_diff_first_row_change_fixture_texts() -> (String, String, String) {
    let old_text = "old first\nkeep\nold last\n".to_string();
    let new_text = "new first\nkeep\nnew last\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
-old first
+new first
 keep
-old last
+new last
"
    .to_string();
    (unified, old_text, new_text)
}

fn set_diff_text_selection_for_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    start_visible_ix: usize,
    end_visible_ix: usize,
    region: DiffTextRegion,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: start_visible_ix,
                    region,
                    offset: 0,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: end_visible_ix,
                    region,
                    offset: 1,
                });
                pane.diff_selection_anchor = Some(end_visible_ix);
                pane.diff_selection_range = None;
                // Seeded selections must own the window like real ones, or the
                // arbitration reaps them on the next global write. Adopted last,
                // once the state it describes is fully seeded.
                pane.diff_text_selection_owner.adopt(window, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
}

/// Loads `fixture` as a Full-mode file diff in `diff_view` with nothing
/// selected, and returns its navigation stops once there are `expected_stops`.
fn activate_full_diff_nav_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
    (unified, old_text, new_text): (String, String, String),
    expected_stops: usize,
) -> Vec<usize> {
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

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff fixture activates file diff view",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight.is_some(),
                pane.file_diff_cache_target.clone(),
                pane.diff_nav_entries(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_content_mode = DiffContentMode::Full;
                pane.diff_view = diff_view;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_autoscroll_pending = false;
                pane.clear_diff_text_selection();
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff has one navigation stop per change block",
        |pane| {
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == diff_view
                && pane.diff_nav_entries().len() == expected_stops
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_nav_entries(),
            )
        },
    );
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_nav_entries())
}

fn press_and_assert_anchor(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    keystroke: &str,
    expected: usize,
    message: &str,
) {
    cx.simulate_keystrokes(keystroke);
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_selection_anchor, Some(expected), "{message}");
        // Anchor only: the landed row gets no selection wash.
        assert_eq!(pane.diff_selection_range, None, "{message}");
    });
}

fn assert_full_diff_change_shortcuts_visit_each_change_block(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    let (e0, e1, e2) = (entries[0], entries[1], entries[2]);
    assert!(
        e1 > e0 + 1,
        "fixture's first change block should span several rows in {diff_view:?}: {entries:?}"
    );

    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e0,
        &format!("first F3 should land on the first change block in Full diff {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e1,
        &format!("second F3 should skip the rest of the block in Full diff {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 should move back one change block in Full diff {diff_view:?}"),
    );

    // From the middle of a block, F2 goes to that block's start and F3 to the next block.
    set_diff_row_selection_for_test(cx, view, e0 + 1, (e0 + 1, e0 + 1));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 mid-block should go to the block's start in Full diff {diff_view:?}"),
    );
    set_diff_row_selection_for_test(cx, view, e0 + 1, (e0 + 1, e0 + 1));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e1,
        &format!("F3 mid-block should go to the next block in Full diff {diff_view:?}"),
    );

    set_diff_row_selection_for_test(cx, view, e0, (e0, e1));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e2,
        &format!("F3 should continue after the selected row range in Full diff {diff_view:?}"),
    );

    set_diff_row_selection_for_test(cx, view, e2, (e1, e2));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 should continue before the selected row range in Full diff {diff_view:?}"),
    );

    let text_region = match diff_view {
        DiffViewMode::Inline => DiffTextRegion::Inline,
        DiffViewMode::Split => DiffTextRegion::SplitLeft,
    };
    set_diff_text_selection_for_test(cx, view, e0, e0 + 1, text_region);
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e1,
        &format!("F3 should continue after the selected text range in Full diff {diff_view:?}"),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    set_diff_text_selection_for_test(cx, view, e1, e2, text_region);
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        e0,
        &format!("F2 should continue before the selected text range in Full diff {diff_view:?}"),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_text_anchor, None);
        assert_eq!(pane.diff_text_head, None);
    });

    // The last stop does not wrap around.
    set_diff_row_selection_for_test(cx, view, e2, (e2, e2));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        e2,
        &format!("F3 on the last block should stay put in Full diff {diff_view:?}"),
    );
}

#[gpui::test]
fn full_diff_inline_change_shortcuts_visit_each_change_block(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_change_shortcuts_visit_each_change_block(
        cx,
        &view,
        gitcomet_state::model::RepoId(70601),
        "full_diff_inline_block_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_change_shortcuts_visit_each_change_block(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_change_shortcuts_visit_each_change_block(
        cx,
        &view,
        gitcomet_state::model::RepoId(70602),
        "full_diff_split_block_nav",
        DiffViewMode::Split,
    );
}

/// The real app binds F2/F3 globally *and* observes keystrokes for a diff
/// shortcut fallback; a press must still move exactly one block.
fn assert_full_diff_app_keys_move_one_block_per_press(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    cx.update(|_window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });

    focus_diff_panel(cx, view);
    for (keystroke, expected, message) in [
        ("f3", entries[0], "first F3"),
        ("f3", entries[1], "second F3"),
        ("f2", entries[0], "F2"),
        ("f7", entries[1], "F7"),
        ("shift-f7", entries[0], "Shift+F7"),
    ] {
        cx.simulate_keystrokes(keystroke);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            assert_eq!(
                view.read(app).main_pane.read(app).diff_selection_anchor,
                Some(expected),
                "{message} should move exactly one change block in {diff_view:?}"
            );
        });
    }
}

#[gpui::test]
fn full_diff_inline_app_keys_move_one_block_per_press(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_app_keys_move_one_block_per_press(
        cx,
        &view,
        gitcomet_state::model::RepoId(70637),
        "full_diff_inline_app_keys_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_app_keys_move_one_block_per_press(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_app_keys_move_one_block_per_press(
        cx,
        &view,
        gitcomet_state::model::RepoId(70638),
        "full_diff_split_app_keys_nav",
        DiffViewMode::Split,
    );
}

fn assert_full_diff_f3_without_selection_reaches_block_at_first_row(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_first_row_change_fixture_texts(),
        2,
    );
    assert_eq!(
        entries[0], 0,
        "fixture's first change block should start at row 0"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_focus_visible_range(), None);
        assert_eq!(
            pane.diff_nav_next_target_ix(&entries),
            Some(0),
            "the Next button should be enabled for a block at row 0 with nothing selected"
        );
        assert_eq!(pane.diff_nav_prev_target_ix(&entries), None);
    });

    // F2 with nothing focused has no target and must not seed a focus that
    // would make the next F3 skip row 0.
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            None
        );
    });

    press_and_assert_anchor(
        cx,
        view,
        "f3",
        0,
        &format!("F3 with nothing selected should reach the block at row 0 in {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        entries[1],
        &format!("F3 should then reach the next block in {diff_view:?}"),
    );
}

#[gpui::test]
fn full_diff_inline_f3_without_selection_reaches_block_at_first_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_f3_without_selection_reaches_block_at_first_row(
        cx,
        &view,
        gitcomet_state::model::RepoId(70608),
        "full_diff_inline_first_row_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_f3_without_selection_reaches_block_at_first_row(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_f3_without_selection_reaches_block_at_first_row(
        cx,
        &view,
        gitcomet_state::model::RepoId(70609),
        "full_diff_split_first_row_nav",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_diff_ignore_whitespace_drops_whitespace_only_block_stop(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let old_text = "alpha\nold one\nmiddle\n  keep\nomega\n".to_string();
    let new_text = "alpha\nnew one\nmiddle\nkeep\nomega\n".to_string();
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@
 alpha
-old one
+new one
 middle
-  keep
+keep
 omega
"
    .to_string();
    let shown = activate_full_diff_nav_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(70610),
        "full_diff_ignore_whitespace_nav",
        DiffViewMode::Split,
        (unified, old_text, new_text),
        2,
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "ignore-whitespace drops the whitespace-only block stop",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.diff_nav_entries() == vec![shown[0]]
        },
        |pane| {
            (
                pane.diff_whitespace_mode,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight.is_some(),
                pane.diff_nav_entries(),
            )
        },
    );
}

/// Focused change block marks painted on a fresh draw, by row then column.
fn painted_focused_change_block_marks(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> Vec<rows::FocusedChangeBlockPaint> {
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |_pane, cx| cx.notify());
        rows::clear_focused_change_block_paint_log_for_tests();
        let _ = window.draw(app);
        let mut painted = rows::focused_change_block_paint_log_for_tests();
        painted.sort_by_key(|mark| (mark.visible_ix, mark.region as u8));
        painted.dedup();
        painted
    })
}

/// Asserts the focused block covers `expected` rows, each column painting the
/// bar with the outline closed on the first and last rows, and returns the
/// outline side each column painted, row by row.
fn assert_focused_change_block_bar(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    expected: std::ops::Range<usize>,
    diff_view: DiffViewMode,
    message: &str,
) -> Vec<Vec<Option<DiffChangeSide>>> {
    assert_eq!(
        focused_change_block_rows(cx, view),
        expected.clone().collect::<Vec<_>>(),
        "{message}: focused block rows in {diff_view:?}"
    );
    let regions: &[DiffTextRegion] = match diff_view {
        DiffViewMode::Inline => &[DiffTextRegion::Inline],
        DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
    };
    let painted = painted_focused_change_block_marks(cx, view);
    let painted_cells = painted
        .iter()
        .map(|mark| (mark.visible_ix, mark.region, mark.top, mark.bottom))
        .collect::<Vec<_>>();
    let expected_cells = expected
        .clone()
        .flat_map(|visible_ix| {
            let top = visible_ix == expected.start;
            let bottom = visible_ix + 1 == expected.end;
            regions
                .iter()
                .map(move |&region| (visible_ix, region, top, bottom))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        painted_cells, expected_cells,
        "{message}: painted marks in {diff_view:?}"
    );
    regions
        .iter()
        .map(|&region| {
            painted
                .iter()
                .filter(|mark| mark.region == region)
                .map(|mark| mark.outline)
                .collect()
        })
        .collect()
}

fn assert_full_diff_focused_change_block_shows_accent_bar(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let entries = activate_full_diff_nav_fixture(
        cx,
        view,
        repo_id,
        fixture_name,
        diff_view,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    let (e0, e1, e2) = (entries[0], entries[1], entries[2]);
    assert!(
        painted_focused_change_block_marks(cx, view).is_empty(),
        "no block is focused before navigating in {diff_view:?}"
    );

    // One unchanged row ("middle one", "middle two") separates the blocks.
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let outlines =
        assert_focused_change_block_bar(cx, view, e0..e1 - 1, diff_view, "F3 onto block one");
    // Every block modifies lines: inline alternates `-`/`+` rows, each in its
    // own colour; split outlines the old column red and the new one green.
    use DiffChangeSide::{Added, Removed};
    let expected_outlines = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed), Some(Added), Some(Removed), Some(Added)]],
        DiffViewMode::Split => vec![vec![Some(Removed); 2], vec![Some(Added); 2]],
    };
    assert_eq!(
        outlines, expected_outlines,
        "outline colours in {diff_view:?}"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_focused_change_block_bar(cx, view, e1..e2 - 1, diff_view, "F3 onto block two");

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_focused_change_block_bar(cx, view, e0..e1 - 1, diff_view, "F2 back to block one");

    // Moving the selection elsewhere takes the marks away.
    set_diff_row_selection_for_test(cx, view, 0, (0, 0));
    assert!(
        painted_focused_change_block_marks(cx, view).is_empty(),
        "the marks should follow the selection off the block in {diff_view:?}"
    );
}

#[gpui::test]
fn full_diff_inline_focused_change_block_shows_accent_bar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_focused_change_block_shows_accent_bar(
        cx,
        &view,
        gitcomet_state::model::RepoId(70634),
        "full_diff_inline_focus_bar",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_focused_change_block_shows_accent_bar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_focused_change_block_shows_accent_bar(
        cx,
        &view,
        gitcomet_state::model::RepoId(70635),
        "full_diff_split_focus_bar",
        DiffViewMode::Split,
    );
}

fn assert_pure_change_blocks_outline_only_their_side(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    // An inserted line, then a deleted one.
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,4 @@
 a
+NEW
 b
-c
 d
"
    .to_string();
    let fixture = (
        unified,
        "a\nb\nc\nd\n".to_string(),
        "a\nNEW\nb\nd\n".to_string(),
    );
    let entries =
        activate_full_diff_nav_fixture(cx, view, repo_id, fixture_name, diff_view, fixture, 2);

    use DiffChangeSide::{Added, Removed};
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let added = assert_focused_change_block_bar(
        cx,
        view,
        entries[0]..entries[0] + 1,
        diff_view,
        "the inserted line",
    );
    let expected_added = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Added)]],
        // Nothing was removed, so the old column's empty filler gets no outline.
        DiffViewMode::Split => vec![vec![None], vec![Some(Added)]],
    };
    assert_eq!(
        added, expected_added,
        "an addition outlines green in {diff_view:?}"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let removed = assert_focused_change_block_bar(
        cx,
        view,
        entries[1]..entries[1] + 1,
        diff_view,
        "the deleted line",
    );
    let expected_removed = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed)]],
        DiffViewMode::Split => vec![vec![Some(Removed)], vec![None]],
    };
    assert_eq!(
        removed, expected_removed,
        "a removal outlines red in {diff_view:?}"
    );
}

#[gpui::test]
fn full_diff_inline_pure_change_blocks_outline_only_their_side(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_pure_change_blocks_outline_only_their_side(
        cx,
        &view,
        gitcomet_state::model::RepoId(70639),
        "full_diff_inline_pure_outline",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_split_pure_change_blocks_outline_only_their_side(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_pure_change_blocks_outline_only_their_side(
        cx,
        &view,
        gitcomet_state::model::RepoId(70640),
        "full_diff_split_pure_outline",
        DiffViewMode::Split,
    );
}

#[gpui::test]
fn full_diff_whitespace_mode_change_does_not_restore_focused_block_on_row_click(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,3 +1,3 @@
 alpha
-  keep
+keep
 omega
"
    .to_string();
    let entries = activate_full_diff_nav_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(70641),
        "full_diff_whitespace_focus_bar",
        DiffViewMode::Split,
        (
            unified,
            "alpha\n  keep\nomega\n".to_string(),
            "alpha\nkeep\nomega\n".to_string(),
        ),
        1,
    );
    let row = entries[0];
    focus_diff_panel(cx, &view);
    press_and_assert_anchor(cx, &view, "f3", row, "F3 onto the whitespace-only block");
    assert_focused_change_block_bar(
        cx,
        &view,
        row..row + 1,
        DiffViewMode::Split,
        "the whitespace-only block before ignoring whitespace",
    );
    let visible_len = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            let visible_len = pane.diff_visible_len();
            pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
            visible_len
        })
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "the whitespace-only block becomes context",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.diff_visible_len() == visible_len
                && pane.diff_nav_entries().is_empty()
        },
        |pane| {
            (
                pane.file_diff_cache_inflight.is_some(),
                pane.diff_visible_len(),
                pane.diff_nav_entries(),
            )
        },
    );

    // Split mode keeps the same row indices and row count. Clicking the old
    // anchor must not revive the block captured before the mode switch.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            assert_eq!(pane.diff_selection_anchor, None);
            let row_ix = pane.diff_mapped_ix_for_visible_ix(row).unwrap();
            assert!(!pane.file_diff_row_is_change(row_ix));
            pane.handle_patch_row_click(row, DiffClickKind::Line, false);
            assert_eq!(pane.diff_selection_anchor, Some(row));
            cx.notify();
        });
    });
    assert!(
        painted_focused_change_block_marks(cx, &view).is_empty(),
        "clicking the former whitespace-only block must not paint its old outline"
    );
    assert!(focused_change_block_rows(cx, &view).is_empty());
}

#[gpui::test]
fn focused_change_block_bar_hides_after_the_layout_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let entries = activate_full_diff_nav_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(70636),
        "full_diff_focus_bar_relayout",
        DiffViewMode::Inline,
        build_full_diff_multi_block_fixture_texts(),
        3,
    );
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_eq!(
        focused_change_block_rows(cx, &view).first().copied(),
        Some(entries[0])
    );

    // Inline and split number their rows differently; the captured rows no
    // longer describe the block, even though the anchor index is unchanged.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(entries[0]),
            "fixture should keep the anchor so only the layout check can hide the bar"
        );
    });
    assert!(
        focused_change_block_rows(cx, &view).is_empty(),
        "a relayout should hide the bar rather than mark whatever rows now sit there"
    );
}

/// One git hunk (lines 17-27) holding two change runs, at lines 20 and 24.
fn build_collapsed_diff_two_runs_in_one_hunk_fixture_texts() -> (String, String, String) {
    let changes = [20usize, 24];
    let line_text = |line: usize, side: &str| {
        if changes.contains(&line) {
            format!("{side} value {line}")
        } else {
            format!("line {line}")
        }
    };
    let old_lines = (1..=40)
        .map(|line| line_text(line, "old"))
        .collect::<Vec<_>>();
    let new_lines = (1..=40)
        .map(|line| line_text(line, "new"))
        .collect::<Vec<_>>();
    let mut unified = String::from(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -17,11 +17,11 @@
",
    );
    for line in 17..=27 {
        if changes.contains(&line) {
            unified.push_str(&format!(
                "-{}\n+{}\n",
                old_lines[line - 1],
                new_lines[line - 1]
            ));
        } else {
            unified.push_str(&format!(" {}\n", old_lines[line - 1]));
        }
    }
    (
        unified,
        format!("{}\n", old_lines.join("\n")),
        format!("{}\n", new_lines.join("\n")),
    )
}

fn assert_collapsed_diff_hunk_with_two_change_runs_has_two_stops(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_two_runs_in_one_hunk_fixture_texts();
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

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed hunk with two change runs has two stops",
        |pane| {
            pane.diff_view == diff_view
                && pane.collapsed_diff_hunk_visible_indices.len() == 1
                && pane.diff_nav_entries().len() == 2
        },
        |pane| {
            (
                pane.diff_view,
                pane.collapsed_diff_hunk_visible_indices.clone(),
                pane.diff_nav_entries(),
            )
        },
    );

    // Lines 20 and 24: split rows 19 and 23; inline adds a `+` row after 20.
    let (first_row, second_row) = match diff_view {
        DiffViewMode::Inline => (19, 24),
        DiffViewMode::Split => (19, 23),
    };
    let entries = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let entries = pane.diff_nav_entries();
        assert_eq!(
            entries,
            vec![
                collapsed_file_row_visible_ix(pane, first_row),
                collapsed_file_row_visible_ix(pane, second_row),
            ],
            "each change run in the hunk should be its own stop, on its first changed row"
        );
        entries
    });

    set_diff_row_selection_for_test(cx, view, entries[0], (entries[0], entries[0]));
    focus_diff_panel(cx, view);
    press_and_assert_anchor(
        cx,
        view,
        "f3",
        entries[1],
        &format!("F3 should stop on the second run inside the same hunk in {diff_view:?}"),
    );
    press_and_assert_anchor(
        cx,
        view,
        "f2",
        entries[0],
        &format!("F2 should return to the first run in {diff_view:?}"),
    );
    // Line 20 is one split row, or a `-`/`+` pair inline.
    let run_len = match diff_view {
        DiffViewMode::Inline => 2,
        DiffViewMode::Split => 1,
    };
    let outlines = assert_focused_change_block_bar(
        cx,
        view,
        entries[0]..entries[0] + run_len,
        diff_view,
        "the marks cover only the focused run, not the whole hunk",
    );
    use DiffChangeSide::{Added, Removed};
    let expected_outlines = match diff_view {
        DiffViewMode::Inline => vec![vec![Some(Removed), Some(Added)]],
        DiffViewMode::Split => vec![vec![Some(Removed)], vec![Some(Added)]],
    };
    assert_eq!(
        outlines, expected_outlines,
        "outline colours in {diff_view:?}"
    );
}

#[gpui::test]
fn collapsed_diff_inline_hunk_with_two_change_runs_has_two_stops(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_hunk_with_two_change_runs_has_two_stops(
        cx,
        &view,
        gitcomet_state::model::RepoId(70611),
        "collapsed_inline_two_runs_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_hunk_with_two_change_runs_has_two_stops(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_hunk_with_two_change_runs_has_two_stops(
        cx,
        &view,
        gitcomet_state::model::RepoId(70630),
        "collapsed_split_two_runs_nav",
        DiffViewMode::Split,
    );
}
