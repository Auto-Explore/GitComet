use super::*;
use crate::view::mod_helpers::MarkdownSearchSurface;
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
fn split_markdown_diff_keeps_an_empty_side_at_half_width(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_markdown_empty_side_width",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/added.md");
    let old_text = "";
    let new_text = "# Added\n\nThis side must stay visible.\n";
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create empty-side markdown diff workdir");
    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, old_text, new_text);
    wait_for_main_pane_condition(
        cx,
        &view,
        "empty-side markdown diff target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let old = crate::view::markdown_preview::parse_markdown(old_text)
                    .expect("empty Markdown document should parse");
                let new = crate::view::markdown_preview::parse_markdown(new_text)
                    .expect("added Markdown document should parse");
                pane.diff_view = DiffViewMode::Split;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.file_markdown_preview_cache_repo_id = Some(repo_id);
                pane.file_markdown_preview_cache_rev = 1;
                pane.file_markdown_preview_cache_target = Some(target.clone());
                pane.file_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::MarkdownPreviewDiff {
                        old,
                        inline: new.clone(),
                        new,
                    },
                ));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let left = cx
        .debug_bounds("diff_text_empty_space_SplitLeft")
        .expect("empty Markdown left column surface");
    let right = cx
        .debug_bounds("diff_text_empty_space_SplitRight")
        .expect("nonempty Markdown right column trailing surface");
    assert!(
        left.right() <= right.left(),
        "the empty surface must stay in its own split column: left={left:?} right={right:?}"
    );
    assert!(
        (left.size.width - right.size.width).abs() <= px(2.0),
        "empty and nonempty Markdown columns should retain equal flex widths: left={left:?} right={right:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup empty-side markdown diff workdir");
}

#[gpui::test]
fn secondary_f_from_markdown_file_preview_searches_the_rendered_rows(
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
            RenderedPreviewMode::Rendered,
            "secondary-f should leave the rendered preview on screen and search it in place"
        );
        assert!(
            pane.diff_search_active,
            "secondary-f should activate diff search from markdown preview"
        );
        assert_eq!(
            pane.markdown_search_surface(),
            Some(MarkdownSearchSurface::Worktree),
            "the rendered file preview should be the surface search scans"
        );
    });

    // The rendered rows are what gets scanned: `Title` is the heading's text
    // with the `#` marker already consumed by the renderer.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "preview body".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("preview body", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_search_matches.is_empty(),
            "expected the rendered markdown preview to report a match"
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
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(72),
        "markdown_code_block_width",
        "```sh\necho hi\n```\n",
    );

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

    fixture.cleanup();
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
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(73),
        "markdown_list_selection_box",
        "- first item\n",
    );

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

    fixture.cleanup();
}

#[gpui::test]
fn secondary_f_from_conflict_markdown_preview_searches_the_rendered_rows(
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
            // The rendered preview only builds its documents once all three
            // sides are loaded; without this it sits waiting on a load the test
            // backend never services.
            repo.conflict_state.conflict_file_load_mode =
                gitcomet_state::model::ConflictFileLoadMode::Full;

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
            ConflictResolverPreviewMode::Preview,
            "secondary-f should leave the rendered conflict preview up and search it in place"
        );
        assert!(
            pane.diff_search_active,
            "secondary-f should activate diff search from conflict markdown preview"
        );
        assert_eq!(
            pane.markdown_search_surface(),
            Some(MarkdownSearchSurface::Conflict),
            "the rendered conflict preview should be the surface search scans"
        );
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "conflict markdown preview documents ready",
        |pane| {
            !pane
                .markdown_search_documents(MarkdownSearchSurface::Conflict)
                .is_empty()
        },
        |pane| {
            format!(
                "documents={}",
                pane.markdown_search_documents(MarkdownSearchSurface::Conflict)
                    .len()
            )
        },
    );

    // `Local` is heading text in the rendered columns; the `#` that made it a
    // heading is not, so only the rendered form is findable.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "Local".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("Local", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_search_matches.is_empty(),
            "expected the rendered conflict columns to report a match"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "# Local".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("# Local", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.is_empty(),
            "the heading marker is not on screen, so it must not be searchable; got {:?}",
            pane.diff_search_matches
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict markdown preview fixture");
}

#[gpui::test]
fn a_document_past_the_render_budget_falls_back_to_source(cx: &mut gpui::TestAppContext) {
    // Unlike the size and parser caps, this one is recoverable: the document
    // parsed, it is only too big to lay out at once. Leaving the reader on an
    // empty pane with a message and a toggle to find would be worse than
    // showing them the source.
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(87);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_render_budget",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("huge.md");
    let abs_path = workdir.join(&file_rel);
    let source = "---\n".repeat(crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS + 1);
    assert!(source.len() < crate::view::markdown_preview::MAX_PREVIEW_SOURCE_BYTES);

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create render budget workdir");
    std::fs::write(&abs_path, &source).expect("write render budget fixture");

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

    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(pane, abs_path.clone(), preview_lines, source.len(), cx);
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
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
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Source,
            "an over-budget document should put the reader on the source"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown_preview
        else {
            panic!(
                "expected the preview to report why it refused, got {:?}",
                pane.worktree_markdown_preview
            );
        };
        assert!(
            message.contains("too large to render"),
            "the message should name this limit, not the parser's: {message}"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup render budget workdir");
}

#[gpui::test]
fn the_renderer_refuses_a_document_past_its_budget(cx: &mut gpui::TestAppContext) {
    // The builder's check is an early exit, not the only guard: a caller that
    // hands the renderer an unbounded document still must not make the pane lay
    // one out. Injecting the document directly is exactly that caller.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let source = "---\n".repeat(crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS + 1);
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(88),
        "markdown_renderer_budget",
        &source,
    );

    assert!(
        fixture.document.rows.len() > crate::view::markdown_preview::MAX_FLOWING_PREVIEW_ROWS,
        "the fixture injects a document the builder would have refused"
    );
    assert!(
        cx.debug_bounds("markdown_preview_text_box_0").is_none(),
        "no row of an over-budget document may be laid out"
    );
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_some(),
        "the pane itself still renders, carrying the refusal"
    );

    fixture.cleanup();
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
                pane.conflict_resolver.active_conflict = Some(0);
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
        assert_eq!(pane.conflict_resolver.active_conflict, Some(0));
        assert!(
            pane.conflict_resolver.nav_anchor.is_none(),
            "preview hotkeys should not mutate conflict navigation state"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                pane.conflict_resolver.active_conflict = Some(1);
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
        assert_eq!(pane.conflict_resolver.active_conflict, Some(1));
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
            let mut session = ConflictSession::from_merged_text(
                file_rel.clone(),
                gitcomet_core::domain::FileConflictKind::BothModified,
                ConflictPayload::Text(base_text.clone().into()),
                ConflictPayload::Text(ours_text.clone().into()),
                ConflictPayload::Text(theirs_text.clone().into()),
                &current_text,
            );
            for region in &mut session.regions {
                region.resolution =
                    gitcomet_core::conflict_session::ConflictRegionResolution::PickOurs;
            }
            repo.conflict_state.conflict_session = Some(session);

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
        let pane = view.read(app).main_pane.read(app);
        let output = pane.conflict_resolver_input.read(app).text().to_string();
        assert!(
            output.lines().any(|line| line.len() >= 240),
            "resolved output should retain the selected long markdown source; output_len={} longest_line={}",
            output.len(),
            output.lines().map(str::len).max().unwrap_or_default(),
        );
    });

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
                // In markdown preview the columns render formatted content
                // rather than the aligned row space, so a vertical
                // correspondence with the output text is even less meaningful
                // than in the code view, where it was already dropped. Only
                // the horizontal axis stays coupled.
                let expected = if axis.includes(mode)
                    && matches!(axis, ScrollSyncAxis::Horizontal)
                {
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
                    if axis.includes(mode) && matches!(axis, ScrollSyncAxis::Horizontal) {
                        "sync"
                    } else {
                        "not sync"
                    },
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
                // The columns share one row space and stay coupled on both
                // axes; the resolved output is a separate document and follows
                // only horizontally.
                let output_expected = if axis.includes(mode)
                    && matches!(axis, ScrollSyncAxis::Horizontal)
                {
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
                    output_expected,
                    "conflict markdown resolved output should {} {} scrolling from the base preview in {:?} mode",
                    if axis.includes(mode) && matches!(axis, ScrollSyncAxis::Horizontal) {
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

/// Source offsets of every picture a document carries, in order.
///
/// A picture's element id and debug selector are both keyed on this, so it is
/// how a test names the picture it wants to look at.
fn picture_offsets(
    document: &crate::view::markdown_preview::MarkdownPreviewDocument,
) -> Vec<usize> {
    document
        .rows
        .iter()
        .flat_map(|row| row.inline_images.iter())
        .map(|inline| inline.source_byte)
        .collect()
}

/// A worktree markdown file, seeded and opened in the rendered preview.
///
/// Every rendered-preview test needs the same seven steps: write the file, push
/// a repo state that lists it, wait for the diff target to settle, hand the
/// pane a ready source preview and a parsed document, and draw. Spelling that
/// out per test hid what each one was actually about.
struct RenderedPreviewFixture {
    workdir: std::path::PathBuf,
    document: Arc<crate::view::markdown_preview::MarkdownPreviewDocument>,
}

impl RenderedPreviewFixture {
    fn open(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        repo_id: gitcomet_state::model::RepoId,
        name: &str,
        source: &str,
    ) -> Self {
        Self::open_with_status(
            cx,
            view,
            repo_id,
            name,
            source,
            gitcomet_core::domain::FileStatusKind::Untracked,
        )
    }

    /// The status matters where the preview's gutter does: an added or removed
    /// file draws a change bar, an untracked one does not.
    fn open_with_status(
        cx: &mut gpui::VisualTestContext,
        view: &gpui::Entity<super::super::GitCometView>,
        repo_id: gitcomet_state::model::RepoId,
        name: &str,
        source: &str,
        status: gitcomet_core::domain::FileStatusKind,
    ) -> Self {
        let workdir = open_rendered_markdown_preview(cx, view, repo_id, name, source, status);
        let document = cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            match &pane.worktree_markdown_preview {
                gitcomet_state::model::Loadable::Ready(document) => Arc::clone(document),
                other => panic!("expected a ready preview, got {other:?}"),
            }
        });
        Self { workdir, document }
    }

    /// Document index of the first row whose text is exactly `text`.
    fn row_ix(&self, text: &str) -> usize {
        self.document
            .rows
            .iter()
            .position(|row| row.text.as_ref() == text)
            .unwrap_or_else(|| {
                panic!(
                    "no row reads {text:?}; rows: {:?}",
                    self.document
                        .rows
                        .iter()
                        .map(|row| row.text.as_ref())
                        .collect::<Vec<_>>()
                )
            })
    }

    /// Source offsets of every picture the document carries, in order.
    fn picture_offsets(&self) -> Vec<usize> {
        picture_offsets(&self.document)
    }

    fn cleanup(self) {
        std::fs::remove_dir_all(&self.workdir).expect("cleanup preview fixture");
    }
}

/// Seed a rendered worktree markdown preview for `source` and draw it.
fn open_rendered_markdown_preview(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    name: &str,
    source: &str,
    status: gitcomet_core::domain::FileStatusKind,
) -> std::path::PathBuf {
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_{name}", std::process::id()));
    let file_rel = std::path::PathBuf::from("docs/preview.md");
    let abs_path = workdir.join(&file_rel);
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(abs_path.parent().expect("fixture parent dir"))
        .expect("create preview workdir");
    std::fs::write(&abs_path, source.as_bytes()).expect("write preview fixture");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                status,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    wait_for_main_pane_condition(
        cx,
        view,
        "rendered markdown preview target activation",
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
                        .expect("preview fixture parses"),
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

    workdir
}

#[gpui::test]
fn markdown_preview_hitboxes_follow_the_scrolled_viewport(cx: &mut gpui::TestAppContext) {
    // Rows are only hit-testable near the window. Every other preview test uses
    // a fixture that fits on screen, so nothing else exercises the gate — and a
    // gate reading the wrong coordinate space would reject visible rows and
    // silently stop selection working in any scrolled preview.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Far taller than any test window, so the tail starts well off screen.
    let source = (0..400)
        .map(|ix| format!("Paragraph number {ix}.\n\n"))
        .collect::<String>();
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(83),
        "markdown_scrolled_hitboxes",
        &source,
    );

    let last_row_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.markdown_preview_row_count()
            .expect("a rendered preview has rows")
            - 1
    });

    let hitbox = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .diff_text_hitbox_bounds_for_tests(row_ix, DiffTextRegion::Inline)
        })
    };

    assert!(
        hitbox(cx, 0).is_some(),
        "the first row is on screen before scrolling"
    );
    assert!(
        hitbox(cx, last_row_ix).is_none(),
        "the tail of a tall document starts far below the window"
    );

    // Scroll to the bottom; the two ends swap.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                let handle = pane.worktree_preview_scroll.0.borrow().base_handle.clone();
                let max = scroll_handle_max_offset(&handle).height;
                set_scroll_handle_offset(&handle, point(px(0.0), -max));
            });
        });
    });
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    assert!(
        hitbox(cx, last_row_ix).is_some(),
        "the last row is hit-testable once it is on screen"
    );
    assert!(
        hitbox(cx, 0).is_none(),
        "and the first row stops being, now that it is far above"
    );

    fixture.cleanup();
}

