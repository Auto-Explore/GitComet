use super::*;
use crate::linux_renderer::{RendererPreference, RendererSession, RendererState};

#[test]
fn linux_renderer_search_finds_the_general_page() {
    for query in ["graphics", "graphics renderer", "vulkan", "opengl"] {
        assert!(SettingsCategory::General.matches_query(query));
        assert!(!SettingsCategory::Appearance.matches_query(query));
    }
    assert_eq!(
        SettingsSection::GraphicsRenderer.category(),
        SettingsCategory::General
    );
}

fn isolated_session(test_name: &str, fail_writes: bool) -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("GITCOMET_RENDERER_TEST_SESSION") {
        return Some(path.into());
    }
    let dir = tempfile::tempdir().unwrap();
    let path = if fail_writes {
        dir.path().to_path_buf()
    } else {
        let path = dir.path().join("session.json");
        session::persist_ui_settings_to_path(
            session::UiSettings {
                linux_renderer: Some("auto".into()),
                windows_renderer: Some("dx11".into()),
                ..Default::default()
            },
            &path,
        )
        .unwrap();
        path
    };
    let output = Command::new(std::env::current_exe().unwrap())
        .arg(format!(
            "view::settings_window::tests::graphics_renderer::{test_name}"
        ))
        .arg("--exact")
        .arg("--nocapture")
        .env("GITCOMET_RENDERER_TEST_SESSION", &path)
        .env("GITCOMET_SESSION_FILE", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success()
            && String::from_utf8_lossy(&output.stdout)
                .contains("test result: ok. 1 passed; 0 failed; 0 ignored;"),
        "renderer subtest failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    None
}

#[gpui::test]
fn linux_choices_save_before_restart_and_respect_overrides_and_focused_launches(
    cx: &mut gpui::TestAppContext,
) {
    let Some(path) = isolated_session(
        "linux_choices_save_before_restart_and_respect_overrides_and_focused_launches",
        false,
    ) else {
        return;
    };
    let session = RendererSession(std::rc::Rc::new(std::cell::RefCell::new(
        RendererState::new(RendererPreference::Auto, true),
    )));
    cx.update(|app| app.set_global(session.clone()));
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.simulate_resize(size(px(720.0), px(1600.0)));
    settings.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::General, cx);
        settings.set_expanded_section(Some(SettingsSection::GraphicsRenderer), cx);
        assert_eq!(settings.preference_settings().linux_renderer, None);
        assert_eq!(settings.preference_settings().windows_renderer, None);
    });
    crate::view::test_support::redraw(cx);
    let automatic = cx.debug_bounds("settings_graphics_renderer_auto").unwrap();
    let vulkan = cx
        .debug_bounds("settings_graphics_renderer_vulkan")
        .unwrap();
    let gl = cx.debug_bounds("settings_graphics_renderer_gl").unwrap();
    assert!(automatic.bottom() <= vulkan.top());
    assert!(vulkan.bottom() <= gl.top());
    cx.simulate_click(gl.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    assert_eq!(session.0.borrow().saved, RendererPreference::Gl);
    let saved = session::load_from_path(&path);
    assert_eq!(saved.linux_renderer.as_deref(), Some("gl"));
    assert_eq!(saved.windows_renderer.as_deref(), Some("dx11"));
    cx.simulate_prompt_answer("Later");
    cx.run_until_parked();
    cx.update(|_, app| {
        assert!(app.try_global::<crate::app::PendingRestart>().is_none());
    });

    session.0.borrow_mut().overridden = true;
    crate::view::test_support::redraw(cx);
    let vulkan = cx
        .debug_bounds("settings_graphics_renderer_vulkan")
        .unwrap();
    cx.simulate_click(vulkan.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert!(!session.0.borrow().saving);
    assert_eq!(session.0.borrow().saved, RendererPreference::Gl);
    assert_eq!(
        session::load_from_path(&path).linux_renderer.as_deref(),
        Some("gl")
    );

    {
        let mut state = session.0.borrow_mut();
        state.overridden = false;
        state.restart_allowed = false;
    }
    crate::view::test_support::redraw(cx);
    let vulkan = cx
        .debug_bounds("settings_graphics_renderer_vulkan")
        .unwrap();
    cx.simulate_click(vulkan.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    assert_eq!(session.0.borrow().saved, RendererPreference::Vulkan);
    assert_eq!(
        session::load_from_path(&path).linux_renderer.as_deref(),
        Some("vulkan")
    );
}

#[gpui::test]
fn linux_renderer_save_failure_keeps_the_previous_selection_without_a_restart_prompt(
    cx: &mut gpui::TestAppContext,
) {
    let Some(path) = isolated_session(
        "linux_renderer_save_failure_keeps_the_previous_selection_without_a_restart_prompt",
        true,
    ) else {
        return;
    };
    let session = RendererSession(std::rc::Rc::new(std::cell::RefCell::new(
        RendererState::new(RendererPreference::Auto, true),
    )));
    cx.update(|app| app.set_global(session.clone()));
    let (settings, cx) = cx.add_window_view(SettingsWindowView::new);
    cx.simulate_resize(size(px(720.0), px(1600.0)));
    settings.update(cx, |settings, cx| {
        settings.select_category(SettingsCategory::General, cx);
        settings.set_expanded_section(Some(SettingsSection::GraphicsRenderer), cx);
    });
    crate::view::test_support::redraw(cx);
    let gl = cx.debug_bounds("settings_graphics_renderer_gl").unwrap();
    cx.simulate_click(gl.center(), Modifiers::default());
    cx.run_until_parked();
    assert!(!cx.has_pending_prompt());
    let state = session.0.borrow();
    assert!(!state.saving);
    assert_eq!(state.saved, RendererPreference::Auto);
    assert!(!state.restart_required());
    assert!(
        state
            .status_message()
            .contains("Could not save graphics renderer:")
    );
    assert!(path.is_dir());
    assert_eq!(std::fs::read_dir(path).unwrap().count(), 0);
}
