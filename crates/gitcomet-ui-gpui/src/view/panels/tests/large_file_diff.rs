use super::*;

#[gpui::test]
fn large_file_diff_keeps_prepared_syntax_documents_above_old_line_gate(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(53);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_diff_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/large_file_diff.rs");
    let line_count = 4_001usize;
    let changed_old_line = format!(
        "let diff_value_{}: usize = {};",
        line_count - 1,
        line_count - 1
    );
    let changed_new_line = format!(
        "let diff_value_{}: usize = {};",
        line_count - 1,
        line_count * 2
    );
    let old_text = (0..line_count)
        .map(|ix| format!("let diff_value_{ix}: usize = {ix};"))
        .collect::<Vec<_>>()
        .join("\n");
    let new_text = (0..line_count)
        .map(|ix| {
            if ix + 1 == line_count {
                changed_new_line.clone()
            } else {
                format!("let diff_value_{ix}: usize = {ix};")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "large file-diff prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Rust)
                && pane.file_diff_old_text.len() == old_text.len()
                && pane.file_diff_old_line_starts.len() == line_count
                && pane.file_diff_new_text.len() == new_text.len()
                && pane.file_diff_new_line_starts.len() == line_count
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(changed_old_line.as_str()))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(changed_new_line.as_str()))
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?} cache_inflight={:?} cache_path={:?} language={:?} old_text_len={} old_line_starts={} new_text_len={} new_line_starts={} left_doc={:?} right_doc={:?} row_count={}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_old_text.len(),
                pane.file_diff_old_line_starts.len(),
                pane.file_diff_new_text.len(),
                pane.file_diff_new_line_starts.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
            )
        },
    );
}

#[gpui::test]
fn oversized_json_file_diff_uses_visible_line_fallback_without_prepared_syntax_documents(
    cx: &mut gpui::TestAppContext,
) {
    const OBJECT_COUNT: usize = 512;
    const PAYLOAD_BYTES: usize = 16 * 1024;
    const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(82);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_oversized_json_file_diff_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/oversized_file_diff.json");
    let old_lines = build_large_json_array_lines(OBJECT_COUNT, PAYLOAD_BYTES);
    let visible_json_line = old_lines[1].clone();
    let visible_inline_text = format!(" {visible_json_line}");
    let mut new_lines = old_lines.clone();
    let changed_line_ix = new_lines.len() - 2;
    let changed_payload = "y".repeat(PAYLOAD_BYTES);
    let changed_old_line = old_lines[changed_line_ix].clone();
    new_lines[changed_line_ix] = format!(
        r#"  {{"line": {}, "flag": false, "payload": "{changed_payload}"}}"#,
        OBJECT_COUNT - 1
    );
    let changed_new_line = new_lines[changed_line_ix].clone();
    let line_count = old_lines.len();
    let old_text = old_lines.join("\n");
    let new_text = new_lines.join("\n");

    assert!(
        line_count < 4_001,
        "fixture should stay below the old line-count gate so this test specifically exercises the new byte gate"
    );
    assert!(
        old_text.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "old-side fixture should exceed the prepared-document byte gate"
    );
    assert!(
        new_text.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "new-side fixture should exceed the prepared-document byte gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create oversized JSON diff workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "oversized JSON file-diff cache build",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Json)
                && pane.file_diff_old_text.len() == old_text.len()
                && pane.file_diff_old_text.len() > PREPARED_DOCUMENT_MAX_BYTES
                && pane.file_diff_old_line_starts.len() == line_count
                && pane.file_diff_new_text.len() == new_text.len()
                && pane.file_diff_new_text.len() > PREPARED_DOCUMENT_MAX_BYTES
                && pane.file_diff_new_line_starts.len() == line_count
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(changed_old_line.as_str()))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(changed_new_line.as_str()))
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} language={:?} old_text_len={} old_line_starts={} new_text_len={} new_line_starts={} left_doc={:?} right_doc={:?} row_count={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_old_text.len(),
                pane.file_diff_old_line_starts.len(),
                pane.file_diff_new_text.len(),
                pane.file_diff_new_line_starts.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "oversized JSON split diff heuristic syntax fallback",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && file_diff_split_cached_styled(
                    pane,
                    DiffTextRegion::SplitRight,
                    &visible_json_line,
                )
                .is_some_and(|styled| {
                    styled.text.as_ref() == visible_json_line && !styled.highlights.is_empty()
                })
        },
        |pane| {
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, &visible_json_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} split_cached={split_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "oversized JSON inline diff heuristic syntax fallback",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && file_diff_inline_cached_styled(
                    pane,
                    gitcomet_core::domain::DiffLineKind::Context,
                    &visible_inline_text,
                )
                .is_some_and(|styled| {
                    styled.text.as_ref() == visible_json_line && !styled.highlights.is_empty()
                })
        },
        |pane| {
            let inline_cached = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &visible_inline_text,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} inline_cached={inline_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let split_cached =
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, &visible_json_line)
                .expect("oversized JSON split diff should cache the visible fallback row");
        let inline_cached = file_diff_inline_cached_styled(
            pane,
            gitcomet_core::domain::DiffLineKind::Context,
            &visible_inline_text,
        )
        .expect("oversized JSON inline diff should cache the visible fallback row");
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none(),
            "oversized JSON diff should keep the left side on the visible-line fallback path"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_none(),
            "oversized JSON diff should keep the right side on the visible-line fallback path"
        );
        assert!(
            !split_cached.highlights.is_empty(),
            "oversized JSON split diff should still render heuristic syntax highlights"
        );
        assert!(
            !inline_cached.highlights.is_empty(),
            "oversized JSON inline diff should still render heuristic syntax highlights"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup oversized JSON diff workdir");
}

