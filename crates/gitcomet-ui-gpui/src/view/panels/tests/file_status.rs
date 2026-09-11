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
                && pane.worktree_preview_text.as_ref() == "one\ntwo\n"
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
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-04-07T12:00:00Z".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![gitcomet_core::domain::CommitFileChange {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Deleted,
                        is_submodule: false,
                        additions: None,
                        deletions: None,
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
                && pane.worktree_preview_text.as_ref() == "{\"removed\":true}\n"
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: file_rel.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
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
                        staged: std::sync::Arc::new(vec![]),
                        unstaged: std::sync::Arc::new(vec![
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
                        ]),
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

/// A window showing one commit's details, with the sha/date/parent fields
/// populated. Returns the fields' expected text.
fn commit_details_metadata_fixture(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<crate::view::GitCometView>,
    &mut gpui::VisualTestContext,
    String,
    String,
    String,
) {
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
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: commit_date.clone(),
                    committed_at_unix: 0,
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

    (view, cx, commit_sha, commit_date, parent_sha)
}

#[gpui::test]
fn commit_details_metadata_fields_are_selectable(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, commit_date, parent_sha) = commit_details_metadata_fixture(cx);

    // One field at a time: only one surface may hold a selection, so selecting
    // all three at once would leave just the last one highlighted.
    for (label, expected) in [
        ("sha", commit_sha),
        ("date", commit_date),
        ("parent", parent_sha),
    ] {
        cx.update(|window, app| {
            let details_pane = view.read(app).details_pane.clone();
            details_pane.update(app, |pane, cx| {
                let input = match label {
                    "sha" => pane.commit_details_sha_input.clone(),
                    "date" => pane.commit_details_date_input.clone(),
                    _ => pane.commit_details_parent_input.clone(),
                };
                input.update(cx, |input, cx| input.select_all_text(window, cx));
            });
        });

        cx.update(|_window, app| {
            let details_pane = view.read(app).details_pane.clone();
            let pane = details_pane.read(app);
            let actual = match label {
                "sha" => pane.commit_details_sha_input.read(app).selected_text(),
                "date" => pane.commit_details_date_input.read(app).selected_text(),
                _ => pane.commit_details_parent_input.read(app).selected_text(),
            };
            assert_eq!(actual, Some(expected), "{label} field must be selectable");
        });
    }
}

/// Any other surface taking the window's selection clears a text input's
/// highlight, not just another input. Stands in for the diff pane, the
/// terminal, and any future selectable surface.
#[gpui::test]
fn another_surface_taking_the_selection_clears_a_text_input(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, _date, _parent) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha)
        );
    });

    cx.update(|window, app| {
        let mut elsewhere = crate::text_selection_owner::SelectionOwnerToken::default();
        elsewhere.adopt(window, app);
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            None,
            "the input must drop its highlight once another surface owns the selection"
        );
    });
}

/// The browser rule end to end: a real press on a surface that does not own the
/// selection collapses it. This is the counterpart to
/// `a_preserved_press_leaves_a_text_input_selection_alone` -- together they show
/// the capture-phase invalidator actually fires.
#[gpui::test]
fn an_ordinary_press_elsewhere_clears_a_text_input_selection(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, _date, _parent) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha),
            "precondition: the input holds a selection"
        );
    });

    // Mid-window, clear of the titlebar and of any selectable text surface.
    let size = cx.update(|window, _app| window.viewport_size());
    let elsewhere = gpui::point(size.width * 0.5, size.height * 0.25);
    cx.simulate_mouse_move(elsewhere, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: elsewhere,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            None,
            "a press outside the owning surface must collapse the selection"
        );
    });
}

/// The overlay flag must fall back to closed on every dismiss path: one that
/// forgot would latch it on and disable click-away blur for the session.
#[gpui::test]
fn blur_still_works_after_a_stale_overlay_flag(cx: &mut gpui::TestAppContext) {
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);

    let input = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .clone()
    });

    // Stand in for a dismiss path that forgot to clear the flag: no popover is
    // open, but the flag says one is. The next render must correct it.
    cx.update(|window, app| crate::text_selection_owner::set_overlay_open(window, true, app));
    cx.run_until_parked();
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let handle = input.read(app).focus_handle();
        handle.focus(window, app);
    });
    cx.run_until_parked();

    let size = cx.update(|window, _app| window.viewport_size());
    let sidebar = gpui::point(size.width * 0.05, size.height * 0.5);
    cx.simulate_mouse_move(sidebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: sidebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    assert!(
        cx.update(|window, app| !input.read(app).focus_handle().is_focused(window)),
        "a stale overlay flag must be corrected by render, so blur still works"
    );
}

/// A selection-neutral gesture -- a scrollbar drag, a splitter -- must leave the
/// highlight alone even though it is a press outside the owning surface.
#[gpui::test]
fn a_preserved_press_leaves_a_text_input_selection_alone(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, _date, _parent) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });

    // The titlebar, a real `preserve` site -- dragging the window is chrome,
    // not content. Pressed for real so the whole capture/bubble/resolve chain
    // runs, exactly as in the clearing test above.
    let titlebar = gpui::point(gpui::px(2.0), gpui::px(2.0));
    cx.simulate_mouse_move(titlebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: titlebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha),
            "a preserved press must not collapse the selection"
        );
    });
}

/// Selecting one commit-details field must drop the highlight on the previous
/// one, the way a browser only ever shows one selection.
#[gpui::test]
fn selecting_a_second_commit_details_field_clears_the_first(cx: &mut gpui::TestAppContext) {
    let (view, cx, commit_sha, commit_date, _parent_sha) = commit_details_metadata_fixture(cx);

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_sha_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        assert_eq!(
            details_pane
                .read(app)
                .commit_details_sha_input
                .read(app)
                .selected_text(),
            Some(commit_sha)
        );
    });

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.commit_details_date_input
                .update(cx, |input, cx| input.select_all_text(window, cx));
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let pane = details_pane.read(app);
        assert_eq!(
            pane.commit_details_date_input.read(app).selected_text(),
            Some(commit_date),
            "the newly selected field keeps its selection"
        );
        assert_eq!(
            pane.commit_details_sha_input.read(app).selected_text(),
            None,
            "the previously selected field must have been cleared"
        );
    });
}

/// Click inside a commit-details text input and report which menu, if any, the
/// popover host opened for it.
fn click_commit_details_link(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    click: gpui::Point<Pixels>,
    click_count: usize,
) -> Option<PopoverKind> {
    simulate_counted_click(cx, click, click_count);
    cx.run_until_parked();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    })
}

/// The first point inside the commit message, which every fixture below puts a
/// link at.
fn commit_details_message_link_point(cx: &mut gpui::VisualTestContext) -> gpui::Point<Pixels> {
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected commit details message bounds");
    point(bounds.left() + px(4.0), bounds.top() + px(8.0))
}

/// Show a single commit whose message is `message`, so its links can be clicked.
fn show_commit_details_message(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
    workdir: &str,
    message: &str,
) {
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new(workdir));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: message.to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                }));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
}

#[gpui::test]
fn commit_details_message_url_click_opens_the_web_link_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(41),
        "/tmp/repo-commit-message-url",
        "https://example.com/issues/42 is fixed",
    );

    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::WebLinkMenu { ref url, .. })
                if url.as_ref() == "https://example.com/issues/42"
        ),
        "clicking a URL should open the same menu the markdown preview shows, got {popover:?}"
    );

    // The same menu the markdown preview offers, entry for entry.
    assert!(
        cx.debug_bounds("context_menu_open_in_web_browser")
            .is_some(),
        "expected an entry that opens the link"
    );
    assert!(
        cx.debug_bounds("context_menu_copy_link_address").is_some(),
        "expected an entry that copies the address"
    );

    // Reached from the details pane, so closing it must not hand the keyboard
    // to the diff panel the way a preview link does.
    cx.update(|_window, app| {
        assert!(
            !view
                .read(app)
                .popover_host
                .read(app)
                .popover_opened_from_diff_panel_for_tests(),
            "a commit message is not a diff-panel invoker"
        );
    });

    // The menu hangs off the link's own box, not off the row or the panel, so
    // it opens flush under the words it describes.
    cx.update(|_window, app| {
        let anchor = view
            .read(app)
            .popover_host
            .read(app)
            .popover_anchor_bounds_for_tests()
            .expect("a link menu anchors on the link's box");
        let details_pane = view.read(app).details_pane.clone();
        let expected = details_pane
            .read(app)
            .commit_details_message_input
            .read(app)
            .hotspot_bounds(&(0.."https://example.com/issues/42".len()))
            .expect("expected bounds for the link");
        assert_eq!(anchor, expected);
        assert!(anchor.contains(&link), "the click landed inside that box");
    });
}

#[gpui::test]
fn commit_details_message_mailto_click_opens_the_web_link_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(42),
        "/tmp/repo-commit-message-mailto",
        "mailto:maintainer@example.com reported this",
    );

    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::WebLinkMenu { ref url, .. })
                if url.as_ref() == "mailto:maintainer@example.com"
        ),
        "clicking a mailto link should open the link menu, got {popover:?}"
    );
}

/// The message that motivated the stricter commit-id rules, end to end: build
/// ids, a Gerrit change id and URL path segments all used to linkify as commits.
#[gpui::test]
fn commit_details_message_trailers_do_not_linkify_as_commits(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(43),
        "/tmp/repo-commit-message-trailers",
        concat!(
            "Cr-Original-Build-Id: 8674534147806418049\n",
            "Change-Id: I7a5d480873e839444e4e188ffa87f9c635e2fb81\n",
        ),
    );

    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        popover.is_none(),
        "a build id is not a commit id, so clicking it should open nothing, got {popover:?}"
    );
}

