//! Small, safe interfaces around native file APIs. All FFI is isolated here.
#![deny(unsafe_op_in_unsafe_fn)]
use std::path::PathBuf;

/// Exact filesystem entries returned by the platform's Trash API.
#[derive(Debug)]
pub struct TrashReceipt {
    pub item: PathBuf,
    pub info: Option<PathBuf>,
}

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::trash_item;
#[cfg(windows)]
mod windows_impl;
#[cfg(windows)]
pub use windows_impl::{file_identity, rename_exclusive, trash_item};
