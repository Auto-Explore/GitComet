use super::*;

#[gpui::test]
fn bottom_status_bar_zoom_button_zooms_this_window(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(709);
    let commit_id = CommitId("9988776655443322".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_bottom_status_zoom_button",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    // At the default scale the button is just its icon.
    assert!(cx.debug_bounds("bottom_status_bar_zoom_icon").is_some());
    let default_button_width = debug_width(cx, "bottom_status_bar_zoom");
    assert!(
        default_button_width < 40.0,
        "expected an icon-only zoom button at the default scale (width={default_button_width})"
    );

    let open_zoom_menu = |cx: &mut gpui::VisualTestContext| {
        let bounds = cx
            .debug_bounds("bottom_status_bar_zoom")
            .expect("zoom button bounds");
        cx.simulate_click(bounds.center(), Modifiers::default());
        draw_and_drain_test_window(cx);
        assert!(popover_is_open(cx, &view), "expected the zoom menu to open");
    };
    let click = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} to be rendered"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        draw_and_drain_test_window(cx);
    };

    open_zoom_menu(cx);
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_125");
    click(cx, "context_menu_125");

    let (percent, own_zoom, default) = cx.update(|window, app| {
        (
            view.read(app).ui_scale_percent,
            crate::ui_scale::window_override(app, window.window_handle().window_id()),
            crate::ui_scale::default_percent(app),
        )
    });
    assert_eq!(percent, 125, "the window takes the chosen zoom");
    assert_eq!(own_zoom, Some(125), "as its own zoom");
    assert_eq!(default, 100, "the default UI scale is untouched");
    assert!(
        !popover_is_open(cx, &view),
        "the menu closes after a choice"
    );
    let zoomed_button_width = debug_width(cx, "bottom_status_bar_zoom");
    assert!(
        zoomed_button_width > default_button_width + 10.0,
        "a zoomed window shows its percent (default={default_button_width}, zoomed={zoomed_button_width})"
    );

    open_zoom_menu(cx);
    click(cx, "context_menu_use_default_100");
    let (percent, own_zoom) = cx.update(|window, app| {
        (
            view.read(app).ui_scale_percent,
            crate::ui_scale::window_override(app, window.window_handle().window_id()),
        )
    });
    assert_eq!((percent, own_zoom), (100, None), "back on the default");
}

/// The cached footer shows whether its window has its own zoom, which can
/// change while the percent does not.
#[gpui::test]
fn clearing_a_zoom_equal_to_the_default_rerenders_the_cached_footer(cx: &mut gpui::TestAppContext) {
    let _cache_guard = crate::view::enable_stable_cached_views_for_test();
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    let repo_id = RepoId(711);
    let commit_id = CommitId("1199228833774466".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_cached_footer_zoom",
        std::process::id()
    ));
    apply_state(
        cx,
        &view,
        app_state_with_active_repo(shortcut_fixture_repo(repo_id, &workdir, &commit_id)),
    );
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let window_id = cx.update(|window, _| window.window_handle().window_id());
    let renders = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| view.read(app).bottom_status_bar.read(app).render_count)
    };
    let set_zoom = |cx: &mut gpui::VisualTestContext, percent: Option<u32>| {
        gpui::TestAppContext::update(cx, |app| {
            crate::app::set_window_ui_scale_percent(app, window_id, percent);
        });
        cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };

    set_zoom(cx, Some(125));
    // The default catches up with the window's zoom; the window keeps its own.
    gpui::TestAppContext::update(cx, |app| {
        crate::ui_scale::set_default(app, 125);
        crate::app::apply_ui_scale_to_windows(app);
    });
    let before = renders(cx);
    set_zoom(cx, None);
    assert!(
        renders(cx) > before,
        "the footer must drop its percent label ({before} renders before)"
    );
    gpui::TestAppContext::update(cx, |app| {
        crate::ui_scale::set_default(app, crate::ui_scale::DEFAULT_UI_SCALE_PERCENT);
    });
}

