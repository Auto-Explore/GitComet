mod app;
mod browser_requests;
pub use browser_requests::{
    BrowserRequestClosed, BrowserRequestReceiver, BrowserRequestSender, browser_request_channel,
};
// Foundations live in the UI kit; these keep the host's `crate::` paths.
pub(crate) use gitcomet_ui_kit as kit;
pub(crate) use gitcomet_ui_kit::{
    appearance, bundled_fonts, clipboard, font_preferences, linux_gui_env, press_gesture,
    text_runs, text_selection, text_selection_owner, theme, ui_probe, ui_runtime, ui_scale,
    window_focus,
};
mod assets;
mod environment;
mod external_editor;
pub mod http;
mod launch_guard;
mod menu_labels;
#[doc(hidden)]
pub mod perf_alloc;
#[doc(hidden)]
pub mod perf_ram_guard;
#[doc(hidden)]
pub mod perf_sidecar;
#[cfg(test)]
mod render_guards;
mod session_ui;
mod startup_probe;
mod view;
mod window_controls;
#[cfg(test)]
mod window_focus_tests;
mod window_root_hook;
mod workspaces;

pub use app::{
    BrowserOpenRequest, BrowserOpenTarget, FocusedMergetoolConfig, UiLaunch, UiRunOutcome,
    run_focused_mergetool,
};
pub use app::{FocusedDiffConfig, run_focused_diff};
#[allow(deprecated)]
pub use app::{
    run, run_with_startup_crash_report, run_with_startup_crash_report_and_shutdown_callback,
    run_with_startup_crash_report_shutdown_callback_and_browser_requests,
    run_with_startup_crash_report_shutdown_callback_and_initial_browser_request,
};
pub use assets::{BRAND_ASSETS, GitCometAssets};
pub use launch_guard::UiLaunchError;
pub use view::StartupCrashReport;

#[cfg(feature = "benchmarks")]
#[doc(hidden)]
pub mod benchmarks {
    pub use crate::view::rows::benchmarks::*;

    /// Benchmarks measure the live app: background work, timers, and
    /// animations run as they do after a real launch.
    pub fn install_live_runtime() {
        crate::ui_runtime::install(crate::ui_runtime::UiRuntime::live());
    }
}

#[cfg(test)]
mod smoke_tests;
#[cfg(test)]
mod test_support;
