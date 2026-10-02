//! Soft wrap: wrapped rows, column budgets, copy across continuations.

use super::*;

/// Two change blocks; the first one's row wraps, so its continuation rows sit
/// between the two navigation stops.
fn build_full_diff_word_wrap_navigation_fixture_texts() -> (String, String, String) {
    let old_one = format!("old first {}", "left_payload_".repeat(160));
    let new_one = format!("new first {}", "right_payload_".repeat(160));
    let old_two = "old second changed row".to_string();
    let new_two = "new second changed row".to_string();
    let old_text = format!("alpha\n{old_one}\nmiddle\n{old_two}\nomega\n");
    let new_text = format!("alpha\n{new_one}\nmiddle\n{new_two}\nomega\n");
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,5 +1,5 @@
 alpha
-{old_one}
+{new_one}
 middle
-{old_two}
+{new_two}
 omega
"
    );
    (unified, old_text, new_text)
}

fn build_collapsed_diff_word_wrap_navigation_fixture_texts() -> (String, String, String) {
    let total_lines = 100usize;
    let changes = [20usize, 60usize];
    let mut old_lines = (1..=total_lines)
        .map(|line| format!("line {line}"))
        .collect::<Vec<_>>();
    let mut new_lines = old_lines.clone();

    old_lines[changes[0] - 1] = format!("old value 20 {}", "left_payload_".repeat(160));
    new_lines[changes[0] - 1] = format!("new value 20 {}", "right_payload_".repeat(160));
    old_lines[changes[1] - 1] = "old value 60".to_string();
    new_lines[changes[1] - 1] = "new value 60".to_string();

    let old_text = format!("{}\n", old_lines.join("\n"));
    let new_text = format!("{}\n", new_lines.join("\n"));
    let mut unified = String::from(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
",
    );
    for line in changes {
        let context_start = line.saturating_sub(3).max(1);
        let context_end = (line + 3).min(total_lines);
        let context_count = context_end.saturating_sub(context_start).saturating_add(1);
        unified.push_str(&format!(
            "@@ -{context_start},{context_count} +{context_start},{context_count} @@\n"
        ));
        for current_line in context_start..=context_end {
            if current_line == line {
                unified.push_str(&format!("-{}\n", old_lines[current_line - 1]));
                unified.push_str(&format!("+{}\n", new_lines[current_line - 1]));
            } else {
                unified.push_str(&format!(" {}\n", old_lines[current_line - 1]));
            }
        }
    }

    (unified, old_text, new_text)
}

/// First changed collapsed file row at or after `from`, as a source-visible index.
fn collapsed_first_changed_source_visible_ix(
    pane: &crate::view::panes::main::MainPaneView,
    from: usize,
) -> usize {
    (from..pane.collapsed_diff_visible_rows.len())
        .find(|&source_visible_ix| {
            pane.collapsed_visible_row(source_visible_ix)
                .and_then(crate::view::panes::main::CollapsedDiffVisibleRow::row_ix)
                .is_some_and(|row_ix| pane.file_diff_row_is_change(row_ix))
        })
        .unwrap_or_else(|| panic!("expected a changed collapsed file row at or after {from}"))
}

