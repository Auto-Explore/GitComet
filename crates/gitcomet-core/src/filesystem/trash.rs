//! Exact native trash destinations. Failure never falls back to deletion.
use super::io::{encoded_path, invalid};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub(super) struct Receipt {
    pub item: PathBuf,
    pub info: PathBuf,
}

#[cfg(target_os = "linux")]
pub(super) fn prepare(source: &Path) -> io::Result<Receipt> {
    let data = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".local/share")))
        .filter(|p| p.is_absolute())
        .ok_or_else(|| invalid("No native Trash directory is available"))?;
    prepare_in(source, &data.join("Trash"))
}

#[cfg(not(target_os = "linux"))]
pub(super) fn prepare(_source: &Path) -> io::Result<Receipt> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Native Trash with restore receipts is not available on this platform",
    ))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn prepare_in(source: &Path, root: &Path) -> io::Result<Receipt> {
    for directory in [root.to_path_buf(), root.join("files"), root.join("info")] {
        crate::fs_utils::ensure_private_dir(&directory)?;
        let metadata = fs::symlink_metadata(&directory)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(invalid("Trash directories must not be symbolic links"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.uid() != rustix::process::getuid().as_raw() || metadata.mode() & 0o077 != 0
            {
                return Err(invalid(
                    "Trash directories must belong to you and be private",
                ));
            }
        }
    }
    let unique = tempfile::Builder::new()
        .prefix("gitcomet-")
        .suffix(".trashinfo")
        .tempfile_in(root.join("info"))?;
    let mut name = unique.path().file_stem().unwrap().to_os_string();
    // tempfile reserves the info name atomically; the corresponding item name
    // is committed with a no-replace rename by the service.
    let item = root.join("files").join(&name);
    name.push(".trashinfo");
    let info = root.join("info").join(name);
    let mut unique = unique;
    writeln!(
        unique,
        "[Trash Info]\nPath={}\nDeletionDate={}",
        encoded_path(source),
        jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:%S")
    )?;
    unique.as_file().sync_all()?;
    unique.keep().map_err(|e| e.error)?;
    Ok(Receipt { item, info })
}
