//! The example product launches under its own identity and paths using the
//! upstream crates, with no product feature flags.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

fn example(args: &[&str], home: &Path) -> Output {
    example_at(Path::new(env!("CARGO_BIN_EXE_comet-example")), args, home)
}

fn example_at(executable: &Path, args: &[&str], home: &Path) -> Output {
    Command::new(executable)
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

fn assert_example_tool_commands(dry_run: &str) {
    let mut keys = BTreeSet::new();
    for line in dry_run.lines() {
        let key = line
            .strip_prefix("git config --global ")
            .and_then(|args| args.split_whitespace().find(|arg| !arg.starts_with('-')));
        if let Some(key) = key.filter(|key| key.ends_with(".cmd")) {
            keys.insert(key);
        } else {
            assert!(!line.contains("gitcomet"), "{line}");
        }
    }
    assert_eq!(
        keys,
        BTreeSet::from([
            "difftool.comet-example-gui.cmd",
            "difftool.comet-example.cmd",
            "mergetool.comet-example-gui.cmd",
            "mergetool.comet-example.cmd",
        ])
    );
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
    let home = tempfile::Builder::new()
        .prefix("gitcomet-parent-")
        .tempdir()
        .unwrap();
    let source = Path::new(env!("CARGO_BIN_EXE_comet-example"));
    let executable = home.path().join(source.file_name().unwrap());
    std::fs::copy(source, &executable).unwrap();
    let setup = example_at(&executable, &["setup", "--dry-run"], home.path());
    assert_eq!(setup.status.code(), Some(0));
    let setup = stdout(&setup);
    assert_example_tool_commands(&setup);
    assert!(setup.contains("comet-example.backup.*"), "{setup}");

    let uninstall = example_at(&executable, &["uninstall", "--dry-run"], home.path());
    assert!(uninstall.status.success());
    assert_example_tool_commands(&stdout(&uninstall));
}

#[test]
fn state_data_and_crash_directories_are_the_examples_own() {
    let home = tempfile::Builder::new()
        .prefix("gitcomet-parent-")
        .tempdir()
        .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_comet-example"))
        .env("COMET_EXAMPLE_PRINT_DIRS", "1")
        .env("HOME", home.path())
        .env("XDG_STATE_HOME", home.path().join("state"))
        .env("XDG_DATA_HOME", home.path().join("data"))
        .env("LOCALAPPDATA", home.path().join("local"))
        .env_remove("GITCOMET_PROFILE_ROOT")
        .output()
        .expect("run comet-example");
    let text = stdout(&output);
    assert!(output.status.success(), "{text}");
    for (line, label) in text.lines().zip(["state", "data", "crash"]) {
        let (actual_label, dir) = line.split_once('=').expect("label=dir");
        assert_eq!(actual_label, label);
        let dir = Path::new(dir);
        assert!(dir.starts_with(home.path()), "{line}");
        let product_dir = if label == "crash" {
            assert_eq!(dir.file_name(), Some(OsStr::new("crashes")));
            dir.parent().unwrap()
        } else {
            dir
        };
        assert_eq!(product_dir.file_name(), Some(OsStr::new("comet-example")));
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
