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

/// Open a private regular file for append without following a pre-existing
/// symlink. Callers use this only inside a directory made private above, which
/// also prevents an unprivileged cross-user check/open race.
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
    temporary.persist(path).map_err(|err| err.error)?;
    Ok(())
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
    use super::{ensure_private_dir, open_private_append, write_private_file};
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
}
