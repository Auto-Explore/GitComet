use super::report::*;
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

#[test]
fn runtime_logger_preserves_deduplicated_metadata_without_a_backtrace() {
    const CHILD: &str = "GITCOMET_RUNTIME_LOGGER_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        install();
        begin_session().expect("begin child session");
        let snapshot = EnvironmentSnapshot {
            git_version: Some("runtime logger test git".into()),
            ..Default::default()
        };
        environment::publish(snapshot);
        let record = log::Record::builder()
            .level(log::Level::Error)
            .target("test::recoverable_driver")
            .file(Some("driver.rs"))
            .line(Some(42))
            .args(format_args!("recoverable driver diagnostic"))
            .build();
        log::logger().log(&record);
        log::logger().log(&record);
        // Leave recovery artifacts for the parent to inspect.
        std::process::exit(42);
    }

    let root = tempdir().expect("child profile root");
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "crashlog::tests::runtime_logger_preserves_deduplicated_metadata_without_a_backtrace",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("GITCOMET_PROFILE_ROOT", root.path())
        .output()
        .expect("run child logger");
    assert_eq!(output.status.code(), Some(42));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stderr.matches("recoverable driver diagnostic").count(), 2);

    let dir = root
        .path()
        .join("state")
        .join(identity::current().directory_name())
        .join("crashes");
    let report = take_startup_report_from_crash_dir(&dir).expect("recover runtime error");
    let contents = std::fs::read_to_string(&report.crash_log_path).unwrap();
    assert_eq!(
        contents.matches("=== GitComet runtime error ===").count(),
        1
    );
    let parsed = parse_crash_log(&contents);
    assert_eq!(parsed.failure_kind.as_deref(), Some("runtime-error"));
    assert_eq!(parsed.location.as_deref(), Some("driver.rs#L42"));
    assert_eq!(
        parsed.message.as_deref(),
        Some("recoverable driver diagnostic")
    );
    assert_eq!(
        parsed.info.as_deref(),
        Some("An error was emitted by log target test::recoverable_driver.")
    );
    assert_eq!(
        parsed.environment.unwrap().git_version.as_deref(),
        Some("runtime logger test git")
    );
    assert!(parsed.backtrace.is_empty());
    assert!(!contents.contains("backtrace:"));
}

extern "C" fn panic_across_ffi() {
    panic!("panic across ffi");
}

/// A contained panic that cannot unwind aborts with a second panic. The
/// hook let the first through for `contain`, so the report carried only the
/// second ("panic in a function that cannot unwind"), without the cause.
#[test]
fn only_panics_that_reach_contain_skip_the_crash_report() {
    const CHILD: &str = "GITCOMET_CONTAINED_PANIC_TEST_CHILD";
    if let Ok(kind) = std::env::var(CHILD) {
        install();
        begin_session().expect("begin child session");
        let contained = gitcomet_core::panic::contain(|| panic!("contained test panic"));
        assert_eq!(
            contained.map_err(|panic| panic.message),
            Err("contained test panic".to_owned())
        );
        match kind.as_str() {
            "uncontained" => panic!("uncontained test panic"),
            "cannot-unwind" => {
                let _ = gitcomet_core::panic::contain(|| panic_across_ffi());
            }
            _ => {}
        }
        finish_session().expect("finish child session");
        std::process::exit(0);
    }

    for (kind, message) in [
        ("contained", None),
        ("uncontained", Some("uncontained test panic")),
        ("cannot-unwind", Some("panic across ffi")),
    ] {
        let root = tempdir().expect("child profile root");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crashlog::tests::only_panics_that_reach_contain_skip_the_crash_report",
                "--nocapture",
            ])
            .env(CHILD, kind)
            .env("GITCOMET_PROFILE_ROOT", root.path())
            .output()
            .expect("run contained panic child");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.success(),
            message.is_none(),
            "{kind}: {stderr}"
        );
        assert!(stderr.contains("contained test panic"), "{kind}: {stderr}");
        let dir = root
            .path()
            .join("state")
            .join(identity::current().directory_name())
            .join("crashes");
        let report = take_startup_report_from_crash_dir(&dir);
        let Some(message) = message else {
            assert!(report.is_none(), "{kind}");
            continue;
        };
        let report = report.expect("crash report");
        let parsed = parse_crash_log(&std::fs::read_to_string(report.crash_log_path).unwrap());
        assert_eq!(parsed.failure_kind.as_deref(), Some("panic"), "{kind}");
        assert_eq!(parsed.message.as_deref(), Some(message), "{kind}");
        assert!(
            parsed
                .location
                .as_deref()
                .and_then(|location| location.rsplit_once("#L"))
                // Component-wise: Windows reports `crashlog\tests.rs`.
                .is_some_and(|(file, line)| {
                    std::path::Path::new(file).ends_with("crashlog/tests.rs")
                        && line.parse::<u32>().is_ok()
                }),
            "{kind}: {:?}",
            parsed.location
        );
    }
}

