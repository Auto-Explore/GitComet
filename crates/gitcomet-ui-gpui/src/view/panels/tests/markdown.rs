use super::*;
use crate::view::panes::main::DiffWrapVisualRow;

#[gpui::test]
fn markdown_diff_preview_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(48);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_rev_stability",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/README.md");
    let old_text =
        "# Preview title\n\n- first item\n- second item\n\n```rust\nlet value = 1;\n```\n"
            .repeat(24);
    let new_text =
        format!("{old_text}\nA trailing paragraph keeps this markdown diff in preview mode.\n");

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
                    gitcomet_core::domain::FileDiffText::new(
                        path.clone(),
                        Some(old_text.clone()),
                        Some(new_text.clone()),
                    ),
                )));

                let next_state = app_state_with_repo(repo, repo_id);

                push_test_state(this, Arc::clone(&next_state), cx);
            });
        });
    };

    set_state(cx, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "initial markdown preview cache build",
        |pane| {
            pane.file_markdown_preview_inflight.is_none()
                && matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                )
        },
        |pane| {
            (
                pane.file_markdown_preview_seq,
                pane.file_markdown_preview_inflight,
                pane.file_markdown_preview_cache_repo_id,
                pane.file_markdown_preview_cache_rev,
                pane.file_markdown_preview_cache_target.clone(),
                pane.file_markdown_preview_cache_content_signature,
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
            )
        },
    );

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).file_markdown_preview_seq);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "markdown diff preview should default to Preview mode"
        );
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
                pane.file_markdown_preview_seq, baseline_seq,
                "identical markdown diff payload should not trigger preview rebuild when diff_file_rev changes"
            );
            assert!(
                pane.file_markdown_preview_inflight.is_none(),
                "markdown preview cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.file_markdown_preview_cache_rev, rev,
                "identical payload refresh should still advance the markdown cache rev marker"
            );
            assert!(
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
                "markdown preview should remain ready across rev-only refreshes"
            );
        });
    }
}

#[gpui::test]
fn worktree_markdown_diff_defaults_to_preview_mode_and_shows_preview_toggle(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_worktree_markdown_diff_default_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/guide.md");
    let old_text = concat!(
        "# Guide\n",
        "\n",
        "- keep\n",
        "- before\n",
        "\n",
        "```rust\n",
        "let value = 1;\n",
        "```\n",
    );
    let new_text = concat!(
        "# Guide\n",
        "\n",
        "- keep\n",
        "- after\n",
        "\n",
        "```rust\n",
        "let value = 2;\n",
        "```\n",
        "\n",
        "| Col | Value |\n",
        "| --- | --- |\n",
        "| add | 3 |\n",
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create commit markdown diff workdir");

    seed_file_diff_state(cx, &view, repo_id, &workdir, &file_rel, old_text, new_text);

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree markdown diff target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.file_markdown_preview_cache_repo_id = Some(repo_id);
                pane.file_markdown_preview_cache_rev = 1;
                pane.file_markdown_preview_cache_target = Some(target.clone());
                pane.file_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
                        .expect("worktree markdown diff preview should parse"),
                ));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(!pane.is_file_preview_active());
        assert!(
            pane.is_markdown_preview_active(),
            "expected worktree markdown diff preview to be active; mode={:?} target_kind={:?} diff_target={:?}",
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            crate::view::diff_target_rendered_preview_kind(
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.as_ref()),
            ),
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone()),
        );
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "expected worktree markdown diff to default to Preview mode"
        );
    });
    assert!(
        cx.debug_bounds("markdown_diff_view_toggle").is_some(),
        "expected markdown Preview/Text toggle for worktree markdown diff"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup worktree markdown diff fixture");
}

#[gpui::test]
fn secondary_f_from_markdown_file_preview_switches_back_to_text_search(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_search",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");
    let abs_path = workdir.join(&file_rel);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");
    std::fs::write(&abs_path, "# Title\n\npreview body\n").expect("write markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let preview_lines = Arc::new(vec![
                "# Title".to_string(),
                "".to_string(),
                "preview body".to_string(),
            ]);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    preview_lines,
                    "# Title\n\npreview body".len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
            });
        });
    });

    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("secondary-f");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Source,
            "secondary-f should switch markdown preview back to source mode before search"
        );
        assert!(
            pane.diff_search_active,
            "secondary-f should activate diff search from markdown preview"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview fixture");
}

#[gpui::test]
fn interactive_markdown_preview_text_multi_clicks_select_word_then_line(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(903);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_interactive_markdown_preview_multi_clicks",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/preview_clicks.md");
    let abs_path = workdir.join(&file_rel);
    let source = "# alpha_beta heading\n\nBody text.\n";
    let preview_lines = Arc::new(vec![
        "# alpha_beta heading".to_string(),
        "".to_string(),
        "Body text.".to_string(),
    ]);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create markdown preview multi-click workdir");
    std::fs::create_dir_all(
        abs_path
            .parent()
            .expect("markdown preview fixture path should have a parent"),
    )
    .expect("create markdown preview fixture parent directory");
    std::fs::write(&abs_path, source).expect("write markdown preview fixture");

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
            let abs_path = abs_path.clone();
            let preview_lines = Arc::clone(&preview_lines);
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, abs_path.clone(), preview_lines, source.len(), cx);
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("markdown preview should parse"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
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
        "markdown preview multi-click hitbox",
    );
    let expected_word = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let offset = pane
            .diff_text_offset_for_position(0, DiffTextRegion::Inline, click)
            .expect("expected markdown preview text offset");
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
            "single click should clear the markdown preview text selection"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview multi-click fixture");
}

