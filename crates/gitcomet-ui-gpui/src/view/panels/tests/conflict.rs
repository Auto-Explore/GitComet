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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(162);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_shared_conflict_arcs",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/shared_conflict_arcs.html");
    let abs_path = workdir.join(&file_rel);

    let base_text: Arc<str> = "<p>base</p>\n".into();
    let ours_text: Arc<str> = "<p>ours</p>\n".into();
    let theirs_text: Arc<str> = "<p>theirs</p>\n".into();
    let current_text: Arc<str> =
        "<<<<<<< ours\n<p>ours</p>\n=======\n<p>theirs</p>\n>>>>>>> theirs\n".into();

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
                    path: file_rel.clone(),
                    base_bytes: None,
                    ours_bytes: None,
                    theirs_bytes: None,
                    current_bytes: None,
                    base: Some(base_text.clone()),
                    ours: Some(ours_text.clone()),
                    theirs: Some(theirs_text.clone()),
                    current: Some(current_text.clone()),
                }));
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.clone()),
                ConflictPayload::Text(ours_text.clone()),
                ConflictPayload::Text(theirs_text.clone()),
                &current_text,
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

#[gpui::test]
fn svg_conflict_preview_rasterizes_off_the_ui_thread(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(163);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_svg_conflict_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_preview.svg");
    let abs_path = workdir.join(&file_rel);

    let base_svg: Arc<str> =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><rect width="16" height="16" fill="#1d4ed8"/></svg>"##
            .into();
    let ours_svg: Arc<str> =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><circle cx="8" cy="8" r="7" fill="#dc2626"/></svg>"##
            .into();
    let theirs_svg: Arc<str> =
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 16 16"><path d="M2 14L8 2L14 14Z" fill="#16a34a"/></svg>"##
            .into();
    let current_text: Arc<str> = format!(
        "<<<<<<< ours\n{}\n=======\n{}\n>>>>>>> theirs\n",
        ours_svg, theirs_svg
    )
    .into();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("svg conflict preview parent"))
        .expect("create svg conflict preview dir");
    std::fs::write(&abs_path, current_text.as_bytes()).expect("write svg conflict preview");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_conflict_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.conflict_state.conflict_file_path = Some(file_rel.clone());
            repo.conflict_state.conflict_file =
                gitcomet_state::model::Loadable::Ready(Some(gitcomet_state::model::ConflictFile {
                    path: file_rel.clone(),
                    base_bytes: None,
                    ours_bytes: None,
                    theirs_bytes: None,
                    current_bytes: None,
                    base: Some(base_svg.clone()),
                    ours: Some(ours_svg.clone()),
                    theirs: Some(theirs_svg.clone()),
                    current: Some(current_text.clone()),
                }));
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_svg.clone()),
                ConflictPayload::Text(ours_svg.clone()),
                ConflictPayload::Text(theirs_svg.clone()),
                &current_text,
            ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "svg conflict resolver bootstrap initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.source_hash.is_some()
        },
        |pane| {
            format!(
                "path={:?} source_hash={:?} preview_path={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.source_hash,
                pane.conflict_resolver.image_preview.path.clone(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_image_preview_cache(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "svg conflict preview cache rasterized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            matches!(
                pane.conflict_resolver.image_preview.image(ThreeWayColumn::Base),
                Loadable::Ready(Some(image)) if image.format() == gpui::ImageFormat::Png
            ) && matches!(
                pane.conflict_resolver.image_preview.image(ThreeWayColumn::Ours),
                Loadable::Ready(Some(image)) if image.format() == gpui::ImageFormat::Png
            ) && matches!(
                pane.conflict_resolver.image_preview.image(ThreeWayColumn::Theirs),
                Loadable::Ready(Some(image)) if image.format() == gpui::ImageFormat::Png
            )
        },
        |pane| {
            format!(
                "base={:?} ours={:?} theirs={:?}",
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Base),
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Ours),
                pane.conflict_resolver
                    .image_preview
                    .image(ThreeWayColumn::Theirs),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup svg conflict preview fixture");
}

#[gpui::test]
fn conflict_resolver_input_lists_measure_later_long_rows_for_horizontal_scroll(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    fn assert_horizontal_overflow(handle: &gpui::UniformListScrollHandle, label: &str) {
        let size = handle
            .0
            .borrow()
            .last_item_size
            .expect("expected rendered list item size");
        assert!(
            size.contents.width > size.item.width,
            "{label} should report horizontal overflow, got item={:?} contents={:?}",
            size.item,
            size.contents,
        );
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(163);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_resolver_hscroll_measure",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("fixtures/conflict_resolver_hscroll_measure.txt");
    let abs_path = workdir.join(&file_rel);

    let long_base = format!("base {}", "X".repeat(320));
    let long_ours = format!("ours {}", "Y".repeat(320));
    let long_theirs = format!("theirs {}", "Z".repeat(320));
    let base_text = ["short", "context", long_base.as_str(), "tail"].join("\n");
    let ours_text = ["short", "context", long_ours.as_str(), "tail"].join("\n");
    let theirs_text = ["short", "context", long_theirs.as_str(), "tail"].join("\n");
    let current_text =
        format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create resolver hscroll fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write resolver hscroll fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
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
            repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.clone().into()),
                ConflictPayload::Text(ours_text.clone().into()),
                ConflictPayload::Text(theirs_text.clone().into()),
                &current_text,
            ));

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolver hscroll fixture initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.two_way_split_visible_len() >= 4
                && pane.conflict_resolver.three_way_visible_len() >= 4
        },
        |pane| {
            format!(
                "path={:?} two_way_visible={} three_way_visible={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.two_way_split_visible_len(),
                pane.conflict_resolver.three_way_visible_len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                assert!(
                    pane.conflict_resolver.two_way_horizontal_measure_row(
                        crate::view::conflict_resolver::ConflictPickSide::Ours,
                    ) > 0,
                    "two-way ours column should not measure only the first short row",
                );
                assert!(
                    pane.conflict_resolver.two_way_horizontal_measure_row(
                        crate::view::conflict_resolver::ConflictPickSide::Theirs,
                    ) > 0,
                    "two-way theirs column should not measure only the first short row",
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        assert_horizontal_overflow(&pane.conflict_resolver_diff_scroll, "two-way ours list");
        assert_horizontal_overflow(&pane.conflict_preview_theirs_scroll, "two-way theirs list");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                assert!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Base)
                        > 0,
                    "three-way base column should not measure only the first short row",
                );
                assert!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Ours)
                        > 0,
                    "three-way ours column should not measure only the first short row",
                );
                assert!(
                    pane.conflict_resolver
                        .three_way_horizontal_measure_row(ThreeWayColumn::Theirs)
                        > 0,
                    "three-way theirs column should not measure only the first short row",
                );
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        assert_horizontal_overflow(&pane.conflict_resolver_diff_scroll, "three-way base list");
        assert_horizontal_overflow(&pane.conflict_preview_ours_scroll, "three-way ours list");
        assert_horizontal_overflow(
            &pane.conflict_preview_theirs_scroll,
            "three-way theirs list",
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup resolver hscroll fixture");
}

