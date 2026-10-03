//! Clicks and selection: bracket matching, hunk-header menus, row selection.

use super::*;

/// Clicking a delimiter in the split file diff lights it and its partner.
///
/// The projection has to route through the *side's* real document: a diff
/// interleaves two file versions, so a raw row index says nothing on its own.
#[gpui::test]
fn split_file_diff_click_lights_the_matching_json_braces(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(931);
    let path = PathBuf::from("config.json");
    let old_text = "{\n  \"items\": [1, 2],\n  \"name\": \"old\"\n}\n".to_string();
    let new_text = "{\n  \"items\": [1, 2],\n  \"name\": \"new\"\n}\n".to_string();
    let unified = concat!(
        "@@ -1,4 +1,4 @@\n",
        " {\n",
        "   \"items\": [1, 2],\n",
        "-  \"name\": \"old\"\n",
        "+  \"name\": \"new\"\n",
        " }\n",
    )
    .to_string();

    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "split_pair_json",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "split file diff with prepared syntax on the new side",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.diff_view == DiffViewMode::Split
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "file_diff_active={} view={:?} doc={}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
            )
        },
    );

    // Row 1 on the right side is `  "items": [1, 2],` -- brackets at 11 and 16.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        1,
        DiffTextRegion::SplitRight,
        11..12,
        "split diff pair bracket hitbox",
    );
    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("clicking `[` in the split diff should light its pair");
        assert_eq!(pair.kind, rows::SyntaxPairKind::Bracket);
        assert_eq!(
            pair.spans
                .iter()
                .map(|span| (span.source_visible_ix, span.region, span.range.clone()))
                .collect::<Vec<_>>(),
            vec![
                (1, DiffTextRegion::SplitRight, 11..12),
                (1, DiffTextRegion::SplitRight, 16..17),
            ],
            "both ends land on the right-hand side, never across the split"
        );
        assert_eq!(
            pane.diff_text_local_pair_ranges(1, DiffTextRegion::SplitRight)
                .into_vec(),
            vec![11..12, 16..17]
        );
        assert!(
            pane.diff_text_local_pair_ranges(1, DiffTextRegion::SplitLeft)
                .is_empty(),
            "the left side renders the old document and must not be washed"
        );
    });

    // And it actually reaches the paint pass -- the state being right is not the
    // same as a quad being drawn on the row.
    cx.update(|_window, _app| rows::clear_diff_paint_log_for_tests());
    draw_and_drain_test_window(cx);
    let painted = rows::diff_paint_log_for_tests()
        .into_iter()
        .find(|record| record.visible_ix == 1 && record.region == DiffTextRegion::SplitRight)
        .expect("row 1 of the right column should have painted");
    assert_eq!(
        painted.pair_quads,
        vec![11..12, 16..17],
        "the pair quad must be painted on the row, not merely computed"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_text_occurrences
                    .entry((1, DiffTextRegion::SplitRight))
                    .or_default()
                    .push(3..8);
            });
        });
    });
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_pair_match_for_tests().is_none(),
            "changing the row projection must discard pair spans keyed by the old rows"
        );
        assert!(
            pane.diff_text_occurrences_for_tests().is_empty(),
            "changing the row projection must discard occurrence spans keyed by the old rows"
        );
    });
}

