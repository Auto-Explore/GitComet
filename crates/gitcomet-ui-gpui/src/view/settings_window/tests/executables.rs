use super::*;

#[test]
fn git_reprobe_preserves_system_and_graphics_environment() {
    use gitcomet_core::environment::{GraphicsDetails, Rendering};
    let mut info = SettingsRuntimeInfo::from_runtime(GitRuntimeState {
        preference: GitExecutablePreference::SystemPath,
        availability: GitExecutableAvailability::Checking,
    });
    info.environment.system.cpu_model = Some("Recorded CPU".into());
    info.environment.graphics.insert(
        1,
        GraphicsDetails {
            device_name: Some("llvmpipe".into()),
            rendering: Rendering::Software,
            ..Default::default()
        },
    );
    let system = info.environment.system.clone();
    let graphics = info.environment.graphics.clone();
    for availability in [
        GitExecutableAvailability::Checking,
        GitExecutableAvailability::Available {
            version_output: "git version 2.51.0".into(),
        },
        GitExecutableAvailability::Unavailable {
            detail: "not found".into(),
        },
    ] {
        let runtime = GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability,
        };
        info.update_git(runtime.clone());
        assert_eq!(info.environment.system, system);
        assert_eq!(info.environment.graphics, graphics);
        assert_eq!(
            info.environment.git_version.as_deref(),
            runtime.version_output()
        );
    }
}

#[gpui::test]
fn environment_copy_matches_displayed_rows_and_refreshes_windows(cx: &mut gpui::TestAppContext) {
    let (view, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.run_until_parked();
    cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    view.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::Environment, cx)
    });
    cx.run_until_parked();
    cx.update(|window, cx| {
        view.update(cx, |settings, cx| {
            // Simulate a stale cache; copying must capture the open window.
            settings.runtime_info.environment.graphics.clear();
            settings.copy_environment_details(window, cx);
            assert_eq!(settings.runtime_info.environment.graphics.len(), 1);
            assert_eq!(
                crate::clipboard::read_text(cx),
                Some(settings.runtime_info.environment.summary())
            );
        });
        let _ = window.draw(cx);
    });
    for row in [
        "settings_window_build",
        "settings_window_git",
        "settings_window_os",
        "settings_window_kernel",
        "settings_window_architecture",
        "settings_window_cpu",
        "settings_window_processors",
        "settings_window_memory",
        "settings_window_gpu_1",
        "settings_window_backend_1",
        "settings_window_rendering_1",
    ] {
        assert!(cx.debug_bounds(row).is_some(), "missing {row} row");
    }
}

#[gpui::test]
fn environment_values_select_with_the_mouse_and_copy(cx: &mut gpui::TestAppContext) {
    let (_view, cx) = open_environment_page(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("stale".into()));

    let drag = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        let value = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected `{selector}` bounds"));
        let start = point(value.left() + px(1.0), value.center().y);
        let end = point(value.right() - px(1.0), value.center().y);
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, Some(MouseButton::Left), Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        cx.run_until_parked();
    };

    drag(cx, "settings_window_cpu_value");
    cx.simulate_keystrokes("secondary-c");
    assert_eq!(clipboard_text(cx).as_deref(), Some("Recorded CPU 9000"));

    // Rows in the graphics sections carry an index suffix.
    drag(cx, "settings_window_rendering_1_value");
    cx.simulate_keystrokes("secondary-c");
    let rendering = clipboard_text(cx).expect("copied rendering value");
    assert!(
        !rendering.is_empty() && rendering != "Recorded CPU 9000",
        "{rendering:?}"
    );
}

#[gpui::test]
fn environment_copy_button_sits_compact_in_the_card_header(cx: &mut gpui::TestAppContext) {
    let (view, cx) = open_environment_page(cx);
    cx.write_to_clipboard(gpui::ClipboardItem::new_string("stale".into()));

    let bounds = |cx: &mut gpui::VisualTestContext, selector: &'static str| {
        cx.debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected `{selector}` bounds"))
    };
    let card = bounds(cx, "settings_window_environment");
    let button = bounds(cx, "settings_window_copy_environment");
    let first_row = bounds(cx, "settings_window_build");
    assert!(
        button.size.width < card.size.width / 4.0,
        "button spans the card: button={button:?}, card={card:?}"
    );
    assert!(
        button.right() <= card.right() && card.right() - button.right() < px(16.0),
        "button is not at the trailing edge: button={button:?}, card={card:?}"
    );
    assert!(
        button.top() - card.top() < px(8.0) && button.bottom() <= first_row.top(),
        "button is not in the header: button={button:?}, card={card:?}, row={first_row:?}"
    );

    cx.simulate_mouse_move(button.center(), None, Modifiers::default());
    cx.simulate_click(button.center(), Modifiers::default());
    cx.run_until_parked();
    let summary = view.update(cx, |settings, _| {
        settings.runtime_info.environment.summary()
    });
    assert_eq!(clipboard_text(cx), Some(summary));
}