#[gpui::test]
fn ui_scale_commands_zoom_the_window_that_runs_them(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, Arc::new(TestBackend));
        let _ = window.draw(app);
    });
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.execute_command("increase-ui-scale", Some(window), cx);
        });
    });
    cx.run_until_parked();

    let own_zoom = cx.update(|window, app| {
        crate::ui_scale::window_override(app, window.window_handle().window_id())
    });
    assert_eq!(own_zoom, Some(110));
    cx.update(|window, app| {
        crate::ui_scale::set_window_percent(app, window.window_handle().window_id(), None);
    });
}

/// The bottom bar only exists in full chrome, so every branding test needs an
/// active repository before the bar is drawn at all.
fn open_repo_for_bottom_status_bar_test(
    cx: &mut gpui::VisualTestContext,
    view: &gpui::Entity<super::super::GitCometView>,
    repo_id: RepoId,
    workdir_suffix: &str,
) {
    let commit_id = CommitId("1122334455667788".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_{workdir_suffix}",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);
}

#[gpui::test]
fn bottom_status_bar_free_badge_opens_editions_page_and_updates_tooltip_on_hover(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    open_repo_for_bottom_status_bar_test(cx, &view, RepoId(710), "bottom_status_free_badge");

    let badge_bounds = cx
        .debug_bounds("bottom_status_bar_free_badge")
        .expect("expected bottom status bar free badge bounds");
    let badge_center = badge_bounds.center();

    cx.simulate_mouse_move(badge_center, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("See GitComet editions".into())
    );

    cx.simulate_click(badge_center, Modifiers::default());
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.opened_url(),
        Some(crate::view::editions_url().unwrap().to_string())
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the free badge click to leave popovers closed"
    );
}

#[gpui::test]
fn bottom_status_bar_pro_link_renders_and_opens_editions_page(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    open_repo_for_bottom_status_bar_test(cx, &view, RepoId(710), "bottom_status_pro_link");

    let link_bounds = cx
        .debug_bounds("bottom_status_bar_pro_link")
        .expect("expected the Pro link to render without panicking");
    cx.simulate_mouse_move(link_bounds.center(), None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("See GitComet Pro".into())
    );

    cx.simulate_click(link_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.opened_url(),
        Some(crate::view::editions_url().unwrap().to_string())
    );
}

#[gpui::test]
fn bottom_status_bar_free_badge_scales_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    open_repo_for_bottom_status_bar_test(cx, &view, RepoId(711), "bottom_status_free_badge_zoom");

    let default_width = debug_width(cx, "bottom_status_bar_free_badge");

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);

    // Unlike the title bar it used to live in, the bottom bar is uncached and
    // sized from design pixels, so the badge tracks UI zoom with its neighbours.
    let zoomed_width = debug_width(cx, "bottom_status_bar_free_badge");
    assert!(
        zoomed_width > default_width * 1.5,
        "expected the FREE badge to grow with UI zoom (default={default_width}, zoomed={zoomed_width})"
    );
}

