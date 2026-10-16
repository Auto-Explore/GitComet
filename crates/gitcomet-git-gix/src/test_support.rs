use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Debug, Eq, PartialEq)]
struct TestGitCommandEnvironment {
    global_config: PathBuf,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    gnupg_home: PathBuf,
}

static TEST_GIT_COMMAND_ENVIRONMENT: OnceLock<TestGitCommandEnvironment> = OnceLock::new();

pub fn install_test_git_command_environment(
    global_config: PathBuf,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    gnupg_home: PathBuf,
) {
    let env = TestGitCommandEnvironment {
        global_config,
        home_dir,
        xdg_config_home,
        gnupg_home,
    };
    if let Some(existing) = TEST_GIT_COMMAND_ENVIRONMENT.get() {
        assert_eq!(
            existing, &env,
            "test git command environment already initialized"
        );
        return;
    }
    let _ = TEST_GIT_COMMAND_ENVIRONMENT.set(env);
}

pub(crate) fn apply_git_command_environment(cmd: &mut Command) {
    let Some(env) = TEST_GIT_COMMAND_ENVIRONMENT.get() else {
        return;
    };

    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", &env.global_config);
    cmd.env("HOME", &env.home_dir);
    cmd.env("XDG_CONFIG_HOME", &env.xdg_config_home);
    cmd.env("GNUPGHOME", &env.gnupg_home);
    cmd.arg("-c").arg("protocol.file.allow=always");
}

pub fn allow_test_repo_local_mergetool_command(repo: &Path, tool_name: &str) {
    crate::repo::allow_test_repo_local_mergetool_command(repo, tool_name);
}
