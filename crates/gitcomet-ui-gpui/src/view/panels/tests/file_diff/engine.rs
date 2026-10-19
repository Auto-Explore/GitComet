//! The viewer's diff engine override and what it leaves behind.

use super::*;
use gitcomet_core::diff_engine::{DiffEngine, DiffEngineStatus};

fn difftastic_by_default(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.set_diff_whitespace_mode(DiffWhitespaceMode::Ignore, cx);
                pane.set_diff_engine_default(DiffEngine::Difftastic, cx);
            });
        });
    });
}

fn assert_formatting_only_collapsed_diff_notice(
    cx: &mut gpui::TestAppContext,
    diff_view: DiffViewMode,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    difftastic_by_default(cx, &view);
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(940),
        "formatting_only_canvas",
        PathBuf::from("examples/01-formatting.js"),
        concat!(
            "@@ -1,3 +1,11 @@\n",
            " function buildProfile(user) {\n",
            "-  return { id: user.id, name: user.name, roles: [\"reader\", \"editor\"], active: true };\n",
            "+  return {\n",
            "+    id: user.id,\n",
            "+    name: user.name,\n",
            "+    roles: [\n",
            "+      \"reader\",\n",
            "+      \"editor\"\n",
            "+    ],\n",
            "+    active: true\n",
            "+  };\n",
            " }\n",
        )
        .to_string(),
        concat!(
            "function buildProfile(user) {\n",
            "  return { id: user.id, name: user.name, roles: [\"reader\", \"editor\"], active: true };\n",
            "}\n",
        )
        .to_string(),
        concat!(
            "function buildProfile(user) {\n",
            "  return {\n",
            "    id: user.id,\n",
            "    name: user.name,\n",
            "    roles: [\n",
            "      \"reader\",\n",
            "      \"editor\"\n",
            "    ],\n",
            "    active: true\n",
            "  };\n",
            "}\n",
        )
        .to_string(),
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "the formatting-only JavaScript comparison",
        |pane| {
            pane.file_diff_cache_target == Some(target.clone())
                && pane.diff_engine_status
                    == DiffEngineStatus::Difftastic {
                        syntactic_changes: false,
                        text_differs: true,
                    }
        },
        |pane| pane.diff_engine_status,
    );
    cx.update(|_window, app| {
        let renderer = view.read(app).diff_renderer.clone();
        renderer.update(app, |pane, cx| {
            pane.diff_view = diff_view;
            pane.set_diff_content_mode(DiffContentMode::Collapsed, cx);
        });
    });
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert!(pane.is_collapsed_diff_projection_active());
        assert!(pane.collapsed_diff_visible_rows.is_empty());
    });
    let notice = cx
        .debug_bounds("difftastic_text_only_canvas")
        .expect("the canvas explains that text differs without syntactic changes");
    assert!(
        cx.debug_bounds("diff_engine_notice").is_none(),
        "the canvas notice replaces the extra engine header"
    );
    assert!(cx.debug_bounds("diff_text_empty_space_Inline").is_none());
    for selector in ["difftastic_show_full", "difftastic_show_line_changes"] {
        let button = cx
            .debug_bounds(selector)
            .expect("the canvas offers both views");
        assert!(notice.contains(&button.center()));
    }
    let show_full = debug_selector_center(cx, "difftastic_show_full");
    cx.simulate_mouse_move(show_full, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        show_full,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        show_full,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.effective_diff_content_mode(), DiffContentMode::Full);
        assert!(pane.diff_visible_len() > 0);
    });
    assert!(cx.debug_bounds("difftastic_text_only_canvas").is_none());
    set_diff_content_mode_for_test(cx, &view, DiffContentMode::Collapsed);
    let show_lines = debug_selector_center(cx, "difftastic_show_line_changes");
    cx.simulate_mouse_move(show_lines, None, gpui::Modifiers::default());
    cx.simulate_mouse_down(
        show_lines,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    cx.simulate_mouse_up(
        show_lines,
        gpui::MouseButton::Left,
        gpui::Modifiers::default(),
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "line changes after leaving the formatting-only canvas",
        |pane| {
            pane.diff_engine_status == DiffEngineStatus::LineByLine && pane.diff_visible_len() > 0
        },
        |pane| (pane.diff_engine_status, pane.diff_visible_len()),
    );
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(pane.diff_engine_override(), Some(DiffEngine::LineByLine));
        assert_eq!(pane.diff_whitespace_mode, DiffWhitespaceMode::Show);
    });
    assert!(cx.debug_bounds("difftastic_text_only_canvas").is_none());
}

