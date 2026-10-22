use std::process::Command;

/// In the parent, runs test `name` of `module` again in a child process with
/// the environment variable `marker` set and `configure` applied, and returns
/// false; in that child, returns true. For tests that change process-wide
/// state such as the working directory, PATH or the locale.
pub(crate) fn in_child(
    marker: &str,
    module: &str,
    name: &str,
    configure: impl FnOnce(&mut Command),
) -> bool {
    if std::env::var_os(marker).is_some() {
        return true;
    }
    let mut command = Command::new(std::env::current_exe().expect("current test executable"));
    command.env(marker, "1");
    configure(&mut command);
    run(&mut command, module, name);
    false
}

/// Runs test `name` of the caller's module (`module_path!()`) again in a child
/// process built from `command`, and fails unless the child ran that test and
/// it passed: an unmatched `--exact` filter would otherwise pass silently.
pub(crate) fn run(command: &mut Command, module: &str, name: &str) {
    let qualified = match module.split_once("::") {
        Some((_, path)) => format!("{path}::{name}"),
        None => name.to_string(),
    };
    let output = command
        .args(["--exact", &qualified, "--nocapture"])
        .output()
        .expect("run child test");
    let stdout = String::from_utf8_lossy(&output.stdout);
    eprint!("{stdout}{}", String::from_utf8_lossy(&output.stderr));
    assert!(
        output.status.success(),
        "{qualified} failed in its child process"
    );
    assert!(
        stdout.contains(" 1 passed;"),
        "{qualified} did not run in its child process"
    );
}
