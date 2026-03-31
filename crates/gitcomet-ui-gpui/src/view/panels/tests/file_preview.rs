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
        assert_eq!(pane.worktree_preview_line_text(0), Some("alpha"));
        assert_eq!(pane.worktree_preview_line_text(1), Some("beta"));
        assert_eq!(pane.worktree_preview_line_text(2), Some(""));
    });
}

#[gpui::test]
fn file_preview_renders_scrollable_syntax_highlighted_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
            max_offset.height > px(0.0)
                && max_offset.width > px(0.0)
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
    disable_view_poller_for_test(cx, &view);

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
    disable_view_poller_for_test(cx, &view);

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
fn large_file_preview_keeps_prepared_syntax_document_above_old_line_gate(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
fn large_file_preview_renders_plain_text_then_upgrades_after_background_syntax(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
                                && style.color == Some(pane.theme.colors.text_muted.into())
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
                    && style.color == Some(pane.theme.colors.text_muted.into())
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
    disable_view_poller_for_test(cx, &view);

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
