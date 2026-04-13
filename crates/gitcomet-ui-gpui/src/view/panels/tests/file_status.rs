use super::*;

#[gpui::test]
fn patch_diff_search_query_keeps_stable_style_cache_entries(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(22);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_patch_search",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let target = gitcomet_core::domain::DiffTarget::Commit {
                commit_id: gitcomet_core::domain::CommitId("feedface".into()),
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
        window.refresh();
        let _ = window.draw(app);
    });

    let mut stable_highlights_hash_before = 0u64;
    let mut stable_text_hash_before = 0u64;
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let stable = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|entry| entry.as_ref().map(|entry| &entry.styled))
            .expect("expected stable cache entry for context row before search");
        assert!(
            pane.diff_text_query_segments_cache.is_empty(),
            "query overlay cache should start empty"
        );
        stable_highlights_hash_before = stable.highlights_hash;
        stable_text_hash_before = stable.text_hash;
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_search_active = true;
            pane.diff_search_input.update(cx, |input, cx| {
                input.set_text("main", cx);
            });
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.untracked_height = Some(px(263.5));
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.untracked_height = Some(px(263.5));
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);

        let stable_after = pane
            .diff_text_segments_cache
            .get(2)
            .and_then(|entry| entry.as_ref().map(|entry| &entry.styled))
            .expect("expected stable cache entry for context row after search query update");
        assert_eq!(
            stable_after.highlights_hash, stable_highlights_hash_before,
            "search query updates should not rewrite stable style highlights"
        );
        assert_eq!(
            stable_after.text_hash, stable_text_hash_before,
            "search query updates should not rewrite stable styled text"
        );

        assert_eq!(pane.diff_text_query_cache_query.as_ref(), "main");
        let query_overlay = pane
            .diff_text_query_segments_cache
            .get(2)
            .and_then(|entry| entry.as_ref().map(|entry| &entry.styled))
            .expect("expected query overlay cache entry for searched context row");
        assert_ne!(
            query_overlay.highlights_hash, stable_after.highlights_hash,
            "query overlay should layer match highlighting on top of stable highlights"
        );
    });
}

#[gpui::test]
fn worktree_preview_search_query_clears_row_cache_without_dropping_source_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(23);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_search",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let lines: Arc<Vec<String>> = Arc::new(vec![
        "fn needle() { let value = 1; }".to_string(),
        "fn keep() { let other = 2; }".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(&preview_abs_path, &preview_text).expect("write preview fixture file");

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

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree preview row cache before enabling search",
        |pane| {
            pane.worktree_preview_segments_cache_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_segments_cache_get(0).is_some()
        },
        |pane| {
            format!(
                "preview_path={:?} cache_path={:?} row_cache_present={} line_count={:?}",
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_segments_cache_path.clone(),
                pane.worktree_preview_segments_cache_get(0).is_some(),
                pane.worktree_preview_line_count(),
            )
        },
    );

    let mut base_highlights_hash = 0u64;
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        assert_eq!(
            pane.worktree_preview_segments_cache_path.as_ref(),
            Some(&preview_abs_path),
            "initial draw should bind the preview row cache to the current path"
        );
        let base = pane
            .worktree_preview_segments_cache_get(0)
            .expect("expected worktree preview row cache before enabling search");
        base_highlights_hash = base.highlights_hash;
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        main_pane.update(app, |pane, cx| {
            pane.diff_search_active = true;
            pane.diff_search_input.update(cx, |input, cx| {
                input.set_text("needle", cx);
            });
            cx.notify();
        });
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "needle");
        assert_eq!(
            pane.worktree_preview_segments_cache_path.as_ref(),
            Some(&preview_abs_path),
            "search query changes should preserve the bound preview source path"
        );
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let searched = pane
            .worktree_preview_segments_cache_get(0)
            .expect("expected worktree preview row cache after search query rebuild");
        assert_ne!(
            searched.highlights_hash, base_highlights_hash,
            "search overlay should change the cached preview row highlights"
        );
        assert!(
            searched
                .highlights
                .iter()
                .any(|(_, style)| style.background_color.is_some()),
            "searched preview row should include a query highlight background"
        );
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn worktree_preview_identical_refresh_preserves_row_cache(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(24);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_refresh_preserves_cache",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("preview_refresh.rs");
    let preview_abs_path = workdir.join(&file_rel);
    let lines: Arc<Vec<String>> = Arc::new(vec![
        "fn keep() { let value = 1; }".to_string(),
        "fn also_keep() { let other = 2; }".to_string(),
    ]);
    let preview_text = lines.join("\n");

    let _ = std::fs::create_dir_all(&workdir);
    std::fs::write(&preview_abs_path, &preview_text).expect("write preview fixture file");

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

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                pane.set_full_document_syntax_budget_override_for_tests(rows::DiffSyntaxBudget {
                    foreground_parse: std::time::Duration::from_secs(1),
                });
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "worktree preview row cache before identical refresh",
        |pane| {
            pane.worktree_preview_segments_cache_path.as_ref() == Some(&preview_abs_path)
                && pane.worktree_preview_segments_cache_get(0).is_some()
        },
        |pane| {
            format!(
                "preview_path={:?} cache_path={:?} prepared_document={:?} row_cache_present={} style_epoch={}",
                pane.worktree_preview_path.clone(),
                pane.worktree_preview_segments_cache_path.clone(),
                pane.worktree_preview_prepared_syntax_document(),
                pane.worktree_preview_segments_cache_get(0).is_some(),
                pane.worktree_preview_style_cache_epoch,
            )
        },
    );

    let mut base_highlights_hash = 0u64;
    let mut base_style_epoch = 0u64;
    let mut base_prepared_syntax_ready = false;
    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let base = pane
            .worktree_preview_segments_cache_get(0)
            .expect("expected worktree preview row cache before identical refresh");
        base_highlights_hash = base.highlights_hash;
        base_style_epoch = pane.worktree_preview_style_cache_epoch;
        base_prepared_syntax_ready = pane.worktree_preview_prepared_syntax_document().is_some();
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let lines = Arc::clone(&lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    lines,
                    preview_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        let refreshed = pane
            .worktree_preview_segments_cache_get(0)
            .expect("identical refresh should preserve the cached preview row");
        assert_eq!(
            pane.worktree_preview_segments_cache_path.as_ref(),
            Some(&preview_abs_path),
            "identical refresh should keep the preview cache bound to the current source"
        );
        if base_prepared_syntax_ready {
            assert_eq!(
                pane.worktree_preview_style_cache_epoch, base_style_epoch,
                "identical refresh should not bump the preview syntax/style epoch once syntax is already ready"
            );
            assert_eq!(
                refreshed.highlights_hash, base_highlights_hash,
                "identical refresh should preserve the existing cached row styling once syntax is already ready"
            );
        } else if pane.worktree_preview_style_cache_epoch == base_style_epoch {
            assert_eq!(
                refreshed.highlights_hash, base_highlights_hash,
                "identical refresh should preserve the fallback cached row styling while syntax is still pending"
            );
        }
    });

    // Phase 2: refresh with different content — cache must be invalidated.
    let changed_lines: Arc<Vec<String>> = Arc::new(vec![
        "fn changed() { let x = 99; }".to_string(),
        "fn also_changed() { let y = 100; }".to_string(),
    ]);
    let changed_text = changed_lines.join("\n");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let changed_lines = Arc::clone(&changed_lines);
            let preview_abs_path = preview_abs_path.clone();
            this.main_pane.update(cx, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    preview_abs_path.clone(),
                    changed_lines,
                    changed_text.len(),
                    cx,
                );
            });
        });
    });

    cx.update(|_window, app| {
        let main_pane = view.read(app).main_pane.clone();
        let pane = main_pane.read(app);
        assert_ne!(
            pane.worktree_preview_style_cache_epoch, base_style_epoch,
            "changed source should bump the preview syntax/style epoch"
        );
        if let Some(refreshed) = pane.worktree_preview_segments_cache_get(0) {
            assert_eq!(
                refreshed.text.as_ref(),
                changed_lines[0].as_str(),
                "changed source may repopulate the cache immediately, but it must render the new preview contents"
            );
            assert_ne!(
                refreshed.highlights_hash, base_highlights_hash,
                "changed source must not retain the old cached preview styling"
            );
        }
    });

    let _ = std::fs::remove_dir_all(&workdir);
}