#[test]
fn panic_and_fatal_launch_reports_keep_backtraces() {
    const CHILD: &str = "GITCOMET_FATAL_BACKTRACE_TEST_CHILD";
    if let Ok(kind) = std::env::var(CHILD) {
        install();
        begin_session().expect("begin child session");
        if kind == "panic" {
            panic!("fatal backtrace test panic");
        }
        record_session_failure("test launch", "fatal backtrace test returned error")
            .expect("record fatal launch failure");
        std::process::exit(42);
    }

    for kind in ["panic", "returned-error"] {
        let root = tempdir().expect("child profile root");
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crashlog::tests::panic_and_fatal_launch_reports_keep_backtraces",
                "--nocapture",
            ])
            .env(CHILD, kind)
            .env("GITCOMET_PROFILE_ROOT", root.path())
            .output()
            .expect("run fatal failure child");
        assert_eq!(
            output.status.code(),
            Some(if kind == "panic" { 101 } else { 42 })
        );
        let dir = root
            .path()
            .join("state")
            .join(identity::current().directory_name())
            .join("crashes");
        let report = take_startup_report_from_crash_dir(&dir).expect("recover fatal failure");
        let parsed = parse_crash_log(&std::fs::read_to_string(report.crash_log_path).unwrap());
        assert_eq!(parsed.failure_kind.as_deref(), Some(kind));
        assert!(
            !parsed.backtrace.trim().is_empty(),
            "missing {kind} backtrace"
        );
        assert!(parsed.location.is_some());
    }
}

#[test]
fn percent_encode_encodes_reserved_characters() {
    assert_eq!(percent_encode("a b&c/d"), "a%20b%26c%2Fd");
}

#[test]
fn build_issue_url_uses_package_repository_issue_endpoint() {
    let url = build_issue_url("Crash: boom", "details");
    let expected_prefix = format!("{}/issues/new?", env!("CARGO_PKG_REPOSITORY"));
    assert!(url.starts_with(&expected_prefix));
}

#[test]
fn parse_crash_log_extracts_fields() {
    let log = r#"=== GitComet crash (panic) ===
timestamp_unix_ms=123
crate=gitcomet version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
failure_context=main GPUI window launch
copy_source=commit-details-diff
clipboard_backend=x11
backtrace:
frame 1
frame 2
"#;

    let parsed = parse_crash_log(log);
    assert_eq!(parsed.failure_kind.as_deref(), Some("panic"));
    assert_eq!(parsed.timestamp_unix_ms.as_deref(), Some("123"));
    assert_eq!(parsed.crate_name.as_deref(), Some("gitcomet"));
    assert_eq!(parsed.crate_version.as_deref(), Some("0.1.0"));
    assert_eq!(parsed.thread.as_deref(), Some("main"));
    assert_eq!(parsed.location.as_deref(), Some("src/main.rs#L42"));
    assert_eq!(parsed.message.as_deref(), Some("boom happened"));
    assert_eq!(parsed.info.as_deref(), Some("panic info"));
    assert_eq!(
        parsed.failure_context.as_deref(),
        Some("main GPUI window launch")
    );
    assert_eq!(parsed.copy_source.as_deref(), Some("commit-details-diff"));
    assert_eq!(parsed.clipboard_backend.as_deref(), Some("x11"));
    assert!(parsed.backtrace.contains("frame 1"));
    assert!(parsed.backtrace.contains("frame 2"));
}