#[gpui::test]
fn diff_word_wrap_toggles_full_file_diff_wrapped_row_path(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = "new line with enough text to exercise the soft wrap render path softwrapneedle abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz";
    let unified = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,2 +1,2 @@
 first line
-old line
+new line with enough text to exercise the soft wrap render path softwrapneedle abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz
"
    .to_string();
    let old_text = "first line\nold line\n".to_string();
    let new_text = format!("first line\n{long_new_line}\n");
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(187),
        "word_wrap_full_file_diff",
        path,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(false, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "full file diff ready for word wrap toggle",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.diff_visible_len() > 0
        },
        |pane| {
            format!(
                "cache_inflight={:?} file_active={} visible_len={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active(),
                pane.diff_visible_len(),
            )
        },
    );
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_word_wrap_scroll").is_none(),
        "word wrap off should render the file diff row list"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    cx.update(|_window, app| {
        assert!(crate::view::test_support::diff_word_wrap(view.read(app)));
        assert!(view.read(app).main_pane.read(app).diff_word_wrap);
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_word_wrap_scroll").is_none(),
        "word wrap on should stay on the normal highlighted diff row renderer"
    );
    assert!(
        cx.debug_bounds("diff_hscrollbar").is_none(),
        "word wrap on should suppress the horizontal scrollbar"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_visible_len() > pane.file_diff_inline_row_len(),
            "word wrap should expand long logical rows into continuation visual rows"
        );
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .any(|row| row.wrap_ix > 0),
            "wrapped continuation rows should be tracked separately from logical rows"
        );
    });

    let (_continuation_ix, source_visible_ix, continuation_start, continuation_text) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.diff_wrap_visible_rows
                .iter()
                .enumerate()
                .find_map(|(visible_ix, visual)| {
                    if visual.wrap_ix == 0 {
                        return None;
                    }
                    let row_ix = pane.diff_mapped_ix_for_visible_ix(visible_ix)?;
                    let row = pane.file_diff_inline_render_data(row_ix)?;
                    if !row.text.as_ref().contains("softwrapneedle") {
                        return None;
                    }
                    let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                    let full_text = pane.diff_text_full_line_for_region(
                        visual.source_visible_ix,
                        DiffTextRegion::Inline,
                    );
                    let start = full_text.as_ref().find(text.as_ref())?;
                    (!text.is_empty() && !row.text.as_ref().starts_with(text.as_ref())).then_some((
                        visible_ix,
                        visual.source_visible_ix,
                        start,
                        text.to_string(),
                    ))
                })
                .expect(
                    "expected a non-prefix wrapped continuation row for the long file-diff line",
                )
        });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: continuation_start,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: continuation_start + continuation_text.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(continuation_text.clone()),
        "copying a wrapped continuation row should copy the visible slice, not the start of the logical line"
    );

    let full_wrapped_line = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let full = pane
            .diff_text_full_line_for_region(source_visible_ix, DiffTextRegion::Inline)
            .to_string();
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .filter(|row| row.source_visible_ix == source_visible_ix)
                .count()
                > 1,
            "expected selected source row to be split across wrapped visual rows"
        );
        full
    });
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: full_wrapped_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "copying across wrapped visual rows should not insert soft-wrap newlines"
    );
    assert!(
        !full_wrapped_line.contains('\n'),
        "fixture line should be a single logical source line"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "softwrapneedle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("softwrapneedle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    rows::clear_diff_paint_log_for_tests();
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "search should match only the wrapped visual row containing the query"
        );
        let match_ix = pane.diff_search_matches[0];
        assert!(
            pane.diff_text_line_for_region(match_ix, DiffTextRegion::Inline)
                .as_ref()
                .contains("softwrapneedle"),
            "the active search match should point at the wrapped slice that contains the query"
        );
    });

    let boundary_query_start = continuation_start.saturating_sub(8);
    let boundary_query_end = (continuation_start + 8).min(full_wrapped_line.len());
    assert!(
        boundary_query_start < continuation_start && continuation_start < boundary_query_end,
        "expected enough text around the soft-wrap boundary"
    );
    let soft_wrap_boundary_literal_query = format!(
        "{}{}",
        &full_wrapped_line[boundary_query_start..continuation_start],
        &full_wrapped_line[continuation_start..boundary_query_end]
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = soft_wrap_boundary_literal_query.clone().into();
                let query_for_input = soft_wrap_boundary_literal_query.clone();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query_for_input, cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "literal search should match across a soft-wrap boundary in the source row"
        );
        let match_ix = pane.diff_search_matches[0];
        assert_eq!(
            pane.diff_wrap_visible_rows
                .get(match_ix)
                .map(|row| row.source_visible_ix),
            Some(source_visible_ix),
            "wrapped boundary matches should map back to the source row"
        );
    });

    let soft_wrap_boundary_query = format!(
        "{}\n{}",
        &full_wrapped_line[boundary_query_start..continuation_start],
        &full_wrapped_line[continuation_start..boundary_query_end]
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = soft_wrap_boundary_query.clone().into();
                let query_for_input = soft_wrap_boundary_query.clone();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query_for_input, cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.is_empty(),
            "search should not treat a soft-wrap boundary as a real newline"
        );
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "softwrapneedle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("softwrapneedle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
            });
        });
    });

    let paint_log = rows::diff_paint_log_for_tests();
    let highlighted_text = paint_log
        .iter()
        .flat_map(|record| {
            record
                .highlights
                .iter()
                .filter(|(_, _, background)| background.is_some())
                .filter_map(|(range, _, _)| record.text.as_ref().get(range.clone()))
        })
        .collect::<Vec<_>>()
        .join("");
    assert!(
        highlighted_text.contains("softwrapneedle"),
        "wrapped row rendering should preserve search highlighting"
    );

    let key_before_resize = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
    });
    cx.simulate_resize(gpui::size(px(760.0), px(520.0)));
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_ne!(
            pane.diff_wrap_visible_cache_key, key_before_resize,
            "resizing should rebuild the wrap projection key"
        );
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "search matches should be recomputed in the resized wrapped-row index space"
        );
        let match_ix = pane.diff_search_matches[0];
        assert!(
            match_ix < pane.diff_visible_len()
                && pane
                    .diff_text_line_for_region(match_ix, DiffTextRegion::Inline)
                    .as_ref()
                    .contains("softwrapneedle"),
            "resized search match should still point at the visual row containing the query"
        );
        assert!(
            !pane.diff_scrollbar_markers_cache.is_empty(),
            "scrollbar markers should be recomputed after the wrap projection changes"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(false, cx);
        });
    });
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_word_wrap_scroll").is_none(),
        "turning word wrap off should restore the file diff row list"
    );
}