#[gpui::test]
fn clicking_a_badge_opens_its_menu_without_arming_a_selection(cx: &mut gpui::TestAppContext) {
    // The row under a picture also listens for a left press, so without the
    // picture stopping propagation the click opens the menu *and* starts a
    // drag-selection behind it.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Two badges, so they stay inline instead of one alone becoming a block.
    let source = "[![one](badge.svg)](https://example.com/badge)\n[![two](badge.svg)](https://example.com/other)\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(85),
        "markdown_badge_click",
        source,
    );
    std::fs::write(
        fixture.workdir.join("docs/badge.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"20\"><rect width=\"80\" height=\"20\"/></svg>",
    )
    .expect("write the badge the link points at");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a picture");
    let badge = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the badge is drawn");

    simulate_counted_click(cx, badge.center(), 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let popover = this.popover_host.read(cx).popover_kind_for_tests();
            assert!(
                matches!(
                    popover,
                    Some(PopoverKind::WebLinkMenu { ref url, .. })
                        if url.as_ref() == "https://example.com/badge"
                ),
                "clicking a badge opens its link menu, got {popover:?}"
            );
            assert!(
                !this.main_pane.read(cx).diff_text_selecting,
                "and the row underneath must not have started selecting text"
            );
        });
    });

    fixture.cleanup();
}

#[gpui::test]
fn linked_blocked_image_menu_loads_one_image_only_in_ask_mode(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
                cx,
            );
        });
    });

    // Keep two pictures in one paragraph so both remain inline and wrapped in
    // their respective links.
    let first_image_url = "https://images.example.invalid/one.svg";
    let second_image_url = "https://images.example.invalid/two.svg";
    let source = format!(
        "[![one]({first_image_url})](https://example.com/one) \
         [![two]({second_image_url})](https://example.com/two)\n"
    );
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(108),
        "markdown_linked_remote_image_approval",
        &source,
    );
    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a linked picture");
    let retry = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_load_{source_byte}"
        )))
        .expect("Ask mode draws the linked image's Retry control");

    // Mouse-down belongs to the enclosing link wrapper, so it opens a menu
    // before the nested Retry control can receive a complete click.
    simulate_counted_click(cx, retry.center(), 1);
    cx.run_until_parked();
    cx.update(|_window, app| {
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(matches!(
            popover,
            Some(PopoverKind::WebLinkMenu {
                ref url,
                load_remote_image_url: Some(ref image_url),
            }) if url.as_ref() == "https://example.com/one"
                && image_url.as_ref() == first_image_url
        ));
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let load_image = cx
        .debug_bounds("context_menu_load_image")
        .expect("a linked blocked image menu offers Load image")
        .center();
    cx.simulate_mouse_move(
        load_image,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: load_image,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.read(app);
        assert_eq!(main_pane.approved_remote_markdown_image_urls.len(), 1);
        assert!(
            main_pane
                .approved_remote_markdown_image_urls
                .contains(first_image_url)
        );
        assert!(
            !main_pane
                .approved_remote_markdown_image_urls
                .contains(second_image_url),
            "Load image must approve only the image represented by the menu"
        );
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::NeverLoad,
                cx,
            );
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let blocked = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_load_{source_byte}_blocked_box"
        )))
        .expect("Never mode draws the linked image's blocked control");
    simulate_counted_click(cx, blocked.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        cx.debug_bounds("context_menu_load_image").is_none(),
        "Never mode must not offer any path to approve the remote image"
    );
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.read(app);
        assert!(main_pane.approved_remote_markdown_image_urls.is_empty());
        let popover = view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests();
        assert!(matches!(
            popover,
            Some(PopoverKind::WebLinkMenu {
                load_remote_image_url: None,
                ..
            })
        ));
    });

    fixture.cleanup();
}

#[gpui::test]
fn preview_mode_copies_the_document_it_draws(cx: &mut gpui::TestAppContext) {
    // The counterpart to `source_mode_copies_the_file_exactly_as_written`: the
    // rendered preview copies what it drew, so the heading loses its `#` and
    // the section break under it comes back as the blank line it looks like.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(96),
        "markdown_preview_copy",
        "# Title\n\nBody paragraph.\n",
    );
    let first = fixture.row_ix("Title");
    let last = fixture.row_ix("Body paragraph.");
    let last_len = fixture.document.rows[last].text.len();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(
                    pane.is_markdown_preview_active(),
                    "the fixture must be showing the rendered document"
                );
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: first,
                    region: DiffTextRegion::Inline,
                    offset: 0,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: last,
                    region: DiffTextRegion::Inline,
                    offset: last_len,
                });
                cx.notify();
            });
        });
    });

    let copied = copied_preview_selection(cx, &view).expect("selecting the preview should copy it");
    assert_eq!(copied, "Title\n\nBody paragraph.");

    fixture.cleanup();
}

#[gpui::test]
fn source_mode_word_wrap_splits_a_long_line_over_several_rows(cx: &mut gpui::TestAppContext) {
    // Text mode draws the file through the same list every source view uses,
    // and that list took the file's line count as its length — so the Word wrap
    // toggle had nothing to act on and a long line just ran off the pane.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let long = "wrap this sentence over several rows ".repeat(12);
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(99),
        "markdown_source_word_wrap",
        &format!("Short.\n\n{long}\n"),
    );
    let set_mode_and_wrap = |cx: &mut gpui::VisualTestContext, wrap: bool| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.rendered_preview_modes
                        .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);
                    pane.diff_word_wrap = wrap;
                    cx.notify();
                });
            });
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };

    set_mode_and_wrap(cx, false);
    let (lines, unwrapped) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.worktree_preview_line_count()
                .expect("the file is ready"),
            pane.worktree_preview_visible_len()
                .expect("the list has rows"),
        )
    });
    assert_eq!(
        unwrapped, lines,
        "with wrap off the list draws one row per line"
    );

    set_mode_and_wrap(cx, true);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let wrapped = pane
            .worktree_preview_visible_len()
            .expect("the list has rows");
        assert!(
            pane.worktree_preview_wrap_active(),
            "turning the toggle on has to reach the file preview"
        );
        assert!(
            wrapped > lines,
            "a line far wider than the pane occupies several rows; \
             wrapped={wrapped} lines={lines}"
        );

        // The rows are slices of one line, in order, covering all of it.
        let long_ix = lines - 2;
        let slices: Vec<_> = (0..wrapped)
            .filter(|ix| pane.diff_source_visible_ix_for_visible_ix(*ix) == Some(long_ix))
            .filter_map(|ix| pane.diff_text_wrap_for_visible_ix(ix))
            .collect();
        assert!(
            slices.len() > 1,
            "the long line is the one that wrapped, got {} rows",
            slices.len()
        );
        assert_eq!(
            slices[0].primary_range.start, 0,
            "the first row opens the line"
        );
        for pair in slices.windows(2) {
            assert_eq!(
                pair[0].primary_range.end, pair[1].primary_range.start,
                "each row picks up where the one above it stopped"
            );
        }
    });

    fixture.cleanup();
}