#[gpui::test]
fn difftastic_formatting_only_collapsed_inline_diff_explains_text_changes_in_canvas(
    cx: &mut gpui::TestAppContext,
) {
    assert_formatting_only_collapsed_diff_notice(cx, DiffViewMode::Inline);
}

#[gpui::test]
fn difftastic_formatting_only_collapsed_split_diff_explains_text_changes_in_canvas(
    cx: &mut gpui::TestAppContext,
) {
    assert_formatting_only_collapsed_diff_notice(cx, DiffViewMode::Split);
}

fn close_viewer(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<GitCometView>,
    repo_id: gitcomet_state::model::RepoId,
) {
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.clear_diff_selection_or_exit(repo_id, cx)
            });
        });
    });
}

/// Hands the pane the store's state once it has handled the close.
fn publish_cleared_viewer(cx: &mut gpui::VisualTestContext, view: &gpui::Entity<GitCometView>) {
    crate::view::test_support::drain_store_worker(view, cx);
    let state = cx.update(|_window, app| view.read(app).store.snapshot());
    assert!(
        state.repos[0].diff_state.diff_target.is_none(),
        "the store cleared the viewer"
    );
    cx.update(|_window, app| view.update(app, |this, cx| push_test_state(this, state, cx)));
    wait_for_main_pane_condition(
        cx,
        view,
        "the viewer cleared",
        |pane| pane.rendered_diff_target().is_none(),
        |pane| pane.rendered_diff_target().cloned(),
    );
}

/// "Show line changes" switches the viewer to the line engine with whitespace
/// shown. Leaving the line engine kept whitespace shown for every later file,
/// while the saved preference still ignored it.
#[gpui::test]
fn leaving_show_line_changes_restores_the_whitespace_mode(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    difftastic_by_default(cx, &view);
    let repo_id = gitcomet_state::model::RepoId(935);
    for leave_by_closing in [false, true] {
        let target = push_regular_diff_content_mode_state(
            cx,
            &view,
            repo_id,
            "show_line_changes",
            PathBuf::from("main.rs"),
            "@@ -1 +1 @@\n-fn a( x: i32 ) {}\n+fn a(x: i32) {}\n".to_string(),
            "fn a( x: i32 ) {}\n".to_string(),
            "fn a(x: i32) {}\n".to_string(),
        );
        wait_for_main_pane_condition(
            cx,
            &view,
            "a formatting-only Difftastic result",
            |pane| {
                pane.file_diff_cache_target == Some(target.clone())
                    && pane.diff_engine_status
                        == DiffEngineStatus::Difftastic {
                            syntactic_changes: false,
                            text_differs: true,
                        }
            },
            |pane| pane.diff_engine_status,
        );
        let button = cx
            .debug_bounds("difftastic_show_line_changes")
            .expect("the formatting-only notice offers line changes")
            .center();
        cx.simulate_mouse_move(button, None, gpui::Modifiers::default());
        cx.simulate_mouse_down(button, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.simulate_mouse_up(button, gpui::MouseButton::Left, gpui::Modifiers::default());
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(pane.diff_engine_override(), Some(DiffEngine::LineByLine));
            assert_eq!(pane.diff_whitespace_mode, DiffWhitespaceMode::Show);
        });
        if leave_by_closing {
            close_viewer(cx, &view, repo_id);
            publish_cleared_viewer(cx, &view);
        } else {
            cx.update(|_window, app| {
                view.update(app, |this, cx| {
                    this.diff_renderer
                        .update(cx, |pane, cx| pane.set_diff_engine_override(None, cx));
                });
            });
        }
        cx.update(|_window, app| {
            let pane = view.read(app).diff_renderer.read(app);
            assert_eq!(
                pane.diff_engine_override(),
                None,
                "closing={leave_by_closing}"
            );
            assert_eq!(
                pane.diff_whitespace_mode,
                DiffWhitespaceMode::Ignore,
                "closing={leave_by_closing}"
            );
        });
    }
}

