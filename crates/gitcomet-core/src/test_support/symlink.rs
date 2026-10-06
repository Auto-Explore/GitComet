//! Symlink fixtures that remain usable on Windows without Developer Mode.
use std::path::Path;

/// Create a directory symlink, returning false only when Windows denies the
/// required symlink privilege. Callers must return from their test in that case.
#[must_use]
pub fn directory(target: impl AsRef<Path>, link: impl AsRef<Path>) -> bool {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_dir(target, link);
    created_or_unavailable(result)
}

/// Create a file symlink with the same capability handling as [`directory`].
#[must_use]
pub fn file(target: impl AsRef<Path>, link: impl AsRef<Path>) -> bool {
    #[cfg(unix)]
    let result = std::os::unix::fs::symlink(target, link);
    #[cfg(windows)]
    let result = std::os::windows::fs::symlink_file(target, link);
    created_or_unavailable(result)
}

fn created_or_unavailable(result: std::io::Result<()>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) if cfg!(windows) && error.raw_os_error() == Some(1314) => {
            eprintln!("Skipping symlink fixture: Windows symlink privilege is unavailable");
            false
        }
        Err(error) => panic!("failed to create symlink fixture: {error}"),
    }
}
