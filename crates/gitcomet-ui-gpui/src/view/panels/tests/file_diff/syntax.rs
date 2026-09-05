use super::*;

fn fixture_git_command(repo_root: &std::path::Path) -> std::process::Command {
    let mut command = std::process::Command::new("git");
    command
        .current_dir(repo_root)
        .args(["-c", &format!("safe.directory={}", repo_root.display())]);
    command
}

pub(super) fn fixture_git_show(repo_root: &std::path::Path, spec: &str, context: &str) -> String {
    let output = fixture_git_command(repo_root)
        .args(["show", spec])
        .output()
        .unwrap_or_else(|_| panic!("git show should run for {context}"));
    assert!(
        output.status.success(),
        "git show {spec} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("git show output should be valid UTF-8")
}

pub(super) fn fixture_git_diff(
    repo_root: &std::path::Path,
    old_spec: &str,
    new_spec: &str,
    context: &str,
) -> String {
    let output = fixture_git_command(repo_root)
        .args(["diff", old_spec, new_spec])
        .output()
        .unwrap_or_else(|_| panic!("git diff should run for {context}"));
    assert!(
        output.status.success(),
        "git diff for {context} failed: status={:?} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).expect("git diff output should be valid UTF-8")
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
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Header,
                        text: "diff --git a/page.njk b/page.njk".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Hunk,
                        text: "@@ -1,1 +1,1 @@".into(),
                    },
                    gitcomet_core::domain::DiffLine {
                        kind: gitcomet_core::domain::DiffLineKind::Context,
                        text: " <nav class=\"menu\">Home</nav>".into(),
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

        assert_eq!(
            pane.diff_language_for_src_ix.get(5).copied().flatten(),
            Some(rows::DiffSyntaxLanguage::Jinja),
            "the patch should use the same path-based language detection as file content"
        );
        let jinja_styled = pane
            .diff_text_segments_cache
            .get(5)
            .and_then(|v| v.as_ref().map(|entry| &entry.styled))
            .expect("expected Nunjucks context line to be styled and cached");
        assert!(
            !jinja_styled.highlights.is_empty(),
            "the patch should apply Jinja's injected HTML highlighting"
        );
    });
}

#[gpui::test]
fn patch_diff_text_multi_clicks_match_editor_selection_behavior(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(901);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_patch_diff_text_multi_clicks",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/multi_click.rs");
    let old_text = "alpha_beta = delta;\n";
    let new_text = "alpha_beta = gamma;\n";

    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, old_text, new_text);
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_view = DiffViewMode::Inline;
            cx.notify();
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "file diff multi-click fixture activation",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.diff_visible_len() >= 1
                && pane
                    .file_diff_inline_cache
                    .iter()
                    .any(|line| line.text.as_ref().contains("gamma"))
        },
        |pane| {
            format!(
                "cache_inflight={:?} cache_path={:?} diff_view={:?} visible_len={} inline_rows={:?}",
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.diff_view,
                pane.diff_visible_len(),
                pane.file_diff_inline_cache
                    .iter()
                    .map(|line| format!("{:?}:{}", line.kind, line.text.as_ref()))
                    .collect::<Vec<_>>(),
            )
        },
    );

    let (visible_ix, expected_line) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let visible_ix = (0..pane.diff_visible_len())
            .find(|&visible_ix| {
                let Some(inline_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                    return false;
                };
                pane.file_diff_inline_row(inline_ix)
                    .is_some_and(|line| line.text.as_ref().contains("gamma"))
            })
            .expect("expected visible file-diff row for changed line");
        let expected_line = pane
            .diff_text_line_for_region(visible_ix, DiffTextRegion::Inline)
            .to_string();
        (visible_ix, expected_line)
    });
    let click = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        visible_ix,
        DiffTextRegion::Inline,
        2..6,
        "file diff multi-click target row hitbox",
    );
    let expected_word = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let offset = pane
            .diff_text_offset_for_position(visible_ix, DiffTextRegion::Inline, click)
            .expect("expected diff text offset for click");
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
        Some(expected_word.clone())
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_selection_anchor,
            Some(visible_ix),
            "double click on diff text should update the diff focus location"
        );
        assert_eq!(
            pane.diff_selection_range, None,
            "double click on diff text should not also select the row"
        );
    });

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
            "single click should clear the text selection"
        );
        assert_eq!(
            pane.diff_selection_range, None,
            "single click used to clear text selection should not trigger row selection"
        );
    });
}

