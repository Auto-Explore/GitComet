use super::*;

#[gpui::test]
fn file_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let stable_left_line = "    x += 1;";
    let stable_right_line = "    x += 1;";
    let old_text = "fn smoke_test_fixture() {\n    let mut x = 1;\n    x += 1;\n}\n".repeat(64);
    let new_text = format!("{old_text}\n// file-diff-cache-rev-stability\n");

    let set_state = |cx: &mut gpui::VisualTestContext, diff_file_rev: u64| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_file_rev = diff_file_rev;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText::new(
                        path.clone(),
                        Some(old_text.clone()),
                        Some(new_text.clone()),
                    ),
                )));

                let next_state = app_state_with_repo(repo, repo_id);

                push_test_state(this, Arc::clone(&next_state), cx);
            });
        });
    };

    set_state(cx, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "initial file-diff cache build for rev-stability check",
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && left_doc.is_some()
                && right_doc.is_some()
                && left_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && right_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && pane.syntax_chunk_poll_task.is_none()
        },
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            format!(
                "seq={} inflight={:?} repo_id={:?} rev={} target={:?} path={:?} inline_rows={} left_doc={:?} right_doc={:?} left_pending={:?} right_pending={:?} chunk_poll={} active_diff_rev={:?} active_target={:?} file_diff_active={}",
                pane.file_diff_cache_seq,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target,
                pane.file_diff_cache_path,
                pane.file_diff_inline_cache.len(),
                left_doc,
                right_doc,
                left_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                pane.syntax_chunk_poll_task.is_some(),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.is_file_diff_view_active(),
            )
        },
    );

    let baseline_seq =
        cx.update(|_window, app| view.read(app).diff_renderer.read(app).file_diff_cache_seq);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (left_epoch_before, right_epoch_before, left_hash_before, right_hash_before) =
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, _cx| {
                    let left_row_ix =
                        file_diff_split_row_ix(pane, DiffTextRegion::SplitLeft, stable_left_line)
                            .expect(
                                "expected left split row to exist before seeding the row cache",
                            );
                    let right_row_ix =
                        file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, stable_right_line)
                            .expect(
                                "expected right split row to exist before seeding the row cache",
                            );
                    let left_key = pane
                        .file_diff_split_cache_key(left_row_ix, DiffTextRegion::SplitLeft)
                        .expect("left split row should produce a cache key");
                    let right_key = pane
                        .file_diff_split_cache_key(right_row_ix, DiffTextRegion::SplitRight)
                        .expect("right split row should produce a cache key");
                    let left_epoch =
                        pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
                    let right_epoch =
                        pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
                    let make_seeded =
                        |text: &str, hue: f32, hash: u64| super::CachedDiffStyledText {
                            text: text.to_string().into(),
                            highlights: Arc::from(vec![(
                                0..text.len().min(4),
                                gpui::HighlightStyle {
                                    color: Some(gpui::hsla(hue, 1.0, 0.5, 1.0)),
                                    ..gpui::HighlightStyle::default()
                                },
                            )]),
                            highlights_hash: hash,
                            text_hash: hash.wrapping_mul(31),
                        };
                    pane.diff_text_segments_cache_set(
                        left_key,
                        left_epoch,
                        make_seeded(stable_left_line, 0.0, 0xA11CE),
                    );
                    pane.diff_text_segments_cache_set(
                        right_key,
                        right_epoch,
                        make_seeded(stable_right_line, 0.6, 0xBEEF),
                    );

                    let left_cached = file_diff_split_cached_styled(
                        pane,
                        DiffTextRegion::SplitLeft,
                        stable_left_line,
                    )
                    .expect("seeded left split row should be immediately readable");
                    let right_cached = file_diff_split_cached_styled(
                        pane,
                        DiffTextRegion::SplitRight,
                        stable_right_line,
                    )
                    .expect("seeded right split row should be immediately readable");
                    (
                        left_epoch,
                        right_epoch,
                        left_cached.highlights_hash,
                        right_cached.highlights_hash,
                    )
                })
            })
        });

    for rev in 2..=6 {
        set_state(cx, rev);
        wait_for_main_pane_condition(
            cx,
            &view,
            "identical file-diff payload refresh to settle",
            |pane| {
                let left_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
                let right_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
                pane.file_diff_cache_rev == rev
                    && pane.file_diff_cache_inflight.is_none()
                    && left_doc.is_some()
                    && right_doc.is_some()
                    && left_doc.is_some_and(|document| {
                        !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                    })
                    && right_doc.is_some_and(|document| {
                        !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                    })
                    && pane.syntax_chunk_poll_task.is_none()
            },
            |pane| {
                let left_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
                let right_doc =
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
                (
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_inflight,
                    pane.file_diff_cache_rev,
                    left_doc,
                    right_doc,
                    left_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                    right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                    pane.syntax_chunk_poll_task.is_some(),
                )
            },
        );

        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.file_diff_cache_seq, baseline_seq,
                "identical diff payload should not trigger file-diff rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_diff_cache_inflight.is_none(),
                "file-diff cache should remain built with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_diff_cache_rev, rev,
                "identical payload refresh should still advance the active file-diff rev marker"
            );
            assert_eq!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                left_epoch_before,
                "identical payload refresh should preserve the left split style epoch"
            );
            assert_eq!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
                right_epoch_before,
                "identical payload refresh should preserve the right split style epoch"
            );
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some(),
                "identical payload refresh should keep the left prepared syntax document reachable"
            );
            assert!(
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
                "identical payload refresh should keep the right prepared syntax document reachable"
            );
            let left_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, stable_left_line)
                    .expect("identical payload refresh should preserve the cached left split row");
            let right_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, stable_right_line)
                    .expect("identical payload refresh should preserve the cached right split row");
            assert_eq!(
                left_cached.highlights_hash, left_hash_before,
                "identical payload refresh should keep the cached left split styling intact"
            );
            assert_eq!(
                right_cached.highlights_hash, right_hash_before,
                "identical payload refresh should keep the cached right split styling intact"
            );
        });
    }
}

