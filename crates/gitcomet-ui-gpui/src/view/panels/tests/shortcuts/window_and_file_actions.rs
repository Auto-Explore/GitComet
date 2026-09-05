use super::*;

#[gpui::test]
fn ui_scale_picker_selection_updates_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    let repo_id = RepoId(707);
    let commit_id = CommitId("1122334455667788".into());
    let workdir = std::env::temp_dir().join(format!(
        "gitcomet_ui_test_{}_ui_scale_picker",
        std::process::id()
    ));
    let repo = shortcut_fixture_repo(repo_id, &workdir, &commit_id);

    apply_state(cx, &view, app_state_with_active_repo(repo));
    cx.update(|window, app| {
        view.update(app, |this, cx| {
            this.popover_host.update(cx, |host, cx| {
                host.open_popover_at(
                    PopoverKind::UiScalePicker,
                    point(px(72.0), px(72.0)),
                    window,
                    cx,
                );
            });
        });
    });
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected opening the UI scale picker to show a popover"
    );
    assert!(
        cx.debug_bounds("context_menu_125").is_some(),
        "expected the UI scale picker to expose a 125% menu item"
    );

    let zoom_125_bounds = cx
        .debug_bounds("context_menu_125")
        .expect("expected the 125% zoom entry to be rendered");
    cx.simulate_click(zoom_125_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let zoom_percent = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoom_percent, 125,
        "expected selecting 125% from the zoom picker to update the UI scale"
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the UI scale picker to close after selecting a zoom level"
    );
}

#[gpui::test]
fn bottom_status_bar_zoom_button_keeps_icon_at_default_scale_and_opens_picker(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

    assert!(
        cx.debug_bounds("bottom_status_bar_zoom_icon").is_some(),
        "expected the bottom status bar zoom icon to be visible at the default scale"
    );

    let default_button_width = debug_width(cx, "bottom_status_bar_zoom");
    assert!(
        default_button_width < 40.0,
        "expected the default zoom button to stay icon-only (width={default_button_width})"
    );

    let zoom_button_bounds = cx
        .debug_bounds("bottom_status_bar_zoom")
        .expect("expected bottom status bar zoom button bounds");
    cx.simulate_click(zoom_button_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    assert!(
        popover_is_open(cx, &view),
        "expected clicking the bottom status bar zoom button to open the UI scale picker"
    );
    assert_context_menu_entry_fills_popover_width(cx, "context_menu_125");

    let zoom_125_bounds = cx
        .debug_bounds("context_menu_125")
        .expect("expected the 125% zoom entry to be rendered");
    cx.simulate_click(zoom_125_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);

    let zoom_percent = cx.update(|_window, app| view.read(app).ui_scale_percent);
    assert_eq!(
        zoom_percent, 125,
        "expected selecting 125% from the zoom button picker to update the UI scale"
    );
    assert!(
        !popover_is_open(cx, &view),
        "expected the UI scale picker to close after selecting a zoom level from the bottom bar"
    );
    assert!(
        cx.debug_bounds("bottom_status_bar_zoom_icon").is_some(),
        "expected the bottom status bar zoom icon to remain visible after changing zoom"
    );

    let zoomed_button_width = debug_width(cx, "bottom_status_bar_zoom");
    assert!(
        zoomed_button_width > default_button_width + 10.0,
        "expected the non-default zoom button to grow to include its percent label (default={default_button_width}, zoomed={zoomed_button_width})"
    );
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

    assert_eq!(cx.opened_url(), Some(crate::view::EDITIONS_URL.to_string()));
    assert!(
        !popover_is_open(cx, &view),
        "expected the free badge click to leave popovers closed"
    );
}

#[gpui::test]
fn bottom_status_bar_free_badge_scales_with_ui_zoom(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
    let (view, cx) = cx.add_window_view(|window, cx| {
        super::super::GitCometView::new(store, events, None, window, cx)
    });

    open_repo_for_bottom_status_bar_test(cx, &view, RepoId(712), "bottom_status_branding");

    let discord_bounds = cx
        .debug_bounds("bottom_status_bar_discord")
        .expect("expected bottom status bar discord badge bounds");
    cx.simulate_click(discord_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(cx.opened_url(), Some(crate::view::DISCORD_URL.to_string()));

    let version_bounds = cx
        .debug_bounds("bottom_status_bar_version")
        .expect("expected bottom status bar version bounds");
    cx.simulate_click(version_bounds.center(), Modifiers::default());
    draw_and_drain_test_window(cx);
    assert_eq!(cx.opened_url(), Some(crate::view::RELEASES_URL.to_string()));

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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

    assert_eq!(cx.opened_url(), Some(crate::view::WEBSITE_URL.to_string()));
    assert!(
        !popover_is_open(cx, &view),
        "expected the wordmark click to leave popovers closed"
    );
}

#[gpui::test]
fn shared_context_menu_rows_fill_the_popover_width(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
fn ctrl_shortcuts_do_not_crash_without_diff_target(cx: &mut gpui::TestAppContext) {
    let _clipboard_guard = crate::test_support::lock_clipboard_test();

    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            staged: vec![],
            unstaged: vec![gitcomet_core::domain::FileStatus {
                path: path.clone(),
                kind: gitcomet_core::domain::FileStatusKind::Modified,
                conflict: None,
            }],
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
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

    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
    let (store, events) = AppStore::new(Arc::new(TestBackend));
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
            unstaged: vec![],
            staged: vec![
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
            ],
        }
        .into(),
    );
    let target = DiffTarget::WorkingTree {
        path: first.clone(),
        area: DiffArea::Staged,
    };
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
