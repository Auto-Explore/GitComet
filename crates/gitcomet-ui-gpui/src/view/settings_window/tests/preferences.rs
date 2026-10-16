use super::*;

#[test]
fn settings_dropdown_background_is_darker_than_card_surface() {
    fn brightness(color: gpui::Rgba) -> f32 {
        color.red + color.green + color.blue
    }

    let dark = AppTheme::gitcomet_dark();
    assert!(
        brightness(settings_dropdown_background(dark)) < brightness(dark.colors.surface.raised),
        "dark dropdown surface should be darker than the card surface"
    );

    let light = AppTheme::gitcomet_light();
    assert!(
        brightness(settings_dropdown_background(light)) < brightness(light.colors.surface.raised),
        "light dropdown surface should still read darker than the card surface"
    );
}

#[gpui::test]
fn expanded_settings_sections_render_scrollable_list_containers(cx: &mut gpui::TestAppContext) {
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1800.0)));
    settings_cx.run_until_parked();

    for (section, selector) in [
        (
            SettingsSection::DateFormat,
            "settings_window_date_format_list_container",
        ),
        (
            SettingsSection::UiFont,
            "settings_window_ui_font_list_container",
        ),
        (
            SettingsSection::EditorFont,
            "settings_window_editor_font_list_container",
        ),
        (
            SettingsSection::ExternalCodeEditor,
            "settings_window_external_code_editor_list_container",
        ),
        (
            SettingsSection::Timezone,
            "settings_window_timezone_list_container",
        ),
        (
            SettingsSection::ChangeTracking,
            "settings_window_change_tracking_list_container",
        ),
        (
            SettingsSection::Diff,
            "settings_window_diff_scroll_sync_list_container",
        ),
        (
            SettingsSection::DiffContentMode,
            "settings_window_diff_content_mode_list_container",
        ),
        (
            SettingsSection::AllowedRemoteProtocols,
            "settings_window_remote_protocols_list_container",
        ),
    ] {
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            settings.set_expanded_section(Some(section), cx);
        });
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        assert!(
            settings_cx.debug_bounds(selector).is_some(),
            "expected `{selector}` to be rendered for the expanded section"
        );
    }
}

#[gpui::test]
fn settings_dropdowns_fit_without_inner_scroll(cx: &mut gpui::TestAppContext) {
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

    for (section, label) in [
        (SettingsSection::DateFormat, "Date time format"),
        (SettingsSection::ChangeTracking, "Untracked files"),
        (SettingsSection::Diff, "Diff scroll sync"),
    ] {
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            settings.set_expanded_section(Some(section), cx);
        });
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });

        let max_offset = settings_window
            .update(&mut settings_cx, |settings, _window, _cx| match section {
                SettingsSection::DateFormat => {
                    uniform_list_vertical_scroll_metrics(&settings.date_format_scroll).2
                }
                SettingsSection::ChangeTracking => {
                    uniform_list_vertical_scroll_metrics(&settings.change_tracking_scroll).2
                }
                SettingsSection::Diff => {
                    uniform_list_vertical_scroll_metrics(&settings.diff_scroll_sync_scroll).2
                }
                _ => px(0.0),
            })
            .expect("settings window should remain readable");

        assert_eq!(
            max_offset,
            px(0.0),
            "expected the {label} dropdown to fit without inner scroll"
        );
    }
}

#[gpui::test]
fn non_macos_settings_window_renders_custom_chrome_controls(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }
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
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for selector in [
        "settings_window_header_drag",
        "settings_window_min",
        "settings_window_max",
        "settings_window_close",
    ] {
        assert!(
            settings_cx.debug_bounds(selector).is_some(),
            "expected `{selector}` in debug bounds"
        );
    }
}

#[gpui::test]
fn hidden_window_controls_keep_only_close_in_settings_chrome(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        crate::kit::window_chrome::controls::set_current(
            app,
            crate::kit::window_chrome::controls::WindowControlsMode::Hide,
        );
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
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(settings_cx.debug_bounds("settings_window_min").is_none());
    assert!(settings_cx.debug_bounds("settings_window_max").is_none());
    assert!(settings_cx.debug_bounds("settings_window_close").is_some());
}