#[gpui::test]
fn bottom_status_bar_branding_opens_discord_and_release_notes(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    open_repo_for_bottom_status_bar_test(cx, &view, RepoId(712), "bottom_status_branding");

    let discord_bounds = cx
        .debug_bounds("bottom_status_bar_discord")
        .expect("expected bottom status bar discord badge bounds");
    cx.simulate_click(discord_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.opened_url(),
        Some(crate::view::community_url().unwrap().to_string())
    );

    let version_bounds = cx
        .debug_bounds("bottom_status_bar_version")
        .expect("expected bottom status bar version bounds");
    cx.simulate_click(version_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(
        cx.opened_url(),
        Some(crate::view::releases_url().unwrap().to_string())
    );

    let brand_bounds = cx
        .debug_bounds("bottom_status_bar_brand")
        .expect("expected the GitComet wordmark to be visible in the bottom bar");
    assert!(
        version_bounds.origin.x > brand_bounds.origin.x,
        "expected the version number to sit at the bar's trailing end, right of the wordmark"
    );
}

#[gpui::test]
fn bottom_status_bar_brand_opens_the_website_and_shows_a_tooltip(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    open_repo_for_bottom_status_bar_test(cx, &view, RepoId(713), "bottom_status_brand_link");

    let brand_bounds = cx
        .debug_bounds("bottom_status_bar_brand_link")
        .expect("expected the GitComet mark and wordmark to share one link");
    let brand_center = brand_bounds.center();

    cx.simulate_mouse_move(brand_center, None, Modifiers::default());
    crate::view::test_support::wait_for_native_tooltip(cx);
    assert_eq!(
        crate::view::test_support::tooltip_text(cx, &view),
        Some("Open gitcomet.dev".into())
    );

    cx.simulate_click(brand_center, Modifiers::default());
    draw_and_drain_test_window(cx);

    assert_eq!(
        cx.opened_url(),
        Some(crate::view::website_url().unwrap().to_string())
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the wordmark click to leave popovers closed"
    );
}

#[gpui::test]
fn shared_context_menu_rows_fill_the_popover_width(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(710);
    let commit_id = CommitId("1234432112344321".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_shared_context_menu_width",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    open_change_tracking_settings_popover(cx, &view);
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings popover to be open"
    );
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_combine_with_unstaged");
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_show_separate_untracked_block");
}

#[gpui::test]
fn context_menus_grow_wider_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(711);
    let commit_id = CommitId("2233445566778899".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_context_menu_zoom_width",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    open_change_tracking_settings_popover(cx, &view);
    draw_and_drain_test_window(cx);

    let default_width = debug_width(cx, "app_popover");
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_combine_with_unstaged");

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected the change-tracking settings context menu to remain open after zooming"
    );

    let zoomed_width = debug_width(cx, "app_popover");
    assert!(
        zoomed_width > default_width * 1.6,
        "expected the context menu to grow substantially with zoom (default={default_width}, zoomed={zoomed_width})"
    );
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_combine_with_unstaged");
}

#[gpui::test]
fn prompt_popovers_grow_wider_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(712);
    let commit_id = CommitId("3344556677889900".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_prompt_popover_zoom_width",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    open_popover_for_test(
        cx,
        &view,
        PopoverKind::CreateBranchFromRefPrompt {
            repo_id: RepoId(1),
            target: "HEAD".to_string(),
            source_selectable: false,
            name_prefix: String::new(),
        },
    );
    draw_and_drain_test_window(cx);

    let default_width = debug_width(cx, "app_popover");

    set_ui_scale_percent_for_test(cx, &view, 200);
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected the create-branch popover to remain open after zooming"
    );

    let zoomed_width = debug_width(cx, "app_popover");
    assert!(
        zoomed_width > default_width * 1.6,
        "expected the prompt popover to grow substantially with zoom (default={default_width}, zoomed={zoomed_width})"
    );
}

#[gpui::test]
fn history_horizontal_wheel_does_not_scroll_vertically(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(709);
    let commit_id = CommitId("8877665544332211".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_history_horizontal_wheel",
        std::process::id()
    ));
    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    let commits = (0..160)
        .map(|ix| gitcomet_core::domain::Commit {
            id: CommitId(format!("{ix:040x}").into()),
            parent_ids: gitcomet_core::domain::CommitParentIds::new(),
            summary: format!("Commit {ix:03}").into(),
            author: "Alice".into(),
            time: std::time::SystemTime::UNIX_EPOCH
                + Duration::from_secs(ix.try_into().unwrap_or(0)),
        })
        .collect();
    repo.log = Loadable::Ready(
        gitcomet_core::domain::LogPage {
            commits,
            next_cursor: None,
        }
        .into(),
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    let (history_bounds, max_offset_y) = cx.update(|_window, app| {
        let pane = view.read(app).main_pane.read(app).history_view.read(app);
        let handle = pane.history_scroll.0.borrow().base_handle.clone();
        (handle.bounds(), handle.max_offset().y)
    });
    let position = history_bounds.center();
    assert!(
        max_offset_y > px(0.0),
        "expected history list to be vertically scrollable"
    );

    let offset_before = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
        ..Default::default()
    });
    draw_and_drain_test_window(cx);
    let offset_after_horizontal = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    assert_eq!(
        offset_after_horizontal.y, offset_before.y,
        "expected horizontal-only wheel scroll not to move history vertically"
    );

    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    draw_and_drain_test_window(cx);
    let offset_after_vertical = cx.update(|_window, app| {
        view.read(app)
            .main_pane
            .read(app)
            .history_view
            .read(app)
            .history_scroll
            .0
            .borrow()
            .base_handle
            .offset()
    });
    assert!(
        offset_after_vertical.y < offset_before.y - px(0.5),
        "expected vertical wheel scroll to continue moving history vertically"
    );
}