#[gpui::test]
fn source_mode_word_wrap_columns_are_measured_in_the_editor_font(cx: &mut gpui::TestAppContext) {
    // The trap this repeats from the diff: the rows are painted in the editor
    // font, but the wrap width is worked out while the ambient UI font is still
    // current. Measuring the wrong face gives the wrong column count, and every
    // wrapped row lands short or runs past the pane.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let long = "measure this in the right font ".repeat(20);
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(100),
        "markdown_source_wrap_font",
        &format!("{long}\n"),
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);
                pane.diff_word_wrap = true;
                cx.notify();
            });
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let editor_font_family = crate::font_preferences::current_editor_font_family(app);
        let main_pane = view.read(app).main_pane.clone();
        let (measured, columns) = main_pane.update(app, |pane, cx| {
            (
                pane.diff_wrap_measure_font_family(cx),
                pane.worktree_preview_wrap_columns(window, cx),
            )
        });

        assert_eq!(
            measured.as_ref(),
            editor_font_family.as_str(),
            "preview wrap columns must be measured in the editor font the rows are painted in"
        );
        // Without this the assertion above guards nothing: it would pass just as
        // well if both fonts happened to be the same.
        assert_ne!(
            window.text_style().font_family.as_ref(),
            editor_font_family.as_str(),
            "ambient text style unexpectedly matches the editor font — this test \
             no longer guards anything"
        );

        // And the projection has to have used that count, not merely reported it.
        let pane = main_pane.read(app);
        let rows = (0..pane.worktree_preview_visible_len().unwrap_or(0))
            .filter(|ix| pane.diff_source_visible_ix_for_visible_ix(*ix) == Some(0))
            .count();
        let expected = long.trim_end().len().div_ceil(columns);
        assert_eq!(
            rows,
            expected,
            "the long line should occupy ceil(len / columns) rows; \
             columns={columns} len={}",
            long.trim_end().len()
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn source_mode_copies_the_file_exactly_as_written(cx: &mut gpui::TestAppContext) {
    // The two modes copy different things: the rendered preview copies the
    // document it draws, but Text mode is showing the file itself, so a
    // selection there has to come back byte for byte — every tag, marker, and
    // blank line the author wrote.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // A picture is what pulls the two modes furthest apart: the rendered
    // document spreads one over several rows, while the file has it on a line.
    let source = "# Title\n\n<img alt=\"demo\" src=\"demo.png\" width=\"26\" />\n\nSome **bold** text.\n\n![second](other.png)\n\n- a list item\n\nTail line.\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(95),
        "markdown_source_copy",
        source,
    );
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

    let lines: Vec<&str> = source.lines().collect();
    let last_ix = lines.len() - 1;
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(
                    !pane.is_markdown_preview_active(),
                    "the fixture must be showing the file, not the rendered document"
                );
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: 0,
                    region: DiffTextRegion::Inline,
                    offset: 0,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: last_ix,
                    region: DiffTextRegion::Inline,
                    offset: lines[last_ix].len(),
                });
                cx.notify();
            });
        });
    });

    let copied = copied_preview_selection(cx, &view).expect("selecting the file should copy it");
    assert_eq!(
        copied,
        lines.join("\n"),
        "Text mode copies the file verbatim; every line the selection covers belongs in it"
    );

    fixture.cleanup();
}

#[gpui::test]
fn copying_a_link_address_says_that_it_was_copied(cx: &mut gpui::TestAppContext) {
    // Nothing on screen changes when a link's address goes to the clipboard —
    // the document shows the link's text, never its destination — so the copy
    // has to say so or the reader cannot tell it happened.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Two badges, so they stay inline instead of one alone becoming a block.
    let source = "[![one](badge.svg)](https://example.com/badge)\n[![two](badge.svg)](https://example.com/other)\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(94),
        "markdown_copy_link_toast",
        source,
    );
    std::fs::write(
        fixture.workdir.join("docs/badge.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"80\" height=\"20\"><rect width=\"80\" height=\"20\"/></svg>",
    )
    .expect("write the badge the link points at");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a picture");
    let badge = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the badge is drawn");

    simulate_counted_click(cx, badge.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let copy_entry = cx
        .debug_bounds("context_menu_copy_link_address")
        .expect("the link menu offers copying the address")
        .center();
    // Menu entries fire on release.
    cx.simulate_mouse_move(
        copy_entry,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: copy_entry,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("https://example.com/badge".to_string())
    );
    cx.update(|_window, app| {
        let toasts = view.read(app).toast_host.read(app).toasts_for_tests(app);
        assert_eq!(
            toasts,
            vec![(
                crate::view::components::ToastKind::Success,
                "Link copied to clipboard".to_string()
            )],
            "copying a link address confirms itself"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn a_wide_table_scrolls_while_a_narrow_one_still_spans_the_pane(cx: &mut gpui::TestAppContext) {
    // A table sizes to its content for the same reason a code block does, so a
    // wide one has somewhere to scroll — but a narrow one must not shrink away
    // from the pane it used to fill.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let wide_cell = "w".repeat(200);
    let source = format!(
        "| a | b |\n| --- | --- |\n| c | d |\n\n| {wide_cell} | {wide_cell} |\n| --- | --- |\n| e | f |\n"
    );
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(90),
        "markdown_table_scroll",
        &source,
    );

    let first_rows: Vec<usize> = fixture
        .document
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::TableRow { is_header: true }
            )
        })
        .map(|(ix, _)| ix)
        .collect();
    assert_eq!(first_rows.len(), 2, "the fixture has two tables");

    let container = cx
        .debug_bounds("worktree_markdown_preview_scroll_container")
        .expect("expected the preview container");
    let narrow = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_row_box_{}",
            first_rows[0]
        )))
        .expect("the narrow table's header row");
    let wide = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_row_box_{}",
            first_rows[1]
        )))
        .expect("the wide table's header row");

    assert!(
        wide.size.width > container.size.width,
        "the wide table must exceed the pane so it can scroll; \
         table={wide:?} container={container:?}"
    );
    assert!(
        narrow.size.width <= container.size.width,
        "and the narrow one must not; table={narrow:?} container={container:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_code_block_wider_than_the_pane_gets_a_scrollbar(cx: &mut gpui::TestAppContext) {
    // A block that scrolls sideways with nothing to say so leaves the reader
    // with no idea there is more of the line, and no way to reach it but a
    // horizontal wheel. The bar is drawn for every block, but only has a thumb
    // where there is somewhere to scroll to.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let wide = "x".repeat(400);
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(98),
        "markdown_code_block_scrollbar",
        &format!("```sh\nfits\n```\n\nBetween.\n\n```sh\n{wide}\n```\n"),
    );

    let first_rows: Vec<usize> = fixture
        .document
        .rows
        .iter()
        .enumerate()
        .filter_map(|(ix, row)| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::CodeLine {
                    is_first: true,
                    ..
                }
            )
            .then_some(ix)
        })
        .collect();
    assert_eq!(
        first_rows.len(),
        2,
        "the fixture opens with two code blocks"
    );

    assert!(
        cx.debug_bounds("markdown_document_code_block_scrollbar")
            .is_some(),
        "a code block carries its own horizontal scrollbar"
    );

    cx.update(|_window, app| {
        let scrolls = view
            .read(app)
            .main_pane
            .read(app)
            .worktree_markdown_preview_block_scrolls
            .clone();
        let narrow = scrolls
            .max_scroll_for_tests(first_rows[0])
            .expect("the narrow block is tracked");
        let wide = scrolls
            .max_scroll_for_tests(first_rows[1])
            .expect("the wide block is tracked");
        assert_eq!(
            narrow,
            px(0.0),
            "a block that fits has nowhere to scroll, so its bar stays empty"
        );
        assert!(
            wide > px(0.0),
            "and one that overflows gives its bar a thumb; got {wide:?}"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn a_code_block_does_not_swallow_the_page_scroll(cx: &mut gpui::TestAppContext) {
    // `gpui` sends a plain wheel to whichever axis an element scrolls, so a
    // block that only scrolls sideways would take the page's scroll the moment
    // the pointer crossed it and the document would stop moving.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let wide = "x".repeat(400);
    let filler = (0..200)
        .map(|ix| format!("Paragraph {ix}.\n\n"))
        .collect::<String>();
    let source = format!("```sh\nfirst {wide}\n```\n\n{filler}");
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(91),
        "markdown_code_block_wheel",
        &source,
    );

    let first_row = fixture
        .document
        .rows
        .iter()
        .position(|row| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::CodeLine {
                    is_first: true,
                    ..
                }
            )
        })
        .expect("the fixture opens with a code block");
    let body = |cx: &mut gpui::VisualTestContext| {
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_code_body_{first_row}"
        )))
        .expect("the code body should be drawn")
    };
    let page_offset = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y
        })
    };

    let block_before = body(cx).left();
    let page_before = page_offset(cx);

    // A plain vertical wheel with the pointer over the code block.
    let over_block = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_code_shell_{first_row}"
        )))
        .expect("the code shell should be drawn")
        .center();
    cx.simulate_mouse_move(over_block, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: over_block,
        delta: gpui::ScrollDelta::Pixels(point(px(0.0), px(-160.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    assert!(
        page_offset(cx) < page_before,
        "the document scrolls; before={page_before:?} after={:?}",
        page_offset(cx)
    );
    assert_eq!(
        body(cx).left(),
        block_before,
        "and the block underneath the pointer does not move sideways"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_preview_code_blocks_scroll_independently(cx: &mut gpui::TestAppContext) {
    // A code line longer than the pane scrolls rather than wrapping or being
    // clipped, and each block holds its own offset — which is what the per-block
    // element id is for. A shared id made them scroll as one.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let wide = "x".repeat(400);
    let source = format!("```sh\nfirst {wide}\n```\n\ntext\n\n```sh\nsecond {wide}\n```\n");
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(86),
        "markdown_code_block_scroll",
        &source,
    );

    // Both blocks are keyed on the row their code starts at.
    let first_rows: Vec<usize> = fixture
        .document
        .rows
        .iter()
        .enumerate()
        .filter(|(_, row)| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::CodeLine {
                    is_first: true,
                    ..
                }
            )
        })
        .map(|(ix, _)| ix)
        .collect();
    assert_eq!(first_rows.len(), 2, "the fixture has two code blocks");

    let shell = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_code_shell_{row_ix}"
        )))
        .unwrap_or_else(|| panic!("code shell for row {row_ix} should be drawn"))
    };
    let body = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_code_body_{row_ix}"
        )))
        .unwrap_or_else(|| panic!("code body for row {row_ix} should be drawn"))
    };

    let scrolled_before = body(cx, first_rows[0]);
    let other_before = body(cx, first_rows[1]);
    assert!(
        scrolled_before.size.width > shell(cx, first_rows[0]).size.width,
        "a long line must exceed its block, or there is nothing to scroll; \
         body={scrolled_before:?} shell={:?}",
        shell(cx, first_rows[0])
    );

    // Scroll the first block sideways; only it may move. The wheel is aimed at
    // the shell, which is what carries the scroll hitbox — the body now reaches
    // well past the window.
    let over_first = shell(cx, first_rows[0]).center();
    cx.simulate_mouse_move(over_first, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: over_first,
        delta: gpui::ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    let scrolled_after = body(cx, first_rows[0]);
    let other_after = body(cx, first_rows[1]);

    assert!(
        scrolled_after.left() < scrolled_before.left(),
        "the scrolled block moves; before={scrolled_before:?} after={scrolled_after:?}"
    );
    assert_eq!(
        other_after.left(),
        other_before.left(),
        "the other block keeps its own offset; before={other_before:?} after={other_after:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_preview_draws_an_inline_picture_beside_its_heading(cx: &mut gpui::TestAppContext) {
    // The pictures are sized by `max_h` against a `flex_none` wrapper, which is
    // the kind of constraint that can collapse to zero without any parse-level
    // assertion noticing.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let logo = "docs/logo.svg";
    let source = "# <img alt=\"logo\" src=\"logo.svg\" width=\"26\" /> Title\n\nBody.\n";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(84),
        "markdown_inline_picture_bounds",
        source,
    );
    std::fs::write(
        fixture.workdir.join(logo),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"26\" height=\"26\"><rect width=\"26\" height=\"26\"/></svg>",
    )
    .expect("write the logo the heading points at");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let source_byte = *fixture
        .picture_offsets()
        .first()
        .expect("the fixture carries a picture");

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_inline_image_{source_byte}"
        )))
        .expect("the inline picture is drawn");
    assert!(
        picture.size.width > px(0.0) && picture.size.height > px(0.0),
        "the picture must occupy space: {picture:?}"
    );
    let heading = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("the heading text box");
    assert!(
        picture.right() <= heading.left() + px(1.0),
        "the logo sits before the heading text it belongs to; picture={picture:?} text={heading:?}"
    );

    fixture.cleanup();
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

    // And the row grid actually draws them: the pictures are sized against a
    // `flex_none` wrapper, which can collapse without any parse-level
    // assertion noticing.
    let source_bytes = picture_offsets(&document);
    assert!(!source_bytes.is_empty(), "the fixture carries pictures");
    for source_byte in source_bytes {
        let picture = cx
            .debug_bounds(leaked_selector(format!(
                "markdown_preview_inline_image_{source_byte}"
            )))
            .unwrap_or_else(|| panic!("picture at {source_byte} should be drawn"));
        assert!(
            picture.size.width > px(0.0) && picture.size.height > px(0.0),
            "a picture in the diff preview must occupy space: {picture:?}"
        );
    }

    std::fs::remove_dir_all(&workdir).ok();
}