#[gpui::test]
fn yaml_commit_file_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
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

    fn quoted_scalar_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start == quote_start && range.end == text.len()).then_some(color)
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= dash_ix && range.end >= dash_ix.saturating_add(1)).then_some(color)
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

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            FileDiffRowKind,
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
            .map(|line_no| {
                let payload = split_right_cached_styled_by_new_line(pane, line_no).and_then(
                    |(_text, styled)| {
                        let kind = split_right_row_by_new_line(pane, line_no)?.kind;
                        Some((
                            kind,
                            styled.text.to_string(),
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        ))
                    },
                );
                (line_no, payload)
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            DiffLineKind,
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
            .map(|line_no| {
                let payload =
                    inline_cached_styled_by_new_line(pane, line_no).and_then(|(_text, styled)| {
                        let kind = inline_row_by_new_line(pane, line_no)?.kind;
                        Some((
                            kind,
                            styled.text.to_string(),
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        ))
                    });
                (line_no, payload)
            })
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(81);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_file_diff",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let path = std::path::PathBuf::from(".github/workflows/deployment-ci.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "YAML commit file-diff regression fixture");
    let git_diff = || {
        fixture_git_diff(
            &repo_root,
            "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml",
            "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml",
            "YAML commit file-diff regression fixture",
        )
    };
    let old_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml");
    let new_text =
        git_show("bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml");
    let unified = git_diff();

    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 33];
    let baseline_nested_key_line = 4u32;
    let affected_nested_key_lines = [19u32, 34u32];
    let baseline_top_key_line = 3u32;
    let affected_top_key_lines = [36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [19u32, 22, 24, 26, 27, 28, 29, 30, 31, 32, 34, 36];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit file-diff cache and prepared syntax documents",
        |pane| {
            pane.file_diff_cache_inflight.is_none()
                && pane.file_diff_cache_repo_id == Some(repo_id)
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_target == Some(target.clone())
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
                "repo_id={:?} rev={} target={:?} cache_path={:?} language={:?} rows={} inline_rows={} left_doc={:?} right_doc={:?}",
                pane.file_diff_cache_repo_id,
                pane.file_diff_cache_rev,
                pane.file_diff_cache_target,
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
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit split syntax stays consistent for repeated paths and keys",
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
            let Some(baseline_path_color) =
                quoted_scalar_color(baseline_path_styled, baseline_path_text)
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
                    || quoted_scalar_color(styled, text) != Some(baseline_path_color)
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
                "diff_view={:?} split_debug={:?}",
                pane.diff_view,
                split_debug(pane, &lines),
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
        "YAML commit inline syntax stays consistent for repeated paths and keys",
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
            let Some(baseline_path_color) =
                quoted_scalar_color(baseline_path_styled, baseline_path_text)
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
                    || quoted_scalar_color(styled, text) != Some(baseline_path_color)
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
                "diff_view={:?} inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_commit_patch_diff_keeps_consistent_highlighting_for_added_paths_and_keys(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(
        FileDiffRowKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &super::CachedDiffStyledText,
    )> {
        for row_ix in 0..pane.patch_diff_split_row_len() {
            let PatchSplitRow::Aligned {
                row, new_src_ix, ..
            } = pane.patch_diff_split_row(row_ix)?
            else {
                continue;
            };
            if row.new_line != Some(new_line) {
                continue;
            }
            let src_ix = new_src_ix?;
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                row.kind,
                src_ix,
                row.new.as_deref()?.to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn inline_cached_styled_by_new_line(
        pane: &MainPaneView,
        new_line: u32,
    ) -> Option<(
        DiffLineKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &super::CachedDiffStyledText,
    )> {
        for src_ix in 0..pane.patch_diff_row_len() {
            let line = pane.patch_diff_row(src_ix)?;
            if line.new_line != Some(new_line) {
                continue;
            }
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                line.kind,
                src_ix,
                diff_content_text(&line).to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn quoted_scalar_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start == quote_start && range.end == text.len()).then_some(color)
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= dash_ix && range.end >= dash_ix.saturating_add(1)).then_some(color)
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

    fn split_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            FileDiffRowKind,
            Option<rows::DiffSyntaxLanguage>,
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
            .map(|line_no| {
                let payload = split_right_cached_styled_by_new_line(pane, line_no).map(
                    |(kind, _src_ix, text, language, styled)| {
                        (
                            kind,
                            language,
                            text,
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        )
                    },
                );
                (line_no, payload)
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            DiffLineKind,
            Option<rows::DiffSyntaxLanguage>,
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
            .map(|line_no| {
                let payload = inline_cached_styled_by_new_line(pane, line_no).map(
                    |(kind, _src_ix, text, language, styled)| {
                        (
                            kind,
                            language,
                            text,
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        )
                    },
                );
                (line_no, payload)
            })
            .collect()
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(82);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_diff",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let repo_root = fixture_repo_root();
    let unified = fixture_git_diff(
        &repo_root,
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/deployment-ci.yml",
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/deployment-ci.yml",
        "YAML commit patch-diff regression fixture",
    );

    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    let baseline_path_line = 17u32;
    let affected_path_lines = [18u32, 30, 31, 32, 33];
    let baseline_key_line = 19u32;
    let affected_key_lines = [21u32, 34u32, 36u32];
    let affected_add_lines = [18u32, 33u32];
    let affected_context_lines = [21u32, 30, 31, 32, 34, 36];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch-diff cache and language assignment",
        |pane| {
            pane.patch_diff_row_len() > 0
                && pane.patch_diff_split_row_len() > 0
                && pane.diff_language_for_src_ix.len() == pane.patch_diff_row_len()
                && (0..pane.patch_diff_row_len()).any(|src_ix| {
                    pane.patch_diff_row(src_ix)
                        .is_some_and(|line| line.new_line == Some(36))
                })
        },
        |pane| {
            format!(
                "diff_view={:?} rows={} split_rows={} visible_len={} languages={:?}",
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.patch_diff_split_row_len(),
                pane.diff_visible_len(),
                (0..pane.patch_diff_row_len())
                    .filter_map(|src_ix| {
                        pane.patch_diff_row(src_ix).map(|line| {
                            (
                                src_ix,
                                line.kind,
                                line.new_line,
                                pane.diff_language_for_src_ix.get(src_ix).copied().flatten(),
                            )
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "YAML commit patch split syntax stays consistent for added paths and keys",
        |pane| {
            let Some((
                baseline_kind,
                _baseline_src_ix,
                baseline_text,
                baseline_language,
                baseline_styled,
            )) = split_right_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            if baseline_kind != FileDiffRowKind::Context
                || baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_dash_color) = list_item_dash_color(baseline_styled, &baseline_text)
            else {
                return false;
            };
            let Some(baseline_path_color) = quoted_scalar_color(baseline_styled, &baseline_text)
            else {
                return false;
            };

            if affected_add_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, &text) != Some(baseline_dash_color)
                    || quoted_scalar_color(styled, &text) != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((
                baseline_key_kind,
                _baseline_key_src_ix,
                baseline_key_text,
                baseline_key_language,
                baseline_key_styled,
            )) = split_right_cached_styled_by_new_line(pane, baseline_key_line)
            else {
                return false;
            };
            if baseline_key_kind != FileDiffRowKind::Context
                || baseline_key_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_key_color) =
                mapping_key_color(baseline_key_styled, &baseline_key_text)
            else {
                return false;
            };
            !affected_key_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    split_right_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, &text) != Some(baseline_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_key_line);
            lines.extend(affected_key_lines);
            format!(
                "diff_view={:?} split_debug={:?}",
                pane.diff_view,
                split_debug(pane, &lines),
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
        "YAML commit patch inline syntax stays consistent for added paths and keys",
        |pane| {
            let Some((
                baseline_kind,
                _baseline_src_ix,
                baseline_text,
                baseline_language,
                baseline_styled,
            )) = inline_cached_styled_by_new_line(pane, baseline_path_line)
            else {
                return false;
            };
            if baseline_kind != DiffLineKind::Context
                || baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_dash_color) = list_item_dash_color(baseline_styled, &baseline_text)
            else {
                return false;
            };
            let Some(baseline_path_color) = quoted_scalar_color(baseline_styled, &baseline_text)
            else {
                return false;
            };

            if affected_add_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_context_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_new_line(pane, line_no).is_some_and(
                    |(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    },
                )
            }) {
                return false;
            }
            if affected_path_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    inline_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                list_item_dash_color(styled, &text) != Some(baseline_dash_color)
                    || quoted_scalar_color(styled, &text) != Some(baseline_path_color)
            }) {
                return false;
            }

            let Some((
                baseline_key_kind,
                _baseline_key_src_ix,
                baseline_key_text,
                baseline_key_language,
                baseline_key_styled,
            )) = inline_cached_styled_by_new_line(pane, baseline_key_line)
            else {
                return false;
            };
            if baseline_key_kind != DiffLineKind::Context
                || baseline_key_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(baseline_key_color) =
                mapping_key_color(baseline_key_styled, &baseline_key_text)
            else {
                return false;
            };
            !affected_key_lines.iter().copied().any(|line_no| {
                let Some((_kind, _src_ix, text, _language, styled)) =
                    inline_cached_styled_by_new_line(pane, line_no)
                else {
                    return true;
                };
                mapping_key_color(styled, &text) != Some(baseline_key_color)
            })
        },
        |pane| {
            let mut lines = Vec::new();
            lines.push(baseline_path_line);
            lines.extend(affected_path_lines);
            lines.push(baseline_key_line);
            lines.extend(affected_key_lines);
            format!(
                "diff_view={:?} inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, &lines),
            )
        },
    );
}

#[gpui::test]
fn yaml_commit_patch_diff_full_fixture_keeps_consistent_highlighting_across_files(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use gitcomet_core::file_diff::FileDiffRowKind;

    fn split_right_cached_styled_by_file_and_new_line<'a>(
        pane: &'a MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<(
        FileDiffRowKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &'a super::CachedDiffStyledText,
    )> {
        for row_ix in 0..pane.patch_diff_split_row_len() {
            let PatchSplitRow::Aligned {
                row, new_src_ix, ..
            } = pane.patch_diff_split_row(row_ix)?
            else {
                continue;
            };
            if row.new_line != Some(new_line) {
                continue;
            }
            let src_ix = new_src_ix?;
            if pane
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|path| path.as_deref())
                != Some(file_path)
            {
                continue;
            }
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                row.kind,
                src_ix,
                row.new.as_deref()?.to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn inline_cached_styled_by_file_and_new_line<'a>(
        pane: &'a MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<(
        DiffLineKind,
        usize,
        String,
        Option<rows::DiffSyntaxLanguage>,
        &'a super::CachedDiffStyledText,
    )> {
        for src_ix in 0..pane.patch_diff_row_len() {
            let line = pane.patch_diff_row(src_ix)?;
            if line.new_line != Some(new_line) {
                continue;
            }
            if pane
                .diff_file_for_src_ix
                .get(src_ix)
                .and_then(|path| path.as_deref())
                != Some(file_path)
            {
                continue;
            }
            let styled = pane.diff_text_segments_cache_get(src_ix, 0)?;
            let language = pane.diff_language_for_src_ix.get(src_ix).copied().flatten();
            return Some((
                line.kind,
                src_ix,
                diff_content_text(&line).to_string(),
                language,
                styled,
            ));
        }
        None
    }

    fn quoted_scalar_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let quote_start = text.find('"')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start == quote_start && range.end == text.len()).then_some(color)
        })
    }

    fn list_item_dash_color(
        styled: &super::CachedDiffStyledText,
        text: &str,
    ) -> Option<gpui::Hsla> {
        let dash_ix = text.find('-')?;
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= dash_ix && range.end >= dash_ix.saturating_add(1)).then_some(color)
        })
    }

    fn mapping_key_color(styled: &super::CachedDiffStyledText, text: &str) -> Option<gpui::Hsla> {
        let key_start = text.find(|ch: char| !ch.is_ascii_whitespace())?;
        let key_end = text[key_start..].find(':')?.saturating_add(key_start);
        styled.highlights.iter().find_map(|(range, style)| {
            let color = style.color?;
            (range.start <= key_start && range.end >= key_end).then_some(color)
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
            (range.start <= value_start && range.end > value_start).then_some(color)
        })
    }

    fn split_debug(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            FileDiffRowKind,
            Option<rows::DiffSyntaxLanguage>,
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
            .map(|line_no| {
                let payload = split_right_cached_styled_by_file_and_new_line(
                    pane, file_path, line_no,
                )
                .map(|(kind, _src_ix, text, language, styled)| {
                    (
                        kind,
                        language,
                        text,
                        styled
                            .highlights
                            .iter()
                            .map(|(range, style)| {
                                (range.clone(), style.color, style.background_color)
                            })
                            .collect(),
                    )
                });
                (line_no, payload)
            })
            .collect()
    }

    fn inline_debug(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<(
        u32,
        Option<(
            DiffLineKind,
            Option<rows::DiffSyntaxLanguage>,
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
            .map(|line_no| {
                let payload = inline_cached_styled_by_file_and_new_line(pane, file_path, line_no)
                    .map(|(kind, _src_ix, text, language, styled)| {
                        (
                            kind,
                            language,
                            text,
                            styled
                                .highlights
                                .iter()
                                .map(|(range, style)| {
                                    (range.clone(), style.color, style.background_color)
                                })
                                .collect(),
                        )
                    });
                (line_no, payload)
            })
            .collect()
    }

    fn split_visible_ix_by_file_and_new_line(
        pane: &MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(row_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            let Some(PatchSplitRow::Aligned {
                row, new_src_ix, ..
            }) = pane.patch_diff_split_row(row_ix)
            else {
                return false;
            };
            let Some(src_ix) = new_src_ix else {
                return false;
            };
            row.new_line == Some(new_line)
                && pane
                    .diff_file_for_src_ix
                    .get(src_ix)
                    .and_then(|path| path.as_deref())
                    == Some(file_path)
        })
    }

    fn inline_visible_ix_by_file_and_new_line(
        pane: &MainPaneView,
        file_path: &str,
        new_line: u32,
    ) -> Option<usize> {
        (0..pane.diff_visible_len()).find(|&visible_ix| {
            let Some(src_ix) = pane.diff_mapped_ix_for_visible_ix(visible_ix) else {
                return false;
            };
            let Some(line) = pane.patch_diff_row(src_ix) else {
                return false;
            };
            line.new_line == Some(new_line)
                && pane
                    .diff_file_for_src_ix
                    .get(src_ix)
                    .and_then(|path| path.as_deref())
                    == Some(file_path)
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

    #[derive(Clone, Copy, Debug)]
    struct ExpectedPaintRow {
        line_no: u32,
        visible_ix: usize,
        expects_add_bg: bool,
    }

    fn split_draw_rows_for_lines(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = split_visible_ix_by_file_and_new_line(pane, file_path, line_no)
                    .unwrap_or_else(|| {
                        panic!("expected split visible row for {file_path} line {line_no}")
                    });
                let row_ix = pane
                    .diff_mapped_ix_for_visible_ix(visible_ix)
                    .unwrap_or_else(|| {
                        panic!("expected split mapped row for {file_path} line {line_no}")
                    });
                let PatchSplitRow::Aligned { row, .. } =
                    pane.patch_diff_split_row(row_ix).unwrap_or_else(|| {
                        panic!("expected aligned split row for {file_path} line {line_no}")
                    })
                else {
                    panic!("expected aligned split row for {file_path} line {line_no}");
                };
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg: row.kind == FileDiffRowKind::Add,
                }
            })
            .collect()
    }

    fn inline_draw_rows_for_lines(
        pane: &MainPaneView,
        file_path: &str,
        lines: &[u32],
    ) -> Vec<ExpectedPaintRow> {
        lines
            .iter()
            .copied()
            .map(|line_no| {
                let visible_ix = inline_visible_ix_by_file_and_new_line(pane, file_path, line_no)
                    .unwrap_or_else(|| {
                        panic!("expected inline visible row for {file_path} line {line_no}")
                    });
                let src_ix = pane
                    .diff_mapped_ix_for_visible_ix(visible_ix)
                    .unwrap_or_else(|| {
                        panic!("expected inline mapped row for {file_path} line {line_no}")
                    });
                let kind = pane
                    .patch_diff_row(src_ix)
                    .unwrap_or_else(|| {
                        panic!("expected inline diff row for {file_path} line {line_no}")
                    })
                    .kind;
                ExpectedPaintRow {
                    line_no,
                    visible_ix,
                    expects_add_bg: kind == DiffLineKind::Add,
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
        file_path: &str,
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
                let Some((_kind, _src_ix, text, _language, styled)) =
                    split_right_cached_styled_by_file_and_new_line(
                        pane,
                        file_path,
                        expected.line_no,
                    )
                else {
                    panic!(
                        "expected cached split-right styled text for {file_path} line {}",
                        expected.line_no
                    );
                };
                (text, highlight_snapshot(styled.highlights.as_ref()))
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
        file_path: &str,
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
                let Some((_kind, _src_ix, text, _language, styled)) =
                    inline_cached_styled_by_file_and_new_line(pane, file_path, expected.line_no)
                else {
                    panic!(
                        "expected cached inline styled text for {file_path} line {}",
                        expected.line_no
                    );
                };
                (text, highlight_snapshot(styled.highlights.as_ref()))
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

    let repo_id = gitcomet_state::model::RepoId(85);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_full_fixture",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let unified =
        std::fs::read_to_string(fixture_repo_root().join("test_data/commit-bd8b4a04.patch"))
            .expect("should read multi-file YAML commit patch regression fixture");
    let target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    let build_release_file = ".github/workflows/build-release-artifacts.yml";
    let build_release_baseline_secret_key_line = 20u32;
    let build_release_affected_secret_key_lines = [22u32, 24u32];
    let build_release_baseline_required_line = 21u32;
    let build_release_affected_required_lines = [23u32];
    let build_release_add_lines = [20u32, 21u32];
    let build_release_context_lines = [22u32, 23u32, 24u32];
    let build_release_draw_lines = [20u32, 21, 22, 23, 24];

    let deployment_file = ".github/workflows/deployment-ci.yml";
    let deployment_baseline_path_line = 17u32;
    let deployment_affected_path_lines = [18u32, 30u32, 31u32, 32u32, 33u32];
    let deployment_baseline_key_line = 19u32;
    let deployment_affected_key_lines = [21u32, 34u32, 36u32];
    let deployment_add_lines = [18u32, 33u32];
    let deployment_context_lines = [21u32, 30u32, 31u32, 32u32, 34u32, 36u32];
    let deployment_draw_lines = [17u32, 18, 19, 21, 30, 31, 32, 33, 34, 36];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch-diff cache and language assignment",
        |pane| {
            pane.patch_diff_row_len() > 0
                && pane.patch_diff_split_row_len() > 0
                && pane.diff_language_for_src_ix.len() == pane.patch_diff_row_len()
                && (0..pane.patch_diff_row_len()).any(|src_ix| {
                    pane.patch_diff_row(src_ix).is_some_and(|line| {
                        line.new_line == Some(36)
                            && pane
                                .diff_file_for_src_ix
                                .get(src_ix)
                                .and_then(|path| path.as_deref())
                                == Some(deployment_file)
                    })
                })
                && (0..pane.patch_diff_row_len()).any(|src_ix| {
                    pane.patch_diff_row(src_ix).is_some_and(|line| {
                        line.new_line == Some(24)
                            && pane
                                .diff_file_for_src_ix
                                .get(src_ix)
                                .and_then(|path| path.as_deref())
                                == Some(build_release_file)
                    })
                })
        },
        |pane| {
            format!(
                "diff_view={:?} rows={} split_rows={} visible_len={} files={:?}",
                pane.diff_view,
                pane.patch_diff_row_len(),
                pane.patch_diff_split_row_len(),
                pane.diff_visible_len(),
                (0..pane.patch_diff_row_len())
                    .filter_map(|src_ix| {
                        pane.patch_diff_row(src_ix).map(|line| {
                            (
                                src_ix,
                                pane.diff_file_for_src_ix
                                    .get(src_ix)
                                    .and_then(|path| path.as_deref())
                                    .map(str::to_owned),
                                line.kind,
                                line.new_line,
                                pane.diff_language_for_src_ix.get(src_ix).copied().flatten(),
                            )
                        })
                    })
                    .collect::<Vec<_>>(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.scroll_diff_to_item_strict(0, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch split syntax stays consistent for build-release top hunk",
        |pane| {
            let Some((
                build_release_baseline_kind,
                _build_release_baseline_src_ix,
                build_release_baseline_text,
                build_release_baseline_language,
                build_release_baseline_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_secret_key_line,
            )
            else {
                return false;
            };
            if build_release_baseline_kind != FileDiffRowKind::Add
                || build_release_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(build_release_baseline_key_color) =
                mapping_key_color(build_release_baseline_styled, &build_release_baseline_text)
            else {
                return false;
            };
            if build_release_add_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_context_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_affected_secret_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(build_release_baseline_key_color)
                })
            {
                return false;
            }

            let Some((
                _build_release_required_kind,
                _build_release_required_src_ix,
                build_release_required_text,
                _build_release_required_language,
                build_release_required_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_required_line,
            )
            else {
                return false;
            };
            let Some(build_release_required_color) = scalar_color_after_colon(
                build_release_required_styled,
                &build_release_required_text,
            ) else {
                return false;
            };
            !build_release_affected_required_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    scalar_color_after_colon(styled, &text) != Some(build_release_required_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} build_release_split_debug={:?}",
                pane.diff_view,
                split_debug(pane, build_release_file, &build_release_draw_lines),
            )
        },
    );

    let build_release_split_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, build_release_file, &build_release_draw_lines)
    });
    assert_split_rows_match_render_cache(
        cx,
        &view,
        "build-release split",
        build_release_file,
        build_release_split_expected,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_diff_visible_indices();
                let target_visible_ix = split_visible_ix_by_file_and_new_line(
                    pane,
                    deployment_file,
                    deployment_baseline_path_line,
                )
                .expect("deployment workflow should have a visible split row in the full fixture");
                pane.scroll_diff_to_item_strict(target_visible_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch split syntax stays consistent for deployment workflow rows",
        |pane| {
            let Some((
                deployment_baseline_kind,
                _deployment_baseline_src_ix,
                deployment_baseline_text,
                deployment_baseline_language,
                deployment_baseline_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_path_line,
            )
            else {
                return false;
            };
            if deployment_baseline_kind != FileDiffRowKind::Context
                || deployment_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(deployment_baseline_dash_color) =
                list_item_dash_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            let Some(deployment_baseline_path_color) =
                quoted_scalar_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            if deployment_add_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_context_lines.iter().copied().any(|line_no| {
                !split_right_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == FileDiffRowKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_affected_path_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            deployment_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    list_item_dash_color(styled, &text) != Some(deployment_baseline_dash_color)
                        || quoted_scalar_color(styled, &text)
                            != Some(deployment_baseline_path_color)
                })
            {
                return false;
            }

            let Some((
                _deployment_key_kind,
                _deployment_key_src_ix,
                deployment_key_text,
                _deployment_key_language,
                deployment_key_styled,
            )) = split_right_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_key_line,
            )
            else {
                return false;
            };
            let Some(deployment_key_color) =
                mapping_key_color(deployment_key_styled, &deployment_key_text)
            else {
                return false;
            };
            !deployment_affected_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        split_right_cached_styled_by_file_and_new_line(
                            pane,
                            deployment_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(deployment_key_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} deployment_split_debug={:?}",
                pane.diff_view,
                split_debug(pane, deployment_file, &deployment_draw_lines),
            )
        },
    );

    let deployment_split_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        split_draw_rows_for_lines(pane, deployment_file, &deployment_draw_lines)
    });
    assert_split_rows_match_render_cache(
        cx,
        &view,
        "deployment split",
        deployment_file,
        deployment_split_expected,
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

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_diff_visible_indices();
                let target_visible_ix = inline_visible_ix_by_file_and_new_line(
                    pane,
                    build_release_file,
                    build_release_baseline_secret_key_line,
                )
                .expect(
                    "build-release workflow should have a visible inline row in the full fixture",
                );
                pane.scroll_diff_to_item_strict(target_visible_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch inline syntax stays consistent for build-release top hunk",
        |pane| {
            let Some((
                build_release_baseline_kind,
                _build_release_baseline_src_ix,
                build_release_baseline_text,
                build_release_baseline_language,
                build_release_baseline_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_secret_key_line,
            )
            else {
                return false;
            };
            if build_release_baseline_kind != DiffLineKind::Add
                || build_release_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(build_release_baseline_key_color) =
                mapping_key_color(build_release_baseline_styled, &build_release_baseline_text)
            else {
                return false;
            };
            if build_release_add_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_context_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, build_release_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if build_release_affected_secret_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(build_release_baseline_key_color)
                })
            {
                return false;
            }

            let Some((
                _build_release_required_kind,
                _build_release_required_src_ix,
                build_release_required_text,
                _build_release_required_language,
                build_release_required_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                build_release_file,
                build_release_baseline_required_line,
            )
            else {
                return false;
            };
            let Some(build_release_required_color) = scalar_color_after_colon(
                build_release_required_styled,
                &build_release_required_text,
            ) else {
                return false;
            };
            !build_release_affected_required_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(
                            pane,
                            build_release_file,
                            line_no,
                        )
                    else {
                        return true;
                    };
                    scalar_color_after_colon(styled, &text) != Some(build_release_required_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} build_release_inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, build_release_file, &build_release_draw_lines),
            )
        },
    );

    let build_release_inline_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, build_release_file, &build_release_draw_lines)
    });
    assert_inline_rows_match_render_cache(
        cx,
        &view,
        "build-release inline",
        build_release_file,
        build_release_inline_expected,
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.ensure_diff_visible_indices();
                let target_visible_ix = inline_visible_ix_by_file_and_new_line(
                    pane,
                    deployment_file,
                    deployment_baseline_path_line,
                )
                .expect("deployment workflow should have a visible inline row in the full fixture");
                pane.scroll_diff_to_item_strict(target_visible_ix, gpui::ScrollStrategy::Top);
                pane.clear_diff_text_style_caches();
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "multi-file YAML commit patch inline syntax stays consistent for deployment workflow rows",
        |pane| {
            let Some((
                deployment_baseline_kind,
                _deployment_baseline_src_ix,
                deployment_baseline_text,
                deployment_baseline_language,
                deployment_baseline_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_path_line,
            )
            else {
                return false;
            };
            if deployment_baseline_kind != DiffLineKind::Context
                || deployment_baseline_language != Some(rows::DiffSyntaxLanguage::Yaml)
            {
                return false;
            }
            let Some(deployment_baseline_dash_color) =
                list_item_dash_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            let Some(deployment_baseline_path_color) =
                quoted_scalar_color(deployment_baseline_styled, &deployment_baseline_text)
            else {
                return false;
            };
            if deployment_add_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Add
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_context_lines.iter().copied().any(|line_no| {
                !inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    .is_some_and(|(kind, _src_ix, _text, language, _styled)| {
                        kind == DiffLineKind::Context
                            && language == Some(rows::DiffSyntaxLanguage::Yaml)
                    })
            }) {
                return false;
            }
            if deployment_affected_path_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    else {
                        return true;
                    };
                    list_item_dash_color(styled, &text) != Some(deployment_baseline_dash_color)
                        || quoted_scalar_color(styled, &text)
                            != Some(deployment_baseline_path_color)
                })
            {
                return false;
            }

            let Some((
                _deployment_key_kind,
                _deployment_key_src_ix,
                deployment_key_text,
                _deployment_key_language,
                deployment_key_styled,
            )) = inline_cached_styled_by_file_and_new_line(
                pane,
                deployment_file,
                deployment_baseline_key_line,
            )
            else {
                return false;
            };
            let Some(deployment_key_color) =
                mapping_key_color(deployment_key_styled, &deployment_key_text)
            else {
                return false;
            };
            !deployment_affected_key_lines
                .iter()
                .copied()
                .any(|line_no| {
                    let Some((_kind, _src_ix, text, _language, styled)) =
                        inline_cached_styled_by_file_and_new_line(pane, deployment_file, line_no)
                    else {
                        return true;
                    };
                    mapping_key_color(styled, &text) != Some(deployment_key_color)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} deployment_inline_debug={:?}",
                pane.diff_view,
                inline_debug(pane, deployment_file, &deployment_draw_lines),
            )
        },
    );

    let deployment_inline_expected = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        inline_draw_rows_for_lines(pane, deployment_file, &deployment_draw_lines)
    });
    assert_inline_rows_match_render_cache(
        cx,
        &view,
        "deployment inline",
        deployment_file,
        deployment_inline_expected,
    );
}

