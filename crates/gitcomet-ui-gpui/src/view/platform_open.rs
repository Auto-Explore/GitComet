use std::io;
use std::path::Path;

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinuxOpenTarget {
    ExternalResource,
    FilePath,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LinuxOpenHelper {
    XdgOpen,
    GioOpen,
    WslView,
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const DEFAULT_LINUX_OPEN_HELPERS: [LinuxOpenHelper; 2] =
    [LinuxOpenHelper::XdgOpen, LinuxOpenHelper::GioOpen];
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
const WSL_LINUX_OPEN_HELPERS: [LinuxOpenHelper; 3] = [
    LinuxOpenHelper::XdgOpen,
    LinuxOpenHelper::GioOpen,
    LinuxOpenHelper::WslView,
];

/// Open a URL in the user's default browser.
pub(super) fn open_url(url: &str) -> Result<(), io::Error> {
    let url = validate_external_url(url)?;
    open_with_default(url)
}

/// Open a file or directory with the system's default application.
pub(super) fn open_path(path: &Path) -> Result<(), io::Error> {
    if path.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Path is empty"));
    }
    #[cfg(target_os = "windows")]
    {
        // Normalize to an absolute path to avoid ambiguous explorer.exe argument parsing.
        let path = std::fs::canonicalize(path)?;
        let path = windows_shell_normalized_path(&path);
        open_with_default_os_str(path.as_os_str())
    }

    #[cfg(not(target_os = "windows"))]
    {
        open_with_default_os_str(path.as_os_str())
    }
}

/// Open the file manager and select/reveal the given path.
pub(super) fn open_file_location(path: &Path) -> Result<(), io::Error> {
    if path.is_dir() {
        return open_path(path);
    }

    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()?.join(path)
        };
        let path = std::fs::canonicalize(&absolute).unwrap_or(absolute);
        let path = windows_shell_normalized_path(&path);
        let mut arg = std::ffi::OsString::from("/select,");
        arg.push(path.as_os_str());
        let _ = std::process::Command::new("explorer.exe")
            .arg(arg)
            .spawn()?;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        if try_show_file_in_file_manager(path).is_ok() {
            return Ok(());
        }

        let parent = path.parent().unwrap_or(path);
        open_path(parent)
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Opening file locations is not supported on this platform",
        ))
    }
}

fn open_with_default(arg: &str) -> Result<(), io::Error> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(arg).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // `explorer.exe <url>` can fall back to opening the current folder for
        // long query-heavy URLs on Windows. Route URLs through the shell's
        // protocol handler instead so GitHub issue links reliably open in the
        // default browser.
        let _ = std::process::Command::new("rundll32.exe")
            .arg("url.dll,FileProtocolHandler")
            .arg(arg)
            .spawn()?;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        run_linux_open_with_fallbacks(
            current_linux_is_wsl(),
            LinuxOpenTarget::ExternalResource,
            |helper| launch_linux_open_helper_str(helper, arg),
        )
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        let _ = arg;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Opening external resources is not supported on this platform",
        ))
    }
}

fn open_with_default_os_str(arg: &std::ffi::OsStr) -> Result<(), io::Error> {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open").arg(arg).spawn()?;
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        let _ = std::process::Command::new("explorer.exe")
            .arg(arg)
            .spawn()?;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    {
        run_linux_open_with_fallbacks(
            current_linux_is_wsl(),
            LinuxOpenTarget::FilePath,
            |helper| launch_linux_open_helper_os_str(helper, arg),
        )
    }

    #[cfg(not(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux",
        target_os = "freebsd"
    )))]
    {
        let _ = arg;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Opening files is not supported on this platform",
        ))
    }
}

fn validate_external_url(url: &str) -> Result<&str, io::Error> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "URL is empty"));
    }

    if !trimmed.contains(':') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL is missing a scheme",
        ));
    }

    if is_supported_link_url(trimmed) {
        Ok(trimmed)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL scheme is not allowed",
        ))
    }
}

/// Schemes we refuse to hand to the OS handler no matter how they are written.
///
/// `javascript:`/`data:`/`vbscript:` are script payloads, and `file:` would let
/// text we did not author — a commit message in a cloned repository, say — open
/// an arbitrary local file with its default application.
const DENIED_URL_SCHEMES: [&str; 4] = ["javascript", "data", "vbscript", "file"];