/// The inline diff routes every row through the side its text came from, so a
/// context row pairs against the new document and a removed row against the old.
#[gpui::test]
fn inline_file_diff_click_lights_the_matching_json_braces(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(932);
    let path = PathBuf::from("config.json");
    let old_text = "{\n  \"items\": [1, 2],\n  \"name\": \"old\"\n}\n".to_string();
    let new_text = "{\n  \"items\": [1, 2],\n  \"name\": \"new\"\n}\n".to_string();
    let unified = concat!(
        "@@ -1,4 +1,4 @@\n",
        " {\n",
        "   \"items\": [1, 2],\n",
        "-  \"name\": \"old\"\n",
        "+  \"name\": \"new\"\n",
        " }\n",
    )
    .to_string();

    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "inline_pair_json",
        path,
        unified,
        old_text,
        new_text,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "inline file diff with prepared syntax on the new side",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_target == Some(target.clone())
                && pane.diff_view == DiffViewMode::Inline
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "file_diff_active={} view={:?} doc={}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
            )
        },
    );

    // Inline row 1 is the context line `  "items": [1, 2],`.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        1,
        DiffTextRegion::Inline,
        11..12,
        "inline diff pair bracket hitbox",
    );
    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("clicking `[` in the inline diff should light its pair");
        assert_eq!(pair.kind, rows::SyntaxPairKind::Bracket);
        assert_eq!(
            pane.diff_text_local_pair_ranges(1, DiffTextRegion::Inline)
                .into_vec(),
            vec![11..12, 16..17]
        );
    });

    let previous_signature = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.file_diff_cache_content_signature
            .expect("the first file-diff generation should be installed")
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_text_occurrences
                    .entry((1, DiffTextRegion::Inline))
                    .or_default()
                    .push(3..8);
            });
        });
    });

    push_regular_diff_content_mode_state_with_rev(
        cx,
        &view,
        repo_id,
        "inline_pair_json",
        PathBuf::from("config.json"),
        2,
        concat!(
            "@@ -1,4 +1,4 @@\n",
            " {\n",
            "   \"items\": (1, 2),\n",
            "-  \"name\": \"old\"\n",
            "+  \"name\": \"newer\"\n",
            " }\n",
        )
        .to_string(),
        "{\n  \"items\": (1, 2),\n  \"name\": \"old\"\n}\n".to_string(),
        "{\n  \"items\": (1, 2),\n  \"name\": \"newer\"\n}\n".to_string(),
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "same-target file-diff highlight generation refresh",
        |pane| {
            pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_content_signature != Some(previous_signature)
        },
        |pane| {
            format!(
                "rev={} inflight={:?} signature={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_content_signature,
            )
        },
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_pair_match_for_tests().is_none(),
            "an accepted source generation must discard its predecessor's pair spans"
        );
        assert!(
            pane.diff_text_occurrences_for_tests().is_empty(),
            "an accepted source generation must discard its predecessor's occurrences"
        );
    });
}

/// Collapsed mode renders the same file-diff rows as Full, so a click in it must
/// pair too. Gating on `is_file_diff_view_active()` (which demands
/// `DiffContentMode::Full`) silently excluded this whole mode.
#[gpui::test]
fn collapsed_file_diff_click_lights_the_matching_json_braces(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(933);
    let path = PathBuf::from("config.json");
    let mut old_lines: Vec<String> = (0..30).map(|i| format!("  \"pad{i}\": {i},")).collect();
    let mut new_lines = old_lines.clone();
    old_lines.insert(0, "{".to_string());
    new_lines.insert(0, "{".to_string());
    old_lines.push("  \"items\": [1, 2],".to_string());
    new_lines.push("  \"items\": [1, 2],".to_string());
    old_lines.push("  \"name\": \"old\"".to_string());
    new_lines.push("  \"name\": \"new\"".to_string());
    old_lines.push("}".to_string());
    new_lines.push("}".to_string());
    let old_text = format!("{}\n", old_lines.join("\n"));
    let new_text = format!("{}\n", new_lines.join("\n"));
    let unified = "@@ -31,3 +31,3 @@\n   \"items\": [1, 2],\n-  \"name\": \"old\"\n+  \"name\": \"new\"\n }\n".to_string();

    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_pair_json",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed pair fixture builds its file diff first",
        |pane| {
            pane.is_file_diff_view_active() && pane.file_diff_cache_target == Some(target.clone())
        },
        |pane| format!("file_diff_active={}", pane.is_file_diff_view_active()),
    );

    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed projection active with prepared syntax",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && !pane.is_file_diff_view_active()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "collapsed={} file_diff_active={} doc={}",
                pane.is_collapsed_diff_projection_active(),
                pane.is_file_diff_view_active(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
            )
        },
    );

    // Find the visible row showing the `"items"` context line and click its `[`.
    let (row_ix, col) = cx
        .update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (0..pane.diff_visible_len()).find_map(|ix| {
                let text = pane.diff_text_line_for_region(ix, DiffTextRegion::SplitRight);
                text.find('[').map(|col| (ix, col))
            })
        })
        .expect("a visible row should show the `items` line");

    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        row_ix,
        DiffTextRegion::SplitRight,
        col..col + 1,
        "collapsed diff pair bracket hitbox",
    );
    simulate_counted_click(cx, click, 1);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("collapsed mode must pair too -- it renders the same file-diff rows");
        assert_eq!(pair.kind, rows::SyntaxPairKind::Bracket);
        assert_eq!(
            pane.diff_text_local_pair_ranges(row_ix, DiffTextRegion::SplitRight)
                .into_vec(),
            vec![col..col + 1, col + 5..col + 6]
        );
    });
}

