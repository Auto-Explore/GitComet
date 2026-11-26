use super::*;
use tempfile::tempdir;

#[test]
fn ignores_gpui_window_notification_after_window_teardown() {
    assert!(!should_record_runtime_error(
        "gpui::window",
        "window not found"
    ));
    assert!(!should_record_runtime_error(
        "gpui::window",
        "  window not found\n"
    ));
}

#[test]
fn ignores_non_fatal_egl_native_fence_validation_error() {
    assert!(!should_record_runtime_error(
        "wgpu_hal::gles::egl",
        "EGL 'eglCreateSyncKHR' code 0x3004: EGL_BAD_ATTRIBUTE error: In \
         eglCreateSyncKHR: EGL_SYNC_NATIVE_FENCE_FD_ANDROID specified valid fd butEGL_SYNC_STATUS \
         is also being set"
    ));
}

#[test]
fn retains_other_runtime_errors() {
    assert!(should_record_runtime_error(
        "gpui::window",
        "rendering failed"
    ));
    assert!(should_record_runtime_error(
        "gitcomet::window",
        "window not found"
    ));
}

fn take_report(dir: &Path) -> Option<StartupCrashReport> {
    session::recover(dir, |_, _| None).unwrap()
}

fn profile_crash_dir(profile: &Path) -> PathBuf {
    profile
        .join("state")
        .join(identity::current().directory_name())
        .join("crashes")
}

fn marker_exists(dir: &Path, prefix: &str) -> bool {
    std::fs::read_dir(dir).ok().is_some_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(prefix) && name.ends_with(".v1.json")
        })
    })
}

#[test]
fn fatal_panic_keeps_the_payload_and_stays_dismissed() {
    const CHILD: &str = "GITCOMET_FATAL_PANIC_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        install();
        begin_session().unwrap();
        let dir = crash_dir().unwrap();
        std::fs::write(
            last_operation_path_for_session(&dir, current_session_token()),
            "copy_source=commit-details-diff\n",
        )
        .unwrap();
        std::process::exit(crate::exit_on_panic(|| {
            panic!("fatal subprocess panic {}", std::process::id());
        }));
    }

    let profile = tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crashlog::tests::fatal_panic_keeps_the_payload_and_stays_dismissed",
        ])
        .env(CHILD, "1")
        .env("GITCOMET_PROFILE_ROOT", profile.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101), "{:?}", output);
    let dir = profile_crash_dir(profile.path());
    let crash = session::recover(&dir, |_, _| panic!("confirmed panic needs no OS lookup"))
        .unwrap()
        .unwrap();
    assert_eq!(
        crash.snapshot.reason(),
        gitcomet_core::startup_exit::StartupExitReason::RustPanic
    );
    assert!(crash.summary.contains("fatal subprocess panic"));
    assert!(crash.issue_url.contains("commit-details-diff"));
    assert!(!crash.issue_url.contains("crashlog%2Ftests.rs"));
    assert!(
        !crash
            .issue_url
            .contains(&crash.snapshot.session_token().unwrap().to_string())
    );
    assert_eq!(take_report(&dir).unwrap().snapshot, crash.snapshot);
    crash.snapshot.acknowledge().unwrap();
    assert!(take_report(&dir).is_none());
}

#[test]
fn a_literal_panic_is_confirmed_and_uncorrelated_details_stay_local() {
    const CHILD: &str = "GITCOMET_LITERAL_PANIC_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        install();
        begin_session().unwrap();
        let code = crate::exit_on_panic(|| panic!("literal callback panic"));
        finish_session().unwrap();
        std::process::exit(code);
    }
    let profile = tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crashlog::tests::a_literal_panic_is_confirmed_and_uncorrelated_details_stay_local",
        ])
        .env(CHILD, "1")
        .env("GITCOMET_PROFILE_ROOT", profile.path())
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(101), "{output:?}");
    let dir = profile_crash_dir(profile.path());
    let crash = take_report(&dir).unwrap();
    assert_eq!(
        crash.snapshot.reason(),
        gitcomet_core::startup_exit::StartupExitReason::RustPanic
    );
    assert!(crash.summary.contains("literal callback panic"));
    assert!(crash.snapshot.panic_capture_id().is_none());
    assert!(!crash.issue_url.contains("tests.rs"));
    let local = std::fs::read_to_string(&crash.crash_log_path).unwrap();
    assert!(local.contains("last_observed_panic"));
    assert!(local.contains("literal callback panic"));
    assert!(local.contains("tests.rs#L"));
}

#[cfg(unix)]
#[test]
fn unix_interrupt_and_external_stops_never_produce_crash_reports() {
    use signal_hook::consts::signal::{SIGHUP, SIGINT, SIGKILL, SIGTERM};
    use std::os::unix::process::ExitStatusExt;

    const CHILD: &str = "GITCOMET_EXTERNAL_STOP_TEST_CHILD";
    if let Ok(mode) = std::env::var(CHILD) {
        install();
        begin_session().unwrap();
        let record = log::Record::builder()
            .level(log::Level::Error)
            .target("test::graphics")
            .file(Some("driver.rs"))
            .args(format_args!("recoverable driver diagnostic"))
            .build();
        log::logger().log(&record);
        log::logger().log(&record);
        let worker = std::thread::spawn(|| {
            let _ =
                std::panic::catch_unwind(|| panic!("caught worker panic {}", std::process::id()));
        });
        worker.join().unwrap();
        signal_hook::low_level::raise(mode.parse().unwrap()).unwrap();
        loop {
            std::thread::park();
        }
    }
    for signal in [SIGTERM, SIGHUP, SIGKILL, SIGINT] {
        let profile = tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crashlog::tests::unix_interrupt_and_external_stops_never_produce_crash_reports",
            ])
            .env(CHILD, signal.to_string())
            .env("GITCOMET_PROFILE_ROOT", profile.path())
            .output()
            .unwrap();
        if signal == SIGINT {
            assert_eq!(output.status.code(), Some(130));
        } else {
            assert_eq!(output.status.signal(), Some(signal));
        }
        let dir = profile_crash_dir(profile.path());
        assert!(take_startup_report_from_profile(&dir).is_none());
        assert!(!marker_exists(&dir, "session-in-progress-"));
        let manual = std::fs::read_to_string(dir.join("latest-session.log")).unwrap();
        assert!(manual.contains("caught worker panic"));
        assert_eq!(manual.matches("recoverable driver diagnostic").count(), 1);
    }
}

