use super::*;

#[gpui::test]
fn restart_cancellation_only_applies_to_application_close_prompts(cx: &mut gpui::TestAppContext) {
    use gitcomet_extension_api::CloseScope;
    cx.update(|app| {
        crate::view::extension_host::install(
            gitcomet_extension_api::Registry::build(vec![Box::new(
                gitcomet_extension_example::review::ReviewExtension,
            )])
            .unwrap(),
            app,
        );
    });
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    for (scope, click_cancel) in [
        (CloseScope::Window, false),
        (CloseScope::Window, true),
        (CloseScope::Application, false),
        (CloseScope::Application, true),
    ] {
        cx.update(|_, app| app.set_global(crate::app::PendingRestart(true)));
        settings.update(cx, |view, cx| {
            view.confirm_close(scope, vec!["Running operation".into()], cx)
        });
        cx.run_until_parked();
        crate::view::test_support::redraw(cx);
        assert!(cx.debug_bounds("settings_hosted_dialog").is_some());
        if click_cancel {
            let cancel = cx.debug_bounds("settings_close_cancel").unwrap().center();
            cx.simulate_click(cancel, Modifiers::default());
        } else {
            cx.simulate_keystrokes("escape");
        }
        cx.run_until_parked();
        cx.update(|_, app| {
            assert_eq!(
                app.global::<crate::app::PendingRestart>().0,
                scope == CloseScope::Window
            );
            assert!(settings.read(app).extension_dialog.is_none());
            assert_eq!(app.windows().len(), 1);
        });
    }
}

#[test]
fn settings_window_titlebar_options_match_platform_chrome_strategy() {
    let options = settings_window_titlebar_options();
    assert_eq!(
        options.appears_transparent,
        cfg!(any(target_os = "macos", target_os = "windows")),
        "settings window titlebar transparency should match the platform chrome strategy"
    );
    assert_eq!(
        options.title.as_ref().map(ToString::to_string),
        Some("Settings: GitComet".to_string()),
        "settings window titlebar should keep the OS-visible title"
    );
    assert_eq!(
        options.traffic_light_position,
        cfg!(target_os = "macos")
            .then_some(crate::kit::window_chrome::macos_traffic_light_position()),
        "the settings header is the same fixed bar as the main window's, so the \
         lights have to land in the same place"
    );
}

#[test]
fn settings_window_frame_strategy_matches_platform_chrome() {
    #[cfg(target_os = "windows")]
    {
        assert_eq!(settings_window_client_inset(), px(0.0));
    }

    #[cfg(not(target_os = "windows"))]
    {
        assert_eq!(
            settings_window_client_inset(),
            crate::kit::window_chrome::CLIENT_SIDE_DECORATION_INSET
        );
    }
}

#[test]
fn settings_window_options_request_client_chrome_and_resize_behavior() {
    let bounds = Bounds::new(
        point(px(12.0), px(24.0)),
        size(
            px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX),
            px(SETTINGS_WINDOW_DEFAULT_HEIGHT_PX),
        ),
    );
    let options = settings_window_options(bounds);

    assert_eq!(
        options.window_bounds,
        Some(WindowBounds::Windowed(bounds)),
        "settings window should open at the requested bounds"
    );
    assert_eq!(
        options.window_min_size,
        Some(size(
            px(SETTINGS_WINDOW_MIN_WIDTH_PX),
            px(SETTINGS_WINDOW_MIN_HEIGHT_PX),
        )),
        "settings window should enforce its minimum size"
    );
    assert_eq!(
        options.window_decorations,
        Some(WindowDecorations::Client),
        "settings window should request client-side decorations"
    );
    assert!(
        options.icon.is_some(),
        "settings window carries the product's window icon"
    );
    assert!(
        options.is_movable,
        "settings window should remain movable with custom chrome"
    );
    assert!(
        options.is_resizable,
        "settings window should remain resizable with custom chrome"
    );
}

#[gpui::test]
fn settings_window_blur_requires_deliberate_input_focus(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.update(|window, _| window.activate());
    cx.run_until_parked();
    let input = cx.update(|window, app| {
        let input = view.read(app).search_input.clone();
        window.focus(&input.read(app).focus_handle(), app);
        let _ = window.draw(app);
        input
    });
    cx.simulate_keystrokes("a");
    cx.deactivate_window();
    cx.update(|window, app| {
        assert!(window.focused(app).is_none());
        window.activate();
    });
    cx.run_until_parked();
    crate::test_support::refresh_and_draw(cx);
    cx.simulate_keystrokes("b");
    cx.update(|window, app| {
        assert_eq!(input.read(app).text(), "a");
        window.focus(&input.read(app).focus_handle(), app);
    });
    crate::test_support::refresh_and_draw(cx);
    cx.simulate_keystrokes("c");
    cx.update(|_, app| assert_eq!(input.read(app).text(), "ac"));
}

