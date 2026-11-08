use super::*;

#[gpui::test]
fn repo_tabs_can_drag_reorder_by_right_half(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_right_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        _view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[0];
    let target = repo_ids[1];
    let expected = vec![repo_ids[1], repo_ids[0], repo_ids[2]];

    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");
    let target_bounds = cx
        .debug_bounds(repo_tab_selector(target))
        .expect("expected target repo tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x + px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    let drop = gpui::point(target_bounds.right() - px(5.0), target_bounds.center().y);
    cx.simulate_mouse_move(drop, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(drop, MouseButton::Left, Modifiers::default());

    wait_for_repo_order(&store_for_test, &expected);
}

#[gpui::test]
fn repo_tab_reorder_does_not_bounce_on_minor_pointer_reversal(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_jitter_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[0];
    let expected = vec![repo_ids[1], repo_ids[0], repo_ids[2]];
    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repository tab bounds");
    let target_bounds = cx
        .debug_bounds(repo_tab_selector(repo_ids[1]))
        .expect("expected target repository tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x + px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    // Cross the rightward takeover point, then move back only a few pixels
    // while the displaced neighbour is still animating through that point.
    let takeover = gpui::point(
        target_bounds.left() + target_bounds.size.width * 0.45,
        target_bounds.center().y,
    );
    cx.simulate_mouse_move(takeover, Some(MouseButton::Left), Modifiers::default());
    wait_for_repo_order(&store_for_test, &expected);
    sync_view_for_tests(cx, &view);

    let jitter = gpui::point(takeover.x - px(5.0), takeover.y);
    cx.simulate_mouse_move(jitter, Some(MouseButton::Left), Modifiers::default());
    sync_view_for_tests(cx, &view);

    let got = store_for_test
        .snapshot()
        .repos
        .iter()
        .map(|repo| repo.id)
        .collect::<Vec<_>>();
    assert_eq!(
        got, expected,
        "a small reverse movement must not make adjacent tabs swap back"
    );

    cx.simulate_mouse_up(jitter, MouseButton::Left, Modifiers::default());
}

#[gpui::test]
fn repo_tabs_can_drag_reorder_by_left_half(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_left_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        _view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[2];
    let target = repo_ids[1];
    let expected = vec![repo_ids[0], repo_ids[2], repo_ids[1]];

    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");
    let target_bounds = cx
        .debug_bounds(repo_tab_selector(target))
        .expect("expected target repo tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x - px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    let drop = gpui::point(target_bounds.left() + px(5.0), target_bounds.center().y);
    cx.simulate_mouse_move(drop, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(drop, MouseButton::Left, Modifiers::default());

    wait_for_repo_order(&store_for_test, &expected);
}

#[gpui::test]
fn repo_tabs_drop_on_self_is_noop(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_self_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        _view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let dragged = repo_ids[1];
    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());

    let moved_x = (start.x + px(10.0)).min(dragged_bounds.right() - px(1.0));
    let moved = gpui::point(moved_x, start.y);
    cx.simulate_mouse_move(moved, Some(MouseButton::Left), Modifiers::default());
    cx.simulate_mouse_up(moved, MouseButton::Left, Modifiers::default());

    let got = store_for_test
        .snapshot()
        .repos
        .iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    assert_eq!(got, repo_ids);
}

#[gpui::test]
fn repo_tabs_middle_click_closes_inactive_tab_without_reactivating(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_middle_close_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    let active_repo = repo_ids[0];
    let closed_repo = repo_ids[1];
    click_debug_selector_with_button(cx, repo_tab_selector(closed_repo), MouseButton::Middle, 1);

    let state = wait_for_repo_count(&store_for_test, 2);
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        vec![repo_ids[0], repo_ids[2]]
    );
    assert_eq!(state.active_repo, Some(active_repo));

    sync_view_for_tests(cx, &view);
    assert!(
        cx.debug_bounds(repo_tab_selector(closed_repo)).is_none(),
        "expected middle-clicked repo tab to be removed from the UI"
    );
}

#[gpui::test]
fn repo_tabs_right_click_does_not_close(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_repo_tabs_right_click_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        view.clone(),
        vec![base.join("repo1"), base.join("repo2"), base.join("repo3")],
    );

    click_debug_selector_with_button(cx, repo_tab_selector(repo_ids[1]), MouseButton::Right, 1);

    let state = store_for_test.snapshot();
    assert_eq!(
        state.repos.iter().map(|repo| repo.id).collect::<Vec<_>>(),
        repo_ids
    );
    assert_eq!(state.active_repo, Some(repo_ids[0]));

    sync_view_for_tests(cx, &view);
    assert!(
        cx.debug_bounds(repo_tab_selector(repo_ids[1])).is_some(),
        "expected right-clicked repo tab to remain visible"
    );
}

#[gpui::test]
fn listed_worktree_badge_double_click_opens_closed_repo_tab(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (_view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_open_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, _view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &_view, repo_id);

    let linked_repo = base.join("repo-feature");
    store_for_test.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![Branch {
                name: "feature/workspace".to_string(),
                target: CommitId("deadbeef".into()),
                upstream: None,
                divergence: None,
            }]),
        },
    ));
    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            bare: false,
            path: linked_repo.clone(),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &_view, "branch_worktree_badge", 64);
    let badge_selector = debug_selector("branch_worktree_badge", badge_ix);
    click_debug_selector(cx, badge_selector, 2);

    wait_until("linked repository tab to open from badge", || {
        let snapshot = store_for_test.snapshot();
        snapshot
            .repos
            .iter()
            .any(|repo| repo.spec.workdir == linked_repo)
            && snapshot
                .active_repo
                .and_then(|active_repo| snapshot.repos.iter().find(|repo| repo.id == active_repo))
                .is_some_and(|repo| repo.spec.workdir == linked_repo)
    });
}