/// A Chromium PGO roll: two build artifacts whose names are built out of two
/// full-length hashes each. Every one of them used to linkify.
#[gpui::test]
fn commit_details_message_filename_hashes_do_not_linkify_as_commits(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    show_commit_details_message(
        cx,
        &view,
        gitcomet_state::model::RepoId(44),
        "/tmp/repo-commit-message-profdata",
        concat!(
            "Roll Chrome Mac PGO profile from ",
            "chrome-mac-7922-1785736271-37240ae8aae5f01fc00cbf0b7ea19b73826e0dba",
            "-d9e99b2bafcc6df3c2a5bf803fcb5483d33dbdd0.profdata to ",
            "chrome-mac-7922-1785755104-c2eee60da6765f60eca833b7c5c0d85ddcbc2940",
            "-551a1e94b700524e479bd2d64ccaf8cdb71d43a6.profdata",
        ),
    );

    cx.update(|_window, app| {
        let details_pane = view.read(app).details_pane.clone();
        let links = details_pane
            .read(app)
            .commit_details_message_link_menu
            .read(app)
            .links_for_tests();
        assert!(
            links.is_empty(),
            "hashes joined into a filename are not commit ids, got {links:?}"
        );
    });
}

#[gpui::test]
fn commit_details_message_sha_click_menu_navigate_reveals_referenced_commit(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(34);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";
    let target_sha_upper = target_sha.to_ascii_uppercase();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-sha"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(current_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "current".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(target_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "target".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha_upper} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let link = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(
            popover,
            Some(PopoverKind::CommitShaLinkMenu {
                ref commit_id,
                allow_navigate: true,
                ..
            }) if commit_id.as_ref() == target_sha
        ),
        // The message spells the SHA in upper case; the link resolves to the
        // lower-case id the repository uses.
        "clicking a commit id should open its menu, got {popover:?}"
    );

    let reveal_bounds = cx
        .debug_bounds("context_menu_reveal_commit")
        .expect("expected reveal commit entry");
    simulate_counted_click(cx, reveal_bounds.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(target_sha.into());
        assert_eq!(selected, Some(&expected));
    });
}

/// Only the link opens a menu. Clicking the prose beside it must leave the
/// popover host alone, otherwise every caret placement would raise a menu.
#[gpui::test]
fn commit_details_message_click_beside_a_sha_opens_no_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(36);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-message-hover-close"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(current_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "current".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(target_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "target".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                }));
            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected commit details message bounds");
    // The SHA occupies the head of the line; " fixes the regression" follows it.
    let link = commit_details_message_link_point(cx);
    let beside_link = point(link.x + px(320.0), link.y);
    assert!(
        beside_link.x < bounds.right(),
        "the point past the SHA has to stay inside the message"
    );

    let popover = click_commit_details_link(cx, &view, beside_link, 1);
    assert!(
        popover.is_none(),
        "clicking plain message text should open no menu, got {popover:?}"
    );

    // …and the link itself still does, so the point above was not simply outside
    // the input.
    let popover = click_commit_details_link(cx, &view, link, 1);
    assert!(
        matches!(popover, Some(PopoverKind::CommitShaLinkMenu { ref commit_id, .. })
            if commit_id.as_ref() == target_sha),
        "clicking the commit id should open its menu, got {popover:?}"
    );
}

#[gpui::test]
fn commit_details_message_sha_retained_details_are_inert_after_selection_changes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(38);
    let retained_sha = "0123456789abcdef0123456789abcdef01234567";
    let selected_sha = "fedcba9876543210fedcba9876543210fedcba98";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-retained-commit-details-sha"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(retained_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "retained".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(selected_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "selected".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(target_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "target".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(selected_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(retained_sha.into()),
                    message: format!("{target_sha} should not reveal from retained details"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let bounds = cx
        .debug_bounds("commit_details_message_scroll_surface")
        .expect("expected retained commit details message bounds");
    let hover = point(bounds.left() + px(4.0), bounds.top() + px(8.0));
    cx.simulate_mouse_move(hover, None, Modifiers::default());
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(301));
    cx.run_until_parked();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(selected_sha.into());
        assert_eq!(selected, Some(&expected));
    });
    let popover = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()
    });
    assert!(
        popover.is_none(),
        "expected retained details to stay inert, got {popover:?}"
    );
}

/// Opening the menu is not the same as acting on it: the click itself must not
/// move the history, only offer to.
#[gpui::test]
fn commit_details_message_sha_click_opens_the_menu_without_navigating(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(35);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-sha-select"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let click = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, click, 1);
    assert!(
        matches!(popover, Some(PopoverKind::CommitShaLinkMenu { ref commit_id, .. })
            if commit_id.as_ref() == target_sha),
        "expected the commit link menu, got {popover:?}"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_details_message_input.read(app).selected_text(),
            None
        );
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(current_sha.into());
        assert_eq!(selected, Some(&expected));
    });
}

/// Selecting the words of a link has to keep working, so only a plain click
/// follows it — a double click is a word selection like anywhere else.
#[gpui::test]
fn commit_details_message_sha_double_click_selects_instead_of_opening_the_menu(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(41);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let target_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-sha-focus"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: format!("{target_sha} fixes the regression"),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let click = commit_details_message_link_point(cx);
    let popover = click_commit_details_link(cx, &view, click, 2);
    assert!(
        popover.is_none(),
        "a double click on a link should select its text, not open a menu, got {popover:?}"
    );

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_details_message_input
                .read(app)
                .selected_text()
                .as_deref(),
            Some(target_sha),
            "the double click should have selected the whole commit id"
        );
    });
}

#[gpui::test]
fn commit_details_parent_sha_click_menu_navigate_reveals_referenced_commit(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(39);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";
    let parent_sha = "89abcdef0123456789abcdef0123456789abcdef";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-parent-hover-menu"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(current_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "current".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                    gitcomet_core::domain::Commit {
                        id: gitcomet_core::domain::CommitId(parent_sha.into()),
                        parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                        summary: "parent".into(),
                        author: "Alice".into(),
                        time: std::time::SystemTime::UNIX_EPOCH,
                    },
                ],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: "subject".into(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(parent_sha.into())],
                    files: vec![],
                }));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let parent_bounds = cx
        .debug_bounds("commit_details_parent_link_menu")
        .expect("expected parent sha link target");
    let popover = click_commit_details_link(
        cx,
        &view,
        point(parent_bounds.left() + px(4.0), parent_bounds.center().y),
        1,
    );
    assert!(
        matches!(
            popover,
            Some(PopoverKind::CommitShaLinkMenu {
                ref commit_id,
                allow_navigate: true,
                ..
            }) if commit_id.as_ref() == parent_sha
        ),
        "clicking the parent id should open its menu, got {popover:?}"
    );

    let reveal_bounds = cx
        .debug_bounds("context_menu_reveal_commit")
        .expect("expected parent reveal commit entry");
    simulate_counted_click(cx, reveal_bounds.center(), 1);
    cx.run_until_parked();
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::sync_store_snapshot(this, cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        let selected = view
            .read(app)
            .state
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.history_state.selected_commit.as_ref());
        let expected = gitcomet_core::domain::CommitId(parent_sha.into());
        assert_eq!(selected, Some(&expected));
    });
}

#[gpui::test]
fn commit_details_parent_sha_dash_has_no_link_menu(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(40);
    let current_sha = "0123456789abcdef0123456789abcdef01234567";

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-parent-dash-hover"));
            repo.open = Loadable::Ready(());
            repo.head_branch = Loadable::Ready("main".into());
            repo.status = Loadable::Ready(gitcomet_core::domain::RepoStatus::default().into());
            repo.log = Loadable::Ready(Arc::new(gitcomet_core::domain::LogPage {
                commits: vec![gitcomet_core::domain::Commit {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    parent_ids: gitcomet_core::domain::CommitParentIds::new(),
                    summary: "current".into(),
                    author: "Alice".into(),
                    time: std::time::SystemTime::UNIX_EPOCH,
                }],
                next_cursor: None,
            }));
            repo.log_rev = 1;
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(current_sha.into()));
            repo.history_state.commit_details =
                Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(current_sha.into()),
                    message: "subject".into(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".into(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![],
                }));

            let next_state = app_state_with_repo(repo, repo_id);
            this.store
                .replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);
        });
    });

    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let parent_bounds = cx
        .debug_bounds("commit_details_parent_link_menu")
        .expect("expected parent sha link target");
    let popover = click_commit_details_link(
        cx,
        &view,
        point(parent_bounds.left() + px(4.0), parent_bounds.center().y),
        1,
    );

    assert!(
        popover.is_none(),
        "expected placeholder parent value to stay non-interactive, got {popover:?}"
    );
}

#[gpui::test]
fn commit_details_added_file_copy_path_works_after_left_clicking_menu_entry(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = lock_clipboard_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(60);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_added_copy_path",
        std::process::id()
    ));
    let added_path = std::path::Path::new("src").join("added_from_commit.rs");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files: vec![gitcomet_core::domain::CommitFileChange {
                        path: added_path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Added,
                        is_submodule: false,
                        additions: None,
                        deletions: None,
                    }],
                },
            ));

            let next_state = app_state_with_repo(repo, repo_id);
            push_test_state(this, next_state, cx);
        });
    });

    cx.write_to_clipboard(gpui::ClipboardItem::new_string("initial".to_string()));

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let row_bounds = cx
        .debug_bounds("commit_file_60_0")
        .expect("expected added commit file row");
    let row_center = row_bounds.center();
    cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        row_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        row_center,
        gpui::MouseButton::Right,
        gpui::Modifiers::default(),
    );

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    let copy_bounds = cx
        .debug_bounds("context_menu_copy_absolute_path")
        .expect("expected Copy absolute path context menu row");
    let copy_center = copy_bounds.center();
    cx.simulate_mouse_move(copy_center, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        copy_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("initial".to_string())
    );
    cx.simulate_mouse_up(
        copy_center,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );

    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(workdir.join(&added_path).display().to_string())
    );
}