#[gpui::test]
fn linux_settings_window_close_button_closes_only_the_settings_window(
    cx: &mut gpui::TestAppContext,
) {
    if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
        return;
    }
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        open_settings_window(app);
    });
    cx.run_until_parked();

    let settings_window = cx.update(|_window, app| {
        assert_eq!(app.windows().len(), 2, "expected main + settings windows");
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .expect("settings window should be open")
    });

    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), cx);
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let close_bounds = settings_cx
        .debug_bounds("settings_window_close")
        .expect("expected settings window close control bounds");
    settings_cx.simulate_mouse_move(close_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_mouse_down(
        close_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    settings_cx.simulate_mouse_up(
        close_bounds.center(),
        MouseButton::Left,
        Modifiers::default(),
    );
    settings_cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            app.windows().len(),
            1,
            "expected the settings close control to close only the settings window"
        );
        assert!(
            app.windows()
                .into_iter()
                .all(|window| window.downcast::<SettingsWindowView>().is_none()),
            "expected the settings window to be removed"
        );
    });
}

#[gpui::test]
fn show_timezone_toggle_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    let next_show_timezone = cx.update(|_window, app| {
        !settings_window
            .read_with(app, |settings, _cx| settings.show_timezone)
            .expect("settings window should be readable")
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_show_timezone(next_show_timezone, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "settings window toggle should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::show_timezone(main_view.read(app)),
            next_show_timezone
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.show_timezone)
                .expect("settings window should remain readable"),
            next_show_timezone
        );
    });
}

#[gpui::test]
fn change_tracking_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    let next_view = cx.update(|_window, app| {
        let current = settings_window
            .read_with(app, |settings, _cx| settings.change_tracking_view)
            .expect("settings window should be readable");
        match current {
            ChangeTrackingView::Combined => ChangeTrackingView::SplitUntracked,
            ChangeTrackingView::SplitUntracked => ChangeTrackingView::Combined,
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_change_tracking_view(next_view, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "change tracking update should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::change_tracking_view(main_view.read(app)),
            next_view
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.change_tracking_view)
                .expect("settings window should remain readable"),
            next_view
        );
    });
}

#[gpui::test]
fn diff_scroll_sync_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    let next_mode = cx.update(|_window, app| {
        let current = settings_window
            .read_with(app, |settings, _cx| settings.diff_scroll_sync)
            .expect("settings window should be readable");
        match current {
            DiffScrollSync::Both => DiffScrollSync::Vertical,
            DiffScrollSync::Vertical => DiffScrollSync::Horizontal,
            DiffScrollSync::Horizontal => DiffScrollSync::None,
            DiffScrollSync::None => DiffScrollSync::Both,
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_diff_scroll_sync(next_mode, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "diff scroll sync update should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::diff_scroll_sync(main_view.read(app)),
            next_mode
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.diff_scroll_sync)
                .expect("settings window should remain readable"),
            next_mode
        );
    });
}

#[gpui::test]
fn diff_content_mode_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    let next_mode = cx.update(|_window, app| {
        let current = settings_window
            .read_with(app, |settings, _cx| settings.diff_content_mode)
            .expect("settings window should be readable");
        match current {
            DiffContentMode::Full => DiffContentMode::Collapsed,
            DiffContentMode::Collapsed => DiffContentMode::Full,
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_diff_content_mode(next_mode, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "diff content mode update should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::diff_content_mode(main_view.read(app)),
            next_mode
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.diff_content_mode)
                .expect("settings window should remain readable"),
            next_mode
        );
    });
}

#[gpui::test]
fn diff_whitespace_mode_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    let next_mode = cx.update(|_window, app| {
        let current = settings_window
            .read_with(app, |settings, _cx| settings.diff_whitespace_mode)
            .expect("settings window should be readable");
        current.toggled()
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_diff_whitespace_mode(next_mode, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "diff whitespace mode update should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            crate::view::test_support::diff_whitespace_mode(main_view.read(app)),
            next_mode
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| settings.diff_whitespace_mode)
                .expect("settings window should remain readable"),
            next_mode
        );
    });
}