#[gpui::test]
fn split_diff_word_wrap_copy_omits_soft_wrap_newlines(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(860.0), px(460.0)));

    let (unified, old_text, new_text) = build_full_diff_word_wrap_navigation_fixture_texts();
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(2192),
        "split_word_wrap_copy_real_content",
        PathBuf::from("src/lib.rs"),
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "split file diff ready for word-wrap copy",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.diff_visible_len() > 0
        },
        |pane| {
            format!(
                "cache_inflight={:?} file_active={} visible_len={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active(),
                pane.diff_visible_len(),
            )
        },
    );
    draw_and_drain_test_window(cx);

    let (source_visible_ix, full_wrapped_line, next_source_visible_ix, next_source_line) = cx
        .update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (source_visible_ix, full_wrapped_line) = pane
                .diff_wrap_visible_rows
                .iter()
                .map(|row| row.source_visible_ix)
                .find_map(|source_visible_ix| {
                    let full = pane
                        .diff_text_full_line_for_region(
                            source_visible_ix,
                            DiffTextRegion::SplitRight,
                        )
                        .to_string();
                    if !full.contains("right_payload_") {
                        return None;
                    }
                    let wrap_row_count = pane
                        .diff_wrap_visible_rows
                        .iter()
                        .filter(|row| row.source_visible_ix == source_visible_ix)
                        .count();
                    (wrap_row_count > 1).then_some((source_visible_ix, full))
                })
                .expect("expected a wrapped split-right source row");
            let (next_source_visible_ix, next_source_line) = pane
                .diff_wrap_visible_rows
                .iter()
                .map(|row| row.source_visible_ix)
                .find_map(|ix| {
                    if ix <= source_visible_ix {
                        return None;
                    }
                    let text = pane
                        .diff_text_full_line_for_region(ix, DiffTextRegion::SplitRight)
                        .to_string();
                    (!text.is_empty()).then_some((ix, text))
                })
                .expect("expected a following real split-right source row");
            (
                source_visible_ix,
                full_wrapped_line,
                next_source_visible_ix,
                next_source_line,
            )
        });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: full_wrapped_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "split diff copy should not insert soft-wrap newlines"
    );
    assert!(!full_wrapped_line.contains('\n'));

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix: next_source_visible_ix,
                region: DiffTextRegion::SplitRight,
                offset: next_source_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(format!("{full_wrapped_line}\n{next_source_line}")),
        "copying real source rows should still preserve real line breaks"
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_continuation_rows_use_source_visible_row(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let repo_id = gitcomet_state::model::RepoId(188);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_word_wrap_source_row",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let (continuation_ix, source_visible_ix, expected_text) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_wrap_visible_rows
            .iter()
            .enumerate()
            .find_map(|(visible_ix, visual)| {
                if visual.wrap_ix == 0 {
                    return None;
                }
                let Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow { row_ix }) =
                    pane.collapsed_visible_row(visual.source_visible_ix)
                else {
                    return None;
                };
                let row = pane.file_diff_inline_render_data(row_ix)?;
                if !row.text.as_ref().contains("right_payload_") {
                    return None;
                }
                let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                text.as_ref().contains("right_payload_").then_some((
                    visible_ix,
                    visual.source_visible_ix,
                    text.to_string(),
                ))
            })
            .expect("expected a wrapped collapsed file row continuation")
    });
    assert_ne!(
        continuation_ix, source_visible_ix,
        "the regression needs a continuation row whose visual index differs from the source row"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(continuation_ix, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });
    rows::clear_diff_paint_log_for_tests();
    draw_and_drain_test_window(cx);
    let record = rows::diff_paint_log_for_tests()
        .into_iter()
        .find(|record| {
            record.visible_ix == continuation_ix && record.region == DiffTextRegion::Inline
        })
        .unwrap_or_else(|| {
            panic!("expected paint record for wrapped continuation {continuation_ix}")
        });
    assert_eq!(
        record.text.as_ref(),
        expected_text,
        "collapsed wrapped continuation rows should render the source row slice, not the next collapsed logical row"
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_copy_uses_continuation_slice(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let repo_id = gitcomet_state::model::RepoId(190);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_word_wrap_copy_slice",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let (continuation_ix, source_visible_ix, selection_start, expected_text) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.diff_wrap_visible_rows
                .iter()
                .enumerate()
                .find_map(|(visible_ix, visual)| {
                    if visual.wrap_ix == 0 {
                        return None;
                    }
                    let Some(crate::view::panes::main::CollapsedDiffVisibleRow::FileRow {
                        row_ix,
                    }) = pane.collapsed_visible_row(visual.source_visible_ix)
                    else {
                        return None;
                    };
                    let row = pane.file_diff_inline_render_data(row_ix)?;
                    if !row.text.as_ref().contains("right_payload_") {
                        return None;
                    }
                    let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                    if !text.as_ref().contains("right_payload_") {
                        return None;
                    }
                    let (_, range) = pane
                        .diff_text_visual_source_range_for_region(visible_ix, DiffTextRegion::Inline);
                    Some((
                        visible_ix,
                        visual.source_visible_ix,
                        range.start,
                        text.to_string(),
                    ))
                })
                .expect("expected a wrapped collapsed file row continuation for copy")
        });
    assert_ne!(
        continuation_ix, source_visible_ix,
        "the copy regression needs a visual continuation row"
    );

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: selection_start,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: selection_start + expected_text.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.sync_diff_focus_to_text_selection();
            cx.notify();
        });
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected_text),
        "Ctrl-C should copy the same wrapped slice that is selected and painted"
    );

    let full_wrapped_line = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let full = pane
            .diff_text_full_line_for_region(source_visible_ix, DiffTextRegion::Inline)
            .to_string();
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .filter(|row| row.source_visible_ix == source_visible_ix)
                .count()
                > 1,
            "expected selected collapsed row to be split across wrapped visual rows"
        );
        full
    });
    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: 0,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: full_wrapped_line.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.copy_selected_diff_text_to_clipboard(cx);
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(full_wrapped_line.clone()),
        "collapsed diff copy should not insert soft-wrap newlines"
    );
    assert!(
        !full_wrapped_line.contains('\n'),
        "fixture line should be a single logical source line"
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_selection_survives_resize(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let repo_id = gitcomet_state::model::RepoId(191);
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        "collapsed_word_wrap_resize_selection",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let marker = "right_payload_";
    let (source_visible_ix, marker_start, key_before_resize) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (source_visible_ix, marker_start) = pane
            .diff_wrap_visible_rows
            .iter()
            .enumerate()
            .find_map(|(visible_ix, visual)| {
                if visual.wrap_ix == 0 {
                    return None;
                }
                let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
                let local = text.as_ref().find(marker)?;
                let (_, range) = pane
                    .diff_text_visual_source_range_for_region(visible_ix, DiffTextRegion::Inline);
                Some((visual.source_visible_ix, range.start + local))
            })
            .expect("expected marker on a wrapped collapsed continuation row");
        (
            source_visible_ix,
            marker_start,
            pane.diff_wrap_visible_cache_key,
        )
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_text_anchor = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: marker_start,
            });
            pane.diff_text_head = Some(DiffTextPos {
                source_visible_ix,
                region: DiffTextRegion::Inline,
                offset: marker_start + marker.len(),
            });
            pane.diff_text_selection_owner.adopt(window, cx);
            pane.sync_diff_focus_to_text_selection();
            cx.notify();
        });
        let focus = main_pane.read(app).diff_panel_focus_handle.clone();
        window.focus(&focus, app);
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(marker.to_string())
    );

    cx.simulate_resize(gpui::size(px(650.0), px(420.0)));
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_ne!(
            pane.diff_wrap_visible_cache_key, key_before_resize,
            "resizing should rebuild wrapped visual rows"
        );
    });

    cx.simulate_keystrokes("ctrl-c");
    let copied_after_resize = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("expected copied selection after resize");
    assert_eq!(
        copied_after_resize.replace('\n', ""),
        marker,
        "the selection should remain anchored to the same source text after wrap rows rebuild"
    );
}