#[gpui::test]
fn repo_tab_strip_plus_button_opens_add_repo_menu(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store_for_view, events, None, window, cx)
    });
    seed_worktree_repo(
        cx,
        &store,
        view.clone(),
        PathBuf::from("/tmp/gitcomet-smoke-add-repo-menu-test"),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let plus_bounds = cx
        .debug_bounds("add_repo_menu")
        .expect("expected + button bounds after the repo tabs");
    cx.simulate_mouse_move(plus_bounds.center(), None, Modifiers::default());
    cx.simulate_mouse_down(
        plus_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.simulate_mouse_up(
        plus_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::add_repo_menu_is_open(view.read(app), app),
            "expected the + button to open the add-repository menu"
        );
    });
    assert!(
        cx.debug_bounds("add_repo_menu_init").is_some(),
        "expected the add-repository menu to include initialize repository"
    );
}

#[gpui::test]
fn titlebar_only_reserves_visible_repository_controls_from_window_drag(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    let repo_ids = open_repo_tabs(
        cx,
        &store_for_test,
        view.clone(),
        "titlebar_visible_hitboxes",
        30,
    );

    let drag_surface = cx
        .debug_bounds("titlebar_drag")
        .expect("expected the shared title-bar drag surface");
    let repo_tab_bounds = cx
        .debug_bounds(repo_tab_selector(repo_ids[0]))
        .expect("expected repository tab bounds");
    let repo_tab_viewport = cx.update(|_window, app| {
        crate::view::test_support::repo_tab_strip_viewport(view.read(app), app)
    });
    assert_eq!(
        (repo_tab_viewport.top(), repo_tab_viewport.bottom()),
        (repo_tab_bounds.top(), repo_tab_bounds.bottom()),
        "expected the scrollable tab strip hitbox to match the visible tab row"
    );
    let mut controls = vec![
        (
            "repository picker",
            cx.debug_bounds("repo_picker_toggle")
                .expect("expected repository picker bounds"),
        ),
        ("repository tab", repo_tab_bounds),
        (
            "add repository",
            cx.debug_bounds("add_repo_menu")
                .expect("expected add repository button bounds"),
        ),
    ];
    if !cfg!(target_os = "macos") {
        controls.push((
            "application menu",
            cx.debug_bounds("app_menu")
                .expect("expected application menu bounds"),
        ));
    }

    for (name, bounds) in controls {
        let above = gpui::point(bounds.center().x, drag_surface.top() + px(1.0));
        assert!(
            above.y < bounds.top(),
            "expected title chrome above the visible {name}"
        );

        cx.simulate_mouse_move(above, None, Modifiers::default());
        cx.simulate_mouse_down(above, MouseButton::Left, Modifiers::default());
        cx.update(|_window, app| {
            assert!(
                crate::view::test_support::titlebar_drag_is_armed(view.read(app), app),
                "expected the area above the visible {name} to arm a window drag"
            );
        });
        cx.simulate_mouse_up(above, MouseButton::Left, Modifiers::default());

        cx.simulate_mouse_move(bounds.center(), None, Modifiers::default());
        cx.simulate_mouse_down(bounds.center(), MouseButton::Left, Modifiers::default());
        cx.update(|_window, app| {
            assert!(
                !crate::view::test_support::titlebar_drag_is_armed(view.read(app), app),
                "expected the visible {name} to keep its own interaction"
            );
        });
        cx.simulate_mouse_up(above, MouseButton::Left, Modifiers::default());
    }
}

