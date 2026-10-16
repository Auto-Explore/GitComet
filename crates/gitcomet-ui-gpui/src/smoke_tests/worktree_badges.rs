use super::*;

#[gpui::test]
fn branch_worktree_badge_aligns_to_edge_and_branch_menu_opens_on_right_click(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_branch_worktree_menu_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    let row_selector = Box::leak(format!("branch_row_{}_{}", repo_id.0, badge_ix).into_boxed_str());
    let badge_bounds = cx
        .debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
        .expect("expected branch worktree badge bounds");
    let row_bounds = cx
        .debug_bounds(row_selector)
        .expect("expected branch row bounds");
    let menu_selector =
        Box::leak(format!("branch_menu_indicator_{}_{}", repo_id.0, badge_ix).into_boxed_str());

    assert!(
        cx.debug_bounds(menu_selector).is_none(),
        "expected branch hamburger menu indicator to be removed"
    );

    let row_center = row_bounds.center();
    cx.simulate_mouse_move(row_center, None, Modifiers::default());
    cx.run_until_parked();
    sync_view_for_tests(cx, &view);

    // Nothing is revealed on hover at the trailing edge any more: the `⋮` button
    // is gone. Content keeps its inset plus trailing padding while the row's
    // background reaches the panel edge.
    assert!(
        cx.debug_bounds(debug_selector("branch_dots", badge_ix))
            .is_none(),
        "expected the branch row's `⋮` slot to be gone"
    );
    assert!(
        (row_bounds.right()
            - badge_bounds.right()
            - px(crate::view::components::ROW_HIGHLIGHT_INSET_PX + 4.0))
        .abs()
            <= px(1.0),
        "expected branch worktree badge to sit one trailing pad off the row's right edge, \
         row right {:?} badge right {:?}",
        row_bounds.right(),
        badge_bounds.right()
    );

    // Right-click over the label (near the leading edge) rather than the center:
    // the trailing area holds the worktree badge, which opens its own menu.
    let row_label_point = gpui::point(row_bounds.left() + px(48.0), row_center.y);
    cx.simulate_mouse_down(row_label_point, MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(row_label_point, MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    sync_view_for_tests(cx, &view);

    cx.update(|_window, app| {
        assert!(
            crate::view::test_support::popover_is_open(view.read(app), app),
            "expected branch row right-click to open a popover even with a worktree badge"
        );
    });
}

#[gpui::test]
fn worktree_branch_badge_shows_full_tooltip_when_truncated(cx: &mut gpui::TestAppContext) {
    // Measures Compact layout; a fresh session now defaults to Comfortable.
    cx.update(crate::appearance::pin_compact_for_test);
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(760.0), px(440.0)));

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_worktree_branch_tooltip_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

    let branch = "feature/super-long-worktree-branch-name-that-needs-truncation-to-fit-the-sidebar"
        .to_string();
    let linked_repo = base
        .join("repo-feature")
        .join("nested")
        .join("workspace-with-a-long-path");
    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            path: linked_repo,
            head: None,
            branch: Some(branch.clone()),
            detached: false,
        }],
    );

    let section_ix = wait_for_debug_index(cx, &view, "worktrees_section", 64);
    // The fixture backend reports unsupported loads as errors, which stay
    // on screen over the sidebar this clicks.
    cx.update(|_window, app| crate::view::test_support::dismiss_error_toasts(&view, app));
    sync_view_for_tests(cx, &view);
    click_debug_selector(cx, debug_selector("worktrees_section", section_ix), 1);

    let label_ix = wait_for_debug_index(cx, &view, "worktree_branch_badge_label", 128);
    let label_bounds = cx
        .debug_bounds(debug_selector("worktree_branch_badge_label", label_ix))
        .expect("expected worktree branch badge label to render");
    cx.simulate_mouse_move(label_bounds.center(), None, Modifiers::default());
    view::test_support::wait_for_native_tooltip(cx);

    assert_eq!(
        view::test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        Some(branch)
    );
}