/// Whether a URL is something the app is willing to linkify and open.
///
/// Anything with a hierarchical `scheme://` part qualifies (`ssh://`, `git://`,
/// `vscode://`, …) plus the flat `mailto:`. Requiring `://` is what keeps the
/// script schemes out structurally — they are written `javascript:…`, never with
/// an authority — and [`DENIED_URL_SCHEMES`] covers the rest.
pub(crate) fn is_supported_link_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once(':') else {
        return false;
    };
    if !is_url_scheme(scheme) {
        return false;
    }
    if DENIED_URL_SCHEMES
        .iter()
        .any(|denied| scheme.eq_ignore_ascii_case(denied))
    {
        return false;
    }

    if scheme.eq_ignore_ascii_case("mailto") {
        !rest.is_empty()
    } else {
        rest.strip_prefix("//").is_some_and(|body| !body.is_empty())
    }
}

/// The RFC 3986 scheme grammar: `ALPHA *( ALPHA / DIGIT / "+" / "-" / "." )`.
fn is_url_scheme(scheme: &str) -> bool {
    let mut bytes = scheme.bytes();
    bytes.next().is_some_and(|byte| byte.is_ascii_alphabetic())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn linux_open_helpers(is_wsl: bool) -> &'static [LinuxOpenHelper] {
    if is_wsl {
        &WSL_LINUX_OPEN_HELPERS
    } else {
        &DEFAULT_LINUX_OPEN_HELPERS
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn linux_missing_opener_error(target: LinuxOpenTarget, is_wsl: bool) -> io::Error {
    let subject = match target {
        LinuxOpenTarget::ExternalResource => "open external resources",
        LinuxOpenTarget::FilePath => "open files or folders",
    };
    let mut message = format!(
        "Unable to {subject}: no supported desktop opener was found. Install `xdg-utils` or make `gio open` available."
    );
    if is_wsl {
        message.push_str(" Under WSL, you can also install `wslu` to provide `wslview`.");
    }
    io::Error::new(io::ErrorKind::NotFound, message)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn run_linux_open_with_fallbacks(
    is_wsl: bool,
    target: LinuxOpenTarget,
    mut launch: impl FnMut(LinuxOpenHelper) -> io::Result<()>,
) -> io::Result<()> {
    let mut deferred_spawn_error = None;

    for helper in linux_open_helpers(is_wsl) {
        match launch(*helper) {
            Ok(()) => return Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
            Err(err) => {
                if is_wsl && *helper != LinuxOpenHelper::WslView {
                    if deferred_spawn_error.is_none() {
                        deferred_spawn_error = Some(err);
                    }
                    continue;
                }
                return Err(err);
            }
        }
    }

    if let Some(err) = deferred_spawn_error {
        return Err(err);
    }

    Err(linux_missing_opener_error(target, is_wsl))
}

#[cfg(target_os = "linux")]
fn current_linux_is_wsl() -> bool {
    crate::linux_gui_env::LinuxGuiEnvironment::detect().is_wsl
}

#[cfg(target_os = "freebsd")]
fn current_linux_is_wsl() -> bool {
    false
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn launch_linux_open_helper_str(helper: LinuxOpenHelper, arg: &str) -> io::Result<()> {
    let mut command = std::process::Command::new(linux_open_helper_program(helper));
    if helper == LinuxOpenHelper::GioOpen {
        command.arg("open");
    }
    let _ = command.arg(arg).spawn()?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn launch_linux_open_helper_os_str(
    helper: LinuxOpenHelper,
    arg: &std::ffi::OsStr,
) -> io::Result<()> {
    let mut command = std::process::Command::new(linux_open_helper_program(helper));
    if helper == LinuxOpenHelper::GioOpen {
        command.arg("open");
    }
    let _ = command.arg(arg).spawn()?;
    Ok(())
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn linux_open_helper_program(helper: LinuxOpenHelper) -> &'static str {
    match helper {
        LinuxOpenHelper::XdgOpen => "xdg-open",
        LinuxOpenHelper::GioOpen => "gio",
        LinuxOpenHelper::WslView => "wslview",
    }
}

#[cfg(target_os = "windows")]
fn windows_shell_normalized_path(path: &Path) -> std::path::PathBuf {
    let mut normalized = std::path::PathBuf::new();
    for component in path.components() {
        normalized.push(component.as_os_str());
    }

    let mut rendered = normalized.display().to_string();
    if let Some(stripped) = rendered.strip_prefix(r"\\?\UNC\") {
        rendered = format!(r"\\{stripped}");
    } else if let Some(stripped) = rendered.strip_prefix(r"\\?\") {
        rendered = stripped.to_string();
    }
    std::path::PathBuf::from(rendered)
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn try_show_file_in_file_manager(path: &Path) -> Result<(), io::Error> {
    let file_uri = file_uri_for_file_manager(path)?;
    let show_items_arg = format!("array:string:{file_uri}");
    let status = std::process::Command::new("dbus-send")
        .arg("--session")
        .arg("--dest=org.freedesktop.FileManager1")
        .arg("--type=method_call")
        .arg("/org/freedesktop/FileManager1")
        .arg("org.freedesktop.FileManager1.ShowItems")
        .arg(show_items_arg)
        .arg("string:")
        .status()?;

    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "dbus-send exited with status {status}"
        )))
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn file_uri_for_file_manager(path: &Path) -> Result<String, io::Error> {
    use std::os::unix::ffi::OsStrExt;

    if path.as_os_str().is_empty() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "Path is empty"));
    }

    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let path_bytes = absolute.as_os_str().as_bytes();
    let mut uri = String::with_capacity(path_bytes.len() + "file://".len());
    uri.push_str("file://");

    for &byte in path_bytes {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                uri.push(byte as char);
            }
            _ => {
                uri.push('%');
                uri.push(char::from(b"0123456789ABCDEF"[(byte >> 4) as usize]));
                uri.push(char::from(b"0123456789ABCDEF"[(byte & 0x0F) as usize]));
            }
        }
    }

    Ok(uri)
}