#[gpui::test]
fn repo_tabs_scroll_with_the_wheel_once_they_overflow(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_wheel", 30);

    let (scrolled, max_scroll) = repo_tab_scroll(cx, &view);
    assert_eq!(scrolled, px(0.0), "expected the strip to start unscrolled");
    assert!(
        max_scroll > px(0.0),
        "expected 30 repository tabs to overflow the strip, got max scroll {max_scroll:?}"
    );

    // A plain vertical wheel has to move the strip: it scrolls on one axis and
    // there is no scrollbar to drag.
    let over_tabs = cx
        .debug_bounds(repo_tab_selector(repo_ids[0]))
        .expect("expected first repo tab bounds")
        .center();
    scroll_over(cx, over_tabs, px(-200.0));
    sync_view_for_tests(cx, &view);

    let (scrolled, _) = repo_tab_scroll(cx, &view);
    assert_eq!(
        scrolled,
        px(200.0),
        "expected the wheel to scroll the repository tab strip"
    );

    scroll_over(cx, over_tabs, px(500.0));
    sync_view_for_tests(cx, &view);

    let (scrolled, _) = repo_tab_scroll(cx, &view);
    assert_eq!(
        scrolled,
        px(0.0),
        "expected scrolling back to stop at the start of the strip"
    );
}

#[gpui::test]
fn repo_tab_strip_never_renders_scroll_arrows(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let roomy_repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_no_arrows_few", 2);
    let roomy_tab = cx
        .debug_bounds(repo_tab_selector(roomy_repo_ids[0]))
        .expect("expected a repository tab in the roomy strip");
    let roomy_label = cx
        .debug_bounds(repo_tab_label_selector(roomy_repo_ids[0]))
        .expect("expected a repository label in the roomy strip");
    let roomy_leading_inset = roomy_label.left() - roomy_tab.left();

    assert!(
        cx.debug_bounds("tab_bar_scroll_left").is_none()
            && cx.debug_bounds("tab_bar_scroll_right").is_none(),
        "expected no scroll arrows while every tab fits"
    );

    let repos: Vec<PathBuf> = (0..30)
        .map(|ix| {
            std::env::temp_dir()
                .join(format!(
                    "gitcomet_ui_test_tab_no_arrows_{}",
                    std::process::id()
                ))
                .join(format!("repository-number-{ix}"))
        })
        .collect();
    // Restore preserves already-open tabs. Close the roomy fixture before
    // replacing it with the dense one.
    store_for_test.dispatch(Msg::CloseRepos {
        repo_ids: roomy_repo_ids,
        activate_after: None,
    });
    let dense_repo_ids = restore_session_and_draw(cx, &store_for_test, view.clone(), repos);
    sync_view_for_tests(cx, &view);

    assert!(
        cx.debug_bounds("tab_bar_scroll_left").is_none()
            && cx.debug_bounds("tab_bar_scroll_right").is_none(),
        "expected overflowing repository tabs to remain free of scroll arrows"
    );
    let dense_tab = cx
        .debug_bounds(repo_tab_selector(dense_repo_ids[0]))
        .expect("expected a repository tab in the dense strip");
    let dense_label = cx
        .debug_bounds(repo_tab_label_selector(dense_repo_ids[0]))
        .expect("expected a repository label in the dense strip");
    let dense_leading_inset = dense_label.left() - dense_tab.left();
    assert!(
        (f32::from(dense_leading_inset) - f32::from(roomy_leading_inset)).abs() <= 0.5,
        "expected fixed side padding at every tab-strip density, got \
         roomy={roomy_leading_inset:?}, dense={dense_leading_inset:?}"
    );
    let drag_region = cx
        .debug_bounds("titlebar_drag")
        .expect("expected a window drag region after overflowing repository tabs");
    assert!(
        drag_region.size.width >= px(60.0),
        "expected repository tabs to leave a useful window drag region, got {:?}",
        drag_region.size.width
    );
}

