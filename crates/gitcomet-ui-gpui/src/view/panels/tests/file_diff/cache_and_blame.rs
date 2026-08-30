use super::*;

#[gpui::test]
fn file_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
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
                this.main_pane.update(cx, |pane, _cx| {
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
            let pane = view.read(app).main_pane.read(app);
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
                this.main_pane.update(cx, |pane, cx| {
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

    fn split_visible_ix_by_old_line(pane: &MainPaneView, old_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.old_line == Some(old_line))
        })
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_line_kind(
        pane: &MainPaneView,
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
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_view = DiffViewMode::Split;
                    pane.clear_diff_text_style_caches();
                    pane.ensure_diff_visible_indices();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let (removed_ix, modified_ix, added_ix) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
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
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_view = DiffViewMode::Inline;
                    pane.clear_diff_text_style_caches();
                    pane.ensure_diff_visible_indices();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        let (removed_inline_ix, added_inline_ix) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(291);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_same_file_patch_ready_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/refresh_highlights.rs");
    let target = DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
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
            cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);

        push_state(cx, previous_patch_rev, next_file_rev - 1, false, false);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
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
            cx.update(|_window, app| view.read(app).main_pane.read(app).file_diff_cache_seq);

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

#[gpui::test]
fn file_image_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(147);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_image_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/gitcomet.png");
    let image_bytes =
        include_bytes!("../../../../../../../assets/linux/hicolor/32x32/apps/gitcomet.png")
            .to_vec();

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(image_bytes.as_slice()),
        Some(image_bytes.as_slice()),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial image diff cache build", |_| true);

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_image_diff_cache_seq);

    for rev in 2..=6 {
        seed_file_image_diff_state_with_rev(
            cx,
            &view,
            repo_id,
            &workdir,
            &path,
            rev,
            Some(image_bytes.as_slice()),
            Some(image_bytes.as_slice()),
        );
        draw_and_drain_test_window(cx);

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_image_diff_cache_seq, baseline_seq,
                "identical image diff payload should not trigger cache rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_image_diff_cache_inflight.is_none(),
                "image diff cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_image_diff_cache_rev, rev,
                "identical payload refresh should still advance the image diff cache rev marker"
            );
            assert!(
                pane.is_file_image_diff_view_active(),
                "image diff preview should remain active across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn file_image_diff_cache_keeps_valid_svg_on_render_fast_path_across_rev_refreshes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(148);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");
    let svg_bytes = image_diff_svg_fixture(4096, 2048, "#00aaff");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(svg_bytes.as_slice()),
        Some(svg_bytes.as_slice()),
    );
    wait_for_file_image_diff_cache(cx, &view, "initial svg image diff cache build", |pane| {
        pane.file_image_diff_cache_old.is_some()
            && pane.file_image_diff_cache_new.is_some()
            && pane.file_image_diff_cache_old_svg_path.is_none()
            && pane.file_image_diff_cache_new_svg_path.is_none()
    });

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_image_diff_cache_seq);

    for rev in 2..=6 {
        seed_file_image_diff_state_with_rev(
            cx,
            &view,
            repo_id,
            &workdir,
            &path,
            rev,
            Some(svg_bytes.as_slice()),
            Some(svg_bytes.as_slice()),
        );
        draw_and_drain_test_window(cx);

        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                pane.file_image_diff_cache_seq, baseline_seq,
                "identical svg image diff payload should not trigger cache rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_image_diff_cache_inflight.is_none(),
                "svg image diff cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_image_diff_cache_rev, rev,
                "identical svg payload refresh should still advance the image diff cache rev marker"
            );
            assert!(
                pane.file_image_diff_cache_old.is_some() && pane.file_image_diff_cache_new.is_some(),
                "valid svg payload should stay on the rasterized render-image path"
            );
            assert!(
                pane.file_image_diff_cache_old_svg_path.is_none()
                    && pane.file_image_diff_cache_new_svg_path.is_none(),
                "valid svg payload should not fall back to cached svg file paths"
            );
            assert!(
                pane.is_file_image_diff_view_active(),
                "svg image diff preview should remain active across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn file_image_diff_cache_keeps_distinct_valid_svg_sides_on_render_fast_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(149);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_distinct",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");
    let old_svg = image_diff_svg_fixture(4096, 2048, "#00aaff");
    let new_svg = image_diff_svg_fixture(2048, 4096, "#ffaa00");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(old_svg.as_slice()),
        Some(new_svg.as_slice()),
    );
    wait_for_file_image_diff_cache(
        cx,
        &view,
        "distinct svg image diff render cache build",
        |pane| {
            pane.file_image_diff_cache_old.is_some()
                && pane.file_image_diff_cache_new.is_some()
                && pane.file_image_diff_cache_old_svg_path.is_none()
                && pane.file_image_diff_cache_new_svg_path.is_none()
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let old = pane
            .file_image_diff_cache_old
            .as_ref()
            .expect("old render image");
        let new = pane
            .file_image_diff_cache_new
            .as_ref()
            .expect("new render image");
        assert_eq!(old.size(0).width.0, 1024);
        assert_eq!(old.size(0).height.0, 512);
        assert_eq!(new.size(0).width.0, 512);
        assert_eq!(new.size(0).height.0, 1024);
    });
}

#[gpui::test]
fn file_image_diff_cache_falls_back_to_cached_svg_paths_for_invalid_svg_payloads(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(150);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_image_diff_invalid",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("assets/diagram.svg");

    seed_file_image_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        Some(&b"<not-valid-svg-old>"[..]),
        Some(&b"<not-valid-svg-new>"[..]),
    );
    wait_for_file_image_diff_cache(
        cx,
        &view,
        "invalid svg image diff fallback cache build",
        |pane| {
            pane.file_image_diff_cache_old.is_none()
                && pane.file_image_diff_cache_new.is_none()
                && pane.file_image_diff_cache_old_svg_path.is_some()
                && pane.file_image_diff_cache_new_svg_path.is_some()
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_image_diff_cache_old_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
        assert!(
            pane.file_image_diff_cache_new_svg_path
                .as_ref()
                .is_some_and(|path| path.exists())
        );
    });
}

/// An untracked SVG is preview-only, so no patch is loaded for it — but an SVG
/// never reaches the text-file preview path, so the diff pane's Code view is
/// the only place its source is ever shown. It has to render the file text in
/// either diff mode, and the Image/Code toggle has to stay reachable.
#[gpui::test]
fn untracked_svg_keeps_the_code_view_and_toggle_in_collapsed_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(151);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_untracked_svg_code_view",
        std::process::id()
    ));
    let _ = std::fs::create_dir_all(&workdir);
    let path = PathBuf::from("assets/diagram.svg");
    let source = String::from_utf8(image_diff_svg_fixture(64, 64, "#22cc66"))
        .expect("svg fixture should be utf-8");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_state_rev = 1;
            // Preview-only: the state layer loads no patch for an untracked file.
            repo.diff_state.diff = gitcomet_state::model::Loadable::NotLoaded;
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(path.clone(), None, Some(source.clone())),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);

            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_content_mode = DiffContentMode::Collapsed;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Svg, RenderedPreviewMode::Source);
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.is_file_preview_active(),
            "an SVG is classified as an image, so it must not take the text-file preview path"
        );
        // Nothing to collapse without a patch, so the pane falls back to Full.
        assert_eq!(pane.effective_diff_content_mode(), DiffContentMode::Full);
        assert!(pane.wants_file_diff_view(false));
        assert!(!pane.is_collapsed_diff_projection_active());
        assert_eq!(
            crate::view::main_diff_rendered_preview_toggle_kind(
                pane.wants_file_diff_view(false),
                pane.wants_collapsed_diff_view(false),
                false,
                crate::view::diff_target_rendered_preview_kind(Some(&target)),
            ),
            Some(RenderedPreviewKind::Svg),
            "the Image/Code toggle must stay available while Collapsed is selected"
        );
        assert!(
            pane.file_diff_inline_row_len() > 0,
            "the Code view should have the SVG source to render"
        );
    });
}

