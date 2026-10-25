use super::*;
use crate::view::test_support::TestBackend;
use gitcomet_core::process::{GitExecutableAvailability, GitExecutablePreference, GitRuntimeState};
use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

mod appearance;
mod chrome_and_layout;
mod executables;
mod extensions;
mod large_file_tools_lifecycle;
mod preferences;
mod restoration;
mod terminal;
mod workspaces;

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
