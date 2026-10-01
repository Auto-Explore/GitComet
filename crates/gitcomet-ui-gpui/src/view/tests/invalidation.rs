//! Invalidation: cached subviews, notifications, and re-entrancy.

use super::*;

#[gpui::test]
fn sidebar_expand_after_collapse_does_not_reenter_root_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(false, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|_window, app| {
        assert!(!view.read(app).sidebar_collapsed);
    });
}

#[gpui::test]
fn collapsed_rail_offers_git_annex_only_in_annex_repos(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx)));
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );
    assert!(
        !rail_has_annex_icon(cx),
        "a repository without git-annex must not get the rail icon"
    );

    let mut support = gitcomet_core::large_files::LargeFileSupport::default();
    support.annex.uuid = Some("u-here".into());
    support.annex.has_annex_branch = true;
    support.annex.repositories = vec![gitcomet_core::large_files::AnnexRepository {
        uuid: "u-here".into(),
        description: "laptop".into(),
        remote_name: None,
        special_type: None,
        special_name: None,
        trust: gitcomet_core::large_files::AnnexTrust::Semitrusted,
        here: true,
    }];
    state.repos[0].large_file_support = Loadable::Ready(Arc::new(support));
    // As `set_large_file_support` does: the Annex rows are cached by this rev.
    state.repos[0].branch_sidebar_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    test_support::redraw(cx);
    assert!(
        rail_has_annex_icon(cx),
        "large-file support landing must add the rail icon"
    );

    cx.update(|_window, app| {
        view.update(app, |this, cx| {
            this.open_sidebar_collapsed_popover(CollapsedSidebarSection::Annex, cx);
        });
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );
    assert!(
        cx.debug_bounds("annex_repository_0").is_some(),
        "the popover must list the annex repositories"
    );
    let menu = cx
        .debug_bounds("collapsed_popover_section_menu")
        .expect("the popover header must expose the annex section menu");
    cx.simulate_click(menu.center(), gpui::Modifiers::default());
    test_support::redraw(cx);
    cx.update(|_window, app| {
        assert_eq!(
            test_support::popover_kind(view.read(app), app),
            Some(PopoverKind::annex(RepoId(1), AnnexPopoverKind::SectionMenu)),
        );
    });

    // Switching to data without annex closes the popover and drops the icon.
    state.repos[0].large_file_support = Loadable::Ready(Arc::new(
        gitcomet_core::large_files::LargeFileSupport::default(),
    ));
    state.repos[0].branch_sidebar_rev += 1;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );
    assert!(!rail_has_annex_icon(cx));
    cx.update(|_window, app| {
        assert_eq!(view.read(app).sidebar_collapsed_popover, None);
    });
}

fn rail_has_annex_icon(cx: &mut gpui::VisualTestContext) -> bool {
    test_support::redraw(cx);
    cx.debug_bounds("collapsed_sidebar_icon_annex").is_some()
}

#[gpui::test]
fn details_expand_after_collapse_does_not_reenter_root_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_details_collapsed(true, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_details_collapsed(false, cx));
    });
    pump_for(
        cx,
        Duration::from_millis(PANE_COLLAPSE_ANIM_MS.saturating_add(180)),
    );

    cx.update(|_window, app| {
        assert!(!view.read(app).details_collapsed);
    });
}

/// The full-chrome layout keeps every large pane, the bottom status bar and
/// the toast host behind a `stable_cached_*` boundary, so a frame requested by
/// one view (a spinner tick in the title bar, a scroll in history) does not
/// re-render the others. The bar and the toasts were uncached until gpui
/// stopped popping the frame's input handler (zed #50665): replaying their
/// paint after a focused input's had panicked in `reuse_paint`.
#[test]
fn full_chrome_layout_caches_the_pane_subviews() {
    let splash_source = include_str!("../splash.rs");
    let normalized: String = splash_source
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();

    // The repo tabs bar lives inside the title bar since the browser-style
    // chrome merge, so its cache boundary is the title bar mount in the render
    // implementation.
    let root_source = include_str!("../gitcomet_view_render.rs");
    let normalized_root: String = root_source.chars().filter(|c| !c.is_whitespace()).collect();

    assert!(
        normalized_root.contains(
            "stable_cached_fixed_height_view(self.title_bar.clone(),chrome::TITLE_BAR_HEIGHT"
        ),
        "expected the title bar (hosting the repo tabs bar) to stay behind the stable cache boundary"
    );
    assert!(
        normalized.contains(
            "stable_cached_fixed_height_view(self.action_bar.clone(),action_bar_height(cx)"
        ),
        "expected action bar to stay behind the stable cache boundary"
    );
    assert!(
        normalized.contains(
            "stable_cached_fixed_height_view(self.bottom_status_bar.clone(),bottom_status_bar_height(cx),"
        ),
        "expected the bottom status bar to stay behind the stable cache boundary"
    );
    assert!(
        normalized_root.contains("stable_cached_overlay_view(self.toast_host.clone())"),
        "expected the toast host to stay behind the stable cache boundary"
    );
    // The full-chrome main pane mounts through the repository-view router,
    // which keeps History behind the same boundary.
    let router_source: String = include_str!("../repository_views.rs")
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    assert!(
        normalized.contains("letmain_content=self.repository_main_content(cx);")
            && router_source.contains("stable_cached_fill_view(self.main_pane.clone())"),
        "expected the full-chrome main pane to mount through the router, cached"
    );
    assert!(
        normalized.contains("stable_cached_fill_view(self.main_pane.clone())"),
        "expected the focused-layout main pane mount to stay cached"
    );
    assert!(
        normalized.contains("d.child(stable_cached_fill_view(self.sidebar_pane.clone()"),
        "expected the expanded sidebar pane to mount behind the stable cache boundary"
    );
    assert!(
        normalized.contains(".child(stable_cached_fill_view(self.details_pane.clone()"),
        "expected the expanded details pane to mount behind the stable cache boundary"
    );
}