#[gpui::test]
fn file_diff_view_renders_split_and_inline_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_syntax_view",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/file_diff_projection.rs");
    let removed_line = "struct Removed {}";
    let added_line = "fn added() { let value = 2; }";
    let removed_inline_text = format!("-{removed_line}");
    let added_inline_text = format!("+{added_line}");
    let old_text = format!("const KEEP: i32 = 1;\n{removed_line}\nconst AFTER: i32 = 2;\n");
    let new_text = format!("const KEEP: i32 = 1;\nconst AFTER: i32 = 2;\n{added_line}\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} repo_id={:?} cache_rev={} cache_target={:?} cache_path={:?} file_diff_active={} active_repo={:?} active_diff_file_rev={:?} active_diff_target={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target.clone(),
                pane.file_diff_cache_path.clone(),
                pane.is_file_diff_view_active(),
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff split syntax render",
        |pane| {
            let Some(remove_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, removed_line)
            else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_line
                && add_styled.text.as_ref() == added_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 0..6)
                && highlights_include_range(add_styled.highlights.as_ref(), 0..2)
        },
        |pane| {
            let remove_row_ix =
                file_diff_split_row_ix(pane, DiffTextRegion::SplitLeft, removed_line);
            let add_row_ix = file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, added_line);
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_line);
            format!(
                "file_diff_active={} diff_view={:?} visible_len={} cache_path={:?} cache_repo_id={:?} cache_rev={} cache_target={:?} active_repo={:?} active_diff_file_rev={:?} active_diff_target={:?} remove_row_ix={remove_row_ix:?} add_row_ix={add_row_ix:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.diff_visible_len(),
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target.clone(),
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file-diff inline syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_line
                && add_styled.text.as_ref() == added_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 0..6)
                && highlights_include_range(add_styled.highlights.as_ref(), 0..2)
        },
        |pane| {
            let remove_inline_ix = file_diff_inline_ix(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_inline_ix = file_diff_inline_ix(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            format!(
                "file_diff_active={} diff_view={:?} visible_len={} remove_inline_ix={remove_inline_ix:?} add_inline_ix={add_inline_ix:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.diff_visible_len(),
            )
        },
    );
}

#[gpui::test]
fn html_file_diff_renders_injected_attribute_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(77);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_diff_html_attribute_injections",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/file_diff_attribute_injections.html");
    let removed_onclick_line = r#"<button onclick="const value = 1;">go</button>"#;
    let added_onclick_line = r#"<button onclick="const value = 2;">go</button>"#;
    let added_style_line = r#"<div style="color: red; display: block">ok</div>"#;
    let removed_inline_text = format!("-{removed_onclick_line}");
    let added_inline_text = format!("+{added_onclick_line}");
    let style_inline_text = format!("+{added_style_line}");
    let old_text = format!("<p>keep</p>\n{removed_onclick_line}\n<p>after</p>\n");
    let new_text = format!("<p>keep</p>\n<p>after</p>\n{added_onclick_line}\n{added_style_line}\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_onclick_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_onclick_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_style_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == style_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff split attribute injection syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_split_cached_styled(
                pane,
                DiffTextRegion::SplitLeft,
                removed_onclick_line,
            ) else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_onclick_line)
            else {
                return false;
            };
            let Some(style_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_style_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_onclick_line
                && add_styled.text.as_ref() == added_onclick_line
                && style_styled.text.as_ref() == added_style_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(add_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(add_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(style_styled.highlights.as_ref(), 12..17)
                && highlights_include_range(style_styled.highlights.as_ref(), 24..31)
        },
        |pane| {
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_onclick_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_onclick_line);
            let style_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_style_line);
            format!(
                "diff_view={:?} remove_cached={remove_cached:?} add_cached={add_cached:?} style_cached={style_cached:?}",
                pane.diff_view,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML file-diff inline attribute injection syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };
            let Some(style_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &style_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_onclick_line
                && add_styled.text.as_ref() == added_onclick_line
                && style_styled.text.as_ref() == added_style_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(add_styled.highlights.as_ref(), 17..22)
                && highlights_include_range(add_styled.highlights.as_ref(), 31..32)
                && highlights_include_range(style_styled.highlights.as_ref(), 12..17)
                && highlights_include_range(style_styled.highlights.as_ref(), 24..31)
        },
        |pane| {
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            let style_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &style_inline_text,
            );
            format!(
                "diff_view={:?} remove_cached={remove_cached:?} add_cached={add_cached:?} style_cached={style_cached:?}",
                pane.diff_view,
            )
        },
    );
}

#[gpui::test]
fn xml_file_diff_renders_syntax_highlights_from_real_documents(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(79);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_xml_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("config/settings.xml");
    let removed_tag_line = r#"<server port="8080">"#;
    let added_tag_line = r#"<server port="9090" mode="prod">"#;
    let comment_line = "<!-- configuration -->";
    let removed_inline_text = format!("-{removed_tag_line}");
    let added_inline_text = format!("+{added_tag_line}");
    let old_text = format!("{comment_line}\n{removed_tag_line}\n  <name>app</name>\n</server>\n");
    let new_text = format!("{comment_line}\n{added_tag_line}\n  <name>app</name>\n</server>\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Xml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(removed_tag_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(added_tag_line))
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Remove
                        && line.text.as_ref() == removed_inline_text
                })
                && pane.file_diff_inline_cache.iter().any(|line| {
                    line.kind == gitcomet_core::domain::DiffLineKind::Add
                        && line.text.as_ref() == added_inline_text
                })
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} language={:?} rows={:?} inline_rows={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows
                    .iter()
                    .map(|row| (row.kind, row.old.clone(), row.new.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| (line.kind, line.text.clone()))
                    .collect::<Vec<_>>(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff split syntax render",
        |pane| {
            let Some(remove_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, removed_tag_line)
            else {
                return false;
            };
            let Some(add_styled) =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, added_tag_line)
            else {
                return false;
            };

            remove_styled.text.as_ref() == removed_tag_line
                && add_styled.text.as_ref() == added_tag_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(add_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 20..24)
        },
        |pane| {
            let remove_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitLeft, removed_tag_line);
            let add_cached =
                file_diff_split_cached_debug(pane, DiffTextRegion::SplitRight, added_tag_line);
            format!(
                "diff_view={:?} language={:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.diff_view, pane.file_diff_cache_language,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML file-diff inline syntax render",
        |pane| {
            let Some(remove_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            ) else {
                return false;
            };
            let Some(add_styled) = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            ) else {
                return false;
            };

            remove_styled.text.as_ref() == removed_tag_line
                && add_styled.text.as_ref() == added_tag_line
                && highlights_include_range(remove_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 1..7)
                && highlights_include_range(add_styled.highlights.as_ref(), 8..12)
                && highlights_include_range(add_styled.highlights.as_ref(), 20..24)
        },
        |pane| {
            let remove_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Remove,
                &removed_inline_text,
            );
            let add_cached = file_diff_inline_cached_debug(
                pane,
                gitcomet_core::domain::DiffLineKind::Add,
                &added_inline_text,
            );
            format!(
                "diff_view={:?} language={:?} remove_cached={remove_cached:?} add_cached={add_cached:?}",
                pane.diff_view, pane.file_diff_cache_language,
            )
        },
    );
}