#[gpui::test]
fn settings_window_sets_platform_title(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();

    assert_eq!(
        settings_cx.window_title().as_deref(),
        Some("Settings: GitComet"),
        "expected settings window to expose the native OS title"
    );
}

#[gpui::test]
fn expanded_diff_content_mode_section_renders_before_scroll_sync_row(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.set_expanded_section(Some(SettingsSection::DiffContentMode), cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let diff_mode_row = settings_cx
        .debug_bounds("settings_window_diff_content_mode")
        .expect("expected diff mode row bounds");
    let diff_mode_container = settings_cx
        .debug_bounds("settings_window_diff_content_mode_list_container")
        .expect("expected diff mode list container bounds");
    let scroll_sync_row = settings_cx
        .debug_bounds("settings_window_diff_scroll_sync")
        .expect("expected scroll sync row bounds");

    assert!(
        diff_mode_row.bottom() <= diff_mode_container.top()
            && diff_mode_container.bottom() <= scroll_sync_row.top(),
        "expected the diff mode selector to expand directly below the diff mode row"
    );
}

#[gpui::test]
fn expanded_history_columns_section_renders_detail_container(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.set_expanded_section(Some(SettingsSection::GitLogColumns), cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_git_log_columns_container")
            .is_some(),
        "expected the history columns section to render its detail container when expanded"
    );
}

#[gpui::test]
fn expanded_git_log_default_mode_section_renders_modes_in_order_and_updates_selection(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.set_expanded_section(Some(SettingsSection::GitLogDefaultMode), cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut previous_top = None;
    for spec in crate::view::history_mode::history_mode_ui_specs() {
        let bounds = settings_cx
            .debug_bounds(spec.settings_row_id)
            .unwrap_or_else(|| panic!("expected `{}` bounds", spec.settings_row_id));
        if let Some(previous_top) = previous_top {
            assert!(
                bounds.top() > previous_top,
                "expected `{}` to appear below the previous history mode row",
                spec.settings_row_id
            );
        }
        previous_top = Some(bounds.top());
    }

    let selected = crate::view::history_mode::history_mode_ui_specs()
        .last()
        .copied()
        .expect("history modes");
    let initial_selected_bounds = settings_cx
        .debug_bounds(selected.settings_row_id)
        .expect("expected selected row bounds");
    let scroll_bounds = settings_cx
        .debug_bounds("settings_window_scroll")
        .expect("expected settings scroll bounds");
    let selected_center = initial_selected_bounds.center();
    if selected_center.y >= scroll_bounds.bottom() {
        let scroll_delta = selected_center.y - scroll_bounds.bottom() + px(24.0);
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            let current = settings.settings_window_scroll.offset();
            settings
                .settings_window_scroll
                .set_offset(point(current.x, current.y - scroll_delta));
            cx.notify();
        });
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
    }
    let selected_bounds = settings_cx
        .debug_bounds(selected.settings_row_id)
        .expect("expected selected row bounds");
    settings_cx.simulate_click(selected_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.default_history_mode)
                .expect("settings window should remain readable"),
            selected.mode
        );
    });
}

