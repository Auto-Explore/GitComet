use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

#[gpui::test]
fn windows_and_search_snapshots_keep_their_own_tab_width(cx: &mut gpui::TestAppContext) {
    use crate::view::{GitCometView, test_support::TestBackend};
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_state::store::AppStore;
    let panes = [2, 8].map(|width| {
        let (store, events) = AppStore::new_test(Arc::new(TestBackend));
        let (view, window_cx) =
            cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
        window_cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.clone();
            let snapshot = pane.update(app, |pane, cx| {
                pane.default_tab_size = width;
                pane.sync_display_tab_width(cx);
                pane.diff_view = DiffViewMode::Inline;
                pane.diff_word_wrap = false;
                pane.diff_cache = Arc::from([AnnotatedDiffLine {
                    kind: DiffLineKind::Add,
                    text: "+\tneedle".into(),
                    old_line: None,
                    new_line: Some(1),
                }]);
                pane.ensure_diff_visible_indices();
                pane.capture_search_document()
            });
            (pane, snapshot, usize::from(width))
        })
    });

    cx.update(|app| {
        // Both windows have rendered. Copying from either must still use
        // the offsets belonging to that window's layout.
        for (pane, _, width) in &panes {
            pane.update(app, |pane, cx| {
                let expected = format!("+{}needle", " ".repeat(*width));
                assert_eq!(
                    pane.diff_text_full_line_for_region(0, DiffTextRegion::Inline)
                        .as_ref(),
                    expected
                );
                assert_eq!(
                    pane.diff_text_line_len_for_region(0, DiffTextRegion::Inline),
                    expected.len()
                );
                let anchor = DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: width + 1,
                };
                pane.diff_text_anchor = Some(anchor);
                pane.diff_text_head = Some(DiffTextPos {
                    offset: width + 1 + "needle".len(),
                    ..anchor
                });
                pane.copy_selected_diff_text_to_clipboard(cx);
                assert_eq!(crate::clipboard::read_text(cx).as_deref(), Some("needle"));
            });
        }

        let keys = panes
            .each_ref()
            .map(|(pane, _, _)| pane.read(app).diff_search_document_key());
        panes[0].0.update(app, |pane, cx| {
            pane.default_tab_size = 3;
            pane.sync_display_tab_width(cx);
            assert!(keys[0] != pane.diff_search_document_key());
        });
        assert!(keys[1] == panes[1].0.read(app).diff_search_document_key());
    });

    // Workers must retain the captured width even after a pane changes it,
    // and must not depend on the thread on which matching runs.
    for (_, snapshot, width) in panes {
        std::thread::spawn(move || {
            let query = format!("+{}needle", " ".repeat(width));
            assert_eq!(
                snapshot
                    .search(
                        &query,
                        DiffSearchOptions::default(),
                        CancellationToken::new()
                    )
                    .matches,
                [0]
            );
            assert!(
                snapshot
                    .search(
                        "+   needle",
                        DiffSearchOptions::default(),
                        CancellationToken::new()
                    )
                    .matches
                    .is_empty()
            );
        })
        .join()
        .unwrap();
    }
}

#[gpui::test]
fn search_snapshot_shares_rows_and_resize_only_invalidates_changed_wrap_plans(
    cx: &mut gpui::TestAppContext,
) {
    use crate::view::{GitCometView, test_support::TestBackend};
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_state::store::AppStore;
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let pane = view.read(app).diff_renderer.clone();
        pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_word_wrap = false;
            pane.diff_cache = ["+needle alpha", "+other"]
                .map(|text| AnnotatedDiffLine {
                    kind: DiffLineKind::Add,
                    text: text.into(),
                    old_line: None,
                    new_line: Some(1),
                })
                .into();
            pane.ensure_diff_visible_indices();
            let key = pane.diff_search_document_key();
            pane.last_window_size.width += gpui::px(137.0);
            pane.last_window_size.height += gpui::px(83.0);
            assert!(
                key == pane.diff_search_document_key(),
                "unwrapped resizing does not change searchable rows"
            );
            let document = pane.capture_search_document();
            assert_eq!(
                Arc::strong_count(&pane.diff_cache),
                2,
                "snapshot must share eager rows"
            );
            assert_eq!(
                Arc::strong_count(&pane.diff_visible_indices),
                2,
                "snapshot must share the projection"
            );
            assert_eq!(
                document
                    .search(
                        "needle",
                        DiffSearchOptions::default(),
                        CancellationToken::new()
                    )
                    .matches,
                [0]
            );

            pane.diff_word_wrap = true;
            pane.ensure_diff_wrap_visible_rows(window, cx);
            let wrapped_key = pane.diff_search_document_key();
            let mut plan = pane.diff_wrap_visible_cache_key.unwrap();
            plan.inline_columns += 1;
            pane.diff_wrap_visible_cache_key = Some(plan);
            assert!(
                wrapped_key != pane.diff_search_document_key(),
                "changed wrap plans invalidate visual row matches"
            );

            pane.diff_cache = Arc::from([]);
            pane.diff_visible_indices = Arc::from([]);
            assert_eq!(
                document
                    .search(
                        "needle",
                        DiffSearchOptions::default(),
                        CancellationToken::new()
                    )
                    .matches,
                [0],
                "a reload must leave the worker's snapshot intact"
            );
        });
    });
}

