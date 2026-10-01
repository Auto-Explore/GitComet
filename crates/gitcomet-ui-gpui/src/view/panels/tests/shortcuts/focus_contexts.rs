//! Focus contexts: which surface owns keys and where focus returns.

use super::*;

fn diff_view_mode(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> DiffViewMode {
    cx.update(|_window, app| view.read(app).main_pane.read(app).diff_view)
}

fn reveal_whitespace_chars(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::super::GitCometView>,
) -> bool {
    cx.update(|_window, app| view.read(app).main_pane.read(app).reveal_whitespace_chars)
}

/// The diff pane is a selection owner like any other: once another surface
/// takes the window's selection, its highlight must go too.
#[gpui::test]
fn another_surface_taking_the_selection_clears_the_diff_text_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70512);
    let commit_id = CommitId("fedcba0987654323".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_selection_ownership",
        std::process::id()
    ));
    let target = DiffTarget::commit(commit_id.clone(), std::path::PathBuf::from("src/only.rs"));

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(target).into());
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    set_diff_text_selection_on_row(cx, &view, 4);
    assert!(
        diff_text_has_selection(cx, &view),
        "precondition: the diff pane holds a text selection"
    );

    cx.update(|window, app| {
        let mut elsewhere = crate::text_selection_owner::SelectionOwnerToken::default();
        elsewhere.adopt(window, app);
    });
    cx.run_until_parked();

    assert!(
        !diff_text_has_selection(cx, &view),
        "the diff pane must drop its highlight once another surface owns the selection"
    );
}

#[gpui::test]
fn commit_diff_target_change_clears_text_selection_and_ctrl_c_copies_new_selection(
    cx: &mut gpui::TestAppContext,
) {
    let _clipboard_guard = crate::test_support::lock_clipboard_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70511);
    let commit_id = CommitId("fedcba0987654322".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_diff_selection_lifecycle",
        std::process::id()
    ));
    let first_path = std::path::PathBuf::from("src/commit_details/first.rs");
    let second_path = std::path::PathBuf::from("src/commit_details/second.rs");
    let first_target = DiffTarget::commit(commit_id.clone(), first_path);
    let second_target = DiffTarget::commit(commit_id.clone(), second_path);

    let mut first_repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    first_repo.diff_state.diff_target = Some(first_target.clone());
    first_repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(first_target).into());
    first_repo.diff_state.diff_rev = 1;
    first_repo.diff_state.diff_state_rev = first_repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(first_repo.clone()));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);
    set_diff_text_selection_on_row(cx, &view, 4);
    assert!(
        diff_text_has_selection(cx, &view),
        "expected the first commit file to have an active text selection"
    );
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("old selection".to_string()));

    let mut closed_repo = first_repo.clone();
    closed_repo.diff_state.diff_target = None;
    closed_repo.diff_state.diff = Loadable::NotLoaded;
    closed_repo.diff_state.diff_rev = 2;
    closed_repo.diff_state.diff_state_rev = closed_repo.diff_state.diff_state_rev.wrapping_add(1);
    apply_state(cx, &view, app_state_with_active_repo(closed_repo));

    assert!(
        !diff_text_has_selection(cx, &view),
        "closing a commit file diff must clear its text selection"
    );
    assert_eq!(diff_selection_anchor(cx, &view), None);
    assert_eq!(diff_selection_range(cx, &view), None);

    let mut second_repo = first_repo;
    second_repo.diff_state.diff_target = Some(second_target.clone());
    second_repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(second_target).into());
    second_repo.diff_state.diff_rev = 3;
    second_repo.diff_state.diff_state_rev = second_repo.diff_state.diff_state_rev.wrapping_add(1);
    apply_state(cx, &view, app_state_with_active_repo(second_repo));

    assert!(
        !diff_text_has_selection(cx, &view),
        "opening another commit file diff must not restore the old selection"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);
    focus_diff_panel(cx, &view);
    set_diff_text_selection_on_row(cx, &view, 5);
    cx.simulate_keystrokes("ctrl-c");

    let copied = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("Ctrl-C should copy the new file's selection");
    assert!(
        !copied.is_empty() && copied != "old selection",
        "Ctrl-C must replace old clipboard content with the new file's selection, got {copied:?}"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.select_all_diff_text(window, cx);
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("ctrl-c");

    let copied_after_reselection = cx
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("Ctrl-C should copy the changed selection");
    assert!(!copied_after_reselection.is_empty());
    assert_ne!(
        copied_after_reselection, copied,
        "changing the selection in one file must replace the previous clipboard text"
    );
}