#[gpui::test]
fn commit_details_file_right_click_only_opens_menu_for_added_modified_and_deleted(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(61);
    let commit_sha = "0123456789abcdef0123456789abcdef01234567".to_string();
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_file_right_click_menu_only",
        std::process::id()
    ));
    let files = [
        (
            std::path::PathBuf::from("src/added.rs"),
            gitcomet_core::domain::FileStatusKind::Added,
        ),
        (
            std::path::PathBuf::from("src/deleted.rs"),
            gitcomet_core::domain::FileStatusKind::Deleted,
        ),
        (
            std::path::PathBuf::from("src/modified.rs"),
            gitcomet_core::domain::FileStatusKind::Modified,
        ),
    ];
    let initial_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: std::path::PathBuf::from("src/current.rs"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.diff_state.diff_target = Some(initial_target.clone());
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![gitcomet_core::domain::CommitId(
                        "89abcdef0123456789abcdef0123456789abcdef".into(),
                    )],
                    files: files
                        .iter()
                        .map(|(path, kind)| gitcomet_core::domain::CommitFileChange {
                            path: path.clone(),
                            kind: *kind,
                            is_submodule: false,
                            additions: None,
                            deletions: None,
                        })
                        .collect(),
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

    for (ix, (path, _kind)) in files.iter().enumerate() {
        let row_selector = format!("commit_file_{}_{}", repo_id.0, ix);
        let row_bounds = cx
            .debug_bounds(Box::leak(row_selector.into_boxed_str()))
            .expect("expected commit file row");
        let row_center = row_bounds.center();
        cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );

        let (popover_kind, diff_target) = cx.update(|_window, app| {
            let view = view.read(app);
            let popover_kind = view.popover_host.read(app).popover_kind_for_tests();
            let diff_target = view
                .state
                .repos
                .iter()
                .find(|repo| repo.id == repo_id)
                .and_then(|repo| repo.diff_state.diff_target.clone());
            (popover_kind, diff_target)
        });

        assert_eq!(
            popover_kind,
            Some(PopoverKind::CommitFileMenu {
                repo_id,
                commit_id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                path: path.clone(),
            })
        );
        assert_eq!(diff_target, Some(initial_target.clone()));

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }
}

#[gpui::test]
fn status_file_right_click_opens_menu_without_opening_diff_or_changing_selection(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store_for_view, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_right_click_menu_only",
        std::process::id()
    ));

    let a = std::path::PathBuf::from("a.txt");
    let b = std::path::PathBuf::from("b.txt");
    let untracked = std::path::PathBuf::from("untracked.txt");
    let staged = std::path::PathBuf::from("staged.txt");

    // The diff panel is parked on a file that none of the right-clicks touch.
    let initial_target = gitcomet_core::domain::DiffTarget::WorkingTree {
        path: std::path::PathBuf::from("parked.txt"),
        area: gitcomet_core::domain::DiffArea::Unstaged,
    };
    let selection = vec![a.clone(), b.clone()];

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.open = gitcomet_state::model::Loadable::Ready(());
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: staged.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: a.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: b.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: untracked.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Untracked,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            repo.diff_state.diff_target = Some(initial_target.clone());

            // Seed the store too, so `Msg::SelectDiff` would really land in
            // `diff_state.diff_target` if the right-click still dispatched one.
            let next_state = app_state_with_repo(repo, repo_id);
            store.replace_snapshot_for_test(Arc::clone(&next_state));
            push_test_state(this, next_state, cx);

            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        unstaged: selection.clone(),
                        unstaged_anchor: Some(a.clone()),
                        ..Default::default()
                    },
                );
                cx.notify();
            });
        });
    });

    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });

    // `a` sits inside the left-click multi-selection; the untracked and staged rows sit
    // outside it. All three must behave the same on right-click.
    let cases = [
        (
            "unstaged",
            0usize,
            gitcomet_core::domain::DiffArea::Unstaged,
            a.clone(),
        ),
        (
            "unstaged",
            2,
            gitcomet_core::domain::DiffArea::Unstaged,
            untracked.clone(),
        ),
        (
            "staged",
            0,
            gitcomet_core::domain::DiffArea::Staged,
            staged.clone(),
        ),
    ];

    for (section_label, ix, area, path) in cases {
        let row_selector = format!("status_row_{}_{}_{}", repo_id.0, section_label, ix);
        let row_bounds = cx
            .debug_bounds(Box::leak(row_selector.clone().into_boxed_str()))
            .unwrap_or_else(|| panic!("expected status row {row_selector} to be rendered"));
        let row_center = row_bounds.center();
        cx.simulate_mouse_move(row_center, None, gpui::Modifiers::default());
        test_support::redraw(cx);
        let action_selector = format!("status_row_action_{}_{}_{}", repo_id.0, section_label, ix);
        let action_bounds = cx
            .debug_bounds(Box::leak(action_selector.into_boxed_str()))
            .expect("expected the hovered row action to be rendered");
        assert_eq!(
            action_bounds.right(),
            row_bounds.right(),
            "the per-file stage action must be flush with the row's right edge"
        );
        cx.simulate_mouse_down(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );
        cx.simulate_mouse_up(
            row_center,
            gpui::MouseButton::Right,
            gpui::Modifiers::default(),
        );

        cx.run_until_parked();

        let diff_target = store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .and_then(|repo| repo.diff_state.diff_target.clone());
        let (popover_kind, multi_selection) = cx.update(|_window, app| {
            let view = view.read(app);
            let popover_kind = view.popover_host.read(app).popover_kind_for_tests();
            let multi_selection = view
                .details_pane
                .read(app)
                .status_multi_selection
                .get(&repo_id)
                .map(|sel| {
                    sel.selected_paths_for_area(gitcomet_core::domain::DiffArea::Unstaged)
                        .to_vec()
                });
            (popover_kind, multi_selection)
        });

        assert_eq!(
            popover_kind,
            Some(PopoverKind::StatusFileMenu {
                repo_id,
                area,
                path: path.clone(),
            }),
            "right-clicking {row_selector} must open that row's file context menu"
        );
        assert_eq!(
            diff_target,
            Some(initial_target.clone()),
            "right-clicking {row_selector} must not open the file diff"
        );
        assert_eq!(
            multi_selection,
            Some(selection.clone()),
            "right-clicking {row_selector} must not change the left-click selection"
        );

        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.popover_host.update(cx, |host, cx| {
                    host.close_popover(cx);
                });
            });
        });
        cx.run_until_parked();
        cx.update(|window, app| {
            window.refresh();
            let _ = window.draw(app);
        });
    }
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
            is_submodule: false,
            additions: None,
            deletions: None,
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
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
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
fn commit_details_file_controls_render_filter_and_open_the_sort_menu(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(611);
    let commit_id = gitcomet_core::domain::CommitId("0123456789abcdef".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-controls"));
            repo.history_state.selected_commit = Some(commit_id.clone());
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![
                        gitcomet_core::domain::CommitFileChange {
                            path: "src/modified.rs".into(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            is_submodule: false,
                            additions: Some(1),
                            deletions: Some(2),
                        },
                        gitcomet_core::domain::CommitFileChange {
                            path: "src/removed.rs".into(),
                            kind: gitcomet_core::domain::FileStatusKind::Deleted,
                            is_submodule: false,
                            additions: Some(0),
                            deletions: Some(4),
                        },
                        gitcomet_core::domain::CommitFileChange {
                            path: "src/added.rs".into(),
                            kind: gitcomet_core::domain::FileStatusKind::Added,
                            is_submodule: false,
                            additions: Some(5),
                            deletions: Some(0),
                        },
                        gitcomet_core::domain::CommitFileChange {
                            path: "src/renamed.rs".into(),
                            kind: gitcomet_core::domain::FileStatusKind::Renamed,
                            is_submodule: false,
                            additions: Some(0),
                            deletions: Some(0),
                        },
                        gitcomet_core::domain::CommitFileChange {
                            path: "src/added-small.rs".into(),
                            kind: gitcomet_core::domain::FileStatusKind::Added,
                            is_submodule: false,
                            additions: Some(1),
                            deletions: Some(0),
                        },
                    ],
                },
            ));
            repo.history_state.commit_details_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert!(cx.debug_bounds("commit_file_sort_button").is_some());
    assert!(cx.debug_bounds("commit_file_filter_tabs").is_some());
    for selector in [
        "commit_file_filter_tab_0",
        "commit_file_filter_tab_1",
        "commit_file_filter_tab_2",
        "commit_file_filter_tab_3",
        "commit_file_filter_tab_4",
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "expected {selector} to be rendered"
        );
    }

    let added = cx
        .debug_bounds("commit_file_filter_tab_3")
        .expect("expected Added filter tab");
    cx.simulate_click(added.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_file_filter,
            crate::view::rows::CommitFileFilter::Added
        );
        assert_eq!(
            pane.active_commit_file_source_indices(repo_id)
                .expect("expected commit-file projection")
                .as_ref(),
            &[4, 2]
        );
    });

    let sort = cx
        .debug_bounds("commit_file_sort_button")
        .expect("expected commit file sort button");
    cx.simulate_click(sort.center(), gpui::Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        cx.update(|_window, app| view
            .read(app)
            .popover_host
            .read(app)
            .popover_kind_for_tests()),
        Some(PopoverKind::CommitFileSortMenu {
            list: crate::view::rows::FileListId::CommitFiles
        })
    );

    draw_and_drain_test_window(cx);
    let largest = cx
        .debug_bounds("context_menu_edit_size_largest")
        .expect("expected Edit size: Largest menu entry");
    cx.simulate_click(largest.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_file_sort,
            crate::view::rows::CommitFileSort::EditSizeDescending
        );
        assert_eq!(
            pane.commit_file_filter,
            crate::view::rows::CommitFileFilter::Added,
            "changing sort keeps the selected filter"
        );
        assert_eq!(
            pane.active_commit_file_source_indices(repo_id)
                .expect("expected sorted commit-file projection")
                .as_ref(),
            &[2, 4]
        );
    });
}

