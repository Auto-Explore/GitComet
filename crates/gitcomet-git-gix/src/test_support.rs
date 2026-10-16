//! The git and repository opens the crate's unit tests share. git runs with
//! an empty global config and no system config, and gix opens read the
//! repository's own config alone, so both see the same settings.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;

#[cfg(unix)]
use std::{fs::Permissions, os::unix::fs::PermissionsExt as _};

struct TestGitEnv {
    _root: tempfile::TempDir,
    global_config: PathBuf,
    home_dir: PathBuf,
    xdg_config_home: PathBuf,
    gnupg_home: PathBuf,
}

fn ensure_isolated_git_test_env() -> &'static TestGitEnv {
    static ENV: OnceLock<TestGitEnv> = OnceLock::new();
    ENV.get_or_init(|| {
        let root = tempfile::tempdir().expect("test git env tempdir");
        let home_dir = root.path().join("home");
        let xdg_config_home = root.path().join("xdg");
        let gnupg_home = root.path().join("gnupg");
        let global_config = root.path().join("gitconfig");

        fs::create_dir_all(&home_dir).expect("test git home");
        fs::create_dir_all(&xdg_config_home).expect("test git xdg config home");
        fs::create_dir_all(&gnupg_home).expect("test gnupg home");
        fs::write(&global_config, "").expect("test global git config");

        #[cfg(unix)]
        fs::set_permissions(&gnupg_home, Permissions::from_mode(0o700))
            .expect("test gnupg home permissions");

        crate::install_test_git_command_environment(
            global_config.clone(),
            home_dir.clone(),
            xdg_config_home.clone(),
            gnupg_home.clone(),
        );

        TestGitEnv {
            _root: root,
            global_config,
            home_dir,
            xdg_config_home,
            gnupg_home,
        }
    })
}

pub(crate) fn git_command() -> Command {
    let env = ensure_isolated_git_test_env();
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", &env.global_config);
    cmd.env("HOME", &env.home_dir);
    cmd.env("XDG_CONFIG_HOME", &env.xdg_config_home);
    cmd.env("GNUPGHOME", &env.gnupg_home);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.env("GCM_INTERACTIVE", "Never");
    cmd.env("GIT_ALLOW_PROTOCOL", "file");
    cmd
}

pub(crate) fn git_output(workdir: &Path, args: &[&str]) -> Output {
    git_command()
        .arg("-C")
        .arg(workdir)
        .args(args)
        .output()
        .expect("spawn git")
}

pub(crate) fn git_success(workdir: &Path, args: &[&str]) {
    let output = git_output(workdir, args);
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Trimmed stdout of a git command that must succeed.
pub(crate) fn git_stdout(workdir: &Path, args: &[&str]) -> String {
    let output = git_output(workdir, args);
    assert!(
        output.status.success(),
        "git {:?} failed\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf8 stdout")
        .trim()
        .to_string()
}

pub(crate) fn git_expect_failure(workdir: &Path, args: &[&str]) -> Output {
    let output = git_output(workdir, args);
    assert!(
        !output.status.success(),
        "expected git {:?} to fail\nstdout:\n{}\nstderr:\n{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub(crate) fn init_test_repo(workdir: &Path) {
    let _ = ensure_isolated_git_test_env();
    git_success(workdir, &["init"]);
    gitcomet_core::test_support::git_fixture::append_config(
        workdir,
        &[
            ("core.autocrlf", "false"),
            ("core.eol", "lf"),
            ("credential.helper", ""),
            ("credential.interactive", "never"),
            ("protocol.file.allow", "always"),
            ("commit.gpgsign", "false"),
            ("user.name", "Test User"),
            ("user.email", "test@example.com"),
        ],
    );
}

pub(crate) fn write_file(workdir: &Path, relative: &str, contents: &str) {
    let path = workdir.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create parent directories");
    }
    fs::write(path, contents).expect("write file");
}

/// Reads only the repository's own config, as `git_command` runs git: a
/// developer's global config must not decide what a test sees.
pub(crate) fn open_gix(workdir: &Path) -> gix::Repository {
    crate::open::open_worktree_repo_with(workdir, gix::open::Permissions::isolated())
        .expect("open repo")
}

/// [`open_gix`], as the crate's repository.
pub(crate) fn open_repo(workdir: &Path) -> crate::repo::GixRepo {
    crate::repo::GixRepo::new(workdir.to_path_buf(), open_gix(workdir).into_sync())
}
