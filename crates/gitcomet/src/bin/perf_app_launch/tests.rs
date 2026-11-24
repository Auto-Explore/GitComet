use super::*;
use serde_json::Value;
use tempfile::tempdir;

fn pipe_fixture_command(mode: &str) -> Command {
    let mut command = Command::new(env::current_exe().unwrap());
    command
        .args(["--exact", "tests::launch_pipe_fixture", "--nocapture"])
        .env("GITCOMET_LAUNCH_PIPE_FIXTURE", mode);
    command
}

#[test]
#[allow(clippy::zombie_processes)] // Exercise a leader exiting before its descendant.
fn launch_pipe_fixture() {
    match env::var("GITCOMET_LAUNCH_PIPE_FIXTURE").as_deref() {
        Ok("parent") => {
            let mut command = pipe_fixture_command("descendant");
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt as _;
                // Reproduce app exit during a background probe's job setup.
                command.creation_flags(0x0800_0004); // NO_WINDOW | SUSPENDED
            }
            let _descendant = command.spawn().unwrap();
        }
        Ok("descendant") => thread::sleep(Duration::from_secs(60)),
        _ => {} // Ordinary test runs do not launch a fixture.
    }
}

#[cfg(windows)]
#[test]
fn launch_child_cleanup_after_repeated_exit_polls_finishes() {
    let (tx, rx) = mpsc::channel();
    let worker = thread::spawn(move || {
        let mut command = pipe_fixture_command("exit");
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = LaunchChild::spawn(command).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while child.try_wait_stopping_tree().unwrap().is_none() {
            assert!(Instant::now() < deadline, "fixture leader did not exit");
            thread::sleep(Duration::from_millis(5));
        }
        // Drain every queued notification in the old implementation,
        // including ACTIVE_PROCESS_ZERO, before exercising the warm-up
        // probe's unconditional cleanup.
        for _ in 0..128 {
            assert!(child.try_wait().unwrap().unwrap().success());
        }
        tx.send(terminate_child(&mut child)).unwrap();
    });
    assert!(
        rx.recv_timeout(Duration::from_secs(10))
            .expect("cleanup must not wait for an already-consumed job notification")
            .unwrap()
            .success()
    );
    worker.join().unwrap();
}

#[cfg(unix)]
#[test]
fn launch_child_stops_its_group_before_reaping_the_leader() {
    let mut command = Command::new("sh");
    command.args(["-c", "exit 3"]).stdin(Stdio::null());
    let mut child = LaunchChild::spawn(command).unwrap();
    let pid = child.id();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !leader_exited_unreaped(pid).unwrap() {
        assert!(Instant::now() < deadline, "leader did not exit");
        thread::sleep(Duration::from_millis(5));
    }
    // Peeking must not reap: the zombie keeps the group ID reserved.
    assert!(leader_exited_unreaped(pid).unwrap());
    let raw = rustix::process::Pid::from_raw(pid as i32).unwrap();
    rustix::process::test_kill_process(raw).expect("the zombie leader still exists");
    assert!(!child.stopped);
    let status = child.try_wait_stopping_tree().unwrap().unwrap();
    assert_eq!(status.code(), Some(3));
    assert!(child.stopped, "the tree is stopped before the reap");
}

#[test]
fn exited_launch_child_cannot_leave_descendants_holding_stderr() {
    let mut command = pipe_fixture_command("parent");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = LaunchChild::spawn(command).unwrap();
    let mut pipe = child.stderr().take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        use std::io::Read as _;
        let mut bytes = Vec::new();
        let result = pipe.read_to_end(&mut bytes);
        tx.send(result).unwrap();
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline, "fixture leader did not exit");
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        rx.try_recv().is_err(),
        "descendant must retain stderr before cleanup"
    );
    child.stop_tree();
    rx.recv_timeout(Duration::from_secs(3))
        .expect("owned descendants must release stderr")
        .unwrap();
    reader.join().unwrap();
}

