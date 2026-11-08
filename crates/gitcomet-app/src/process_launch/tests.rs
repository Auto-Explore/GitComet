use super::*;
use std::ffi::OsStr;
use std::io::IsTerminal;
use std::os::unix::ffi::OsStringExt;

const CHILD_TEST: &str =
    "process_launch::tests::startup_acknowledgement_leaves_the_session_running";

fn child_command(scenario: &str, startup: &StartupListener, report: &Path) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD_TEST, "--nocapture"])
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
        "unacknowledged" => {
            assert!(notify_ready(&endpoint).is_err());
            eprintln!("written after the handoff");
            std::process::exit(0);
        }
        #[cfg(target_os = "linux")]
        "stderr-exhausted" => {
            use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};

            let limit = getrlimit(Resource::Nofile);
            setrlimit(
                Resource::Nofile,
                Rlimit {
                    current: Some(limit.current.unwrap_or(64).min(64)),
                    maximum: limit.maximum,
                },
            )
            .unwrap();
            let mut descriptors = Vec::new();
            loop {
                match std::fs::File::open("/dev/null") {
                    Ok(file) => descriptors.push(file),
                    Err(error) => {
                        assert_eq!(
                            error.raw_os_error(),
                            Some(rustix::io::Errno::MFILE.raw_os_error())
                        );
                        break;
                    }
                }
            }
            // Opening /dev/null succeeds, but duplicating stderr exhausts
            // the limit. Closing /dev/null then permits the startup socket.
            drop(descriptors.pop().expect("filled descriptor slots"));
            assert!(notify_ready(&endpoint).is_err());
            std::process::exit(2);
        }
        "cancel" => {
            crate::crashlog::install();
            crate::crashlog::begin_session().unwrap();
            std::fs::write(report.with_extension("session-started"), b"started").unwrap();
            let signal: i32 = std::env::var("GITCOMET_TEST_STARTUP_CANCEL")
                .unwrap()
                .parse()
                .unwrap();
            rustix::process::kill_process(
                rustix::process::getppid().unwrap(),
                Signal::from_named_raw(signal).unwrap(),
            )
            .unwrap();
        }
        "ready" | "retained-stderr" | "large-output" => {
            assert!(!std::io::stdin().is_terminal());
            assert!(!std::io::stdout().is_terminal());
            assert!(std::fs::File::open("/dev/tty").is_err());
            let pid = rustix::process::getpid();
            assert_eq!(rustix::process::getsid(None).unwrap(), pid);
            assert_eq!(rustix::process::getpgrp(), pid);
            if scenario == "retained-stderr" {
                #[allow(clippy::zombie_processes)] // The descendant deliberately retains stderr.
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

#[cfg(target_os = "linux")]
#[test]
fn stderr_detachment_failure_does_not_acknowledge_startup() {
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("pid");
    let startup = StartupListener::new().unwrap();
    let command = child_command("stderr-exhausted", &startup, &report);
    let error = supervise(command, startup, STARTUP_TIMEOUT).unwrap_err();
    assert!(error.to_string().contains("before confirming startup"));
    let stderr = std::fs::read_to_string(report.with_extension("stderr")).unwrap();
    assert!(
        stderr.contains("Could not detach browser stderr:"),
        "{stderr}"
    );
}

#[test]
fn startup_deadline_and_spawn_failure_are_errors() {
    let directory = tempfile::tempdir().unwrap();
    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = child_command("timeout", &startup, &directory.path().join("pid"));
    let error = supervise(command, startup, Duration::ZERO).unwrap_err();
    assert!(
        matches!(&error, LaunchError::Startup(error) if error.kind() == io::ErrorKind::TimedOut),
        "{error:?}"
    );
    assert!(!endpoint.exists());

    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = Command::new(directory.path().join("missing-executable"));
    let error = supervise(command, startup, STARTUP_TIMEOUT).unwrap_err();
    assert!(
        matches!(&error, LaunchError::Setup(error) if error.kind() == io::ErrorKind::NotFound),
        "{error:?}"
    );
    assert!(!endpoint.exists());
}

#[test]
fn startup_cancellation_stops_and_reaps_the_child() {
    const CANCEL_TEST: &str =
        "process_launch::tests::startup_cancellation_stops_and_reaps_the_child";
    let Some(signal) = std::env::var_os("GITCOMET_TEST_STARTUP_CANCEL") else {
        // A terminal can end the launcher with any of these; the browser
        // must retire its marker for each.
        for signal in [
            signal_hook::consts::SIGINT,
            signal_hook::consts::SIGTERM,
            signal_hook::consts::SIGHUP,
        ] {
            let output = Command::new(std::env::current_exe().unwrap())
                .args(["--exact", CANCEL_TEST, "--nocapture"])
                .env("GITCOMET_TEST_STARTUP_CANCEL", signal.to_string())
                .output()
                .unwrap();
            assert!(output.status.success(), "signal {signal}: {output:?}");
        }
        return;
    };
    let signal: i32 = signal.to_str().unwrap().parse().unwrap();

    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("pid");
    let startup = StartupListener::new().unwrap();
    let endpoint = startup.endpoint();
    let command = child_command("cancel", &startup, &report);
    assert_eq!(
        supervise(command, startup, STARTUP_TIMEOUT).unwrap(),
        128 + signal
    );
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
        "cancelling the launch must retire the pending startup's recovery marker"
    );
}

#[test]
fn detached_launch_keeps_supplied_argument_boundaries() {
    let repository = OsString::from_vec(b"repo \xff".to_vec());
    let args = vec!["custom-product".into(), "--".into(), repository.clone()];
    let endpoint = Path::new("/run/gc-start/ready");
    let command = browser_command(Path::new("/custom/product"), endpoint, &args);
    assert_eq!(command.get_program(), "/custom/product");
    let mut child_args = vec![OsString::from("custom-product")];
    child_args.extend(command.get_args().map(OsStr::to_owned));
    assert_eq!(
        child_args[1..],
        [
            OsString::from("--__gitcomet-startup-socket"),
            endpoint.into(),
            "--".into(),
            repository.clone(),
        ]
    );
    let parsed = crate::cli::parse_launch(child_args, crate::cli::DEFAULT_ABOUT).unwrap();
    assert_eq!(parsed.startup_socket.as_deref(), Some(endpoint));
    assert!(matches!(
        parsed.outcome,
        crate::cli::CliOutcome::Run(crate::cli::AppMode::Browser { path: Some(path) })
            if path.as_os_str() == repository
    ));
}

#[test]
fn startup_socket_skips_unusable_and_too_long_directories() {
    let directory = tempfile::tempdir().unwrap();
    let too_long = directory.path().join("d".repeat(MAX_SOCKET_PATH));
    std::fs::create_dir(&too_long).unwrap();
    let missing = directory.path().join("missing");
    let error = StartupListener::new_in(&[missing.clone(), too_long.clone()])
        .err()
        .expect("no usable directory");
    assert_eq!(error.kind(), io::ErrorKind::InvalidInput, "{error}");
    let startup = StartupListener::new_in(&[missing, too_long, directory.path().into()]).unwrap();
    assert!(startup.endpoint().starts_with(directory.path()));
}

#[test]
fn signals_fall_back_to_their_default_action_after_supervision() {
    const CHILD: &str = "GITCOMET_TEST_SIGNAL_DEFAULT";
    const TEST: &str =
        "process_launch::tests::signals_fall_back_to_their_default_action_after_supervision";
    if std::env::var_os(CHILD).is_some() {
        drop(StartupSignals::new().unwrap());
        rustix::process::kill_process(rustix::process::getpid(), Signal::TERM).unwrap();
        std::thread::sleep(Duration::from_secs(5));
        return;
    }
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "1")
        .status()
        .unwrap();
    use std::os::unix::process::ExitStatusExt as _;
    assert_eq!(
        status.signal(),
        Some(signal_hook::consts::SIGTERM),
        "{status:?}"
    );
}