#[cfg(test)]
mod url_policy_tests {
    use super::{is_supported_link_url, validate_external_url};

    #[test]
    fn hierarchical_schemes_are_supported() {
        for url in [
            "http://example.com",
            "https://example.com/a?b=c#d",
            "HTTPS://EXAMPLE.COM",
            "ssh://git@example.com/repo.git",
            "git://example.com/repo.git",
            "ftp://example.com/pub",
            "vscode://file/tmp/x",
        ] {
            assert!(is_supported_link_url(url), "expected {url} to be supported");
        }
    }

    #[test]
    fn mailto_is_supported_without_an_authority() {
        assert!(is_supported_link_url("mailto:someone@example.com"));
        assert!(is_supported_link_url("MAILTO:someone@example.com"));
        // A bare `mailto:` addresses nobody.
        assert!(!is_supported_link_url("mailto:"));
    }

    #[test]
    fn script_and_file_schemes_are_refused() {
        for url in [
            "javascript:alert(1)",
            // Denied even when disguised with an authority.
            "javascript://example.com/%0Aalert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox(1)",
            "file:///etc/passwd",
            "FILE:///etc/passwd",
        ] {
            assert!(!is_supported_link_url(url), "expected {url} to be refused");
        }
    }

    #[test]
    fn flat_schemes_other_than_mailto_are_refused() {
        // Requiring `://` is what keeps script payloads out structurally, so
        // every other flat scheme falls on the same side of the line.
        assert!(!is_supported_link_url("tel:+358401234567"));
        assert!(!is_supported_link_url("http:example.com"));
        assert!(!is_supported_link_url("https://"));
    }

    #[test]
    fn malformed_urls_are_refused() {
        assert!(!is_supported_link_url("example.com"));
        assert!(!is_supported_link_url("://example.com"));
        assert!(!is_supported_link_url("1http://example.com"));
        assert!(!is_supported_link_url("ht tp://example.com"));
    }

    #[test]
    fn validation_trims_and_reports_why_it_refused() {
        assert_eq!(
            validate_external_url("  https://example.com  ").expect("supported url"),
            "https://example.com"
        );
        assert_eq!(
            validate_external_url("   ").expect_err("empty").to_string(),
            "URL is empty"
        );
        assert_eq!(
            validate_external_url("example.com")
                .expect_err("no scheme")
                .to_string(),
            "URL is missing a scheme"
        );
        assert_eq!(
            validate_external_url("javascript:alert(1)")
                .expect_err("denied scheme")
                .to_string(),
            "URL scheme is not allowed"
        );
    }
}