#[gpui::test]
fn staged_deleted_file_preview_uses_old_contents(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(3);
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_deleted", std::process::id()));
    let file_rel = std::path::PathBuf::from("deleted.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);

            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Deleted,
                gitcomet_core::domain::DiffArea::Staged,
            );
            let preview_source_path = workdir.join(".deleted_preview_source.txt");
            let _ = std::fs::create_dir_all(&workdir);
            std::fs::write(&preview_source_path, "one\ntwo\n")
                .expect("write staged deleted preview source");
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Error(
                "materialized diff_file should not be consulted for deleted preview".into(),
            );
            repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: preview_source_path,
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
        "staged deleted preview loads from preview text file",
        |pane| {
            pane.worktree_preview_path.as_ref() == Some(&workdir.join(&file_rel))
                && pane.worktree_preview_source_path.as_ref()
                    == Some(&workdir.join(".deleted_preview_source.txt"))
                && matches!(
                    pane.worktree_preview,
                    gitcomet_state::model::Loadable::Ready(3)
                )
                && pane.worktree_preview_text.is_empty()
        },
        |pane| {
            format!(
                "preview_path={:?} source_path={:?} preview={:?} text_len={} line_count={:?}",
                pane.worktree_preview_path,
                pane.worktree_preview_source_path,
                pane.worktree_preview,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_count(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.deleted_file_preview_abs_path(),
            Some(workdir.join(&file_rel))
        );
        assert!(
            matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Ready(_)
            ),
            "expected worktree preview to be ready"
        );
        assert_eq!(pane.worktree_preview_line_count(), Some(3));
        assert_eq!(pane.worktree_preview_line_text(0).as_deref(), Some("one"));
        assert_eq!(pane.worktree_preview_line_text(1).as_deref(), Some("two"));
        assert_eq!(pane.worktree_preview_line_text(2).as_deref(), Some(""));
    });
}

