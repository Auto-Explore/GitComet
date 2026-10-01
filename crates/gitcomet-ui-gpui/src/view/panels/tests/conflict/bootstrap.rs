//! Bootstrap and streaming: large-file indexes, background syntax, outlines.

use super::*;

#[gpui::test]
fn large_conflict_bootstrap_trace_records_stage_counts(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::mergetool_trace::{self, MergetoolTraceStage};

    fn trace_line_count(text: &str) -> usize {
        if text.is_empty() {
            0
        } else {
            text.as_bytes()
                .iter()
                .filter(|&&byte| byte == b'\n')
                .count()
                + 1
        }
    }

    let _trace = mergetool_trace::capture();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(161);
    let fixture = SyntheticLargeConflictFixture::new(
        "large_conflict_bootstrap_trace",
        "fixtures/large_conflict_trace.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 100,
        1,
    );
    fixture.write();

    let expected_resolved = crate::view::conflict_resolver::generate_resolved_text(
        crate::view::conflict_resolver::parse_conflict_markers(&fixture.current_text).as_slice(),
    );
    let expected_resolved_line_count = trace_line_count(&expected_resolved);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large conflict bootstrap trace initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_rows={} visible_rows={} resolved_path={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
                pane.conflict_resolver.two_way_split_visible_len(),
                pane.conflict_resolved_preview_path,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
            });
        });
    });

    let trace = mergetool_trace::snapshot();
    let path_events: Vec<_> = trace
        .events
        .iter()
        .filter(|event| event.path.as_deref() == Some(fixture.file_rel.as_path()))
        .collect();
    assert!(
        !path_events.is_empty(),
        "expected mergetool trace events for the focused large conflict fixture"
    );

    // Giant mode skips BuildInlineRows since inline is not supported.
    let is_streamed = path_events.iter().any(|event| {
        event.rendering_mode
            == Some(gitcomet_core::mergetool_trace::MergetoolTraceRenderingMode::StreamedLargeFile)
    });
    for stage in [
        MergetoolTraceStage::ParseConflictMarkers,
        MergetoolTraceStage::GenerateResolvedText,
        MergetoolTraceStage::SideBySideRows,
        MergetoolTraceStage::BuildThreeWayConflictMaps,
        MergetoolTraceStage::ComputeThreeWayWordHighlights,
        MergetoolTraceStage::ComputeTwoWayWordHighlights,
        MergetoolTraceStage::ResolvedOutlineRecompute,
        MergetoolTraceStage::ConflictResolverBootstrapTotal,
    ] {
        assert!(
            path_events.iter().any(|event| event.stage == stage),
            "missing {stage:?} trace event for large conflict bootstrap"
        );
    }
    if !is_streamed {
        assert!(
            path_events
                .iter()
                .any(|event| event.stage == MergetoolTraceStage::ConflictResolverInputSetText),
            "missing ConflictResolverInputSetText trace event for non-streamed bootstrap"
        );
    }
    if !is_streamed {
        assert!(
            path_events
                .iter()
                .any(|event| event.stage == MergetoolTraceStage::BuildInlineRows),
            "missing BuildInlineRows trace event for non-streamed bootstrap"
        );
    }

    let bootstrap_event = path_events
        .iter()
        .find(|event| event.stage == MergetoolTraceStage::ConflictResolverBootstrapTotal)
        .copied()
        .expect("missing bootstrap-total trace event");
    // SyntheticLargeConflictFixture ensures base/ours/theirs all have fixture_line_count lines.
    assert_eq!(bootstrap_event.base.lines, Some(fixture.fixture_line_count));
    assert_eq!(bootstrap_event.ours.lines, Some(fixture.fixture_line_count));
    assert_eq!(
        bootstrap_event.theirs.lines,
        Some(fixture.fixture_line_count)
    );
    assert_eq!(
        bootstrap_event.conflict_block_count,
        Some(fixture.conflict_block_count)
    );
    assert_eq!(
        bootstrap_event.rendering_mode,
        Some(gitcomet_core::mergetool_trace::MergetoolTraceRenderingMode::StreamedLargeFile),
        "large fixture bootstrap should opt into the explicit large-file rendering mode",
    );
    assert_eq!(
        bootstrap_event.whole_block_diff_ran,
        Some(false),
        "large fixture bootstrap should keep whole-block two-way diffs disabled",
    );
    assert_eq!(
        bootstrap_event.full_output_generated,
        Some(false),
        "streamed bootstrap should keep the resolved output virtual until an explicit edit or save path needs the full text",
    );
    assert_eq!(
        bootstrap_event.full_syntax_parse_requested,
        Some(true),
        "large fixture bootstrap should still request prepared syntax for streamed conflict inputs",
    );
    // In giant mode the diff_row_count is the paged index total (large);
    // in eager mode it stays bounded by conflict block size + context.
    let diff_row_count = bootstrap_event.diff_row_count.unwrap_or_default();
    if is_streamed {
        assert!(
            diff_row_count > 0,
            "streamed mode should still report a non-zero diff row count, got {diff_row_count}",
        );
        let inline_row_count = bootstrap_event.inline_row_count.unwrap_or_default();
        assert_eq!(
            inline_row_count, 0,
            "streamed mode should not build inline rows, got {inline_row_count}",
        );
    } else {
        let max_rows_per_block =
            (crate::view::conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES * 2) + 2;
        assert!(
            diff_row_count > 0 && diff_row_count <= max_rows_per_block,
            "block-local diff should stay bounded by one conflict block plus context, got {diff_row_count}"
        );
        let inline_row_count = bootstrap_event.inline_row_count.unwrap_or_default();
        assert!(
            inline_row_count > 0 && inline_row_count <= max_rows_per_block + 1,
            "inline rows should stay bounded by the block-local diff rows, got {inline_row_count}"
        );
    }
    assert_eq!(
        bootstrap_event.resolved_output_line_count,
        Some(expected_resolved_line_count)
    );

    let outline_event = path_events
        .iter()
        .rev()
        .find(|event| event.stage == MergetoolTraceStage::ResolvedOutlineRecompute)
        .copied()
        .expect("missing resolved-outline trace event");
    assert_eq!(
        outline_event.resolved_output_line_count,
        Some(expected_resolved_line_count)
    );
    assert_eq!(
        outline_event.conflict_block_count,
        Some(fixture.conflict_block_count)
    );

    fixture.cleanup();
}