#[gpui::test]
fn yaml_commit_patch_diff_matches_commit_file_diff_for_build_release_artifacts(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::domain::DiffLineKind;
    use std::collections::{BTreeMap, BTreeSet};

    #[derive(Clone, Debug, PartialEq)]
    struct LineSyntaxSnapshot {
        text: String,
        syntax: Vec<(std::ops::Range<usize>, Option<gpui::Hsla>)>,
    }

    fn parse_hunk_start(text: &str) -> Option<(u32, u32)> {
        let text = text.strip_prefix("@@")?.trim_start();
        let text = text.split("@@").next()?.trim();
        let mut parts = text.split_whitespace();
        let old = parts.next()?.strip_prefix('-')?;
        let new = parts.next()?.strip_prefix('+')?;
        let old_start = old.split(',').next()?.parse::<u32>().ok()?;
        let new_start = new.split(',').next()?.parse::<u32>().ok()?;
        Some((old_start, new_start))
    }

    fn patch_visible_line_numbers(
        diff: &gitcomet_core::domain::Diff,
    ) -> (BTreeSet<u32>, BTreeSet<u32>) {
        let mut old_lines = BTreeSet::new();
        let mut new_lines = BTreeSet::new();
        let mut old_line = None;
        let mut new_line = None;

        for line in &diff.lines {
            match line.kind {
                DiffLineKind::Header => {}
                DiffLineKind::Hunk => {
                    if let Some((old_start, new_start)) = parse_hunk_start(line.text.as_ref()) {
                        old_line = Some(old_start);
                        new_line = Some(new_start);
                    } else {
                        old_line = None;
                        new_line = None;
                    }
                }
                DiffLineKind::Context => {
                    if let Some(line_no) = old_line {
                        old_lines.insert(line_no);
                        old_line = Some(line_no.saturating_add(1));
                    }
                    if let Some(line_no) = new_line {
                        new_lines.insert(line_no);
                        new_line = Some(line_no.saturating_add(1));
                    }
                }
                DiffLineKind::Remove => {
                    if let Some(line_no) = old_line {
                        old_lines.insert(line_no);
                        old_line = Some(line_no.saturating_add(1));
                    }
                }
                DiffLineKind::Add => {
                    if let Some(line_no) = new_line {
                        new_lines.insert(line_no);
                        new_line = Some(line_no.saturating_add(1));
                    }
                }
            }
        }

        (old_lines, new_lines)
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

    fn yaml_patch_snapshot_for_src_ix(
        pane: &MainPaneView,
        theme: AppTheme,
        string_color: gpui::Hsla,
        src_ix: usize,
        text: &str,
    ) -> LineSyntaxSnapshot {
        let force_full_string = pane
            .diff_yaml_block_scalar_for_src_ix
            .get(src_ix)
            .copied()
            .unwrap_or(false);

        if force_full_string {
            return LineSyntaxSnapshot {
                text: text.to_string(),
                syntax: (!text.is_empty())
                    .then_some(vec![(0..text.len(), Some(string_color))])
                    .unwrap_or_default(),
            };
        }

        let highlights = rows::syntax_highlights_for_line(
            theme,
            text,
            rows::DiffSyntaxLanguage::Yaml,
            pane.patch_diff_syntax_mode(),
        );
        LineSyntaxSnapshot {
            text: text.to_string(),
            syntax: highlights
                .into_iter()
                .filter(|(_, style)| style.background_color.is_none())
                .map(|(range, style)| (range, style.color))
                .collect(),
        }
    }

    fn patch_split_snapshot_by_line(
        pane: &MainPaneView,
        region: DiffTextRegion,
        theme: AppTheme,
        string_color: gpui::Hsla,
        line_no: u32,
    ) -> Option<LineSyntaxSnapshot> {
        for row_ix in 0..pane.patch_diff_split_row_len() {
            let PatchSplitRow::Aligned {
                row,
                old_src_ix,
                new_src_ix,
            } = pane.patch_diff_split_row(row_ix)?
            else {
                continue;
            };

            let (src_ix, text) = match region {
                DiffTextRegion::SplitLeft if row.old_line == Some(line_no) => {
                    (old_src_ix?, row.old.as_deref()?)
                }
                DiffTextRegion::SplitRight if row.new_line == Some(line_no) => {
                    (new_src_ix?, row.new.as_deref()?)
                }
                DiffTextRegion::Inline | DiffTextRegion::SplitLeft | DiffTextRegion::SplitRight => {
                    continue;
                }
            };

            return Some(yaml_patch_snapshot_for_src_ix(
                pane,
                theme,
                string_color,
                src_ix,
                text,
            ));
        }

        None
    }

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let theme = cx.update(|_window, app| view.read(app).main_pane.read(app).theme);
    let yaml_string_color = rows::syntax_highlights_for_line(
        theme,
        "\"yaml-string\"",
        rows::DiffSyntaxLanguage::Yaml,
        rows::DiffSyntaxMode::Auto,
    )
    .into_iter()
    .find_map(|(_, style)| style.color)
    .expect("expected YAML string token color");

    let repo_id = gitcomet_state::model::RepoId(83);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_yaml_commit_patch_file_parity",
        std::process::id()
    ));
    let commit_id =
        gitcomet_core::domain::CommitId("bd8b4a04b4d7a04caf97392d6a66cbeebd665606".into());
    let path = std::path::PathBuf::from(".github/workflows/build-release-artifacts.yml");
    let repo_root = fixture_repo_root();
    let git_show =
        |spec: &str| fixture_git_show(&repo_root, spec, "YAML commit patch/file parity fixture");
    let unified = fixture_git_diff(
        &repo_root,
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
        "YAML commit patch/file parity fixture",
    );
    let old_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606^:.github/workflows/build-release-artifacts.yml",
    );
    let new_text = git_show(
        "bd8b4a04b4d7a04caf97392d6a66cbeebd665606:.github/workflows/build-release-artifacts.yml",
    );

    let file_target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: Some(path.clone()),
    };
    let file_diff = gitcomet_core::domain::Diff::from_unified(file_target.clone(), &unified);
    let patch_target = gitcomet_core::domain::DiffTarget::Commit {
        commit_id: commit_id.clone(),
        path: None,
    };
    let patch_diff = gitcomet_core::domain::Diff::from_unified(patch_target.clone(), &unified);
    let (visible_old_lines, visible_new_lines) = patch_visible_line_numbers(&patch_diff);
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
    let baseline_old_by_line = visible_old_lines
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
    let baseline_new_by_line = visible_new_lines
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
                    foreground_parse: std::time::Duration::from_millis(50),
                });
            });

            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(file_target.clone());
            repo.diff_state.diff_rev = 1;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(file_diff));
            repo.diff_state.diff_file_rev = 1;
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

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
        "YAML commit file-diff baseline prepared syntax ready",
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);

            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
                && pane.file_diff_cache_repo_id == Some(repo_id)
                && pane.file_diff_cache_rev == 1
                && pane.file_diff_cache_target == Some(file_target.clone())
                && pane.file_diff_cache_path == Some(workdir.join(&path))
                && pane.file_diff_cache_language == Some(rows::DiffSyntaxLanguage::Yaml)
                && left_doc.is_some()
                && right_doc.is_some()
                && left_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
                && right_doc.is_some_and(|document| {
                    !rows::has_pending_prepared_diff_syntax_chunk_builds_for_document(document)
                })
        },
        |pane| {
            format!(
                "diff_view={:?} file_diff_active={} rev={} old_lines={} new_lines={} left_doc={:?} right_doc={:?}",
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.file_diff_cache_rev,
                visible_old_lines.len(),
                visible_new_lines.len(),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft),
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus::default().into(),
            );
            repo.diff_state.diff_target = Some(patch_target.clone());
            repo.diff_state.diff_rev = 2;
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(patch_diff));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

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
        "YAML commit patch rows ready for build-release split parity check",
        |pane| {
            !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && pane.patch_diff_split_row_len() > 0
                && pane.diff_yaml_block_scalar_for_src_ix.len() == pane.patch_diff_row_len()
                && visible_old_lines.iter().copied().all(|line_no| {
                    patch_split_snapshot_by_line(
                        pane,
                        DiffTextRegion::SplitLeft,
                        theme,
                        yaml_string_color,
                        line_no,
                    )
                    .is_some()
                })
                && visible_new_lines.iter().copied().all(|line_no| {
                    patch_split_snapshot_by_line(
                        pane,
                        DiffTextRegion::SplitRight,
                        theme,
                        yaml_string_color,
                        line_no,
                    )
                    .is_some()
                })
        },
        |pane| {
            format!(
                "diff_view={:?} file_diff_active={} split_rows={} block_scalar_flags={} left_ready={}/{} right_ready={}/{}",
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.patch_diff_split_row_len(),
                pane.diff_yaml_block_scalar_for_src_ix.len(),
                visible_old_lines
                    .iter()
                    .filter(|&&line_no| {
                        patch_split_snapshot_by_line(
                            pane,
                            DiffTextRegion::SplitLeft,
                            theme,
                            yaml_string_color,
                            line_no,
                        )
                        .is_some()
                    })
                    .count(),
                visible_old_lines.len(),
                visible_new_lines
                    .iter()
                    .filter(|&&line_no| {
                        patch_split_snapshot_by_line(
                            pane,
                            DiffTextRegion::SplitRight,
                            theme,
                            yaml_string_color,
                            line_no,
                        )
                        .is_some()
                    })
                    .count(),
                visible_new_lines.len(),
            )
        },
    );

    let split_mismatches = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut mismatches = Vec::new();

        for (&line_no, expected) in &baseline_old_by_line {
            let actual = patch_split_snapshot_by_line(
                pane,
                DiffTextRegion::SplitLeft,
                theme,
                yaml_string_color,
                line_no,
            );
            if actual.as_ref() != Some(expected) && mismatches.len() < 16 {
                mismatches.push(("left", line_no, actual, expected.clone()));
            }
        }

        for (&line_no, expected) in &baseline_new_by_line {
            let actual = patch_split_snapshot_by_line(
                pane,
                DiffTextRegion::SplitRight,
                theme,
                yaml_string_color,
                line_no,
            );
            if actual.as_ref() != Some(expected) && mismatches.len() < 16 {
                mismatches.push(("right", line_no, actual, expected.clone()));
            }
        }

        mismatches
    });
    assert!(
        split_mismatches.is_empty(),
        "patch split YAML highlighting should match commit file-diff highlighting for build-release-artifacts.yml: {split_mismatches:?}",
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
        "YAML commit patch rows ready for build-release inline parity check",
        |pane| {
            !pane.is_file_diff_view_active()
                && pane.patch_diff_row_len() > 0
                && pane.diff_yaml_block_scalar_for_src_ix.len() == pane.patch_diff_row_len()
        },
        |pane| {
            format!(
                "diff_view={:?} file_diff_active={} rows={} block_scalar_flags={}",
                pane.diff_view,
                pane.is_file_diff_view_active(),
                pane.patch_diff_row_len(),
                pane.diff_yaml_block_scalar_for_src_ix.len(),
            )
        },
    );

    let inline_mismatches = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let mut mismatches = Vec::new();

        for src_ix in 0..pane.patch_diff_row_len() {
            let Some(line) = pane.patch_diff_row(src_ix) else {
                continue;
            };

            let expected = match line.kind {
                DiffLineKind::Context | DiffLineKind::Remove => line
                    .old_line
                    .and_then(|line_no| baseline_old_by_line.get(&line_no)),
                DiffLineKind::Add => line
                    .new_line
                    .and_then(|line_no| baseline_new_by_line.get(&line_no)),
                DiffLineKind::Header | DiffLineKind::Hunk => None,
            };
            let Some(expected) = expected else {
                continue;
            };

            let actual = Some(yaml_patch_snapshot_for_src_ix(
                pane,
                theme,
                yaml_string_color,
                src_ix,
                diff_content_text(&line),
            ));
            if actual.as_ref() != Some(expected) && mismatches.len() < 16 {
                mismatches.push((
                    line.kind,
                    line.old_line,
                    line.new_line,
                    actual,
                    expected.clone(),
                ));
            }
        }

        mismatches
    });
    assert!(
        inline_mismatches.is_empty(),
        "patch inline YAML highlighting should match commit file-diff highlighting for build-release-artifacts.yml: {inline_mismatches:?}",
    );
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
    let old_text = include_str!("../../../../smoke_tests.rs");
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
                gitcomet_core::domain::FileDiffText::new(
                    path,
                    Some(old_text.to_string()),
                    Some(new_text),
                ),
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

    wait_for_main_pane_condition(
        cx,
        &view,
        "steady smoke_tests.rs diff warmup",
        |pane| {
            let left_doc = pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitLeft);
            let right_doc =
                pane.file_diff_split_prepared_syntax_document(DiffTextRegion::SplitRight);
            pane.file_diff_cache_inflight.is_none()
                && pane.is_file_diff_view_active()
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
            (
                pane.file_diff_cache_inflight,
                pane.file_diff_cache_path.clone(),
                pane.is_file_diff_view_active(),
                left_doc,
                right_doc,
                left_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                right_doc.map(rows::has_pending_prepared_diff_syntax_chunk_builds_for_document),
                pane.syntax_chunk_poll_task.is_some(),
            )
        },
    );

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
