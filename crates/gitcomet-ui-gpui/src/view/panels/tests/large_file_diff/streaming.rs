use super::*;

#[gpui::test]
fn oversized_json_file_diff_uses_visible_line_fallback_without_prepared_syntax_documents(
    cx: &mut gpui::TestAppContext,
) {
    const OBJECT_COUNT: usize = 512;
    const PAYLOAD_BYTES: usize = 16 * 1024;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

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
            this.diff_renderer.update(cx, |pane, cx| {
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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

/// Ceiling for the prepared-document path; the large-file fixtures build
/// payloads past this so the streamed path is what runs.
const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;

#[gpui::test]
fn minified_json_file_diff_streams_visible_slices_and_inline_search(cx: &mut gpui::TestAppContext) {
    const PAYLOAD_BYTES: usize = PREPARED_DOCUMENT_MAX_BYTES + 256 * 1024;

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
            this.diff_renderer.update(cx, |pane, cx| {
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
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "streamed-inline-search".into();
                pane.diff_search_recompute_matches();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
fn minified_json_file_diff_partial_copy_uses_streamed_inline_row_source(
    cx: &mut gpui::TestAppContext,
) {
    const PAYLOAD_BYTES: usize = 256 * 1024;

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

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

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: start,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: end,
                });
                pane.diff_text_selection_owner.adopt(window, cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
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

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

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
            this.diff_renderer.update(cx, |pane, _cx| {
                pane.diff_view = DiffViewMode::Inline;
            });
        });
    });

    cx.update(|window, app| {
        let diff_renderer = view.read(app).diff_renderer.clone();
        diff_renderer.update(app, |pane, cx| {
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

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

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

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.clear_diff_text_style_caches();
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::SplitLeft,
                    offset: start,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::SplitLeft,
                    offset: end,
                });
                pane.diff_text_selection_owner.adopt(window, cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
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
