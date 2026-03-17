use std::backtrace::Backtrace;
use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

static WRITING_CRASH_LOG: AtomicBool = AtomicBool::new(false);
const CRASH_ISSUE_URL: &str = concat!(env!("CARGO_PKG_REPOSITORY"), "/issues/new");
const CRASH_ISSUE_TEMPLATE: &str = "crash_report.md";
const PENDING_REPORT_FILE: &str = "pending-report-path.txt";
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

pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        write_panic_log(info);
        previous(info);
    }));
}

pub fn take_startup_report() -> Option<StartupCrashReport> {
    let dir = crash_dir()?;
    take_startup_report_from_dir(&dir)
}

fn take_startup_report_from_dir(dir: &Path) -> Option<StartupCrashReport> {
    let pending_path = pending_report_path(dir);
    let crash_log_path = read_pending_report_path(&pending_path)?;
    let _ = std::fs::remove_file(&pending_path);
    let crash_log = std::fs::read_to_string(&crash_log_path).ok()?;
    Some(build_startup_report(crash_log_path, &crash_log))
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
    let _ = file.flush();
    let _ = write_pending_report_path(&pending_report_path(&dir), &path);
}

fn crash_dir() -> Option<PathBuf> {
    crash_dir_base().map(|base| base.join("gitcomet").join("crashes"))
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

fn read_pending_report_path(marker: &Path) -> Option<PathBuf> {
    #[cfg(unix)]
    {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;

        let bytes = std::fs::read(marker).ok()?;
        if bytes.is_empty() {
            return None;
        }
        Some(PathBuf::from(OsString::from_vec(bytes)))
    }

    #[cfg(windows)]
    {
        use std::ffi::OsString;
        use std::os::windows::ffi::OsStringExt as _;

        let raw = std::fs::read_to_string(marker).ok()?;
        let value = raw.trim();
        if let Some(hex) = value.strip_prefix(PENDING_REPORT_PATH_WIDE_PREFIX)
            && let Some(bytes) = hex_decode(hex)
            && bytes.len() % 2 == 0
        {
            let mut wide = Vec::with_capacity(bytes.len() / 2);
            for chunk in bytes.chunks_exact(2) {
                wide.push(u16::from_le_bytes([chunk[0], chunk[1]]));
            }
            return Some(PathBuf::from(OsString::from_wide(&wide)));
        }
        if value.is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
    }

    #[cfg(not(any(unix, windows)))]
    {
        std::fs::read_to_string(marker)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
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
        .unwrap_or_else(|| "unknown panic".to_string());
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
    timestamp_unix_ms: Option<String>,
    crate_name: Option<String>,
    crate_version: Option<String>,
    thread: Option<String>,
    location: Option<String>,
    message: Option<String>,
    info: Option<String>,
    backtrace: String,
}

fn parse_crash_log(crash_log: &str) -> ParsedCrashLog {
    let mut parsed = ParsedCrashLog::default();
    let mut in_backtrace = false;

    for raw_line in crash_log.lines() {
        let line = raw_line.trim_end_matches('\r');

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
        }
    }

    parsed
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
    let panic_message = parsed
        .message
        .as_deref()
        .map(single_line_text)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown panic".to_string());
    format!("Crash: {}", truncate_chars(&panic_message, MAX_TITLE_CHARS))
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
    let location = parsed
        .location
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown>");
    let message = parsed
        .message
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown panic message>");
    let info = parsed
        .info
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or("<unknown panic info>");

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
    let _ = writeln!(body, "GitComet crashed with a panic.");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Environment");
    let _ = writeln!(body);
    let _ = writeln!(body, "- GitComet crate: `{crate_name}`");
    let _ = writeln!(body, "- GitComet version: `{crate_version}`");
    let _ = writeln!(body, "- OS: `{}`", std::env::consts::OS);
    let _ = writeln!(body, "- Arch: `{}`", std::env::consts::ARCH);
    let _ = writeln!(body, "- Crash timestamp (unix ms): `{timestamp}`");
    let _ = writeln!(body, "- Thread: `{thread}`");
    let _ = writeln!(body, "- Panic location: `{location}`");
    let _ = writeln!(body, "- Crash log path: `{}`", crash_log_path.display());
    let _ = writeln!(body);

    let _ = writeln!(body, "## Panic Message");
    let _ = writeln!(body);
    let _ = writeln!(body, "```text");
    let _ = writeln!(body, "{message}");
    let _ = writeln!(body, "```");
    let _ = writeln!(body);

    let _ = writeln!(body, "## Panic Info");
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
crate=gitcomet-app version=0.1.0
thread=main
location=src/main.rs#L42
message=boom happened
info=panic info
backtrace:
frame 1
frame 2
"#;

        let parsed = parse_crash_log(log);
        assert_eq!(parsed.timestamp_unix_ms.as_deref(), Some("123"));
        assert_eq!(parsed.crate_name.as_deref(), Some("gitcomet-app"));
        assert_eq!(parsed.crate_version.as_deref(), Some("0.1.0"));
        assert_eq!(parsed.thread.as_deref(), Some("main"));
        assert_eq!(parsed.location.as_deref(), Some("src/main.rs#L42"));
        assert_eq!(parsed.message.as_deref(), Some("boom happened"));
        assert_eq!(parsed.info.as_deref(), Some("panic info"));
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
crate=gitcomet-app version=0.1.0
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
    fn take_startup_report_from_dir_returns_none_without_pending_marker() {
        let dir = tempdir().expect("temp dir");
        assert!(take_startup_report_from_dir(dir.path()).is_none());
    }

    #[test]
    fn take_startup_report_from_dir_consumes_pending_marker_and_returns_report() {
        let dir = tempdir().expect("temp dir");
        let crash_log_path = dir.path().join("panic.log");
        let crash_log = r#"timestamp_unix_ms=123
crate=gitcomet-app version=0.1.0
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

        let report =
            take_startup_report_from_dir(dir.path()).expect("startup report should be available");
        assert_eq!(report.crash_log_path, crash_log_path);
        assert!(report.issue_url.contains("template=crash_report.md"));
        assert!(report.summary.contains("boom happened"));
        assert!(
            !pending_report_path(dir.path()).exists(),
            "pending marker should be removed after consumption"
        );
    }

    #[test]
    fn take_startup_report_from_dir_missing_log_clears_pending_marker() {
        let dir = tempdir().expect("temp dir");
        let missing_log_path = dir.path().join("missing.log");
        write_pending_report_path(&pending_report_path(dir.path()), &missing_log_path)
            .expect("write pending marker");

        assert!(take_startup_report_from_dir(dir.path()).is_none());
        assert!(
            !pending_report_path(dir.path()).exists(),
            "pending marker should be removed even when crash log is missing"
        );
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
