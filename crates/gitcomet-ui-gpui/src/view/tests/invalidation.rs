//! Invalidation: cached subviews, notifications, and re-entrancy.

use super::*;

#[gpui::test]
fn store_publications_apply_the_latest_snapshot_once_per_frame(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let source = Arc::new(store.clone());
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let (send, receive) = smol::channel::bounded(1);
    let applications = std::rc::Rc::new(std::cell::Cell::new(0));
    let (_poller, _subscription) = cx.update(|window, app| {
        let model = view.read(app).ui_model.clone();
        let count = applications.clone();
        let subscription = app.observe(&model, move |_, _| count.set(count.get() + 1));
        let poller = view.update(app, |_, cx| {
            crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
                Poller::start(source.clone(), receive, model.downgrade(), window, cx)
            })
        });
        (poller, subscription)
    });

    let publish = |revision: u64, cx: &mut gpui::VisualTestContext| {
        let mut state = AppState {
            repo_open_failure_revision: revision,
            ..AppState::test_default()
        };
        for warning in 1..=revision.min(4) {
            state.push_notification(gitcomet_state::model::AppNotification {
                time: std::time::SystemTime::UNIX_EPOCH,
                kind: gitcomet_state::model::AppNotificationKind::Warning,
                message: format!("Warning {warning}"),
            });
        }
        source.replace_snapshot_for_test(Arc::new(state));
        let _ = send.try_send(StoreEvent::StateChanged);
        cx.run_until_parked();
    };
    publish(1, cx);
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    for revision in 2..=4 {
        publish(revision, cx);
    }
    assert_eq!(
        applications.get(),
        1,
        "the first idle publication is prompt"
    );
    cx.update(|window, app| {
        assert_eq!(view.read(app).state.repo_open_failure_revision, 1);
        window.simulate_next_frame(app);
        assert_eq!(view.read(app).state.repo_open_failure_revision, 1);
    });
    cx.run_until_parked();
    assert_eq!(applications.get(), 2);
    cx.update(|_, app| {
        let view = view.read(app);
        assert_eq!(view.state.repo_open_failure_revision, 4);
        let warnings: Vec<_> = view
            .toast_host
            .read(app)
            .toasts_for_tests(app)
            .into_iter()
            .map(|(_, message)| message)
            .collect();
        assert_eq!(
            warnings,
            ["Warning 1", "Warning 2", "Warning 3", "Warning 4"]
        );
    });

    publish(5, cx);
    assert_eq!(applications.get(), 2);
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();
    assert_eq!(applications.get(), 3);
    cx.update(|_, app| assert_eq!(view.read(app).state.repo_open_failure_revision, 5));

    for (latest, count) in [(7, 4), (9, 5)] {
        publish(latest - 1, cx);
        publish(latest, cx);
        cx.executor().advance_clock(Duration::from_millis(251));
        cx.run_until_parked();
        assert_eq!(
            applications.get(),
            count,
            "a hidden window must drain its burst"
        );
        cx.update(|_, app| assert_eq!(view.read(app).state.repo_open_failure_revision, latest));
    }
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.executor().advance_clock(Duration::from_secs(1));
    cx.run_until_parked();
    assert_eq!(
        applications.get(),
        5,
        "an idle window replayed a hidden publication"
    );
    publish(10, cx);
    publish(11, cx);
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(applications.get(), 6);
    cx.update(|_, app| assert_eq!(view.read(app).state.repo_open_failure_revision, 11));
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();
    assert_eq!(applications.get(), 6);
    publish(12, cx);
    source.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(42))));
    let _ = send.try_send(StoreEvent::StateChanged);
    cx.run_until_parked();
    assert_eq!(
        applications.get(),
        7,
        "switching tabs preempts the idle batch"
    );
    cx.update(|_, app| assert_eq!(view.read(app).state.active_repo, Some(RepoId(42))));
    source.replace_snapshot_for_test(Arc::new(view_state_with_active_ready_repo(RepoId(43))));
    let _ = send.try_send(StoreEvent::StateChanged);
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    assert_eq!(applications.get(), 7, "the next switch waits for a frame");
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();
    assert_eq!(applications.get(), 8);
    cx.update(|_, app| assert_eq!(view.read(app).state.active_repo, Some(RepoId(43))));
}