#[gpui::test]
fn ui_scale_ctrl_scroll_wheel_changes_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(708);
    let commit_id = CommitId("8877665544332211".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ui_scale_ctrl_scroll",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    draw_and_drain_test_window(cx);

    let position = point(px(320.0), px(240.0));
    cx.simulate_mouse_move(position, None, Modifiers::default());
    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.0), px(120.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_and_drain_test_window(cx);

    let zoomed_in = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoomed_in, 110,
        "expected Ctrl/Cmd + wheel up to step the UI zoom to the next preset"
    );

    cx.simulate_event(ScrollWheelEvent {
        position,
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..Default::default()
    });
    draw_and_drain_test_window(cx);

    let zoomed_back_out = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoomed_back_out, 100,
        "expected Ctrl/Cmd + wheel down to step the UI zoom back to the previous preset"
    );
}

#[gpui::test]
fn ctrl_s_stages_current_file_and_advances_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70600);
    let commit_id = CommitId("abcdef00112233bb".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_s_stage",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");
    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        &[first.clone(), second.clone()],
        &first,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-s");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected Ctrl+S to stage the active file and advance the diff target"
    );
}

#[gpui::test]
fn ctrl_s_stages_last_file_and_clears_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70601);
    let commit_id = CommitId("abcdef00112233cc".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_s_last_file",
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
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-s");
    draw_and_drain_test_window(cx);
    wait_until(
        cx,
        "store diff target to clear after staging last file",
        |cx| {
            cx.update(|_window, app| {
                let snapshot = view.read(app).store.snapshot();
                let Some(repo_id) = snapshot.active_repo else {
                    return false;
                };
                let Some(repo) = snapshot.repos.iter().find(|r| r.id == repo_id) else {
                    return false;
                };
                repo.diff_state.diff_target.is_none()
            })
        },
    );
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        None,
        "expected Ctrl+S on the last unstaged file to stage it and clear the diff target"
    );
}

#[gpui::test]
fn ctrl_shift_c_copies_current_file_path(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = crate::test_support::lock_clipboard_test();

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70602);
    let commit_id = CommitId("abcdef00112233dd".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_shift_c",
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
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-shift-c");
    draw_and_drain_test_window(cx);

    let clipboard_text = cx.read_from_clipboard().and_then(|item| item.text());
    assert!(
        clipboard_text
            .as_ref()
            .is_some_and(|text| text.contains("src/lib.rs")),
        "expected Ctrl+Shift+C to copy the current file path to clipboard, got: {clipboard_text:?}"
    );
}

#[gpui::test]
fn ctrl_d_opens_discard_confirm_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70603);
    let commit_id = CommitId("abcdef00112233ee".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_d_discard",
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
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-d");
    draw_and_drain_test_window(cx);

    let is_discard_confirm = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        matches!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::DiscardChangesConfirm { .. })
        )
    });
    assert!(
        is_discard_confirm,
        "expected Ctrl+D to open the DiscardChangesConfirm popover"
    );
}

#[gpui::test]
fn ctrl_h_opens_file_history_popover(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70604);
    let commit_id = CommitId("abcdef00112233ff".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_h_history",
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
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-h");
    draw_and_drain_test_window(cx);

    let is_file_history = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        matches!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::FileHistory { .. })
        )
    });
    assert!(
        is_file_history,
        "expected Ctrl+H to open the FileHistory popover"
    );
}

#[gpui::test]
fn ctrl_h_opens_file_history_for_a_file_at_a_commit(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70604);
    let commit_id = CommitId("abcdef00112233ff".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_h_history",
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

    repo.diff_state.diff_target = Some(DiffTarget::commit(commit_id.clone(), path.clone()));
    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-h");
    draw_and_drain_test_window(cx);

    let is_file_history = cx.update(|_window, app| {
        let host = view.read(app).popover_host.read(app);
        matches!(
            host.popover_kind_for_tests(),
            Some(PopoverKind::FileHistory { .. })
        )
    });
    assert!(
        is_file_history,
        "expected Ctrl+H to open the FileHistory popover"
    );
}