#[test]
fn parse_crash_log_supports_inline_backtrace_header() {
    let log = "message=boom\nbacktrace:frame 1\nframe 2\n";
    let parsed = parse_crash_log(log);
    assert_eq!(parsed.message.as_deref(), Some("boom"));
    assert!(parsed.backtrace.contains("frame 1"));
    assert!(parsed.backtrace.contains("frame 2"));
}

#[test]
fn build_startup_report_populates_issue_url_and_summary() {
    let log = r#"timestamp_unix_ms=123
crate=gitcomet version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
backtrace:
frame 1
frame 2
"#;
    let report = build_startup_report(PathBuf::from("/tmp/panic.log"), log);
    assert!(report.issue_url.contains("template=crash_report.md"));
    assert!(
        report
            .issue_url
            .contains("title=Crash%3A%20boom%20happened")
    );
    assert!(report.summary.contains("boom happened"));
    assert!(report.summary.contains("src/main.rs#L42"));
}

#[test]
fn take_startup_report_returns_none_without_recovery_state() {
    let dir = tempdir().expect("temp dir");
    assert!(take_startup_report_from_crash_dir(dir.path()).is_none());
}

#[test]
fn panic_report_is_snapshotted_before_pending_marker_is_removed() {
    let dir = tempdir().expect("temp dir");
    let crash_log_path = dir.path().join("panic.log");
    let crash_log = r#"timestamp_unix_ms=123
crate=gitcomet version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
backtrace:
frame 1
frame 2
"#;
    std::fs::write(&crash_log_path, crash_log).expect("write crash log");
    write_pending_report_path(&pending_report_path(dir.path()), &crash_log_path)
        .expect("write pending marker");

    let report =
        take_startup_report_from_crash_dir(dir.path()).expect("startup report should be available");
    assert_eq!(report.crash_log_path, startup_report_path(dir.path()));
    assert!(report.issue_url.contains("template=crash_report.md"));
    assert!(report.summary.contains("boom happened"));
    assert!(
        !pending_report_path(dir.path()).exists(),
        "the panic marker should be removed after a durable snapshot is written"
    );
    assert!(
        !crash_log_path.exists(),
        "the source panic log should be retired after it is snapshotted"
    );
    assert_eq!(
        std::fs::read_to_string(startup_report_path(dir.path()))
            .expect("read persisted startup report"),
        crash_log
    );
}

#[test]
fn missing_panic_log_clears_pending_marker() {
    let dir = tempdir().expect("temp dir");
    let missing_log_path = dir.path().join("missing.log");
    write_pending_report_path(&pending_report_path(dir.path()), &missing_log_path)
        .expect("write pending marker");

    assert!(take_startup_report_from_crash_dir(dir.path()).is_none());
    assert!(
        !pending_report_path(dir.path()).exists(),
        "pending marker should be removed even when crash log is missing"
    );
}

#[test]
fn previous_session_report_is_snapshotted_before_marker_is_removed() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    let last_operation = last_operation_path(dir.path());
    std::fs::write(
        &marker,
        "timestamp_unix_ms=123\ncrate=gitcomet version=0.1.0\n\
         thread=main\nmessage=GitComet did not exit cleanly\n\
         info=The previous UI process ended before shutdown.\n",
    )
    .expect("write session marker");
    std::fs::write(
        &last_operation,
        "copy_source=commit-details-diff\ncopy_text_bytes=128\n",
    )
    .expect("write last operation");

    let report = take_startup_report_from_crash_dir(dir.path())
        .expect("stale session should produce a startup report");

    assert_eq!(report.crash_log_path, startup_report_path(dir.path()));
    assert!(report.summary.contains("did not exit cleanly"));
    assert!(report.issue_url.contains("commit-details-diff"));
    assert!(
        !marker.exists(),
        "session marker should be removed after its report is persisted"
    );
    assert!(
        !last_operation.exists(),
        "last-operation diagnostics should be included in the persisted report"
    );
    let startup_report =
        std::fs::read_to_string(startup_report_path(dir.path())).expect("read startup report");
    assert!(startup_report.contains("copy_source=commit-details-diff"));
}

