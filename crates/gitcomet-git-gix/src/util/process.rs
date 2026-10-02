//! Running git: command setup, auth, timeouts, cancellation, and process-tree cleanup.

use super::*;

/// Poll cadence for a child wait: tight right after spawn so short commands
/// return promptly, capped afterwards, never past the silence budget left.
pub(super) fn git_command_wait_poll(
    since_start: Duration,
    remaining: Duration,
) -> Option<Duration> {
    if remaining.is_zero() {
        return None;
    }

    let poll = if since_start < Duration::from_millis(2) {
        Duration::from_micros(250)
    } else if since_start < Duration::from_millis(20) {
        Duration::from_millis(1)
    } else {
        GIT_COMMAND_WAIT_POLL_MAX
    };

    Some(poll.min(remaining))
}

pub(super) fn configure_git_process_tree(cmd: &mut Command) {
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

pub(super) fn configure_non_interactive_git(cmd: &mut Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.stdin(Stdio::null());
}

/// For reads nobody is waiting on: credential helpers may answer from stored
/// credentials but must not open a sign-in dialog. Call before the subcommand.
pub(crate) fn credentials_without_prompts(cmd: &mut Command) {
    cmd.args(["-c", "credential.interactive=false"]);
    cmd.env("GCM_INTERACTIVE", "Never");
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

pub(super) fn apply_test_git_command_environment(cmd: &mut Command) {
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

pub(super) fn command_may_require_auth(cmd: &Command) -> bool {
    // Network commands need credentials. The remaining commands can create
    // signatures and, with `gpg.format = ssh`, invoke `ssh-keygen -Y sign`,
    // which also obtains its passphrase through askpass. Git LFS and git-annex
    // reach their servers through Git's credentials.
    git_subcommand(cmd, false).is_some_and(|(subcommand, _)| {
        matches!(
            subcommand,
            "clone"
                | "fetch"
                | "pull"
                | "push"
                | "submodule"
                | "ls-remote"
                | "commit"
                | "commit-tree"
                | "tag"
                | "merge"
                | "rebase"
                | "cherry-pick"
                | "revert"
                | "am"
                | "lfs"
                | "annex"
        )
    })
}

pub(super) fn git_timeout_error(
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
            "after {} seconds without output (set {GIT_COMMAND_TIMEOUT_ENV} to override)",
            timeout.as_secs()
        )),
    )))
}

/// "a.txt", "a.txt and b.txt", or "a.txt, b.txt and 3 more" for messages.
pub(crate) fn describe_path_list<P: AsRef<[u8]>>(paths: &[P]) -> String {
    const SHOWN: usize = 3;
    let names: Vec<String> = paths
        .iter()
        .take(if paths.len() > SHOWN + 1 {
            SHOWN
        } else {
            paths.len()
        })
        .map(|path| bytes_to_text_preserving_utf8(path.as_ref()))
        .collect();
    match (names.as_slice(), paths.len() - names.len()) {
        ([], _) => String::new(),
        ([only], 0) => only.clone(),
        ([init @ .., last], 0) => format!("{} and {last}", init.join(", ")),
        (shown, more) => format!("{} and {more} more", shown.join(", ")),
    }
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
    let id = classify_git_failure(&String::from_utf8_lossy(&stderr));
    let detail = detail.map(|detail| add_lfs_failure_hint(detail, id));
    Error::new(ErrorKind::Git(GitFailure::new(
        label,
        id,
        status.code(),
        stdout,
        stderr,
        detail,
    )))
}

pub(super) fn add_git_failure_hint(mut detail: String) -> String {
    if git_failure_looks_like_missing_gpg(&detail)
        && !detail.contains("git config --global gpg.program")
    {
        let name = gitcomet_core::identity::current().display_name();
        detail.push_str(&format!(
            "\n\nHint: Git could not complete GPG signing. {name} may be running with a GUI app PATH that differs from your shell PATH. If Git cannot find gpg, configure an absolute GPG path with `git config --global gpg.program /path/to/gpg`, or make gpg available on {name}'s PATH.",
        ));
    }
    detail
}

