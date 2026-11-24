use gitcomet_core::platform::dirs;
use std::{env, path::PathBuf};

const SESSION_FILE_ENV: &str = "GITCOMET_SESSION_FILE";
const DISABLE_SESSION_PERSIST_ENV: &str = "GITCOMET_DISABLE_SESSION_PERSIST";

pub(crate) fn default_session_file_path() -> Option<PathBuf> {
    #[cfg(test)]
    if let Some(path) = super::test_support::session_file_path_override() {
        return path;
    }

    if let Some(path) = env::var_os(SESSION_FILE_ENV)
        && !path.is_empty()
    {
        return Some(PathBuf::from(path));
    }

    if env::var_os(DISABLE_SESSION_PERSIST_ENV).is_some() {
        return None;
    }

    user_path(dirs::state_dir, "session.json")
}

/// Per-user rendezvous file used by the browser-process broker.
pub fn browser_instance_file_path() -> Option<PathBuf> {
    user_path(dirs::state_dir, "browser-instance.json")
}

/// Where the filesystem service keeps its undo and staging areas.
pub fn journal_storage_dir() -> Option<PathBuf> {
    user_path(dirs::state_dir, "journal")
}

pub fn user_themes_dir() -> Option<PathBuf> {
    user_path(dirs::data_dir, "themes")
}

fn user_path(directory: fn() -> Option<PathBuf>, name: &str) -> Option<PathBuf> {
    #[cfg(any(test, feature = "test-support"))]
    if cfg!(test) || gitcomet_core::platform::test_harness::named_like_test_binary() {
        return None;
    }

    Some(directory()?.join(name))
}
