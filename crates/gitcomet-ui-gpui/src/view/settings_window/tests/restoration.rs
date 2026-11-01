use super::*;

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
        assert!(view.diff_renderer.read(app).reveal_whitespace_chars);
        assert!(view.diff_renderer.read(app).diff_word_wrap);
        assert!(!view.diff_renderer.read(app).diff_show_line_numbers);
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

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";
const DIFF_DEFAULTS_SESSION_SUBTEST_ENV: &str = "GITCOMET_DIFF_DEFAULTS_SESSION_SUBTEST";