#[gpui::test]
fn minified_json_file_diff_streams_visible_slices_and_inline_search(cx: &mut gpui::TestAppContext) {
    const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;
    const PAYLOAD_BYTES: usize = PREPARED_DOCUMENT_MAX_BYTES + 256 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(92);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_file_diff_streamed",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/streamed_diff.json");
    let old_text = format!(
        r#"{{"needle":"streamed-inline-search","payload":"{}","version":1}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let new_text = format!(
        r#"{{"needle":"streamed-inline-search","payload":"{}","version":2}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );

    assert!(
        old_text.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "old-side fixture should exceed the prepared-document byte gate"
    );
    assert!(
        new_text.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "new-side fixture should exceed the prepared-document byte gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed diff workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "streamed minified file-diff cache build",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Json)
                && pane.file_diff_old_text.len() == old_text.len()
                && pane.file_diff_old_text.len() > PREPARED_DOCUMENT_MAX_BYTES
                && pane.file_diff_new_text.len() == new_text.len()
                && pane.file_diff_new_text.len() > PREPARED_DOCUMENT_MAX_BYTES
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane.diff_visible_len() >= 1
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} language={:?} old_text_len={} new_text_len={} left_doc={:?} right_doc={:?} diff_visible_len={} inline_provider={} split_provider={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_old_text.len(),
                pane.file_diff_new_text.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.diff_visible_len(),
                pane.file_diff_inline_row_provider.is_some(),
                pane.file_diff_row_provider.is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "streamed minified inline diff horizontal overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Json)
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane.diff_scroll.0.borrow().base_handle.max_offset().x > px(0.0)
        },
        |pane| {
            format!(
                "language={:?} left_doc={:?} right_doc={:?} max_offset={:?}",
                pane.file_diff_cache_language,
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.diff_scroll.0.borrow().base_handle.max_offset()
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hitbox = pane
            .diff_text_hitboxes
            .get(&(0, DiffTextRegion::Inline))
            .expect("streamed inline diff row should install a diff hitbox");
        assert!(
            hitbox.streamed_ascii_monospace_cell_width.is_some(),
            "giant inline diff row should use streamed monospace hit-testing"
        );
        assert_eq!(
            pane.diff_text_segments_cache.iter().flatten().count(),
            0,
            "streamed giant inline diff rows should bypass the full-line styled cache"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_none(),
            "oversized minified inline diff should keep the left side on the streamed heuristic fallback path"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_none(),
            "oversized minified inline diff should keep the right side on the streamed heuristic fallback path"
        );

        let paint_record = rows::diff_paint_log_for_tests()
            .into_iter()
            .find(|record| record.visible_ix == 0 && record.region == DiffTextRegion::Inline)
            .expect("streamed inline diff draw should record the visible line paint");
        assert!(
            paint_record.text.len() < old_text.len(),
            "streamed inline diff should paint only a visible slice, got {} of {} bytes",
            paint_record.text.len(),
            old_text.len()
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let handle = pane.diff_scroll.0.borrow().base_handle.clone();
                let max_offset = handle.max_offset();
                handle.set_offset(point(-max_offset.x.min(px(2400.0)), px(0.0)));
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, _app| {
        let paint_record = rows::diff_paint_log_for_tests()
            .into_iter()
            .find(|record| record.visible_ix == 0 && record.region == DiffTextRegion::Inline)
            .expect(
                "horizontally scrolled streamed inline diff should record the visible line paint",
            );
        assert!(
            paint_record.text.as_ref().starts_with('x'),
            "scrolled inline diff slice should start inside the JSON payload string, got {:?}",
            &paint_record.text.as_ref()[..paint_record.text.len().min(32)]
        );
        assert!(
            paint_record
                .highlights
                .iter()
                .any(|(range, color, background)| {
                    range.start == 0 && range.end > 32 && color.is_some() && background.is_none()
                }),
            "scrolled inline diff slice should keep payload string highlighting: {:?}",
            paint_record.highlights
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "streamed-inline-search".into();
                pane.diff_search_recompute_matches();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            pane.diff_visible_len(),
            "inline file-diff search should match every visible streamed row"
        );
        assert_eq!(
            pane.diff_text_segments_cache.iter().flatten().count(),
            0,
            "streamed inline search should not backfill the full-line styled cache"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_hitbox = pane
            .diff_text_hitboxes
            .get(&(0, DiffTextRegion::SplitLeft))
            .expect("streamed split diff row should install a left hitbox");
        let right_hitbox = pane
            .diff_text_hitboxes
            .get(&(0, DiffTextRegion::SplitRight))
            .expect("streamed split diff row should install a right hitbox");
        assert!(left_hitbox.streamed_ascii_monospace_cell_width.is_some());
        assert!(right_hitbox.streamed_ascii_monospace_cell_width.is_some());
        assert_eq!(
            pane.diff_text_segments_cache.iter().flatten().count(),
            0,
            "streamed split diff rows should bypass the full-line styled cache"
        );

        let paint_records = rows::diff_paint_log_for_tests();
        let latest_left_record = paint_records
            .iter()
            .rev()
            .find(|record| {
                record.visible_ix == 0 && record.region == DiffTextRegion::SplitLeft
            })
            .expect("streamed split diff should paint the visible left side");
        let latest_right_record = paint_records
            .iter()
            .rev()
            .find(|record| {
                record.visible_ix == 0 && record.region == DiffTextRegion::SplitRight
            })
            .expect("streamed split diff should paint the visible right side");
        assert!(
            latest_left_record.text.len() < old_text.len(),
            "streamed split diff should paint only a visible slice on the left, got {} of {} bytes",
            latest_left_record.text.len(),
            old_text.len()
        );
        assert!(
            latest_right_record.text.len() < new_text.len(),
            "streamed split diff should paint only a visible slice on the right, got {} of {} bytes",
            latest_right_record.text.len(),
            new_text.len()
        );
        assert!(
            !latest_left_record.text.is_empty() && !latest_right_record.text.is_empty(),
            "streamed split diff should still paint non-empty visible slices on both sides"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed diff workdir");
}

#[gpui::test]
fn split_file_diff_scroll_sync_matrix_covers_all_modes_and_axes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(214);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_scroll_sync_none",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/split_scroll_sync_none.rs");
    let old_text = (0..160)
        .map(|ix| format!("const LEFT_{ix:03}: &str = \"{}\";", "L".repeat(240)))
        .collect::<Vec<_>>()
        .join("\n");
    let new_text = (0..160)
        .map(|ix| format!("const RIGHT_{ix:03}: &str = \"{}\";", "R".repeat(240)))
        .collect::<Vec<_>>()
        .join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create split scroll-sync-none workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "split file-diff scroll-sync-none fixture initialized",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.diff_visible_len() >= 1
        },
        |pane| {
            format!(
                "cache_inflight={:?} cache_path={:?} diff_visible_len={} left_max={:?} right_max={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.diff_visible_len(),
                uniform_list_max_offset(&pane.diff_scroll),
                uniform_list_max_offset(&pane.diff_split_right_scroll),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.diff_split_right_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split file-diff scroll-sync matrix overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.diff_view == DiffViewMode::Split
                && uniform_list_max_offset(&pane.diff_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.diff_split_right_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.diff_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.diff_split_right_scroll).height > px(120.0)
        },
        |pane| {
            format!(
                "diff_view={:?} left_offset={:?} right_offset={:?} left_max={:?} right_max={:?}",
                pane.diff_view,
                uniform_list_offset(&pane.diff_scroll),
                uniform_list_offset(&pane.diff_split_right_scroll),
                uniform_list_max_offset(&pane.diff_scroll),
                uniform_list_max_offset(&pane.diff_split_right_scroll),
            )
        },
    );

    let reset_offsets = |cx: &mut gpui::VisualTestContext,
                         view: &gpui::Entity<super::super::GitCometView>| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    reset_uniform_list_offsets(&[&pane.diff_scroll, &pane.diff_split_right_scroll]);
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);
    };

    for mode in ALL_DIFF_SCROLL_SYNC_MODES {
        set_diff_scroll_sync_for_test(cx, &view, mode);
        cx.update(|_window, app| {
            assert_eq!(view.read(app).diff_scroll_sync_for_test(), mode);
        });

        for axis in ScrollSyncAxis::ALL {
            let left_offset = axis.offset(px(72.0));
            reset_offsets(cx, &view);
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        set_uniform_list_offset(&pane.diff_scroll, left_offset);
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let left = uniform_list_offset(&pane.diff_scroll);
                let right = uniform_list_offset(&pane.diff_split_right_scroll);
                let expected = if axis.includes(mode) {
                    axis.component(left_offset)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(left),
                    axis.component(left_offset),
                    "split diff left pane should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(right),
                    expected,
                    "split diff right pane should {} {} scrolling from the left pane in {:?} mode",
                    if axis.includes(mode) {
                        "sync"
                    } else {
                        "not sync"
                    },
                    axis.label(),
                    mode,
                );
            });

            let right_offset = axis.offset(px(96.0));
            reset_offsets(cx, &view);
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        set_uniform_list_offset(&pane.diff_split_right_scroll, right_offset);
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let left = uniform_list_offset(&pane.diff_scroll);
                let right = uniform_list_offset(&pane.diff_split_right_scroll);
                let expected = if axis.includes(mode) {
                    axis.component(right_offset)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(right),
                    axis.component(right_offset),
                    "split diff right pane should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(left),
                    expected,
                    "split diff left pane should {} {} scrolling from the right pane in {:?} mode",
                    if axis.includes(mode) {
                        "sync"
                    } else {
                        "not sync"
                    },
                    axis.label(),
                    mode,
                );
            });
        }
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup split scroll-sync-none workdir");
}