#[gpui::test]
fn commit_file_filter_resets_on_commit_switch_while_sort_is_retained(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(612);
    let make_state = |suffix: &str| {
        let commit_id = gitcomet_core::domain::CommitId(format!("commit-{suffix}").into());
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-state"));
        repo.history_state.selected_commit = Some(commit_id.clone());
        repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
            gitcomet_core::domain::CommitDetails {
                id: commit_id,
                message: "subject".to_string(),
                author_name: String::new(),
                author_email: String::new(),
                authored_at_unix: 0,
                committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                committed_at_unix: 0,
                parent_ids: vec![],
                files: vec![],
            },
        ));
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state("a"), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_file_sort(
                    crate::view::rows::CommitFileSort::EditSizeDescending,
                    cx,
                );
                pane.set_commit_file_filter(crate::view::rows::CommitFileFilter::Removed, cx);
            });
        });
    });
    cx.update(|_window, app| {
        view.update(app, |this, cx| push_test_state(this, make_state("b"), cx));
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.commit_file_sort,
            crate::view::rows::CommitFileSort::EditSizeDescending
        );
        assert_eq!(
            pane.commit_file_filter,
            crate::view::rows::CommitFileFilter::All
        );
    });
}

/// Commit-diff file navigation is rendered by the cached main pane but derives
/// its endpoints from the details pane's active file projection. Guard that
/// cross-entity invalidation boundary for both ways the projection can change.
#[gpui::test]
fn commit_file_projection_changes_notify_the_cached_main_pane(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let (details_pane, main_pane) = cx.update(|_window, app| {
        let view = view.read(app);
        (view.details_pane.clone(), view.main_pane.clone())
    });

    let main_notifies = Arc::new(AtomicUsize::new(0));
    let _main_notify_subscription = cx.update(|_window, app| {
        let main_notifies = Arc::clone(&main_notifies);
        main_pane.update(app, |_pane, cx| {
            cx.observe_self(move |_pane, _cx| {
                main_notifies.fetch_add(1, Ordering::Relaxed);
            })
        })
    });
    cx.run_until_parked();

    main_notifies.store(0, Ordering::Relaxed);
    cx.update(|_window, app| {
        details_pane.update(app, |pane, cx| {
            pane.set_commit_file_filter(crate::view::rows::CommitFileFilter::Added, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        main_notifies.load(Ordering::Relaxed) > 0,
        "filtering the commit file projection must invalidate the cached main pane"
    );

    main_notifies.store(0, Ordering::Relaxed);
    cx.update(|_window, app| {
        details_pane.update(app, |pane, cx| {
            pane.set_commit_file_sort(crate::view::rows::CommitFileSort::EditSizeDescending, cx);
        });
    });
    cx.run_until_parked();
    assert!(
        main_notifies.load(Ordering::Relaxed) > 0,
        "sorting the commit file projection must invalidate the cached main pane"
    );
}

#[gpui::test]
fn ui_scale_commit_details_file_list_content_height_scales(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(62);
    let commit_sha = "fedcba9876543210fedcba9876543210fedcba98".to_string();
    let files = (0..48)
        .map(|ix| gitcomet_core::domain::CommitFileChange {
            path: std::path::PathBuf::from(format!("src/commit_zoom/dir_{ix}/file_{ix}.rs")),
            kind: gitcomet_core::domain::FileStatusKind::Modified,
            is_submodule: false,
            additions: None,
            deletions: None,
        })
        .collect::<Vec<_>>();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-files-zoom"));
            repo.history_state.selected_commit =
                Some(gitcomet_core::domain::CommitId(commit_sha.clone().into()));
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: gitcomet_core::domain::CommitId(commit_sha.clone().into()),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
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

    cx.simulate_resize(gpui::size(px(1024.0), px(420.0)));
    draw_and_drain_test_window(cx);

    let default_contents_height = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let item_size = pane
            .commit_files_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected commit details files list measurements at the default zoom");
        let height: f32 = item_size.contents.height.into();
        height
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.apply_ui_scale_percent(200, window, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let zoomed_contents_height = cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let item_size = pane
            .commit_files_scroll
            .0
            .borrow()
            .last_item_size
            .expect("expected commit details files list measurements after zooming");
        let height: f32 = item_size.contents.height.into();
        height
    });

    assert!(
        zoomed_contents_height > default_contents_height * 1.7,
        "expected the commit details file list content height to grow substantially with zoom (default={default_contents_height}, zoomed={zoomed_contents_height})",
    );
}

#[gpui::test]
fn details_row_renderers_begin_separate_alignment_groups_for_status_and_commit_files(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(631);
    let commit_id = gitcomet_core::domain::CommitId("0123456789abcdef".into());

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo =
                opening_repo_state(repo_id, Path::new("/tmp/repo-details-path-alignment"));
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "staged/really_long_directory_name/files/staged_alpha.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "staged/another_super_long_directory_name/files/staged_beta.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "src/components/really_long_directory_name/status/file_name_alpha.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(
                                "src/components/dir/another_super_long_directory_name/file_name_beta.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            repo.status_rev = repo.status_rev.wrapping_add(1);
            repo.history_state.selected_commit = Some(commit_id.clone());
            repo.history_state.selected_commit_rev =
                repo.history_state.selected_commit_rev.wrapping_add(1);
            repo.history_state.commit_details = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::CommitDetails {
                    id: commit_id.clone(),
                    message: "subject".to_string(),
                    author_name: String::new(),
                    author_email: String::new(),
                    authored_at_unix: 0,
                    committed_at: "2026-03-08 12:34:56 +0200".to_string(),
                    committed_at_unix: 0,
                    parent_ids: vec![],
                    files: vec![
                        gitcomet_core::domain::CommitFileChange {
                            path: std::path::PathBuf::from(
                                "history/really_long_commit_directory_name/files/commit_file_alpha.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            is_submodule: false,
                            additions: None,
                            deletions: None,
                        },
                        gitcomet_core::domain::CommitFileChange {
                            path: std::path::PathBuf::from(
                                "history/dir/another_super_long_commit_directory_name/commit_file_beta.rs",
                            ),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            is_submodule: false,
                            additions: None,
                            deletions: None,
                        },
                    ],
                },
            ));
            repo.history_state.commit_details_rev =
                repo.history_state.commit_details_rev.wrapping_add(1);

            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });

    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            let unstaged =
                crate::view::panes::DetailsPaneView::render_unstaged_rows(pane, 0..2, window, cx);
            let staged =
                crate::view::panes::DetailsPaneView::render_staged_rows(pane, 0..2, window, cx);
            let commit_files = crate::view::panes::DetailsPaneView::render_commit_file_rows(
                pane,
                0..2,
                window,
                cx,
            );

            assert_eq!(unstaged.len(), 2);
            assert_eq!(staged.len(), 2);
            assert_eq!(commit_files.len(), 2);
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let staged = pane.staged_path_alignment_group.snapshot_for_test();
        let unstaged = pane.unstaged_path_alignment_group.snapshot_for_test();
        let commit_files = pane.commit_files_path_alignment_group.snapshot_for_test();
        let untracked = pane.untracked_path_alignment_group.snapshot_for_test();

        assert!(staged.visible_signature.is_some());
        assert!(unstaged.visible_signature.is_some());
        assert!(commit_files.visible_signature.is_some());
        assert_eq!(untracked.visible_signature, None);
        assert_ne!(staged.visible_signature, unstaged.visible_signature);
        assert_ne!(unstaged.visible_signature, commit_files.visible_signature);
        assert_ne!(staged.visible_signature, commit_files.visible_signature);
    });
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

fn state_with_recent_commit_message(
    repo_id: gitcomet_state::model::RepoId,
    workdir: &str,
    recent: gitcomet_state::model::Loadable<Arc<Vec<gitcomet_core::domain::RecentCommitMessage>>>,
) -> Arc<AppState> {
    let mut repo = opening_repo_state(repo_id, Path::new(workdir));
    repo.recent_commit_messages_rev = u64::from(!matches!(
        recent,
        gitcomet_state::model::Loadable::NotLoaded
    ));
    repo.recent_commit_messages = recent;
    app_state_with_repo(repo, repo_id)
}

fn recent_messages(
    messages: &[&str],
) -> gitcomet_state::model::Loadable<Arc<Vec<gitcomet_core::domain::RecentCommitMessage>>> {
    gitcomet_state::model::Loadable::Ready(Arc::new(
        messages
            .iter()
            .enumerate()
            .map(|(ix, message)| gitcomet_core::domain::RecentCommitMessage {
                id: gitcomet_core::domain::CommitId(format!("commit{ix}").into()),
                summary: (*message).into(),
                message: (*message).to_string(),
            })
            .collect(),
    ))
}

#[gpui::test]
fn amend_prefills_commit_message_from_previous_commit_when_empty(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(50);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-prefill",
                    recent_messages(&["previous subject\n\nbody"]),
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_amend_enabled(true, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(
            pane.read(app).commit_message_input.read(app).text(),
            "previous subject\n\nbody"
        );
    });
}

#[gpui::test]
fn amend_does_not_overwrite_existing_commit_message(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(51);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-no-overwrite",
                    recent_messages(&["previous subject"]),
                ),
                cx,
            );
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
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_amend_enabled(true, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(
            pane.read(app).commit_message_input.read(app).text(),
            "draft message"
        );
    });
}

#[gpui::test]
fn amend_prefills_commit_message_once_recent_messages_load(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(52);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-deferred",
                    gitcomet_state::model::Loadable::NotLoaded,
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_commit_amend_enabled(true, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(pane.read(app).commit_message_input.read(app).text(), "");
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                state_with_recent_commit_message(
                    repo_id,
                    "/tmp/repo-amend-deferred",
                    recent_messages(&["deferred subject"]),
                ),
                cx,
            );
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.clone();
        assert_eq!(
            pane.read(app).commit_message_input.read(app).text(),
            "deferred subject"
        );
    });
}

