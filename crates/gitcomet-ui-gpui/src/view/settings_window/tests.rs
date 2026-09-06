use super::*;
use crate::test_support::lock_visual_test;
use crate::view::test_support::TestBackend;
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";
const DIFF_DEFAULTS_SESSION_SUBTEST_ENV: &str = "GITCOMET_DIFF_DEFAULTS_SESSION_SUBTEST";

fn wait_for_store_setting(description: &str, ready: impl Fn() -> bool) {
    // Draining GPUI's executor does not synchronize with AppStore's worker thread.
    let deadline = Instant::now() + Duration::from_secs(3);
    while !ready() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {description}"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn unique_session_file(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "gitcomet-settings-window-{label}-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("create settings session temp dir");
    dir.join("session.json")
}

fn run_subtest_with_session_env(filter: &str, session_file: &Path) {
    let current_exe = std::env::current_exe().expect("locate current test binary");
    let output = Command::new(current_exe)
        .arg(filter)
        .arg("--nocapture")
        .env(SESSION_FILE_ENV, session_file)
        .env(DIFF_DEFAULTS_SESSION_SUBTEST_ENV, "1")
        .output()
        .expect("spawn settings subtest process");
    assert!(
        output.status.success(),
        "subtest {filter} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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

#[test]
fn parse_git_version_extracts_first_version_token() {
    assert_eq!(
        parse_git_version("git version 2.50.7"),
        Some(GitVersion {
            major: 2,
            minor: 50
        })
    );
}

#[test]
fn parse_git_version_token_accepts_numeric_prefixes_and_rejects_non_numeric_prefixes() {
    assert_eq!(
        parse_git_version_token("2.45.1.windows.1"),
        Some(GitVersion {
            major: 2,
            minor: 45
        })
    );
    assert_eq!(parse_git_version_token("v2.45.1"), None);
    assert_eq!(parse_u32_prefix("53rc1"), Some(53));
    assert_eq!(parse_u32_prefix("rc53"), None);
}

#[test]
fn supported_version_requires_minimum_2_50() {
    assert!(is_supported_git_version(GitVersion {
        major: MIN_GIT_MAJOR,
        minor: MIN_GIT_MINOR,
    }));
    assert!(is_supported_git_version(GitVersion {
        major: MIN_GIT_MAJOR,
        minor: MIN_GIT_MINOR + 1,
    }));
    assert!(!is_supported_git_version(GitVersion {
        major: MIN_GIT_MAJOR,
        minor: MIN_GIT_MINOR - 1,
    }));
    assert!(is_supported_git_version(GitVersion {
        major: MIN_GIT_MAJOR + 1,
        minor: 0,
    }));
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
        Some(SETTINGS_WINDOW_TITLE.to_string()),
        "settings window titlebar should keep the OS-visible title"
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
            chrome::CLIENT_SIDE_DECORATION_INSET
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
        options.is_movable,
        "settings window should remain movable with custom chrome"
    );
    assert!(
        options.is_resizable,
        "settings window should remain resizable with custom chrome"
    );
}

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

#[test]
fn settings_theme_modes_include_automatic_and_all_available_named_themes() {
    let modes = settings_theme_modes();
    assert_eq!(modes.first(), Some(&ThemeMode::Automatic));

    let named_modes = modes.iter().skip(1).map(ThemeMode::key).collect::<Vec<_>>();
    let available_themes = crate::theme::available_themes()
        .into_iter()
        .map(|theme| theme.key.to_string())
        .collect::<Vec<_>>();

    assert_eq!(
        named_modes,
        available_themes
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
}

#[gpui::test]
fn settings_window_sets_platform_title(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
        Some(SETTINGS_WINDOW_TITLE),
        "expected settings window to expose the native OS title"
    );
}

#[gpui::test]
fn expanded_settings_sections_render_scrollable_list_containers(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
            SettingsSection::Theme,
            "settings_window_theme_list_container",
        ),
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
fn expanded_diff_content_mode_section_renders_before_scroll_sync_row(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
fn expanded_theme_section_renders_theme_utilities_and_opens_theme_guide(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
        settings.set_expanded_section(Some(SettingsSection::Theme), cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_links_container")
            .is_some(),
        "expected the expanded theme section to render theme utility links"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_custom_folder")
            .is_some(),
        "expected the expanded theme section to render the custom folder action"
    );

    let guide_bounds = settings_cx
        .debug_bounds("settings_window_theme_guide")
        .expect("expected theme guide row bounds");
    settings_cx.simulate_click(guide_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();

    assert_eq!(cx.opened_url(), Some(THEMES_GUIDE_URL.to_string()));
}

#[gpui::test]
fn expanded_history_columns_section_renders_detail_container(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
fn custom_git_executable_mode_renders_detail_container(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let options = custom_external_editor_path_prompt_options();

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
    let _visual_guard = lock_visual_test();
    let _external_editor_guard = crate::external_editor::configured_setting_override_test_guard();
    let pending_setting = ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/tmp/gitcomet-pending-editor"),
        arguments: Some("--reuse-window {path}".to_string()),
    };
    crate::external_editor::set_configured_setting_override(Some(pending_setting.clone()));

    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let queue = ExternalEditorPreferencePersistQueue::default();
    let stale_setting = Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/tmp/editor"),
        arguments: Some("--reuse".to_string()),
    });
    let latest_setting = Some(ExternalCodeEditorSetting::Custom {
        executable: PathBuf::from("/tmp/editor-final"),
        arguments: Some("--reuse-window {path}".to_string()),
    });

    let stale_sequence = queue.next_sequence();
    let latest_sequence = queue.next_sequence();

    assert!(
        queue
            .persist_to_path_if_latest(latest_sequence, latest_setting.clone(), &session_file)
            .expect("persist latest custom editor draft")
    );
    assert!(
        !queue
            .persist_to_path_if_latest(stale_sequence, stale_setting, &session_file)
            .expect("skip stale custom editor draft")
    );

    let loaded = gitcomet_state::session::load_from_path(&session_file);
    assert_eq!(loaded.external_code_editor, latest_setting);
}

#[gpui::test]
fn generic_preference_persistence_omits_external_editor_snapshot(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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

#[gpui::test]
fn settings_dropdowns_fit_without_inner_scroll(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
        (SettingsSection::Theme, "Theme"),
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
                SettingsSection::Theme => {
                    uniform_list_vertical_scroll_metrics(&settings.theme_scroll).2
                }
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
fn settings_window_open_source_licenses_row_switches_content(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
        Some(SETTINGS_WINDOW_TITLE),
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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

    assert_eq!(cx.opened_url(), Some(EDITIONS_URL.to_string()));
}

#[gpui::test]
fn settings_window_links_card_includes_theme_guide_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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

    assert!(
        settings_cx
            .debug_bounds("settings_window_links_theme_guide")
            .is_some(),
        "expected the Links card to include a Theme guide row"
    );
}

#[gpui::test]
fn settings_window_root_view_renders_visible_scrollbar(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
        settings.runtime_info.app_version_display =
            "GitComet v0.0.0-overflow-regression-build".into();
        settings.runtime_info.operating_system =
            "linux (gnu-linux-overflow-regression-platform, x86_64-extra-build-metadata)".into();
        settings.runtime_info.git.version_display =
            "git version 2.51.0 (overflow-regression-build-with-very-long-metadata)".into();
        settings.runtime_info.git.compatibility = GitCompatibility::Supported;
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
    ] {
        assert_debug_bounds_within(&mut settings_cx, row_selector, label_selector);
        assert_debug_bounds_within(&mut settings_cx, row_selector, value_selector);
    }
}

#[gpui::test]
fn settings_window_containers_fill_available_width_when_content_wraps(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
        settings.set_expanded_section(Some(SettingsSection::UiFont), cx);
        settings.git_executable_mode = GitExecutableMode::Custom;
        settings.runtime_info.app_version_display =
            "GitComet v0.0.0-overflow-regression-build-with-extra-layout-metadata".into();
        settings.runtime_info.operating_system =
            "linux (gnu-linux-overflow-regression-platform with verbose wrapping metadata, x86_64)"
                .into();
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
        (SettingsCategory::Tags, "settings_window_tags_card"),
        (
            SettingsCategory::GitExecutable,
            "settings_window_git_executable",
        ),
        (SettingsCategory::Environment, "settings_window_environment"),
        (SettingsCategory::Links, "settings_window_links"),
    ] {
        let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
            settings.select_category(category, cx);
            // The General page keeps a dropdown expanded to exercise wrapping.
            if category == SettingsCategory::General {
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

        if category == SettingsCategory::General {
            assert_debug_matching_horizontal_insets(
                &mut settings_cx,
                "settings_window_general",
                "settings_window_ui_font_list_container",
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

#[gpui::test]
fn non_macos_settings_window_renders_custom_chrome_controls(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
fn linux_settings_window_close_button_closes_only_the_settings_window(
    cx: &mut gpui::TestAppContext,
) {
    if !cfg!(any(target_os = "linux", target_os = "freebsd")) {
        return;
    }

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
fn terminal_settings_sections_toggle_and_render_controls(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
            .debug_bounds("settings_window_terminal_action_bar_embedded")
            .is_none(),
        "expected action bar terminal options to stay collapsed until opened"
    );

    let action_bar_bounds = settings_cx
        .debug_bounds("settings_window_terminal_action_bar")
        .expect("expected action bar terminal row bounds");
    settings_cx.simulate_click(action_bar_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    for selector in [
        "settings_window_terminal_action_bar_embedded",
        "settings_window_terminal_action_bar_external",
    ] {
        assert!(
            settings_cx.debug_bounds(selector).is_some(),
            "expected `{selector}` when the action bar terminal section is expanded"
        );
    }

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.toggle_section(SettingsSection::TerminalActionBar, cx);
    });
    settings_cx.run_until_parked();
    assert!(
        settings_window
            .update(&mut settings_cx, |settings, _window, _cx| {
                settings.expanded_section
            })
            .expect("settings window should remain readable")
            != Some(SettingsSection::TerminalActionBar),
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
fn action_bar_terminal_target_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
                settings.terminal_preferences.action_bar_terminal_target
            })
            .expect("settings window should be readable");
        match current {
            ActionBarTerminalTarget::Embedded => ActionBarTerminalTarget::External,
            ActionBarTerminalTarget::External => ActionBarTerminalTarget::Embedded,
        }
    });

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        cx.update(|_window, app| {
            main_view.update(app, |_view, cx| {
                let _ = settings_window.update(cx, |settings, _window, cx| {
                    settings.set_action_bar_terminal_target(next_target, cx);
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
                .action_bar_terminal_target,
            next_target
        );
        assert_eq!(
            settings_window
                .read_with(app, |settings, _cx| {
                    settings.terminal_preferences.action_bar_terminal_target
                })
                .expect("settings window should remain readable"),
            next_target
        );
    });
}

#[gpui::test]
fn diff_scroll_sync_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
            !main_view.read(app).main_pane.read(app).auto_save_file_edits,
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
            main_view.read(app).main_pane.read(app).auto_save_file_edits,
            "the pane that owns the editor must see the new value"
        );
        assert!(
            settings_window
                .read_with(app, |settings, _cx| settings.auto_save_file_edits)
                .expect("settings window should remain readable")
        );
    });
}

#[gpui::test]
fn remote_prune_toggle_reaches_the_global_store_setting(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    cx.run_until_parked();

    wait_for_store_setting(
        "the Remotes setting to update the global store setting",
        || {
            !store
                .snapshot()
                .remote_settings
                .prune_deleted_remote_branches_on_fetch
        },
    );
}

#[gpui::test]
fn allowed_remote_protocol_toggle_reaches_the_main_window_and_store(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    wait_for_store_setting(
        "the command store to receive the new protocol policy",
        || {
            observed_store
                .snapshot()
                .remote_url_policy
                .allows(RemoteProtocol::Http)
        },
    );
}

#[gpui::test]
fn diff_render_settings_update_main_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
fn diff_render_defaults_from_session_wrapper() {
    let session_file = unique_session_file("diff-defaults");
    gitcomet_state::session::persist_ui_settings_to_path(
        gitcomet_state::session::UiSettings {
            diff_reveal_whitespace_chars: Some(true),
            diff_word_wrap: Some(true),
            diff_show_line_numbers: Some(false),
            ..Default::default()
        },
        &session_file,
    )
    .expect("seed diff defaults session");

    run_subtest_with_session_env(
        "diff_render_defaults_from_session_subprocess",
        &session_file,
    );
}

#[gpui::test]
fn diff_render_defaults_from_session_subprocess(cx: &mut gpui::TestAppContext) {
    if std::env::var_os(DIFF_DEFAULTS_SESSION_SUBTEST_ENV).is_none() {
        return;
    }

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
    let (main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|_window, app| {
        let view = main_view.read(app);
        assert!(crate::view::test_support::diff_reveal_whitespace_chars(
            view
        ));
        assert!(crate::view::test_support::diff_word_wrap(view));
        assert!(!crate::view::test_support::diff_show_line_numbers(view));
        assert!(view.main_pane.read(app).reveal_whitespace_chars);
        assert!(view.main_pane.read(app).diff_word_wrap);
        assert!(!view.main_pane.read(app).diff_show_line_numbers);
    });

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

#[gpui::test]
fn external_terminal_mode_setting_defers_main_window_update(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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

#[gpui::test]
fn ui_font_dropdown_wheel_scrolls_inner_list_before_outer_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new(std::sync::Arc::new(TestBackend));
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(460.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let list_bounds = settings_cx
        .debug_bounds("settings_window_ui_font_list_container")
        .expect("expected UI font list bounds");

    let (outer_before, inner_before, outer_max, inner_max) = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            (
                absolute_scroll_y(&settings.settings_window_scroll),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
                settings.settings_window_scroll.max_offset().y.max(px(0.0)),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).2,
            )
        })
        .expect("settings window should remain readable");
    assert!(
        outer_max > px(0.0),
        "expected the settings page to be scrollable during the test"
    );
    assert!(
        inner_max > px(0.0),
        "expected the UI font list to be scrollable during the test"
    );

    settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_event(ScrollWheelEvent {
        position: list_bounds.center(),
        delta: ScrollDelta::Pixels(point(px(-120.0), px(0.0))),
        ..Default::default()
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (outer_after_horizontal_scroll, inner_after_horizontal_scroll) = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            (
                absolute_scroll_y(&settings.settings_window_scroll),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
            )
        })
        .expect("settings window should remain readable");

    assert!(
        (inner_after_horizontal_scroll - inner_before).abs() <= px(0.5),
        "expected horizontal-only wheel scroll not to move the UI font list vertically"
    );
    assert!(
        (outer_after_horizontal_scroll - outer_before).abs() <= px(0.5),
        "expected horizontal-only wheel scroll not to move the outer settings page vertically"
    );

    settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_event(ScrollWheelEvent {
        position: list_bounds.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let (outer_after_inner_scroll, inner_after_inner_scroll) = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            (
                absolute_scroll_y(&settings.settings_window_scroll),
                uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll).1,
            )
        })
        .expect("settings window should remain readable");

    assert!(
        inner_after_inner_scroll > inner_before + px(0.5),
        "expected the UI font list to consume wheel scroll first"
    );
    assert!(
        (outer_after_inner_scroll - outer_before).abs() <= px(0.5),
        "expected the outer settings page to stay still while the UI font list can still scroll"
    );

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        let (raw_offset, _scroll_offset, max_offset) =
            uniform_list_vertical_scroll_metrics(&settings.ui_font_scroll);
        let current_x = settings.ui_font_scroll.0.borrow().base_handle.offset().x;
        let target_y = if raw_offset > px(0.0) {
            max_offset
        } else {
            -max_offset
        };
        settings
            .ui_font_scroll
            .0
            .borrow()
            .base_handle
            .set_offset(point(current_x, target_y));
        cx.notify();
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let outer_before_boundary_handoff = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            absolute_scroll_y(&settings.settings_window_scroll)
        })
        .expect("settings window should remain readable");

    settings_cx.simulate_mouse_move(list_bounds.center(), None, Modifiers::default());
    settings_cx.simulate_event(ScrollWheelEvent {
        position: list_bounds.center(),
        delta: ScrollDelta::Pixels(point(px(0.0), px(-120.0))),
        ..Default::default()
    });
    settings_cx.run_until_parked();

    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });
    let outer_after_boundary_handoff = settings_window
        .update(&mut settings_cx, |settings, _window, _cx| {
            absolute_scroll_y(&settings.settings_window_scroll)
        })
        .expect("settings window should remain readable");

    assert!(
        outer_after_boundary_handoff > outer_before_boundary_handoff + px(0.5),
        "expected wheel scrolling to bubble to the outer settings page once the UI font list reaches its boundary"
    );
}
