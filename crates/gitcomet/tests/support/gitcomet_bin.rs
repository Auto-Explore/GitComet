use gitcomet_core::path_utils::canonicalize_or_original;
use std::path::PathBuf;

pub(crate) fn gitcomet_bin() -> PathBuf {
    if let Some(path) = std::env::var_os("CARGO_BIN_EXE_gitcomet").map(PathBuf::from)
        && path.is_file()
    {
        return path;
    }

    if let Some(path) = gitcomet_bin_from_current_exe() {
        return path;
    }

    panic!(
        "gitcomet binary path was not found. Tried CARGO_BIN_EXE_gitcomet and a fallback relative to current test executable"
    );
}

fn gitcomet_bin_from_current_exe() -> Option<PathBuf> {
    let test_exe = canonicalize_or_original(std::env::current_exe().ok()?);
    let profile_dir = test_exe.parent()?.parent()?;
    let candidate = profile_dir.join(format!("gitcomet{}", std::env::consts::EXE_SUFFIX));
    candidate
        .is_file()
        .then(|| canonicalize_or_original(candidate))
}
