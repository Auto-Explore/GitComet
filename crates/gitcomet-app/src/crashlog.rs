mod report;

use gitcomet_core::identity;
use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use gitcomet_core::environment::{self, EnvironmentSnapshot};
use gitcomet_core::fs_utils::{open_private_append, write_private_file};

static WRITING_CRASH_LOG: AtomicBool = AtomicBool::new(false);
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
static SESSION_LIFECYCLE: Mutex<()> = Mutex::new(());
static WRITING_RUNTIME_ERROR_LOG: Mutex<()> = Mutex::new(());
/// `(location, message)` hashes already written this session. A driver or
/// GPUI diagnostic that repeats every frame only needs one durable record
/// per distinct error in the recovered report.
static RUNTIME_ERRORS_SEEN: Mutex<Vec<u64>> = Mutex::new(Vec::new());
const MAX_RUNTIME_ERRORS_SEEN: usize = 1_024;
static CRASH_LOGGER: CrashLogger = CrashLogger;
const CRASH_ISSUE_TEMPLATE: &str = "crash_report.md";
const PENDING_REPORT_FILE: &str = "pending-report-path.txt";
const STARTUP_REPORT_FILE: &str = "pending-startup-report.log";
const SESSION_MARKER_FILE_PREFIX: &str = "session-in-progress";
const LAST_OPERATION_FILE_PREFIX: &str = "last-operation";
const RUNTIME_ERROR_FILE_PREFIX: &str = "last-runtime-error";
const MAX_TITLE_CHARS: usize = 96;
const MAX_BACKTRACE_CHARS: usize = 2_400;
#[cfg(windows)]
const PENDING_REPORT_PATH_WIDE_PREFIX: &str = "gitcomet-crashlog-utf16le:";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupCrashReport {
    pub issue_url: String,
    pub summary: String,
    pub crash_log_path: PathBuf,
}

struct AbnormalExitLog {
    marker_path: PathBuf,
    last_operation_path: PathBuf,
    runtime_error_path: PathBuf,
    environment_path: PathBuf,
    contents: String,
}

pub fn install() {
    let product = identity::current().display_name();
    if log::set_logger(&CRASH_LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Error);
    }

    #[cfg(unix)]
    if let Err(err) = install_terminal_interrupt_handler() {
        eprintln!("Failed to install {product} terminal interrupt handler: {err}");
    }

    #[cfg(windows)]
    if !install_terminal_interrupt_handler() {
        eprintln!("Failed to install {product} terminal interrupt handler");
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let gitcomet_core::panic::Containment::Report { escaped } =
            gitcomet_core::panic::note_contained(info)
        {
            write_panic_log(info, escaped.as_ref());
        }
        previous(info);
    }));
}

struct CrashLogger;

impl log::Log for CrashLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= log::Level::Error
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let message = record.args().to_string();
        if !should_record_runtime_error(record.target(), &message) {
            return;
        }

        // Keep actionable GPUI terminal diagnostics visible while also making
        // them durable enough to survive an event-loop or native process exit.
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{message}");
        write_runtime_error_log(record, &message);
    }

    fn flush(&self) {}
}

fn should_record_runtime_error(target: &str, message: &str) -> bool {
    // GPUI can deliver a queued native-window notification after the window was
    // removed from App. Its callback logs the failed handle lookup even though
    // teardown is proceeding normally. Treating that diagnostic as evidence of
    // a crash causes the next launch to report a false abnormal exit.
    if target == "gpui::window" && message.trim() == "window not found" {
        return false;
    }

    // Some EGL drivers emit this validation message through wgpu's debug
    // callback while the application continues normally. It is not evidence
    // that GitComet itself failed and can otherwise obscure a later exit's
    // actual cause in the recovered report.
    if target == "wgpu_hal::gles::egl"
        && message.contains("eglCreateSyncKHR")
        && message.contains("EGL_BAD_ATTRIBUTE")
        && message.contains("EGL_SYNC_NATIVE_FENCE_FD_ANDROID")
        && message.contains("EGL_SYNC_STATUS")
    {
        return false;
    }

    true
}

