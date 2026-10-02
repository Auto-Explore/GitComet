//! Hook tracing through Git's trace2 event stream.

use super::*;

pub(super) struct Trace2Monitor {
    pub(super) _path: tempfile::TempPath,
    pub(super) done: Arc<AtomicBool>,
    pub(super) handle: Option<thread::JoinHandle<()>>,
}

impl Trace2Monitor {
    pub(super) fn start(
        cmd: &mut Command,
        context: Option<&GitOperationContext>,
        liveness: &LivenessClock,
    ) -> Option<Self> {
        let context = context?.clone();
        if command_is_known_hook_free(cmd) {
            return None;
        }
        let liveness = liveness.clone();
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
            trace2_tail_loop(&thread_path, &context, &thread_done, &liveness);
        });
        Some(Self {
            _path: path,
            done,
            handle: Some(handle),
        })
    }

    pub(super) fn finish(mut self) {
        self.stop();
    }

    pub(super) fn stop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            // The token also covers completion between the worker's done check
            // and park, so a short command never waits for the next trace poll.
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

/// Trace2 is only used for hook events. On Windows even a tiny traced command
/// walks the process ancestry, so avoid enabling it for these known builtins.
/// Keep unknown programs, global options and subcommands traced: status, for
/// example, can invoke a configured fsmonitor hook.
pub(super) fn command_is_known_hook_free(cmd: &Command) -> bool {
    // Explicit custom executables may be wrappers that run their own hooks,
    // even if their file name happens to be git.exe.
    if cmd.get_program() != "git" {
        return false;
    }
    let Some((subcommand, mut args)) = git_subcommand(cmd, true) else {
        return false;
    };
    match subcommand {
        "remote" => args.next().is_some_and(|arg| arg == "set-url"),
        "config" => args.next().is_some_and(|arg| {
            matches!(
                arg.to_str(),
                Some("--get" | "--get-all" | "--get-regexp" | "--list" | "get" | "list")
            )
        }),
        _ => false,
    }
}

/// Skips Git's global options and returns the subcommand with the arguments
/// after it. `None` for a non-UTF-8 argument or no subcommand. `strict` also
/// rejects global options outside a known side-effect-free set.
pub(super) fn git_subcommand(
    cmd: &Command,
    strict: bool,
) -> Option<(&str, std::process::CommandArgs<'_>)> {
    let mut args = cmd.get_args();
    while let Some(arg) = args.next() {
        match arg.to_str()? {
            "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace" => {
                args.next()?;
            }
            "--no-optional-locks" | "--no-pager" | "--literal-pathspecs" => {}
            value if value.starts_with('-') => {
                if strict {
                    return None;
                }
            }
            subcommand => return Some((subcommand, args)),
        }
    }
    None
}

impl Drop for Trace2Monitor {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Tails `GIT_LFS_PROGRESS` for commands that can move LFS content. git-lfs
/// prints no progress when stderr is not a terminal, so this file is the only
/// sign of life during a long transfer: every line keeps the silence deadline
/// open, and parsed lines become activity progress.
pub(super) struct LfsProgressMonitor {
    pub(super) _path: tempfile::TempPath,
    done: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

/// Subcommands that can upload, download or check out LFS content.
pub(super) fn may_transfer_lfs_content(cmd: &Command) -> bool {
    matches!(
        git_subcommand(cmd, false).map(|(subcommand, _)| subcommand),
        Some(
            "lfs"
                | "push"
                | "pull"
                | "fetch"
                | "clone"
                | "checkout"
                | "switch"
                | "restore"
                | "reset"
                | "merge"
                | "rebase"
                | "stash"
                | "cherry-pick"
                | "revert"
                | "worktree"
                | "submodule"
        )
    )
}

impl LfsProgressMonitor {
    pub(super) fn start(
        cmd: &mut Command,
        context: Option<&GitOperationContext>,
        liveness: &LivenessClock,
    ) -> Option<Self> {
        // Not gated on the repository using LFS: the command itself (checkout,
        // pull, merge...) can be what brings LFS attributes in.
        if !may_transfer_lfs_content(cmd) {
            return None;
        }
        let file = tempfile::Builder::new()
            .prefix("gitcomet-lfs-progress-")
            .tempfile()
            .ok()?;
        let path = file.into_temp_path();
        cmd.env("GIT_LFS_PROGRESS", path.as_os_str());
        let done = Arc::new(AtomicBool::new(false));
        let thread_done = Arc::clone(&done);
        let thread_path = path.to_path_buf();
        let context = context.cloned();
        let liveness = liveness.clone();
        let handle = thread::spawn(move || {
            lfs_progress_tail_loop(&thread_path, context.as_ref(), &thread_done, &liveness);
        });
        Some(Self {
            _path: path,
            done,
            handle: Some(handle),
        })
    }