#[gpui::test]
fn expanded_git_log_default_mode_section_renders_before_history_columns_row(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.set_expanded_section(Some(SettingsSection::GitLogDefaultMode), cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let default_mode_container = settings_cx
        .debug_bounds("settings_window_git_log_default_mode_container")
        .expect("expected default history mode container bounds");
    let history_columns_row = settings_cx
        .debug_bounds("settings_window_git_log_columns")
        .expect("expected history columns row bounds");

    assert!(
        default_mode_container.bottom() <= history_columns_row.top(),
        "expected the default history mode container to appear before the history columns row"
    );
}

#[gpui::test]
fn expanded_auto_fetch_tags_section_renders_detail_container(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.history_show_tags = true;
        settings.set_expanded_section(Some(SettingsSection::GitLogTagFetch), cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_git_log_tag_fetch_container")
            .is_some(),
        "expected the auto fetch tags section to render its detail container when expanded"
    );
}

#[gpui::test]
fn settings_window_open_source_licenses_row_switches_content(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Links, cx);
        // Keep the interaction test resilient as rows are added to the root links card.
        let current_x = settings.settings_window_scroll.offset().x;
        let max_offset = settings.settings_window_scroll.max_offset().y.max(px(0.0));
        settings
            .settings_window_scroll
            .set_offset(point(current_x, -max_offset));
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let row_bounds = settings_cx
        .debug_bounds("settings_window_open_source_licenses")
        .expect("expected open source licenses row bounds");
    settings_cx.simulate_click(row_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert_eq!(
            app.windows().len(),
            2,
            "expected the settings window to reuse the existing window"
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.current_view)
                .expect("settings window should remain readable"),
            SettingsView::OpenSourceLicenses,
            "expected the settings window to switch to open source licenses content"
        );
    });

    assert_eq!(
        settings_cx.window_title().as_deref(),
        Some("Settings: GitComet"),
        "expected the settings window to keep its OS title"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_breadcrumb_settings")
            .is_some(),
        "expected a breadcrumb back control in the licenses view"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_open_source_licenses_columns")
            .is_some(),
        "expected open source licenses columns in debug bounds"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_open_source_licenses_scrollbar")
            .is_some(),
        "expected a visible scrollbar in the open source licenses view"
    );

    let back_bounds = settings_cx
        .debug_bounds("settings_window_breadcrumb_settings")
        .expect("expected breadcrumb back control bounds");
    settings_cx.simulate_click(back_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    cx.update(|_window, app| {
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.current_view)
                .expect("settings window should remain readable"),
            SettingsView::Root,
            "expected the breadcrumb back control to return to the root settings view"
        );
    });
}

#[gpui::test]
fn settings_window_professional_edition_waitlist_row_opens_editions_page(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Links, cx);
        // Keep the interaction test resilient as sections are added above the links card.
        let current_x = settings.settings_window_scroll.offset().x;
        let max_offset = settings.settings_window_scroll.max_offset().y.max(px(0.0));
        settings
            .settings_window_scroll
            .set_offset(point(current_x, -max_offset));
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let row_bounds = settings_cx
        .debug_bounds("settings_window_professional_edition_waitlist")
        .expect("expected professional edition waitlist row bounds");
    settings_cx.simulate_click(row_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();

    assert_eq!(
        cx.opened_url(),
        Some(crate::view::editions_url().unwrap().to_string())
    );
}

#[gpui::test]
fn settings_window_root_view_renders_visible_scrollbar(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let synthetic_fonts: Arc<[String]> = (0..200)
        .map(|ix| format!("Test UI Font {ix:03}"))
        .collect::<Vec<_>>()
        .into();

    cx.update(|_window, app| {
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.ui_font_options = synthetic_fonts.clone();
            settings.ui_font_family = synthetic_fonts[0].clone();
            settings.set_expanded_section(Some(SettingsSection::UiFont), cx);
            settings.settings_window_scroll = ScrollHandle::default();
            settings.ui_font_scroll = UniformListScrollHandle::default();
            cx.notify();
        });
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(
        px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX),
        px(SETTINGS_WINDOW_MIN_HEIGHT_PX),
    ));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let max_offset = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            settings.settings_window_scroll.max_offset().y.max(px(0.0))
        })
        .expect("settings window should remain readable");
    assert!(
        max_offset > px(0.0),
        "expected the root settings page to be scrollable during the test"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_scrollbar")
            .is_some(),
        "expected a visible scrollbar in the root settings view"
    );
}