#[cfg(unix)]
fn install_terminal_interrupt_handler() -> std::io::Result<()> {
    use signal_hook::consts::signal::SIGINT;
    use signal_hook::iterator::Signals;

    let mut signals = Signals::new([SIGINT])?;
    std::thread::Builder::new()
        .name("gitcomet-sigint".to_string())
        .spawn(move || {
            if signals.forever().next().is_some() {
                // SIGINT is the user's explicit request to stop a terminal
                // launch, then preserve the conventional shell exit status.
                retire_active_session();
                // `std::process::exit` runs native exit handlers while the UI
                // and renderer threads are still active. Graphics-driver exit
                // handlers can then tear down state those threads are using.
                // The signal-hook primitive is a raw `_exit`, so it preserves
                // status 130 without running that concurrent native teardown.
                signal_hook::low_level::exit(128 + SIGINT);
            }
        })?;
    Ok(())
}

/// Windows has no SIGINT: the console delivers Ctrl+C, Ctrl+Break and console
/// close as control events. Retiring the recovery marker there keeps a
/// deliberate terminal interrupt from being reported as a crash on the next
/// launch, and ending the process from the handler keeps the interrupt from
/// hanging the UI. See [`handle_console_ctrl_event`].
#[cfg(windows)]
fn install_terminal_interrupt_handler() -> bool {
    gitcomet_win32_window_utils::install_console_ctrl_handler(handle_console_ctrl_event)
}

/// How long the recovery-marker cleanup gets before a console interrupt ends
/// the process regardless of whether it finished.
#[cfg(windows)]
const CONSOLE_CTRL_CLEANUP_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

#[cfg(windows)]
fn handle_console_ctrl_event(event: u32) -> bool {
    if !console_ctrl_event_ends_process(event) {
        // Not an event that ends the process, so leave it to the console.
        return false;
    }

    // Retiring the marker takes a lock and writes to the crash directory, and
    // either can stall — another thread mid-session-lifecycle, or a LOCALAPPDATA
    // on an unreachable network share. Arm the exit before running it so a
    // stalled cleanup cannot restore the very hang this handler prevents.
    let _ = std::thread::Builder::new()
        .name("gitcomet-ctrl-c-watchdog".to_string())
        .spawn(|| {
            std::thread::sleep(CONSOLE_CTRL_CLEANUP_GRACE);
            gitcomet_win32_window_utils::terminate_current_process(
                gitcomet_win32_window_utils::CONTROL_C_EXIT_CODE,
            );
        });

    retire_active_session();
    // Declining the event here would hand termination to the console's default
    // handler, which calls `ExitProcess`. That deadlocks in a running GitComet
    // window — it takes the loader lock before stopping the renderer, executor
    // and allocator threads, then runs `DLL_PROCESS_DETACH` for every loaded
    // module — so Ctrl+C would leave the process alive forever. End the process
    // here instead, reporting the status the invoking shell expects for an
    // interrupted program.
    gitcomet_win32_window_utils::terminate_current_process(
        gitcomet_win32_window_utils::CONTROL_C_EXIT_CODE,
    );
}

#[cfg(windows)]
fn console_ctrl_event_ends_process(event: u32) -> bool {
    use gitcomet_win32_window_utils::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, CTRL_CLOSE_EVENT, CTRL_LOGOFF_EVENT, CTRL_SHUTDOWN_EVENT,
    };

    matches!(
        event,
        CTRL_C_EVENT
            | CTRL_BREAK_EVENT
            | CTRL_CLOSE_EVENT
            | CTRL_LOGOFF_EVENT
            | CTRL_SHUTDOWN_EVENT
    )
}

/// Retires only this process's active recovery marker, so a deliberate
/// shutdown outside the UI event loop is not recovered as an abnormal exit.
#[cfg(any(unix, windows))]
fn retire_active_session() {
    retire_active_session_in_dir(crash_dir().as_deref());
}

