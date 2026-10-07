//! Rendering and caches: preview modes, fallbacks, layout, scroll sync, frame budgets.

use super::*;

#[gpui::test]
fn auth_token_readme_tables_render_multiline_code_inside_cells(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend_with_system_fonts(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1400.0), px(1200.0)));
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(168),
        "auth_token_readme_tables",
        include_str!("../../../markdown_preview/fixtures/auth_token_tables.md"),
    );
    let rows = table_row_ixs(&fixture);
    assert_eq!(rows.len(), 6);
    for width in [1400.0, 950.0] {
        cx.simulate_resize(gpui::size(px(width), px(1200.0)));
        draw_frames(cx, 2);
        let cell = cx
            .debug_bounds(leaked_selector(format!(
                "markdown_preview_cell_box_{}_1",
                rows[0]
            )))
            .unwrap();
        let text = cx
            .debug_bounds(leaked_selector(format!(
                "markdown_preview_cell_text_box_{}_1",
                rows[0]
            )))
            .unwrap();
        assert!(
            text.size.height > px(50.0),
            "code retains multiple lines: {text:?}"
        );
        assert!(text.size.width > px(0.0));
        assert!(text.right() <= cell.right() + px(0.5));
        assert!(text.bottom() <= cell.bottom() + px(0.5));
        assert!(
            cx.debug_bounds(leaked_selector(format!(
                "markdown_preview_cell_box_{}_2",
                rows[2]
            )))
            .is_some(),
            "the authentication table also renders"
        );
    }
    fixture.cleanup();
}

#[gpui::test]
fn markdown_empty_cells_and_code_lines_keep_their_selection_layout(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend_with_system_fonts(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(480),
        "markdown_empty_text_layout",
        "| Left | Middle | Right |\n| :--- | :---: | ---: |\n| | value | |\n| | | |\n\n```rust\nbefore\n\nafter\n```\n",
    );
    let rows = table_row_ixs(&fixture);
    assert_eq!(rows.len(), 3);
    let empty_code_row = fixture
        .document
        .rows
        .iter()
        .position(|row| {
            matches!(
                row.kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::CodeLine { .. }
            ) && row.text.is_empty()
        })
        .expect("fixture has an empty code line");

    // Redrawing exercises the cached backend layout as well as its first build.
    draw_frames(cx, 2);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for row_ix in &rows[1..] {
            let row = pane
                .diff_text_hitboxes
                .get(&(*row_ix, DiffTextRegion::Inline))
                .expect("table row retains selection targets");
            assert_eq!(row.cells.len(), 3);
            for cell in &row.cells {
                assert!(cell.bounds.size.height > px(0.0));
                let layout = &cell.wrapped.as_ref().expect("cell text layout").layout;
                assert!(layout.line_layout_for_index(0).is_some());
                if cell.text_len == 0 {
                    assert!(cell.painted_text.is_empty());
                }
            }
        }
        let code = pane
            .diff_text_hitboxes
            .get(&(empty_code_row, DiffTextRegion::Inline))
            .expect("empty code line retains a selection target");
        assert_eq!(code.text_len, 0);
        assert!(code.bounds.size.height > px(0.0));
        assert!(
            code.wrapped
                .as_ref()
                .expect("code text layout")
                .layout
                .line_layout_for_index(0)
                .is_some()
        );
    });
    fixture.cleanup();
}

#[gpui::test]
fn markdown_empty_shared_highlights_text_keeps_its_line_height(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend_with_system_fonts(cx);
    struct EmptyMarkdownText;
    impl gpui::Render for EmptyMarkdownText {
        fn render(
            &mut self,
            _window: &mut Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl IntoElement {
            div().flex().child(
                div()
                    .debug_selector(|| "empty_markdown_shared_text".into())
                    .child(crate::view::rows::markdown_preview_highlighted_text(
                        SharedString::default(),
                        Arc::from(Vec::new()),
                    )),
            )
        }
    }
    let (_view, cx) = cx.add_window_view(|_window, _cx| EmptyMarkdownText);
    draw_frames(cx, 2);
    let bounds = cx
        .debug_bounds("empty_markdown_shared_text")
        .expect("empty text is laid out");
    assert!(bounds.size.height > px(0.0));
    assert_eq!(bounds.size.width, px(0.0));
}

#[gpui::test]
fn markdown_diff_preview_cache_does_not_rebuild_when_rev_changes_with_identical_payload(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            pane.diff_markdown.inflight.is_none()
                && matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Ready(_)
                )
        },
        |pane| {
            (
                pane.diff_markdown.seq,
                pane.diff_markdown.inflight,
                pane.diff_markdown.cache_repo_id,
                pane.diff_markdown.cache_rev,
                pane.diff_markdown.cache_target.clone(),
                pane.diff_markdown.cache_content_signature,
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
            )
        },
    );

    let baseline_seq =
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_markdown.seq);

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
                pane.diff_markdown.seq, baseline_seq,
                "identical markdown diff payload should not trigger preview rebuild when diff_file_rev changes"
            );
            assert!(
                pane.diff_markdown.inflight.is_none(),
                "markdown preview cache should remain ready with no background rebuild for identical payload refreshes"
            );
            assert_eq!(
                pane.diff_markdown.cache_rev, rev,
                "identical payload refresh should still advance the markdown cache rev marker"
            );
            assert!(
                matches!(
                    pane.diff_markdown.preview,
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(old_text, new_text)
                        .expect("worktree markdown diff preview should parse"),
                ));
                pane.diff_markdown.inflight = None;
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_markdown_empty_side_width",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/added.md");
    let old_text = "";
    let new_text = "# Added\n\nThis side must stay visible.\n";
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::MarkdownPreviewDiff::new(old, new.clone(), new),
                ));
                pane.diff_markdown.inflight = None;
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
fn worktree_markdown_preview_short_code_block_shell_spans_preview_width(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn a_document_as_long_as_the_parser_allows_renders_as_a_preview(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A frame builds the blocks near the viewport whatever the document's
    // length, so the preview's only limit is the parser's own.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(87);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_render_budget",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("huge.md");
    let abs_path = workdir.join(&file_rel);
    let source = "---\n".repeat(crate::view::markdown_preview::MAX_PREVIEW_ROWS);
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
            RenderedPreviewMode::Rendered,
            "a long document stays a preview"
        );
        let gitcomet_state::model::Loadable::Ready(document) = &pane.worktree_markdown.document
        else {
            panic!(
                "expected the preview, got {:?}",
                pane.worktree_markdown.document
            );
        };
        assert_eq!(
            document.rows.len(),
            crate::view::markdown_preview::MAX_PREVIEW_ROWS
        );
    });
    assert!(
        cx.debug_bounds("markdown_preview_thematic_break_0")
            .is_some(),
        "its first rows are drawn"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup render budget workdir");
}