#[gpui::test]
fn committed_deleted_file_preview_uses_preview_text_file_without_patch_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(303);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_committed_deleted",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("report.json");
    let commit_id = gitcomet_core::domain::CommitId("deadbeef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::Commit {
                commit_id: commit_id.clone(),
                path: Some(file_rel.clone()),
            });
            repo.diff_state.diff = gitcomet_state::model::Loadable::Error(
                "parsed patch diff should not be consulted for deleted file preview".into(),
            );
            let preview_source_path = workdir.join(".committed_deleted_preview_source.json");
            let _ = std::fs::create_dir_all(&workdir);
            std::fs::write(&preview_source_path, "{\"removed\":true}\n")
                .expect("write committed deleted preview source");
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Error(
                "materialized diff_file should not be consulted for committed deleted preview"
                    .into(),
            );
            repo.diff_state.diff_preview_text_file = gitcomet_state::model::Loadable::Ready(Some(
                Arc::new(gitcomet_core::domain::DiffPreviewTextFile {
                    path: preview_source_path,
                    side: gitcomet_core::domain::DiffPreviewTextSide::Old,
                }),
            ));
            repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "remove report".to_string(),
                    committed_at: "2026-04-07T12:00:00Z".to_string(),
                    parent_ids: vec![],
                    files: vec![gitcomet_core::domain::CommitFileChange {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Deleted,
                    }],
                },
            ));
            repo.history_state.commit_details_rev =
                repo.history_state.commit_details_rev.wrapping_add(1);

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
        "committed deleted preview loads from preview text file",
        |pane| {
            pane.worktree_preview_path.as_ref() == Some(&workdir.join(&file_rel))
                && pane.worktree_preview_source_path.as_ref()
                    == Some(&workdir.join(".committed_deleted_preview_source.json"))
                && matches!(
                    pane.worktree_preview,
                    gitcomet_state::model::Loadable::Ready(2)
                )
                && pane.worktree_preview_text.is_empty()
        },
        |pane| {
            format!(
                "preview_path={:?} source_path={:?} preview={:?} text_len={} line_count={:?}",
                pane.worktree_preview_path,
                pane.worktree_preview_source_path,
                pane.worktree_preview,
                pane.worktree_preview_text.len(),
                pane.worktree_preview_line_count(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.deleted_file_preview_abs_path(),
            Some(workdir.join(&file_rel))
        );
        assert!(matches!(
            pane.worktree_preview,
            gitcomet_state::model::Loadable::Ready(_)
        ));
        assert_eq!(pane.worktree_preview_line_count(), Some(2));
        assert_eq!(
            pane.worktree_preview_line_text(0).as_deref(),
            Some("{\"removed\":true}")
        );
        assert_eq!(pane.worktree_preview_line_text(1).as_deref(), Some(""));
    });
}

#[gpui::test]
fn untracked_markdown_file_preview_defaults_to_preview_mode_and_renders_container(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(59);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_untracked_default_preview",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");
    let abs_path = workdir.join(&file_rel);
    let source = "# Preview title\n\n- first item\n- second item\n";
    let preview_lines = Arc::new(source.lines().map(ToOwned::to_owned).collect::<Vec<_>>());

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create untracked markdown workdir");
    std::fs::write(&abs_path, source).expect("write untracked markdown fixture");

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

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

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
                pane.worktree_markdown_preview_path = Some(abs_path.clone());
                pane.worktree_markdown_preview_source_rev = pane.worktree_preview_content_rev;
                pane.worktree_markdown_preview = gitcomet_state::model::Loadable::Ready(Arc::new(
                    crate::view::markdown_preview::parse_markdown(source)
                        .expect("untracked markdown preview should parse"),
                ));
                pane.worktree_markdown_preview_inflight = None;
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    wait_for_main_pane_condition(
        cx,
        &view,
        "untracked markdown preview activation",
        |pane| pane.is_file_preview_active() && pane.is_markdown_preview_active(),
        |pane| {
            format!(
                "active_repo={:?} diff_target={:?} is_file_preview_active={} is_markdown_preview_active={}",
                pane.active_repo().map(|repo| repo.id),
                pane.active_repo()
                    .and_then(|repo| repo.diff_state.diff_target.clone()),
                pane.is_file_preview_active(),
                pane.is_markdown_preview_active(),
            )
        },
    );

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(pane.is_file_preview_active());
        assert!(pane.is_markdown_preview_active());
        assert_eq!(
            pane.rendered_preview_modes
                .get(RenderedPreviewKind::Markdown),
            RenderedPreviewMode::Rendered,
            "expected untracked markdown preview to default to Preview mode"
        );
    });
    assert!(
        cx.debug_bounds("markdown_diff_view_toggle").is_some(),
        "expected markdown Preview/Text toggle for untracked markdown preview"
    );
    assert!(
        cx.debug_bounds("worktree_markdown_preview_scroll_container")
            .is_some(),
        "expected rendered markdown preview container for untracked markdown preview"
    );

    std::fs::remove_dir_all(&workdir).expect("cleanup untracked markdown preview fixture");
}

#[gpui::test]
fn staged_added_markdown_file_preview_shows_preview_text_toggle(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(57);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_added_toggle",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");

    assert_markdown_file_preview_toggle_visible(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Added,
        None,
        Some("# Added markdown\n\nnew body\n"),
        true,
    );
}

#[gpui::test]
fn staged_deleted_markdown_file_preview_shows_preview_text_toggle(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(58);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_markdown_deleted_toggle",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("notes.md");

    assert_markdown_file_preview_toggle_visible(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Deleted,
        Some("# Deleted markdown\n\nold body\n"),
        None,
        false,
    );
}

#[gpui::test]
fn unstaged_deleted_gitlink_preview_does_not_stay_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_gitlink",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let unified = format!(
        "diff --git a/{0} b/{0}\nindex 1234567..0000000 160000\n--- a/{0}\n+++ /dev/null\n@@ -1 +0,0 @@\n-Subproject commit c35be02cd52b18c7b2894dc570825b43c94130ed\n",
        file_rel.display()
    );
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Deleted,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(None);

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "unstaged gitlink-like deleted target should not remain stuck in File Loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup unstaged gitlink fixture");
}