#[gpui::test]
fn auto_save_file_edits_toggle_reaches_the_main_window(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    cx.update(|_window, app| {
        assert!(
            !main_view
                .read(app)
                .diff_renderer
                .read(app)
                .auto_save_file_edits,
            "auto-save is off until it is turned on"
        );
    });

    // Nested inside a `GitCometView` update, as the deferral regression
    // tests do: the settings window must not re-enter the main view.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_auto_save_file_edits(true, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "the auto-save toggle should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(
            main_view
                .read(app)
                .diff_renderer
                .read(app)
                .auto_save_file_edits,
            "the pane that owns the editor must see the new value"
        );
        assert!(
            settings_window
                .read_with(app, |settings, _cx| settings.auto_save_file_edits)
                .expect("settings window should remain readable")
        );
    });
}

/// Every layout and sort is offered, under the label the lists' own menus use.
#[test]
fn changed_file_list_options_cover_every_layout_and_sort() {
    assert_eq!(
        FILE_LIST_LAYOUT_OPTIONS
            .iter()
            .map(|(_, layout, _)| *layout)
            .collect::<Vec<_>>(),
        FileListLayout::ALL.to_vec()
    );
    assert_eq!(
        FILE_LIST_SORT_OPTIONS
            .iter()
            .map(|(_, sort, _)| *sort)
            .collect::<Vec<_>>(),
        crate::view::rows::CommitFileSort::ALL.to_vec()
    );
    for sort in crate::view::rows::CommitFileSort::ALL {
        assert_eq!(
            crate::view::rows::CommitFileSort::from_key(sort.key()),
            Some(sort)
        );
    }
}

/// The defaults reach every list: the details pane, its sections (Untracked
/// keeps path order under a sort it cannot offer), and the global a list an
/// extension opens reads.
#[gpui::test]
fn changed_file_list_defaults_reach_the_main_window(cx: &mut gpui::TestAppContext) {
    use crate::view::rows::CommitFileSort;
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    cx.update(|_window, app| {
        main_view.update(app, |view, cx| {
            view.details_pane.update(cx, |pane, cx| {
                pane.set_status_file_sort(
                    StatusSection::Unstaged,
                    CommitFileSort::PathDescending,
                    cx,
                )
            });
        });
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.set_file_list_layout(FileListLayout::Groups, cx);
            settings.set_file_list_sort(CommitFileSort::Edits, cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let pane = main_view.read(app).details_pane.read(app);
        assert_eq!(pane.file_list_layout, FileListLayout::Groups);
        assert_eq!(pane.commit_file_sort, CommitFileSort::Edits);
        assert_eq!(
            pane.status_file_sort_for(StatusSection::Unstaged),
            CommitFileSort::Edits,
            "the new default replaces a sort chosen by hand"
        );
        assert_eq!(
            pane.status_file_sort_for(StatusSection::Untracked),
            CommitFileSort::PathAscending
        );
        assert_eq!(
            crate::view::FileListDefaults::current(app),
            crate::view::FileListDefaults {
                layout: FileListLayout::Groups,
                sort: CommitFileSort::Edits,
            }
        );
    });
}

#[gpui::test]
fn remote_prune_toggle_reaches_the_global_store_setting(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));

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

    assert!(
        store
            .snapshot()
            .remote_settings
            .prune_deleted_remote_branches_on_fetch,
        "remote pruning should default to enabled"
    );

    cx.update(|_window, app| {
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.set_prune_deleted_remote_branches_on_fetch(false, cx);
        });
    });
    wait_for_store(
        cx,
        &store,
        "the Remotes setting to reach the store",
        |state| !state.remote_settings.prune_deleted_remote_branches_on_fetch,
    );
}

