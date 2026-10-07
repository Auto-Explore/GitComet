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
    let _visual_guard = lock_visual_test();
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

#[gpui::test]
fn environment_values_select_with_the_mouse_and_copy(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = crate::test_support::lock_clipboard_test();
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
    let _visual_guard = lock_visual_test();
    let _clipboard_guard = crate::test_support::lock_clipboard_test();
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

fn wait_for_store(
    cx: &mut gpui::VisualTestContext,
    store: &AppStore,
    description: &str,
    ready: impl Fn(&AppState) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        // GPUI can be idle while the store's separate worker is still running.
        cx.run_until_parked();
        if ready(&store.snapshot()) {
            return;
        }
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
        cfg!(target_os = "macos").then_some(chrome::macos_traffic_light_position()),
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
fn theme_orbs_paint_each_themes_chrome_accent_and_keyword() {
    use super::theme_grid::{OrbPaint, orb_svg};
    let hex = |color: gpui::Rgba| {
        format!(
            "#{:02x}{:02x}{:02x}",
            (color.red * 255.0).round() as u8,
            (color.green * 255.0).round() as u8,
            (color.blue * 255.0).round() as u8
        )
    };
    let tokyo = crate::theme::theme_preview_colors("tokyo_night").expect("bundled theme");
    let svg = orb_svg(OrbPaint::Single(tokyo));
    assert!(
        svg.contains(&format!("stop-color=\"{}\"", hex(tokyo.glow))),
        "{svg}"
    );
    assert!(
        svg.contains(&format!("stop-color=\"{}\"", hex(tokyo.secondary))),
        "{svg}"
    );
    assert!(svg.contains("radialGradient"));
    // The circle and its edge are gpui's anti-aliased corners, not the image's.
    assert!(!svg.contains("<circle"), "{svg}");

    let themes = crate::theme::ThemeCatalog::load();
    let OrbPaint::Split { light, dark } =
        OrbPaint::for_mode(&ThemeMode::Automatic, &themes).expect("automatic orb")
    else {
        panic!("Automatic paints GitComet Light and Dark halves");
    };
    assert_eq!(light.base, AppTheme::gitcomet_light().colors.surface.chrome);
    assert_eq!(dark.base, AppTheme::gitcomet_dark().colors.surface.chrome);
    let split = orb_svg(OrbPaint::Split { light, dark });
    for color in [light.glow, dark.glow] {
        assert!(split.contains(&hex(color)), "{split}");
    }
    assert!(split.contains("url(#l)") && split.contains("url(#r)"));
}

#[test]
fn theme_orbs_keep_a_translucent_accent_translucent() {
    use super::theme_grid::{OrbPaint, orb_svg};
    let opaque = crate::theme::theme_preview_colors("tokyo_night").expect("bundled theme");
    let translucent = crate::theme::ThemePreviewColors {
        glow: crate::theme::with_alpha(opaque.glow, 0.5),
        ..opaque
    };
    assert_ne!(
        orb_svg(OrbPaint::Single(opaque)),
        orb_svg(OrbPaint::Single(translucent)),
        "the accent's alpha reaches the orb"
    );
}

/// Each save of a live-reloaded custom theme is a new palette.
#[test]
fn orb_image_cache_stays_bounded_across_palette_edits() {
    use super::theme_grid::{OrbImageCache, OrbPaint};
    let mut cache = OrbImageCache::default();
    let base = crate::theme::theme_preview_colors("tokyo_night").expect("bundled theme");
    for step in 0..300u32 {
        let mut glow = base.glow;
        glow.red = (step % 256) as f32 / 255.0;
        glow.green = (step / 256) as f32 / 255.0;
        let _ = cache.get(OrbPaint::Single(crate::theme::ThemePreviewColors {
            glow,
            ..base
        }));
    }
    assert!(cache.len() <= 128, "{} orb images cached", cache.len());
}

#[test]
fn theme_tiles_list_automatic_first_then_every_theme_once_by_appearance() {
    let groups = super::theme_grid::grouped_tile_keys();
    assert_eq!(groups[0], ("automatic", vec!["automatic".to_string()]));

    let mut listed = Vec::new();
    for (group, keys) in &groups[1..] {
        for key in keys {
            let option = crate::theme::available_themes()
                .into_iter()
                .find(|option| &option.key == key)
                .unwrap_or_else(|| panic!("`{key}` is not an available theme"));
            let expected = match (option.custom, option.is_dark) {
                (true, _) => "custom",
                (false, true) => "dark",
                (false, false) => "light",
            };
            assert_eq!(*group, expected, "{key}");
            listed.push(key.clone());
        }
    }
    let mut available = crate::theme::available_themes()
        .into_iter()
        .map(|option| option.key)
        .collect::<Vec<_>>();
    available.sort();
    let mut sorted = listed.clone();
    sorted.sort();
    assert_eq!(sorted, available, "every theme gets exactly one tile");

    let group = |name: &str| {
        groups
            .iter()
            .find(|(group, _)| *group == name)
            .map(|(_, keys)| keys.clone())
            .unwrap_or_default()
    };
    assert_eq!(
        group("dark").first().map(String::as_str),
        Some("gitcomet_dark")
    );
    assert_eq!(
        group("light").first().map(String::as_str),
        Some("gitcomet_light")
    );
}

#[gpui::test]
fn settings_window_blur_requires_deliberate_input_focus(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
fn expanded_settings_sections_render_scrollable_list_containers(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
fn expanded_diff_content_mode_section_renders_before_scroll_sync_row(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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

/// The Appearance page runs Theme -> Interface -> Typography, and typography
/// orders font pickers -> ligatures -> sizes with no presets popover.
#[gpui::test]
fn appearance_card_puts_ligatures_between_the_font_pickers_and_the_sizes(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(2400.0)));
    settings_cx.run_until_parked();
    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let mut bounds = |selector: &'static str| {
        settings_cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("expected `{selector}` in the appearance card"))
    };
    let theme_heading = bounds("settings_window_appearance_theme");
    let tiles = bounds("settings_window_theme_grid");
    let interface_heading = bounds("settings_window_appearance_interface");
    let ui_scale = bounds("settings_window_ui_scale");
    let density = bounds("settings_window_density_controls");
    let window_controls = bounds("settings_window_window_controls");
    let typography_heading = bounds("settings_window_appearance_typography");
    let ui_font = bounds("settings_window_ui_font");
    let editor_font = bounds("settings_window_editor_font");
    let ligatures = bounds("settings_window_use_font_ligatures");
    let sizes = bounds("settings_window_font_size_controls");

    for (upper, lower, what) in [
        (theme_heading, tiles, "theme heading -> tiles"),
        (tiles, interface_heading, "tiles -> interface heading"),
        (interface_heading, ui_scale, "interface heading -> UI scale"),
        (ui_scale, density, "UI scale -> density"),
        (density, window_controls, "density -> window controls"),
        (
            window_controls,
            typography_heading,
            "window controls -> typography",
        ),
        (typography_heading, ui_font, "typography heading -> UI font"),
    ] {
        assert!(
            upper.bottom() <= lower.top(),
            "expected {what}: {upper:?} {lower:?}"
        );
    }

    assert!(
        ui_font.bottom() <= editor_font.top()
            && editor_font.bottom() <= ligatures.top()
            && ligatures.bottom() <= sizes.top(),
        "expected UI font -> editor font -> ligatures -> sizes, got \
         {ui_font:?} {editor_font:?} {ligatures:?} {sizes:?}"
    );

    for presets in [
        "font_size_0_presets",
        "font_size_1_presets",
        "font_size_2_presets",
    ] {
        assert!(
            settings_cx.debug_bounds(presets).is_none(),
            "the font size rows must not offer a presets popover"
        );
    }
}

#[gpui::test]
fn appearance_page_renders_theme_utilities_and_opens_theme_guide(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_links_container")
            .is_some(),
        "expected the appearance page to render theme utility links"
    );
    assert!(
        settings_cx
            .debug_bounds("settings_window_theme_custom_folder")
            .is_some(),
        "expected the appearance page to render the custom folder action"
    );

    let guide_bounds = settings_cx
        .debug_bounds("settings_window_theme_guide")
        .expect("expected theme guide row bounds");
    settings_cx.simulate_click(guide_bounds.center(), Modifiers::default());
    settings_cx.run_until_parked();

    assert_eq!(cx.opened_url(), themes_guide_url());
}