#[gpui::test]
fn unstaged_modified_gitlink_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(45);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_gitlink_mod",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create gitlink-like directory");

    let target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: file_rel.clone(),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let unified = format!(
        "diff --git a/{0} b/{0}\nindex 1234567..89abcde 160000\n--- a/{0}\n+++ b/{0}\n@@ -1 +1 @@\n-Subproject commit 1234567890123456789012345678901234567890\n+Subproject commit 89abcdef0123456789abcdef0123456789abcdef\n",
        file_rel.display()
    );
    let diff = gitcomet_core::domain::Diff::from_unified(target.clone(), &unified);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(target);
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(diff));
            repo.diff_state.diff_file = gitcomet_state::model::Loadable::Ready(None);

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_worktree_target_directory(),
            "gitlink-like target should be treated as directory-backed for unified diff mode"
        );
        assert!(
            !pane.is_file_preview_active(),
            "unstaged modified gitlink target should bypass file preview mode"
        );
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "unstaged modified gitlink target should not show stuck File Loading state"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup unstaged gitlink modified fixture");
}

#[gpui::test]
fn ensure_preview_loading_does_not_reenter_loading_from_error_for_same_path(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let temp = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_loading_error",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&temp);
    std::fs::create_dir_all(&temp).expect("create temp directory");
    let path_a = temp.join("a.txt");
    let path_b = temp.join("b.txt");
    std::fs::write(&path_a, "a\n").expect("write a.txt");
    std::fs::write(&path_b, "b\n").expect("write b.txt");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(path_a.clone());
                pane.worktree_preview = gitcomet_state::model::Loadable::Error("boom".into());

                // Same path: keep showing the existing error, do not bounce back to Loading.
                pane.ensure_preview_loading(path_a.clone());
                assert!(
                    matches!(
                        pane.worktree_preview,
                        gitcomet_state::model::Loadable::Error(_)
                    ),
                    "same-path retry should not reset Error to Loading"
                );

                // Different path: loading the newly selected file is expected.
                pane.ensure_preview_loading(path_b.clone());
                assert_eq!(pane.worktree_preview_path, Some(path_b.clone()));
                assert!(
                    matches!(
                        pane.worktree_preview,
                        gitcomet_state::model::Loadable::Loading
                    ),
                    "new path selection should enter Loading"
                );
            });
        });
    });

    std::fs::remove_dir_all(&temp).expect("cleanup temp directory");
}

#[gpui::test]
fn switching_diff_target_clears_stale_worktree_preview_loading(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(36);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_switch_preview_target",
        std::process::id()
    ));
    let file_a = std::path::PathBuf::from("a.txt");
    let file_b = std::path::PathBuf::from("b.txt");

    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

    let make_state = |target_path: std::path::PathBuf, diff_state_rev: u64| {
        Arc::new(AppState {
            repos: vec![{
                let mut repo = opening_repo_state(repo_id, &workdir);
                repo.status = gitcomet_state::model::Loadable::Ready(
                    gitcomet_core::domain::RepoStatus {
                        staged: vec![],
                        unstaged: vec![
                            gitcomet_core::domain::FileStatus {
                                path: file_a.clone(),
                                kind: gitcomet_core::domain::FileStatusKind::Untracked,
                                conflict: None,
                            },
                            gitcomet_core::domain::FileStatus {
                                path: file_b.clone(),
                                kind: gitcomet_core::domain::FileStatusKind::Untracked,
                                conflict: None,
                            },
                        ],
                    }
                    .into(),
                );
                repo.diff_state.diff_target =
                    Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                        path: target_path,
                        area: gitcomet_core::domain::DiffArea::Unstaged,
                    });
                repo.diff_state.diff_state_rev = diff_state_rev;
                repo
            }],
            active_repo: Some(repo_id),
            ..Default::default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let first = make_state(file_a.clone(), 1);
            push_test_state(this, first, cx);
            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_a));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let second = make_state(file_b.clone(), 2);
            push_test_state(this, second, cx);
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let stale_path = workdir.join(&file_a);
        let is_stale_loading =
            matches!(pane.worktree_preview, gitcomet_state::model::Loadable::Loading)
                && pane.worktree_preview_path.as_ref() == Some(&stale_path);
        assert!(
            !is_stale_loading,
            "switching selected file should not keep stale Loading on previous path; state={:?} path={:?}",
            pane.worktree_preview,
            pane.worktree_preview_path
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup workdir");
}

#[gpui::test]
fn staged_directory_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(34);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_staged_dir",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("subproject");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create staged directory path");

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
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_worktree_target_directory(),
            "expected staged directory target detection for gitlink-like entries"
        );
        assert!(
            !pane.is_file_preview_active(),
            "directory targets should avoid file preview mode to show unified subproject diffs"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup staged directory fixture");
}

#[gpui::test]
fn staged_added_missing_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(43);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_staged_added_missing",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("subproject");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(&workdir).expect("create workdir");

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
        let pane = view.read(app).main_pane.read(app);
        assert!(
            !pane.is_file_preview_active(),
            "staged Added targets that are not real files should bypass file preview to avoid stuck loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup staged-added-missing fixture");
}