#[gpui::test]
fn split_markdown_diff_scroll_sync_matrix_covers_all_modes_and_axes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(71);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_code_block_scrollbar",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/overflow.md");
    let build_markdown = |label: &str, fill: char| {
        let long_code = fill.to_string().repeat(160);
        let mut out = String::from("# Guide\n");
        for ix in 0..96 {
            out.push_str(&format!(
                "\n## Section {ix}\n\nParagraph {label} {ix}.\n\n```rust\nlet {label}_{ix} = \"{long_code}\";\n```\n"
            ));
        }
        out
    };
    let old_text = build_markdown("old", 'L');
    let new_text = build_markdown("new", 'R');
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create markdown code block diff workdir");

    seed_file_diff_state(
        cx, &view, repo_id, &workdir, &file_rel, &old_text, &new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown code block diff target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.file_markdown_preview_cache_repo_id = Some(repo_id);
                pane.file_markdown_preview_cache_rev = 1;
                pane.file_markdown_preview_cache_target = Some(target.clone());
                pane.file_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(
                        &old_text, &new_text,
                    )
                    .expect("markdown diff preview with overflowing code block should parse"),
                ));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "split markdown preview scroll-sync matrix overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.is_markdown_preview_active()
                && pane.diff_view == DiffViewMode::Split
                && uniform_list_max_offset(&pane.diff_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.diff_split_right_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.diff_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.diff_split_right_scroll).height > px(120.0)
        },
        |pane| {
            format!(
                "preview_active={} diff_view={:?} left_offset={:?} right_offset={:?} left_max={:?} right_max={:?}",
                pane.is_markdown_preview_active(),
                pane.diff_view,
                uniform_list_offset(&pane.diff_scroll),
                uniform_list_offset(&pane.diff_split_right_scroll),
                uniform_list_max_offset(&pane.diff_scroll),
                uniform_list_max_offset(&pane.diff_split_right_scroll),
            )
        },
    );
    assert!(
        cx.debug_bounds("markdown_preview_code_block_hscrollbar")
            .is_none(),
        "expected overflowing markdown preview code blocks to rely on preview-level horizontal scrolling, not a local code-block scrollbar"
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
                    "split markdown preview left pane should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(right),
                    expected,
                    "split markdown preview right pane should {} {} scrolling from the left pane in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
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
                    "split markdown preview right pane should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(left),
                    expected,
                    "split markdown preview left pane should {} {} scrolling from the right pane in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });
        }
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown code block diff workdir");
}

#[gpui::test]
fn worktree_markdown_preview_short_code_block_shell_spans_preview_width(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(72);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_code_block_width",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/snippet.md");
    let abs_path = workdir.join(&file_rel);
    let source = "```sh\necho hi\n```\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown code block width workdir");
    std::fs::write(&abs_path, source).expect("write markdown code block width fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree markdown code block width target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("short fenced markdown preview should parse"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let container_bounds = cx
        .debug_bounds("worktree_markdown_preview_scroll_container")
        .expect("expected worktree markdown preview container bounds");
    let code_shell_bounds = cx
        .debug_bounds("markdown_preview_code_shell_0")
        .expect("expected code shell bounds for the first markdown preview row");
    let width_ratio = code_shell_bounds.size.width / container_bounds.size.width;
    assert!(
        width_ratio >= 0.95,
        "expected short fenced code block shell to span preview width; ratio={width_ratio}, shell={code_shell_bounds:?}, container={container_bounds:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown code block width workdir");
}

#[gpui::test]
fn worktree_markdown_preview_list_text_box_stays_shorter_than_row_shell(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(73);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_list_selection_box",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/list.md");
    let abs_path = workdir.join(&file_rel);
    let source = "- first item\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown list workdir");
    std::fs::write(&abs_path, source).expect("write markdown list fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree markdown list target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("markdown list preview should parse"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let row_bounds = cx
        .debug_bounds("markdown_preview_row_box_0")
        .expect("expected list row shell bounds");
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected list row text box bounds");
    // The selection highlight is painted inside the text box, so the box has to
    // be the glyphs and nothing else: the bullet's column sits outside it, and
    // the row adds no vertical padding of its own.
    assert!(
        text_bounds.left() > row_bounds.left(),
        "expected the list marker column to sit outside the text box; text={text_bounds:?}, row={row_bounds:?}"
    );
    assert_eq!(
        text_bounds.size.height, row_bounds.size.height,
        "expected the list row to be exactly as tall as its text; text={text_bounds:?}, row={row_bounds:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown list fixture");
}

#[gpui::test]
fn secondary_f_from_conflict_markdown_preview_switches_back_to_text_search(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(48);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_markdown_preview_search",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("conflict.md");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

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
                "# Base\n",
                "# Local\n",
                "# Remote\n",
                "<<<<<<< ours\n# Local\n=======\n# Remote\n>>>>>>> theirs\n",
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert_eq!(
                    pane.conflict_resolver.path.as_ref(),
                    Some(&file_rel),
                    "expected conflict resolver state to be ready before toggling preview mode"
                );
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                cx.notify();
            });
        });
    });

    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("secondary-f");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.resolver_preview_mode,
            ConflictResolverPreviewMode::Text,
            "secondary-f should switch conflict markdown preview back to text mode before search"
        );
        assert!(
            pane.diff_search_active,
            "secondary-f should activate diff search from conflict markdown preview"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict markdown preview fixture");
}