#[gpui::test]
fn settings_window_rows_clamp_under_lilex_at_minimum_width(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.ui_font_family = crate::bundled_fonts::LILEX_FONT_FAMILY.to_string();
        settings.runtime_info.environment.app_version =
            "GitComet v0.0.0-overflow-regression-build".into();
        settings.runtime_info.environment.system.operating_system = Some(
            "linux (gnu-linux-overflow-regression-platform, x86_64-extra-build-metadata)".into(),
        );
        settings.runtime_info.git.version_display =
            "git version 2.51.0 (overflow-regression-build-with-very-long-metadata)".into();
        settings.runtime_info.git.compatibility = GitCompatibility::Supported;
        settings.runtime_info.signing_tools = Some({
            use gitcomet_core::signing_tools::{
                SigningTool, SigningToolAvailability, SigningToolsState,
            };
            let found = |program: &str, version: &str| SigningTool {
                program: program.to_string(),
                availability: SigningToolAvailability::Available {
                    version: Some(version.to_string()),
                },
            };
            SigningToolsState {
                gpg: found(
                    "gpg",
                    "gpg (GnuPG) 2.4.7 (overflow-regression-build-with-very-long-metadata)",
                ),
                ssh_keygen: found("ssh-keygen", "OpenSSH_10.3p1, LibreSSL 3.3.6"),
            }
        });
        settings.overflow_probe = true;
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(
        px(SETTINGS_WINDOW_MIN_WIDTH_PX),
        px(SETTINGS_WINDOW_DEFAULT_HEIGHT_PX),
    ));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for (row_selector, label_selector, value_selector) in [
        (
            "settings_window_overflow_summary",
            "settings_window_overflow_summary_label",
            "settings_window_overflow_summary_value",
        ),
        (
            "settings_window_overflow_toggle",
            "settings_window_overflow_toggle_label",
            "settings_window_overflow_toggle_value",
        ),
        (
            "settings_window_overflow_info",
            "settings_window_overflow_info_label",
            "settings_window_overflow_info_value",
        ),
        (
            "settings_window_overflow_link",
            "settings_window_overflow_link_label",
            "settings_window_overflow_link_value",
        ),
        (
            "settings_window_git_runtime",
            "settings_window_git_runtime_label",
            "settings_window_git_runtime_value",
        ),
        (
            "settings_window_gpg_runtime",
            "settings_window_gpg_runtime_label",
            "settings_window_gpg_runtime_value",
        ),
    ] {
        assert_debug_bounds_within(&mut settings_cx, row_selector, label_selector);
        assert_debug_bounds_within(&mut settings_cx, row_selector, value_selector);
    }

    // Executables stack the version under the program name, so a long version
    // never competes with the name and status for one line on narrow windows.
    for (row_selector, label_selector, status_selector, version_selector) in [
        (
            "settings_window_git_runtime",
            "settings_window_git_runtime_label",
            "settings_window_git_runtime_status",
            "settings_window_git_runtime_value",
        ),
        (
            "settings_window_gpg_runtime",
            "settings_window_gpg_runtime_label",
            "settings_window_gpg_runtime_status",
            "settings_window_gpg_runtime_value",
        ),
    ] {
        assert_debug_bounds_within(&mut settings_cx, row_selector, status_selector);
        let label = settings_cx
            .debug_bounds(label_selector)
            .unwrap_or_else(|| panic!("expected `{label_selector}` bounds"));
        let version = settings_cx
            .debug_bounds(version_selector)
            .unwrap_or_else(|| panic!("expected `{version_selector}` bounds"));
        assert!(
            version.top() >= label.bottom() - px(0.5),
            "expected `{version_selector}` on its own line below `{label_selector}` \
             (label={label:?}, version={version:?})"
        );
    }
}

#[gpui::test]
fn settings_window_containers_fill_available_width_when_content_wraps(
    cx: &mut gpui::TestAppContext,
) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let synthetic_fonts: Arc<[String]> = (0..24)
        .map(|ix| format!("Overflow Regression UI Font {ix:02} With Extended Width Coverage"))
        .collect::<Vec<_>>()
        .into();

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.ui_font_options = synthetic_fonts.clone();
        settings.ui_font_family = synthetic_fonts[0].clone();
        settings.git_executable_mode = GitExecutableMode::Custom;
        settings.runtime_info.environment.app_version =
            "GitComet v0.0.0-overflow-regression-build-with-extra-layout-metadata".into();
        settings.runtime_info.environment.system.operating_system =
            Some("linux (gnu-linux-overflow-regression-platform with verbose wrapping metadata, x86_64)"
                .into());
        settings.runtime_info.git.version_display =
            "git version 2.51.0 (overflow-regression-build-with-very-long-metadata)".into();
        settings.runtime_info.git.compatibility = GitCompatibility::Unknown;
        settings.runtime_info.git.detail = Some(
            "This deliberately long compatibility detail must wrap inside the Git executable card without shrinking the settings containers into narrow blocks."
                .into(),
        );
        settings.settings_window_scroll = ScrollHandle::default();
        settings.ui_font_scroll = UniformListScrollHandle::default();
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_MIN_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    // Each category renders its card on its own page now, so visit every
    // category and verify the visible card fills the content-pane width.
    for (category, card_selector) in [
        (SettingsCategory::General, "settings_window_general"),
        (SettingsCategory::Appearance, "settings_window_appearance"),
        (
            SettingsCategory::SecurityPrivacy,
            "settings_window_security_privacy_card",
        ),
        (
            SettingsCategory::ChangeTracking,
            "settings_window_change_tracking_card",
        ),
        (SettingsCategory::Diff, "settings_window_diff_card"),
        (
            SettingsCategory::FileEditing,
            "settings_window_file_editing_card",
        ),
        (SettingsCategory::GitLog, "settings_window_git_log_card"),
        (SettingsCategory::Remotes, "settings_window_remotes_card"),
        (
            SettingsCategory::LargeFiles,
            "settings_window_large_files_card",
        ),
        (SettingsCategory::Tags, "settings_window_tags_card"),
        (
            SettingsCategory::Maintenance,
            "settings_window_maintenance_card",
        ),
        (
            SettingsCategory::GitExecutable,
            "settings_window_git_executable",
        ),
        (SettingsCategory::Environment, "settings_window_environment"),
        (SettingsCategory::Links, "settings_window_links"),
    ] {
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            settings.select_category(category, cx);
            // The Appearance page keeps a dropdown expanded to exercise wrapping.
            if category == SettingsCategory::Appearance {
                settings.set_expanded_section(Some(SettingsSection::UiFont), cx);
            }
            cx.notify();
        });
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert_debug_matching_horizontal_insets(
            &mut settings_cx,
            "settings_window_scroll",
            card_selector,
        );

        if category == SettingsCategory::Appearance {
            assert_debug_matching_horizontal_insets(
                &mut settings_cx,
                "settings_window_appearance",
                "settings_window_ui_font_list_container",
            );
            assert_debug_matching_horizontal_insets(
                &mut settings_cx,
                "settings_window_appearance",
                "settings_window_theme_grid",
            );
        }
        if category == SettingsCategory::GitExecutable {
            assert_debug_matching_horizontal_insets(
                &mut settings_cx,
                "settings_window_git_executable",
                "settings_window_git_executable_custom_container",
            );
        }
    }
}

