use gitcomet_core::auth::askpass::{
    GIT_COMMAND_TIMEOUT_ENV, append_host_prompt_to_stderr, configure_git_auth_prompt,
    create_askpass_script, remember_successful_prompt_auth, take_pending_git_auth,
};
use gitcomet_core::domain::{Commit, CommitId, CommitParentIds, LogPage};
use gitcomet_core::error::{Error, ErrorKind, GitFailure, GitFailureId};
use gitcomet_core::git_operation::{
    self, GitOperationContext, GitOperationEvent, GitOutputChunk, GitOutputStream, HookExecutionId,
};
use gitcomet_core::process::{configure_background_command, git_command};
use gitcomet_core::services::{CancellationToken, CommandOutput, Result};
use std::collections::HashMap;
use std::io::{self, BufRead as _, Read as _};
use std::path::{Path, PathBuf};
use std::process::{ChildStdout, Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

pub(crate) use gitcomet_core::auth::askpass::git_command_timeout;

// Used by test-only helpers below.
#[cfg(test)]
use gitcomet_core::domain::RemoteBranch;
#[cfg(test)]
use std::ffi::OsString;

const GIT_COMMAND_WAIT_POLL_MAX: Duration = Duration::from_millis(5);
const GIT_ACTIVITY_OUTPUT_FLUSH: Duration = Duration::from_millis(100);
const GIT_ACTIVITY_OUTPUT_BATCH_BYTES: usize = 16 * 1024;
const GIT_TRACE2_POLL: Duration = Duration::from_millis(20);
const GIT_PROCESS_TERMINATE_GRACE: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct TestGitCommandEnvironment {
    pub(crate) global_config: PathBuf,
    pub(crate) home_dir: PathBuf,
    pub(crate) xdg_config_home: PathBuf,
    pub(crate) gnupg_home: PathBuf,
}

static TEST_GIT_COMMAND_ENVIRONMENT: OnceLock<TestGitCommandEnvironment> = OnceLock::new();

fn io_err(e: std::io::Error) -> Error {
    Error::new(ErrorKind::Io(e.kind()))
}

fn git_command_wait_poll(elapsed: Duration, timeout: Duration) -> Option<Duration> {
    if elapsed >= timeout {
        return None;
    }

    let remaining = timeout.saturating_sub(elapsed);
    let poll = if elapsed < Duration::from_millis(2) {
        Duration::from_micros(250)
    } else if elapsed < Duration::from_millis(20) {
        Duration::from_millis(1)
    } else {
        GIT_COMMAND_WAIT_POLL_MAX
    };

    Some(poll.min(remaining))
}

fn spawn_read_pipe(
    pipe: Option<impl std::io::Read + Send + 'static>,
    activity: Option<(mpsc::Sender<(GitOutputStream, String)>, GitOutputStream)>,
) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let mut stream_pending = Vec::new();
        if let Some(mut reader) = pipe {
            let mut chunk = [0u8; 8192];
            loop {
                match reader.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        buf.extend_from_slice(&chunk[..read]);
                        if let Some((sender, stream)) = activity.as_ref() {
                            let text =
                                decode_stream_chunk(&mut stream_pending, &chunk[..read], false);
                            if !text.is_empty() {
                                let _ = sender.send((*stream, text));
                            }
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            if let Some((sender, stream)) = activity.as_ref() {
                let text = decode_stream_chunk(&mut stream_pending, &[], true);
                if !text.is_empty() {
                    let _ = sender.send((*stream, text));
                }
            }
        }
        buf
    })
}

/// Converts a stream incrementally, retaining an incomplete UTF-8 suffix for
/// the next read while escaping bytes that are definitively invalid.
fn decode_stream_chunk(pending: &mut Vec<u8>, bytes: &[u8], eof: bool) -> String {
    use std::fmt::Write as _;

    pending.extend_from_slice(bytes);
    let mut out = String::with_capacity(pending.len());
    let mut cursor = 0usize;
    while cursor < pending.len() {
        match std::str::from_utf8(&pending[cursor..]) {
            Ok(valid) => {
                out.push_str(valid);
                cursor = pending.len();
            }
            Err(error) => {
                let valid_len = error.valid_up_to();
                if valid_len > 0 {
                    let end = cursor + valid_len;
                    out.push_str(
                        std::str::from_utf8(&pending[cursor..end])
                            .expect("valid_up_to identified valid UTF-8"),
                    );
                    cursor = end;
                }
                let Some(invalid_len) = error.error_len() else {
                    if eof {
                        for byte in &pending[cursor..] {
                            let _ = write!(out, "\\x{byte:02x}");
                        }
                        cursor = pending.len();
                    }
                    break;
                };
                let end = cursor.saturating_add(invalid_len).min(pending.len());
                for byte in &pending[cursor..end] {
                    let _ = write!(out, "\\x{byte:02x}");
                }
                cursor = end;
            }
        }
    }
    if cursor > 0 {
        pending.drain(..cursor);
    }
    out
}

type ActivityOutputAggregator = (
    Option<mpsc::Sender<(GitOutputStream, String)>>,
    Option<thread::JoinHandle<()>>,
);

fn start_activity_output_aggregator(
    context: Option<&GitOperationContext>,
) -> ActivityOutputAggregator {
    let Some(context) = context.cloned() else {
        return (None, None);
    };
    let (sender, receiver) = mpsc::channel::<(GitOutputStream, String)>();
    let handle = thread::spawn(move || {
        let mut chunks = Vec::<GitOutputChunk>::new();
        let mut bytes = 0usize;
        loop {
            match receiver.recv_timeout(GIT_ACTIVITY_OUTPUT_FLUSH) {
                Ok((stream, text)) => {
                    bytes = bytes.saturating_add(text.len());
                    if let Some(last) = chunks.last_mut()
                        && last.stream == stream
                    {
                        last.text.push_str(&text);
                    } else {
                        chunks.push(GitOutputChunk { stream, text });
                    }
                    if bytes < GIT_ACTIVITY_OUTPUT_BATCH_BYTES {
                        continue;
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) if chunks.is_empty() => continue,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) if chunks.is_empty() => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    context.emit(GitOperationEvent::Output {
                        chunks: std::mem::take(&mut chunks),
                    });
                    break;
                }
            }
            context.emit(GitOperationEvent::Output {
                chunks: std::mem::take(&mut chunks),
            });
            bytes = 0;
        }
    });
    (Some(sender), Some(handle))
}

fn join_activity_output_aggregator(handle: Option<thread::JoinHandle<()>>) {
    if let Some(handle) = handle {
        let _ = handle.join();
    }
}

