use super::*;

#[gpui::test]
fn a_document_as_long_as_the_parser_allows_renders_as_a_preview(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // A frame builds the blocks near the viewport whatever the document's
    // length, so the preview's only limit is the parser's own.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
fn markdown_diff_preview_over_limit_shows_fallback_instead_of_rendering(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
        let pane = view.read(app).diff_renderer.read(app);
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
fn the_budget_fallback_to_source_ends_with_the_file_that_needed_it(cx: &mut gpui::TestAppContext) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
                .diff_renderer
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
fn the_budget_fallback_to_source_ends_when_another_repo_shows_the_same_path(
    cx: &mut gpui::TestAppContext,
) {
    gitcomet_ui_kit::test_support::use_real_text_backend(cx);
    // Two repositories' `README.md` are equal diff targets; the switch between
    // them is still a different file.
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
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
            this.diff_renderer.update(cx, |pane, cx| {
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
                .diff_renderer
                .read(app)
                .rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "only the other repository's huge file had to be shown as source"
        );
    });

    let _ = std::fs::remove_dir_all(&root);
}