    pub(super) fn stop(&mut self) {
        self.done.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
}

impl Drop for LfsProgressMonitor {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) fn lfs_progress_tail_loop(
    path: &Path,
    context: Option<&GitOperationContext>,
    done: &AtomicBool,
    liveness: &LivenessClock,
) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    let mut pending = Vec::<u8>::new();
    loop {
        // Observe completion before reading: a stop racing the read/emit must
        // leave another pass to drain the writer's final line.
        let final_read = done.load(Ordering::Acquire);
        let before = pending.len();
        let _ = file.read_to_end(&mut pending);
        let grew = pending.len() != before;
        if grew {
            liveness.touch();
        }
        // Only the newest complete line matters; older ones are superseded.
        if let Some(end) = pending.iter().rposition(|byte| *byte == b'\n') {
            let complete = pending.drain(..=end).collect::<Vec<_>>();
            let latest = String::from_utf8_lossy(&complete);
            if let (Some(context), Some(progress)) = (
                context,
                latest
                    .lines()
                    .rev()
                    .find_map(gitcomet_core::lfs::parse_progress_line),
            ) {
                context.emit(GitOperationEvent::TransferProgress(progress));
            }
        }
        if final_read {
            break;
        }
        if !grew {
            thread::park_timeout(GIT_TRACE2_POLL);
        }
    }
}

pub(super) struct TracedHook {
    pub(super) id: HookExecutionId,
    pub(super) name: String,
    pub(super) started: Instant,
}

pub(super) fn trace2_tail_loop(
    path: &Path,
    context: &GitOperationContext,
    done: &AtomicBool,
    liveness: &LivenessClock,
) {
    let Ok(mut file) = std::fs::File::open(path) else {
        return;
    };
    let mut pending = Vec::<u8>::new();
    let mut hooks = rustc_hash::FxHashMap::<(String, u64), TracedHook>::default();
    loop {
        let before = pending.len();
        let _ = file.read_to_end(&mut pending);
        if pending.len() != before {
            // A hook or child event proves the tree is alive while its pipes are quiet.
            liveness.touch();
        }
        parse_trace2_lines(&mut pending, false, context, &mut hooks);
        if done.load(Ordering::Acquire) {
            let _ = file.read_to_end(&mut pending);
            parse_trace2_lines(&mut pending, true, context, &mut hooks);
            break;
        }
        if pending.len() == before {
            thread::park_timeout(GIT_TRACE2_POLL);
        }
    }
}

pub(super) fn parse_trace2_lines(
    pending: &mut Vec<u8>,
    eof: bool,
    context: &GitOperationContext,
    hooks: &mut rustc_hash::FxHashMap<(String, u64), TracedHook>,
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

pub(super) fn trace2_u64(value: &serde_json::Value, key: &str) -> Option<u64> {
    value
        .get(key)
        .and_then(|value| value.as_u64().or_else(|| value.as_str()?.parse().ok()))
}

pub(super) fn apply_trace2_event(
    value: &serde_json::Value,
    context: &GitOperationContext,
    hooks: &mut rustc_hash::FxHashMap<(String, u64), TracedHook>,
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