struct Trace2Monitor {
    _path: tempfile::TempPath,
    done: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Trace2Monitor {
    fn start(cmd: &mut Command, context: Option<&GitOperationContext>) -> Option<Self> {
        let context = context?.clone();
        let file = tempfile::Builder::new()
            .prefix("gitcomet-trace2-")
            .suffix(".json")
            .tempfile()
            .ok()?;
        let path = file.into_temp_path();
        cmd.env("GIT_TRACE2_EVENT", path.as_os_str());

        let done = Arc::new(AtomicBool::new(false));
        let thread_done = Arc::clone(&done);
        let thread_path = path.to_path_buf();
        let handle = thread::spawn(move || {
            trace2_tail_loop(&thread_path, &context, &thread_done);
        });
        Some(Self {
            _path: path,
            done,
            handle: Some(handle),
        })
    }

    fn finish(mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for Trace2Monitor {
    fn drop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

struct TracedHook {
    id: HookExecutionId,
    name: String,
    started: Instant,
}

fn trace2_tail_loop(path: &Path, context: &GitOperationContext, done: &AtomicBool) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    let mut pending = Vec::<u8>::new();
    let mut hooks = HashMap::<(String, u64), TracedHook>::new();
    loop {
        let before = pending.len();
        let _ = file.read_to_end(&mut pending);
        parse_trace2_lines(&mut pending, false, context, &mut hooks);
        if done.load(Ordering::Acquire) {
            let _ = file.read_to_end(&mut pending);
            parse_trace2_lines(&mut pending, true, context, &mut hooks);
            break;
        }
        if pending.len() == before {
            thread::sleep(GIT_TRACE2_POLL);
        }
    }
}

fn parse_trace2_lines(
    pending: &mut Vec<u8>,
    eof: bool,
    context: &GitOperationContext,
    hooks: &mut HashMap<(String, u64), TracedHook>,
) {
    let consumed = pending
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or_else(|| eof.then_some(pending.len()), |index| Some(index + 1));
    let Some(consumed) = consumed else {
        return;
    };
    let complete = pending.drain(..consumed).collect::<Vec<_>>();
    for line in complete.split(|byte| *byte == b'\n') {
        if line.is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else {
            continue;
        };
        apply_trace2_event(&value, context, hooks);
    }
}

fn trace2_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    value
        .get(key)
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
}

fn apply_trace2_event(
    value: &serde_json::Value,
    context: &GitOperationContext,
    hooks: &mut HashMap<(String, u64), TracedHook>,
) {
    let event = value.get("event").and_then(serde_json::Value::as_str);
    let sid = value.get("sid").and_then(serde_json::Value::as_str);
    let child_id = trace2_u64(value, "child_id");
    let (Some(event), Some(sid), Some(child_id)) = (event, sid, child_id) else {
        return;
    };
    let key = (sid.to_string(), child_id);
    match event {
        "child_start"
            if value.get("child_class").and_then(serde_json::Value::as_str) == Some("hook") =>
        {
            let Some(name) = value
                .get("hook_name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.is_empty())
            else {
                return;
            };
            let id = HookExecutionId {
                sid: Arc::<str>::from(sid),
                child_id,
            };
            hooks.insert(
                key,
                TracedHook {
                    id: id.clone(),
                    name: name.to_string(),
                    started: Instant::now(),
                },
            );
            context.emit(GitOperationEvent::HookStarted {
                id,
                name: name.to_string(),
            });
        }
        "child_exit" => {
            let Some(hook) = hooks.remove(&key) else {
                return;
            };
            let duration = value
                .get("t_rel")
                .and_then(serde_json::Value::as_f64)
                .filter(|seconds| seconds.is_finite() && *seconds >= 0.0)
                .map(Duration::from_secs_f64)
                .unwrap_or_else(|| hook.started.elapsed());
            let exit_code = value
                .get("code")
                .and_then(serde_json::Value::as_i64)
                .and_then(|code| i32::try_from(code).ok());
            context.emit(GitOperationEvent::HookFinished {
                id: hook.id,
                name: hook.name,
                exit_code,
                duration,
            });
        }
        _ => {}
    }
}

fn configure_git_process_tree(cmd: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;
        cmd.process_group(0);
    }

    #[cfg(not(unix))]
    {
        let _ = cmd;
    }
}

fn configure_non_interactive_git(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.stdin(Stdio::null());
}

pub(crate) fn install_test_git_command_environment(env: TestGitCommandEnvironment) {
    if let Some(existing) = TEST_GIT_COMMAND_ENVIRONMENT.get() {
        assert_eq!(
            existing, &env,
            "test git command environment already initialized"
        );
        return;
    }
    let _ = TEST_GIT_COMMAND_ENVIRONMENT.set(env);
}

fn apply_test_git_command_environment(cmd: &mut Command) {
    let Some(env) = TEST_GIT_COMMAND_ENVIRONMENT.get() else {
        return;
    };

    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    cmd.env("GIT_CONFIG_GLOBAL", &env.global_config);
    cmd.env("HOME", &env.home_dir);
    cmd.env("XDG_CONFIG_HOME", &env.xdg_config_home);
    cmd.env("GNUPGHOME", &env.gnupg_home);
    cmd.arg("-c").arg("protocol.file.allow=always");
}

pub(crate) fn git_workdir_cmd_for(workdir: &Path) -> Command {
    let mut cmd = git_command();
    apply_test_git_command_environment(&mut cmd);
    cmd.arg("-C").arg(workdir);
    cmd
}

fn command_may_require_auth(cmd: &Command) -> bool {
    let mut args = cmd.get_args();
    while let Some(arg) = args.next() {
        let Some(arg) = arg.to_str() else {
            return false;
        };
        match arg {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                let _ = args.next();
            }
            value if value.starts_with('-') => {}
            "clone" | "fetch" | "pull" | "push" | "submodule" | "ls-remote" | "commit" => {
                return true;
            }
            _ => return false,
        }
    }
    false
}

fn git_timeout_error(
    label: &str,
    timeout: Duration,
    exit_code: Option<i32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
) -> Error {
    Error::new(ErrorKind::Git(GitFailure::new(
        label,
        GitFailureId::Timeout,
        exit_code,
        stdout,
        stderr,
        Some(format!(
            "after {} seconds (set {GIT_COMMAND_TIMEOUT_ENV} to override)",
            timeout.as_secs()
        )),
    )))
}

pub(crate) fn git_command_failed_error(label: &str, output: Output) -> Error {
    let Output {
        status,
        stdout,
        stderr,
    } = output;
    let detail = [stderr.as_slice(), stdout.as_slice()]
        .into_iter()
        .map(bytes_to_text_preserving_utf8)
        .map(|text| text.trim().to_string())
        .find(|text| !text.is_empty())
        .map(add_git_failure_hint);
    Error::new(ErrorKind::Git(GitFailure::new(
        label,
        GitFailureId::CommandFailed,
        status.code(),
        stdout,
        stderr,
        detail,
    )))
}

fn add_git_failure_hint(mut detail: String) -> String {
    if git_failure_looks_like_missing_gpg(&detail)
        && !detail.contains("git config --global gpg.program")
    {
        detail.push_str(
            "\n\nHint: Git could not complete GPG signing. GitComet may be running with a GUI app PATH that differs from your shell PATH. If Git cannot find gpg, configure an absolute GPG path with `git config --global gpg.program /path/to/gpg`, or make gpg available on GitComet's PATH.",
        );
    }
    detail
}

fn git_failure_looks_like_missing_gpg(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("cannot run") && lower.contains("gpg")
}

/// The result of waiting on a spawned child process with a timeout.
struct ChildWaitOutcome {
    status: std::process::ExitStatus,
    /// The wait ended because the cancellation token was tripped (child killed).
    cancelled: bool,
    /// The wait ended because `timeout` elapsed (child killed).
    timed_out: bool,
    /// Timeout accounting continues until inherited output pipes close, not
    /// merely until the direct Git child exits.
    wait_started: Instant,
}

fn command_cancellation_requested(
    cancellation: Option<&CancellationToken>,
    operation_cancellation: Option<&CancellationToken>,
) -> bool {
    cancellation.is_some_and(CancellationToken::is_cancelled)
        || operation_cancellation.is_some_and(CancellationToken::is_cancelled)
}

fn reject_cancelled_command(
    cancellation: Option<&CancellationToken>,
    operation_cancellation: Option<&CancellationToken>,
) -> Result<()> {
    if command_cancellation_requested(cancellation, operation_cancellation) {
        Err(Error::new(ErrorKind::Cancelled))
    } else {
        Ok(())
    }
}

/// Block until `child` exits, the `cancellation` token is tripped, or `timeout`
/// elapses, polling with [`git_command_wait_poll`] backoff. On cancellation or
/// timeout the child is killed and reaped before returning. Callers drain
/// stdout/stderr (typically via reader threads) *after* this returns and map
/// `cancelled`/`timed_out` to their own error.
///
/// Single source of truth for the kill-then-wait / poll loop shared by every
/// long-running git invocation, so the cancellation and timeout semantics can't
/// drift between call sites.
fn wait_for_child_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
    operation_cancellation: Option<&CancellationToken>,
) -> Result<ChildWaitOutcome> {
    let start = Instant::now();
    let mut timed_out = false;
    let mut cancelled = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if command_cancellation_requested(cancellation, operation_cancellation) {
                    cancelled = true;
                    match terminate_process_tree_and_wait(child) {
                        Ok(status) => break status,
                        Err(e) => return Err(io_err(e)),
                    }
                }
                let elapsed = start.elapsed();
                if elapsed >= timeout {
                    timed_out = true;
                    match terminate_process_tree_and_wait(child) {
                        Ok(status) => break status,
                        Err(e) => return Err(io_err(e)),
                    }
                }
                if let Some(poll) = git_command_wait_poll(elapsed, timeout) {
                    thread::sleep(poll);
                }
            }
            Err(e) => return Err(io_err(e)),
        }
    };
    Ok(ChildWaitOutcome {
        status,
        cancelled,
        timed_out,
        wait_started: start,
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OutputDrainOutcome {
    cancelled: bool,
    timed_out: bool,
}

/// Keep ownership of the spawned process group until every worker consuming
/// inherited pipes has finished. A Git leader may exit while a hook descendant
/// remains alive with stdout/stderr open; Stop and timeout must still terminate
/// that group instead of blocking forever in `JoinHandle::join`.
fn wait_for_output_workers(
    child: &mut std::process::Child,
    workers_finished: impl Fn() -> bool,
    wait_started: Instant,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
    operation_cancellation: Option<&CancellationToken>,
) -> Result<OutputDrainOutcome> {
    while !workers_finished() {
        if command_cancellation_requested(cancellation, operation_cancellation) {
            terminate_process_tree_and_wait(child).map_err(io_err)?;
            return Ok(OutputDrainOutcome {
                cancelled: true,
                timed_out: false,
            });
        }
        if wait_started.elapsed() >= timeout {
            terminate_process_tree_and_wait(child).map_err(io_err)?;
            return Ok(OutputDrainOutcome {
                cancelled: false,
                timed_out: true,
            });
        }
        thread::sleep(GIT_COMMAND_WAIT_POLL_MAX);
    }
    Ok(OutputDrainOutcome::default())
}

fn terminate_process_tree_and_wait(
    child: &mut std::process::Child,
) -> io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group, test_kill_process_group};

        if let Some(pid) = Pid::from_raw(child.id() as i32) {
            let _ = kill_process_group(pid, Signal::TERM);
            let deadline = Instant::now() + GIT_PROCESS_TERMINATE_GRACE;
            let mut leader_status = None;
            loop {
                if leader_status.is_none() {
                    leader_status = child.try_wait()?;
                }
                // The process-group leader can exit before a TERM-resistant
                // hook descendant. Keep managing the group until it is truly
                // empty instead of using the leader's status as a proxy.
                if test_kill_process_group(pid).is_err() {
                    return match leader_status {
                        Some(status) => Ok(status),
                        None => child.wait(),
                    };
                }
                if Instant::now() >= deadline {
                    let _ = kill_process_group(pid, Signal::KILL);
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
            if let Some(status) = leader_status {
                return Ok(status);
            }
        } else {
            let _ = child.kill();
        }
        child.wait()
    }

    #[cfg(windows)]
    {
        // Windows has no Unix-style process groups. `taskkill /T` walks the
        // exact spawned Git process tree so a hook cannot keep running after
        // the user confirms Stop. Fall back to killing Git itself if the tree
        // walk races with process exit or is unavailable.
        let pid = child.id().to_string();
        let mut tree_kill = Command::new("taskkill");
        configure_background_command(&mut tree_kill);
        let _ = tree_kill.args(["/PID", pid.as_str(), "/T", "/F"]).status();
        let _ = child.kill();
        child.wait()
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = child.kill();
        child.wait()
    }
}

fn run_command_with_timeout(
    mut cmd: Command,
    label: &str,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<Output> {
    configure_background_command(&mut cmd);
    configure_git_process_tree(&mut cmd);
    configure_non_interactive_git(&mut cmd);
    let operation = git_operation::current();
    reject_cancelled_command(
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    let trace2 = Trace2Monitor::start(&mut cmd, operation.as_ref());
    let askpass_context = if command_may_require_auth(&cmd) {
        let auth = take_pending_git_auth();
        let script = create_askpass_script().map_err(io_err)?;
        configure_git_auth_prompt(&mut cmd, auth.as_ref(), &script);
        Some((script, auth))
    } else {
        None
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(io_err)?;

    let (activity_sender, activity_handle) = start_activity_output_aggregator(operation.as_ref());
    let stdout_handle = spawn_read_pipe(
        child.stdout.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stdout)),
    );
    let stderr_handle = spawn_read_pipe(
        child.stderr.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stderr)),
    );
    drop(activity_sender);

    let ChildWaitOutcome {
        status,
        mut cancelled,
        mut timed_out,
        wait_started,
    } = wait_for_child_with_timeout(
        &mut child,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;

    let drain = wait_for_output_workers(
        &mut child,
        || stdout_handle.is_finished() && stderr_handle.is_finished(),
        wait_started,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    cancelled |= drain.cancelled;
    timed_out |= drain.timed_out;

    let stdout = stdout_handle.join().unwrap_or_default();
    let mut stderr = stderr_handle.join().unwrap_or_default();
    join_activity_output_aggregator(activity_handle);
    if let Some(trace2) = trace2 {
        trace2.finish();
    }

    if let Some((askpass_script, _)) = askpass_context.as_ref() {
        append_host_prompt_to_stderr(&mut stderr, askpass_script);
    }

    if cancelled {
        return Err(Error::new(ErrorKind::Cancelled));
    }

    if timed_out {
        return Err(git_timeout_error(
            label,
            timeout,
            status.code(),
            stdout,
            stderr,
        ));
    }

    if let Some((askpass_script, auth)) = askpass_context.as_ref()
        && status.success()
    {
        remember_successful_prompt_auth(auth.as_ref(), askpass_script);
    }

    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub(crate) fn run_git_raw_output(cmd: Command, label: &str) -> Result<Output> {
    run_command_with_timeout(cmd, label, git_command_timeout(), None)
}

/// Run a local git command, feeding `input` to its stdin and returning captured
/// stdout. Used for `git blame --contents -`, where the file content to blame is
/// provided on stdin. Writes stdin and drains stdout/stderr on separate threads
/// so large inputs cannot deadlock the pipes.
pub(crate) fn run_git_with_stdin_capture(
    mut cmd: Command,
    input: Vec<u8>,
    label: &str,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<Vec<u8>> {
    use std::io::Write as _;

    configure_background_command(&mut cmd);
    configure_git_process_tree(&mut cmd);
    let operation = git_operation::current();
    reject_cancelled_command(
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    let trace2 = Trace2Monitor::start(&mut cmd, operation.as_ref());
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(io_err)?;

    let stdin = child.stdin.take();
    let writer = thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(&input);
            // Dropping `stdin` here closes the pipe so git sees EOF.
        }
    });
    let (activity_sender, activity_handle) = start_activity_output_aggregator(operation.as_ref());
    let stdout_handle = spawn_read_pipe(
        child.stdout.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stdout)),
    );
    let stderr_handle = spawn_read_pipe(
        child.stderr.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stderr)),
    );
    drop(activity_sender);

    let ChildWaitOutcome {
        status,
        mut cancelled,
        mut timed_out,
        wait_started,
    } = wait_for_child_with_timeout(
        &mut child,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;

    let drain = wait_for_output_workers(
        &mut child,
        || writer.is_finished() && stdout_handle.is_finished() && stderr_handle.is_finished(),
        wait_started,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    cancelled |= drain.cancelled;
    timed_out |= drain.timed_out;

    let _ = writer.join();
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    join_activity_output_aggregator(activity_handle);
    if let Some(trace2) = trace2 {
        trace2.finish();
    }

    if cancelled {
        return Err(Error::new(ErrorKind::Cancelled));
    }

    if timed_out {
        return Err(git_timeout_error(
            label,
            timeout,
            status.code(),
            stdout,
            stderr,
        ));
    }

    if !status.success() {
        return Err(git_command_failed_error(
            label,
            Output {
                status,
                stdout,
                stderr,
            },
        ));
    }
    Ok(stdout)
}

pub(crate) fn run_git_parsed_stdout<T, F>(
    cmd: Command,
    label: &str,
    allow_exit_code_one: bool,
    parse_stdout: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T> + Send + 'static,
{
    run_git_parsed_stdout_maybe_cancellable(cmd, label, allow_exit_code_one, None, parse_stdout)
}

pub(crate) fn run_git_parsed_stdout_cancellable<T, F>(
    cmd: Command,
    label: &str,
    allow_exit_code_one: bool,
    cancellation: &CancellationToken,
    parse_stdout: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T> + Send + 'static,
{
    run_git_parsed_stdout_maybe_cancellable(
        cmd,
        label,
        allow_exit_code_one,
        Some(cancellation),
        parse_stdout,
    )
}

fn run_git_parsed_stdout_maybe_cancellable<T, F>(
    mut cmd: Command,
    label: &str,
    allow_exit_code_one: bool,
    cancellation: Option<&CancellationToken>,
    parse_stdout: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ChildStdout) -> Result<T> + Send + 'static,
{
    configure_background_command(&mut cmd);
    configure_git_process_tree(&mut cmd);
    configure_non_interactive_git(&mut cmd);
    let operation = git_operation::current();
    reject_cancelled_command(
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    let trace2 = Trace2Monitor::start(&mut cmd, operation.as_ref());
    let askpass_context = if command_may_require_auth(&cmd) {
        let auth = take_pending_git_auth();
        let script = create_askpass_script().map_err(io_err)?;
        configure_git_auth_prompt(&mut cmd, auth.as_ref(), &script);
        Some((script, auth))
    } else {
        None
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(io_err)?;
    let stdout = child.stdout.take().ok_or_else(|| {
        Error::new(ErrorKind::Backend(format!(
            "{label} did not provide piped stdout"
        )))
    })?;
    let (activity_sender, activity_handle) = start_activity_output_aggregator(operation.as_ref());
    let stderr_handle = spawn_read_pipe(
        child.stderr.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stderr)),
    );
    drop(activity_sender);
    let stdout_handle = thread::spawn(move || parse_stdout(stdout));

    let timeout = git_command_timeout();
    let ChildWaitOutcome {
        status,
        mut cancelled,
        mut timed_out,
        wait_started,
    } = wait_for_child_with_timeout(
        &mut child,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;

    let drain = wait_for_output_workers(
        &mut child,
        || stdout_handle.is_finished() && stderr_handle.is_finished(),
        wait_started,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    cancelled |= drain.cancelled;
    timed_out |= drain.timed_out;

    let parsed_result = stdout_handle
        .join()
        .unwrap_or_else(|_| Err(Error::new(ErrorKind::Io(io::ErrorKind::Other))));
    let mut stderr = stderr_handle.join().unwrap_or_default();
    join_activity_output_aggregator(activity_handle);
    if let Some(trace2) = trace2 {
        trace2.finish();
    }

    if let Some((askpass_script, _)) = askpass_context.as_ref() {
        append_host_prompt_to_stderr(&mut stderr, askpass_script);
    }

    if cancelled {
        return Err(Error::new(ErrorKind::Cancelled));
    }

    if timed_out {
        return Err(git_timeout_error(
            label,
            timeout,
            status.code(),
            Vec::new(),
            stderr,
        ));
    }

    let ok_exit = status.success() || (allow_exit_code_one && status.code() == Some(1));
    if !ok_exit {
        return Err(git_command_failed_error(
            label,
            Output {
                status,
                stdout: Vec::new(),
                stderr,
            },
        ));
    }

    if let Some((askpass_script, auth)) = askpass_context.as_ref() {
        remember_successful_prompt_auth(auth.as_ref(), askpass_script);
    }

    parsed_result
}

fn run_git_checked_output(cmd: Command, label: &str) -> Result<Output> {
    let output = run_git_raw_output(cmd, label)?;
    if output.status.success() {
        Ok(output)
    } else {
        Err(git_command_failed_error(label, output))
    }
}

pub(crate) fn run_git_simple(cmd: Command, label: &str) -> Result<()> {
    run_git_checked_output(cmd, label)?;
    Ok(())
}

pub(crate) fn validate_ref_like_arg(value: &str, kind: &str) -> Result<()> {
    if value.is_empty() {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid {kind}: value is empty"
        ))));
    }
    if value.starts_with('-') {
        return Err(Error::new(ErrorKind::Backend(format!(
            "invalid {kind}: values starting with '-' are not allowed"
        ))));
    }
    Ok(())
}

pub(crate) fn validate_hex_commit_id(id: &CommitId) -> Result<()> {
    let value = id.as_ref();
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Error::new(ErrorKind::Backend(
            "invalid commit id: must contain only hexadecimal characters".to_string(),
        )));
    }
    Ok(())
}