#[cfg(any(unix, windows))]
fn retire_active_session_in_dir(dir: Option<&Path>) {
    let _lifecycle = SESSION_LIFECYCLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if SESSION_ACTIVE.swap(false, Ordering::SeqCst)
        && let Some(dir) = dir
    {
        let _ = finish_session_in_dir(dir);
    }
}

/// Whether this `(location, message)` pair is new this session; records it.
fn first_runtime_error_occurrence(location: &str, message: &str) -> bool {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    location.hash(&mut hasher);
    message.hash(&mut hasher);
    let key = hasher.finish();
    let mut seen = RUNTIME_ERRORS_SEEN
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if seen.contains(&key) {
        return false;
    }
    if seen.len() >= MAX_RUNTIME_ERRORS_SEEN {
        seen.clear();
    }
    seen.push(key);
    true
}

fn write_runtime_error_log(record: &log::Record<'_>, message: &str) {
    if !SESSION_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let Ok(_guard) = WRITING_RUNTIME_ERROR_LOG.lock() else {
        return;
    };
    let location = record
        .file()
        .map(|file| match record.line() {
            Some(line) => format!("{file}#L{line}"),
            None => file.to_string(),
        })
        .unwrap_or_else(|| "<unknown log call site>".to_string());
    let message = single_line_text(message);
    if !first_runtime_error_occurrence(&location, &message) {
        return;
    }
    let Some(dir) = crash_dir() else {
        return;
    };
    let info = if record.target().is_empty() {
        "An error was emitted through the application logging facade.".to_string()
    } else {
        format!("An error was emitted by log target {}.", record.target())
    };
    // Logging a recoverable driver diagnostic can happen on the UI thread.
    // Symbolizing its stack here stalls startup, especially with Windows PDBs.
    // The log record already identifies the caller; reserve backtraces for
    // panics and explicit fatal session failures.
    let _ = write_runtime_error_log_in_dir(&dir, &location, &message, &info);
}

fn write_runtime_error_log_in_dir(
    dir: &Path,
    location: &str,
    message: &str,
    info: &str,
) -> std::io::Result<()> {
    let path = runtime_error_path(dir);
    let mut file = open_private_append(&path)?;
    let has_existing_log = file.metadata()?.len() > 0;
    if has_existing_log {
        writeln!(file)?;
    }
    writeln!(file, "=== GitComet runtime error ===")?;
    writeln!(file, "failure_kind=runtime-error")?;
    writeln!(file, "timestamp_unix_ms={}", unix_time_ms())?;
    writeln!(
        file,
        "crate={} version={}",
        identity::current().executable_name(),
        identity::current().version()
    )?;
    writeln!(
        file,
        "thread={}",
        std::thread::current().name().unwrap_or("<unnamed>")
    )?;
    writeln!(file, "location={}", single_line_text(location))?;
    writeln!(file, "message={}", single_line_text(message))?;
    writeln!(file, "info={}", single_line_text(info))?;
    write_cached_environment(&mut file)?;
    // No `sync_data`: the record only has to outlive this process, which the
    // page cache guarantees, and an fsync per `error!` stalled the frame that
    // logged it.
    file.flush()
}

/// Records entry to the UI event loop. A stale marker lets the next launch
/// report native aborts and unexpected signals. Terminal SIGINT is handled on
/// a dedicated thread and removes the marker before exiting.
pub fn begin_session() -> std::io::Result<()> {
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    let _lifecycle = SESSION_LIFECYCLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Err(err) = begin_session_in_dir(&dir) {
        SESSION_ACTIVE.store(false, Ordering::SeqCst);
        let _ = finish_session_in_dir(&dir);
        return Err(err);
    }
    SESSION_ACTIVE.store(true, Ordering::SeqCst);
    start_environment_writer(dir)?;
    Ok(())
}

fn environment_path_for_pid(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("environment-{pid}.json"))
}

