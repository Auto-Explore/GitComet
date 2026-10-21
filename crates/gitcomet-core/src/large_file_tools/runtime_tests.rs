use super::*;
use crate::process::GitExecutableAvailability;
use std::path::PathBuf;
use std::sync::Mutex;

fn available_runtime() -> GitRuntimeState {
    GitRuntimeState {
        preference: GitExecutablePreference::Custom(PathBuf::from("captured-git-A")),
        availability: GitExecutableAvailability::Available {
            version_output: "git version 2.50.0".into(),
        },
    }
}

#[test]
#[ignore = "native subprocess fixture for frozen-runtime detector validation"]
fn frozen_runtime_native_child() {
    let root = PathBuf::from(std::env::var_os("GITCOMET_FROZEN_TOOL_FIXTURE").unwrap());
    let args: Vec<_> = std::env::args().collect();
    let tool = args
        .iter()
        .find(|arg| *arg == "lfs" || *arg == "annex")
        .unwrap();
    assert!(args.iter().any(|arg| arg == "version"));
    if tool == "annex" {
        assert!(args.iter().any(|arg| arg == "--raw"));
    }
    std::fs::write(root.join(tool), b"invoked").unwrap();
    println!("fixture-{tool}-version");
}

#[test]
fn frozen_runtime_is_passed_to_both_native_subcommands() {
    let captured = available_runtime();
    let dir = tempfile::tempdir().unwrap();
    let preferences = Mutex::new(Vec::new());
    let state = detect_for_runtime_with_factory(
        &captured,
        &|preference| {
            preferences.lock().unwrap().push(preference.clone());
            let mut command = crate::process::background_command(std::env::current_exe().unwrap());
            command
                .args([
                    "--ignored",
                    "--exact",
                    "large_file_tools::runtime_tests::frozen_runtime_native_child",
                    "--nocapture",
                    "--",
                ])
                .env("GITCOMET_FROZEN_TOOL_FIXTURE", dir.path());
            command
        },
        &CancellationToken::new(),
    );
    assert_eq!(
        *preferences.lock().unwrap(),
        vec![captured.preference.clone(), captured.preference]
    );
    assert!(dir.path().join("lfs").exists());
    assert!(dir.path().join("annex").exists());
    assert!(matches!(state.git_lfs, ToolAvailability::Available { .. }));
    assert!(matches!(
        state.git_annex,
        ToolAvailability::Available { .. }
    ));
}

#[test]
fn unavailable_frozen_runtime_never_constructs_commands() {
    let mut runtime = available_runtime();
    runtime.availability = GitExecutableAvailability::Checking;
    let token = CancellationToken::new();
    assert_eq!(
        detect_for_runtime_with_factory(
            &runtime,
            &|_| panic!("unavailable runtime spawned"),
            &token
        ),
        LargeFileToolsState::default(),
    );
    assert_eq!(
        detect_large_file_tools_for_runtime_cancellable(&runtime, &token),
        LargeFileToolsState::default()
    );
}

#[test]
fn cancelled_frozen_runtime_never_constructs_commands() {
    let runtime = available_runtime();
    let token = CancellationToken::new();
    token.cancel();
    assert_eq!(
        detect_for_runtime_with_factory(&runtime, &|_| panic!("cancelled request spawned"), &token),
        LargeFileToolsState::default(),
    );
    assert_eq!(
        detect_large_file_tools_for_runtime_cancellable(&runtime, &token),
        LargeFileToolsState::default()
    );
}