#[test]
fn runtime_error_supplies_failure_metadata_without_a_backtrace() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    let last_operation = last_operation_path(dir.path());
    std::fs::write(
        &marker,
        "=== GitComet abnormal exit candidate ===\n\
         failure_kind=returned-error\nfailure_context=main GPUI event loop\n\
         message=GPUI event loop ended unexpectedly\n",
    )
    .expect("write session marker");
    std::fs::write(
        &last_operation,
        "copy_source=diff-context-menu\nclipboard_backend=x11\n",
    )
    .expect("write last operation");
    write_runtime_error_log_in_dir(
        dir.path(),
        "crates/gpui_linux/src/linux/wayland/client.rs#L993",
        "Io error: Connection reset by peer (os error 104)",
        "An error was emitted by log target gpui_linux::linux::wayland::client.",
    )
    .expect("write runtime error");

    let report = take_startup_report_from_crash_dir(dir.path())
        .expect("runtime error should produce a startup report");
    let parsed = parse_crash_log(
        &std::fs::read_to_string(&report.crash_log_path).expect("read startup report"),
    );

    assert_eq!(parsed.failure_kind.as_deref(), Some("runtime-error"));
    assert_eq!(
        parsed.location.as_deref(),
        Some("crates/gpui_linux/src/linux/wayland/client.rs#L993")
    );
    assert_eq!(
        parsed.message.as_deref(),
        Some("Io error: Connection reset by peer (os error 104)")
    );
    assert_eq!(
        parsed.failure_context.as_deref(),
        Some("main GPUI event loop")
    );
    assert_eq!(parsed.copy_source.as_deref(), Some("diff-context-menu"));
    assert_eq!(parsed.clipboard_backend.as_deref(), Some("x11"));
    assert!(parsed.backtrace.is_empty());
    assert_eq!(
        parsed.info.as_deref(),
        Some("An error was emitted by log target gpui_linux::linux::wayland::client.")
    );
    assert!(report.summary.contains("Connection reset by peer"));
    assert!(report.issue_url.contains("wayland%2Fclient.rs%23L993"));
    assert!(
        report
            .issue_url
            .contains("Clipboard%20backend%3A%20%60x11%60")
    );
    assert!(!runtime_error_path(dir.path()).exists());
}

#[test]
fn newest_failure_after_a_backtrace_is_parsed_as_a_separate_event() {
    let log = r#"=== GitComet crash (panic) ===
failure_kind=panic
location=src/old.rs#L1
message=old panic
backtrace:
old frame

=== GitComet abnormal exit candidate ===
failure_kind=returned-error
failure_context=main GPUI event loop
copy_source=diff-context-menu
message=generic event-loop failure

=== GitComet runtime error ===
failure_kind=runtime-error
location=crates/gpui_linux/src/linux/wayland/client.rs#L993
message=Io error: Connection reset by peer
backtrace:
new frame
"#;

    let parsed = parse_crash_log(log);
    assert_eq!(parsed.failure_kind.as_deref(), Some("runtime-error"));
    assert_eq!(
        parsed.location.as_deref(),
        Some("crates/gpui_linux/src/linux/wayland/client.rs#L993")
    );
    assert_eq!(
        parsed.message.as_deref(),
        Some("Io error: Connection reset by peer")
    );
    assert_eq!(parsed.copy_source.as_deref(), Some("diff-context-menu"));
    assert_eq!(
        parsed.failure_context.as_deref(),
        Some("main GPUI event loop")
    );
    assert!(parsed.backtrace.contains("new frame"));
    assert!(!parsed.backtrace.contains("old frame"));
}