#[test]
fn git_executable_mode_tracks_runtime_preference() {
    assert_eq!(
        GitExecutableMode::from_preference(&GitExecutablePreference::SystemPath),
        GitExecutableMode::SystemPath
    );
    assert_eq!(
        GitExecutableMode::from_preference(&GitExecutablePreference::Custom(PathBuf::from(
            "/opt/git/bin/git"
        ),)),
        GitExecutableMode::Custom
    );
}

#[test]
fn git_runtime_info_from_state_surfaces_unavailable_detail() {
    let runtime = GitRuntimeState {
            preference: GitExecutablePreference::Custom(PathBuf::new()),
            availability: GitExecutableAvailability::Unavailable {
                detail: "Custom Git executable is not configured. Choose an executable or switch back to System PATH.".to_string(),
            },
        };

    let info = git_runtime_info_from_state(runtime.clone());
    assert_eq!(info.runtime, runtime);
    assert_eq!(info.compatibility, GitCompatibility::Unavailable);
    assert_eq!(info.version_display.as_ref(), "Unavailable");
    assert_eq!(
        info.detail.as_ref().map(|detail| detail.as_ref()),
        Some(
            "Custom Git executable is not configured. Choose an executable or switch back to System PATH."
        )
    );
}

#[test]
fn applied_git_executable_path_tracks_runtime_preference() {
    assert_eq!(
        applied_git_executable_path(&GitRuntimeState {
            preference: GitExecutablePreference::SystemPath,
            availability: GitExecutableAvailability::Available {
                version_output: "git version 2.51.0".to_string(),
            },
        }),
        None
    );
    assert_eq!(
        applied_git_executable_path(&GitRuntimeState {
            preference: GitExecutablePreference::Custom(PathBuf::from("/opt/git/bin/git")),
            availability: GitExecutableAvailability::Available {
                version_output: "git version 2.51.0".to_string(),
            },
        }),
        Some(PathBuf::from("/opt/git/bin/git"))
    );
    assert_eq!(
        applied_git_executable_path(&GitRuntimeState {
            preference: GitExecutablePreference::Custom(PathBuf::new()),
            availability: GitExecutableAvailability::Unavailable {
                detail: "missing".to_string(),
            },
        }),
        Some(PathBuf::new())
    );
}

#[test]
fn git_executable_scope_note_mentions_browser_only_scope() {
    let note = git_executable_scope_note();
    assert!(
        note.contains("browser window"),
        "expected browser-only scope note, got: {note}"
    );
    assert!(
        note.contains("System PATH"),
        "expected command-mode fallback note, got: {note}"
    );
}

#[gpui::test]
fn custom_git_executable_mode_renders_detail_container(cx: &mut gpui::TestAppContext) {
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
        settings.select_category(SettingsCategory::GitExecutable, cx);
        settings.git_executable_mode = GitExecutableMode::Custom;
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_git_executable_custom_container")
            .is_some(),
        "expected custom git executable mode to render its detail container"
    );
}

#[gpui::test]
fn custom_external_editor_renders_detail_container(cx: &mut gpui::TestAppContext) {
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
    assert!(
        settings_cx
            .debug_bounds("settings_window_external_code_editor_custom_container")
            .is_none(),
        "expected external editor custom details to stay hidden for the default None setting"
    );

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.external_editor_setting = Some(ExternalCodeEditorSetting::Custom {
            executable: PathBuf::new(),
            arguments: None,
        });
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_external_code_editor_custom_container")
            .is_some(),
        "expected custom external editor mode to render its detail container"
    );
}

