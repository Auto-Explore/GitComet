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
        persist_with_windows_retry(temporary, path, |file, path| file.persist(path))
    }
    #[cfg(not(windows))]
    {
        temporary.persist(path).map_err(|err| err.error)?;
        Ok(())
    }
}

/// MoveFileExW can reject replacement while a reader holds the destination
/// open, even with delete sharing. Retry the same flushed file so readers see
/// either complete version; never remove or truncate the destination first.
#[cfg(any(windows, test))]
fn persist_with_windows_retry(
    mut temporary: tempfile::NamedTempFile,
    path: &Path,
    mut persist: impl FnMut(tempfile::NamedTempFile, &Path) -> Result<File, tempfile::PersistError>,
) -> io::Result<()> {
    // At most 630 ms of backoff, then surface persistent failures normally.
    let mut delays = [10, 20, 40, 80, 160, 320].into_iter();
    loop {
        match persist(temporary, path) {
            Ok(_) => return Ok(()),
            Err(err) => {
                // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
                let retryable = matches!(err.error.raw_os_error(), Some(5 | 32 | 33));
                let Some(delay) = delays.next().filter(|_| retryable) else {
                    return Err(err.error);
                };
                temporary = err.file;
                std::thread::sleep(std::time::Duration::from_millis(delay));
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
    use super::{open_private_append, persist_with_windows_retry, write_private_file};
    use std::io::Write as _;

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

    #[test]
    fn private_replacement_retries_windows_reader_errors_with_the_same_file() {
        for code in [5, 32, 33] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("session.json");
            write_private_file(&path, b"old complete session").unwrap();
            let mut temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
            temporary.write_all(b"new complete session").unwrap();
            temporary.as_file().sync_all().unwrap();
            let original_path = temporary.path().to_path_buf();
            let mut blocked = true;
            persist_with_windows_retry(temporary, &path, |file, path| {
                assert_eq!(file.path(), original_path);
                assert_eq!(std::fs::read(file.path()).unwrap(), b"new complete session");
                assert_eq!(std::fs::read(path).unwrap(), b"old complete session");
                if std::mem::take(&mut blocked) {
                    // Inject Windows errors on every platform so the recovery
                    // and file ownership are also exercised by Linux/macOS CI.
                    Err(tempfile::PersistError {
                        error: std::io::Error::from_raw_os_error(code),
                        file,
                    })
                } else {
                    file.persist(path)
                }
            })
            .unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), b"new complete session");
            assert!(!original_path.exists());
        }
    }

    #[test]
    fn private_replacement_failures_preserve_the_destination_and_clean_up() {
        // A persistent sharing error must eventually fail; unrelated errors
        // must fail immediately. Neither may destroy the existing session.
        for code in [5, 87] {
            let root = tempfile::tempdir().unwrap();
            let path = root.path().join("session.json");
            write_private_file(&path, b"keep this session").unwrap();
            let temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
            let temporary_path = temporary.path().to_path_buf();
            let mut attempts = 0;
            let err = persist_with_windows_retry(temporary, &path, |file, _| {
                attempts += 1;
                Err(tempfile::PersistError {
                    error: std::io::Error::from_raw_os_error(code),
                    file,
                })
            })
            .unwrap_err();
            assert_eq!(err.raw_os_error(), Some(code));
            if code == 5 {
                assert!(attempts > 1, "sharing errors should be retried");
            } else {
                assert_eq!(attempts, 1, "unrelated errors should not be retried");
            }
            assert_eq!(std::fs::read(&path).unwrap(), b"keep this session");
            assert!(!temporary_path.exists(), "failed temporary file is removed");
        }
    }

    #[cfg(windows)]
    #[test]
    fn private_replacement_recovers_after_a_windows_reader_closes() {
        use std::io::Read as _;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.json");
        write_private_file(&path, b"old session").unwrap();
        // The preference test polls using ordinary shared reads, including
        // FILE_SHARE_DELETE. Legacy replacement can still reject this handle.
        let mut reader = Some(std::fs::File::open(&path).unwrap());
        let mut temporary = tempfile::NamedTempFile::new_in(root.path()).unwrap();
        temporary.write_all(b"new session").unwrap();
        temporary.as_file().sync_all().unwrap();
        persist_with_windows_retry(temporary, &path, |file, path| {
            let result = file.persist(path);
            if let Some(mut reader) = reader.take() {
                assert!(result.is_err(), "the open reader must block replacement");
                let mut original = Vec::new();
                reader.read_to_end(&mut original).unwrap();
                assert_eq!(original, b"old session");
                // Release only after a real sharing failure, with no timing
                // dependency on the CI machine's speed or thread scheduling.
                drop(reader);
            }
            result
        })
        .unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"new session");
    }
}
