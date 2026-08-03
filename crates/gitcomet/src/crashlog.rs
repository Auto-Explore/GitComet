use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

static WRITING_CRASH_LOG: AtomicBool = AtomicBool::new(false);
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(false);
static WRITING_RUNTIME_ERROR_LOG: Mutex<()> = Mutex::new(());
static CRASH_LOGGER: CrashLogger = CrashLogger;
const CRASH_ISSUE_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new");
const CRASH_ISSUE_TEMPLATE: &str = "crash_report.md";
const PENDING_REPORT_FILE: &str = "pending-report-path.txt";
const STARTUP_REPORT_FILE: &str = "pending-startup-report.log";
const SESSION_MARKER_FILE: &str = "session-in-progress.log";
const LAST_OPERATION_FILE: &str = "last-operation.log";
const RUNTIME_ERROR_FILE: &str = "last-runtime-error.log";
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
    contents: String,
}

pub fn install() {
    if log::set_logger(&CRASH_LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Error);
    }

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_panic_log(info);
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

        // Keep GPUI's existing terminal diagnostics visible while also making
        // them durable enough to survive an event-loop or native process exit.
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "{}", record.args());
        write_runtime_error_log(record);
    }

    fn flush(&self) {}
}

fn write_runtime_error_log(record: &log::Record<'_>) {
    if !SESSION_ACTIVE.load(Ordering::SeqCst) {
        return;
    }
    let Ok(_guard) = WRITING_RUNTIME_ERROR_LOG.lock() else {
        return;
    };
    let Some(dir) = crash_dir() else {
        return;
    };
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }

    let location = record
        .file()
        .map(|file| match record.line() {
            Some(line) => format!("{file}#L{line}"),
            None => file.to_string(),
        })
        .unwrap_or_else(|| "<unknown log call site>".to_string());
    let message = single_line_text(&record.args().to_string());
    let info = if record.target().is_empty() {
        "An error was emitted through the application logging facade.".to_string()
    } else {
        format!("An error was emitted by log target {}.", record.target())
    };
    let backtrace = Backtrace::force_capture().to_string();
    let _ = write_runtime_error_log_in_dir(&dir, &location, &message, &info, &backtrace);
}

fn write_runtime_error_log_in_dir(
    dir: &Path,
    location: &str,
    message: &str,
    info: &str,
    backtrace: &str,
) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let path = runtime_error_path(dir);
    let has_existing_log = std::fs::metadata(&path)
        .map(|metadata| metadata.len() > 0)
        .unwrap_or(false);
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    if has_existing_log {
        writeln!(file)?;
    }
    writeln!(file, "=== GitComet runtime error ===")?;
    writeln!(file, "failure_kind=runtime-error")?;
    writeln!(file, "timestamp_unix_ms={}", unix_time_ms())?;
    writeln!(
        file,
        "crate={} version={}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )?;
    writeln!(
        file,
        "thread={}",
        std::thread::current().name().unwrap_or("<unnamed>")
    )?;
    writeln!(file, "location={}", single_line_text(location))?;
    writeln!(file, "message={}", single_line_text(message))?;
    writeln!(file, "info={}", single_line_text(info))?;
    writeln!(file, "backtrace:")?;
    writeln!(file, "{backtrace}")?;
    file.flush()?;
    file.sync_data()
}

/// Records entry to the UI event loop. A stale marker lets the next launch
/// report native aborts and signals without attempting I/O in a signal handler.
pub fn begin_session() -> std::io::Result<()> {
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    begin_session_in_dir(&dir)?;
    SESSION_ACTIVE.store(true, Ordering::SeqCst);
    Ok(())
}

fn begin_session_in_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    remove_file_if_exists(&last_operation_path(dir))?;
    remove_file_if_exists(&runtime_error_path(dir))?;

    let mut file = File::create(session_marker_path(dir))?;
    writeln!(file, "=== GitComet abnormal exit candidate ===")?;
    writeln!(file, "failure_kind=abnormal-exit")?;
    writeln!(file, "failure_context=")?;
    writeln!(file, "copy_source=")?;
    writeln!(file, "timestamp_unix_ms={}", unix_time_ms())?;
    writeln!(
        file,
        "crate={} version={}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    )?;
    writeln!(
        file,
        "thread={}",
        std::thread::current().name().unwrap_or("<unnamed>")
    )?;
    writeln!(file, "message=GitComet did not exit cleanly")?;
    writeln!(
        file,
        "info=The previous UI process ended before it completed its shutdown sequence."
    )?;
    writeln!(file, "os={}", std::env::consts::OS)?;
    writeln!(file, "arch={}", std::env::consts::ARCH)?;
    writeln!(file, "display={}", env_value("DISPLAY"))?;
    writeln!(file, "wayland_display={}", env_value("WAYLAND_DISPLAY"))?;
    file.flush()?;
    file.sync_data()
}

/// Clears the current session marker after the UI event loop returns normally.
pub fn finish_session() -> std::io::Result<()> {
    SESSION_ACTIVE.store(false, Ordering::SeqCst);
    let dir = crash_dir().ok_or_else(crash_directory_unavailable_error)?;
    finish_session_in_dir(&dir)
}

