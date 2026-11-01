use super::*;

#[test]
fn workspaces_category_is_listed_after_appearance_and_matches_its_search_terms() {
    assert_eq!(SettingsCategory::ALL[1], SettingsCategory::Appearance);
    assert_eq!(SettingsCategory::ALL[2], SettingsCategory::Workspaces);
    for query in ["workspace", "title bar color", "rename"] {
        assert!(
            SettingsCategory::Workspaces.matches_query(query),
            "{query} should find the Workspaces page"
        );
    }
    assert_eq!(
        SettingsSection::WorkspaceTheme.category(),
        SettingsCategory::Workspaces
    );
}

#[gpui::test]
fn appearance_theme_tiles_switch_main_windows_and_explain_workspace_overrides(
    cx: &mut gpui::TestAppContext,
) {
    let mut workspace =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/appearance-tiles-a")]);
    workspace.theme_mode = Some("tokyo_night".to_string());
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace]);
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(2400.0)));
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for group in ["automatic", "dark", "light"] {
        let selector: &'static str = format!("settings_window_theme_group_{group}").leak();
        assert!(settings_cx.debug_bounds(selector).is_some(), "{selector}");
    }
    for option in crate::theme::available_themes() {
        let selector: &'static str = format!("settings_window_theme_{}", option.key).leak();
        assert!(
            settings_cx.debug_bounds(selector).is_some(),
            "expected a tile for {}",
            option.key
        );
    }

    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_tokyo_night_orb")
            .is_some(),
        "each tile carries its orb"
    );

    let click = |settings_cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = settings_cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} to be rendered"));
        settings_cx.simulate_click(bounds.center(), Modifiers::default());
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };
    click(&mut settings_cx, "settings_window_theme_gitcomet_light");
    let light = AppTheme::gitcomet_light();
    main_view.update(&mut settings_cx, |view, _cx| {
        assert_eq!(
            view.theme_mode,
            ThemeMode::Named(crate::theme::DEFAULT_LIGHT_THEME_KEY.to_string())
        );
        assert_eq!(
            view.theme.colors.surface.canvas,
            light.colors.surface.canvas
        );
    });
    let _ = settings_window.update(&mut settings_cx, |settings, _window, _cx| {
        assert_eq!(
            settings.theme_mode,
            ThemeMode::Named(crate::theme::DEFAULT_LIGHT_THEME_KEY.to_string())
        );
    });

    click(
        &mut settings_cx,
        "settings_window_theme_workspace_overrides",
    );
    let _ = settings_window.update(&mut settings_cx, |settings, _window, _cx| {
        assert_eq!(settings.selected_category, SettingsCategory::Workspaces);
    });
}

#[gpui::test]
fn appearance_page_hides_the_workspace_override_note_without_overrides(
    cx: &mut gpui::TestAppContext,
) {
    let workspace =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/appearance-tiles-b")]);
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace]);
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
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_grid")
            .is_some()
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_workspace_overrides")
            .is_none()
    );
}

/// The app theme can name a user theme deleted since it was picked; a
/// workspace must still be able to go back to following it.
#[gpui::test]
fn workspace_theme_picker_offers_follow_app_when_the_app_theme_is_gone(
    cx: &mut gpui::TestAppContext,
) {
    let mut workspace =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/workspaces-page-c")]);
    workspace.theme_mode = Some("tokyo_night".to_string());
    let id = workspace.id;
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace]);
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1400.0)));
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.theme_mode = ThemeMode::Named("deleted_custom_theme".to_string());
        settings.select_category(SettingsCategory::Workspaces, cx);
        settings.select_workspace(id, cx);
        settings.toggle_section(SettingsSection::WorkspaceTheme, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let follow = settings_cx
        .debug_bounds("settings_window_workspace_theme_follow_app")
        .expect("Follow app theme stays offered");
    settings_cx.simulate_click(follow.center(), Modifiers::default());
    settings_cx.run_until_parked();
    assert_eq!(
        settings_cx.update(|_window, app| {
            crate::workspaces::workspace(app, id).and_then(|workspace| workspace.theme_mode)
        }),
        None
    );
}