#[gpui::test]
fn commit_message_text_input_secondary_enter_commits_staged_changes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(705315);
    let commit_id = CommitId("8899aabbccddef10".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_submit_shortcut",
        std::process::id()
    ));
    let staged_path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: staged_path,
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
            unstaged: std::sync::Arc::new(vec![]),
        }
        .into(),
    );
    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_commit_message_input(cx, &view);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.details_pane.update(cx, |pane, cx| {
                pane.commit_message_input.update(cx, |input, cx| {
                    input.set_text("hello shortcut".to_string(), cx);
                });
            });
        });
        let _ = window.draw(app);
    });

    let ops_rev_before = crate::view::test_support::repo_ops_rev(&view, cx, repo_id);
    cx.simulate_keystrokes("secondary-enter");
    draw_and_drain_test_window(cx);

    crate::view::test_support::drain_store_worker(&view, cx);
    assert!(
        crate::view::test_support::repo_ops_rev(&view, cx, repo_id) > ops_rev_before,
        "expected secondary-enter from the commit message input to dispatch a commit"
    );

    cx.update(|window, app| {
        let root = view.read(app);
        let focus = root
            .details_pane
            .read(app)
            .commit_message_input
            .read(app)
            .focus_handle();
        assert!(
            focus.is_focused(window),
            "expected commit-message input to keep focus after secondary-enter commit"
        );
        assert_eq!(
            root.details_pane
                .read(app)
                .commit_message_input
                .read(app)
                .text(),
            "",
            "expected secondary-enter commit to clear the commit message input"
        );
    });
}

/// Icons are sized in design px, which only zoom through the UI scale; a bare
/// `px()` holds them at 100% while the buttons around them grow.
#[gpui::test]
fn diff_toolbar_and_commit_box_icons_follow_ui_scale(cx: &mut gpui::TestAppContext) {
    let _guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70547);
    let commit_id = CommitId("1122334455667747".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_icons_follow_ui_scale",
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
    // Wide and tall enough that nothing collapses into an overflow at 200%.
    cx.simulate_resize(gpui::size(px(2400.0), px(1400.0)));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    const ICONS: [&str; 8] = [
        "diff_prev_hunk_icon",
        "diff_next_hunk_icon",
        "diff_action_menu_icon",
        "diff_close_icon",
        "commit_button_icon",
        "commit_options_icon",
        "previous_commit_messages_icon",
        "change_tracking_unstaged_header_chevron",
    ];
    let sizes = |cx: &mut gpui::VisualTestContext| {
        ICONS.map(|selector| {
            cx.debug_bounds(selector)
                .unwrap_or_else(|| panic!("expected {selector} to render"))
                .size
        })
    };
    let normal = sizes(cx);

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);
    let zoomed = sizes(cx);

    let unscaled: Vec<String> = ICONS
        .into_iter()
        .enumerate()
        .filter(|(ix, _)| {
            (zoomed[*ix].width, zoomed[*ix].height)
                != (normal[*ix].width * 2.0, normal[*ix].height * 2.0)
        })
        .map(|(ix, selector)| format!("{selector}: {:?} -> {:?}", normal[ix], zoomed[ix]))
        .collect();
    assert!(
        unscaled.is_empty(),
        "icons must double at 200% UI scale: {unscaled:#?}"
    );
}

