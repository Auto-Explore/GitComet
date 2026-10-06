//! Selection, copy, drag autoscroll, hit testing, and preview search.

use super::*;

#[gpui::test]
fn secondary_f_from_markdown_file_preview_searches_the_rendered_rows(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
                pane.worktree_markdown.path = Some(abs_path.clone());
                pane.worktree_markdown.source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown.document = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("markdown preview should parse"),
                ));
                pane.worktree_markdown.inflight = None;
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
fn secondary_f_from_conflict_markdown_preview_searches_the_rendered_rows(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn markdown_preview_hitboxes_follow_the_scrolled_viewport(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Rows are only hit-testable near the window. Every other preview test uses
    // a fixture that fits on screen, so nothing else exercises the gate — and a
    // gate reading the wrong coordinate space would reject visible rows and
    // silently stop selection working in any scrolled preview.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn preview_mode_copies_the_document_it_draws(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The counterpart to `source_mode_copies_the_file_exactly_as_written`: the
    // rendered preview copies what it drew, so the heading loses its `#` and
    // the section break under it comes back as the blank line it looks like.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

    cx.update(|window, app| {
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
                pane.diff_text_selection_owner.adopt(window, cx);
                cx.notify();
            });
        });
    });

    let copied = copied_preview_selection(cx, &view).expect("selecting the preview should copy it");
    assert_eq!(copied, "Title\n\nBody paragraph.");

    fixture.cleanup();
}

#[gpui::test]
fn source_mode_selection_highlights_whole_lines_after_a_wrapped_paragraph(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Once a paragraph wraps, row positions run ahead of line numbers. A row
    // that does not wrap must still be measured by its own line, or the
    // highlight stops at another line's width and blank lines get none.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let long = "wrap this sentence over several rows ".repeat(12);
    let source = format!(
        "{long}\n\n| Badge | Git code | Meaning |\n| --- | --- | --- |\n\
         | **Verified** | `G` | Good signature from a trusted key. |\n\
         | **Untrusted key** | `U` | See [Trust a GPG key](#trust-a-gpg-key). |\n\
         \n| **Bad** | `B` | Mismatch. |\nTail."
    );
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(85),
        "markdown_source_wrapped_selection",
        &source,
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

    let lines: Vec<&str> = source.lines().collect();
    let last_ix = lines.len() - 1;
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                assert!(
                    pane.worktree_preview_wrap_active(),
                    "the fixture must be wrapping"
                );
                // From the table header through the line before the last.
                pane.diff_text_anchor = Some(DiffTextPos {
                    source_visible_ix: 2,
                    region: DiffTextRegion::Inline,
                    offset: 0,
                });
                pane.diff_text_head = Some(DiffTextPos {
                    source_visible_ix: last_ix,
                    region: DiffTextRegion::Inline,
                    offset: 0,
                });
                pane.diff_text_selection_owner.adopt(window, cx);
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let rows = pane
            .worktree_preview_visible_len()
            .expect("the list has rows");
        assert!(rows > lines.len(), "the paragraph wraps over several rows");
        for visible_ix in 0..rows {
            let line_ix = pane
                .diff_source_visible_ix_for_visible_ix(visible_ix)
                .expect("every row maps to a line");
            if !(2..last_ix).contains(&line_ix) {
                continue;
            }
            // A blank line has nothing to highlight, with or without wrap.
            let line_len = lines[line_ix].len();
            assert_eq!(
                pane.diff_text_local_selection_range(visible_ix, DiffTextRegion::Inline),
                (line_len > 0).then_some(0..line_len),
                "row {visible_ix} (line {line_ix}: {:?}) is selected end to end",
                lines[line_ix]
            );
        }
    });

    // Select All ends at the end of the last line, whatever row it sits on.
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.select_all_diff_text(window, cx);
            });
        });
    });
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_text_head,
            Some(DiffTextPos {
                source_visible_ix: last_ix,
                region: DiffTextRegion::Inline,
                offset: lines[last_ix].len(),
            }),
            "Select All reaches the end of the file"
        );
    });

    fixture.cleanup();
}