#[gpui::test]
fn markdown_file_preview_over_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_over_limit",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("oversized.md");
    let abs_path = workdir.join(&file_rel);
    let oversized_len = crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1;
    let oversized_source = "x".repeat(oversized_len);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create oversize workdir");
    std::fs::write(&abs_path, &oversized_source).expect("write oversize markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::new(vec![oversized_source]),
                    oversized_len,
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
        assert!(
            pane.worktree_markdown_preview_inflight.is_none(),
            "oversized preview should fail synchronously without background parsing"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown_preview
        else {
            panic!(
                "expected oversize markdown file preview to show fallback error, got {:?}",
                pane.worktree_markdown_preview
            );
        };
        assert!(
            message.contains("1 MiB"),
            "oversize file preview should mention the 1 MiB limit: {message}"
        );
    });
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_none(),
        "oversized markdown file preview should not render the virtualized preview list"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup oversize markdown preview fixture");
}

#[gpui::test]
fn markdown_file_preview_uses_exact_source_length_for_over_limit_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(56);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_exact_source_len",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("exact-source-len.md");
    let abs_path = workdir.join(&file_rel);
    let mut row_limit_source = "x".repeat(crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);
    row_limit_source.push('\n');
    let preview_lines = Arc::new(
        row_limit_source
            .lines()
            .map(ToOwned::to_owned)
            .collect::<Vec<_>>(),
    );
    assert_eq!(preview_lines.len(), 1);
    assert_eq!(
        preview_lines[0].len(),
        crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES
    );
    assert_eq!(
        row_limit_source.len(),
        crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES + 1
    );
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create exact-source-len workdir");
    std::fs::write(&abs_path, &row_limit_source).expect("write exact-source-len markdown fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    row_limit_source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.ensure_single_markdown_preview_cache(cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
        assert!(
            pane.worktree_markdown_preview_inflight.is_none(),
            "over-limit preview should fail synchronously when exact source length exceeds the markdown cap"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown_preview
        else {
            panic!(
                "expected exact-source-len markdown file preview to show fallback error, got {:?}",
                pane.worktree_markdown_preview
            );
        };
        assert!(
            message.contains("1 MiB"),
            "exact-source-len file preview should mention the 1 MiB limit: {message}"
        );
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_none(),
        "exact-source-len markdown file preview should not render the virtualized preview list"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup exact-source-len markdown preview fixture");
}

#[gpui::test]
fn diff_target_change_clears_worktree_markdown_preview_cache_state(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(55);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_cache_reset",
        std::process::id()
    ));
    let preview_path = std::path::PathBuf::from("docs/preview.md");
    let preview_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: preview_path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let set_state = |cx: &mut gpui::VisualTestContext,
                     diff_target: Option<gitcomet_core::domain::DiffTarget>,
                     diff_state_rev: u64,
                     status_rev: u64| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                repo.status = gitcomet_state::model::Loadable::Ready(
                    gitcomet_core::domain::RepoStatus::default().into(),
                );
                repo.status_rev = status_rev;
                repo.diff_state.diff_target = diff_target;
                repo.diff_state.diff_state_rev = diff_state_rev;

                let next_state = app_state_with_repo(repo, repo_id);

                push_test_state(this, next_state, cx);
            });
        });
    };

    set_state(cx, Some(preview_target.clone()), 1, 1);

    wait_for_main_pane_condition(
        cx,
        &view,
        "initial markdown preview target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(preview_target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_preview_path = Some(workdir.join(&preview_path));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
                pane.worktree_preview_content_rev = 9;
                pane.worktree_preview_text = "preview".into();
                pane.worktree_preview_line_starts = Arc::from(vec![0usize]);
                pane.worktree_markdown_preview_path = Some(workdir.join(&preview_path));
                pane.worktree_markdown_preview_source_rev = 9;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Loading;
                pane.worktree_markdown_preview_inflight = Some(3);
                cx.notify();
            });
        });
    });

    set_state(cx, None, 2, 2);

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown preview cache reset after diff target change",
        |pane| {
            pane.worktree_preview_path.is_none()
                && pane.worktree_preview_content_rev == 0
                && pane.worktree_preview_text.is_empty()
                && pane.worktree_preview_line_starts.is_empty()
                && pane.worktree_markdown_preview_path.is_none()
                && pane.worktree_markdown_preview_source_rev == 0
                && matches!(
                    pane.worktree_markdown_preview,
                    gitcomet_state::model::Loadable::NotLoaded
                )
                && pane.worktree_markdown_preview_inflight.is_none()
        },
        |pane| {
            format!(
                "worktree_path={:?} worktree_rev={} worktree_text_len={} worktree_line_starts={} worktree_markdown_path={:?} worktree_markdown_rev={} worktree_markdown_inflight={:?} worktree_markdown_not_loaded={}",
                pane.worktree_preview_path,
                pane.worktree_preview_content_rev,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_starts.len(),
                pane.worktree_markdown_preview_path,
                pane.worktree_markdown_preview_source_rev,
                pane.worktree_markdown_preview_inflight,
                matches!(
                    pane.worktree_markdown_preview,
                    gitcomet_state::model::Loadable::NotLoaded
                ),
            )
        },
    );
}

#[gpui::test]
fn markdown_diff_preview_over_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(52);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_over_limit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/oversized.md");
    let oversized_side =
        "x".repeat(crate::view::markdown_preview::MAX_DIFF_PREVIEW_SOURCE_BYTES / 2 + 1);

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
                    path.clone(),
                    Some(oversized_side.clone()),
                    Some(oversized_side.clone()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
        assert!(
            pane.file_markdown_preview_inflight.is_none(),
            "oversized diff preview should fail synchronously without background parsing"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.file_markdown_preview else {
            panic!(
                "expected oversize markdown diff preview to show fallback error, got {:?}",
                pane.file_markdown_preview
            );
        };
        assert!(
            message.contains("2 MiB"),
            "oversize diff preview should mention the 2 MiB limit: {message}"
        );
    });
    assert!(
        cx.debug_bounds("diff_markdown_preview_container").is_none(),
        "oversized markdown diff preview should not render the split preview container"
    );
}