/// Every row that did not wrap measures its own line, and at least one such
/// row sits below a wrapped line, where row position and line number differ.
fn assert_unwrapped_rows_measure_their_own_line(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut shifted_rows = 0;
        for visible_ix in 0..pane.diff_wrap_visible_rows.len() {
            if pane.diff_text_wrap_for_visible_ix(visible_ix).is_some() {
                continue;
            }
            let source_ix = pane
                .diff_source_visible_ix_for_visible_ix(visible_ix)
                .expect("every row maps to a line");
            if source_ix != visible_ix {
                shifted_rows += 1;
            }
            assert_eq!(
                pane.diff_text_line_len_for_region(visible_ix, DiffTextRegion::Inline),
                pane.diff_text_full_line_for_region(source_ix, DiffTextRegion::Inline)
                    .len(),
                "row {visible_ix} measures line {source_ix}"
            );
        }
        assert!(
            shifted_rows > 0,
            "the fixture needs rows below a wrapped line"
        );
    });
}

#[gpui::test]
fn collapsed_diff_word_wrap_measures_rows_below_a_wrapped_line(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        gitcomet_state::model::RepoId(192),
        "collapsed_word_wrap_row_len",
        DiffViewMode::Inline,
        unified,
        old_text,
        new_text,
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_word_wrap(true, cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert_unwrapped_rows_measure_their_own_line(cx, &view);
}

#[gpui::test]
fn full_diff_word_wrap_measures_rows_below_a_wrapped_line(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(820.0), px(420.0)));

    let long_new_line = format!("wrapped {}", "payload ".repeat(60));
    let tail = (1..=6)
        .map(|n| format!("{}\n", "x".repeat(n)))
        .collect::<String>();
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old line
+{long_new_line}
"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(193),
        "full_word_wrap_row_len",
        PathBuf::from("src/lib.rs"),
        unified,
        format!("old line\n{tail}"),
        format!("{long_new_line}\n{tail}"),
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "full diff fixture ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| format!("cache_inflight={:?}", pane.file_diff_cache_inflight),
    );
    draw_and_drain_test_window(cx);

    assert_unwrapped_rows_measure_their_own_line(cx, &view);
}