#[gpui::test]
fn markdown_file_preview_over_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            pane.worktree_markdown.inflight.is_none(),
            "oversized preview should fail synchronously without background parsing"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown.document
        else {
            panic!(
                "expected oversize markdown file preview to show fallback error, got {:?}",
                pane.worktree_markdown.document
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            pane.worktree_markdown.inflight.is_none(),
            "over-limit preview should fail synchronously when exact source length exceeds the markdown cap"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.worktree_markdown.document
        else {
            panic!(
                "expected exact-source-len markdown file preview to show fallback error, got {:?}",
                pane.worktree_markdown.document
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(55);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_preview_cache_reset",
        std::process::id()
    ));
    let preview_path = std::path::PathBuf::from("docs/preview.md");
    let preview_target = gitcomet_core::domain::DiffTarget::working_tree(
        preview_path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
                pane.worktree_markdown.path = Some(workdir.join(&preview_path));
                pane.worktree_markdown.source_rev = 9;
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Loading;
                pane.worktree_markdown.inflight = Some(3);
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
                && pane.worktree_preview_content_rev > 9
                && pane.worktree_preview_text.is_empty()
                && pane.worktree_preview_line_starts.is_empty()
                && pane.worktree_markdown.path.is_none()
                && pane.worktree_markdown.source_rev == 0
                && matches!(
                    pane.worktree_markdown.document,
                    gitcomet_state::model::Loadable::NotLoaded
                )
                && pane.worktree_markdown.inflight.is_none()
        },
        |pane| {
            format!(
                "worktree_path={:?} worktree_rev={} worktree_text_len={} worktree_line_starts={} worktree_markdown_path={:?} worktree_markdown_rev={} worktree_markdown_inflight={:?} worktree_markdown_not_loaded={}",
                pane.worktree_preview_path,
                pane.worktree_preview_content_rev,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_starts.len(),
                pane.worktree_markdown.path,
                pane.worktree_markdown.source_rev,
                pane.worktree_markdown.inflight,
                matches!(
                    pane.worktree_markdown.document,
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            pane.diff_markdown.inflight.is_none(),
            "oversized diff preview should fail synchronously without background parsing"
        );
        let gitcomet_state::model::Loadable::Error(message) = &pane.diff_markdown.preview else {
            panic!(
                "expected oversize markdown diff preview to show fallback error, got {:?}",
                pane.diff_markdown.preview
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            pane.diff_markdown.inflight.is_none()
                && matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Error(_)
                )
        },
        |pane| {
            (
                pane.diff_markdown.seq,
                pane.diff_markdown.inflight,
                pane.diff_markdown.cache_repo_id,
                pane.diff_markdown.cache_rev,
                pane.diff_markdown.cache_target.clone(),
                pane.diff_markdown.cache_content_signature,
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Loading
                ),
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
                matches!(
                    pane.diff_markdown.preview,
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
        let gitcomet_state::model::Loadable::Error(message) = &pane.diff_markdown.preview else {
            panic!(
                "expected row-limit markdown diff preview to show fallback error, got {:?}",
                pane.diff_markdown.preview
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

/// A diff each side of which the parser accepts, but whose inline form — the
/// unchanged rows once, each changed row twice — holds more rows than one
/// document may, so it is shown as source.
fn inline_overflowing_markdown_diff() -> (String, String) {
    let limit = crate::view::markdown_preview::MAX_PREVIEW_ROWS;
    let shared: String = (0..limit / 2).map(|ix| format!("line {ix}\n\n")).collect();
    let changed = |label: &str| -> String {
        (0..=limit / 4)
            .map(|ix| format!("{label} {ix}\n\n"))
            .collect()
    };
    (
        format!("{shared}{}", changed("old")),
        format!("{shared}{}", changed("new")),
    )
}

#[gpui::test]
fn a_markdown_diff_too_big_to_lay_out_falls_back_to_the_text_diff(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(97);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_diff_flow_limit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("docs/row-limit.md");
    let (old_text, new_text) = inline_overflowing_markdown_diff();

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
            pane.diff_markdown.inflight.is_none()
                && matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Error(_)
                )
        },
        |pane| {
            (
                pane.diff_markdown.seq,
                pane.diff_markdown.inflight,
                pane.diff_markdown.cache_repo_id,
                pane.diff_markdown.cache_rev,
                pane.diff_markdown.cache_target.clone(),
                pane.diff_markdown.cache_content_signature,
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Loading
                ),
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Ready(_)
                ),
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Error(_)
                ),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        // Each side parsed, only the inline form outgrew the cap: the diff
        // reads fine as text, so the pane goes there rather than to an error.
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Source
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn markdown_diff_preview_keeps_layout_controls_and_ignores_text_hotkeys(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    use gitcomet_core::conflict_session::{ConflictPayload, ConflictSession};

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            // The rendered preview parses once all three sides are loaded.
            repo.conflict_state.conflict_file_load_mode =
                gitcomet_state::model::ConflictFileLoadMode::Full;

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
        "conflict markdown preview columns drawn",
        BACKGROUND_SYNTAX_MAIN_PANE_WAIT_TIMEOUT,
        |pane| {
            let docs = &pane.conflict_resolver.markdown_preview.documents;
            pane.is_conflict_rendered_preview_active()
                && [&docs.base, &docs.ours, &docs.theirs]
                    .iter()
                    .all(|doc| matches!(doc, gitcomet_state::model::Loadable::Ready(_)))
                && uniform_list_max_offset(&pane.conflict_resolver_diff_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_ours_scroll).height > px(120.0)
                && uniform_list_max_offset(&pane.conflict_preview_theirs_scroll).height > px(120.0)
                && scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll).width
                    > px(80.0)
        },
        |pane| {
            let docs = &pane.conflict_resolver.markdown_preview.documents;
            format!(
                "base_bounds={:?} ready={:?} preview_active={} base_max={:?} ours_max={:?} theirs_max={:?} output_max={:?}",
                pane.conflict_resolver_diff_scroll
                    .0
                    .borrow()
                    .base_handle
                    .bounds(),
                [&docs.base, &docs.ours, &docs.theirs].map(|doc| match doc {
                    gitcomet_state::model::Loadable::Ready(doc) =>
                        format!("rows={}", doc.rows.len()),
                    gitcomet_state::model::Loadable::Error(e) => format!("error {e}"),
                    _ => "pending".to_string(),
                }),
                pane.is_conflict_rendered_preview_active(),
                uniform_list_max_offset(&pane.conflict_resolver_diff_scroll),
                uniform_list_max_offset(&pane.conflict_preview_ours_scroll),
                uniform_list_max_offset(&pane.conflict_preview_theirs_scroll),
                scroll_handle_max_offset(&pane.conflict_resolved_output_editor_scroll),
            )
        },
    );
    // The long code lines scroll inside their own blocks, so no column has a
    // sideways range for the output to drive.
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        for handle in [
            &pane.conflict_resolver_diff_scroll,
            &pane.conflict_preview_ours_scroll,
            &pane.conflict_preview_theirs_scroll,
        ] {
            assert_eq!(uniform_list_max_offset(handle).width, px(0.0));
        }
    });

    let reset_offsets =
        |cx: &mut gpui::VisualTestContext,
         view: &gpui::Entity<super::super::super::GitCometView>| {
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

        // Vertically the columns move together when the mode says so; the
        // resolved output is a different document and stands on its own.
        reset_offsets(cx, &view);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    set_uniform_list_offset(
                        &pane.conflict_resolver_diff_scroll,
                        point(px(0.0), px(-80.0)),
                    );
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let expected = if ScrollSyncAxis::Vertical.includes(mode) {
                px(-80.0)
            } else {
                px(0.0)
            };
            assert_eq!(
                uniform_list_offset(&pane.conflict_resolver_diff_scroll).y,
                px(-80.0)
            );
            for (label, handle) in [
                ("ours", &pane.conflict_preview_ours_scroll),
                ("theirs", &pane.conflict_preview_theirs_scroll),
            ] {
                assert_eq!(
                    uniform_list_offset(handle).y,
                    expected,
                    "the {label} column follows the base vertically in {mode:?} mode"
                );
            }
            assert_eq!(
                scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).y,
                px(0.0),
                "the output does not follow the columns vertically in {mode:?} mode"
            );
        });

        // Sideways the output scrolls on its own, and the columns, with
        // nothing to scroll, must not pull it back.
        reset_offsets(cx, &view);
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    set_scroll_handle_offset(
                        &pane.conflict_resolved_output_editor_scroll,
                        point(px(-72.0), px(0.0)),
                    );
                    cx.notify();
                });
            });
        });
        draw_and_drain_test_window(cx);
        draw_and_drain_test_window(cx);
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert_eq!(
                scroll_handle_offset(&pane.conflict_resolved_output_editor_scroll).x,
                px(-72.0),
                "the output keeps its sideways scroll in {mode:?} mode"
            );
            for handle in [
                &pane.conflict_resolver_diff_scroll,
                &pane.conflict_preview_ours_scroll,
                &pane.conflict_preview_theirs_scroll,
            ] {
                assert_eq!(uniform_list_offset(handle).x, px(0.0));
            }
        });
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict markdown matrix fixture");
}

