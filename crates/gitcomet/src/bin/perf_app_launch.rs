use gitcomet_core::services::GitBackend;
use gitcomet_ui_gpui::perf_alloc::{PerfAllocMetrics, TRACKING_MIMALLOC};
use gitcomet_ui_gpui::perf_ram_guard::{
    benchmark_ram_limit_kib, install_benchmark_process_ram_guard, process_rss_kib,
};
use gitcomet_ui_gpui::perf_sidecar::{PerfSidecarReport, write_criterion_sidecar};
use serde::{Deserialize, Serialize};
use serde_json::{Map, json};
use std::collections::VecDeque;
use std::env;
use std::fs;
use std::io::{BufRead as _, BufReader, Write as _};
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::sync::mpsc::TryRecvError;
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const EVENT_PREFIX: &str = "GITCOMET_PERF_STARTUP_EVENT ";
const DEFAULT_BENCH: &str = "app_launch/cold_empty_workspace";
const DEFAULT_TIMEOUT_MS: u64 = 30_000;
const SESSION_FILE_VERSION: u32 = 2;
const EXPECT_READY_REPOS_ENV: &str = "GITCOMET_PERF_STARTUP_EXPECT_READY_REPOS";
const CLI_USAGE_EXIT_CODE: i32 = 2;
const ENVIRONMENT_BLOCKER_EXIT_CODE: i32 = 3;
const ENVIRONMENT_BLOCKER_MARKER: &str = "This is an environment blocker, not a benchmark result.";
#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
const APP_LAUNCH_HEADLESS_NOTE: &str = "GPUI headless mode is not a substitute for app_launch because it cannot open the main window or emit comparable first_paint/first_interactive metrics.";

