use super::*;
use palette::IntoColor;

#[gpui::test]
fn source_backed_pair_text_tracks_accepted_file_diff_generation(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(881);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_source_backed_pair_generation",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create source-backed generation fixture");
    let path = std::path::PathBuf::from("src/pair_generation.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");
    let base_text = "fn base() { let value = 0; }\n";
    let first_text = "fn first() { let value = 1; }\n";
    let second_text = "fn second() { let value = 2; }\n";

    let push_generation = |cx: &mut gpui::VisualTestContext,
                           revision: u64,
                           old_text: &str,
                           new_text: &str,
                           old_identity: &str,
                           new_identity: &str| {
        std::fs::write(&old_source_path, old_text).expect("write old source generation");
        std::fs::write(&new_source_path, new_text).expect("write new source generation");
        let unified = format!(
            "@@ -1 +1 @@\n-{}\n+{}\n",
            old_text.trim_end(),
            new_text.trim_end(),
        );
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                let target = repo
                    .diff_state
                    .diff_target
                    .clone()
                    .expect("test file status should select a diff target");
                repo.diff_state.diff_rev = revision;
                repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                    gitcomet_core::domain::Diff::from_unified(target, &unified),
                ));
                repo.diff_state.diff_file_rev = revision;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText::new_sources(
                        path.clone(),
                        Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                            old_source_path.clone(),
                            old_identity,
                        )),
                        Some(gitcomet_core::domain::FileDiffTextSource::with_identity(
                            new_source_path.clone(),
                            new_identity,
                        )),
                    ),
                )));
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            });
        });
    };

    push_generation(cx, 1, base_text, first_text, "old-1", "new-1");
    wait_for_main_pane_condition(
        cx,
        &view,
        "first source-backed file-diff generation",
        |pane| pane.file_diff_cache_rev == 1 && pane.file_diff_cache_inflight.is_none(),
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.file_diff_pair_syntax_document(DiffTextRegion::SplitRight)
                    .expect("the first source generation should parse on click");
                assert_eq!(
                    pane.file_diff_pair_syntax_text
                        .get(&DiffTextRegion::SplitRight)
                        .map(AsRef::as_ref),
                    Some(first_text),
                );
            });
        });
    });

    push_generation(cx, 2, first_text, second_text, "old-2", "new-2");
    wait_for_main_pane_condition(
        cx,
        &view,
        "second source-backed file-diff generation",
        |pane| pane.file_diff_cache_rev == 2 && pane.file_diff_cache_inflight.is_none(),
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.file_diff_pair_syntax_document(DiffTextRegion::SplitRight)
                    .expect("the replacement source generation should parse on click");
                assert_eq!(
                    pane.file_diff_pair_syntax_text
                        .get(&DiffTextRegion::SplitRight)
                        .map(AsRef::as_ref),
                    Some(second_text),
                    "retained syntax text must belong to the installed generation",
                );
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup source-backed generation fixture");
}

