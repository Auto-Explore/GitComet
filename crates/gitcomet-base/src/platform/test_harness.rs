//! Test fixture isolation for Cargo test executables.
//!
//! `cfg(test)` is set only for a crate's own unit tests. A dependency built
//! into another crate's tests, or into an integration test, has to look at
//! the executable to keep away from the user's own application state.

use std::ffi::OsStr;
use std::path::Path;

/// Whether the executable has Cargo's test binary suffix.
pub fn named_like_test_binary() -> bool {
    static NAMED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NAMED.get_or_init(|| std::env::current_exe().is_ok_and(|exe| is_named_like_test_binary(&exe)))
}

fn is_named_like_test_binary(exe: &Path) -> bool {
    exe.file_stem()
        .and_then(OsStr::to_str)
        .and_then(|stem| stem.rsplit_once('-'))
        .is_some_and(|(_, suffix)| {
            suffix.len() == 16 && suffix.bytes().all(|b| b.is_ascii_hexdigit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_test_binary_name_counts_by_name() {
        assert!(named_like_test_binary());
        assert!(is_named_like_test_binary(Path::new(
            "/tmp/target/debug/deps/gitcomet_state-3ad1b0fd3f0c0d3e"
        )));
        for installed in ["/opt/deps/gitcomet", "/home/u/nextest/bin/gitcomet"] {
            assert!(
                !is_named_like_test_binary(Path::new(installed)),
                "{installed}"
            );
        }
    }
}
