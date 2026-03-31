#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;

struct IsolatedGitConfigEnv {
    _root: tempfile::TempDir,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    global_config: PathBuf,
}

fn isolated_git_config_env() -> &'static IsolatedGitConfigEnv {
    static ENV: OnceLock<IsolatedGitConfigEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("create isolated git config tempdir");
        let home_dir = root.path().join("home");
        let xdg_config_home = root.path().join("xdg");
        let global_config = root.path().join("global.gitconfig");

        fs::create_dir_all(&home_dir).expect("create isolated HOME directory");
        fs::create_dir_all(&xdg_config_home).expect("create isolated XDG_CONFIG_HOME directory");
        fs::write(&global_config, "").expect("create isolated global git config file");

        IsolatedGitConfigEnv {
            _root: root,
            home_dir,
            xdg_config_home,
            global_config,
        }
    })
}

pub(crate) fn apply(cmd: &mut Command) {
    let env = isolated_git_config_env();
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &env.global_config)
        .env("HOME", &env.home_dir)
        .env("XDG_CONFIG_HOME", &env.xdg_config_home)
        .env_remove("GIT_CONFIG_SYSTEM");
}