#[gpui::test]
fn commit_message_focus_after_initial_draw_accepts_typed_input(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(44);
    let make_state = || {
        let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-message-focus"));
        repo.status = gitcomet_state::model::Loadable::Ready(
            gitcomet_core::domain::RepoStatus {
                staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                    path: std::path::PathBuf::from("staged.txt"),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    conflict: None,
                }]),
                unstaged: std::sync::Arc::new(Vec::new()),
            }
            .into(),
        );
        app_state_with_repo(repo, repo_id)
    };

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            push_test_state(this, make_state(), cx);
        });
        let _ = window.draw(app);
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_input("x");

    let text = cx.update(|window, app| {
        let _ = window.draw(app);
        view.read(app)
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .text()
            .to_string()
    });
    assert_eq!(text, "x");
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
                staged: std::sync::Arc::new(
                    (0..staged_count)
                        .map(|ix| gitcomet_core::domain::FileStatus {
                            path: std::path::PathBuf::from(format!("staged-{ix}.txt")),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        })
                        .collect(),
                ),
                unstaged: std::sync::Arc::new(Vec::new()),
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("unstaged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(vec![
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
                    ]),
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
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
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
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
                    staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                        path: std::path::PathBuf::from("staged.txt"),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: std::sync::Arc::new(
                        (0..30)
                            .map(|ix| gitcomet_core::domain::FileStatus {
                                path: std::path::PathBuf::from(format!("unstaged-{ix}.txt")),
                                kind: gitcomet_core::domain::FileStatusKind::Modified,
                                conflict: None,
                            })
                            .collect(),
                    ),
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

/// Both "Stage all" buttons go through the same helper, so the confirmation
/// cannot be present on one and missing on the other. The split view's button
/// names its section's paths; the combined one passes an empty set meaning
/// "everything". Either way a conflicted file with markers left in it has to
/// stop the stage.
#[gpui::test]
fn stage_all_asks_before_staging_unresolved_conflicts(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let store_for_assertions = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(70615);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_stage_all_conflict_confirm",
        std::process::id()
    ));
    let conflicted = std::path::PathBuf::from("conflicted.rs");
    let clean = std::path::PathBuf::from("clean.rs");
    std::fs::create_dir_all(&workdir).unwrap();
    std::fs::write(
        workdir.join(&conflicted),
        "a\n<<<<<<< HEAD\nours\n=======\ntheirs\n>>>>>>> other\nb\n",
    )
    .unwrap();
    std::fs::write(workdir.join(&clean), "resolved\n").unwrap();

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(vec![]),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: conflicted.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: Some(gitcomet_core::domain::FileConflictKind::BothModified),
                        },
                        gitcomet_core::domain::FileStatus {
                            path: clean.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            let state = app_state_with_repo(repo, repo_id);
            // Both sides: the pane renders from the UI model, while the
            // "nothing was staged" assertion reads the store's own snapshot.
            this.store.replace_snapshot_for_test(Arc::clone(&state));
            push_test_state(this, state, cx);
        });
    });
    draw_and_drain_test_window(cx);

    // The split view's "Stage all": the tracked-changes section by name.
    let split_paths = vec![conflicted.clone(), clean.clone()];
    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.stage_all_with_conflict_confirmation(repo_id, split_paths, window, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(
            kind,
            Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                if unresolved == &vec![conflicted.clone()]
        ),
        "the split view's Stage all must warn about the conflicted file, got {kind:?}"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.popover_host
                .update(cx, |host, cx| host.close_popover(cx));
        });
    });
    draw_and_drain_test_window(cx);

    // The combined view's "Stage all": everything, expressed as an empty set.
    cx.update(|window, app| {
        let details_pane = view.read(app).details_pane.clone();
        details_pane.update(app, |pane, cx| {
            pane.stage_all_with_conflict_confirmation(repo_id, Vec::new(), window, cx);
        });
    });
    draw_and_drain_test_window(cx);

    let kind =
        cx.update(|_window, app| crate::view::test_support::popover_kind(view.read(app), app));
    assert!(
        matches!(
            kind,
            Some(PopoverKind::StageConflictMarkersConfirm { ref unresolved, .. })
                if unresolved == &vec![conflicted.clone()]
        ),
        "the combined view's Stage all must warn about the conflicted file, got {kind:?}"
    );

    assert!(
        store_for_assertions
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .is_some_and(|repo| repo.local_actions_in_flight == 0),
        "nothing may be staged until the confirmation is answered"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The section header's action labels collapse to initials once the details
/// pane is too narrow to hold the full wording. Verified by width because the
/// test text system has no glyph metrics — but it does give every glyph the
/// same advance, so a label with fewer characters is reliably narrower.
#[gpui::test]
fn stage_selected_label_shrinks_when_the_details_pane_is_narrow(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1600.0), px(900.0)));

    let repo_id = gitcomet_state::model::RepoId(63);
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_status_header_shrink",
        std::process::id()
    ));
    let a = std::path::PathBuf::from("a.txt");
    let b = std::path::PathBuf::from("b.txt");

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, &workdir);
            repo.open = gitcomet_state::model::Loadable::Ready(());
            repo.status = gitcomet_state::model::Loadable::Ready(
                gitcomet_core::domain::RepoStatus {
                    staged: std::sync::Arc::new(Vec::new()),
                    unstaged: std::sync::Arc::new(vec![
                        gitcomet_core::domain::FileStatus {
                            path: a.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                        gitcomet_core::domain::FileStatus {
                            path: b.clone(),
                            kind: gitcomet_core::domain::FileStatusKind::Modified,
                            conflict: None,
                        },
                    ]),
                }
                .into(),
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.status_multi_selection.insert(
                    repo_id,
                    StatusMultiSelection {
                        unstaged: vec![a.clone(), b.clone()],
                        unstaged_anchor: Some(a.clone()),
                        ..Default::default()
                    },
                );
                cx.notify();
            });
        });
    });

    let set_details_width = |cx: &mut gpui::VisualTestContext, width: f32| {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.details_width = px(width);
                this.details_render_width = px(width);
                cx.notify();
            });
            window.refresh();
            let _ = window.draw(app);
        });
        // The header reads a width the probe measured while painting, so the
        // switch lands on the frame after the one that resized the pane.
        draw_and_drain_test_window(cx);
    };

    set_details_width(cx, 700.0);
    let wide = cx
        .debug_bounds("stage_selected_button")
        .expect("expected the Stage (n) button while files are selected")
        .size
        .width;

    set_details_width(cx, 300.0);
    let narrow = cx
        .debug_bounds("stage_selected_button")
        .expect("expected the Stage (n) button to survive the narrower pane")
        .size
        .width;

    assert!(
        narrow < wide,
        "expected `Stage (2)` to collapse to `S (2)` in a narrow pane (wide={wide:?}, narrow={narrow:?})"
    );

    let _ = std::fs::remove_dir_all(&workdir);
}

/// The worktree scan revision bumps per repo-wide rescan, not per worktree, so
/// two worktrees with the same file count and the same visible range would share
/// a path-truncation signature if the path were left out of it — and the second
/// one would render with the first one's measured alignment.
#[gpui::test]
fn worktree_file_alignment_signatures_differ_per_worktree(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo_id = gitcomet_state::model::RepoId(4);
        let signature = |path: &str| {
            pane.worktree_files_visible_signature(
                repo_id,
                7,
                std::path::Path::new(path),
                &(0..5),
                5,
            )
        };

        assert_ne!(
            signature("/wt/a"),
            signature("/wt/b"),
            "two worktrees scanned at the same revision must not share a signature"
        );
        assert_eq!(
            signature("/wt/a"),
            signature("/wt/a"),
            "the same worktree keeps its alignment across renders"
        );
    });
}

/// The worktree file list is virtualized, but its inputs are one entry per
/// changed file: building them inline made every layout pass O(all files). They
/// are derived once per scan instead, and keyed by worktree — the scan revision
/// alone bumps for the whole repo, so it cannot tell two worktrees apart.
#[gpui::test]
fn worktree_file_inputs_are_derived_once_per_scan_and_keyed_by_worktree(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let summary = |path: &str, files: usize| gitcomet_core::domain::WorktreeDirtySummary {
        path: std::path::PathBuf::from(path),
        head: Some(gitcomet_core::domain::CommitId("tip".into())),
        branch: Some("side".to_string()),
        detached: false,
        added: files,
        modified: 0,
        deleted: 0,
        staged: (0..files)
            .map(|ix| gitcomet_core::domain::FileStatus {
                path: std::path::PathBuf::from(format!("staged_{ix}.rs")),
                kind: gitcomet_core::domain::FileStatusKind::Added,
                conflict: None,
            })
            .collect(),
        unstaged: (0..files)
            .map(|ix| gitcomet_core::domain::FileStatus {
                path: std::path::PathBuf::from(format!("unstaged_{ix}.rs")),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
        line_stats: Default::default(),
    };

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo_id = gitcomet_state::model::RepoId(9);
        let worktree_a = summary("/wt/a", 3);
        let worktree_b = summary("/wt/b", 3);

        let first = pane.cached_worktree_file_inputs(repo_id, 1, &worktree_a);
        let again = pane.cached_worktree_file_inputs(repo_id, 1, &worktree_a);
        assert!(
            Arc::ptr_eq(&first, &again),
            "a second frame at the same scan revision must reuse the derived inputs"
        );
        assert_eq!(first.files.len(), 6);
        assert_eq!(first.entries.len(), 6);

        // `selected_ix` indexes `entries` by the row's position in `files`, so the
        // two vectors have to stay in step -- staged first, then unstaged, each
        // entry pointing at its own file and carrying the section it came from.
        let staged_then_unstaged: Vec<_> =
            first.files.iter().map(|file| file.path.clone()).collect();
        assert_eq!(
            staged_then_unstaged,
            vec![
                std::path::PathBuf::from("staged_0.rs"),
                std::path::PathBuf::from("staged_1.rs"),
                std::path::PathBuf::from("staged_2.rs"),
                std::path::PathBuf::from("unstaged_0.rs"),
                std::path::PathBuf::from("unstaged_1.rs"),
                std::path::PathBuf::from("unstaged_2.rs"),
            ],
            "staged files come first, in scan order"
        );
        for (ix, entry) in first.entries.iter().enumerate() {
            assert_eq!(
                entry.path, first.files[ix].path,
                "entry {ix} must describe the file rendered at row {ix}"
            );
            let staged = ix < 3;
            assert_eq!(
                entry.section,
                if staged {
                    gitcomet_state::model::InlineSubmoduleDiffSection::LiveStaged
                } else {
                    gitcomet_state::model::InlineSubmoduleDiffSection::LiveUnstaged
                },
                "entry {ix} must open in the section its file was scanned in"
            );
            assert_eq!(
                entry.target,
                gitcomet_core::domain::DiffTarget::WorkingTree {
                    path: first.files[ix].path.clone(),
                    area: if staged {
                        gitcomet_core::domain::DiffArea::Staged
                    } else {
                        gitcomet_core::domain::DiffArea::Unstaged
                    },
                },
                "entry {ix} must diff against the right side of the index"
            );
        }

        // Same repo, same revision, same file count: only the path tells them apart.
        let other = pane.cached_worktree_file_inputs(repo_id, 1, &worktree_b);
        assert!(
            !Arc::ptr_eq(&first, &other),
            "another worktree must not be served the first one's files"
        );

        let rescanned = pane.cached_worktree_file_inputs(repo_id, 2, &worktree_b);
        assert!(
            !Arc::ptr_eq(&other, &rescanned),
            "a new scan revision must rebuild them"
        );
    });
}

