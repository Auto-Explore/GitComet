use super::*;

#[gpui::test]
fn terminal_settings_sections_toggle_and_render_controls(cx: &mut gpui::TestAppContext) {
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
        settings.select_category(SettingsCategory::Terminal, cx);
    });
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_terminal_button_embedded")
            .is_none(),
        "expected terminal button options to stay collapsed until opened"
    );

    let action_bar_bounds = settings_cx
        .debug_bounds("settings_window_terminal_button")
        .expect("expected action bar terminal row bounds");
    settings_cx.simulate_click(action_bar_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for selector in [
        "settings_window_terminal_button_embedded",
        "settings_window_terminal_button_external",
    ] {
        assert!(
            settings_cx.debug_bounds(selector).is_some(),
            "expected `{selector}` when the action bar terminal section is expanded"
        );
    }

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.toggle_section(SettingsSection::TerminalButton, cx);
    });
    settings_cx.run_until_parked();
    assert!(
        settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                settings.expanded_section
            })
            .expect("settings window should remain readable")
            != Some(SettingsSection::TerminalButton),
        "expected action bar terminal section state to collapse when toggled again"
    );

    let external_bounds = settings_cx
        .debug_bounds("settings_window_terminal_external")
        .expect("expected external terminal row bounds");
    settings_cx.simulate_click(external_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for selector in [
        "settings_window_terminal_external_default",
        "settings_window_terminal_external_custom",
    ] {
        assert!(
            settings_cx.debug_bounds(selector).is_some(),
            "expected `{selector}` when the external terminal section is expanded"
        );
    }

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.toggle_section(SettingsSection::TerminalExternal, cx);
    });
    settings_cx.run_until_parked();
    assert!(
        settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                settings.expanded_section
            })
            .expect("settings window should remain readable")
            != Some(SettingsSection::TerminalExternal),
        "expected external terminal section state to collapse when toggled again"
    );
}

#[gpui::test]
fn terminal_button_target_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
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

    let next_target = cx.update(|_window, app| {
        let current = settings_window
            .read_with(app, |settings, _cx| {
                settings.terminal_preferences.terminal_button_target
            })
            .expect("settings window should be readable");
        match current {
            TerminalButtonTarget::Embedded => TerminalButtonTarget::External,
            TerminalButtonTarget::External => TerminalButtonTarget::Embedded,
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_terminal_button_target(next_target, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "action bar terminal target updates should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            main_view
                .read(app)
                .terminal_preferences_for_test()
                .terminal_button_target,
            next_target
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| {
                    settings.terminal_preferences.terminal_button_target
                })
                .expect("settings window should remain readable"),
            next_target
        );
    });
}

#[gpui::test]
fn external_terminal_mode_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
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
            .read_with(app, |settings, _cx| {
                settings.terminal_preferences.external_terminal_mode
            })
            .expect("settings window should be readable");
        match current {
            ExternalTerminalMode::SystemDefault => ExternalTerminalMode::CustomProgram,
            ExternalTerminalMode::CustomProgram => ExternalTerminalMode::SystemDefault,
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_external_terminal_mode(next_mode, cx);
                });
            });
        });
    }));
    assert!(
        result.is_ok(),
        "external terminal mode updates should not re-enter GitCometView updates"
    );

    cx.run_until_parked();

    cx.update(|_window, app| {
        assert_eq!(
            main_view
                .read(app)
                .terminal_preferences_for_test()
                .external_terminal_mode,
            next_mode
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| {
                    settings.terminal_preferences.external_terminal_mode
                })
                .expect("settings window should remain readable"),
            next_mode
        );
    });
}