/// Source-backed sides have no resident full text. A syntax-sized side above
/// the small synchronous-click allowance must still become interactive, but its
/// disk read and full parse must happen after the input event rather than
/// blocking it.
#[gpui::test]
fn source_backed_diff_click_syntax_prepares_a_two_megabyte_document_off_thread(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(882);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_source_backed_large_click_syntax",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create large click-syntax fixture");
    let path = std::path::PathBuf::from("src/large_click_syntax.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");

    let padding_line = "// source-backed click-syntax padding ........................................................\n";
    let old_foreground_completion_ceiling = 1024usize * 1024;
    let target_bytes = old_foreground_completion_ceiling
        .saturating_mul(2)
        .saturating_add(16 * 1024);
    let padding_count = target_bytes.div_ceil(padding_line.len());
    let prefix = padding_line.repeat(padding_count);
    let old_target = "fn target() { let target_value = 1; target_value }\n";
    let new_target = "fn target() { let target_value = 2; target_value }\n";
    let old_text = format!("{prefix}{old_target}");
    let new_text = format!("{prefix}{new_target}");
    assert!(new_text.len() > old_foreground_completion_ceiling);
    assert!(new_text.len() <= rows::OCCURRENCE_MAX_TEXT_BYTES);
    std::fs::write(&old_source_path, &old_text).expect("write old large source");
    std::fs::write(&new_source_path, &new_text).expect("write new large source");

    let target_line = padding_count + 1;
    let unified = format!("@@ -{target_line} +{target_line} @@\n-{old_target}+{new_target}");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, &unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "two-megabyte source-backed file diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_text.is_empty()
                && pane.file_diff_new_source_path.is_some()
        },
        |pane| {
            format!(
                "rev={} inflight={:?} text_len={} source_path={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_new_text.len(),
                pane.file_diff_new_source_path,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                assert!(
                    pane.file_diff_pair_syntax_document(DiffTextRegion::SplitRight)
                        .is_none(),
                    "a cold multi-megabyte click must not finish its parse on the input path"
                );
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    let visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (0..pane.diff_visible_len())
            .find(|&visible_ix| {
                pane.diff_mapped_ix_for_visible_ix(visible_ix)
                    .and_then(|row_ix| pane.file_diff_inline_render_data(row_ix))
                    .is_some_and(|row| row.new_line == Some(target_line as u32))
            })
            .expect("two-megabyte target row should be visible")
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });
    cx.run_until_parked();
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        visible_ix,
        DiffTextRegion::Inline,
        18..30,
        "two-megabyte target name",
    );
    simulate_counted_click(cx, click, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "replayed click syntax for a two-megabyte source",
        |pane| {
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_some()
                && pane
                    .file_diff_pair_syntax_text
                    .get(&DiffTextRegion::SplitRight)
                    .is_some_and(|text| text.len() == new_text.len())
                && pane
                    .diff_text_occurrences_for_tests()
                    .iter()
                    .any(|(row, range)| *row == visible_ix && range.contains(&18))
        },
        |pane| {
            format!(
                "prepared={} retained_len={} inflight={:?}",
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some(),
                pane.file_diff_pair_syntax_text
                    .get(&DiffTextRegion::SplitRight)
                    .map_or(0, |text| text.len()),
                pane.file_diff_click_syntax_inflight,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let document = pane
                    .file_diff_pair_syntax_document(DiffTextRegion::SplitRight)
                    .expect("a two-megabyte source-backed side should be cached after its worker");
                let line_ix = target_line - 1;
                let pair = rows::prepared_diff_syntax_pair_at_display_offset(document, line_ix, 12)
                    .expect("clicking the target function's brace should find its pair");
                assert_eq!(
                    pair.open
                        .iter()
                        .chain(pair.close.iter())
                        .map(|span| (span.line_ix, span.display_range.clone()))
                        .collect::<Vec<_>>(),
                    vec![(line_ix, 12..13), (line_ix, 49..50)]
                );
                assert_eq!(
                    rows::prepared_diff_syntax_occurrences_at_display_offset(
                        document, line_ix, 18,
                    )
                    .iter()
                    .map(|span| (span.line_ix, span.display_range.clone()))
                    .collect::<Vec<_>>(),
                    vec![(line_ix, 18..30), (line_ix, 36..48)],
                    "a syntax-sized diff should light occurrences after background preparation"
                );
            });
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup large click-syntax fixture");
}

/// Exercise the reported file itself through row mapping, a real mouse event,
/// the source-backed parse, span projection, and canvas paint. Collapsed inline
/// is the most indirect projection; split/full behavior is covered by the
/// neighboring click tests.
#[gpui::test]
fn source_backed_syntax_rs_mouse_click_lights_syntax(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(883);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_actual_syntax_rs_click",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create actual syntax.rs click fixture");
    let path = std::path::PathBuf::from("src/view/rows/diff_text/syntax.rs");
    let new_source_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(&path);
    let old_source_path = source_dir.join("old.rs");
    let new_text = std::fs::read_to_string(&new_source_path).expect("read actual syntax.rs");
    let new_target = "fn ensure_tree_sitter_allocator() {";
    let old_target = "fn ensure_tree_sitter_allocator_old() {";
    let target_offset = new_text
        .find(new_target)
        .expect("actual syntax.rs should retain the allocator funnel");
    let target_line = new_text.as_bytes()[..target_offset]
        .iter()
        .filter(|&&byte| byte == b'\n')
        .count()
        + 1;
    let old_text = new_text.replacen(new_target, old_target, 1);
    std::fs::write(&old_source_path, old_text).expect("write old actual syntax.rs source");
    let unified = format!("@@ -{target_line} +{target_line} @@\n-{old_target}\n+{new_target}\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, &unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "actual source-backed syntax.rs file diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_text.is_empty()
                && pane.file_diff_new_source_path.as_deref() == Some(&new_source_path)
        },
        |pane| {
            format!(
                "rev={} inflight={:?} text_len={} source_path={:?}",
                pane.file_diff_cache_rev,
                pane.file_diff_cache_inflight,
                pane.file_diff_new_text.len(),
                pane.file_diff_new_source_path,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    wait_for_main_pane_condition(
        cx,
        &view,
        "actual syntax.rs collapsed inline projection",
        |pane| pane.is_collapsed_diff_projection_active() && pane.diff_view == DiffViewMode::Inline,
        |pane| {
            format!(
                "collapsed={} view={:?} visible_len={}",
                pane.is_collapsed_diff_projection_active(),
                pane.diff_view,
                pane.diff_visible_len(),
            )
        },
    );

    let visible_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (0..pane.diff_visible_len())
            .find(|&visible_ix| {
                pane.diff_mapped_ix_for_visible_ix(visible_ix)
                    .and_then(|row_ix| pane.file_diff_inline_render_data(row_ix))
                    .is_some_and(|row| row.new_line == Some(target_line as u32))
            })
            .expect("actual syntax.rs target row should be visible in the full projection")
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.scroll_diff_to_item_strict(visible_ix, gpui::ScrollStrategy::Top);
                cx.notify();
            });
        });
    });
    cx.run_until_parked();

    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        visible_ix,
        DiffTextRegion::Inline,
        3..3 + "ensure_tree_sitter_allocator".len(),
        "actual syntax.rs allocator name",
    );
    simulate_counted_click(cx, click, 1);

    let occurrences = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_text_occurrences_for_tests()
    });
    assert!(
        occurrences
            .iter()
            .any(|(row, range)| *row == visible_ix && range.contains(&3)),
        "the actual syntax.rs click should light the allocator name; occurrences={occurrences:?}"
    );

    cx.update(|_window, _app| rows::clear_diff_paint_log_for_tests());
    draw_and_drain_test_window(cx);
    let painted = rows::diff_paint_log_for_tests()
        .into_iter()
        .find(|record| record.visible_ix == visible_ix && record.region == DiffTextRegion::Inline)
        .expect("the clicked actual syntax.rs row should paint");
    assert!(
        painted
            .occurrence_quads
            .iter()
            .any(|range| range.contains(&3)),
        "the clicked allocator occurrence should reach paint; quads={:?}",
        painted.occurrence_quads,
    );

    let brace_col = new_target.find('{').expect("target has an opening brace");
    let brace_click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        visible_ix,
        DiffTextRegion::Inline,
        brace_col..brace_col + 1,
        "actual syntax.rs allocator opening brace",
    );
    simulate_counted_click(cx, brace_click, 1);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_pair_match_for_tests().is_some(),
            "clicking the actual syntax.rs function brace should light its pair"
        );
        assert!(
            pane.diff_text_local_pair_ranges(visible_ix, DiffTextRegion::Inline)
                .iter()
                .any(|range| range.contains(&brace_col)),
            "the clicked opening brace should reach the visible row"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup actual syntax.rs click fixture");
}