#[gpui::test]
fn untracked_directory_target_uses_unified_diff_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(35);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_dir",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("subproject");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create untracked directory path");

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

            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.is_worktree_target_directory(),
            "expected untracked directory target detection for gitlink-like entries"
        );
        assert!(
            !pane.is_file_preview_active(),
            "untracked directory targets should avoid file preview loading mode"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup untracked directory fixture");
}

#[gpui::test]
fn untracked_directory_target_clears_stale_file_loading_state(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_dir_stale_loading",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create untracked directory path");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);

            set_test_file_status(
                &mut repo,
                file_rel.clone(),
                gitcomet_core::domain::FileStatusKind::Untracked,
                gitcomet_core::domain::DiffArea::Unstaged,
            );
            repo.diff_state.diff = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::Diff::from_unified(
                    gitcomet_core::domain::DiffTarget::WorkingTree {
                        path: file_rel.clone(),
                        area: gitcomet_core::domain::DiffArea::Unstaged,
                    },
                    "",
                ),
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);

            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_rel));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.untracked_directory_notice().is_some(),
            "expected untracked directory selection to expose a directory-specific notice"
        );
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "untracked directory target should not stay stuck in File Loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup stale-loading untracked directory fixture");
}

#[gpui::test]
fn directory_target_with_loading_status_clears_stale_file_loading_state(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_directory_loading_status",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("chess3");
    let _ = std::fs::remove_dir_all(&workdir);
    std::fs::create_dir_all(workdir.join(&file_rel)).expect("create directory target path");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);

            repo.status = gitcomet_state::model::Loadable::Loading;
            repo.diff_state.diff_target = Some(gitcomet_core::domain::DiffTarget::WorkingTree {
                path: file_rel.clone(),
                area: gitcomet_core::domain::DiffArea::Unstaged,
            });
            repo.diff_state.diff = gitcomet_state::model::Loadable::Loading;

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, Arc::clone(&next_state), cx);

            this.main_pane.update(cx, |pane, _cx| {
                pane.worktree_preview_path = Some(workdir.join(&file_rel));
                pane.worktree_preview = gitcomet_state::model::Loadable::Loading;
            });
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.untracked_directory_notice().is_some(),
            "expected directory target to expose a non-file notice even while status is loading"
        );
        assert!(
            !matches!(
                pane.worktree_preview,
                gitcomet_state::model::Loadable::Loading
            ),
            "directory target should not stay stuck in File Loading when status is loading"
        );
    });

    std::fs::remove_dir_all(&workdir).expect("cleanup directory-loading-status fixture");
}

#[gpui::test]
fn added_file_preview_ctrl_a_ctrl_c_copies_all_content(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(31);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_added_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("added.rs");
    let lines: Arc<Vec<String>> = Arc::new(vec!["alpha".into(), "beta".into(), "gamma".into()]);
    assert_file_preview_ctrl_a_ctrl_c_copies_all(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Added,
        lines,
    );
}

#[gpui::test]
fn deleted_file_preview_ctrl_a_ctrl_c_copies_all_content(cx: &mut gpui::TestAppContext) {
    let repo_id = gitcomet_state::model::RepoId(32);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_preview_deleted_copy",
        std::process::id()
    ));
    let file_rel = std::path::PathBuf::from("deleted.rs");
    let lines: Arc<Vec<String>> = Arc::new(vec!["old one".into(), "old two".into()]);
    assert_file_preview_ctrl_a_ctrl_c_copies_all(
        cx,
        repo_id,
        workdir,
        file_rel,
        gitcomet_core::domain::FileStatusKind::Deleted,
        lines,
    );
}

#[gpui::test]
fn commit_details_metadata_fields_are_selectable(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(33);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let parent_sha = "89abcdef0123456789abcdef0123456789abcdef".to_string();
    let commit_date = "2026-03-08 12:34:56 +0200".to_string();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-metadata-copy"));
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    committed_at: commit_date.clone(),
                    parent_ids: vec![gitcomet_core::domain::CommitId(parent_sha.clone().into())],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_details_sha_input.read(app).text(), commit_sha);
        assert_eq!(pane.commit_details_date_input.read(app).text(), commit_date);
        assert_eq!(
            pane.commit_details_parent_input.read(app).text(),
            parent_sha
        );
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(cx));
            pane.commit_details_date_input
                .update(cx, |input, cx| input.select_all_text(cx));
            pane.commit_details_parent_input
                .update(cx, |input, cx| input.select_all_text(cx));
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_details_sha_input.read(app).selected_text(),
            Some(commit_sha)
        );
        assert_eq!(
            pane.commit_details_date_input.read(app).selected_text(),
            Some(commit_date)
        );
        assert_eq!(
            pane.commit_details_parent_input.read(app).selected_text(),
            Some(parent_sha)
        );
    });
}

#[gpui::test]
fn commit_details_file_list_keeps_visible_viewport_when_overflowing(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(61);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let files = (0..48)
        .map(|ix| gitcomet_core::domain::CommitFileChange {
            path: std::path::PathBuf::from(format!("src/commit_details/dir_{ix}/file_{ix}.rs")),
            kind: gitcomet_core::domain::FileStatusKind::Modified,
        })
        .collect::<Vec<_>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-files-list"));
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files,
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);

            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.simulate_resize(gpui::size(px(1024.0), px(420.0)));

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let mut viewport_height = 0.0f32;
    let mut contents_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        let item_size = pane
            .commit_files_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected commit details files list to report its measured viewport");
        viewport_height = item_size.item.height.into();
        contents_height = item_size.contents.height.into();
    });

    assert!(
        contents_height > viewport_height,
        "expected commit details file list to overflow so the scrollbar has content to represent (viewport_height={viewport_height}, contents_height={contents_height})",
    );
    assert!(
        viewport_height >= 24.0,
        "expected commit details file list to keep at least one visible row when overflowing (viewport_height={viewport_height}, contents_height={contents_height})",
    );
}

