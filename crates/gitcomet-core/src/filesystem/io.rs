use super::Cancellation;
use sha2::{Digest, Sha256};
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

pub(super) fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message.into())
}

/// Resolve parents but preserve the final directory entry (including a link).
pub fn absolute_identity(path: &Path) -> io::Result<PathBuf> {
    let absolute = std::path::absolute(path)?;
    let name = absolute
        .file_name()
        .ok_or_else(|| invalid("A filesystem root is not a file"))?;
    validate_name(name)?;
    Ok(fs::canonicalize(absolute.parent().ok_or_else(|| invalid("Missing parent"))?)?.join(name))
}

pub fn validate_name(name: &OsStr) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.as_encoded_bytes().contains(&0)
        || Path::new(name)
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        || Path::new(name).components().count() != 1
    {
        return Err(invalid(
            "Enter one native file name, without a path separator",
        ));
    }
    #[cfg(windows)]
    {
        let text = name.to_string_lossy();
        let stem = text
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        if text.ends_with([' ', '.'])
            || text.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
            || ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str())
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && matches!(stem.as_bytes()[3], b'1'..=b'9'))
        {
            return Err(invalid("This name is reserved by Windows"));
        }
    }
    Ok(())
}

pub(super) fn exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

pub(super) fn check_cancel(cancellation: &Cancellation) -> io::Result<()> {
    if cancellation.is_cancelled() {
        Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Operation cancelled",
        ))
    } else {
        Ok(())
    }
}

/// Used before execution and again before reversing each step. Includes bytes,
/// link targets, native names and permission bits; timestamps alone are not a
/// sufficient test for an externally replaced or edited file.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiskVersion {
    digest: [u8; 32],
    identity: (u64, u64),
}

impl DiskVersion {
    pub fn same_contents(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
    pub fn read(path: &Path) -> io::Result<Self> {
        Self::read_cancellable(path, &Cancellation::default())
    }

    pub(super) fn read_cancellable(path: &Path, cancellation: &Cancellation) -> io::Result<Self> {
        let mut hasher = Sha256::new();
        hash_entry(path, &mut hasher, cancellation)?;
        Ok(Self {
            digest: hasher.finalize().into(),
            identity: entry_identity(path)?,
        })
    }

    pub(super) fn matches(&self, path: &Path) -> io::Result<()> {
        if Self::read(path)? == *self {
            Ok(())
        } else {
            Err(invalid(format!(
                "{} changed since this operation; its current contents were preserved",
                path.display()
            )))
        }
    }
}

pub(super) fn entry_identity(path: &Path) -> io::Result<(u64, u64)> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let metadata = fs::symlink_metadata(path)?;
        Ok((metadata.dev(), metadata.ino()))
    }
    #[cfg(windows)]
    {
        gitcomet_filesystem_native::file_identity(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Filesystem identity is not available on this platform",
        ))
    }
}

/// A source may change between inspecting its directory entry and opening it.
/// Refuse a replacement link or special file instead of following or blocking
/// on it. Mutations separately validate the source identity before removal.
fn open_regular_file(path: &Path) -> io::Result<File> {
    #[cfg(unix)]
    let file = File::from(rustix::fs::open(
        path,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::NONBLOCK
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?);
    #[cfg(windows)]
    let file = {
        use std::os::windows::fs::OpenOptionsExt;
        OpenOptions::new()
            .read(true)
            .custom_flags(0x0020_0000) // FILE_FLAG_OPEN_REPARSE_POINT
            .open(path)?
    };
    #[cfg(not(any(unix, windows)))]
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(invalid("The source changed into a link or special file"));
    }
    Ok(file)
}

fn hash_entry(path: &Path, hasher: &mut Sha256, cancellation: &Cancellation) -> io::Result<()> {
    check_cancel(cancellation)?;
    let m = fs::symlink_metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        hasher.update(m.permissions().mode().to_le_bytes());
    }
    if m.file_type().is_symlink() {
        hasher.update(b"link");
        hash_bytes(hasher, fs::read_link(path)?.as_os_str().as_encoded_bytes());
    } else if m.is_file() {
        hasher.update(b"file");
        hasher.update(m.len().to_le_bytes());
        let mut file = open_regular_file(path)?;
        let mut buffer = [0u8; 64 * 1024];
        loop {
            check_cancel(cancellation)?;
            let len = file.read(&mut buffer)?;
            if len == 0 {
                break;
            }
            hasher.update(&buffer[..len]);
        }
    } else if m.is_dir() {
        hasher.update(b"directory");
        for entry in children(path)? {
            hash_bytes(
                hasher,
                entry.file_name().unwrap_or_default().as_encoded_bytes(),
            );
            hash_entry(&entry, hasher, cancellation)?;
        }
        hasher.update(b"end-directory");
    } else {
        return Err(invalid(
            "Only regular files, directories and symbolic links are supported",
        ));
    }
    Ok(())
}