#[gpui::test]
fn markdown_diff_preview_row_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(54);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_row_limit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/row-limit.md");
    let old_text = "---\n".repeat(crate::view::markdown_preview::MAX_PREVIEW_ROWS + 1);
    let new_text = "# still small\n".to_string();

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
                    path.clone(),
                    Some(old_text.clone()),
                    Some(new_text.clone()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown diff preview row-limit fallback",
        |pane| {
            pane.file_markdown_preview_inflight.is_none()
                && matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Error(_)
                )
        },
        |pane| {
            (
                pane.file_markdown_preview_seq,
                pane.file_markdown_preview_inflight,
                pane.file_markdown_preview_cache_repo_id,
                pane.file_markdown_preview_cache_rev,
                pane.file_markdown_preview_cache_target.clone(),
                pane.file_markdown_preview_cache_content_signature,
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Loading
                ),
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
                matches!(
                    pane.file_markdown_preview,
                    gitcomet_state::model::Loadable::Error(_)
                ),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.file_markdown_preview else {
            panic!(
                "expected row-limit markdown diff preview to show fallback error, got {:?}",
                pane.file_markdown_preview
            );
        };
        assert!(
            message.contains("row limit"),
            "row-limit diff preview should mention the rendered row limit: {message}"
        );
    });
    assert!(
        cx.debug_bounds("diff_markdown_preview_container").is_none(),
        "row-limit markdown diff preview should not render the split preview container"
    );
}

#[gpui::test]
fn markdown_diff_preview_keeps_layout_controls_and_ignores_text_hotkeys(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_hotkeys",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/preview.md");
    let old_text = concat!(
        "# Preview\n",
        "one\n",
        "two before\n",
        "three\n",
        "four\n",
        "five\n",
        "six before\n",
        "seven\n",
    );
    let new_text = concat!(
        "# Preview\n",
        "one\n",
        "two after\n",
        "three\n",
        "four\n",
        "five\n",
        "six after\n",
        "seven\n",
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
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                gitcomet_core::domain::FileDiffText::new(
                    path.clone(),
                    Some(old_text.to_string()),
                    Some(new_text.to_string()),
                ),
            )));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Split;
                pane.reveal_whitespace_chars = false;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
    });
    // The change-nav buttons stay: `diff_nav_entries` walks the rendered
    // preview's changed blocks, so Alt+Up / Alt+Down still work here. So does
    // the inline/split toggle: `render_markdown_diff_preview` draws a merged
    // list or an old/new column pair from the same `diff_view`.
    assert!(
        cx.debug_bounds("diff_view_toggle").is_some(),
        "markdown diff preview should keep the inline/split toggle"
    );
    // Blame keeps its slot but greys out — the preview has no annotation
    // gutter, so the click is dropped and only Text mode annotates.
    let blame_bounds = cx
        .debug_bounds("diff_annotate")
        .expect("markdown diff preview should keep the blame toggle visible");
    cx.simulate_click(blame_bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert!(
            !view.read(app).main_pane.read(app).annotate_enabled,
            "clicking blame in the markdown preview should not enable annotations"
        );
    });

    // Without the fallback installed the Alt keystrokes below reach nothing,
    // so the layout assertions would hold no matter how the guard is written.
    cx.update(|_window, app| {
        crate::app::install_global_diff_shortcut_fallback_for_test(app);
    });

    // Alt+I switches the preview to its merged inline list; Alt+W stays inert
    // because the whitespace toggles only drive the diff-text rows.
    cx.simulate_keystrokes("alt-i alt-w");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Inline);
        assert!(!pane.reveal_whitespace_chars);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered
        );
    });

    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("alt-s");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_view, DiffViewMode::Split);
        assert!(!pane.reveal_whitespace_chars);
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered
        );
    });

    // Back in Text mode the same button annotates, so blame is greyed out by
    // the preview rather than unavailable for markdown files.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let blame_bounds = cx
        .debug_bounds("diff_annotate")
        .expect("text mode should keep the blame toggle");
    cx.simulate_click(blame_bounds.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_window, app| {
        assert!(
            view.read(app).main_pane.read(app).annotate_enabled,
            "clicking blame in markdown text mode should enable annotations"
        );
    });
}