#[gpui::test]
fn maintenance_toggle_reaches_the_store_and_withdraws_cards(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let mut seeded = (*store.snapshot()).clone();
    let mut repo = gitcomet_state::model::RepoState::new_opening(
        gitcomet_state::model::RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/tmp/maintenance-toggle"),
        },
    );
    repo.maintenance.recommended = true;
    seeded.repos.push(repo);
    store.replace_snapshot_for_test(std::sync::Arc::new(seeded));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));

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
    wait_for_store(
        cx,
        &store,
        "the default setting to reach the store",
        |state| state.maintenance_settings.recommend && state.repos[0].maintenance.recommended,
    );

    cx.update(|_window, app| {
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.set_recommend_repo_maintenance(false, cx);
        });
    });
    wait_for_store(
        cx,
        &store,
        "the Maintenance setting to reach the store",
        |state| {
            !state.maintenance_settings.recommend
                && state.repos.iter().all(|repo| !repo.maintenance.recommended)
        },
    );
}

#[gpui::test]
fn files_follow_toggle_reaches_the_global_store_setting(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store.clone(), events, None, window, cx));

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

    assert!(
        store
            .snapshot()
            .file_browser_settings
            .follow_selected_commit,
        "following the selected commit should default to enabled"
    );

    cx.update(|_window, app| {
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.set_files_follow_selected_commit(false, cx);
        });
    });
    cx.run_until_parked();

    // GPUI parking only drains its executor; the store has a separate worker.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while store
        .snapshot()
        .file_browser_settings
        .follow_selected_commit
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(1));
    }

    assert!(
        !store
            .snapshot()
            .file_browser_settings
            .follow_selected_commit,
        "the Git log setting should update the global store setting"
    );
}

#[gpui::test]
fn allowed_remote_protocol_toggle_reaches_the_main_window_and_store(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let observed_store = store.clone();
    let (main_view, cx) =
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

    cx.update(|_window, app| {
        let settings_policy = settings_window
            .read_with(app, |settings, _cx| settings.remote_url_policy)
            .expect("settings window should remain readable");
        assert_eq!(settings_policy, RemoteUrlPolicy::default());
        assert!(!settings_policy.allows(RemoteProtocol::Http));
        assert_eq!(main_view.read(app).remote_url_policy, settings_policy);
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.toggle_remote_protocol(RemoteProtocol::Http, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "the protocol toggle should not re-enter GitCometView updates"
    );

    cx.run_until_parked();
    cx.update(|_window, app| {
        assert!(
            main_view
                .read(app)
                .remote_url_policy
                .allows(RemoteProtocol::Http)
        );
        assert!(
            settings_window
                .read_with(app, |settings, _cx| settings
                    .remote_url_policy
                    .allows(RemoteProtocol::Http))
                .expect("settings window should remain readable")
        );
    });
    wait_for_store(
        cx,
        &observed_store,
        "the protocol policy to reach the store",
        |state| state.remote_url_policy.allows(RemoteProtocol::Http),
    );
}

#[gpui::test]
fn diff_render_settings_update_main_window(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
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

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_diff_reveal_whitespace_chars(true, cx);
                    settings.set_diff_word_wrap(true, cx);
                    settings.set_diff_show_line_numbers(false, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "diff render setting updates should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert!(crate::view::test_support::diff_reveal_whitespace_chars(
            main_view.read(app)
        ));
        assert!(crate::view::test_support::diff_word_wrap(
            main_view.read(app)
        ));
        assert!(!crate::view::test_support::diff_show_line_numbers(
            main_view.read(app)
        ));
        assert!(
            settings_window
                .read_with(app, |settings, _cx| settings.diff_reveal_whitespace_chars)
                .expect("settings window should remain readable")
        );
        assert!(
            settings_window
                .read_with(app, |settings, _cx| settings.diff_word_wrap)
                .expect("settings window should remain readable")
        );
        assert!(
            !settings_window
                .read_with(app, |settings, _cx| settings.diff_show_line_numbers)
                .expect("settings window should remain readable")
        );
    });
}

#[test]
fn signature_verification_is_discoverable_in_settings_search() {
    for query in ["signature", "Verify commit signatures", "verification"] {
        assert!(
            SettingsCategory::GitLog.matches_query(query),
            "query: {query}"
        );
    }
}

