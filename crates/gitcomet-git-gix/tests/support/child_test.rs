use std::process::Command;

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
