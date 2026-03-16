mod app;
mod assets;
pub mod focused_diff;
mod kit;
mod launch_guard;
mod theme;
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