#[test]
fn handled_launch_failure_is_reported_at_next_startup() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    std::fs::write(
        &marker,
        "timestamp_unix_ms=123\ncrate=gitcomet version=0.1.0\n\
         thread=main\nmessage=GitComet did not exit cleanly\n",
    )
    .expect("write session marker");

    record_session_failure_in_dir(
        dir.path(),
        "main GPUI window launch",
        "main GPUI window launch failed: no compatible graphics device",
    )
    .expect("record launch failure");

    let report = take_startup_report_from_crash_dir(dir.path())
        .expect("handled launch failure should produce a startup report");

    assert!(report.summary.contains("no compatible graphics device"));
    assert!(report.issue_url.contains("main%20GPUI%20window%20launch"));
    assert!(
        !marker.exists(),
        "session marker should be removed after its report is persisted"
    );
    assert!(
        startup_report_path(dir.path()).exists(),
        "the report must survive until the user handles the notification"
    );
}

#[test]
fn handled_failure_diagnostics_include_observation_location_and_backtrace() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    std::fs::write(
        &marker,
        "=== GitComet abnormal exit candidate ===\nmessage=initial marker\n",
    )
    .expect("write session marker");
    std::fs::write(
        last_operation_path(dir.path()),
        "copy_source=diff-context-menu\n",
    )
    .expect("write operation context");

    record_session_failure_in_dir_with_diagnostics(
        dir.path(),
        "main GPUI window launch",
        "no compatible graphics device",
        Some("src/main.rs#L275"),
        Some("observation frame 1\nobservation frame 2"),
    )
    .expect("record handled failure");

    let report = take_startup_report_from_crash_dir(dir.path())
        .expect("handled failure should produce a startup report");
    let parsed = parse_crash_log(
        &std::fs::read_to_string(&report.crash_log_path).expect("read startup report"),
    );
    assert_eq!(parsed.failure_kind.as_deref(), Some("returned-error"));
    assert_eq!(parsed.location.as_deref(), Some("src/main.rs#L275"));
    assert_eq!(parsed.copy_source.as_deref(), Some("diff-context-menu"));
    assert!(parsed.backtrace.contains("observation frame 1"));
}

#[test]
fn pending_startup_report_survives_a_failed_relaunch() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    std::fs::write(
        &marker,
        "message=GitComet did not exit cleanly\ncopy_source=commit-details-diff\n",
    )
    .expect("write initial session marker");

    take_startup_report_from_crash_dir(dir.path())
        .expect("initial abnormal exit should create a startup report");
    begin_session_in_dir(dir.path()).expect("begin retry session");
    assert!(
        startup_report_path(dir.path()).exists(),
        "beginning a retry must not discard the prior report"
    );
    record_session_failure_in_dir(
        dir.path(),
        "main GPUI window launch",
        "main GPUI window launch failed: no compatible graphics device",
    )
    .expect("record retry failure");

    let report = take_startup_report_from_crash_dir(dir.path())
        .expect("retry failure should preserve a startup report");
    assert!(report.summary.contains("no compatible graphics device"));
    let persisted =
        std::fs::read_to_string(startup_report_path(dir.path())).expect("read startup report");
    assert!(persisted.contains("copy_source=commit-details-diff"));
    assert!(persisted.contains("no compatible graphics device"));
}

#[test]
fn clean_shutdown_keeps_unacknowledged_startup_report() {
    let dir = tempdir().expect("temp dir");
    std::fs::write(startup_report_path(dir.path()), "message=previous crash\n")
        .expect("write startup report");
    begin_session_in_dir(dir.path()).expect("begin session");

    finish_session_in_dir(dir.path()).expect("finish session");

    assert!(
        startup_report_path(dir.path()).exists(),
        "a report is retired only when the user handles the notification"
    );
}