#[gpui::test]
fn minified_json_file_diff_partial_copy_uses_streamed_inline_row_source(
    cx: &mut gpui::TestAppContext,
) {
    const PAYLOAD_BYTES: usize = 256 * 1024;

    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(193);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_file_diff_partial_copy",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/streamed_diff_copy.json");
    let needle = "streamed-inline-copy";
    let old_text = format!(
        r#"{{"needle":"{needle}","payload":"{}","version":1}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let new_text = format!(
        r#"{{"needle":"{needle}","payload":"{}","version":2}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let start = old_text
        .find(needle)
        .expect("streamed inline copy needle should exist");
    let end = start + needle.len();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed diff copy workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "streamed minified file-diff copy cache build",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_inline_row_provider.is_some()
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} inline_provider={} split_provider={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_inline_row_provider.is_some(),
                pane.file_diff_row_provider.is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_text_anchor = Some(DiffTextPos {
                    visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: start,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: end,
                });
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.copy_selected_diff_text_to_clipboard(cx);
            });
        });
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(needle.to_string())
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed diff copy workdir");
}

#[gpui::test]
fn minified_json_file_diff_context_menu_copy_uses_streamed_inline_row_source(
    cx: &mut gpui::TestAppContext,
) {
    const PAYLOAD_BYTES: usize = 96 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(194);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_file_diff_context_menu_copy",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/streamed_diff_context_menu.json");
    let old_text = format!(
        r#"{{"needle":"streamed-inline-context-menu","payload":"{}","version":1}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let new_text = format!(
        r#"{{"needle":"streamed-inline-context-menu","payload":"{}","version":2}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed diff context-menu workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "streamed minified file-diff context-menu cache build",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_inline_row_provider.is_some()
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} inline_provider={} split_provider={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_inline_row_provider.is_some(),
                pane.file_diff_row_provider.is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_view = DiffViewMode::Inline;
            });
        });
    });

    cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.open_diff_editor_context_menu(
                0,
                DiffTextRegion::Inline,
                gpui::point(px(24.0), px(24.0)),
                window,
                cx,
            );
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, _cx| {
                let Some(popover_kind) = host.popover_kind_for_tests() else {
                    panic!("expected streamed inline diff context menu popover");
                };

                match popover_kind {
                    PopoverKind::DiffEditorMenu { copy_text, .. } => {
                        assert_eq!(copy_text, Some(old_text.clone()));
                    }
                    _ => panic!("expected streamed inline diff editor menu"),
                }
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed diff context-menu workdir");
}