#[gpui::test]
fn external_editor_detection_waits_for_the_row_to_expand(cx: &mut gpui::TestAppContext) {
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

    // Opening the window must not pay for the installed-editor scan: the list
    // holds only the fixed entries (and the saved editor, if any) until the row
    // is expanded.
    let _ = settings_window.update(&mut settings_cx, |settings, _window, _cx| {
        assert!(settings.external_editor_options_loading());
        assert!(
            settings
                .external_editor_options
                .iter()
                .all(|option| !matches!(
                    option.kind,
                    crate::external_editor::ExternalEditorOptionKind::Detected(_)
                )),
            "no detected editors before the row expands: {:?}",
            settings.external_editor_options
        );
    });

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.toggle_section(SettingsSection::ExternalCodeEditor, cx);
    });
    settings_cx.run_until_parked();

    let _ = settings_window.update(&mut settings_cx, |settings, _window, _cx| {
        assert!(!settings.external_editor_options_loading());
        let expected = crate::external_editor::external_editor_options_from_detected(
            settings.external_editor_setting.as_ref(),
            crate::external_editor::detect_external_editors(),
        );
        assert_eq!(
            settings.external_editor_options.as_ref(),
            expected.as_slice()
        );
    });
}

#[gpui::test]
fn browsed_external_editor_path_updates_custom_setting_and_notifies(cx: &mut gpui::TestAppContext) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
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

    let editor_path = PathBuf::from("/tmp/gitcomet-custom-editor");
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.apply_browsed_external_editor_path(editor_path.clone(), cx);

        assert_eq!(
            settings.external_editor_setting,
            Some(ExternalCodeEditorSetting::Custom {
                executable: editor_path.clone(),
                arguments: None,
            })
        );
        assert_eq!(
            settings.external_editor_custom_path_draft,
            editor_path.display().to_string()
        );
        assert_eq!(
            settings
                .external_editor_custom_path_input
                .read(cx)
                .text()
                .to_string(),
            editor_path.display().to_string()
        );
        assert_eq!(settings.external_editor_browse_notify_count, 1);
    });
}

#[test]
fn custom_external_editor_browse_prompt_allows_app_bundle_directories() {
    let options = custom_external_editor_path_prompt_options(None);

    assert!(
        options.files,
        "custom external editor browsing should still allow executable files"
    );
    assert!(
        options.directories,
        "custom external editor browsing should allow macOS .app bundle directories"
    );
    assert!(
        !options.multiple,
        "custom external editor browsing should remain a single-selection prompt"
    );
    assert_eq!(
        options.prompt.as_ref().map(ToString::to_string),
        Some("Select external code editor".to_string())
    );
}

#[gpui::test]
fn external_editor_setting_seeds_from_pending_override_and_can_clear(
    cx: &mut gpui::TestAppContext,
) {
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    let pending_setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/tmp/gitcomet-pending-editor"),
        arguments: Some("--reuse-window {path}".to_string()),
    };
    crate::external_editor::set_configured_setting_override(Some(pending_setting.clone()));

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

    cx.update(|_window, app| {
            let _ = settings_window.update(app, |settings, _window, cx| {
                assert_eq!(
                    settings.external_editor_setting,
                    Some(pending_setting.clone()),
                    "settings should use the pending in-memory editor preference before session persistence finishes"
                );

                settings.set_external_editor_setting(None, cx);

                assert_eq!(settings.external_editor_setting, None);
                assert_eq!(
                    crate::external_editor::configured_setting_preference_override(),
                    Some(None),
                    "clearing the reopened settings window should replace the pending editor preference"
                );
            });
        });
}

#[test]
fn external_editor_preference_persist_queue_skips_stale_custom_draft_writes() {
    let session_file = unique_session_file("external-editor-draft-sequence");
    let queue = PreferenceWriter::default();
    let stale_setting = Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/tmp/editor"),
        arguments: Some("--reuse".to_string()),
    });
    let latest_setting = Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/tmp/editor-final"),
        arguments: Some("--reuse-window {path}".to_string()),
    });

    let stale_sequence = queue.next();
    let latest_sequence = queue.next();

    assert!(
        queue
            .persist(latest_sequence, || session::persist_ui_settings_to_path(
                external_editor_preference_settings(latest_setting.clone()),
                &session_file
            ))
            .expect("persist latest custom editor draft")
    );
    assert!(
        !queue
            .persist(stale_sequence, || session::persist_ui_settings_to_path(
                external_editor_preference_settings(stale_setting),
                &session_file
            ))
            .expect("skip stale custom editor draft")
    );

    let loaded = gitcomet_state::session::load_from_path(&session_file);
    assert_eq!(loaded.external_code_editor, latest_setting);
}

