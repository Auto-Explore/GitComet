use super::*;

#[gpui::test]
fn worktree_preview_ready_rows_preserve_trailing_empty_line(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let preview_path = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_trailing_empty_row.rs",
        std::process::id()
    ));
    let preview_lines = Arc::new(vec!["alpha".to_string(), "beta".to_string()]);
    let preview_text = "alpha\nbeta\n";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let preview_lines = Arc::clone(&preview_lines);
            let preview_path = preview_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_path,
                    preview_lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.worktree_preview_line_count(), Some(3));
        assert_eq!(pane.worktree_preview_line_starts.as_ref(), &[0, 6, 11]);
        assert_eq!(pane.worktree_preview_line_text(0).as_deref(), Some("alpha"));
        assert_eq!(pane.worktree_preview_line_text(1).as_deref(), Some("beta"));
        assert_eq!(pane.worktree_preview_line_text(2).as_deref(), Some(""));
    });
}

#[gpui::test]
fn file_preview_text_multi_clicks_select_word_then_line(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(902);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_file_preview_multi_clicks",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_clicks.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let preview_lines = Arc::new(vec!["alpha_beta = gamma;".to_string()]);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create preview multi-click workdir");
    std::fs::write(&preview_abs_path, preview_lines.join("\n")).expect("write preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let preview_abs_path = preview_abs_path.clone();
            let preview_lines = Arc::clone(&preview_lines);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path,
                    preview_lines,
                    "alpha_beta = gamma;".len(),
                    cx,
                );
            });
        });
    });

    let expected_line = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.diff_text_line_for_region(0, DiffTextRegion::Inline)
            .to_string()
    });
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        0,
        DiffTextRegion::Inline,
        1..5,
        "file preview multi-click hitbox",
    );
    let expected_word = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let offset = pane
            .diff_text_offset_for_position_for_tests(0, DiffTextRegion::Inline, click)
            .expect("expected file-preview diff text offset");
        let word_range = crate::text_selection::token_range_for_offset(&expected_line, offset);
        expected_line[word_range].to_string()
    });

    simulate_counted_click(cx, click, 2);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.copy_selected_diff_text_to_clipboard(cx)
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected_word)
    );

    simulate_counted_click(cx, click, 3);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.copy_selected_diff_text_to_clipboard(cx)
        });
    });
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(expected_line)
    );

    simulate_counted_click(cx, click, 1);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_text_has_selection(),
            "single click should clear the file preview text selection"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup preview multi-click fixture");
}