fn persist_environment(dir: &Path, snapshot: &EnvironmentSnapshot) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(snapshot)?;
    write_private_file(&environment_path_for_pid(dir, std::process::id()), &bytes)
}

/// Serial, background persistence prevents stale writes from overtaking newer
/// snapshots. The lifecycle lock also prevents an in-flight write from
/// recreating artifacts after clean shutdown has removed them.
fn start_environment_writer(dir: PathBuf) -> std::io::Result<()> {
    let updates = environment::subscribe();
    std::thread::Builder::new()
        .name("gitcomet-environment".into())
        .spawn(move || {
            while let Ok(mut snapshot) = updates.recv() {
                for next in updates.try_iter() {
                    snapshot = next;
                }
                let _lifecycle = SESSION_LIFECYCLE.lock().unwrap_or_else(|e| e.into_inner());
                if !SESSION_ACTIVE.load(Ordering::SeqCst) {
                    break;
                }
                if let Err(err) = persist_environment(&dir, &snapshot) {
                    eprintln!(
                        "Failed to persist {} environment: {err}",
                        identity::current().display_name()
                    );
                }
            }
        })?;
    Ok(())
}

fn write_environment(
    writer: &mut impl std::io::Write,
    snapshot: &EnvironmentSnapshot,
) -> std::io::Result<()> {
    writeln!(
        writer,
        "environment_json={}",
        serde_json::to_string(snapshot)?
    )
}

fn write_cached_environment(writer: &mut impl std::io::Write) -> std::io::Result<()> {
    if let Some(snapshot) = environment::try_cached() {
        write_environment(writer, &snapshot)?;
    }
    Ok(())
}

fn begin_session_in_dir(dir: &Path) -> std::io::Result<()> {
    let product = identity::current().display_name();
    remove_file_if_exists(&last_operation_path(dir))?;
    remove_file_if_exists(&runtime_error_path(dir))?;
    // A reused PID must not inherit an earlier process's renderer.
    persist_environment(dir, &environment::cached())?;

    let mut contents = Vec::new();
    writeln!(contents, "=== GitComet abnormal exit candidate ===")?;
    writeln!(contents, "failure_kind=abnormal-exit")?;
    writeln!(contents, "failure_context=")?;
    writeln!(contents, "copy_source=")?;
    writeln!(contents, "timestamp_unix_ms={}", unix_time_ms())?;
    writeln!(
        contents,
        "crate={} version={}",
        identity::current().executable_name(),
        identity::current().version()
    )?;
    writeln!(
        contents,
        "thread={}",
        std::thread::current().name().unwrap_or("<unnamed>")
    )?;
    writeln!(contents, "message={product} did not exit cleanly")?;
    writeln!(
        contents,
        "info=The previous UI process ended before it completed its shutdown sequence."
    )?;
    writeln!(contents, "os={}", std::env::consts::OS)?;
    writeln!(contents, "arch={}", std::env::consts::ARCH)?;
    writeln!(contents, "display={}", env_value("DISPLAY"))?;
    writeln!(contents, "wayland_display={}", env_value("WAYLAND_DISPLAY"))?;
    write_private_file(&session_marker_path(dir), &contents)
}

/// Clears the current session marker after the UI event loop returns normally.
pub fn finish_session() -> std::io::Result<()> {
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    let _lifecycle = SESSION_LIFECYCLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let result = finish_session_in_dir(&dir);
    SESSION_ACTIVE.store(false, Ordering::SeqCst);
    result
}

fn finish_session_in_dir(dir: &Path) -> std::io::Result<()> {
    remove_file_if_exists(&session_marker_path(dir))?;
    remove_file_if_exists(&last_operation_path(dir))?;
    remove_file_if_exists(&runtime_error_path(dir))?;
    remove_file_if_exists(&environment_path_for_pid(dir, std::process::id()))
}

