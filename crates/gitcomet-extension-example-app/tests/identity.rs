//! The example product launches under its own identity and paths using the
//! upstream crates, with no product feature flags.

use std::path::Path;
use std::process::{Command, Output};

fn example(args: &[&str], home: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_comet-example"))
        .args(args)
        .env("HOME", home)
        .env_remove("XDG_STATE_HOME")
        .env_remove("XDG_DATA_HOME")
        .env("LOCALAPPDATA", home.join("local"))
        .env("GIT_CONFIG_GLOBAL", home.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("run comet-example")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

#[test]
fn version_and_help_name_the_example_product() {
    let home = tempfile::tempdir().unwrap();
    let version = example(&["--version"], home.path());
    assert_eq!(version.status.code(), Some(0));
    assert_eq!(
        stdout(&version).trim(),
        format!("comet-example {}", env!("CARGO_PKG_VERSION"))
    );

    let help = example(&["--help"], home.path());
    assert_eq!(help.status.code(), Some(0));
    let help = stdout(&help);
    assert!(
        help.contains("An example product built on unmodified GitComet crates"),
        "{help}"
    );
    assert!(!help.contains("gitcomet"), "{help}");
    let setup_help = stdout(&example(&["setup", "--help"], home.path()));
    assert!(
        setup_help.contains("Configure git to use comet-example"),
        "{setup_help}"
    );
}

#[test]
fn git_tool_setup_registers_only_the_example_tools() {
    let home = tempfile::tempdir().unwrap();
    let setup = example(&["setup", "--dry-run"], home.path());
    assert_eq!(setup.status.code(), Some(0));
    let setup = stdout(&setup);
    assert!(setup.contains("mergetool.comet-example.cmd"), "{setup}");
    assert!(setup.contains("difftool.comet-example-gui.cmd"), "{setup}");
    assert!(setup.contains("comet-example.backup.*"), "{setup}");
    assert!(!setup.contains("gitcomet"), "{setup}");

    let uninstall = stdout(&example(&["uninstall", "--dry-run"], home.path()));
    assert!(
        uninstall.contains("mergetool.comet-example.cmd"),
        "{uninstall}"
    );
    assert!(!uninstall.contains("gitcomet"), "{uninstall}");
}

#[test]
fn state_data_and_crash_directories_are_the_examples_own() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_comet-example"))
        .env("COMET_EXAMPLE_PRINT_DIRS", "1")
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("LOCALAPPDATA", home.path().join("local"))
        .output()
        .expect("run comet-example");
    let text = stdout(&output);
    for line in text.lines() {
        let (_, dir) = line.split_once('=').expect("label=dir");
        assert!(
            Path::new(dir)
                .components()
                .any(|c| c.as_os_str() == "comet-example"),
            "{line}"
        );
        assert!(!dir.contains("gitcomet"), "{line}");
    }
    assert_eq!(text.lines().count(), 3, "{text}");
}

#[test]
fn invalid_arguments_exit_with_the_error_code() {
    let home = tempfile::tempdir().unwrap();
    let bad = example(&["mergetool"], home.path());
    assert_eq!(
        bad.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&bad.stderr)
    );
}
