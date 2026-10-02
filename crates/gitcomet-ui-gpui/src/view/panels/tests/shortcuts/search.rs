//! Diff search: query entry, match navigation, background results.

use super::*;

fn wait_for_diff_search_debounce(cx: &mut gpui::VisualTestContext) {
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(150));
    cx.run_until_parked();
    draw_and_drain_test_window(cx);
}

fn focus_diff_search_input(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) {
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                let focus = pane.diff_search_input.read(cx).focus_handle();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
}

fn searchable_scroll_diff(target: DiffTarget) -> gitcomet_core::domain::Diff {
    let mut lines = vec![
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Header,
            text: "diff --git a/src/lib.rs b/src/lib.rs".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Header,
            text: "--- a/src/lib.rs".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Header,
            text: "+++ b/src/lib.rs".into(),
        },
        gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Hunk,
            text: "@@ -1,160 +1,160 @@".into(),
        },
    ];

    for ix in 0..160 {
        let text = match ix {
            1 => " context needle first".to_string(),
            120 => " context needle second".to_string(),
            _ => format!(" context filler line {ix}"),
        };
        lines.push(gitcomet_core::domain::DiffLine {
            kind: gitcomet_core::domain::DiffLineKind::Context,
            text: text.into(),
        });
    }

    gitcomet_core::domain::Diff { target, lines }
}

/// The author dropdown is a picker: its search box takes focus as the popover
/// opens, so the user can start typing without clicking into it first.
#[gpui::test]
fn history_author_filter_focuses_its_search_box_and_narrows_the_list(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(712);
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(author_filter_fixture_repo(repo_id)),
    );
    open_popover_for_test(cx, &view, PopoverKind::HistoryAuthorFilter { repo_id });
    draw_and_drain_test_window(cx);

    let search_is_focused = cx.update(|window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .history_author_filter_search_input_for_test()
            .is_some_and(|input| input.read(app).focus_handle().is_focused(window))
    });
    assert!(
        search_is_focused,
        "opening the author filter must focus its search box"
    );

    // Rows: "All authors" (index 0) then the loaded authors, alphabetically.
    assert!(cx.debug_bounds("picker_prompt_item_0").is_some());
    assert!(cx.debug_bounds("picker_prompt_item_1").is_some());
    assert!(cx.debug_bounds("picker_prompt_item_2").is_some());

    cx.simulate_keystrokes("b o");
    draw_and_drain_test_window(cx);

    let query = cx.update(|_window, app| {
        view.read(app)
            .popover_host
            .read(app)
            .history_author_filter_search_input_for_test()
            .map(|input| input.read(app).text().to_string())
            .unwrap_or_default()
    });
    assert_eq!(query, "bo", "keystrokes must reach the search box");
    // Every author is handed to the picker, which does the narrowing; the
    // selectors carry each row's original index — "All authors" 0, `Alice` 1,
    // `Bob` 2 — so only `Bob`'s survives.
    assert!(
        cx.debug_bounds("picker_prompt_item_2").is_some(),
        "`Bob` must survive the query"
    );
    assert!(
        cx.debug_bounds("picker_prompt_item_1").is_none(),
        "`Alice` must be filtered out"
    );
    assert!(
        cx.debug_bounds("picker_prompt_item_0").is_none(),
        "`All authors` does not match the query either"
    );
}

#[gpui::test]
fn commit_message_text_input_f3_prefers_diff_search_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7053);
    let commit_id = CommitId("8899aabbccddeeff".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_search_nav",
        std::process::id()
    ));
    let hunk_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: hunk_path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(
        hunk_path.clone(),
        DiffArea::Unstaged,
    ));
    repo.diff_state.diff = Loadable::Ready(
        simple_hunk_diff(DiffTarget::working_tree(hunk_path, DiffArea::Unstaged)).into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_matches = vec![3, 5];
                pane.diff_search_match_ix = Some(0);
                cx.notify();
            });
            this.details_pane.update(cx, |pane, cx| {
                let focus = pane.commit_message_input.read(cx).focus_handle();
                window.focus(&focus, cx);
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let root = view.read(app);
        assert_eq!(
            root.main_pane.read(app).diff_search_match_ix,
            Some(1),
            "expected F3 from commit-message input to advance the active diff search match"
        );
        let focus = root
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-message input to keep focus after F3 search navigation"
        );
    });
}

#[gpui::test]
fn commit_message_text_input_f2_prefers_previous_diff_search_match(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70531);
    let commit_id = CommitId("8899aabbccddef00".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_search_prev",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_matches = vec![3, 5];
                pane.diff_search_match_ix = Some(1);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let root = view.read(app);
        assert_eq!(
            root.main_pane.read(app).diff_search_match_ix,
            Some(0),
            "expected F2 from commit-message input to move to the previous diff search match"
        );
        let focus = root
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-message input to keep focus after F2 search navigation"
        );
    });
}