// Reading rows through the paged provider cached every page it built, so
// one search kept the whole patch materialized. Each row was also copied
// twice by an unconditional tab expansion.
#[gpui::test]
fn inline_patch_search_reads_diff_lines_without_materializing_pages(cx: &mut gpui::TestAppContext) {
    use crate::view::panes::diff::diff_cache::PagedPatchDiffRows;
    use crate::view::{GitCometView, test_support::TestBackend};
    use gitcomet_core::domain::{Diff, DiffArea, DiffTarget};
    use gitcomet_state::store::AppStore;
    let mut text = String::from("diff --git a/a.txt b/a.txt\n@@ -1,0 +1,3000 @@\n");
    for ix in 0..3000 {
        let suffix = if ix % 1000 == 7 { " needle" } else { "" };
        text.push_str(&format!("+line {ix}{suffix}\n"));
    }
    text.push_str("+\tneedle\n");
    let target = DiffTarget::working_tree("a.txt".into(), DiffArea::Unstaged);
    let diff = Arc::new(Diff::from_unified(target, &text));
    let provider = Arc::new(PagedPatchDiffRows::new(diff.clone(), 256));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|_, app| {
        let pane = view.read(app).diff_renderer.clone();
        pane.update(app, |pane, _| {
            pane.diff_view = DiffViewMode::Inline;
            pane.diff_word_wrap = false;
            pane.diff_row_provider = Some(provider.clone());
            pane.diff_visible_indices = (0..diff.lines.len()).collect();
            let document = pane.capture_search_document();
            let matches = document
                .search(
                    "needle",
                    DiffSearchOptions::default(),
                    CancellationToken::new(),
                )
                .matches;
            let tab_row = diff.lines.len() - 1;
            assert_eq!(matches, [9, 1009, 2009, tab_row]);
            assert_eq!(provider.materialized_row_count(), 0, "search built pages");

            let DocumentSource::Rows(rows) = &document.0 else {
                panic!("patch search uses row text");
            };
            let row = (rows.text)(matches[0], 0).expect("row text");
            assert_eq!(
                row.as_ref().as_ptr(),
                diff.lines[matches[0]].text.as_ref().as_ptr(),
                "a row without tabs is shared, not copied"
            );
            let row = (rows.text)(tab_row, 0).expect("row text");
            assert_eq!(
                row.as_ref(),
                "+    needle",
                "the diff sign does not advance the content's tab stop"
            );
        });
    });
}