#[gpui::test]
fn worktree_branch_and_path_stay_within_one_sidebar_row(cx: &mut gpui::TestAppContext) {
    // Measures Compact layout; a fresh session now defaults to Comfortable.
    cx.update(crate::appearance::pin_compact_for_test);
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    cx.simulate_resize(gpui::size(px(1200.0), px(440.0)));

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_worktree_row_layout_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            crate::view::test_support::set_sidebar_width_for_test(this, px(500.0), cx);
        });
    });
    sync_view_for_tests(cx, &view);

    let worktree_root = base.join("ae");
    let branch = "feature/badge-expands".to_string();
    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            path: worktree_root.join("agent4"),
            head: None,
            branch: Some(branch.clone()),
            detached: false,
        }],
    );

    let section_ix = wait_for_debug_index(cx, &view, "worktrees_section", 64);
    // The fixture backend reports unsupported loads as errors, which stay
    // on screen over the sidebar this clicks.
    cx.update(|_window, app| crate::view::test_support::dismiss_error_toasts(&view, app));
    sync_view_for_tests(cx, &view);
    click_debug_selector(cx, debug_selector("worktrees_section", section_ix), 1);

    let row_ix = wait_for_debug_index(cx, &view, "worktree_branch_badge", 128);
    let row_selector = Box::leak(format!("worktree_row_{}_{}", repo_id.0, row_ix).into_boxed_str());
    let branch_selector = debug_selector("worktree_branch_badge", row_ix);
    let branch_label_selector = debug_selector("worktree_branch_badge_label", row_ix);
    let path_selector = debug_selector("worktree_path_label", row_ix);

    sync_view_for_tests(cx, &view);
    let row_bounds = cx
        .debug_bounds(row_selector)
        .expect("expected worktree row bounds");
    let branch_bounds = cx
        .debug_bounds(branch_selector)
        .expect("expected worktree branch badge bounds");
    let branch_label_bounds = cx
        .debug_bounds(branch_label_selector)
        .expect("expected worktree branch badge label bounds");
    let path_bounds = cx
        .debug_bounds(path_selector)
        .expect("expected worktree path label bounds");

    assert_eq!(
        path_bounds.center().y,
        row_bounds.center().y,
        "expected path label {path_bounds:?} to be centered in row {row_bounds:?}",
    );
    assert_eq!(
        branch_bounds.center().y,
        row_bounds.center().y,
        "expected branch badge {branch_bounds:?} to be centered in row {row_bounds:?}",
    );
    assert!(
        path_bounds.right() <= branch_bounds.left(),
        "expected path label {path_bounds:?} to stay to the left of branch badge {branch_bounds:?}",
    );
    assert!(
        branch_bounds.right() - branch_bounds.left() > px(104.0),
        "expected branch badge {branch_bounds:?} to grow beyond the old fixed cap",
    );
    let available_label_width = row_bounds.right() - path_bounds.left();
    assert!(
        branch_bounds.right() - branch_bounds.left() <= (available_label_width / 2.0) + px(1.0),
        "expected branch badge {branch_bounds:?} to stay within half of the available label width {available_label_width:?}",
    );

    cx.simulate_mouse_move(branch_label_bounds.center(), None, Modifiers::default());
    view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        view::test_support::tooltip_text(cx, &view).map(|text| text.to_string()),
        None,
        "expected branch badge label to fit without truncation when the row has room"
    );
}

#[gpui::test]
fn worktree_badge_appears_when_worktree_added_for_branch(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_appears_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to appear when worktree is added for a branch"
    );
}

#[gpui::test]
fn worktree_badge_disappears_when_worktree_removed(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_disappears_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to appear"
    );

    publish_worktrees_for_test(&store_for_test, repo_id, vec![]);

    let deadline = Instant::now() + Duration::from_secs(1);
    let disappeared = loop {
        sync_view_for_tests(cx, &view);

        if find_debug_index(cx, "branch_worktree_badge", 64).is_none() {
            break true;
        }

        if Instant::now() >= deadline {
            break false;
        }

        cx.run_until_parked();
        std::thread::yield_now();
    };
    assert!(
        disappeared,
        "expected workspace badge to disappear after all worktrees are removed"
    );
}

#[gpui::test]
fn worktree_badge_disappears_when_worktree_detaches(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_detach_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to appear"
    );

    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            path: base.join("repo-feature"),
            head: None,
            branch: None,
            detached: true,
        }],
    );

    let deadline = Instant::now() + Duration::from_secs(1);
    let disappeared = loop {
        sync_view_for_tests(cx, &view);

        if find_debug_index(cx, "branch_worktree_badge", 64).is_none() {
            break true;
        }

        if Instant::now() >= deadline {
            break false;
        }

        cx.run_until_parked();
        std::thread::yield_now();
    };
    assert!(
        disappeared,
        "expected workspace badge to disappear when worktree becomes detached"
    );
}