#[test]
fn parse_cli_args_defaults_to_harness_mode() {
    let cli = parse_cli_args(Vec::<String>::new()).expect("parse");
    assert!(!cli.child);
    assert!(!cli.preflight_only);
    assert_eq!(cli.bench, DEFAULT_BENCH);
    assert_eq!(cli.timeout_ms, DEFAULT_TIMEOUT_MS);
}

#[test]
fn parse_cli_args_accepts_overrides() {
    let cli = parse_cli_args(vec![
        "--child".to_string(),
        "--bench".to_string(),
        "app_launch/cold_single_repo".to_string(),
        "--timeout-ms".to_string(),
        "1500".to_string(),
    ])
    .expect("parse");
    assert!(cli.child);
    assert!(!cli.preflight_only);
    assert_eq!(cli.bench, "app_launch/cold_single_repo");
    assert_eq!(cli.timeout_ms, 1500);
}

#[test]
fn parse_cli_args_accepts_preflight_only() {
    let cli = parse_cli_args(vec![
        "--preflight-only".to_string(),
        "--bench".to_string(),
        "app_launch/cold_single_repo".to_string(),
    ])
    .expect("parse");
    assert!(!cli.child);
    assert!(cli.preflight_only);
    assert_eq!(cli.bench, "app_launch/cold_single_repo");
}

#[test]
fn parse_cli_args_rejects_preflight_only_with_child() {
    let err = parse_cli_args(vec!["--child".to_string(), "--preflight-only".to_string()])
        .expect_err("reject incompatible flags");
    assert!(err.contains("--preflight-only"));
}

#[test]
fn launch_scenario_supports_single_and_five_repo_cases() {
    assert_eq!(
        LaunchScenario::from_bench("app_launch/cold_single_repo").expect("single repo case"),
        LaunchScenario::ColdSingleRepo
    );
    assert_eq!(
        LaunchScenario::from_bench("app_launch/cold_five_repos").expect("five repo case"),
        LaunchScenario::ColdFiveRepos
    );
}

#[test]
fn launch_scenario_supports_twenty_repo_and_warm_cases() {
    assert_eq!(
        LaunchScenario::from_bench("app_launch/cold_twenty_repos").expect("cold twenty repo case"),
        LaunchScenario::ColdTwentyRepos
    );
    assert_eq!(
        LaunchScenario::from_bench("app_launch/warm_single_repo").expect("warm single repo case"),
        LaunchScenario::WarmSingleRepo
    );
    assert_eq!(
        LaunchScenario::from_bench("app_launch/warm_twenty_repos").expect("warm twenty repo case"),
        LaunchScenario::WarmTwentyRepos
    );
}

#[test]
fn launch_scenario_expected_ready_repos_matches_variant() {
    assert_eq!(LaunchScenario::ColdEmptyWorkspace.expected_ready_repos(), 0);
    assert_eq!(LaunchScenario::ColdSingleRepo.expected_ready_repos(), 1);
    assert_eq!(LaunchScenario::ColdFiveRepos.expected_ready_repos(), 5);
    assert_eq!(LaunchScenario::ColdTwentyRepos.expected_ready_repos(), 20);
    assert_eq!(LaunchScenario::WarmSingleRepo.expected_ready_repos(), 1);
    assert_eq!(LaunchScenario::WarmTwentyRepos.expected_ready_repos(), 20);
}

#[test]
fn launch_scenario_warm_variants_need_warm_up_pass() {
    assert!(!LaunchScenario::ColdEmptyWorkspace.needs_warm_up_pass());
    assert!(!LaunchScenario::ColdSingleRepo.needs_warm_up_pass());
    assert!(!LaunchScenario::ColdFiveRepos.needs_warm_up_pass());
    assert!(!LaunchScenario::ColdTwentyRepos.needs_warm_up_pass());
    assert!(LaunchScenario::WarmSingleRepo.needs_warm_up_pass());
    assert!(LaunchScenario::WarmTwentyRepos.needs_warm_up_pass());
}