#[test]
fn background_search_reuses_rows_and_preserves_query_semantics() {
    let texts = ["needle alpha", "Needle beta", "gamma", "猫 alpha", "tail"];
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let document = SearchDocument(DocumentSource::Rows(RowDocument {
        len: texts.len(),
        columns: 1,
        text: Box::new(move |row, _| {
            counter.fetch_add(1, Ordering::Relaxed);
            Some(texts[row].into())
        }),
        wrapped: Arc::from([]),
        streamed: false,
        collapsed: false,
        previous: Mutex::new(VecDeque::new()),
    }));
    let search = |query, options| {
        document
            .search(query, options, CancellationToken::new())
            .matches
    };
    assert_eq!(search("needle", DiffSearchOptions::default()), vec![0, 1]);
    assert_eq!(
        reads.load(Ordering::Relaxed),
        texts.len(),
        "the first query scans once without building an eager index"
    );
    reads.store(0, Ordering::Relaxed);
    assert_eq!(search("needle beta", DiffSearchOptions::default()), vec![1]);
    assert_eq!(
        reads.load(Ordering::Relaxed),
        2,
        "query refinement only checks previous matches"
    );
    reads.store(0, Ordering::Relaxed);
    assert_eq!(search("needle", DiffSearchOptions::default()), vec![0, 1]);
    assert_eq!(
        reads.load(Ordering::Relaxed),
        2,
        "backspacing reuses the broader cached query, preserving its matches"
    );
    for (query, options) in [
        ("alpha\nNeedle", DiffSearchOptions::default()),
        ("猫", DiffSearchOptions::default()),
        (
            "^Needle",
            DiffSearchOptions {
                regex: true,
                match_case: true,
                ..Default::default()
            },
        ),
        (
            "alpha",
            DiffSearchOptions {
                whole_word: true,
                ..Default::default()
            },
        ),
    ] {
        let mut expected = Vec::new();
        collect_stream_match_visible_rows(
            texts.iter().enumerate(),
            &DiffSearchMatcher::new(query, options),
            &mut expected,
        );
        assert_eq!(search(query, options), expected);
    }
}

// `slice_text` returns "" for a range that splits a character, which made
// both scanners skip the whole 32 KiB chunk the match was in.
#[test]
fn long_line_match_survives_a_chunk_ending_inside_a_character() {
    let mut line = "x".repeat(100);
    line.push_str("needle");
    line.push_str(&"x".repeat(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES - line.len() - 1));
    line.push('é');
    line.push_str(&"y".repeat(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES));
    assert!(!line.is_char_boundary(FILE_PREVIEW_SEARCH_SCAN_CHUNK_BYTES));
    let text: FileDiffLineText = line.into();

    let matcher = DiffSearchMatcher::new("NEEDLE", DiffSearchOptions::default());
    assert!(RowDocument::matches_text(&matcher, &text));
    let needle = AsciiCaseInsensitiveNeedle::new("NEEDLE").unwrap();
    assert!(resolved_output_line_ix_matches_query(&text, needle));
    // A match in the chunk after the split character is still found.
    let matcher = DiffSearchMatcher::new("xéy", DiffSearchOptions::default());
    assert!(RowDocument::matches_text(&matcher, &text));
}

/// A split file diff reads both sides from disk. Searching once opened,
/// seeked and read the file per row per chunk, twice with the tab check:
/// ~400k system calls and ~1.3 s for the first query on 100k rows.
#[test]
fn searching_source_backed_rows_opens_each_side_once() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sides = ["old.txt", "new.txt"].map(|name| {
        let path = Arc::new(dir.path().join(name));
        let text: String = (0..2_000)
            .map(|row| {
                let tab = if row % 7 == 0 { "\t" } else { "" };
                let needle = if row % 100 == 0 { "needle" } else { "plain" };
                format!("row {row:04}:{tab} {needle} {name}\n")
            })
            .collect();
        std::fs::write(&*path, &text).expect("write side");
        let mut start = 0;
        let lines: Vec<_> = text
            .split_inclusive('\n')
            .map(|line| {
                let range = start..start + line.len() - 1;
                start += line.len();
                (range, line.contains('\t'))
            })
            .collect();
        (path, lines)
    });
    let rows = RowDocument {
        len: 2_000,
        columns: 2,
        text: Box::new(move |ix, column| {
            let (path, lines) = &sides[column];
            let (range, has_tabs) = lines.get(ix)?.clone();
            Some(FileDiffLineText::file_slice(
                Arc::clone(path),
                range,
                true,
                has_tabs,
            ))
        }),
        wrapped: Arc::from([]),
        streamed: false,
        collapsed: false,
        previous: Mutex::new(VecDeque::new()),
    };
    let document = SearchDocument(DocumentSource::Rows(rows));

    let _ = gitcomet_core::file_diff::take_file_slice_opens_for_tests();
    let found = document.search(
        "needle",
        DiffSearchOptions::default(),
        CancellationToken::new(),
    );
    assert_eq!(found.matches, (0..2_000).step_by(100).collect::<Vec<_>>());
    assert_eq!(
        gitcomet_core::file_diff::take_file_slice_opens_for_tests(),
        2
    );

    // Refining reuses the cached candidates and still reads each side once.
    let _ = gitcomet_core::file_diff::take_file_slice_bytes_read_for_tests();
    let refined = document.search(
        "needle old",
        DiffSearchOptions::default(),
        CancellationToken::new(),
    );
    assert_eq!(refined.matches, found.matches);
    assert!(gitcomet_core::file_diff::take_file_slice_opens_for_tests() <= 2);
    // It reads the 20 candidate rows, not whole sides (~48 KB each).
    let read = gitcomet_core::file_diff::take_file_slice_bytes_read_for_tests();
    assert!(read < 4_096, "refinement read {read} bytes");

    // Searches that scan column by column batch the same way.
    let matched_case = document.search(
        "needle",
        DiffSearchOptions {
            match_case: true,
            ..Default::default()
        },
        CancellationToken::new(),
    );
    assert_eq!(matched_case.matches, found.matches);
    assert_eq!(
        gitcomet_core::file_diff::take_file_slice_opens_for_tests(),
        2
    );
}