#[gpui::test]
fn active_repository_lookup_handles_reordered_and_lagging_panes(cx: &mut gpui::TestAppContext) {
    let mut first = view_state_with_active_ready_repo(RepoId(1));
    let second = view_state_with_active_ready_repo(RepoId(2));
    first.repos.extend(second.repos);
    first.active_repo = Some(RepoId(2));
    let first = Arc::new(first);
    let mut reordered = (*first).clone();
    reordered.repos.swap(0, 1);
    let mut missing = reordered.clone();
    missing.repos.remove(0);
    let model = cx.new(|cx| AppUiModel::new(first.clone(), Default::default(), cx));
    cx.update(|app| {
        let slot = model.read(app).active_repo_index.clone();
        assert_eq!(slot.position(&first), Some(1));
        model.update(app, |model, cx| {
            model.set_state(Arc::new(reordered.clone()), cx)
        });
        assert_eq!(slot.position(&reordered), Some(0));
        assert_eq!(slot.get(&first).map(|r| r.id), Some(RepoId(2)));
        assert!(slot.get(&missing).is_none());
        missing.active_repo = None;
        assert!(slot.get(&missing).is_none());
    });
}

#[gpui::test]
fn matching_selected_commit_details_end_the_idle_batch(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let source = Arc::new(store.clone());
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let (send, receive) = smol::channel::bounded(1);
    let _poller = cx.update(|window, app| {
        let model = view.read(app).ui_model.downgrade();
        view.update(app, |_, cx| {
            crate::ui_runtime::with_override(crate::ui_runtime::UiRuntime::live(), || {
                Poller::start(source.clone(), receive, model, window, cx)
            })
        })
    });
    let publish = |state: &AppState, cx: &mut gpui::VisualTestContext| {
        let snapshot = Arc::new(state.clone());
        source.replace_snapshot_for_test(snapshot.clone());
        let _ = send.try_send(StoreEvent::StateChanged);
        cx.run_until_parked();
        cx.update(|_, app| Arc::ptr_eq(&snapshot, &view.read(app).state))
    };
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state
        .repos
        .extend(view_state_with_active_ready_repo(RepoId(2)).repos);
    assert!(publish(&state, cx));
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();

    let first = Arc::new(gitcomet_core::domain::CommitDetails {
        id: CommitId("first".into()),
        message: String::new(),
        author_name: String::new(),
        author_email: String::new(),
        authored_at_unix: 0,
        committed_at: String::new(),
        committed_at_unix: 0,
        parent_ids: Vec::new(),
        files: Vec::new(),
    });
    let second = Arc::new(gitcomet_core::domain::CommitDetails {
        id: CommitId("second".into()),
        ..(*first).clone()
    });
    state.repos[0].history_state.selected_commit = Some(first.id.clone());
    state.repos[0].history_state.commit_details = Loadable::Loading;
    assert!(!publish(&state, cx));
    state.repos[1].history_state.selected_commit = Some(first.id.clone());
    state.repos[1].history_state.commit_details = Loadable::Ready(first.clone());
    assert!(!publish(&state, cx), "another repository is ready");
    state.repos[0].history_state.commit_details = Loadable::Ready(second.clone());
    assert!(
        !publish(&state, cx),
        "stale details do not match the selection"
    );
    state.repos[0].history_state.commit_details = Loadable::Ready(first);
    assert!(
        publish(&state, cx),
        "the selected details are ready before the idle deadline"
    );
    assert!(!publish(&state, cx), "the frame slot has been used");
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();

    assert!(!publish(&state, cx), "unchanged ready details still batch");
    state.repos[0].history_state.selected_commit = Some(second.id.clone());
    state.repos[0].history_state.commit_details = Loadable::Loading;
    assert!(!publish(&state, cx));
    cx.executor().advance_clock(Duration::from_millis(1));
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(matches!(
            view.read(app).state.repos[0].history_state.commit_details,
            Loadable::Loading
        ))
    });
    cx.update(|window, app| window.simulate_next_frame(app));
    cx.run_until_parked();
    state.repos[0].history_state.commit_details = Loadable::Ready(second);
    assert!(
        publish(&state, cx),
        "the loaded details replace the visible loading state"
    );
}