fn leaked(selector: String) -> &'static str {
    Box::leak(selector.into_boxed_str())
}

fn commit_details_state_with_paths(
    repo_id: gitcomet_state::model::RepoId,
    commit_id: &gitcomet_core::domain::CommitId,
    paths: &[&str],
    rev: u64,
) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-commit-file-tree"));
    repo.history_state.selected_commit = Some(commit_id.clone());
    repo.history_state.commit_details =
        gitcomet_state::model::Loadable::Ready(Arc::new(gitcomet_core::domain::CommitDetails {
            id: commit_id.clone(),
            message: "subject".to_string(),
            author_name: String::new(),
            author_email: String::new(),
            authored_at_unix: 0,
            committed_at: "2026-03-08 12:34:56 +0200".to_string(),
            committed_at_unix: 0,
            parent_ids: vec![],
            files: paths
                .iter()
                .map(|path| gitcomet_core::domain::CommitFileChange {
                    path: (*path).into(),
                    kind: gitcomet_core::domain::FileStatusKind::Modified,
                    is_submodule: false,
                    additions: Some(1),
                    deletions: Some(1),
                })
                .collect(),
        }));
    repo.history_state.commit_details_rev = rev;
    // The details pane only re-reads a snapshot when its notify fingerprint
    // moves, and that hashes the rev, not the commit id.
    repo.history_state.selected_commit_rev = rev;
    repo
}

#[gpui::test]
fn commit_file_layout_toggle_groups_rows_into_folders(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(613);
    let commit_id = gitcomet_core::domain::CommitId("tree0123456789ab".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(
                repo_id,
                &commit_id,
                &["src/view/a.rs", "src/view/b.rs", "root.rs"],
                1,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
            .is_none(),
        "flat is the default, so no folder rows"
    );

    let toggle = cx
        .debug_bounds("commit_file_layout_button")
        .expect("expected the layout toggle");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_layout_for(repo_id, crate::view::rows::FileListId::CommitFiles),
            crate::view::FileListLayout::Tree
        );
    });
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
            .is_some(),
        "the single-child chain src/view collapses into one folder row"
    );
}

/// Collapsing hides rows; it must not narrow what F1/F4 steps through, or the
/// prev/next arrows vanish the moment a folder is shut.
#[gpui::test]
fn collapsing_a_folder_keeps_every_file_navigable(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(614);
    let commit_id = gitcomet_core::domain::CommitId("collapse0123456a".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(
                repo_id,
                &commit_id,
                &["src/view/a.rs", "src/view/b.rs", "root.rs"],
                1,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    // Drawn before the toggle: the snapshot lands on the next draw, and it is
    // what resets a per-list override.
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let before = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .active_commit_file_source_indices(repo_id)
            .expect("expected a projection")
    });
    assert_eq!(before.len(), 3);

    let folder = cx
        .debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
        .expect("expected a folder row");
    cx.simulate_click(folder.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    let after = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .active_commit_file_source_indices(repo_id)
            .expect("expected a projection")
    });
    assert_eq!(
        after.as_ref(),
        before.as_ref(),
        "collapsing changes which rows render, not which files navigate"
    );
    // Rows are [folder, root.rs] now; before the collapse they were
    // [folder, a.rs, b.rs, root.rs].
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_{}_3", repo_id.0)))
            .is_none(),
        "the collapsed folder's children stop rendering"
    );
}

/// The override is scoped to the commit it was made on.
#[gpui::test]
fn commit_file_layout_override_resets_on_commit_switch(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(615);
    let first = gitcomet_core::domain::CommitId("first01234567890".into());
    let second = gitcomet_core::domain::CommitId("second1234567890".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(repo_id, &first, &["src/view/a.rs"], 1);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_layout_for(repo_id, crate::view::rows::FileListId::CommitFiles),
            crate::view::FileListLayout::Tree,
            "the override applies while the commit is unchanged"
        );
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = commit_details_state_with_paths(repo_id, &second, &["src/view/a.rs"], 2);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_layout_for(repo_id, crate::view::rows::FileListId::CommitFiles),
            crate::view::FileListLayout::Flat,
            "a different commit re-reads the global default"
        );
    });
}

fn repo_with_unstaged_paths(
    repo_id: gitcomet_state::model::RepoId,
    paths: &[&str],
) -> gitcomet_state::model::RepoState {
    let mut repo = opening_repo_state(repo_id, Path::new("/tmp/repo-status-tree"));
    repo.worktree_status = gitcomet_state::model::Loadable::Ready(Arc::new(
        paths
            .iter()
            .map(|path| gitcomet_core::domain::FileStatus {
                path: (*path).into(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            })
            .collect(),
    ));
    repo.worktree_status_rev = 1;
    repo
}

#[gpui::test]
fn status_layout_toggle_groups_rows_into_folders(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(620);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo =
                repo_with_unstaged_paths(repo_id, &["src/view/a.rs", "src/view/b.rs", "root.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    let dir_selector = leaked(format!("status_dir_{}_unstaged_0", repo_id.0));
    assert!(
        cx.debug_bounds(dir_selector).is_none(),
        "flat is the default"
    );

    let toggle = cx
        .debug_bounds("status_unstaged_layout_button")
        .expect("expected the unstaged layout toggle");
    cx.simulate_click(toggle.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds(dir_selector).is_some(),
        "src/view folds into one folder row"
    );
}

/// The anchor index is a position in the display order. Sorting reorders the
/// list without touching `status_rev`, so an index hint captured under the old
/// order must not be trusted — it would silently select the wrong range.
#[gpui::test]
fn status_shift_click_range_follows_the_sorted_display_order(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(621);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["a.rs", "b.rs", "c.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_status_file_sort(
                    StatusSection::CombinedUnstaged,
                    crate::view::rows::CommitFileSort::PathDescending,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    // Displayed order is now c, b, a.
    let first = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_0", repo_id.0)))
        .expect("expected the first status row");
    cx.simulate_click(first.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    let third = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_2", repo_id.0)))
        .expect("expected the third status row");
    cx.simulate_click(
        third.center(),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let selected =
            pane.status_selected_paths_for_area(repo_id, gitcomet_core::domain::DiffArea::Unstaged);
        let mut names: Vec<String> = selected
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()],
            "shift-clicking row 0 to row 2 spans the whole displayed range"
        );
    });
}

/// The folder covers its whole subtree, nested collapsed folders included.
#[gpui::test]
fn status_folder_stage_covers_the_subtree_and_ignores_the_selection(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(640);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(
                repo_id,
                &["src/nested/deep.rs", "src/shallow.rs", "docs/other.md"],
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    // No selection input at all, so `docs/other.md` cannot leak in.
    let paths = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_folder_subtree_paths(
                repo_id,
                StatusSection::CombinedUnstaged,
                std::path::Path::new("src"),
            )
    });

    let mut names: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
    names.sort();
    assert_eq!(
        names,
        vec![
            "src/nested/deep.rs".to_string(),
            "src/shallow.rs".to_string()
        ],
        "the folder covers its whole subtree and nothing outside it"
    );
}

/// `Path::starts_with` is component-wise, so a sibling sharing a name prefix
/// must not be swept in.
#[gpui::test]
fn status_folder_paths_do_not_capture_a_name_prefixed_sibling(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(641);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["src/a.rs", "src2/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    let paths = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .status_folder_subtree_paths(
                repo_id,
                StatusSection::CombinedUnstaged,
                std::path::Path::new("src"),
            )
    });
    assert_eq!(
        paths,
        vec![std::path::PathBuf::from("src/a.rs")],
        "src must not capture src2"
    );
}

/// Folder actions must use the original path even when display labels collide.
#[cfg(unix)]
#[gpui::test]
fn status_folder_actions_keep_non_utf8_directory_paths(cx: &mut gpui::TestAppContext) {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = gitcomet_state::model::RepoId(642);
    let paths = [
        std::path::PathBuf::from(OsStr::from_bytes(b"bad\xff/nested/a.rs")),
        std::path::PathBuf::from(OsStr::from_bytes(b"bad\xfe/nested/b.rs")),
    ];
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = repo_with_unstaged_paths(repo_id, &[]);
            let entries = Arc::new(
                paths
                    .iter()
                    .map(|path| gitcomet_core::domain::FileStatus {
                        path: path.clone(),
                        kind: gitcomet_core::domain::FileStatusKind::Modified,
                        conflict: None,
                    })
                    .collect(),
            );
            repo.worktree_status = Loadable::Ready(Arc::clone(&entries));
            repo.staged_status = Loadable::Ready(entries);
            repo.staged_status_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
            this.details_pane.update(cx, |pane, cx| {
                pane.set_file_list_layout(crate::view::FileListLayout::Tree, cx);
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo = pane.active_repo().expect("repo");
        for section in [StatusSection::CombinedUnstaged, StatusSection::Staged] {
            let plan = pane.status_file_plan(repo, section);
            let keys: Vec<_> = (0..plan.row_len())
                .filter_map(|ix| match plan.row_at(crate::view::rows::RowIx(ix)) {
                    Some(crate::view::rows::FileListRow::Directory { key, .. }) => Some(key),
                    _ => None,
                })
                .collect();
            assert_eq!(keys.len(), 2, "each raw directory needs its own action");
            for path in &paths {
                let key = keys
                    .iter()
                    .find(|key| key.as_ref() == path.parent().unwrap())
                    .expect("folder action retains the raw path");
                assert_eq!(
                    pane.status_folder_subtree_paths(repo_id, section, key),
                    vec![path.clone()],
                );
            }
        }
    });
}

