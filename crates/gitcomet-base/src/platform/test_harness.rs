//! Telling a test binary from the application.
//!
//! `cfg(test)` is set only for a crate's own unit tests. A dependency built
//! into another crate's tests, or into an integration test, has to look at
//! the executable to keep away from the user's own application state.

use std::ffi::OsStr;
use std::path::Path;

/// Whether this process is a `cargo test` or nextest binary.
pub fn running_under_test_harness() -> bool {
    static RUNNING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNING.get_or_init(|| std::env::current_exe().is_ok_and(|exe| looks_like_test_binary(&exe)))
}

/// Whether this process's file is named the way Cargo names a test binary.
/// Unlike [`running_under_test_harness`], a GitComet installed under a
/// directory called `deps` is no test binary to this.
pub fn named_like_test_binary() -> bool {
    static NAMED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NAMED.get_or_init(|| std::env::current_exe().is_ok_and(|exe| is_named_like_test_binary(&exe)))
}

fn is_named_like_test_binary(exe: &Path) -> bool {
    exe.file_stem()
        .is_some_and(looks_like_cargo_test_binary_name)
}

/// Whether `exe` is where Cargo or nextest puts a test binary, or is named
/// like one.
pub fn looks_like_test_binary(exe: &Path) -> bool {
    if exe.components().any(|component| {
        component.as_os_str() == OsStr::new("deps")
            || component.as_os_str() == OsStr::new("nextest")
    }) {
        return true;
    }

    exe.file_stem()
        .is_some_and(looks_like_cargo_test_binary_name)
}

fn looks_like_cargo_test_binary_name(stem: &OsStr) -> bool {
    let Some(stem) = stem.to_str() else {
        return false;
    };
    let Some((_prefix, suffix)) = stem.rsplit_once('-') else {
        return false;
    };
    // Cargo test binaries typically end in a 16-hex-digit hash suffix, e.g. `mycrate-3ad1b0fd3f0c0d3e`.
    suffix.len() == 16 && suffix.chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_test_harness_executable_paths() {
        // `cargo test` / nextest binaries are typically located under a `deps` directory.
        assert!(looks_like_test_binary(Path::new(
            "/tmp/target/debug/deps/foo"
        )));
        assert!(!looks_like_test_binary(Path::new("/tmp/target/debug/foo")));

        // nextest uses a separate target subdir.
        assert!(looks_like_test_binary(Path::new(
            "/tmp/target/nextest/default/foo"
        )));

        // Cargo test binaries also have a hash suffix.
        assert!(looks_like_test_binary(Path::new(
            "/tmp/target/debug/gitcomet_ui_gpui-3ad1b0fd3f0c0d3e"
        )));
        assert!(!looks_like_test_binary(Path::new(
            "/tmp/target/debug/gitcomet"
        )));
        assert!(running_under_test_harness());
        assert!(named_like_test_binary());
    }

    #[test]
    fn only_a_test_binary_name_counts_by_name() {
        assert!(is_named_like_test_binary(Path::new(
            "/tmp/target/debug/deps/gitcomet_state-3ad1b0fd3f0c0d3e"
        )));
        // GitComet installed where a directory has a test-like name.
        for installed in ["/opt/deps/gitcomet", "/home/u/nextest/bin/gitcomet"] {
            assert!(looks_like_test_binary(Path::new(installed)));
            assert!(
                !is_named_like_test_binary(Path::new(installed)),
                "{installed}"
            );
        }
    }
}