pub(crate) fn path_buf_from_git_bytes(path_bytes: &[u8], context: &str) -> Result<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let _ = context;
        Ok(PathBuf::from(OsStr::from_bytes(path_bytes)))
    }

    #[cfg(windows)]
    {
        let path_text = std::str::from_utf8(path_bytes).map_err(|_| {
            Error::new(ErrorKind::Backend(format!(
                "{context}: non-UTF-8 git path bytes are not representable on Windows",
            )))
        })?;
        Ok(PathBuf::from(path_text))
    }
}

/// Renders a path as a platform-stable byte sequence for hashing config keys.
///
/// Raw `OsStr` bytes on Unix, UTF-16 LE units on Windows, so the same
/// repository path yields the same key across processes and reboots.
pub(crate) fn stable_path_bytes(path: &Path) -> Vec<u8> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        path.as_os_str().as_bytes().to_vec()
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut bytes = Vec::new();
        for unit in path.as_os_str().encode_wide() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        bytes
    }

    #[cfg(not(any(unix, windows)))]
    {
        path.to_str()
            .map(|text| text.as_bytes().to_vec())
            .unwrap_or_else(|| format!("{path:?}").into_bytes())
    }
}

pub(crate) fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

// Test helper: constructs a git stage:path blob spec for index stage testing.
#[cfg(test)]
pub(crate) fn git_stage_blob_spec(stage: u8, path: &Path) -> Result<OsString> {
    git_revision_with_path(&format!(":{stage}:"), path, "build conflict stage revision")
}