#[gpui::test]
fn worktree_markdown_preview_wraps_long_rows_within_the_viewport(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
                pane.worktree_markdown.path = Some(abs_path.clone());
                pane.worktree_markdown.source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown.document =
                    gitcomet_state::model::Loadable::Ready(Arc::new(document));
                pane.worktree_markdown.inflight = None;
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
            let document = match &pane.worktree_markdown.document {
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

#[gpui::test]
fn source_mode_word_wrap_splits_a_long_line_over_several_rows(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Text mode draws the file through the same list every source view uses,
    // and that list took the file's line count as its length — so the Word wrap
    // toggle had nothing to act on and a long line just ran off the pane.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The trap this repeats from the diff: the rows are painted in the editor
    // font, but the wrap width is worked out while the ambient UI font is still
    // current. Measuring the wrong face gives the wrong column count, and every
    // wrapped row lands short or runs past the pane.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

fn cell_box(cx: &mut gpui::VisualTestContext, row_ix: usize, column: usize) -> Bounds<Pixels> {
    cx.debug_bounds(leaked_selector(format!(
        "markdown_preview_cell_box_{row_ix}_{column}"
    )))
    .unwrap_or_else(|| panic!("cell {column} of row {row_ix} is drawn"))
}

#[gpui::test]
fn a_table_is_a_grid_whose_long_cells_wrap_inside_the_pane(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let long_note = "a note that goes on ".repeat(30);
    let source = format!(
        "| Name | Notes | Count |\n|:--|---|--:|\n| a | short | 1 |\n| bb | {long_note}| 22 |\n| ccc | x | 333 |\n"
    );
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(91),
        "markdown_table_grid",
        &source,
    );
    let rows = table_row_ixs(&fixture);
    assert_eq!(rows.len(), 4);
    assert!(
        cx.debug_bounds("markdown_preview_block_change_bar")
            .is_none(),
        "a file that is not being diffed has no change bars"
    );

    // Every row's cells sit in the same columns.
    for column in 0..3 {
        let lefts: Vec<Pixels> = rows
            .iter()
            .map(|row| cell_box(cx, *row, column).left())
            .collect();
        assert!(
            lefts.windows(2).all(|pair| pair[0] == pair[1]),
            "column {column} lines up: {lefts:?}"
        );
    }
    // The long cell wraps, so its row grows, and every cell of that row
    // stretches with it; the table still fits the pane.
    let short = cell_box(cx, rows[1], 1);
    let long = cell_box(cx, rows[2], 1);
    assert!(
        long.size.height > short.size.height * 2.0,
        "the long note wraps over several lines: {long:?} vs {short:?}"
    );
    assert_eq!(cell_box(cx, rows[2], 0).size.height, long.size.height);
    let container = cx
        .debug_bounds("worktree_markdown_preview_scroll_container")
        .expect("expected the preview container");
    assert!(
        cell_box(cx, rows[2], 2).right() <= container.right(),
        "a wrapped table fits the pane"
    );

    // `--:` puts the count against the right edge of its cell; `:--` keeps
    // the name at the left.
    let text_box = |cx: &mut gpui::VisualTestContext, row: usize, column: usize| {
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_text_box_{row}_{column}"
        )))
        .expect("cell text")
    };
    let pad = |cx: &mut gpui::VisualTestContext, row: usize, column: usize| {
        (
            text_box(cx, row, column).left() - cell_box(cx, row, column).left(),
            cell_box(cx, row, column).right() - text_box(cx, row, column).right(),
        )
    };
    let (name_left, name_right) = pad(cx, rows[1], 0);
    assert!(name_left < name_right, "left-aligned `a` hugs the left");
    let (count_left, count_right) = pad(cx, rows[1], 2);
    assert!(count_left > count_right, "right-aligned `1` hugs the right");
    assert_eq!(
        text_box(cx, rows[1], 2).right(),
        text_box(cx, rows[3], 2).right(),
        "right-aligned values end at the same x"
    );
    let (header_left, header_right) = pad(cx, rows[0], 0);
    assert!(
        (header_left - header_right).abs() <= px(1.0),
        "headers are centred"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_table_hugs_its_columns_and_a_long_word_breaks_inside_the_pane(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let wide_cell = "w".repeat(200);
    let source = format!(
        "| a | b |\n| --- | --- |\n| c | d |\n\n| {wide_cell} | x |\n| --- | --- |\n| e | f |\n"
    );
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(92),
        "markdown_table_scroll",
        &source,
    );
    let rows = table_row_ixs(&fixture);
    let container = cx
        .debug_bounds("worktree_markdown_preview_scroll_container")
        .expect("expected the preview container");

    let narrow_right = cell_box(cx, rows[0], 1).right();
    assert!(
        narrow_right < container.left() + container.size.width / 2.0,
        "a small table is as wide as its columns, not the pane"
    );
    let wide = cell_box(cx, rows[2], 0);
    assert!(
        wide.right() <= container.right(),
        "a word longer than the pane breaks inside its cell rather than widening the page"
    );
    assert!(
        wide.size.height > cell_box(cx, rows[0], 0).size.height,
        "so the cell grows downwards"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_code_block_wider_than_the_pane_gets_a_scrollbar(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A block that scrolls sideways with nothing to say so leaves the reader
    // with no idea there is more of the line, and no way to reach it but a
    // horizontal wheel. The bar is drawn for every block, but only has a thumb
    // where there is somewhere to scroll to.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            .worktree_markdown
            .block_scrolls
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // `gpui` sends a plain wheel to whichever axis an element scrolls, so a
    // block that only scrolls sideways would take the page's scroll the moment
    // the pointer crossed it and the document would stop moving.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A code line longer than the pane scrolls rather than wrapping or being
    // clipped, and each block holds its own offset — which is what the per-block
    // element id is for. A shared id made them scroll as one.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn worktree_markdown_preview_change_bar_is_unbroken_for_a_wholly_added_file(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn split_markdown_diff_leaves_blank_space_so_both_sides_stay_lined_up(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(700.0)));

    let long = "a replacement paragraph that is far longer than the one it replaces ".repeat(8);
    let old_text = "Intro.\n\nShort.\n\n- one\n- two\n\nEnd.\n";
    let new_text = format!(
        "Intro.\n\n{}\n\n- one\n- two\n- three\n\nEnd.\n",
        long.trim_end()
    );
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(94),
        "markdown_split_bands",
        old_text,
        &new_text,
        DiffViewMode::Split,
    );

    let (old_end, new_end) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        (
            row_ix_with_text(&preview.old, "End."),
            row_ix_with_text(&preview.new, "End."),
        )
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let left = pane
            .diff_text_hitbox_bounds_for_tests(old_end, DiffTextRegion::SplitLeft)
            .expect("old End. is drawn");
        let right = pane
            .diff_text_hitbox_bounds_for_tests(new_end, DiffTextRegion::SplitRight)
            .expect("new End. is drawn");
        assert_eq!(
            left.top(),
            right.top(),
            "the long paragraph and the extra list item are matched by blank space on the old side"
        );
        assert!(
            left.right() <= right.left(),
            "old on the left, new on the right"
        );
    });

    assert!(
        cx.debug_bounds("markdown_preview_block_change_bar")
            .is_some(),
        "the wholly replaced paragraph is marked down its side"
    );

    // One scroller carries both sides.
    let document = cx
        .debug_bounds("diff_markdown_preview_document")
        .expect("the flowing diff scrolls as one document");
    assert!(document.size.height > px(0.0));

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn markdown_diff_scrollbar_markers_sit_where_the_change_is_drawn(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // By row count the change is near the end: one long paragraph follows it.
    // Drawn, that paragraph wraps into many lines, so the change is mid-way.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(400.0)));

    let short: String = (0..20).map(|ix| format!("Line {ix}.\n\n")).collect();
    let long = "a paragraph long enough to wrap over a great many lines ".repeat(60);
    let old_text = format!("{short}Before.\n\n{long}\n");
    let new_text = format!("{short}After.\n\n{long}\n");
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(96),
        "markdown_diff_markers",
        &old_text,
        &new_text,
        DiffViewMode::Split,
    );

    let markers = cx.update(|window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview
            else {
                panic!("the preview is ready");
            };
            let preview = Arc::clone(preview);
            let scroll = pane.diff_scroll.0.borrow().base_handle.clone();
            pane.markdown_diff_scrollbar_markers(&preview, &scroll, window, cx)
        })
    });
    assert!(!markers.is_empty(), "the change is marked");
    assert!(
        markers.iter().all(|marker| marker.start < 0.75),
        "a row count would put it near the end; drawn, it is mid-way: {markers:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn inline_markdown_diff_shows_the_removed_version_before_the_added_one(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(700.0)));

    let old_text = "Intro.\n\n| A | B |\n|---|---|\n| 1 | old |\n\nEnd.\n";
    let new_text = "Intro.\n\n| A | B |\n|---|---|\n| 1 | new |\n\nEnd.\n";
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(95),
        "markdown_inline_bands",
        old_text,
        new_text,
        DiffViewMode::Inline,
    );

    let (removed, added) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        (
            row_ix_with_text(&preview.inline, "1\told"),
            row_ix_with_text(&preview.inline, "1\tnew"),
        )
    });
    assert_eq!(
        removed + 1,
        added,
        "the old row sits right above its replacement"
    );
    // Both rows are cells of one table grid, so they share its columns.
    let removed_cell = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_box_{removed}_1"
        )))
        .expect("removed row drawn as table cells");
    let added_cell = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_box_{added}_1"
        )))
        .expect("added row drawn as table cells");
    assert_eq!(removed_cell.left(), added_cell.left());
    assert!(removed_cell.bottom() <= added_cell.top() + px(1.0));

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn split_markdown_eof_ignores_trailing_alignment_padding(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
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
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(preview));
                pane.diff_markdown.inflight = None;
                cx.notify();
            });
            this.set_diff_word_wrap(true, cx);
        });
    });
    for _ in 0..3 {
        draw_and_drain_test_window(cx);
    }

    // The flowing split addresses document rows: the padding after the tail
    // draws nothing, so the tail is the old side's last row.
    let old_tail_visual_ix = old_tail_row_ix;
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
                pane.worktree_markdown.path = Some(abs_path.clone());
                pane.worktree_markdown.source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown.document =
                    gitcomet_state::model::Loadable::Ready(Arc::new(document));
                pane.worktree_markdown.inflight = None;
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
fn markdown_preview_text_box_starts_where_the_text_is_painted(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The selection highlight is painted inside the text box, so the box must
    // be the glyph box. Padding applied to the box itself shifted the highlight
    // left of the text and cut it short at the end of the line.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

/// While the rendered preview is still parsing, the pane paints a notice rather
/// than the document. Nothing is on screen to find, and the markdown source
/// underneath is not what the reader is looking at, so search reports nothing
/// instead of quietly scanning a view that is not there.
#[gpui::test]
fn a_markdown_preview_without_a_document_reports_no_matches(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Loading;
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

#[gpui::test]
fn split_markdown_diff_new_side_rows_take_clicks_and_context_menus(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Both columns of a band render the same row indices, so every element id
    // under them has to be told apart by its column or the two sides share one
    // element's click state.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(700.0)));

    let old_text = "Shared [the docs](https://example.com/docs) paragraph.\n\nOld ending.\n";
    let new_text = "Shared [the docs](https://example.com/docs) paragraph.\n\nNew ending.\n";
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(960),
        "markdown_split_new_side_clicks",
        old_text,
        new_text,
        DiffViewMode::Split,
    );
    let row = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        let row = row_ix_with_text(&preview.new, "Shared the docs paragraph.");
        assert_eq!(
            row_ix_with_text(&preview.old, "Shared the docs paragraph."),
            row,
            "the unchanged row sits at one index on both sides"
        );
        row
    });

    let on_link = point_on_link_in_region(cx, &view, row, DiffTextRegion::SplitRight);
    simulate_counted_click(cx, on_link, 1);
    cx.run_until_parked();
    let popover = popover_kind(cx, &view);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::WebLinkMenu { ref url, .. }) if url.as_ref() == "https://example.com/docs"
        ),
        "a link on the new side opens its menu, got {popover:?}"
    );
    close_popover(cx, &view);

    let text = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_text_hitbox_bounds_for_tests(row, DiffTextRegion::SplitRight)
            .expect("the new row is drawn")
    });
    let on_words = point(text.left() + px(4.0), text.center().y);
    cx.simulate_mouse_move(on_words, None, Modifiers::default());
    cx.simulate_mouse_down(on_words, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(on_words, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| view.read(app).active_context_menu_invoker.clone()),
        Some("diff_editor_menu".into()),
        "a right-click on the new side opens the diff context menu"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn inline_markdown_diff_keeps_table_rows_whole_when_a_column_is_added(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(700.0)));

    let old_text = "| A | B | C |\n|---|---|---|\n| 1 | 2 | 3 |\n| 4 | 5 | 6 |\n";
    let new_text = "| A | B | C | D |\n|---|---|---|---|\n| 1 | 2 | 3 | x |\n| 4 | 5 | 6 | y |\n";
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(961),
        "markdown_inline_table_column_added",
        old_text,
        new_text,
        DiffViewMode::Inline,
    );
    let rows: Vec<(usize, usize)> = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        preview
            .inline
            .rows
            .iter()
            .enumerate()
            .filter_map(|(ix, row)| row.table.as_ref().map(|table| (ix, table.cells.len())))
            .collect()
    });
    assert!(rows.len() >= 6, "old and new rows are both drawn: {rows:?}");

    for (row_ix, cells) in rows {
        let first = cell_box(cx, row_ix, 0);
        for column in 1..cells {
            let cell = cell_box(cx, row_ix, column);
            assert_eq!(
                cell.top(),
                first.top(),
                "cell {column} of row {row_ix} left its row: first={first:?} cell={cell:?}"
            );
            assert!(
                cell.left() >= first.right() - px(1.0),
                "cell {column} of row {row_ix} sits right of the first cell"
            );
        }
    }

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn the_budget_fallback_to_source_ends_with_the_file_that_needed_it(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(965);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_budget_fallback_scope",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");
    let huge = std::path::PathBuf::from("docs/huge.md");
    let small = std::path::PathBuf::from("docs/small.md");
    let (huge_old, huge_new) = inline_overflowing_markdown_diff();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });
    seed_file_diff_state(cx, &view, repo_id, &workdir, &huge, &huge_old, &huge_new);
    wait_for_main_pane_condition(
        cx,
        &view,
        "the huge diff falls back to source",
        |pane| {
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Source
        },
        |pane| {
            (
                pane.diff_markdown.inflight,
                matches!(
                    pane.diff_markdown.preview,
                    gitcomet_state::model::Loadable::Error(_)
                ),
            )
        },
    );

    seed_file_diff_state(cx, &view, repo_id, &workdir, &small, "# one\n", "# two\n");
    let small_target = gitcomet_core::domain::DiffTarget::working_tree(
        small.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "the small diff is shown",
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
                == Some(small_target.clone())
        },
        |pane| {
            pane.active_repo()
                .and_then(|repo| repo.diff_state.diff_target.clone())
        },
    );
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "the reader chose Rendered; only the huge file had to be shown as source"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn split_markdown_diff_of_an_added_file_says_the_old_side_is_empty(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(967),
        "markdown_split_added_file_empty_side",
        "",
        "# Added\n\nNew words.\n",
        DiffViewMode::Split,
    );
    let empty = cx
        .debug_bounds("markdown_diff_empty_side_SplitLeft")
        .expect("the empty old column says so");
    let added = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_text_hitbox_bounds_for_tests(0, DiffTextRegion::SplitRight)
            .expect("the added heading is drawn")
    });
    assert!(
        empty.right() <= added.left(),
        "the notice sits in the old column"
    );
    assert!(
        cx.debug_bounds("markdown_diff_empty_side_SplitRight")
            .is_none(),
        "the new column has content"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn inline_code_keeps_the_surrounding_prose_in_the_body_font(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8801),
        "markdown_inline_code_font",
        "Run `cargo test` before pushing.\n",
    );
    let row_ix = fixture.row_ix("Run cargo test before pushing.");
    let editor_family: SharedString =
        cx.update(|_window, app| crate::font_preferences::current_editor_font_family(app).into());

    crate::view::rows::begin_markdown_flow_font_capture_for_tests();
    draw_frames(cx, 1);
    let runs = crate::view::rows::markdown_flow_fonts_for_tests(row_ix);
    let family_at = |offset: usize| {
        runs.iter()
            .find(|(range, _)| range.contains(&offset))
            .map(|(_, family)| family.clone())
            .unwrap_or_else(|| panic!("no run covers byte {offset}: {runs:?}"))
    };

    // "Run " is prose, "cargo test" (bytes 4..14) is the code span.
    assert_ne!(
        family_at(0),
        editor_family,
        "prose around inline code must keep the body font, not the editor font: {runs:?}"
    );
    assert_eq!(
        family_at(5),
        editor_family,
        "the code span itself is set in the editor font: {runs:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn switching_between_dark_themes_restyles_an_open_markdown_preview(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let tab_width = 4;

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let set_pane_theme = |cx: &mut gpui::VisualTestContext, theme: AppTheme| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane
                    .update(cx, |pane, cx| pane.set_theme(theme, cx));
            });
        });
    };
    let source = "A [link](https://example.com) and `code`.\n";

    set_pane_theme(cx, AppTheme::gitcomet_dark());
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8802),
        "markdown_theme_switch",
        source,
    );
    // The first frames styled every row under GitComet Dark.
    let amber = AppTheme::from_key(crate::theme::AMBER_DARK_THEME_KEY).expect("Amber Dark");
    set_pane_theme(cx, amber);
    draw_frames(cx, 2);

    let (shown, fresh) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(document) = &pane.worktree_markdown.document
        else {
            panic!("expected a ready preview");
        };
        let shown = crate::view::rows::markdown_preview_styled_row_with_query(
            tab_width,
            pane.theme,
            &document.rows[0],
            0,
            None,
            None,
        )
        .highlights
        .clone();
        let fresh_document =
            crate::view::markdown_preview::parse_markdown(source).expect("fixture parses");
        let fresh = crate::view::rows::markdown_preview_styled_row_with_query(
            tab_width,
            amber,
            &fresh_document.rows[0],
            0,
            None,
            None,
        )
        .highlights
        .clone();
        (shown, fresh)
    });
    assert_eq!(
        shown, fresh,
        "after a theme switch the open preview keeps the previous theme's link and code colours"
    );

    fixture.cleanup();
}