#[gpui::test]
fn large_file_diff_keeps_prepared_syntax_documents_above_old_line_gate(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

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
fn source_backed_file_diff_uses_auto_fallback_highlighting_in_full_and_collapsed(
    cx: &mut gpui::TestAppContext,
) {
    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        highlights: Vec<(
            std::ops::Range<usize>,
            Option<gpui::Hsla>,
            Option<gpui::Hsla>,
        )>,
    }

    fn source_text(changed_value: usize) -> String {
        let mut lines = (1..=70usize)
            .map(|line| format!("let filler_{line}: usize = {line};"))
            .collect::<Vec<_>>();
        lines[33] = "pub fn stable_context(value: usize) -> usize { value + 1 }".to_string();
        lines[34] = format!("let changed_value: usize = {changed_value};");
        format!("{}\n", lines.join("\n"))
    }

    fn unified_patch(path: &std::path::Path, old_text: &str, new_text: &str) -> String {
        let old_lines = old_text.lines().collect::<Vec<_>>();
        let new_lines = new_text.lines().collect::<Vec<_>>();
        let path = path.to_string_lossy();
        format!(
            "\
diff --git a/{path} b/{path}
index 1111111..2222222 100644
--- a/{path}
+++ b/{path}
@@ -32,7 +32,7 @@
 {}
 {}
 {}
-{}
+{}
 {}
 {}
 {}
",
            old_lines[31],
            old_lines[32],
            old_lines[33],
            old_lines[34],
            new_lines[34],
            old_lines[35],
            old_lines[36],
            old_lines[37],
        )
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

    fn syntax_snapshot(
        theme: AppTheme,
        text: &str,
        mode: rows::DiffSyntaxMode,
    ) -> LineSyntaxSnapshot {
        LineSyntaxSnapshot {
            text: text.to_string(),
            highlights: highlight_snapshot(
                rows::syntax_highlights_for_line(theme, text, rows::DiffSyntaxLanguage::Rust, mode)
                    .as_slice(),
            ),
        }
    }

    fn paint_snapshot(record: &rows::DiffPaintRecord) -> LineSyntaxSnapshot {
        LineSyntaxSnapshot {
            text: record.text.to_string(),
            highlights: record.highlights.clone(),
        }
    }

    fn split_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            pane.diff_mapped_ix_for_visible_ix(visible_ix)
                .and_then(|row_ix| pane.file_diff_split_render_data(row_ix))
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn inline_visible_ix_by_new_line(pane: &MainPaneView, new_line: u32) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            pane.diff_mapped_ix_for_visible_ix(visible_ix)
                .and_then(|row_ix| pane.file_diff_inline_render_data(row_ix))
                .is_some_and(|row| row.new_line == Some(new_line))
        })
    }

    fn visible_ix_by_new_line(
        pane: &MainPaneView,
        diff_view: DiffViewMode,
        new_line: u32,
    ) -> Option<usize> {
        match diff_view {
            DiffViewMode::Inline => inline_visible_ix_by_new_line(pane, new_line),
            DiffViewMode::Split => split_visible_ix_by_new_line(pane, new_line),
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

    fn activate_file_diff_mode(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        diff_view: DiffViewMode,
        content_mode: DiffContentMode,
        target_line: u32,
    ) {
        set_diff_content_mode_for_test(cx, view, DiffContentMode::Full);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_view = diff_view;
                    pane.clear_diff_text_style_caches();
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);

        if content_mode == DiffContentMode::Collapsed {
            set_diff_content_mode_for_test(cx, view, DiffContentMode::Collapsed);
            wait_for_main_pane_condition(
                cx,
                view,
                "source-backed collapsed projection for syntax fallback",
                |pane| {
                    pane.is_collapsed_diff_projection_active()
                        && visible_ix_by_new_line(pane, diff_view, target_line).is_some()
                },
                |pane| {
                    format!(
                        "mode={:?} view={:?} visible_len={} collapsed_rows={} target_visible={:?}",
                        pane.diff_content_mode,
                        pane.diff_view,
                        pane.diff_visible_len(),
                        pane.collapsed_diff_visible_rows.len(),
                        visible_ix_by_new_line(pane, diff_view, target_line),
                    )
                },
            );
        }
    }

    fn assert_mode_highlights(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        label: &str,
        diff_view: DiffViewMode,
        content_mode: DiffContentMode,
        region: DiffTextRegion,
        target_line: u32,
        expected: &LineSyntaxSnapshot,
    ) {
        activate_file_diff_mode(cx, view, diff_view, content_mode, target_line);
        let visible_ix = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            visible_ix_by_new_line(pane, diff_view, target_line).unwrap_or_else(|| {
                panic!(
                    "expected visible row for {label} line {target_line}; visible_len={}",
                    pane.diff_visible_len()
                )
            })
        });
        let record = draw_paint_record_for_visible_ix(cx, view, visible_ix, region);
        assert_eq!(
            paint_snapshot(&record),
            *expected,
            "{label} should use Auto syntax fallback for source-backed file rows"
        );
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(88);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_source_backed_file_diff_syntax",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create source-backed file-diff fixture dir");

    let path = std::path::PathBuf::from("src/source_backed.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");
    let old_text = source_text(1);
    let new_text = source_text(2);
    std::fs::write(&old_source_path, &old_text).expect("write old source-backed fixture");
    std::fs::write(&new_source_path, &new_text).expect("write new source-backed fixture");
    let unified = unified_patch(&path, &old_text, &new_text);
    let target_line = 34u32;
    let target_line_text = "pub fn stable_context(value: usize) -> usize { value + 1 }";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, &unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "source-backed file-diff cache without resident full text",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Rust)
                && pane.file_diff_old_text.is_empty()
                && pane.file_diff_new_text.is_empty()
                && pane.file_diff_old_line_starts.len() >= 70
                && pane.file_diff_new_line_starts.len() >= 70
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_none()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_none()
                && pane.file_diff_cache_rows.iter().any(|row| {
                    row.new_line == Some(target_line)
                        && row.kind == gitcomet_core::file_diff::FileDiffRowKind::Context
                })
        },
        |pane| {
            format!(
                "inflight={:?} path={:?} language={:?} old_text_len={} new_text_len={} old_starts={} new_starts={} left_doc={:?} right_doc={:?} rows={}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.file_diff_cache_language,
                pane.file_diff_old_text.len(),
                pane.file_diff_new_text.len(),
                pane.file_diff_old_line_starts.len(),
                pane.file_diff_new_line_starts.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                pane.file_diff_cache_rows.len(),
            )
        },
    );

    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let expected = syntax_snapshot(theme, target_line_text, rows::DiffSyntaxMode::Auto);
    let heuristic_only =
        syntax_snapshot(theme, target_line_text, rows::DiffSyntaxMode::HeuristicOnly);
    assert_ne!(
        expected, heuristic_only,
        "the source-backed syntax regression test must cover a line where Auto adds colors"
    );

    assert_mode_highlights(
        cx,
        &view,
        "full split",
        DiffViewMode::Split,
        DiffContentMode::Full,
        DiffTextRegion::SplitRight,
        target_line,
        &expected,
    );
    assert_mode_highlights(
        cx,
        &view,
        "full inline",
        DiffViewMode::Inline,
        DiffContentMode::Full,
        DiffTextRegion::Inline,
        target_line,
        &expected,
    );
    assert_mode_highlights(
        cx,
        &view,
        "collapsed split",
        DiffViewMode::Split,
        DiffContentMode::Collapsed,
        DiffTextRegion::SplitRight,
        target_line,
        &expected,
    );
    assert_mode_highlights(
        cx,
        &view,
        "collapsed inline",
        DiffViewMode::Inline,
        DiffContentMode::Collapsed,
        DiffTextRegion::Inline,
        target_line,
        &expected,
    );
}