#[gpui::test]
fn sidebar_expand_after_collapse_does_not_reenter_root_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx));
    });
    finish_pane_transitions(cx, &view);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_sidebar_collapsed(false, cx));
    });
    finish_pane_transitions(cx, &view);

    cx.update(|_window, app| {
        assert!(!view.read(app).sidebar_collapsed);
    });
}

#[gpui::test]
fn collapsed_rail_offers_git_annex_only_in_annex_repos(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));

    let mut state = view_state_with_active_ready_repo(RepoId(1));
    store.replace_snapshot_for_test(Arc::new(state.clone()));
    sync_view_snapshot(cx, &view);
    cx.update(|_window, app| view.update(app, |this, cx| this.set_sidebar_collapsed(true, cx)));
    finish_pane_transitions(cx, &view);
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
    finish_pane_transitions(cx, &view);
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
    finish_pane_transitions(cx, &view);
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
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_details_collapsed(true, cx));
    });
    finish_pane_transitions(cx, &view);

    cx.update(|window, app| {
        let _ = window.draw(app);
        view.update(app, |this, cx| this.set_details_collapsed(false, cx));
    });
    finish_pane_transitions(cx, &view);

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
            "stable_cached_fixed_height_view(self.title_bar.clone(),crate::kit::window_chrome::TITLE_BAR_HEIGHT"
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
        normalized.contains("letmain_content=self.repository_main_content();")
            && router_source.contains("stable_cached_fill_view(self.main_pane.clone())"),
        "expected the full-chrome main pane to mount through the router, cached"
    );
    assert!(
        normalized.contains("stable_cached_fill_view(self.main_pane.clone())"),
        "expected the focused-layout main pane mount to stay cached"
    );
    let (_, main_render) = include_str!("../panes/main.rs")
        .split_once("impl Render for MainPaneView")
        .expect("the main pane composes its retained child views");
    let normalized_main: String = main_render.chars().filter(|c| !c.is_whitespace()).collect();
    assert!(
        normalized_main
            .contains("MainPaneContent::History=>self.history_view.clone().into_any_element()"),
        "expected the cached compositor to render its retained history view"
    );
    assert!(
        normalized_main
            .contains("MainPaneContent::Diff=>self.diff_renderer.clone().into_any_element()"),
        "expected the cached compositor to render its retained diff renderer"
    );
    assert!(
        normalized.contains("content.child(stable_cached_fill_view(self.sidebar_pane.clone()"),
        "expected the expanded sidebar pane to mount behind the stable cache boundary"
    );
    assert!(
        normalized.contains(".child(stable_cached_fill_view(self.details_pane.clone()"),
        "expected the expanded details pane to mount behind the stable cache boundary"
    );
}