struct SyntheticLargeConflictFixture {
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    abs_path: std::path::PathBuf,
    fixture_line_count: usize,
    conflict_block_count: usize,
    first_conflict_line: u32,
    base_text: String,
    ours_text: String,
    theirs_text: String,
    current_text: String,
}

impl SyntheticLargeConflictFixture {
    fn new(
        workdir_label: &str,
        file_rel: &str,
        fixture_line_count: usize,
        conflict_block_count: usize,
    ) -> Self {
        assert!(
            fixture_line_count >= conflict_block_count.saturating_add(3),
            "fixture needs room for 3 header lines plus at least 1 line per conflict"
        );
        assert!(
            conflict_block_count > 0,
            "synthetic large conflict fixture requires at least one conflict block"
        );

        let workdir = std::env::temp_dir().join(format!(
            "gitcomet_ui_test_{}_{}",
            std::process::id(),
            workdir_label
        ));
        let file_rel = std::path::PathBuf::from(file_rel);
        let abs_path = workdir.join(&file_rel);

        let mut base_lines = vec![
            "<!doctype html>".to_string(),
            "<html lang=\"en\">".to_string(),
            "<body class=\"fixture-root\">".to_string(),
        ];
        let mut ours_lines = base_lines.clone();
        let mut theirs_lines = base_lines.clone();
        let mut current_lines = base_lines.clone();

        let remaining_context = fixture_line_count
            .saturating_sub(base_lines.len())
            .saturating_sub(conflict_block_count);
        let context_per_slot = remaining_context / conflict_block_count;
        let context_remainder = remaining_context % conflict_block_count;
        let mut next_context_row = 0usize;
        let mut first_conflict_line = None;

        for conflict_ix in 0..conflict_block_count {
            let base_line = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"base\">base {conflict_ix}</main>"
            );
            let ours_line = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"ours\">ours {conflict_ix}</main>"
            );
            let theirs_line = format!(
                "<main id=\"choice-{conflict_ix}\" data-side=\"theirs\">theirs {conflict_ix}</main>"
            );
            let conflict_line =
                u32::try_from(ours_lines.len().saturating_add(1)).unwrap_or(u32::MAX);
            first_conflict_line.get_or_insert(conflict_line);

            base_lines.push(base_line);
            ours_lines.push(ours_line.clone());
            theirs_lines.push(theirs_line.clone());
            current_lines.push("<<<<<<< ours".to_string());
            current_lines.push(ours_line);
            current_lines.push("=======".to_string());
            current_lines.push(theirs_line);
            current_lines.push(">>>>>>> theirs".to_string());

            let slot_lines = context_per_slot + usize::from(conflict_ix < context_remainder);
            append_synthetic_large_conflict_context(
                &mut base_lines,
                &mut ours_lines,
                &mut theirs_lines,
                &mut current_lines,
                &mut next_context_row,
                slot_lines,
            );
        }

        assert_eq!(base_lines.len(), fixture_line_count);
        assert_eq!(ours_lines.len(), fixture_line_count);
        assert_eq!(theirs_lines.len(), fixture_line_count);

        Self {
            workdir,
            file_rel,
            abs_path,
            fixture_line_count,
            conflict_block_count,
            first_conflict_line: first_conflict_line.unwrap_or(1),
            base_text: base_lines.join("\n"),
            ours_text: ours_lines.join("\n"),
            theirs_text: theirs_lines.join("\n"),
            current_text: current_lines.join("\n"),
        }
    }

    fn write(&self) {
        let _ = std::fs::remove_dir_all(&self.workdir);
        std::fs::create_dir_all(self.abs_path.parent().expect("fixture file parent"))
            .expect("create fixture dir");
        std::fs::write(&self.abs_path, &self.current_text).expect("write fixture");
    }

    fn repo_state(
        &self,
        repo_id: gitcomet_state::model::RepoId,
    ) -> gitcomet_state::model::RepoState {
        use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

        let mut repo = opening_repo_state(repo_id, &self.workdir);
        set_test_conflict_status(
            &mut repo,
            self.file_rel.clone(),
            gitcomet_core::domain::DiffArea::Unstaged,
        );
        set_test_conflict_file(
            &mut repo,
            self.file_rel.clone(),
            self.base_text.clone(),
            self.ours_text.clone(),
            self.theirs_text.clone(),
            self.current_text.clone(),
        );
        repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
            self.file_rel.clone(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(self.base_text.clone().into()),
            ConflictPayload::Text(self.ours_text.clone().into()),
            ConflictPayload::Text(self.theirs_text.clone().into()),
            &self.current_text,
        ));
        repo
    }

    fn cleanup(&self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup fixture");
    }
}

fn append_synthetic_large_conflict_context(
    base_lines: &mut Vec<String>,
    ours_lines: &mut Vec<String>,
    theirs_lines: &mut Vec<String>,
    current_lines: &mut Vec<String>,
    next_context_row: &mut usize,
    count: usize,
) {
    for _ in 0..count {
        let row = *next_context_row;
        let line = format!(
            "<section id=\"panel-{row}\" data-row=\"{row}\"><div class=\"copy\">row {row}</div></section>"
        );
        base_lines.push(line.clone());
        ours_lines.push(line.clone());
        theirs_lines.push(line.clone());
        current_lines.push(line);
        *next_context_row = next_context_row.saturating_add(1);
    }
}