#[gpui::test]
fn workspaces_page_edits_colour_theme_name_and_deletes(cx: &mut gpui::TestAppContext) {
    let mut workspace =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/workspaces-page-a")]);
    workspace.last_activation_order = 5;
    let id = workspace.id;
    let other =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/workspaces-page-b")]);
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace, other]);
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1400.0)));
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Workspaces, cx);
    });
    let redraw = |settings_cx: &mut gpui::VisualTestContext| {
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };
    let click = |settings_cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let bounds = settings_cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} to be rendered"));
        settings_cx.simulate_click(bounds.center(), Modifiers::default());
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
    };
    redraw(&mut settings_cx);

    let selected = settings_window
        .read_with(&settings_cx, |settings, _| settings.selected_workspace)
        .expect("settings window");
    assert_eq!(
        selected,
        Some(id),
        "the most recently used workspace is preselected"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_workspaces_intro")
            .is_some(),
        "the page explains how to start a new workspace"
    );
    let row: &'static str = format!("settings_window_workspace_{id}").leak();
    assert!(settings_cx.debug_bounds(row).is_some());
    let dot: &'static str = format!("settings_window_swatch_{id}").leak();
    assert!(
        settings_cx.debug_bounds(dot).is_some(),
        "rows are the picker's workspace rows, colour dot included"
    );
    let open = settings_cx
        .debug_bounds("settings_window_workspace_open")
        .expect("open action");
    let delete = settings_cx
        .debug_bounds("settings_window_workspace_delete")
        .expect("delete button");
    assert!(
        delete.top() > open.bottom(),
        "delete sits apart, below open"
    );

    click(&mut settings_cx, "settings_window_workspace_color_blue");
    let read = |settings_cx: &mut gpui::VisualTestContext| {
        settings_cx.update(|_window, app| crate::workspaces::workspace(app, id))
    };
    assert_eq!(
        read(&mut settings_cx).and_then(|workspace| workspace.color),
        Some(gitcomet_state::session::WorkspaceColor::Blue)
    );

    click(&mut settings_cx, "settings_window_workspace_theme");
    click(
        &mut settings_cx,
        "settings_window_workspace_theme_tokyo_night",
    );
    assert_eq!(
        read(&mut settings_cx).and_then(|workspace| workspace.theme_mode),
        Some("tokyo_night".to_string())
    );
    // The grid stays open after a pick so themes can be compared in turn.
    click(&mut settings_cx, "settings_window_workspace_theme_nord");
    assert_eq!(
        read(&mut settings_cx).and_then(|workspace| workspace.theme_mode),
        Some("nord".to_string())
    );
    click(
        &mut settings_cx,
        "settings_window_workspace_theme_follow_app",
    );
    assert_eq!(
        read(&mut settings_cx).and_then(|workspace| workspace.theme_mode),
        None
    );
    // Only the header collapses it.
    click(&mut settings_cx, "settings_window_workspace_theme");
    assert!(
        settings_cx
            .debug_bounds("settings_window_workspace_theme_grid")
            .is_none()
    );

    // Typing alone does not save; the Save button beside the field does.
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings
            .workspace_name_input
            .update(cx, |input, cx| input.set_text("  Client work ", cx));
    });
    redraw(&mut settings_cx);
    assert_eq!(read(&mut settings_cx).and_then(|w| w.custom_name), None);
    click(&mut settings_cx, "settings_window_workspace_name_save");
    assert_eq!(
        read(&mut settings_cx).and_then(|workspace| workspace.custom_name),
        Some("Client work".to_string())
    );

    click(&mut settings_cx, "settings_window_workspace_delete");
    assert!(
        read(&mut settings_cx).is_some(),
        "delete asks for confirmation first"
    );
    click(&mut settings_cx, "settings_window_workspace_delete_cancel");
    assert!(read(&mut settings_cx).is_some());
    assert!(
        settings_cx
            .debug_bounds("settings_window_workspace_delete")
            .is_some(),
        "cancel brings the delete button back"
    );
    click(&mut settings_cx, "settings_window_workspace_delete");
    click(&mut settings_cx, "settings_window_workspace_delete_confirm");
    assert!(read(&mut settings_cx).is_none());
    let selected = settings_window
        .read_with(&settings_cx, |settings, _| settings.selected_workspace)
        .expect("settings window");
    assert!(
        selected.is_some_and(|selected| selected != id),
        "the selection moves to a remaining workspace"
    );
}