/// Records a handled UI launch or event-loop failure for the next startup
/// report. Unlike a panic hook, this covers errors returned by GPUI after the
/// session marker was created.
#[track_caller]
pub fn record_session_failure(context: &str, message: &str) -> std::io::Result<()> {
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    let caller = std::panic::Location::caller();
    let location = format!("{}#L{}", caller.file(), caller.line());
    let backtrace = Backtrace::force_capture().to_string();
    record_session_failure_in_dir_with_diagnostics(
        &dir,
        context,
        message,
        Some(&location),
        Some(&backtrace),
    )
}

#[cfg(test)]
fn record_session_failure_in_dir(dir: &Path, context: &str, message: &str) -> std::io::Result<()> {
    record_session_failure_in_dir_with_diagnostics(dir, context, message, None, None)
}

fn record_session_failure_in_dir_with_diagnostics(
    dir: &Path,
    context: &str,
    message: &str,
    location: Option<&str>,
    backtrace: Option<&str>,
) -> std::io::Result<()> {
    let product = identity::current().display_name();
    let mut file = open_private_append(&session_marker_path(dir))?;
    writeln!(file, "failure_kind=returned-error")?;
    writeln!(file, "failure_context={}", single_line_text(context))?;
    writeln!(file, "timestamp_unix_ms={}", unix_time_ms())?;
    if let Some(location) = location {
        writeln!(file, "location={}", single_line_text(location))?;
    }
    writeln!(file, "message={}", single_line_text(message))?;
    writeln!(
        file,
        "info={product} could not complete its GPUI launch or event loop."
    )?;
    write_cached_environment(&mut file)?;
    if let Some(backtrace) = backtrace {
        writeln!(file, "backtrace:")?;
        writeln!(file, "{backtrace}")?;
    }
    file.flush()?;
    file.sync_data()
}

pub fn take_startup_report() -> Option<StartupCrashReport> {
    let dir = crash_dir()?;
    take_startup_report_from_crash_dir_with_process_check(
        &dir,
        crate::process_liveness::process_is_running,
    )
}

#[cfg(test)]
fn take_startup_report_from_crash_dir(dir: &Path) -> Option<StartupCrashReport> {
    take_startup_report_from_crash_dir_with_process_check(dir, |_| false)
}

fn take_startup_report_from_crash_dir_with_process_check(
    dir: &Path,
    process_is_running: impl FnMut(u32) -> bool,
) -> Option<StartupCrashReport> {
    let product = identity::current().display_name();
    let report_path = startup_report_path(dir);
    let mut report_log = match std::fs::read_to_string(&report_path) {
        Ok(report_log) => report_log,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            eprintln!(
                "Failed to read pending {product} crash report {}: {err}",
                report_path.display()
            );
            return None;
        }
    };
    let session_logs = stale_abnormal_exit_logs(dir, process_is_running);
    let pending_path = pending_report_path(dir);
    let (panic_log, pending_panic_read_failed) = match read_pending_panic_log(&pending_path) {
        Ok(panic_log) => (panic_log, false),
        Err(err) => {
            eprintln!(
                "Failed to read {product} pending panic report {}: {err}",
                pending_path.display()
            );
            (None, true)
        }
    };
    if !pending_panic_read_failed && panic_log.is_none() && pending_path.exists() {
        let _ = std::fs::remove_file(&pending_path);
    }

    if session_logs.is_empty() && panic_log.is_none() {
        return (!report_log.trim().is_empty())
            .then(|| build_startup_report(report_path, &report_log));
    }

    for session_log in &session_logs {
        append_report_log(&mut report_log, &session_log.contents);
    }
    // Preserve the existing panic-over-session priority when both reports are
    // available while retaining the session's last-operation diagnostics.
    if let Some(panic_log) = &panic_log {
        append_report_log(&mut report_log, &panic_log.contents);
    }

    let report_path = match write_startup_report_snapshot(dir, &report_log) {
        Ok(report_path) => report_path,
        Err(err) => {
            eprintln!("Failed to persist {product} startup crash report: {err}");
            return None;
        }
    };
    for session_log in &session_logs {
        if let Err(err) = remove_file_if_exists(&session_log.marker_path) {
            eprintln!(
                "Failed to clear recovered {product} session marker {}: {err}",
                session_log.marker_path.display()
            );
        } else {
            if let Err(err) = remove_file_if_exists(&session_log.last_operation_path) {
                eprintln!(
                    "Failed to clear recovered {product} last-operation diagnostics {}: {err}",
                    session_log.last_operation_path.display()
                );
            }
            if let Err(err) = remove_file_if_exists(&session_log.runtime_error_path) {
                eprintln!(
                    "Failed to clear recovered {product} runtime-error diagnostics {}: {err}",
                    session_log.runtime_error_path.display()
                );
            }
            if let Err(err) = remove_file_if_exists(&session_log.environment_path) {
                eprintln!(
                    "Failed to clear recovered {product} environment {}: {err}",
                    session_log.environment_path.display()
                );
            }
        }
    }
    if let Some(panic_log) = &panic_log {
        let _ = std::fs::remove_file(&pending_path);
        if let Err(err) = remove_file_if_exists(&panic_log.path) {
            eprintln!(
                "Failed to clear snapshotted {product} panic log {}: {err}",
                panic_log.path.display()
            );
        }
    }
    Some(build_startup_report(report_path, &report_log))
}