#[gpui::test]
fn source_mode_copies_the_file_exactly_as_written(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The two modes copy different things: the rendered preview copies the
    // document it draws, but Text mode is showing the file itself, so a
    // selection there has to come back byte for byte — every tag, marker, and
    // blank line the author wrote.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    cx.update(|window, app| {
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
                pane.diff_text_selection_owner.adopt(window, cx);
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
fn dragging_across_table_cells_copies_tab_separated_rows(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(93),
        "markdown_table_copy",
        "| Key | Value |\n| --- | --- |\n| one | first |\n| two | second |\n",
    );
    let rows = table_row_ixs(&fixture);
    let start = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_text_box_{}_0",
            rows[0]
        )))
        .expect("header key text");
    let end = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_cell_text_box_{}_1",
            rows[2]
        )))
        .expect("last value text");
    drag_preview_selection(
        cx,
        point(start.left() + px(1.0), start.center().y),
        // Just past the last word, as a drag to the end of a cell ends.
        point(end.right() + px(3.0), end.center().y),
    );

    assert_eq!(
        copied_preview_selection(cx, &view).as_deref(),
        Some("Key\tValue\none\tfirst\ntwo\tsecond"),
        "cells copy as tab-separated values, one line per row"
    );

    fixture.cleanup();
}

#[gpui::test]
fn markdown_preview_selection_highlights_every_line_of_a_wrapped_row(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The highlight is a paint-time computation, so a regression that puts
    // every quad on the first visual line, or steps them by the wrong amount,
    // is invisible to every other assertion in this file.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Inline-code styling owns a run background. If StyledText paints that
    // background after the selection quad, the selected part of the code span
    // looks unselected even though copy and selection geometry are correct.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Selecting a whole row is the easy case: every quad spans its line. A drag
    // that starts and ends mid-line is where the first and last quads have to
    // be measured rather than assumed.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

/// A preview too long for its 600 px window, with the pointer pressed on its
/// first paragraph. Returns the fixture, the preview's scroll handle and where
/// the press landed.
fn press_in_long_preview(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: u64,
    name: &str,
    before_press: impl FnOnce(&mut gpui::VisualTestContext, Bounds<Pixels>),
) -> (
    RenderedPreviewFixture,
    gpui::ScrollHandle,
    gpui::Point<Pixels>,
) {
    cx.simulate_resize(gpui::size(px(900.0), px(600.0)));
    let source: String = (0..300)
        .map(|ix| format!("Paragraph number {ix}.\n\n"))
        .collect();
    let fixture = RenderedPreviewFixture::open(
        cx,
        view,
        gitcomet_state::model::RepoId(repo_id),
        name,
        &source,
    );
    let scroll = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        pane.worktree_preview_scroll.0.borrow().base_handle.clone()
    });
    let first = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{}",
            fixture.row_ix("Paragraph number 0.")
        )))
        .expect("the first paragraph is drawn");
    before_press(cx, first);
    let at = point(first.left() + px(1.0), first.center().y);
    cx.simulate_mouse_move(at, None, Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: at,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    (fixture, scroll, at)
}

/// Let `ticks` autoscroll ticks of 16 ms run.
fn run_autoscroll_ticks(cx: &mut gpui::VisualTestContext, ticks: usize) {
    for _ in 0..ticks {
        cx.executor()
            .advance_clock(std::time::Duration::from_millis(16));
        cx.run_until_parked();
    }
}