#[gpui::test]
fn file_diff_word_highlight_caches_stay_bounded_for_sparse_deep_rows(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let deep_row = 1_000_000usize;

                assert!(pane.file_diff_inline_word_ranges(deep_row).is_empty());
                assert!(
                    pane.file_diff_split_word_ranges(deep_row, DiffTextRegion::SplitLeft)
                        .is_empty()
                );
                assert!(
                    pane.file_diff_split_word_ranges(deep_row + 1, DiffTextRegion::SplitRight)
                        .is_empty()
                );

                assert!(
                    pane.file_diff_inline_word_highlights.len() <= 1,
                    "inline word-highlight cache should be keyed sparsely, not resized to deep row"
                );
                assert!(
                    pane.file_diff_split_word_highlights.len() <= 2,
                    "split word-highlight cache should be keyed sparsely, not resized to deep row"
                );
            });
        });
    });
}

#[gpui::test]
fn oversized_json_file_diff_uses_visible_line_fallback_without_prepared_syntax_documents(
    cx: &mut gpui::TestAppContext,
) {
    const OBJECT_COUNT: usize = 512;
    const PAYLOAD_BYTES: usize = 16 * 1024;
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
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

/// Ceiling for the prepared-document path; the large-file fixtures build
/// payloads past this so the streamed path is what runs.
const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;

#[gpui::test]
fn minified_json_file_diff_streams_visible_slices_and_inline_search(cx: &mut gpui::TestAppContext) {
    const PAYLOAD_BYTES: usize = PREPARED_DOCUMENT_MAX_BYTES + 256 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
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
            assert_eq!(
                crate::view::test_support::diff_scroll_sync(view.read(app)),
                mode
            );
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
                    source_visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: start,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: 0,
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
                    source_visible_ix: 0,
                    region: DiffTextRegion::SplitLeft,
                    offset: start,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: 0,
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
                            && style.color == Some(pane.theme.syntax.comment.into_color())
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
                                    && style.color == Some(pane.theme.syntax.comment.into_color())
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
                    && style.color == Some(pane.theme.syntax.comment.into_color())
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
                            && style.color == Some(pane.theme.syntax.comment.into_color())
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
                            && style.color == Some(pane.theme.syntax.comment.into_color())
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
                                    && style.color == Some(pane.theme.syntax.comment.into_color())
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
                    && style.color == Some(pane.theme.syntax.comment.into_color())
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
                            && style.color == Some(pane.theme.syntax.comment.into_color())
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
                                && style.color == Some(pane.theme.syntax.comment.into_color())
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
                    && style.color == Some(pane.theme.syntax.comment.into_color())
            }),
            "the left comment row should be comment-highlighted after the background parse completes"
        );
    });
}

