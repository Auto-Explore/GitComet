mod backend;
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