/// Closing the viewer reset its override at once, which rebuilt the closing
/// file with the default engine: a Difftastic comparison started for a file
/// that was about to be cleared.
#[gpui::test]
fn closing_the_viewer_resets_its_override_without_rebuilding_the_file(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    difftastic_by_default(cx, &view);
    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.diff_renderer.update(cx, |pane, cx| {
                pane.set_diff_engine_override(Some(DiffEngine::LineByLine), cx)
            });
        });
    });
    let repo_id = gitcomet_state::model::RepoId(936);
    let push = |cx: &mut gpui::VisualTestContext| {
        push_regular_diff_content_mode_state(
            cx,
            &view,
            repo_id,
            "close_resets_override",
            PathBuf::from("main.rs"),
            "@@ -1 +1 @@\n-fn a() -> i32 { 1 }\n+fn a() -> i32 { 2 }\n".to_string(),
            "fn a() -> i32 { 1 }\n".to_string(),
            "fn a() -> i32 { 2 }\n".to_string(),
        )
    };
    let target = push(cx);
    wait_for_main_pane_condition(
        cx,
        &view,
        "the file built by the line engine",
        |pane| {
            pane.file_diff_cache_target == Some(target.clone())
                && pane.file_diff_cache_inflight.is_none()
                && pane.diff_engine_status == DiffEngineStatus::LineByLine
        },
        |pane| (pane.file_diff_cache_inflight, pane.diff_engine_status),
    );
    let built =
        cx.update(|_window, app| view.read(app).diff_renderer.read(app).file_diff_cache_seq);
    close_viewer(cx, &view, repo_id);
    draw_and_drain_test_window(cx);
    cx.update(|_window, app| {
        let pane = view.read(app).diff_renderer.read(app);
        assert_eq!(
            pane.file_diff_cache_seq, built,
            "the closing file was rebuilt"
        );
    });
    publish_cleared_viewer(cx, &view);
    cx.update(|_window, app| {
        assert_eq!(
            view.read(app)
                .diff_renderer
                .read(app)
                .diff_engine_override(),
            None
        );
    });
    let target = push(cx);
    wait_for_main_pane_condition(
        cx,
        &view,
        "the reopened file compared by the default engine",
        |pane| pane.file_diff_cache_target == Some(target.clone()) && pane.structural_diff_active(),
        |pane| pane.diff_engine_status,
    );
}

/// A whitespace mode the user picks while "Show line changes" lasts is their
/// choice now: ending the override must not put the earlier one back.
#[gpui::test]
fn a_whitespace_mode_picked_during_line_changes_outlives_them(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::super::GitCometView::new(store, events, None, window, cx)
    });
    difftastic_by_default(cx, &view);
    let target = push_regular_diff_content_mode_state(
        cx,
        &view,
        gitcomet_state::model::RepoId(939),
        "picked_whitespace",
        PathBuf::from("main.rs"),
        "@@ -1 +1 @@\n-fn a( x: i32 ) {}\n+fn a(x: i32) {}\n".to_string(),
        "fn a( x: i32 ) {}\n".to_string(),
        "fn a(x: i32) {}\n".to_string(),
    );
    wait_for_main_pane_condition(
        cx,
        &view,
        "a formatting-only Difftastic result",
        |pane| {
            pane.file_diff_cache_target == Some(target.clone())
                && pane.diff_engine_status
                    == DiffEngineStatus::Difftastic {
                        syntactic_changes: false,
                        text_differs: true,
                    }
        },
        |pane| pane.diff_engine_status,
    );
    // The persisting setter reports to the root view, so the pane is updated
    // on its own rather than inside the root's update.
    let renderer = cx.update(|_window, app| view.read(app).diff_renderer.clone());
    cx.update(|_window, app| {
        renderer.update(app, |pane, cx| {
            pane.show_line_changes(cx);
            pane.set_diff_whitespace_mode_and_persist(DiffWhitespaceMode::Show, cx);
            pane.set_diff_engine_override(None, cx);
            assert_eq!(pane.diff_whitespace_mode, DiffWhitespaceMode::Show);
        });
    });
}