#[gpui::test]
fn diff_word_wrap_columns_follow_scaled_font_metrics(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = format!("scaled {}", "wrapmetric".repeat(32));
    let unified = format!(
        "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1 +1 @@
-old line
+{long_new_line}
"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(189),
        "word_wrap_scaled_font_metrics",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "scaled font metrics fixture ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "cache_inflight={:?} file_active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);
    let default_columns = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
            .expect("expected default wrap cache key")
            .inline_columns
    });

    cx.update(|_window, app| {
        crate::ui_scale::set_default(app, 200);
    });
    draw_and_drain_test_window(cx);
    let zoomed_columns = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
            .expect("expected zoomed wrap cache key")
            .inline_columns
    });

    assert!(
        zoomed_columns < default_columns,
        "wrap columns should decrease when the active diff font scales up (default={default_columns}, zoomed={zoomed_columns})"
    );

    cx.update(|_window, app| {
        crate::ui_scale::set_default(app, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
        let mut appearance = crate::appearance::current(app);
        appearance.editor_font_size_px = 26;
        app.set_global(appearance);
        view.update(app, |view, cx| view.notify_font_preferences_changed(cx));
    });
    draw_and_drain_test_window(cx);
    let larger_editor_columns = cx.update(|_, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_wrap_visible_cache_key
            .expect("expected updated wrap cache after changing editor size")
            .inline_columns
    });
    assert!(
        larger_editor_columns < default_columns,
        "changing only the editor font must remeasure wrapping and gutters"
    );
}

#[gpui::test]
async fn diff_word_wrap_column_count_consistency_with_available_width(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1200.0), px(600.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = format!("consistency {}", "x".repeat(200));
    let unified = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1 +1 @@\n\
         -old line\n\
         +{long_new_line}\n"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(199),
        "wrap_column_consistency",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "wrap column consistency ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "inflight={:?} active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);

    let (inline_columns, char_width, show_line_numbers) = cx.update(|window, app| {
        let editor_font_family = crate::font_preferences::current_editor_font_family(app);
        let pane = view.read(app).main_pane.read(app);
        let cache_key = pane
            .diff_wrap_visible_cache_key
            .expect("wrap cache key must be populated after rendering with wrap on");
        let inline_columns = cache_key.inline_columns;
        let char_width = rows::diff_canvas_text_wrap_char_width(
            window,
            editor_font_family,
            crate::appearance::Appearance::default().editor_font_size_px,
        );
        let show_line_numbers = pane.diff_show_line_numbers;
        (inline_columns, char_width, show_line_numbers)
    });

    // Compute expected text-area pixel width.
    let ui_scale_percent = 100u32;
    let content_width = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        crate::view::panes::main::pane_content_width_for_layout(
            pane.last_window_size.width,
            pane.layout_sidebar_render_width,
            pane.layout_details_render_width,
            pane.layout_sidebar_collapsed,
            pane.layout_details_collapsed,
        )
    });
    let scrollbar_gutter = components::Scrollbar::gutter(components::ScrollbarAxis::Vertical);
    let available_width = (content_width - scrollbar_gutter).max(px(0.0));
    let pad = rows::diff_canvas_row_horizontal_padding(ui_scale_percent);
    let inline_text_start = if show_line_numbers {
        rows::diff_canvas_inline_text_start(ui_scale_percent)
    } else {
        pad
    };
    let text_area_px = (available_width - inline_text_start - pad).max(px(0.0));

    let expected_columns = diff_wrap_column_for_width(text_area_px, char_width);

    assert!(
        inline_columns > 1,
        "wrap columns should be > 1 (got {inline_columns})"
    );

    let diff = inline_columns.abs_diff(expected_columns);
    let max_tol = (expected_columns / 10).max(2);
    assert!(
        diff <= max_tol,
        "inline_columns ({inline_columns}) differs from expected ({expected_columns}) \
         by {diff}, max acceptable {max_tol} \
         (text_area_px={text_area_px:?}, char_width={char_width:?})"
    );

    let occupied_px = char_width * inline_columns as f32;
    assert!(
        occupied_px <= text_area_px + char_width,
        "wrapped text width ({occupied_px:?}) should not exceed text area \
         ({text_area_px:?}) by more than one char width"
    );

    let unused = (text_area_px - occupied_px).max(px(0.0));
    assert!(
        unused <= char_width * 3.0,
        "unused space ({unused:?}) should be < 3 chars ({:?}) — \
         lines break too early if larger",
        char_width * 3.0
    );
}

fn diff_wrap_column_for_width(width: Pixels, char_width: Pixels) -> usize {
    let cw = f32::from(char_width.max(px(1.0)));
    ((f32::from(width.max(px(0.0))) / cw).floor() as usize).max(1)
}

/// Display columns a wrapped segment occupies, matching the tab expansion the
/// wrap algorithm uses (`DIFF_WRAP_TAB_EXPANDED_COLUMNS`).
fn wrap_display_columns(text: &str) -> usize {
    text.chars().map(|ch| if ch == '\t' { 4 } else { 1 }).sum()
}