/// Recognise Git LFS failures, which arrive as filter or hook stderr on an
/// otherwise ordinary command, so the UI can say what to do next.
pub(super) fn classify_git_failure(stderr: &str) -> GitFailureId {
    let lower = stderr.to_ascii_lowercase();
    // One line must name git-lfs as the program that is missing: a present
    // git-lfs also says "no such file or directory" about a missing object.
    let lfs_binary_missing = lower.lines().any(|line| {
        line.contains("'git-lfs' was not found on your path")
            || line.contains("'lfs' is not a git command")
            || (line.contains("git-lfs")
                && (line.contains("command not found")
                    || line.contains(": not found")
                    || line.contains("is not recognized as")
                    || ((line.contains("cannot run git-lfs")
                        || line.contains("cannot spawn git-lfs"))
                        && line.contains("no such file or directory"))))
    });
    if lfs_binary_missing {
        GitFailureId::LfsNotInstalled
    } else if lower.contains("unable to push locked files")
        || lower.contains("lock exists")
        || lower.contains("already created lock")
    {
        GitFailureId::LfsLocked
    } else if lower.contains("lfs upload failed")
        || lower.contains("unable to find source for object")
    {
        GitFailureId::LfsUploadFailed
    } else if lower.contains("smudge error")
        || lower.contains("smudge filter lfs failed")
        || lower.contains("object does not exist on the server")
    {
        GitFailureId::LfsObjectMissing
    } else {
        GitFailureId::CommandFailed
    }
}

pub(super) fn add_lfs_failure_hint(mut detail: String, id: GitFailureId) -> String {
    let name = gitcomet_core::identity::current().display_name();
    let hint: std::borrow::Cow<'static, str> = match id {
        GitFailureId::LfsNotInstalled => format!(
            "Git LFS is not installed where Git can find it. Install git-lfs, then run `git lfs install`. Settings > Executables shows whether {name} can see it."
        )
        .into(),
        GitFailureId::LfsObjectMissing => "An LFS object could not be downloaded. Check that the remote has it (`git lfs fetch --all`) and that you can authenticate to its LFS server.".into(),
        GitFailureId::LfsLocked => "Another user holds an LFS lock on a file you changed. Ask them to unlock it, or unlock it yourself if you have permission.".into(),
        GitFailureId::LfsUploadFailed => "LFS objects could not be uploaded. Run `git lfs push --all <remote>` or check the LFS server's credentials.".into(),
        _ => return detail,
    };
    detail.push_str("\n\nHint: ");
    detail.push_str(&hint);
    detail
}

pub(super) fn git_failure_looks_like_missing_gpg(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("cannot run") && lower.contains("gpg")
}

/// The result of waiting on a spawned child process with a timeout.
pub(super) struct ChildWaitOutcome {
    pub(super) status: std::process::ExitStatus,
    /// The wait ended because the cancellation token was tripped (child killed).
    pub(super) cancelled: bool,
    /// The wait ended because the child stayed silent for `timeout` (child killed).
    pub(super) timed_out: bool,
}

/// Last sign of life from a child's process tree: an output chunk, a trace2
/// event, or a progress line. Deadlines measure silence rather than run time,
/// so a long transfer that keeps reporting is never killed mid-way.
#[derive(Clone)]
pub(super) struct LivenessClock {
    epoch: Instant,
    last_millis: Arc<AtomicU64>,
}

impl LivenessClock {
    pub(super) fn new() -> Self {
        Self {
            epoch: Instant::now(),
            last_millis: Arc::new(AtomicU64::new(0)),
        }
    }