#[gpui::test]
fn diff_search_secondary_f_selects_existing_query(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70540);
    let commit_id = CommitId("1122334455667740".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_secondary_f_selects",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let query = "needle";

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = query.into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query, cx));
                let focus = pane.diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    cx.update(|window, app| {
        let pane = view.read(app).main_pane.read(app);
        let input = pane.diff_search_input.read(app);
        assert!(
            input.focus_handle().is_focused(window),
            "expected secondary-f to focus the diff search input"
        );
        assert_eq!(
            input.selected_range(),
            0..query.len(),
            "expected secondary-f to select the whole existing diff search query"
        );
    });
}

#[gpui::test]
fn diff_search_input_accepts_spaces_without_staging_file(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70546);
    let commit_id = CommitId("1122334455667746".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_space",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second],
        &first,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.simulate_input("needle one");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "needle one");
        assert_eq!(pane.diff_search_input.read(app).text(), "needle one");
    });

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected Space from the diff search input to avoid staging or advancing the diff target"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected the diff search input to keep focus after Space"
    );
}

#[gpui::test]
fn diff_search_close_clears_query_and_input(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70547);
    let commit_id = CommitId("1122334455667747".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_close_clears",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.simulate_input("needle one");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "needle one");
        assert_eq!(pane.diff_search_input.read(app).text(), "needle one");
    });

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(!pane.diff_search_active);
        assert_eq!(pane.diff_search_query.as_ref(), "");
        assert_eq!(pane.diff_search_input.read(app).text(), "");
        assert!(pane.diff_search_matches.is_empty());
        assert_eq!(pane.diff_search_match_ix, None);
    });
}

/// A synchronous recompute (data arrival, option toggle) cancels the worker
/// but leaves the rows unchanged. It used to drop the captured document too,
/// so the next keystroke recaptured it and lost its query cache.
#[gpui::test]
fn diff_search_document_survives_synchronous_recompute_until_close(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70548);
    let commit_id = CommitId("1122334455667748".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_document_reuse",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    // Without file text the file-diff cache resets on every render, which
    // bumps the projection and would recapture the document regardless.
    repo.diff_state.diff_file =
        Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
            path.clone(),
            Some("old one\nunchanged\n".into()),
            Some("new one\nunchanged\n".into()),
        ))));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let set_query = |cx: &mut gpui::VisualTestContext, query: &'static str| {
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_search_input
                        .update(cx, |input, cx| input.set_text(query, cx));
                });
            });
        });
        draw_and_drain_test_window(cx);
    };
    let document = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let pane = view.read(app).main_pane.read(app);
            assert!(!pane.diff_search_worker_running);
            pane.diff_search_document
                .as_ref()
                .map(|(_, document)| Arc::clone(document))
        })
    };

    set_query(cx, "new");
    let first = document(cx).expect("the worker captured a document");
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane
                .update(cx, |pane, _| pane.diff_search_recompute_matches());
        });
    });
    set_query(cx, "ne");
    let second = document(cx).expect("the worker captured a document");
    assert!(
        Arc::ptr_eq(&first, &second),
        "unchanged rows must keep the captured document"
    );
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = match pane.diff_view {
                    DiffViewMode::Split => DiffViewMode::Inline,
                    DiffViewMode::Inline => DiffViewMode::Split,
                };
                pane.diff_search_recompute_matches();
            });
        });
    });
    assert!(
        document(cx).is_none(),
        "changed rows drop the stale document instead of pinning it"
    );

    focus_diff_search_input(cx, &view);
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(
        document(cx).is_none(),
        "closing search releases the document"
    );
}

/// A split file diff searches only the file rows. The capture also cloned
/// the patch header map and pinned the patch and inline row sources.
#[gpui::test]
fn split_file_diff_search_captures_only_file_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(70549);
    let commit_id = CommitId("1122334455667749".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_capture_scope",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    repo.diff_state.diff_file =
        Loadable::Ready(Some(Arc::new(gitcomet_core::domain::FileDiffText::new(
            path.clone(),
            Some("old one\nunchanged\n".into()),
            Some("new one\nunchanged\n".into()),
        ))));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, _| {
                pane.diff_view = DiffViewMode::Split;
                pane.ensure_diff_visible_indices();
                assert!(pane.is_file_diff_view_active());
                let patch = pane.diff_row_provider.clone().expect("patch rows loaded");
                let counts = |pane: &crate::view::panes::MainPaneView| {
                    (
                        Arc::strong_count(&patch),
                        Arc::strong_count(&pane.diff_cache),
                        Arc::strong_count(&pane.diff_split_cache),
                        Arc::strong_count(&pane.file_diff_inline_cache),
                        pane.diff_split_row_provider.as_ref().map(Arc::strong_count),
                        pane.file_diff_inline_row_provider
                            .as_ref()
                            .map(Arc::strong_count),
                    )
                };
                let before = counts(pane);
                let file_rows = pane.file_diff_row_provider.as_ref().map(Arc::strong_count);
                let _document = pane.capture_search_document();
                assert_eq!(counts(pane), before, "the capture pinned unused rows");
                assert_eq!(
                    pane.file_diff_row_provider.as_ref().map(Arc::strong_count),
                    file_rows.map(|count| count + 1),
                    "the capture shares the file rows it searches"
                );
            });
        });
    });
}

