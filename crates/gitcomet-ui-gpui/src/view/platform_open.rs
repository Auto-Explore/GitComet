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

    let Some((scheme, _)) = trimmed.split_once(':') else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL is missing a scheme",
        ));
    };

    if is_allowed_url_scheme(scheme) {
        Ok(trimmed)
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "URL scheme is not allowed",
        ))
    }
}

fn is_allowed_url_scheme(scheme: &str) -> bool {
    scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("https")
        || scheme.eq_ignore_ascii_case("mailto")
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