    pub(super) fn touch(&self) {
        let now = u64::try_from(self.epoch.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.last_millis.fetch_max(now, Ordering::Relaxed);
    }

    pub(super) fn idle(&self) -> Duration {
        self.epoch.elapsed().saturating_sub(Duration::from_millis(
            self.last_millis.load(Ordering::Relaxed),
        ))
    }
}

/// Reader that keeps the child alive for the silence deadline while a caller
/// consumes its stdout directly instead of through [`spawn_read_pipe`].
pub(crate) struct ActivityReader<R> {
    inner: R,
    liveness: LivenessClock,
}

impl<R: Read> Read for ActivityReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buf)?;
        if read > 0 {
            self.liveness.touch();
        }
        Ok(read)
    }
}

pub(super) fn command_cancellation_requested(
    cancellation: Option<&CancellationToken>,
    operation_cancellation: Option<&CancellationToken>,
) -> bool {
    cancellation.is_some_and(CancellationToken::is_cancelled)
        || operation_cancellation.is_some_and(CancellationToken::is_cancelled)
}

pub(super) fn reject_cancelled_command(
    cancellation: Option<&CancellationToken>,
    operation_cancellation: Option<&CancellationToken>,
) -> Result<()> {
    if command_cancellation_requested(cancellation, operation_cancellation) {
        Err(Error::new(ErrorKind::Cancelled))
    } else {
        Ok(())
    }
}

/// Block until `child` exits, the `cancellation` token is tripped, or the
/// child's tree has been silent for `timeout` (no output, trace2 event, or
/// progress line on `liveness`), polling with [`git_command_wait_poll`]
/// backoff. On cancellation or timeout the child is killed and reaped before
/// returning. Callers drain stdout/stderr (typically via reader threads)
/// *after* this returns and map `cancelled`/`timed_out` to their own error.
///
/// Single source of truth for the kill-then-wait / poll loop shared by every
/// long-running git invocation, so the cancellation and timeout semantics can't
/// drift between call sites.
pub(super) fn wait_for_child_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
    liveness: &LivenessClock,
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
                let idle = liveness.idle();
                if idle >= timeout {
                    timed_out = true;
                    match terminate_process_tree_and_wait(child) {
                        Ok(status) => break status,
                        Err(e) => return Err(io_err(e)),
                    }
                }
                if let Some(poll) =
                    git_command_wait_poll(start.elapsed(), timeout.saturating_sub(idle))
                {
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
    })
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct OutputDrainOutcome {
    pub(super) cancelled: bool,
    pub(super) timed_out: bool,
}

