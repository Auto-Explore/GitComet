pub(super) use super::*;
pub(super) use crate::test_support::lock_clipboard_test;
pub(super) use gitcomet_core::error::{Error, ErrorKind};
pub(super) use gitcomet_core::services::{GitBackend, GitRepository, Result};
pub(super) use std::path::Path;
pub(super) use std::sync::Arc;
pub(super) use std::time::SystemTime;

pub(super) fn assert_window_focus(
    window: &gpui::Window,
    app: &gpui::App,
    expected: FocusHandle,
    message: &str,
) {
    assert_eq!(window.focused(app), Some(expected), "{message}");
}

pub(super) fn simulate_key_press(cx: &mut gpui::VisualTestContext, key: &str) {
    let keystroke = gpui::Keystroke::parse(key)
        .unwrap_or_else(|err| panic!("failed to parse test keystroke `{key}`: {err}"))
        .with_simulated_ime();

    cx.update(|window, app| {
        let _ = window.dispatch_event(
            gpui::PlatformInput::KeyDown(gpui::KeyDownEvent {
                keystroke: keystroke.clone(),
                is_held: false,
                prefer_character_input: false,
            }),
            app,
        );
        let _ = window.dispatch_event(
            gpui::PlatformInput::KeyUp(gpui::KeyUpEvent { keystroke }),
            app,
        );
    });
    cx.run_until_parked();
}

pub(super) struct TestBackend;

impl GitBackend for TestBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "Test backend does not open repositories",
        )))
    }
}

mod branch;
mod clone;
mod context_shortcuts;
mod file_actions;
mod layout;
mod picker;
mod recent_repo_picker;
mod refs;
mod stash;
mod status;
mod submodule;