#[gpui::test]
fn markdown_preview_selection_highlights_every_line_of_a_wrapped_row(
    cx: &mut gpui::TestAppContext,
) {
    // The highlight is a paint-time computation, so a regression that puts
    // every quad on the first visual line, or steps them by the wrong amount,
    // is invisible to every other assertion in this file.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(82),
        "markdown_wrapped_selection",
        &format!("{}\n", "select this paragraph across its lines ".repeat(40)),
    );

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected the wrapped paragraph's text box");

    // A triple click selects the whole source row, so every visual line the row
    // occupies has to carry a highlight.
    simulate_counted_click(cx, text_bounds.center(), 3);
    cx.run_until_parked();
    crate::view::rows::clear_markdown_selection_paint_log_for_tests();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let rects = crate::view::rows::markdown_selection_paint_log_for_tests(0);
    assert!(
        rects.len() >= 3,
        "a paragraph wrapped over several lines needs a quad per line, got {}: text={text_bounds:?}",
        rects.len()
    );

    let line_height = rects[0].size.height;
    assert!(line_height > px(0.0), "quads must have height: {rects:?}");
    for (ix, pair) in rects.windows(2).enumerate() {
        let (above, below) = (pair[0], pair[1]);
        assert!(
            (below.top() - above.top() - line_height).abs() <= px(0.5),
            "quad {} must sit exactly one line under quad {ix}; above={above:?} below={below:?}",
            ix + 1
        );
        assert_eq!(
            below.size.height, above.size.height,
            "every line of one selection is the same height: {rects:?}"
        );
    }
    for rect in &rects {
        assert!(
            rect.left() >= text_bounds.left() - px(0.5)
                && rect.right() <= text_bounds.right() + px(0.5),
            "a quad must stay inside the text box; quad={rect:?} text={text_bounds:?}"
        );
    }
    // The middle lines of a fully selected row are covered end to end, which is
    // what distinguishes a real multi-line highlight from one box per line at
    // the same x.
    let widest = rects
        .iter()
        .map(|rect| rect.size.width)
        .fold(px(0.0), |a, b| if b > a { b } else { a });
    assert!(
        widest > text_bounds.size.width * 0.5,
        "a wrapped selection must cover whole lines, widest={widest:?} text={text_bounds:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_preview_selection_paints_over_inline_code_backgrounds(cx: &mut gpui::TestAppContext) {
    // Inline-code styling owns a run background. If StyledText paints that
    // background after the selection quad, the selected part of the code span
    // looks unselected even though copy and selection geometry are correct.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let row_text = "before inline code after";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(106),
        "markdown_inline_code_selection_layer",
        "before `inline code` after\n",
    );
    let row_ix = fixture.row_ix(row_text);
    assert!(
        fixture.document.rows[row_ix]
            .inline_spans
            .iter()
            .any(|span| { span.style == crate::view::markdown_preview::MarkdownInlineStyle::Code }),
        "the fixture must carry the background-producing inline-code span"
    );

    let text_bounds = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{row_ix}"
        )))
        .expect("expected the inline-code paragraph's text box");
    simulate_counted_click(cx, text_bounds.center(), 3);
    cx.run_until_parked();

    crate::view::rows::begin_markdown_flow_paint_phase_capture_for_tests();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    use crate::view::rows::MarkdownFlowPaintPhase::{Glyphs, RunBackgrounds, Selection};
    assert_eq!(
        crate::view::rows::markdown_flow_paint_phases_for_tests(row_ix),
        vec![RunBackgrounds, Selection, Glyphs],
        "selection must be composited between inline-code backgrounds and glyphs"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_partial_wrapped_selection_starts_and_ends_where_the_drag_did(cx: &mut gpui::TestAppContext) {
    // Selecting a whole row is the easy case: every quad spans its line. A drag
    // that starts and ends mid-line is where the first and last quads have to
    // be measured rather than assumed.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(89),
        "markdown_partial_selection",
        &format!("{}\n", "drag across part of this paragraph ".repeat(40)),
    );

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected the wrapped paragraph's text box");
    let line_height = text_bounds.size.height / 6.0;
    // Start a third of the way into the second visual line and end two thirds
    // across the fourth, so both ends fall mid-line.
    let start = point(
        text_bounds.left() + text_bounds.size.width / 3.0,
        text_bounds.top() + line_height * 1.5,
    );
    let end = point(
        text_bounds.left() + text_bounds.size.width * 2.0 / 3.0,
        text_bounds.top() + line_height * 3.5,
    );

    drag_preview_selection(cx, start, end);
    crate::view::rows::clear_markdown_selection_paint_log_for_tests();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let rects = crate::view::rows::markdown_selection_paint_log_for_tests(0);
    assert!(
        rects.len() >= 2,
        "a drag spanning visual lines needs a quad per line, got {}",
        rects.len()
    );

    let first = rects.first().expect("a first quad");
    let last = rects.last().expect("a last quad");
    assert!(
        first.left() > text_bounds.left() + px(1.0),
        "the first quad starts where the drag did, not at the line start; \
         quad={first:?} text={text_bounds:?}"
    );
    assert!(
        last.right() < text_bounds.right() - px(1.0),
        "and the last stops where it ended, not at the line end; \
         quad={last:?} text={text_bounds:?}"
    );
    // Whatever lies between them is a whole line.
    for middle in rects.iter().take(rects.len().saturating_sub(1)).skip(1) {
        assert!(
            middle.size.width > text_bounds.size.width * 0.5,
            "a line inside the selection is covered end to end: {middle:?}"
        );
    }

    fixture.cleanup();
}

