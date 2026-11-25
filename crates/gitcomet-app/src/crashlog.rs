mod native;
mod panic_capture;
mod report;
mod session;

pub(crate) use panic_capture::confirm as confirm_panic;

use gitcomet_core::identity;
use gitcomet_core::startup_exit::{SessionToken, StartupExitReport, current_session_token};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, Once};

use gitcomet_core::environment::{self, EnvironmentSnapshot};
use gitcomet_core::fs_utils::{open_private_append, write_private_file};

static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
static SESSION_LIFECYCLE: Mutex<()> = Mutex::new(());
/// `(location, message)` hashes already written this session. A driver or
/// GPUI diagnostic that repeats every frame only needs one durable record
/// per distinct error in the recovered report.
static RUNTIME_ERRORS_SEEN: Mutex<Vec<u64>> = Mutex::new(Vec::new());
const MAX_RUNTIME_ERRORS_SEEN: usize = 1_024;
const MAX_RUNTIME_LOG_BYTES: u64 = 8 * 1024;
static CRASH_LOGGER: CrashLogger = CrashLogger;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartupCrashReport {
    pub issue_url: String,
    pub summary: String,
    pub crash_log_path: PathBuf,
    pub snapshot: StartupExitReport,
}

pub fn install() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
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

        gitcomet_ui_gpui::set_native_panic_observer(confirm_panic);
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            panic_capture::observe(info);
            if !gitcomet_core::startup_exit::has_private_panic_capture() {
                previous(info);
            }
        }));
    });
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
        let _ = session::finish(dir);
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
    if !SESSION_ACTIVE.load(Ordering::SeqCst) || !panic_capture::include_issue_details() {
        return;
    }
    let Ok(_guard) = SESSION_LIFECYCLE.lock() else {
        return;
    };
    if !SESSION_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
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
    let mut file = open_private_append(&runtime_error_path(dir))?;
    let remaining = MAX_RUNTIME_LOG_BYTES.saturating_sub(file.metadata()?.len());
    if remaining == 0 {
        return Ok(());
    }
    let entry = format!(
        "\nRuntime error at {} (Unix ms)\n{location}\n{message}\n{info}\n",
        unix_time_ms()
    );
    let end = entry.floor_char_boundary(entry.len().min(remaining as usize));
    file.write_all(&entry.as_bytes()[..end])
}

pub fn begin_session() -> std::io::Result<()> {
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    let _lifecycle = SESSION_LIFECYCLE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    session::begin(&dir)?;
    SESSION_ACTIVE.store(true, Ordering::SeqCst);
    persist_environment(&dir, &environment::cached())?;
    start_environment_writer(dir)
}

fn persist_environment(dir: &Path, snapshot: &EnvironmentSnapshot) -> std::io::Result<()> {
    let bytes = serde_json::to_vec(snapshot)?;
    write_private_file(&environment_path(dir, current_session_token()), &bytes)
}

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

pub fn finish_session() -> std::io::Result<()> {
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    let _lifecycle = SESSION_LIFECYCLE
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    SESSION_ACTIVE.store(false, Ordering::SeqCst);
    session::finish(&dir)
}

pub fn record_session_failure(context: &str, message: &str) -> std::io::Result<()> {
    if !panic_capture::include_issue_details() {
        return Ok(());
    }
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    write_runtime_error_log_in_dir(&dir, context, message, "The UI runtime returned an error.")
}

pub fn take_startup_report() -> Option<StartupCrashReport> {
    let dir = crash_dir()?;
    if !dir.is_dir() {
        return None;
    }
    session::recover(&dir, native::find)
        .inspect_err(|error| eprintln!("Could not recover previous crash reports: {error}"))
        .ok()
        .flatten()
}

fn crash_dir() -> Option<PathBuf> {
    gitcomet_core::platform::dirs::crash_dir()
}

fn crash_directory_unavailable_error() -> std::io::Error {
    std::io::Error::other("the per-user crash directory is unavailable")
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn clear_context(dir: &Path, token: SessionToken) -> std::io::Result<()> {
    for path in [
        last_operation_path_for_session(dir, token),
        runtime_error_path_for_session(dir, token),
        environment_path(dir, token),
        panic_capture::observation_path(dir, token),
    ] {
        remove_file_if_exists(&path)?;
    }
    Ok(())
}

fn last_operation_path_for_session(dir: &Path, token: SessionToken) -> PathBuf {
    dir.join(format!("last-operation-{token}.log"))
}

fn runtime_error_path_for_session(dir: &Path, token: SessionToken) -> PathBuf {
    dir.join(format!("last-runtime-error-{token}.log"))
}

fn runtime_error_path(dir: &Path) -> PathBuf {
    runtime_error_path_for_session(dir, current_session_token())
}

fn environment_path(dir: &Path, token: SessionToken) -> PathBuf {
    dir.join(format!("environment-{token}.json"))
}

fn unix_time_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|duration| u64::try_from(duration.as_millis()).ok())
        .unwrap_or(0)
}

fn single_line_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