struct SyntheticWholeFileConflictFixture {
    workdir: std::path::PathBuf,
    file_rel: std::path::PathBuf,
    abs_path: std::path::PathBuf,
    line_count: usize,
    base_text: String,
    ours_text: String,
    theirs_text: String,
    current_text: String,
}

impl SyntheticWholeFileConflictFixture {
    fn new(workdir_label: &str, file_rel: &str, line_count: usize) -> Self {
        assert!(
            line_count >= 5,
            "whole-file conflict fixture needs room for html wrapper lines"
        );

        let workdir = std::env::temp_dir().join(format!(
            "gitcomet_ui_test_{}_{}",
            std::process::id(),
            workdir_label
        ));
        let file_rel = std::path::PathBuf::from(file_rel);
        let abs_path = workdir.join(&file_rel);

        let build_side = |side: &str| {
            let mut lines = vec![
                "<!doctype html>".to_string(),
                "<html lang=\"en\">".to_string(),
                format!("<body class=\"whole-file-{side}\">"),
            ];
            let middle_count = line_count.saturating_sub(5);
            for row in 0..middle_count {
                lines.push(format!(
                    "<section id=\"panel-{row}\" data-side=\"{side}\"><div>{side} {row}</div></section>"
                ));
            }
            lines.push("</body>".to_string());
            lines.push("</html>".to_string());
            lines
        };

        let base_lines = build_side("base");
        let ours_lines = build_side("ours");
        let theirs_lines = build_side("theirs");
        assert_eq!(base_lines.len(), line_count);
        assert_eq!(ours_lines.len(), line_count);
        assert_eq!(theirs_lines.len(), line_count);

        let base_text = base_lines.join("\n");
        let ours_text = ours_lines.join("\n");
        let theirs_text = theirs_lines.join("\n");
        let current_text =
            format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");

        Self {
            workdir,
            file_rel,
            abs_path,
            line_count,
            base_text,
            ours_text,
            theirs_text,
            current_text,
        }
    }

    fn write(&self) {
        let _ = std::fs::remove_dir_all(&self.workdir);
        std::fs::create_dir_all(self.abs_path.parent().expect("fixture file parent"))
            .expect("create fixture dir");
        std::fs::write(&self.abs_path, &self.current_text).expect("write fixture");
    }

    fn repo_state(
        &self,
        repo_id: gitcomet_state::model::RepoId,
    ) -> gitcomet_state::model::RepoState {
        use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

        let mut repo = opening_repo_state(repo_id, &self.workdir);
        set_test_conflict_status(
            &mut repo,
            self.file_rel.clone(),
            gitcomet_core::domain::DiffArea::Unstaged,
        );
        set_test_conflict_file(
            &mut repo,
            self.file_rel.clone(),
            self.base_text.clone(),
            self.ours_text.clone(),
            self.theirs_text.clone(),
            self.current_text.clone(),
        );
        repo.conflict_state.conflict_session = Some(ConflictSession::from_merged_text(
            self.file_rel.clone(),
            gitcomet_core::domain::FileConflictKind::BothModified,
            ConflictPayload::Text(self.base_text.clone().into()),
            ConflictPayload::Text(self.ours_text.clone().into()),
            ConflictPayload::Text(self.theirs_text.clone().into()),
            &self.current_text,
        ));
        repo
    }

    fn cleanup(&self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup fixture");
    }
}

fn load_synthetic_whole_file_conflict(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    fixture: &SyntheticWholeFileConflictFixture,
) {
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
}

fn assert_streamed_whole_file_two_way_state(pane: &MainPaneView, line_count: usize) -> usize {
    assert_eq!(
        pane.conflict_resolver.rendering_mode(),
        crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile,
        "whole-file conflicts past the large threshold should enter streamed mode",
    );
    assert_eq!(
        pane.conflict_resolver.three_way_len, line_count,
        "three-way line count should still reflect the full document",
    );
    let index = pane
        .conflict_resolver
        .split_row_index()
        .expect("streamed whole-file mode should build a paged split-row index");
    let projection = pane
        .conflict_resolver
        .two_way_split_projection()
        .expect("streamed whole-file mode should expose a split projection");
    assert_eq!(
        pane.conflict_resolver.two_way_row_counts(),
        (index.total_rows(), 0),
        "streamed whole-file mode should expose paged split rows without inline materialization",
    );
    assert_eq!(
        projection.visible_len(),
        pane.conflict_resolver.two_way_split_visible_len(),
        "streamed whole-file mode should expose a split projection",
    );
    assert!(
        index.total_rows() >= line_count,
        "paged split row index should expose at least the full line count, got {}",
        index.total_rows(),
    );

    let total = pane.conflict_resolver.two_way_split_visible_len();
    assert!(
        total >= line_count,
        "streamed two-way visible length should cover the full file, got {total}",
    );

    let deep_ix = total / 2;
    let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
        source_row_ix: _source_ix,
        row,
        conflict_ix: _conflict_ix,
    } = pane
        .conflict_resolver
        .two_way_split_visible_row(deep_ix)
        .expect("deep streamed two-way row should resolve on demand");
    assert!(
        row.old.is_some() || row.new.is_some(),
        "deep streamed two-way row should expose real source text",
    );

    total
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
fn whole_file_conflict_bootstrap_uses_streamed_large_file_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(169);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_streamed",
        "fixtures/whole_file_conflict.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1_000,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file conflict streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == 1
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
                && pane.conflict_resolver.two_way_split_projection().is_some()
                && pane.conflict_resolved_output_projection.is_some()
        },
        |pane| {
            format!(
                "path={:?} conflicts={} rendering_mode={:?} split_row_index={} projection={} output_projection={} three_way_len={}",
                pane.conflict_resolver.path.clone(),
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.two_way_split_projection().is_some(),
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
                    "streamed whole-file bootstrap should keep resolved output in projection mode",
                );
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver_input.read(app).text(),
            "",
            "streamed whole-file bootstrap should not materialize the resolved output buffer",
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn whole_file_conflict_stage_anyway_uses_streamed_output_without_materializing(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(172);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "whole_file_conflict_stage_anyway_streamed",
        "fixtures/whole_file_conflict_stage_anyway.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1_000,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "whole-file conflict streamed stage-anyway bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolved_output_projection.is_some()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} output_projection={} preview_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    let (expected, actual, input_before, input_after, projection_after) =
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    let expected = crate::view::conflict_resolver::generate_resolved_text(
                        &pane.conflict_resolver.marker_segments,
                    );
                    let input_before = pane.conflict_resolver_input.read(cx).text().to_string();
                    let actual = pane.conflict_resolver_save_contents(cx);
                    let input_after = pane.conflict_resolver_input.read(cx).text().to_string();
                    (
                        expected,
                        actual,
                        input_before,
                        input_after,
                        pane.conflict_resolved_output_projection.is_some(),
                    )
                })
            })
        });

    assert_eq!(
        input_before, "",
        "streamed whole-file output should still be virtual before stage confirmation"
    );
    assert_eq!(
        actual, expected,
        "stage confirmation should serialize the streamed resolved output, not the empty editor buffer"
    );
    assert!(
        !actual.is_empty(),
        "streamed stage-confirm contents should contain the resolved output text"
    );
    assert_eq!(
        input_after, "",
        "stage confirmation should not materialize the resolved-output editor"
    );
    assert!(
        projection_after,
        "stage confirmation should keep the resolved-output projection active"
    );

    fixture.cleanup();
}