#[gpui::test]
fn code_block_inside_a_list_item_is_indented_with_its_item(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    // Loose on purpose: a tight item currently loses its text (see the parser
    // test `tight_list_item_keeps_its_text_before_a_fenced_code_block`).
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8803),
        "markdown_code_in_list_indent",
        "1. Install:\n\n   ```sh\n   cargo install foo\n   ```\n",
    );
    let item_ix = fixture.row_ix("Install:");
    let code_ix = fixture.row_ix("cargo install foo");

    let item_text = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{item_ix}"
        )))
        .expect("item text box");
    let code_shell = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_code_shell_{code_ix}"
        )))
        .expect("code block shell");
    assert!(
        code_shell.left() + px(1.0) >= item_text.left(),
        "a code block inside a list item starts at the document margin instead of under its \
         item's text: code={code_shell:?} item={item_text:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn footnote_definition_shows_its_label(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8804),
        "markdown_footnote_label",
        "- item\n\nText[^1].\n\n[^1]: Note body.\n",
    );
    let item_ix = fixture.row_ix("item");
    let note_ix = fixture.row_ix("Note body.");

    // Control: a list item's marker slot carries the selector.
    assert!(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_marker_{item_ix}"
        )))
        .is_some(),
        "the list marker slot must be addressable"
    );
    assert!(
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_marker_{note_ix}"
        )))
        .is_some(),
        "a footnote definition renders without its `[^1]:` label"
    );

    fixture.cleanup();
}