#[gpui::test]
fn file_preview_renders_scrollable_syntax_highlighted_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(1);
    let workdir = std::env::temp_dir().join(format!("gitcomet_ui_test_{}", std::process::id()));
    let file_rel = std::path::PathBuf::from("preview.rs");
    let lines: Arc<Vec<String>> = Arc::new(
        (0..300)
            .map(|_| {
                "fn main() { let x = 1; } // this line is intentionally long to force horizontal overflow in preview rows........................................".to_string()
            })
            .collect(),
    );
    let preview_text = lines.join("\n");

    // Create the file on disk so is_file_preview_active() can detect it.
    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(workdir.join(&file_rel), &preview_text).expect("write preview fixture file");

    // Push state through the model first; the observer will clear stale
    // worktree_preview on diff-target change.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    // Set preview data in a separate update so it runs after the observer
    // has cleared the stale preview state.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let workdir = workdir.clone();
            let file_rel = file_rel.clone();
            let lines = Arc::clone(&lines);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    workdir.join(&file_rel),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "file preview first visible row syntax cache",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            let max_offset = pane
                .worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .max_offset();
            max_offset.y > px(0.0)
                && max_offset.x > px(0.0)
                && pane
                    .worktree_preview_segments_cache_get(0)
                    .is_some_and(|styled| !styled.highlights.is_empty())
        },
        |pane| {
            let max_offset = pane
                .worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .max_offset();
            let row_cache = pane
                .worktree_preview_segments_cache_get(0)
                .map(styled_debug_info_with_styles);
            format!(
                "max_offset={max_offset:?} style_epoch={} cache_path={:?} row_cache={row_cache:?}",
                pane.worktree_preview_style_cache_epoch, pane.worktree_preview_segments_cache_path,
            )
        },
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn html_file_preview_renders_injected_javascript_and_css_from_real_document(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(75);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_html_preview_injections",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_injections.html");
    let preview_abs_path = workdir.join(&file_rel);
    let script_line = "const previewValue = 7;";
    let style_line = "color: red;";
    let script_line_ix = 1usize;
    let style_line_ix = 4usize;
    let lines: Arc<Vec<String>> = Arc::new(vec![
        "<script>".to_string(),
        script_line.to_string(),
        "</script>".to_string(),
        "<style>".to_string(),
        style_line.to_string(),
        "</style>".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create HTML preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write HTML preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML preview injected syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Html)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(script_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == script_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..5)
                            && highlights_include_range(styled.highlights.as_ref(), 21..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..5)
                    })
        },
        |pane| {
            let script_cached = pane
                .worktree_preview_segments_cache_get(script_line_ix)
                .map(styled_debug_info);
            let style_cached = pane
                .worktree_preview_segments_cache_get(style_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} language={:?} prepared={:?} script_cached={script_cached:?} style_cached={style_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup HTML preview fixture");
}

#[gpui::test]
fn html_file_preview_renders_injected_attribute_syntax_from_real_document(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(76);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_html_preview_attribute_injections",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_attribute_injections.html");
    let preview_abs_path = workdir.join(&file_rel);
    let onclick_line = r#"<button onclick="const value = 1;">go</button>"#;
    let style_line = r#"<div style="color: red; display: block">ok</div>"#;
    let onclick_line_ix = 0usize;
    let style_line_ix = 1usize;
    let lines: Arc<Vec<String>> = Arc::new(vec![onclick_line.to_string(), style_line.to_string()]);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create HTML attribute preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write HTML attribute preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "HTML preview attribute injection syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Html)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(onclick_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == onclick_line
                            && highlights_include_range(styled.highlights.as_ref(), 17..22)
                            && highlights_include_range(styled.highlights.as_ref(), 31..32)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_ref(), 12..17)
                            && highlights_include_range(styled.highlights.as_ref(), 24..31)
                    })
        },
        |pane| {
            let onclick_cached = pane
                .worktree_preview_segments_cache_get(onclick_line_ix)
                .map(styled_debug_info);
            let style_cached = pane
                .worktree_preview_segments_cache_get(style_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} language={:?} prepared={:?} onclick_cached={onclick_cached:?} style_cached={style_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup HTML attribute preview fixture");
}

#[gpui::test]
fn added_file_preview_text_file_materializes_and_uses_prepared_syntax_highlighting(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(77);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_added_preview_prepared_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("added_preview.html");
    let preview_abs_path = workdir.join(&file_rel);
    let preview_source_path = workdir.join(".added_preview_source.html");
    let script_line = "const previewValue = 7;";
    let style_line = "color: red;";
    let script_line_ix = 1usize;
    let style_line_ix = 4usize;
    let preview_text = [
        "<script>",
        script_line,
        "</script>",
        "<style>",
        style_line,
        "</style>",
    ]
    .join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create added preview prepared syntax workdir");
    std::fs::write(&preview_source_path, &preview_text)
        .expect("write added preview prepared syntax fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Error(
                "materialized diff_file should not be consulted for added preview prepared syntax"
                    .into(),
            );
            repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: preview_source_path.clone(),
                    side: gitcomet_core::domain::DiffPreviewTextSide::New,
                }),
            ));
            repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "added preview text-file prepared syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_source_path.as_ref() == Some(&preview_source_path)
                && pane.worktree_preview_text.as_ref() == preview_text
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Html)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(script_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == script_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..5)
                            && highlights_include_range(styled.highlights.as_ref(), 21..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..5)
                    })
        },
        |pane| {
            let script_cached = pane
                .worktree_preview_segments_cache_get(script_line_ix)
                .map(styled_debug_info);
            let style_cached = pane
                .worktree_preview_segments_cache_get(style_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} source_path={:?} text_len={} language={:?} prepared={:?} script_cached={script_cached:?} style_cached={style_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_source_path.clone(),
                pane.worktree_preview_text.len(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup added preview prepared syntax fixture");
}

#[gpui::test]
fn deleted_file_preview_text_file_materializes_and_uses_prepared_syntax_highlighting(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(78);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_deleted_preview_prepared_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("deleted_preview.html");
    let preview_abs_path = workdir.join(&file_rel);
    let preview_source_path = workdir.join(".deleted_preview_source.html");
    let script_line = "const removedValue = 9;";
    let style_line = "color: blue;";
    let script_line_ix = 1usize;
    let style_line_ix = 4usize;
    let preview_text = [
        "<script>",
        script_line,
        "</script>",
        "<style>",
        style_line,
        "</style>",
    ]
    .join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create deleted preview prepared syntax workdir");
    std::fs::write(&preview_source_path, &preview_text)
        .expect("write deleted preview prepared syntax fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Deleted,
                gitcomet_core::domain::DiffArea::Staged,
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Error(
                "materialized diff_file should not be consulted for deleted preview prepared syntax"
                    .into(),
            );
            repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: preview_source_path.clone(),
                    side: gitcomet_core::domain::DiffPreviewTextSide::Old,
                }),
            ));
            repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "deleted preview text-file prepared syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_source_path.as_ref() == Some(&preview_source_path)
                && pane.worktree_preview_text.as_ref() == preview_text
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Html)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(script_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == script_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..5)
                            && highlights_include_range(styled.highlights.as_ref(), 21..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..5)
                    })
        },
        |pane| {
            let script_cached = pane
                .worktree_preview_segments_cache_get(script_line_ix)
                .map(styled_debug_info);
            let style_cached = pane
                .worktree_preview_segments_cache_get(style_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} source_path={:?} text_len={} language={:?} prepared={:?} script_cached={script_cached:?} style_cached={style_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_source_path.clone(),
                pane.worktree_preview_text.len(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup deleted preview prepared syntax fixture");
}

#[gpui::test]
fn large_file_preview_keeps_prepared_syntax_document_above_old_line_gate(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(52);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_preview_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("large_preview.rs");
    let line_count = 4_001usize;
    let lines: Arc<Vec<String>> = Arc::new(
        (0..line_count)
            .map(|ix| format!("let preview_value_{ix}: usize = {ix};"))
            .collect(),
    );
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create large preview workdir");
    std::fs::write(workdir.join(&file_rel), &preview_text).expect("write large preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let workdir = workdir.clone();
            let file_rel = file_rel.clone();
            let lines = Arc::clone(&lines);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    workdir.join(&file_rel),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "large file preview prepared syntax document",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_line_count() == Some(line_count)
                && pane.worktree_preview_text.len() == preview_text.len()
                && pane.worktree_preview_line_starts.len() == line_count
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Rust)
                && pane.worktree_preview_prepared_syntax_document().is_some()
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?} preview_path={:?} line_count={:?} text_len={} line_starts={} syntax_language={:?} prepared_document={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_line_count(),
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_starts.len(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup large preview fixture");
}

#[gpui::test]
fn oversized_json_preview_uses_visible_line_fallback_without_prepared_syntax_document(
    cx: &mut gpui::TestAppContext,
) {
    const OBJECT_COUNT: usize = 512;
    const PAYLOAD_BYTES: usize = 16 * 1024;
    const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(81);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_oversized_json_preview_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("oversized_preview.json");
    let preview_abs_path = workdir.join(&file_rel);
    let lines_vec = build_large_json_array_lines(OBJECT_COUNT, PAYLOAD_BYTES);
    let target_line_ix = 1usize;
    let target_line = lines_vec[target_line_ix].clone();
    let line_count = lines_vec.len();
    let preview_text = lines_vec.join("\n");
    let lines: Arc<Vec<String>> = Arc::new(lines_vec);

    assert!(
        line_count < 4_001,
        "fixture should stay below the old line-count gate so this test specifically exercises the new byte gate"
    );
    assert!(
        preview_text.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "fixture should exceed the prepared-document byte gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create oversized JSON preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write oversized JSON preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "oversized JSON preview heuristic syntax fallback",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_line_count() == Some(line_count)
                && pane.worktree_preview_text.len() == preview_text.len()
                && pane.worktree_preview_text.len() > PREPARED_DOCUMENT_MAX_BYTES
                && pane.worktree_preview_line_starts.len() == line_count
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Json)
                && pane.worktree_preview_prepared_syntax_document().is_none()
                && pane
                    .worktree_preview_segments_cache_get(target_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == target_line && !styled.highlights.is_empty()
                    })
        },
        |pane| {
            let row_cache = pane
                .worktree_preview_segments_cache_get(target_line_ix)
                .map(styled_debug_info_with_styles);
            format!(
                "preview_path={:?} line_count={:?} text_len={} language={:?} prepared_document={:?} row_cache={row_cache:?}",
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_line_count(),
                pane.worktree_preview_text.len(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .worktree_preview_segments_cache_get(target_line_ix)
            .expect("oversized JSON preview should cache the visible fallback row");
        assert!(
            pane.worktree_preview_prepared_syntax_document().is_none(),
            "oversized JSON preview should stay on the visible-line fallback path"
        );
        assert!(
            !styled.highlights.is_empty(),
            "oversized JSON preview should still render heuristic syntax highlights for visible rows"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup oversized JSON preview fixture");
}

#[gpui::test]
fn minified_json_preview_streams_visible_slice_for_giant_line(cx: &mut gpui::TestAppContext) {
    const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;
    const PAYLOAD_BYTES: usize = PREPARED_DOCUMENT_MAX_BYTES + 256 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(91);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_preview_streamed",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("streamed_preview.json");
    let preview_abs_path = workdir.join(&file_rel);
    let long_json = format!(
        r#"{{"title":"Ä","needle":"preview-streamed","payload":"{}","tail":true}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let lines: Arc<Vec<String>> = Arc::new(vec![long_json.clone()]);

    assert!(
        long_json.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "fixture should exceed the prepared-document byte gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed preview workdir");
    std::fs::write(&preview_abs_path, &long_json).expect("write streamed preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );
            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, long_json.len(), cx);
            });
        });
    });

    cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "streamed minified preview horizontal overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_text.len() == long_json.len()
                && pane.worktree_preview_text.len() > PREPARED_DOCUMENT_MAX_BYTES
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Json)
                && pane.worktree_preview_prepared_syntax_document().is_none()
                && pane
                    .worktree_preview_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x
                    > px(0.0)
        },
        |pane| {
            format!(
                "preview_path={:?} text_len={} language={:?} prepared_document={:?} max_offset={:?}",
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_text.len(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
                pane.worktree_preview_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let hitbox = pane
            .diff_text_hitboxes
            .get(&(0, DiffTextRegion::Inline))
            .expect("streamed preview row should install a diff hitbox");
        assert!(
            hitbox.streamed_ascii_monospace_cell_width.is_some(),
            "giant preview row should use streamed monospace hit-testing"
        );
        assert!(
            pane.worktree_preview_segments_cache_get(0).is_none(),
            "streamed preview rows should bypass the full-line styled row cache"
        );
        assert!(
            pane.worktree_preview_prepared_syntax_document().is_none(),
            "oversized minified preview should stay on the streamed heuristic fallback path"
        );

        let paint_record = rows::diff_paint_log_for_tests()
            .into_iter()
            .find(|record| record.visible_ix == 0 && record.region == DiffTextRegion::Inline)
            .expect("streamed preview draw should record the visible line paint");
        assert!(
            paint_record.text.len() < long_json.len(),
            "streamed preview should paint only a visible slice, got {} of {} bytes",
            paint_record.text.len(),
            long_json.len()
        );
        assert!(
            !paint_record.text.is_empty(),
            "streamed preview should still paint a non-empty visible slice"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let handle = pane.worktree_preview_scroll.0.borrow().base_handle.clone();
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
                "horizontally scrolled streamed preview draw should record the visible line paint",
            );
        assert!(
            paint_record.text.as_ref().starts_with('x'),
            "scrolled preview slice should start inside the payload string, got {:?}",
            &paint_record.text.as_ref()[..paint_record.text.len().min(32)]
        );
        assert!(
            paint_record
                .highlights
                .iter()
                .any(|(range, color, background)| {
                    range.start == 0 && range.end > 32 && color.is_some() && background.is_none()
                }),
            "scrolled preview slice should keep string highlighting from the payload context: {:?}",
            paint_record.highlights
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed preview fixture");
}

#[gpui::test]
fn committed_deleted_minified_utf8_json_preview_streams_from_indexed_source(
    cx: &mut gpui::TestAppContext,
) {
    const PREPARED_DOCUMENT_MAX_BYTES: usize = 8 * 1024 * 1024;
    const PAYLOAD_BYTES: usize = PREPARED_DOCUMENT_MAX_BYTES + 256 * 1024;

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(193);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_committed_deleted_streamed_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("report.json");
    let preview_source_path = workdir.join(".committed_deleted_preview_source.json");
    let commit_id = gitcomet_core::domain::CommitId("deadbeef".into());
    let long_json = format!(
        r#"{{"title":"Ä","needle":"preview-streamed","payload":"{}","tail":true}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    assert!(
        long_json.len() > PREPARED_DOCUMENT_MAX_BYTES,
        "fixture should exceed the prepared-document byte gate"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create committed deleted preview workdir");
    std::fs::write(&preview_source_path, &long_json)
        .expect("write committed deleted preview source");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(file_rel.clone()),
            });
            repo.diff_state.diff_state_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Error(
                "parsed patch diff should not be consulted for committed deleted preview".into(),
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Error(
                "materialized diff_file should not be consulted for committed deleted preview"
                    .into(),
            );
            repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: preview_source_path.clone(),
                    side: gitcomet_core::domain::DiffPreviewTextSide::Old,
                }),
            ));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "remove report".to_string(),
                    committed_at: "2026-04-07T12:00:00Z".to_string(),
                    parent_ids: vec![],
                    files: vec![gitcomet_core::domain::CommitFileChange {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Deleted,
                        is_submodule: false,
                    }],
                },
            ));
            repo.history_state.commit_details_rev = 1;

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "committed deleted streamed preview indexed source",
        |pane| {
            pane.worktree_preview_path.as_ref() == Some(&workdir.join(&file_rel))
                && pane.worktree_preview_source_path.as_ref() == Some(&preview_source_path)
                && matches!(
                    pane.worktree_preview,
                    gitcomet_state::model::Loadable::Ready(1)
                )
                && pane.worktree_preview_text.is_empty()
        },
        |pane| {
            format!(
                "preview={:?} preview_path={:?} source_path={:?} text_len={} line_count={:?}",
                pane.worktree_preview,
                pane.worktree_preview_path,
                pane.worktree_preview_source_path,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_count(),
            )
        },
    );

    cx.update(|window, app| {
        rows::clear_diff_paint_log_for_tests();
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "committed deleted streamed preview horizontal overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.worktree_preview_path.as_ref() == Some(&workdir.join(&file_rel))
                && pane.worktree_preview_source_path.as_ref() == Some(&preview_source_path)
                && pane.worktree_preview_text.is_empty()
                && pane.worktree_preview_prepared_syntax_document().is_none()
                && pane
                    .worktree_preview_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
                    .x
                    > px(0.0)
        },
        |pane| {
            format!(
                "preview_path={:?} source_path={:?} prepared_document={:?} max_offset={:?}",
                pane.worktree_preview_path,
                pane.worktree_preview_source_path,
                pane.worktree_preview_prepared_syntax_document(),
                pane.worktree_preview_scroll
                    .0
                    .borrow()
                    .base_handle
                    .max_offset()
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.worktree_preview_text.is_empty(),
            "committed deleted preview should stay file-backed"
        );
        assert!(
            pane.worktree_preview_prepared_syntax_document().is_none(),
            "committed deleted preview should stay on the streamed heuristic path"
        );
        let hitbox = pane
            .diff_text_hitboxes
            .get(&(0, DiffTextRegion::Inline))
            .expect("streamed preview row should install a diff hitbox");
        assert!(
            hitbox.streamed_ascii_monospace_cell_width.is_some(),
            "giant UTF-8 preview row should still use streamed hit-testing"
        );
        assert!(
            pane.worktree_preview_segments_cache_get(0).is_none(),
            "streamed indexed preview rows should bypass the full-line styled row cache"
        );

        let paint_record = rows::diff_paint_log_for_tests()
            .into_iter()
            .find(|record| record.visible_ix == 0 && record.region == DiffTextRegion::Inline)
            .expect("committed deleted streamed preview should record the visible line paint");
        assert!(
            paint_record.text.len() < long_json.len(),
            "committed deleted streamed preview should paint only a visible slice, got {} of {} bytes",
            paint_record.text.len(),
            long_json.len()
        );
        assert!(
            !paint_record.text.is_empty(),
            "committed deleted streamed preview should still paint a non-empty visible slice"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let handle = pane.worktree_preview_scroll.0.borrow().base_handle.clone();
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
                "horizontally scrolled committed deleted streamed preview should record the visible line paint",
            );
        assert!(
            paint_record.text.as_ref().starts_with('x'),
            "scrolled committed deleted preview slice should start inside the payload string, got {:?}",
            &paint_record.text.as_ref()[..paint_record.text.len().min(32)]
        );
        assert!(
            paint_record
                .highlights
                .iter()
                .any(|(range, color, background)| {
                    range.start == 0 && range.end > 32 && color.is_some() && background.is_none()
                }),
            "scrolled committed deleted preview slice should keep payload string highlighting: {:?}",
            paint_record.highlights
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup committed deleted preview fixture");
}

#[gpui::test]
fn minified_json_preview_partial_copy_uses_streamed_line_slice(cx: &mut gpui::TestAppContext) {
    const PAYLOAD_BYTES: usize = 256 * 1024;

    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(191);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_preview_partial_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("streamed_preview_copy.json");
    let preview_abs_path = workdir.join(&file_rel);
    let needle = "preview-streamed-copy";
    let long_json = format!(
        r#"{{"needle":"{needle}","payload":"{}","tail":true}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );
    let lines: Arc<Vec<String>> = Arc::new(vec![long_json.clone()]);
    let start = long_json
        .find(needle)
        .expect("streamed preview copy needle should exist");
    let end = start + needle.len();

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed preview copy workdir");
    std::fs::write(&preview_abs_path, &long_json).expect("write streamed preview copy fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );
            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, long_json.len(), cx);
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
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
                pane.copy_selected_diff_text_to_clipboard(cx);
            });
        });
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(needle.to_string())
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.worktree_preview_segments_cache_get(0).is_none(),
            "streamed preview partial copy should not populate the styled row cache"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed preview copy fixture");
}

