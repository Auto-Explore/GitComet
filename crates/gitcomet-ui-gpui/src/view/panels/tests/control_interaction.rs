use super::shortcuts::{app_state_with_active_repo, apply_state, shortcut_fixture_repo};
use super::*;
use crate::test_support::{painted_control_quads as paint, refresh_and_draw};

fn toolbar_repo() -> RepoState {
    let mut repo = shortcut_fixture_repo(RepoId(1), Path::new("/tmp"), &CommitId("abc123".into()));
    repo.branches = Loadable::Ready(Arc::new(vec![gitcomet_core::domain::Branch {
        name: "main".into(),
        target: CommitId("abc123".into()),
        upstream: Some(gitcomet_core::domain::Upstream {
            remote: "origin".into(),
            branch: "main".into(),
        }),
        divergence: None,
    }]));
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: Arc::new(vec![]),
            unstaged: Arc::new(vec![gitcomet_core::domain::FileStatus {
                path: "file.txt".into(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }]),
        }
        .into(),
    );
    repo
}

fn leave_controls(cx: &mut gpui::VisualTestContext) {
    cx.simulate_mouse_move(point(px(400.0), px(300.0)), None, Modifiers::default());
    cx.run_until_parked();
    refresh_and_draw(cx);
}

#[gpui::test]
fn toolbar_menu_highlights_end_on_escape_and_outside_click(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    apply_state(cx, &view, app_state_with_active_repo(toolbar_repo()));
    cx.update(|_, app| crate::app::bind_text_input_keys_for_test(app));
    for selector in ["push_menu", "pull_menu", "stash", "bottom_status_bar_zoom"] {
        for escape in [true, false] {
            leave_controls(cx);
            let resting = paint(cx, selector);
            let bounds = cx
                .debug_bounds(selector)
                .expect("menu button must be drawn");
            cx.simulate_click(bounds.center(), Modifiers::default());
            leave_controls(cx);
            cx.update(|_, app| assert!(view.read(app).popover_host.read(app).is_open()));
            assert_ne!(
                paint(cx, selector),
                resting,
                "{selector} must show its open surface"
            );
            if escape {
                cx.simulate_keystrokes("escape");
            } else {
                cx.simulate_click(point(px(2.0), px(2.0)), Modifiers::default());
            }
            leave_controls(cx);
            cx.update(|_, app| assert!(!view.read(app).popover_host.read(app).is_open()));
            assert_eq!(
                paint(cx, selector),
                resting,
                "{selector} must clear when dismissed"
            );
        }
    }
}

#[gpui::test]
fn toolbar_busy_fill_lasts_until_each_operation_counter_reaches_zero(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut repo = toolbar_repo();
    apply_state(cx, &view, app_state_with_active_repo(repo.clone()));
    leave_controls(cx);
    let push_rest = paint(cx, "push_main");
    let pull_rest = paint(cx, "pull_main");
    let push_menu_rest = paint(cx, "push_menu");
    let pull_menu_rest = paint(cx, "pull_menu");
    for (pushes, pulls) in [(2, 1), (1, 0), (0, 0)] {
        repo.push_in_flight = pushes;
        repo.pull_in_flight = pulls;
        repo.ops_rev += 1;
        apply_state(cx, &view, app_state_with_active_repo(repo.clone()));
        leave_controls(cx);
        assert_eq!(paint(cx, "push_main") == push_rest, pushes == 0);
        assert_eq!(paint(cx, "pull_main") == pull_rest, pulls == 0);
        assert_eq!(paint(cx, "push_menu"), push_menu_rest);
        assert_eq!(paint(cx, "pull_menu"), pull_menu_rest);
    }
}

#[gpui::test]
fn footer_panel_toggles_keep_the_same_resting_fill_in_both_states(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    apply_state(cx, &view, app_state_with_active_repo(toolbar_repo()));
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.set_sidebar_collapsed(false, cx);
            view.set_details_collapsed(false, cx);
        })
    });
    for selector in ["sidebar_toggle", "details_toggle"] {
        leave_controls(cx);
        let visible = paint(cx, selector);
        let position = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_click(position, Modifiers::default());
        leave_controls(cx);
        cx.update(|_, app| {
            let view = view.read(app);
            assert!(if selector == "sidebar_toggle" {
                view.sidebar_collapsed
            } else {
                view.details_collapsed
            });
        });
        assert_eq!(
            paint(cx, selector),
            visible,
            "{selector} must keep its resting fill after hiding its panel"
        );
        let position = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_click(position, Modifiers::default());
        leave_controls(cx);
        assert_eq!(
            paint(cx, selector),
            visible,
            "{selector} must keep its resting fill after showing its panel"
        );
    }
}

#[gpui::test]
fn sidebar_tabs_preserve_selected_fill_when_hovered(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    for (mode, selector) in [
        (
            gitcomet_state::model::SidebarMode::Branches,
            "sidebar_tab_branches",
        ),
        (
            gitcomet_state::model::SidebarMode::Files,
            "sidebar_tab_files",
        ),
    ] {
        let mut state = app_state_with_active_repo(toolbar_repo());
        Arc::make_mut(&mut state).sidebar_mode = mode;
        apply_state(cx, &view, state);
        cx.update(|_, app| view.update(app, |view, cx| view.set_sidebar_collapsed(false, cx)));
        leave_controls(cx);
        let selected = paint(cx, selector);
        let position = cx.debug_bounds(selector).unwrap().center();
        cx.simulate_mouse_move(position, None, Modifiers::default());
        refresh_and_draw(cx);
        assert_eq!(
            paint(cx, selector),
            selected,
            "{selector} must retain selection on hover"
        );
        leave_controls(cx);
        assert_eq!(paint(cx, selector), selected);
    }
}

#[gpui::test]
fn repository_sort_preserves_its_open_fill_on_hover_and_clears_when_closed(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        view.update(app, |view, cx| {
            view.open_popover_at(
                PopoverKind::RepoPicker,
                point(px(72.0), px(72.0)),
                window,
                cx,
            );
        })
    });
    leave_controls(cx);
    let resting = paint(cx, "repo_picker_sort_toggle");
    let position = cx.debug_bounds("repo_picker_sort_toggle").unwrap().center();
    cx.simulate_click(position, Modifiers::default());
    leave_controls(cx);
    let open = paint(cx, "repo_picker_sort_toggle");
    assert_ne!(open, resting);
    cx.simulate_mouse_move(position, None, Modifiers::default());
    refresh_and_draw(cx);
    assert_eq!(paint(cx, "repo_picker_sort_toggle"), open);
    cx.simulate_click(position, Modifiers::default());
    leave_controls(cx);
    assert_eq!(paint(cx, "repo_picker_sort_toggle"), resting);
}