/// Press at `from`, drag to `to`, release.
///
/// A click and a drag are different gestures: the press begins the selection,
/// the move extends it, and only the release ends it.
fn drag_preview_selection(
    cx: &mut gpui::VisualTestContext,
    from: gpui::Point<Pixels>,
    to: gpui::Point<Pixels>,
) {
    cx.simulate_mouse_move(from, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: from,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_mouse_move(
        to,
        Some(gpui::MouseButton::Left),
        gpui::Modifiers::default(),
    );
    cx.simulate_event(gpui::MouseUpEvent {
        position: to,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
    });
    cx.run_until_parked();
}

/// Whatever the preview's selection would put on the clipboard.
fn copied_preview_selection(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
) -> Option<String> {
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.copy_selected_diff_text_to_clipboard(cx)
        });
    });
    cx.read_from_clipboard().and_then(|item| item.text())
}

#[gpui::test]
fn an_inter_block_gap_starts_markdown_selection_upward_and_downward(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(100),
        "markdown_inter_block_gap_selection",
        "Above block.\n\n## Middle block\n\nBelow block.\n",
    );
    let middle_row_ix = fixture.row_ix("Middle block");
    let gap = cx
        .debug_bounds("markdown_preview_block_gap_1")
        .expect("interactive gap before the heading");
    let above = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Above block.")
        )))
        .expect("paragraph above the gap");
    let below = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Below block.")
        )))
        .expect("paragraph below the gap");
    assert!(
        gap.top() >= above.bottom() && gap.bottom() <= below.top(),
        "the selectable gap must occupy only the space between blocks: gap={gap:?} above={above:?} below={below:?}"
    );

    cx.simulate_click(gap.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: middle_row_ix,
            region: DiffTextRegion::Inline,
            offset: 0,
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });

    drag_preview_selection(cx, gap.center(), point(above.left(), above.center().y));
    let upward = copied_preview_selection(cx, &view)
        .expect("dragging upward from the block gap should select text");
    assert!(upward.contains("Above block."), "upward={upward:?}");
    assert!(
        !upward.contains("Middle block") && !upward.contains("Below block."),
        "an upward drag should stop at the following block boundary: {upward:?}"
    );

    drag_preview_selection(cx, gap.center(), point(below.right(), below.center().y));
    let downward = copied_preview_selection(cx, &view)
        .expect("dragging downward from the block gap should select text");
    assert!(
        downward.contains("Middle block") && downward.contains("Below block."),
        "a downward drag should start with the following block: {downward:?}"
    );
    assert!(
        !downward.contains("Above block."),
        "a downward drag must not reach behind the gap boundary: {downward:?}"
    );

    let selection_before_menu = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (pane.diff_text_anchor, pane.diff_text_head)
    });
    cx.simulate_mouse_down(gap.center(), MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(gap.center(), MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (pane.diff_text_anchor, pane.diff_text_head)
        }),
        selection_before_menu,
        "opening the gap context menu should preserve the selection"
    );
    assert_eq!(
        cx.update(|_window, app| view.read(app).active_context_menu_invoker.clone()),
        Some("diff_editor_menu".into())
    );

    fixture.cleanup();
}

#[gpui::test]
fn fenced_code_padding_starts_flowing_selection_at_code_boundaries(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(102),
        "markdown_code_padding_selection",
        "Above block.\n\n```rust\nshared_call();\n```\n\nBelow block.\n",
    );
    let code_ix = fixture.row_ix("shared_call();");
    let top = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_code_padding_top_{code_ix}"
        )))
        .expect("interactive padding above fenced code");
    let bottom = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_code_padding_bottom_{code_ix}"
        )))
        .expect("interactive padding below fenced code");
    let above = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Above block.")
        )))
        .expect("paragraph above fenced code");
    let below = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Below block.")
        )))
        .expect("paragraph below fenced code");

    cx.simulate_click(top.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: code_ix,
            region: DiffTextRegion::Inline,
            offset: 0,
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });
    drag_preview_selection(cx, top.center(), point(below.right(), below.center().y));
    let from_top = copied_preview_selection(cx, &view)
        .expect("dragging from fenced-code top padding should select text");
    assert!(
        from_top.contains("shared_call();") && from_top.contains("Below block."),
        "the top code boundary should select the code and following paragraph: {from_top:?}"
    );
    assert!(
        !from_top.contains("Above block."),
        "the top code boundary must exclude the preceding paragraph: {from_top:?}"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_local_selection_range(code_ix, DiffTextRegion::Inline),
            Some(0.."shared_call();".len()),
            "the flowing fenced-code row should receive a full highlight"
        );
    });

    cx.simulate_click(bottom.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: code_ix,
            region: DiffTextRegion::Inline,
            offset: "shared_call();".len(),
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });
    drag_preview_selection(cx, bottom.center(), point(above.left(), above.center().y));
    let from_bottom = copied_preview_selection(cx, &view)
        .expect("dragging upward from fenced-code bottom padding should select text");
    assert!(
        from_bottom.contains("Above block.") && from_bottom.contains("shared_call();"),
        "the bottom code boundary should select the code and preceding paragraph: {from_bottom:?}"
    );
    assert!(
        !from_bottom.contains("Below block."),
        "the bottom code boundary must exclude the following paragraph: {from_bottom:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn split_markdown_block_gaps_start_selection_in_both_columns(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(101);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_markdown_gap_selection",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/split-gaps.md");
    let old_text = concat!(
        "Above block.\n\n",
        "## Middle block\n\n",
        "Paragraph before code.\n\n",
        "```rust\nshared_call();\n```\n\n",
        "Paragraph before list.\n\n",
        "- shared item\n\n",
        "Below old.\n",
    );
    let new_text = concat!(
        "Above block.\n\n",
        "## Middle block\n\n",
        "Paragraph before code.\n\n",
        "```rust\nshared_call();\n```\n\n",
        "Paragraph before list.\n\n",
        "- shared item\n\n",
        "Below new.\n",
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: path.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let preview = crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
        .expect("split Markdown gap fixture should parse");
    let above_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "Above block.")
        .expect("old paragraph above the gap");
    let middle_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "Middle block")
        .expect("old heading below the gap");
    let below_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "Below old.")
        .expect("old paragraph below the gap");
    let code_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "shared_call();")
        .expect("old fenced-code row");
    let before_list_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "Paragraph before list.")
        .expect("old paragraph before list");
    let list_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "shared item")
        .expect("old list row");
    let gap_ix = (above_ix + 1..middle_ix)
        .find(|&row_ix| {
            matches!(
                preview.old.rows[row_ix].kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::Spacer
            )
        })
        .expect("old split column should retain a spacer before the heading");
    assert!(
        matches!(
            preview.new.rows.get(gap_ix).map(|row| row.kind),
            Some(crate::view::markdown_preview::MarkdownPreviewRowKind::Spacer)
        ),
        "the aligned new column should have the same spacer boundary"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create split Markdown gap workdir");
    seed_file_diff_state(cx, &view, repo_id, &workdir, &path, old_text, new_text);
    wait_for_main_pane_condition(
        cx,
        &view,
        "split Markdown gap target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
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
                pane.file_markdown_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(preview));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
            this.set_diff_word_wrap(false, cx);
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let left_gap = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_gap_SplitLeft_{gap_ix}"
        )))
        .expect("interactive spacer in the old split column");
    let right_gap = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_gap_SplitRight_{gap_ix}"
        )))
        .expect("interactive spacer in the new split column");
    assert!(
        left_gap.right() <= right_gap.left(),
        "each gap must remain inside its own split column: left={left_gap:?} right={right_gap:?}"
    );

    cx.simulate_click(right_gap.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: gap_ix,
            region: DiffTextRegion::SplitRight,
            offset: 0,
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });

    cx.simulate_click(left_gap.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: gap_ix,
            region: DiffTextRegion::SplitLeft,
            offset: 0,
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });

    let above = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        above_ix,
        DiffTextRegion::SplitLeft,
        0..1,
        "old paragraph above split Markdown gap",
    );
    let below = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        below_ix,
        DiffTextRegion::SplitLeft,
        "Below old.".len() - 1.."Below old.".len(),
        "old paragraph below split Markdown gap",
    );

    drag_preview_selection(cx, left_gap.center(), above);
    let upward = copied_preview_selection(cx, &view)
        .expect("dragging upward from a split spacer should select text");
    assert!(upward.contains("Above block."), "upward={upward:?}");
    assert!(
        !upward.contains("Middle block") && !upward.contains("Below old."),
        "an upward split drag should stop at the spacer boundary: {upward:?}"
    );

    drag_preview_selection(cx, left_gap.center(), below);
    let downward = copied_preview_selection(cx, &view)
        .expect("dragging downward from a split spacer should select text");
    assert!(
        downward.contains("Middle block") && downward.contains("Below old"),
        "a downward split drag should start after the spacer: {downward:?}"
    );
    assert!(
        !downward.contains("Above block."),
        "a downward split drag must not reach behind the spacer: {downward:?}"
    );

    let (code_text_bounds, before_list_text_bounds, list_text_bounds) =
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (
                pane.diff_text_hitbox_bounds_for_tests(code_ix, DiffTextRegion::SplitLeft)
                    .expect("old fenced-code text hitbox"),
                pane.diff_text_hitbox_bounds_for_tests(before_list_ix, DiffTextRegion::SplitLeft)
                    .expect("old paragraph-before-list text hitbox"),
                pane.diff_text_hitbox_bounds_for_tests(list_ix, DiffTextRegion::SplitLeft)
                    .expect("old list text hitbox"),
            )
        });
    let code_top_padding = point(
        code_text_bounds.center().x,
        code_text_bounds.top() - px(2.0),
    );
    let code_bottom_padding = point(
        code_text_bounds.center().x,
        code_text_bounds.bottom() + px(2.0),
    );

    cx.simulate_click(code_top_padding, Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: code_ix,
            region: DiffTextRegion::SplitLeft,
            offset: 0,
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });

    drag_preview_selection(cx, code_top_padding, below);
    let from_code_top = copied_preview_selection(cx, &view)
        .expect("dragging down from fenced-code top padding should select text");
    assert!(
        from_code_top.contains("shared_call();")
            && from_code_top.contains("shared item")
            && from_code_top.contains("Below old"),
        "the code-top boundary should select every following block: {from_code_top:?}"
    );
    assert!(
        !from_code_top.contains("Paragraph before code."),
        "the code-top boundary must not reach into the preceding paragraph: {from_code_top:?}"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_local_selection_range(code_ix, DiffTextRegion::SplitLeft),
            Some(0.."shared_call();".len()),
            "the fenced-code text should receive a full selection highlight"
        );
    });

    cx.simulate_click(code_bottom_padding, Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: code_ix,
            region: DiffTextRegion::SplitLeft,
            offset: "shared_call();".len(),
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });
    drag_preview_selection(cx, code_bottom_padding, below);
    let from_code_bottom = copied_preview_selection(cx, &view)
        .expect("dragging down from fenced-code bottom padding should select text");
    assert!(
        from_code_bottom.contains("Paragraph before list.")
            && from_code_bottom.contains("shared item")
            && from_code_bottom.contains("Below old"),
        "the code-bottom boundary should select the following blocks: {from_code_bottom:?}"
    );
    assert!(
        !from_code_bottom.contains("shared_call();")
            && !from_code_bottom.contains("Paragraph before code."),
        "the code-bottom boundary must exclude the fenced code and preceding text: {from_code_bottom:?}"
    );

    let before_list_bottom_padding = point(
        before_list_text_bounds.center().x,
        before_list_text_bounds.bottom() + px(2.0),
    );
    cx.simulate_click(before_list_bottom_padding, Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: before_list_ix,
            region: DiffTextRegion::SplitLeft,
            offset: "Paragraph before list.".len(),
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });
    drag_preview_selection(cx, before_list_bottom_padding, below);
    let into_list = copied_preview_selection(cx, &view)
        .expect("dragging from paragraph padding into a list should select text");
    assert!(
        into_list.contains("shared item") && into_list.contains("Below old"),
        "the paragraph-list boundary should select the list and following paragraph: {into_list:?}"
    );
    assert!(
        !into_list.contains("Paragraph before list.") && !into_list.contains("shared_call();"),
        "the paragraph-list boundary must exclude preceding blocks: {into_list:?}"
    );

    let list_bottom_padding = point(
        list_text_bounds.center().x,
        list_text_bounds.bottom() + px(2.0),
    );
    cx.simulate_click(list_bottom_padding, Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: list_ix,
            region: DiffTextRegion::SplitLeft,
            offset: "shared item".len(),
        };
        assert_eq!(pane.diff_text_anchor, Some(boundary));
        assert_eq!(pane.diff_text_head, Some(boundary));
    });
    drag_preview_selection(cx, list_bottom_padding, below);
    let out_of_list = copied_preview_selection(cx, &view)
        .expect("dragging from list padding into a paragraph should select text");
    assert!(
        out_of_list.contains("Below old"),
        "the list-paragraph boundary should select the following paragraph: {out_of_list:?}"
    );
    assert!(
        !out_of_list.contains("shared item") && !out_of_list.contains("Paragraph before list."),
        "the list-paragraph boundary must exclude the list and preceding paragraph: {out_of_list:?}"
    );

    let selection_before_menu = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (pane.diff_text_anchor, pane.diff_text_head)
    });
    cx.simulate_mouse_down(left_gap.center(), MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(left_gap.center(), MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            (pane.diff_text_anchor, pane.diff_text_head)
        }),
        selection_before_menu,
        "opening a split spacer context menu should preserve the selection"
    );
    assert_eq!(
        cx.update(|_window, app| view.read(app).active_context_menu_invoker.clone()),
        Some("diff_editor_menu".into())
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup split Markdown gap workdir");
}