/// A collapsed view's document holds a few rows of large sides; both scans
/// read those rows rather than loading the sides whole.
#[test]
fn collapsed_background_searches_read_only_their_rows() {
    let dir = tempfile::tempdir().expect("tempdir");
    let sides = ["old.txt", "new.txt"].map(|name| {
        let path = Arc::new(dir.path().join(name));
        let text: String = (0..20_000)
            .map(|row| {
                let needle = if row == 10_002 { "needle" } else { "plain" };
                format!("row {row:05}: {needle} {name}\n")
            })
            .collect();
        std::fs::write(&*path, &text).expect("write side");
        let mut start = 0;
        let lines: Vec<_> = text
            .split_inclusive('\n')
            .map(|line| {
                let range = start..start + line.len() - 1;
                start += line.len();
                range
            })
            .collect();
        (path, lines)
    });
    let document = SearchDocument(DocumentSource::Rows(RowDocument {
        len: 5,
        columns: 2,
        text: Box::new(move |ix, column| {
            let (path, lines) = &sides[column];
            let range = lines.get(10_000 + ix)?.clone();
            Some(FileDiffLineText::file_slice(
                Arc::clone(path),
                range,
                true,
                false,
            ))
        }),
        wrapped: Arc::from([]),
        streamed: false,
        collapsed: true,
        previous: Mutex::new(VecDeque::new()),
    }));

    for match_case in [false, true] {
        let _ = gitcomet_core::file_diff::take_file_slice_bytes_read_for_tests();
        let found = document.search(
            "needle",
            DiffSearchOptions {
                match_case,
                ..Default::default()
            },
            CancellationToken::new(),
        );
        assert_eq!(found.matches, vec![2], "match_case={match_case}");
        let read = gitcomet_core::file_diff::take_file_slice_bytes_read_for_tests();
        // Ten ~25-byte rows, not two ~500 KB sides.
        assert!(read < 4_096, "match_case={match_case}: read {read} bytes");
    }
}

#[test]
fn background_search_cancellation_does_not_cache_partial_results() {
    let token = CancellationToken::new();
    let cancel = token.clone();
    let reads = Arc::new(AtomicUsize::new(0));
    let counter = reads.clone();
    let rows = RowDocument {
        len: 100_000,
        columns: 1,
        text: Box::new(move |_, _| {
            if counter.fetch_add(1, Ordering::Relaxed) == 2 {
                cancel.cancel();
            }
            Some("needle".into())
        }),
        wrapped: Arc::from([]),
        streamed: false,
        collapsed: false,
        previous: Mutex::new(VecDeque::new()),
    };
    let mut matcher = DiffSearchMatcher::new("needle", DiffSearchOptions::default());
    matcher.set_cancellation(token);
    assert!(rows.search(&matcher).is_empty());
    assert_eq!(reads.load(Ordering::Relaxed), 3);
    assert!(rows.previous.lock().unwrap().is_empty());
}