/// Greedy word wrap must emit *maximal* segments: a non-final segment may only
/// stop short of the column budget when the next word could not have fit.
///
/// This is the invariant that "lines break too early" violates. It is stated in
/// columns rather than pixels on purpose — `#[gpui::test]` runs on gpui's
/// `NoopTextSystem`, where every glyph advances an identical 0.6em regardless of
/// font, so pixel measurements taken in a test cannot distinguish fonts at all.
#[gpui::test]
async fn diff_word_wrap_segments_are_maximal_for_their_column_budget(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    // Short words only, so every break lands on whitespace. A segment that
    // stops early here is a genuine wrap bug — unlike a single long token,
    // which correctly gets pushed to the next row and leaves the previous one
    // partly empty.
    let long_new_line = "let value = compute(alpha, beta, gamma) + delta * epsilon - zeta / eta; "
        .repeat(6)
        .trim_end()
        .to_string();
    let unified = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1 +1 @@\n\
         -old line\n\
         +{long_new_line}\n"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(200),
        "wrap_segments_maximal",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "wrap segments ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "inflight={:?} active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let wrap_columns = pane
            .diff_wrap_visible_cache_key
            .expect("wrap cache key must be populated after rendering with wrap on")
            .inline_columns;
        assert!(
            wrap_columns > 8,
            "degenerate wrap budget ({wrap_columns} columns)"
        );

        let visible_len = pane.diff_visible_len();
        let mut checked = 0usize;
        for visible_ix in 0..visible_len {
            let rows = &pane.diff_wrap_visible_rows;
            let (Some(row), Some(next_row)) = (rows.get(visible_ix), rows.get(visible_ix + 1))
            else {
                continue;
            };
            // Only non-final segments of a wrapped source row are constrained;
            // the last segment is free to be short.
            if next_row.source_visible_ix != row.source_visible_ix {
                continue;
            }

            let text = pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline);
            let next_text = pane.diff_text_line_for_region(visible_ix + 1, DiffTextRegion::Inline);
            if text.is_empty() || next_text.is_empty() {
                continue;
            }

            let used = wrap_display_columns(text.as_ref());
            // What appending the next row's first word to this row would have
            // cost, including any whitespace the row opens with (a row starts
            // with whitespace when the previous word ended exactly on the
            // column boundary).
            let next_word: String = {
                let next_text = next_text.as_ref();
                let leading_ws = next_text.len() - next_text.trim_start().len();
                let word = next_text
                    .trim_start()
                    .chars()
                    .take_while(|ch| !ch.is_whitespace());
                next_text[..leading_ws].chars().chain(word).collect()
            };
            let next_word_columns = wrap_display_columns(&next_word);

            assert!(
                used <= wrap_columns,
                "visible_ix={visible_ix}: segment overflows its budget \
                 ({used} columns of {wrap_columns}). text={text:?}"
            );
            assert!(
                used + next_word_columns > wrap_columns,
                "visible_ix={visible_ix}: line broke too early — {used} of \
                 {wrap_columns} columns used and the next word {next_word:?} \
                 ({next_word_columns} columns) would still have fit. \
                 text={text:?}"
            );
            checked += 1;
        }
        assert!(
            checked > 0,
            "expected at least one source row that wraps to multiple visual rows"
        );
    });
}

/// Wrap columns must be measured in the font the rows are *painted* in.
///
/// `MainPane::diff_wrap_columns` runs while the diff pane is building its
/// element tree, before the rows container pushes
/// `.font_family(editor_font_family)` onto the window text style stack. The
/// ambient style there is a proportional UI font, and the wrap width sample is
/// `"WWWWWWWWWW"` — the widest glyph in a proportional face (IBM Plex Sans `W`
/// is 0.891em against Lilex's uniform 0.600em), which overestimated the column
/// width by ~1.5x and wrapped every line at roughly two thirds of the width it
/// actually had.
///
/// The assertion is on font *identity*, not measured width: gpui's test
/// `NoopTextSystem` maps every font descriptor to the same `FontId` and every
/// glyph to the same advance, so no width-based test can catch this.
#[gpui::test]
async fn diff_word_wrap_columns_are_measured_in_the_editor_font(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1000.0), px(520.0)));

    let path = PathBuf::from("src/lib.rs");
    let long_new_line = format!("fonttest {}", "abc def ghi ".repeat(30));
    let unified = format!(
        "diff --git a/src/lib.rs b/src/lib.rs\n\
         index 1111111..2222222 100644\n\
         --- a/src/lib.rs\n\
         +++ b/src/lib.rs\n\
         @@ -1 +1 @@\n\
         -old line\n\
         +{long_new_line}\n"
    );
    push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(201),
        "wrap_measure_font",
        path,
        unified,
        "old line\n".to_string(),
        format!("{long_new_line}\n"),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Inline;
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "wrap measure font ready",
        |pane| pane.file_diff_cache_inflight.is_none() && pane.is_file_diff_view_active(),
        |pane| {
            format!(
                "inflight={:?} active={}",
                pane.file_diff_cache_inflight,
                pane.is_file_diff_view_active()
            )
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let editor_font_family = crate::font_preferences::current_editor_font_family(app);
        let main_pane = view.read(app).main_pane.clone();
        let measured = main_pane.update(app, |pane, cx| pane.diff_wrap_measure_font_family(cx));

        assert_eq!(
            measured.as_ref(),
            editor_font_family.as_str(),
            "wrap columns must be measured in the editor font the rows are painted in"
        );
        // The trap: outside the rows container the ambient text style is never
        // the editor font, so measuring against `window.text_style()` silently
        // measures the wrong face.
        assert_ne!(
            window.text_style().font_family.as_ref(),
            editor_font_family.as_str(),
            "ambient text style unexpectedly matches the editor font — this test \
             no longer guards anything"
        );
    });
}