#[gpui::test]
fn a_drag_that_runs_past_a_short_line_still_selects_it(cx: &mut gpui::TestAppContext) {
    // A code block sizes every line to its own text so the block has something
    // to scroll, which leaves the space beside a short line belonging to no
    // row at all. Hit testing used to refuse any point outside a row, so a drag
    // that crossed one of those gaps stopped extending the selection and the
    // reader was left with whatever they had already covered.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let long = "one line that runs a good deal wider than the line beneath it";
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(91),
        "markdown_drag_past_short_line",
        &format!("Intro.\n\n```\n{long}\ntail\n```\n"),
    );

    let long_box = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix(long)
        )))
        .expect("expected the long code line's text box");
    let short_box = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("tail")
        )))
        .expect("expected the short code line's text box");
    assert!(
        short_box.right() < long_box.right() - px(8.0),
        "the fixture needs one code line to end well before the other; \
         long={long_box:?} short={short_box:?}"
    );

    // Ends level with the short line but past where its text stops, which is
    // the gap a code block leaves beside it.
    drag_preview_selection(
        cx,
        long_box.center(),
        point(long_box.right() - px(2.0), short_box.center().y),
    );

    let copied = copied_preview_selection(cx, &view).expect("the drag should have selected text");
    assert!(
        copied.ends_with("\ntail"),
        "a drag past the end of a short line still ends on that line, got {copied:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn copying_across_a_picture_writes_its_description_once(cx: &mut gpui::TestAppContext) {
    // An image block occupies as many rows as it is tall, and every one of them
    // carries the alt text so the row grid can describe a picture it cannot
    // draw. Copying walks rows, so a selection that crossed a picture used to
    // repeat its description once per row.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(92),
        "markdown_copy_over_picture",
        "Above.\n\n![demo](demo.png)\n\nBelow.\n",
    );
    assert!(
        fixture
            .document
            .rows
            .iter()
            .filter(|row| row.text.as_ref() == "demo")
            .count()
            > 1,
        "the fixture needs a picture spread over several rows"
    );

    let above = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Above.")
        )))
        .expect("expected the paragraph above the picture");
    let below = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Below.")
        )))
        .expect("expected the paragraph below the picture");

    drag_preview_selection(
        cx,
        point(above.left(), above.center().y),
        point(below.right(), below.center().y),
    );

    let copied = copied_preview_selection(cx, &view).expect("the drag should have selected text");
    assert_eq!(
        copied, "Above.\ndemo\nBelow.",
        "a picture is one line of the document however many rows it occupies"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_picture_draws_at_the_size_its_skeleton_reserved(cx: &mut gpui::TestAppContext) {
    // The other half of `a_skeleton_holds_the_box_the_picture_will_fill`: that
    // one pins the box the skeleton claims from the picture's header, this one
    // pins the box the picture actually lands in. They have to be the same
    // numbers, or reserving the room would just move the jump rather than
    // remove it. The decode itself is too fast to catch mid-flight in a test,
    // so the skeleton is measured through its own unit test instead.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(97),
        "markdown_picture_skeleton",
        "![demo](demo.png)\n\nAfter.\n",
    );
    // Narrower than the pane, so the picture keeps its own size rather than
    // being clamped and the reserved box has to match it exactly.
    std::fs::write(
        fixture.workdir.join("docs/demo.png"),
        test_png_bytes(40, 20).as_slice(),
    )
    .expect("write the picture the document points at");
    let row_ix = fixture.row_ix("demo");
    for _ in 0..3 {
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    }

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .expect("the picture is drawn once it has decoded");
    assert!(
        (picture.size.width - px(40.0)).abs() <= px(0.5)
            && (picture.size.height - px(20.0)).abs() <= px(0.5),
        "a picture narrower than the pane draws at its own size, which is the \
         box its skeleton reserved; got {picture:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn ask_mode_blocks_remote_markdown_images_and_offers_approval_controls(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
                cx,
            );
        });
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(107),
        "markdown_remote_image_approval",
        concat!(
            "<img alt=\"remote\" src=\"https://example.invalid/tracking.png\" ",
            "width=\"120\" height=\"60\" />\n\n",
            "<img alt=\"other\" src=\"https://example.invalid/other.png\" ",
            "width=\"80\" height=\"40\" />\n",
        ),
    );
    let row_ix = fixture.row_ix("remote");
    let retry_selector = leaked_selector(format!("markdown_preview_block_image_load_{row_ix}"));
    let retry_icon_selector = leaked_selector(format!(
        "markdown_preview_block_image_load_{row_ix}_retry_icon"
    ));
    let blocked_icon_selector = leaked_selector(format!(
        "markdown_preview_block_image_load_{row_ix}_blocked_icon"
    ));

    let retry = cx
        .debug_bounds(retry_selector)
        .expect("approval mode should draw a per-image Retry control");
    assert!(
        (retry.size.width - px(120.0)).abs() <= px(0.5)
            && (retry.size.height - px(60.0)).abs() <= px(0.5),
        "the blocked box should preserve the declared image size; got {retry:?}"
    );
    assert!(cx.debug_bounds(retry_icon_selector).is_some());
    assert!(cx.debug_bounds(blocked_icon_selector).is_none());
    assert!(
        cx.debug_bounds("markdown_preview_load_all_remote_images")
            .is_some(),
        "the preview header should offer Load all images"
    );
    assert!(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .is_none(),
        "the remote image element must not be created before approval"
    );

    cx.simulate_click(retry.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            assert_eq!(pane.approved_remote_markdown_image_urls.len(), 1);
            assert!(
                pane.approved_remote_markdown_image_urls
                    .contains("https://example.invalid/tracking.png"),
                "clicking Retry should approve only that image's exact URL"
            );
            assert!(
                !pane
                    .approved_remote_markdown_image_urls
                    .contains("https://example.invalid/other.png")
            );
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::NeverLoad,
                cx,
            );
            assert!(
                pane.approved_remote_markdown_image_urls.is_empty(),
                "changing policy should clear preview-scoped approvals"
            );
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let blocked = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_load_{row_ix}_blocked_box"
        )))
        .expect("never-load mode should draw a blocked-image box");
    assert!(
        (blocked.size.width - px(120.0)).abs() <= px(0.5)
            && (blocked.size.height - px(60.0)).abs() <= px(0.5),
        "the never-load box should preserve the declared image size; got {blocked:?}"
    );
    assert!(cx.debug_bounds(blocked_icon_selector).is_some());
    assert!(cx.debug_bounds(retry_icon_selector).is_none());

    fixture.cleanup();
}

