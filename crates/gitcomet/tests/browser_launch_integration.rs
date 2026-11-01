#![cfg(all(target_os = "linux", feature = "ui-gpui-runtime"))]

use rustix::pty::{OpenptFlags, ioctl_tiocgptpeer, openpt, unlockpt};
use std::fs::File;
use std::process::{Command, Output};

#[path = "support/gitcomet_bin.rs"]
mod gitcomet_test_bin;

fn run_without_display(mut command: Command, profile: &std::path::Path) -> Output {
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC).unwrap();
    unlockpt(&master).unwrap();
    let slave = ioctl_tiocgptpeer(
        &master,
        OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC,
    )
    .unwrap();
    command
        .stdin(File::from(slave))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("XDG_SESSION_TYPE")
        .env("GITCOMET_PROFILE_ROOT", profile)
        .env(
            "GITCOMET_BROWSER_INSTANCE_FILE",
            profile.join("instance.json"),
        )
        .env("GITCOMET_FOREGROUND", "0")
        .output()
        .unwrap()
}

#[test]
fn terminal_startup_failure_keeps_diagnostics_and_error_status() {
    let profile = tempfile::tempdir().unwrap();
    let mut command = Command::new(gitcomet_test_bin::gitcomet_bin());
    command.env("GITCOMET_SKIP_TERMINAL_DETACH", "1");
    let output = run_without_display(command, profile.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("Failed to start the browser:"), "{stderr}");
    assert!(stderr.contains("before confirming startup"), "{stderr}");
    assert!(stderr.contains("Failed to launch"), "{stderr}");
}

#[test]
fn library_launch_preserves_supplied_arguments_and_process_state_with_terminal_stdin() {
    const CHILD: &str = "GITCOMET_TEST_LIBRARY_LAUNCH";
    if std::env::var_os(CHILD).is_some() {
        use std::cell::Cell;
        use std::io::IsTerminal;
        use std::rc::Rc;

        assert!(std::io::stdin().is_terminal());
        let prepared = Rc::new(Cell::new(false));
        let seen = prepared.clone();
        let result = gitcomet_app::AppLaunch::gitcomet()
            .on_prepare(move |mode| {
                assert!(
                    matches!(mode, gitcomet_app::AppMode::Browser { path: Some(path) }
                    if path == std::path::Path::new("supplied repo"))
                );
                seen.set(true);
                Ok(())
            })
            .run_with_args(vec![
                "embedded-product".into(),
                "--".into(),
                "supplied repo".into(),
            ]);
        assert_eq!(result, 2);
        assert!(prepared.get());
        return;
    }
    let profile = tempfile::tempdir().unwrap();
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "library_launch_preserves_supplied_arguments_and_process_state_with_terminal_stdin",
            "--nocapture",
        ])
        .env_remove("GITCOMET_SKIP_TERMINAL_DETACH")
        .env(CHILD, "1");
    let output = run_without_display(command, profile.path());
    assert!(output.status.success(), "{output:?}");
}