#[gpui::test]
fn whitespace_only_diff_search_query_recomputes_on_whitespace_mode_change(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_query = " ".into();
                pane.diff_search_matches = vec![7];
                pane.diff_search_match_ix = Some(0);
                pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert!(
            pane.diff_search_matches.is_empty(),
            "expected whitespace-only queries to refresh instead of leaving stale matches behind"
        );
        assert_eq!(
            pane.diff_search_match_ix, None,
            "expected recomputing an empty result set to clear the active diff search match"
        );
    });
}

#[gpui::test]
fn diff_search_overlay_does_not_reflow_action_bar_or_content(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70545);
    let commit_id = CommitId("1122334455667745".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_overlay_layout",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                let focus = pane.diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    assert!(
        cx.debug_bounds("diff_search_overlay").is_none(),
        "expected diff search overlay to be absent before search opens"
    );
    let close_before = cx
        .debug_bounds("diff_close")
        .expect("expected diff close button before search opens");
    let content_before = cx
        .debug_bounds("diff_body_container")
        .expect("expected diff body before search opens");

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    let close_after = cx
        .debug_bounds("diff_close")
        .expect("expected diff close button after search opens");
    let content_after = cx
        .debug_bounds("diff_body_container")
        .expect("expected diff body after search opens");
    assert!(
        cx.debug_bounds("diff_search_overlay").is_some(),
        "expected diff search overlay after secondary-f"
    );
    let overlay_empty_query = cx
        .debug_bounds("diff_search_overlay")
        .expect("expected diff search overlay bounds after secondary-f");
    let input_slot_empty_query = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot bounds after secondary-f");
    let match_label_empty_query = cx
        .debug_bounds("diff_search_match_label")
        .expect("expected diff search match label bounds after secondary-f");
    assert_eq!(
        close_after, close_before,
        "expected diff close button bounds to remain stable when search opens"
    );
    assert_eq!(
        content_after.top(),
        content_before.top(),
        "expected diff content top to remain stable when search opens"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("new", cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    wait_for_diff_search_debounce(cx);

    let overlay_with_matches = cx
        .debug_bounds("diff_search_overlay")
        .expect("expected diff search overlay bounds after entering a query");
    let input_slot_with_matches = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot bounds after entering a query");
    let match_label_with_matches = cx
        .debug_bounds("diff_search_match_label")
        .expect("expected diff search match label bounds after entering a query");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_matches.len(),
            2,
            "expected the query to switch the search label to a match count"
        );
    });
    assert_eq!(
        overlay_with_matches.size.width, overlay_empty_query.size.width,
        "expected diff search overlay width to stay stable when the status text changes"
    );
    assert_eq!(
        input_slot_with_matches.origin.x, input_slot_empty_query.origin.x,
        "expected diff search input slot x position to stay stable when the status text changes"
    );
    assert_eq!(
        input_slot_with_matches.size.width, input_slot_empty_query.size.width,
        "expected diff search input slot width to stay stable when the status text changes"
    );
    assert_eq!(
        match_label_with_matches.size.width, match_label_empty_query.size.width,
        "expected diff search status label width to stay stable when the status text changes"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_search_overlay").is_none(),
        "expected Escape to remove diff search overlay"
    );

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    let search_close_bounds = cx
        .debug_bounds("diff_search_close")
        .expect("expected diff search close button after reopening search");
    cx.simulate_click(search_close_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_search_overlay").is_none(),
        "expected search close button to remove diff search overlay"
    );
}

/// Opens diff search (`secondary-f`) over a two-hunk working tree diff.
fn open_diff_search_on_two_hunk_diff(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    repo_id: RepoId,
    name: &str,
) {
    let commit_id = CommitId("1122334455667748".into());
    let workdir =
        std::env::temp_dir().join(format!("gitcomet_ui_test_{}_{name}", std::process::id()));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                let focus = pane.diff_panel_focus_handle.clone();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    assert!(
        cx.debug_bounds("diff_search_overlay").is_some(),
        "expected diff search overlay after secondary-f"
    );
}

