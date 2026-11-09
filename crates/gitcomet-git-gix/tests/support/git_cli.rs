use crate::test_git_env;
use gitcomet_core::test_support::git_fixture::FixtureTimer;
use std::path::Path;
use std::process::{Command, Output};

/// Fixture git isolated from the user's config; first use also isolates the
/// backend under test.
pub(crate) fn git_command() -> Command {
    let mut cmd = gitcomet_core::test_support::git_fixture::git_command();
    test_git_env::apply(&mut cmd);
    cmd
}

/// [`git_command`] in `repo`, with editors stubbed so fixture commits,
/// merges and rebases never wait on one; a caller's own `.env` still wins.
pub(crate) fn git_cmd(repo: &Path) -> Command {
    let mut cmd = git_command();
    cmd.arg("-C")
        .arg(repo)
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true");
    cmd
}

/// Runs `cmd` with `args` and captured output (Git-for-Windows transport
/// shells hang when they share the runner's console); panics on failure.
pub(crate) fn run_checked(cmd: &mut Command, args: &[&str]) -> Output {
    let _timer = FixtureTimer::new("subprocess", args.first().copied().unwrap_or("git"));
    let output = cmd.args(args).output().expect("git command to run");
    assert!(
        output.status.success(),
        "git {args:?} failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

pub(crate) fn git_output(repo: &Path, args: &[&str]) -> Output {
    let _timer = FixtureTimer::new("subprocess", args.first().copied().unwrap_or("git"));
    git_cmd(repo)
        .args(args)
        .output()
        .expect("git command to run")
}

pub(crate) fn run_git(repo: &Path, args: &[&str]) {
    run_checked(&mut git_cmd(repo), args);
}

pub(crate) fn run_git_with_env(repo: &Path, args: &[&str], envs: &[(&str, &str)]) {
    let mut cmd = git_cmd(repo);
    cmd.envs(envs.iter().copied());
    run_checked(&mut cmd, args);
}

pub(crate) fn git_stdout(repo: &Path, args: &[&str]) -> String {
    git_stdout_raw(repo, args).trim().to_string()
}

/// Stdout exactly as git printed it, trailing newline included.
pub(crate) fn git_stdout_raw(repo: &Path, args: &[&str]) -> String {
    stdout_text(run_checked(&mut git_cmd(repo), args))
}

pub(crate) fn stdout_text(output: Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}