#[gpui::test]
fn cached_sidebar_rerenders_when_the_mode_changes(cx: &mut gpui::TestAppContext) {
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

/// Untracked rows carry counts too, so every section reading the unstaged
/// lane moves with its line stats.
#[test]
fn unstaged_lane_sections_follow_their_line_stats() {
    let mut repo = RepoState::new_opening(
        RepoId(1),
        RepoSpec {
            workdir: PathBuf::new(),
        },
    );
    let sections = [
        StatusSection::CombinedUnstaged,
        StatusSection::Untracked,
        StatusSection::Unstaged,
    ];
    let before = sections.map(|section| status_section_content_rev(&repo, section));
    let staged = status_section_content_rev(&repo, StatusSection::Staged);
    repo.unstaged_line_stats_rev += 1;
    for (section, before) in sections.into_iter().zip(before) {
        assert_ne!(
            status_section_content_rev(&repo, section),
            before,
            "{section:?}"
        );
    }
    assert_eq!(
        status_section_content_rev(&repo, StatusSection::Staged),
        staged
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
    let workspace = named_saved_workspace("Saved", "/tmp/pr530-home");
    let id = workspace.id;
    let (view, cx) = home_view_with(cx, vec![workspace], Vec::new());
    cx.update(|_, app| {
        let home = view.read(app).home.clone();
        home.update(app, |view, cx| {
            view.sync_rows(cx);
            let rows = view.rows.workspaces.as_ptr();
            view.sync_rows(cx);
            assert_eq!(
                rows,
                view.rows.workspaces.as_ptr(),
                "a repaint rebuilt Home rows"
            );
        });
        crate::workspaces::set_workspace_name(app, id, "Renamed");
        home.update(app, |view, cx| {
            view.sync_rows(cx);
            assert_eq!(view.rows.workspaces[0].display_name(), "Renamed");
            view.search_query = "missing".to_string();
            view.sync_rows(cx);
            assert!(view.rows.workspaces.is_empty());
            view.search_query.clear();
            view.recent_repos.push(PathBuf::from("/tmp/pr530-recent"));
            view.sync_rows(cx);
            assert_eq!(
                view.rows.repositories,
                vec![PathBuf::from("/tmp/pr530-recent")]
            );
        });
    });
}

/// A shell with an open repository at 1400x900, cached views on from here.
fn resize_test_shell<'a>(
    cx: &'a mut gpui::TestAppContext,
    handles: &[&'static str],
) -> (
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    Vec<gpui::Bounds<Pixels>>,
    StableCachedViewsTestGuard,
) {
    resize_test_shell_with(cx, view_state_with_active_ready_repo(RepoId(1)), handles)
}

/// The same shell showing `state`.
fn resize_test_shell_with<'a>(
    cx: &'a mut gpui::TestAppContext,
    state: AppState,
    handles: &[&'static str],
) -> (
    gpui::Entity<GitCometView>,
    &'a mut gpui::VisualTestContext,
    Vec<gpui::Bounds<Pixels>>,
    StableCachedViewsTestGuard,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let store_for_view = store.clone();
    let (view, cx) = cx
        .add_window_view(|window, cx| GitCometView::new(store_for_view, events, None, window, cx));
    store.replace_snapshot_for_test(Arc::new(state));
    sync_view_snapshot(cx, &view);
    cx.simulate_resize(gpui::size(px(1400.0), px(900.0)));
    test_support::redraw(cx);
    // Cached frames do not replay debug bounds, so read them first.
    let bounds = handles
        .iter()
        .map(|id| cx.debug_bounds(id).expect("resize strip"))
        .collect();
    let guard = enable_stable_cached_views_for_test();
    cx.update(|window, app| {
        window.refresh();
        let _ = window.draw(app);
    });
    (view, cx, bounds, guard)
}

/// Drags horizontally from `start` by `step` per move and returns `probe`
/// after the first move (which starts the drag) and after seven more.
fn drag_and_probe<T>(
    cx: &mut gpui::VisualTestContext,
    start: gpui::Point<Pixels>,
    step: Pixels,
    probe: impl Fn(&mut gpui::VisualTestContext) -> T,
) -> (T, T) {
    drag_by_and_probe(cx, start, gpui::point(step, px(0.0)), probe)
}

/// The same, moving by `step` on both axes.
fn drag_by_and_probe<T>(
    cx: &mut gpui::VisualTestContext,
    start: gpui::Point<Pixels>,
    step: gpui::Point<Pixels>,
    probe: impl Fn(&mut gpui::VisualTestContext) -> T,
) -> (T, T) {
    let left = gpui::MouseButton::Left;
    let at = |ix: usize| start + gpui::point(step.x * ix as f32, step.y * ix as f32);
    cx.simulate_mouse_down(start, left, gpui::Modifiers::default());
    cx.simulate_mouse_move(at(1), Some(left), gpui::Modifiers::default());
    let before = probe(cx);
    for ix in 2..=8 {
        cx.simulate_mouse_move(at(ix), Some(left), gpui::Modifiers::default());
    }
    let after = probe(cx);
    cx.simulate_mouse_up(at(8), left, gpui::Modifiers::default());
    (before, after)
}

/// Divider drags redraw only the views they move: gpui's default redraws
/// every cached view per pointer move. Starting and ending a drag may each
/// cost one render.
#[gpui::test]
fn pane_resize_drags_leave_unmoved_cached_views_alone(cx: &mut gpui::TestAppContext) {
    let (view, cx, handles, _cache_guard) =
        resize_test_shell(cx, &["pane_resize_sidebar", "pane_resize_details"]);
    let probe = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let root = view.read(app);
            (
                root.bottom_status_bar.read(app).render_count,
                root.sidebar_pane.read(app).render_count,
                root.sidebar_width,
                root.details_width,
            )
        })
    };

    let (before, after) = drag_and_probe(cx, handles[0].center(), px(6.0), probe);
    assert!(
        after.2 > before.2,
        "the sidebar drag must resize the sidebar"
    );
    assert_eq!(
        after.0, before.0,
        "sidebar drag moves must not re-render the status bar"
    );

    let (before, after) = drag_and_probe(cx, handles[1].center(), px(-6.0), probe);
    assert!(
        after.3 > before.3,
        "the details drag must resize the details pane"
    );
    assert_eq!(
        (after.0, after.1),
        (before.0, before.1),
        "details drag moves must not re-render the status bar or the sidebar"
    );
}