#[gpui::test]
fn roomy_repo_tab_expands_to_show_its_full_repository_name(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    let workdir =
        std::env::temp_dir().join("repository-with-a-long-descriptive-name-that-still-fits");
    let repo_ids = restore_session_and_draw(cx, &store_for_test, view.clone(), vec![workdir]);
    let repo_id = repo_ids[0];

    let tab = cx
        .debug_bounds(repo_tab_selector(repo_id))
        .expect("expected roomy repository tab bounds");
    let label = cx
        .debug_bounds(repo_tab_label_selector(repo_id))
        .expect("expected roomy repository label bounds");
    let natural_text = cx
        .debug_bounds(repo_tab_label_text_selector(repo_id))
        .expect("expected natural repository label text bounds");

    assert!(
        tab.size.width > px(180.0),
        "expected a roomy tab to expand beyond the former 180px cap, got {:?}",
        tab.size.width
    );
    assert!(
        label.size.width + px(0.5) >= natural_text.size.width,
        "expected the full repository name to fit without fading (label={:?}, text={:?})",
        label.size.width,
        natural_text.size.width
    );
    assert_eq!(
        repo_tab_scroll(cx, &view).1,
        px(0.0),
        "expected the single naturally sized tab not to overflow the strip"
    );
}

/// AppKit paints the traffic lights straight over our bar, so the bar has to
/// keep its own leading control out from under them. Nothing else measures the
/// reserved inset against what actually gets painted.
#[cfg(target_os = "macos")]
#[gpui::test]
fn the_title_bar_keeps_its_leading_control_clear_of_the_traffic_lights(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_traffic_lights_{}",
        std::process::id()
    ));
    restore_session_and_draw(cx, &store_for_test, view.clone(), vec![workdir]);

    let picker = cx
        .debug_bounds("repo_picker_toggle")
        .expect("expected repository picker bounds");

    assert!(
        picker.left() >= crate::kit::window_chrome::MACOS_TRAFFIC_LIGHTS_SAFE_INSET,
        "the first control in the bar starts at {:?}, inside the {:?} reserved for the lights",
        picker.left(),
        crate::kit::window_chrome::MACOS_TRAFFIC_LIGHTS_SAFE_INSET
    );
}