#[gpui::test]
fn generic_preference_persistence_omits_external_editor_snapshot(cx: &mut gpui::TestAppContext) {
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

    cx.update(|_window, app| {
        let _ = settings_window.update(app, |settings, _window, _cx| {
            settings.external_editor_setting = Some(ExternalCodeEditorSetting::Custom {
                executable: PathBuf::from("/tmp/editor-before-theme-change"),
                arguments: Some("--reuse-window {path}".to_string()),
            });
            let persisted = settings.preference_settings();
            assert_eq!(persisted.external_code_editor, None);
        });
    });
}

#[test]
fn a_missing_signing_tool_explains_what_is_lost_and_how_to_fix_it() {
    let tools = signing_tools_fixture();
    let gpg = gpg_info(Some(&tools));

    assert_eq!(gpg.status, SigningToolStatus::NotFound);
    let detail = gpg.detail.expect("a missing gpg must explain itself");
    assert!(detail.contains("not verified"), "{detail}");
    assert!(detail.contains("gpg.program"), "{detail}");
}

#[test]
fn a_found_signing_tool_shows_its_version_and_custom_program() {
    let tools = signing_tools_fixture();
    let ssh_keygen = ssh_keygen_info(Some(&tools));

    assert_eq!(ssh_keygen.status, SigningToolStatus::Found);
    assert_eq!(ssh_keygen.version_display.as_ref(), "OpenSSH_10.3p1");
    let detail = ssh_keygen.detail.expect("a non-default program is named");
    assert!(detail.contains("gpg.ssh.program"), "{detail}");
}

#[test]
fn signing_tools_show_as_detecting_until_probed() {
    assert_eq!(gpg_info(None).status, SigningToolStatus::Detecting);
    assert_eq!(ssh_keygen_info(None).status, SigningToolStatus::Detecting);
}

#[test]
fn unrequested_signing_tools_are_not_displayed_as_a_running_probe() {
    let tools = gitcomet_core::signing_tools::SigningToolsState::default();
    assert_eq!(gpg_info(Some(&tools)).status, SigningToolStatus::NotChecked);
    assert_eq!(
        ssh_keygen_info(Some(&tools)).status,
        SigningToolStatus::NotChecked
    );
}

#[test]
fn pending_git_discovery_is_displayed_without_an_unavailable_error() {
    let info = git_runtime_info_from_state(GitRuntimeState {
        preference: GitExecutablePreference::SystemPath,
        availability: gitcomet_core::process::GitExecutableAvailability::Checking,
    });
    assert_eq!(info.compatibility, GitCompatibility::Checking);
    assert_eq!(info.version_display.as_ref(), "Checking...");
    assert!(info.detail.is_none());
}

#[test]
fn the_executables_page_is_found_by_its_tools() {
    for query in ["executables", "git executable", "gpg", "ssh-keygen"] {
        assert!(
            SettingsCategory::GitExecutable.matches_query(query),
            "{query}"
        );
    }
}

#[gpui::test]
fn the_executables_page_links_to_the_signature_guide(cx: &mut gpui::TestAppContext) {
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
        settings.select_category(SettingsCategory::GitExecutable, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let guide_bounds = settings_cx
        .debug_bounds("settings_window_signature_guide")
        .expect("expected signature guide row bounds");
    settings_cx.simulate_click(guide_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();

    assert_eq!(cx.opened_url(), signature_guide_url());
}

/// "Recheck" re-probes `git lfs` / `git annex` from this window. The main
/// windows only probe once per Git runtime, so without the result they keep
/// offering "(install git-lfs)" after the user installed it.
#[gpui::test]
fn large_file_tools_recheck_reaches_the_main_windows(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::large_file_tools::{LargeFileToolsState, ToolAvailability};
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let main_store = store.clone();
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    main_store.dispatch(Msg::SetLargeFileToolsState(LargeFileToolsState {
        git_lfs: ToolAvailability::NotFound {
            detail: "Git cannot run `git lfs`.".into(),
        },
        git_annex: ToolAvailability::Unknown,
    }));
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

    let installed = LargeFileToolsState {
        git_lfs: ToolAvailability::Available {
            version: Some("git-lfs/3.8.0".into()),
        },
        git_annex: ToolAvailability::Unknown,
    };
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.apply_large_file_tools_probe(installed.clone(), cx);
    });
    wait_for_store(
        &mut settings_cx,
        &main_store,
        "the main window to learn git-lfs is installed",
        |state| state.large_file_tools == installed,
    );
}

