use super::*;
use std::io::IsTerminal;
use std::os::unix::ffi::OsStringExt;

const CHILD_TEST: &str =
    "process_launch::tests::startup_acknowledgement_leaves_the_session_running";

fn child_command(scenario: &str, startup: &StartupListener, report: &Path) -> Command {
    let executable = std::env::current_exe().unwrap();
    let args = [
        executable.as_os_str().to_owned(),
        "--exact".into(),
        CHILD_TEST.into(),
        "--nocapture".into(),
    ];
    let mut command = relaunch_command(executable.as_os_str(), &args);
    command
        .env("GITCOMET_TEST_STARTUP_SCENARIO", scenario)
        .env("GITCOMET_TEST_STARTUP_ENDPOINT", startup.endpoint())
        .env("GITCOMET_TEST_STARTUP_REPORT", report)
        .env("GITCOMET_PROFILE_ROOT", report.parent().unwrap())
        .stderr(std::fs::File::create(report.with_extension("stderr")).unwrap());
    command
}

fn run_child(scenario: &str) -> ! {
    if scenario == "hold-stderr" {
        loop {
            std::thread::park();
        }
    }
    let endpoint = PathBuf::from(std::env::var_os("GITCOMET_TEST_STARTUP_ENDPOINT").unwrap());
    let report = PathBuf::from(std::env::var_os("GITCOMET_TEST_STARTUP_REPORT").unwrap());
    std::fs::write(&report, std::process::id().to_string()).unwrap();
    eprintln!("startup diagnostic from {scenario}");
    match scenario {
        "exit-zero" => std::process::exit(0),
        "exit-error" => std::process::exit(2),
        "panic" => panic!("startup panic fixture"),
        "malformed" => {
            UnixStream::connect(endpoint)
                .unwrap()
                .write_all(b"BAD READY\n")
                .unwrap();
        }
        "timeout" => {}
        "cancel" => {
            crate::crashlog::install();
            crate::crashlog::begin_session().unwrap();
            std::fs::write(report.with_extension("session-started"), b"started").unwrap();
            rustix::process::kill_process(rustix::process::getppid().unwrap(), Signal::INT)
                .unwrap();
        }
        "ready" | "retained-stderr" | "large-output" => {
            assert!(!std::io::stdin().is_terminal());
            assert!(!std::io::stdout().is_terminal());
            assert!(std::fs::File::open("/dev/tty").is_err());
            let pid = rustix::process::getpid();
            assert_eq!(rustix::process::getsid(None).unwrap(), pid);
            assert_eq!(rustix::process::getpgrp(), pid);
            let shell = Command::new("/bin/sh")
                .args(["-c", "test -z \"${GITCOMET_SKIP_TERMINAL_DETACH+x}\""])
                .status()
                .unwrap();
            assert!(
                shell.success(),
                "the internal detach guard reached a user shell"
            );
            if scenario == "retained-stderr" {
                let _descendant = Command::new(std::env::current_exe().unwrap())
                    .args(["--exact", CHILD_TEST, "--nocapture"])
                    .env("GITCOMET_TEST_STARTUP_SCENARIO", "hold-stderr")
                    .spawn()
                    .unwrap();
            }
            if scenario == "large-output" {
                std::io::stderr()
                    .write_all(&vec![b'x'; 256 * 1024])
                    .unwrap();
                eprintln!("last startup diagnostic");
            }
            notify_ready_io(&endpoint).unwrap();
        }
        other => panic!("unknown startup scenario {other}"),
    }
    loop {
        std::thread::park();
    }
}

struct RunningTestChild(Pid);

impl Drop for RunningTestChild {
    fn drop(&mut self) {
        let _ = kill_process_group(self.0, Signal::KILL);
        let _ = rustix::process::waitpid(Some(self.0), rustix::process::WaitOptions::empty());
    }
}