fn set_diff_search_text(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
    text: &'static str,
) {
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(text, cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    wait_for_diff_search_debounce(cx);
}

fn diff_search_match_state(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> (usize, Option<usize>) {
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (pane.diff_search_matches.len(), pane.diff_search_match_ix)
    })
}

#[gpui::test]
fn diff_search_arrow_buttons_step_through_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    open_diff_search_on_two_hunk_diff(cx, &view, RepoId(70941), "diff_search_arrow_buttons");

    let prev = cx
        .debug_bounds("diff_search_prev")
        .expect("expected a previous-match button in diff search");
    let next = cx
        .debug_bounds("diff_search_next")
        .expect("expected a next-match button in diff search");
    let label = cx
        .debug_bounds("diff_search_match_label")
        .expect("expected the diff search match label");
    let close = cx
        .debug_bounds("diff_search_close")
        .expect("expected the diff search close button");
    assert!(
        label.right() <= prev.left() && prev.right() <= next.left() && next.right() <= close.left(),
        "expected the arrow buttons between the match label and the close button"
    );

    set_diff_search_text(cx, &view, "new");
    let (total, first_ix) = diff_search_match_state(cx, &view);
    assert_eq!(total, 2, "expected the query to match both hunks");
    let first_ix = first_ix.unwrap_or(0);

    let next = cx.debug_bounds("diff_search_next").expect("next button");
    cx.simulate_click(next.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_search_match_state(cx, &view),
        (2, Some((first_ix + 1) % 2)),
        "expected the down arrow to step to the next match"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected the search input to keep focus after stepping"
    );

    let next = cx.debug_bounds("diff_search_next").expect("next button");
    cx.simulate_click(next.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_search_match_state(cx, &view),
        (2, Some(first_ix)),
        "expected the down arrow to wrap to the first match"
    );

    let prev = cx.debug_bounds("diff_search_prev").expect("prev button");
    cx.simulate_click(prev.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_search_match_state(cx, &view),
        (2, Some((first_ix + 1) % 2)),
        "expected the up arrow to wrap to the last match"
    );
}

#[gpui::test]
fn diff_search_arrow_buttons_are_disabled_without_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    open_diff_search_on_two_hunk_diff(cx, &view, RepoId(70942), "diff_search_arrow_disabled");

    set_diff_search_text(cx, &view, "new");
    let found = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_matches
            .clone()
    });
    assert_eq!(found.len(), 2, "expected the query to match both hunks");

    set_diff_search_text(cx, &view, "absent_from_the_diff");
    assert_eq!(diff_search_match_state(cx, &view), (0, None));

    for selector in ["diff_search_next", "diff_search_prev"] {
        // Draw the arrows for "no matches", then hand the pane matches without
        // re-rendering: a click on the arrows on screen must not reach it.
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_search_matches.clear();
                    pane.diff_search_match_ix = None;
                    cx.notify();
                });
            });
            let _ = window.draw(app);
        });
        draw_and_drain_test_window(cx);
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} while search is open"));
        cx.update(|_window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, _cx| {
                    pane.diff_search_matches = found.clone();
                });
            });
        });
        cx.simulate_click(bounds.center(), Modifiers::default());
        assert_eq!(
            diff_search_match_state(cx, &view),
            (2, None),
            "expected {selector} to be disabled when there are no matches"
        );
    }
}

#[gpui::test]
fn diff_search_shift_enter_inserts_a_newline(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        window.activate();
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    open_diff_search_on_two_hunk_diff(cx, &view, RepoId(70943), "diff_search_shift_enter");
    assert!(diff_search_input_is_focused(cx, &view));

    cx.simulate_input("new");
    cx.simulate_keystrokes("shift-enter");
    cx.simulate_input("x");
    draw_and_drain_test_window(cx);

    let text = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_input
            .read(app)
            .text()
            .to_string()
    });
    assert_eq!(text, "new\nx", "expected Shift+Enter to insert a newline");
    assert!(
        cx.debug_bounds("diff_search_overlay").is_some(),
        "expected diff search to stay open"
    );
}

