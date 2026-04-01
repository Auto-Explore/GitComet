use std::io;
use std::path::Path;

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
        match std::process::Command::new("xdg-open").arg(arg).spawn() {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let _ = std::process::Command::new("gio")
                    .args(["open"])
                    .arg(arg)
                    .spawn()?;
                Ok(())
            }
            Err(err) => Err(err),
        }
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
        match std::process::Command::new("xdg-open").arg(arg).spawn() {
            Ok(_) => Ok(()),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let _ = std::process::Command::new("gio")
                    .args(["open"])
                    .arg(arg)
                    .spawn()?;
                Ok(())
            }
            Err(err) => Err(err),
        }
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
    use super::file_uri_for_file_manager;
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