#[gpui::test]
fn minified_json_file_diff_split_partial_copy_uses_streamed_row_source(
    cx: &mut gpui::TestAppContext,
) {
    const PAYLOAD_BYTES: usize = 256 * 1024;

    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(195);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_file_diff_split_partial_copy",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/streamed_diff_split_copy.json");
    let needle = "streamed-split-copy";
    let old_text = format!(
        r#"{{"needle":"{needle}","payload":"{}","version":1}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let new_text = format!(
        r#"{{"needle":"{needle}","payload":"{}","version":2}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let start = old_text
        .find(needle)
        .expect("streamed split copy needle should exist");
    let end = start + needle.len();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed split-copy workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "streamed minified split-copy cache build",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_row_provider.is_some()
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} inline_provider={} split_provider={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_inline_row_provider.is_some(),
                pane.file_diff_row_provider.is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                pane.diff_text_anchor = Some(DiffTextPos {
                    visible_ix: 0,
                    region: DiffTextRegion::SplitLeft,
                    offset: start,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    visible_ix: 0,
                    region: DiffTextRegion::SplitLeft,
                    offset: end,
                });
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.copy_selected_diff_text_to_clipboard(cx);
            });
        });
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(needle.to_string())
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed split-copy workdir");
}

#[gpui::test]
fn large_file_diff_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(61);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_diff_background_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/large_file_diff_bg.rs");
    let line_count = 4_001usize;
    let mut old_lines = vec![
        "/* start block comment".to_string(),
        "still inside block comment".to_string(),
        "end */".to_string(),
    ];
    old_lines.extend((3..line_count).map(|ix| format!("let diff_bg_{ix}: usize = {ix};")));
    let comment_line = old_lines[1].clone();
    let comment_inline_text = format!(" {comment_line}");
    let old_text = old_lines.join("\n");
    let mut new_lines = old_lines.clone();
    *new_lines.last_mut().unwrap() = format!(
        "let diff_bg_{}: usize = {};",
        line_count - 1,
        line_count * 2
    );
    let new_text = new_lines.join("\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });
    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, &old_text, &new_text);

    // Wait for the file-diff cache rows to be built. The zero foreground budget
    // means syntax timed out and a background parse has been spawned.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file-diff cache build (rows populated, syntax pending)",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && !pane.file_diff_cache_rows.is_empty()
        },
        |pane| {
            format!(
                "inflight={:?} cache_path={:?} rows={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    // Right after the cache build, the deterministic test scheduler may still
    // observe either the fallback path or an already-completed prepared document.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let _ = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
        let _ = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let (split_epoch_after_first_draw, fallback_split_highlights_hash) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let styled = file_diff_split_cached_styled(
                pane,
                DiffTextRegion::SplitLeft,
                comment_line.as_str(),
            )
            .expect("initial wait should populate the visible fallback split row cache");
            assert_eq!(
                styled.text.as_ref(),
                comment_line,
                "expected the cached split row to match the multiline comment text"
            );
            if styled.highlights.is_empty() {
                assert!(
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                        .is_none(),
                    "the first split draw should still be using the plain-text fallback before the background parse is applied"
                );
                assert!(
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                        .is_none(),
                    "the first split draw should still be using the plain-text fallback before the background parse is applied"
                );
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    Some(styled.highlights_hash),
                )
            } else {
                assert!(
                    styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    }),
                    "if the background parse wins the race before the first split draw, the cached split row should already be syntax highlighted"
                );
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    None,
                )
            }
        });

    // Wait for the background syntax parse to complete.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file-diff background syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            let left_epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, &comment_line)
                    .is_some_and(|styled| {
                        let upgraded_from_fallback = fallback_split_highlights_hash
                            .map(|hash| {
                                left_epoch > split_epoch_after_first_draw
                                    && styled.highlights_hash != hash
                            })
                            .unwrap_or(true);
                        upgraded_from_fallback
                            && styled.highlights.iter().any(|(range, style)| {
                                range.start == 0
                                    && range.end == comment_line.len()
                                    && style.color == Some(pane.theme.colors.text_muted.into())
                            })
                    })
        },
        |pane| {
            let left_epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, &comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_epoch={} split_epoch_after_first_draw={split_epoch_after_first_draw} fallback_split_highlights_hash={fallback_split_highlights_hash:?} split_cached={split_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                left_epoch,
            )
        },
    );

    // Verify both old and new sides have valid document-backed syntax sessions.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let split_styled = file_diff_split_cached_styled(
            pane,
            DiffTextRegion::SplitLeft,
            comment_line.as_str(),
        )
            .expect("background syntax completion should repopulate the split row cache");
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some(),
            "background parse should produce the left (old) prepared syntax document"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            "background parse should produce the right (new) prepared syntax document"
        );
        if let Some(initial_split_highlights_hash) = fallback_split_highlights_hash {
            assert!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft)
                    > split_epoch_after_first_draw,
                "background syntax completion should bump the left style cache epoch after the plain-text fallback draw"
            );
            assert_ne!(
                split_styled.highlights_hash, initial_split_highlights_hash,
                "background syntax should replace the plain-text split row styling"
            );
        }
        assert!(
            split_styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "split comment row should upgrade to comment highlighting after background parsing"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file-diff inline projection after background syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .is_some_and(|styled| {
                styled.text.as_ref() == comment_line
                    && styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    })
            })
        },
        |pane| {
            let inline_cached = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "inline_doc_left={:?} inline_doc_right={:?} inline_cached={inline_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );
}