#[gpui::test]
fn yaml_file_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn force_file_diff_fallback_mode(pane: &mut MainPaneView) {
        pane.file_diff_syntax_generation = pane.file_diff_syntax_generation.wrapping_add(1);
        for view_mode in [
            PreparedSyntaxViewMode::FileDiffSplitLeft,
            PreparedSyntaxViewMode::FileDiffSplitRight,
        ] {
            if let Some(key) = pane.file_diff_prepared_syntax_key(view_mode) {
                pane.prepared_syntax_documents.remove(&key);
            }
        }
        pane.clear_diff_text_style_caches();
    }

    fn quoted_scalar_style(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<(std::ops::Range<usize>, gpui::Hsla)> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start == quote_start
                && range.end == text.len())
            .then_some((range.clone(), color))
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start <= dash_ix
                && range.end >= dash_ix.saturating_add(1))
            .then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn line_debug(
        line: Option<(&str, &super::CachedDiffStyledText)>,
    ) -> Option<(
        String,
        Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    )> {
        let (text, styled) = line?;
        Some((
            text.to_string(),
            styled
                .highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
        ))
    }

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(split_right_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(inline_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn split_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<FileDiffRowKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    split_right_row_by_new_line(pane, line).map(|row| row.kind),
                )
            })
            .collect()
    }

    fn inline_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<DiffLineKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| (line, inline_row_by_new_line(pane, line).map(|row| row.kind)))
            .collect()
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    #[derive(Clone, Copy, Debug)]
    struct ExpectedPaintRow {
        line_no: u32,
        visible_ix: usize,
        expects_add_bg: bool,
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn split_draw_rows_for_lines(pane: &MainPaneView, lines: &[u32]) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = split_visible_ix_by_new_line(pane, line_no)
                    .unwrap_or_else(|| panic!("expected split visible row for line {line_no}"));
                let expects_add_bg = split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add);
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg,
                }
            })
            .collect()
    }

    fn inline_draw_rows_for_lines(pane: &MainPaneView, lines: &[u32]) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = inline_visible_ix_by_new_line(pane, line_no)
                    .unwrap_or_else(|| panic!("expected inline visible row for line {line_no}"));
                let expects_add_bg = inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add);
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg,
                }
            })
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
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

    fn assert_split_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::SplitRight,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let (text, styled) = split_right_cached_styled_by_new_line(pane, expected.line_no)
                    .unwrap_or_else(|| {
                        panic!(
                            "expected cached split-right styled text for line {}",
                            expected.line_no
                        )
                    });
                (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref()),
                )
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    fn assert_inline_rows_match_render_cache(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        expected_rows: Vec<ExpectedPaintRow>,
    ) {
        let mut add_bg = None;
        let mut context_bg = None;

        for expected in expected_rows {
            let record = draw_paint_record_for_visible_ix(
                cx,
                view,
                expected.visible_ix,
                DiffTextRegion::Inline,
            );
            let (text, highlights) = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let (text, styled) = inline_cached_styled_by_new_line(pane, expected.line_no)
                    .unwrap_or_else(|| {
                        panic!(
                            "expected cached inline styled text for line {}",
                            expected.line_no
                        )
                    });
                (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref()),
                )
            });
            assert_eq!(
                record.text.as_ref(),
                text.as_str(),
                "{label} render text mismatch for line {}",
                expected.line_no,
            );
            assert_eq!(
                record.highlights, highlights,
                "{label} render highlights mismatch for line {}",
                expected.line_no,
            );

            if expected.expects_add_bg {
                match add_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} add-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => add_bg = record.row_bg,
                }
            } else {
                match context_bg {
                    Some(bg) => assert_eq!(
                        record.row_bg,
                        Some(bg),
                        "{label} context-row background mismatch for line {}",
                        expected.line_no,
                    ),
                    None => context_bg = record.row_bg,
                }
            }
        }

        if let (Some(add_bg), Some(context_bg)) = (add_bg, context_bg) {
            assert_ne!(
                add_bg, context_bg,
                "{label} should paint add rows with a different background than context rows",
            );
        }
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(80);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let repo_root = fixture_repo_root();
    let git_show = |spec: &str| fixture_git_show(&repo_root, spec, "YAML diff regression fixture");
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_nested_key_line = 4u32;
    let affected_nested_key_lines = [19u32, 34u32];
    let baseline_top_key_line = 3u32;
    let affected_top_key_lines = [36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [19u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 34, 36];
    let render_lines = [
        17u32, 18, 19, 21, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36,
    ];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 0, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff cache build before fallback highlighting checks",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Other YAML tests can warm the shared prepared-syntax cache before this
                // test runs. Clear the local prepared documents and invalidate any in-flight
                // background parse so the next draw deterministically exercises fallback mode.
                force_file_diff_fallback_mode(pane);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff fallback mode forced for highlight checks",
        |pane| {
            pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (baseline_path_text, baseline_path_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_path_line)
                .expect("fallback split draw should cache the baseline YAML path row");
        let baseline_dash_color = list_item_dash_color(baseline_path_styled, baseline_path_text)
            .expect("fallback split draw should syntax-highlight the YAML list dash");
        let (_, baseline_path_color) = quoted_scalar_style(baseline_path_styled, baseline_path_text)
            .expect("fallback split draw should syntax-highlight the YAML quoted path");
        for line_no in affected_path_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML row {line_no}"));
            assert_eq!(
                list_item_dash_color(styled, text),
                Some(baseline_dash_color),
                "fallback split draw should keep YAML list punctuation highlighting on line {line_no}",
            );
            assert_eq!(
                quoted_scalar_style(styled, text).map(|(_, color)| color),
                Some(baseline_path_color),
                "fallback split draw should keep YAML quoted-string highlighting on line {line_no}",
            );
        }

        let (baseline_nested_key_text, baseline_nested_key_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
                .expect("fallback split draw should cache the baseline YAML nested key row");
        let baseline_nested_key_color = mapping_key_color(
            baseline_nested_key_styled,
            baseline_nested_key_text,
        )
        .expect("fallback split draw should syntax-highlight the YAML nested key");
        for line_no in affected_nested_key_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_nested_key_color),
                "fallback split draw should keep YAML key highlighting on line {line_no}",
            );
        }

        let (baseline_top_key_text, baseline_top_key_styled) =
            split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
                .expect("fallback split draw should cache the baseline YAML top-level key row");
        let baseline_top_key_color =
            mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
                .expect("fallback split draw should syntax-highlight the YAML top-level key");
        for line_no in affected_top_key_lines {
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback split draw should cache YAML top-level key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_top_key_color),
                "fallback split draw should keep YAML top-level key highlighting on line {line_no}",
            );
        }
    });

    let fallback_split_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, &render_lines)
    });
    assert_split_rows_match_render_cache(cx, &view, "fallback split", fallback_split_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let (baseline_path_text, baseline_path_styled) =
            inline_cached_styled_by_new_line(pane, baseline_path_line)
                .expect("fallback inline draw should cache the baseline YAML path row");
        let baseline_dash_color = list_item_dash_color(baseline_path_styled, baseline_path_text)
            .expect("fallback inline draw should syntax-highlight the YAML list dash");
        let (_, baseline_path_color) = quoted_scalar_style(baseline_path_styled, baseline_path_text)
            .expect("fallback inline draw should syntax-highlight the YAML quoted path");
        for line_no in affected_path_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML row {line_no}"));
            assert_eq!(
                list_item_dash_color(styled, text),
                Some(baseline_dash_color),
                "fallback inline draw should keep YAML list punctuation highlighting on line {line_no}",
            );
            assert_eq!(
                quoted_scalar_style(styled, text).map(|(_, color)| color),
                Some(baseline_path_color),
                "fallback inline draw should keep YAML quoted-string highlighting on line {line_no}",
            );
        }

        let (baseline_nested_key_text, baseline_nested_key_styled) =
            inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
                .expect("fallback inline draw should cache the baseline YAML nested key row");
        let baseline_nested_key_color = mapping_key_color(
            baseline_nested_key_styled,
            baseline_nested_key_text,
        )
        .expect("fallback inline draw should syntax-highlight the YAML nested key");
        for line_no in affected_nested_key_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_nested_key_color),
                "fallback inline draw should keep YAML key highlighting on line {line_no}",
            );
        }

        let (baseline_top_key_text, baseline_top_key_styled) =
            inline_cached_styled_by_new_line(pane, baseline_top_key_line)
                .expect("fallback inline draw should cache the baseline YAML top-level key row");
        let baseline_top_key_color =
            mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
                .expect("fallback inline draw should syntax-highlight the YAML top-level key");
        for line_no in affected_top_key_lines {
            let (text, styled) = inline_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("fallback inline draw should cache YAML top-level key row {line_no}"));
            assert_eq!(
                mapping_key_color(styled, text),
                Some(baseline_top_key_color),
                "fallback inline draw should keep YAML top-level key highlighting on line {line_no}",
            );
        }
    });

    let fallback_inline_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, &render_lines)
    });
    assert_inline_rows_match_render_cache(cx, &view, "fallback inline", fallback_inline_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &old_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff baseline revision prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 2, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff split syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some((_, baseline_path_color)) =
                quoted_scalar_style(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            if affected_add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_style(styled, text).map(|(_, color)| color)
                        != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} split_kinds={:?} split_debug={:?}",
                pane.diff_view,
                split_kind_debug(pane, &lines),
                split_debug(pane, &lines),
            )
        },
    );

    let prepared_split_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, &render_lines)
    });
    assert_split_rows_match_render_cache(cx, &view, "prepared split", prepared_split_draw_rows);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML file-diff inline syntax stays consistent for repeated paths and keys",
        |pane| {
            let Some((baseline_path_text, baseline_path_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            let Some(baseline_dash_color) =
                list_item_dash_color(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            let Some((_, baseline_path_color)) =
                quoted_scalar_style(baseline_path_styled, baseline_path_text)
            else {
                return false;
            };
            if affected_add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                list_item_dash_color(styled, text) != Some(baseline_dash_color)
                    || quoted_scalar_style(styled, text).map(|(_, color)| color)
                        != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((baseline_nested_key_text, baseline_nested_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_nested_key_line)
            else {
                return false;
            };
            let Some(baseline_nested_key_color) =
                mapping_key_color(baseline_nested_key_styled, baseline_nested_key_text)
            else {
                return false;
            };
            if affected_nested_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_nested_key_color)
            }) {
                return false;
            }

            let Some((baseline_top_key_text, baseline_top_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_top_key_line)
            else {
                return false;
            };
            let Some(baseline_top_key_color) =
                mapping_key_color(baseline_top_key_styled, baseline_top_key_text)
            else {
                return false;
            };
            !affected_top_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_top_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_nested_key_line);
            lines.extend(affected_nested_key_lines);
            lines.push(baseline_top_key_line);
            lines.extend(affected_top_key_lines);
            format!(
                "diff_view={:?} inline_kinds={:?} inline_debug={:?}",
                pane.diff_view,
                inline_kind_debug(pane, &lines),
                inline_debug(pane, &lines),
            )
        },
    );

    let prepared_inline_draw_rows = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, &render_lines)
    });
    assert_inline_rows_match_render_cache(cx, &view, "prepared inline", prepared_inline_draw_rows);
}

