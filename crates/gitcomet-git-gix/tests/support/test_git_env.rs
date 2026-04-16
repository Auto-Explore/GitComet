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
    gnupg_home: PathBuf,
}

fn isolated_git_config_env() -> &'static IsolatedGitConfigEnv {
    static ENV: OnceLock<IsolatedGitConfigEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("create isolated git config tempdir");
        let home_dir = root.path().join("home");
        let xdg_config_home = root.path().join("xdg");
        let global_config = root.path().join("global.gitconfig");
        let gnupg_home = root.path().join("gnupg");

        fs::create_dir_all(&home_dir).expect("create isolated HOME directory");
        fs::create_dir_all(&xdg_config_home).expect("create isolated XDG_CONFIG_HOME directory");
        fs::create_dir_all(&gnupg_home).expect("create isolated GNUPGHOME directory");
        fs::write(&global_config, "").expect("create isolated global git config file");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            fs::set_permissions(&gnupg_home, fs::Permissions::from_mode(0o700))
                .expect("set isolated GNUPGHOME permissions");
        }

        gitcomet_git_gix::install_test_git_command_environment(
            global_config.clone(),
            home_dir.clone(),
            xdg_config_home.clone(),
            gnupg_home.clone(),
        );

        IsolatedGitConfigEnv {
            _root: root,
            home_dir,
            xdg_config_home,
            global_config,
            gnupg_home,
        }
    })
}

pub(crate) fn apply(cmd: &mut Command) {
    let env = isolated_git_config_env();
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &env.global_config)
        .env("HOME", &env.home_dir)
        .env("XDG_CONFIG_HOME", &env.xdg_config_home)
        .env("GNUPGHOME", &env.gnupg_home)
        .env_remove("GIT_CONFIG_SYSTEM");
}