#[test]
fn launch_scenario_only_empty_workspace_disables_auto_restore() {
    assert!(LaunchScenario::ColdEmptyWorkspace.disable_auto_restore());
    assert!(!LaunchScenario::ColdSingleRepo.disable_auto_restore());
    assert!(!LaunchScenario::ColdFiveRepos.disable_auto_restore());
    assert!(!LaunchScenario::ColdTwentyRepos.disable_auto_restore());
    assert!(!LaunchScenario::WarmSingleRepo.disable_auto_restore());
    assert!(!LaunchScenario::WarmTwentyRepos.disable_auto_restore());
}

#[test]
fn launch_scenario_rejects_unknown_case() {
    let err = LaunchScenario::from_bench("app_launch/unknown_case").expect_err("reject case");
    assert!(err.contains("unsupported app launch benchmark"));
}

#[test]
fn parse_probe_event_line_ignores_unrelated_output() {
    assert!(parse_probe_event_line("normal stderr line").is_none());
}

#[test]
fn parse_probe_event_line_reads_json_payload() {
    let event = parse_probe_event_line(
        "GITCOMET_PERF_STARTUP_EVENT {\"name\":\"first_interactive\",\"rss_kib\":98304}",
    )
    .expect("event");
    assert_eq!(event.name, "first_interactive");
    assert_eq!(event.rss_kib, Some(98_304));
}

#[test]
fn parse_probe_event_line_reads_repo_progress_payload() {
    let event = parse_probe_event_line(
            "GITCOMET_PERF_STARTUP_EVENT {\"name\":\"repos_loaded\",\"repos_loaded\":5,\"repos_total\":5}",
        )
        .expect("repo progress event");
    assert_eq!(event.name, "repos_loaded");
    assert_eq!(event.repos_loaded, Some(5));
    assert_eq!(event.repos_total, Some(5));
}

#[test]
fn active_repo_milestone_is_distinct_from_open_handles_and_first_interactive() {
    let mut progress = LaunchProgress::default();
    let mut stderr = VecDeque::new();
    let start = Instant::now();
    for payload in [
        r#"{"name":"first_interactive"}"#,
        r#"{"name":"repos_loaded","repos_loaded":5,"repos_total":5}"#,
    ] {
        process_probe_line(
            format!("{EVENT_PREFIX}{payload}"),
            start,
            &mut progress,
            &mut stderr,
        );
    }
    assert!(progress.first_interactive.is_some());
    assert_eq!(progress.repos_loaded, 5);
    assert!(progress.active_repo_ready.is_none());
    assert!(require_active_repo_ready("fixture", "child", 5, None).is_err());
    assert!(require_active_repo_ready("empty", "child", 0, None).is_ok());
    process_probe_line(
        format!(
            "{EVENT_PREFIX}{}",
            r#"{"name":"active_repo_ready","rss_kib":123}"#
        ),
        start,
        &mut progress,
        &mut stderr,
    );
    assert_eq!(progress.active_repo_ready.unwrap().rss_kib, Some(123));
    assert!(stderr.is_empty());
}

#[test]
fn parse_probe_event_line_reads_allocation_payload() {
    let event = parse_probe_event_line(
            "GITCOMET_PERF_STARTUP_EVENT {\"name\":\"first_paint\",\"alloc_ops\":12,\"dealloc_ops\":4,\"realloc_ops\":1,\"alloc_bytes\":4096,\"dealloc_bytes\":1024,\"realloc_bytes_delta\":512,\"net_alloc_bytes\":3072}",
        )
        .expect("allocation event");
    let metrics = alloc_metrics_from_event(&event).expect("alloc metrics");
    assert_eq!(metrics.alloc_ops, 12);
    assert_eq!(metrics.dealloc_ops, 4);
    assert_eq!(metrics.realloc_ops, 1);
    assert_eq!(metrics.alloc_bytes, 4_096);
    assert_eq!(metrics.dealloc_bytes, 1_024);
    assert_eq!(metrics.realloc_bytes_delta, 512);
    assert_eq!(metrics.net_alloc_bytes, 3_072);
}