/// Counts arrive on their own effect, after the list is drawn and without
/// `worktree_status_rev` moving. Keyed on the status rev alone, the cache would
/// keep serving pre-stats rows until some unrelated change shifted the key.
#[gpui::test]
fn status_rows_pick_up_line_stats_that_arrive_without_a_status_change(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(650);
    let paths = ["src/a.rs", "src/b.rs"];

    let folder_additions = |cx: &mut gpui::VisualTestContext| -> Option<u64> {
        cx.update(|_window, app| {
            let pane = view.read(app).details_pane.read(app);
            let repo = pane.active_repo().expect("active repo");
            let plan = pane.status_file_plan(repo, StatusSection::CombinedUnstaged);
            match plan.row_at(crate::view::rows::RowIx(0)) {
                Some(crate::view::rows::FileListRow::Directory { additions, .. }) => additions,
                other => panic!("expected a folder row, got {other:?}"),
            }
        })
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(repo_with_unstaged_paths(repo_id, &paths), repo_id),
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);
    assert_eq!(folder_additions(cx), None, "no counts before they load");

    // Same status, same rev — only the stats lane moves.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let mut repo = repo_with_unstaged_paths(repo_id, &paths);
            let mut unstaged = rustc_hash::FxHashMap::default();
            for (ix, path) in paths.iter().enumerate() {
                unstaged.insert(
                    std::path::PathBuf::from(path),
                    gitcomet_core::domain::LineStats {
                        additions: Some(ix as u32 + 1),
                        deletions: Some(1),
                    },
                );
            }
            repo.uncommitted_line_stats = gitcomet_state::model::Loadable::Ready(Arc::new(
                gitcomet_core::domain::UncommittedLineStats {
                    staged: Default::default(),
                    unstaged,
                },
            ));
            repo.unstaged_line_stats_rev = 1;
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    assert_eq!(
        folder_additions(cx),
        Some(3),
        "the cached tree must be rebuilt when only the line-stats rev moves"
    );
}

/// A tree hoists directories above files, so a range computed in ordinal space
/// skips files displayed between the clicks — and those files then get staged.
#[gpui::test]
fn status_shift_click_in_tree_layout_spans_the_displayed_rows(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(660);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            // Projection 0,1,2 = root.rs, a, b; tree order [src/, a, b, root.rs].
            let repo = repo_with_unstaged_paths(repo_id, &["root.rs", "src/a.rs", "src/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let row = |cx: &mut gpui::VisualTestContext, ix: usize| {
        cx.debug_bounds(leaked(format!("status_row_{}_unstaged_{ix}", repo_id.0)))
            .unwrap_or_else(|| panic!("expected status row {ix}"))
    };

    // Row 1 is src/a.rs, row 3 is root.rs, with src/b.rs displayed between them.
    let first = row(cx, 1);
    cx.simulate_click(first.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    let last = row(cx, 3);
    cx.simulate_click(
        last.center(),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let mut names: Vec<String> = pane
            .status_selected_paths_for_area(repo_id, gitcomet_core::domain::DiffArea::Unstaged)
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "root.rs".to_string(),
                "src/a.rs".to_string(),
                "src/b.rs".to_string()
            ],
            "the range must cover every row displayed between the two clicks"
        );
    });
}

/// Sorting reorders rows without moving any status or line-stats rev, so an
/// anchor captured under the old order must not be trusted.
#[gpui::test]
fn status_shift_click_drops_an_anchor_invalidated_by_a_sort_change(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(670);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["a.rs", "b.rs", "c.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);

    // Anchor a.rs at row 0.
    let first = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_0", repo_id.0)))
        .expect("expected the first row");
    cx.simulate_click(first.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    // Z-A puts a.rs at row 2; no status or stats rev moves.
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_status_file_sort(
                    StatusSection::CombinedUnstaged,
                    crate::view::rows::CommitFileSort::PathDescending,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let same_file_now_last = cx
        .debug_bounds(leaked(format!("status_row_{}_unstaged_2", repo_id.0)))
        .expect("expected the third row");
    cx.simulate_click(
        same_file_now_last.center(),
        gpui::Modifiers {
            shift: true,
            ..Default::default()
        },
    );
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let names: Vec<String> = pane
            .status_selected_paths_for_area(repo_id, gitcomet_core::domain::DiffArea::Unstaged)
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        assert_eq!(
            names,
            vec!["a.rs".to_string()],
            "shift-clicking the anchor's own row selects only it, whatever the sort"
        );
    });
}

/// Drives the folder Stage button on a tree-layout Unstaged section and reports
/// whether the store accepted a staging command.
fn folder_stage_button_stages(
    cx: &mut gpui::TestAppContext,
    repo_id: gitcomet_state::model::RepoId,
    button: gpui::MouseButton,
) -> bool {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["src/a.rs", "src/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let folder = cx
        .debug_bounds(leaked(format!("status_dir_{}_unstaged_0", repo_id.0)))
        .expect("expected a folder row");
    // The action is revealed on hover, so the pointer has to be on the row.
    cx.simulate_mouse_move(folder.center(), None, gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    let action = cx
        .debug_bounds(leaked(format!(
            "status_dir_action_{}_unstaged_0",
            repo_id.0
        )))
        .expect("expected the folder action");
    let at = action.center();
    cx.simulate_mouse_down(at, button, gpui::Modifiers::default());
    cx.simulate_mouse_up(at, button, gpui::Modifiers::default());
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.read(app)
            .store
            .snapshot()
            .repos
            .first()
            .is_some_and(|repo| repo.local_actions_in_flight > 0)
    })
}

/// A left click stages the folder — the positive control, so the right-click
/// assertion below cannot pass just because the button was never hit.
#[gpui::test]
fn folder_stage_button_stages_on_a_left_click(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    assert!(folder_stage_button_stages(
        cx,
        gitcomet_state::model::RepoId(680),
        gpui::MouseButton::Left
    ));
}

/// A right click opens the context menu; it must not stage the subtree.
#[gpui::test]
fn folder_stage_button_ignores_a_right_click(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    assert!(!folder_stage_button_stages(
        cx,
        gitcomet_state::model::RepoId(681),
        gpui::MouseButton::Right
    ));
}

/// A "Renamed" filter carried to a worktree with no renames would empty the
/// list under a header still counting changes.
#[gpui::test]
fn worktree_filter_resets_when_the_shown_worktree_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(690);
    let list = crate::view::rows::FileListId::WorktreeFiles;
    let with_selection = |path: &str, rev: u64| {
        let mut repo = repo_with_unstaged_paths(repo_id, &["a.rs"]);
        repo.history_state.worktree_selection = Some(std::path::PathBuf::from(path));
        repo.history_state.worktree_selection_rev = rev;
        repo
    };

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(with_selection("/tmp/wt-a", 1), repo_id),
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.set_file_list_filter(list, crate::view::rows::CommitFileFilter::Renamed, cx);
                assert_eq!(
                    pane.file_list_filter_for(list),
                    crate::view::rows::CommitFileFilter::Renamed,
                    "the filter applies to the repo it was set on"
                );
            });
        });
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            push_test_state(
                this,
                app_state_with_repo(with_selection("/tmp/wt-b", 2), repo_id),
                cx,
            );
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        assert_eq!(
            pane.file_list_filter_for(list),
            crate::view::rows::CommitFileFilter::All,
            "a different worktree must not inherit the previous one's filter"
        );
    });
}

/// Navigation hands over a position among files, but `scroll_to_item_strict`
/// indexes rows — which a tree pads with directories and may be hiding.
#[gpui::test]
fn commit_file_scroll_resolves_a_file_position_to_its_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(700);
    let commit_id = gitcomet_core::domain::CommitId("scroll0123456789".into());
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            // Tree order [src/, a, b, root.rs]: file 0 is row 1, file 2 row 3.
            let repo = commit_details_state_with_paths(
                repo_id,
                &commit_id,
                &["root.rs", "src/a.rs", "src/b.rs"],
                1,
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::CommitFiles,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    let row_for = |cx: &mut gpui::VisualTestContext, position: usize| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.details_pane
                    .update(cx, |pane, cx| pane.reveal_commit_file_row(position, cx))
            })
        })
    };

    assert_eq!(
        row_for(cx, 0),
        Some(1),
        "the first file sits below its folder"
    );
    assert_eq!(row_for(cx, 2), Some(3));

    // Collapsed, so its files have no row until the reveal reopens it.
    let folder = cx
        .debug_bounds(leaked(format!("commit_file_dir_{}_0", repo_id.0)))
        .expect("expected the folder row");
    cx.simulate_click(folder.center(), gpui::Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds(leaked(format!("commit_file_{}_2", repo_id.0)))
            .is_none(),
        "the folder is collapsed"
    );

    assert_eq!(
        row_for(cx, 0),
        Some(1),
        "navigating to a hidden file reopens its folder rather than scrolling to a stranger"
    );
}