/// A CHANGELOG-shaped document: release headings, sections of list items with
/// inline code and links, and a short code block per release.
fn changelog_markdown(releases: usize) -> String {
    let mut source = String::from("# Changelog\n\nAll notable changes to this project.\n\n");
    for release in (0..releases).rev() {
        source.push_str(&format!(
            "## [1.{release}.0] - 2026-09-{:02}\n\n",
            release % 28 + 1
        ));
        source.push_str("### Added\n\n");
        for item in 0..4 {
            source.push_str(&format!(
                "- Support `option_{release}_{item}` in the [config loader](https://example.com/pr/{release}{item}) (#{release}{item})\n"
            ));
        }
        source.push_str("\n### Fixed\n\n- A crash when `path` is empty\n\n");
        source.push_str(&format!("```rust\nlet release = {release};\n```\n\n"));
    }
    source
}

#[gpui::test]
#[ignore = "production GPUI draw benchmark for the flowing markdown preview"]
fn markdown_preview_real_frame_benchmark(cx: &mut gpui::TestAppContext) {
    use std::time::Instant;
    let _visual_guard = lock_visual_test();
    let releases: usize = std::env::var("GITCOMET_BENCH_MD_RELEASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(280);
    let source = match std::env::var("GITCOMET_BENCH_MD_FILE") {
        Ok(path) => std::fs::read_to_string(path).expect("read benchmark markdown"),
        Err(_) => changelog_markdown(releases),
    };
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(1000.0)));
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8810),
        "markdown_real_frame_benchmark",
        &source,
    );
    let rows = fixture.document.rows.len();
    let blocks = crate::view::markdown_preview::markdown_document_blocks(&fixture.document).len();
    let Some(first_text) = cx.debug_bounds("markdown_preview_text_box_0") else {
        eprintln!(
            "markdown flowing preview rows={rows}: not drawn (past the parser's cap, \
             the pane shows its notice instead)"
        );
        fixture.cleanup();
        return;
    };
    let _cached_views = std::env::var_os("GITCOMET_BENCH_CACHED_VIEWS")
        .map(|_| crate::view::enable_stable_cached_views_for_test());
    for _ in 0..2 {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }

    const FRAMES: usize = 40;
    let percentile = |samples: &mut Vec<f64>, p: usize| {
        samples.sort_by(f64::total_cmp);
        samples[(samples.len() - 1) * p / 100]
    };

    // A frame that rebuilds everything, as a tooltip, hover change, or any
    // `window.refresh()` elsewhere in the window forces.
    let mut rebuild_ms = Vec::new();
    let mut rebuild_allocs = crate::perf_alloc::PerfAllocMetrics::default();
    for _ in 0..FRAMES {
        let started = Instant::now();
        let (_, allocations) = crate::perf_alloc::measure_allocations(|| {
            cx.update(|window, app| {
                window.refresh();
                let _ = window.draw(app);
            })
        });
        rebuild_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        rebuild_allocs = rebuild_allocs.saturating_add(allocations);
    }

    // A wheel tick over the document and the frame it produces.
    let mut wheel_ms = Vec::new();
    for frame in 0..FRAMES {
        let started = Instant::now();
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: first_text.center(),
            delta: gpui::ScrollDelta::Pixels(point(
                px(0.0),
                px(if frame % 2 == 0 { -3.25 } else { 3.25 }),
            )),
            ..Default::default()
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        wheel_ms.push(started.elapsed().as_secs_f64() * 1000.0);
    }

    // Pointer moves inside one paragraph: nothing changes on screen, so this
    // is the cost of dispatching the event to every registered listener.
    let mut move_us = Vec::new();
    for step in 0..FRAMES {
        let x = first_text.left() + px(2.0 + (step % 8) as f32);
        let started = Instant::now();
        cx.simulate_mouse_move(
            point(x, first_text.center().y),
            None,
            gpui::Modifiers::none(),
        );
        move_us.push(started.elapsed().as_secs_f64() * 1e6);
    }

    eprintln!(
        "markdown flowing preview rows={rows} blocks={blocks} profile={} \
         rebuild_ms_p50={:.2} rebuild_ms_p95={:.2} allocs_per_rebuild={:.0} \
         wheel_frame_ms_p50={:.2} wheel_frame_ms_p95={:.2} mouse_move_us_p50={:.1} mouse_move_us_p95={:.1}",
        if cfg!(debug_assertions) {
            "test"
        } else {
            "release"
        },
        percentile(&mut rebuild_ms, 50),
        percentile(&mut rebuild_ms, 95),
        rebuild_allocs.alloc_ops as f64 / FRAMES as f64,
        percentile(&mut wheel_ms, 50),
        percentile(&mut wheel_ms, 95),
        percentile(&mut move_us, 50),
        percentile(&mut move_us, 95),
    );

    fixture.cleanup();
}