#[gpui::test]
fn whole_file_conflict_switch_to_three_way_stays_fully_reviewable(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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

/// Verifies huge conflicts stay on the streamed split path and avoid
/// bootstrap diff/highlight work.
#[gpui::test]
fn large_conflict_bootstrap_stays_streamed_for_huge_files(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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

    // Right after bootstrap with ZERO budget, prepared documents should be None.
    cx.update(|_window, app| {
        view.update(app, |this, _cx| {
            this.main_pane.update(_cx, |pane, _cx| {
                assert!(
                    pane.conflict_three_way_prepared_syntax_documents
                        .base
                        .is_none(),
                    "with zero foreground budget, base prepared document should be None initially"
                );
                assert!(
                    pane.conflict_three_way_prepared_syntax_documents
                        .ours
                        .is_none(),
                    "with zero foreground budget, ours prepared document should be None initially"
                );
                assert!(
                    pane.conflict_three_way_prepared_syntax_documents
                        .theirs
                        .is_none(),
                    "with zero foreground budget, theirs prepared document should be None initially"
                );
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
        assert!(
            pane.conflict_three_way_prepared_syntax_documents
                .base
                .is_none(),
            "initial draw should still be using fallback line syntax before the background parse completes"
        );
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
        let has_comment_highlight = styled_has_leading_muted_highlight(
            styled,
            comment_prefix_end,
            pane.theme.colors.text_muted.into(),
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
                        && styled_has_leading_muted_highlight(
                            styled,
                            comment_prefix_end,
                            pane.theme.colors.text_muted.into(),
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
            styled_has_leading_muted_highlight(
                styled,
                comment_prefix_end,
                pane.theme.colors.text_muted.into(),
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
            styled_has_leading_muted_highlight(
                styled,
                comment_prefix_end,
                pane.theme.colors.text_muted.into(),
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
            styled_has_leading_muted_highlight(
                styled,
                comment_prefix_end,
                pane.theme.colors.text_muted.into(),
            ),
            "prepared syntax should continue to drive split-row styling after toggling back from three-way",
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup fixture");
}

#[gpui::test]
fn conflict_compare_split_renderer_uses_streamed_visible_rows_for_large_conflicts(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(176);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "conflict_compare_split_streamed",
        "fixtures/conflict_compare_split_streamed.html",
        crate::view::conflict_resolver::LARGE_CONFLICT_BLOCK_DIFF_MAX_LINES + 1,
    );
    fixture.write();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(
                conflict_compare_repo_state(
                    repo_id,
                    &fixture.workdir,
                    &fixture.file_rel,
                    &fixture.base_text,
                    &fixture.ours_text,
                    &fixture.theirs_text,
                    &fixture.current_text,
                ),
                repo_id,
            );
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "streamed compare split bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} split_row_index={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver.split_row_index().is_some(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.conflict_diff_segments_cache_split.clear();
                pane.conflict_diff_query_segments_cache_split.clear();

                let visible_ix = pane.conflict_resolver.two_way_split_visible_len() / 2;
                let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix: _source_ix,
                    row,
                    conflict_ix: _conflict_ix,
                } = pane
                    .conflict_resolver
                    .two_way_split_visible_row(visible_ix)
                    .expect("deep streamed compare row should resolve through the split provider");

                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "compare split style cache should start empty for this focused render",
                );

                let elements = MainPaneView::render_conflict_compare_diff_rows(
                    pane,
                    visible_ix..visible_ix + 1,
                    window,
                    cx,
                );
                assert_eq!(elements.len(), 1);

                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "large streamed compare render should skip per-row style caching and render plain text",
                );
                assert!(
                    row.old.is_some() || row.new.is_some(),
                    "deep streamed compare row should still expose real source text",
                );
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn conflict_compare_split_renderer_uses_visible_projection_when_rows_are_hidden(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(177);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_compare_split_hidden",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/conflict_compare_split_hidden.rs");
    let abs_path = workdir.join(&file_rel);

    let base_text = [
        "fn main() {",
        "    let first = 0;",
        "    let between = 1;",
        "    let second = 2;",
        "}",
    ]
    .join("\n");
    let ours_text = [
        "fn main() {",
        "    let first = 10;",
        "    let between = 1;",
        "    let second = 20;",
        "}",
    ]
    .join("\n");
    let theirs_text = [
        "fn main() {",
        "    let first = 11;",
        "    let between = 1;",
        "    let second = 21;",
        "}",
    ]
    .join("\n");
    let current_text = [
        "fn main() {",
        "<<<<<<< ours",
        "    let first = 10;",
        "=======",
        "    let first = 11;",
        ">>>>>>> theirs",
        "    let between = 1;",
        "<<<<<<< ours",
        "    let second = 20;",
        "=======",
        "    let second = 21;",
        ">>>>>>> theirs",
        "}",
    ]
    .join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(
                conflict_compare_repo_state(
                    repo_id,
                    &workdir,
                    &file_rel,
                    &base_text,
                    &ours_text,
                    &theirs_text,
                    &current_text,
                ),
                repo_id,
            );
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split compare streamed bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolver.rendering_mode()
                    == crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} rendering_mode={:?} split_row_index={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.rendering_mode(),
                pane.conflict_resolver.split_row_index().is_some(),
            )
        },
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let first_block = pane
                    .conflict_resolver
                    .marker_segments
                    .iter_mut()
                    .find_map(|segment| match segment {
                        crate::view::conflict_resolver::ConflictSegment::Block(block) => {
                            Some(block)
                        }
                        crate::view::conflict_resolver::ConflictSegment::Text(_) => None,
                    })
                    .expect("fixture should contain a first conflict block");
                first_block.resolved = true;
                pane.conflict_resolver.hide_resolved = true;
                pane.conflict_resolver.rebuild_three_way_visible_state();
                pane.conflict_resolver.rebuild_two_way_visible_state();
                pane.diff_view = DiffViewMode::Split;
                pane.conflict_diff_segments_cache_split.clear();
                pane.conflict_diff_query_segments_cache_split.clear();

                let (visible_ix, source_ix, row) =
                    (0..pane.conflict_resolver.two_way_split_visible_len()).find_map(
                        |visible_ix| {
                        let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
                            source_row_ix: source_ix,
                            row,
                            conflict_ix: _conflict_ix,
                        } = pane
                            .conflict_resolver
                            .two_way_split_visible_row(visible_ix)?;
                        (source_ix != visible_ix && (row.old.is_some() || row.new.is_some()))
                            .then_some((visible_ix, source_ix, row))
                    },
                    )
                    .expect("hide-resolved compare view should remap at least one split row");

                let elements = MainPaneView::render_conflict_compare_diff_rows(
                    pane,
                    visible_ix..visible_ix + 1,
                    window,
                    cx,
                );
                assert_eq!(elements.len(), 1);

                if let Some(expected_text) = row.old.as_deref() {
                    if let Some(styled) = pane.conflict_diff_segments_cache_split.get(&(
                        source_ix,
                        crate::view::conflict_resolver::ConflictPickSide::Ours,
                    )) {
                        assert_eq!(styled.text.as_ref(), expected_text);
                    }
                    assert!(
                        !pane.conflict_diff_segments_cache_split.contains_key(&(
                            visible_ix,
                            crate::view::conflict_resolver::ConflictPickSide::Ours,
                        )),
                        "compare split render should cache ours styling by source row index, not visible row index",
                    );
                }
                if let Some(expected_text) = row.new.as_deref() {
                    if let Some(styled) = pane.conflict_diff_segments_cache_split.get(&(
                        source_ix,
                        crate::view::conflict_resolver::ConflictPickSide::Theirs,
                    )) {
                        assert_eq!(styled.text.as_ref(), expected_text);
                    }
                    assert!(
                        !pane.conflict_diff_segments_cache_split.contains_key(&(
                            visible_ix,
                            crate::view::conflict_resolver::ConflictPickSide::Theirs,
                        )),
                        "compare split render should cache theirs styling by source row index, not visible row index",
                    );
                }
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup fixture");
}

#[ignore = "manual stress: 500k-line whole-file conflict bootstrap"]
#[gpui::test]
fn very_large_whole_file_conflict_bootstrap_manual_regression_stays_streamed(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
                "path={:?} preview_lines={} meta={} markers={} prepared_document={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolved_preview_prepared_syntax_document,
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

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
                    "default resolved output should map conflict lines to the ours side in two-way mode",
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

#[gpui::test]
fn structured_conflict_edit_reuses_stashed_outline_base_while_background_recompute_is_pending(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(168);
    let fixture = SyntheticLargeConflictFixture::new(
        "resolved_outline_pending_incremental",
        "fixtures/resolved_outline_pending.html",
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
            let next_state = app_state_with_repo(fixture.repo_state(repo_id), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved outline pending incremental initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel),
        |pane| {
            format!(
                "path={:?} preview_lines={} meta={} markers={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_resolved_output_materialized(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved outline pending incremental materialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolved_output_projection.is_none()
                && pane.conflict_resolved_preview_line_count == expected_resolved_line_count
        },
        |pane| {
            format!(
                "path={:?} projection_present={} preview_lines={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
                pane.set_conflict_resolved_outline_background_delay_override_for_tests(
                    std::time::Duration::from_millis(1_000),
                );
                assert_eq!(
                    pane.conflict_resolver.resolved_outline.meta.len(),
                    expected_resolved_line_count,
                    "forced outline recompute should seed current metadata before the pending fallback test starts",
                );
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = fixture.repo_state(repo_id);
            repo.conflict_state.conflict_hide_resolved = true;
            repo.conflict_state.conflict_rev = repo.conflict_state.conflict_rev.wrapping_add(1);

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "resolved outline state sync clears visible metadata while delayed background recompute is pending",
        std::time::Duration::from_millis(500),
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolved_preview_line_count == expected_resolved_line_count
                && pane.conflict_resolver.resolved_outline.meta.is_empty()
                && pane.conflict_resolver.resolved_outline.markers.is_empty()
        },
        |pane| {
            format!(
                "hide_resolved={} preview_lines={} meta={} markers={} stash={} pending_seq={}",
                pane.conflict_resolver.hide_resolved,
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolved_outline_stash.is_some(),
                pane.conflict_resolver.resolver_pending_recompute_seq,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let first_block = pane
                    .conflict_resolver
                    .marker_segments
                    .iter_mut()
                    .find_map(|segment| match segment {
                        crate::view::conflict_resolver::ConflictSegment::Block(block) => {
                            Some(block)
                        }
                        crate::view::conflict_resolver::ConflictSegment::Text(_) => None,
                    })
                    .expect("fixture should contain at least one conflict block");
                first_block.choice = crate::view::conflict_resolver::ConflictChoice::Theirs;
                first_block.resolved = true;

                let resolved = crate::view::conflict_resolver::generate_resolved_text(
                    &pane.conflict_resolver.marker_segments,
                );
                pane.conflict_resolver_set_output(resolved, cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "structured edit incrementally restores outline metadata from stashed base before delayed background fallback completes",
        std::time::Duration::from_millis(500),
        |pane| {
            pane.conflict_resolver.resolved_outline.meta.len() == expected_resolved_line_count
                && pane.conflict_resolver.resolved_outline.markers.len()
                    == expected_resolved_line_count
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .any(|marker| marker.conflict_ix == 0 && !marker.unresolved)
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .any(|marker| marker.conflict_ix == 1 && marker.unresolved)
        },
        |pane| {
            let first_markers: Vec<(usize, bool, bool)> = pane
                .conflict_resolver
                .resolved_outline
                .markers
                .iter()
                .flatten()
                .take(8)
                .map(|marker| (marker.conflict_ix, marker.unresolved, marker.is_start))
                .collect();
            format!(
                "meta={} markers={} stash={} first_markers={first_markers:?} preview_hash={:?}",
                pane.conflict_resolver.resolved_outline.meta.len(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolved_outline_stash.is_some(),
                pane.conflict_resolved_preview_source_hash,
            )
        },
    );

    fixture.cleanup();
}

/// Verifies that giant two-way split mode uses the paged provider to generate
/// rows on demand instead of building an eager `diff_rows` array. Deep rows
/// should be accessible without materializing rows for earlier indices.
#[gpui::test]
fn giant_two_way_paged_provider_generates_rows_on_demand(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(170);
    let fixture = SyntheticWholeFileConflictFixture::new(
        "giant_two_way_paged_on_demand",
        "fixtures/paged_on_demand.html",
        20_001,
    );
    load_synthetic_whole_file_conflict(cx, &view, repo_id, &fixture);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "giant two-way paged bootstrap",
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
                let total = assert_streamed_whole_file_two_way_state(pane, fixture.line_count);

                // Generate a deep row on demand without touching earlier rows.
                let deep_ix = total / 2;
                let crate::view::conflict_resolver::TwoWaySplitVisibleRow {
                    source_row_ix: source_ix,
                    row,
                    conflict_ix: _conflict_ix,
                } = pane
                    .conflict_resolver
                    .two_way_split_visible_row(deep_ix)
                    .expect("deep visible row should be accessible on demand");
                assert!(
                    row.old.is_some() || row.new.is_some(),
                    "on-demand row at visible index {deep_ix} (source {source_ix}) should have text",
                );

                // Verify the first and last visible rows are accessible too.
                assert!(
                    pane.conflict_resolver.two_way_split_visible_row(0).is_some(),
                    "first visible row should be accessible",
                );
                assert!(
                    pane.conflict_resolver
                        .two_way_split_visible_row(total - 1)
                        .is_some(),
                    "last visible row should be accessible",
                );

                // Out-of-bounds returns None.
                assert!(
                    pane.conflict_resolver
                        .two_way_split_visible_row(total)
                        .is_none(),
                    "out-of-bounds visible row should return None",
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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

#[gpui::test]
fn giant_two_way_resync_rebuilds_split_index_after_manual_session_edit(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(172);
    let fixture = SyntheticLargeConflictFixture::new(
        "giant_two_way_resync_manual_edit",
        "fixtures/resync_manual_edit.html",
        20_001,
        4,
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
        "giant two-way resync bootstrap",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.split_row_index().is_some()
        },
        |pane| {
            format!(
                "path={:?} split_row_index={} conflict_rev={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.split_row_index().is_some(),
                pane.conflict_resolver.conflict_rev,
            )
        },
    );

    let initial_visible_len = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.conflict_resolver.two_way_split_visible_len()
            })
        })
    });

    let manual_text = "<article id=\"manual-0\">manual block 0</article>\n<article id=\"manual-1\">manual block 1</article>\n";
    let (
        updated_repo,
        expected_rev,
        expected_conflict_count,
        expected_total_rows,
        expected_visible_len,
    ) = {
        let mut repo = fixture.repo_state(repo_id);
        let session = repo
            .conflict_state
            .conflict_session
            .as_mut()
            .expect("fixture should include a text conflict session");
        session.regions[0].resolution =
            gitcomet_core::conflict_session::ConflictRegionResolution::ManualEdit(
                manual_text.to_string(),
            );

        let mut expected_segments =
            crate::view::conflict_resolver::parse_conflict_markers(&fixture.current_text);
        crate::view::conflict_resolver::apply_session_region_resolutions_with_index_map(
            &mut expected_segments,
            &session.regions,
        );
        let expected_conflict_count =
            crate::view::conflict_resolver::conflict_count(&expected_segments);
        let expected_index = crate::view::conflict_resolver::ConflictSplitRowIndex::new(
            &expected_segments,
            crate::view::conflict_resolver::BLOCK_LOCAL_DIFF_CONTEXT_LINES,
        );
        let expected_projection = crate::view::conflict_resolver::TwoWaySplitProjection::new(
            &expected_index,
            &expected_segments,
            false,
        );
        repo.conflict_state.conflict_rev = repo.conflict_state.conflict_rev.wrapping_add(1);
        let expected_rev = repo.conflict_state.conflict_rev;
        (
            repo,
            expected_rev,
            expected_conflict_count,
            expected_index.total_rows(),
            expected_projection.visible_len(),
        )
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = app_state_with_repo(updated_repo.clone(), repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "giant two-way resync applied manual session edit",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&fixture.file_rel)
                && pane.conflict_resolver.conflict_rev == expected_rev
                && crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ) == expected_conflict_count
        },
        |pane| {
            format!(
                "path={:?} conflict_rev={} conflicts={} visible_len={} split_rows={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.conflict_rev,
                crate::view::conflict_resolver::conflict_count(
                    &pane.conflict_resolver.marker_segments,
                ),
                pane.conflict_resolver.two_way_split_visible_len(),
                pane.conflict_resolver
                    .split_row_index()
                    .map(|index| index.total_rows())
                    .unwrap_or_default(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);

                assert_eq!(
                    pane.conflict_resolver.rendering_mode(),
                    crate::view::conflict_resolver::ConflictRenderingMode::StreamedLargeFile,
                    "large fixture should remain in streamed large-file mode after re-sync",
                );
                assert_eq!(
                    crate::view::conflict_resolver::conflict_count(
                        &pane.conflict_resolver.marker_segments,
                    ),
                    expected_conflict_count,
                    "manual session edit should materialize one conflict block into text during re-sync",
                );
                assert_eq!(
                    pane.conflict_resolver.conflict_region_indices.len(),
                    expected_conflict_count,
                    "visible region indices should shrink with the remaining conflict blocks",
                );

                let index = pane
                    .conflict_resolver
                    .split_row_index()
                    .expect("re-sync should rebuild the giant split row index");
                assert_eq!(
                    index.total_rows(),
                    expected_total_rows,
                    "split row index should be rebuilt from the updated marker structure",
                );
                assert_eq!(
                    pane.conflict_resolver.two_way_split_visible_len(),
                    expected_visible_len,
                    "two-way projection should reflect the rebuilt split index",
                );
                assert_ne!(
                    pane.conflict_resolver.two_way_split_visible_len(),
                    initial_visible_len,
                    "manual materialization should change the visible giant split layout",
                );

                assert!(
                    index.first_row_for_conflict(expected_conflict_count).is_none(),
                    "rebuilt split index should drop the removed conflict block entirely",
                );
                let first_conflict_row_ix = index
                    .first_row_for_conflict(0)
                    .expect("remaining first conflict should still have rows after re-sync");
                let first_conflict_row = index
                    .row_at(
                        &pane.conflict_resolver.marker_segments,
                        first_conflict_row_ix,
                    )
                    .expect("remaining first conflict row should be generatable after re-sync");
                let row_has_shifted_conflict = first_conflict_row
                    .old
                    .as_deref()
                    .is_some_and(|text| text.contains("choice-1"))
                    || first_conflict_row
                        .new
                        .as_deref()
                        .is_some_and(|text| text.contains("choice-1"));
                assert!(
                    row_has_shifted_conflict,
                    "re-synced first remaining conflict row should now point at the old second block",
                );
                let first_conflict_visible_ix = pane
                    .conflict_resolver
                    .two_way_split_projection()
                    .and_then(|projection| projection.source_to_visible(first_conflict_row_ix));
                assert!(
                    first_conflict_visible_ix
                        .and_then(|visible_ix| {
                            pane.conflict_resolver.two_way_split_visible_row(visible_ix)
                        })
                        .is_some(),
                    "rebuilt projection should resolve the shifted first-conflict row as visible",
                );
            });
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn large_conflict_resolved_output_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_conflict_resolved_output_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/large_conflict_resolved_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let comment_line = "still inside block comment";
    let fixture_line_count = 20_001usize;

    let mut base_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
        "let chosen = 0;".to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count).map(|ix| format!("let base_bg_{ix}: usize = {ix};")),
    );
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[3] = "let chosen = 1;".to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[3] = "let chosen = 2;".to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
        "<<<<<<< ours".to_string(),
        "let chosen = 1;".to_string(),
        "=======".to_string(),
        "let chosen = 2;".to_string(),
        ">>>>>>> theirs".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count)
            .map(|ix| format!("let resolved_bg_{ix}: usize = {ix};")),
    );
    let current_text = current_lines.join("\n");
    let resolved_output = crate::view::conflict_resolver::generate_resolved_text(
        crate::view::conflict_resolver::parse_conflict_markers(&current_text).as_slice(),
    );
    let line_count = resolved_output.lines().count();
    assert!(
        fixture_line_count > rows::MAX_LINES_FOR_SYNTAX_HIGHLIGHTING,
        "fixture should stay above the old syntax gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict resolver fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict resolver fixture");

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
        "large conflict resolved output initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| {
            format!(
                "path={:?} line_count={} syntax_language={:?} prepared_document={:?} source_hash={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_syntax_language,
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_source_hash,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
                assert_eq!(
                    pane.conflict_resolved_preview_line_count, line_count,
                    "forced recompute should materialize the expected resolved output line count"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_syntax_language,
                    Some(rows::DiffSyntaxLanguage::Rust),
                    "resolved output should still use the file-derived Rust syntax language"
                );
                assert!(
                    pane.conflict_resolved_preview_prepared_syntax_document.is_none(),
                    "zero foreground budget should leave resolved-output syntax pending until the background parse completes"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolved_output_projection
                .as_ref()
                .and_then(|projection| {
                    projection.line_text(&pane.conflict_resolver.marker_segments, target_ix)
                })
                .expect("streamed preview should expose the requested resolved-output line")
                .as_ref(),
            comment_line,
            "expected the streamed resolved-output row to match the multiline comment text"
        );
        assert!(
            pane.conflict_resolved_output_projection.is_some(),
            "large-mode bootstrap should keep the resolved output in streamed projection mode"
        );
        assert!(
            pane.conflict_resolved_preview_segments_cache_get(target_ix).is_none(),
            "streamed resolved-output rows should bypass the materialized syntax row cache"
        );
        assert!(
            pane.conflict_resolved_preview_prepared_syntax_document.is_none(),
            "streamed resolved-output preview should not prepare a full syntax document before materialization"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_resolved_output_materialized(cx);
                assert!(
                    pane.conflict_resolved_output_projection.is_none(),
                    "explicit materialization should drop the streamed projection immediately"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_line_count, line_count,
                    "materialized preview should preserve the streamed output line count"
                );
                assert_eq!(
                    pane.conflict_resolved_preview_syntax_language,
                    Some(rows::DiffSyntaxLanguage::Rust),
                    "materialized resolved output should still keep the path-derived syntax language"
                );
                assert!(
                    pane.conflict_resolved_preview_prepared_syntax_document.is_none(),
                    "zero foreground budget should keep syntax preparation deferred immediately after materialization"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.conflict_resolved_output_projection.is_none(),
            "explicit materialization should drop the streamed projection"
        );
        assert_eq!(
            pane.conflict_resolved_preview_line_count, line_count,
            "materialized preview should preserve the streamed output line count"
        );
        assert_eq!(
            pane.conflict_resolved_preview_syntax_language,
            Some(rows::DiffSyntaxLanguage::Rust),
            "materialized resolved output should still keep the path-derived syntax language"
        );
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("materialized output draw should populate the visible fallback row cache");
        assert_eq!(
            styled.text.as_ref(),
            comment_line,
            "materialized row cache should preserve the expected resolved-output text"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict resolver fixture");
}

#[gpui::test]
fn edited_conflict_resolved_output_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_edited_conflict_resolved_output_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("src/edited_conflict_resolved_bg.rs");
    let abs_path = workdir.join(&file_rel);
    let inserted_comment_line = "still inside block comment";
    let inserted_prefix = format!("/* start block comment\n{inserted_comment_line}\nend */\n");
    let fixture_line_count = 20_001usize;

    let mut base_lines = vec![
        "fn large_demo() {".to_string(),
        "    let chosen = 0;".to_string(),
        "    let tail = 9;".to_string(),
        "}".to_string(),
    ];
    base_lines.extend(
        (base_lines.len()..fixture_line_count).map(|ix| format!("let base_bg_{ix}: usize = {ix};")),
    );
    let base_text = base_lines.join("\n");

    let mut ours_lines = base_lines.clone();
    ours_lines[1] = "    let chosen = 1;".to_string();
    let ours_text = ours_lines.join("\n");

    let mut theirs_lines = base_lines.clone();
    theirs_lines[1] = "    let chosen = 2;".to_string();
    let theirs_text = theirs_lines.join("\n");

    let mut current_lines = vec![
        "fn large_demo() {".to_string(),
        "<<<<<<< ours".to_string(),
        "    let chosen = 1;".to_string(),
        "=======".to_string(),
        "    let chosen = 2;".to_string(),
        ">>>>>>> theirs".to_string(),
        "    let tail = 9;".to_string(),
        "}".to_string(),
    ];
    current_lines.extend(
        (current_lines.len()..fixture_line_count)
            .map(|ix| format!("let resolved_bg_{ix}: usize = {ix};")),
    );
    let current_text = current_lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture file parent"))
        .expect("create conflict resolver fixture dir");
    std::fs::write(&abs_path, &current_text).expect("write conflict resolver fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
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
        "edited conflict resolved output initialized",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolver.path.as_ref() == Some(&file_rel),
        |pane| {
            format!(
                "path={:?} line_count={} prepared_document={:?} source_hash={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_source_hash,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_conflict_resolved_output_materialized(cx);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output materialized for editing",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| pane.conflict_resolved_output_projection.is_none(),
        |pane| {
            format!(
                "projection_present={} line_count={} prepared_document={:?}",
                pane.conflict_resolved_output_projection.is_some(),
                pane.conflict_resolved_preview_line_count,
                pane.conflict_resolved_preview_prepared_syntax_document,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.recompute_conflict_resolved_outline_for_tests(cx);
                pane.conflict_resolver.resolver_pending_recompute_seq = pane
                    .conflict_resolver
                    .resolver_pending_recompute_seq
                    .wrapping_add(1);
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output initial syntax ready",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some()
                && pane.conflict_resolved_preview_syntax_language
                    == Some(rows::DiffSyntaxLanguage::Rust)
        },
        |pane| {
            format!(
                "prepared_document={:?} style_epoch={} syntax_language={:?} line_count={}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_syntax_language,
                pane.conflict_resolved_preview_line_count,
            )
        },
    );

    let initial_epoch = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some(),
            "initial recompute should build a prepared syntax document before the edit"
        );
        pane.conflict_resolved_preview_style_cache_epoch
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
                pane.conflict_resolver_input.update(cx, |input, cx| {
                    input.replace_utf8_range(0..0, &inserted_prefix, cx);
                });
            });
        });
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output falls back to plain text while background syntax reparses",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_text
                .as_ref()
                .starts_with(inserted_prefix.as_str())
                && pane
                    .conflict_resolved_preview_prepared_syntax_document
                    .is_none()
                && pane.conflict_resolved_preview_style_cache_epoch > initial_epoch
        },
        |pane| {
            let preview_prefix: Vec<&str> = pane
                .conflict_resolved_preview_text
                .as_ref()
                .lines()
                .take(3)
                .collect();
            format!(
                "preview_prefix={preview_prefix:?} prepared_document={:?} style_epoch={} initial_epoch={initial_epoch} inflight={:?}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_syntax_inflight,
            )
        },
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    let (pending_epoch, pending_highlights_hash) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("edit redraw should populate the visible fallback resolved-output row cache");
        assert_eq!(
            styled.text.as_ref(),
            inserted_comment_line,
            "expected the cached resolved-output row to reflect the inserted comment continuation line"
        );
        assert!(
            styled.highlights.is_empty(),
            "while the edited document reparses in the background, the continuation row should render as plain text"
        );
        (
            pane.conflict_resolved_preview_style_cache_epoch,
            styled.highlights_hash,
        )
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "edited conflict resolved output background syntax upgrade",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.conflict_resolved_preview_prepared_syntax_document
                .is_some()
                && pane.conflict_resolved_preview_style_cache_epoch > pending_epoch
                && pane
                    .conflict_resolved_preview_segments_cache_get(target_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == inserted_comment_line
                            && styled.highlights.iter().any(|(range, style)| {
                                range.start == 0
                                    && range.end == inserted_comment_line.len()
                                    && style.color == Some(pane.theme.colors.text_muted.into())
                            })
                    })
        },
        |pane| {
            let row_cache = pane
                .conflict_resolved_preview_segments_cache_get(target_ix)
                .map(styled_debug_info_with_styles);
            format!(
                "prepared_document={:?} style_epoch={} pending_epoch={pending_epoch} inflight={:?} row_cache={row_cache:?}",
                pane.conflict_resolved_preview_prepared_syntax_document,
                pane.conflict_resolved_preview_style_cache_epoch,
                pane.conflict_resolved_preview_syntax_inflight,
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .conflict_resolved_preview_segments_cache_get(target_ix)
            .expect("background syntax completion should repopulate the edited resolved-output row cache");
        assert_ne!(
            styled.highlights_hash, pending_highlights_hash,
            "background syntax should replace the plain-text fallback row styling after the edit"
        );
        assert!(
            styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == inserted_comment_line.len()
                    && style.color == Some(pane.theme.colors.text_muted.into())
            }),
            "the inserted comment continuation row should upgrade to multiline comment highlighting after background reparsing"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict resolver fixture");
}