#[test]
fn begin_session_creates_a_missing_crash_dir() {
    let root = tempdir().expect("temp dir");
    let dir = root.path().join("state/gitcomet/crashes");

    begin_session_in_dir(&dir).expect("begin session below a missing dir");

    assert!(session_marker_path(&dir).exists());
}

#[cfg(unix)]
#[test]
fn diagnostics_still_land_in_a_symlinked_crash_dir() {
    use std::os::unix::fs::PermissionsExt as _;

    // A state dir relocated with a symlink is supported; losing logs is not.
    let root = tempdir().expect("temp dir");
    let real = root.path().join("real-state");
    std::fs::create_dir(&real).expect("create state dir");
    let mode = std::fs::metadata(&real)
        .expect("state dir")
        .permissions()
        .mode();
    let dir = root.path().join("crashes");
    std::os::unix::fs::symlink(&real, &dir).expect("symlink");

    begin_session_in_dir(&dir).expect("begin session in a symlinked dir");
    write_runtime_error_log_in_dir(&dir, "target", "boom", "")
        .expect("runtime error log in a symlinked dir");

    assert!(session_marker_path(&real).exists());
    assert!(runtime_error_path(&real).exists());
    assert_eq!(
        std::fs::metadata(&real).unwrap().permissions().mode(),
        mode,
        "a directory the user pointed us at keeps its own permissions"
    );
}

#[test]
fn begin_session_starts_with_fresh_diagnostics() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    let last_operation = last_operation_path(dir.path());
    let runtime_error = runtime_error_path(dir.path());
    std::fs::write(&marker, "message=previous abnormal exit\n").expect("write old marker");
    std::fs::write(&last_operation, "copy_source=commit-details-diff\n")
        .expect("write last-operation diagnostics");
    std::fs::write(&runtime_error, "message=previous runtime error\n")
        .expect("write runtime-error diagnostics");

    begin_session_in_dir(dir.path()).expect("begin session");

    let contents = std::fs::read_to_string(&marker).expect("read session marker");
    assert!(contents.contains("=== GitComet abnormal exit candidate ==="));
    assert!(
        !contents.contains("previous abnormal exit"),
        "a new session must replace an already-recovered marker"
    );
    assert!(!last_operation.exists());
    assert!(!runtime_error.exists());
}

#[test]
fn panic_report_takes_precedence_over_previous_session_marker() {
    let dir = tempdir().expect("temp dir");
    let crash_log_path = dir.path().join("panic.log");
    std::fs::write(&crash_log_path, "message=panic wins\n").expect("write panic log");
    write_pending_report_path(&pending_report_path(dir.path()), &crash_log_path)
        .expect("write pending marker");
    let marker = session_marker_path(dir.path());
    std::fs::write(&marker, "message=GitComet did not exit cleanly\n")
        .expect("write session marker");

    let panic_report =
        take_startup_report_from_crash_dir(dir.path()).expect("panic report should be available");
    assert!(panic_report.summary.contains("panic wins"));
    assert!(
        !marker.exists(),
        "the stale marker should be removed after the combined report is persisted"
    );
    let persisted =
        std::fs::read_to_string(startup_report_path(dir.path())).expect("read startup report");
    assert!(
        persisted.contains("message=GitComet did not exit cleanly"),
        "the persisted panic report should retain the stale-session context"
    );
}

#[test]
fn startup_recovery_consumes_a_stale_session_marker() {
    let dir = tempdir().expect("temp dir");
    let marker = session_marker_path(dir.path());
    std::fs::write(&marker, "message=GitComet did not exit cleanly\n")
        .expect("write previous session marker");

    let report = take_startup_report_from_crash_dir(dir.path())
        .expect("the marker should produce a startup report");
    assert!(report.summary.contains("did not exit cleanly"));
    assert!(
        !marker.exists(),
        "stale recovery state should be removed after it is snapshotted"
    );
}