fn assert_full_diff_word_wrap_change_shortcuts_skip_continuations(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    cx.simulate_resize(gpui::size(px(760.0), px(420.0)));
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_full_diff_word_wrap_navigation_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        view,
        repo_id,
        fixture_name,
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff word-wrap fixture activates file diff view",
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
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = diff_view;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_autoscroll_pending = false;
                pane.clear_diff_text_selection();
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "full diff word-wrap navigation entries are visual rows",
        |pane| {
            let entries = pane.diff_nav_entries();
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == diff_view
                && pane.diff_word_wrap
                && pane.diff_wrap_visible_cache_key.is_some()
                && pane
                    .diff_wrap_visible_rows
                    .iter()
                    .any(|row| row.wrap_ix > 0)
                && entries.len() >= 2
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_wrap_visible_cache_key,
                pane.diff_nav_entries(),
            )
        },
    );

    let (first_entry, second_entry) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let entries = pane.diff_nav_entries();
        let first_entry = entries[0];
        let second_entry = entries[1];
        let first_row = pane.diff_wrap_visible_rows[first_entry];
        let second_row = pane.diff_wrap_visible_rows[second_entry];
        assert_eq!(
            first_row.wrap_ix, 0,
            "first navigation entry should be the first visual row of its change block"
        );
        assert_eq!(
            second_row.wrap_ix, 0,
            "second navigation entry should be the first visual row of its change block"
        );
        assert!(
            second_row.source_visible_ix > first_row.source_visible_ix,
            "second navigation entry should advance to the next change block"
        );
        let has_wrapped_continuation_between_entries = pane
            .diff_wrap_visible_rows
            .iter()
            .enumerate()
            .any(|(visible_ix, row)| {
                visible_ix > first_entry
                    && visible_ix < second_entry
                    && row.source_visible_ix == first_row.source_visible_ix
                    && row.wrap_ix > 0
            });
        assert!(
            has_wrapped_continuation_between_entries,
            "fixture should put wrapped continuation rows between the first two navigation entries; entries={entries:?}, first_row={first_row:?}, second_row={second_row:?}"
        );
        (first_entry, second_entry)
    });

    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "first F3 should select the first change block in wrapped Full diff {diff_view:?}"
        );
    });
    let expected_bar_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let rows = &pane.diff_wrap_visible_rows;
        // One unchanged row ("middle") separates the two blocks.
        let block = rows[first_entry].source_visible_ix..rows[second_entry].source_visible_ix - 1;
        let expected = (0..rows.len())
            .filter(|&ix| block.contains(&rows[ix].source_visible_ix))
            .collect::<Vec<_>>();
        assert!(
            expected.iter().any(|&ix| rows[ix].wrap_ix > 0),
            "fixture's first block should wrap in {diff_view:?}"
        );
        expected
    });
    assert_eq!(
        focused_change_block_rows(cx, view),
        expected_bar_rows,
        "the focus bar should cover the block's wrapped continuation rows in {diff_view:?}"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(second_entry),
            "second F3 should skip wrap continuations in wrapped Full diff {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "F2 should move back to the previous change block in wrapped Full diff {diff_view:?}"
        );
    });
}

