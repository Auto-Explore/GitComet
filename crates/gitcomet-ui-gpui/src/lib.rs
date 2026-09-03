mod app;
mod assets;
mod bundled_fonts;
mod clipboard;
mod external_editor;
pub mod focused_diff;
mod font_preferences;
mod http;
mod kit;
mod launch_guard;
mod linux_gui_env;
mod menu_labels;
#[doc(hidden)]
pub mod perf_alloc;
#[doc(hidden)]
pub mod perf_ram_guard;
#[doc(hidden)]
pub mod perf_sidecar;
mod press_gesture;
mod startup_probe;
mod text_runs;
mod text_selection;
mod theme;
mod ui_probe;
mod ui_runtime;
mod ui_scale;
mod view;

pub use app::{
    FocusedMergetoolConfig, UiRunOutcome, run, run_focused_mergetool,
    run_with_startup_crash_report, run_with_startup_crash_report_and_shutdown_callback,
};
pub use focused_diff::{FocusedDiffConfig, run_focused_diff};
pub use launch_guard::UiLaunchError;
pub use view::StartupCrashReport;

#[cfg(feature = "benchmarks")]
#[doc(hidden)]
pub mod benchmarks {
    pub use crate::view::rows::benchmarks::*;
}

#[cfg(test)]
mod smoke_tests;
#[cfg(test)]
mod test_support;