#[test]
fn startup_recovery_ignores_live_process_markers() {
    let dir = tempdir().expect("temp dir");
    let live_pid = 41;
    let stale_pid = 42;
    let live_marker = session_marker_path_for_pid(dir.path(), live_pid);
    let live_operation = last_operation_path_for_pid(dir.path(), live_pid);
    let live_runtime_error = runtime_error_path_for_pid(dir.path(), live_pid);
    let stale_marker = session_marker_path_for_pid(dir.path(), stale_pid);
    let stale_operation = last_operation_path_for_pid(dir.path(), stale_pid);

    std::fs::write(&live_marker, "message=live GitComet session\n").expect("write live marker");
    std::fs::write(&live_operation, "copy_source=live-instance\n")
        .expect("write live operation diagnostics");
    std::fs::write(&live_runtime_error, "message=live runtime error\n")
        .expect("write live runtime diagnostics");
    std::fs::write(&stale_marker, "message=stale GitComet session\n").expect("write stale marker");
    std::fs::write(&stale_operation, "copy_source=stale-instance\n")
        .expect("write stale operation diagnostics");

    let report =
        take_startup_report_from_crash_dir_with_process_check(dir.path(), |pid| pid == live_pid)
            .expect("the stale marker should produce a report");

    let persisted = std::fs::read_to_string(report.crash_log_path).expect("read startup report");
    assert!(persisted.contains("stale GitComet session"));
    assert!(persisted.contains("copy_source=stale-instance"));
    assert!(!persisted.contains("live GitComet session"));
    assert!(live_marker.exists());
    assert!(live_operation.exists());
    assert!(live_runtime_error.exists());
    assert!(!stale_marker.exists());
    assert!(!stale_operation.exists());
}

#[test]
fn session_diagnostic_paths_include_the_owning_pid() {
    let dir = Path::new("/tmp/gitcomet-crash-path-test");
    assert_eq!(
        session_marker_path_for_pid(dir, 123),
        dir.join("session-in-progress-123.log")
    );
    assert_eq!(
        last_operation_path_for_pid(dir, 123),
        dir.join("last-operation-123.log")
    );
    assert_eq!(
        runtime_error_path_for_pid(dir, 123),
        dir.join("last-runtime-error-123.log")
    );
}

#[test]
fn clean_shutdown_removes_session_diagnostics() {
    let dir = tempdir().expect("temp dir");
    begin_session_in_dir(dir.path()).expect("begin session");
    let marker = session_marker_path(dir.path());
    let last_operation = last_operation_path(dir.path());
    let runtime_error = runtime_error_path(dir.path());
    std::fs::write(&last_operation, "copy_source=commit-details-diff\n")
        .expect("write operation diagnostics");
    std::fs::write(&runtime_error, "message=current runtime error\n")
        .expect("write current runtime error");

    finish_session_in_dir(dir.path()).expect("finish current session");

    assert!(!marker.exists());
    assert!(!last_operation.exists());
    assert!(!runtime_error.exists());
}