#[gpui::test]
fn yaml_file_diff_fallback_matches_prepared_document_for_deployment_ci(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(180);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_fallback_prepared_baseline",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "YAML fallback prepared baseline fixture");
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };

    let old_lines = [3u32, 4];
    let new_lines = [
        3u32, 4, 17, 18, 19, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33, 34, 36,
    ];
    let baseline_old_by_line = old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "deployment-ci YAML rows ready for prepared-baseline comparison",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(36))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(36))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in new_lines {
            let actual = split_right_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected fallback split-right styled text for deployment-ci line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for deployment-ci line {line_no}"));
            assert_eq!(
                actual, expected,
                "fallback split-right YAML highlighting should match prepared baseline for deployment-ci line {line_no}"
            );
        }
    });

    let split_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                split_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected split visible row for deployment-ci line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    for (&line_no, &visible_ix) in new_lines.iter().zip(split_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!(
                        "expected cached split-right styled text for deployment-ci line {line_no}"
                    )
                });
            let kind = split_right_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-right row for deployment-ci line {line_no}")
                })
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "deployment-ci split render text should match cache for line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "deployment-ci split render highlights should match cache for line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify),
            "deployment-ci split render should preserve diff background for line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                inline_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected inline visible row for deployment-ci line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in new_lines {
            let actual = inline_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected fallback inline styled text for deployment-ci line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for deployment-ci line {line_no}"));
            assert_eq!(
                actual, expected,
                "fallback inline YAML highlighting should match prepared baseline for deployment-ci line {line_no}"
            );
        }
    });

    for (&line_no, &visible_ix) in new_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) =
                inline_cached_styled_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected cached inline styled text for deployment-ci line {line_no}")
                });
            let kind = inline_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| panic!("expected inline row for deployment-ci line {line_no}"))
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "deployment-ci inline render text should match cache for line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "deployment-ci inline render highlights should match cache for line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, DiffLineKind::Add | DiffLineKind::Remove),
            "deployment-ci inline render should preserve diff background for line {line_no}"
        );
    }

    assert_eq!(
        baseline_old_by_line.len(),
        old_lines.len(),
        "old-side YAML baselines should be materialized for the deployment-ci fixture"
    );
}