/// A click-syntax worker must leave its side unmarked however it ends.
///
/// The marker exists to stop two workers racing for one side, so it has to be
/// released whenever the task is over -- including when a rev-only refresh
/// supersedes it without replacing the visible row generation. It used to be
/// removed *after* the result guards, so that worker left the side marked and
/// `request_file_diff_click_syntax_document` drops every later click on a marked
/// side.
#[gpui::test]
fn superseded_click_syntax_worker_releases_its_side(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(884);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_superseded_click_worker",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create superseded-worker fixture");
    let path = std::path::PathBuf::from("src/superseded.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");

    // Past `DIFF_CLICK_FOREGROUND_COMPLETION_MAX_TEXT_BYTES`, so the click is
    // answered by the background worker rather than synchronously -- which is
    // the only path that takes the marker.
    let filler: String = (0..48_000)
        .map(|ix| format!("fn filler{ix}() {{ let v = {ix}; }}\n"))
        .collect();
    let old_text = format!("fn f() {{ g([zzz]); }}\n{filler}");
    let new_text = format!("fn f() {{ g([aaa]); }}\n{filler}");
    assert!(
        new_text.len() > 1024 * 1024,
        "the fixture must exceed the foreground completion ceiling, got {}",
        new_text.len()
    );
    std::fs::write(&old_source_path, &old_text).expect("write old source");
    std::fs::write(&new_source_path, &new_text).expect("write new source");
    let unified = "@@ -1 +1 @@\n-fn f() { g([zzz]); }\n+fn f() { g([aaa]); }\n".to_string();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, &unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "source-backed superseded-worker diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_source_path.as_deref() == Some(&new_source_path)
        },
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });
    // Row 0 on the right is `fn f() { g([aaa]); }`: the `[` sits at column 11.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        0,
        DiffTextRegion::SplitRight,
        11..12,
        "superseded-worker bracket hitbox",
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // A cold side: no prepared document and no retained body, so the
                // click has to go through the worker.
                pane.prepared_syntax_documents.clear();
                pane.file_diff_pair_syntax_text.clear();
                pane.begin_diff_text_selection(0, DiffTextRegion::SplitRight, click, cx);
                assert!(
                    pane.file_diff_click_syntax_inflight
                        .contains_key(&DiffTextRegion::SplitRight),
                    "a cold click on a source-backed side should take the marker"
                );

                // A same-content refresh can advance the cache rev without
                // replacing rows or their syntax generation.
                pane.file_diff_cache_rev = pane.file_diff_cache_rev.wrapping_add(1);
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane
                .file_diff_click_syntax_inflight
                .contains_key(&DiffTextRegion::SplitRight),
            "a superseded worker must release its side, or every later click on \
             it is dropped; inflight={:?}",
            pane.file_diff_click_syntax_inflight,
        );
    });

    // And the side is genuinely usable again: the next click reaches the worker
    // and resolves the pair.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.begin_diff_text_selection(0, DiffTextRegion::SplitRight, click, cx);
            });
        });
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("the click after the rebuild window should still light its pair");
        assert_eq!(
            pair.spans
                .iter()
                .map(|span| span.range.clone())
                .collect::<Vec<_>>(),
            vec![11..12, 15..16],
            "`[` and `]` of `g([aaa])`"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A superseded worker owns only the marker it acquired. If a same-file rebuild
/// clears that marker and a click in the new generation reacquires the side, the
/// old worker must not make the new worker appear absent when it completes.
#[gpui::test]
fn superseded_click_syntax_worker_preserves_a_new_generation_marker(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(888);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_click_worker_marker_ownership",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create marker-ownership fixture");
    let path = std::path::PathBuf::from("src/marker_ownership.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");
    // Keep the render path from preparing this synchronously before the test
    // can stage the two workers.
    let filler: String = (0..48_000)
        .map(|ix| format!("fn filler{ix}() {{ let v = {ix}; }}\n"))
        .collect();
    let old_text = format!("fn f() {{ g([zzz]); }}\n{filler}");
    let new_text = format!("fn f() {{ g([aaa]); }}\n{filler}");
    std::fs::write(&old_source_path, &old_text).expect("write old marker source");
    std::fs::write(&new_source_path, &new_text).expect("write new marker source");
    let unified = "@@ -1 +1 @@\n-fn f() { g([zzz]); }\n+fn f() { g([aaa]); }\n";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "source-backed marker-ownership diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_source_path.as_deref() == Some(&new_source_path)
        },
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    let new_worker_completed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let new_worker_saw_marker = Arc::new(std::sync::atomic::AtomicBool::new(false));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.prepared_syntax_documents.clear();
                pane.file_diff_pair_syntax_text.clear();
                pane.file_diff_click_syntax_before_complete_hook = None;
                pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitRight, cx);
                assert!(
                    pane.file_diff_click_syntax_inflight
                        .contains_key(&DiffTextRegion::SplitRight),
                    "the old generation should start a worker; language={:?} path={:?}",
                    pane.file_diff_cache_language,
                    pane.file_diff_new_source_path,
                );
                // A same-file rebuild installs a new generation and clears the
                // old generation's markers before the next click arrives. Both
                // workers are now queued in generation order.
                pane.file_diff_syntax_generation = pane.file_diff_syntax_generation.wrapping_add(1);
                pane.file_diff_click_syntax_inflight.clear();
                let completed = Arc::clone(&new_worker_completed);
                let saw_marker = Arc::clone(&new_worker_saw_marker);
                pane.file_diff_click_syntax_before_complete_hook = Some(Arc::new(move |pane| {
                    saw_marker.store(
                        pane.file_diff_click_syntax_inflight
                            .contains_key(&DiffTextRegion::SplitRight),
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    completed.store(true, std::sync::atomic::Ordering::SeqCst);
                }));
                pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitRight, cx);
                assert!(
                    pane.file_diff_click_syntax_inflight
                        .contains_key(&DiffTextRegion::SplitRight),
                    "the new generation should own the side"
                );
            });
        });
    });
    cx.run_until_parked();
    assert!(
        new_worker_completed.load(std::sync::atomic::Ordering::SeqCst),
        "the new worker must reach its completion callback"
    );
    assert!(
        new_worker_saw_marker.load(std::sync::atomic::Ordering::SeqCst),
        "the old worker removed the marker owned by the still-running new generation"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// Source freshness must be checked after parsing, not only around the worker's
/// file read. This hook changes the file in the otherwise-unobservable window
/// after the tree is ready and before the UI callback tries to cache it.
#[gpui::test]
fn click_syntax_worker_rejects_a_source_changed_after_parse(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(887);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_click_worker_completion_freshness",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create completion-freshness fixture");
    let path = std::path::PathBuf::from("src/completion_freshness.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");

    // Exceed the foreground allowance so request_file_diff_click_syntax_document
    // owns the parse. The completion hook then makes a same-length edit, which
    // preserves all line starts and isolates source identity as the guard.
    let filler: String = (0..48_000)
        .map(|ix| format!("fn filler{ix}() {{ let v = {ix}; }}\n"))
        .collect();
    let old_text = format!("fn f() {{ g([zzz]); }}\n{filler}");
    let indexed_text = format!("fn f() {{ g([aaa]); }}\n{filler}");
    let changed_text = format!("fn f() {{ g((aaa)); }}\n{filler}");
    assert!(indexed_text.len() > 1024 * 1024);
    assert_eq!(indexed_text.len(), changed_text.len());
    std::fs::write(&old_source_path, &old_text).expect("write old completion source");
    std::fs::write(&new_source_path, &indexed_text).expect("write indexed completion source");
    let unified = "@@ -1 +1 @@\n-fn f() { g([zzz]); }\n+fn f() { g([aaa]); }\n";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "source-backed completion-freshness diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_source_path.as_deref() == Some(&new_source_path)
        },
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    let hook_ran = Arc::new(std::sync::atomic::AtomicBool::new(false));
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.prepared_syntax_documents.clear();
                pane.file_diff_pair_syntax_text.clear();
                let hook_path = new_source_path.clone();
                let hook_text = changed_text.clone();
                let hook_ran = Arc::clone(&hook_ran);
                pane.file_diff_click_syntax_after_prepare_hook = Some(Arc::new(move || {
                    std::fs::write(&hook_path, &hook_text)
                        .expect("change source after worker parse");
                    hook_ran.store(true, std::sync::atomic::Ordering::SeqCst);
                }));
                pane.request_file_diff_click_syntax_document(DiffTextRegion::SplitRight, cx);
                assert!(
                    pane.file_diff_click_syntax_inflight
                        .contains_key(&DiffTextRegion::SplitRight),
                    "the cold source-backed side should start a click worker"
                );
            });
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            hook_ran.load(std::sync::atomic::Ordering::SeqCst),
            "the regression must mutate after a successful parse"
        );
        assert!(
            pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                .is_none(),
            "a tree parsed from the pre-edit source must not enter the cache"
        );
        assert!(
            !pane
                .file_diff_pair_syntax_text
                .contains_key(&DiffTextRegion::SplitRight),
            "the stale source allocation must not be retained"
        );
        assert!(
            !pane
                .file_diff_click_syntax_inflight
                .contains_key(&DiffTextRegion::SplitRight),
            "the completed worker must release its side"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A click must never be answered from a file the rows are no longer showing.
///
/// A source-backed side is re-read at click time and the read is retained for
/// later clicks, while the rows are per-line slices resolved on every render.
/// The only staleness guard was `line_starts_describe`, which validates where
/// the newlines are and nothing about the bytes between them -- so an edit that
/// keeps every line length passed it, and the retained body then answered later
/// clicks from text that was two edits old. The highlight landed on characters
/// that were not delimiters at all.
#[gpui::test]
fn same_shape_worktree_edit_is_not_answered_from_the_indexed_body(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(885);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_same_shape_worktree_edit",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create same-shape fixture");
    let path = std::path::PathBuf::from("src/same_shape.rs");
    let old_source_path = source_dir.join("old.rs");
    let new_source_path = source_dir.join("new.rs");

    // All three are the same length with newlines in the same places, so every
    // one of them satisfies `line_starts_describe` against the indexed starts.
    let indexed_text = "fn f() { g([aaa], (b)); }\n";
    let edited_once = "fn f() { g([a], (bbb)); }\n";
    let edited_twice = "fn f() { g([aaaaa], b); }\n";
    assert_eq!(indexed_text.len(), edited_once.len());
    assert_eq!(indexed_text.len(), edited_twice.len());

    let old_text = "fn f() { g([zzz], (b)); }\n";
    std::fs::write(&old_source_path, old_text).expect("write old source");
    std::fs::write(&new_source_path, indexed_text).expect("write new source");
    let unified = format!(
        "@@ -1 +1 @@\n-{}\n+{}\n",
        old_text.trim_end(),
        indexed_text.trim_end()
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, &unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "source-backed same-shape diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_source_path.as_deref() == Some(&new_source_path)
        },
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });
    // `fn f() { g([aaa], (b)); }`: the `[` is at column 11 and its `]` at 15.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        0,
        DiffTextRegion::SplitRight,
        11..12,
        "same-shape bracket hitbox",
    );

    let pair_ranges = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .diff_text_pair_match_for_tests()
                .map(|pair| {
                    pair.spans
                        .iter()
                        .map(|span| span.range.clone())
                        .collect::<Vec<_>>()
                })
        })
    };

    simulate_counted_click(cx, click, 1);
    cx.run_until_parked();
    assert_eq!(
        pair_ranges(cx),
        Some(vec![11..12, 15..16]),
        "against the file as indexed, the click pairs the brackets it landed on"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let document = pane
                    .file_diff_pair_syntax_document(DiffTextRegion::SplitRight)
                    .expect("the indexed source should have a prepared document");
                pane.cache_file_diff_pair_syntax_document_for_tests(
                    DiffTextRegion::SplitRight,
                    document,
                );
                assert!(
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                        .is_some(),
                    "the regression must exercise a prepared-document cache hit"
                );
            });
        });
    });

    // The worktree file changes under the open diff, keeping every line length.
    std::fs::write(&new_source_path, edited_once).expect("first same-shape edit");
    simulate_counted_click(cx, click, 1);
    cx.run_until_parked();
    assert_eq!(
        pair_ranges(cx),
        None,
        "the file is no longer the one this generation indexed, so the click \
         declines rather than answering from bytes the rows do not describe"
    );

    // And a second edit cannot be answered from the body the first click read.
    std::fs::write(&new_source_path, edited_twice).expect("second same-shape edit");
    simulate_counted_click(cx, click, 1);
    cx.run_until_parked();
    assert_eq!(
        pair_ranges(cx),
        None,
        "a retained body must not outlive the file it was read from"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// A click whose parse blows its budget defers to the worker instead of
/// finishing on the UI thread.
///
/// The budget is the whole of what a mouse press may spend in front of the user.
/// Timing out used to fall through to the same parse with no budget at all: a
/// 900 KiB C++ file spent its 50 ms, threw that away, and then held the UI
/// thread for a further 210 ms before the press returned. The click still has to
/// be answered, so this pins both halves -- nothing resolves synchronously, and
/// the pair is there once the worker lands.
#[gpui::test]
fn a_click_too_slow_to_parse_in_budget_defers_instead_of_blocking(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(886);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_click_defers_instead_of_blocking",
        std::process::id()
    ));
    let source_dir = workdir.join(".source-backed");
    std::fs::create_dir_all(&source_dir).expect("create deferral fixture");
    let path = std::path::PathBuf::from("src/deferral.cpp");
    let old_source_path = source_dir.join("old.cpp");
    let new_source_path = source_dir.join("new.cpp");

    // Templates, so the parse is slow enough to blow a 50 ms budget, and under
    // 1 MiB so the click reads it synchronously and only the *parse* defers.
    let mut filler = String::new();
    let mut ix = 0usize;
    while filler.len() < 900 * 1024 {
        filler.push_str(&format!(
            "template <typename T> struct Holder{ix} {{ T value; int id = {ix}; }};\n"
        ));
        ix += 1;
    }
    let old_text = format!("int f() {{ return g([0]); }}\n{filler}");
    let new_text = format!("int f() {{ return g([1]); }}\n{filler}");
    assert!(
        new_text.len() < 1024 * 1024,
        "the fixture must stay under the synchronous read ceiling, got {}",
        new_text.len()
    );
    std::fs::write(&old_source_path, &old_text).expect("write old source");
    std::fs::write(&new_source_path, &new_text).expect("write new source");
    let unified =
        "@@ -1 +1 @@\n-int f() { return g([0]); }\n+int f() { return g([1]); }\n".to_string();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            let target = repo
                .diff_state
                .diff_target
                .clone()
                .expect("test file status should select a diff target");
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(target, &unified),
            ));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new_sources(
                    path.clone(),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        old_source_path.clone(),
                    )),
                    Some(gitcomet_core::domain::FileDiffTextSource::new(
                        new_source_path.clone(),
                    )),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "source-backed deferral diff",
        |pane| {
            pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_new_source_path.as_deref() == Some(&new_source_path)
        },
        |pane| {
            format!(
                "rev={} inflight={:?}",
                pane.file_diff_cache_rev, pane.file_diff_cache_inflight
            )
        },
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });
    // `int f() { return g([1]); }` -- the `[` sits at column 19.
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        0,
        DiffTextRegion::SplitRight,
        19..20,
        "deferral bracket hitbox",
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                // Cold: no prepared document and no retained body.
                pane.prepared_syntax_documents.clear();
                pane.file_diff_pair_syntax_text.clear();
                pane.file_diff_click_syntax_inflight.clear();
                pane.begin_diff_text_selection(0, DiffTextRegion::SplitRight, click, cx);
                assert!(
                    pane.diff_text_pair_match_for_tests().is_none(),
                    "a parse this slow must not be finished on the UI thread"
                );
                assert!(
                    pane.diff_text_pending_syntax_click.is_some()
                        && pane
                            .file_diff_click_syntax_inflight
                            .contains_key(&DiffTextRegion::SplitRight),
                    "it must be recorded as pending with a worker in flight"
                );
            });
        });
    });

    cx.run_until_parked();
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let pair = pane
            .diff_text_pair_match_for_tests()
            .expect("the worker landing must replay the click");
        assert_eq!(
            pair.spans
                .iter()
                .map(|span| span.range.clone())
                .collect::<Vec<_>>(),
            vec![19..20, 21..22],
            "`[` and `]` of `g([1])`"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}