/// The same for a divider inside a pane.
#[gpui::test]
fn history_column_drags_leave_other_cached_views_alone(cx: &mut gpui::TestAppContext) {
    let (view, cx, handles, _cache_guard) = resize_test_shell(cx, &["history_col_resize_author"]);
    let probe = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let root = view.read(app);
            (
                root.bottom_status_bar.read(app).render_count,
                root.sidebar_pane.read(app).render_count,
                root.details_pane.read(app).render_count,
                root.main_pane
                    .read(app)
                    .history_view
                    .read(app)
                    .history_col_author,
            )
        })
    };

    let (before, after) = drag_and_probe(cx, handles[0].center(), px(-6.0), probe);
    assert!(after.3 > before.3, "the drag must widen the author column");
    assert_eq!(
        (after.0, after.1, after.2),
        (before.0, before.1, before.2),
        "history column drag moves must not re-render the status bar, sidebar or details"
    );
}

/// A status-section drag is the details pane's own: its moves redraw that
/// pane and nothing else.
#[gpui::test]
fn status_section_drags_leave_other_cached_views_alone(cx: &mut gpui::TestAppContext) {
    let file = |path: &str| gitcomet_core::domain::FileStatus {
        path: PathBuf::from(path),
        kind: gitcomet_core::domain::FileStatusKind::Modified,
        conflict: None,
    };
    let mut state = view_state_with_active_ready_repo(RepoId(1));
    state.repos[0].status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            staged: Arc::new(vec![file("staged.txt")]),
            unstaged: Arc::new(vec![file("unstaged.txt")]),
        }
        .into(),
    );
    let (view, cx, handles, _cache_guard) =
        resize_test_shell_with(cx, state, &["status_resize_change_tracking_staged"]);
    let probe = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let root = view.read(app);
            let details = root.details_pane.read(app);
            (
                (
                    root.bottom_status_bar.read(app).render_count,
                    root.sidebar_pane.read(app).render_count,
                    root.main_pane.read(app).render_count,
                ),
                details.render_count,
                details.change_tracking_height,
            )
        })
    };

    // The press and the release still refresh the window: gpui tracks the
    // active state of every stateful element that way.
    let step = gpui::point(px(0.0), px(6.0));
    let (before, after) = drag_by_and_probe(cx, handles[0].center(), step, probe);
    assert_ne!(after.2, before.2, "the drag must resize the status section");
    assert!(
        after.1 >= before.1 + 7,
        "every move must redraw the details pane"
    );
    assert_eq!(
        after.0, before.0,
        "status-section drag moves must not re-render the status bar, sidebar or main pane"
    );
}