#[gpui::test]
fn diff_search_input_slot_grows_for_multiline_query(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70548);
    let commit_id = CommitId("1122334455667748".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_multiline_grows",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));
    focus_diff_search_input(cx, &view);
    draw_and_drain_test_window(cx);

    let single_line = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot for a single-line query");
    assert!(
        single_line.size.height <= px(30.0),
        "expected compact one-line diff search slot height; got {single_line:?}"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("alpha\nbeta", cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let multiline = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot for a multiline query");
    assert_eq!(
        multiline.size.width, single_line.size.width,
        "expected multiline diff search to preserve the input slot width"
    );
    assert!(
        multiline.size.height > single_line.size.height + px(8.0),
        "expected multiline diff search slot to grow; single={single_line:?} multiline={multiline:?}"
    );
}

#[gpui::test]
fn diff_search_input_slot_caps_tall_multiline_query(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70549);
    let commit_id = CommitId("1122334455667749".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_multiline_caps",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(1000.0), px(640.0)));
    focus_diff_search_input(cx, &view);

    let tall_query = (0..40)
        .map(|ix| format!("needle_{ix}"))
        .collect::<Vec<_>>()
        .join("\n");
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(tall_query.clone(), cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let tall_slot = cx
        .debug_bounds("diff_search_input_slot")
        .expect("expected diff search input slot for a tall multiline query");
    let max_height = px(super::super::super::COMMIT_MESSAGE_INPUT_MAX_HEIGHT_PX);
    assert!(
        tall_slot.size.height <= max_height + px(1.0),
        "expected tall diff search slot to cap at {max_height:?}; got {tall_slot:?}"
    );

    let max_scroll_y = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_scroll
            .max_offset()
            .y
    });
    assert!(
        max_scroll_y > px(0.0),
        "expected tall diff search query to be vertically scrollable"
    );
}

#[gpui::test]
fn diff_search_query_edit_selects_first_match_and_updates_count(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70541);
    let commit_id = CommitId("1122334455667741".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_query_edit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("new", cx));
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert!(
            !pane.diff_search_worker_running,
            "background search should finish without a debounce timer"
        );
    });
    wait_for_diff_search_debounce(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert_eq!(
            pane.diff_search_matches.len(),
            2,
            "expected the edited query to find both matching diff rows"
        );
        assert_eq!(
            pane.diff_search_match_ix,
            Some(0),
            "expected query edits to select the first match"
        );
        assert_eq!(
            pane.diff_selection_anchor,
            pane.diff_search_matches.first().copied(),
            "expected query edits to scroll/anchor to the first match"
        );
    });
}

#[gpui::test]
fn diff_search_navigation_keys_follow_background_results(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70543);
    let commit_id = CommitId("1122334455667743".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_nav_flush",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        app.clear_key_bindings();
        crate::app::bind_text_input_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                let focus = pane.diff_search_input.read(cx).focus_handle();
                window.focus(&focus, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    let set_pending_query = |cx: &mut gpui::VisualTestContext, query: &str| {
        let query = query.to_string();
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                this.main_pane.update(cx, |pane, cx| {
                    pane.diff_search_input
                        .update(cx, |input, cx| input.set_text(query.clone(), cx));
                    cx.notify();
                });
            });
            let _ = window.draw(app);
        });
        draw_and_drain_test_window(cx);
    };

    set_pending_query(cx, "new");
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert_eq!(pane.diff_search_matches.len(), 2);
    });
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert_eq!(pane.diff_search_matches.len(), 2);
        assert_eq!(
            pane.diff_search_match_ix,
            Some(1),
            "expected F3 to flush the pending search before advancing"
        );
    });

    set_pending_query(cx, "old");
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "old");
        assert_eq!(pane.diff_search_matches.len(), 2);
        assert_eq!(
            pane.diff_search_match_ix,
            Some(1),
            "expected F2 to flush the pending search before moving backward"
        );
    });

    set_pending_query(cx, "unchanged");
    cx.simulate_keystrokes("enter");
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "unchanged");
        assert_eq!(pane.diff_search_matches.len(), 1);
        assert_eq!(
            pane.diff_search_match_ix,
            Some(0),
            "expected Enter to flush the pending search before navigating"
        );
    });
}

#[gpui::test]
fn diff_search_preserve_current_scrolls_when_matches_first_appear(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70542);
    let commit_id = CommitId("1122334455667742".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_first_matches",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                pane.diff_search_query = "new".into();
                pane.diff_search_matches.clear();
                pane.diff_search_match_ix = None;
                pane.diff_selection_anchor = None;
                pane.diff_selection_range = None;
                pane.diff_scroll.0.borrow_mut().deferred_scroll_to_item = None;
                pane.diff_search_recompute_matches();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        let first_match = pane
            .diff_search_matches
            .first()
            .copied()
            .expect("expected query to find diff search matches");
        assert_eq!(
            pane.diff_search_match_ix,
            Some(0),
            "expected first newly discovered match to become active"
        );
        assert_eq!(
            pane.diff_selection_anchor,
            Some(first_match),
            "expected first newly discovered match to be scrolled into view"
        );
        assert_eq!(
            pane.diff_selection_range,
            Some((first_match, first_match)),
            "expected scroll-to-match to update the diff selection range"
        );
    });
}

