use super::*;

#[gpui::test]
fn diff_search_scrolls_sideways_to_a_match_far_along_a_long_line(cx: &mut gpui::TestAppContext) {
    // Measures Compact layout; a fresh session now defaults to Comfortable.
    cx.update(crate::appearance::pin_compact_for_test);
    assert_diff_search_scrolls_sideways(
        cx,
        gitcomet_state::model::RepoId(9141),
        "search_horizontal_reveal",
    );
}

#[gpui::test]
fn review_split_search_retries_when_one_column_has_stale_scroll_geometry(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let patch = "diff --git a/wide.txt b/wide.txt\n--- a/wide.txt\n+++ b/wide.txt\n@@ -1 +1 @@\n-old\n+new\n".to_string();
    push_file_patch_diff_state_with_rev(
        cx,
        &view,
        gitcomet_state::model::RepoId(9142),
        "split_search_retry",
        patch,
        1,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "split diff ready",
        |pane| pane.file_diff_cache_rev == 1 && pane.diff_visible_len() > 0,
        |pane| (pane.file_diff_cache_rev, pane.diff_visible_len()),
    );
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            })
    });
    draw_and_drain_test_window(cx);
    fn assert_pending_reveal(pane: &mut crate::view::panes::DiffRenderer, window: &mut Window) {
        let ix = pane
            .diff_text_hitboxes
            .keys()
            .find_map(|(ix, region)| {
                (*region == DiffTextRegion::SplitRight
                    && pane
                        .diff_text_hitboxes
                        .contains_key(&(*ix, DiffTextRegion::SplitLeft)))
                .then_some(*ix)
            })
            .expect("a row painted in both columns");
        for stale_region in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight] {
            for region in [DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight] {
                let hitbox = pane.diff_text_hitboxes.get_mut(&(ix, region)).unwrap();
                hitbox.painted_text = if region == stale_region {
                    format!("{}needle", "x".repeat(10_000)).into()
                } else {
                    "no match".into()
                };
                hitbox.streamed_ascii_monospace_cell_width = Some(px(8.0));
            }
            pane.diff_search_query = "needle".into();
            pane.diff_search_horizontal_reveal = Some((ix, 3));
            pane.apply_pending_diff_search_horizontal_reveal(window);
            assert_eq!(
                pane.diff_search_horizontal_reveal,
                Some((ix, 2)),
                "the settled opposite column dropped {stale_region:?}'s pending reveal"
            );
        }
    }

    // Run the reveal during layout, as production does: retrying schedules a
    // frame for the current rendered view.
    struct RevealProbe {
        pane: Entity<crate::view::panes::DiffRenderer>,
    }
    impl gpui::Render for RevealProbe {
        fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
            let pane = self.pane.clone();
            gpui::canvas(
                move |_, window, app| {
                    pane.update(app, |pane, _| {
                        assert_pending_reveal(pane, window);
                    });
                },
                |_, _, _, _| {},
            )
            .size_full()
        }
    }
    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.clone();
        window.replace_root(app, |_, _| RevealProbe { pane });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

/// A match far along a long line has to be scrolled to sideways as well as
/// down. Without it the row comes into view with the hit still off the right
/// edge, which reads as "search found it but did not go there".
fn assert_diff_search_scrolls_sideways(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
) {
    let mut unified = concat!(
        "diff --git a/wide.txt b/wide.txt\n",
        "--- a/wide.txt\n",
        "+++ b/wide.txt\n",
        "@@ -1,12 +1,12 @@\n",
    )
    .to_string();
    for ix in 0..12 {
        if ix == 6 {
            // The needle sits well past any plausible viewport width.
            unified.push_str(&format!(" {}needle tail\n", "pad ".repeat(200)));
        } else {
            unified.push_str(&format!(" context {ix}\n"));
        }
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(900.0), px(560.0)));
    push_file_patch_diff_state_with_rev(cx, &view, repo_id, fixture_name, unified, 1);
    wait_for_main_pane_condition(
        cx,
        &view,
        "wide file diff ready for horizontal search reveal",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.diff_visible_len() > 0
                && pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
        },
        |pane| {
            (
                pane.file_diff_cache_rev,
                pane.diff_visible_len(),
                pane.diff_view,
                pane.diff_word_wrap,
                pane.diff_scroll.0.borrow().base_handle.max_offset(),
            )
        },
    );

    cx.update(|_window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        diff_renderer.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle tail".into();
            pane.diff_search_input.update(cx, |input, cx| {
                input.set_text("needle tail", cx);
            });
            pane.diff_search_recompute_matches_and_scroll_to_first();
            cx.notify();
        });
    });
    // Three passes: the vertical jump lands, the row paints its hitbox, and the
    // sideways reveal reads that hitbox on the frame after.
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "expected the long line to be the only match, got {:?}",
            pane.diff_search_matches
        );
        let handle = pane.diff_scroll.0.borrow().base_handle.clone();
        assert!(
            handle.max_offset().x > px(0.0),
            "fixture must overflow horizontally for this to mean anything; max={:?}",
            handle.max_offset()
        );
        assert!(
            handle.offset().x < px(0.0),
            "expected the diff to scroll right to the match, x stayed at {:?} (mode={:?})",
            handle.offset(),
            pane.diff_view,
        );

        assert_eq!(
            pane.diff_search_horizontal_reveal, None,
            "the reveal should be claimed once, not re-applied every frame"
        );
    });
}