#[gpui::test]
fn file_diff_cache_rebuilds_when_patch_arrives_after_same_file_refresh(
    cx: &mut gpui::TestAppContext,
) {
    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_selection_anchor = None;
                    pane.diff_selection_range = None;
                    pane.diff_autoscroll_pending = false;
                    pane.clear_diff_text_selection();
                    pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                    cx.notify();
                });
            });
        });
        cx.run_until_parked();

        cx.update(|window, app| {
            rows::clear_diff_paint_log_for_tests();
            let _ = window.draw(app);
            rows::diff_paint_log_for_tests()
                .into_iter()
                .find(|record| record.visible_ix == visible_ix && record.region == region)
                .unwrap_or_else(|| {
                    panic!("expected paint record for visible_ix={visible_ix} region={region:?}")
                })
        })
    }

    fn split_visible_ix_by_old_line(pane: &DiffRenderer, old_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.old_line == Some(old_line))
        })
    }

    fn split_visible_ix_by_new_line(pane: &DiffRenderer, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_line_kind(
        pane: &DiffRenderer,
        old_line: Option<u32>,
        new_line: Option<u32>,
        kind: gitcomet_core::domain::DiffLineKind,
    ) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix).is_some_and(|line| {
                line.kind == kind && line.old_line == old_line && line.new_line == new_line
            })
        })
    }

    fn wait_for_file_diff_seq_after(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_path: &std::path::Path,
        expected_rev: u64,
        previous_seq: u64,
    ) {
        wait_for_main_pane_condition(
            cx,
            view,
            label,
            |pane| {
                pane.file_diff_cache_rev == expected_rev
                    && pane.file_diff_cache_seq > previous_seq
                    && pane.file_diff_cache_inflight.is_none()
                    && pane.file_diff_cache_path.as_deref() == Some(expected_path)
                    && pane.is_file_diff_view_active()
            },
            |pane| {
                format!(
                    "seq={} previous_seq={} inflight={:?} cache_rev={} path={:?} active={} content_signature={:?}",
                    pane.file_diff_cache_seq,
                    previous_seq,
                    pane.file_diff_cache_inflight,
                    pane.file_diff_cache_rev,
                    pane.file_diff_cache_path,
                    pane.is_file_diff_view_active(),
                    pane.file_diff_cache_content_signature,
                )
            },
        );
    }

    fn assert_file_diff_backgrounds(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
    ) {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_view = DiffViewMode::Split;
                    pane.clear_diff_text_style_caches();
                    pane.ensure_diff_visible_indices();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let (removed_ix, modified_ix, added_ix) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            (
                split_visible_ix_by_old_line(pane, 2)
                    .expect("expected split visible row for removed old line 2"),
                split_visible_ix_by_new_line(pane, 2)
                    .expect("expected split visible row for modified new line 2"),
                split_visible_ix_by_new_line(pane, 4)
                    .expect("expected split visible row for added new line 4"),
            )
        });
        assert!(
            draw_paint_record_for_visible_ix(cx, view, removed_ix, DiffTextRegion::SplitLeft)
                .row_bg
                .is_some(),
            "{label} should paint split-left removal background after refresh",
        );
        assert!(
            draw_paint_record_for_visible_ix(cx, view, modified_ix, DiffTextRegion::SplitRight)
                .row_bg
                .is_some(),
            "{label} should paint split-right modification background after refresh",
        );
        assert!(
            draw_paint_record_for_visible_ix(cx, view, added_ix, DiffTextRegion::SplitRight)
                .row_bg
                .is_some(),
            "{label} should paint split-right addition background after refresh",
        );

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.diff_renderer.update(cx, |pane, cx| {
                    pane.diff_view = DiffViewMode::Inline;
                    pane.clear_diff_text_style_caches();
                    pane.ensure_diff_visible_indices();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let (removed_inline_ix, added_inline_ix) = cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            (
                inline_visible_ix_by_line_kind(
                    pane,
                    Some(2),
                    None,
                    gitcomet_core::domain::DiffLineKind::Remove,
                )
                .expect("expected inline remove row for old line 2"),
                inline_visible_ix_by_line_kind(
                    pane,
                    None,
                    Some(4),
                    gitcomet_core::domain::DiffLineKind::Add,
                )
                .expect("expected inline add row for new line 4"),
            )
        });
        assert!(
            draw_paint_record_for_visible_ix(cx, view, removed_inline_ix, DiffTextRegion::Inline)
                .row_bg
                .is_some(),
            "{label} should paint inline removal background after refresh",
        );
        assert!(
            draw_paint_record_for_visible_ix(cx, view, added_inline_ix, DiffTextRegion::Inline)
                .row_bg
                .is_some(),
            "{label} should paint inline addition background after refresh",
        );
    }

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(291);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_same_file_patch_ready_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/refresh_highlights.rs");
    let target = DiffTarget::working_tree(path.clone(), gitcomet_core::domain::DiffArea::Unstaged);
    let old_text = "fn main() {\n    let value = 1;\n    let stable = 10;\n}\n";
    let new_text = "fn main() {\n    let value = 2;\n    let stable = 10;\n    let added = value + stable;\n}\n";
    let unified = "\
diff --git a/src/refresh_highlights.rs b/src/refresh_highlights.rs
index 1111111..2222222 100644
--- a/src/refresh_highlights.rs
+++ b/src/refresh_highlights.rs
@@ -1,4 +1,5 @@
 fn main() {
-    let value = 1;
+    let value = 2;
     let stable = 10;
+    let added = value + stable;
 }
