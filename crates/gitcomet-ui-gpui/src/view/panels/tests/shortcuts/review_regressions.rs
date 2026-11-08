use super::*;

#[gpui::test]
fn escape_from_sidebar_does_not_enter_review_files(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    cx.update(|window, app| {
        view.update(app, |root, cx| {
            root.set_sidebar_collapsed(false, cx);
            root.set_details_collapsed(true, cx);
        });
        let focus = view.read(app).sidebar_pane.read(app).sidebar_focus.clone();
        window.focus(&focus, app);
    });
    draw_and_drain_test_window(cx);
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(cx.update(|window, app| {
        let root = view.read(app);
        root.details_collapsed && root.sidebar_pane.read(app).sidebar_focus.is_focused(window)
    }));
}

#[gpui::test]
fn escape_from_unrelated_diff_does_not_open_first_status_file(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = working_copy_repo(&[], &["unrelated.rs"]);
    repo.diff_state.diff_target = Some(DiffTarget::commit(
        CommitId("other-commit".into()),
        "history.rs".into(),
    ));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    diff_content(cx, &view, false, false);
    cx.update(|window, app| {
        let focus = view
            .read(app)
            .diff_renderer
            .read(app)
            .diff_panel_focus_handle
            .clone();
        window.focus(&focus, app);
    });
    cx.simulate_keystrokes("escape");
    wait_until(cx, "unrelated preview closed", |cx| {
        sync_store_snapshot(cx, &view);
        selected_path(cx, &view).is_none()
    });
    assert!(graph_focused(cx, &view));
}

#[gpui::test]
fn reopened_file_search_does_not_consume_old_escape(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("a.rs"));
    sync_store_snapshot(cx, &view);
    cx.simulate_keystrokes("secondary-f escape secondary-f");
    draw_and_drain_test_window(cx);
    cx.simulate_input("b.rs");
    draw_and_drain_test_window(cx);
    assert!(cx.debug_bounds("review_file_search").is_some());
    cx.simulate_keystrokes("enter");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
}

#[gpui::test]
fn returning_to_graph_ends_temporary_diff_engine(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    cx.update(|_, app| {
        view.read(app).diff_renderer.clone().update(app, |pane, _| {
            pane.diff_engine_override = Some(
                crate::view::panes::diff::helpers::state::DiffEngineOverride::new(
                    gitcomet_core::diff_engine::DiffEngine::LineByLine,
                ),
            );
        });
    });
    cx.simulate_keystrokes("escape");
    draw_and_drain_test_window(cx);
    assert!(cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .diff_engine_override()
            .is_some()
    }));
    cx.simulate_keystrokes("escape");
    wait_until(cx, "diff viewer closed", |cx| {
        sync_store_snapshot(cx, &view);
        selected_path(cx, &view).is_none()
    });
    assert!(cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .diff_engine_override()
            .is_none()
    }));
}

#[gpui::test]
fn landing_pending_jump_does_not_also_advance_a_change(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    open_file(cx, &view, "a.rs");
    diff_content(cx, &view, false, false);
    cx.update(|window, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                let target = pane.rendered_diff_target().unwrap().clone();
                pane.prepare_review_file_jump(RepoId(9071), target, false);
                pane.review_file_jump.as_mut().unwrap().from =
                    DiffTarget::commit(CommitId("abcdef0123456789".into()), "previous.rs".into());
                pane.diff_autoscroll_pending = true;
                pane.diff_selection_anchor = None;
                assert!(pane.review_adjacent_change(false, window, cx));
                assert_eq!(
                    pane.diff_selection_anchor,
                    pane.diff_nav_entries().first().copied()
                );
                assert!(pane.review_file_jump.is_none());
            });
    });
}

fn preview_fixture(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<GitCometView>,
    markdown: bool,
) -> tempfile::TempDir {
    prepare(cx, view);
    let dir = tempfile::tempdir().unwrap();
    let path = if markdown { "a.md" } else { "a.rs" };
    let source = (0..200)
        .map(|index| format!("line {index}\n\n"))
        .collect::<String>();
    std::fs::write(dir.path().join(path), &source).unwrap();
    let mut repo = working_copy_repo(&[], &[path, "b.rs"]);
    repo.spec.workdir = dir.path().to_owned();
    repo.diff_state.diff_target = Some(DiffTarget::working_tree(path.into(), DiffArea::Unstaged));
    repo.diff_state.content_preview = true;
    apply_state(cx, view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                set_ready_worktree_preview(
                    pane,
                    dir.path().join(path),
                    Arc::new(source.lines().map(str::to_owned).collect()),
                    source.len(),
                    cx,
                );
                window.focus(&pane.diff_panel_focus_handle, cx);
            });
    });
    wait_until(cx, "whole-file preview laid out", |cx| {
        cx.update(|_, app| {
            let pane = view.read(app).diff_renderer.read(app);
            pane.worktree_preview_scroll
                .0
                .borrow()
                .base_handle
                .max_offset()
                .y
                > px(0.0)
        })
    });
    dir
}

#[gpui::test]
fn alt_arrows_navigate_from_whole_file_preview(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let _dir = preview_fixture(cx, &view, false);
    cx.simulate_keystrokes("alt-down");
    wait_until_store_diff_target_path(cx, &view, std::path::Path::new("b.rs"));
}

fn page_preview(cx: &mut gpui::TestAppContext, markdown: bool) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let _dir = preview_fixture(cx, &view, markdown);
    cx.simulate_keystrokes("pagedown");
    draw_and_drain_test_window(cx);
    // Pixels order by total_cmp, so an unscrolled -0.0 would pass `< px(0.0)`.
    assert!(cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .read(app)
            .worktree_preview_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y
            < px(-1.0)
    }));
    cx.simulate_keystrokes("pageup");
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_, app| view
            .read(app)
            .diff_renderer
            .read(app)
            .worktree_preview_scroll
            .0
            .borrow()
            .base_handle
            .offset()
            .y),
        px(0.0)
    );
}

#[gpui::test]
fn page_keys_scroll_visible_file_preview(cx: &mut gpui::TestAppContext) {
    page_preview(cx, false);
}

#[gpui::test]
fn page_keys_scroll_visible_markdown_preview(cx: &mut gpui::TestAppContext) {
    page_preview(cx, true);
}

#[gpui::test]
fn status_list_does_not_forward_diff_only_keys(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut repo = working_copy_repo(&[], &["a.rs"]);
    repo.diff_state.diff_target = Some(DiffTarget::working_tree("a.rs".into(), DiffArea::Unstaged));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    diff_content(cx, &view, false, false);
    cx.update(|_, app| {
        view.read(app)
            .diff_renderer
            .clone()
            .update(app, |pane, cx| {
                pane.set_diff_view_mode(DiffViewMode::Inline, cx)
            })
    });
    focus_status(cx, &view, StatusSection::CombinedUnstaged);
    cx.simulate_keystrokes("alt-s");
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.update(|_, app| view.read(app).diff_renderer.read(app).diff_view),
        DiffViewMode::Inline
    );
}

#[gpui::test]
fn tab_navigation_reaches_graph_and_file_lists(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    prepare(cx, &view);
    let mut reached_graph = false;
    let mut reached_files = false;
    for _ in 0..100 {
        cx.update(|window, app| window.focus_next(app));
        reached_graph |= graph_focused(cx, &view);
        reached_files |= files_focused(cx, &view);
        if reached_graph && reached_files {
            break;
        }
    }
    assert!(
        reached_graph && reached_files,
        "Tab reaches graph={reached_graph}, files={reached_files}"
    );
}