#[cfg(unix)]
fn take_startup_report_from_profile(dir: &Path) -> Option<StartupCrashReport> {
    session::recover(dir, native::find).unwrap()
}

#[test]
fn old_reports_and_pending_panic_paths_stay_available_without_a_prompt() {
    let dir = tempdir().unwrap();
    for (file, contents) in [
        (
            "pending-startup-report.log",
            "=== GitComet abnormal exit candidate ===\nfailure_kind=abnormal-exit\nmessage=GitComet did not exit cleanly\n",
        ),
        (
            "session-in-progress-222.log",
            "failure_kind=returned-error\n",
        ),
        ("panic-222-1.log", "failure_kind=panic\nmessage=old panic\n"),
        ("pending-report-path.txt", "panic-222-1.log"),
    ] {
        std::fs::write(dir.path().join(file), contents).unwrap();
    }
    let recovered = session::recover(dir.path(), |_, _| {
        panic!("legacy files must not trigger an OS lookup")
    })
    .unwrap();
    assert!(recovered.is_none());
    assert!(dir.path().join("pending-startup-report.log").exists());
    assert!(dir.path().join("panic-222-1.log").exists());
}

#[cfg(windows)]
#[test]
fn console_control_events_that_terminate_are_not_reported_as_crashes() {
    use gitcomet_win32_window_utils::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };

    for event in [
        CTRL_C_EVENT,
        CTRL_BREAK_EVENT,
        CTRL_CLOSE_EVENT,
        CTRL_LOGOFF_EVENT,
        CTRL_SHUTDOWN_EVENT,
    ] {
        assert!(console_ctrl_event_ends_process(event));
    }
    assert!(!console_ctrl_event_ends_process(u32::MAX));
    // `handle_console_ctrl_event` itself ends the process for each of these
    // events, so it can only be exercised from a child process. See
    // `terminal_interrupt_ends_process_and_clears_active_session_state`.
}

/// A console control event must end the process and leave no recovery
/// marker behind. The handler ends the process, so it can only be exercised
/// from a child. The deadlock this guards against needs a full UI process to
/// reproduce; what is checked here is the contract the handler must keep.
#[cfg(windows)]
#[test]
fn terminal_interrupt_ends_process_and_clears_active_session_state() {
    use std::os::windows::process::CommandExt;

    const CHILD_PROCESS: &str = "GITCOMET_CTRL_BREAK_CLEANUP_TEST_CHILD";
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CHILD_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const CHILD_EXIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

    if std::env::var_os(CHILD_PROCESS).is_some() {
        assert!(
            install_terminal_interrupt_handler(),
            "install console control handler"
        );
        begin_session().expect("begin child session");
        let dir = crash_dir().expect("child crash directory");
        std::fs::write(
            last_operation_path_for_session(&dir, current_session_token()),
            "copy_source=test\n",
        )
        .expect("write child operation diagnostics");
        std::fs::write(runtime_error_path(&dir), "message=test runtime error\n")
            .expect("write child runtime diagnostics");
        loop {
            std::thread::park();
        }
    }

    let state_root = tempdir().expect("temporary state root");
    // A new process group keeps the Ctrl+Break event off the test runner
    // itself, which shares this console with the child.
    let mut child = std::process::Command::new(std::env::current_exe().expect("test executable"))
        .arg("terminal_interrupt_ends_process_and_clears_active_session_state")
        .env(CHILD_PROCESS, "1")
        .env("LOCALAPPDATA", state_root.path())
        .creation_flags(CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .expect("run console control cleanup child");
    let child_pid = child.id();
    let dir = state_root.path().join("gitcomet").join("crashes");
    let prefix = format!("session-in-progress-{child_pid}-");

    let ready_deadline = std::time::Instant::now() + CHILD_READY_TIMEOUT;
    while !marker_exists(&dir, &prefix) && std::time::Instant::now() < ready_deadline {
        std::thread::sleep(POLL_INTERVAL);
    }
    assert!(
        marker_exists(&dir, &prefix),
        "child should register its recovery marker before the interrupt"
    );

    if !gitcomet_win32_window_utils::send_console_ctrl_break(child_pid) {
        // Test runners without a console cannot deliver control events.
        let _ = child.kill();
        let _ = child.wait();
        eprintln!(
            "skipping console control interrupt test: {}",
            std::io::Error::last_os_error()
        );
        return;
    }

    let exit_deadline = std::time::Instant::now() + CHILD_EXIT_TIMEOUT;
    let status = loop {
        match child.try_wait().expect("poll console control child") {
            Some(status) => break status,
            None if std::time::Instant::now() >= exit_deadline => {
                let _ = child.kill();
                let _ = child.wait();
                panic!("console control event did not end the child process");
            }
            None => std::thread::sleep(POLL_INTERVAL),
        }
    };

    assert_eq!(
        status.code(),
        Some(gitcomet_win32_window_utils::CONTROL_C_EXIT_CODE as i32)
    );
    assert!(!marker_exists(&dir, &prefix));
    assert!(take_report(&dir).is_none());
}
