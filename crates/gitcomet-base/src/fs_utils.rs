use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::Path;

/// Create an application-owned directory, owner-only where the platform has
/// Unix permissions.
///
/// The mode is set only on a directory this creates: the path can come from a
/// user override, and re-permissioning a directory we did not make is not ours
/// to do. An existing one is accepted as long as it resolves to a directory,
/// including through a symlink, so relocated state still gets its diagnostics.
pub fn ensure_private_dir(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(_) => {
            if std::fs::metadata(path)?.is_dir() {
                return Ok(());
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("private state path is not a directory: {}", path.display()),
            ));
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(path)
}

/// The directory a private file lives in; a bare file name is refused.
fn private_file_parent(path: &Path) -> io::Result<&Path> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("private file needs a directory: {}", path.display()),
            )
        })
}

/// Open a private regular file for append without following a symlink, whether
/// one is already there or is swapped in while this runs.
pub fn open_private_append(path: &Path) -> io::Result<File> {
    ensure_private_dir(private_file_parent(path)?)?;

    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "private state path is not a regular file: {}",
                    path.display()
                ),
            ));
        }
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }

    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() || !opened_the_named_file(&file, path) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "private state path is not a regular file: {}",
                path.display()
            ),
        ));
    }
    make_file_private(&file);
    Ok(file)
}

/// Whether the open handle is the entry `path` names, catching a symlink
/// swapped in between the check above and the open. Nothing is written before
/// this, so a mismatch costs only the log line.
#[cfg(unix)]
fn opened_the_named_file(file: &File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt as _;

    let (Ok(opened), Ok(named)) = (file.metadata(), std::fs::symlink_metadata(path)) else {
        return false;
    };
    opened.dev() == named.dev() && opened.ino() == named.ino()
}

#[cfg(not(unix))]
fn opened_the_named_file(_file: &File, _path: &Path) -> bool {
    true
}

/// Atomically replace `path` with a private regular file.
pub fn write_private_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = private_file_parent(path)?;
    ensure_private_dir(parent)?;

    let mut temporary = tempfile::Builder::new()
        .prefix(".gitcomet-write-")
        .tempfile_in(parent)?;
    temporary.write_all(bytes)?;
    make_file_private(temporary.as_file());
    temporary.as_file().sync_all()?;
    #[cfg(windows)]
    {
        persist_with_windows_retry(temporary, |temporary| temporary.persist(path))
    }
    #[cfg(not(windows))]
    {
        temporary.persist(path).map_err(|err| err.error)?;
        Ok(())
    }
}

// MoveFileExW can deny replacement while a reader holds the destination open,
// even with Rust's default sharing flags. Retry the same synced temporary file
// so a brief read (or a scanner's handle) cannot discard the user's last save.
#[cfg(any(windows, test))]
fn persist_with_windows_retry(
    mut temporary: tempfile::NamedTempFile,
    mut persist: impl FnMut(tempfile::NamedTempFile) -> Result<File, tempfile::PersistError>,
) -> io::Result<()> {
    use std::time::Duration;

    let mut retries_remaining = 10;
    let mut delay = Duration::from_millis(10);
    loop {
        match persist(temporary) {
            Ok(_) => return Ok(()),
            Err(err) => {
                // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
                // Permanent failures still return the final error and clean up
                // the temporary file after at most 750 ms of retry delays.
                if retries_remaining == 0 || !matches!(err.error.raw_os_error(), Some(5 | 32 | 33))
                {
                    return Err(err.error);
                }
                temporary = err.file;
                retries_remaining -= 1;
                std::thread::sleep(delay);
                delay = (delay * 2).min(Duration::from_millis(100));
            }
        }
    }
}