/// The window chrome is deliberately outside the UI scale: a title bar shares
/// its row with the OS window controls, which do not resize, so the bar, its
/// buttons and the repository tabs must not move when the workspace under them
/// zooms.
#[gpui::test]
fn the_title_bar_and_repository_tabs_hold_their_size_at_every_ui_scale(
    cx: &mut gpui::TestAppContext,
) {
    // Width matters as much as height here: the scale parameter this change
    // dropped from `Tab::natural_width` governs the strip's widths.
    fn box_of(
        cx: &mut gpui::VisualTestContext,
        selector: &'static str,
        at: u32,
    ) -> gpui::Size<gpui::Pixels> {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("missing {selector} at {at}% UI scale"))
            .size
    }
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_fixed_chrome_{}",
        std::process::id()
    ));
    let repo_id = restore_session_and_draw(cx, &store_for_test, view.clone(), vec![workdir])[0];

    let fixed = [
        "titlebar_drag",
        "repo_picker_toggle",
        "add_repo_menu",
        repo_tab_selector(repo_id),
        repo_tab_label_text_selector(repo_id),
    ];
    // Proves the zoom actually landed: without it every assertion below would
    // also hold for a scale change that never took effect.
    let zooms = "sidebar_tab_files";

    let default_percent = crate::ui_scale::DEFAULT_UI_SCALE_PERCENT;
    let at_default = fixed.map(|selector| box_of(cx, selector, default_percent));
    let zooms_at_default = box_of(cx, zooms, default_percent).height;

    for percent in [80, 200] {
        cx.update(|window, app| {
            view.update(app, |this, cx| {
                crate::ui_scale::set_default(cx, percent);
                this.apply_ui_scale_percent(percent, window, cx);
            });
        });
        sync_view_for_tests(cx, &view);

        for (ix, selector) in fixed.into_iter().enumerate() {
            assert_eq!(
                box_of(cx, selector, percent),
                at_default[ix],
                "{selector} sits in the title bar and must not move at {percent}% UI scale"
            );
        }
        // Absolute, not merely unchanged: the bar is mounted uncached in tests,
        // so a bar that silently compressed would still "not move".
        assert_eq!(
            box_of(cx, "titlebar_drag", percent).height,
            crate::kit::window_chrome::TITLE_BAR_HEIGHT,
            "the title bar must hold its own fixed height at {percent}% UI scale"
        );
        // Re-checked every step, so neither iteration can pass vacuously.
        let zoomed = box_of(cx, zooms, percent).height;
        if percent < default_percent {
            assert!(
                zoomed < zooms_at_default,
                "the workspace under the title bar must shrink at {percent}%"
            );
        } else {
            assert!(
                zoomed > zooms_at_default,
                "the workspace under the title bar must still zoom at {percent}%"
            );
        }
    }
}