#[gpui::test]
fn reveal_whitespace_toggle_invalidates_wrapped_diff_rows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_word_wrap = true;
                pane.diff_wrap_visible_cache_key = Some(DiffWrapVisibleCacheKey {
                    source_len: 1,
                    diff_view: DiffViewMode::Inline,
                    is_file_view: false,
                    collapsed_projection_active: false,
                    projection_rev: 0,
                    diff_cache_rev: 0,
                    file_diff_cache_seq: 0,
                    inline_columns: 8,
                    split_columns: 8,
                    preview_columns: 8,
                    preview_content_rev: 0,
                    reveal_whitespace_chars: false,
                });
                pane.diff_wrap_visible_rows = Arc::from([DiffWrapVisualRow {
                    source_visible_ix: 0,
                    wrap_ix: 0,
                    primary_range: rows::DiffWrapByteRange { start: 0, end: 4 },
                    secondary_range: rows::DiffWrapByteRange::default(),
                }]);
                pane.set_diff_reveal_whitespace_chars(true, cx);
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app);
        assert_eq!(
            pane.diff_wrap_visible_cache_key, None,
            "expected reveal-whitespace changes to invalidate wrapped-row cache keys"
        );
        assert!(
            pane.diff_wrap_visible_rows.is_empty(),
            "expected reveal-whitespace changes to drop cached wrapped rows"
        );
    });
}

#[gpui::test]
fn diff_view_toolbar_toggle_restores_diff_panel_focus(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70547);
    let commit_id = CommitId("1122334455667747".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_diff_view_toggle_focus",
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
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    focus_commit_message_input(cx, &view);
    let split_bounds = cx
        .debug_bounds("diff_split")
        .expect("expected split diff toolbar button");
    cx.simulate_click(split_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Split,
        "expected clicking Split to switch diff view"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected clicking Split to restore diff-panel focus"
    );

    let toggle_bounds = cx
        .debug_bounds("diff_view_toggle")
        .expect("expected diff view toggle container");
    let inline_bounds = cx
        .debug_bounds("diff_inline")
        .expect("expected inline diff toolbar button");
    let split_bounds = cx
        .debug_bounds("diff_split")
        .expect("expected split diff toolbar button");
    assert_eq!(inline_bounds.top(), toggle_bounds.top());
    assert_eq!(inline_bounds.bottom(), toggle_bounds.bottom());
    assert_eq!(split_bounds.top(), toggle_bounds.top());
    assert_eq!(split_bounds.bottom(), toggle_bounds.bottom());

    let file_header_bounds = cx
        .debug_bounds("diff_file_header")
        .expect("expected diff file header");
    let body_bounds = cx
        .debug_bounds("diff_body_container")
        .expect("expected diff body container");
    assert_eq!(body_bounds.left(), file_header_bounds.left());
    assert_eq!(body_bounds.right(), file_header_bounds.right());

    let details_bounds = cx
        .debug_bounds("details_pane")
        .expect("expected details pane");
    let resize_bounds = cx
        .debug_bounds("pane_resize_details")
        .expect("expected overlaid details resize handle");
    assert_eq!(file_header_bounds.right(), details_bounds.left());
    assert_eq!(resize_bounds.center().x, details_bounds.left());
}

#[gpui::test]
fn commit_message_text_input_view_and_whitespace_shortcuts_do_not_fallback(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7056);
    let commit_id = CommitId("1111222233335555".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_view_toggle",
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
    install_global_diff_shortcut_fallback_for_test(cx);

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.reveal_whitespace_chars = false;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-i");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Split,
        "expected Alt-I from commit-message input to avoid switching the diff view"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Inline;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-s");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Inline,
        "expected Alt-S from commit-message input to avoid switching the diff view"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.reveal_whitespace_chars = false;
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    cx.simulate_keystrokes("alt-w");
    draw_and_drain_test_window(cx);
    assert!(
        !reveal_whitespace_chars(cx, &view),
        "expected Alt-W from commit-message input to avoid toggling whitespace visibility"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Alt-I/Alt-S/Alt-W"
    );
}