#[gpui::test]
fn inline_hunk_header_text_right_click_opens_hunk_menu(cx: &mut gpui::TestAppContext) {
    assert_hunk_header_text_right_click_opens_hunk_menu(cx, DiffViewMode::Inline);
}

#[gpui::test]
fn split_hunk_header_text_right_click_opens_hunk_menu(cx: &mut gpui::TestAppContext) {
    assert_hunk_header_text_right_click_opens_hunk_menu(cx, DiffViewMode::Split);
}

fn assert_hunk_header_text_right_click_opens_hunk_menu(
    cx: &mut gpui::TestAppContext,
    diff_view: DiffViewMode,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(70721);
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    activate_collapsed_diff_fixture(
        cx,
        &view,
        repo_id,
        &format!("header_context_menu_{diff_view:?}"),
        diff_view,
        unified,
        old_text,
        new_text,
    );
    let (visible_ix, src_ix) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = pane.collapsed_diff_hunk_visible_indices[0];
        (visible_ix, pane.collapsed_diff_hunks[0].src_ix)
    });
    let regions: &[DiffTextRegion] = match diff_view {
        DiffViewMode::Inline => &[DiffTextRegion::Inline],
        DiffViewMode::Split => &[DiffTextRegion::SplitLeft, DiffTextRegion::SplitRight],
    };
    for &region in regions {
        let click = wait_for_diff_text_click_position_for_offset_range(
            cx,
            &view,
            visible_ix,
            region,
            0..1,
            "hunk header text right-click target",
        );
        cx.simulate_mouse_down(click, MouseButton::Right, Modifiers::default());
        cx.update(|_window, app| {
            assert!(
                !view.read(app).popover_host.read(app).is_open(),
                "menu waits for release"
            );
        });
        cx.simulate_mouse_up(click, MouseButton::Right, Modifiers::default());
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            assert!(
                matches!(
                    view.read(app).popover_host.read(app).popover_kind_for_tests(),
                    Some(PopoverKind::DiffHunkMenu { repo_id: actual_repo, src_ix: actual_src })
                        if actual_repo == repo_id && actual_src == src_ix
                ),
                "right-clicking {region:?} header text must open the parent hunk menu"
            );
        });
        cx.simulate_keystrokes("escape");
        draw_and_drain_test_window(cx);

        let text_click = wait_for_diff_text_click_position_for_offset_range(
            cx,
            &view,
            visible_ix + 1,
            region,
            0..1,
            "ordinary diff text right-click target",
        );
        cx.simulate_mouse_down(text_click, MouseButton::Right, Modifiers::default());
        cx.simulate_mouse_up(text_click, MouseButton::Right, Modifiers::default());
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            assert!(
                matches!(
                    view.read(app)
                        .popover_host
                        .read(app)
                        .popover_kind_for_tests(),
                    Some(PopoverKind::DiffEditorMenu { .. })
                ),
                "ordinary {region:?} text must retain its editor menu"
            );
        });
        cx.simulate_keystrokes("escape");
        draw_and_drain_test_window(cx);
    }
}