#[gpui::test]
fn minified_json_preview_context_menu_copy_uses_streamed_line_target(
    cx: &mut gpui::TestAppContext,
) {
    const PAYLOAD_BYTES: usize = 96 * 1024;

    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(192);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_minified_json_preview_context_menu_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("streamed_preview_context_menu.json");
    let preview_abs_path = workdir.join(&file_rel);
    let long_json = format!(
        r#"{{"needle":"preview-streamed-context-menu","payload":"{}","tail":true}}"#,
        "x".repeat(PAYLOAD_BYTES)
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create streamed preview context-menu workdir");
    std::fs::write(&preview_abs_path, &long_json)
        .expect("write streamed preview context-menu fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Error(
                "materialized diff_file should not be consulted for streamed preview context menu"
                    .into(),
            );
            repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: preview_abs_path.clone(),
                    side: gitcomet_core::domain::DiffPreviewTextSide::New,
                }),
            ));
            repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "streamed materialized preview ready before opening preview context menu",
        |pane| {
            pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_source_path.as_ref() == Some(&preview_abs_path)
                && matches!(
                    pane.worktree_preview,
                    gitcomet_state::model::Loadable::Ready(1)
                )
                && pane.worktree_preview_text.as_ref() == long_json
        },
        |pane| {
            format!(
                "preview={:?} preview_path={:?} source_path={:?} text_len={} line_count={:?}",
                pane.worktree_preview,
                pane.worktree_preview_path,
                pane.worktree_preview_source_path,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_count(),
            )
        },
    );

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
                    panic!("expected streamed preview context menu popover");
                };

                match popover_kind {
                    PopoverKind::DiffEditorMenu {
                        copy_text,
                        copy_target,
                        ..
                    } => {
                        assert_eq!(copy_text, None);
                        assert_eq!(copy_target, Some((0, DiffTextRegion::Inline)));
                    }
                    _ => panic!("expected streamed preview diff editor menu"),
                }
            });
        });
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                let popover_kind = host
                    .popover_kind_for_tests()
                    .expect("expected streamed preview context menu popover kind");
                let model = host
                    .context_menu_model(&popover_kind, cx)
                    .expect("expected streamed preview context menu model");
                let copy_action = model.items.iter().find_map(|item| match item {
                    ContextMenuItem::Entry { label, action, .. } if label.as_ref() == "Copy" => {
                        Some((**action).clone())
                    }
                    _ => None,
                });
                match &copy_action {
                    Some(ContextMenuAction::CopyDiffText { visible_ix, region }) => {
                        assert_eq!((*visible_ix, *region), (0, DiffTextRegion::Inline));
                    }
                    _ => panic!("expected lazy CopyDiffText action for streamed preview"),
                }
                host.context_menu_activate_action(
                    copy_action.expect("copy action should exist"),
                    window,
                    cx,
                );
            });
        });
    });

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(long_json.clone())
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.worktree_preview_segments_cache_get(0).is_none(),
            "streamed preview context-menu copy should not populate the styled row cache"
        );
        assert!(
            pane.worktree_preview_text.as_ref() == long_json,
            "streamed preview context-menu copy should keep normal-sized preview text materialized"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup streamed preview context-menu fixture");
}