fn stale_abnormal_exit_logs(
    dir: &Path,
    mut process_is_running: impl FnMut(u32) -> bool,
) -> Vec<AbnormalExitLog> {
    let product = identity::current().display_name();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            eprintln!(
                "Failed to enumerate {product} session markers in {}: {err}",
                dir.display()
            );
            return Vec::new();
        }
    };

    let mut markers = entries
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            session_marker_pid(&path).map(|pid| (pid, path))
        })
        .collect::<Vec<_>>();
    markers.sort_unstable_by_key(|(pid, _)| *pid);

    markers
        .into_iter()
        .filter(|(pid, _)| !process_is_running(*pid))
        .filter_map(|(pid, marker_path)| {
            let last_operation_path = last_operation_path_for_pid(dir, pid);
            let runtime_error_path = runtime_error_path_for_pid(dir, pid);
            let environment_path = environment_path_for_pid(dir, pid);
            read_abnormal_exit_log(
                &marker_path,
                &last_operation_path,
                &runtime_error_path,
                &environment_path,
            )
            .map(|contents| AbnormalExitLog {
                marker_path,
                last_operation_path,
                runtime_error_path,
                environment_path,
                contents,
            })
        })
        .collect()
}

fn read_abnormal_exit_log(
    marker: &Path,
    last_operation_path: &Path,
    runtime_error_path: &Path,
    environment_path: &Path,
) -> Option<String> {
    let product = identity::current().display_name();
    let mut session_log = match std::fs::read_to_string(marker) {
        Ok(session_log) => session_log,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            eprintln!(
                "Failed to read {product} session marker {}: {err}",
                marker.display()
            );
            return None;
        }
    };
    if session_log.trim().is_empty() {
        return None;
    }
    match std::fs::read_to_string(last_operation_path) {
        Ok(last_operation) => {
            let _ = writeln!(session_log, "\n=== GitComet operation context ===");
            let _ = write!(session_log, "{last_operation}");
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            eprintln!(
                "Failed to read {product} last-operation diagnostics {}: {err}",
                last_operation_path.display()
            );
        }
    }
    match std::fs::read_to_string(runtime_error_path) {
        Ok(runtime_error) => append_report_log(&mut session_log, &runtime_error),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            eprintln!(
                "Failed to read {product} runtime-error diagnostics {}: {err}",
                runtime_error_path.display()
            );
        }
    }
    // Read the failed PID's file, never the current process cache. An inline
    // snapshot belongs to the actual error and can be newer than the background
    // writer's last flush; don't replace it with a stale sidecar. For exits
    // without an inline snapshot, attach the sidecar after any backtrace.
    if parse_crash_log(&session_log).environment.is_none()
        && let Ok(bytes) = std::fs::read(environment_path)
        && let Ok(snapshot) = serde_json::from_slice::<EnvironmentSnapshot>(&bytes)
    {
        let _ = writeln!(session_log, "\n=== GitComet environment ===");
        if let Ok(json) = serde_json::to_string(&snapshot) {
            let _ = writeln!(session_log, "environment_json={json}");
        }
    }
    Some(session_log)
}