#[gpui::test]
fn a_drag_past_the_preview_autoscrolls_after_an_earlier_selection(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A press over an existing selection began a new one without the timer
    // that scrolls it, so after any selection a drag stopped at the pane edge.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let (fixture, scroll, at) = press_in_long_preview(
        cx,
        &view,
        8841,
        "markdown_autoscroll_after_selection",
        |cx, first| {
            // The earlier selection: a double-clicked word.
            simulate_counted_click(cx, point(first.left() + px(20.0), first.center().y), 2);
            cx.update(|_window, app| {
                assert!(
                    view.read(app).main_pane.read(app).diff_text_has_selection(),
                    "the double-click selects a word"
                );
            });
        },
    );
    let under_pane = point(at.x, scroll.bounds().bottom() + px(10.0));
    assert!(under_pane.y < px(600.0), "the point is inside the window");
    cx.simulate_mouse_move(under_pane, Some(MouseButton::Left), Modifiers::default());
    run_autoscroll_ticks(cx, 10);

    assert!(
        scroll.offset().y < px(0.0),
        "held below the pane, the drag scrolls the preview"
    );

    cx.simulate_mouse_up(under_pane, MouseButton::Left, Modifiers::default());
    fixture.cleanup();
}

#[gpui::test]
fn a_drag_held_outside_the_window_autoscrolls_by_the_pointers_distance(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Past the window edge only the drag's own window-wide listener sees the
    // pointer. Each tick replaced that with the last point the root view saw
    // inside the window, so the drag slowed to a crawl and the selection
    // snapped back to that point.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let (fixture, scroll, at) = press_in_long_preview(
        cx,
        &view,
        8842,
        "markdown_autoscroll_outside_window",
        |_, _| {},
    );
    // Out through the strip of window under the pane, as a real pointer goes.
    let under_pane = point(at.x, scroll.bounds().bottom() + px(10.0));
    assert!(under_pane.y < px(600.0), "the point is inside the window");
    cx.simulate_mouse_move(under_pane, Some(MouseButton::Left), Modifiers::default());
    let outside = point(at.x, px(900.0));
    cx.simulate_mouse_move(outside, Some(MouseButton::Left), Modifiers::default());
    let before = scroll.offset().y;
    run_autoscroll_ticks(cx, 5);

    // A pointer that far out scrolls at the 48 px cap; ten pixels under the
    // pane, 4 px a tick.
    let moved = before - scroll.offset().y;
    assert!(
        moved >= px(200.0),
        "five ticks with the pointer 300 px past the pane moved {moved:?}"
    );

    cx.simulate_mouse_up(outside, MouseButton::Left, Modifiers::default());
    fixture.cleanup();
}