#[gpui::test]
fn collapsed_diff_hunk_header_click_does_not_create_row_selection(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(189);
    let path = PathBuf::from("src/lib.rs");
    let (unified, old_text, new_text) = build_collapsed_diff_fixture_texts();
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        repo_id,
        "collapsed_header_click",
        path,
        unified,
        old_text,
        new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "collapsed diff fixture activates full file diff first",
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
        "collapsed diff projection becomes active",
        |pane| {
            pane.is_collapsed_diff_projection_active()
                && !pane.collapsed_diff_hunk_visible_indices.is_empty()
        },
        |pane| {
            format!(
                "collapsed_active={} visible_len={} hunk_rows={:?}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_visible_len(),
                pane.collapsed_diff_hunk_visible_indices,
            )
        },
    );

    let hunk_visible_ix = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .collapsed_diff_hunk_visible_indices[0]
    });

    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        hunk_visible_ix,
        DiffTextRegion::SplitLeft,
        0..1,
        "collapsed hunk header click target",
    );
    simulate_counted_click(cx, click, 1);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor, None,
            "collapsed hunk header click should not create a row selection anchor"
        );
        assert_eq!(
            pane.diff_selection_range, None,
            "collapsed hunk header click should not create a row selection range"
        );
    });
}

fn assert_collapsed_diff_reveal_click_preserves_horizontal_scroll(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    diff_view: DiffViewMode,
) {
    let (unified, old_text, new_text) = build_collapsed_diff_horizontal_scroll_fixture_texts();
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
        "collapsed diff horizontal overflow becomes available before reveal",
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

    let (hunk_src_ix, hidden_up_before) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hunk = pane
            .collapsed_diff_hunks
            .first()
            .copied()
            .expect("expected collapsed diff fixture to expose one hunk");
        let hidden_up = pane.collapsed_diff_hidden_up_rows(hunk.src_ix);
        assert!(
            hidden_up > 0,
            "fixture should expose hidden rows above the collapsed hunk"
        );
        (hunk.src_ix, hidden_up)
    });

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

    let reveal_selector = match diff_view {
        DiffViewMode::Inline => "collapsed_diff_inline_hunk_up",
        DiffViewMode::Split => "collapsed_diff_split_left_hunk_up",
    };
    let reveal_click = debug_selector_center(cx, reveal_selector);
    simulate_counted_click(cx, reveal_click, 1);
    draw_and_drain_test_window(cx);

    let (hidden_up_after, left_after_x, right_after_x) = cx.update(|_window, app| {
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
        (
            pane.collapsed_diff_hidden_up_rows(hunk_src_ix),
            left_x,
            right_x,
        )
    });

    assert!(
        hidden_up_after < hidden_up_before,
        "clicking the collapsed reveal button should expand hidden context"
    );
    assert!(
        (left_after_x - left_before_x).abs() < 0.01,
        "collapsed reveal should preserve left/inline horizontal scroll (before={left_before_x}, after={left_after_x})"
    );
    if diff_view == DiffViewMode::Split {
        assert!(
            (right_after_x - right_before_x).abs() < 0.01,
            "collapsed reveal should preserve split-right horizontal scroll (before={right_before_x}, after={right_after_x})"
        );
    }
}

#[gpui::test]
fn collapsed_diff_inline_reveal_click_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_reveal_click_preserves_horizontal_scroll(
        cx,
        &view,
        gitcomet_state::model::RepoId(264),
        "collapsed_inline_reveal_preserves_hscroll",
        DiffViewMode::Inline,
    );
}

#[gpui::test]
fn collapsed_diff_split_reveal_click_preserves_horizontal_scroll(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    assert_collapsed_diff_reveal_click_preserves_horizontal_scroll(
        cx,
        &view,
        gitcomet_state::model::RepoId(265),
        "collapsed_split_reveal_preserves_hscroll",
        DiffViewMode::Split,
    );
}