#[global_allocator]
static GLOBAL: &gitcomet_ui_gpui::perf_alloc::PerfTrackingAllocator = &TRACKING_MIMALLOC;

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliArgs {
    child: bool,
    preflight_only: bool,
    bench: String,
    timeout_ms: u64,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct StartupProbeEvent {
    name: String,
    #[serde(default)]
    rss_kib: Option<u64>,
    #[serde(default)]
    alloc_ops: Option<u64>,
    #[serde(default)]
    dealloc_ops: Option<u64>,
    #[serde(default)]
    realloc_ops: Option<u64>,
    #[serde(default)]
    alloc_bytes: Option<u64>,
    #[serde(default)]
    dealloc_bytes: Option<u64>,
    #[serde(default)]
    realloc_bytes_delta: Option<i64>,
    #[serde(default)]
    net_alloc_bytes: Option<i64>,
    #[serde(default)]
    repos_loaded: Option<u64>,
    #[serde(default)]
    repos_total: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct ObservedMilestone {
    elapsed_ms: u64,
    rss_kib: Option<u64>,
    alloc_metrics: Option<PerfAllocMetrics>,
}

#[derive(Debug)]
struct HarnessRunResult {
    status: ExitStatus,
    first_paint: Option<ObservedMilestone>,
    first_interactive: Option<ObservedMilestone>,
    repos_loaded: u64,
    repos_total: u64,
    stderr_tail: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LaunchScenario {
    ColdEmptyWorkspace,
    ColdSingleRepo,
    ColdFiveRepos,
    ColdTwentyRepos,
    WarmSingleRepo,
    WarmTwentyRepos,
}

impl LaunchScenario {
    fn from_bench(bench: &str) -> Result<Self, String> {
        match bench {
            "app_launch/cold_empty_workspace" => Ok(Self::ColdEmptyWorkspace),
            "app_launch/cold_single_repo" => Ok(Self::ColdSingleRepo),
            "app_launch/cold_five_repos" => Ok(Self::ColdFiveRepos),
            "app_launch/cold_twenty_repos" => Ok(Self::ColdTwentyRepos),
            "app_launch/warm_single_repo" => Ok(Self::WarmSingleRepo),
            "app_launch/warm_twenty_repos" => Ok(Self::WarmTwentyRepos),
            _ => Err(format!("unsupported app launch benchmark {bench:?}")),
        }
    }

    fn disable_auto_restore(self) -> bool {
        matches!(self, Self::ColdEmptyWorkspace)
    }

    fn expected_ready_repos(self) -> usize {
        match self {
            Self::ColdEmptyWorkspace => 0,
            Self::ColdSingleRepo | Self::WarmSingleRepo => 1,
            Self::ColdFiveRepos => 5,
            Self::ColdTwentyRepos | Self::WarmTwentyRepos => 20,
        }
    }

    fn needs_warm_up_pass(self) -> bool {
        matches!(self, Self::WarmSingleRepo | Self::WarmTwentyRepos)
    }
}

struct LaunchFixture {
    _root: TempDir,
    session_file: PathBuf,
    expected_ready_repos: usize,
    disable_auto_restore: bool,
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
#[derive(Clone, Debug, Eq, PartialEq)]
struct LinuxDisplaySocketProbe {
    label: &'static str,
    path: PathBuf,
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct LinuxDisplaySocketPlan {
    probes: Vec<LinuxDisplaySocketProbe>,
    issues: Vec<String>,
}

#[derive(Debug, Serialize)]
struct LaunchSessionFile {
    version: u32,
    open_repos: Vec<String>,
    active_repo: Option<String>,
}

impl LaunchFixture {
    fn build(scenario: LaunchScenario) -> Result<Self, String> {
        let root = tempfile::tempdir()
            .map_err(|err| format!("failed to create app launch fixture tempdir: {err}"))?;
        let expected_ready_repos = scenario.expected_ready_repos();
        let mut open_repos = Vec::with_capacity(expected_ready_repos);
        for seed in 0..expected_ready_repos {
            let repo_path = root.path().join(format!("repo-{seed:02}"));
            build_launch_repo(&repo_path, seed)?;
            open_repos.push(repo_path);
        }

        let session_file = root.path().join("session.json");
        write_launch_session_file(&session_file, &open_repos)?;

        Ok(Self {
            _root: root,
            session_file,
            expected_ready_repos,
            disable_auto_restore: scenario.disable_auto_restore(),
        })
    }
}

fn main() {
    match parse_cli_args(env::args().skip(1)) {
        Ok(args) => {
            let result = if args.child {
                run_child()
            } else if args.preflight_only {
                run_preflight(&args).map(|_| ())
            } else {
                run_harness(&args).map(|_| ())
            };
            if let Err(err) = result {
                eprintln!("{err}");
                std::process::exit(exit_code_for_run_error(&err));
            }
        }
        Err(err) => {
            eprintln!("{err}");
            eprintln!();
            eprintln!("{}", usage());
            std::process::exit(CLI_USAGE_EXIT_CODE);
        }
    }
}

fn run_preflight(args: &CliArgs) -> Result<(), String> {
    LaunchScenario::from_bench(&args.bench)?;
    if let Some(blocker) = linux_display_preflight_blocker(&args.bench) {
        return Err(blocker);
    }
    let fixture = LaunchFixture::build(LaunchScenario::ColdEmptyWorkspace)?;
    run_first_interactive_probe(&args.bench, "preflight child", &fixture, args.timeout_ms)?;

    println!("{} launch environment preflight ok", args.bench);
    Ok(())
}

fn run_harness(args: &CliArgs) -> Result<(), String> {
    let scenario = LaunchScenario::from_bench(&args.bench)?;
    if let Some(blocker) = linux_display_preflight_blocker(&args.bench) {
        return Err(blocker);
    }
    let fixture = LaunchFixture::build(scenario)?;

    if scenario.needs_warm_up_pass() {
        run_warm_up_pass(&args.bench, &fixture, args.timeout_ms)?;
    }

    let current_exe = env::current_exe()
        .map_err(|err| format!("failed to resolve current executable path: {err}"))?;
    let mut command = Command::new(current_exe);
    command
        .arg("--child")
        .env("GITCOMET_PERF_STARTUP_PROBE", "1")
        .env("GITCOMET_PERF_STARTUP_AUTO_EXIT", "1")
        .env(
            EXPECT_READY_REPOS_ENV,
            fixture.expected_ready_repos.to_string(),
        )
        .env("GITCOMET_SESSION_FILE", &fixture.session_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if fixture.disable_auto_restore {
        command.env("GITCOMET_PERF_STARTUP_DISABLE_AUTO_RESTORE", "1");
    }

    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to spawn launch probe child process: {err}"))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "child process stderr pipe was not available".to_string())?;
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    let sent = tx.send(Ok(line.trim_end_matches(['\r', '\n']).to_string()));
                    if sent.is_err() {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err.to_string()));
                    break;
                }
            }
        }
    });

    let timeout = Duration::from_millis(args.timeout_ms.max(1));
    let rss_limit_kib = benchmark_ram_limit_kib();
    let started_at = Instant::now();
    let mut first_paint = None;
    let mut first_interactive = None;
    let mut repos_loaded = 0u64;
    let mut repos_total = 0u64;
    let mut stderr_tail = VecDeque::with_capacity(12);

    while started_at.elapsed() <= timeout {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => process_probe_line(
                line,
                started_at,
                &mut first_paint,
                &mut first_interactive,
                &mut repos_loaded,
                &mut repos_total,
                &mut stderr_tail,
            ),
            Ok(Err(err)) => {
                let _ = terminate_child(&mut child);
                let _ = reader.join();
                return Err(format!("failed while reading child stderr: {err}"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }

        if let Err(err) = enforce_child_ram_limit(&mut child, rss_limit_kib, "launch probe child") {
            let _ = reader.join();
            return Err(err);
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("failed to poll child process: {err}"))?
        {
            let _ = reader.join();
            drain_probe_channel(
                &rx,
                started_at,
                &mut first_paint,
                &mut first_interactive,
                &mut repos_loaded,
                &mut repos_total,
                &mut stderr_tail,
            )?;
            return finish_harness(
                args,
                fixture.expected_ready_repos,
                HarnessRunResult {
                    status,
                    first_paint,
                    first_interactive,
                    repos_loaded,
                    repos_total,
                    stderr_tail: stderr_tail_to_vec(stderr_tail),
                },
            );
        }
    }

    let status = terminate_child(&mut child)?;
    let _ = reader.join();
    drain_probe_channel(
        &rx,
        started_at,
        &mut first_paint,
        &mut first_interactive,
        &mut repos_loaded,
        &mut repos_total,
        &mut stderr_tail,
    )?;
    finish_harness(
        args,
        fixture.expected_ready_repos,
        HarnessRunResult {
            status,
            first_paint,
            first_interactive,
            repos_loaded,
            repos_total,
            stderr_tail: stderr_tail_to_vec(stderr_tail),
        },
    )
}

fn exit_code_for_run_error(err: &str) -> i32 {
    if is_environment_blocker_message(err) {
        ENVIRONMENT_BLOCKER_EXIT_CODE
    } else {
        1
    }
}

fn is_environment_blocker_message(err: &str) -> bool {
    err.contains(ENVIRONMENT_BLOCKER_MARKER)
}

fn run_warm_up_pass(bench: &str, fixture: &LaunchFixture, timeout_ms: u64) -> Result<(), String> {
    run_first_interactive_probe(bench, "warm-up child", fixture, timeout_ms)
}

fn run_first_interactive_probe(
    bench: &str,
    stage: &str,
    fixture: &LaunchFixture,
    timeout_ms: u64,
) -> Result<(), String> {
    let current_exe = env::current_exe()
        .map_err(|err| format!("failed to resolve current executable path: {err}"))?;
    let mut command = Command::new(current_exe);
    command
        .arg("--child")
        .env("GITCOMET_PERF_STARTUP_PROBE", "1")
        .env("GITCOMET_PERF_STARTUP_AUTO_EXIT", "1")
        .env(
            EXPECT_READY_REPOS_ENV,
            fixture.expected_ready_repos.to_string(),
        )
        .env("GITCOMET_SESSION_FILE", &fixture.session_file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    if fixture.disable_auto_restore {
        command.env("GITCOMET_PERF_STARTUP_DISABLE_AUTO_RESTORE", "1");
    }

    let mut child = command
        .spawn()
        .map_err(|err| format!("failed to spawn {stage} process: {err}"))?;

    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| format!("{stage} stderr pipe was not available"))?;
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {
                    if tx
                        .send(Ok(line.trim_end_matches(['\r', '\n']).to_string()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(err) => {
                    let _ = tx.send(Err(err.to_string()));
                    break;
                }
            }
        }
    });

    let timeout = Duration::from_millis(timeout_ms.max(1));
    let rss_limit_kib = benchmark_ram_limit_kib();
    let started_at = Instant::now();
    let mut first_paint = None;
    let mut first_interactive = None;
    let mut repos_loaded = 0u64;
    let mut repos_total = 0u64;
    let mut stderr_tail = VecDeque::with_capacity(12);
    let mut child_status = None;

    while started_at.elapsed() <= timeout {
        match rx.recv_timeout(Duration::from_millis(50)) {
            Ok(Ok(line)) => process_probe_line(
                line,
                started_at,
                &mut first_paint,
                &mut first_interactive,
                &mut repos_loaded,
                &mut repos_total,
                &mut stderr_tail,
            ),
            Ok(Err(err)) => {
                let _ = terminate_child(&mut child);
                let _ = reader.join();
                return Err(format!("failed reading {stage} stderr: {err}"));
            }
            Err(mpsc::RecvTimeoutError::Timeout) | Err(mpsc::RecvTimeoutError::Disconnected) => {}
        }

        if let Err(err) = enforce_child_ram_limit(&mut child, rss_limit_kib, stage) {
            let _ = reader.join();
            return Err(err);
        }

        if let Some(status) = child
            .try_wait()
            .map_err(|err| format!("failed to poll {stage}: {err}"))?
        {
            child_status = Some(status);
            break;
        }

        if first_interactive.is_some() {
            break;
        }
    }

    if child_status.is_none() {
        let _ = terminate_child(&mut child);
    }
    let _ = reader.join();
    drain_probe_channel(
        &rx,
        started_at,
        &mut first_paint,
        &mut first_interactive,
        &mut repos_loaded,
        &mut repos_total,
        &mut stderr_tail,
    )?;
    let stderr_tail = stderr_tail_to_vec(stderr_tail);

    if let Some(status) = child_status
        && !status.success()
    {
        if let Some(blocker) = detect_linux_display_environment_blocker(bench, stage, &stderr_tail)
        {
            return Err(blocker);
        }
        return Err(format!(
            "{stage} exited unsuccessfully with status {status}{}",
            format_stderr_tail(&stderr_tail)
        ));
    }

    if first_interactive.is_none() {
        if let Some(blocker) = detect_linux_display_environment_blocker(bench, stage, &stderr_tail)
        {
            return Err(blocker);
        }
        return Err(format!(
            "{stage} did not reach first_interactive before timeout"
        ));
    }
    require_launch_milestones(bench, stage, first_paint, first_interactive)?;
    Ok(())
}

fn require_launch_milestones(
    bench: &str,
    stage: &str,
    first_paint: Option<ObservedMilestone>,
    first_interactive: Option<ObservedMilestone>,
) -> Result<(ObservedMilestone, ObservedMilestone), String> {
    let first_paint = first_paint.ok_or_else(|| {
        format!("{stage} exited without emitting a first_paint milestone for {bench}")
    })?;
    let first_interactive = first_interactive.ok_or_else(|| {
        format!("{stage} exited without emitting a first_interactive milestone for {bench}")
    })?;

    require_launch_alloc_metrics(bench, stage, "first_paint", first_paint)?;
    require_launch_alloc_metrics(bench, stage, "first_interactive", first_interactive)?;

    Ok((first_paint, first_interactive))
}

fn require_launch_alloc_metrics(
    bench: &str,
    stage: &str,
    milestone_name: &str,
    milestone: ObservedMilestone,
) -> Result<(), String> {
    if milestone.alloc_metrics.is_some() {
        return Ok(());
    }

    Err(format!(
        "{stage} emitted {milestone_name} for {bench} without allocation metrics; app_launch requires first_paint and first_interactive allocation snapshots before preflight succeeds or a sidecar is written"
    ))
}

fn finish_harness(
    args: &CliArgs,
    expected_ready_repos: usize,
    result: HarnessRunResult,
) -> Result<(), String> {
    let HarnessRunResult {
        status,
        first_paint,
        first_interactive,
        repos_loaded,
        repos_total,
        stderr_tail,
    } = result;

    if !status.success() {
        if let Some(blocker) = detect_linux_display_environment_blocker(
            &args.bench,
            "launch probe child",
            &stderr_tail,
        ) {
            return Err(blocker);
        }
        return Err(format!(
            "launch probe child exited unsuccessfully with status {status}{}",
            format_stderr_tail(&stderr_tail)
        ));
    }

    if let Some(blocker) =
        detect_linux_display_environment_blocker(&args.bench, "launch probe child", &stderr_tail)
    {
        return Err(blocker);
    }

    let (first_paint, first_interactive) = require_launch_milestones(
        &args.bench,
        "launch probe child",
        first_paint,
        first_interactive,
    )?;

    if repos_loaded < expected_ready_repos as u64 {
        return Err(format!(
            "launch probe child exited after opening only {repos_loaded} of {expected_ready_repos} expected repos for {} (observed repo slots: {repos_total}){}",
            args.bench,
            format_stderr_tail(&stderr_tail)
        ));
    }

    let metrics = build_launch_sidecar_metrics(first_paint, first_interactive, repos_loaded);
    let report = PerfSidecarReport::new(&args.bench, metrics);
    let sidecar_path = write_criterion_sidecar(&report)
        .map_err(|err| format!("failed to write sidecar report for {}: {err}", args.bench))?;

    println!(
        "{} first_paint_ms={} first_interactive_ms={} repos_loaded={} sidecar={}",
        args.bench,
        first_paint.elapsed_ms,
        first_interactive.elapsed_ms,
        repos_loaded,
        sidecar_path.display()
    );
    Ok(())
}

fn build_launch_sidecar_metrics(
    first_paint: ObservedMilestone,
    first_interactive: ObservedMilestone,
    repos_loaded: u64,
) -> Map<String, serde_json::Value> {
    let mut metrics = Map::new();
    metrics.insert("first_paint_ms".to_string(), json!(first_paint.elapsed_ms));
    metrics.insert(
        "first_interactive_ms".to_string(),
        json!(first_interactive.elapsed_ms),
    );
    if let Some(rss_kib) = first_paint.rss_kib {
        metrics.insert("rss_at_first_paint_kib".to_string(), json!(rss_kib));
    }
    if let Some(rss_kib) = first_interactive.rss_kib {
        metrics.insert("rss_at_interactive_kib".to_string(), json!(rss_kib));
    }
    if let Some(alloc_metrics) = first_paint.alloc_metrics {
        alloc_metrics.append_to_payload_with_prefix(&mut metrics, "first_paint_");
    }
    if let Some(alloc_metrics) = first_interactive.alloc_metrics {
        alloc_metrics.append_to_payload_with_prefix(&mut metrics, "first_interactive_");
    }
    metrics.insert("repos_loaded".to_string(), json!(repos_loaded));
    metrics
}

fn terminate_child(child: &mut std::process::Child) -> Result<ExitStatus, String> {
    let _ = child.kill();
    child
        .wait()
        .map_err(|err| format!("failed to wait for terminated child process: {err}"))
}

fn enforce_child_ram_limit(
    child: &mut std::process::Child,
    rss_limit_kib: Option<u64>,
    label: &str,
) -> Result<(), String> {
    let Some(rss_limit_kib) = rss_limit_kib else {
        return Ok(());
    };
    let Some(rss_kib) = process_rss_kib(child.id()) else {
        return Ok(());
    };
    if rss_kib <= rss_limit_kib {
        return Ok(());
    }

    let _ = terminate_child(child);
    Err(format!(
        "{label} exceeded benchmark RAM guard: RSS {rss_kib} KiB exceeded limit {rss_limit_kib} KiB (smaller of 8 GiB and 75% of startup available RAM)"
    ))
}

fn run_child() -> Result<(), String> {
    install_benchmark_process_ram_guard();
    gitcomet_ui_gpui::run(build_backend()).map_err(|err| format!("child launch failed: {err}"))
}

fn build_backend() -> Arc<dyn GitBackend> {
    if cfg!(feature = "gix") {
        #[cfg(feature = "gix")]
        {
            Arc::new(gitcomet_git_gix::GixBackend)
        }

        #[cfg(not(feature = "gix"))]
        {
            gitcomet_git::default_backend()
        }
    } else {
        gitcomet_git::default_backend()
    }
}

fn parse_probe_event_line(line: &str) -> Option<StartupProbeEvent> {
    let payload = line.strip_prefix(EVENT_PREFIX)?;
    serde_json::from_str(payload).ok()
}

fn duration_to_millis_u64(duration: Duration) -> u64 {
    duration.as_millis().min(u128::from(u64::MAX)) as u64
}

fn process_probe_line(
    line: String,
    started_at: Instant,
    first_paint: &mut Option<ObservedMilestone>,
    first_interactive: &mut Option<ObservedMilestone>,
    repos_loaded: &mut u64,
    repos_total: &mut u64,
    stderr_tail: &mut VecDeque<String>,
) {
    if let Some(event) = parse_probe_event_line(&line) {
        let milestone = ObservedMilestone {
            elapsed_ms: duration_to_millis_u64(started_at.elapsed()),
            rss_kib: event.rss_kib,
            alloc_metrics: alloc_metrics_from_event(&event),
        };
        match event.name.as_str() {
            "first_paint" if first_paint.is_none() => *first_paint = Some(milestone),
            "first_interactive" if first_interactive.is_none() => {
                *first_interactive = Some(milestone)
            }
            "repos_loaded" => {
                *repos_loaded = (*repos_loaded).max(event.repos_loaded.unwrap_or(0));
                *repos_total = (*repos_total).max(event.repos_total.unwrap_or(0));
            }
            _ => {}
        }
    } else if !line.trim().is_empty() {
        push_stderr_tail(stderr_tail, line);
    }
}

fn alloc_metrics_from_event(event: &StartupProbeEvent) -> Option<PerfAllocMetrics> {
    Some(PerfAllocMetrics {
        alloc_ops: event.alloc_ops?,
        dealloc_ops: event.dealloc_ops?,
        realloc_ops: event.realloc_ops?,
        alloc_bytes: event.alloc_bytes?,
        dealloc_bytes: event.dealloc_bytes?,
        realloc_bytes_delta: event.realloc_bytes_delta?,
        net_alloc_bytes: event.net_alloc_bytes?,
    })
}

fn drain_probe_channel(
    rx: &mpsc::Receiver<Result<String, String>>,
    started_at: Instant,
    first_paint: &mut Option<ObservedMilestone>,
    first_interactive: &mut Option<ObservedMilestone>,
    repos_loaded: &mut u64,
    repos_total: &mut u64,
    stderr_tail: &mut VecDeque<String>,
) -> Result<(), String> {
    loop {
        match rx.try_recv() {
            Ok(Ok(line)) => process_probe_line(
                line,
                started_at,
                first_paint,
                first_interactive,
                repos_loaded,
                repos_total,
                stderr_tail,
            ),
            Ok(Err(err)) => return Err(format!("failed while reading child stderr: {err}")),
            Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => return Ok(()),
        }
    }
}

fn push_stderr_tail(lines: &mut VecDeque<String>, line: String) {
    const MAX_LINES: usize = 12;
    if lines.len() == MAX_LINES {
        lines.pop_front();
    }
    lines.push_back(line);
}

fn stderr_tail_to_vec(lines: VecDeque<String>) -> Vec<String> {
    lines.into_iter().collect()
}

fn format_stderr_tail(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }

    format!("\nchild stderr tail:\n{}", lines.join("\n"))
}