/// A CHANGELOG whose contents list links to release headings near its top,
/// middle and end — the last with a screenful below it, so it can reach the
/// top of the viewport. Returns the source and each link's text with the
/// heading it names.
fn changelog_with_contents(releases: usize) -> (String, Vec<(&'static str, String)>) {
    let heading = |release: usize| format!("[1.{release}.0] - 2026-09-{:02}", release % 28 + 1);
    let targets = [
        ("Jump near", releases.saturating_sub(1)),
        ("Jump middle", releases / 2),
        ("Jump far", 6.min(releases / 4)),
    ];
    let mut contents = String::from("## Contents\n\n");
    for (label, release) in targets {
        let slug = crate::view::markdown_preview::markdown_heading_slug(&heading(release));
        contents.push_str(&format!("- [{label}](#{slug})\n"));
    }
    contents.push('\n');
    let source = changelog_markdown(releases).replacen("\n\n", &format!("\n\n{contents}"), 1);
    let targets = targets
        .into_iter()
        .map(|(label, release)| (label, heading(release)))
        .collect();
    (source, targets)
}

/// Draw the next frame until one passes without the main pane asking for
/// another, and return how many did. `notified` counts the pane's notifies;
/// other views (a busy spinner) keep queueing frames of their own.
fn settle_preview_frames(
    cx: &mut gpui::VisualTestContext,
    notified: &std::cell::Cell<usize>,
    limit: usize,
) -> usize {
    for frame in 0..limit {
        let before = notified.get();
        cx.update(|window, app| window.simulate_next_frame(app));
        cx.run_until_parked();
        if notified.get() == before {
            return frame;
        }
    }
    limit
}

#[gpui::test]
#[ignore = "production GPUI benchmark for markdown selection and anchor links"]
fn markdown_preview_interaction_benchmark(cx: &mut gpui::TestAppContext) {
    use std::time::{Duration, Instant};
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let releases: usize = std::env::var("GITCOMET_BENCH_MD_RELEASES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(280);
    let (source, targets) = changelog_with_contents(releases);
    // Production mounts the panes as cached views; frames reuse the ones that
    // did not change.
    let _cached_views = std::env::var_os("GITCOMET_BENCH_UNCACHED_VIEWS")
        .is_none()
        .then(crate::view::enable_stable_cached_views_for_test);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(1000.0)));
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8830),
        "markdown_interaction_benchmark",
        &source,
    );
    // Loaded, not opening: an opening repository spins a busy icon in its tab,
    // which queues a frame every frame and would count in every measurement.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut state = (*this.store.snapshot()).clone();
            for repo in &mut state.repos {
                repo.open = gitcomet_state::model::Loadable::Ready(());
            }
            push_test_state(this, Arc::new(state), cx);
        });
    });
    draw_frames(cx, 2);
    let rows = fixture.document.rows.len();
    let profile = if cfg!(debug_assertions) {
        "test"
    } else {
        "release"
    };
    // Cached views replay no debug bounds, so geometry comes from the text
    // hitboxes the preview records as it paints.
    let row_bounds = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .diff_text_hitbox_bounds_for_tests(row_ix, DiffTextRegion::Inline)
        })
    };
    let point_on_link = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        let bounds = row_bounds(cx, row_ix).expect("the link's row is drawn");
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let mut y = bounds.top() + px(2.0);
            while y < bounds.bottom() {
                let mut x = bounds.left();
                while x < bounds.right() {
                    let position = point(x, y);
                    if pane
                        .markdown_preview_link_span_at(row_ix, DiffTextRegion::Inline, position)
                        .is_some()
                    {
                        return position;
                    }
                    x += px(2.0);
                }
                y += px(4.0);
            }
            panic!("no link in row {row_ix}");
        })
    };
    if row_bounds(cx, 0).is_none() {
        eprintln!("markdown interaction rows={rows}: not drawn (past the parser's cap)");
        fixture.cleanup();
        return;
    }
    let scroll = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .worktree_preview_scroll
            .0
            .borrow()
            .base_handle
            .clone()
    });
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let _subscription = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let notified = std::rc::Rc::clone(&notified);
        app.observe(&main_pane, move |_, _| notified.set(notified.get() + 1))
    });
    let to_top = |cx: &mut gpui::VisualTestContext| {
        set_scroll_handle_offset(&scroll, point(px(0.0), px(0.0)));
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        settle_preview_frames(cx, &notified, 64);
    };
    let elapsed_ms = |started: Instant| started.elapsed().as_secs_f64() * 1000.0;
    let percentile = |samples: &mut Vec<f64>, p: usize| {
        samples.sort_by(f64::total_cmp);
        samples[(samples.len() - 1) * p / 100]
    };
    let press = |cx: &mut gpui::VisualTestContext, position, click_count| {
        cx.simulate_event(gpui::MouseDownEvent {
            position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count,
            first_mouse: false,
        });
    };
    let release = |cx: &mut gpui::VisualTestContext, position| {
        cx.simulate_event(gpui::MouseUpEvent {
            position,
            modifiers: Modifiers::default(),
            button: MouseButton::Left,
            click_count: 1,
        });
    };
    let selection_rows = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            let rows = pane
                .diff_text_anchor
                .zip(pane.diff_text_head)
                .map_or(0, |(anchor, head)| {
                    anchor.source_visible_ix.abs_diff(head.source_visible_ix)
                });
            (rows, pane.diff_text_hitboxes.len())
        })
    };
    eprintln!(
        "markdown interaction rows={rows} blocks={} profile={profile}",
        crate::view::markdown_preview::markdown_document_blocks(&fixture.document).len()
    );

    // Anchor links: hover (the first hover checks the link goes somewhere),
    // click, then the frames the jump takes to come to rest.
    for (label, heading) in &targets {
        to_top(cx);
        let heading_row = fixture.row_ix(heading);
        let on_link = point_on_link(cx, fixture.row_ix(label));
        let started = Instant::now();
        cx.simulate_mouse_move(on_link, None, Modifiers::default());
        let hover_ms = elapsed_ms(started);
        let started = Instant::now();
        press(cx, on_link, 1);
        release(cx, on_link);
        let click_ms = elapsed_ms(started);
        let started = Instant::now();
        let frames = settle_preview_frames(cx, &notified, 64);
        let settle_ms = elapsed_ms(started);
        let heading_offset = row_bounds(cx, heading_row)
            .map(|bounds| f32::from(bounds.top() - scroll.bounds().top()));
        eprintln!(
            "markdown interaction anchor={label} heading_row={heading_row} hover_ms={hover_ms:.2} \
             click_ms={click_ms:.2} settle_frames={frames} settle_ms={settle_ms:.2} \
             heading_offset_px={heading_offset:?} scroll_y={:.0}",
            f32::from(scroll.offset().y)
        );
    }

    const FRAMES: usize = 40;
    to_top(cx);
    let start_row = fixture.row_ix("All notable changes to this project.");
    let start = row_bounds(cx, start_row).expect("the first paragraph is drawn");
    let viewport = scroll.bounds();
    let anchor_at = point(start.left() + px(1.0), start.center().y);

    // Word and line selection.
    let on_word = point(start.left() + px(30.0), start.center().y);
    let mut word_ms = Vec::new();
    let mut line_ms = Vec::new();
    for _ in 0..FRAMES / 4 {
        for (clicks, samples) in [(2, &mut word_ms), (3, &mut line_ms)] {
            cx.simulate_mouse_move(on_word, None, Modifiers::default());
            let started = Instant::now();
            press(cx, on_word, clicks);
            release(cx, on_word);
            samples.push(elapsed_ms(started));
        }
    }

    // A drag down the window: every move lands on a new row, so each one
    // extends the selection and draws a frame.
    cx.simulate_mouse_move(anchor_at, None, Modifiers::default());
    let started = Instant::now();
    press(cx, anchor_at, 1);
    let press_ms = elapsed_ms(started);
    let mut drag_ms = Vec::new();
    let mut drag_allocs = crate::perf_alloc::PerfAllocMetrics::default();
    let mut last = anchor_at;
    for step in 0..FRAMES {
        last = point(
            viewport.left() + px(120.0 + 37.0 * (step % 7) as f32),
            viewport.top() + viewport.size.height * ((step + 1) as f32 / (FRAMES + 1) as f32),
        );
        let started = Instant::now();
        let (_, allocations) = crate::perf_alloc::measure_allocations(|| {
            cx.simulate_mouse_move(last, Some(MouseButton::Left), Modifiers::default())
        });
        drag_ms.push(elapsed_ms(started));
        drag_allocs = drag_allocs.saturating_add(allocations);
    }
    let (drag_rows, hitboxes) = selection_rows(cx);
    release(cx, last);
    eprintln!(
        "markdown interaction select press_ms={press_ms:.2} word_ms_p50={:.2} line_ms_p50={:.2} \
         drag_move_ms_p50={:.2} drag_move_ms_p95={:.2} allocs_per_drag_move={:.0} \
         drag_rows={drag_rows} hitboxes={hitboxes}",
        percentile(&mut word_ms, 50),
        percentile(&mut line_ms, 50),
        percentile(&mut drag_ms, 50),
        percentile(&mut drag_ms, 95),
        drag_allocs.alloc_ops as f64 / FRAMES as f64,
    );

    // A drag held below the window: the selection autoscrolls one tick every
    // 16 ms, each tick moving the view and extending the selection. It leaves
    // the pane across the strip of window below it, as a real pointer does.
    to_top(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.clear_diff_text_selection();
                cx.notify();
            });
        });
    });
    cx.simulate_mouse_move(anchor_at, None, Modifiers::default());
    press(cx, anchor_at, 1);
    let under_pane = point(viewport.center().x, viewport.bottom() + px(20.0));
    cx.simulate_mouse_move(under_pane, Some(MouseButton::Left), Modifiers::default());
    let below = point(viewport.center().x, viewport.bottom() + px(200.0));
    cx.simulate_mouse_move(below, Some(MouseButton::Left), Modifiers::default());
    let offset_before = scroll.offset().y;
    let mut tick_ms = Vec::new();
    let mut tick_frames = 0usize;
    const TICKS: usize = 120;
    for _ in 0..TICKS {
        let started = Instant::now();
        cx.executor().advance_clock(Duration::from_millis(16));
        cx.run_until_parked();
        tick_frames += 1 + settle_preview_frames(cx, &notified, 8);
        tick_ms.push(elapsed_ms(started));
    }
    let scrolled = f32::from(offset_before - scroll.offset().y);
    let (autoscroll_rows, _) = selection_rows(cx);
    release(cx, below);
    eprintln!(
        "markdown interaction autoscroll ticks={TICKS} tick_ms_p50={:.2} tick_ms_p95={:.2} \
         frames_per_tick={:.2} scrolled_px={scrolled:.0} rows_selected={autoscroll_rows} \
         rows_per_second={:.0}",
        percentile(&mut tick_ms, 50),
        percentile(&mut tick_ms, 95),
        tick_frames as f64 / TICKS as f64,
        autoscroll_rows as f64 / (TICKS as f64 * 0.016),
    );

    // Select everything, draw with it selected, copy it.
    to_top(cx);
    let started = Instant::now();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.select_all_diff_text(window, cx);
                cx.notify();
            });
        });
    });
    let select_all_ms = elapsed_ms(started);
    let mut selected_frame_ms = Vec::new();
    crate::view::rows::take_markdown_flow_texts_built_for_tests();
    for _ in 0..FRAMES {
        let started = Instant::now();
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
        selected_frame_ms.push(elapsed_ms(started));
    }
    let rows_built = crate::view::rows::take_markdown_flow_texts_built_for_tests();
    let started = Instant::now();
    let (_, copy_allocs) = crate::perf_alloc::measure_allocations(|| {
        cx.update(|_window, app| {
            let main_pane = view.read(app).main_pane.clone();
            main_pane.update(app, |pane, cx| {
                pane.copy_selected_diff_text_to_clipboard(cx)
            });
        })
    });
    let copy_ms = elapsed_ms(started);
    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .map_or(0, |text| text.len());
    eprintln!(
        "markdown interaction select_all_ms={select_all_ms:.2} selected_frame_ms_p50={:.2} \
         rows_built_per_frame={} copy_ms={copy_ms:.2} copy_allocs={} copied_bytes={copied}",
        percentile(&mut selected_frame_ms, 50),
        rows_built / FRAMES,
        copy_allocs.alloc_ops,
    );

    fixture.cleanup();
}