";
    let patch_diff = Arc::new(gitcomet_core::domain::Diff::from_unified(
        target.clone(),
        unified,
    ));
    let file_diff = Arc::new(gitcomet_core::domain::FileDiffText::new(
        path.clone(),
        Some(old_text.to_string()),
        Some(new_text.to_string()),
    ));
    let expected_path = workdir.join(&path);

    let push_state = |cx: &mut gpui::VisualTestContext,
                      diff_rev: u64,
                      diff_file_rev: u64,
                      patch_ready: bool,
                      file_ready: bool| {
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
                repo.diff_state.diff_rev = diff_rev;
                repo.diff_state.diff = if patch_ready {
                    gitcomet_state::model::Loadable::Ready(Arc::clone(&patch_diff))
                } else {
                    gitcomet_state::model::Loadable::Loading
                };
                repo.diff_state.diff_file_rev = diff_file_rev;
                repo.diff_state.diff_file = if file_ready {
                    gitcomet_state::model::Loadable::Ready(Some(Arc::clone(&file_diff)))
                } else {
                    gitcomet_state::model::Loadable::Loading
                };

                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
    };

    push_state(cx, 1, 1, true, true);
    wait_for_file_diff_seq_after(
        cx,
        &view,
        "initial patch-backed file-diff cache build",
        expected_path.as_path(),
        1,
        0,
    );
    assert_file_diff_backgrounds(cx, &view, "initial patch-backed render");

    for (cycle_ix, (previous_patch_rev, next_file_rev, next_patch_rev)) in
        [(1, 2, 2), (2, 3, 3)].into_iter().enumerate()
    {
        let seq_before_refresh =
            cx.update(|_window, app| view.read(app).diff_renderer.read(app).file_diff_cache_seq);

        push_state(cx, previous_patch_rev, next_file_rev - 1, false, false);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.file_diff_cache_seq, seq_before_refresh,
                "cycle {cycle_ix}: same-target loading should keep the existing cache alive"
            );
        });

        push_state(cx, previous_patch_rev, next_file_rev, false, true);
        wait_for_file_diff_seq_after(
            cx,
            &view,
            "file-ready same-target refresh builds temporary file-only cache",
            expected_path.as_path(),
            next_file_rev,
            seq_before_refresh,
        );
        let file_only_seq =
            cx.update(|_window, app| view.read(app).diff_renderer.read(app).file_diff_cache_seq);

        push_state(cx, next_patch_rev, next_file_rev, true, true);
        wait_for_file_diff_seq_after(
            cx,
            &view,
            "patch-ready same-target refresh rebuilds patch-backed cache",
            expected_path.as_path(),
            next_file_rev,
            file_only_seq,
        );
        assert_file_diff_backgrounds(cx, &view, &format!("cycle {cycle_ix} patch-backed render"));
    }
}