/// `cargo test -p gitcomet-ui-gpui --lib timing_divider_drag -- --ignored
/// --nocapture`: mean time per pointer move, frame included, with cached views.
#[gpui::test]
#[ignore = "timing probe"]
fn timing_divider_drag_moves(cx: &mut gpui::TestAppContext) {
    // The history strip first: the pane drags move it.
    let ids = [
        "history_col_resize_author",
        "pane_resize_sidebar",
        "pane_resize_details",
    ];
    let (view, cx, handles, _cache_guard) = resize_test_shell(cx, &ids);
    let width = |cx: &mut gpui::VisualTestContext, id: &str| {
        cx.update(|_window, app| {
            let root = view.read(app);
            match id {
                "pane_resize_sidebar" => root.sidebar_width,
                "pane_resize_details" => root.details_width,
                _ => {
                    root.main_pane
                        .read(app)
                        .history_view
                        .read(app)
                        .history_col_author
                }
            }
        })
    };
    let left = gpui::MouseButton::Left;
    for (id, handle) in ids.iter().zip(handles) {
        let mut widths = Vec::new();
        let start = handle.center();
        let step = if *id == "pane_resize_sidebar" {
            2.0
        } else {
            -2.0
        };
        cx.simulate_mouse_down(start, left, gpui::Modifiers::default());
        cx.simulate_mouse_move(
            gpui::point(start.x + px(step), start.y),
            Some(left),
            gpui::Modifiers::default(),
        );
        let moves = 200;
        let started = std::time::Instant::now();
        for ix in 0..moves {
            // To and fro, so every move changes the width.
            let offset = (ix % 40) as f32 - 20.0;
            let offset = 20.0 - offset.abs();
            cx.simulate_mouse_move(
                gpui::point(start.x + px(step * (offset + 2.0)), start.y),
                Some(left),
                gpui::Modifiers::default(),
            );
            widths.push(width(cx, id));
        }
        let per_move = started.elapsed() / moves;
        widths.dedup();
        cx.simulate_mouse_up(start, left, gpui::Modifiers::default());
        println!(
            "timing divider_drag {id} per_move {per_move:?} width_changes {}",
            widths.len()
        );
    }
}

/// The panes hold no theme and nothing tells them of one. Setting the
/// window's theme has to redraw them, though they sit behind a cache.
#[gpui::test]
fn a_theme_change_redraws_the_cached_panes_with_the_new_theme(cx: &mut gpui::TestAppContext) {
    let (view, cx, _handles, _cache_guard) = resize_test_shell(cx, &["pane_resize_sidebar"]);
    let renders = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let root = view.read(app);
            [
                root.bottom_status_bar.read(app).render_count,
                root.sidebar_pane.read(app).render_count,
                root.main_pane.read(app).render_count,
                root.details_pane.read(app).render_count,
            ]
        })
    };

    // Away from the theme the window starts in first: the same theme is no change.
    let (light, dark) = (AppTheme::gitcomet_light(), AppTheme::gitcomet_dark());
    let starts_dark = cx.update(|_window, app| AppTheme::current(app).is_dark);
    for theme in if starts_dark {
        [light, dark]
    } else {
        [dark, light]
    } {
        cx.run_until_parked();
        let before = renders(cx);
        cx.update(|_window, app| view.update(app, |view, cx| view.set_theme(theme, cx)));
        cx.run_until_parked();
        for (before, after) in before.into_iter().zip(renders(cx)) {
            assert!(after > before, "a pane kept the frame it drew before");
        }
        let chrome: gpui::Background = theme.colors.surface.chrome.into();
        assert!(
            crate::kit::test_support::painted_control_quads(cx, "action_bar")
                .iter()
                .any(|(fill, _)| *fill == chrome),
            "the action bar did not paint with the window's theme"
        );
    }
}