// ── Frame-cost regression tests: counts, not timings, so they are stable ──

#[gpui::test]
fn a_frame_of_a_long_markdown_preview_builds_only_rows_near_the_viewport(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let source = (0..1_200)
        .map(|ix| format!("Paragraph number {ix}.\n\n"))
        .collect::<String>();
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8811),
        "markdown_frame_rows_built",
        &source,
    );
    assert_eq!(fixture.document.rows.len(), 1_200);

    crate::view::rows::take_markdown_flow_texts_built_for_tests();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    let built = crate::view::rows::take_markdown_flow_texts_built_for_tests();
    // The window shows a few dozen rows; a generous overscan is still far
    // below the document.
    assert!(
        built <= 300,
        "one frame built {built} row texts for a 1,200-row document; every row is laid out and \
         painted on every frame, so frame cost grows with the document instead of the window"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_frame_of_the_markdown_preview_checks_the_preview_surface_a_bounded_number_of_times(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let source = (0..300)
        .map(|ix| format!("Paragraph number {ix}.\n\n"))
        .collect::<String>();
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8812),
        "markdown_frame_surface_checks",
        &source,
    );

    crate::view::panes::main::take_file_preview_active_checks_for_tests();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    let checks = crate::view::panes::main::take_file_preview_active_checks_for_tests();
    // Each check stats the previewed file; paint used to make two per row.
    assert!(
        checks <= 8,
        "one frame ran the file-preview surface check {checks} times, each with filesystem stats"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_reveal_of_a_row_only_the_new_side_draws_brings_that_row_into_view(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The old side pads the row the new side inserted, inside its own list.
    // The padded side must not answer the reveal by centring its whole list.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(500.0)));

    let items = |insert: bool| {
        let mut text = String::new();
        for ix in 0..60 {
            text.push_str(&format!("- item {ix}\n"));
            if insert && ix == 50 {
                text.push_str("- inserted\n");
            }
        }
        text
    };
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(8815),
        "markdown_split_reveal_padding",
        &items(false),
        &items(true),
        DiffViewMode::Split,
    );
    let row = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        let row = row_ix_with_text(&preview.new, "inserted");
        assert!(
            matches!(
                preview.old.rows[row].kind,
                crate::view::markdown_preview::MarkdownPreviewRowKind::Spacer
            ),
            "the old side pads the inserted row"
        );
        row
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_scroll
                    .0
                    .borrow()
                    .base_handle
                    .set_offset(point(px(0.0), px(0.0)));
                pane.markdown_interaction.reveal.request(row);
                cx.notify();
            });
        });
    });
    draw_frames(cx, 3);

    let (viewport, inserted) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.diff_scroll.0.borrow().base_handle.bounds(),
            pane.diff_text_hitbox_bounds_for_tests(row, DiffTextRegion::SplitRight),
        )
    });
    let inserted = inserted.expect("the inserted row is drawn");
    assert!(
        inserted.top() >= viewport.top() && inserted.bottom() <= viewport.bottom(),
        "the inserted row is on screen: row={inserted:?} viewport={viewport:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn the_budget_fallback_to_source_ends_when_another_repo_shows_the_same_path(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Two repositories' `README.md` are equal diff targets; the switch between
    // them is still a different file.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let root = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_budget_fallback_repo_switch",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let (first, second) = (root.join("first"), root.join("second"));
    std::fs::create_dir_all(&first).expect("create first workdir");
    std::fs::create_dir_all(&second).expect("create second workdir");
    let path = std::path::PathBuf::from("README.md");
    let (huge_old, huge_new) = inline_overflowing_markdown_diff();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rendered_preview_modes
                    .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                cx.notify();
            });
        });
    });
    let first_id = gitcomet_state::model::RepoId(8817);
    seed_file_diff_state(cx, &view, first_id, &first, &path, &huge_old, &huge_new);
    wait_for_main_pane_condition(
        cx,
        &view,
        "the huge diff falls back to source",
        |pane| {
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown)
                == RenderedPreviewMode::Source
        },
        |pane| pane.diff_markdown.inflight,
    );

    let second_id = gitcomet_state::model::RepoId(8818);
    seed_file_diff_state(cx, &view, second_id, &second, &path, "# one\n", "# two\n");
    wait_for_main_pane_condition(
        cx,
        &view,
        "the second repository is shown",
        |pane| pane.active_repo().map(|repo| repo.id) == Some(second_id),
        |pane| pane.active_repo().map(|repo| repo.id),
    );
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .main_pane
                .read(app)
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "only the other repository's huge file had to be shown as source"
        );
    });

    let _ = std::fs::remove_dir_all(&root);
}