fn linux_display_preflight_blocker(bench: &str) -> Option<String> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        let plan = linux_display_socket_plan_from_env();
        if plan.probes.is_empty() {
            return Some(format_linux_display_discovery_blocker(
                bench,
                plan.issues.as_slice(),
            ));
        }

        let mut failures = Vec::new();
        for probe in plan.probes {
            match UnixStream::connect(&probe.path) {
                Ok(_) => return None,
                Err(err) => {
                    failures.push(format!("{}={} ({err})", probe.label, probe.path.display()))
                }
            }
        }

        Some(format_linux_display_socket_probe_blocker(
            bench,
            failures.as_slice(),
            plan.issues.as_slice(),
        ))
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = bench;
        None
    }
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn format_linux_display_discovery_blocker(bench: &str, issues: &[String]) -> String {
    let reason = if issues.is_empty() {
        "Neither WAYLAND_DISPLAY nor DISPLAY resolved to a usable local compositor/X11 endpoint on this runner.".to_string()
    } else {
        format!(
            "No usable local Wayland or X11 endpoint was configured: {}.",
            issues.join("; ")
        )
    };

    format!(
        "{bench} is blocked on a usable local Wayland or X11 session before benchmark launch ({}). {} {} {}",
        linux_display_environment_snapshot(),
        reason,
        ENVIRONMENT_BLOCKER_MARKER,
        APP_LAUNCH_HEADLESS_NOTE
    )
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn format_linux_display_socket_probe_blocker(
    bench: &str,
    failures: &[String],
    issues: &[String],
) -> String {
    let rejected_suffix = if issues.is_empty() {
        String::new()
    } else {
        format!(
            " Rejected non-local or incomplete display settings: {}.",
            issues.join("; ")
        )
    };

    format!(
        "{bench} is blocked on a usable local Wayland or X11 session before benchmark launch ({}). Direct Unix-socket probes failed: {}.{} {} {}",
        linux_display_environment_snapshot(),
        failures.join("; "),
        rejected_suffix,
        ENVIRONMENT_BLOCKER_MARKER,
        APP_LAUNCH_HEADLESS_NOTE
    )
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn linux_display_socket_plan_from_env() -> LinuxDisplaySocketPlan {
    linux_display_socket_plan(
        env::var("WAYLAND_DISPLAY").ok().as_deref(),
        env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        env::var("DISPLAY").ok().as_deref(),
    )
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn linux_display_socket_plan(
    wayland_display: Option<&str>,
    xdg_runtime_dir: Option<&str>,
    display: Option<&str>,
) -> LinuxDisplaySocketPlan {
    let mut plan = LinuxDisplaySocketPlan::default();

    if let Some(wayland_display) = wayland_display
        .map(str::trim)
        .filter(|wayland_display| !wayland_display.is_empty())
    {
        if let Some(path) = linux_wayland_socket_path(Some(wayland_display), xdg_runtime_dir) {
            plan.probes.push(LinuxDisplaySocketProbe {
                label: "WAYLAND_DISPLAY",
                path,
            });
        } else {
            plan.issues.push(format!(
                "WAYLAND_DISPLAY={wayland_display} requires XDG_RUNTIME_DIR to resolve a local Wayland socket"
            ));
        }
    }

    if let Some(display) = display.map(str::trim).filter(|display| !display.is_empty()) {
        if let Some(path) = linux_x11_socket_path(Some(display)) {
            plan.probes.push(LinuxDisplaySocketProbe {
                label: "DISPLAY",
                path,
            });
        } else {
            plan.issues
                .push(format!("DISPLAY={display} is not a local X11 display"));
        }
    }

    plan
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn linux_wayland_socket_path(
    wayland_display: Option<&str>,
    xdg_runtime_dir: Option<&str>,
) -> Option<PathBuf> {
    let wayland_display = wayland_display?.trim();
    if wayland_display.is_empty() {
        return None;
    }

    let path = Path::new(wayland_display);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }

    let xdg_runtime_dir = xdg_runtime_dir?.trim();
    if xdg_runtime_dir.is_empty() {
        return None;
    }

    Some(Path::new(xdg_runtime_dir).join(path))
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn linux_x11_socket_path(display: Option<&str>) -> Option<PathBuf> {
    let display_number = local_x11_display_number(display?)?;
    Some(PathBuf::from(format!("/tmp/.X11-unix/X{display_number}")))
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn local_x11_display_number(display: &str) -> Option<&str> {
    let display = display.trim();
    let local = if let Some(rest) = display.strip_prefix(':') {
        rest
    } else if let Some(rest) = display.strip_prefix("unix/:") {
        rest
    } else if let Some(rest) = display.strip_prefix("unix:") {
        rest
    } else {
        return None;
    };

    let display_number = local.split('.').next()?.trim();
    if display_number.is_empty() || !display_number.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }

    Some(display_number)
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn format_linux_display_environment_blocker(
    bench: &str,
    stage: &str,
    stderr_tail: &[String],
) -> Option<String> {
    if !is_linux_display_launch_failure(stderr_tail) {
        return None;
    }

    Some(format!(
        "{bench} is blocked on a usable local Wayland or X11 session; the {stage} could not open one on this runner ({}). {} {}{}",
        linux_display_environment_snapshot(),
        ENVIRONMENT_BLOCKER_MARKER,
        APP_LAUNCH_HEADLESS_NOTE,
        format_stderr_tail(stderr_tail)
    ))
}

#[cfg(any(test, target_os = "linux", target_os = "freebsd"))]
fn is_linux_display_launch_failure(stderr_tail: &[String]) -> bool {
    let combined = stderr_tail.join("\n").to_ascii_lowercase();
    [
        "nocompositor",
        "no compositor",
        "unknown connection error",
        "unable to open display",
        "failed to initialize x11 client",
        "neither display nor wayland_display is set",
        "you can run in headless mode",
    ]
    .iter()
    .any(|pattern| combined.contains(pattern))
}

fn detect_linux_display_environment_blocker(
    bench: &str,
    stage: &str,
    stderr_tail: &[String],
) -> Option<String> {
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        format_linux_display_environment_blocker(bench, stage, stderr_tail)
    }

    #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
    {
        let _ = (bench, stage, stderr_tail);
        None
    }
}

#[cfg(all(
    any(test, target_os = "linux", target_os = "freebsd"),
    any(target_os = "linux", target_os = "freebsd")
))]
fn linux_display_environment_snapshot() -> String {
    [
        format_env_var("DISPLAY"),
        format_env_var("WAYLAND_DISPLAY"),
        format_env_var("XDG_RUNTIME_DIR"),
        format_env_var("XAUTHORITY"),
    ]
    .join(", ")
}

#[cfg(all(
    any(test, target_os = "linux", target_os = "freebsd"),
    not(any(target_os = "linux", target_os = "freebsd"))
))]
fn linux_display_environment_snapshot() -> String {
    "DISPLAY=<unsupported>, WAYLAND_DISPLAY=<unsupported>, XDG_RUNTIME_DIR=<unsupported>, XAUTHORITY=<unsupported>"
        .to_string()
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn format_env_var(key: &str) -> String {
    match env::var_os(key) {
        Some(value) if value.is_empty() => format!("{key}=<empty>"),
        Some(value) => format!("{key}={}", value.to_string_lossy()),
        None => format!("{key}=<unset>"),
    }
}

fn write_launch_session_file(path: &Path, open_repos: &[PathBuf]) -> Result<(), String> {
    let payload = LaunchSessionFile {
        version: SESSION_FILE_VERSION,
        open_repos: open_repos
            .iter()
            .map(|path| path_to_session_string(path))
            .collect(),
        active_repo: open_repos.first().map(|path| path_to_session_string(path)),
    };
    let json = serde_json::to_vec(&payload).map_err(|err| {
        format!(
            "failed to serialize launch session file {}: {err}",
            path.display()
        )
    })?;
    fs::write(path, json).map_err(|err| {
        format!(
            "failed to write launch session file {}: {err}",
            path.display()
        )
    })
}

fn path_to_session_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

fn build_launch_repo(repo: &Path, seed: usize) -> Result<(), String> {
    fs::create_dir_all(repo)
        .map_err(|err| format!("failed to create launch repo {}: {err}", repo.display()))?;
    run_git(repo, &["init", "-q", "-b", "main"])?;

    let total_commits = 64usize;
    let mut import = String::with_capacity(total_commits.saturating_mul(256));
    for index in 1..=total_commits {
        let blob_mark = seed.saturating_mul(10_000).saturating_add(index);
        let commit_mark = seed
            .saturating_mul(10_000)
            .saturating_add(100_000)
            .saturating_add(index);
        let previous_commit_mark = commit_mark.saturating_sub(1);
        let file_path = format!("src/module_{:02}/file_{:02}.txt", seed % 16, index % 24);
        let payload = format!("repo-{seed:02} commit-{index:03}\n");
        let message = format!("repo-{seed:02}-c{index:03}");
        let timestamp = 1_700_100_000usize
            .saturating_add(seed.saturating_mul(1_000))
            .saturating_add(index);

        import.push_str("blob\n");
        import.push_str(&format!("mark :{blob_mark}\n"));
        import.push_str(&format!("data {}\n", payload.len()));
        import.push_str(&payload);
        import.push('\n');
        import.push_str("commit refs/heads/main\n");
        import.push_str(&format!("mark :{commit_mark}\n"));
        import.push_str(&format!(
            "author Bench <bench@example.com> {timestamp} +0000\n"
        ));
        import.push_str(&format!(
            "committer Bench <bench@example.com> {timestamp} +0000\n"
        ));
        import.push_str(&format!("data {}\n", message.len()));
        import.push_str(&message);
        import.push('\n');
        if index > 1 {
            import.push_str(&format!("from :{previous_commit_mark}\n"));
        }
        import.push_str(&format!("M 100644 :{blob_mark} {file_path}\n"));
    }

    run_git_with_input(repo, &["fast-import", "--quiet"], &import)?;
    run_git(repo, &["branch", &format!("feature/{seed:02}")])?;
    run_git(repo, &["tag", &format!("launch-{seed:02}")])?;
    Ok(())
}

fn run_git(repo: &Path, args: &[&str]) -> Result<(), String> {
    let output = git_command(repo)
        .args(args)
        .output()
        .map_err(|err| format!("failed to run git {:?} in {}: {err}", args, repo.display()))?;
    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "git {:?} failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn run_git_with_input(repo: &Path, args: &[&str], input: &str) -> Result<(), String> {
    let mut child = git_command(repo)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            format!(
                "failed to spawn git {:?} in {}: {err}",
                args,
                repo.display()
            )
        })?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| format!("git {:?} stdin unavailable in {}", args, repo.display()))?;
    stdin.write_all(input.as_bytes()).map_err(|err| {
        format!(
            "failed to write stdin for git {:?} in {}: {err}",
            args,
            repo.display()
        )
    })?;
    drop(stdin);

    let output = child.wait_with_output().map_err(|err| {
        format!(
            "failed to wait for git {:?} in {}: {err}",
            args,
            repo.display()
        )
    })?;
    if output.status.success() {
        return Ok(());
    }

    Err(format!(
        "git {:?} failed in {}:\nstdout:\n{}\nstderr:\n{}",
        args,
        repo.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

// Git on Windows ARM64 cannot access the NUL device as a config path
// (`unable to access 'NUL': Invalid argument`).  Use a process-lifetime
// empty file instead, which works identically on every platform.
fn empty_git_config() -> &'static Path {
    use std::sync::OnceLock;
    static EMPTY_CONFIG: OnceLock<PathBuf> = OnceLock::new();
    EMPTY_CONFIG.get_or_init(|| {
        let path = std::env::temp_dir().join("gitcomet-perf-empty.gitconfig");
        fs::write(&path, "").expect("create empty git config for perf harness");
        path
    })
}

fn git_command(repo: &Path) -> Command {
    let empty_config = empty_git_config();
    let mut command = Command::new("git");
    command
        .arg("-C")
        .arg(repo)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", empty_config)
        .env("GIT_CONFIG_SYSTEM", empty_config)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("EDITOR", "true")
        .env("VISUAL", "true");
    command
}

fn parse_cli_args(args: impl IntoIterator<Item = String>) -> Result<CliArgs, String> {
    let mut child = false;
    let mut preflight_only = false;
    let mut bench = DEFAULT_BENCH.to_string();
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--child" => child = true,
            "--preflight-only" => preflight_only = true,
            "--bench" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--bench requires a value".to_string())?;
                if value.trim().is_empty() {
                    return Err("--bench value must not be empty".to_string());
                }
                bench = value;
            }
            "--timeout-ms" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--timeout-ms requires a value".to_string())?;
                timeout_ms = value
                    .parse::<u64>()
                    .map_err(|err| format!("invalid --timeout-ms value {value:?}: {err}"))?;
            }
            "--help" | "-h" => return Err(usage().to_string()),
            other => return Err(format!("unrecognized argument {other:?}")),
        }
    }

    if child && preflight_only {
        return Err("--preflight-only cannot be combined with --child".to_string());
    }

    Ok(CliArgs {
        child,
        preflight_only,
        bench,
        timeout_ms,
    })
}

fn usage() -> &'static str {
    "Usage: perf-app-launch [--bench <label>] [--timeout-ms <ms>] [--preflight-only] [--child]"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use tempfile::tempdir;

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
            LaunchScenario::from_bench("app_launch/cold_twenty_repos")
                .expect("cold twenty repo case"),
            LaunchScenario::ColdTwentyRepos
        );
        assert_eq!(
            LaunchScenario::from_bench("app_launch/warm_single_repo")
                .expect("warm single repo case"),
            LaunchScenario::WarmSingleRepo
        );
        assert_eq!(
            LaunchScenario::from_bench("app_launch/warm_twenty_repos")
                .expect("warm twenty repo case"),
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

        let metrics = build_launch_sidecar_metrics(first_paint, first_interactive, 5);

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

        let json: Value =
            serde_json::from_slice(&fs::read(&session_file).expect("read session file"))
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
        let message =
            format_linux_display_discovery_blocker("app_launch/cold_empty_workspace", &[]);
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
}