fn verify_success(scenario: &str) {
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("pid");
    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = child_command(scenario, &startup, &report);
    let result = supervise(command, startup, STARTUP_TIMEOUT);
    let pid: i32 = std::fs::read_to_string(&report).unwrap().parse().unwrap();
    let child = RunningTestChild(Pid::from_raw(pid).unwrap());
    assert_eq!(result.unwrap(), 0);
    assert_eq!(rustix::process::getsid(Some(child.0)).unwrap(), child.0);
    assert!(
        !endpoint.exists(),
        "startup transport outlived the launcher"
    );
    let diagnostics = std::fs::read_to_string(report.with_extension("stderr")).unwrap();
    assert!(diagnostics.contains(&format!("startup diagnostic from {scenario}")));
    if scenario == "large-output" {
        assert!(diagnostics.contains("last startup diagnostic"));
    }
}

#[test]
fn startup_acknowledgement_leaves_the_session_running() {
    if let Ok(scenario) = std::env::var("GITCOMET_TEST_STARTUP_SCENARIO") {
        run_child(&scenario);
    }
    verify_success("ready");
}

#[test]
fn inherited_stderr_descriptors_do_not_delay_readiness() {
    verify_success("retained-stderr");
}

#[test]
fn large_startup_diagnostics_are_relayed_without_deadlocking() {
    verify_success("large-output");
}

#[test]
fn startup_failure_is_visible_even_when_the_child_exits_zero() {
    for scenario in ["exit-zero", "exit-error", "panic", "malformed"] {
        let directory = tempfile::tempdir().unwrap();
        let report = directory.path().join("pid");
        let startup = StartupListener::new().unwrap();
        let endpoint = startup.endpoint();
        let command = child_command(scenario, &startup, &report);
        let error = supervise(command, startup, STARTUP_TIMEOUT).unwrap_err();
        assert!(
            error.to_string().contains(if scenario == "malformed" {
                "invalid browser startup message"
            } else {
                "before confirming startup"
            }),
            "{scenario}: {error}"
        );
        assert!(
            std::fs::read_to_string(report.with_extension("stderr"))
                .unwrap()
                .contains(&format!("startup diagnostic from {scenario}"))
        );
        assert!(!endpoint.exists());
        let pid: i32 = std::fs::read_to_string(report).unwrap().parse().unwrap();
        assert!(rustix::process::test_kill_process(Pid::from_raw(pid).unwrap()).is_err());
    }
}

#[test]
fn startup_deadline_and_spawn_failure_are_errors() {
    let directory = tempfile::tempdir().unwrap();
    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = child_command("timeout", &startup, &directory.path().join("pid"));
    let error = supervise(command, startup, Duration::ZERO).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(!endpoint.exists());

    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = Command::new(directory.path().join("missing-executable"));
    let error = supervise(command, startup, STARTUP_TIMEOUT).unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::NotFound);
    assert!(!endpoint.exists());
}

#[test]
fn startup_cancellation_stops_and_reaps_the_child() {
    const CANCEL_TEST: &str =
        "process_launch::tests::startup_cancellation_stops_and_reaps_the_child";
    if std::env::var_os("GITCOMET_TEST_STARTUP_CANCEL").is_none() {
        let output = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", CANCEL_TEST, "--nocapture"])
            .env("GITCOMET_TEST_STARTUP_CANCEL", "1")
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("pid");
    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = child_command("cancel", &startup, &report);
    assert_eq!(supervise(command, startup, STARTUP_TIMEOUT).unwrap(), 130);
    let pid: i32 = std::fs::read_to_string(&report).unwrap().parse().unwrap();
    assert!(rustix::process::test_kill_process(Pid::from_raw(pid).unwrap()).is_err());
    assert!(!endpoint.exists());
    assert!(report.with_extension("session-started").exists());
    let crashes = directory.path().join("state/gitcomet/crashes");
    assert!(
        std::fs::read_dir(crashes).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("session-in-progress")),
        "Ctrl-C must retire the pending startup's recovery marker"
    );
}

#[test]
fn relaunch_keeps_supplied_argument_boundaries_and_removes_stale_guard() {
    let args = vec![
        "custom-product".into(),
        "--".into(),
        OsString::from_vec(b"repo \xff".to_vec()),
    ];
    let command = relaunch_command(OsStr::new("/custom/product"), &args);
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        args[1..]
            .iter()
            .map(OsString::as_os_str)
            .collect::<Vec<_>>()
    );
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "GITCOMET_SKIP_TERMINAL_DETACH" && value.is_none())
    );
}