#[test]
fn build_launch_sidecar_metrics_includes_milestone_allocations() {
    let first_paint = ObservedMilestone {
        elapsed_ms: 111,
        rss_kib: Some(22_222),
        alloc_metrics: Some(PerfAllocMetrics {
            alloc_ops: 12,
            dealloc_ops: 3,
            realloc_ops: 1,
            alloc_bytes: 4_096,
            dealloc_bytes: 1_024,
            realloc_bytes_delta: 512,
            net_alloc_bytes: 3_072,
        }),
    };
    let first_interactive = ObservedMilestone {
        elapsed_ms: 222,
        rss_kib: Some(33_333),
        alloc_metrics: Some(PerfAllocMetrics {
            alloc_ops: 24,
            dealloc_ops: 6,
            realloc_ops: 2,
            alloc_bytes: 8_192,
            dealloc_bytes: 2_048,
            realloc_bytes_delta: 1_024,
            net_alloc_bytes: 6_144,
        }),
    };

    let metrics =
        build_launch_sidecar_metrics(first_paint, first_interactive, Some(first_interactive), 5);

    assert_eq!(metrics.get("first_paint_ms"), Some(&Value::from(111)));
    assert_eq!(metrics.get("first_interactive_ms"), Some(&Value::from(222)));
    assert_eq!(
        metrics.get("rss_at_first_paint_kib"),
        Some(&Value::from(22_222))
    );
    assert_eq!(
        metrics.get("rss_at_interactive_kib"),
        Some(&Value::from(33_333))
    );
    assert_eq!(metrics.get("first_paint_alloc_ops"), Some(&Value::from(12)));
    assert_eq!(
        metrics.get("first_paint_net_alloc_bytes"),
        Some(&Value::from(3_072))
    );
    assert_eq!(
        metrics.get("first_interactive_alloc_bytes"),
        Some(&Value::from(8_192))
    );
    assert_eq!(
        metrics.get("first_interactive_net_alloc_bytes"),
        Some(&Value::from(6_144))
    );
    assert_eq!(metrics.get("repos_loaded"), Some(&Value::from(5)));
    assert_eq!(metrics.get("active_repo_ready_ms"), Some(&Value::from(222)));
    assert_eq!(
        metrics.get("active_repo_ready_alloc_bytes"),
        Some(&Value::from(8_192))
    );
    assert_eq!(
        metrics.get("rss_at_active_repo_ready_kib"),
        Some(&Value::from(33_333))
    );
}

#[test]
fn require_launch_milestones_rejects_missing_allocation_snapshot() {
    let err = require_launch_milestones(
        "app_launch/cold_empty_workspace",
        "launch probe child",
        Some(ObservedMilestone {
            elapsed_ms: 111,
            rss_kib: Some(22_222),
            alloc_metrics: None,
        }),
        Some(ObservedMilestone {
            elapsed_ms: 222,
            rss_kib: Some(33_333),
            alloc_metrics: Some(PerfAllocMetrics {
                alloc_ops: 24,
                dealloc_ops: 6,
                realloc_ops: 2,
                alloc_bytes: 8_192,
                dealloc_bytes: 2_048,
                realloc_bytes_delta: 1_024,
                net_alloc_bytes: 6_144,
            }),
        }),
    )
    .expect_err("reject missing allocation snapshot");

    assert!(err.contains("first_paint"));
    assert!(err.contains("allocation metrics"));
}