#[gpui::test]
fn large_file_preview_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(60);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_large_file_preview_background_syntax",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("large_preview_background.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let comment_line = "still inside block comment";
    let mut preview_lines = vec![
        "/* start block comment".to_string(),
        comment_line.to_string(),
        "end */".to_string(),
    ];
    preview_lines.extend((3..4_001).map(|ix| format!("let preview_value_{ix}: usize = {ix};")));
    let lines: Arc<Vec<String>> = Arc::new(preview_lines);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create background preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write background preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::ZERO,
                });
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
                assert!(
                    pane.worktree_preview_prepared_syntax_document().is_none(),
                    "zero foreground budget should force worktree preview syntax into the background"
                );
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let target_ix = 1usize;
    let (initial_epoch, initial_highlights_hash) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .worktree_preview_segments_cache_get(target_ix)
            .expect("initial draw should populate the visible fallback preview row cache");
        assert_eq!(
            styled.text.as_ref(),
            comment_line,
            "expected the cached preview row to match the multiline comment text"
        );
        assert!(
            styled.highlights.is_empty(),
            "before the background parse completes, the multiline comment row should render as plain text"
        );
        (pane.worktree_preview_style_cache_epoch, styled.highlights_hash)
    });

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "large file preview background syntax upgrade",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.worktree_preview_prepared_syntax_document().is_some()
                && pane.worktree_preview_style_cache_epoch > initial_epoch
                && pane
                    .worktree_preview_segments_cache_get(target_ix)
                    .is_some_and(|styled| {
                        styled.highlights.iter().any(|(range, style)| {
                            range.start == 0
                                && range.end == comment_line.len()
                                && style.color == Some(pane.theme.syntax.comment.into())
                        })
                    })
        },
        |pane| {
            let row_cache = pane
                .worktree_preview_segments_cache_get(target_ix)
                .map(styled_debug_info_with_styles);
            format!(
                "prepared_document={:?} style_epoch={} cache_path={:?} row_cache={row_cache:?}",
                pane.worktree_preview_prepared_syntax_document(),
                pane.worktree_preview_style_cache_epoch,
                pane.worktree_preview_segments_cache_path.clone(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let styled = pane
            .worktree_preview_segments_cache_get(target_ix)
            .expect("background syntax completion should repopulate the preview row cache");
        assert_ne!(
            styled.highlights_hash, initial_highlights_hash,
            "background syntax should replace the plain-text fallback row styling"
        );
        assert!(
            styled.highlights.iter().any(|(range, style)| {
                range.start == 0
                    && range.end == comment_line.len()
                    && style.color == Some(pane.theme.syntax.comment.into())
            }),
            "multiline comment row should upgrade to comment highlighting after background parsing"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup background preview fixture");
}

#[gpui::test]
fn xml_file_preview_renders_syntax_highlights_from_real_document(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(78);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_xml_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("config.xml");
    let preview_abs_path = workdir.join(&file_rel);
    let tag_line = r#"<server port="8080">"#;
    let comment_line = "<!-- configuration -->";
    let tag_line_ix = 1usize;
    let comment_line_ix = 0usize;
    let lines: Arc<Vec<String>> = Arc::new(vec![
        comment_line.to_string(),
        tag_line.to_string(),
        "  <name>app</name>".to_string(),
        "</server>".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create XML preview workdir");
    std::fs::write(&preview_abs_path, &preview_text).expect("write XML preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Added,
                gitcomet_core::domain::DiffArea::Staged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, preview_abs_path, lines, preview_text.len(), cx);
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "XML preview syntax render",
        |pane| {
            pane.is_file_preview_active()
                && pane.worktree_preview_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_syntax_language == Some(rows::DiffSyntaxLanguage::Xml)
                && pane.worktree_preview_prepared_syntax_document().is_some()
                && pane
                    .worktree_preview_segments_cache_get(comment_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == comment_line
                            && highlights_include_range(styled.highlights.as_ref(), 0..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(tag_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == tag_line
                            && highlights_include_range(styled.highlights.as_ref(), 1..7)
                            && highlights_include_range(styled.highlights.as_ref(), 8..12)
                    })
        },
        |pane| {
            let comment_cached = pane
                .worktree_preview_segments_cache_get(comment_line_ix)
                .map(styled_debug_info);
            let tag_cached = pane
                .worktree_preview_segments_cache_get(tag_line_ix)
                .map(styled_debug_info);
            format!(
                "active={} preview_path={:?} language={:?} prepared={:?} comment_cached={comment_cached:?} tag_cached={tag_cached:?}",
                pane.is_file_preview_active(),
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_syntax_language,
                pane.worktree_preview_prepared_syntax_document(),
            )
        },
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup XML preview fixture");
}