fn hash_bytes(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

pub(super) fn children(path: &Path) -> io::Result<Vec<PathBuf>> {
    let mut paths = fs::read_dir(path)?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<io::Result<Vec<_>>>()?;
    paths.sort();
    Ok(paths)
}

pub(super) fn protect(path: &Path, recursive: bool, cancellation: &Cancellation) -> io::Result<()> {
    check_cancel(cancellation)?;
    if path
        .components()
        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
        || path.parent().is_none()
    {
        return Err(invalid("Git metadata and filesystem roots are protected"));
    }
    let metadata = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    if metadata.is_dir() {
        // Includes linked worktrees and submodules, whose .git is a file.
        if exists(&path.join(".git"))?
            || (exists(&path.join("HEAD"))?
                && exists(&path.join("objects"))?
                && exists(&path.join("refs"))?)
        {
            return Err(invalid(
                "Repository roots, submodules and nested repositories are protected",
            ));
        }
        if recursive {
            for child in children(path)? {
                protect(&child, true, cancellation)?;
            }
        }
    }
    Ok(())
}

pub(super) fn copy_tree(
    source: &Path,
    destination: &Path,
    cancellation: &Cancellation,
) -> io::Result<()> {
    check_cancel(cancellation)?;
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source)?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, destination)?;
        #[cfg(windows)]
        {
            use std::os::windows::fs::{FileTypeExt, symlink_dir, symlink_file};
            if metadata.file_type().is_symlink_dir() {
                symlink_dir(target, destination)?;
            } else {
                symlink_file(target, destination)?;
            }
        }
        #[cfg(not(any(unix, windows)))]
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Symbolic links are not supported on this platform",
        ));
    } else if metadata.is_dir() {
        fs::create_dir(destination)?;
        for child in children(source)? {
            copy_tree(
                &child,
                &destination.join(child.file_name().unwrap()),
                cancellation,
            )?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
        filetime::set_file_times(
            destination,
            filetime::FileTime::from_last_access_time(&metadata),
            filetime::FileTime::from_last_modification_time(&metadata),
        )?;
    } else if metadata.is_file() {
        let mut input = open_regular_file(source)?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let mut buffer = [0; 64 * 1024];
        loop {
            check_cancel(cancellation)?;
            let len = input.read(&mut buffer)?;
            if len == 0 {
                break;
            }
            output.write_all(&buffer[..len])?;
        }
        output.set_permissions(metadata.permissions())?;
        filetime::set_file_handle_times(
            &output,
            Some(filetime::FileTime::from_last_access_time(&metadata)),
            Some(filetime::FileTime::from_last_modification_time(&metadata)),
        )?;
        output.sync_all()?;
    } else {
        return Err(invalid("Special filesystem items cannot be transferred"));
    }
    Ok(())
}

/// Never replace a directory entry created by another process after validation.
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    rustix::fs::renameat_with(
        rustix::fs::CWD,
        source,
        rustix::fs::CWD,
        destination,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(Into::into)
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
pub(super) fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    if exists(destination)? {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Destination exists",
        ));
    }
    // Windows MoveFileExW requires REPLACE_EXISTING to replace a file; std's
    // rename enables that flag. Refuse unsupported platforms until a native
    // no-replace implementation is installed instead of risking a lost file.
    let _ = source;
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Exclusive rename is unavailable on this platform",
    ))
}

pub(super) fn remove_tree(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path)?.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

pub(super) fn copy_name(path: &Path) -> io::Result<PathBuf> {
    let directory = fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    let stem = if directory {
        path.file_name()
    } else {
        path.file_stem()
    }
    .ok_or_else(|| invalid("Missing file name"))?;
    for n in 1..=100_000 {
        let mut name = stem.to_os_string();
        name.push(if n == 1 {
            " copy".to_string()
        } else {
            format!(" copy {n}")
        });
        if !directory && let Some(extension) = path.extension() {
            name.push(".");
            name.push(extension);
        }
        let candidate = path.with_file_name(name);
        if !exists(&candidate)? {
            return Ok(candidate);
        }
    }
    Err(invalid("No available copy name"))
}

pub(super) fn encoded_path(path: &Path) -> String {
    let mut encoded = String::new();
    #[cfg(unix)]
    let bytes = {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().to_vec()
    };
    #[cfg(not(unix))]
    let bytes = path.as_os_str().as_encoded_bytes().to_vec();
    for b in bytes {
        use std::fmt::Write;
        if b.is_ascii_alphanumeric() || b"/-_.~".contains(&b) {
            encoded.push(b as char);
        } else {
            let _ = write!(encoded, "%{b:02X}");
        }
    }
    encoded
}

#[cfg(windows)]
pub(super) fn rename_exclusive(source: &Path, destination: &Path) -> io::Result<()> {
    gitcomet_filesystem_native::rename_exclusive(source, destination)
}