#[gpui::test]
fn an_inter_block_gap_starts_markdown_selection_upward_and_downward(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        path.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );
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
    // The flowing split draws the gap before each band; the heading's band is
    // the one whose gap separates it from the paragraph above.
    let heading_band = preview
        .bands
        .iter()
        .position(|band| band.rows.contains(&middle_ix))
        .expect("the heading has a band");
    let band_start = preview.bands[heading_band].rows.start;
    let below_band_start = preview
        .bands
        .iter()
        .find(|band| band.rows.contains(&below_ix))
        .expect("the last paragraph has a band")
        .rows
        .start;

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
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview =
                    gitcomet_state::model::Loadable::Ready(Arc::new(preview));
                pane.diff_markdown.inflight = None;
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
            "markdown_preview_block_gap_{}",
            heading_band * 2
        )))
        .expect("interactive gap in the old split column");
    let right_gap = cx
        .debug_bounds(leaked_selector(format!(
            "markdown_preview_block_gap_{}",
            heading_band * 2 + 1
        )))
        .expect("interactive gap in the new split column");
    assert!(
        left_gap.right() <= right_gap.left(),
        "each gap must remain inside its own split column: left={left_gap:?} right={right_gap:?}"
    );

    cx.simulate_click(right_gap.center(), Modifiers::default());
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let boundary = DiffTextPos {
            source_visible_ix: band_start,
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
            source_visible_ix: band_start,
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
        // Below the paragraph is the gap before the list's band, which opens
        // at the list: the same boundary, seen from the other side.
        let boundary = DiffTextPos {
            source_visible_ix: list_ix,
            region: DiffTextRegion::SplitLeft,
            offset: 0,
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
        // As above: the gap below the list opens the next band.
        let boundary = DiffTextPos {
            source_visible_ix: below_band_start,
            region: DiffTextRegion::SplitLeft,
            offset: 0,
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A code block sizes every line to its own text so the block has something
    // to scroll, which leaves the space beside a short line belonging to no
    // row at all. Hit testing used to refuse any point outside a row, so a drag
    // that crossed one of those gaps stopped extending the selection and the
    // reader was left with whatever they had already covered.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn markdown_below_eof_drag_selects_a_thematic_break_only_document(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn markdown_below_eof_resolves_after_a_trailing_thematic_break(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

#[gpui::test]
fn markdown_preview_hit_testing_follows_a_row_onto_its_wrapped_lines(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A flowing row covers several visual lines, so a click has to resolve in
    // two dimensions. Reading only the x offset along one shaped line put the
    // caret near the start of the row wherever the reader clicked low and left.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
fn dragging_the_split_markdown_preview_divider_resizes_its_columns(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The rendered split drew its two halves 50/50 with a plain line between
    // them: nothing to drag, unlike the text split beside it.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1400.0), px(700.0)));
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(8845),
        "markdown_split_resize",
        "Intro.\n\nOld paragraph.\n",
        "Intro.\n\nNew paragraph.\n",
        DiffViewMode::Split,
    );
    let intro = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        row_ix_with_text(&preview.old, "Intro.")
    });
    let left_width = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            view.read(app)
                .main_pane
                .read(app)
                .diff_text_hitbox_bounds_for_tests(intro, DiffTextRegion::SplitLeft)
                .expect("the old side's first row is drawn")
                .size
                .width
        })
    };
    assert!(
        cx.debug_bounds("markdown_split_resize_handle_header")
            .is_some(),
        "the column header carries the divider too"
    );
    let handle = cx
        .debug_bounds("markdown_split_resize_handle_body")
        .expect("the split preview mounts a resize handle on its divider");
    let before = left_width(cx);

    let from = handle.center();
    let to = point(from.x + px(150.0), from.y);
    cx.simulate_mouse_move(from, None, Modifiers::default());
    cx.simulate_mouse_down(from, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        point(from.x + px(10.0), from.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    cx.simulate_mouse_move(to, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(to, MouseButton::Left, Modifiers::default());
    draw_and_drain_test_window(cx);

    let ratio = cx.update(|_window, app| view.read(app).main_pane.read(app).diff_split_ratio);
    assert!(ratio > 0.55, "the drag moves the split, got ratio {ratio}");
    let grown = left_width(cx) - before;
    assert!(
        (grown - px(150.0)).abs() <= px(2.0),
        "the old side widens by as much as the divider moved, got {grown:?}"
    );
    let moved = cx
        .debug_bounds("markdown_split_resize_handle_body")
        .expect("the handle is still drawn");
    assert!(
        (moved.center().x - to.x).abs() <= px(2.0),
        "the divider follows the pointer: {moved:?} vs {to:?}"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown split resize fixture");
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
            pane.markdown_interaction.reveal.pending(),
            None,
            "the reveal should be claimed once so it stops fighting later scrolling"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup markdown preview scroll fixture");
}

/// The rendered markdown *diff* is the other in-place search surface. It flows
/// like the file preview, so the match is revealed once its row is laid out.
#[gpui::test]
fn markdown_diff_preview_search_scrolls_the_list_to_the_match(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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
    let target = gitcomet_core::domain::DiffTarget::working_tree(
        file_rel.clone(),
        gitcomet_core::domain::DiffArea::Unstaged,
    );

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
                pane.diff_markdown.cache_repo_id = Some(repo_id);
                pane.diff_markdown.cache_rev = 1;
                pane.diff_markdown.cache_target = Some(target.clone());
                pane.diff_markdown.preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::build_markdown_diff_preview(
                        &old_text, &new_text,
                    )
                    .expect("markdown diff preview should parse"),
                ));
                pane.diff_markdown.inflight = None;
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
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
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

#[gpui::test]
fn copying_across_alignment_padding_adds_no_blank_lines(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // The old side of this split diff is [a, Spacer, Spacer, b]; each spacer
    // yields `Some(0..0)` from `diff_text_source_selection_range`, so
    // `selected_diff_text_string` writes an empty line for it.
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(900.0), px(700.0)));
    let workdir = open_rendered_markdown_diff_in(
        cx,
        &view,
        gitcomet_state::model::RepoId(8807),
        "markdown_split_copy_padding",
        "- a\n- b\n",
        "- a\n- x\n- y\n- b\n",
        DiffViewMode::Split,
    );
    let (a_ix, b_ix) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let gitcomet_state::model::Loadable::Ready(preview) = &pane.diff_markdown.preview else {
            panic!("the preview is ready");
        };
        (
            row_ix_with_text(&preview.old, "a"),
            row_ix_with_text(&preview.old, "b"),
        )
    });
    let (from, to) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let a = pane
            .diff_text_hitbox_bounds_for_tests(a_ix, DiffTextRegion::SplitLeft)
            .expect("a is drawn");
        let b = pane
            .diff_text_hitbox_bounds_for_tests(b_ix, DiffTextRegion::SplitLeft)
            .expect("b is drawn");
        (
            point(a.left(), a.center().y),
            point(b.right(), b.center().y),
        )
    });
    drag_preview_selection(cx, from, to);
    let copied = copied_preview_selection(cx, &view).expect("the drag selected text");
    assert_eq!(copied, "a\nb");
    std::fs::remove_dir_all(&workdir).expect("cleanup");
}