#[gpui::test]
fn cached_sidebar_rerenders_when_the_mode_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let _cache_guard = enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.sidebar_mode = gitcomet_state::model::SidebarMode::Branches;
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    let renders_before =
        cx.update(|_window, app| view.read(app).sidebar_pane.read(app).render_count);

    state.sidebar_mode = gitcomet_state::model::SidebarMode::Files;
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    let renders_after =
        cx.update(|_window, app| view.read(app).sidebar_pane.read(app).render_count);

    assert!(
        renders_after > renders_before,
        "the cached sidebar must be dirtied by a Branches/Files mode change"
    );
}

#[gpui::test]
fn review_regression_repo_tab_move_to_new_window_does_not_reenter_popover_host(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    install_repo_tab_test_state_with_count(&store, &view, cx, RepoId(1), 1);
    open_repo_tab_context_menu(cx, "repo_tab_1");

    // The click is delivered while `PopoverHost` is being updated. The move
    // workflow must not synchronously read that same entity while GPUI still
    // holds its update guard.
    click_debug_selector(cx, "context_menu_move_to_new_window");
    cx.run_until_parked();
}

#[test]
fn untracked_content_revision_ignores_line_stats() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );
    let untracked = status_section_content_rev(&repo, StatusSection::Untracked);
    let unstaged = status_section_content_rev(&repo, StatusSection::Unstaged);
    repo.unstaged_line_stats_rev += 1;
    assert_eq!(
        status_section_content_rev(&repo, StatusSection::Untracked),
        untracked
    );
    assert_ne!(
        status_section_content_rev(&repo, StatusSection::Unstaged),
        unstaged
    );
}

/// Store ticks that change nothing must not lease the workspace manager:
/// every lease notifies its observers, such as an open Settings window.
#[gpui::test]
fn review_regression_unchanged_snapshots_do_not_notify_workspace_observers(
    cx: &mut gpui::TestAppContext,
) {
    use std::cell::Cell;
    use std::rc::Rc;

    let _visual_guard = crate::test_support::lock_visual_test();
    cx.update(|app| crate::workspaces::initialize_for_test(app, Vec::new()));
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let repo_id = RepoId(51);
    let state = AppState {
        repos: vec![RepoState::new_opening(
            repo_id,
            RepoSpec {
                workdir: std::env::temp_dir().join("gitcomet-unchanged-snapshot"),
            },
        )],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };
    let apply = |cx: &mut gpui::VisualTestContext| {
        cx.update(|window, app| {
            view.update(app, |view, cx| {
                test_support::apply_state_snapshot_for_test(view, Arc::new(state.clone()), cx);
            });
            let _ = window.draw(app);
        });
        cx.run_until_parked();
    };
    apply(cx);

    let notifications = Rc::new(Cell::new(0usize));
    let counter = Rc::clone(&notifications);
    let _subscription = cx.update(|_window, app| {
        app.observe_global::<crate::workspaces::WorkspaceManager>(move |_cx| {
            counter.set(counter.get() + 1);
        })
    });
    for _ in 0..4 {
        let paths_before =
            cx.update(|_, app| view.read(app).persisted_workspace_repo_paths.as_ptr());
        apply(cx);
        cx.update(|_, app| {
            assert_eq!(
                paths_before,
                view.read(app).persisted_workspace_repo_paths.as_ptr(),
                "an unchanged store tick must reuse the workspace membership"
            )
        });
    }

    assert_eq!(
        notifications.get(),
        0,
        "an unchanged snapshot notified observers"
    );
}

#[gpui::test]
fn pr530_home_rows_reuse_data_until_an_input_changes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = crate::test_support::lock_visual_test();
    let workspace = named_saved_workspace("Saved", "/tmp/pr530-home");
    let id = workspace.id;
    let (view, cx) = home_view_with(cx, vec![workspace], Vec::new());
    cx.update(|_, app| {
        view.update(app, |view, cx| {
            view.sync_home_rows(cx);
            let rows = view.home_rows.workspaces.as_ptr();
            view.sync_home_rows(cx);
            assert_eq!(
                rows,
                view.home_rows.workspaces.as_ptr(),
                "a repaint rebuilt Home rows"
            );
        });
        crate::workspaces::set_workspace_name(app, id, "Renamed");
        view.update(app, |view, cx| {
            view.sync_home_rows(cx);
            assert_eq!(view.home_rows.workspaces[0].display_name(), "Renamed");
            view.home_search_query = "missing".to_string();
            view.sync_home_rows(cx);
            assert!(view.home_rows.workspaces.is_empty());
            view.home_search_query.clear();
            view.home_recent_repos
                .push(PathBuf::from("/tmp/pr530-recent"));
            view.sync_home_rows(cx);
            assert_eq!(
                view.home_rows.repositories,
                vec![PathBuf::from("/tmp/pr530-recent")]
            );
        });
    });
}