#[test]
fn require_launch_milestones_accepts_allocation_aware_milestones() {
    let (first_paint, first_interactive) = require_launch_milestones(
        "app_launch/cold_empty_workspace",
        "launch probe child",
        Some(ObservedMilestone {
            elapsed_ms: 111,
            rss_kib: Some(22_222),
            alloc_metrics: Some(PerfAllocMetrics {
                alloc_ops: 12,
                dealloc_ops: 3,
                realloc_ops: 1,
                alloc_bytes: 4_096,
                dealloc_bytes: 1_024,
                realloc_bytes_delta: 512,
                net_alloc_bytes: 3_072,
            }),
        }),
        Some(ObservedMilestone {
            elapsed_ms: 222,
            rss_kib: Some(33_333),
            alloc_metrics: Some(PerfAllocMetrics {
                alloc_ops: 24,
                dealloc_ops: 6,
                realloc_ops: 2,
                alloc_bytes: 8_192,
                dealloc_bytes: 2_048,
                realloc_bytes_delta: 1_024,
                net_alloc_bytes: 6_144,
            }),
        }),
    )
    .expect("accept allocation-aware milestones");

    assert_eq!(first_paint.elapsed_ms, 111);
    assert_eq!(first_interactive.elapsed_ms, 222);
}

#[test]
fn write_launch_session_file_sets_active_repo_to_first_entry() {
    let dir = tempdir().expect("tempdir");
    let session_file = dir.path().join("session.json");
    let repo_a = dir.path().join("repo-a");
    let repo_b = dir.path().join("repo-b");

    write_launch_session_file(&session_file, &[repo_a.clone(), repo_b.clone()])
        .expect("write session file");

    let json: Value = serde_json::from_slice(&fs::read(&session_file).expect("read session file"))
        .expect("parse session file");
    assert_eq!(json["version"], SESSION_FILE_VERSION);
    assert_eq!(json["open_repos"].as_array().map(Vec::len), Some(2));
    assert_eq!(
        json["active_repo"],
        Value::String(path_to_session_string(&repo_a))
    );
}

#[test]
fn format_stderr_tail_renders_recent_lines() {
    let rendered = format_stderr_tail(&["line a".to_string(), "line b".to_string()]);
    assert!(rendered.contains("child stderr tail"));
    assert!(rendered.contains("line a"));
    assert!(rendered.contains("line b"));
}

#[test]
fn linux_wayland_socket_path_joins_runtime_dir_for_relative_socket() {
    assert_eq!(
        linux_wayland_socket_path(Some("wayland-0"), Some("/run/user/1000")),
        Some(PathBuf::from("/run/user/1000/wayland-0"))
    );
}

#[test]
fn linux_wayland_socket_path_accepts_absolute_socket() {
    assert_eq!(
        linux_wayland_socket_path(Some("/tmp/wayland-perf"), Some("/run/user/1000")),
        Some(PathBuf::from("/tmp/wayland-perf"))
    );
}

#[test]
fn linux_x11_socket_path_accepts_local_display_formats() {
    assert_eq!(
        linux_x11_socket_path(Some(":0.0")),
        Some(PathBuf::from("/tmp/.X11-unix/X0"))
    );
    assert_eq!(
        linux_x11_socket_path(Some("unix/:42")),
        Some(PathBuf::from("/tmp/.X11-unix/X42"))
    );
    assert_eq!(
        linux_x11_socket_path(Some("unix:7.1")),
        Some(PathBuf::from("/tmp/.X11-unix/X7"))
    );
}

#[test]
fn linux_x11_socket_path_rejects_remote_display_formats() {
    assert_eq!(linux_x11_socket_path(Some("localhost:10.0")), None);
    assert_eq!(linux_x11_socket_path(Some("example.com:0")), None);
}

#[test]
fn linux_display_socket_plan_tracks_local_probes_and_invalid_settings() {
    let plan = linux_display_socket_plan(
        Some("wayland-0"),
        Some("/run/user/1000"),
        Some("localhost:10.0"),
    );
    assert_eq!(
        plan.probes,
        vec![LinuxDisplaySocketProbe {
            label: "WAYLAND_DISPLAY",
            path: PathBuf::from("/run/user/1000/wayland-0"),
        }]
    );
    assert_eq!(
        plan.issues,
        vec!["DISPLAY=localhost:10.0 is not a local X11 display".to_string()]
    );
}