/// Deleting the workspace of an open window closes that window rather than
/// leaving it to re-create the workspace on its next sync.
#[gpui::test]
fn deleting_an_open_workspace_from_settings_closes_its_window(cx: &mut gpui::TestAppContext) {
    let mut workspace = gitcomet_state::session::Workspace::new(Vec::new());
    workspace.custom_name = Some("Doomed".into());
    let id = workspace.id;
    let backend: std::sync::Arc<dyn gitcomet_core::services::GitBackend> =
        std::sync::Arc::new(TestBackend);
    let (store, events) = AppStore::new_test(std::sync::Arc::clone(&backend));
    let (main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let main_window = cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![workspace.clone()]);
        crate::app::install_app_shortcuts_for_test(app, backend);
        let _ = window.draw(app);
        window.window_handle().window_id()
    });
    cx.update(|_window, app| {
        main_view.update(app, |view, cx| view.adopt_workspace(workspace, cx));
    });
    cx.run_until_parked();
    // A second main window, so the deleted one closes instead of going Home.
    cx.update(|_window, app| crate::app::open_new_empty_window(app));
    cx.update(|_window, app| open_settings_window(app));
    cx.run_until_parked();
    let settings_window = cx.update(|_window, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1400.0)));
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Workspaces, cx);
        settings.select_workspace(id, cx);
    });
    let click = |settings_cx: &mut gpui::VisualTestContext, selector: &'static str| {
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let bounds = settings_cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected {selector} to be rendered"));
        settings_cx.simulate_click(bounds.center(), Modifiers::default());
        settings_cx.run_until_parked();
    };
    click(&mut settings_cx, "settings_window_workspace_delete");
    click(&mut settings_cx, "settings_window_workspace_delete_confirm");

    settings_cx.update(|_window, app| {
        assert!(crate::workspaces::workspace(app, id).is_none());
        assert!(
            app.windows()
                .iter()
                .all(|window| window.window_id() != main_window),
            "the workspace's window closes"
        );
        assert_eq!(
            app.windows()
                .iter()
                .filter(|window| window.downcast::<GitCometView>().is_some())
                .count(),
            1,
            "the other main window stays"
        );
    });
}

#[gpui::test]
fn opening_settings_to_a_workspace_selects_its_page_and_row(cx: &mut gpui::TestAppContext) {
    let first = gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/ws-link-a")]);
    let second = gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/ws-link-b")]);
    let (first_id, second_id) = (first.id, second.id);
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, vec![first, second]);
        let _ = window.draw(app);
    });

    let selection = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            let window = app
                .windows()
                .into_iter()
                .find_map(|window| window.downcast::<SettingsWindowView>())
                .expect("settings window");
            window
                .read_with(app, |view, _| {
                    (view.selected_category, view.selected_workspace)
                })
                .expect("readable settings window")
        })
    };

    cx.update(|_window, app| open_settings_window_to_workspace(app, second_id));
    cx.run_until_parked();
    assert_eq!(
        selection(cx),
        (SettingsCategory::Workspaces, Some(second_id))
    );

    // Already open on another page: the link still lands on the workspace.
    cx.update(|_window, app| {
        let window = app
            .windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window");
        let _ = window.update(app, |view, _window, cx| {
            view.select_category(SettingsCategory::Diff, cx);
        });
        open_settings_window_to_workspace(app, first_id);
    });
    cx.run_until_parked();
    assert_eq!(
        selection(cx),
        (SettingsCategory::Workspaces, Some(first_id))
    );
}

#[gpui::test]
fn new_workspace_button_opens_an_empty_window(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::app::install_app_shortcuts_for_test(app, std::sync::Arc::new(TestBackend));
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
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Workspaces, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let count_views = |cx: &mut gpui::VisualTestContext| {
        cx.update(|_window, app| {
            app.windows()
                .into_iter()
                .filter(|window| window.downcast::<GitCometView>().is_some())
                .count()
        })
    };
    let before = count_views(&mut settings_cx);

    let button = settings_cx
        .debug_bounds("settings_window_workspace_new")
        .expect("New Workspace button");
    settings_cx.simulate_click(button.center(), Modifiers::default());
    settings_cx.run_until_parked();

    assert_eq!(count_views(&mut settings_cx), before + 1);
}