/// Keep ownership of the spawned process group until every worker consuming
/// inherited pipes has finished. A Git leader may exit while a hook descendant
/// remains alive with stdout/stderr open; Stop and timeout must still terminate
/// that group instead of blocking forever in `JoinHandle::join`.
pub(super) fn wait_for_output_workers(
    child: &mut std::process::Child,
    workers_finished: impl Fn() -> bool,
    liveness: &LivenessClock,
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
        if liveness.idle() >= timeout {
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

/// Report whether a process group still holds a member that can run.
///
/// `kill(-pgid, 0)` — what `test_kill_process_group` performs — also succeeds
/// for zombies, and a descendant that outlives our leader is reparented to the
/// init process, which in a container frequently never reaps (GitHub Actions
/// runs job containers with `tail -f /dev/null` as init). Such a zombie answers
/// the signal probe forever and would keep every cancellation waiting out the
/// whole `GIT_PROCESS_TERMINATE_GRACE`. A zombie has already released its
/// descriptors and cannot run a hook, so it must not count as a live member.
#[cfg(unix)]
pub(super) fn process_group_has_live_member(pgid: rustix::process::Pid) -> bool {
    if rustix::process::test_kill_process_group(pgid).is_err() {
        return false;
    }

    #[cfg(target_os = "linux")]
    {
        // Without `/proc` to separate zombies from running members, the signal
        // probe is all we have.
        proc_process_group_has_live_member(pgid.as_raw_nonzero().get()).unwrap_or(true)
    }

    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

/// Scan `/proc` for a member of `pgid` that has not exited yet. Returns `None`
/// when the group is not represented in `/proc` at all, so the caller keeps
/// trusting the signal probe rather than declaring a group finished that an
/// unreadable, filtered, or racing `/proc` merely failed to show.
#[cfg(target_os = "linux")]
pub(super) fn proc_process_group_has_live_member(pgid: i32) -> Option<bool> {
    let mut saw_member = false;
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let Some((state, group)) = proc_process_state_and_group(pid) else {
            // The process exited mid-scan, or its stat line is unreadable.
            continue;
        };
        if group != pgid {
            continue;
        }
        if state != 'Z' && state != 'X' {
            return Some(true);
        }
        saw_member = true;
    }
    saw_member.then_some(false)
}

/// Read the process state and process-group id out of `/proc/<pid>/stat`.
#[cfg(target_os = "linux")]
pub(super) fn proc_process_state_and_group(pid: i32) -> Option<(char, i32)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // `pid (comm) state ppid pgrp ...`, where `comm` may itself contain spaces
    // and parentheses, so the fixed-width fields start after its final `)`.
    let (_, fields) = stat.rsplit_once(')')?;
    let mut fields = fields.split_whitespace();
    let state = fields.next()?.chars().next()?;
    let _ppid = fields.next()?;
    let group = fields.next()?.parse().ok()?;
    Some((state, group))
}

pub(super) fn terminate_process_tree_and_wait(
    child: &mut std::process::Child,
) -> io::Result<std::process::ExitStatus> {
    #[cfg(unix)]
    {
        use rustix::process::{Pid, Signal, kill_process_group};

        if let Some(pid) = Pid::from_raw(child.id() as i32) {
            let _ = kill_process_group(pid, Signal::TERM);
            let deadline = Instant::now() + GIT_PROCESS_TERMINATE_GRACE;
            let mut leader_status = None;
            loop {
                if leader_status.is_none() {
                    leader_status = child.try_wait()?;
                }
                // The process-group leader can exit before a TERM-resistant
                // hook descendant. Keep managing the group until nothing in it
                // can run instead of using the leader's status as a proxy.
                if !process_group_has_live_member(pid) {
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

pub(super) fn run_command_with_timeout(
    cmd: Command,
    label: &str,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
) -> Result<Output> {
    run_command_with_timeout_auth(cmd, label, timeout, cancellation, true)
}

/// Read-only network probes must not open auth prompts or consume credentials
/// staged for a user-initiated command.
pub(crate) fn run_git_preview_output(
    cmd: Command,
    label: &str,
    cancellation: &CancellationToken,
) -> Result<Output> {
    run_command_with_timeout_auth(
        cmd,
        label,
        Duration::from_secs(30),
        Some(cancellation),
        false,
    )
}

/// Background reads leave credentials staged for a user-initiated command
/// alone, like [`run_git_preview_output`], but keep the usual silence budget.
pub(crate) fn run_git_background_capture(
    cmd: Command,
    label: &str,
    cancellation: &CancellationToken,
) -> Result<String> {
    background_capture(cmd, label, git_command_timeout(), cancellation)
}

/// A background read that prints nothing until it is done (`git annex
/// unused` scans every ref first): cancellable, but no silence deadline.
pub(crate) fn run_git_background_capture_until_done(
    cmd: Command,
    label: &str,
    cancellation: &CancellationToken,
) -> Result<String> {
    background_capture(cmd, label, Duration::MAX, cancellation)
}

fn background_capture(
    cmd: Command,
    label: &str,
    timeout: Duration,
    cancellation: &CancellationToken,
) -> Result<String> {
    let output = run_command_with_timeout_auth(cmd, label, timeout, Some(cancellation), false)?;
    if !output.status.success() {
        return Err(git_command_failed_error(label, output));
    }
    Ok(bytes_to_text_preserving_utf8(&output.stdout))
}

/// Like the background capture, but lets callers interpret expected nonzero
/// statuses before turning genuine command failures into errors.
pub(crate) fn run_git_background_output(
    cmd: Command,
    label: &str,
    cancellation: &CancellationToken,
) -> Result<Output> {
    run_command_with_timeout_auth(cmd, label, git_command_timeout(), Some(cancellation), false)
}

pub(super) fn run_command_with_timeout_auth(
    cmd: Command,
    label: &str,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
    allow_auth: bool,
) -> Result<Output> {
    run_command_with_timeout_auth_stdin(cmd, label, timeout, cancellation, allow_auth, None)
}

fn run_command_with_timeout_auth_stdin(
    mut cmd: Command,
    label: &str,
    timeout: Duration,
    cancellation: Option<&CancellationToken>,
    allow_auth: bool,
    stdin: Option<Stdio>,
) -> Result<Output> {
    let mut timing = crate::command_trace::CommandTimer::new(label);
    configure_background_command(&mut cmd);
    configure_git_process_tree(&mut cmd);
    configure_non_interactive_git(&mut cmd);
    if let Some(stdin) = stdin {
        cmd.stdin(stdin);
    }
    let operation = git_operation::current();
    reject_cancelled_command(
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    let liveness = LivenessClock::new();
    let trace2 = Trace2Monitor::start(&mut cmd, operation.as_ref(), &liveness);
    let lfs_progress = LfsProgressMonitor::start(&mut cmd, operation.as_ref(), &liveness);
    let askpass_context = if command_may_require_auth(&cmd) {
        let auth = if allow_auth { command_git_auth() } else { None };
        let script = create_askpass_script().map_err(io_err)?;
        configure_git_auth_prompt(&mut cmd, auth.as_ref(), &script);
        Some((script, auth))
    } else {
        None
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    // Meters stream to the operation's progress; the captured stderr reads as
    // it would without them.
    let strip_progress = cmd.get_args().any(|arg| arg == "--progress");

    timing.stage("prepare");
    let mut child = cmd.spawn().map_err(io_err)?;
    timing.stage("spawn");

    let (activity_sender, activity_handle) = start_activity_output_aggregator(operation.as_ref());
    let stdout_handle = spawn_read_pipe(
        child.stdout.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stdout)),
        liveness.clone(),
    );
    let stderr_handle = spawn_read_pipe(
        child.stderr.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stderr)),
        liveness.clone(),
    );
    drop(activity_sender);

    timing.stage("workers-start");
    let ChildWaitOutcome {
        status,
        mut cancelled,
        mut timed_out,
    } = wait_for_child_with_timeout(
        &mut child,
        timeout,
        &liveness,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;

    timing.stage("child-wait");
    let drain = wait_for_output_workers(
        &mut child,
        || stdout_handle.is_finished() && stderr_handle.is_finished(),
        &liveness,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    timing.stage("output-drain");
    cancelled |= drain.cancelled;
    timed_out |= drain.timed_out;

    let stdout = stdout_handle.join().unwrap_or_default();
    let mut stderr = stderr_handle.join().unwrap_or_default();
    if strip_progress && let Ok(text) = std::str::from_utf8(&stderr) {
        stderr = gitcomet_core::git_progress::strip_progress(text).into_bytes();
    }
    timing.stage("workers-join");
    join_activity_output_aggregator(activity_handle);
    timing.stage("activity-finish");
    if let Some(trace2) = trace2 {
        trace2.finish();
    }
    // Drain the last progress line before reporting the result.
    drop(lfs_progress);
    timing.stage("trace-finish");

    if let Some((askpass_script, _)) = askpass_context.as_ref() {
        append_host_prompt_to_stderr(&mut stderr, askpass_script);
        if !status.success() {
            append_passphrase_prompt_to_stderr(&mut stderr, askpass_script);
        }
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

    let mut timing = crate::command_trace::CommandTimer::new(label);
    configure_background_command(&mut cmd);
    configure_git_process_tree(&mut cmd);
    let operation = git_operation::current();
    reject_cancelled_command(
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    let liveness = LivenessClock::new();
    let trace2 = Trace2Monitor::start(&mut cmd, operation.as_ref(), &liveness);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    cmd.stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    timing.stage("prepare");
    let mut child = cmd.spawn().map_err(io_err)?;
    timing.stage("spawn");

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
        liveness.clone(),
    );
    let stderr_handle = spawn_read_pipe(
        child.stderr.take(),
        activity_sender
            .as_ref()
            .map(|sender| (sender.clone(), GitOutputStream::Stderr)),
        liveness.clone(),
    );
    drop(activity_sender);

    timing.stage("workers-start");
    let ChildWaitOutcome {
        status,
        mut cancelled,
        mut timed_out,
    } = wait_for_child_with_timeout(
        &mut child,
        timeout,
        &liveness,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;

    timing.stage("child-wait");
    let drain = wait_for_output_workers(
        &mut child,
        || writer.is_finished() && stdout_handle.is_finished() && stderr_handle.is_finished(),
        &liveness,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    timing.stage("output-drain");
    cancelled |= drain.cancelled;
    timed_out |= drain.timed_out;

    let _ = writer.join();
    let stdout = stdout_handle.join().unwrap_or_default();
    let stderr = stderr_handle.join().unwrap_or_default();
    timing.stage("workers-join");
    join_activity_output_aggregator(activity_handle);
    timing.stage("activity-finish");
    if let Some(trace2) = trace2 {
        trace2.finish();
    }
    timing.stage("trace-finish");

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
    F: FnOnce(ActivityReader<ChildStdout>) -> Result<T> + Send + 'static,
{
    run_git_parsed_stdout_maybe_cancellable(
        cmd,
        label,
        allow_exit_code_one,
        None,
        git_command_timeout(),
        parse_stdout,
    )
}

/// For commands the user started and can cancel, whose tools print nothing
/// while one item runs (git-annex transfers): no silence deadline.
pub(crate) fn run_git_parsed_stdout_until_done<T, F>(
    cmd: Command,
    label: &str,
    parse_stdout: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ActivityReader<ChildStdout>) -> Result<T> + Send + 'static,
{
    run_git_parsed_stdout_maybe_cancellable(cmd, label, false, None, Duration::MAX, parse_stdout)
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
    F: FnOnce(ActivityReader<ChildStdout>) -> Result<T> + Send + 'static,
{
    run_git_parsed_stdout_maybe_cancellable(
        cmd,
        label,
        allow_exit_code_one,
        Some(cancellation),
        git_command_timeout(),
        parse_stdout,
    )
}

pub(super) fn run_git_parsed_stdout_maybe_cancellable<T, F>(
    mut cmd: Command,
    label: &str,
    allow_exit_code_one: bool,
    cancellation: Option<&CancellationToken>,
    timeout: Duration,
    parse_stdout: F,
) -> Result<T>
where
    T: Send + 'static,
    F: FnOnce(ActivityReader<ChildStdout>) -> Result<T> + Send + 'static,
{
    let mut timing = crate::command_trace::CommandTimer::new(label);
    configure_background_command(&mut cmd);
    configure_git_process_tree(&mut cmd);
    configure_non_interactive_git(&mut cmd);
    let operation = git_operation::current();
    reject_cancelled_command(
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    let liveness = LivenessClock::new();
    let trace2 = Trace2Monitor::start(&mut cmd, operation.as_ref(), &liveness);
    let askpass_context = if command_may_require_auth(&cmd) {
        let auth = command_git_auth();
        let script = create_askpass_script().map_err(io_err)?;
        configure_git_auth_prompt(&mut cmd, auth.as_ref(), &script);
        Some((script, auth))
    } else {
        None
    };
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    timing.stage("prepare");
    let mut child = cmd.spawn().map_err(io_err)?;
    timing.stage("spawn");
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
        liveness.clone(),
    );
    drop(activity_sender);
    let stdout = ActivityReader {
        inner: stdout,
        liveness: liveness.clone(),
    };
    let stdout_handle = thread::spawn(move || parse_stdout(stdout));

    timing.stage("workers-start");
    let ChildWaitOutcome {
        status,
        mut cancelled,
        mut timed_out,
    } = wait_for_child_with_timeout(
        &mut child,
        timeout,
        &liveness,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;

    timing.stage("child-wait");
    let drain = wait_for_output_workers(
        &mut child,
        || stdout_handle.is_finished() && stderr_handle.is_finished(),
        &liveness,
        timeout,
        cancellation,
        operation.as_ref().map(GitOperationContext::cancellation),
    )?;
    timing.stage("output-drain");
    cancelled |= drain.cancelled;
    timed_out |= drain.timed_out;

    let parsed_result = stdout_handle
        .join()
        .unwrap_or_else(|_| Err(Error::new(ErrorKind::Io(io::ErrorKind::Other))));
    let mut stderr = stderr_handle.join().unwrap_or_default();
    timing.stage("workers-join");
    join_activity_output_aggregator(activity_handle);
    timing.stage("activity-finish");
    if let Some(trace2) = trace2 {
        trace2.finish();
    }
    timing.stage("trace-finish");

    if let Some((askpass_script, _)) = askpass_context.as_ref() {
        append_host_prompt_to_stderr(&mut stderr, askpass_script);
        if !status.success() {
            append_passphrase_prompt_to_stderr(&mut stderr, askpass_script);
        }
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

pub(super) fn run_git_checked_output(cmd: Command, label: &str) -> Result<Output> {
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

pub(super) fn command_path_budget_len(path: &Path) -> usize {
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
    Ok(command_output(label, output))
}

/// Like [`run_git_parsed_stdout_until_done`]: ends when the command does or
/// the user cancels it, never on silence.
pub(crate) fn run_git_with_output_until_done(cmd: Command, label: &str) -> Result<CommandOutput> {
    run_git_with_output_and_timeout(cmd, label, Duration::MAX)
}

/// [`run_git_with_output`] for work that can rightly take hours, such as a
/// repack of a large repository; the user stops it rather than a timeout.
pub(crate) fn run_git_with_output_and_timeout(
    cmd: Command,
    label: &str,
    timeout: Duration,
) -> Result<CommandOutput> {
    let output = run_command_with_timeout(cmd, label, timeout, None)?;
    if !output.status.success() {
        return Err(git_command_failed_error(label, output));
    }
    Ok(command_output(label, output))
}

fn command_output(label: &str, output: Output) -> CommandOutput {
    let exit_code = output.status.code();
    let stdout = bytes_to_text_preserving_utf8(&output.stdout);
    let stderr = bytes_to_text_preserving_utf8(&output.stderr);
    CommandOutput {
        command: label.to_string(),
        stdout,
        stderr,
        exit_code,
    }
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
    let bytes = run_git_capture_bytes_cancellable(cmd, label, cancellation)?;
    Ok(bytes_to_text_preserving_utf8(&bytes))
}

pub(crate) fn run_git_capture_bytes_cancellable(
    cmd: Command,
    label: &str,
    cancellation: &CancellationToken,
) -> Result<Vec<u8>> {
    let output = run_command_with_timeout(cmd, label, git_command_timeout(), Some(cancellation))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(git_command_failed_error(label, output))
    }
}

pub(crate) fn run_git_capture_bytes(cmd: Command, label: &str) -> Result<Vec<u8>> {
    let output = run_git_checked_output(cmd, label)?;
    Ok(output.stdout)
}