#[gpui::test]
fn repository_tabs_shrink_long_names_before_short_names(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });
    cx.simulate_resize(gpui::size(px(1800.0), px(700.0)));

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_tab_priority_{}",
        std::process::id()
    ));
    let repo_ids = restore_session_and_draw(
        cx,
        &store_for_test,
        view.clone(),
        vec![
            base.join("small-project"),
            base.join("medium-sized-repository"),
            base.join("repository-with-an-exceptionally-long-descriptive-name"),
        ],
    );
    sync_view_for_tests(cx, &view);

    let tab_widths = |cx: &mut gpui::VisualTestContext| {
        repo_ids
            .iter()
            .map(|repo_id| {
                cx.debug_bounds(repo_tab_selector(*repo_id))
                    .expect("expected repository tab bounds")
                    .size
                    .width
            })
            .collect::<Vec<_>>()
    };
    let natural = tab_widths(cx);
    assert!(natural[0] < natural[1] && natural[1] < natural[2]);
    let first_tab_left = cx
        .debug_bounds(repo_tab_selector(repo_ids[0]))
        .expect("expected first repository tab bounds")
        .left();

    // Keep the cap between the medium and long natural widths: only the long
    // tab should have to yield at this pressure level.
    let long_only_cap = (natural[1] + natural[2]) / 2.0;
    // The strip viewport holds only the tabs, so the sole extra width to fund
    // is each tab's gutter on both sides.
    let outer_chrome = px(components::Tab::HORIZONTAL_MARGIN_PX * 2.0 * repo_ids.len() as f32);
    resize_repo_tab_strip_to(
        cx,
        &view,
        natural[0] + natural[1] + long_only_cap + outer_chrome,
    );
    let long_only = tab_widths(cx);
    sync_view_for_tests(cx, &view);
    assert_eq!(
        tab_widths(cx),
        long_only,
        "repository tab widths must not adjust again on a follow-up frame"
    );
    assert_eq!(
        cx.debug_bounds(repo_tab_selector(repo_ids[0]))
            .expect("expected first repository tab bounds")
            .left(),
        first_tab_left,
        "the repository tab strip must remain anchored while resizing"
    );
    assert_eq!(long_only[0], natural[0], "short tab should stay natural");
    assert_eq!(long_only[1], natural[1], "medium tab should stay natural");
    assert!(
        long_only[2] < natural[2],
        "the longest tab should fade first"
    );

    // Push the common cap below the shortest natural tab but above the minimum
    // width floor. At this threshold all three tabs should shrink together.
    let all_tabs_cap = (px(components::Tab::MIN_WIDTH_PX) + natural[0]) / 2.0;
    resize_repo_tab_strip_to(
        cx,
        &view,
        all_tabs_cap * repo_ids.len() as f32 + outer_chrome,
    );
    let all_shrunk = tab_widths(cx);
    sync_view_for_tests(cx, &view);
    assert_eq!(
        tab_widths(cx),
        all_shrunk,
        "repository tabs must settle in the resize frame at every pressure level"
    );
    assert_eq!(
        cx.debug_bounds(repo_tab_selector(repo_ids[0]))
            .expect("expected first repository tab bounds")
            .left(),
        first_tab_left,
        "the repository tab strip must not move horizontally as all tabs shrink"
    );
    assert!(
        all_shrunk
            .iter()
            .zip(&natural)
            .all(|(shrunk, natural)| shrunk < natural),
        "every repository tab should shrink once the cap crosses the shortest name"
    );
    assert!(
        all_shrunk
            .windows(2)
            .all(|pair| (f32::from(pair[0]) - f32::from(pair[1])).abs() <= 1.0),
        "all tabs should share the same cap after the threshold: {all_shrunk:?}"
    );
}

#[gpui::test]
fn repo_tabs_show_separators_only_between_inactive_tabs(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view, "tab_inactive_separators", 3);

    assert!(
        cx.debug_bounds(repo_tab_separator_selector(repo_ids[0]))
            .is_none(),
        "expected no separator between the active tab and its inactive neighbour"
    );
    assert!(
        cx.debug_bounds(repo_tab_separator_selector(repo_ids[1]))
            .is_some(),
        "expected a separator between adjacent inactive tabs"
    );
    assert!(
        cx.debug_bounds(repo_tab_separator_selector(repo_ids[2]))
            .is_none(),
        "expected no separator after the final tab"
    );
}

