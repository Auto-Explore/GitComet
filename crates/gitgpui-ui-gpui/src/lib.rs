mod app;
mod assets;
pub mod focused_diff;
mod kit;
mod theme;
mod view;
mod zed_port;

pub use app::{FocusedMergetoolConfig, run, run_focused_mergetool};
pub use focused_diff::{FocusedDiffConfig, run_focused_diff};

#[doc(hidden)]
pub mod benchmarks {
    pub use crate::view::rows::benchmarks::*;
}

#[cfg(test)]
mod smoke_tests;