#[gpui::test]
fn conflict_markdown_preview_hides_text_controls_and_ignores_text_hotkeys(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(50);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_preview_hotkeys",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("conflict.md");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create conflict workdir");

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
                "# Base one\n\n# Base two\n",
                "# Local one\n\n# Local two\n",
                "# Remote one\n\n# Remote two\n",
                concat!(
                    "<<<<<<< ours\n",
                    "# Local one\n",
                    "=======\n",
                    "# Remote one\n",
                    ">>>>>>> theirs\n",
                    "\n",
                    "<<<<<<< ours\n",
                    "# Local two\n",
                    "=======\n",
                    "# Remote two\n",
                    ">>>>>>> theirs\n",
                ),
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    let nav_entries = cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
                pane.reveal_whitespace_chars = false;
                cx.notify();
            });
        });
        view.read(app).main_pane.read(app).conflict_nav_entries()
    });
    assert!(
        nav_entries.len() > 1,
        "expected at least two conflict navigation entries for preview hotkey coverage"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                pane.conflict_resolver.active_conflict = 0;
                pane.conflict_resolver.nav_anchor = None;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_conflict_rendered_preview_active());
    });
    assert!(
        cx.debug_bounds("conflict_reveal_whitespace_chars_pill")
            .is_none(),
        "conflict markdown preview should hide whitespace control"
    );
    assert!(
        cx.debug_bounds("conflict_mode_toggle").is_none(),
        "conflict markdown preview should hide diff mode toggle"
    );
    assert!(
        cx.debug_bounds("conflict_view_mode_toggle").is_none(),
        "conflict markdown preview should hide view mode toggle"
    );
    assert!(
        cx.debug_bounds("conflict_prev").is_none(),
        "conflict markdown preview should hide previous-conflict navigation"
    );
    assert!(
        cx.debug_bounds("conflict_next").is_none(),
        "conflict markdown preview should hide next-conflict navigation"
    );

    cx.simulate_keystrokes("alt-i alt-w f2 f3 f7");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::TwoWayDiff
        );
        assert!(!pane.reveal_whitespace_chars);
        assert_eq!(pane.conflict_resolver.active_conflict, 0);
        assert!(
            pane.conflict_resolver.nav_anchor.is_none(),
            "preview hotkeys should not mutate conflict navigation state"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                pane.conflict_resolver.active_conflict = 1;
                cx.notify();
            });
        });
    });
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("alt-s");

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.conflict_resolver.view_mode,
            ConflictResolverViewMode::TwoWayDiff
        );
        assert!(!pane.reveal_whitespace_chars);
        assert_eq!(pane.conflict_resolver.active_conflict, 1);
        assert!(
            pane.conflict_resolver.nav_anchor.is_none(),
            "preview hotkeys should not mutate conflict navigation state",
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict hotkey fixture");
}

#[gpui::test]
fn conflict_markdown_preview_scroll_sync_matrix_covers_all_modes_and_axes(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(215);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_markdown_scroll_sync_matrix",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("conflict_scroll_sync_matrix.md");
    let abs_path = workdir.join(&file_rel);
    let build_markdown = |label: &str, fill: char| {
        let long_code = fill.to_string().repeat(400);
        let mut out = String::from("# Guide\n");
        for ix in 0..96 {
            out.push_str(&format!(
                "\n## Section {ix}\n\nParagraph {label} {ix}.\n\n```rust\nlet {label}_{ix} = \"{long_code}\";\n```\n"
            ));
        }
        out
    };
    let base_text = build_markdown("base", 'B');
    let ours_text = build_markdown("ours", 'O');
    let theirs_text = build_markdown("theirs", 'T');
    let current_text =
        format!("<<<<<<< ours\n{ours_text}\n=======\n{theirs_text}\n>>>>>>> theirs\n");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create conflict markdown matrix workdir");
    std::fs::write(&abs_path, &current_text).expect("write conflict markdown matrix fixture");

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

    wait_for_main_pane_condition(
        cx,
        &view,
        "conflict markdown matrix fixture initialized",
        |pane| {
            pane.conflict_resolver.path.as_ref() == Some(&file_rel)
                && pane.conflict_resolved_preview_line_count >= 1
        },
        |pane| {
            format!(
                "path={:?} resolved_lines={} preview_active={}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolved_preview_line_count,
                pane.is_conflict_rendered_preview_active(),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::ThreeWay, cx);
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition_with_timeout(
        cx,
        &view,
        "conflict markdown preview matrix overflow",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            pane.is_conflict_rendered_preview_active()
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_ours_scroll).width > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).width > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).width
                    > px(80.0)
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_ours_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).height > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).height
                    > px(120.0)
        },
        |pane| {
            format!(
                "preview_active={} base_offset={:?} ours_offset={:?} theirs_offset={:?} output_offset={:?} base_max={:?} ours_max={:?} theirs_max={:?} output_max={:?}",
                pane.is_conflict_rendered_preview_active(),
                uniform_list_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_offset(&pane.conflict_preview_ours_scroll),
                uniform_list_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll),
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_max_offset(&pane.conflict_preview_ours_scroll),
                uniform_list_max_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );

    let reset_offsets = |cx: &mut gpui::VisualTestContext,
                         view: &gpui::Entity<super::super::GitCometView>| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    reset_uniform_list_offsets(&[
                        &pane.conflict_resolver_diff_scroll,
                        &pane.conflict_preview_ours_scroll,
                        &pane.conflict_preview_theirs_scroll,
                        &pane.conflict_resolved_preview_scroll,
                        &pane.conflict_resolved_preview_gutter_scroll,
                    ]);
                    set_scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                        point(px(0.0), px(0.0)),
                    );
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);
    };

    for mode in ALL_DIFF_SCROLL_SYNC_MODES {
        set_diff_scroll_sync_for_test(cx, &view, mode);

        for axis in ScrollSyncAxis::ALL {
            let output_offset = axis.offset(px(72.0));
            reset_offsets(cx, &view);
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        set_scroll_handle_offset(
                            &pane.conflict_resolved_output_editor_scroll,
                            output_offset,
                        );
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let expected = if axis.includes(mode) {
                    axis.component(output_offset)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                    )),
                    axis.component(output_offset),
                    "conflict markdown output should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_resolver_diff_scroll)),
                    expected,
                    "conflict markdown base preview should {} {} scrolling from resolved output in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_ours_scroll)),
                    expected,
                    "conflict markdown ours preview should {} {} scrolling from resolved output in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_theirs_scroll)),
                    expected,
                    "conflict markdown theirs preview should {} {} scrolling from resolved output in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });

            let base_offset = axis.offset(px(80.0));
            reset_offsets(cx, &view);
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.main_pane.update(cx, |pane, cx| {
                        set_uniform_list_offset(&pane.conflict_resolver_diff_scroll, base_offset);
                        cx.notify();
                    });
                });
            });
            draw_and_drain_test_window(cx);

            cx.update(|_window, app| {
                let pane = view.read(app).main_pane.read(app);
                let expected = if axis.includes(mode) {
                    axis.component(base_offset)
                } else {
                    px(0.0)
                };
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_resolver_diff_scroll)),
                    axis.component(base_offset),
                    "conflict markdown base preview should keep its {} offset in {:?} mode",
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_ours_scroll)),
                    expected,
                    "conflict markdown ours preview should {} {} scrolling from the base preview in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(uniform_list_offset(&pane.conflict_preview_theirs_scroll)),
                    expected,
                    "conflict markdown theirs preview should {} {} scrolling from the base preview in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
                assert_eq!(
                    axis.component(scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                    )),
                    expected,
                    "conflict markdown resolved output should {} {} scrolling from the base preview in {:?} mode",
                    if axis.includes(mode) { "sync" } else { "not sync" },
                    axis.label(),
                    mode,
                );
            });
        }
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict markdown matrix fixture");
}

