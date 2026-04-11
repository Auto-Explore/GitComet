mod app;
mod assets;
mod bundled_fonts;
pub mod focused_diff;
mod font_preferences;
mod kit;
mod launch_guard;
mod linux_gui_env;
#[doc(hidden)]
pub mod perf_alloc;
#[doc(hidden)]
pub mod perf_ram_guard;
#[doc(hidden)]
pub mod perf_sidecar;
mod startup_probe;
mod theme;
mod ui_runtime;
mod view;

pub use app::{FocusedMergetoolConfig, run, run_focused_mergetool, run_with_startup_crash_report};
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
