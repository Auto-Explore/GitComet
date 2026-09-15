//! Native macOS streams with bounded exclusions and process-wide resource limits.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::FsEventsWatcher;
