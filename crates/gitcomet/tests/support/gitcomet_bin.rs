use gitcomet_core::path_utils::canonicalize_or_original;
use std::path::PathBuf;

pub(crate) fn gitcomet_bin() -> PathBuf {
    // Nextest names the binary at run time. Cargo also fixes the path at
    // compile time, which a test executable launched on its own still carries.
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_gitcomet")
        .map(PathBuf::from)
        .into_iter()
        .chain(option_env!("CARGO_BIN_EXE_gitcomet").map(PathBuf::from))
        .find(|path| path.is_file())
    {
        return path;
    }

    if let Some(path) = gitcomet_bin_from_current_exe() {
        return path;
    }

    panic!(
        "gitcomet binary path was not found. Tried CARGO_BIN_EXE_gitcomet at run and compile time, and the profile directory above the test executable"
    );
}

/// The binary sits in the profile directory. A test executable is in `deps/`
/// one level below it, or four below it in `build/<package>/<hash>/out/`, as
/// Cargo 1.100 lays out a build.
fn gitcomet_bin_from_current_exe() -> Option<PathBuf> {
    let test_exe = canonicalize_or_original(std::env::current_exe().ok()?);
    let name = format!("gitcomet{}", std::env::consts::EXE_SUFFIX);
    test_exe
        .ancestors()
        .skip(2)
        .take(4)
        .map(|dir| dir.join(&name))
        .find(|candidate| candidate.is_file())
        .map(canonicalize_or_original)
}