#[gpui::test]
fn redrawing_under_an_unchanged_search_does_not_rebuild_the_matcher(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(8813),
        "markdown_search_matcher_reuse",
        "Alpha paragraph.\n\nBeta paragraph.\n",
    );
    focus_diff_panel(cx, &view);
    cx.simulate_keystrokes("secondary-f");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = "paragraph".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("paragraph", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
    });
    draw_frames(cx, 1);

    crate::view::panes::main::diff_search::take_search_matchers_built_for_tests();
    for _ in 0..3 {
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }
    let built = crate::view::panes::main::diff_search::take_search_matchers_built_for_tests();
    assert_eq!(
        built, 0,
        "each frame builds a new search matcher (a regex compile for regex queries) although \
         the query did not change"
    );

    fixture.cleanup();
}

#[gpui::test]
fn searching_the_merge_tool_preview_scrolls_the_column_holding_the_match(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1400.0), px(800.0)));
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_preview_search_reveal",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create conflict workdir");

    // One unique paragraph far below the fold of the Local column.
    let mut ours: String = (0..300)
        .map(|ix| format!("paragraph {ix:03}\n\n"))
        .collect();
    ours.push_str("the needle paragraph\n");
    open_conflict_markdown_preview(
        cx,
        &view,
        gitcomet_state::model::RepoId(8825),
        &workdir,
        ["# Base\n", &ours, "# Remote\n"],
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.markdown_search_surface(),
            Some(MarkdownSearchSurface::Conflict)
        );
        assert_eq!(
            uniform_list_offset(&pane.conflict_preview_ours_scroll).y,
            px(0.0)
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
        assert_eq!(pane.diff_search_matches.len(), 1);
        assert!(
            uniform_list_offset(&pane.conflict_preview_ours_scroll).y < px(0.0),
            "the Local column scrolls down to the match"
        );
        assert_eq!(
            pane.conflict_resolver
                .markdown_preview
                .columns
                .ours
                .reveal
                .pending(),
            None,
            "the reveal is claimed once"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn aligned_rows_are_highlighted_where_their_lines_are_painted(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Selection geometry must include inherited alignment before glyphs paint.
    // Cover wrapped centred text and short centred/right-aligned lines.
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let long = "select this centred paragraph across its lines ".repeat(30);
    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(140),
        "markdown_aligned_selection",
        &format!(
            "<p align=\"center\">{long}</p>\n\n<p align=\"center\">short</p>\n\n<p align=\"right\">tail</p>\n"
        ),
    );

    let mut highlighted = |row_ix: usize| {
        let text = cx
            .debug_bounds(leaked_selector(format!(
                "markdown_preview_text_box_{row_ix}"
            )))
            .expect("expected the row's text box");
        simulate_counted_click(cx, text.center(), 3);
        cx.run_until_parked();
        crate::view::rows::clear_markdown_selection_paint_log_for_tests();
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
        (
            text,
            crate::view::rows::markdown_selection_paint_log_for_tests(row_ix),
        )
    };
    let gaps = |text: Bounds<Pixels>, rect: Bounds<Pixels>| {
        (rect.left() - text.left(), text.right() - rect.right())
    };

    let (text, rects) = highlighted(fixture.row_ix(long.trim_end()));
    assert!(rects.len() >= 3, "the paragraph wraps: {rects:?}");
    // A line is centred on its words; the space it wrapped at hangs past its
    // right edge, so a selected line ends one space beyond the centred span.
    let (last, wrapped) = rects.split_last().expect("rects");
    let (left, right) = gaps(text, *last);
    assert!(
        (left - right).abs() <= px(1.0) && left > px(0.0),
        "the last line is highlighted centred: rect={last:?} text={text:?}"
    );
    let space = {
        let (left, right) = gaps(text, wrapped[0]);
        left - right
    };
    assert!(
        space > px(1.0) && space < px(20.0),
        "a wrapped line overhangs by one space: {space:?}"
    );
    for rect in wrapped {
        let (left, right) = gaps(text, *rect);
        assert!(
            (left - right - space).abs() <= px(1.0) && left > px(0.0),
            "every wrapped line is centred on its words: rect={rect:?} text={text:?}"
        );
    }

    let (text, rects) = highlighted(fixture.row_ix("short"));
    let [rect] = rects.as_slice() else {
        panic!("one line, one quad: {rects:?}");
    };
    let (left, right) = gaps(text, *rect);
    assert!(
        (left - right).abs() <= px(1.0) && left > text.size.width * 0.25,
        "a short centred line is highlighted in the middle: rect={rect:?} text={text:?}"
    );

    let (text, rects) = highlighted(fixture.row_ix("tail"));
    let [rect] = rects.as_slice() else {
        panic!("one line, one quad: {rects:?}");
    };
    assert!(
        (text.right() - rect.right()).abs() <= px(1.0) && rect.left() > text.center().x,
        "a right-aligned line is highlighted at the right: rect={rect:?} text={text:?}"
    );

    fixture.cleanup();
}

#[gpui::test]
fn a_double_click_on_an_aligned_word_selects_the_word_painted_there(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let fixture = RenderedPreviewFixture::open(
        cx,
        &view,
        gitcomet_state::model::RepoId(141),
        "markdown_aligned_double_click",
        "<p align=\"center\">alpha beta gamma</p>\n\n<p align=\"right\">one two</p>\n",
    );
    let text_box = |cx: &mut gpui::VisualTestContext, row_ix: usize| {
        cx.debug_bounds(leaked_selector(format!(
            "markdown_preview_text_box_{row_ix}"
        )))
        .expect("expected the row's text box")
    };

    // The middle word is painted at the middle of the row.
    let centred = text_box(cx, fixture.row_ix("alpha beta gamma"));
    simulate_counted_click(cx, centred.center(), 2);
    cx.run_until_parked();
    assert_eq!(copied_preview_selection(cx, &view).as_deref(), Some("beta"));

    // The last word is painted against the right edge, which the pane's
    // scrollbar overlays, so the click lands just inside it.
    let right = text_box(cx, fixture.row_ix("one two"));
    simulate_counted_click(cx, point(right.right() - px(12.0), right.center().y), 2);
    cx.run_until_parked();
    assert_eq!(copied_preview_selection(cx, &view).as_deref(), Some("two"));

    fixture.cleanup();
}