#[gpui::test]
fn markdown_image_access_snapshots_share_approved_url_storage(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let first = pane.markdown_remote_image_access(None);
        let second = pane.markdown_remote_image_access(None);
        assert!(
            Arc::ptr_eq(&first.approved_urls, &second.approved_urls),
            "render snapshots should share approval storage instead of cloning every URL"
        );
    });
}

#[gpui::test]
fn blocked_remote_image_summary_does_not_rescan_unchanged_document(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.set_remote_markdown_image_policy(
                crate::view::RemoteMarkdownImagePolicy::AskBeforeLoading,
                cx,
            );
        });
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(109),
        "markdown_remote_image_summary_cache",
        "![remote](https://example.invalid/tracking.png)\n",
    );
    crate::view::panes::main::reset_remote_markdown_image_row_visits_for_tests();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.has_blocked_remote_markdown_images());
        let after_first = crate::view::panes::main::remote_markdown_image_row_visits_for_tests();
        assert!(pane.has_blocked_remote_markdown_images());
        let after_second = crate::view::panes::main::remote_markdown_image_row_visits_for_tests();
        assert_eq!(
            after_second, after_first,
            "an unchanged render must reuse the blocked-image summary"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn markdown_below_eof_drag_selects_an_image_only_document(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(103),
        "markdown_below_eof_image_only",
        "![demo](demo.png)\n",
    );
    std::fs::write(
        fixture.workdir.join("docs/demo.png"),
        test_png_bytes(40, 20).as_slice(),
    )
    .expect("write image-only preview picture");
    let row_ix = fixture.row_ix("demo");
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .expect("image-only document picture bounds");
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_Inline")
        .expect("image-only document below-EOF surface");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_hitboxes.is_empty(),
            "a block image must not need a synthetic painted-text hitbox"
        );
        assert!(
            !pane.diff_text_motion_targets.is_empty(),
            "the image still needs a logical selection-motion target"
        );
    });

    drag_preview_selection(cx, empty_space.center(), picture.center());
    assert_eq!(
        copied_preview_selection(cx, &view).as_deref(),
        Some("demo"),
        "dragging upward from EOF should copy an image-only document's alt text"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_below_eof_drag_selects_a_thematic_break_only_document(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(104),
        "markdown_below_eof_rule_only",
        "---\n",
    );
    let row_ix = fixture
        .document
        .rows
        .iter()
        .position(|row| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::ThematicBreak
            )
        })
        .expect("thematic-break source row");
    let rule_text = fixture.document.rows[row_ix].text.clone();
    let rule = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_thematic_break_{row_ix}"
        )))
        .expect("thematic-break-only document bounds");
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_Inline")
        .expect("thematic-break-only document below-EOF surface");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_text_hitboxes.is_empty(),
            "a thematic break must not need a synthetic painted-text hitbox"
        );
        assert!(
            !pane.diff_text_motion_targets.is_empty(),
            "the thematic break still needs a logical selection-motion target"
        );
    });

    drag_preview_selection(cx, empty_space.center(), rule.center());
    assert_eq!(
        copied_preview_selection(cx, &view).as_deref(),
        Some(rule_text.as_ref()),
        "dragging upward from EOF should copy a thematic-break-only document"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_below_eof_surface_starts_after_a_trailing_picture(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(98),
        "markdown_below_eof_trailing_picture",
        "Before.\n\n![demo](demo.png)\n",
    );
    std::fs::write(
        fixture.workdir.join("docs/demo.png"),
        test_png_bytes(40, 20).as_slice(),
    )
    .expect("write trailing preview picture");
    let row_ix = fixture.row_ix("demo");
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let picture = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_image_{row_ix}"
        )))
        .expect("trailing picture bounds");
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_Inline")
        .expect("flowing preview below-EOF surface");
    let before = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Before.")
        )))
        .expect("paragraph before the trailing picture");
    assert!(
        empty_space.top() >= picture.bottom(),
        "the EOF surface must begin after the complete picture block; picture={picture:?} surface={empty_space:?}"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .max_offset()
                .y,
            px(0.0),
            "a short flowing document should not gain vertical scroll range"
        );
    });

    cx.simulate_mouse_down(
        empty_space.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        empty_space.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let last_row_ix = fixture
            .document
            .rows
            .len()
            .checked_sub(1)
            .expect("the preview document should contain picture rows");
        let last_row_len = fixture.document.rows[last_row_ix].text.len();
        assert_eq!(pane.diff_text_anchor, pane.diff_text_head);
        assert_eq!(
            pane.diff_text_head,
            Some(DiffTextPos {
                source_visible_ix: last_row_ix,
                region: DiffTextRegion::Inline,
                offset: last_row_len,
            }),
            "below-EOF selection must end after every row of the trailing picture"
        );
    });

    drag_preview_selection(
        cx,
        empty_space.center(),
        point(before.left(), before.center().y),
    );
    let copied = copied_preview_selection(cx, &view)
        .expect("dragging upward from below EOF should select the document");
    assert!(
        copied.contains("demo"),
        "dragging upward from below EOF must include a trailing picture's alt text: {copied:?}"
    );
    assert_eq!(
        copied.matches("demo").count(),
        1,
        "a multi-row trailing picture should contribute its alt text once: {copied:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_below_eof_resolves_after_a_trailing_thematic_break(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(99),
        "markdown_below_eof_trailing_rule",
        "Before.\n\n---\n",
    );
    let before = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Before.")
        )))
        .expect("paragraph before the trailing thematic break");
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_Inline")
        .expect("flowing preview below-EOF surface");

    cx.simulate_click(empty_space.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let last_row_ix = fixture
            .document
            .rows
            .len()
            .checked_sub(1)
            .expect("the preview document should contain a thematic-break row");
        assert_eq!(
            pane.diff_text_head,
            Some(DiffTextPos {
                source_visible_ix: last_row_ix,
                region: DiffTextRegion::Inline,
                offset: fixture.document.rows[last_row_ix].text.len(),
            }),
            "below-EOF selection must end after the trailing thematic-break row"
        );
    });

    drag_preview_selection(
        cx,
        empty_space.center(),
        point(before.left(), before.center().y),
    );
    let copied = copied_preview_selection(cx, &view)
        .expect("dragging upward from below EOF should select the document");
    assert!(
        copied.contains("───"),
        "dragging upward from below EOF must include the trailing thematic break: {copied:?}"
    );

    fixture.cleanup();
}

/// A minimal PNG of the given size — only its header is ever read.
fn test_png_bytes(width: u32, height: u32) -> Vec<u8> {
    use image::ImageEncoder as _;
    let mut out = std::io::Cursor::new(Vec::new());
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            &vec![0u8; (width * height * 4) as usize],
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .expect("encode a test png");
    out.into_inner()
}

