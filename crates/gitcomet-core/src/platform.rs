use std::env;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

pub const APP_ID: &str = "dev.gitcomet.GitComet";
pub const APP_DESKTOP_FILE_NAME: &str = "dev.gitcomet.GitComet.desktop";
pub const APP_ICON_NAME: &str = APP_ID;

pub fn is_flatpak_sandbox() -> bool {
    cfg!(any(target_os = "linux", target_os = "freebsd")) && Path::new("/.flatpak-info").exists()
}

pub fn host_command<S: AsRef<OsStr>>(program: S) -> Command {
    let program = program.as_ref();
    if is_flatpak_sandbox() {
        let mut command = Command::new("flatpak-spawn");
        command.arg("--host").arg(program);
        command
    } else {
        Command::new(program)
    }
}

pub fn host_tempdir(prefix: &str) -> io::Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix(prefix);
    if let Some(root) = host_visible_temp_root()? {
        builder.tempdir_in(root)
    } else {
        builder.tempdir()
    }
}

fn host_visible_temp_root() -> io::Result<Option<PathBuf>> {
    if !is_flatpak_sandbox() {
        return Ok(None);
    }

    let cache_home = env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
    let root = cache_home
        .unwrap_or_else(env::temp_dir)
        .join("gitcomet/tmp");
    fs::create_dir_all(&root)?;
    Ok(Some(root))
}