#[gpui::test]
fn focused_mergetool_bootstrap_reuses_shared_text_arcs(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(162);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_shared_conflict_arcs",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/shared_conflict_arcs.html");
    let abs_path = workdir.join(&file_rel);

    // `SharedString` is backed by `SmolStr`, which stores strings up to 23 bytes
    // inline (copied, not `Arc`-shared). Each fixture must exceed that inline
    // capacity so the zero-copy `Arc` path is actually exercised below.
    let base_text: Arc<str> = "<p>base content paragraph</p>\n".into();
    let ours_text: Arc<str> = "<p>ours content paragraph</p>\n".into();
    let theirs_text: Arc<str> = "<p>theirs content paragraph</p>\n".into();
    let current_text: Arc<str> =
        "<<<<<<< ours\n<p>ours content paragraph</p>\n=======\n<p>theirs content paragraph</p>\n>>>>>>> theirs\n".into();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("shared conflict fixture parent"))
        .expect("create shared conflict fixture dir");
    std::fs::write(&abs_path, current_text.as_bytes()).expect("write shared conflict fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            // Must set conflict_file manually here: this test checks Arc<str> pointer
            // identity, which requires passing Arc<str> directly instead of converting
            // to String via set_test_conflict_file().
            repo.conflict_state.conflict_file_path = Some(file_rel.clone());
            repo.conflict_state.conflict_file =
                gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
                    path: file_rel.clone().into(),
                    base_bytes: None,
                    ours_bytes: None,
                    theirs_bytes: None,
                    current_bytes: None,
                    base: Some(base_text.clone()),
                    ours: Some(ours_text.clone()),
                    theirs: Some(theirs_text.clone()),
                    current: Some(current_text.clone()),
                }));
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_shared_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.clone()),
                ConflictPayload::Text(ours_text.clone()),
                ConflictPayload::Text(theirs_text.clone()),
                current_text.clone(),
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "shared conflict arc bootstrap initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.current.as_deref() == Some(current_text.as_ref())
                && !pane
                    .conflict_resolver
                    .three_way_text
                    .base
                    .as_ref()
                    .is_empty()
        },
        |pane| {
            format!(
                "path={:?} current={} base_len={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.current.is_some(),
                pane.conflict_resolver.three_way_text.base.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let base_arc: Arc<str> = pane.conflict_resolver.three_way_text.base.clone().into();
                let ours_arc: Arc<str> = pane.conflict_resolver.three_way_text.ours.clone().into();
                let theirs_arc: Arc<str> =
                    pane.conflict_resolver.three_way_text.theirs.clone().into();
                let current_arc = pane
                    .conflict_resolver
                    .current
                    .as_ref()
                    .expect("current text should be cached")
                    .clone();

                assert!(
                    Arc::ptr_eq(&base_text, &base_arc),
                    "base text should be shared into SharedString without a new allocation",
                );
                assert!(
                    Arc::ptr_eq(&ours_text, &ours_arc),
                    "ours text should be shared into SharedString without a new allocation",
                );
                assert!(
                    Arc::ptr_eq(&theirs_text, &theirs_arc),
                    "theirs text should be shared into SharedString without a new allocation",
                );
                assert!(
                    Arc::ptr_eq(&current_text, &current_arc),
                    "current text should stay Arc-shared in resolver state",
                );
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup shared conflict fixture");
}

fn assert_streamed_whole_file_three_way_state(pane: &MainPaneView, line_count: usize) {
    assert_eq!(
        pane.conflict_resolver.rendering_mode(),
        crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile,
        "large whole-file conflicts should select the explicit large-file rendering mode",
    );
    assert_eq!(
        pane.conflict_resolver.three_way_len, line_count,
        "three-way mode should still preserve the full document line count",
    );
    assert_eq!(
        pane.conflict_resolver.three_way_visible_len(),
        line_count,
        "large whole-file three-way mode should expose every visible line",
    );
    assert!(
        pane.conflict_resolver.has_three_way_visible_state_ready(),
        "streamed large-file mode should rebuild the visible three-way projection",
    );
    assert!(
        !pane
            .conflict_resolver
            .three_way_conflict_ranges
            .ours
            .is_empty(),
        "streamed large-file mode should keep conflict ranges for three-way lookups",
    );

    let mid_visible_ix = line_count / 2;
    assert_eq!(
        pane.conflict_resolver
            .three_way_visible_item(mid_visible_ix),
        Some(crate::view::conflict_resolver::ThreeWayVisibleItem::Line(
            mid_visible_ix
        )),
        "deep rows in streamed large-file mode should resolve to real lines",
    );
    assert!(
        pane.conflict_resolver
            .three_way_word_highlights
            .base
            .is_empty()
            && pane
                .conflict_resolver
                .three_way_word_highlights
                .ours
                .is_empty()
            && pane
                .conflict_resolver
                .three_way_word_highlights
                .theirs
                .is_empty(),
        "giant whole-file three-way blocks should skip eager word highlights",
    );
}

#[gpui::test]
fn whole_file_conflict_switch_to_three_way_stays_fully_reviewable(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(171);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_three_way_switch",
        "fixtures/whole_file_conflict_switch.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 100,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file conflict initialized for three-way switch",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                assert_eq!(
                    pane.conflict_resolver.view_mode,
                    ConflictResolverViewMode::TwoWayDiff,
                    "fixture should be in two-way mode before switching back to three-way",
                );
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                assert_eq!(
                    pane.conflict_resolver.view_mode,
                    ConflictResolverViewMode::ThreeWay,
                    "switching a large whole-file conflict into three-way mode should succeed",
                );
                assert_streamed_whole_file_three_way_state(pane, fixture.line_count);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    fixture.cleanup();
}

#[gpui::test]
fn whole_file_conflict_streamed_three_way_syntax_survives_view_mode_switch(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(172);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_three_way_streamed_syntax",
        "fixtures/whole_file_conflict_streamed_syntax.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 100,
    );
    let ours_body_line = r#"<body class="whole-file-ours">"#;

    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file streamed syntax fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                pane.conflict_resolver_scroll_all_columns(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_three_way_segments_cache
            .get(&(2, ThreeWayColumn::Ours))
            .expect("three-way draw should cache the visible streamed HTML body row");
        assert_eq!(
            styled.text.as_ref(),
            ours_body_line,
            "expected the streamed three-way cache to contain the visible ours HTML body row",
        );
        assert!(
            !styled.highlights.is_empty(),
            "streamed three-way rows above the old 20k line gate should still be syntax highlighted; got {:?}",
            styled_debug_info_with_styles(styled),
        );
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file streamed three-way background syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_three_way_prepared_syntax_documents
                .base
                .is_some()
                && pane
                    .conflict_three_way_prepared_syntax_documents
                    .ours
                    .is_some()
                && pane
                    .conflict_three_way_prepared_syntax_documents
                    .theirs
                    .is_some()
        },
        |pane| {
            format!(
                "base={:?} ours={:?} theirs={:?} inflight={:?}",
                pane.conflict_three_way_prepared_syntax_documents.base,
                pane.conflict_three_way_prepared_syntax_documents.ours,
                pane.conflict_three_way_prepared_syntax_documents.theirs,
                pane.conflict_three_way_syntax_inflight,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.conflict_resolver_scroll_all_columns(0, gpui::ScrollStrategy::Top);
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
        "streamed two-way HTML row cache after three-way switch",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            conflict_split_cached_styled(
                pane,
                crate::view::conflict_resolver::ConflictPickSide::Ours,
                ours_body_line,
            )
            .is_some_and(|styled| !styled.highlights.is_empty())
        },
        |pane| {
            let split_cached = conflict_split_cached_styled(
                pane,
                crate::view::conflict_resolver::ConflictPickSide::Ours,
                ours_body_line,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "split_cached={split_cached:?} split_cache_len={} three_way_cache_len={}",
                pane.conflict_diff_segments_cache_split.len(),
                pane.conflict_three_way_segments_cache.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                pane.conflict_resolver_scroll_all_columns(0, gpui::ScrollStrategy::Top);
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
        "streamed three-way HTML row cache after toggling back",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_three_way_segments_cache
                .get(&(2, ThreeWayColumn::Ours))
                .is_some_and(|styled| !styled.highlights.is_empty())
        },
        |pane| {
            let three_way_cached = pane
                .conflict_three_way_segments_cache
                .get(&(2, ThreeWayColumn::Ours))
                .map(styled_debug_info_with_styles);
            format!(
                "three_way_cached={three_way_cached:?} split_cache_len={} three_way_cache_len={}",
                pane.conflict_diff_segments_cache_split.len(),
                pane.conflict_three_way_segments_cache.len(),
            )
        },
    );

    fixture.cleanup();
}

#[gpui::test]
fn three_way_view_survives_incomplete_line_syntax_fragments(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(173);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_three_way_incomplete_fragments",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/three_way_incomplete_fragments.ts");
    let abs_path = workdir.join(&file_rel);

    let shared_prefix_line = "const element = document.querySelector(";
    let base_line = r#"  ".base""#;
    let ours_line = r#"  ".ours""#;
    let theirs_line = r#"  ".theirs""#;

    let base_text = [
        shared_prefix_line,
        base_line,
        ");",
        "type Example<T extends Record<string,",
        "  number>> = HTMLElement;",
    ]
    .join("\n");
    let ours_text = [
        shared_prefix_line,
        ours_line,
        ");",
        "type Example<T extends Record<string,",
        "  number>> = HTMLElement;",
    ]
    .join("\n");
    let theirs_text = [
        shared_prefix_line,
        theirs_line,
        ");",
        "type Example<T extends Record<string,",
        "  number>> = HTMLElement;",
    ]
    .join("\n");
    let current_text = [
        shared_prefix_line,
        "<<<<<<< ours",
        ours_line,
        "=======",
        theirs_line,
        ">>>>>>> theirs",
        ");",
        "type Example<T extends Record<string,",
        "  number>> = HTMLElement;",
    ]
    .join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create three-way fragment fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write three-way fragment fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.clone(),
                base_text.clone(),
                ours_text.clone(),
                theirs_text.clone(),
                current_text.clone(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "three-way incomplete-fragment fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                pane.conflict_resolver_scroll_all_columns(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_three_way_segments_cache
            .get(&(0, ThreeWayColumn::Base))
            .expect("three-way draw should cache the visible incomplete base line");
        assert_eq!(
            styled.text.as_ref(),
            shared_prefix_line,
            "expected the cached base line to preserve the incomplete source fragment"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup three-way fragment fixture");
}

/// Verifies huge conflicts stay on the streamed split path and avoid
/// bootstrap diff/highlight work.
#[gpui::test]
fn large_conflict_bootstrap_stays_streamed_for_huge_files(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(162);
    let fixture = SyntheticLargeConflictFixture::new(
        "large_conflict_block_local_sparse",
        "fixtures/huge_conflict.html",
        55_001,
        1,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    // Wait for the conflict resolver to be populated with the streamed split
    // index used for giant files.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large conflict streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_rows={} split_row_index={} three_way_len={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.three_way_len,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let index = pane
                    .conflict_resolver
                    .split_row_index()
                    .expect("huge conflict should stay on streamed split index");
                assert!(
                    pane
                        .conflict_resolver
                        .three_way_word_highlights
                        .ours
                        .is_empty(),
                    "streamed huge-file bootstrap should skip three-way word diff computation",
                );
                assert!(
                    pane.conflict_resolver.two_way_split_word_highlight(0).is_none(),
                    "streamed huge-file bootstrap should keep two-way word highlights on-demand",
                );
                assert!(
                    index.total_rows() > 0,
                    "paged split row index should have rows",
                );
                assert!(
                    pane.conflict_resolver.two_way_split_projection().is_some(),
                    "giant mode should have a split projection",
                );

                // View mode should NOT be forced to ThreeWay — two-way now has data.
                // (Default for FullTextResolver with base is ThreeWay, but it's
                // not forced by the large-file path.)

                // Three-way data should still be populated correctly.
                assert!(
                    pane.conflict_resolver.three_way_len >= fixture.fixture_line_count,
                    "three_way_len should be at least fixture_line_count ({}), got {}",
                    fixture.fixture_line_count,
                    pane.conflict_resolver.three_way_len,
                );
                assert!(
                    !pane
                        .conflict_resolver
                        .three_way_text
                        .base
                        .as_ref()
                        .is_empty(),
                    "three-way base text should be populated",
                );

                // Conflict marker parsing should still work.
                assert_eq!(
                    crate::view::conflict_resolver::conflict_count(
                        &pane.conflict_resolver.marker_segments
                    ),
                    fixture.conflict_block_count,
                    "should have parsed {} conflict block(s)",
                    fixture.conflict_block_count,
                );
                let current = pane
                    .conflict_resolver
                    .current
                    .clone()
                    .expect("huge streamed bootstrap should retain current merged text");
                let first_block = pane
                    .conflict_resolver
                    .marker_segments
                    .iter()
                    .find_map(|segment| match segment {
                        crate::view::conflict_resolver::ConflictSegment::Block(block) => {
                            Some(block)
                        }
                        crate::view::conflict_resolver::ConflictSegment::Text(_) => None,
                    })
                    .expect("huge streamed bootstrap should keep a conflict block");
                assert!(
                    first_block.ours.shares_backing_with(&current)
                        && first_block.theirs.shares_backing_with(&current),
                    "huge streamed bootstrap should reuse current-text backing for marker block sides",
                );
                let first_row_ix = index
                    .first_row_for_conflict(0)
                    .expect("paged index should expose the first conflict row");
                let first_row = index
                    .row_at(&pane.conflict_resolver.marker_segments, first_row_ix)
                    .expect("paged index should serve the first conflict row");
                let expected_first_row_line = fixture.first_conflict_line;
                assert!(
                    first_row.old_line == Some(expected_first_row_line)
                        || first_row.new_line == Some(expected_first_row_line),
                    "first streamed conflict row should align to the first conflict line {}, got old={:?} new={:?}",
                    expected_first_row_line,
                    first_row.old_line,
                    first_row.new_line,
                );
                assert!(
                    pane.conflict_resolver
                        .two_way_visible_ix_for_conflict(0)
                        .is_some(),
                    "streamed projection should expose the first conflict in visible space",
                );

                let _ = cx;
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn large_conflict_bootstrap_uses_streamed_split_index_for_dense_huge_files(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(163);
    let fixture = SyntheticLargeConflictFixture::new(
        "large_conflict_block_local_dense",
        "fixtures/huge_conflict_dense.html",
        60_000,
        256,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "dense large conflict streamed split bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == fixture.conflict_block_count
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_rows={} split_row_index={} conflicts={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
                pane.conflict_resolver.split_row_index().is_some(),
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments
                ),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                assert_eq!(
                    crate::view::conflict_resolver::conflict_count(
                        &pane.conflict_resolver.marker_segments
                    ),
                    fixture.conflict_block_count,
                );
                let index = pane
                    .conflict_resolver
                    .split_row_index()
                    .expect("dense huge conflicts should now always use the streamed split index");
                assert!(
                    index.total_rows() >= fixture.conflict_block_count,
                    "paged index should have at least one row per conflict block, got {}",
                    index.total_rows(),
                );
                assert!(
                    pane.conflict_resolver.two_way_split_projection().is_some(),
                    "streamed dense conflicts should have a split projection",
                );
                assert_eq!(
                    pane.conflict_resolver.two_way_row_counts().1,
                    0,
                    "streamed dense conflicts should not materialize inline rows",
                );
                assert!(
                    pane.conflict_resolver
                        .two_way_split_word_highlight(0)
                        .is_none(),
                    "streamed dense conflicts should keep word highlights on-demand",
                );
            });
        });
    });

    fixture.cleanup();
}