#[gpui::test]
fn worktree_markdown_preview_wraps_long_rows_within_the_viewport(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(74);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_word_wrap",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/wrap.md");
    let abs_path = workdir.join(&file_rel);
    // A one-line paragraph to measure a single line against, then one far wider
    // than any test viewport, which therefore has to wrap.
    let source = format!(
        "short\n\n{}\n",
        "wrap this paragraph across many rows ".repeat(40)
    );
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown word wrap workdir");
    std::fs::write(&abs_path, source.as_bytes()).expect("write markdown word wrap fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree markdown word wrap target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    let document = crate::view::markdown_preview::parse_markdown(&source)
        .expect("long paragraph markdown preview should parse");
    let short_row_ix = document
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "short")
        .expect("fixture should contain the short paragraph");
    let long_row_ix = document
        .rows
        .iter()
        .position(|row| row.text.len() > 200)
        .expect("fixture should contain the long paragraph");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(document));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    let draw = |cx: &mut gpui::VisualTestContext| {
        for _ in 0..3 {
            cx.update(|window, app| {
                let _ = window.draw(app);
            });
            cx.run_until_parked();
        }
    };
    draw(cx);

    // The single document lays out as flowing text, so it wraps by itself and
    // never builds the visual-row plan the diff preview's fixed row grid needs.
    assert_eq!(
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane
                    .read(cx)
                    .markdown_preview_wrap
                    .plan_len(MarkdownPreviewList::Worktree)
            })
        }),
        None,
        "the flowing preview must not build a wrap plan"
    );

    let container_bounds = cx
        .debug_bounds("worktree_markdown_preview_scroll_container")
        .expect("expected worktree markdown preview container bounds");
    let short_bounds = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{short_row_ix}"
        )))
        .expect("expected bounds for the one-line paragraph");
    let long_bounds = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{long_row_ix}"
        )))
        .expect("expected bounds for the wrapped paragraph");

    assert!(
        long_bounds.size.width <= container_bounds.size.width + px(1.0),
        "wrapped text must fit the viewport; text={long_bounds:?} container={container_bounds:?}"
    );
    assert!(
        long_bounds.size.height >= short_bounds.size.height * 3.0,
        "a paragraph far wider than the viewport must wrap onto several lines; \
         long={long_bounds:?} short={short_bounds:?}"
    );

    // Hit testing, selection, and copy address rows by document index, which a
    // wrapped row keeps — the whole paragraph stays one row.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let pane = this.main_pane.read(cx);
            let document = match &pane.worktree_markdown_preview {
                gitcomet_state::model::Loadable::Ready(document) => Arc::clone(document),
                other => panic!("expected a ready preview document, got {other:?}"),
            };
            for (row_ix, row) in document.rows.iter().enumerate() {
                assert_eq!(
                    pane.markdown_preview_row_text(row_ix, DiffTextRegion::Inline),
                    row.text,
                    "row {row_ix} must resolve to the whole row the preview painted"
                );
            }
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown word wrap workdir");
}

/// `debug_bounds` takes a `&'static str`; tests that build a selector from a
/// row index need one that outlives the call.
fn leaked_selector(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

#[gpui::test]
fn markdown_diff_preview_draws_rows_that_carry_inline_pictures(cx: &mut gpui::TestAppContext) {
    // The diff preview paints a fixed row grid, so a picture written on a line
    // with text has to fit into the line rather than take a block of its own.
    // Its rows still have to draw.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(81);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_inline_images",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/badges.md");
    let old_text = concat!(
        "# <img alt=\"logo\" src=\"logo.svg\" width=\"26\" /> Title\n",
        "\n",
        "[![One](one.svg)](https://a.example) [![Two](two.svg)](https://b.example)\n",
        "\n",
        "Body before.\n",
    );
    let new_text = old_text.replace("Body before.", "Body after.");

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
                    path.clone(),
                    Some(old_text.to_string()),
                    Some(new_text.clone()),
                ),
            )));
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Split;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_markdown_preview_active());
    });

    // The heading keeps its text beside the logo, and the badges stay on the
    // line they were written on rather than becoming blocks.
    let document = crate::view::markdown_preview::parse_markdown(&new_text)
        .expect("badge markdown should parse");
    let with_pictures: Vec<&str> = document
        .rows
        .iter()
        .filter(|row| !row.inline_images.is_empty())
        .map(|row| row.text.as_ref())
        .collect();
    assert_eq!(
        with_pictures,
        vec!["Title", ""],
        "rows: {:?}",
        document
            .rows
            .iter()
            .map(|row| row.text.as_ref())
            .collect::<Vec<_>>()
    );

    std::fs::remove_dir_all(&workdir).ok();
}