#[gpui::test]
fn ctrl_shortcuts_do_not_crash_without_diff_target(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = crate::test_support::lock_clipboard_test();

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70605);
    let commit_id = CommitId("abcdef00112233gg".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_no_diff_target",
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

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-s ctrl-d ctrl-h ctrl-shift-c ctrl-e");
    draw_and_drain_test_window(cx);

    let clipboard_text = cx.read_from_clipboard().and_then(|item| item.text());

    assert!(
        clipboard_text.is_none(),
        "expected Ctrl+Shift+C to not copy anything without a diff target, got: {clipboard_text:?}"
    );
}

#[gpui::test]
fn ctrl_e_opens_file_in_code_editor(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    crate::external_editor::set_configured_setting_override(Some(
        gitcomet_state::session::ExternalCodeEditorSetting::Custom {
            executable: std::path::PathBuf::from("/usr/bin/true"),
            arguments: None,
        },
    ));

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70620);
    let commit_id = CommitId("abcdef00112233cc".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_e_code_editor",
        std::process::id()
    ));

    // Create the actual workdir and file so that path.exists() passes
    std::fs::create_dir_all(&workdir).expect("should create temp workdir");
    let path = std::path::PathBuf::from("src/lib.rs");
    let full_path = workdir.join(&path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).expect("should create parent dir");
    }
    std::fs::write(&full_path, "// test file").expect("should write test file");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    // Should not panic; Ctrl+E opens the current file in the code editor
    cx.simulate_keystrokes("ctrl-e");
    draw_and_drain_test_window(cx);
}

#[gpui::test]
fn ctrl_e_is_ignored_when_no_editor_configured(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();

    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70621);
    let commit_id = CommitId("abcdef00112233dd".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_e_no_editor",
        std::process::id()
    ));

    std::fs::create_dir_all(&workdir).expect("should create temp workdir");
    let path = std::path::PathBuf::from("src/lib.rs");
    let full_path = workdir.join(&path);
    if let Some(parent) = full_path.parent() {
        std::fs::create_dir_all(parent).expect("should create parent dir");
    }
    std::fs::write(&full_path, "// test file").expect("should write test file");

    let repo = simple_worktree_repo(
        repo_id,
        &workdir,
        &commit_id,
        std::slice::from_ref(&path),
        &path,
    );

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-e");
    draw_and_drain_test_window(cx);
}

#[gpui::test]
fn ctrl_u_unstages_current_file_and_advances_diff(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(70606);
    let commit_id = CommitId("abcdef00112233hh".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ctrl_u_unstage",
        std::process::id()
    ));
    let first = std::path::PathBuf::from("src/first.rs");
    let second = std::path::PathBuf::from("src/second.rs");

    let mut repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);
    repo.status = Loadable::Ready(
        gitcomet_core::domain::RepoStatus {
            unstaged: std::sync::Arc::new(vec![]),
            staged: std::sync::Arc::new(vec![
                gitcomet_core::domain::FileStatus {
                    path: first.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Added,
                    conflict: None,
                },
                gitcomet_core::domain::FileStatus {
                    path: second.clone(),
                    kind: gitcomet_core::domain::FileStatusKind::Added,
                    conflict: None,
                },
            ]),
        }
        .into(),
    );
    let target = DiffTarget::working_tree(first.clone(), DiffArea::Staged);
    repo.diff_state.diff_target = Some(target.clone());
    repo.diff_state.diff = Loadable::Ready(simple_hunk_diff(target).into());
    repo.diff_state.diff_rev = 1;
    repo.diff_state.diff_state_rev = repo.diff_state.diff_state_rev.wrapping_add(1);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    bind_app_keys_and_global_diff_fallback_for_test(cx);
    focus_diff_panel(cx, &view);

    cx.simulate_keystrokes("ctrl-u");
    draw_and_drain_test_window(cx);
    wait_until_store_diff_target_path(cx, &view, second.as_path());
    sync_store_snapshot(cx, &view);

    assert_eq!(
        active_worktree_diff_target_path(cx, &view),
        Some(second),
        "expected Ctrl+U to unstage the active file and advance the diff target"
    );
}