/// Verifies that merge-input (three-way) sides get background syntax
/// preparation when the foreground parse budget is exhausted, and that
/// the visible-row fallback still uses `Auto` syntax above the old line gate
/// before the prepared documents become available for rendering.
#[gpui::test]
fn large_conflict_three_way_sides_get_background_syntax_documents(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(165);
    let fixture_line_count = rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING + 101;
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_three_way_bg_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/three_way_syntax_bg.xml");
    let abs_path = workdir.join(&file_rel);
    let shared_root_line = r#"<root attr="shared">"#;
    let base_conflict_line = r#"<button class="base" disabled="true" />"#;
    let ours_conflict_line = r#"<button class="ours" disabled="true" />"#;
    let theirs_conflict_line = r#"<button class="theirs" disabled="true" />"#;
    let closing_root_line = "</root>";
    let tag_or_attr_before_quote_ix = shared_root_line
        .find('"')
        .expect("shared XML line should include a quoted attribute value");

    assert!(
        fixture_line_count > rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING,
        "fixture should stay above the old conflict-resolver syntax gate"
    );

    let mut base_lines = vec![shared_root_line.to_string(), base_conflict_line.to_string()];
    base_lines.extend(
        (base_lines.len()..fixture_line_count.saturating_sub(1))
            .map(|ix| format!(r#"<item ix="{ix}" />"#)),
    );
    base_lines.push(closing_root_line.to_string());
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[1] = ours_conflict_line.to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[1] = theirs_conflict_line.to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        shared_root_line.to_string(),
        "<<<<<<< ours".to_string(),
        ours_conflict_line.to_string(),
        "=======".to_string(),
        theirs_conflict_line.to_string(),
        ">>>>>>> theirs".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count.saturating_sub(1))
            .map(|ix| format!(r#"<item ix="{ix}" />"#)),
    );
    current_lines.push(closing_root_line.to_string());
    let current_text = current_lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            // Set foreground budget to zero so all sides go to background.
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.clone(),
                base_text.clone(),
                ours_text.clone(),
                theirs_text.clone(),
                current_text.clone(),
            );

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    // Wait for bootstrap to complete.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "three-way background syntax bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path),
    );

    // Right after bootstrap with ZERO budget, the test may still observe either
    // the fallback path or an already-completed prepared document, depending on
    // how quickly the deterministic test scheduler drains the queued task.
    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                assert_eq!(
                    pane.conflict_resolver.conflict_syntax_language,
                    Some(rows::DiffSyntaxLanguage::Xml),
                    "syntax language should be XML for .xml file"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        if pane
            .conflict_three_way_prepared_syntax_documents
            .base
            .is_none()
        {
            let styled = pane
                .conflict_three_way_segments_cache
                .get(&(0, ThreeWayColumn::Base))
                .expect("initial draw should populate the visible three-way base-row cache");
            assert_eq!(
                styled.text.as_ref(),
                shared_root_line,
                "expected the cached three-way fallback row to match the shared XML root line"
            );
            assert!(
                styled
                    .highlights
                    .iter()
                    .any(|(range, _)| range.start < tag_or_attr_before_quote_ix),
                "three-way fallback should use Auto syntax and highlight XML tag/attribute ranges before the quoted string above the old line gate; got {:?}",
                styled_debug_info_with_styles(styled),
            );
        }
    });

    // Wait for background syntax parses to complete for all three sides.
    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "three-way background syntax completion",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_three_way_prepared_syntax_documents
                .base
                .is_some()
                && pane
                    .conflict_three_way_prepared_syntax_documents
                    .ours
                    .is_some()
                && pane
                    .conflict_three_way_prepared_syntax_documents
                    .theirs
                    .is_some()
        },
        |pane| {
            format!(
                "base={:?} ours={:?} theirs={:?}",
                pane.conflict_three_way_prepared_syntax_documents.base,
                pane.conflict_three_way_prepared_syntax_documents.ours,
                pane.conflict_three_way_prepared_syntax_documents.theirs,
            )
        },
    );

    // After background parses complete, inflight flags should be cleared
    // and documents should be available for rendering.
    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                assert!(!pane.conflict_three_way_syntax_inflight.base);
                assert!(!pane.conflict_three_way_syntax_inflight.ours);
                assert!(!pane.conflict_three_way_syntax_inflight.theirs);
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup fixture");
}