#[gpui::test]
fn switching_active_repo_restores_commit_message_draft_per_repo(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_a = gitcomet_state::model::RepoId(41);
    let repo_b = gitcomet_state::model::RepoId(42);
    let make_state = |active_repo: gitcomet_state::model::RepoId| {
        Arc::new(AppState {
            repos: vec![
                opening_repo_state(repo_a, Path::new("/tmp/repo-a")),
                opening_repo_state(repo_b, Path::new("/tmp/repo-b")),
            ],
            active_repo: Some(active_repo),
            ..Default::default()
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_a);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_b);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("repo-b draft".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_a);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "draft message");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let next_state = make_state(repo_b);
            push_test_state(this, Arc::clone(&next_state), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(pane.commit_message_input.read(app).text(), "repo-b draft");
    });
}

#[gpui::test]
fn merge_start_prefills_default_commit_message(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(43);
    let make_state = |merge_message: Option<&str>| {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-merge"));
        repo.merge_commit_message = gitcomet_state::model::Loadable::Ready(
            merge_message.map(std::string::ToString::to_string),
        );
        repo.merge_message_rev = u64::from(merge_message.is_some());
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(None), cx);
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("draft message".to_string(), cx)
                });
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(Some("Merge branch 'feature'")), cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).text(),
            "Merge branch 'feature'"
        );
    });
}

#[gpui::test]
fn commit_click_dispatches_after_state_update_without_intermediate_redraw(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let make_state = |staged_count: usize, local_actions_in_flight: u32| {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-click"));
        repo.status = gitcomet_state::model::Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: (0..staged_count)
                    .map(|ix| gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from(format!("staged-{ix}.txt")),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    })
                    .collect(),
                unstaged: Vec::new(),
            }
            .into(),
        );
        repo.local_actions_in_flight = local_actions_in_flight;
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(0, 0), cx);
        });
        let _ = window.draw(app);
    });

    let commit_center = cx
        .debug_bounds("commit_button")
        .expect("expected commit button bounds")
        .center();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(1, 0), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input
                    .update(cx, |input, cx| input.set_text("hello".to_string(), cx));
                cx.notify();
            });
        });
    });

    cx.simulate_mouse_move(commit_center, None, Modifiers::default());
    cx.simulate_event(MouseDownEvent {
        position: commit_center,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: commit_center,
        modifiers: Modifiers::default(),
        button: MouseButton::Left,
        click_count: 1,
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_message_input.read(app).text(),
            "",
            "expected first click to execute commit handler and clear the input"
        );
    });
}

#[gpui::test]
fn theme_change_clears_conflict_three_way_segments_cache(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    // Seed the three-way segments cache with dummy entries, then change theme
    // and verify the cache was cleared. Before this fix, set_theme() cleared
    // all other conflict style caches but missed the three-way cache, leaving
    // stale highlight colors after a theme switch.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let dummy = super::CachedDiffStyledText {
                    text: "dummy".into(),
                    highlights: Arc::from(Vec::new()),
                    highlights_hash: 0,
                    text_hash: 0,
                };
                pane.conflict_three_way_segments_cache
                    .insert((0, ThreeWayColumn::Base), dummy.clone());
                pane.conflict_three_way_segments_cache
                    .insert((1, ThreeWayColumn::Ours), dummy.clone());
                pane.conflict_diff_segments_cache_split
                    .insert(
                        (0, crate::view::conflict_resolver::ConflictPickSide::Ours),
                        dummy.clone(),
                    );
                assert_eq!(pane.conflict_three_way_segments_cache.len(), 2);
                assert_eq!(pane.conflict_diff_segments_cache_split.len(), 1);

                let new_theme = crate::theme::AppTheme::gitcomet_light();
                pane.set_theme(new_theme, cx);

                assert!(
                    pane.conflict_three_way_segments_cache.is_empty(),
                    "set_theme should clear the three-way segments cache to avoid stale highlight colors"
                );
                assert!(
                    pane.conflict_diff_segments_cache_split.is_empty(),
                    "set_theme should clear the two-way split segments cache"
                );
            });
        });
    });
}

#[gpui::test]
fn status_section_drag_updates_saved_height(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(46);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_resize_drag",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                    unstaged: vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("unstaged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut initial_status_sections_bounds = None;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        initial_status_sections_bounds = pane.current_status_sections_bounds();
        assert!(
            initial_status_sections_bounds.is_some(),
            "expected status sections to report measured bounds after draw"
        );
        assert_eq!(
            pane.saved_status_section_heights().0,
            None,
            "status resize height should start unset before dragging"
        );
    });

    let initial_handle_bounds = cx
        .debug_bounds("status_resize_change_tracking_staged")
        .expect("expected status resize handle bounds");
    let handle_center = initial_handle_bounds.center();
    let drag_target = gpui::point(handle_center.x, handle_center.y + px(48.0));
    let initial_change_tracking_height = initial_handle_bounds.top()
        - initial_status_sections_bounds
            .expect("expected status section bounds while computing drag start height")
            .top();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.status_section_resize = Some(StatusSectionResizeState {
                handle: StatusSectionResizeHandle::ChangeTrackingAndStaged,
                start_y: handle_center.y,
                start_height: initial_change_tracking_height,
            });
            assert!(
                pane.update_status_section_resize(drag_target.y, cx),
                "expected direct resize update to change the saved change-tracking height"
            );
            assert!(
                pane.finish_status_section_resize(cx),
                "expected direct resize finish to persist the updated change-tracking height"
            );
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert!(
            pane.change_tracking_height.is_some(),
            "expected dragging the resize handle to store a height"
        );
        assert!(
            pane.saved_status_section_heights().0.is_some(),
            "expected dragging the resize handle to persist a saved change-tracking height"
        );
    });
    let updated_handle_bounds = cx
        .debug_bounds("status_resize_change_tracking_staged")
        .expect("expected updated status resize handle bounds after dragging");
    assert!(
        updated_handle_bounds.top() > initial_handle_bounds.top(),
        "expected resizing the outer divider downward to move the staged section downward"
    );
}

