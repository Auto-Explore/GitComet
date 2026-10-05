mod backend;
#[doc(hidden)]
pub mod command_trace;
mod ignore;
mod open;
mod refs;
mod repo;
mod util;

pub use backend::GixBackend;

#[doc(hidden)]
pub fn install_test_git_command_environment(
    global_config: std::path::PathBuf,
    home_dir: std::path::PathBuf,
    xdg_config_home: std::path::PathBuf,
    gnupg_home: std::path::PathBuf,
) {
    util::install_test_git_command_environment(util::TestGitCommandEnvironment {
        global_config,
        home_dir,
        xdg_config_home,
        gnupg_home,
    });
}

#[doc(hidden)]
pub fn allow_test_repo_local_mergetool_command(repo: &std::path::Path, tool_name: &str) {
    repo::allow_test_repo_local_mergetool_command(repo, tool_name);
}

/// Optional decoded retention and caller-pinned allocations, in bytes, followed
/// by optional retained row count. Pinned bytes may overlap retained bytes.
#[doc(hidden)]
pub fn shared_history_memory() -> (usize, usize, usize) {
    repo::shared_history_memory()
}