#[gpui::test]
fn terminal_external_draft_save_trims_multiline_args_before_persistence(
    cx: &mut gpui::TestAppContext,
) {
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
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.set_external_terminal_mode(ExternalTerminalMode::CustomProgram, cx);
            settings
                .terminal_external_program_input
                .update(cx, |input, cx| input.set_text("  wezterm  ", cx));
            settings
                .terminal_external_args_input
                .update(cx, |input, cx| {
                    input.set_text("  start  \n\n  --cwd  \n  {cwd}  \n", cx);
                });
            settings.save_terminal_external_draft(cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let root_preferences = main_view.read(app).terminal_preferences_for_test().clone();
        assert_eq!(
            root_preferences.external_terminal_mode,
            ExternalTerminalMode::CustomProgram
        );
        assert_eq!(root_preferences.external_terminal_program, "wezterm");
        assert_eq!(
            root_preferences.external_terminal_args,
            vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
            ]
        );

        let (program, args, program_input, args_input, status) = settings_window
            .read_with(app, |settings, cx| {
                (
                    settings
                        .terminal_preferences
                        .external_terminal_program
                        .clone(),
                    settings.terminal_preferences.external_terminal_args.clone(),
                    settings
                        .terminal_external_program_input
                        .read_with(cx, |input, _| input.text().to_string()),
                    settings
                        .terminal_external_args_input
                        .read_with(cx, |input, _| input.text().to_string()),
                    settings
                        .terminal_status
                        .as_ref()
                        .map(|status| status.text.to_string()),
                )
            })
            .expect("settings window should remain readable");

        assert_eq!(program, "wezterm");
        assert_eq!(
            args,
            vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
            ]
        );
        assert_eq!(program_input, "  wezterm  ");
        assert_eq!(args_input, "  start  \n\n  --cwd  \n  {cwd}  \n");
        assert_eq!(status.as_deref(), Some("External terminal settings saved."));
    });
}

#[gpui::test]
fn terminal_external_draft_save_and_reset(cx: &mut gpui::TestAppContext) {
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
        let _ = settings_window.update(app, |settings, _window, cx| {
            settings.set_external_terminal_mode(ExternalTerminalMode::CustomProgram, cx);
            settings
                .terminal_external_program_input
                .update(cx, |input, cx| input.set_text("wezterm", cx));
            settings
                .terminal_external_args_input
                .update(cx, |input, cx| {
                    input.set_text("start\n--cwd\n{cwd}", cx);
                });
            settings.save_terminal_external_draft(cx);

            settings
                .terminal_external_program_input
                .update(cx, |input, cx| input.set_text("kitty", cx));
            settings
                .terminal_external_args_input
                .update(cx, |input, cx| {
                    input.set_text("--directory\n/tmp", cx);
                });
            settings.reset_terminal_external_draft(cx);
        });
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let root_preferences = main_view.read(app).terminal_preferences_for_test().clone();
        assert_eq!(
            root_preferences.external_terminal_mode,
            ExternalTerminalMode::CustomProgram
        );
        assert_eq!(root_preferences.external_terminal_program, "wezterm");
        assert_eq!(
            root_preferences.external_terminal_args,
            vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
            ]
        );

        let (external_program, external_args, external_program_input, external_args_input, status) =
            settings_window
                .read_with(app, |settings, cx| {
                    (
                        settings
                            .terminal_preferences
                            .external_terminal_program
                            .clone(),
                        settings.terminal_preferences.external_terminal_args.clone(),
                        settings
                            .terminal_external_program_input
                            .read_with(cx, |input, _| input.text().to_string()),
                        settings
                            .terminal_external_args_input
                            .read_with(cx, |input, _| input.text().to_string()),
                        settings
                            .terminal_status
                            .as_ref()
                            .map(|status| status.text.to_string()),
                    )
                })
                .expect("settings window should remain readable");

        assert_eq!(external_program, "wezterm");
        assert_eq!(
            external_args,
            vec![
                "start".to_string(),
                "--cwd".to_string(),
                "{cwd}".to_string(),
            ]
        );
        assert_eq!(external_program_input, "wezterm");
        assert_eq!(external_args_input, "start\n--cwd\n{cwd}");
        assert_eq!(status.as_deref(), Some("External terminal draft reset."));
    });
}