#[gpui::test]
fn markdown_preview_hit_testing_follows_a_row_onto_its_wrapped_lines(
    cx: &mut gpui::TestAppContext,
) {
    // A flowing row covers several visual lines, so a click has to resolve in
    // two dimensions. Reading only the x offset along one shaped line put the
    // caret near the start of the row wherever the reader clicked low and left.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(80);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_wrapped_hit_test",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/wrapped_hits.md");
    let abs_path = workdir.join(&file_rel);
    let source = format!(
        "{}\n",
        "one paragraph wrapped over several lines ".repeat(40)
    );
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown wrapped hit test workdir");
    std::fs::write(&abs_path, source.as_bytes()).expect("write markdown wrapped hit test fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown wrapped hit test target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| format!("repo={:?}", pane.active_repo().map(|repo| repo.id)),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(&source)
                        .expect("wrapped paragraph markdown parses"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected the wrapped paragraph's text box");
    let near_top_right = point(
        text_bounds.right() - px(8.0),
        text_bounds.top() + text_bounds.size.height * 0.1,
    );
    let near_bottom_left = point(
        text_bounds.left() + px(8.0),
        text_bounds.bottom() - text_bounds.size.height * 0.1,
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let top_right = pane
            .diff_text_offset_for_position(0, DiffTextRegion::Inline, near_top_right)
            .expect("the first visual line must resolve to an offset");
        let bottom_left = pane
            .diff_text_offset_for_position(0, DiffTextRegion::Inline, near_bottom_left)
            .expect("the last visual line must resolve to an offset");
        assert!(
            bottom_left > top_right,
            "a click low and left belongs later in the row than one high and right; \
             bottom_left={bottom_left} top_right={top_right} text={text_bounds:?}"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown wrapped hit test workdir");
}

#[gpui::test]
fn worktree_markdown_preview_change_bar_is_unbroken_for_a_wholly_added_file(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(75);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_change_bar",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/added.md");
    let abs_path = workdir.join(&file_rel);
    // A top-level heading makes the preview insert a spacer row, and headings
    // carry vertical insets — both used to punch holes in the change bar.
    let source = "# Title\n\nBody paragraph.\n\n## Section\n\nMore body.\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown change bar workdir");
    std::fs::write(&abs_path, source).expect("write markdown change bar fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree markdown change bar target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    let document = crate::view::markdown_preview::parse_markdown(source)
        .expect("headed markdown preview should parse");
    let last_row_ix = document
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "More body.")
        .expect("fixture should end with a paragraph");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(document));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    // The flowing preview marks the file with one gutter element rather than a
    // segment per row: blocks are separated by margins, and a per-row bar left
    // a hole in every one of them.
    let bar = cx
        .debug_bounds("markdown_preview_change_bar")
        .expect("an added file's preview should carry a change bar");
    let first_row = cx
        .debug_bounds("markdown_preview_row_box_0")
        .expect("expected bounds for the first preview row");
    let last_row = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_row_box_{last_row_ix}"
        )))
        .expect("expected bounds for the last preview row");

    assert!(
        bar.left() < first_row.left(),
        "the change bar belongs in the gutter left of the text; bar={bar:?} row={first_row:?}"
    );
    assert!(
        bar.top() <= first_row.top() && bar.bottom() >= last_row.bottom(),
        "the change bar must run unbroken past every row; \
         bar={bar:?} first={first_row:?} last={last_row:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown change bar workdir");
}

#[gpui::test]
fn split_markdown_diff_word_wrap_keeps_both_columns_row_aligned(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(76);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_split_word_wrap",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/split-wrap.md");
    // One side has a paragraph far wider than the column, the other a short
    // one at the same aligned row, so wrapping the columns independently
    // would slide the left side out of step with the right.
    let old_text = format!(
        "# Guide\n\n{}\n\nshared tail\n",
        "old side text that has to wrap several times ".repeat(20)
    );
    let new_text = "# Guide\n\nshort\n\nshared tail\n".to_string();
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create markdown split wrap workdir");

    seed_file_diff_state(
        cx, &view, repo_id, &workdir, &file_rel, &old_text, &new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown split wrap target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.file_markdown_preview_cache_repo_id = Some(repo_id);
                pane.file_markdown_preview_cache_rev = 1;
                pane.file_markdown_preview_cache_target = Some(target.clone());
                pane.file_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(
                        &old_text, &new_text,
                    )
                    .expect("markdown split diff preview should parse"),
                ));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let pane = this.main_pane.read(cx);
            let old_len = pane
                .markdown_preview_wrap
                .plan_len(MarkdownPreviewList::Old)
                .expect("old column wrap plan");
            let new_len = pane
                .markdown_preview_wrap
                .plan_len(MarkdownPreviewList::New)
                .expect("new column wrap plan");

            assert_eq!(
                old_len, new_len,
                "split columns must render the same number of visual rows"
            );

            let old_plan = pane
                .markdown_preview_wrap
                .plan(MarkdownPreviewList::Old)
                .expect("old plan");
            let new_plan = pane
                .markdown_preview_wrap
                .plan(MarkdownPreviewList::New)
                .expect("new plan");
            let mut saw_wrapped_row = false;
            for visual_ix in 0..old_len {
                let old_visual = old_plan.get(visual_ix).expect("old visual row");
                let new_visual = new_plan.get(visual_ix).expect("new visual row");
                assert_eq!(
                    (old_visual.row_ix, old_visual.wrap_ix),
                    (new_visual.row_ix, new_visual.wrap_ix),
                    "visual row {visual_ix} must show the same aligned source row on both sides"
                );
                saw_wrapped_row |= old_visual.is_continuation();
            }
            assert!(
                saw_wrapped_row,
                "fixture should wrap at least one row; old_len={old_len}"
            );
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown split wrap workdir");
}