#[gpui::test]
fn dragging_a_repo_tab_to_the_edge_scrolls_the_strip(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_drag_scroll", 30);
    let (scrolled, max_scroll) = repo_tab_scroll(cx, &view);
    assert_eq!(scrolled, px(0.0));
    assert!(max_scroll > px(0.0), "expected the tabs to overflow");

    let dragged = repo_ids[0];
    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");
    let strip = cx.update(|_window, app| {
        crate::view::test_support::repo_tab_strip_viewport(view.read(app), app)
    });

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x + px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    // Park the pointer just inside the strip's trailing edge and let frames run
    // without moving it again.
    let parked = gpui::point(strip.right() - px(38.0), start.y);
    cx.simulate_mouse_move(parked, Some(MouseButton::Left), Modifiers::default());
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_for_tests(cx, &view);
    }

    let (scrolled_right, _) = repo_tab_scroll(cx, &view);
    assert!(
        scrolled_right > px(0.0),
        "expected a tab held at the trailing edge to scroll the strip, got {scrolled_right:?}"
    );

    // Scrolling has to keep re-picking the drop target, or the tab would land
    // back where it started however far the strip travelled.
    let position = store_for_test
        .snapshot()
        .repos
        .iter()
        .position(|repo| repo.id == dragged)
        .expect("expected the dragged repo to still be open");
    assert!(
        position > 0,
        "expected the tabs scrolling under a parked drag to move it along, still at {position}"
    );

    // Holding at the opposite edge winds it back.
    let parked_left = gpui::point(strip.left() + px(4.0), start.y);
    cx.simulate_mouse_move(parked_left, Some(MouseButton::Left), Modifiers::default());
    for _ in 0..12 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_for_tests(cx, &view);
    }

    let (scrolled_left, _) = repo_tab_scroll(cx, &view);
    assert!(
        scrolled_left < scrolled_right,
        "expected the leading edge to scroll back, went {scrolled_right:?} -> {scrolled_left:?}"
    );

    cx.simulate_mouse_up(parked_left, MouseButton::Left, Modifiers::default());
    sync_view_for_tests(cx, &view);

    // The drag is over: the strip must sit still even with the pointer left in
    // the edge zone.
    let (settled, _) = repo_tab_scroll(cx, &view);
    for _ in 0..3 {
        std::thread::sleep(Duration::from_millis(10));
        sync_view_for_tests(cx, &view);
    }
    let (after_drop, _) = repo_tab_scroll(cx, &view);
    assert_eq!(
        settled, after_drop,
        "expected auto-scroll to stop once the tab is dropped"
    );
}