#[cfg(unix)]
#[test]
fn terminal_interrupt_clears_active_session_state() {
    const CHILD_PROCESS: &str = "GITCOMET_SIGINT_CLEANUP_TEST_CHILD";
    const SIGINT_CHILD: &str = "sigint";
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    const PROCESS_EXIT_CONTROL_CHILD: &str = "process-exit-control";

    let child_mode = std::env::var_os(CHILD_PROCESS);
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if child_mode.as_deref() == Some(std::ffi::OsStr::new(PROCESS_EXIT_CONTROL_CHILD)) {
        std::process::exit(130);
    }

    if child_mode.is_some() {
        install_terminal_interrupt_handler().expect("install SIGINT handler");
        begin_session().expect("begin child session");
        let dir = crash_dir().expect("child crash directory");
        std::fs::write(last_operation_path(&dir), "copy_source=test\n")
            .expect("write child operation diagnostics");
        std::fs::write(runtime_error_path(&dir), "message=test runtime error\n")
            .expect("write child runtime diagnostics");
        signal_hook::low_level::raise(signal_hook::consts::signal::SIGINT).expect("raise SIGINT");
        loop {
            std::thread::park();
        }
    }

    let state_root = tempdir().expect("temporary state root");

    // glibc's loader diagnostics give the test a safe way to distinguish
    // normal process exit from raw `_exit`: the control child must run
    // native finalizers, while the SIGINT child must not run any.
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .arg("terminal_interrupt_clears_active_session_state")
            .env(CHILD_PROCESS, PROCESS_EXIT_CONTROL_CHILD)
            .env("LD_DEBUG", "files")
            .output()
            .expect("run process-exit control child");
        assert_eq!(output.status.code(), Some(130));
        assert!(loader_diagnostics_show_finalizers(&output.stderr));
    }

    let mut command = std::process::Command::new(std::env::current_exe().expect("test executable"));
    command
        .arg("terminal_interrupt_clears_active_session_state")
        .env(CHILD_PROCESS, SIGINT_CHILD)
        .env("XDG_STATE_HOME", state_root.path());
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    command
        .env("LD_DEBUG", "files")
        .stderr(std::process::Stdio::piped());
    let child = command.spawn().expect("run SIGINT cleanup child");
    let child_pid = child.id();
    let output = child
        .wait_with_output()
        .expect("wait for SIGINT cleanup child");

    assert_eq!(output.status.code(), Some(130));
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    assert!(!loader_diagnostics_show_finalizers(&output.stderr));
    let dir = state_root.path().join("gitcomet").join("crashes");
    assert!(!session_marker_path_for_pid(&dir, child_pid).exists());
    assert!(!last_operation_path_for_pid(&dir, child_pid).exists());
    assert!(!runtime_error_path_for_pid(&dir, child_pid).exists());
}

#[cfg(all(unix, target_os = "linux", target_env = "gnu"))]
fn loader_diagnostics_show_finalizers(stderr: &[u8]) -> bool {
    String::from_utf8_lossy(stderr)
        .lines()
        .any(|line| line.contains("calling fini:"))
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

#[cfg(windows)]
#[test]
fn terminal_interrupt_clears_active_session_state() {
    let dir = tempdir().expect("temp dir");
    begin_session_in_dir(dir.path()).expect("begin session");
    std::fs::write(last_operation_path(dir.path()), "copy_source=test\n")
        .expect("write operation diagnostics");
    SESSION_ACTIVE.store(true, Ordering::SeqCst);

    retire_active_session_in_dir(Some(dir.path()));

    assert!(!SESSION_ACTIVE.load(Ordering::SeqCst));
    assert!(!session_marker_path(dir.path()).exists());
    assert!(!last_operation_path(dir.path()).exists());
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
        std::fs::write(last_operation_path(&dir), "copy_source=test\n")
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
    let marker = session_marker_path_for_pid(&dir, child_pid);

    let ready_deadline = std::time::Instant::now() + CHILD_READY_TIMEOUT;
    while !marker.exists() && std::time::Instant::now() < ready_deadline {
        std::thread::sleep(POLL_INTERVAL);
    }
    assert!(
        marker.exists(),
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
    assert!(!marker.exists());
    assert!(!last_operation_path_for_pid(&dir, child_pid).exists());
    assert!(!runtime_error_path_for_pid(&dir, child_pid).exists());
}

#[test]
fn build_issue_body_trims_very_long_backtrace() {
    let parsed = ParsedCrashLog {
        backtrace: "x".repeat(MAX_BACKTRACE_CHARS + 128),
        ..Default::default()
    };

    let body = build_issue_body(&parsed, Path::new("/tmp/panic.log"));
    let marker = "## Backtrace (trimmed)\n\n```text\n";
    let start = body.find(marker).expect("backtrace section should exist") + marker.len();
    let end = start
        + body[start..]
            .find("\n```")
            .expect("backtrace code block should close");
    let backtrace_text = &body[start..end];

    assert_eq!(backtrace_text.chars().count(), MAX_BACKTRACE_CHARS);
    assert!(backtrace_text.ends_with("..."));
}
