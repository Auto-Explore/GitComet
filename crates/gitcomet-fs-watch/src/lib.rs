//! Native macOS streams with bounded exclusions and process-wide resource limits.
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::FsEventsWatcher;

#[cfg(all(windows, feature = "test-support"))]
mod windows_test_support;
#[cfg(all(windows, feature = "test-support"))]
pub use windows_test_support::is_local_ntfs;