#[gpui::test]
fn large_conflict_two_way_views_upgrade_to_prepared_document_syntax(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(166);
    let fixture_line_count = rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING + 101;
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_two_way_bg_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/two_way_syntax_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let opening_line = "fn main() {";
    let comment_open_line = "/* open comment";
    let base_comment_line = "still base comment */ let base_value = 0;";
    let ours_comment_line = "still ours comment */ let ours_value = 1;";
    let theirs_comment_line = "still theirs comment */ let theirs_value = 2;";
    let closing_line = "}";
    let comment_prefix_end = ours_comment_line
        .find("*/")
        .map(|ix| ix + 2)
        .expect("comment line should include a closing block comment delimiter");
    let ours_comment_line_ix = 2usize;

    let mut base_lines = vec![
        opening_line.to_string(),
        comment_open_line.to_string(),
        base_comment_line.to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count.saturating_sub(1))
            .map(|ix| format!("let filler_{ix} = {ix};")),
    );
    base_lines.push(closing_line.to_string());
    let base_text = base_lines.join("\n");

    let mut ours_lines = vec![
        opening_line.to_string(),
        comment_open_line.to_string(),
        ours_comment_line.to_string(),
    ];
    ours_lines.extend(
        (ours_lines.len()..fixture_line_count.saturating_sub(1))
            .map(|ix| format!("let filler_{ix} = {ix};")),
    );
    ours_lines.push(closing_line.to_string());
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = vec![
        opening_line.to_string(),
        comment_open_line.to_string(),
        theirs_comment_line.to_string(),
    ];
    theirs_lines.extend(
        (theirs_lines.len()..fixture_line_count.saturating_sub(1))
            .map(|ix| format!("let filler_{ix} = {ix};")),
    );
    theirs_lines.push(closing_line.to_string());
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        opening_line.to_string(),
        comment_open_line.to_string(),
        "<<<<<<< ours".to_string(),
        ours_comment_line.to_string(),
        "=======".to_string(),
        theirs_comment_line.to_string(),
        ">>>>>>> theirs".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count.saturating_sub(1))
            .map(|ix| format!("let filler_{ix} = {ix};")),
    );
    current_lines.push(closing_line.to_string());
    let current_text = current_lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            set_test_conflict_file(
                &mut repo,
                file_rel.clone(),
                base_text.clone(),
                ours_text.clone(),
                theirs_text.clone(),
                current_text.clone(),
            );

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "two-way background syntax bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| format!("path={:?}", pane.conflict_resolver.path),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.conflict_resolver_scroll_all_columns(0, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let fallback_split_highlights_hash = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = conflict_split_cached_styled(
            pane,
            crate::view::conflict_resolver::ConflictPickSide::Ours,
            ours_comment_line,
        )
        .expect("initial split draw should populate the visible conflict diff cache");
        assert_eq!(
            styled.text.as_ref(),
            ours_comment_line,
            "expected the cached two-way split row to match the multiline comment text"
        );
        let has_comment_highlight = styled_has_leading_color_highlight(
            styled,
            comment_prefix_end,
            pane.theme.syntax.comment.into_color(),
        );
        if has_comment_highlight {
            None
        } else {
            assert!(
                pane.conflict_three_way_prepared_syntax_documents
                    .ours
                    .is_none(),
                "if the first split draw is still using fallback syntax, the prepared ours document should not exist yet"
            );
            assert!(
                pane.conflict_three_way_prepared_syntax_documents
                    .theirs
                    .is_none(),
                "if the first split draw is still using fallback syntax, the prepared theirs document should not exist yet"
            );
            Some(styled.highlights_hash)
        }
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "two-way split syntax upgrade after background preparation",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_three_way_prepared_syntax_documents
                .ours
                .is_some()
                && pane
                    .conflict_three_way_prepared_syntax_documents
                    .theirs
                    .is_some()
                && conflict_split_cached_styled(
                    pane,
                    crate::view::conflict_resolver::ConflictPickSide::Ours,
                    ours_comment_line,
                )
                .is_some_and(|styled| {
                    fallback_split_highlights_hash
                        .map(|hash| styled.highlights_hash != hash)
                        .unwrap_or(true)
                        && styled_has_leading_color_highlight(
                            styled,
                            comment_prefix_end,
                            pane.theme.syntax.comment.into_color(),
                        )
                })
        },
        |pane| {
            let split_cached = conflict_split_cached_styled(
                pane,
                crate::view::conflict_resolver::ConflictPickSide::Ours,
                ours_comment_line,
            )
            .map(styled_debug_info_with_styles);
            format!(
                "ours_doc={:?} theirs_doc={:?} split_cached={split_cached:?}",
                pane.conflict_three_way_prepared_syntax_documents.ours,
                pane.conflict_three_way_prepared_syntax_documents.theirs,
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = conflict_split_cached_styled(
            pane,
            crate::view::conflict_resolver::ConflictPickSide::Ours,
            ours_comment_line,
        )
        .expect("split cache should stay available after background syntax preparation");
        assert!(
            styled_has_leading_color_highlight(
                styled,
                comment_prefix_end,
                pane.theme.syntax.comment.into_color(),
            ),
            "prepared syntax should continue to drive split-row styling after background preparation",
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "switching to three-way should invalidate stale split-row styling caches",
                );
                assert!(
                    pane.conflict_three_way_segments_cache.is_empty(),
                    "switching to three-way should invalidate stale three-way styling caches",
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_three_way_segments_cache
            .get(&(ours_comment_line_ix, ThreeWayColumn::Ours))
            .expect("three-way draw should restyle the visible ours row after toggling from two-way");
        assert_eq!(
            styled.text.as_ref(),
            ours_comment_line,
            "expected the cached three-way ours row to match the multiline comment text",
        );
        assert!(
            styled_has_leading_color_highlight(
                styled,
                comment_prefix_end,
                pane.theme.syntax.comment.into_color(),
            ),
            "prepared syntax should continue to drive three-way row styling after toggling from two-way",
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "switching back to two-way should invalidate stale split-row styling caches",
                );
                assert!(
                    pane.conflict_three_way_segments_cache.is_empty(),
                    "switching back to two-way should invalidate stale three-way styling caches",
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = conflict_split_cached_styled(
            pane,
            crate::view::conflict_resolver::ConflictPickSide::Ours,
            ours_comment_line,
        )
        .expect("split cache should rebuild after returning from three-way mode");
        assert!(
            styled_has_leading_color_highlight(
                styled,
                comment_prefix_end,
                pane.theme.syntax.comment.into_color(),
            ),
            "prepared syntax should continue to drive split-row styling after toggling back from three-way",
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup fixture");
}

#[ignore = "manual stress: 500k-line whole-file conflict bootstrap"]
#[gpui::test]
fn very_large_whole_file_conflict_bootstrap_manual_regression_stays_streamed(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(170);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_manual_500k",
        "fixtures/very_large_whole_file_conflict.html",
        500_000,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "very large whole-file conflict streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == 1
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
                && pane.conflict_resolved_output_projection.is_some()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} split_rows={} split_row_index={} output_projection={} three_way_len={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolver.three_way_len,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                assert_streamed_whole_file_two_way_state(pane, fixture.line_count);
                assert!(
                    pane.conflict_resolved_output_projection.is_some(),
                    "500k-line whole-file bootstrap should keep resolved output streamed",
                );
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver_input.read(app).text(),
            "",
            "500k-line whole-file bootstrap should not materialize the resolved output buffer",
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                assert_eq!(
                    pane.conflict_resolver.view_mode,
                    ConflictResolverViewMode::ThreeWay,
                    "500k-line whole-file conflict should survive switching back to three-way mode",
                );
                assert_streamed_whole_file_three_way_state(pane, fixture.line_count);
            });
        });
    });

    fixture.cleanup();
}