struct PendingPanicLog {
    path: PathBuf,
    contents: String,
}

fn read_pending_panic_log(pending_path: &Path) -> std::io::Result<Option<PendingPanicLog>> {
    let Some(crash_log_path) = read_pending_report_path(pending_path)? else {
        return Ok(None);
    };
    match std::fs::read_to_string(&crash_log_path) {
        Ok(contents) => Ok(Some(PendingPanicLog {
            path: crash_log_path,
            contents,
        })),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

fn append_report_log(destination: &mut String, report_log: &str) {
    if !destination.is_empty() && !destination.ends_with('\n') {
        destination.push('\n');
    }
    if !destination.trim().is_empty() {
        destination.push('\n');
    }
    destination.push_str(report_log);
    if !destination.ends_with('\n') {
        destination.push('\n');
    }
}

fn write_startup_report_snapshot(dir: &Path, report_log: &str) -> std::io::Result<PathBuf> {
    let report_path = startup_report_path(dir);
    write_private_file(&report_path, report_log.as_bytes())?;
    Ok(report_path)
}

/// `escaped` is this thread's earlier contained panic that never reached
/// `contain`, logged as the failure. When it could not unwind it caused the
/// abort in `info`; when the contained code caught it, `info` is the panic
/// that followed.
fn write_panic_log(
    info: &std::panic::PanicHookInfo<'_>,
    escaped: Option<&gitcomet_core::panic::ContainedPanic>,
) {
    if WRITING_CRASH_LOG
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return;
    }

    let _guard = ResetFlagOnDrop;

    let Some(dir) = crash_dir() else {
        return;
    };
    let Some(path) = crash_log_path(&dir) else {
        return;
    };

    let mut file = match open_private_append(&path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let _ = writeln!(file, "=== GitComet crash (panic) ===");
    let _ = writeln!(file, "failure_kind=panic");
    let _ = writeln!(file, "timestamp_unix_ms={}", unix_time_ms());
    let _ = writeln!(
        file,
        "crate={} version={}",
        identity::current().executable_name(),
        identity::current().version()
    );
    let _ = writeln!(
        file,
        "thread={}",
        std::thread::current().name().unwrap_or("<unnamed>")
    );

    match escaped {
        Some(cause) => {
            if let Some(location) = &cause.location {
                let _ = writeln!(file, "location={}#L{}", location.file, location.line);
            }
            let _ = writeln!(file, "message={}", cause.message);
        }
        None => {
            if let Some(location) = info.location() {
                let _ = writeln!(file, "location={}#L{}", location.file(), location.line());
            }
            let payload = gitcomet_core::panic::payload_message(info.payload());
            let _ = writeln!(file, "message={payload}");
        }
    }
    let _ = writeln!(file, "info={info}");
    let _ = write_cached_environment(&mut file);

    let bt = Backtrace::force_capture();
    let _ = writeln!(file, "backtrace:\n{bt}");
    let _ = writeln!(file);
    if file.flush().is_err() || file.sync_data().is_err() {
        return;
    }
    let _ = write_pending_report_path(&pending_report_path(&dir), &path);
}

fn crash_dir() -> Option<PathBuf> {
    gitcomet_core::platform::dirs::crash_dir()
}

fn crash_directory_unavailable_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "{} crash state directory is unavailable: no per-user state directory is configured",
            identity::current().display_name()
        ),
    )
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn crash_log_path(dir: &Path) -> Option<PathBuf> {
    let pid = std::process::id();
    Some(dir.join(format!("panic-{pid}-{}.log", unix_time_ms())))
}

fn pending_report_path(dir: &Path) -> PathBuf {
    dir.join(PENDING_REPORT_FILE)
}