#[gpui::test]
fn edited_large_file_diff_reparses_incrementally_in_background_after_timeout(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(64);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_edited_large_file_diff_background_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/edited_large_file_diff_bg.rs");
    let comment_line = "still inside block comment";
    let comment_inline_text = format!(" {comment_line}");
    let inserted_prefix = format!("/* start block comment\n{comment_line}\nend */\n");
    let line_count = 8_001usize;

    let mut old_lines = vec![
        "fn edited_demo() {".to_string(),
        "    let kept = 1;".to_string(),
        "}".to_string(),
    ];
    old_lines.extend((3..line_count).map(|ix| format!("let edited_bg_{ix}: usize = {ix};")));
    let old_text_v1 = old_lines.join("\n");
    let mut new_lines = old_lines.clone();
    *new_lines
        .last_mut()
        .expect("fixture should have a tail line") = format!(
        "let edited_bg_{}: usize = {};",
        line_count - 1,
        line_count * 2
    );
    let new_text_v1 = new_lines.join("\n");
    let old_text_v2 = format!("{inserted_prefix}{old_text_v1}");
    let new_text_v2 = format!("{inserted_prefix}{new_text_v1}");

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        &old_text_v1,
        &new_text_v1,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff initial syntax ready",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Rust)
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

    let (initial_left_version, initial_right_version) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .expect("initial left syntax document should be ready");
        let right_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            .expect("initial right syntax document should be ready");
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(left_document),
            Some(rows::PreparedDiffSyntaxParseMode::Full),
            "the first file-diff prepare should start from a full parse without a prior document seed"
        );
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(right_document),
            Some(rows::PreparedDiffSyntaxParseMode::Full),
            "the first file-diff prepare should start from a full parse without a prior document seed"
        );
        (
            rows::prepared_diff_syntax_source_version(left_document)
                .expect("initial left document should have a source version"),
            rows::prepared_diff_syntax_source_version(right_document)
                .expect("initial right document should have a source version"),
        )
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        2,
        &old_text_v2,
        &new_text_v2,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff cache rebuild for new revision",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == 2
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane
                    .file_diff_old_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str())
                && pane
                    .file_diff_new_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str())
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(comment_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(comment_line))
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} old_prefix={} new_prefix={} row_count={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_old_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str()),
                pane.file_diff_new_text
                    .as_ref()
                    .starts_with(inserted_prefix.as_str()),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff split comment row cached",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line).is_some()
        },
        |pane| {
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_epoch={} split_cached={split_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
            )
        },
    );

    let (split_epoch_after_first_draw, fallback_split_highlights_hash) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let styled = file_diff_split_cached_styled(
                pane,
                DiffTextRegion::SplitLeft,
                comment_line,
            )
            .expect("edited split comment row should be cached before background completion wait");
            assert_eq!(
                styled.text.as_ref(),
                comment_line,
                "expected the cached split row to match the edited multiline comment text"
            );
            if styled.highlights.is_empty() {
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    Some(styled.highlights_hash),
                )
            } else {
                assert!(
                    styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    }),
                    "if the background parse wins the race before the first observable split cache fill, the cached edited row should already be syntax highlighted"
                );
                (
                    pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                    None,
                )
            }
        });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff background incremental syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            let Some(left_document) =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            else {
                return false;
            };
            let Some(right_document) =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            else {
                return false;
            };
            let left_epoch = pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft);
            rows::prepared_diff_syntax_parse_mode(left_document)
                == Some(rows::PreparedDiffSyntaxParseMode::Incremental)
                && rows::prepared_diff_syntax_parse_mode(right_document)
                    == Some(rows::PreparedDiffSyntaxParseMode::Incremental)
                && rows::prepared_diff_syntax_source_version(left_document)
                    .is_some_and(|version| version > initial_left_version)
                && rows::prepared_diff_syntax_source_version(right_document)
                    .is_some_and(|version| version > initial_right_version)
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .is_some_and(|styled| {
                        let upgraded_from_fallback = fallback_split_highlights_hash
                            .map(|hash| {
                                left_epoch > split_epoch_after_first_draw
                                    && styled.highlights_hash != hash
                            })
                            .unwrap_or(true);
                        upgraded_from_fallback
                            && styled.highlights.iter().any(|(range, style)| {
                                range.start == 0
                                    && range.end == comment_line.len()
                                    && style.color == Some(pane.theme.colors.text_muted.into())
                            })
                    })
        },
        |pane| {
            let left_document =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_document =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            let split_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={left_document:?} right_doc={right_document:?} left_mode={:?} right_mode={:?} left_version={:?} right_version={:?} left_epoch={} split_epoch_after_first_draw={split_epoch_after_first_draw} fallback_split_highlights_hash={fallback_split_highlights_hash:?} split_cached={split_cached:?}",
                left_document.and_then(rows::prepared_diff_syntax_parse_mode),
                right_document.and_then(rows::prepared_diff_syntax_parse_mode),
                left_document.and_then(rows::prepared_diff_syntax_source_version),
                right_document.and_then(rows::prepared_diff_syntax_source_version),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .expect("background reparse should produce the edited left syntax document");
        let right_document = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
            .expect("background reparse should produce the edited right syntax document");
        let split_styled = file_diff_split_cached_styled(
            pane,
            DiffTextRegion::SplitLeft,
            comment_line,
        )
        .expect("background reparse should repopulate the edited split row cache");
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(left_document),
            Some(rows::PreparedDiffSyntaxParseMode::Incremental),
            "the edited left document should reuse the previous tree during background reparsing"
        );
        assert_eq!(
            rows::prepared_diff_syntax_parse_mode(right_document),
            Some(rows::PreparedDiffSyntaxParseMode::Incremental),
            "the edited right document should reuse the previous tree during background reparsing"
        );
        assert!(
            rows::prepared_diff_syntax_source_version(left_document)
                .is_some_and(|version| version > initial_left_version),
            "the edited left document should advance its source version after incremental reparsing"
        );
        assert!(
            rows::prepared_diff_syntax_source_version(right_document)
                .is_some_and(|version| version > initial_right_version),
            "the edited right document should advance its source version after incremental reparsing"
        );
        if let Some(initial_split_highlights_hash) = fallback_split_highlights_hash {
            assert!(
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft)
                    > split_epoch_after_first_draw,
                "background syntax completion should bump the edited left style cache epoch after the fallback draw"
            );
            assert_ne!(
                split_styled.highlights_hash, initial_split_highlights_hash,
                "background syntax should replace the fallback split row styling after the edited revision rebuild"
            );
        }
        assert!(
            split_styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "the edited split comment row should upgrade to comment highlighting after incremental background parsing"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited file-diff inline projection after incremental background syntax",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .is_some_and(|styled| {
                styled.text.as_ref() == comment_line
                    && styled.highlights.iter().any(|(range, style)| {
                        range.start == 0
                            && range.end == comment_line.len()
                            && style.color == Some(pane.theme.colors.text_muted.into())
                    })
            })
        },
        |pane| {
            let inline_cached = file_diff_inline_cached_styled(
                pane,
                gitcomet_core::domain::DiffLineKind::Context,
                &comment_inline_text,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_mode={:?} right_mode={:?} inline_cached={inline_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .and_then(rows::prepared_diff_syntax_parse_mode),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .and_then(rows::prepared_diff_syntax_parse_mode),
            )
        },
    );
}

