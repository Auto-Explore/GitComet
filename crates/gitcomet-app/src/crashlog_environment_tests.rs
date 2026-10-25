use super::*;
use gitcomet_core::environment::{GraphicsDetails, Rendering};

fn snapshot(software: bool) -> EnvironmentSnapshot {
    let mut snapshot = EnvironmentSnapshot {
        app_version: "0.2.5".into(),
        git_version: Some("git version 2.51.0".into()),
        ..Default::default()
    };
    snapshot.system.operating_system = Some("Recorded distribution 42".into());
    snapshot.graphics.insert(
        1,
        GraphicsDetails {
            device_name: Some(if software { "llvmpipe" } else { "hardware GPU" }.into()),
            backend: Some(if software { "Vulkan" } else { "OpenGL" }.into()),
            rendering: if software {
                Rendering::Software
            } else {
                Rendering::Hardware
            },
            ..Default::default()
        },
    );
    snapshot
}

#[test]
fn recovered_environment_is_owned_by_failed_pid_and_survives_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let failed_pid = 123;
    let live_pid = 456;
    for (pid, software) in [(failed_pid, true), (live_pid, false)] {
        std::fs::write(
            session_marker_path_for_pid(dir.path(), pid),
            "=== GitComet abnormal exit candidate ===\nmessage=lost process\n",
        )
        .unwrap();
        std::fs::write(
            environment_path_for_pid(dir.path(), pid),
            serde_json::to_vec(&snapshot(software)).unwrap(),
        )
        .unwrap();
    }
    let report =
        take_startup_report_from_crash_dir_with_process_check(dir.path(), |pid| pid == live_pid)
            .unwrap();
    let contents = std::fs::read_to_string(&report.crash_log_path).unwrap();
    let parsed = parse_crash_log(&contents);
    assert_eq!(parsed.environment, Some(snapshot(true)));
    let body = build_issue_body(&parsed, &report.crash_log_path);
    assert!(body.contains(&snapshot(true).summary()));
    assert!(body.contains("- GitComet version: `0.2.5`"));
    assert!(!body.contains("hardware GPU"));
    assert!(!environment_path_for_pid(dir.path(), failed_pid).exists());
    assert!(environment_path_for_pid(dir.path(), live_pid).exists());
    assert!(session_marker_path_for_pid(dir.path(), live_pid).exists());
    let next =
        take_startup_report_from_crash_dir_with_process_check(dir.path(), |pid| pid == live_pid)
            .unwrap();
    assert_eq!(report, next);
}

#[test]
fn runtime_error_snapshot_is_not_overwritten_by_a_pending_background_write() {
    let dir = tempfile::tempdir().unwrap();
    begin_session_in_dir(dir.path()).unwrap();
    persist_environment(dir.path(), &snapshot(false)).unwrap();
    let mut log = b"=== GitComet runtime error ===\nmessage=renderer failed\n".to_vec();
    write_environment(&mut log, &snapshot(true)).unwrap();
    log.extend_from_slice(b"backtrace:\nframe\n");
    std::fs::write(runtime_error_path(dir.path()), log).unwrap();
    let report = take_startup_report_from_crash_dir(dir.path()).unwrap();
    let parsed = parse_crash_log(&std::fs::read_to_string(report.crash_log_path).unwrap());
    assert_eq!(parsed.environment, Some(snapshot(true)));
}

#[test]
fn old_and_malformed_reports_do_not_inherit_another_process_environment() {
    let mut log = b"=== GitComet crash (panic) ===\n".to_vec();
    write_environment(&mut log, &snapshot(true)).unwrap();
    log.extend_from_slice(
        b"=== GitComet crash (panic) ===\ncrate=gitcomet version=0.1.0\nmessage=old panic\n",
    );
    let parsed = parse_crash_log(std::str::from_utf8(&log).unwrap());
    assert!(parsed.environment.is_none());
    let body = build_issue_body(&parsed, Path::new("old.log"));
    assert!(body.contains("- OS: `Unavailable`"));
    assert!(!body.contains("llvmpipe"));
    let legacy = parse_crash_log("os=old-os\narch=old-arch\n");
    let body = build_issue_body(&legacy, Path::new("old.log"));
    assert!(body.contains("- OS: `old-os`"));
    assert!(body.contains("- Arch: `old-arch`"));
    assert!(body.contains("- GitComet version: `Unavailable`"));

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(session_marker_path(dir.path()), "message=old exit\n").unwrap();
    std::fs::write(
        environment_path_for_pid(dir.path(), std::process::id()),
        "{truncated",
    )
    .unwrap();
    assert!(take_startup_report_from_crash_dir(dir.path()).is_some());
}

#[test]
fn environment_is_replaced_for_reused_pid_and_removed_on_clean_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    persist_environment(dir.path(), &snapshot(true)).unwrap();
    begin_session_in_dir(dir.path()).unwrap();
    let path = environment_path_for_pid(dir.path(), std::process::id());
    let fresh: EnvironmentSnapshot =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_ne!(fresh, snapshot(true));
    finish_session_in_dir(dir.path()).unwrap();
    assert!(!path.exists());
}

/// The child has no Git on PATH and never initializes GPUI. Both crash paths
/// must use only the supplied cache, including when the next launch uses a
/// different renderer. A real panic exercises the installed hook.
#[cfg(target_os = "linux")]
#[test]
fn panic_and_abnormal_exit_keep_cached_environment() {
    const CHILD: &str = "GITCOMET_ENVIRONMENT_CRASH_CHILD";
    if let Ok(kind) = std::env::var(CHILD) {
        install();
        begin_session().unwrap();
        environment::publish(snapshot(true));
        let dir = crash_dir().unwrap();
        let path = environment_path_for_pid(&dir, std::process::id());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if std::fs::read(&path)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<EnvironmentSnapshot>(&bytes).ok())
                == Some(snapshot(true))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background environment writer timed out"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        write_runtime_error_log_in_dir(&dir, "test", "renderer error", "driver failed").unwrap();
        let runtime = std::fs::read_to_string(runtime_error_path(&dir)).unwrap();
        assert_eq!(parse_crash_log(&runtime).environment, Some(snapshot(true)));
        if kind == "panic" {
            panic!("cached environment panic");
        }
        std::process::exit(42);
    }
    for kind in ["panic", "abnormal"] {
        let root = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "crashlog::environment_tests::panic_and_abnormal_exit_keep_cached_environment",
                "--nocapture",
            ])
            .env(CHILD, kind)
            .env("XDG_STATE_HOME", root.path())
            .env("PATH", "")
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(if kind == "panic" { 101 } else { 42 }),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let dir = root.path().join("gitcomet/crashes");
        // Simulate the new launch's hardware renderer before recovering.
        begin_session_in_dir(&dir).unwrap();
        persist_environment(&dir, &snapshot(false)).unwrap();
        let report = take_startup_report_from_crash_dir_with_process_check(&dir, |pid| {
            pid == std::process::id()
        })
        .unwrap();
        let log = std::fs::read_to_string(&report.crash_log_path).unwrap();
        let parsed = parse_crash_log(&log);
        assert_eq!(parsed.environment, Some(snapshot(true)), "{kind}: {log}");
        assert!(report.issue_url.contains("Software%20%28CPU%29"));
        assert!(environment_path_for_pid(&dir, std::process::id()).exists());
        if kind == "panic" {
            assert_eq!(parsed.failure_kind.as_deref(), Some("panic"));
        }
    }
}