/// Prev/next-file must step through the rows as drawn. The order navigation
/// walks comes from `active_status_section_order`, which is the projection —
/// and a tree reorders that.
#[gpui::test]
fn status_navigation_order_matches_the_drawn_rows_in_a_tree(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = gitcomet_state::model::RepoId(710);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            let repo = repo_with_unstaged_paths(repo_id, &["root.rs", "src/a.rs", "src/b.rs"]);
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.toggle_file_list_layout(
                    repo_id,
                    crate::view::rows::FileListId::Status(StatusSection::CombinedUnstaged),
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).details_pane.read(app);
        let repo = pane.active_repo().expect("active repo");
        let entries = repo.worktree_status_entries().expect("status entries");
        let navigated: Vec<&std::path::Path> = pane
            .active_status_section_order(repo_id, StatusSection::CombinedUnstaged)
            .expect("nav order")
            .iter()
            .filter_map(|source_ix| entries.get(*source_ix).map(|entry| entry.path.as_path()))
            .collect();
        let drawn = pane.status_display_order_paths(repo_id, StatusSection::CombinedUnstaged);

        assert_eq!(
            navigated,
            drawn.iter().map(|path| path.as_path()).collect::<Vec<_>>(),
            "prev/next must visit files in the order the rows are drawn"
        );
        assert_eq!(drawn.len(), 3, "and the comparison is not two empty lists");
        for (position, path) in drawn.iter().enumerate() {
            assert_eq!(
                pane.status_path_display_position(StatusSection::CombinedUnstaged, path),
                Some(position)
            );
        }
        assert_eq!(
            pane.status_path_display_position(
                StatusSection::CombinedUnstaged,
                std::path::Path::new("missing")
            ),
            None
        );
    });
}

/// A press on something that takes no focus of its own must still blur a
/// focused input, the way clicking page background does in a browser.
#[gpui::test]
fn clicking_outside_a_focused_input_blurs_it(cx: &mut gpui::TestAppContext) {
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);

    let input = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .clone()
    });
    cx.update(|window, app| {
        let handle = input.read(app).focus_handle();
        handle.focus(window, app);
    });
    cx.run_until_parked();
    assert!(
        cx.update(|window, app| input.read(app).focus_handle().is_focused(window)),
        "precondition: the input is focused"
    );

    // The branch sidebar: a real surface that takes no focus of its own, so
    // without an explicit blur the input stays focused behind the user's back.
    let size = cx.update(|window, _app| window.viewport_size());
    let sidebar = gpui::point(size.width * 0.05, size.height * 0.5);
    cx.simulate_mouse_move(sidebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: sidebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    assert!(
        cx.update(|window, app| !input.read(app).focus_handle().is_focused(window)),
        "a press on a surface that takes no focus must still blur the input"
    );
    // Deliberately nothing focused rather than a fallback surface: the press
    // landed on something that is not a pane, so handing focus to one would
    // silently redirect the user's next keystroke. Global bindings still
    // resolve at the window root.
    assert!(
        cx.update(|window, app| window.focused(app).is_none()),
        "blur leaves no element focused"
    );
}

/// ...but a selection-neutral gesture must not. Dragging a scrollbar or the
/// titlebar while typing is not "clicking away".
#[gpui::test]
fn a_preserved_press_does_not_blur_a_focused_input(cx: &mut gpui::TestAppContext) {
    let (view, cx, _sha, _date, _parent) = commit_details_metadata_fixture(cx);

    let input = cx.update(|_window, app| {
        view.read(app)
            .details_pane
            .read(app)
            .commit_details_sha_input
            .clone()
    });
    cx.update(|window, app| {
        let handle = input.read(app).focus_handle();
        handle.focus(window, app);
    });
    cx.run_until_parked();

    // The titlebar: a real `preserve` site.
    let titlebar = gpui::point(gpui::px(2.0), gpui::px(2.0));
    cx.simulate_mouse_move(titlebar, None, gpui::Modifiers::default());
    cx.simulate_event(gpui::MouseDownEvent {
        position: titlebar,
        modifiers: gpui::Modifiers::default(),
        button: gpui::MouseButton::Left,
        click_count: 1,
        first_mouse: false,
    });
    cx.run_until_parked();

    assert!(
        cx.update(|window, app| input.read(app).focus_handle().is_focused(window)),
        "dragging the window must not blur the input being typed in"
    );
}

/// Menu-opening chips across the workspace: click targets that stayed fixed
/// pills while Comfortable grew everything around them. The title bar is the
/// deliberate exception -- it shares its row with OS window controls that never
/// resize, so its own chips must not move.
#[gpui::test]
fn comfortable_chrome_chips_grow_with_the_density(cx: &mut gpui::TestAppContext) {
    use crate::appearance::{Appearance, UiDensity};

    fn heights(
        cx: &mut gpui::VisualTestContext,
        selectors: &[&'static str],
        density: UiDensity,
    ) -> Vec<gpui::Pixels> {
        selectors
            .iter()
            .map(|selector| {
                cx.debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector} at {density:?} density"))
                    .size
                    .height
            })
            .collect()
    }

    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(83);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/comfortable-chips"));
            repo.open = Loadable::Ready(());
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        })
    });

    let selectors = [
        "sidebar_tab_branches",
        "sidebar_tab_files",
        "history_mode_header",
        "change_tracking_unstaged_header",
    ];
    // Everything in the title bar. `add_repo_menu` and the app menu are the two
    // `Button::unscaled()` call sites, and `add_repo_menu` is the one whose
    // height actually comes from that pin -- the others carry explicit sizes.
    let leak = |selector: String| -> &'static str { Box::leak(selector.into_boxed_str()) };
    let tab_selector = leak(format!("repo_tab_{}", repo_id.0));
    let plate_selector = leak(format!("repo_tab_hover_box_{}", repo_id.0));
    let mut fixed_selectors = vec![
        "repo_picker_toggle",
        "add_repo_menu",
        tab_selector,
        plate_selector,
    ];
    // macOS supplies its own application menu and caption buttons.
    if !cfg!(target_os = "macos") {
        fixed_selectors.extend(["app_menu", "titlebar_win_close"]);
    }

    let mut per_density: Vec<Vec<gpui::Pixels>> = Vec::new();
    let mut fixed_per_density: Vec<Vec<gpui::Pixels>> = Vec::new();

    for density in UiDensity::ALL {
        cx.update(|_, app| {
            app.set_global(Appearance {
                density,
                ..Appearance::default()
            });
            view.update(app, |this, cx| {
                this.notify_font_preferences_changed(cx);
            });
        });
        draw_and_drain_test_window(cx);
        per_density.push(heights(cx, &selectors, density));
        fixed_per_density.push(heights(cx, &fixed_selectors, density));

        // The plate behind the label and the tab around it are both fixed, so
        // the plate has to stay inside its tab at every density.
        let tab = cx
            .debug_bounds(tab_selector)
            .expect("expected repository tab bounds");
        let plate = cx
            .debug_bounds(plate_selector)
            .expect("expected repository tab label plate bounds");

        assert!(
            plate.top() > tab.top() && plate.bottom() < tab.bottom(),
            "{density:?}: the label plate {plate:?} must stay inside its tab {tab:?}"
        );
    }

    for (step, pair) in per_density.windows(2).enumerate() {
        for (ix, selector) in selectors.into_iter().enumerate() {
            assert!(
                pair[1][ix] > pair[0][ix],
                "{selector} must grow from {:?} to {:?}, stayed {:?}",
                UiDensity::ALL[step],
                UiDensity::ALL[step + 1],
                pair[1][ix]
            );
        }
    }

    for (step, pair) in fixed_per_density.windows(2).enumerate() {
        for (ix, selector) in fixed_selectors.iter().enumerate() {
            assert_eq!(
                pair[1][ix],
                pair[0][ix],
                "{selector} sits in the title bar and must not move from {:?} to {:?}",
                UiDensity::ALL[step],
                UiDensity::ALL[step + 1],
            );
        }
    }
}

#[gpui::test]
fn comfortable_stage_and_commit_targets_fit_rows_at_laptop_and_4k_sizes(
    cx: &mut gpui::TestAppContext,
) {
    use crate::appearance::{Appearance, UiDensity};
    use gitcomet_core::domain::{FileStatus, FileStatusKind, RepoStatus};
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(79);
    cx.update(|_, app| {
        view.update(app, |this, cx| {
            let mut repo = opening_repo_state(repo_id, Path::new("/tmp/comfortable-targets"));
            repo.open = Loadable::Ready(());
            repo.status = Loadable::Ready(
                RepoStatus {
                    staged: Arc::new(vec![FileStatus {
                        path: "staged.txt".into(),
                        kind: FileStatusKind::Modified,
                        conflict: None,
                    }]),
                    unstaged: Arc::new(vec![FileStatus {
                        path: "unstaged.txt".into(),
                        kind: FileStatusKind::Modified,
                        conflict: None,
                    }]),
                }
                .into(),
            );
            push_test_state(this, app_state_with_repo(repo, repo_id), cx);
        })
    });
    for (width, height, percent) in [
        (1280.0, 900.0, 80),
        (1440.0, 1000.0, 100),
        (3840.0, 2160.0, 200),
    ] {
        cx.simulate_resize(gpui::size(px(width), px(height)));
        for ui_font_size_px in [14, 24] {
            cx.update(|_, app| {
                app.set_global(Appearance {
                    density: UiDensity::Comfortable,
                    ui_font_size_px,
                    ..Appearance::default()
                });
                crate::app::set_app_ui_scale_percent(app, percent);
                view.update(app, |this, cx| {
                    this.notify_font_preferences_changed(cx);
                });
            });
            draw_and_drain_test_window(cx);
            let minimum = px(32.0 * percent as f32 / 100.0);
            for selector in [
                "stage_all_button",
                "commit_button",
                "status_row_79_unstaged_0",
                "status_row_79_staged_0",
            ] {
                let bounds = cx
                    .debug_bounds(selector)
                    .unwrap_or_else(|| panic!("missing {selector} at {percent}%"));
                assert!(
                    bounds.size.height + px(0.5) >= minimum,
                    "{selector} target {:?} must reach {minimum:?} at UI font {ui_font_size_px}",
                    bounds.size
                );
            }
            let row = cx.debug_bounds("status_row_79_unstaged_0").unwrap();
            cx.simulate_mouse_move(row.center(), None, Modifiers::default());
            draw_and_drain_test_window(cx);
            let button = cx
                .debug_bounds("status_stage_button_79_unstaged_0")
                .unwrap();
            assert!(
                button.top() >= row.top() - px(0.5) && button.bottom() <= row.bottom() + px(0.5),
                "stage target must fit inside its row"
            );
        }
    }
}