#[gpui::test]
fn yaml_file_diff_keeps_consistent_highlighting_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_left_row_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.old_line == Some(old_line))
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_left_cached_styled_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.old_line == Some(old_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.old.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitLeft)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none() && range.start <= key_start && range.end >= key_end)
                .then_some(color)
        })
    }

    fn scalar_color_after_colon(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let value_start = text.find(':')?.checked_add(1).and_then(|start| {
            text[start..]
                .find(|ch: char| !ch.is_ascii_whitespace())
                .map(|offset| start.saturating_add(offset))
        })?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (style.background_color.is_none()
                && range.start <= value_start
                && range.end > value_start)
                .then_some(color)
        })
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn expected_yaml_snapshot(
        theme: AppTheme,
        text: &str,
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlight_snapshot(
            rows::syntax_highlights_for_line(
                theme,
                text,
                rows::DiffSyntaxLanguage::Yaml,
                rows::DiffSyntaxMode::Auto,
            )
            .as_slice(),
        )
    }

    fn line_debug(
        line: Option<(&str, &super::CachedDiffStyledText)>,
    ) -> Option<(
        String,
        Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    )> {
        let (text, styled) = line?;
        Some((
            text.to_string(),
            styled
                .highlights
                .iter()
                .map(|(range, style)| (range.clone(), style.color, style.background_color))
                .collect(),
        ))
    }

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(split_right_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            String,
            Vec<(
                std::ops::Range<usize>,
                Option<gpui::Hsla>,
                Option<gpui::Hsla>,
            )>,
        )>,
    )> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    line_debug(inline_cached_styled_by_new_line(pane, line)),
                )
            })
            .collect()
    }

    fn split_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<FileDiffRowKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| {
                (
                    line,
                    split_right_row_by_new_line(pane, line).map(|row| row.kind),
                )
            })
            .collect()
    }

    fn inline_kind_debug(pane: &MainPaneView, lines: &[u32]) -> Vec<(u32, Option<DiffLineKind>)> {
        lines
            .iter()
            .copied()
            .map(|line| (line, inline_row_by_new_line(pane, line).map(|row| row.kind)))
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(84);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_build_release_file_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show = |spec: &str| {
        fixture_git_show(
            &repo_root,
            spec,
            "build-release YAML file-diff regression fixture",
        )
    };
    let old_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    );
    let new_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    );

    let baseline_secret_key_line = 20u32;
    let affected_secret_key_lines = [22u32, 24, 26, 28, 30, 32];
    let baseline_required_line = 21u32;
    let affected_required_lines = [23u32, 25, 27, 29, 31, 33];
    let add_lines = [20u32, 21u32];
    let context_lines = [22u32, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];
    let old_baseline_secret_key_line = 20u32;
    let old_affected_secret_key_lines = [22u32, 24, 26, 28, 30];
    let old_baseline_required_line = 21u32;
    let old_affected_required_lines = [23u32, 25, 27, 29, 31];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 0, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 0
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(33))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(33))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff split syntax keeps repeated secret keys and booleans consistent",
        |pane| {
            let Some((baseline_secret_key_text, baseline_secret_key_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_secret_key_line)
            else {
                return false;
            };
            let Some(baseline_secret_key_color) =
                mapping_key_color(baseline_secret_key_styled, baseline_secret_key_text)
            else {
                return false;
            };
            if add_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Add)
            }) {
                return false;
            }
            if context_lines.iter().copied().any(|line_no| {
                !split_right_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == FileDiffRowKind::Context)
            }) {
                return false;
            }
            if affected_secret_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_secret_key_color)
            }) {
                return false;
            }

            let Some((baseline_required_text, baseline_required_styled)) =
                split_right_cached_styled_by_new_line(pane, baseline_required_line)
            else {
                return false;
            };
            let Some(baseline_required_color) =
                scalar_color_after_colon(baseline_required_styled, baseline_required_text)
            else {
                return false;
            };
            !affected_required_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                scalar_color_after_colon(styled, text) != Some(baseline_required_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_secret_key_line);
            lines.extend(affected_secret_key_lines);
            lines.push(baseline_required_line);
            lines.extend(affected_required_lines);
            format!(
                "diff_view={:?} split_kinds={:?} split_debug={:?}",
                pane.diff_view,
                split_kind_debug(pane, &lines),
                split_debug(pane, &lines),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut old_lines = Vec::new();
        old_lines.push(old_baseline_secret_key_line);
        old_lines.extend(old_affected_secret_key_lines);
        old_lines.push(old_baseline_required_line);
        old_lines.extend(old_affected_required_lines);

        for old_line in old_lines {
            let Some(row) = split_left_row_by_old_line(pane, old_line) else {
                panic!("expected split-left row for old line {old_line}");
            };
            assert_eq!(
                row.kind,
                FileDiffRowKind::Context,
                "expected build-release old line {old_line} to remain a context row on the left side"
            );
            let Some((text, styled)) = split_left_cached_styled_by_old_line(pane, old_line) else {
                panic!("expected cached split-left styled text for old line {old_line}");
            };
            let expected = expected_yaml_snapshot(theme, text);
            let actual = highlight_snapshot(styled.highlights.as_ref());
            assert_eq!(
                actual, expected,
                "split-left YAML highlighting should match direct single-line YAML highlights for build-release old line {old_line}: text={text:?}"
            );
        }
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML file-diff inline syntax keeps repeated secret keys and booleans consistent",
        |pane| {
            let Some((baseline_secret_key_text, baseline_secret_key_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_secret_key_line)
            else {
                return false;
            };
            let Some(baseline_secret_key_color) =
                mapping_key_color(baseline_secret_key_styled, baseline_secret_key_text)
            else {
                return false;
            };
            if add_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Add)
            }) {
                return false;
            }
            if context_lines.iter().copied().any(|line_no| {
                !inline_row_by_new_line(pane, line_no)
                    .is_some_and(|row| row.kind == DiffLineKind::Context)
            }) {
                return false;
            }
            if affected_secret_key_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                mapping_key_color(styled, text) != Some(baseline_secret_key_color)
            }) {
                return false;
            }

            let Some((baseline_required_text, baseline_required_styled)) =
                inline_cached_styled_by_new_line(pane, baseline_required_line)
            else {
                return false;
            };
            let Some(baseline_required_color) =
                scalar_color_after_colon(baseline_required_styled, baseline_required_text)
            else {
                return false;
            };
            !affected_required_lines.iter().copied().any(|line_no| {
                let Some((text, styled)) = inline_cached_styled_by_new_line(pane, line_no) else {
                    return true;
                };
                scalar_color_after_colon(styled, text) != Some(baseline_required_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_secret_key_line);
            lines.extend(affected_secret_key_lines);
            lines.push(baseline_required_line);
            lines.extend(affected_required_lines);
            format!(
                "diff_view={:?} inline_kinds={:?} inline_debug={:?}",
                pane.diff_view,
                inline_kind_debug(pane, &lines),
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_file_diff_matches_prepared_document_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_left_cached_styled_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.old_line == Some(old_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.old.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitLeft)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn split_right_row_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.new_line == Some(new_line))
    }

    fn split_left_row_by_old_line(
        pane: &MainPaneView,
        old_line: u32,
    ) -> Option<&gitcomet_core::file_diff::FileDiffRow> {
        pane.file_diff_cache_rows
            .iter()
            .find(|row| row.old_line == Some(old_line))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn inline_row_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<&AnnotatedDiffLine> {
        pane.file_diff_inline_cache
            .iter()
            .find(|line| line.new_line == Some(new_line))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);

    let repo_id = gitcomet_state::model::RepoId(184);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_build_release_prepared_baseline",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "build-release prepared-baseline fixture");
    let old_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    );
    let new_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    );
    let (old_shared_text, old_line_starts) = shared_text_and_line_starts(old_text.as_str());
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let old_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        old_shared_text,
        Arc::clone(&old_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared old YAML baseline document, got {other:?}"),
    };
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(1),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!("expected prepared new YAML baseline document, got {other:?}"),
    };

    let old_lines = [20u32, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31];
    let new_lines = [20u32, 21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_old_by_line = old_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                old_text.as_str(),
                old_line_starts.as_ref(),
                old_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for old line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();
    let baseline_new_by_line = new_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| panic!("expected prepared YAML baseline for new line {line_no}"));
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release YAML rows ready for prepared-baseline comparison",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new_line == Some(33))
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.new_line == Some(33))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} language={:?} left_doc={:?} right_doc={:?} rows={} inline_rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
                pane.file_diff_inline_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in old_lines {
            let actual = split_left_cached_styled_by_old_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected split-left styled text for build-release old line {line_no}")
                });
            let expected = baseline_old_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release old line {line_no}"));
            assert_eq!(
                actual, expected,
                "split-left YAML highlighting should match prepared baseline for build-release old line {line_no}"
            );
        }

        for line_no in new_lines {
            let actual = split_right_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected split-right styled text for build-release new line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release new line {line_no}"));
            assert_eq!(
                actual, expected,
                "split-right YAML highlighting should match prepared baseline for build-release new line {line_no}"
            );
        }
    });

    let split_left_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        old_lines
            .iter()
            .copied()
            .map(|line_no| {
                (0..pane.diff_visible_len())
                    .find(|&visible_ix| {
                        let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                            return false;
                        };
                        pane.file_diff_split_row(row_ix)
                            .is_some_and(|row| row.old_line == Some(line_no))
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "expected split-left visible row for build-release old line {line_no}"
                        )
                    })
            })
            .collect::<Vec<_>>()
    });
    let split_right_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                split_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected split-right visible row for build-release new line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_left_visible_indices.as_slice());
    draw_rows_for_visible_indices(cx, &view, split_right_visible_indices.as_slice());

    for (&line_no, &visible_ix) in old_lines.iter().zip(split_left_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitLeft);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) = split_left_cached_styled_by_old_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected cached split-left styled text for build-release old line {line_no}")
                });
            let kind = split_left_row_by_old_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-left row for build-release old line {line_no}")
                })
                .kind;
            (text.to_string(), highlight_snapshot(styled.highlights.as_ref()), kind)
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release split-left render text should match cache for old line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release split-left render highlights should match cache for old line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Remove | FileDiffRowKind::Modify),
            "build-release split-left render should preserve diff background for old line {line_no}"
        );
    }

    for (&line_no, &visible_ix) in new_lines.iter().zip(split_right_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) = split_right_cached_styled_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected cached split-right styled text for build-release new line {line_no}")
                });
            let kind = split_right_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected split-right row for build-release new line {line_no}")
                })
                .kind;
            (text.to_string(), highlight_snapshot(styled.highlights.as_ref()), kind)
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release split-right render text should match cache for new line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release split-right render highlights should match cache for new line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, FileDiffRowKind::Add | FileDiffRowKind::Modify),
            "build-release split-right render should preserve diff background for new line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        new_lines
            .iter()
            .copied()
            .map(|line_no| {
                inline_visible_ix_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!("expected inline visible row for build-release new line {line_no}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for line_no in new_lines {
            let actual = inline_cached_styled_by_new_line(pane, line_no)
                .map(cached_snapshot)
                .unwrap_or_else(|| {
                    panic!("expected inline styled text for build-release new line {line_no}")
                });
            let expected = baseline_new_by_line
                .get(&line_no)
                .cloned()
                .unwrap_or_else(|| panic!("missing prepared baseline for build-release new line {line_no}"));
            assert_eq!(
                actual, expected,
                "inline YAML highlighting should match prepared baseline for build-release new line {line_no}"
            );
        }
    });

    for (&line_no, &visible_ix) in new_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let (text, styled, kind) = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let (text, styled) =
                inline_cached_styled_by_new_line(pane, line_no).unwrap_or_else(|| {
                    panic!(
                        "expected cached inline styled text for build-release new line {line_no}"
                    )
                });
            let kind = inline_row_by_new_line(pane, line_no)
                .unwrap_or_else(|| {
                    panic!("expected inline row for build-release new line {line_no}")
                })
                .kind;
            (
                text.to_string(),
                highlight_snapshot(styled.highlights.as_ref()),
                kind,
            )
        });
        assert_eq!(
            record.text.as_ref(),
            text.as_str(),
            "build-release inline render text should match cache for new line {line_no}"
        );
        assert_eq!(
            record.highlights, styled,
            "build-release inline render highlights should match cache for new line {line_no}"
        );
        assert_eq!(
            record.row_bg.is_some(),
            matches!(kind, DiffLineKind::Add | DiffLineKind::Remove),
            "build-release inline render should preserve diff background for new line {line_no}"
        );
    }
}