fn startup_report_path(dir: &Path) -> PathBuf {
    dir.join(STARTUP_REPORT_FILE)
}

fn session_marker_path(dir: &Path) -> PathBuf {
    session_marker_path_for_pid(dir, std::process::id())
}

fn last_operation_path(dir: &Path) -> PathBuf {
    last_operation_path_for_pid(dir, std::process::id())
}

fn runtime_error_path(dir: &Path) -> PathBuf {
    runtime_error_path_for_pid(dir, std::process::id())
}

fn session_marker_path_for_pid(dir: &Path, pid: u32) -> PathBuf {
    process_diagnostic_path(dir, SESSION_MARKER_FILE_PREFIX, pid)
}

fn last_operation_path_for_pid(dir: &Path, pid: u32) -> PathBuf {
    process_diagnostic_path(dir, LAST_OPERATION_FILE_PREFIX, pid)
}

fn runtime_error_path_for_pid(dir: &Path, pid: u32) -> PathBuf {
    process_diagnostic_path(dir, RUNTIME_ERROR_FILE_PREFIX, pid)
}

fn process_diagnostic_path(dir: &Path, prefix: &str, pid: u32) -> PathBuf {
    dir.join(format!("{prefix}-{pid}.log"))
}

fn session_marker_pid(path: &Path) -> Option<u32> {
    let filename = path.file_name()?.to_str()?;
    filename
        .strip_prefix(SESSION_MARKER_FILE_PREFIX)?
        .strip_prefix('-')?
        .strip_suffix(".log")?
        .parse()
        .ok()
}

fn env_value(name: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "<unset>".to_string())
}

fn write_pending_report_path(marker: &Path, crash_log_path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        write_private_file(marker, crash_log_path.as_os_str().as_bytes())
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let mut raw = Vec::new();
        for unit in crash_log_path.as_os_str().encode_wide() {
            raw.extend_from_slice(&unit.to_le_bytes());
        }
        let mut out = String::with_capacity(PENDING_REPORT_PATH_WIDE_PREFIX.len() + raw.len() * 2);
        out.push_str(PENDING_REPORT_PATH_WIDE_PREFIX);
        out.push_str(&hex_encode(&raw));
        write_private_file(marker, out.as_bytes())
    }

    #[cfg(not(any(unix, windows)))]
    {
        let Some(path_text) = crash_log_path.to_str() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "crash log path is not valid Unicode on this platform",
            ));
        };
        write_private_file(marker, path_text.as_bytes())
    }
}

fn read_pending_report_path(marker: &Path) -> std::io::Result<Option<PathBuf>> {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let bytes = match std::fs::read(marker) {
            Ok(bytes) => bytes,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        if bytes.is_empty() {
            return Ok(None);
        }
        Ok(Some(PathBuf::from(OsString::from_vec(bytes))))
    }

    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        let raw = match std::fs::read_to_string(marker) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let value = raw.trim();
        if let Some(hex) = value.strip_prefix(PENDING_REPORT_PATH_WIDE_PREFIX)
            && let Some(bytes) = hex_decode(hex)
            && bytes.len() % 2 == 0
        {
            let mut wide = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.as_chunks::<2>().0 {
                wide.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            return Ok(Some(PathBuf::from(OsString::from_wide(&wide))));
        }
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(value)))
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        let raw = match std::fs::read_to_string(marker) {
            Ok(raw) => raw,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(err) => return Err(err),
        };
        let value = raw.trim();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(PathBuf::from(value)))
        }
    }
}

#[cfg(windows)]
use crate::hex_encode;
#[cfg(windows)]
use gitcomet_core::hex::decode as hex_decode;
use report::{build_startup_report, parse_crash_log, single_line_text};

fn unix_time_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

struct ResetFlagOnDrop;

impl Drop for ResetFlagOnDrop {
    fn drop(&mut self) {
        WRITING_CRASH_LOG.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod environment_tests;
#[cfg(test)]
mod tests;