#[gpui::test]
fn graph_gradient_toggle_updates_every_main_window(cx: &mut gpui::TestAppContext) {
    for query in ["graph gradient", "fade", "glow"] {
        assert!(SettingsCategory::GitLog.matches_query(query));
    }
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (first, first_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings_window = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (second, second_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    second_cx.update(|_, app| {
        for (view, opacity) in [(&first, 0.35), (&second, 0.0)] {
            view.update(app, |view, cx| {
                view.set_theme(
                    AppTheme {
                        graph_fade_opacity: opacity,
                        ..view.theme
                    },
                    cx,
                );
            });
        }
    });
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), second_cx);
    settings_cx.simulate_resize(size(px(720.0), px(1200.0)));
    settings_window
        .update(&mut settings_cx, |settings, _, cx| {
            settings.select_category(SettingsCategory::GitLog, cx);
            settings.set_history_show_graph_gradient(true, cx);
        })
        .unwrap();
    settings_cx.run_until_parked();
    for enabled in [false, true] {
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let toggle = settings_cx
            .debug_bounds("settings_window_git_log_show_graph_gradient")
            .expect("graph gradient toggle");
        settings_cx.simulate_click(toggle.center(), Modifiers::default());
        settings_cx.run_until_parked();
        settings_window
            .update(&mut settings_cx, |settings, _, _| {
                assert_eq!(settings.history_show_graph_gradient, enabled);
                assert_eq!(
                    settings.preference_settings().history_show_graph_gradient,
                    Some(enabled)
                );
            })
            .unwrap();
        settings_cx.update(|_, app| {
            for (view, opacity) in [(&first, 0.35), (&second, 0.0)] {
                let root = view.read(app);
                assert_eq!(
                    root.ui_model
                        .read(app)
                        .preferences
                        .history
                        .show_graph_gradient,
                    enabled
                );
                let history = root.main_pane.read(app).history_view.read(app);
                assert_eq!(history.history_show_graph_gradient, enabled);
                assert_eq!(history.theme.graph_fade_opacity, opacity);
                assert!(history.history_show_graph);
            }
        });
    }
}

#[gpui::test]
fn history_branch_names_options_update_every_main_window(cx: &mut gpui::TestAppContext) {
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (first, first_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings_window = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    let (store, events) = AppStore::new_test(Arc::new(TestBackend));
    let (second, second_cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    let mut settings_cx = gpui::VisualTestContext::from_window(*settings_window.deref(), second_cx);
    settings_cx.simulate_resize(size(px(720.0), px(1200.0)));
    settings_window
        .update(&mut settings_cx, |settings, _, cx| {
            assert_eq!(
                settings.history_branch_names,
                HistoryBranchNamesMode::SeparateColumn
            );
            settings.set_expanded_section(Some(SettingsSection::GitLogBranchNames), cx);
        })
        .unwrap();
    settings_cx.run_until_parked();
    for (mode, selector) in [
        (
            HistoryBranchNamesMode::Inline,
            "settings_window_git_log_branch_names_inline",
        ),
        (
            HistoryBranchNamesMode::SeparateColumn,
            "settings_window_git_log_branch_names_separate",
        ),
    ] {
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
        });
        let option = settings_cx
            .debug_bounds(selector)
            .expect("branch names option");
        settings_cx.simulate_click(option.center(), Modifiers::default());
        settings_cx.run_until_parked();
        settings_window
            .update(&mut settings_cx, |settings, _, _| {
                assert_eq!(settings.history_branch_names, mode);
                assert_eq!(
                    settings
                        .preference_settings()
                        .history_branch_names
                        .as_deref(),
                    Some(mode.key())
                );
            })
            .unwrap();
        settings_cx.update(|_, app| {
            for view in [&first, &second] {
                let root = view.read(app);
                assert_eq!(
                    root.ui_model.read(app).preferences.history.branch_names,
                    mode
                );
                let history = root.main_pane.read(app).history_view.read(app);
                assert_eq!(history.history_branch_names, mode);
                assert_eq!(
                    history.history_ref_column_width(),
                    if mode == HistoryBranchNamesMode::Inline {
                        px(0.0)
                    } else {
                        history.history_col_branch
                    }
                );
            }
        });
    }
}