#[gpui::test]
fn staged_section_remains_visible_after_window_resize_with_saved_split_height(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_resize_window_shrink",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                    unstaged: (0..30)
                        .map(|ix| gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        })
                        .collect(),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut initial_window_size = gpui::size(px(0.0), px(0.0));
    let mut initial_status_height = px(0.0);
    cx.update(|window, app| {
        initial_window_size = window.viewport_size();
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        initial_status_height = pane
            .current_status_sections_bounds()
            .expect("expected status section bounds before shrinking the window")
            .size
            .height;
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(initial_status_height);
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.simulate_resize(gpui::size(
        initial_window_size.width,
        initial_window_size.height - px(120.0),
    ));

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let staged_header_bounds = cx
        .debug_bounds("staged_header")
        .expect("expected staged header bounds after shrinking the window");

    let mut staged_viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        staged_viewport_height = pane
            .staged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected staged list viewport after shrinking the window")
            .item
            .height
            .into();
    });

    assert!(
        staged_viewport_height > 0.0,
        "expected staged section to keep a visible list viewport after shrinking the window (staged_header={staged_header_bounds:?}, staged_viewport_height={staged_viewport_height})"
    );
}

#[gpui::test]
fn split_status_section_resize_moves_untracked_section(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(47);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_status_resize_drag",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                    unstaged: vec![
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from("new.txt"),
                            kind: gitcomet_core::domain::FileStatusKind::Untracked,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from("tracked.txt"),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ],
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let mut initial_stack_bounds = None;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            crate::view::test_support::change_tracking_view(view.read(app)),
            ChangeTrackingView::SplitUntracked,
            "expected the root view to store split change-tracking mode"
        );
        assert_eq!(
            pane.change_tracking_view,
            ChangeTrackingView::SplitUntracked,
            "expected the details pane to store split change-tracking mode"
        );
        assert!(
            pane.current_change_tracking_stack_bounds().is_some(),
            "expected split change-tracking stack bounds after initial draw"
        );
        initial_stack_bounds = pane.current_change_tracking_stack_bounds();
    });
    assert!(
        cx.debug_bounds("status_resize_change_tracking_staged")
            .is_some(),
        "expected the outer status resize handle to still be present in split mode"
    );
    let initial_split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected split unstaged header bounds in split change-tracking view");

    let initial_handle_bounds = cx
        .debug_bounds("status_resize_untracked_unstaged")
        .expect("expected inner status resize handle bounds in split change-tracking view");
    let initial_untracked_wrapper_bounds = cx
        .debug_bounds("status_untracked_wrapper")
        .expect("expected untracked wrapper bounds in split change-tracking view");
    let handle_center = initial_handle_bounds.center();
    let drag_target = gpui::point(handle_center.x, handle_center.y + px(48.0));
    let initial_top_height = initial_handle_bounds.top()
        - initial_stack_bounds.expect(
            "expected initial split change-tracking stack bounds while computing drag start height",
        )
        .top();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.status_section_resize = Some(StatusSectionResizeState {
                handle: StatusSectionResizeHandle::UntrackedAndUnstaged,
                start_y: handle_center.y,
                start_height: initial_top_height,
            });
            assert!(
                pane.update_status_section_resize(drag_target.y, cx),
                "expected direct resize update to change the untracked height"
            );
            assert!(
                pane.finish_status_section_resize(cx),
                "expected direct resize finish to persist the updated height"
            );
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let mut updated_untracked_height = None;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            crate::view::test_support::change_tracking_view(view.read(app)),
            ChangeTrackingView::SplitUntracked,
            "expected split change-tracking view to remain active while resizing"
        );
        assert!(
            pane.untracked_height.is_some(),
            "expected dragging the inner resize handle to store an untracked height"
        );
        updated_untracked_height = pane.untracked_height;
    });
    let updated_handle_bounds = cx
        .debug_bounds("status_resize_untracked_unstaged")
        .expect("expected updated inner status resize handle bounds after dragging");
    let updated_split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected updated split unstaged header bounds after dragging");
    assert!(
        updated_split_unstaged_header_bounds.top() > initial_split_unstaged_header_bounds.top(),
        "expected resizing the inner divider downward to move the split unstaged section downward (initial_header_top={:?}, updated_header_top={:?}, initial_untracked_wrapper={:?}, updated_untracked_height={:?})",
        initial_split_unstaged_header_bounds.top(),
        updated_split_unstaged_header_bounds.top(),
        initial_untracked_wrapper_bounds,
        updated_untracked_height,
    );
    assert!(
        updated_handle_bounds.center().y > initial_handle_bounds.center().y,
        "expected the inner divider to move downward after resizing (initial_handle_y={:?}, updated_handle_y={:?}, updated_untracked_height={:?})",
        initial_handle_bounds.center().y,
        updated_handle_bounds.center().y,
        updated_untracked_height,
    );
}