#[gpui::test]
fn yaml_commit_file_diff_transition_from_patch_clears_stale_split_cache(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffTarget;

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn highlight_snapshot(
        highlights: &[(std::ops::Range<usize>, gpui::HighlightStyle)],
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlights
            .iter()
            .map(|(range, style)| (range.clone(), style.color, style.background_color))
            .collect()
    }

    fn expected_yaml_snapshot(
        theme: AppTheme,
        text: &str,
    ) -> Vec<(
        std::ops::Range<usize>,
        Option<gpui::Hsla>,
        Option<gpui::Hsla>,
    )> {
        highlight_snapshot(
            rows::syntax_highlights_for_line(
                theme,
                text,
                rows::DiffSyntaxLanguage::Yaml,
                rows::DiffSyntaxMode::Auto,
            )
            .as_slice(),
        )
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let repo_id = gitcomet_state::model::RepoId(85);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_to_file_transition",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let patch_text =
        std::fs::read_to_string(fixture_repo_root().join("test_data/commit-bd8b4a04.patch"))
            .expect("read patch fixture");
    let patch_target = DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let patch_diff = gitcomet_core::domain::Diff::from_unified(patch_target.clone(), &patch_text);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(patch_target);
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(patch_diff));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "patch diff split cache seeded before switching to file diff",
        |pane| {
            !pane.is_file_diff_view_active()
                && pane.patch_diff_split_row_len() > 0
                && !pane.diff_text_segments_cache.is_empty()
        },
        |pane| {
            format!(
                "file_diff_active={} diff_view={:?} patch_rows={} split_rows={} text_cache_len={}",
                pane.is_file_diff_view_active(),
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.patch_diff_split_row_len(),
                pane.diff_text_segments_cache.len(),
            )
        },
    );

    let repo_root = fixture_repo_root();
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "patch->file YAML transition fixture");
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");
    let unified = fixture_git_diff(
        &repo_root,
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml",
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml",
        "patch->file YAML transition fixture",
    );
    let file_target = DiffTarget::Commit {
        commit_id,
        path: Some(path.clone()),
    };
    let file_diff = gitcomet_core::domain::Diff::from_unified(file_target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(file_target.clone());
            repo.diff_state.diff_rev = 2;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(file_diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "patch -> file diff transition yields fresh deployment-ci split highlights",
        |pane| {
            pane.is_file_diff_view_active()
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_target == Some(file_target.clone())
                && split_right_cached_styled_by_new_line(pane, 17).is_some()
                && split_right_cached_styled_by_new_line(pane, 18).is_some()
                && split_right_cached_styled_by_new_line(pane, 33).is_some()
        },
        |pane| {
            format!(
                "file_diff_active={} inflight={:?} cache_target={:?} active_target={:?} cache_len={} split17={:?} split18={:?} split33={:?}",
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_target.clone(),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.diff_text_segments_cache.len(),
                split_right_cached_styled_by_new_line(pane, 17).map(|(text, styled)| (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref())
                )),
                split_right_cached_styled_by_new_line(pane, 18).map(|(text, styled)| (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref())
                )),
                split_right_cached_styled_by_new_line(pane, 33).map(|(text, styled)| (
                    text.to_string(),
                    highlight_snapshot(styled.highlights.as_ref())
                )),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for new_line in [17u32, 18, 22, 33] {
            let Some((text, styled)) = split_right_cached_styled_by_new_line(pane, new_line) else {
                panic!("expected cached split-right styled text for deployment-ci new line {new_line}");
            };
            let expected = expected_yaml_snapshot(theme, text);
            let actual = highlight_snapshot(styled.highlights.as_ref());
            assert_eq!(
                actual, expected,
                "patch->file transition should not reuse stale split-right styling for deployment-ci new line {new_line}: text={text:?}"
            );
        }
    });
}