#[gpui::test]
fn full_diff_word_wrap_inline_change_shortcuts_skip_continuation_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_word_wrap_change_shortcuts_skip_continuations(
        cx,
        &view,
        gitcomet_state::model::RepoId(70603),
        "full_diff_word_wrap_inline_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn full_diff_word_wrap_inline_change_shortcuts_map_provider_rows_through_visible_map(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(760.0), px(420.0)));
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_full_diff_word_wrap_navigation_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(70607),
        "full_diff_word_wrap_inline_visible_map_nav",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "full diff word-wrap visible-map fixture activates file diff view",
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
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.set_diff_content_mode(DiffContentMode::Full, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_autoscroll_pending = false;
                pane.clear_diff_text_selection();
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "full diff word-wrap visible-map fixture has wrapped inline rows",
        |pane| {
            pane.diff_content_mode == DiffContentMode::Full
                && pane.diff_view == DiffViewMode::Inline
                && pane.diff_word_wrap
                && pane.diff_wrap_visible_cache_key.is_some()
                && pane
                    .diff_wrap_visible_rows
                    .iter()
                    .any(|row| row.wrap_ix > 0)
                && pane
                    .file_diff_inline_row_provider
                    .as_ref()
                    .is_some_and(|provider| !provider.change_blocks().is_empty())
        },
        |pane| {
            (
                pane.diff_content_mode,
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_wrap_visible_cache_key,
                pane.diff_nav_entries(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let inline_len = pane.file_diff_inline_row_len();
                assert!(inline_len > 1, "fixture should expose file inline rows");
                let mut hidden = vec![false; inline_len];
                hidden[0] = true;
                pane.diff_visible_inline_map =
                    Some(PatchInlineVisibleMap::from_hidden_flags(hidden.as_slice()));
                pane.diff_visible_indices = Arc::from([]);
                pane.diff_wrap_visible_rows = Arc::from([]);
                pane.diff_wrap_visible_cache_key = None;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.clear_diff_text_selection();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let provider = pane
            .file_diff_inline_row_provider
            .as_ref()
            .expect("fixture should use the paged inline file provider");
        let first_changed_provider_ix = provider
            .change_blocks()
            .first()
            .map(|block| block.start)
            .expect("fixture should contain a changed inline row");
        let visible_map = pane
            .diff_visible_inline_map
            .as_ref()
            .expect("test should install a non-identity inline visible map");
        let first_changed_source_visible_ix = visible_map
            .visible_ix_for_src_ix(first_changed_provider_ix)
            .expect("changed provider row should remain visible");
        assert_ne!(
            first_changed_provider_ix, first_changed_source_visible_ix,
            "fixture should exercise a non-identity provider-to-visible mapping"
        );

        let entries = pane.diff_nav_entries();
        let expected_first_entry =
            pane.diff_visual_ix_for_source_visible_ix(first_changed_source_visible_ix);
        assert_eq!(
            entries.first().copied(),
            Some(expected_first_entry),
            "wrapped Full inline diff navigation should convert provider rows through the visible map"
        );
        assert_eq!(
            pane.diff_wrap_visible_rows
                .get(expected_first_entry)
                .map(|row| row.source_visible_ix),
            Some(first_changed_source_visible_ix),
            "navigation should target the first wrapped visual row for the mapped source-visible row"
        );
    });
}

#[gpui::test]
fn full_diff_word_wrap_split_change_shortcuts_skip_continuation_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_full_diff_word_wrap_change_shortcuts_skip_continuations(
        cx,
        &view,
        gitcomet_state::model::RepoId(70604),
        "full_diff_word_wrap_split_nav",
        DiffViewMode::Split,
    );
}

fn assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_block_starts(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    cx.simulate_resize(gpui::size(px(760.0), px(420.0)));
    let (unified, old_text, new_text) = build_collapsed_diff_word_wrap_navigation_fixture_texts();
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

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.clear_diff_text_selection();
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        view,
        "collapsed diff word-wrap navigation entries are visual rows",
        |pane| {
            pane.diff_view == diff_view
                && pane.diff_word_wrap
                && pane.is_collapsed_diff_projection_active()
                && pane.diff_wrap_visible_cache_key.is_some()
                && pane.collapsed_diff_hunk_visible_indices.len() >= 2
                && pane.diff_nav_entries().len() >= 2
        },
        |pane| {
            (
                pane.diff_view,
                pane.diff_visible_len(),
                pane.diff_wrap_visible_cache_key,
                pane.collapsed_diff_hunk_visible_indices.clone(),
                pane.diff_nav_entries(),
            )
        },
    );

    let (first_entry, second_entry) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        // Each hunk holds one change; its stop is the first changed row, not
        // the hunk's header or leading context.
        let raw_first =
            collapsed_first_changed_source_visible_ix(pane, pane.collapsed_diff_hunk_visible_indices[0]);
        let raw_second =
            collapsed_first_changed_source_visible_ix(pane, pane.collapsed_diff_hunk_visible_indices[1]);
        let expected_first = pane.diff_visual_ix_for_source_visible_ix(raw_first);
        let expected_second = pane.diff_visual_ix_for_source_visible_ix(raw_second);
        assert_eq!(
            pane.diff_nav_entries(),
            vec![expected_first, expected_second],
            "collapsed nav entries should be the mapped visual rows of each block's first changed row"
        );
        assert_ne!(
            expected_second, raw_second,
            "fixture should expose the stale source-visible second block index regression"
        );
        let second_row = pane.diff_wrap_visible_rows[expected_second];
        assert_eq!(second_row.source_visible_ix, raw_second);
        assert_eq!(
            second_row.wrap_ix, 0,
            "collapsed navigation should land on the first visual row of the block's first row"
        );
        assert!(
            pane.diff_wrap_visible_rows
                .iter()
                .take(expected_second)
                .any(|row| row.wrap_ix > 0),
            "fixture should include wrapped visual rows before the second block"
        );
        (expected_first, expected_second)
    });

    set_diff_row_selection_for_test(cx, view, first_entry, (first_entry, first_entry));
    focus_diff_panel(cx, view);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(second_entry),
            "F3 should navigate to the next collapsed change block's visual row in {diff_view:?}"
        );
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app).main_pane.read(app).diff_selection_anchor,
            Some(first_entry),
            "F2 should navigate back to the previous collapsed change block's visual row in {diff_view:?}"
        );
    });
}

#[gpui::test]
fn collapsed_diff_word_wrap_inline_change_shortcuts_use_visual_block_starts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_block_starts(
        cx,
        &view,
        gitcomet_state::model::RepoId(70605),
        "collapsed_diff_word_wrap_inline_nav",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_word_wrap_split_change_shortcuts_use_visual_block_starts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_word_wrap_change_shortcuts_use_visual_block_starts(
        cx,
        &view,
        gitcomet_state::model::RepoId(70606),
        "collapsed_diff_word_wrap_split_nav",
        DiffViewMode::Split,
    );
}
