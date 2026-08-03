#![allow(dead_code)]

use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

struct IsolatedGitConfigEnv {
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    global_config: PathBuf,
    gnupg_home: PathBuf,
}

fn isolated_git_config_env() -> &'static IsolatedGitConfigEnv {
    static ENV: OnceLock<IsolatedGitConfigEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = unique_test_dir("gitcomet-git-env");
        let home_dir = root.join("home");
        let xdg_config_home = root.join("xdg");
        let global_config = root.join("global.gitconfig");
        let gnupg_home = root.join("gnupg");

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
            home_dir,
            xdg_config_home,
            global_config,
            gnupg_home,
        }
    })
}

fn unique_test_dir(prefix: &str) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let id = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let dir = std::env::temp_dir().join(format!("{prefix}-{pid}-{id}"));
    fs::create_dir_all(&dir).expect("create isolated git config tempdir");
    dir
}

pub(crate) fn ensure_initialized() {
    let _ = isolated_git_config_env();
}

pub(crate) fn apply(cmd: &mut Command) {
    let env = isolated_git_config_env();
    cmd.env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &env.global_config)
        .env("HOME", &env.home_dir)
        .env("XDG_CONFIG_HOME", &env.xdg_config_home)
        .env("GNUPGHOME", &env.gnupg_home)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env_remove("GIT_CONFIG_SYSTEM");
}