#[gpui::test]
fn dragging_a_repo_tab_past_the_strip_ends_keeps_it_inside(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_drag_clamp", 6);
    let dragged = repo_ids[2];
    let dragged_bounds = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected dragged repo tab bounds");
    let strip = cx.update(|_window, app| {
        crate::view::test_support::repo_tab_strip_viewport(view.read(app), app)
    });

    let start = dragged_bounds.center();
    cx.simulate_mouse_move(start, None, Modifiers::default());
    cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
    cx.simulate_mouse_move(
        gpui::point(start.x - px(10.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );

    // Haul it far past the leading edge, out where the sidebar lives.
    cx.simulate_mouse_move(
        gpui::point(strip.left() - px(400.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    sync_view_for_tests(cx, &view);

    let at_left = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected the dragged tab to stay mounted in the strip");
    assert!(
        at_left.left() >= strip.left() - px(1.0),
        "a tab dragged off the leading edge must stop at it \
         (tab left={:?}, strip left={:?})",
        at_left.left(),
        strip.left()
    );

    // And the same past the trailing edge, where the add-repo button sits.
    cx.simulate_mouse_move(
        gpui::point(strip.right() + px(400.0), start.y),
        Some(MouseButton::Left),
        Modifiers::default(),
    );
    sync_view_for_tests(cx, &view);

    let at_right = cx
        .debug_bounds(repo_tab_selector(dragged))
        .expect("expected the dragged tab to stay mounted after the trailing haul");
    assert!(
        at_right.right() <= strip.right() + px(1.0),
        "a tab dragged off the trailing edge must stop at it \
         (tab right={:?}, strip right={:?})",
        at_right.right(),
        strip.right()
    );

    cx.simulate_mouse_up(
        gpui::point(strip.right() + px(400.0), start.y),
        MouseButton::Left,
        Modifiers::default(),
    );
    sync_view_for_tests(cx, &view);
}

#[gpui::test]
fn add_repo_button_stays_after_the_overflowing_tab_viewport(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_plus_pinned", 30);

    let plus = cx
        .debug_bounds("add_repo_menu")
        .expect("expected the + button to stay rendered while tabs overflow");
    let strip = cx.update(|_window, app| {
        crate::view::test_support::repo_tab_strip_viewport(view.read(app), app)
    });
    let first_tab = cx
        .debug_bounds(repo_tab_selector(repo_ids[0]))
        .expect("expected first repository tab bounds");
    assert!(cx.debug_bounds("tab_bar_scroll_left").is_none());
    assert!(cx.debug_bounds("tab_bar_scroll_right").is_none());
    assert!(
        plus.left() >= strip.right() - px(1.0),
        "expected the + button immediately after the overflowing tab viewport"
    );
    assert!(
        plus.left() <= strip.right() + px(12.0),
        "expected the + button to stay attached to the overflowing tab viewport"
    );
    assert!(
        first_tab.right() <= strip.right() + px(1.0),
        "expected visible repository tabs to stay within the scroll viewport"
    );
}

#[gpui::test]
fn add_repo_button_stays_at_the_strip_end_when_tabs_fit(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_plus_sticky", 3);
    let (_, max_scroll) = repo_tab_scroll(cx, &view);
    assert_eq!(max_scroll, px(0.0), "expected three tabs not to overflow");

    let plus = cx
        .debug_bounds("add_repo_menu")
        .expect("expected the + button");
    let last_tab = cx
        .debug_bounds(repo_tab_selector(*repo_ids.last().expect("repo ids")))
        .expect("expected final repository tab bounds");

    assert!(
        plus.left() >= last_tab.right(),
        "expected the + button after the final repository tab"
    );
    assert!(
        plus.left() <= last_tab.right() + px(12.0),
        "expected the + button to hug the final repository tab, got a {:?} gap",
        plus.left() - last_tab.right()
    );
    assert!(
        cx.debug_bounds("tab_bar_scroll_right").is_none(),
        "expected no scroll arrows while the tabs fit"
    );
}

#[gpui::test]
fn activating_a_repo_scrolls_its_tab_into_view(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let repo_ids = open_repo_tabs(cx, &store_for_test, view.clone(), "tab_reveal", 30);
    let (scrolled, _) = repo_tab_scroll(cx, &view);
    assert_eq!(scrolled, px(0.0), "expected the strip to start unscrolled");

    let last = *repo_ids.last().expect("repo ids");
    store_for_test.dispatch(Msg::SetActiveRepo { repo_id: last });
    // The store reduces on its own thread; syncing before it has would draw
    // the old active tab.
    wait_until("the store to activate the last repository", || {
        store_for_test.snapshot().active_repo == Some(last)
    });
    sync_view_for_tests(cx, &view);
    sync_view_for_tests(cx, &view);

    let (scrolled, max_scroll) = repo_tab_scroll(cx, &view);
    assert_eq!(
        scrolled, max_scroll,
        "expected activating the last repository to scroll its tab into view"
    );
}

fn repo_tab_separator_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("repo_tab_separator_after_{}", repo_id.0).into_boxed_str())
}

fn repo_tab_label_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("repo_tab_label_{}", repo_id.0).into_boxed_str())
}

fn repo_tab_label_text_selector(repo_id: RepoId) -> &'static str {
    Box::leak(format!("repo_tab_label_text_{}", repo_id.0).into_boxed_str())
}

fn wait_for_repo_order(store: &AppStore, expected: &[RepoId]) {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        let state = store.snapshot();
        let got = state.repos.iter().map(|r| r.id).collect::<Vec<_>>();
        if got == expected {
            return;
        }
        if Instant::now() >= deadline {
            panic!("timed out waiting for repo order {expected:?}, got {got:?}");
        }
        std::thread::yield_now();
    }
}

fn resize_repo_tab_strip_to(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<crate::view::GitCometView>,
    target_width: Pixels,
) {
    let current_strip = cx.update(|_window, app| {
        crate::view::test_support::repo_tab_strip_viewport(view.read(app), app)
            .size
            .width
    });
    let window_size = cx.update(|window, _app| window.viewport_size());
    let delta = target_width - current_strip;
    if f32::from(delta).abs() > 1.0 {
        cx.simulate_resize(gpui::size(
            (window_size.width + delta).max(px(640.0)),
            window_size.height,
        ));
        sync_view_for_tests(cx, view);
    }
}

fn scroll_over(cx: &mut gpui::VisualTestContext, position: gpui::Point<Pixels>, delta_y: Pixels) {
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(gpui::point(px(0.0), delta_y)),
        modifiers: Modifiers::default(),
        touch_phase: Default::default(),
    });
}