#[ignore = "manual stress: 500k-line focused mergetool bootstrap"]
#[gpui::test]
fn very_large_conflict_bootstrap_manual_regression_stays_sparse(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(164);
    let fixture = SyntheticLargeConflictFixture::new(
        "large_conflict_block_local_manual_500k",
        "fixtures/very_large_conflict.html",
        500_001,
        12,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "very large conflict streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == fixture.conflict_block_count
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_rows={} split_row_index={} three_way_len={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.three_way_len,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                let index = pane
                    .conflict_resolver
                    .split_row_index()
                    .expect("500k-line manual fixture should use the streamed split index");
                assert!(
                    pane
                        .conflict_resolver
                        .three_way_word_highlights
                        .ours
                        .is_empty(),
                    "500k-line manual fixture should skip eager three-way word highlights",
                );
                assert!(
                    pane.conflict_resolver.two_way_split_word_highlight(0).is_none(),
                    "500k-line manual fixture should keep two-way word highlights on-demand",
                );
                assert!(
                    index.total_rows() > fixture.conflict_block_count,
                    "500k-line manual fixture should expose paged rows for the streamed split view",
                );
                let first_row = index
                    .first_row_for_conflict(0)
                    .expect("manual streamed fixture should expose a first conflict row");
                let row = index
                    .row_at(&pane.conflict_resolver.marker_segments, first_row)
                    .expect("manual streamed fixture should resolve rows on demand");
                assert!(
                    row.old.as_deref().is_some() || row.new.as_deref().is_some(),
                    "manual streamed fixture should still expose real diff content through the page index",
                );
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn large_conflict_bootstrap_populates_resolved_outline_in_background(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(167);
    let fixture = SyntheticLargeConflictFixture::new(
        "large_conflict_resolved_outline_bg",
        "fixtures/resolved_outline_bg.html",
        20_000,
        4,
    );
    fixture.write();

    let expected_resolved_line_count = crate::view::conflict_resolver::generate_resolved_text(
        crate::view::conflict_resolver::parse_conflict_markers(&fixture.current_text).as_slice(),
    )
    .split('\n')
    .count();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
            });

            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "background resolved outline bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolved_preview_line_count == expected_resolved_line_count
                && pane.conflict_resolver.resolved_outline.meta.len()
                    == expected_resolved_line_count
                && pane.conflict_resolver.resolved_outline.markers.len()
                    == expected_resolved_line_count
        },
        |pane| {
            format!(
                "path={:?} preview_lines={} meta={} markers={} live_syntax={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolved_output_live_syntax
                    .as_ref()
                    .map(|document| document.version()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                let start_markers = pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .filter(|marker| marker.is_start)
                    .count();
                assert_eq!(
                    start_markers, fixture.conflict_block_count,
                    "background outline rebuild should materialize one start marker per conflict",
                );
                assert!(
                    pane.conflict_resolver
                        .resolved_outline
                        .markers
                        .iter()
                        .flatten()
                        .any(|marker| marker.unresolved),
                    "bootstrap outline markers should preserve unresolved conflict state",
                );
                assert!(
                    pane.conflict_resolver
                        .resolved_outline
                        .meta
                        .iter()
                        .any(|meta| meta.source
                            != crate::view::conflict_resolver::ResolvedLineSource::Manual),
                    "background provenance rebuild should classify source-backed output lines",
                );
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn large_conflict_two_way_resolved_outline_uses_indexed_sources_in_streamed_mode(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(168);
    let fixture = SyntheticLargeConflictFixture::new(
        "large_conflict_two_way_resolved_outline_streamed",
        "fixtures/resolved_outline_two_way_streamed.html",
        20_001,
        4,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = fixture.repo_state(repo_id);
            for region in &mut repo
                .conflict_state
                .conflict_session
                .as_mut()
                .expect("large conflict session")
                .regions
            {
                region.resolution =
                    gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs;
            }
            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "two-way streamed resolved outline bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_rows={} split_row_index={} resolved_meta={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.resolved_outline.meta.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.recompute_conflict_resolved_outline_for_tests(cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let conflict_line_ix =
                    usize::try_from(fixture.first_conflict_line.saturating_sub(1)).unwrap_or(0);
                let conflict_meta = pane
                    .conflict_resolver
                    .resolved_outline
                    .meta
                    .get(conflict_line_ix)
                    .expect("conflict line metadata");
                assert_eq!(
                    pane.conflict_resolver.resolved_outline.meta.len(),
                    fixture.fixture_line_count,
                    "two-way streamed outline should populate one metadata row per output line",
                );
                assert_eq!(
                    conflict_meta.source,
                    crate::view::conflict_resolver::ResolvedLineSource::A,
                    "an explicit Local selection should map conflict lines to the ours side in two-way mode",
                );
                assert_eq!(
                    conflict_meta.input_line,
                    Some(fixture.first_conflict_line),
                    "two-way streamed outline should keep the original source line number for conflict rows",
                );
            });
        });
    });

    fixture.cleanup();
}