#[gpui::test]
fn file_diff_background_left_syntax_upgrade_preserves_right_cached_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(65);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_one_sided_file_diff_background_syntax",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/one_sided_file_diff_bg.rs");
    let next_rev = 2u64;
    let rebuild_timeout = std::time::Duration::from_secs(30);

    let initial_old_text = "fn before_change() {}\n";
    let top_right_line = "fn stable_top() { let keep_top: usize = 1; }";
    let cached_right_line = "let stable_cached_right_90: usize = 90;";
    let mut new_lines = vec![top_right_line.to_string()];
    new_lines.extend((1..120).map(|ix| {
        if ix == 90 {
            cached_right_line.to_string()
        } else {
            format!("let stable_right_{ix}: usize = {ix};")
        }
    }));
    let new_text = new_lines.join("\n");

    let comment_line = "still inside block comment";
    let mut updated_old_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
    ];
    updated_old_lines.extend((3..12_001).map(|ix| {
        format!(
            "let one_sided_background_{ix}: Option<Result<Vec<usize>, usize>> = Some(Ok(vec![{ix}, {ix} + 1, {ix} + 2]));"
        )
    }));
    let updated_old_text = updated_old_lines.join("\n");

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        1,
        initial_old_text,
        &new_text,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "initial one-sided file-diff syntax ready",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let right_document = pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .expect("initial right syntax document should be ready before preseeding");
                let original_rev = pane.file_diff_cache_rev;
                pane.file_diff_cache_rev = next_rev;
                let next_right_key = pane
                    .file_diff_prepared_syntax_key(PreparedSyntaxViewMode::FileDiffSplitRight)
                    .expect(
                        "future right key should be available while the file-diff cache is built",
                    );
                pane.file_diff_cache_rev = original_rev;
                pane.prepared_syntax_documents
                    .insert(next_right_key, right_document);
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });
        });
    });

    seed_file_diff_state_with_rev(
        cx,
        &view,
        repo_id,
        &workdir,
        &path,
        next_rev,
        &updated_old_text,
        &new_text,
    );

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff rebuild (left pending, right ready)",
        rebuild_timeout,
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_rev == next_rev
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane
                    .file_diff_old_text
                    .as_ref()
                    .starts_with("/* start block comment")
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.old.as_deref() == Some(comment_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(top_right_line))
                && pane
                    .file_diff_cache_rows
                    .iter()
                    .any(|row| row.new.as_deref() == Some(cached_right_line))
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} cache_path={:?} left_doc={:?} right_doc={:?} rows={}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    let cached_right_row_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        file_diff_split_row_ix(pane, DiffTextRegion::SplitRight, cached_right_line)
            .expect("expected the cached right row to exist in the rebuilt split diff")
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.diff_scroll
                    .scroll_to_item_strict(cached_right_row_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff cached lower right row",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                .is_some()
        },
        |pane| {
            let cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} cached_right={cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_scroll
                    .scroll_to_item_strict(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff cached top right row",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                .is_some()
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .is_some()
        },
        |pane| {
            let top_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .map(styled_debug_info_with_styles);
            let lower_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} top_cached={top_cached:?} lower_cached={lower_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    let (
        left_epoch_before,
        right_epoch_before,
        top_right_hash,
        cached_right_hash,
        left_initial_hash,
        left_was_pending,
    ) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some(),
            "the preseeded right syntax document should stay ready"
        );
        let left_was_pending = pane
            .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
            .is_none();

        let top_cached =
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line).expect(
                "expected the top right row to be cached before left background completion",
            );
        let lower_cached = file_diff_split_cached_styled(
            pane,
            DiffTextRegion::SplitRight,
            cached_right_line,
        )
        .expect(
            "expected the offscreen right row to remain cached before left background completion",
        );
        let left_fallback =
            file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line).expect(
                "expected the pending left comment row to be cached before background completion",
            );
        assert!(
            !top_cached.highlights.is_empty(),
            "the preseeded top right row should already be syntax highlighted"
        );
        assert!(
            !lower_cached.highlights.is_empty(),
            "the preseeded offscreen right row should already be syntax highlighted"
        );

        (
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            top_cached.highlights_hash,
            lower_cached.highlights_hash,
            left_fallback.highlights_hash,
            left_was_pending,
        )
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "one-sided file-diff background left syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                .is_some()
                && pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight)
                    == right_epoch_before
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .is_some_and(|styled| styled.highlights_hash == top_right_hash)
                && file_diff_split_cached_styled(
                    pane,
                    DiffTextRegion::SplitRight,
                    cached_right_line,
                )
                .is_some_and(|styled| styled.highlights_hash == cached_right_hash)
                && file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .is_some_and(|styled| {
                        styled.highlights.iter().any(|(range, style)| {
                            range.start == 0
                                && range.end == comment_line.len()
                                && style.color == Some(pane.theme.colors.text_muted.into())
                        }) && (!left_was_pending
                            || pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft)
                                > left_epoch_before
                            || styled.highlights_hash != left_initial_hash)
                    })
        },
        |pane| {
            let top_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, top_right_line)
                    .map(styled_debug_info_with_styles);
            let lower_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitRight, cached_right_line)
                    .map(styled_debug_info_with_styles);
            let left_cached =
                file_diff_split_cached_styled(pane, DiffTextRegion::SplitLeft, comment_line)
                    .map(styled_debug_info_with_styles);
            format!(
                "left_doc={:?} right_doc={:?} left_epoch={} right_epoch={} top_cached={top_cached:?} lower_cached={lower_cached:?} left_cached={left_cached:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitLeft),
                pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let top_cached = file_diff_split_cached_styled(
            pane,
            DiffTextRegion::SplitRight,
            top_right_line,
        )
        .expect("top right row should remain cached after left background completion");
        let lower_cached = file_diff_split_cached_styled(
            pane,
            DiffTextRegion::SplitRight,
            cached_right_line,
        )
        .expect("offscreen right row should remain cached after left background completion");
        let left_cached = file_diff_split_cached_styled(
            pane,
            DiffTextRegion::SplitLeft,
            comment_line,
        )
        .expect("left comment row should be cached after background completion");

        assert_eq!(
            pane.file_diff_split_style_cache_epoch(DiffTextRegion::SplitRight),
            right_epoch_before,
            "left-only background syntax completion should not bump the right-side cache epoch"
        );
        assert_eq!(
            top_cached.highlights_hash, top_right_hash,
            "the visible right row should keep its cached styling when only the left side upgrades"
        );
        assert_eq!(
            lower_cached.highlights_hash, cached_right_hash,
            "the offscreen right row should survive left-only syntax completion without a cache clear"
        );
        if left_was_pending {
            assert_ne!(
                left_cached.highlights_hash, left_initial_hash,
                "the left comment row should replace its pending fallback styling after the background parse"
            );
        }
        assert!(
            left_cached.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "the left comment row should be comment-highlighted after the background parse completes"
        );
    });
}