fn finish_session_in_dir(dir: &Path) -> std::io::Result<()> {
    remove_file_if_exists(&session_marker_path(dir))?;
    remove_file_if_exists(&last_operation_path(dir))?;
    remove_file_if_exists(&runtime_error_path(dir))
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
    std::fs::create_dir_all(dir)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(session_marker_path(dir))?;
    writeln!(file, "failure_kind=returned-error")?;
    writeln!(file, "failure_context={}", single_line_text(context))?;
    writeln!(file, "timestamp_unix_ms={}", unix_time_ms())?;
    if let Some(location) = location {
        writeln!(file, "location={}", single_line_text(location))?;
    }
    writeln!(file, "message={}", single_line_text(message))?;
    writeln!(
        file,
        "info=GitComet could not complete its GPUI launch or event loop."
    )?;
    if let Some(backtrace) = backtrace {
        writeln!(file, "backtrace:")?;
        writeln!(file, "{backtrace}")?;
    }
    file.flush()?;
    file.sync_data()
}

pub fn take_startup_report() -> Option<StartupCrashReport> {
    let dir = crash_dir()?;
    take_startup_report_from_crash_dir(&dir)
}

fn take_startup_report_from_crash_dir(dir: &Path) -> Option<StartupCrashReport> {
    let report_path = startup_report_path(dir);
    let mut report_log = match std::fs::read_to_string(&report_path) {
        Ok(report_log) => report_log,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => {
            eprintln!(
                "Failed to read pending GitComet crash report {}: {err}",
                report_path.display()
            );
            return None;
        }
    };
    let session_log = read_abnormal_exit_log(
        &session_marker_path(dir),
        &last_operation_path(dir),
        &runtime_error_path(dir),
    )
    .map(|contents| AbnormalExitLog {
        marker_path: session_marker_path(dir),
        last_operation_path: last_operation_path(dir),
        runtime_error_path: runtime_error_path(dir),
        contents,
    });
    let pending_path = pending_report_path(dir);
    let (panic_log, pending_panic_read_failed) = match read_pending_panic_log(&pending_path) {
        Ok(panic_log) => (panic_log, false),
        Err(err) => {
            eprintln!(
                "Failed to read GitComet pending panic report {}: {err}",
                pending_path.display()
            );
            (None, true)
        }
    };
    if !pending_panic_read_failed && panic_log.is_none() && pending_path.exists() {
        let _ = std::fs::remove_file(&pending_path);
    }

    if session_log.is_none() && panic_log.is_none() {
        return (!report_log.trim().is_empty())
            .then(|| build_startup_report(report_path, &report_log));
    }

    if let Some(session_log) = &session_log {
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
            eprintln!("Failed to persist GitComet startup crash report: {err}");
            return None;
        }
    };
    if let Some(session_log) = &session_log {
        if let Err(err) = remove_file_if_exists(&session_log.marker_path) {
            eprintln!(
                "Failed to clear recovered GitComet session marker {}: {err}",
                session_log.marker_path.display()
            );
        } else {
            if let Err(err) = remove_file_if_exists(&session_log.last_operation_path) {
                eprintln!(
                    "Failed to clear recovered GitComet last-operation diagnostics {}: {err}",
                    session_log.last_operation_path.display()
                );
            }
            if let Err(err) = remove_file_if_exists(&session_log.runtime_error_path) {
                eprintln!(
                    "Failed to clear recovered GitComet runtime-error diagnostics {}: {err}",
                    session_log.runtime_error_path.display()
                );
            }
        }
    }
    if let Some(panic_log) = &panic_log {
        let _ = std::fs::remove_file(&pending_path);
        if let Err(err) = remove_file_if_exists(&panic_log.path) {
            eprintln!(
                "Failed to clear snapshotted GitComet panic log {}: {err}",
                panic_log.path.display()
            );
        }
    }
    Some(build_startup_report(report_path, &report_log))
}

