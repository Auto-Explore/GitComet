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
    let command = Command::new(gitcomet_test_bin::gitcomet_bin());
    let output = run_without_display(command, profile.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("Failed to start the browser:"), "{stderr}");
    assert!(stderr.contains("before confirming startup"), "{stderr}");
    assert!(stderr.contains("Failed to launch"), "{stderr}");
}

#[test]
fn a_running_browser_receives_requests_before_relaunch_or_gui_preparation() {
    use std::io::{BufRead, BufReader, Write};
    use std::net::{Ipv4Addr, TcpListener};
    use std::time::Duration;

    let profile = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let descriptor = serde_json::json!({
        "version": 1,
        "port": listener.local_addr().unwrap().port(),
        "token": "early-forward-fixture",
        "pid": std::process::id(),
        "product": gitcomet_app::ProductIdentity::gitcomet().app_id(),
    });
    std::fs::write(profile.path().join("instance.json"), descriptor.to_string()).unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(&mut stream).read_line(&mut line).unwrap();
        let request: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(request["token"], "early-forward-fixture");
        stream.write_all(b"ok\n").unwrap();
    });
    let trace_path = profile.path().join("startup.jsonl");
    let mut command = Command::new(gitcomet_test_bin::gitcomet_bin());
    command.env("GITCOMET_STARTUP_TRACE", &trace_path);
    let output = run_without_display(command, profile.path());
    assert!(output.status.success(), "{output:?}");
    server.join().unwrap();
    let trace = std::fs::read_to_string(trace_path).unwrap();
    let names: Vec<String> = trace
        .lines()
        .map(|line| {
            let row: serde_json::Value = serde_json::from_str(line).unwrap();
            row["name"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(names.iter().filter(|name| *name == "app_entry").count(), 1);
    for stage in [
        "crashlog_install",
        "git_executable_preference",
        "native_application",
    ] {
        assert!(!names.iter().any(|name| name == stage), "{stage}: {trace}");
    }
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
        .env(CHILD, "1");
    let output = run_without_display(command, profile.path());
    assert!(output.status.success(), "{output:?}");
}

#[test]
fn a_cli_launch_probes_a_running_browser_only_once() {
    use std::io::{BufRead, BufReader, ErrorKind, Write};
    use std::net::{Ipv4Addr, TcpListener};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::Duration;

    let profile = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let descriptor = serde_json::json!({
        "version": 1,
        "port": listener.local_addr().unwrap().port(),
        "token": "single-probe-fixture",
        "pid": std::process::id(),
        "product": gitcomet_app::ProductIdentity::gitcomet().app_id(),
    });
    std::fs::write(profile.path().join("instance.json"), descriptor.to_string()).unwrap();
    let launch_exited = Arc::new(AtomicBool::new(false));
    let exited = launch_exited.clone();
    // Rejects every request, as a browser that is shutting down does.
    let server = std::thread::spawn(move || {
        let mut connections = 0;
        loop {
            // Read before accepting: once the launch has exited, every
            // connection it made is already waiting in the backlog.
            let drained = exited.load(Ordering::Acquire);
            match listener.accept() {
                Ok((mut stream, _)) => {
                    connections += 1;
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(Duration::from_secs(5)))
                        .unwrap();
                    let mut line = String::new();
                    let _ = BufReader::new(&mut stream).read_line(&mut line);
                    let _ = stream.write_all(b"rejected\n");
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock && drained => {
                    return connections;
                }
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => panic!("fake browser accept failed: {error}"),
            }
        }
    });
    let mut command = Command::new(gitcomet_test_bin::gitcomet_bin());
    command.arg("--foreground");
    let output = run_without_display(command, profile.path());
    launch_exited.store(true, Ordering::Release);
    let connections = server.join().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(2), "{stderr}");
    assert!(stderr.contains("Failed to launch"), "{stderr}");
    assert_eq!(
        connections, 1,
        "the early probe's miss must lead straight to claim arbitration"
    );
}