#[test]
fn large_file_tool_rows_report_found_missing_and_detecting() {
    use gitcomet_core::large_file_tools::{LargeFileToolsState, ToolAvailability};
    let tools = LargeFileToolsState {
        git_lfs: ToolAvailability::Available {
            version: Some("git-lfs/3.8.0 (GitHub; linux amd64)".into()),
        },
        git_annex: ToolAvailability::NotFound {
            detail: "Git cannot run `git annex`.".into(),
        },
    };
    let lfs = git_lfs_info(Some(&tools));
    assert_eq!(lfs.status, SigningToolStatus::Found);
    assert_eq!(
        lfs.version_display.as_ref(),
        "git-lfs/3.8.0 (GitHub; linux amd64)"
    );
    let annex = git_annex_info(Some(&tools));
    assert_eq!(annex.status, SigningToolStatus::NotFound);
    assert!(
        annex
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("Install git-annex where Git can find it")),
        "{:?}",
        annex.detail
    );
    assert_eq!(git_lfs_info(None).status, SigningToolStatus::Detecting);
}

fn open_environment_page(
    cx: &mut gpui::TestAppContext,
) -> (Entity<SettingsWindowView>, &mut gpui::VisualTestContext) {
    let (view, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.update(|_, app| crate::app::bind_text_input_keys_for_test(app));
    cx.run_until_parked();
    cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    cx.update(|_, app| {
        let mut environment = app.global::<crate::environment::Environment>().0.clone();
        environment.system.cpu_model = Some("Recorded CPU 9000".into());
        app.set_global(crate::environment::Environment(environment));
    });
    view.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::Environment, cx)
    });
    cx.run_until_parked();
    cx.update(|window, app| {
        let _ = window.draw(app);
    });
    (view, cx)
}

fn clipboard_text(cx: &mut gpui::VisualTestContext) -> Option<String> {
    cx.read_from_clipboard().and_then(|item| item.text())
}

fn signing_tools_fixture() -> gitcomet_core::signing_tools::SigningToolsState {
    use gitcomet_core::signing_tools::{SigningTool, SigningToolAvailability, SigningToolsState};
    SigningToolsState {
        gpg: SigningTool {
            program: "gpg".to_string(),
            availability: SigningToolAvailability::NotFound {
                detail: "`gpg` was not found on Git's PATH.".to_string(),
            },
        },
        ssh_keygen: SigningTool {
            program: "/usr/bin/ssh-keygen".to_string(),
            availability: SigningToolAvailability::Available {
                version: Some("OpenSSH_10.3p1".to_string()),
            },
        },
    }
}

#[gpui::test]
fn git_executable_browse_opens_beside_the_typed_executable(cx: &mut gpui::TestAppContext) {
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(1200.0)));
    settings_cx.run_until_parked();

    let dir = tempfile::tempdir().expect("temp dir");
    let executable = dir.path().join("git");
    std::fs::write(&executable, "").expect("executable");
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::GitExecutable, cx);
        settings.git_executable_mode = GitExecutableMode::Custom;
        settings.git_executable_input.update(cx, |input, cx| {
            input.set_text(executable.display().to_string(), cx)
        });
        cx.notify();
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let center = settings_cx
        .debug_bounds("settings_window_git_executable_browse")
        .expect("Browse is drawn")
        .center();
    settings_cx.simulate_mouse_down(center, MouseButton::Left, gpui::Modifiers::default());
    settings_cx.simulate_mouse_up(center, MouseButton::Left, gpui::Modifiers::default());
    settings_cx.run_until_parked();

    assert!(
        settings_cx.did_prompt_for_paths(),
        "Browse opens a file dialog"
    );
    let mut directory = None;
    settings_cx.simulate_path_prompt_response(|options| {
        directory = options.directory.clone();
        None
    });
    assert_eq!(directory, Some(dir.path().to_path_buf()));
}