#[allow(dead_code)]
fn yaml_same_content_rev_refresh_invalidates_cached_heuristic_file_diff_rows(
    cx: &mut gpui::TestAppContext,
) {
    use std::collections::BTreeMap;

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let row_ix = pane
            .file_diff_cache_rows
            .iter()
            .position(|row| row.new_line == Some(new_line))?;
        let text = pane.file_diff_cache_rows.get(row_ix)?.new.as_deref()?;
        let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
        let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
        let styled = pane.diff_text_segments_cache_get(key, epoch)?;
        Some((text, styled))
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(&str, &super::CachedDiffStyledText)> {
        let inline_ix = pane
            .file_diff_inline_cache
            .iter()
            .position(|line| line.new_line == Some(new_line))?;
        let line = pane.file_diff_inline_cache.get(inline_ix)?;
        let epoch = pane.file_diff_inline_style_cache_epoch(line);
        let styled = pane.diff_text_segments_cache_get(inline_ix, epoch)?;
        Some((styled.text.as_ref(), styled))
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_split_row(row_ix)
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            pane.file_diff_inline_row(inline_ix)
                .is_some_and(|line| line.new_line == Some(new_line))
        })
    }

    fn draw_rows_for_visible_indices(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_indices: &[usize],
    ) {
        for &visible_ix in visible_indices {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                        cx.notify();
                    });
                });
            });
            cx.run_until_parked();
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
        }
    }

    fn one_based_line_byte_range(
        text: &str,
        line_starts: &[usize],
        line_no: u32,
    ) -> Option<std::ops::Range<usize>> {
        let line_ix = usize::try_from(line_no).ok()?.checked_sub(1)?;
        let start = (*line_starts.get(line_ix)?).min(text.len());
        let mut end = line_starts
            .get(line_ix.saturating_add(1))
            .copied()
            .unwrap_or(text.len())
            .min(text.len());
        if end > start && text.as_bytes().get(end.saturating_sub(1)) == Some(&b'\n') {
            end = end.saturating_sub(1);
        }
        Some(start..end)
    }

    fn shared_text_and_line_starts(text: &str) -> (gpui::SharedString, Arc<[usize]>) {
        let mut line_starts = Vec::with_capacity(text.len().saturating_div(64).saturating_add(1));
        line_starts.push(0usize);
        for (ix, byte) in text.as_bytes().iter().enumerate() {
            if *byte == b'\n' {
                line_starts.push(ix.saturating_add(1));
            }
        }
        (text.to_string().into(), Arc::from(line_starts))
    }

    fn prepared_document_snapshot_for_line(
        theme: AppTheme,
        text: &str,
        line_starts: &[usize],
        document: rows::PreparedDiffSyntaxDocument,
        language: rows::DiffSyntaxLanguage,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        let byte_range = one_based_line_byte_range(text, line_starts, line_no)?;
        let line_text = text.get(byte_range.clone())?.to_string();
        let started = std::time::Instant::now();

        loop {
            let highlights = rows::request_syntax_highlights_for_prepared_document_byte_range(
                theme,
                text,
                line_starts,
                document,
                language,
                byte_range.clone(),
            )?;

            if !highlights.pending {
                return Some(LineSyntaxSnapshot {
                    text: line_text.clone(),
                    syntax: highlights
                        .highlights
                        .into_iter()
                        .filter(|(_, style)| style.background_color.is_none())
                        .map(|(range, style)| {
                            (
                                range.start.saturating_sub(byte_range.start)
                                    ..range.end.saturating_sub(byte_range.start),
                                style.color,
                            )
                        })
                        .collect(),
                });
            }

            let completed =
                rows::drain_completed_prepared_diff_syntax_chunk_builds_for_document(document);
            if completed == 0 && started.elapsed() >= std::time::Duration::from_secs(2) {
                return None;
            }
            if completed == 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }

    fn cached_snapshot(line: (&str, &super::CachedDiffStyledText)) -> LineSyntaxSnapshot {
        let (text, styled) = line;
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: styled
                .highlights
                .iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range.clone(), style.color))
                .collect(),
        }
    }

    fn paint_snapshot(record: &rows::DiffPaintRecord) -> LineSyntaxSnapshot {
        LineSyntaxSnapshot {
            text: record.text.to_string(),
            syntax: record
                .highlights
                .iter()
                .filter(|(_, _, bg)| bg.is_none())
                .map(|(range, color, _)| (range.clone(), *color))
                .collect(),
        }
    }

    fn draw_paint_record_for_visible_ix(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        visible_ix: usize,
        region: DiffTextRegion,
    ) -> rows::DiffPaintRecord {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
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

    fn split_mismatch_lines(
        pane: &MainPaneView,
        baselines: &BTreeMap<u32, LineSyntaxSnapshot>,
        lines: &[u32],
    ) -> Vec<u32> {
        lines
            .iter()
            .copied()
            .filter(|line| {
                let Some(actual) =
                    split_right_cached_styled_by_new_line(pane, *line).map(cached_snapshot)
                else {
                    return false;
                };
                baselines
                    .get(line)
                    .is_some_and(|expected| actual != *expected)
            })
            .collect()
    }

    fn inline_mismatch_lines(
        pane: &MainPaneView,
        baselines: &BTreeMap<u32, LineSyntaxSnapshot>,
        lines: &[u32],
    ) -> Vec<u32> {
        lines
            .iter()
            .copied()
            .filter(|line| {
                let Some(actual) =
                    inline_cached_styled_by_new_line(pane, *line).map(cached_snapshot)
                else {
                    return false;
                };
                baselines
                    .get(line)
                    .is_some_and(|expected| actual != *expected)
            })
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let repo_id = gitcomet_state::model::RepoId(87);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_same_content_rev_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show = |spec: &str| {
        fixture_git_show(
            &repo_root,
            spec,
            "same-content YAML refresh regression fixture",
        )
    };
    fn append_yaml_padding(text: &str) -> String {
        use std::fmt::Write as _;

        const PADDING_LINES: usize = 65_536;
        let mut out = String::with_capacity(text.len().saturating_add(PADDING_LINES * 64));
        out.push_str(text);
        if !out.ends_with('\n') {
            out.push('\n');
        }
        for ix in 0..PADDING_LINES {
            let _ = writeln!(
                out,
                "# syntax-padding-{ix:05}-abcdefghijklmnopqrstuvwxyz0123456789"
            );
        }
        out
    }

    let old_text = append_yaml_padding(&git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    ));
    let new_text = append_yaml_padding(&git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    ));
    let affected_lines = [173u32, 175, 176, 183, 190, 193, 206, 212, 218, 221];
    let (new_shared_text, new_line_starts) = shared_text_and_line_starts(new_text.as_str());
    let new_document = match rows::prepare_diff_syntax_document_with_budget_reuse_text(
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
        new_shared_text,
        Arc::clone(&new_line_starts),
        rows::DiffSyntaxBudget {
            foreground_parse: std::time::Duration::from_secs(5),
        },
        None,
        None,
    ) {
        rows::PrepareDiffSyntaxDocumentResult::Ready(document) => document,
        other => panic!(
            "expected prepared YAML baseline document for same-content refresh, got {other:?}"
        ),
    };
    let baseline_new_by_line = affected_lines
        .iter()
        .copied()
        .map(|line_no| {
            let snapshot = prepared_document_snapshot_for_line(
                theme,
                new_text.as_str(),
                new_line_starts.as_ref(),
                new_document,
                rows::DiffSyntaxLanguage::Yaml,
                line_no,
            )
            .unwrap_or_else(|| {
                panic!("expected prepared YAML baseline for build-release line {line_no}")
            });
            (line_no, snapshot)
        })
        .collect::<BTreeMap<_, _>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 1, &old_text, &new_text);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release file-diff rows ready before same-content refresh",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 1
                && affected_lines
                    .iter()
                    .copied()
                    .all(|line| split_visible_ix_by_new_line(pane, line).is_some())
        },
        |pane| {
            let split_mismatches =
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines);
            let first_mismatch = split_mismatches.first().copied();
            let cache_row_ix = first_mismatch.and_then(|line_no| {
                pane.file_diff_cache_rows
                    .iter()
                    .position(|row| row.new_line == Some(line_no))
            });
            let provider_row_ix = first_mismatch.and_then(|line_no| {
                (0..pane.file_diff_split_row_len()).find(|&row_ix| {
                    pane.file_diff_split_row(row_ix)
                        .is_some_and(|row| row.new_line == Some(line_no))
                })
            });
            let actual = first_mismatch.and_then(|line_no| {
                split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
            });
            let cached_text = cache_row_ix.and_then(|row_ix| {
                let key = pane.file_diff_split_cache_key(row_ix, DiffTextRegion::SplitRight)?;
                let epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight);
                pane.diff_text_segments_cache_get(key, epoch)
                    .map(|styled| styled.text.to_string())
            });
            let expected =
                first_mismatch.and_then(|line_no| baseline_new_by_line.get(&line_no).cloned());
            let doc_actual = pane
                .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .and_then(|document| {
                    first_mismatch.and_then(|line_no| {
                        prepared_document_snapshot_for_line(
                            theme,
                            new_text.as_str(),
                            new_line_starts.as_ref(),
                            document,
                            rows::DiffSyntaxLanguage::Yaml,
                            line_no,
                        )
                    })
                });
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={} split_mismatches={split_mismatches:?} first_mismatch={first_mismatch:?} cache_row_ix={cache_row_ix:?} provider_row_ix={provider_row_ix:?} cached_text={cached_text:?} actual={actual:?} doc_actual={doc_actual:?} expected={expected:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );

    let split_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        affected_lines
            .iter()
            .copied()
            .map(|line| {
                split_visible_ix_by_new_line(pane, line).unwrap_or_else(|| {
                    panic!("expected split visible row for build-release line {line}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    let (epoch_before, right_doc_ready_before, heuristic_mismatches) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines),
        )
    });
    if !right_doc_ready_before {
        assert!(
            !heuristic_mismatches.is_empty(),
            "expected at least one build-release YAML block-scalar line to differ while only heuristic styling is cached"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(500),
                });
            });
        });
    });

    seed_file_diff_state_with_rev(cx, &view, repo_id, &workdir, &path, 2, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "build-release file-diff rows ready after same-content refresh",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && affected_lines
                    .iter()
                    .copied()
                    .all(|line| split_visible_ix_by_new_line(pane, line).is_some())
        },
        |pane| {
            let split_mismatches =
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines);
            let first_mismatch = split_mismatches.first().copied();
            let actual = first_mismatch.and_then(|line_no| {
                split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
            });
            let expected =
                first_mismatch.and_then(|line_no| baseline_new_by_line.get(&line_no).cloned());
            let doc_actual = pane
                .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .and_then(|document| {
                    first_mismatch.and_then(|line_no| {
                        prepared_document_snapshot_for_line(
                            theme,
                            new_text.as_str(),
                            new_line_starts.as_ref(),
                            document,
                            rows::DiffSyntaxLanguage::Yaml,
                            line_no,
                        )
                    })
                });
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={} split_mismatches={split_mismatches:?} first_mismatch={first_mismatch:?} actual={actual:?} doc_actual={doc_actual:?} expected={expected:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should expose the build-release right document",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && (right_doc_ready_before
                    || pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight)
                        > epoch_before)
        },
        |pane| {
            format!(
                "rev={} inflight={:?} right_doc={:?} split_epoch={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should finish build-release right-doc chunk requests",
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
        },
        |pane| {
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            format!(
                "rev={} right_doc={right_doc:?} right_pending={:?} split_mismatches={:?}",
                pane.file_diff_cache_rev,
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                split_mismatch_lines(pane, &baseline_new_by_line, &affected_lines),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, split_visible_indices.as_slice());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });
    cx.run_until_parked();

    for (&line_no, &visible_ix) in affected_lines.iter().zip(split_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::SplitRight);
        let cached = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            split_right_cached_styled_by_new_line(pane, line_no).map(cached_snapshot)
        });
        let expected = baseline_new_by_line
            .get(&line_no)
            .unwrap_or_else(|| panic!("missing build-release baseline for line {line_no}"));
        assert_eq!(
            cached,
            Some(expected.clone()),
            "diagnostic: split-right cache should match the prepared baseline after painting line {line_no}"
        );
        let actual = paint_snapshot(&record);
        assert_eq!(
            actual, *expected,
            "same-content refresh should repaint split-right build-release YAML highlighting for line {line_no}"
        );

        let expects_row_bg = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (0..pane.file_diff_split_row_len()).any(|row_ix| {
                pane.file_diff_split_row(row_ix).is_some_and(|row| {
                    row.new_line == Some(line_no)
                        && matches!(
                            row.kind,
                            gitcomet_core::file_diff::FileDiffRowKind::Add
                                | gitcomet_core::file_diff::FileDiffRowKind::Modify
                        )
                })
            })
        });
        assert_eq!(
            record.row_bg.is_some(),
            expects_row_bg,
            "same-content refresh should preserve split-right diff background for line {line_no}"
        );
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    let inline_visible_indices = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        affected_lines
            .iter()
            .copied()
            .map(|line| {
                inline_visible_ix_by_new_line(pane, line).unwrap_or_else(|| {
                    panic!("expected inline visible row for build-release line {line}")
                })
            })
            .collect::<Vec<_>>()
    });
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    wait_for_main_pane_condition(
        cx,
        &view,
        "same-content file-diff rev refresh should expose inline build-release rows",
        |pane| {
            pane.file_diff_cache_rev == 2
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );
    draw_rows_for_visible_indices(cx, &view, inline_visible_indices.as_slice());

    for (&line_no, &visible_ix) in affected_lines.iter().zip(inline_visible_indices.iter()) {
        let record =
            draw_paint_record_for_visible_ix(cx, &view, visible_ix, DiffTextRegion::Inline);
        let expected = baseline_new_by_line
            .get(&line_no)
            .unwrap_or_else(|| panic!("missing build-release baseline for line {line_no}"));
        let actual = paint_snapshot(&record);
        assert_eq!(
            actual, *expected,
            "same-content refresh should repaint inline build-release YAML highlighting for line {line_no}"
        );

        let expects_row_bg = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (0..pane.file_diff_inline_row_len()).any(|inline_ix| {
                pane.file_diff_inline_row(inline_ix).is_some_and(|line| {
                    line.new_line == Some(line_no)
                        && line.kind == gitcomet_core::domain::DiffLineKind::Add
                })
            })
        });
        assert_eq!(
            record.row_bg.is_some(),
            expects_row_bg,
            "same-content refresh should preserve inline diff background for line {line_no}"
        );
    }
}