#[gpui::test]
fn a_picture_that_is_still_decoding_is_waited_on(cx: &mut gpui::TestAppContext) {
    // `gpui` wakes only the first view that asked for an image, so a pane that
    // starts showing one another pane is already decoding is never told the
    // decode finished and holds an empty slot. The pane waits on its own
    // pictures instead of relying on that.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(93),
        "markdown_image_wait",
        "![demo](demo.svg)\n\nAfter.\n",
    );
    // Written after the preview opened, so the first draw resolved nothing and
    // the next one is the one that finds a picture to load.
    std::fs::write(
        fixture.workdir.join("docs/demo.svg"),
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"20\"><rect width=\"40\" height=\"20\"/></svg>",
    )
    .expect("write the picture the document points at");
    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.worktree_markdown_preview_image_waits.is_empty(),
            "a picture that has not decoded yet needs something waiting to repaint the pane"
        );
    });

    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.worktree_markdown_preview_image_waits.is_empty(),
            "and the wait is released once the picture has been decided one way or the other"
        );
    });

    fixture.cleanup();
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

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(80),
        "markdown_wrapped_hit_test",
        &format!(
            "{}\n",
            "one paragraph wrapped over several lines ".repeat(40)
        ),
    );

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

    fixture.cleanup();
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
    // A top-level heading makes the preview insert a spacer row, and headings
    // carry vertical insets — both used to punch holes in the change bar.
    let fixture = RenderedPreviewFixture::open_with_status(
        cx,
        &view,
        gitcomet_state::model::RepoId(75),
        "markdown_change_bar",
        "# Title\n\nBody paragraph.\n\n## Section\n\nMore body.\n",
        gitcomet_core::domain::FileStatusKind::Untracked,
    );
    let last_row_ix = fixture.row_ix("More body.");

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

    fixture.cleanup();
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
fn split_markdown_eof_ignores_trailing_alignment_and_wrap_padding(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(105);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_split_eof_padding",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/split-eof.md");
    let old_text = "Shared paragraph.\n\nold tail\n";
    let new_text = format!(
        "{old_text}\n{}\n",
        "new-only words that wrap on the other side ".repeat(18)
    );
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let preview = crate::view::markdown_preview::build_markdown_diff_preview(old_text, &new_text)
        .expect("split EOF padding fixture should parse");
    let old_tail_row_ix = preview
        .old
        .rows
        .iter()
        .position(|row| row.text.as_ref() == "old tail")
        .expect("old tail row");
    assert!(
        preview.old.rows[old_tail_row_ix + 1..].iter().all(|row| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::Spacer
            )
        }),
        "the old side should end in alignment spacers supplied for the new-only paragraph"
    );

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create split EOF padding workdir");
    seed_file_diff_state(cx, &view, repo_id, &workdir, &file_rel, old_text, &new_text);
    wait_for_main_pane_condition(
        cx,
        &view,
        "split Markdown EOF padding target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
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
                pane.file_markdown_preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(preview));
                pane.file_markdown_preview_inflight = None;
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    let old_tail_visual_ix = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let plan = pane
            .markdown_preview_wrap
            .plan(MarkdownPreviewList::Old)
            .expect("old split wrap plan");
        let visible_ix = plan.visual_ix_for_row(old_tail_row_ix);
        assert!(
            plan.rows[visible_ix + 1..]
                .iter()
                .all(|visual| visual.byte_range.is_empty()),
            "everything after the old tail should be synthetic empty padding"
        );
        visible_ix
    });
    let empty_space = cx
        .debug_bounds("diff_text_empty_space_SplitLeft")
        .expect("old split column below-EOF surface");

    cx.simulate_click(empty_space.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_head,
            Some(DiffTextPos {
                source_visible_ix: old_tail_visual_ix,
                region: DiffTextRegion::SplitLeft,
                offset: "old tail".len(),
            }),
            "old-side EOF must stop before aligned and wrapped padding"
        );
    });

    let old_tail_start = wait_for_diff_text_click_position_for_offset_range(
        cx,
        &view,
        old_tail_visual_ix,
        DiffTextRegion::SplitLeft,
        0..1,
        "start of the old Markdown tail",
    );
    drag_preview_selection(cx, empty_space.center(), old_tail_start);
    assert_eq!(
        copied_preview_selection(cx, &view).as_deref(),
        Some("old tail"),
        "synthetic split padding must not become copied blank lines"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup split EOF padding workdir");
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

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(78),
        "markdown_link_menu",
        "[the docs](https://example.com/docs)\n",
    );

    let text_bounds = cx
        .debug_bounds("markdown_preview_text_box_0")
        .expect("expected preview text bounds");
    // Left edge of the row's text is inside the link, which spans the row.
    let on_link = point(text_bounds.left() + px(4.0), text_bounds.center().y);

    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let link = this.main_pane.read(cx).markdown_preview_link_span_at(
                0,
                DiffTextRegion::Inline,
                on_link,
            );
            assert_eq!(
                link.as_ref().map(|(url, _)| url.as_ref()),
                Some("https://example.com/docs"),
                "the click position must resolve to the link destination"
            );

            let popover = this.popover_host.read(cx).popover_kind_for_tests();
            assert!(
                matches!(
                    popover,
                    Some(PopoverKind::WebLinkMenu { ref url, .. })
                        if url.as_ref() == "https://example.com/docs"
                ),
                "clicking a link should open its menu, got {popover:?}"
            );

            // The same menu is reachable from a commit message, where handing
            // focus back to the diff panel on close would be wrong. Closing
            // reads this flag, so a preview link has to set it.
            assert!(
                this.popover_host
                    .read(cx)
                    .popover_opened_from_diff_panel_for_tests(),
                "a preview link is a diff-panel invoker, so its focus returns there"
            );

            // The menu hangs off the link's own box rather than the row that
            // holds it, so it opens flush under the words it describes.
            let anchor = this
                .popover_host
                .read(cx)
                .popover_anchor_bounds_for_tests()
                .expect("a preview link menu anchors on the link's box");
            assert!(
                anchor.contains(&on_link),
                "the anchor must be the box the click landed in, got {anchor:?}"
            );
            assert!(
                anchor.top() >= text_bounds.top()
                    && anchor.bottom() <= text_bounds.bottom() + px(1.0),
                "the anchor must be a line of the row, not the row's own edges; \
                 anchor={anchor:?} row={text_bounds:?}"
            );
        });
    });

    fixture.cleanup();
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

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(79),
        "markdown_text_box",
        "A plain paragraph with enough words to fill the row.\n",
    );

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

    fixture.cleanup();
}

/// Ctrl+F in the rendered file preview has to bring the hit into view.
///
/// The flowing document is not a `uniform_list`, so there is no
/// `scroll_to_item` to hand this to: the renderer measures the target row
/// during prepaint and sets the offset itself.
#[gpui::test]
fn markdown_file_preview_search_scrolls_the_rendered_document_to_the_match(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(471);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_search_scroll",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("long_notes.md");
    let abs_path = workdir.join(&file_rel);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    // One unique paragraph far below the fold, so a match there can only be on
    // screen if the preview actually scrolled.
    let mut lines: Vec<String> = (0..300).map(|ix| format!("paragraph {ix:03}")).collect();
    lines.push(String::new());
    lines.push("the needle paragraph".to_string());
    let source = lines.join("\n");
    std::fs::write(&abs_path, &source).expect("write markdown fixture");

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

    let source_len = source.len();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::new(lines.clone()),
                    source_len,
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.markdown_search_surface(),
            Some(MarkdownSearchSurface::Worktree),
            "expected the rendered file preview to be the search surface"
        );
        assert_eq!(
            pane.worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y,
            px(0.0),
            "expected the preview to start at the top"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "needle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("needle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            1,
            "expected exactly one rendered row to match, got {:?}",
            pane.diff_search_matches
        );
        assert!(
            pane.worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .offset()
                .y
                < px(0.0),
            "expected the rendered preview to scroll down to the match, offset stayed at {:?}",
            pane.worktree_preview_scroll.0.borrow().base_handle.offset(),
        );
        assert_eq!(
            pane.markdown_preview_reveal.pending(),
            None,
            "the reveal should be claimed once so it stops fighting later scrolling"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview scroll fixture");
}

/// The rendered markdown *diff* is the other in-place search surface. It is a
/// `uniform_list`, so the reveal is the ordinary scroll-to-row — but the match
/// list has to be built from the rendered rows and mapped through the wrap plan.
#[gpui::test]
fn markdown_diff_preview_search_scrolls_the_list_to_the_match(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(472);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_search_scroll",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("docs/long.md");

    let body: String = (0..300)
        .map(|ix| format!("entry {ix:03}\n\n"))
        .collect::<Vec<_>>()
        .join("");
    let old_text = format!("# Long\n\n{body}");
    let new_text = format!("{old_text}\nthe needle entry\n");
    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create markdown diff search workdir");
    seed_file_diff_state(
        cx, &view, repo_id, &workdir, &file_rel, &old_text, &new_text,
    );

    wait_for_main_pane_condition(
        cx,
        &view,
        "markdown diff search target activation",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(target.clone())
        },
        |pane| {
            format!(
                "diff_target={:?}",
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone())
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
                    crate::view::markdown_preview::build_markdown_diff_preview(
                        &old_text, &new_text,
                    )
                    .expect("markdown diff preview should parse"),
                ));
                pane.file_markdown_preview_inflight = None;
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.markdown_search_surface(),
            Some(MarkdownSearchSurface::DiffInline),
            "expected the inline rendered markdown diff to be the search surface"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                reset_uniform_list_offsets(&[&pane.diff_scroll]);
                pane.diff_search_active = true;
                pane.diff_search_query = "needle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("needle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.diff_search_matches.is_empty(),
            "expected the rendered markdown diff to report a match"
        );
        assert!(
            uniform_list_offset(&pane.diff_scroll).y < px(0.0),
            "expected the markdown diff list to scroll to the match, offset stayed at {:?}",
            uniform_list_offset(&pane.diff_scroll),
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown diff search fixture");
}

/// Rendered rows and source lines are different row spaces, so toggling the
/// preview under an open search has to rescan — otherwise the match list keeps
/// indices that address the view the user just left.
#[gpui::test]
fn toggling_the_preview_under_an_open_search_rescans_the_new_row_space(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(473);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_toggle_rescan",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("toggle.md");
    let abs_path = workdir.join(&file_rel);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    // `##` survives only in the source: the renderer consumes it into a heading.
    let lines = vec![
        "## Heading".to_string(),
        String::new(),
        "body text".to_string(),
    ];
    let source = lines.join("\n");
    std::fs::write(&abs_path, &source).expect("write markdown fixture");

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

    let source_len = source.len();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::new(lines.clone()),
                    source_len,
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = "##".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("##", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.is_empty(),
            "the rendered preview shows no `##`, so nothing should match; got {:?}",
            pane.diff_search_matches
        );
    });

    // Switching to Source puts the markdown itself on screen, and the same
    // query must now find it.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Source);
                pane.diff_search_recompute_matches();
                cx.notify();
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.markdown_search_surface(),
            None,
            "source mode is not a markdown search surface"
        );
        assert!(
            !pane.diff_search_matches.is_empty(),
            "expected the source view to find the `##` the rendered view hid"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown toggle fixture");
}

/// While the rendered preview is still parsing, the pane paints a notice rather
/// than the document. Nothing is on screen to find, and the markdown source
/// underneath is not what the reader is looking at, so search reports nothing
/// instead of quietly scanning a view that is not there.
#[gpui::test]
fn a_markdown_preview_without_a_document_reports_no_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(474);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_no_document",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("pending.md");
    let abs_path = workdir.join(&file_rel);
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let lines = vec!["needle line".to_string()];
    let source = lines.join("\n");
    std::fs::write(&abs_path, &source).expect("write markdown fixture");

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

    let source_len = source.len();
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    abs_path.clone(),
                    Arc::new(lines.clone()),
                    source_len,
                    cx,
                );
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
            });
        });
    });
    draw_and_drain_test_window(cx);

    // Stand in for the window before the parse lands, or after it failed.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Loading;
                pane.diff_search_active = true;
                pane.diff_search_query = "needle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("needle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.rendered_markdown_preview_owns_view(),
            "the preview toggle is still on Rendered"
        );
        assert_eq!(
            pane.markdown_search_surface(),
            None,
            "a preview with no document is not a searchable surface"
        );
        assert!(
            pane.diff_search_matches.is_empty(),
            "expected no matches while the document is not on screen, got {}",
            pane.diff_search_matches.len()
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup pending markdown fixture");
}