#[gpui::test]
fn diff_search_passive_visible_refresh_preserves_scroll_and_match(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70544);
    let commit_id = CommitId("1122334455667744".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_passive_refresh",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        searchable_scroll_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.simulate_resize(gpui::size(px(900.0), px(420.0)));

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                pane.diff_search_query = "needle".into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text("needle", cx));
                pane.diff_search_recompute_matches_and_scroll_to_first();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    wait_for_main_pane_condition(
        cx,
        &view,
        "diff search fixture matches",
        |pane| pane.diff_search_matches.len() >= 2,
        |pane| {
            format!(
                "matches={:?} offset={:?} deferred_scroll={:?}",
                pane.diff_search_matches,
                pane.diff_scroll.0.borrow().base_handle.offset(),
                pane.diff_scroll.0.borrow().deferred_scroll_to_item,
            )
        },
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                let last_match_ix = pane.diff_search_matches.len() - 1;
                pane.diff_search_match_ix = Some(last_match_ix);
                set_uniform_list_offset(&pane.diff_scroll, gpui::point(px(0.0), px(-120.0)));
                pane.diff_scroll.0.borrow_mut().deferred_scroll_to_item = None;
                cx.notify();
            });
        });
    });

    let (before_offset, expected_match_ix) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        (
            pane.diff_scroll.0.borrow().base_handle.offset(),
            pane.diff_search_match_ix,
        )
    });

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_visible_cache_len = usize::MAX;
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
    });

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_match_ix, expected_match_ix,
            "expected passive visible-index refresh to preserve the active search match"
        );
        assert_eq!(
            pane.diff_scroll.0.borrow().base_handle.offset(),
            before_offset,
            "expected passive visible-index refresh not to move the diff scroll position"
        );
        assert!(
            pane.diff_scroll
                .0
                .borrow()
                .deferred_scroll_to_item
                .is_none(),
            "expected passive visible-index refresh not to schedule a diff scroll"
        );
    });
}

#[gpui::test]
fn diff_search_text_input_file_navigation_preserves_focus_and_last_file_boundary(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70542);
    let commit_id = CommitId("1122334455667700".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_file_nav",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &second,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to hold focus before adjacent-file navigation"
    );

    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second.clone()),
        "expected F4 from diff-search input at the last file to leave the diff target unchanged"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to keep focus after a no-op F4 navigation"
    );

    cx.simulate_keystrokes("f1");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, first.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected F1 from diff-search input to select the previous diff target"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to keep focus after F1 navigation"
    );
}

#[gpui::test]
fn conflict_diff_search_input_change_navigation_preserves_focus(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70543);
    let commit_id = CommitId("1122334455667711".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_conflict_input_nav",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/conflicted.rs");

    let repo = simple_conflict_repo(repo_id, &workdir, &commit_id, path.as_path());
    apply_state(cx, &view, app_state_with_active_repo(repo));
    wait_for_main_pane_condition(
        cx,
        &view,
        "conflict resolver state for text-input navigation",
        |pane| {
            pane.conflict_resolver.path.as_deref() == Some(path.as_path())
                && pane
                    .conflict_resolver
                    .resolved_outline
                    .markers
                    .iter()
                    .flatten()
                    .map(|marker| marker.conflict_ix)
                    .max()
                    .is_some_and(|ix| ix >= 1)
        },
        |pane| {
            format!(
                "path={:?} markers={} active_conflict={:?}",
                pane.conflict_resolver.path.clone(),
                pane.conflict_resolver.resolved_outline.markers.len(),
                pane.conflict_resolver.active_conflict,
            )
        },
    );
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.conflict_resolver_set_view_mode(ConflictResolverViewMode::TwoWayDiff, cx);
            });
        });
        let _ = window.draw(app);
    });
    wait_for_main_pane_condition(
        cx,
        &view,
        "two-way conflict navigation entries for text-input navigation",
        |pane| {
            pane.conflict_resolver.view_mode == ConflictResolverViewMode::TwoWayDiff
                && pane.conflict_nav_entries().len() >= 2
        },
        |pane| {
            format!(
                "view_mode={:?} nav_entries={:?}",
                pane.conflict_resolver.view_mode,
                pane.conflict_nav_entries(),
            )
        },
    );
    focus_diff_search_input(cx, &view);

    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to hold focus before conflict navigation"
    );
    assert_eq!(
        active_conflict_ix(cx, &view),
        0,
        "expected the first conflict to be active before navigation"
    );

    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let first_anchor = conflict_navigation_anchor(cx, &view)
        .expect("expected F7 from diff search input to set a navigation anchor");
    assert_eq!(
        active_conflict_ix(cx, &view),
        1,
        "expected one F7 from the fresh first-conflict anchor to advance"
    );

    cx.simulate_keystrokes("f7");
    draw_and_drain_test_window(cx);
    let second_anchor = conflict_navigation_anchor(cx, &view)
        .expect("expected the second F7 to keep a conflict navigation anchor");
    assert_eq!(
        second_anchor, first_anchor,
        "explicit conflict navigation does not wrap past the last target"
    );
    assert_eq!(
        active_conflict_ix(cx, &view),
        1,
        "expected repeated F7 at the end to keep the second conflict active"
    );

    cx.simulate_keystrokes("shift-f7");
    draw_and_drain_test_window(cx);

    assert_eq!(
        active_conflict_ix(cx, &view),
        0,
        "expected Shift-F7 from diff search input to return to the previous conflict"
    );
    assert!(
        conflict_navigation_anchor(cx, &view).is_some_and(|anchor| anchor < second_anchor),
        "expected Shift-F7 from diff search input to move the navigation anchor backward"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected diff search input to keep focus after conflict navigation shortcuts"
    );
}