#[cfg(test)]
fn git_revision_with_path(prefix: &str, path: &Path, context: &str) -> Result<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt as _, OsStringExt as _};

        let _ = context;
        let path_bytes = path.as_os_str().as_bytes();
        let mut rev = Vec::with_capacity(prefix.len().saturating_add(path_bytes.len()));
        rev.extend_from_slice(prefix.as_bytes());
        rev.extend_from_slice(path_bytes);
        Ok(OsString::from_vec(rev))
    }

    #[cfg(windows)]
    {
        let path_text = path.to_str().ok_or_else(|| {
            Error::new(ErrorKind::Backend(format!(
                "{context}: non-Unicode path cannot be represented for git command arguments",
            )))
        })?;
        Ok(OsString::from(format!(
            "{prefix}{}",
            path_text.replace('\\', "/")
        )))
    }
}

fn command_path_budget_len(path: &Path) -> usize {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        path.as_os_str().as_bytes().len().saturating_add(1)
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        path.as_os_str()
            .encode_wide()
            .count()
            .saturating_mul(std::mem::size_of::<u16>())
            .saturating_add(std::mem::size_of::<u16>())
    }
}

pub(crate) fn run_git_simple_with_paths(
    workdir: &Path,
    label: &str,
    args: &[&str],
    paths: &[&Path],
) -> Result<()> {
    const MAX_PATH_BYTES_PER_CMD: usize = 28_000;
    const MAX_PATHS_PER_CMD: usize = 1024;

    let run_batch = |batch: &[&Path]| -> Result<()> {
        let mut cmd = git_workdir_cmd_for(workdir);
        cmd.args(args);
        if !batch.is_empty() {
            cmd.arg("--");
            for p in batch {
                cmd.arg(p);
            }
        }
        run_git_simple(cmd, label)
    };

    if paths.is_empty() {
        return run_batch(&[]);
    }

    let mut batch: Vec<&Path> = Vec::with_capacity(paths.len().min(MAX_PATHS_PER_CMD));
    let mut bytes: usize = 0;
    for path in paths {
        let path_len = command_path_budget_len(path);

        if !batch.is_empty()
            && (batch.len() >= MAX_PATHS_PER_CMD
                || bytes.saturating_add(path_len) > MAX_PATH_BYTES_PER_CMD)
        {
            run_batch(&batch)?;
            batch.clear();
            bytes = 0;
        }

        batch.push(*path);
        bytes = bytes.saturating_add(path_len);
    }

    if !batch.is_empty() {
        run_batch(&batch)?;
    }

    Ok(())
}

pub(crate) use gitcomet_core::process::bytes_to_text_preserving_utf8;

pub(crate) fn run_git_with_output(cmd: Command, label: &str) -> Result<CommandOutput> {
    let output = run_git_checked_output(cmd, label)?;
    let exit_code = output.status.code();
    let stdout = bytes_to_text_preserving_utf8(&output.stdout);
    let stderr = bytes_to_text_preserving_utf8(&output.stderr);
    Ok(CommandOutput {
        command: label.to_string(),
        stdout,
        stderr,
        exit_code,
    })
}

pub(crate) fn run_git_capture(cmd: Command, label: &str) -> Result<String> {
    let bytes = run_git_capture_bytes(cmd, label)?;
    Ok(bytes_to_text_preserving_utf8(&bytes))
}

pub(crate) fn run_git_capture_cancellable(
    cmd: Command,
    label: &str,
    cancellation: &CancellationToken,
) -> Result<String> {
    let output = run_command_with_timeout(cmd, label, git_command_timeout(), Some(cancellation))?;
    if output.status.success() {
        Ok(bytes_to_text_preserving_utf8(&output.stdout))
    } else {
        Err(git_command_failed_error(label, output))
    }
}

pub(crate) fn run_git_capture_bytes(cmd: Command, label: &str) -> Result<Vec<u8>> {
    let output = run_git_checked_output(cmd, label)?;
    Ok(output.stdout)
}

#[derive(Default)]
struct GitLogPrettyParseState {
    repeated_author: Option<Arc<str>>,
    next_commit_id_cache: Option<CommitId>,
}

impl GitLogPrettyParseState {
    fn push_record(&mut self, record: &str, commits: &mut Vec<Commit>) {
        let record = record.trim();
        if record.is_empty() {
            return;
        }
        let mut parts = record.split('\u{001f}');
        let Some(id) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            return;
        };
        let parents = parts.next().unwrap_or_default();
        let author = parts.next().unwrap_or_default();
        let time_secs = parts
            .next()
            .and_then(|s| s.trim().parse::<i64>().ok())
            .unwrap_or(0);
        let summary = parts.next().unwrap_or_default();

        let time = unix_seconds_to_system_time_or_epoch(time_secs);

        let id = if let Some(cached) = self.next_commit_id_cache.as_ref()
            && cached.as_ref() == id
        {
            cached.clone()
        } else {
            CommitId(id.into())
        };

        let parent_ids = parents
            .split_whitespace()
            .map(|p| CommitId(p.into()))
            .collect::<CommitParentIds>();

        self.next_commit_id_cache = parent_ids.first().cloned();

        let author = if let Some(cached) = self.repeated_author.as_ref()
            && cached.as_ref() == author
        {
            Arc::clone(cached)
        } else {
            let author: Arc<str> = author.into();
            self.repeated_author = Some(Arc::clone(&author));
            author
        };

        commits.push(Commit {
            id,
            parent_ids,
            summary: summary.into(),
            author,
            time,
        });
    }
}

#[cfg(test)]
pub(crate) fn parse_git_log_pretty_records(output: &str) -> LogPage {
    let approx_commits = output
        .as_bytes()
        .iter()
        .filter(|&&b| b == b'\x1e')
        .count()
        .saturating_add(1);
    let mut commits = Vec::with_capacity(approx_commits);
    let mut state = GitLogPrettyParseState::default();
    for record in output.split('\u{001e}') {
        state.push_record(record, &mut commits);
    }

    LogPage {
        commits,
        next_cursor: None,
    }
}

pub(crate) fn parse_git_log_pretty_records_from_reader(reader: impl io::Read) -> Result<LogPage> {
    let mut reader = io::BufReader::new(reader);
    let mut raw_record = Vec::new();
    let mut commits = Vec::new();
    let mut state = GitLogPrettyParseState::default();

    loop {
        raw_record.clear();
        let bytes_read = reader
            .read_until(b'\x1e', &mut raw_record)
            .map_err(io_err)?;
        if bytes_read == 0 {
            break;
        }
        if raw_record.last() == Some(&b'\x1e') {
            raw_record.pop();
        }
        let record = bytes_to_text_preserving_utf8(&raw_record);
        state.push_record(&record, &mut commits);
    }

    Ok(LogPage {
        commits,
        next_cursor: None,
    })
}