fn read_abnormal_exit_log(
    marker: &Path,
    last_operation_path: &Path,
    runtime_error_path: &Path,
) -> Option<String> {
    let mut session_log = match std::fs::read_to_string(marker) {
        Ok(session_log) => session_log,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return None,
        Err(err) => {
            eprintln!(
                "Failed to read GitComet session marker {}: {err}",
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
                "Failed to read GitComet last-operation diagnostics {}: {err}",
                last_operation_path.display()
            );
        }
    }
    match std::fs::read_to_string(runtime_error_path) {
        Ok(runtime_error) => append_report_log(&mut session_log, &runtime_error),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => {
            eprintln!(
                "Failed to read GitComet runtime-error diagnostics {}: {err}",
                runtime_error_path.display()
            );
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
    std::fs::create_dir_all(dir)?;
    let report_path = startup_report_path(dir);
    let temporary_path = dir.join(format!(
        ".{STARTUP_REPORT_FILE}-{}-{}.tmp",
        std::process::id(),
        unix_time_ms()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = File::create(&temporary_path)?;
        file.write_all(report_log.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary_path, &report_path)
    })();
    if let Err(err) = result {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(err);
    }
    Ok(report_path)
}

fn write_panic_log(info: &std::panic::PanicHookInfo<'_>) {
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
    let _ = std::fs::create_dir_all(&dir);

    let Some(path) = crash_log_path(&dir) else {
        return;
    };

    let mut file = match open_append(&path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let _ = writeln!(file, "=== GitComet crash (panic) ===");
    let _ = writeln!(file, "failure_kind=panic");
    let _ = writeln!(file, "timestamp_unix_ms={}", unix_time_ms());
    let _ = writeln!(
        file,
        "crate={} version={}",
        env!("CARGO_PKG_NAME"),
        env!("CARGO_PKG_VERSION")
    );
    let _ = writeln!(
        file,
        "thread={}",
        std::thread::current().name().unwrap_or("<unnamed>")
    );

    if let Some(location) = info.location() {
        let _ = writeln!(file, "location={}#L{}", location.file(), location.line());
    }

    let payload = info
        .payload()
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<non-string panic payload>".to_string());
    let _ = writeln!(file, "message={payload}");
    let _ = writeln!(file, "info={info}");

    let bt = Backtrace::force_capture();
    let _ = writeln!(file, "backtrace:\n{bt}");
    let _ = writeln!(file);
    if file.flush().is_err() || file.sync_data().is_err() {
        return;
    }
    let _ = write_pending_report_path(&pending_report_path(&dir), &path);
}

fn crash_dir() -> Option<PathBuf> {
    crash_dir_base().map(|base| base.join("gitcomet").join("crashes"))
}

fn crash_directory_unavailable_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "GitComet crash state directory is unavailable because XDG_STATE_HOME and HOME are unset",
    )
}

fn remove_file_if_exists(path: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

fn non_empty_path(value: Option<&str>) -> Option<PathBuf> {
    let value = value?.trim();
    if value.is_empty() {
        return None;
    }
    Some(PathBuf::from(value))
}

#[cfg(target_os = "linux")]
fn crash_dir_base() -> Option<PathBuf> {
    crash_dir_base_linux(
        std::env::var("XDG_STATE_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

#[cfg(target_os = "linux")]
fn crash_dir_base_linux(xdg_state_home: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
    non_empty_path(xdg_state_home)
        .or_else(|| non_empty_path(home).map(|home| home.join(".local").join("state")))
}

#[cfg(target_os = "macos")]
fn crash_dir_base() -> Option<PathBuf> {
    crash_dir_base_macos(std::env::var("HOME").ok().as_deref())
}

#[cfg(target_os = "macos")]
fn crash_dir_base_macos(home: Option<&str>) -> Option<PathBuf> {
    non_empty_path(home).map(|home| home.join("Library").join("Logs"))
}

#[cfg(target_os = "windows")]
fn crash_dir_base() -> Option<PathBuf> {
    crash_dir_base_windows(
        std::env::var("LOCALAPPDATA").ok().as_deref(),
        std::env::var("APPDATA").ok().as_deref(),
    )
}

#[cfg(target_os = "windows")]
fn crash_dir_base_windows(local_app_data: Option<&str>, app_data: Option<&str>) -> Option<PathBuf> {
    non_empty_path(local_app_data).or_else(|| non_empty_path(app_data))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn crash_dir_base() -> Option<PathBuf> {
    crash_dir_base_other(std::env::var("HOME").ok().as_deref())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn crash_dir_base_other(home: Option<&str>) -> Option<PathBuf> {
    non_empty_path(home)
}

fn crash_log_path(dir: &Path) -> Option<PathBuf> {
    let pid = std::process::id();
    Some(dir.join(format!("panic-{pid}-{}.log", unix_time_ms())))
}

fn open_append(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

fn pending_report_path(dir: &Path) -> PathBuf {
    dir.join(PENDING_REPORT_FILE)
}

fn startup_report_path(dir: &Path) -> PathBuf {
    dir.join(STARTUP_REPORT_FILE)
}

fn session_marker_path(dir: &Path) -> PathBuf {
    dir.join(SESSION_MARKER_FILE)
}

fn last_operation_path(dir: &Path) -> PathBuf {
    dir.join(LAST_OPERATION_FILE)
}

fn runtime_error_path(dir: &Path) -> PathBuf {
    dir.join(RUNTIME_ERROR_FILE)
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
        std::fs::write(marker, crash_log_path.as_os_str().as_bytes())
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
        std::fs::write(marker, out)
    }

    #[cfg(not(any(unix, windows)))]
    {
        let Some(path_text) = crash_log_path.to_str() else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "crash log path is not valid Unicode on this platform",
            ));
        };
        std::fs::write(marker, path_text)
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
            for chunk in bytes.chunks_exact(2) {
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
fn hex_decode(hex: &str) -> Option<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        return None;
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let high = hex_value(pair[0])?;
        let low = hex_value(pair[1])?;
        out.push((high << 4) | low);
    }
    Some(out)
}

#[cfg(windows)]
fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn unix_time_ms() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn build_startup_report(crash_log_path: PathBuf, crash_log: &str) -> StartupCrashReport {
    let parsed = parse_crash_log(crash_log);
    let issue_title = build_issue_title(&parsed);
    let issue_body = build_issue_body(&parsed, &crash_log_path);
    let summary_message = parsed
        .message
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown failure".to_string());
    let summary_location = parsed
        .location
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown location".to_string());

    StartupCrashReport {
        issue_url: build_issue_url(&issue_title, &issue_body),
        summary: format!(
            "{} at {}",
            truncate_chars(&summary_message, 160),
            truncate_chars(&summary_location, 160)
        ),
        crash_log_path,
    }
}

#[derive(Default)]
struct ParsedCrashLog {
    failure_kind: Option<String>,
    timestamp_unix_ms: Option<String>,
    crate_name: Option<String>,
    crate_version: Option<String>,
    thread: Option<String>,
    location: Option<String>,
    message: Option<String>,
    info: Option<String>,
    failure_context: Option<String>,
    copy_source: Option<String>,
    clipboard_backend: Option<String>,
    backtrace: String,
}

fn parse_crash_log(crash_log: &str) -> ParsedCrashLog {
    let mut parsed = ParsedCrashLog::default();
    let mut in_backtrace = false;

    for raw_line in crash_log.lines() {
        let line = raw_line.trim_end_matches('\r');

        if line == "=== GitComet operation context ===" {
            in_backtrace = false;
            continue;
        }

        if line.starts_with("=== GitComet ") && line.ends_with(" ===") {
            reset_parsed_failure(&mut parsed);
            parsed.failure_kind = if line.contains("panic") {
                Some("panic".to_string())
            } else if line.contains("runtime error") {
                Some("runtime-error".to_string())
            } else if line.contains("abnormal exit") {
                Some("abnormal-exit".to_string())
            } else {
                None
            };
            in_backtrace = false;
            continue;
        }

        if in_backtrace {
            parsed.backtrace.push_str(line);
            parsed.backtrace.push('\n');
            continue;
        }

        if line == "backtrace:" {
            in_backtrace = true;
            continue;
        }

        if let Some(rest) = line.strip_prefix("backtrace:") {
            in_backtrace = true;
            let rest = rest.trim_start();
            if !rest.is_empty() {
                parsed.backtrace.push_str(rest);
                parsed.backtrace.push('\n');
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("timestamp_unix_ms=") {
            parsed.timestamp_unix_ms = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("failure_kind=") {
            parsed.failure_kind = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("crate=") {
            if let Some((name, version)) = rest.split_once(" version=") {
                parsed.crate_name = Some(name.trim().to_string());
                parsed.crate_version = Some(version.trim().to_string());
            } else {
                parsed.crate_name = Some(rest.trim().to_string());
            }
            continue;
        }

        if let Some(rest) = line.strip_prefix("thread=") {
            parsed.thread = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("location=") {
            parsed.location = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("message=") {
            parsed.message = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("info=") {
            parsed.info = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("failure_context=") {
            parsed.failure_context = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("copy_source=") {
            parsed.copy_source = Some(rest.trim().to_string());
            continue;
        }

        if let Some(rest) = line.strip_prefix("clipboard_backend=") {
            parsed.clipboard_backend = Some(rest.trim().to_string());
        }
    }

    parsed
}

fn reset_parsed_failure(parsed: &mut ParsedCrashLog) {
    parsed.failure_kind = None;
    parsed.timestamp_unix_ms = None;
    parsed.crate_name = None;
    parsed.crate_version = None;
    parsed.thread = None;
    parsed.location = None;
    parsed.message = None;
    parsed.info = None;
    parsed.backtrace.clear();
}

fn build_issue_url(title: &str, body: &str) -> String {
    format!(
        "{CRASH_ISSUE_URL}?template={}&title={}&body={}",
        percent_encode(CRASH_ISSUE_TEMPLATE),
        percent_encode(title),
        percent_encode(body)
    )
}

fn build_issue_title(parsed: &ParsedCrashLog) -> String {
    let failure_message = parsed
        .message
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown failure".to_string());
    format!(
        "Crash: {}",
        truncate_chars(&failure_message, MAX_TITLE_CHARS)
    )
}

fn build_issue_body(parsed: &ParsedCrashLog, crash_log_path: &Path) -> String {
    let crate_name = parsed
        .crate_name
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(env!("CARGO_PKG_NAME"));
    let crate_version = parsed
        .crate_version
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(env!("CARGO_PKG_VERSION"));
    let timestamp = parsed
        .timestamp_unix_ms
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let thread = parsed
        .thread
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let failure_kind = parsed
        .failure_kind
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let location = parsed
        .location
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let message = parsed
        .message
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown failure message>");
    let info = parsed
        .info
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown failure info>");
    let copy_source = parsed
        .copy_source
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<no UI operation recorded>");
    let failure_context = parsed
        .failure_context
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");

    let backtrace = {
        let trimmed = parsed.backtrace.trim();
        if trimmed.is_empty() {
            "<no backtrace captured>".to_string()
        } else {
            truncate_chars(trimmed, MAX_BACKTRACE_CHARS)
        }
    };

    let mut body = String::new();
    let _ = writeln!(body, "## Crash Summary");
    let _ = writeln!(body);
    let _ = writeln!(
        body,
        "<!-- Please describe what you were doing right before the crash. -->"
    );
    let _ = writeln!(body, "GitComet ended unexpectedly.");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Environment");
    let _ = writeln!(body);
    let _ = writeln!(body, "- GitComet crate: `{crate_name}`");
    let _ = writeln!(body, "- GitComet version: `{crate_version}`");
    let _ = writeln!(body, "- OS: `{}`", std::env::consts::OS);
    let _ = writeln!(body, "- Arch: `{}`", std::env::consts::ARCH);
    let _ = writeln!(body, "- Crash timestamp (unix ms): `{timestamp}`");
    let _ = writeln!(body, "- Thread: `{thread}`");
    let _ = writeln!(body, "- Failure kind: `{failure_kind}`");
    let _ = writeln!(body, "- Failure location: `{location}`");
    let _ = writeln!(body, "- Failure context: `{failure_context}`");
    let _ = writeln!(body, "- Last UI operation: `{copy_source}`");
    if let Some(clipboard_backend) = parsed
        .clipboard_backend
        .as_deref()
        .filter(|backend| !backend.is_empty())
    {
        let _ = writeln!(body, "- Clipboard backend: `{clipboard_backend}`");
    }
    let _ = writeln!(body, "- Crash log path: `{}`", crash_log_path.display());
    let _ = writeln!(body);

    let _ = writeln!(body, "## Failure Message");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{message}");
    let _ = writeln!(body, "```");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Failure Info");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{info}");
    let _ = writeln!(body, "```");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Backtrace (trimmed)");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{backtrace}");
    let _ = writeln!(body, "```");
    body
}

fn percent_encode(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        let is_unreserved =
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~');
        if is_unreserved {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

fn single_line_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    if max_chars <= 3 {
        return ".".repeat(max_chars);
    }
    let mut out = String::with_capacity(max_chars);
    for (idx, ch) in input.chars().enumerate() {
        if idx + 3 >= max_chars {
            break;
        }
        out.push(ch);
    }
    out.push_str("...");
    out
}

struct ResetFlagOnDrop;

impl Drop for ResetFlagOnDrop {
    fn drop(&mut self) {
        WRITING_CRASH_LOG.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn percent_encode_encodes_reserved_characters() {
        assert_eq!(percent_encode("a b&c/d"), "a%20b%26c%2Fd");
    }

    #[test]
    fn build_issue_url_uses_package_repository_issue_endpoint() {
        let url = build_issue_url("Crash: boom", "details");
        let expected_prefix = format!("{}/issues/new?", env!("CARGO_PKG_REPOSITORY"));
        assert!(url.starts_with(&expected_prefix));
    }

    #[test]
    fn parse_crash_log_extracts_fields() {
        let log = r#"=== GitComet crash (panic) ===
timestamp_unix_ms=123
crate=gitcomet version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
failure_context=main GPUI window launch
copy_source=commit-details-diff
clipboard_backend=x11
backtrace:
frame 1
frame 2
"#;

        let parsed = parse_crash_log(log);
        assert_eq!(parsed.failure_kind.as_deref(), Some("panic"));
        assert_eq!(parsed.timestamp_unix_ms.as_deref(), Some("123"));
        assert_eq!(parsed.crate_name.as_deref(), Some("gitcomet"));
        assert_eq!(parsed.crate_version.as_deref(), Some("0.1.0"));
        assert_eq!(parsed.thread.as_deref(), Some("main"));
        assert_eq!(parsed.location.as_deref(), Some("src/main.rs#L42"));
        assert_eq!(parsed.message.as_deref(), Some("boom happened"));
        assert_eq!(parsed.info.as_deref(), Some("panic info"));
        assert_eq!(
            parsed.failure_context.as_deref(),
            Some("main GPUI window launch")
        );
        assert_eq!(parsed.copy_source.as_deref(), Some("commit-details-diff"));
        assert_eq!(parsed.clipboard_backend.as_deref(), Some("x11"));
        assert!(parsed.backtrace.contains("frame 1"));
        assert!(parsed.backtrace.contains("frame 2"));
    }

    #[test]
    fn parse_crash_log_supports_inline_backtrace_header() {
        let log = "message=boom\nbacktrace:frame 1\nframe 2\n";
        let parsed = parse_crash_log(log);
        assert_eq!(parsed.message.as_deref(), Some("boom"));
        assert!(parsed.backtrace.contains("frame 1"));
        assert!(parsed.backtrace.contains("frame 2"));
    }

    #[test]
    fn build_startup_report_populates_issue_url_and_summary() {
        let log = r#"timestamp_unix_ms=123
crate=gitcomet version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
backtrace:
frame 1
frame 2
"#;
        let report = build_startup_report(PathBuf::from("/tmp/panic.log"), log);
        assert!(report.issue_url.contains("template=crash_report.md"));
        assert!(
            report
                .issue_url
                .contains("title=Crash%3A%20boom%20happened")
        );
        assert!(report.summary.contains("boom happened"));
        assert!(report.summary.contains("src/main.rs#L42"));
    }

    #[test]
    fn take_startup_report_returns_none_without_recovery_state() {
        let dir = tempdir().expect("temp dir");
        assert!(take_startup_report_from_crash_dir(dir.path()).is_none());
    }

    #[test]
    fn panic_report_is_snapshotted_before_pending_marker_is_removed() {
        let dir = tempdir().expect("temp dir");
        let crash_log_path = dir.path().join("panic.log");
        let crash_log = r#"timestamp_unix_ms=123
crate=gitcomet version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
backtrace:
frame 1
frame 2
"#;
        std::fs::write(&crash_log_path, crash_log).expect("write crash log");
        write_pending_report_path(&pending_report_path(dir.path()), &crash_log_path)
            .expect("write pending marker");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("startup report should be available");
        assert_eq!(report.crash_log_path, startup_report_path(dir.path()));
        assert!(report.issue_url.contains("template=crash_report.md"));
        assert!(report.summary.contains("boom happened"));
        assert!(
            !pending_report_path(dir.path()).exists(),
            "the panic marker should be removed after a durable snapshot is written"
        );
        assert!(
            !crash_log_path.exists(),
            "the source panic log should be retired after it is snapshotted"
        );
        assert_eq!(
            std::fs::read_to_string(startup_report_path(dir.path()))
                .expect("read persisted startup report"),
            crash_log
        );
    }

    #[test]
    fn missing_panic_log_clears_pending_marker() {
        let dir = tempdir().expect("temp dir");
        let missing_log_path = dir.path().join("missing.log");
        write_pending_report_path(&pending_report_path(dir.path()), &missing_log_path)
            .expect("write pending marker");

        assert!(take_startup_report_from_crash_dir(dir.path()).is_none());
        assert!(
            !pending_report_path(dir.path()).exists(),
            "pending marker should be removed even when crash log is missing"
        );
    }

    #[test]
    fn previous_session_report_is_snapshotted_before_marker_is_removed() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        let last_operation = last_operation_path(dir.path());
        std::fs::write(
            &marker,
            "timestamp_unix_ms=123\ncrate=gitcomet version=0.1.0\n\
             thread=main\nmessage=GitComet did not exit cleanly\n\
             info=The previous UI process ended before shutdown.\n",
        )
        .expect("write session marker");
        std::fs::write(
            &last_operation,
            "copy_source=commit-details-diff\ncopy_text_bytes=128\n",
        )
        .expect("write last operation");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("stale session should produce a startup report");

        assert_eq!(report.crash_log_path, startup_report_path(dir.path()));
        assert!(report.summary.contains("did not exit cleanly"));
        assert!(report.issue_url.contains("commit-details-diff"));
        assert!(
            !marker.exists(),
            "session marker should be removed after its report is persisted"
        );
        assert!(
            !last_operation.exists(),
            "last-operation diagnostics should be included in the persisted report"
        );
        let startup_report =
            std::fs::read_to_string(startup_report_path(dir.path())).expect("read startup report");
        assert!(startup_report.contains("copy_source=commit-details-diff"));
    }

    #[test]
    fn runtime_error_supplies_failure_location_message_and_backtrace() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        let last_operation = last_operation_path(dir.path());
        std::fs::write(
            &marker,
            "=== GitComet abnormal exit candidate ===\n\
             failure_kind=returned-error\nfailure_context=main GPUI event loop\n\
             message=GPUI event loop ended unexpectedly\n",
        )
        .expect("write session marker");
        std::fs::write(
            &last_operation,
            "copy_source=diff-context-menu\nclipboard_backend=x11\n",
        )
        .expect("write last operation");
        write_runtime_error_log_in_dir(
            dir.path(),
            "crates/gpui_linux/src/linux/wayland/client.rs#L993",
            "Io error: Connection reset by peer (os error 104)",
            "An error was emitted by log target gpui_linux::linux::wayland::client.",
            "runtime frame 1\nruntime frame 2",
        )
        .expect("write runtime error");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("runtime error should produce a startup report");
        let parsed = parse_crash_log(
            &std::fs::read_to_string(&report.crash_log_path).expect("read startup report"),
        );

        assert_eq!(parsed.failure_kind.as_deref(), Some("runtime-error"));
        assert_eq!(
            parsed.location.as_deref(),
            Some("crates/gpui_linux/src/linux/wayland/client.rs#L993")
        );
        assert_eq!(
            parsed.message.as_deref(),
            Some("Io error: Connection reset by peer (os error 104)")
        );
        assert_eq!(
            parsed.failure_context.as_deref(),
            Some("main GPUI event loop")
        );
        assert_eq!(parsed.copy_source.as_deref(), Some("diff-context-menu"));
        assert_eq!(parsed.clipboard_backend.as_deref(), Some("x11"));
        assert!(parsed.backtrace.contains("runtime frame 1"));
        assert!(report.summary.contains("Connection reset by peer"));
        assert!(report.issue_url.contains("wayland%2Fclient.rs%23L993"));
        assert!(
            report
                .issue_url
                .contains("Clipboard%20backend%3A%20%60x11%60")
        );
        assert!(!runtime_error_path(dir.path()).exists());
    }

    #[test]
    fn newest_failure_after_a_backtrace_is_parsed_as_a_separate_event() {
        let log = r#"=== GitComet crash (panic) ===
failure_kind=panic
location=src/old.rs#L1
message=old panic
backtrace:
old frame

=== GitComet abnormal exit candidate ===
failure_kind=returned-error
failure_context=main GPUI event loop
copy_source=diff-context-menu
message=generic event-loop failure

=== GitComet runtime error ===
failure_kind=runtime-error
location=crates/gpui_linux/src/linux/wayland/client.rs#L993
message=Io error: Connection reset by peer
backtrace:
new frame
"#;

        let parsed = parse_crash_log(log);
        assert_eq!(parsed.failure_kind.as_deref(), Some("runtime-error"));
        assert_eq!(
            parsed.location.as_deref(),
            Some("crates/gpui_linux/src/linux/wayland/client.rs#L993")
        );
        assert_eq!(
            parsed.message.as_deref(),
            Some("Io error: Connection reset by peer")
        );
        assert_eq!(parsed.copy_source.as_deref(), Some("diff-context-menu"));
        assert_eq!(
            parsed.failure_context.as_deref(),
            Some("main GPUI event loop")
        );
        assert!(parsed.backtrace.contains("new frame"));
        assert!(!parsed.backtrace.contains("old frame"));
    }

    #[test]
    fn handled_launch_failure_is_reported_at_next_startup() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        std::fs::write(
            &marker,
            "timestamp_unix_ms=123\ncrate=gitcomet version=0.1.0\n\
             thread=main\nmessage=GitComet did not exit cleanly\n",
        )
        .expect("write session marker");

        record_session_failure_in_dir(
            dir.path(),
            "main GPUI window launch",
            "main GPUI window launch failed: no compatible graphics device",
        )
        .expect("record launch failure");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("handled launch failure should produce a startup report");

        assert!(report.summary.contains("no compatible graphics device"));
        assert!(report.issue_url.contains("main%20GPUI%20window%20launch"));
        assert!(
            !marker.exists(),
            "session marker should be removed after its report is persisted"
        );
        assert!(
            startup_report_path(dir.path()).exists(),
            "the report must survive until the user handles the notification"
        );
    }

    #[test]
    fn handled_failure_diagnostics_include_observation_location_and_backtrace() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        std::fs::write(
            &marker,
            "=== GitComet abnormal exit candidate ===\nmessage=initial marker\n",
        )
        .expect("write session marker");
        std::fs::write(
            last_operation_path(dir.path()),
            "copy_source=diff-context-menu\n",
        )
        .expect("write operation context");

        record_session_failure_in_dir_with_diagnostics(
            dir.path(),
            "main GPUI window launch",
            "no compatible graphics device",
            Some("src/main.rs#L275"),
            Some("observation frame 1\nobservation frame 2"),
        )
        .expect("record handled failure");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("handled failure should produce a startup report");
        let parsed = parse_crash_log(
            &std::fs::read_to_string(&report.crash_log_path).expect("read startup report"),
        );
        assert_eq!(parsed.failure_kind.as_deref(), Some("returned-error"));
        assert_eq!(parsed.location.as_deref(), Some("src/main.rs#L275"));
        assert_eq!(parsed.copy_source.as_deref(), Some("diff-context-menu"));
        assert!(parsed.backtrace.contains("observation frame 1"));
    }

    #[test]
    fn pending_startup_report_survives_a_failed_relaunch() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        std::fs::write(
            &marker,
            "message=GitComet did not exit cleanly\ncopy_source=commit-details-diff\n",
        )
        .expect("write initial session marker");

        take_startup_report_from_crash_dir(dir.path())
            .expect("initial abnormal exit should create a startup report");
        begin_session_in_dir(dir.path()).expect("begin retry session");
        assert!(
            startup_report_path(dir.path()).exists(),
            "beginning a retry must not discard the prior report"
        );
        record_session_failure_in_dir(
            dir.path(),
            "main GPUI window launch",
            "main GPUI window launch failed: no compatible graphics device",
        )
        .expect("record retry failure");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("retry failure should preserve a startup report");
        assert!(report.summary.contains("no compatible graphics device"));
        let persisted =
            std::fs::read_to_string(startup_report_path(dir.path())).expect("read startup report");
        assert!(persisted.contains("copy_source=commit-details-diff"));
        assert!(persisted.contains("no compatible graphics device"));
    }

    #[test]
    fn clean_shutdown_keeps_unacknowledged_startup_report() {
        let dir = tempdir().expect("temp dir");
        std::fs::write(startup_report_path(dir.path()), "message=previous crash\n")
            .expect("write startup report");
        begin_session_in_dir(dir.path()).expect("begin session");

        finish_session_in_dir(dir.path()).expect("finish session");

        assert!(
            startup_report_path(dir.path()).exists(),
            "a report is retired only when the user handles the notification"
        );
    }

    #[test]
    fn begin_session_starts_with_fresh_diagnostics() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        let last_operation = last_operation_path(dir.path());
        let runtime_error = runtime_error_path(dir.path());
        std::fs::write(&marker, "message=previous abnormal exit\n").expect("write old marker");
        std::fs::write(&last_operation, "copy_source=commit-details-diff\n")
            .expect("write last-operation diagnostics");
        std::fs::write(&runtime_error, "message=previous runtime error\n")
            .expect("write runtime-error diagnostics");

        begin_session_in_dir(dir.path()).expect("begin session");

        let contents = std::fs::read_to_string(&marker).expect("read session marker");
        assert!(contents.contains("=== GitComet abnormal exit candidate ==="));
        assert!(
            !contents.contains("previous abnormal exit"),
            "a new session must replace an already-recovered marker"
        );
        assert!(!last_operation.exists());
        assert!(!runtime_error.exists());
    }

    #[test]
    fn panic_report_takes_precedence_over_previous_session_marker() {
        let dir = tempdir().expect("temp dir");
        let crash_log_path = dir.path().join("panic.log");
        std::fs::write(&crash_log_path, "message=panic wins\n").expect("write panic log");
        write_pending_report_path(&pending_report_path(dir.path()), &crash_log_path)
            .expect("write pending marker");
        let marker = session_marker_path(dir.path());
        std::fs::write(&marker, "message=GitComet did not exit cleanly\n")
            .expect("write session marker");

        let panic_report = take_startup_report_from_crash_dir(dir.path())
            .expect("panic report should be available");
        assert!(panic_report.summary.contains("panic wins"));
        assert!(
            !marker.exists(),
            "the stale marker should be removed after the combined report is persisted"
        );
        let persisted =
            std::fs::read_to_string(startup_report_path(dir.path())).expect("read startup report");
        assert!(
            persisted.contains("message=GitComet did not exit cleanly"),
            "the persisted panic report should retain the stale-session context"
        );
    }

    #[test]
    fn startup_recovery_consumes_the_previous_session_marker() {
        let dir = tempdir().expect("temp dir");
        let marker = session_marker_path(dir.path());
        std::fs::write(&marker, "message=GitComet did not exit cleanly\n")
            .expect("write previous session marker");

        let report = take_startup_report_from_crash_dir(dir.path())
            .expect("the marker should produce a startup report");
        assert!(report.summary.contains("did not exit cleanly"));
        assert!(
            !marker.exists(),
            "recovery should not use PID liveness checks"
        );
    }

    #[test]
    fn clean_shutdown_removes_session_diagnostics() {
        let dir = tempdir().expect("temp dir");
        begin_session_in_dir(dir.path()).expect("begin session");
        let marker = session_marker_path(dir.path());
        let last_operation = last_operation_path(dir.path());
        let runtime_error = runtime_error_path(dir.path());
        std::fs::write(&last_operation, "copy_source=commit-details-diff\n")
            .expect("write operation diagnostics");
        std::fs::write(&runtime_error, "message=current runtime error\n")
            .expect("write current runtime error");

        finish_session_in_dir(dir.path()).expect("finish current session");

        assert!(!marker.exists());
        assert!(!last_operation.exists());
        assert!(!runtime_error.exists());
    }

    #[test]
    fn build_issue_body_trims_very_long_backtrace() {
        let parsed = ParsedCrashLog {
            backtrace: "x".repeat(MAX_BACKTRACE_CHARS + 128),
            ..Default::default()
        };

        let body = build_issue_body(&parsed, Path::new("/tmp/panic.log"));
        let marker = "## Backtrace (trimmed)\n\n```text\n";
        let start = body.find(marker).expect("backtrace section should exist") + marker.len();
        let end = start
            + body[start..]
                .find("\n```")
                .expect("backtrace code block should close");
        let backtrace_text = &body[start..end];

        assert_eq!(backtrace_text.chars().count(), MAX_BACKTRACE_CHARS);
        assert!(backtrace_text.ends_with("..."));
    }

    #[test]
    fn non_empty_path_trims_and_rejects_empty_values() {
        assert_eq!(non_empty_path(None), None);
        assert_eq!(non_empty_path(Some("   ")), None);
        assert_eq!(non_empty_path(Some(" /tmp ")), Some(PathBuf::from("/tmp")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn crash_dir_base_linux_prefers_xdg_state_home() {
        let base = crash_dir_base_linux(Some("/state"), Some("/home/alice"));
        assert_eq!(base, Some(PathBuf::from("/state")));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn crash_dir_base_linux_falls_back_to_home_state_dir() {
        let base = crash_dir_base_linux(Some("   "), Some("/home/alice"));
        assert_eq!(
            base,
            Some(PathBuf::from("/home/alice").join(".local").join("state"))
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn crash_dir_base_linux_returns_none_when_no_usable_env() {
        assert_eq!(crash_dir_base_linux(None, Some("  ")), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn crash_dir_base_macos_uses_home_logs_dir() {
        let base = crash_dir_base_macos(Some("/Users/alice"));
        assert_eq!(
            base,
            Some(PathBuf::from("/Users/alice").join("Library").join("Logs"))
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn crash_dir_base_macos_returns_none_without_home() {
        assert_eq!(crash_dir_base_macos(Some("   ")), None);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn crash_dir_base_windows_prefers_local_app_data() {
        let base = crash_dir_base_windows(Some(r"C:\Users\alice\AppData\Local"), Some("unused"));
        assert_eq!(base, Some(PathBuf::from(r"C:\Users\alice\AppData\Local")));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn crash_dir_base_windows_falls_back_to_app_data() {
        let base = crash_dir_base_windows(Some("   "), Some(r"C:\Users\alice\AppData\Roaming"));
        assert_eq!(base, Some(PathBuf::from(r"C:\Users\alice\AppData\Roaming")));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn crash_dir_base_windows_returns_none_when_no_usable_env() {
        assert_eq!(crash_dir_base_windows(None, Some("   ")), None);
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    #[test]
    fn crash_dir_base_other_uses_home() {
        let base = crash_dir_base_other(Some("/home/alice"));
        assert_eq!(base, Some(PathBuf::from("/home/alice")));
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    #[test]
    fn crash_dir_base_other_returns_none_without_home() {
        assert_eq!(crash_dir_base_other(Some("   ")), None);
    }
}