#[gpui::test]
fn commit_message_text_input_secondary_f_activates_diff_search(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7055);
    let commit_id = CommitId("1111222233334444".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_secondary_f",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    let query = "needle";
    cx.update(|window, app| {
        crate::app::bind_app_keys_for_test(app);
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_query = query.into();
                pane.diff_search_input
                    .update(cx, |input, cx| input.set_text(query.to_string(), cx));
            });
        });
        let _ = window.draw(app);
    });

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    assert!(
        diff_search_active(cx, &view),
        "expected secondary-f from commit-message input to activate diff search when a diff is visible"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected secondary-f from commit-message input to focus diff search when a diff is visible"
    );
    assert!(
        !commit_message_input_is_focused(cx, &view),
        "expected secondary-f from commit-message input to move focus to diff search"
    );
    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_search_input.read(app).selected_range(),
            0..query.len(),
            "expected secondary-f to select the full existing diff search query"
        );
    });
}

/// With no diff visible the main pane shows the history list, so Cmd-F from
/// the commit-message input opens the history find bar instead of the diff
/// search.
#[gpui::test]
fn commit_message_text_input_secondary_f_without_visible_diff_opens_history_find(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70551);
    let commit_id = CommitId("1111222233334445".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_secondary_f_no_diff",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff_target = None;
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);
    cx.update(|window, app| {
        window.activate();
        crate::app::bind_app_keys_for_test(app);
        let _ = window.draw(app);
    });
    let history_view =
        cx.update(|_window, app| view.read(app).main_pane.read(app).history_view.clone());
    assert!(
        !cx.update(|_window, app| history_view.read(app).history_find_is_open()),
        "the history find bar starts closed"
    );

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);

    assert!(
        !diff_search_active(cx, &view),
        "expected secondary-f to avoid activating diff search when no diff is visible"
    );
    assert!(
        cx.update(|_window, app| history_view.read(app).history_find_is_open()),
        "expected secondary-f with no visible diff to open the history find bar"
    );
    assert!(
        cx.debug_bounds("history_find_input_slot").is_some(),
        "expected the history find bar to be rendered"
    );
    assert!(
        !commit_message_input_is_focused(cx, &view),
        "expected secondary-f to move focus out of the commit-message input"
    );
    cx.update(|window, app| {
        let focus = &history_view.read(app).history_panel_focus_handle;
        assert!(
            focus.contains_focused(window, app) && !focus.is_focused(window),
            "expected focus inside the history find bar, not on the list itself"
        );
    });

    // Typing lands in the find input: the query matches the fixture's only
    // commit ("Initial commit" by Alice).
    cx.simulate_input("initial");
    wait_until(cx, "the history find query to match the commit", |cx| {
        cx.update(|_window, app| {
            history_view.update(app, |history, _cx| {
                history
                    .history_find_matches()
                    .is_some_and(|matches| matches.visible == [0])
            })
        })
    });
    assert!(
        !diff_search_active(cx, &view),
        "typing in the history find bar must not open the diff search"
    );
}

#[gpui::test]
fn non_text_context_menu_focus_f2_f3_use_diff_search_matches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70562);
    let commit_id = CommitId("abcdef0011223366".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_menu_f2_f3",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_for_test(cx);
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_search_active = true;
                pane.diff_search_matches = vec![3, 5];
                pane.diff_search_match_ix = Some(0);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    open_change_tracking_settings_popover(cx, &view);

    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected context-menu focus to exercise the app-level search shortcut fallback"
    );

    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_match_ix),
        Some(1),
        "expected F3 from non-text context-menu focus to advance the diff search match"
    );

    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_window, app| view.read(app).main_pane.read(app).diff_search_match_ix),
        Some(0),
        "expected F2 from non-text context-menu focus to move to the previous diff search match"
    );
    assert!(
        popover_is_open(cx, &view),
        "expected app-level F2/F3 navigation not to dismiss an unrelated context menu"
    );
}