#[gpui::test]
fn expanded_history_columns_section_renders_detail_container(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
fn custom_git_executable_mode_renders_detail_container(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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

#[gpui::test]
fn settings_dropdowns_fit_without_inner_scroll(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
fn settings_window_open_source_licenses_row_switches_content(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
fn settings_window_links_card_includes_theme_guide_row(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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

#[gpui::test]
fn non_macos_settings_window_renders_custom_chrome_controls(cx: &mut gpui::TestAppContext) {
    if cfg!(target_os = "macos") {
        return;
    }

    let _visual_guard = lock_visual_test();
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

    let _visual_guard = lock_visual_test();
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));

    cx.update(|window, app| {
        let _ = window.draw(app);
        crate::window_controls::set_current(app, crate::window_controls::WindowControlsMode::Hide);
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

    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
fn terminal_settings_sections_toggle_and_render_controls(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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

#[gpui::test]
fn ui_font_dropdown_wheel_scrolls_inner_list_before_outer_window(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    settings_cx.simulate_resize(size(px(SETTINGS_WINDOW_DEFAULT_WIDTH_PX), px(460.0)));
    settings_cx.run_until_parked();
    settings_cx.update(|window, app| {
        let _ = window.draw(app);
    });

    let initial_list_bounds = settings_cx
        .debug_bounds("settings_window_ui_font_list_container")
        .expect("expected UI font list bounds");
    let scroll_bounds = settings_cx
        .debug_bounds("settings_window_scroll")
        .expect("expected settings scroll bounds");
    let list_center = initial_list_bounds.center();
    if list_center.y >= scroll_bounds.bottom() {
        let scroll_delta = list_center.y - scroll_bounds.bottom() + px(24.0);
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
    let list_bounds = settings_cx
        .debug_bounds("settings_window_ui_font_list_container")
        .expect("expected visible UI font list bounds");

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

#[gpui::test]
fn appearance_sizes_apply_live_to_every_main_window_and_keep_ui_scale_independent(
    cx: &mut gpui::TestAppContext,
) {
    let _guard = lock_visual_test();
    let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
    let (first, first_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(first_store, first_events, None, window, cx)
    });
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    // A second window participates in the same application-wide preferences.
    let (second_store, second_events) = AppStore::new_test(Arc::new(TestBackend));
    let (second, second_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(second_store, second_events, None, window, cx)
    });
    let before_rem = second_cx.update(|window, _| window.rem_size());
    settings
        .update(second_cx, |settings, _, cx| {
            settings.set_density(UiDensity::Comfortable, cx);
            settings.set_font_size(FontRole::Ui, 20, cx);
            settings.set_font_size(FontRole::Editor, 26, cx);
            settings.set_font_size(FontRole::Markdown, 18, cx);
        })
        .unwrap();
    second_cx.run_until_parked();
    second_cx.update(|window, app| {
        assert_eq!(
            window.rem_size(),
            before_rem,
            "font sizes must not rescale panel geometry"
        );
        let metrics = crate::appearance::current(app);
        for view in [&first, &second] {
            let view = view.read(app);
            assert_eq!(view.theme.metrics, metrics);
            assert_eq!(view.theme.metrics.ui_font_size_px, 20);
            assert_eq!(view.theme.metrics.editor_font_size_px, 26);
            assert_eq!(view.theme.metrics.markdown_preview_font_size_px, 18);
            assert_eq!(
                view.main_pane
                    .read(app)
                    .file_editor_input
                    .read(app)
                    .line_height_override(),
                Some(view.theme.editor_row_height(view.ui_scale_percent))
            );
        }
    });
    settings
        .update(second_cx, |settings, _, cx| {
            settings.set_font_size(FontRole::Ui, 0, cx);
            settings.set_font_size(FontRole::Editor, 99, cx);
            assert_eq!(settings.appearance_metrics.ui_font_size_px, 20);
            assert_eq!(settings.appearance_metrics.editor_font_size_px, 26);
            settings.set_font_size(FontRole::Ui, FontRole::Ui.default_size(), cx);
            assert_eq!(settings.appearance_metrics.editor_font_size_px, 26);
            assert_eq!(
                settings.appearance_metrics.markdown_preview_font_size_px,
                18
            );
            settings.font_size_inputs[FontRole::Ui.index()]
                .update(cx, |input, cx| input.set_text("invalid", cx));
            settings.set_font_size(FontRole::Ui, FontRole::Ui.default_size(), cx);
            assert_eq!(
                settings.font_size_inputs[FontRole::Ui.index()]
                    .read(cx)
                    .text(),
                "14",
                "Reset must repair an invalid draft even at the default size"
            );
        })
        .unwrap();
}

#[gpui::test]
fn zoom_is_per_window_and_the_default_moves_only_unzoomed_windows(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
    let (first_store, first_events) = AppStore::new_test(Arc::new(TestBackend));
    let (first, first_cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(first_store, first_events, None, window, cx)
    });
    let first_window = first_cx.window_handle();
    first_cx.update(|_, app| open_settings_window(app));
    first_cx.run_until_parked();
    let settings = first_cx.update(|_, app| {
        app.windows()
            .into_iter()
            .find_map(|window| window.downcast::<SettingsWindowView>())
            .unwrap()
    });
    let (second_store, second_events) = AppStore::new_test(Arc::new(TestBackend));
    let (second, cx) = cx.add_window_view(|window, cx| {
        GitCometView::new(second_store, second_events, None, window, cx)
    });
    let second_window = cx.window_handle();

    let scales = |cx: &mut gpui::VisualTestContext| {
        (
            first.read_with(cx, |view, _| view.ui_scale_percent),
            second.read_with(cx, |view, _| view.ui_scale_percent),
        )
    };
    // What a window's own reads resolve to, and its rem size.
    let window_scale = |cx: &mut gpui::VisualTestContext, handle: gpui::AnyWindowHandle| {
        handle
            .update(cx, |_, window, app| {
                assert_eq!(
                    window.rem_size(),
                    crate::ui_scale::rem_size_for_percent(crate::ui_scale::current(app).percent)
                );
                crate::ui_scale::current(app).percent
            })
            .unwrap()
    };
    let set_zoom = |cx: &mut gpui::VisualTestContext, percent: Option<u32>| {
        gpui::TestAppContext::update(cx, |app| {
            crate::app::set_window_ui_scale_percent(app, first_window.window_id(), percent);
        });
        cx.run_until_parked();
    };

    set_zoom(cx, Some(125));
    assert_eq!(scales(cx), (125, 100), "only the zoomed window changes");
    assert_eq!(window_scale(cx, first_window), 125);
    assert_eq!(window_scale(cx, second_window), 100);

    // A new default moves windows without their own zoom, Settings included.
    settings
        .update(cx, |settings, window, cx| {
            settings.set_ui_scale_percent(110, window, cx);
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(scales(cx), (125, 110), "the zoomed window keeps its zoom");
    assert_eq!(window_scale(cx, second_window), 110);
    settings
        .update(cx, |settings, _, _| {
            assert_eq!(settings.default_ui_scale_percent, 110);
            assert_eq!(settings.ui_scale_percent, 110);
            assert_eq!(settings.preference_settings().ui_scale_percent, Some(110));
        })
        .unwrap();

    // Resetting returns the window to the default.
    set_zoom(cx, None);
    assert_eq!(scales(cx), (110, 110));
    assert_eq!(window_scale(cx, first_window), 110);
}

#[test]
fn density_and_font_geometry_remain_independent_across_scales() {
    for percent in [80, 100, 125, 200] {
        for density in UiDensity::ALL {
            for ui_size in [10, 14, 24] {
                for editor_size in [8, 13, 32] {
                    let metrics = Appearance {
                        density,
                        ui_font_size_px: ui_size,
                        editor_font_size_px: editor_size,
                        markdown_preview_font_size_px: 18,
                    };
                    let theme = AppTheme::gitcomet_dark().with_appearance(metrics);
                    let scale = ui_scale::UiScale::from_percent(percent).with_appearance(metrics);
                    assert!(
                        components::control_height(scale)
                            >= scale.px(if density == UiDensity::Comfortable {
                                32.0
                            } else {
                                22.0
                            })
                    );
                    assert_eq!(
                        theme.editor_font_size(percent),
                        scale.px(editor_size as f32)
                    );
                    assert!(theme.editor_row_height(percent) > theme.editor_font_size(percent));
                    assert_eq!(theme.markdown_px(13.0, percent), scale.px(18.0));
                    let gutter = crate::view::rows::resolved_output_line_no_width(1234, scale);
                    assert!(
                        gutter >= scale.px(4.0 * editor_size as f32 * 0.6),
                        "gutter must hold every digit at the selected editor size"
                    );
                }
            }
        }
    }
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
fn history_branch_names_options_update_every_main_window(cx: &mut gpui::TestAppContext) {
    let _guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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

#[test]
fn appearance_page_owns_themes_fonts_scale_and_density_in_search() {
    for query in [
        "theme",
        "solarized",
        "catppuccin",
        "density",
        "font size",
        "ui font",
        "ligatures",
        "ui scale",
        "window controls",
    ] {
        assert!(
            SettingsCategory::Appearance.matches_query(query),
            "{query} should find the Appearance page"
        );
    }
    for query in ["theme", "density", "font size", "ui font"] {
        assert!(
            !SettingsCategory::General.matches_query(query),
            "{query} moved off the General page"
        );
    }
    for query in ["external code editor", "timezone", "command line"] {
        assert!(
            SettingsCategory::General.matches_query(query),
            "{query} stays on the General page"
        );
    }
    for section in [
        SettingsSection::UiScale,
        SettingsSection::WindowControls,
        SettingsSection::UiFont,
        SettingsSection::EditorFont,
    ] {
        assert_eq!(
            section.category(),
            SettingsCategory::Appearance,
            "{section:?}"
        );
    }
    assert_eq!(
        SettingsSection::BrowserOpenTarget.category(),
        SettingsCategory::General
    );
}

/// Custom and newly bundled themes are found by name without a hand-kept list.
#[test]
fn appearance_search_finds_every_theme_by_name() {
    for option in crate::theme::available_themes() {
        let query = option.label.to_lowercase();
        assert!(
            SettingsCategory::Appearance.matches_query(&query),
            "{query} should find the Appearance page"
        );
    }
}

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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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

/// A workspace whose theme was deleted follows the app theme, so the note must
/// not count it.
#[gpui::test]
fn appearance_page_does_not_count_an_override_whose_theme_is_gone(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let mut workspace =
        gitcomet_state::session::Workspace::new(vec![PathBuf::from("/tmp/appearance-tiles-c")]);
    workspace.theme_mode = Some("deleted_custom_theme".to_string());
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
            .is_none(),
        "an override naming a deleted theme is no override"
    );
}

/// The app theme can name a user theme deleted since it was picked; a
/// workspace must still be able to go back to following it.
#[gpui::test]
fn workspace_theme_picker_offers_follow_app_when_the_app_theme_is_gone(
    cx: &mut gpui::TestAppContext,
) {
    let _visual_guard = lock_visual_test();
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

/// Every theme lookup walks the themes folder in the app, so a page resolves
/// all its tiles, labels and overrides from one read per draw.
#[gpui::test]
fn theme_pages_read_the_themes_folder_once_per_draw(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    let workspaces = ["tokyo_night", "nord", "deleted_custom_theme"].map(|key| {
        let mut workspace = gitcomet_state::session::Workspace::new(vec![PathBuf::from(format!(
            "/tmp/theme-lookups-{key}"
        ))]);
        workspace.theme_mode = Some(key.to_string());
        workspace
    });
    let id = workspaces[0].id;
    let (store, events) = AppStore::new_test(std::sync::Arc::new(TestBackend));
    let (_main_view, cx) =
        cx.add_window_view(|window, cx| GitCometView::new(store, events, None, window, cx));
    cx.update(|window, app| {
        crate::workspaces::initialize_for_test(app, workspaces.to_vec());
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
    let lookups_per_draw = |settings_cx: &mut gpui::VisualTestContext| {
        settings_cx.run_until_parked();
        settings_cx.update(|window, app| {
            let _ = window.draw(app);
            let before = crate::theme::runtime_theme_lookups_for_test();
            window.refresh();
            let _ = window.draw(app);
            crate::theme::runtime_theme_lookups_for_test() - before
        })
    };

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Appearance, cx);
    });
    let appearance = lookups_per_draw(&mut settings_cx);

    let _ = settings_window.update(&mut settings_cx, |settings, _window, cx| {
        settings.select_category(SettingsCategory::Workspaces, cx);
        settings.select_workspace(id, cx);
        settings.toggle_section(SettingsSection::WorkspaceTheme, cx);
    });
    let workspaces_page = lookups_per_draw(&mut settings_cx);

    assert_eq!(
        (appearance, workspaces_page),
        (1, 1),
        "(Appearance, Workspaces) theme-folder reads per draw"
    );
}

#[gpui::test]
fn workspaces_page_edits_colour_theme_name_and_deletes(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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
    let _visual_guard = lock_visual_test();
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

/// Unoptimized CI builds give every builder temporary its own stack slot, so
/// the old single-function render needed ~2 MiB and overflowed the 2 MiB
/// Windows test thread. Now ~670 KiB; an overflow here aborts the binary.
#[test]
fn settings_window_renders_every_category_within_a_bounded_stack() {
    std::thread::Builder::new()
        .name("settings_render_stack_budget".into())
        .stack_size(1024 * 1024)
        .spawn(|| {
            let _visual_guard = lock_visual_test();
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

/// "Recheck" re-probes `git lfs` / `git annex` from this window. The main
/// windows only probe once per Git runtime, so without the result they keep
/// offering "(install git-lfs)" after the user installed it.
#[gpui::test]
fn large_file_tools_recheck_reaches_the_main_windows(cx: &mut gpui::TestAppContext) {
    use gitcomet_core::large_file_tools::{LargeFileToolsState, ToolAvailability};
    let _visual_guard = lock_visual_test();
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

#[gpui::test]
fn settings_pages_are_listed_and_built_only_when_selected(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    cx.update(|app| {
        let registry = gitcomet_extension_api::Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .expect("valid registration");
        crate::view::extension_host::install(registry, app);
    });
    let (_settings, cx) = cx.add_window_view(SettingsWindowView::new);
    crate::view::test_support::redraw(cx);
    let nav = "settings_window_nav_extension_com.example.review/review-settings";
    assert!(cx.debug_bounds(nav).is_some(), "the page is listed");
    assert!(
        cx.debug_bounds("example_review_settings").is_none(),
        "an unselected page is not built"
    );

    let center = cx.debug_bounds(nav).unwrap().center();
    cx.simulate_click(center, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_settings").is_some());

    let general = cx
        .debug_bounds("settings_window_nav_general")
        .unwrap()
        .center();
    cx.simulate_click(general, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("example_review_settings").is_none());
    assert!(cx.debug_bounds("settings_window_general").is_some());
}

/// A settings page gets the window's host: its reset opens a hosted dialog,
/// confirming closes it, and its toasts keep their kind.
#[gpui::test]
fn settings_pages_open_dialogs_and_toasts_through_their_host(cx: &mut gpui::TestAppContext) {
    let _visual_guard = lock_visual_test();
    cx.update(|app| {
        let registry = gitcomet_extension_api::Registry::build(vec![Box::new(
            gitcomet_extension_example::review::ReviewExtension,
        )])
        .expect("valid registration");
        crate::view::extension_host::install(registry, app);
    });
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    crate::view::test_support::redraw(cx);
    let nav = "settings_window_nav_extension_com.example.review/review-settings";
    let center = cx.debug_bounds(nav).unwrap().center();
    cx.simulate_click(center, gpui::Modifiers::default());
    crate::view::test_support::redraw(cx);

    let reset = cx.debug_bounds("example_review_reset").unwrap().center();
    cx.simulate_click(reset, gpui::Modifiers::default());
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("settings_hosted_dialog").is_some());
    assert!(cx.debug_bounds("example_reset_confirm").is_some());

    let confirm = cx
        .debug_bounds("example_reset_confirm_button")
        .unwrap()
        .center();
    cx.simulate_click(confirm, gpui::Modifiers::default());
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("settings_hosted_dialog").is_none());
    assert!(cx.debug_bounds("settings_notice").is_some());

    let host = cx.update(|_, app| settings.read(app).extension_window.as_ref().unwrap().host());
    cx.update(|_, app| {
        host.report_error("Could not reach the server", Vec::new(), app)
            .unwrap()
    });
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    cx.update(|_, app| {
        let (kind, message, _) = settings.read(app).extension_notice.clone().unwrap();
        assert_eq!(kind, gitcomet_extension_api::NotificationKind::Error);
        assert_eq!(message.as_ref(), "Could not reach the server");
    });
}

#[gpui::test]
fn settings_extensions_have_a_host_revisioned_gates_and_window_lifetime(
    cx: &mut gpui::TestAppContext,
) {
    use gitcomet_extension_api::*;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    let _guard = lock_visual_test();
    struct Instance(Rc<Cell<usize>>);
    impl WindowExtension for Instance {}
    impl Drop for Instance {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }
    struct ExtensionProbe {
        host: Rc<RefCell<Option<WindowHost>>>,
        drops: Rc<Cell<usize>>,
        active: Rc<Cell<bool>>,
        calls: Rc<Cell<usize>>,
        signal: SlotSignal,
    }
    impl Extension for ExtensionProbe {
        fn id(&self) -> ExtensionId {
            ExtensionId::new("com.example.settings-test").unwrap()
        }
        fn register(&self, r: &mut Registrar) {
            let active = self.active.clone();
            let calls = self.calls.clone();
            r.window_gate(
                "gate",
                WindowGateDescriptor::new(
                    self.signal.clone(),
                    move |host, _| {
                        assert_eq!(host.kind(), gitcomet_core::identity::WindowKind::Settings);
                        calls.set(calls.get() + 1);
                        active.get()
                    },
                    |_, _, cx| cx.new(|_| gpui::Empty).into(),
                ),
            );
        }
        fn window_opened(
            &self,
            host: WindowHost,
            _: &mut Window,
            _: &mut App,
        ) -> Option<Box<dyn WindowExtension>> {
            *self.host.borrow_mut() = Some(host);
            Some(Box::new(Instance(self.drops.clone())))
        }
    }
    let host = Rc::new(RefCell::new(None));
    let drops = Rc::new(Cell::new(0));
    let active = Rc::new(Cell::new(true));
    let calls = Rc::new(Cell::new(0));
    let signal = SlotSignal::default();
    cx.update(|app| {
        crate::view::extension_host::install(
            Registry::build(vec![Box::new(ExtensionProbe {
                host: host.clone(),
                drops: drops.clone(),
                active: active.clone(),
                calls: calls.clone(),
                signal: signal.clone(),
            })])
            .unwrap(),
            app,
        )
    });
    let (_, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.run_until_parked();
    let host = host.borrow().clone().expect("settings window host");
    for _ in 0..3 {
        crate::view::test_support::redraw(cx);
    }
    assert_eq!(calls.get(), 1, "unchanged gates are not reevaluated");
    assert!(cx.debug_bounds("settings_window_general").is_none());
    active.set(false);
    signal.bump();
    host.notifier().notify(Slot::Gate);
    cx.run_until_parked();
    crate::view::test_support::redraw(cx);
    assert!(cx.debug_bounds("settings_window_general").is_some());
    assert_eq!(calls.get(), 2);
    cx.update(|window, _| window.remove_window());
    cx.run_until_parked();
    assert_eq!(drops.get(), 1);
    cx.cx.update(|app| assert!(!host.is_open(app)));
}
