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
                            && highlights_include_range(styled.highlights.as_slice(), 0..5)
                            && highlights_include_range(styled.highlights.as_slice(), 21..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_slice(), 0..5)
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
                            && highlights_include_range(styled.highlights.as_slice(), 17..22)
                            && highlights_include_range(styled.highlights.as_slice(), 31..32)
                    })
                && pane
                    .worktree_preview_segments_cache_get(style_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == style_line
                            && highlights_include_range(styled.highlights.as_slice(), 12..17)
                            && highlights_include_range(styled.highlights.as_slice(), 24..31)
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
fn patch_view_applies_syntax_highlighting_to_context_lines(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(2);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_patch", std::process::id()));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let target = gitcomet_core::domain::DiffTarget::Commit {
                commit_id: gitcomet_core::domain::CommitId("deadbeef".into()),
                path: None,
            };

            let diff = gitcomet_core::domain::Diff {
                target: target.clone(),
                lines: vec![
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Header,
                        text: "diff --git a/foo.rs b/foo.rs".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Context,
                        text: " fn main() { let x = 1; }".into(),
                    },
                ],
            };

            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(diff.into());

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let styled = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|v| v.as_ref().map(|entry| &entry.styled))
            .expect("expected context line to be syntax-highlighted and cached");
        assert!(
            !styled.highlights.is_empty(),
            "expected syntax highlighting highlights for context line"
        );
    });
}

#[gpui::test]
fn smoke_tests_diff_draw_stabilizes_without_notify_churn(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_smoke_tests_diff_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("crates/gitcomet-ui-gpui/src/smoke_tests.rs");
    let old_text = include_str!("../../../smoke_tests.rs");
    let new_text = format!("{old_text}\n// refresh-loop-regression\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                path.clone(),
                gitcomet_core::domain::FileStatusKind::Modified,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText {
                    path,
                    old: Some(old_text.to_string()),
                    new: Some(new_text),
                },
            )));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    let root_notifies = Arc::new(AtomicUsize::new(0));
    let _root_notify_sub = cx.update(|_window, app| {
        let root_notifies = Arc::clone(&root_notifies);
        view.update(app, |_this, cx| {
            cx.observe_self(move |_this, _cx| {
                root_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    let main_notifies = Arc::new(AtomicUsize::new(0));
    let main_pane = cx.update(|_window, app| view.read(app).main_pane.clone());
    let _main_notify_sub = cx.update(|_window, app| {
        let main_notifies = Arc::clone(&main_notifies);
        main_pane.update(app, |_pane, cx| {
            cx.observe_self(move |_pane, _cx| {
                main_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });

    for _ in 0..4 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    root_notifies.store(0, Ordering::Relaxed);
    main_notifies.store(0, Ordering::Relaxed);

    for _ in 0..8 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let root_notify_count = root_notifies.load(Ordering::Relaxed);
    let main_notify_count = main_notifies.load(Ordering::Relaxed);
    assert!(
        root_notify_count <= 1,
        "root view kept notifying during steady smoke_tests.rs diff draws: {root_notify_count}",
    );
    assert!(
        main_notify_count <= 1,
        "main pane kept notifying during steady smoke_tests.rs diff draws: {main_notify_count}",
    );
}

#[gpui::test]
fn file_diff_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
                    gitcomet_core::domain::FileDiffText {
                        path: path.clone(),
                        old: Some(old_text.clone()),
                        new: Some(new_text.clone()),
                    },
                )));

                let next_state = app_state_with_repo(repo, repo_id);

                push_test_state(this, Arc::clone(&next_state), cx);
            });
        });
    };

    set_state(cx, 1);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
    loop {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

        let ready = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path.is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft)
                    .is_some()
                && pane
                    .file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight)
                    .is_some()
        });
        if ready {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let snapshot = cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                (
                    pane.file_diff_cache_seq,
                    pane.file_diff_cache_inflight,
                    pane.file_diff_cache_repo_id,
                    pane.file_diff_cache_rev,
                    pane.file_diff_cache_target.clone(),
                    pane.file_diff_cache_path.clone(),
                    pane.file_diff_inline_cache.len(),
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                    pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
                    pane.active_repo().map(|repo| repo.diff_state.diff_file_rev),
                    pane.active_repo()
                        .and_then(|repo| repo.diff_state.diff_target.clone()),
                    pane.is_file_diff_view_active(),
                )
            });
            panic!("timed out waiting for initial file-diff cache build: {snapshot:?}");
        }
    }

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
                            highlights: Arc::new(vec![(
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
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();

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
                file_diff_split_cached_styled(&pane, DiffTextRegion::SplitLeft, stable_left_line)
                    .expect("identical payload refresh should preserve the cached left split row");
            let right_cached =
                file_diff_split_cached_styled(&pane, DiffTextRegion::SplitRight, stable_right_line)
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
fn file_diff_view_renders_split_and_inline_syntax_from_real_documents(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    disable_view_poller_for_test(cx, &view);

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
                && highlights_include_range(remove_styled.highlights.as_slice(), 0..6)
                && highlights_include_range(add_styled.highlights.as_slice(), 0..2)
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
                && highlights_include_range(remove_styled.highlights.as_slice(), 0..6)
                && highlights_include_range(add_styled.highlights.as_slice(), 0..2)
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
    disable_view_poller_for_test(cx, &view);

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
                && highlights_include_range(remove_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(add_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(add_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(style_styled.highlights.as_slice(), 12..17)
                && highlights_include_range(style_styled.highlights.as_slice(), 24..31)
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
                && highlights_include_range(remove_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(remove_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(add_styled.highlights.as_slice(), 17..22)
                && highlights_include_range(add_styled.highlights.as_slice(), 31..32)
                && highlights_include_range(style_styled.highlights.as_slice(), 12..17)
                && highlights_include_range(style_styled.highlights.as_slice(), 24..31)
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
    disable_view_poller_for_test(cx, &view);

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
                && highlights_include_range(remove_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(add_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 20..24)
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
                && highlights_include_range(remove_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(remove_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 1..7)
                && highlights_include_range(add_styled.highlights.as_slice(), 8..12)
                && highlights_include_range(add_styled.highlights.as_slice(), 20..24)
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
                            && highlights_include_range(styled.highlights.as_slice(), 0..22)
                    })
                && pane
                    .worktree_preview_segments_cache_get(tag_line_ix)
                    .is_some_and(|styled| {
                        styled.text.as_ref() == tag_line
                            && highlights_include_range(styled.highlights.as_slice(), 1..7)
                            && highlights_include_range(styled.highlights.as_slice(), 8..12)
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