#[gpui::test]
fn background_search_keeps_latest_query_and_queued_navigation(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70541);
    let commit_id = CommitId("1122334455667741".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_query_edit",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    cx.update(|_, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            pane.rebuild_diff_cache(cx);
            pane.ensure_diff_visible_indices();
            pane.diff_search_active = true;
            for query in ["old", "absent", "new"] {
                let previous = std::mem::replace(&mut pane.diff_search_query, query.into());
                // A stale projection must be rebuilt without scanning on the
                // UI thread before the background search even starts.
                pane.diff_visible_cache_len = usize::MAX;
                pane.diff_search_schedule_query_recompute(previous, cx);
                assert!(
                    pane.diff_search_matches.is_empty(),
                    "UI callback must not synchronously scan"
                );
            }
            pane.diff_search_next_match();
            assert!(pane.diff_search_worker_running);
            assert_eq!(pane.diff_search_pending_navigation, 1);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "new");
        assert_eq!(pane.diff_search_matches.len(), 2);
        assert_eq!(pane.diff_search_match_ix, Some(1));
        assert!(!pane.diff_search_worker_running);
        assert!(pane.diff_search_pending_previous_query.is_none());
    });
}

// A diff reload recomputes synchronously and cancels the running worker, whose
// result is then discarded. Navigation must act on the fresh synchronous
// matches instead of queueing behind a worker that will never publish.
#[gpui::test]
fn diff_search_navigation_after_synchronous_recompute_is_not_lost(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70547);
    let commit_id = CommitId("1122334455667747".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_cancelled_worker_nav",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    let navigated = cx.update(|_, app| {
        let pane = view.read(app).main_pane.clone();
        pane.update(app, |pane, cx| {
            pane.rebuild_diff_cache(cx);
            pane.ensure_diff_visible_indices();
            pane.diff_search_active = true;
            let previous = std::mem::replace(&mut pane.diff_search_query, "new".into());
            pane.diff_search_schedule_query_recompute(previous, cx);
            assert!(pane.diff_search_worker_running);
            // What a diff reload does while the worker is still running.
            pane.diff_search_recompute_matches();
            assert_eq!(pane.diff_search_matches.len(), 2);
            let before = pane.diff_search_match_ix;
            pane.diff_search_next_match();
            assert_ne!(
                pane.diff_search_match_ix, before,
                "F3 must step through the synchronously recomputed matches"
            );
            pane.diff_search_match_ix
        })
    });
    draw_and_drain_test_window(cx);
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_matches.len(), 2);
        assert_eq!(pane.diff_search_match_ix, navigated);
        assert_eq!(pane.diff_search_pending_navigation, 0);
        assert!(!pane.diff_search_worker_running);
    });
}

// A query edit leaves the document unchanged, so the previous matches stay
// valid. Clearing them on every keystroke blanked the highlights and counter
// until the worker published.
#[gpui::test]
fn diff_search_query_edit_keeps_previous_matches_until_the_worker_publishes(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70549);
    let commit_id = CommitId("1122334455667749".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_search_keeps_matches",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(path.clone(), DiffArea::Unstaged)).into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_search_input(cx, &view);

    let schedule = |cx: &mut gpui::VisualTestContext, query: &'static str| {
        cx.update(|_, app| {
            let pane = view.read(app).main_pane.clone();
            pane.update(app, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                pane.diff_search_active = true;
                let previous = std::mem::replace(&mut pane.diff_search_query, query.into());
                pane.diff_search_schedule_query_recompute(previous, cx);
                (pane.diff_search_matches.clone(), pane.diff_search_match_ix)
            })
        })
    };
    schedule(cx, "new");
    draw_and_drain_test_window(cx);
    let published = cx.update(|_, app| {
        view.read(app)
            .main_pane
            .read(app)
            .diff_search_matches
            .clone()
    });
    assert_eq!(published.len(), 2);

    let (while_pending, ix_while_pending) = schedule(cx, "ne");
    assert_eq!(
        while_pending, published,
        "the previous query's matches stay on screen while the worker runs"
    );
    assert!(ix_while_pending.is_some());
    draw_and_drain_test_window(cx);
    cx.update(|_, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(pane.diff_search_query.as_ref(), "ne");
        assert!(!pane.diff_search_worker_running);
        assert_eq!(pane.diff_search_match_ix, Some(0));
    });
}