pub(crate) fn unix_seconds_to_system_time(seconds: i64) -> Option<SystemTime> {
    if seconds >= 0 {
        Some(SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64))
    } else {
        None
    }
}

pub(crate) fn unix_seconds_to_system_time_or_epoch(seconds: i64) -> SystemTime {
    unix_seconds_to_system_time(seconds).unwrap_or(SystemTime::UNIX_EPOCH)
}

// Test helper: parses `git branch -r` output for remote branch integration tests.
#[cfg(test)]
pub(crate) fn parse_remote_branches(output: &str) -> Vec<RemoteBranch> {
    let approx_branches = output
        .as_bytes()
        .iter()
        .filter(|&&b| b == b'\n')
        .count()
        .saturating_add(1);
    let mut branches = Vec::with_capacity(approx_branches);
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let Some(full_name) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        if full_name.ends_with("/HEAD") {
            continue;
        }
        let Some(sha) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let Some((remote, name)) = full_name.split_once('/') else {
            continue;
        };
        branches.push(RemoteBranch {
            remote: remote.to_string(),
            name: name.to_string(),
            target: CommitId(sha.into()),
        });
    }
    branches.sort_by(|a, b| a.remote.cmp(&b.remote).then_with(|| a.name.cmp(&b.name)));
    branches
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::auth::askpass::{
        GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV, GITCOMET_ASKPASS_PROMPT_LOG_ENV, PromptAuth,
    };
    use gitcomet_core::auth::{
        CachedPassphraseEntry, GITCOMET_AUTH_CACHE_SIZE_ENV, GITCOMET_AUTH_KIND_ENV,
        GITCOMET_AUTH_KIND_HOST_VERIFICATION, GITCOMET_AUTH_KIND_PASSPHRASE,
        GITCOMET_AUTH_KIND_PASSPHRASE_CACHED, GITCOMET_AUTH_KIND_USERNAME_PASSWORD,
        GITCOMET_AUTH_SECRET_ENV, GITCOMET_AUTH_USERNAME_ENV, GitAuthKind, StagedGitAuth,
    };
    use std::process::Command;
    #[cfg(unix)]
    use std::sync::Mutex;

    const GITPY_FOR_EACH_REF_WITH_PATH_COMPONENT: &[u8] =
        include_bytes!("../tests/fixtures/gitpython/for_each_ref_with_path_component");
    const GITPY_UNCOMMON_BRANCH_PREFIX_FETCH_HEAD: &str =
        include_str!("../tests/fixtures/gitpython/uncommon_branch_prefix_FETCH_HEAD");
    const GITPY_REV_LIST_SINGLE: &str = include_str!("../tests/fixtures/gitpython/rev_list_single");
    const GITPY_REV_LIST_COMMIT_STATS: &str =
        include_str!("../tests/fixtures/gitpython/rev_list_commit_stats");

    #[cfg(unix)]
    #[test]
    fn path_buf_from_git_bytes_preserves_non_utf8_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let raw_path = b"docs/\xff-topic.md";
        let path = path_buf_from_git_bytes(raw_path, "test").expect("path conversion");
        assert_eq!(path.as_os_str(), OsStr::from_bytes(raw_path));
    }

    #[cfg(unix)]
    #[test]
    fn git_stage_blob_spec_preserves_non_utf8_path_bytes() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt as _;

        let path = Path::new(OsStr::from_bytes(b"nested/\xff-file.bin"));
        let rev = git_stage_blob_spec(2, path).expect("stage spec");
        assert_eq!(rev.as_os_str().as_bytes(), b":2:nested/\xff-file.bin");
    }

    #[cfg(windows)]
    #[test]
    fn git_stage_blob_spec_normalizes_windows_separators() {
        let rev = git_stage_blob_spec(3, Path::new(r"nested\file.bin")).expect("stage spec");
        assert_eq!(
            rev.to_str()
                .expect("ascii revision should be valid unicode"),
            ":3:nested/file.bin"
        );
    }

    fn gitpython_fetch_head_to_remote_ref_output(fetch_head: &str, remote: &str) -> String {
        let mut out = String::new();
        for line in fetch_head.lines() {
            let Some((sha, rest)) = line.split_once('\t') else {
                continue;
            };
            let sha = sha.trim();
            if sha.is_empty() {
                continue;
            }
            let Some(start) = rest.find("'refs/") else {
                continue;
            };
            let refs_and_after = &rest[start + 1..];
            let Some((full_ref, _)) = refs_and_after.split_once('\'') else {
                continue;
            };
            let short_ref = full_ref.strip_prefix("refs/").unwrap_or(full_ref);
            out.push_str(remote);
            out.push('/');
            out.push_str(short_ref);
            out.push('\t');
            out.push_str(sha);
            out.push('\n');
        }
        out
    }

    #[cfg(unix)]
    fn shell_command(script: &str) -> Command {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(script);
        cmd
    }

    #[cfg(windows)]
    fn shell_command(script: &str) -> Command {
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-Command", script]);
        cmd
    }

    #[cfg(unix)]
    fn failing_command_with_streams() -> Command {
        shell_command("printf out; printf err >&2; exit 7")
    }

    #[cfg(windows)]
    fn failing_command_with_streams() -> Command {
        shell_command("[Console]::Out.Write('out'); [Console]::Error.Write('err'); exit 7")
    }

    #[cfg(unix)]
    fn failing_command_with_stdout_only() -> Command {
        shell_command("printf 'stdout only'; exit 9")
    }

    #[cfg(windows)]
    fn failing_command_with_stdout_only() -> Command {
        shell_command("[Console]::Out.Write('stdout only'); exit 9")
    }

    #[cfg(unix)]
    fn failing_command_with_missing_gpg() -> Command {
        shell_command(
            "printf 'error: cannot run gpg: No such file or directory\nerror: gpg failed to sign the data:\nfatal: failed to write commit object\n' >&2; exit 128",
        )
    }

    #[cfg(windows)]
    fn failing_command_with_missing_gpg() -> Command {
        shell_command(
            "[Console]::Error.Write(\"error: cannot run gpg: No such file or directory`nerror: gpg failed to sign the data:`nfatal: failed to write commit object`n\"); exit 128",
        )
    }

    #[cfg(unix)]
    fn failing_command_with_gpg_signing_error() -> Command {
        shell_command(
            "printf 'error: gpg failed to sign the data\nfatal: failed to write commit object\n' >&2; exit 128",
        )
    }

    #[cfg(windows)]
    fn failing_command_with_gpg_signing_error() -> Command {
        shell_command(
            "[Console]::Error.Write(\"error: gpg failed to sign the data`nfatal: failed to write commit object`n\"); exit 128",
        )
    }

    #[cfg(unix)]
    fn failing_command_with_missing_gpg_program_path() -> Command {
        shell_command(
            "printf 'error: cannot run /opt/homebrew/bin/gpg: Datei oder Verzeichnis nicht gefunden\nerror: gpg failed to sign the data:\nfatal: failed to write commit object\n' >&2; exit 128",
        )
    }

    #[cfg(windows)]
    fn failing_command_with_missing_gpg_program_path() -> Command {
        shell_command(
            "[Console]::Error.Write(\"error: cannot run C:/Program Files/Git/usr/bin/gpg.exe: Het systeem kan het opgegeven bestand niet vinden.`nerror: gpg failed to sign the data:`nfatal: failed to write commit object`n\"); exit 128",
        )
    }

    #[cfg(unix)]
    fn sleep_command(seconds: u64) -> Command {
        shell_command(&format!("sleep {seconds}"))
    }

    #[cfg(windows)]
    fn sleep_command(seconds: u64) -> Command {
        shell_command(&format!("Start-Sleep -Seconds {seconds}"))
    }

    #[cfg(unix)]
    fn run_git_test_setup(workdir: &Path, args: &[&str]) {
        let mut cmd = git_workdir_cmd_for(workdir);
        cmd.args(args);
        let output = cmd.output().expect("test Git command should start");
        assert!(
            output.status.success(),
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(unix)]
    fn write_test_hook(workdir: &Path, name: &str, script: &str) {
        use std::os::unix::fs::PermissionsExt as _;

        let hooks = workdir.join(".githooks");
        std::fs::create_dir_all(&hooks).expect("create test hooks directory");
        let path = hooks.join(name);
        std::fs::write(&path, script).expect("write test hook");
        let mut permissions = std::fs::metadata(&path)
            .expect("read test hook metadata")
            .permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions).expect("make test hook executable");
    }

    #[cfg(unix)]
    #[test]
    fn operation_context_reports_real_hooks_output_and_exit_codes() {
        let repo = tempfile::tempdir().expect("create test repository");
        run_git_test_setup(repo.path(), &["init", "--quiet"]);
        run_git_test_setup(repo.path(), &["config", "user.name", "GitComet Test"]);
        run_git_test_setup(
            repo.path(),
            &["config", "user.email", "gitcomet@example.invalid"],
        );
        run_git_test_setup(repo.path(), &["config", "commit.gpgsign", "false"]);
        run_git_test_setup(repo.path(), &["config", "core.hooksPath", ".githooks"]);
        write_test_hook(
            repo.path(),
            "pre-commit",
            "#!/bin/sh\nprintf 'pre-commit stdout\\n'\nprintf 'pre-commit stderr\\n' >&2\n",
        );
        write_test_hook(
            repo.path(),
            "post-commit",
            "#!/bin/sh\nprintf 'post-commit stdout\\n'\nprintf 'post-commit stderr\\n' >&2\nexit 7\n",
        );
        std::fs::write(repo.path().join("file.txt"), "content\n").expect("write test file");
        run_git_test_setup(repo.path(), &["add", "--", "file.txt"]);

        let events = Arc::new(Mutex::new(Vec::<GitOperationEvent>::new()));
        let captured = Arc::clone(&events);
        let operation = GitOperationContext::new("Commit", move |_, event| {
            captured
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(event);
        });
        let mut cmd = git_workdir_cmd_for(repo.path());
        cmd.args(["commit", "--quiet", "-m", "exercise hooks"]);
        {
            let _scope = git_operation::attach(&operation);
            run_git_simple(cmd, "git commit")
                .expect("post-commit failures must not fail the outer commit");
        }

        let events = events.lock().unwrap_or_else(|error| error.into_inner());
        let started = events
            .iter()
            .filter_map(|event| match event {
                GitOperationEvent::HookStarted { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(started, ["pre-commit", "post-commit"]);

        let finished = events
            .iter()
            .filter_map(|event| match event {
                GitOperationEvent::HookFinished {
                    name, exit_code, ..
                } => Some((name.as_str(), *exit_code)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            finished,
            [("pre-commit", Some(0)), ("post-commit", Some(7))]
        );

        let output = events
            .iter()
            .filter_map(|event| match event {
                GitOperationEvent::Output { chunks } => Some(chunks),
                _ => None,
            })
            .flatten()
            .map(|chunk| chunk.text.as_str())
            .collect::<String>();
        assert!(output.contains("pre-commit stdout"));
        assert!(output.contains("pre-commit stderr"));
        assert!(output.contains("post-commit stdout"));
        assert!(output.contains("post-commit stderr"));
    }

    #[test]
    fn run_git_with_output_failure_preserves_structured_details() {
        let err = run_git_with_output(failing_command_with_streams(), "git synthetic")
            .expect_err("expected failing command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                assert_eq!(failure.command(), "git synthetic");
                assert_eq!(failure.id(), GitFailureId::CommandFailed);
                assert_eq!(failure.exit_code(), Some(7));
                assert_eq!(failure.stdout(), b"out");
                assert_eq!(failure.stderr(), b"err");
                assert_eq!(failure.detail(), Some("err"));
                assert_eq!(failure.to_string(), "git synthetic failed: err");
            }
            other => panic!("expected structured git failure, got {other:?}"),
        }
    }

    #[test]
    fn run_git_with_output_failure_falls_back_to_stdout_when_stderr_is_empty() {
        let err = run_git_with_output(failing_command_with_stdout_only(), "git synthetic")
            .expect_err("expected failing command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                assert_eq!(failure.command(), "git synthetic");
                assert_eq!(failure.id(), GitFailureId::CommandFailed);
                assert_eq!(failure.exit_code(), Some(9));
                assert_eq!(failure.stdout(), b"stdout only");
                assert_eq!(failure.stderr(), b"");
                assert_eq!(failure.detail(), Some("stdout only"));
                assert_eq!(failure.to_string(), "git synthetic failed: stdout only");
            }
            other => panic!("expected structured git failure, got {other:?}"),
        }
    }

    #[test]
    fn run_git_failure_adds_gpg_signing_hint() {
        let err = run_git_with_output(failing_command_with_missing_gpg(), "git commit")
            .expect_err("expected failing command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                let detail = failure.detail().expect("expected failure detail");
                assert!(detail.contains("cannot run gpg"));
                assert!(detail.contains("GUI app PATH"));
                assert!(detail.contains("git config --global gpg.program /path/to/gpg"));
            }
            other => panic!("expected structured git failure, got {other:?}"),
        }
    }

    #[test]
    fn run_git_failure_does_not_add_path_hint_for_other_gpg_signing_errors() {
        let err = run_git_with_output(failing_command_with_gpg_signing_error(), "git commit")
            .expect_err("expected failing command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                let detail = failure.detail().expect("expected failure detail");
                assert!(detail.contains("gpg failed to sign the data"));
                assert!(!detail.contains("GUI app PATH"));
                assert!(!detail.contains("git config --global gpg.program /path/to/gpg"));
            }
            other => panic!("expected structured git failure, got {other:?}"),
        }
    }

    #[test]
    fn run_git_failure_adds_gpg_signing_hint_for_missing_gpg_program_path() {
        let err = run_git_with_output(
            failing_command_with_missing_gpg_program_path(),
            "git commit",
        )
        .expect_err("expected failing command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                let detail = failure.detail().expect("expected failure detail");
                assert!(detail.contains("cannot run"));
                assert!(detail.contains("gpg"));
                assert!(detail.contains("GUI app PATH"));
                assert!(detail.contains("git config --global gpg.program /path/to/gpg"));
            }
            other => panic!("expected structured git failure, got {other:?}"),
        }
    }

    #[test]
    fn run_command_with_timeout_returns_structured_timeout_failure() {
        let err = run_command_with_timeout(
            sleep_command(2),
            "git synthetic",
            Duration::from_millis(50),
            None,
        )
        .expect_err("expected timed out command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                assert_eq!(failure.command(), "git synthetic");
                assert_eq!(failure.id(), GitFailureId::Timeout);
                assert!(failure.detail().is_some_and(|detail| {
                    detail.contains("set GITCOMET_GIT_COMMAND_TIMEOUT_SECS to override")
                }));
                assert!(
                    failure
                        .to_string()
                        .starts_with("git synthetic timed out after")
                );
            }
            other => panic!("expected structured git timeout, got {other:?}"),
        }
    }

    #[test]
    fn git_command_wait_poll_is_short_for_fast_commands_and_capped_for_slow_ones() {
        assert_eq!(
            git_command_wait_poll(Duration::from_micros(500), Duration::from_secs(1)),
            Some(Duration::from_micros(250))
        );
        assert_eq!(
            git_command_wait_poll(Duration::from_millis(10), Duration::from_secs(1)),
            Some(Duration::from_millis(1))
        );
        assert_eq!(
            git_command_wait_poll(Duration::from_millis(50), Duration::from_secs(1)),
            Some(Duration::from_millis(5))
        );
        assert_eq!(
            git_command_wait_poll(Duration::from_millis(50), Duration::from_millis(52)),
            Some(Duration::from_millis(2))
        );
        assert_eq!(
            git_command_wait_poll(Duration::from_millis(50), Duration::from_millis(50)),
            None
        );
    }

    fn gitpython_rev_list_fixture_to_pretty_record(fixture: &str) -> String {
        let id = fixture
            .lines()
            .find_map(|line| line.strip_prefix("commit "))
            .expect("rev-list fixture should contain commit id")
            .trim();

        let parents = fixture
            .lines()
            .filter_map(|line| line.strip_prefix("parent "))
            .map(str::trim)
            .collect::<Vec<_>>()
            .join(" ");

        let author_line = fixture
            .lines()
            .find(|line| line.starts_with("author "))
            .expect("rev-list fixture should contain author line");
        let author = author_line
            .strip_prefix("author ")
            .and_then(|line| line.split_once(" <").map(|(name, _)| name))
            .expect("author line should include actor and email");
        let time = author_line
            .split_whitespace()
            .rev()
            .nth(1)
            .expect("author line should contain unix timestamp")
            .trim();

        let summary = fixture
            .lines()
            .find_map(|line| line.strip_prefix("    "))
            .unwrap_or_default()
            .trim();

        format!("{id}\x1f{parents}\x1f{author}\x1f{time}\x1f{summary}\x1e")
    }

    #[test]
    fn parse_remote_branches_splits_and_skips_head() {
        let output =
            "origin/HEAD\tdeadbeef\norigin/main\t1111111\nupstream/feature/foo\t2222222\n\n";
        let branches = parse_remote_branches(output);
        assert_eq!(
            branches,
            vec![
                RemoteBranch {
                    remote: "origin".to_string(),
                    name: "main".to_string(),
                    target: CommitId("1111111".into())
                },
                RemoteBranch {
                    remote: "upstream".to_string(),
                    name: "feature/foo".to_string(),
                    target: CommitId("2222222".into())
                },
            ]
        );
    }

    #[test]
    fn unix_seconds_to_system_time_clamps_negative_to_epoch() {
        assert_eq!(
            unix_seconds_to_system_time_or_epoch(-1),
            SystemTime::UNIX_EPOCH
        );
        assert_eq!(
            unix_seconds_to_system_time_or_epoch(1),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1)
        );
    }

    #[test]
    fn parse_remote_branches_handles_path_components_from_gitpython_fixture() {
        let raw = std::str::from_utf8(GITPY_FOR_EACH_REF_WITH_PATH_COMPONENT)
            .expect("fixture should be valid UTF-8");
        let mut fields = raw.trim().split('\0');
        let full_ref = fields.next().expect("refname field");
        let oid = fields.next().expect("object id field");
        let short = full_ref
            .strip_prefix("refs/heads/")
            .expect("heads ref prefix in fixture");

        let output = format!("origin/{short}\t{oid}\norigin/HEAD\tdeadbeef\n");
        let branches = parse_remote_branches(&output);

        assert_eq!(branches.len(), 1);
        assert_eq!(branches[0].remote, "origin");
        assert_eq!(branches[0].name, "refactoring/feature1");
        assert_eq!(branches[0].target, CommitId(oid.to_string().into()));
    }

    #[test]
    fn parse_git_log_pretty_records_parses_single_commit_from_gitpython_fixture() {
        let output = gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_SINGLE);
        let page = parse_git_log_pretty_records(&output);

        assert_eq!(page.commits.len(), 1);
        assert!(page.next_cursor.is_none());
        let commit = &page.commits[0];
        assert_eq!(
            commit.id,
            CommitId("4c8124ffcf4039d292442eeccabdeca5af5c5017".into())
        );
        assert_eq!(
            commit.parent_ids.as_slice(),
            &[CommitId("634396b2f541a9f2d58b00be1a07f0c358b999b3".into())]
        );
        assert_eq!(&*commit.author, "Tom Preston-Werner");
        assert_eq!(&*commit.summary, "implement Grit#heads");
        assert_eq!(
            commit.time,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_191_999_972)
        );
    }

    #[test]
    fn parse_git_log_pretty_records_parses_multiple_gitpython_fixtures() {
        let output = format!(
            "{}{}",
            gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_SINGLE),
            gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_COMMIT_STATS)
        );
        let page = parse_git_log_pretty_records(&output);

        assert_eq!(page.commits.len(), 2);
        assert!(page.next_cursor.is_none());

        assert_eq!(
            page.commits[1].id,
            CommitId("634396b2f541a9f2d58b00be1a07f0c358b999b3".into())
        );
        assert!(page.commits[1].parent_ids.is_empty());
        assert_eq!(&*page.commits[1].author, "Tom Preston-Werner");
        assert_eq!(&*page.commits[1].summary, "initial grit setup");
        assert!(Arc::ptr_eq(
            &page.commits[0].author,
            &page.commits[1].author
        ));
        assert!(Arc::ptr_eq(
            &page.commits[0].parent_ids[0].0,
            &page.commits[1].id.0
        ));
        assert_eq!(
            page.commits[1].time,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_191_997_100)
        );
    }

    #[test]
    fn parse_git_log_pretty_records_from_reader_matches_string_parser() {
        let output = format!(
            "{}{}",
            gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_SINGLE),
            gitpython_rev_list_fixture_to_pretty_record(GITPY_REV_LIST_COMMIT_STATS)
        );

        let from_string = parse_git_log_pretty_records(&output);
        let from_reader =
            parse_git_log_pretty_records_from_reader(std::io::Cursor::new(output.as_bytes()))
                .expect("streaming parser");

        assert_eq!(from_reader, from_string);
        assert!(Arc::ptr_eq(
            &from_reader.commits[0].author,
            &from_reader.commits[1].author
        ));
        assert!(Arc::ptr_eq(
            &from_reader.commits[0].parent_ids[0].0,
            &from_reader.commits[1].id.0
        ));
    }

    #[test]
    fn parse_remote_branches_handles_pull_ref_prefixes_from_gitpython_fixture() {
        let mut output = gitpython_fetch_head_to_remote_ref_output(
            GITPY_UNCOMMON_BRANCH_PREFIX_FETCH_HEAD,
            "origin",
        );
        output.push_str("origin/HEAD\tdeadbeef\n");
        let branches = parse_remote_branches(&output);

        let names = branches.iter().map(|b| b.name.as_str()).collect::<Vec<_>>();
        assert_eq!(
            names,
            vec![
                "pull/1/head",
                "pull/1/merge",
                "pull/2/head",
                "pull/2/merge",
                "pull/3/head",
                "pull/3/merge",
            ]
        );
        assert_eq!(branches.len(), 6);
        assert_eq!(
            branches[0].target,
            CommitId("c2e3c20affa3e2b61a05fdc9ee3061dd416d915e".into())
        );
    }

    #[test]
    fn command_may_require_auth_detects_auth_related_git_commands() {
        let mut push = Command::new("git");
        push.args(["-C", "/tmp/repo", "push", "origin", "main"]);
        assert!(command_may_require_auth(&push));

        let mut fetch = Command::new("git");
        fetch.args(["-c", "color.ui=false", "fetch", "--all"]);
        assert!(command_may_require_auth(&fetch));

        let mut ls_remote = Command::new("git");
        ls_remote.args(["ls-remote", "origin"]);
        assert!(command_may_require_auth(&ls_remote));

        let mut commit = Command::new("git");
        commit.args(["commit", "-m", "msg"]);
        assert!(command_may_require_auth(&commit));

        let mut status = Command::new("git");
        status.args(["-C", "/tmp/repo", "status", "--short"]);
        assert!(!command_may_require_auth(&status));

        let mut log = Command::new("git");
        log.args(["log", "--oneline", "-n", "1"]);
        assert!(!command_may_require_auth(&log));
    }

    #[test]
    fn create_askpass_script_writes_expected_content_and_permissions() {
        let askpass = create_askpass_script().expect("askpass script creation");
        assert!(askpass.path().exists());

        let contents =
            std::fs::read_to_string(askpass.path()).expect("askpass script should be readable");
        assert!(contents.contains("GITCOMET_AUTH_SECRET"));
        assert!(contents.contains("GITCOMET_AUTH_KIND"));
        assert!(contents.contains("host_verification"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mode = std::fs::metadata(askpass.path())
                .expect("askpass metadata")
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o700);
        }
    }

    #[test]
    fn append_host_prompt_to_stderr_includes_logged_prompt_with_fingerprint() {
        let askpass = create_askpass_script().expect("askpass script creation");
        std::fs::write(
            askpass.host_prompt_log_path(),
            "The authenticity of host 'github.com (140.82.121.3)' can't be established.\nED25519 key fingerprint is: SHA256:+DiY...\nAre you sure you want to continue connecting (yes/no/[fingerprint])?",
        )
        .expect("write prompt log");

        let mut stderr = b"Host key verification failed.\n".to_vec();
        append_host_prompt_to_stderr(&mut stderr, &askpass);

        let rendered = String::from_utf8(stderr).expect("stderr should be utf-8 for test");
        assert!(rendered.contains("SSH host verification prompt:"));
        assert!(rendered.contains("ED25519 key fingerprint is: SHA256:+DiY..."));
        assert!(rendered.contains("yes/no/[fingerprint]"));
    }

    #[test]
    fn append_host_prompt_to_stderr_skips_when_prompt_already_present() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let prompt = "Are you sure you want to continue connecting (yes/no/[fingerprint])?";
        std::fs::write(askpass.host_prompt_log_path(), prompt).expect("write prompt log");

        let mut stderr = format!("Host key verification failed.\n{prompt}\n").into_bytes();
        append_host_prompt_to_stderr(&mut stderr, &askpass);

        let rendered = String::from_utf8(stderr).expect("stderr should be utf-8 for test");
        assert_eq!(rendered.matches("SSH host verification prompt:").count(), 0);
        assert_eq!(rendered.matches(prompt).count(), 1);
    }

    fn command_env_value(cmd: &Command, key: &str) -> Option<String> {
        use std::ffi::OsStr;

        cmd.get_envs().find_map(|(k, v)| {
            if k == OsStr::new(key) {
                v.and_then(|value| value.to_str().map(ToOwned::to_owned))
            } else {
                None
            }
        })
    }

    fn command_env_removed(cmd: &Command, key: &str) -> bool {
        use std::ffi::OsStr;

        cmd.get_envs()
            .any(|(k, v)| k == OsStr::new(key) && v.is_none())
    }

    #[test]
    fn configure_git_auth_prompt_sets_username_password_env() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        let auth = PromptAuth::Explicit(StagedGitAuth {
            kind: GitAuthKind::UsernamePassword,
            username: Some("alice".to_string()),
            secret: "secret-token".to_string(),
        });

        configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

        let askpass_path = askpass
            .path()
            .to_str()
            .expect("temporary askpass path should be unicode")
            .to_string();
        assert_eq!(
            command_env_value(&cmd, "GIT_ASKPASS").as_deref(),
            Some(askpass_path.as_str())
        );
        assert_eq!(
            command_env_value(&cmd, "SSH_ASKPASS").as_deref(),
            Some(askpass_path.as_str())
        );
        assert_eq!(
            command_env_value(&cmd, "SSH_ASKPASS_REQUIRE").as_deref(),
            Some("force")
        );
        assert_eq!(
            command_env_value(&cmd, GITCOMET_ASKPASS_PROMPT_LOG_ENV).as_deref(),
            askpass.host_prompt_log_path().to_str()
        );
        assert_eq!(
            command_env_value(&cmd, GITCOMET_ASKPASS_PASSPHRASE_PROMPT_LOG_ENV).as_deref(),
            askpass.passphrase_prompt_log_path().to_str()
        );
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
            Some(GITCOMET_AUTH_KIND_USERNAME_PASSWORD)
        );
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_USERNAME_ENV).as_deref(),
            Some("alice")
        );
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_SECRET_ENV).as_deref(),
            Some("secret-token")
        );
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_CACHE_SIZE_ENV).as_deref(),
            Some("0")
        );

        if cfg!(all(unix, not(target_os = "macos"))) && std::env::var_os("DISPLAY").is_none() {
            assert_eq!(
                command_env_value(&cmd, "DISPLAY").as_deref(),
                Some("gitcomet:0")
            );
        }
    }

    #[test]
    fn configure_git_auth_prompt_sets_passphrase_env_and_removes_username() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
        let auth = PromptAuth::Explicit(StagedGitAuth {
            kind: GitAuthKind::Passphrase,
            username: None,
            secret: "ssh-passphrase".to_string(),
        });

        configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
            Some(GITCOMET_AUTH_KIND_PASSPHRASE)
        );
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_SECRET_ENV).as_deref(),
            Some("ssh-passphrase")
        );
    }

    #[test]
    fn configure_git_auth_prompt_sets_cached_passphrase_env_and_removes_username() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
        let auth = PromptAuth::CachedPassphrases(vec![
            CachedPassphraseEntry {
                prompt: "Enter passphrase for key '/tmp/key-a':".to_string(),
                secret: "ssh-passphrase-a".to_string(),
            },
            CachedPassphraseEntry {
                prompt: "Enter passphrase for key '/tmp/key-b':".to_string(),
                secret: "ssh-passphrase-b".to_string(),
            },
        ]);

        configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
            Some(GITCOMET_AUTH_KIND_PASSPHRASE_CACHED)
        );
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_SECRET_ENV));
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_CACHE_SIZE_ENV).as_deref(),
            Some("2")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_0").as_deref(),
            Some("Enter passphrase for key '/tmp/key-a':")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_0").as_deref(),
            Some("ssh-passphrase-a")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_PROMPT_1").as_deref(),
            Some("Enter passphrase for key '/tmp/key-b':")
        );
        assert_eq!(
            command_env_value(&cmd, "GITCOMET_AUTH_CACHE_SECRET_1").as_deref(),
            Some("ssh-passphrase-b")
        );
    }

    #[test]
    fn configure_git_auth_prompt_sets_host_verification_env_and_removes_username() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
        let auth = PromptAuth::Explicit(StagedGitAuth {
            kind: GitAuthKind::HostVerification,
            username: None,
            secret: "yes".to_string(),
        });

        configure_git_auth_prompt(&mut cmd, Some(&auth), &askpass);

        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_KIND_ENV).as_deref(),
            Some(GITCOMET_AUTH_KIND_HOST_VERIFICATION)
        );
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
        assert_eq!(
            command_env_value(&cmd, GITCOMET_AUTH_SECRET_ENV).as_deref(),
            Some("yes")
        );
    }

    #[test]
    fn configure_git_auth_prompt_without_staged_auth_clears_auth_env() {
        let askpass = create_askpass_script().expect("askpass script creation");
        let mut cmd = Command::new("git");
        cmd.env(GITCOMET_AUTH_KIND_ENV, "legacy-kind");
        cmd.env(GITCOMET_AUTH_USERNAME_ENV, "legacy-user");
        cmd.env(GITCOMET_AUTH_SECRET_ENV, "legacy-secret");

        configure_git_auth_prompt(&mut cmd, None, &askpass);

        let askpass_path = askpass
            .path()
            .to_str()
            .expect("temporary askpass path should be unicode")
            .to_string();
        assert_eq!(
            command_env_value(&cmd, "GIT_ASKPASS").as_deref(),
            Some(askpass_path.as_str())
        );
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_KIND_ENV));
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_USERNAME_ENV));
        assert!(command_env_removed(&cmd, GITCOMET_AUTH_SECRET_ENV));
    }

    #[test]
    fn run_git_with_stdin_capture_times_out_when_command_exceeds_timeout() {
        let err = run_git_with_stdin_capture(
            sleep_command(2),
            vec![],
            "git synthetic",
            Duration::from_millis(50),
            None,
        )
        .expect_err("expected timed out command");

        match err.kind() {
            ErrorKind::Git(failure) => {
                assert_eq!(failure.command(), "git synthetic");
                assert_eq!(failure.id(), GitFailureId::Timeout);
            }
            other => panic!("expected structured git timeout, got {other:?}"),
        }
    }

    #[test]
    fn run_git_with_stdin_capture_respects_cancellation() {
        let token = CancellationToken::new();
        let child_token = token.clone();

        let handle = thread::spawn(move || {
            run_git_with_stdin_capture(
                sleep_command(10),
                vec![],
                "git synthetic",
                Duration::from_secs(30),
                Some(&child_token),
            )
        });

        thread::sleep(Duration::from_millis(50));
        token.cancel();

        let result = handle.join().expect("thread should not panic");
        match result {
            Err(err) => match err.kind() {
                ErrorKind::Cancelled => {}
                other => panic!("expected cancellation error, got {other:?}"),
            },
            Ok(_) => panic!("expected cancellation error, but command succeeded"),
        }
    }

    #[test]
    fn pre_cancelled_command_returns_cancelled_before_spawn() {
        let token = CancellationToken::new();
        token.cancel();
        let missing_command = Command::new("gitcomet-command-that-must-not-be-spawned");

        let error = run_command_with_timeout(
            missing_command,
            "git synthetic",
            Duration::from_secs(1),
            Some(&token),
        )
        .expect_err("an already-cancelled operation must reject the command");

        assert!(
            matches!(error.kind(), ErrorKind::Cancelled),
            "cancellation must win before spawn, got {error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_after_leader_exit_stops_pipe_holding_descendant() {
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let handle = thread::spawn(move || {
            run_command_with_timeout(
                shell_command("sleep 3 &"),
                "git synthetic",
                Duration::from_secs(10),
                Some(&worker_token),
            )
        });

        // The shell leader exits immediately; its background child keeps the
        // captured stdout/stderr descriptors open.
        thread::sleep(Duration::from_millis(150));
        let cancelled_at = Instant::now();
        token.cancel();
        let result = handle.join().expect("command thread should not panic");

        assert!(
            matches!(result, Err(ref error) if matches!(error.kind(), ErrorKind::Cancelled)),
            "Stop must remain observable while output readers drain, got {result:?}"
        );
        assert!(
            cancelled_at.elapsed() < Duration::from_secs(1),
            "the pipe-holding descendant was not terminated promptly"
        );
    }

    #[test]
    fn operation_registry_cancellation_stops_the_attached_process() {
        let operation = GitOperationContext::new("synthetic", |_, _| {});
        let worker_operation = operation.clone();
        let handle = thread::spawn(move || {
            let _scope = git_operation::attach(&worker_operation);
            run_git_with_stdin_capture(
                sleep_command(10),
                vec![],
                "git synthetic",
                Duration::from_secs(30),
                None,
            )
        });

        thread::sleep(Duration::from_millis(50));
        assert!(git_operation::cancel(operation.id()));

        let result = handle.join().expect("thread should not panic");
        match result {
            Err(err) => match err.kind() {
                ErrorKind::Cancelled => {}
                other => panic!("expected cancellation error, got {other:?}"),
            },
            Ok(_) => panic!("expected cancellation error, but command succeeded"),
        }
    }

    #[cfg(unix)]
    #[test]
    fn process_tree_termination_keeps_the_group_grace_when_the_leader_exits() {
        use rustix::process::{Pid, Signal, kill_process_group};

        let mut cmd = shell_command(
            "trap 'exit 0' TERM; (trap '' TERM; sleep 10) & while :; do sleep 1; done",
        );
        configure_git_process_tree(&mut cmd);
        let mut child = cmd.spawn().expect("synthetic process group should start");
        let group_id = child.id();
        thread::sleep(Duration::from_millis(100));

        let started = Instant::now();
        let result = terminate_process_tree_and_wait(&mut child);
        let elapsed = started.elapsed();

        // Always clean up the intentionally TERM-resistant descendant when
        // this regression fails against the old implementation.
        if let Some(pid) = Pid::from_raw(group_id as i32) {
            let _ = kill_process_group(pid, Signal::KILL);
        }
        result.expect("process-group termination should reap the leader");

        assert!(
            elapsed >= GIT_PROCESS_TERMINATE_GRACE.saturating_sub(Duration::from_millis(100)),
            "termination returned after {elapsed:?}, before TERM-resistant descendants received the final KILL"
        );
    }

    #[test]
    fn run_git_with_stdin_capture_forwards_stdin_and_captures_stdout() {
        #[cfg(unix)]
        let cmd = shell_command("cat");
        #[cfg(windows)]
        let cmd = shell_command("[Console]::Out.Write([Console]::In.ReadToEnd())");

        let input = b"hello stdin\nline two\n".to_vec();

        let output = run_git_with_stdin_capture(
            cmd,
            input.clone(),
            "cat stdin",
            Duration::from_secs(5),
            None,
        )
        .expect("stdin forwarding should succeed");

        assert_eq!(output, input);
    }
}