#[gpui::test]
fn commit_message_text_input_space_does_not_stage_or_advance_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(7058);
    let commit_id = CommitId("1111222233337777".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_commit_message_space",
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
    focus_commit_message_input(cx, &view);
    install_global_diff_shortcut_fallback_for_test(cx);

    cx.simulate_keystrokes("space");
    draw_and_drain_test_window(cx);
    std::thread::sleep(Duration::from_millis(20));
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(first),
        "expected Space from commit-message input to avoid staging or advancing the diff selection"
    );
    assert!(
        commit_message_input_is_focused(cx, &view),
        "expected commit-message input to keep focus after Space"
    );
}

#[gpui::test]
fn detached_window_focus_uses_global_diff_shortcut_fallback(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70563);
    let commit_id = CommitId("abcdef0011223377".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_detached_focus_global_shortcuts",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let mut repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );
    repo.diff_state.diff = Loadable::Ready(
        two_hunk_diff(DiffTarget::working_tree(first.clone(), DiffArea::Unstaged)).into(),
    );
    repo.diff_state.diff_rev = repo.diff_state.diff_rev.wrapping_add(1);
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_detached_window_focus(cx);
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected detached focus to avoid the rendered diff-panel key path"
    );

    cx.simulate_keystrokes("secondary-f");
    draw_and_drain_test_window(cx);
    assert!(
        diff_search_active(cx, &view),
        "expected secondary-f from detached focus to activate diff search"
    );
    assert!(
        diff_search_input_is_focused(cx, &view),
        "expected secondary-f from detached focus to focus diff search"
    );

    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.main_pane.update(cx, |pane, cx| {
                pane.diff_view = DiffViewMode::Split;
                pane.reveal_whitespace_chars = false;
                pane.diff_search_active = false;
                pane.diff_search_matches.clear();
                pane.diff_search_match_ix = None;
                pane.rebuild_diff_cache(cx);
                pane.ensure_diff_visible_indices();
                cx.notify();
            });
        });
        let _ = window.draw(app);
    });
    draw_and_drain_test_window(cx);

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("alt-i");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Inline,
        "expected Alt-I from detached focus to switch to inline diff view"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("alt-s");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_view_mode(cx, &view),
        DiffViewMode::Split,
        "expected Alt-S from detached focus to switch to split diff view"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("alt-w");
    draw_and_drain_test_window(cx);
    assert!(
        reveal_whitespace_chars(cx, &view),
        "expected Alt-W from detached focus to toggle whitespace visibility"
    );

    set_diff_selection_anchor(cx, &view, None);
    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let first_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 from detached focus to navigate to the first diff change");

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f3");
    draw_and_drain_test_window(cx);
    let second_change = diff_selection_anchor(cx, &view)
        .expect("expected F3 from detached focus to navigate to the second diff change");
    assert!(
        second_change > first_change,
        "expected repeated F3 from detached focus to move forward through diff changes"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f2");
    draw_and_drain_test_window(cx);
    assert_eq!(
        diff_selection_anchor(cx, &view),
        Some(first_change),
        "expected F2 from detached focus to move back to the previous diff change"
    );

    focus_detached_window_focus(cx);
    cx.simulate_keystrokes("f4");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected F4 from detached focus to select the next diff target"
    );
}

#[gpui::test]
fn dismissing_change_tracking_settings_with_escape_restores_diff_panel_focus(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(706);
    let commit_id = CommitId("8765432187654321".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_change_tracking_focus_escape",
        std::process::id()
    ));
    let path = std::path::PathBuf::from("src/lib.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: std::sync::Arc::new(vec![]),
            unstaged: std::sync::Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
        }
        .into(),
    );
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(path, DiffArea::Unstaged));

    apply_state(cx, &view, app_state_with_active_repo(repo));
    focus_diff_panel(cx, &view);
    open_change_tracking_settings_popover(cx, &view);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings popover to be open before dismissing it"
    );
    assert!(
        !diff_panel_is_focused(cx, &view),
        "expected the change-tracking settings popover to hold focus while it is open"
    );

    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);

    assert!(
        !popover_is_open(cx, &view),
        "expected Escape to close the change-tracking settings popover"
    );
    assert!(
        diff_panel_is_focused(cx, &view),
        "expected dismissing change-tracking settings to restore diff-panel focus"
    );
}