#[cfg(all(test, any(target_os = "linux", target_os = "freebsd")))]
mod tests {
    use super::{
        DEFAULT_LINUX_OPEN_HELPERS, LinuxOpenHelper, LinuxOpenTarget, WSL_LINUX_OPEN_HELPERS,
        file_uri_for_file_manager, linux_open_helpers, run_linux_open_with_fallbacks,
    };
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::Path;

    #[test]
    fn file_uri_percent_encodes_utf8_and_reserved_characters() {
        let uri = file_uri_for_file_manager(Path::new("/tmp/my file#1/\u{00E4}.txt"))
            .expect("uri for absolute path");
        assert_eq!(uri, "file:///tmp/my%20file%231/%C3%A4.txt");
    }

    #[test]
    fn file_uri_percent_encodes_non_utf8_bytes() {
        let path = std::path::PathBuf::from(OsString::from_vec(b"/tmp/nonutf8-\xFF.bin".to_vec()));
        let uri = file_uri_for_file_manager(&path).expect("uri for non-utf8 path");
        assert_eq!(uri, "file:///tmp/nonutf8-%FF.bin");
    }

    #[test]
    fn file_uri_makes_relative_paths_absolute() {
        let uri = file_uri_for_file_manager(Path::new("folder/with space.txt"))
            .expect("uri for relative path");
        assert!(uri.starts_with("file:///"));
        assert!(uri.ends_with("/folder/with%20space.txt"));
    }

    #[test]
    fn linux_open_helpers_only_include_wslview_inside_wsl() {
        assert_eq!(linux_open_helpers(false), &DEFAULT_LINUX_OPEN_HELPERS);
        assert_eq!(linux_open_helpers(true), &WSL_LINUX_OPEN_HELPERS);
    }

    #[test]
    fn linux_open_fallback_tries_wslview_after_spawn_errors_inside_wsl() {
        let mut seen = Vec::new();
        let result =
            run_linux_open_with_fallbacks(true, LinuxOpenTarget::ExternalResource, |helper| {
                seen.push(helper);
                match helper {
                    LinuxOpenHelper::XdgOpen => {
                        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
                    }
                    LinuxOpenHelper::GioOpen => {
                        Err(std::io::Error::from(std::io::ErrorKind::NotFound))
                    }
                    LinuxOpenHelper::WslView => Ok(()),
                }
            });

        assert!(result.is_ok());
        assert_eq!(
            seen,
            vec![
                LinuxOpenHelper::XdgOpen,
                LinuxOpenHelper::GioOpen,
                LinuxOpenHelper::WslView
            ]
        );
    }

    #[test]
    fn linux_open_missing_helper_error_mentions_wslview_under_wsl() {
        let err = run_linux_open_with_fallbacks(true, LinuxOpenTarget::FilePath, |_helper| {
            Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        })
        .expect_err("expected missing-opener error");

        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let message = err.to_string();
        assert!(message.contains("xdg-utils"));
        assert!(message.contains("wslview"));
    }

    #[test]
    fn linux_open_missing_helper_error_omits_wslview_outside_wsl() {
        let err =
            run_linux_open_with_fallbacks(false, LinuxOpenTarget::ExternalResource, |_helper| {
                Err(std::io::Error::from(std::io::ErrorKind::NotFound))
            })
            .expect_err("expected missing-opener error");

        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
        let message = err.to_string();
        assert!(message.contains("xdg-utils"));
        assert!(!message.contains("wslview"));
    }
}

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::windows_shell_normalized_path;
    use std::path::Path;

    #[test]
    fn windows_shell_path_normalizes_mixed_separators() {
        let mixed = Path::new(r"C:\git\GitComet\crates/gitcomet-ui-gpui/src/smoke_tests.rs");
        let normalized = windows_shell_normalized_path(mixed).display().to_string();

        assert!(!normalized.contains('/'));
        assert!(normalized.contains('\\'));
    }

    #[test]
    fn windows_shell_path_strips_verbatim_prefix() {
        let prefixed = Path::new(r"\\?\C:\git\GitComet\src\main.rs");
        let normalized = windows_shell_normalized_path(prefixed)
            .display()
            .to_string();
        assert!(!normalized.starts_with(r"\\?\"));
        assert_eq!(normalized, r"C:\git\GitComet\src\main.rs");
    }
}