#[test]
fn linux_display_socket_plan_requires_runtime_dir_for_relative_wayland_socket() {
    let plan = linux_display_socket_plan(Some("wayland-0"), None, None);
    assert!(plan.probes.is_empty());
    assert_eq!(
        plan.issues,
        vec![
            "WAYLAND_DISPLAY=wayland-0 requires XDG_RUNTIME_DIR to resolve a local Wayland socket"
                .to_string()
        ]
    );
}

#[test]
fn linux_display_discovery_blocker_mentions_missing_local_session() {
    let message = format_linux_display_discovery_blocker("app_launch/cold_empty_workspace", &[]);
    assert!(message.contains("app_launch/cold_empty_workspace"));
    assert!(message.contains("Neither WAYLAND_DISPLAY nor DISPLAY resolved"));
    assert!(message.contains("environment blocker"));
    assert!(message.contains("headless mode is not a substitute"));
}

#[test]
fn linux_display_socket_probe_blocker_mentions_probe_failures() {
    let message = format_linux_display_socket_probe_blocker(
        "app_launch/cold_empty_workspace",
        &[
            "WAYLAND_DISPLAY=/run/user/1000/wayland-0 (Operation not permitted (os error 1))"
                .to_string(),
            "DISPLAY=/tmp/.X11-unix/X0 (Operation not permitted (os error 1))".to_string(),
        ],
        &["DISPLAY=localhost:10.0 is not a local X11 display".to_string()],
    );
    assert!(message.contains("app_launch/cold_empty_workspace"));
    assert!(message.contains("Direct Unix-socket probes failed"));
    assert!(message.contains("WAYLAND_DISPLAY=/run/user/1000/wayland-0"));
    assert!(message.contains("DISPLAY=/tmp/.X11-unix/X0"));
    assert!(message.contains("DISPLAY=localhost:10.0 is not a local X11 display"));
    assert!(message.contains("environment blocker"));
    assert!(message.contains("headless mode is not a substitute"));
}

#[test]
fn detects_wayland_display_launch_failure() {
    assert!(is_linux_display_launch_failure(&[
            "child launch failed: main GPUI window launch panicked: called `Result::unwrap()` on an `Err` value: NoCompositor".to_string(),
        ]));
}

#[test]
fn detects_x11_display_launch_failure() {
    assert!(is_linux_display_launch_failure(&[
        "child launch failed: main GPUI window launch panicked: Unknown connection error"
            .to_string(),
    ]));
}

#[test]
fn ignores_unrelated_stderr_for_display_detection() {
    assert!(!is_linux_display_launch_failure(&[
        "benchmark timed out after 30s".to_string(),
        "repo open metrics missing".to_string(),
    ]));
}

#[test]
fn linux_display_environment_blocker_mentions_bench_and_stage() {
    let message = format_linux_display_environment_blocker(
        "app_launch/cold_empty_workspace",
        "launch probe child",
        &["child launch failed: NoCompositor".to_string()],
    )
    .expect("display blocker");
    assert!(message.contains("app_launch/cold_empty_workspace"));
    assert!(message.contains("launch probe child"));
    assert!(message.contains("environment blocker"));
    assert!(message.contains("NoCompositor"));
    assert!(message.contains("headless mode is not a substitute"));
}

#[test]
fn environment_blocker_messages_use_dedicated_exit_code() {
    assert_eq!(
        exit_code_for_run_error(
            "app_launch/cold_empty_workspace is blocked. This is an environment blocker, not a benchmark result."
        ),
        ENVIRONMENT_BLOCKER_EXIT_CODE
    );
}

#[test]
fn non_blocker_messages_use_generic_failure_exit_code() {
    assert_eq!(
        exit_code_for_run_error("failed to create app launch fixture tempdir"),
        1
    );
}