#[gpui::test]
fn unstaged_scroll_viewport_tracks_resized_section_height(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(48);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_unstaged_scroll_viewport",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                    unstaged: (0..30)
                        .map(|ix| gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        })
                        .collect(),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(160.0));
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let unstaged_wrapper_bounds = cx
        .debug_bounds("status_change_tracking_wrapper")
        .expect("expected unstaged section bounds after resizing");
    let unstaged_header_bounds = cx
        .debug_bounds("unstaged_header")
        .expect("expected unstaged header bounds after resizing");

    let mut is_scrollable = false;
    let mut viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        is_scrollable = pane.unstaged_scroll.is_scrollable();
        viewport_height = pane
            .unstaged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected unstaged uniform list size after draw")
            .item
            .height
            .into();
    });

    let visible_height: f32 =
        (unstaged_wrapper_bounds.bottom() - unstaged_header_bounds.bottom()).into();
    assert!(
        is_scrollable,
        "expected unstaged list to become scrollable after shrinking the unstaged section"
    );
    assert!(
        (viewport_height - visible_height).abs() <= 1.0,
        "expected unstaged uniform list viewport to match visible container height after resize (viewport_height={viewport_height}, visible_height={visible_height})"
    );
}

#[gpui::test]
fn split_unstaged_scroll_viewport_tracks_resized_section_height(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(49);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_unstaged_scroll_viewport",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![],
                    unstaged: (0..30)
                        .map(|ix| gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        })
                        .collect(),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(240.0));
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let change_tracking_wrapper_bounds = cx
        .debug_bounds("status_change_tracking_wrapper")
        .expect("expected change-tracking section bounds after resizing");
    let split_unstaged_wrapper_bounds = cx
        .debug_bounds("status_split_unstaged_wrapper")
        .expect("expected split unstaged section bounds after resizing");
    let split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected split unstaged header bounds after resizing");

    let mut is_scrollable = false;
    let mut viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        is_scrollable = pane.unstaged_scroll.is_scrollable();
        viewport_height = pane
            .unstaged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected split unstaged uniform list size after draw")
            .item
            .height
            .into();
    });

    let visible_bottom = split_unstaged_wrapper_bounds
        .bottom()
        .min(change_tracking_wrapper_bounds.bottom());
    let visible_height: f32 = (visible_bottom - split_unstaged_header_bounds.bottom())
        .max(px(0.0))
        .into();
    assert!(
        is_scrollable,
        "expected split unstaged list to become scrollable after shrinking the outer change-tracking section"
    );
    assert!(
        (viewport_height - visible_height).abs() <= 1.0,
        "expected split unstaged uniform list viewport to match visible container height after resize (viewport_height={viewport_height}, visible_height={visible_height})"
    );
}

#[gpui::test]
fn split_unstaged_scroll_viewport_updates_after_outer_resize_shrink(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(50);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_split_unstaged_outer_resize",
        std::process::id()
    ));

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }],
                    unstaged: (0..30)
                        .map(|ix| gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        })
                        .collect(),
                }
                .into(),
            );

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.set_change_tracking_view(ChangeTrackingView::SplitUntracked, cx);
        });
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(360.0));
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.change_tracking_height = Some(px(180.0));
            cx.notify();
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let change_tracking_wrapper_bounds = cx
        .debug_bounds("status_change_tracking_wrapper")
        .expect("expected change-tracking section bounds after shrinking the outer resize");
    let split_unstaged_wrapper_bounds = cx
        .debug_bounds("status_split_unstaged_wrapper")
        .expect("expected split unstaged section bounds after shrinking the outer resize");
    let split_unstaged_header_bounds = cx
        .debug_bounds("split_unstaged_header")
        .expect("expected split unstaged header bounds after shrinking the outer resize");

    let mut is_scrollable = false;
    let mut viewport_height = 0.0f32;
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        is_scrollable = pane.unstaged_scroll.is_scrollable();
        viewport_height = pane
            .unstaged_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected split unstaged uniform list size after outer resize shrink")
            .item
            .height
            .into();
    });

    let visible_bottom = split_unstaged_wrapper_bounds
        .bottom()
        .min(change_tracking_wrapper_bounds.bottom());
    let visible_height: f32 = (visible_bottom - split_unstaged_header_bounds.bottom())
        .max(px(0.0))
        .into();

    assert!(
        split_unstaged_wrapper_bounds.bottom() <= change_tracking_wrapper_bounds.bottom() + px(1.0),
        "expected split unstaged section to stay within the visible change-tracking area after shrinking the outer resize (split_unstaged_bottom={:?}, change_tracking_bottom={:?})",
        split_unstaged_wrapper_bounds.bottom(),
        change_tracking_wrapper_bounds.bottom(),
    );
    assert!(
        is_scrollable,
        "expected split unstaged list to become scrollable after shrinking the outer resize"
    );
    assert!(
        (viewport_height - visible_height).abs() <= 1.0,
        "expected split unstaged uniform list viewport to match the visible clipped height after shrinking the outer resize (viewport_height={viewport_height}, visible_height={visible_height})"
    );
}