/// Owner-only, via the handle so a swapped path cannot redirect it. Best-effort.
#[cfg(unix)]
fn make_file_private(file: &File) {
    use std::os::unix::fs::PermissionsExt as _;

    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn make_file_private(_file: &File) {}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use super::ensure_private_dir;
    use super::{open_private_append, write_private_file};
    use std::io::Write as _;

    #[test]
    fn private_replacement_retries_windows_conflicts_without_losing_contents() {
        for code in [5, 32, 33] {
            let root = tempfile::tempdir().expect("tempdir");
            let path = root.path().join("session.json");
            write_private_file(&path, b"previous").expect("initial contents");
            let mut temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
            temporary.write_all(b"replacement").unwrap();
            temporary.as_file().sync_all().unwrap();
            let temporary_path = temporary.path().to_path_buf();
            let mut attempts = 0;
            super::persist_with_windows_retry(temporary, |temporary| {
                attempts += 1;
                assert_eq!(std::fs::read(&path).unwrap(), b"previous");
                assert_eq!(temporary.path(), temporary_path);
                assert_eq!(std::fs::read(temporary.path()).unwrap(), b"replacement");
                if attempts <= 2 {
                    Err(tempfile::PersistError {
                        error: std::io::Error::from_raw_os_error(code),
                        file: temporary,
                    })
                } else {
                    temporary.persist(&path)
                }
            })
            .expect("save after the conflict clears");
            assert_eq!(attempts, 3);
            assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
            assert!(!temporary_path.exists());
        }
    }

    #[test]
    fn private_replacement_bounds_windows_retries_and_cleans_up_failed_saves() {
        for (code, expected_attempts) in [(5, 11), (32, 11), (33, 11), (112, 1)] {
            let root = tempfile::tempdir().expect("tempdir");
            let path = root.path().join("session.json");
            write_private_file(&path, b"previous").expect("initial contents");
            let temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
            let temporary_path = temporary.path().to_path_buf();
            let mut attempts = 0;
            let error = super::persist_with_windows_retry(temporary, |temporary| {
                attempts += 1;
                Err(tempfile::PersistError {
                    error: std::io::Error::from_raw_os_error(code),
                    file: temporary,
                })
            })
            .expect_err("persistent errors must be reported");
            assert_eq!(error.raw_os_error(), Some(code));
            assert_eq!(attempts, expected_attempts);
            assert_eq!(std::fs::read(&path).unwrap(), b"previous");
            assert!(!temporary_path.exists());
        }
    }

    #[cfg(windows)]
    #[test]
    fn private_replacement_recovers_after_a_windows_reader_closes() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("session.json");
        write_private_file(&path, b"previous").expect("initial contents");
        // Permit reads and writes but hold back FILE_SHARE_DELETE so the first
        // real replacement fails deterministically, independently of timing.
        let mut reader = Some(
            std::fs::OpenOptions::new()
                .read(true)
                .share_mode(1 | 2)
                .open(&path)
                .unwrap(),
        );
        let mut temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        temporary.write_all(b"replacement").unwrap();
        temporary.as_file().sync_all().unwrap();
        let mut saw_conflict = false;
        super::persist_with_windows_retry(temporary, |temporary| {
            let result = temporary.persist(&path);
            if result.is_err() && reader.is_some() {
                saw_conflict = true;
                assert_eq!(std::fs::read(&path).unwrap(), b"previous");
                drop(reader.take());
            }
            result
        })
        .expect("save after the reader closes");
        assert!(saw_conflict, "the open reader must block replacement");
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
    }

    #[cfg(unix)]
    #[test]
    fn private_state_uses_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("state");
        let path = dir.join("diagnostic.log");
        ensure_private_dir(&dir).expect("private dir");
        write_private_file(&path, b"diagnostic").expect("private file");

        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn an_existing_directory_keeps_its_own_permissions() {
        use std::os::unix::fs::PermissionsExt as _;

        // The path can come from a user override, so this must never widen or
        // narrow a directory it did not create.
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("shared");
        std::fs::create_dir(&dir).expect("create dir");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).expect("chmod");

        write_private_file(&dir.join("state.json"), b"{}").expect("private file");

        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert_eq!(
            std::fs::metadata(dir.join("state.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_replacement_does_not_follow_existing_symlink() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("state");
        ensure_private_dir(&dir).expect("private dir");
        let victim = root.path().join("victim");
        std::fs::write(&victim, b"keep").expect("victim");
        let path = dir.join("diagnostic.log");
        std::os::unix::fs::symlink(&victim, &path).expect("symlink");

        write_private_file(&path, b"replacement").expect("replace symlink safely");

        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert!(
            !std::fs::symlink_metadata(path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_append_refuses_existing_symlink() {
        let root = tempfile::tempdir().expect("tempdir");
        let dir = root.path().join("state");
        ensure_private_dir(&dir).expect("private dir");
        let victim = root.path().join("victim");
        std::fs::write(&victim, b"keep").expect("victim");
        let path = dir.join("diagnostic.log");
        std::os::unix::fs::symlink(&victim, &path).expect("symlink");

        let err = open_private_append(&path).expect_err("symlink must be refused");
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(victim).unwrap(), b"keep");
    }

    #[test]
    fn private_append_preserves_regular_file_contents() {
        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("diagnostic.log");
        write_private_file(&path, b"first").expect("initial contents");
        let mut file = open_private_append(&path).expect("append");
        file.write_all(b" second").expect("write append");
        drop(file);
        assert_eq!(std::fs::read(path).unwrap(), b"first second");
    }

    #[cfg(windows)]
    #[test]
    fn private_replacement_retries_until_a_reader_releases_the_destination() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("session.json");
        write_private_file(&path, b"old").expect("initial contents");
        // Allow reads and writes but deny deletion, as a scanner can do.
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&path)
            .expect("open reader that prevents replacement");
        let mut temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        temporary.write_all(b"new").unwrap();
        let blocked = temporary
            .persist(&path)
            .expect_err("reader blocks replacement");
        assert!(matches!(blocked.error.raw_os_error(), Some(5 | 32 | 33)));
        assert_eq!(std::fs::read(&path).unwrap(), b"old");

        std::thread::scope(|scope| {
            scope.spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(100));
                drop(reader);
            });
            super::persist_with_windows_retry(blocked.file, |temporary| temporary.persist(&path))
                .expect("retry replacement");
        });
        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn private_replacement_reports_a_persistent_lock_and_cleans_up() {
        use std::os::windows::fs::OpenOptionsExt as _;

        let root = tempfile::tempdir().expect("tempdir");
        let path = root.path().join("session.json");
        write_private_file(&path, b"old").expect("initial contents");
        let _reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1 | 2)
            .open(&path)
            .expect("open reader that prevents replacement");
        let error = write_private_file(&path, b"new").expect_err("lock remains held");
        assert!(matches!(error.raw_os_error(), Some(5 | 32 | 33)));
        assert_eq!(std::fs::read(&path).unwrap(), b"old");
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