#[gpui::test]
fn split_markdown_diff_says_why_a_side_shows_nothing(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A side missing from the change is not an empty file, and neither is one
    // whose text renders nothing.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let repo_id = gitcomet_state::model::RepoId(8819);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_split_side_notices",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");
    let path = std::path::PathBuf::from("docs/notes.md");
    let cases = [
        (
            Some("# Gone\n\nOld words.\n"),
            None,
            DiffTextRegion::SplitRight,
            "File deleted.",
        ),
        (
            None,
            Some("# Added\n\nNew words.\n"),
            DiffTextRegion::SplitLeft,
            "File added.",
        ),
        (
            Some("# Was\n"),
            Some("[spec]: https://example.com/spec\n"),
            DiffTextRegion::SplitRight,
            "Nothing to render.",
        ),
    ];
    for (rev, (old, new, region, notice)) in (1u64..).zip(cases) {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                let mut repo = opening_repo_state(repo_id, &workdir);
                set_test_file_status(
                    &mut repo,
                    path.clone(),
                    gitcomet_core::domain::FileStatusKind::Modified,
                    gitcomet_core::domain::DiffArea::Unstaged,
                );
                repo.diff_state.diff_file_rev = rev;
                repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(Some(Arc::new(
                    gitcomet_core::domain::FileDiffText::new(
                        path.clone(),
                        old.map(str::to_string),
                        new.map(str::to_string),
                    ),
                )));
                push_test_state(this, app_state_with_repo(repo, repo_id), cx);
                this.main_pane.update(cx, |pane, cx| {
                    pane.rendered_preview_modes
                        .set(RenderedPreviewKind::Markdown, RenderedPreviewMode::Rendered);
                    pane.diff_view = DiffViewMode::Split;
                    cx.notify();
                });
            });
        });
        wait_for_main_pane_condition(
            cx,
            &view,
            "the diff preview is built",
            |pane| {
                pane.diff_markdown.inflight.is_none()
                    && pane.diff_markdown.cache_rev == rev
                    && matches!(
                        pane.diff_markdown.preview,
                        gitcomet_state::model::Loadable::Ready(_)
                    )
            },
            |pane| (pane.diff_markdown.cache_rev, pane.diff_markdown.inflight),
        );
        draw_frames(cx, 2);
        assert!(
            cx.debug_bounds(leaked_selector(format!(
                "markdown_diff_side_notice_{region:?}_{notice}"
            )))
            .is_some(),
            "{region:?} says {notice:?}"
        );
    }

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn opening_a_conflicted_markdown_preview_parses_off_the_render_path(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Three sides of a large file take tens of milliseconds to parse. The frame
    // that opens the preview shows them processing rather than stalling on it.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(8821);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_preview_parse_off_render",
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
                "# Base\n",
                "# Local\n",
                "# Remote\n",
                "<<<<<<< ours\n# Local\n=======\n# Remote\n>>>>>>> theirs\n",
            );
            // The preview parses only once all three sides are loaded in full.
            repo.conflict_state.conflict_file_load_mode =
                gitcomet_state::model::ConflictFileLoadMode::Full;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Preview;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    let sides_ready = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let documents = &view
                .read(app)
                .main_pane
                .read(app)
                .conflict_resolver
                .markdown_preview
                .documents;
            [&documents.base, &documents.ours, &documents.theirs]
                .map(|document| matches!(document, gitcomet_state::model::Loadable::Ready(_)))
        })
    };
    assert_eq!(
        sides_ready(cx),
        [false; 3],
        "the frame that opens the preview parses nothing"
    );

    cx.run_until_parked();
    assert_eq!(sides_ready(cx), [true; 3], "the parse lands afterwards");

    std::fs::remove_dir_all(&workdir).expect("cleanup conflict fixture");
}

#[gpui::test]
fn a_frame_of_the_markdown_preview_installs_pointer_listeners_once_per_document(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Rows and the gaps between blocks used to carry their own click targets,
    // several closures each, rebuilt every frame. One set on the document
    // resolves which row a press is over, so a longer document installs no
    // more of them. The rest of the window installs the same number either way.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));

    let mut frame = |repo_id: u64, name: &str, paragraphs: usize| {
        let source: String = (0..paragraphs)
            .map(|ix| format!("Paragraph {ix} with a [link](https://example.com/{ix}).\n\n"))
            .collect();
        let fixture = RenderedPreviewFixture::open(
            cx,
            &view,
            gitcomet_state::model::RepoId(repo_id),
            name,
            &source,
        );
        draw_frames(cx, 3);
        crate::kit::click::take_click_targets_installed_for_tests();
        crate::view::rows::take_markdown_flow_texts_built_for_tests();
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |_pane, cx| cx.notify());
            });
            let _ = window.draw(app);
        });
        let rows = crate::view::rows::take_markdown_flow_texts_built_for_tests();
        let targets = crate::kit::click::take_click_targets_installed_for_tests();
        fixture.cleanup();
        (rows, targets)
    };
    let (short_rows, short_targets) = frame(8822, "markdown_pointer_listeners_short", 2);
    let (long_rows, long_targets) = frame(8823, "markdown_pointer_listeners_long", 300);
    assert!(
        long_rows > short_rows + 10,
        "the long document draws more rows: {long_rows} vs {short_rows}"
    );
    assert_eq!(
        long_targets, short_targets,
        "{long_rows} rows install as many click targets as {short_rows}"
    );
}

#[gpui::test]
fn a_jump_into_a_long_markdown_preview_comes_to_rest(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Blocks above the viewport that were never drawn are measured out of
    // sight. Measured at another width than the column lays them out at, each
    // frame threw away every height the other had taken and asked for one more.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));
    let source: String = (0..1_200)
        .map(|ix| format!("Paragraph number {ix}.\n\n"))
        .collect();
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8840),
        "markdown_jump_comes_to_rest",
        &source,
    );
    let notified = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let _subscription = cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let notified = std::rc::Rc::clone(&notified);
        app.observe(&main_pane, move |_, _| notified.set(notified.get() + 1))
    });
    let scroll = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.worktree_preview_scroll.0.borrow().base_handle.clone()
    });

    // As a scrollbar drag does: halfway down, past blocks never drawn.
    let max = scroll_handle_max_offset(&scroll).height;
    set_scroll_handle_offset(&scroll, point(px(0.0), -max / 2.0));
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    let frames = settle_preview_frames(cx, &notified, 16);
    assert!(
        frames < 16,
        "the preview still asks for a frame after {frames}: its layout never settles"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_conflicted_markdown_file_in_text_mode_is_not_a_markdown_preview(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The merge tool shows the conflict's text; the rendered markdown diff it
    // replaces is stale. Search, the text hotkeys, and copy must see the text.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_text_mode_surface",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create conflict workdir");
    open_conflict_markdown_preview(
        cx,
        &view,
        gitcomet_state::model::RepoId(8826),
        &workdir,
        ["# Base\n", "# Local\n", "# Remote\n"],
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver.resolver_preview_mode = ConflictResolverPreviewMode::Text;
                cx.notify();
            });
        });
    });
    draw_frames(cx, 2);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_conflict_resolver_active(),
            "the merge tool is showing"
        );
        assert!(
            !pane.is_markdown_preview_active(),
            "the merge tool's text is not a rendered markdown preview"
        );
        assert_eq!(pane.markdown_search_surface(), None);
    });

    let _ = std::fs::remove_dir_all(&workdir);
}