#[test]
fn ctrl_c_reaches_the_crash_handler_after_a_failed_detach() {
    const CHILD: &str = "GITCOMET_TEST_FOREGROUND_INTERRUPT";
    const TEST: &str =
        "process_launch::tests::ctrl_c_reaches_the_crash_handler_after_a_failed_detach";
    if std::env::var_os(CHILD).is_some() {
        // A spawn failure falls back to the foreground, which installs crash
        // logging next and opens a session.
        let startup = StartupListener::new().unwrap();
        let missing = Command::new("/nonexistent/gitcomet-startup-fixture");
        assert!(matches!(
            supervise(missing, startup, STARTUP_TIMEOUT),
            Err(LaunchError::Setup(_))
        ));
        crate::crashlog::install();
        crate::crashlog::begin_session().unwrap();
        rustix::process::kill_process(rustix::process::getpid(), Signal::INT).unwrap();
        std::thread::sleep(Duration::from_secs(5));
        return;
    }
    let profile = tempfile::tempdir().unwrap();
    let status = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", TEST, "--nocapture"])
        .env(CHILD, "1")
        .env("GITCOMET_PROFILE_ROOT", profile.path())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(130), "{status:?}");
    let crashes = profile.path().join("state/gitcomet/crashes");
    assert!(
        std::fs::read_dir(crashes).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("session-in-progress")),
        "Ctrl-C must retire the foreground session's recovery marker"
    );
}

#[test]
fn a_lost_acknowledgement_still_hands_off_startup() {
    let mut startup = StartupListener::new().unwrap();
    let mut client = UnixStream::connect(startup.endpoint()).unwrap();
    client.write_all(READY).unwrap();
    drop(client);
    let deadline = Instant::now() + Duration::from_secs(5);
    while !startup
        .poll()
        .expect("READY arrived, so the browser owns the session")
    {
        assert!(Instant::now() < deadline, "READY was never read");
        std::thread::sleep(SUPERVISOR_POLL);
    }
}

#[test]
fn a_failed_handoff_reports_to_the_original_stderr_then_stays_quiet() {
    let directory = tempfile::tempdir().unwrap();
    let report = directory.path().join("pid");
    let endpoint = directory.path().join("ready");
    let listener = UnixListener::bind(&endpoint).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", CHILD_TEST, "--nocapture"])
        .env("GITCOMET_TEST_STARTUP_SCENARIO", "unacknowledged")
        .env("GITCOMET_TEST_STARTUP_ENDPOINT", &endpoint)
        .env("GITCOMET_TEST_STARTUP_REPORT", &report)
        .stdout(Stdio::null())
        .stderr(std::fs::File::create(report.with_extension("stderr")).unwrap())
        .spawn()
        .unwrap();
    let (mut stream, _) = listener.accept().unwrap();
    let mut ready = [0; READY.len()];
    stream.read_exact(&mut ready).unwrap();
    assert_eq!(ready, READY);
    drop(stream);
    assert!(child.wait().unwrap().success());
    let stderr = std::fs::read_to_string(report.with_extension("stderr")).unwrap();
    assert!(
        stderr.contains("Could not acknowledge browser startup"),
        "{stderr}"
    );
    assert!(!stderr.contains("written after the handoff"), "{stderr}");
}