/// Unoptimized CI builds give every builder temporary its own stack slot, so
/// the old single-function render needed ~2 MiB and overflowed the 2 MiB
/// Windows test thread. Now ~670 KiB; an overflow here aborts the binary.
#[test]
fn settings_window_renders_every_category_within_a_bounded_stack() {
    std::thread::Builder::new()
        .name("settings_render_stack_budget".into())
        .stack_size(1024 * 1024)
        .spawn(|| {
            let mut app = gpui::TestAppContext::single();
            app.update(open_settings_window);
            let window = app.update(|app| {
                app.windows()
                    .into_iter()
                    .find_map(|window| window.downcast::<SettingsWindowView>())
                    .expect("settings window should be open")
            });
            let view = window.root(&mut app).unwrap();
            let cx = &mut gpui::VisualTestContext::from_window(*window.deref(), &app);
            for &category in SettingsCategory::ALL {
                view.update(cx, |view, cx| {
                    view.expanded_section = None;
                    view.select_category(category, cx);
                });
                crate::test_support::refresh_and_draw(cx);
            }
            view.update(cx, |view, cx| view.show_open_source_licenses(cx));
            crate::test_support::refresh_and_draw(cx);
        })
        .unwrap()
        .join()
        .unwrap();
}

fn assert_debug_bounds_within(
    cx: &mut gpui::VisualTestContext,
    outer_selector: &'static str,
    inner_selector: &'static str,
) {
    let outer_bounds = cx
        .debug_bounds(outer_selector)
        .unwrap_or_else(|| panic!("expected `{outer_selector}` bounds"));
    let inner_bounds = cx
        .debug_bounds(inner_selector)
        .unwrap_or_else(|| panic!("expected `{inner_selector}` bounds"));
    let tolerance = px(0.5);

    assert!(
        inner_bounds.left() >= outer_bounds.left() - tolerance
            && inner_bounds.right() <= outer_bounds.right() + tolerance
            && inner_bounds.top() >= outer_bounds.top() - tolerance
            && inner_bounds.bottom() <= outer_bounds.bottom() + tolerance,
        "expected `{inner_selector}` to stay within `{outer_selector}` \
             (outer={outer_bounds:?}, inner={inner_bounds:?})"
    );
}

fn assert_debug_matching_horizontal_insets(
    cx: &mut gpui::VisualTestContext,
    outer_selector: &'static str,
    inner_selector: &'static str,
) {
    let outer_bounds = cx
        .debug_bounds(outer_selector)
        .unwrap_or_else(|| panic!("expected `{outer_selector}` bounds"));
    let inner_bounds = cx
        .debug_bounds(inner_selector)
        .unwrap_or_else(|| panic!("expected `{inner_selector}` bounds"));
    let left_inset = inner_bounds.left() - outer_bounds.left();
    let right_inset = outer_bounds.right() - inner_bounds.right();
    let tolerance = px(1.0);

    assert!(
        (left_inset - right_inset).abs() <= tolerance,
        "expected `{inner_selector}` to use the full horizontal content width inside \
             `{outer_selector}` (left inset={left_inset:?}, right inset={right_inset:?}, \
             outer={outer_bounds:?}, inner={inner_bounds:?})"
    );
}