/// Verifies that search in giant two-way mode works over source texts without
/// generating eager diff rows. The search should find text in the middle of a
/// large conflict block.
#[gpui::test]
fn giant_two_way_search_finds_text_in_middle_of_large_block(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(171);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "giant_two_way_search_mid_block",
        "fixtures/search_mid_block.html",
        20_001,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "giant two-way search bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_row_index={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.split_row_index().is_some(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                assert_streamed_whole_file_two_way_state(pane, fixture.line_count);

                // The whole-file conflict fixture has lines like 'panel-25000'
                // in the middle of the block. Search for it via the paged index.
                let index = pane
                    .conflict_resolver
                    .split_row_index()
                    .expect("split row index should be present");
                index.clear_cached_pages();
                assert_eq!(
                    index.cached_page_count(),
                    0,
                    "search should start without materialized split pages"
                );

                let target = "panel-10000";
                let matches = index
                    .search_matching_rows(&pane.conflict_resolver.marker_segments, |line_text| {
                        line_text.contains(target)
                    });
                assert!(
                    !matches.is_empty(),
                    "search should find '{target}' in the middle of the large block",
                );
                assert_eq!(
                    index.cached_page_count(),
                    0,
                    "source-text search should not materialize split pages"
                );

                // Verify the matching row actually contains the search text.
                let matched_row_ix = matches[0];
                let row = index
                    .row_at(&pane.conflict_resolver.marker_segments, matched_row_ix)
                    .expect("matched row should be generatable");
                let row_has_target = row.old.as_ref().is_some_and(|t| t.contains(target))
                    || row.new.as_ref().is_some_and(|t| t.contains(target));
                assert!(
                    row_has_target,
                    "generated row at source index {matched_row_ix} should contain '{target}'",
                );
                assert_eq!(
                    index.cached_page_count(),
                    1,
                    "reading the matched row should materialize only the destination split page"
                );

                // The matching row should have a visible index via the projection.
                if let Some(proj) = pane.conflict_resolver.two_way_split_projection() {
                    let visible_ix = proj.source_to_visible(matched_row_ix);
                    assert!(
                        visible_ix.is_some(),
                        "source row {matched_row_ix} should map to a visible index",
                    );
                }
            });
        });
    });

    fixture.cleanup();
}