#[gpui::test]
fn markdown_preview_ignores_the_text_diff_wrap_projection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(77);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_stale_wrap",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/stale.md");
    let abs_path = workdir.join(&file_rel);
    let source = "# Title\n\nBody paragraph.\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown stale wrap workdir");
    std::fs::write(&abs_path, source).expect("write markdown stale wrap fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown stale wrap target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| format!("diff_target={:?}", pane.active_repo().map(|repo| repo.id)),
    );

    let document = crate::view::markdown_preview::parse_markdown(source).expect("preview parses");
    let row_count = document.rows.len();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(document));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
            // A text diff viewed earlier with wrap on leaves its own visual-row
            // map behind; the preview must not be remapped through it.
            this.set_diff_word_wrap(true, cx);
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.diff_wrap_visible_rows = (0..4)
                    .map(|ix| DiffWrapVisualRow {
                        source_visible_ix: ix + 900,
                        wrap_ix: 0,
                        primary_range: rows::DiffWrapByteRange::from_range(0..1),
                        secondary_range: rows::DiffWrapByteRange::from_range(0..1),
                    })
                    .collect();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let pane = this.main_pane.read(cx);
            assert_eq!(
                pane.markdown_preview_row_count(),
                Some(row_count),
                "the preview row count must come from the preview"
            );
            for visible_ix in 0..row_count {
                assert_eq!(
                    pane.diff_source_visible_ix_for_visible_ix(visible_ix),
                    Some(visible_ix),
                    "the stale diff wrap map must not remap preview row {visible_ix}"
                );
                assert!(
                    pane.diff_text_wrap_for_visible_ix(visible_ix).is_none(),
                    "the stale diff wrap map must not re-slice preview row {visible_ix}"
                );
                assert_eq!(
                    pane.diff_text_line_for_region(visible_ix, DiffTextRegion::Inline),
                    pane.markdown_preview_row_text(visible_ix, DiffTextRegion::Inline),
                    "row {visible_ix} must resolve to the text the preview painted"
                );
            }
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown stale wrap workdir");
}

#[gpui::test]
fn clicking_a_markdown_preview_link_opens_the_open_in_browser_menu(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(78);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_link_menu",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/link.md");
    let abs_path = workdir.join(&file_rel);
    let source = "[the docs](https://example.com/docs)\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown link menu workdir");
    std::fs::write(&abs_path, source).expect("write markdown link menu fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown link menu target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| format!("repo={:?}", pane.active_repo().map(|repo| repo.id)),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("link markdown parses"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    // Left edge of the row's text is inside the link, which spans the row.
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let link = this.main_pane.read(cx).markdown_preview_link_at(
                0,
                DiffTextRegion::Inline,
                on_link,
            );
            assert_eq!(
                link.as_deref(),
                Some("https://example.com/docs"),
                "the click position must resolve to the link destination"
            );

            let popover = this.popover_host.read(cx).popover_kind_for_tests();
            assert!(
                matches!(
                    popover,
                    Some(PopoverKind::MarkdownLinkMenu { ref url })
                        if url.as_ref() == "https://example.com/docs"
                ),
                "clicking a link should open its menu, got {popover:?}"
            );
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown link menu workdir");
}

#[gpui::test]
fn markdown_preview_text_box_starts_where_the_text_is_painted(cx: &mut gpui::TestAppContext) {
    // The selection highlight is painted inside the text box, so the box must
    // be the glyph box. Padding applied to the box itself shifted the highlight
    // left of the text and cut it short at the end of the line.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(79);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_text_box",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/box.md");
    let abs_path = workdir.join(&file_rel);
    let source = "A plain paragraph with enough words to fill the row.\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create markdown text box workdir");
    std::fs::write(&abs_path, source).expect("write markdown text box fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown text box target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| format!("repo={:?}", pane.active_repo().map(|repo| repo.id)),
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::clone(&preview_lines),
                    source.len(),
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("paragraph markdown parses"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let container_bounds = cx
        .debug_bounds("worktree_markdown_preview_scroll_container")
        .expect("expected preview container bounds");
    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");

    assert!(
        text_bounds.left() > container_bounds.left(),
        "the document's left padding must sit outside the text box; \
         container={container_bounds:?} text={text_bounds:?}"
    );
    assert!(
        text_bounds.right() <= container_bounds.right(),
        "the text box must stay inside the preview; \
         container={container_bounds:?} text={text_bounds:?}"
    );

    // The hitbox the selection overlay paints into is the text box, so the two
    // must agree — that is what keeps the highlight on top of the glyphs.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let hitbox = this
                .main_pane
                .read(cx)
                .diff_text_hitbox_bounds_for_tests(0, DiffTextRegion::Inline)
                .expect("expected a diff text hitbox for the preview row");
            assert!(
                (hitbox.left() - text_bounds.left()).abs() <= px(0.5),
                "selection hitbox must start at the text box; \
                 hitbox={hitbox:?} text={text_bounds:?}"
            );
            assert!(
                (hitbox.right() - text_bounds.right()).abs() <= px(0.5),
                "selection hitbox must end at the text box; \
                 hitbox={hitbox:?} text={text_bounds:?}"
            );
        });
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown text box workdir");
}