#[gpui::test]
fn worktree_badge_moves_when_worktree_branch_renames(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_move_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

    store_for_test.dispatch(Msg::Internal(
        gitcomet_state::msg::InternalMsg::BranchesLoaded {
            repo_id,
            result: Ok(vec![
                Branch {
                    name: "feature/old".to_string(),
                    target: CommitId("deadbeef".into()),
                    upstream: None,
                    divergence: None,
                },
                Branch {
                    name: "feature/new".to_string(),
                    target: CommitId("deadbeef".into()),
                    upstream: None,
                    divergence: None,
                },
            ]),
        },
    ));
    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/old".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to appear on old branch"
    );

    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/new".to_string()),
            detached: false,
        }],
    );

    let deadline = Instant::now() + Duration::from_secs(1);
    let index = loop {
        sync_view_for_tests(cx, &view);

        if let Some(ix) = find_debug_index(cx, "branch_worktree_badge", 64) {
            break Some(ix);
        }

        if Instant::now() >= deadline {
            break None;
        }

        cx.run_until_parked();
        std::thread::yield_now();
    };
    assert!(
        index.is_some(),
        "expected workspace badge to still be present on new branch"
    );
}

#[gpui::test]
fn worktree_branch_badge_hidden_for_detached_worktree_item(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_detached_badge_hidden_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

    publish_worktrees_for_test(
        &store_for_test,
        repo_id,
        vec![Worktree {
            path: base.join("repo-detached"),
            head: None,
            branch: None,
            detached: true,
        }],
    );

    let section_ix = wait_for_debug_index(cx, &view, "worktrees_section", 64);
    click_debug_selector(cx, debug_selector("worktrees_section", section_ix), 1);

    let label_ix = wait_for_debug_index(cx, &view, "worktree_path_label", 128);
    assert!(
        cx.debug_bounds(debug_selector("worktree_path_label", label_ix))
            .is_some(),
        "expected worktree path label to render for detached worktree"
    );

    assert!(
        find_debug_index(cx, "worktree_branch_badge", 128).is_some(),
        "expected branch badge on detached worktree item showing (detached)"
    );
}

#[gpui::test]
fn worktree_badge_survives_reload_repo_and_manual_worktree_resupply(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_reload_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to appear"
    );

    let branches_rev = |store: &AppStore| {
        store
            .snapshot()
            .repos
            .iter()
            .find(|repo| repo.id == repo_id)
            .map(|repo| repo.branches_rev)
    };
    let before_reload = branches_rev(&store_for_test);
    store_for_test.dispatch(Msg::ReloadRepo { repo_id });
    // The pre-reload snapshot already looks settled.
    wait_until("reload to reset branches", || {
        branches_rev(&store_for_test) != before_reload
    });
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to survive repo reload and manual worktree resupply"
    );
}

#[gpui::test]
fn worktree_badge_reappears_after_sidebar_data_request_cycle(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(SubmoduleTestBackend::default()));
    let store_for_test = store.clone();
    let (view, cx) = cx.add_window_view(|window, cx| {
        crate::view::GitCometView::new(store, events, None, window, cx)
    });

    let base = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_badge_sidebar_cycle_{}",
        std::process::id()
    ));
    let repo_ids =
        restore_session_and_draw(cx, &store_for_test, view.clone(), vec![base.join("repo1")]);
    let repo_id = repo_ids[0];
    wait_for_repo_open(&store_for_test, repo_id);
    wait_for_sidebar_loads_to_settle(cx, &store_for_test, &view, repo_id);

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
            path: base.join("repo-feature"),
            head: None,
            branch: Some("feature/workspace".to_string()),
            detached: false,
        }],
    );

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to appear"
    );

    store_for_test.dispatch(Msg::EnsureSidebarData {
        repo_id,
        request: SidebarDataRequest {
            worktrees: false,
            submodules: false,
            stashes: false,
        },
    });

    let deadline = Instant::now() + Duration::from_secs(1);
    let disappeared = loop {
        sync_view_for_tests(cx, &view);
        if find_debug_index(cx, "branch_worktree_badge", 64).is_none() {
            break true;
        }
        if Instant::now() >= deadline {
            break false;
        }
        cx.run_until_parked();
        std::thread::yield_now();
    };
    assert!(
        !disappeared,
        "workspace badge should NOT disappear when EnsureSidebarData sets worktrees:false while data is ready"
    );

    store_for_test.dispatch(Msg::EnsureSidebarData {
        repo_id,
        request: SidebarDataRequest {
            worktrees: true,
            submodules: true,
            stashes: true,
        },
    });

    let badge_ix = wait_for_debug_index(cx, &view, "branch_worktree_badge", 64);
    assert!(
        cx.debug_bounds(debug_selector("branch_worktree_badge", badge_ix))
            .is_some(),
        "expected workspace badge to still be present after re-enabling sidebar data request"
    );
}