/// The command palette and the Settings window route mode changes through
/// `GitCometView::set_diff_view_mode` rather than the toolbar buttons, so the
/// styled-segment cache clear has to live in the pane setter: inline keys those
/// segments by `row_ix` while split keys them by `row_ix * 2` / `row_ix * 2 + 1`
/// against the same epochs, so a stale entry can paint the wrong row.
#[gpui::test]
fn toggle_diff_view_command_clears_styled_segment_caches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(284);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "toggle_diff_view_cache",
        DiffViewMode::Inline,
    );

    // Seed the inline key space directly: which rows a draw happens to cache
    // depends on syntax availability and streaming heuristics, and the contract
    // under test is only that a mode change drops whatever is cached.
    let cached = cx.update(|_window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        diff_renderer.update(app, |pane, _cx| {
            for key in 0..3 {
                pane.diff_text_segments_cache_set(
                    key,
                    0,
                    crate::view::diff_text_model::CachedDiffStyledText {
                        text: "let x = 1;".into(),
                        highlights: Arc::from(Vec::new()),
                        highlights_hash: 0,
                        text_hash: 0,
                    },
                );
            }
            pane.diff_text_pair_match = Some(DiffTextPairMatch {
                kind: rows::SyntaxPairKind::Bracket,
                spans: Vec::new(),
            });
            pane.diff_text_occurrences
                .entry((0, DiffTextRegion::Inline))
                .or_default()
                .push(0..3);
            pane.diff_text_segments_cache.iter().flatten().count()
        })
    });
    assert_eq!(cached, 3);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.execute_command("toggle-diff-view", None, cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.diff_renderer.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Split);
        assert_eq!(
            pane.diff_text_segments_cache.iter().flatten().count(),
            0,
            "switching modes outside the toolbar must still clear the aliasing cache"
        );
        assert!(pane.diff_text_pair_match_for_tests().is_none());
        assert!(pane.diff_text_occurrences_for_tests().is_empty());
    });
}