/// Opens an unstaged text diff so the diff toolbar (Inline/Split + Blame)
/// renders, and puts the pane in `mode`.
fn push_unstaged_text_diff_for_blame_toggle(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture_name: &str,
    mode: DiffViewMode,
) {
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{fixture_name}",
        std::process::id()
    ));
    let path = PathBuf::from("src/lib.rs");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

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
            repo.diff_state.diff =
                gitcomet_state::model::Loadable::Ready(Arc::new(gitcomet_core::domain::Diff {
                    target: target.clone(),
                    lines: vec![
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Context,
                            text: "fn main() {".into(),
                        },
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Add,
                            text: "    let x = 1;".into(),
                        },
                        gitcomet_core::domain::DiffLine {
                            kind: gitcomet_core::domain::DiffLineKind::Context,
                            text: "}".into(),
                        },
                    ],
                }));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    // Go through the root setter so the root and the pane agree, exactly as the
    // toolbar buttons and the session restore do.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.set_diff_view_mode(mode, cx);
        });
    });
    draw_and_drain_test_window(cx);
}

fn click_blame_toggle(cx: &mut gpui::VisualTestContext) {
    let bounds = cx
        .debug_bounds("diff_annotate")
        .expect("the diff toolbar should render the blame toggle");
    cx.simulate_click(bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
}

/// Regression: enabling blame used to force Split → Inline (and restore it on
/// toggle-off). Blame is an annotation column, not a view mode — the split left
/// column renders it just as the inline view does — so the selected mode must
/// survive the toggle in both directions.
#[gpui::test]
fn blame_toggle_keeps_split_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(281);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_toggle_split",
        DiffViewMode::Split,
    );

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(pane.annotate_enabled, "clicking blame should enable it");
        assert_eq!(
            pane.diff_view,
            DiffViewMode::Split,
            "enabling blame must not switch the diff view to Inline"
        );
        assert_eq!(root.diff_view_mode, DiffViewMode::Split);
    });

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(!pane.annotate_enabled);
        assert_eq!(
            pane.diff_view,
            DiffViewMode::Split,
            "disabling blame must not change the diff view either"
        );
        assert_eq!(root.diff_view_mode, DiffViewMode::Split);
    });
}

#[gpui::test]
fn blame_toggle_keeps_inline_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(282);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_toggle_inline",
        DiffViewMode::Inline,
    );

    click_blame_toggle(cx);
    cx.update(|_window, app| {
        let root = view.read(app);
        let pane = root.main_pane.read(app);
        assert!(pane.annotate_enabled);
        assert_eq!(pane.diff_view, DiffViewMode::Inline);
        assert_eq!(root.diff_view_mode, DiffViewMode::Inline);
    });
}

/// The annotation column narrows the left split column, so the shared split
/// wrap width must shrink when blame is on — the guarantee that made forcing
/// Inline unnecessary in the first place.
#[gpui::test]
fn split_annotate_reserves_the_annotation_column(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(283);

    push_unstaged_text_diff_for_blame_toggle(
        cx,
        &view,
        repo_id,
        "blame_split_columns",
        DiffViewMode::Split,
    );

    let (_, split_without_blame) = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| pane.diff_wrap_columns(window, cx))
    });

    click_blame_toggle(cx);

    let (_, split_with_blame) = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            assert!(
                pane.annotation_active(),
                "an unstaged working-tree diff supports blame, so the column is active"
            );
            pane.diff_wrap_columns(window, cx)
        })
    });

    assert!(
        split_with_blame < split_without_blame,
        "the annotation column must narrow the split wrap width \
         (with blame: {split_with_blame}, without: {split_without_blame})"
    );
}

/// The command palette and the Settings window route mode changes through
/// `GitCometView::set_diff_view_mode` rather than the toolbar buttons, so the
/// styled-segment cache clear has to live in the pane setter: inline keys those
/// segments by `row_ix` while split keys them by `row_ix * 2` / `row_ix * 2 + 1`
/// against the same epochs, so a stale entry can paint the wrong row.
#[gpui::test]
fn toggle_diff_view_command_clears_styled_segment_caches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, _cx| {
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
        let pane = root.main_pane.read(app);
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));
    push_raw_patch_diff_state_with_rev(cx, &view, repo_id, fixture_name, unified, 1, true);
    wait_for_main_pane_condition(
        cx,
        &view,
        "wide patch diff ready for horizontal search reveal",
        |pane| pane.diff_cache_rev == 1 && pane.patch_diff_row_len() > 0,
        |pane| (pane.diff_cache_rev, pane.patch_diff_row_len()),
    );

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_search_active = true;
            pane.diff_search_query = "needle tail".into();
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
        let pane = view.read(app).main_pane.read(app);
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

#[gpui::test]
fn diff_search_scrolls_sideways_to_a_match_far_along_a_long_line(cx: &mut gpui::TestAppContext) {
    assert_diff_search_scrolls_sideways(
        cx,
        gitcomet_state::model::RepoId(9141),
        "search_horizontal_reveal",
    );
}
