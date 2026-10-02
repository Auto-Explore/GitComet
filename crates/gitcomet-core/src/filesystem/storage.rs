//! Where the filesystem service keeps staging and undo areas.
use crate::path_utils::{OPERATION_AREA_PREFIX, resolved_git_dir};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const LOCK_FILE: &str = "lock";
/// An unlocked instance directory younger than this may still be starting.
const UNLOCKED_GRACE: Duration = Duration::from_secs(10 * 60);

/// Areas must share the data's volume so moves in and out stay renames.
#[derive(Debug, Default)]
pub(super) struct StoragePolicy {
    /// This process's locked directory under the app state dir, once configured.
    instance_dir: Option<PathBuf>,
    #[cfg(test)]
    candidates: Option<Vec<PathBuf>>,
}

impl StoragePolicy {
    pub(super) fn instance(dir: PathBuf) -> Self {
        Self {
            instance_dir: Some(dir),
            #[cfg(test)]
            candidates: None,
        }
    }

    /// Replaces the temp and state dirs, which tests cannot redirect.
    #[cfg(test)]
    pub(super) fn with_candidates(candidates: Vec<PathBuf>) -> Self {
        Self {
            candidates: Some(candidates),
            ..Self::default()
        }
    }

    pub(super) fn instance_dir(&self) -> Option<&Path> {
        self.instance_dir.as_deref()
    }

    /// Temp dir, app state dir, then the repository's Git dir. A user folder is
    /// the last resort; listings, status and the watcher ignore the area there.
    pub(super) fn reserve(&self, parent: &Path) -> io::Result<tempfile::TempDir> {
        let git_dirs = parent.ancestors().filter_map(resolved_git_dir);
        for candidate in self.preferred().into_iter().chain(git_dirs) {
            if same_volume(&candidate, parent)
                && let Ok(area) = create_area(&candidate)
            {
                return Ok(area);
            }
        }
        create_area(parent)
    }

    fn preferred(&self) -> Vec<PathBuf> {
        #[cfg(test)]
        if let Some(candidates) = &self.candidates {
            return candidates.clone();
        }
        std::iter::once(std::env::temp_dir())
            .chain(self.instance_dir.clone())
            .collect()
    }
}

fn create_area(storage: &Path) -> io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix(OPERATION_AREA_PREFIX)
        .tempdir_in(storage)
}

fn same_volume(candidate: &Path, parent: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        fs::metadata(candidate)
            .ok()
            .zip(fs::metadata(parent).ok())
            .is_some_and(|(a, b)| a.dev() == b.dev())
    }
    #[cfg(not(unix))]
    {
        candidate.components().next() == parent.components().next()
    }
}

/// A directory under `root` for this process, locked until the process exits.
pub(super) fn create_instance(root: &Path) -> io::Result<(PathBuf, File)> {
    crate::fs_utils::ensure_private_dir(root)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = root.join(format!("{}-{nanos}", std::process::id()));
    fs::create_dir(&dir)?;
    let lock = File::create_new(dir.join(LOCK_FILE))?;
    lock.try_lock().map_err(io::Error::from)?;
    Ok((dir, lock))
}

/// Remove the instance directory unless it retains recovery areas. Call after
/// the lock handle is dropped: Windows refuses to delete a locked file.
pub(super) fn release_instance(dir: &Path) {
    let only_lock = fs::read_dir(dir).is_ok_and(|entries| {
        entries
            .flatten()
            .all(|entry| entry.file_name() == LOCK_FILE)
    });
    if only_lock {
        let _ = fs::remove_file(dir.join(LOCK_FILE));
        let _ = fs::remove_dir(dir);
    }
}

/// Remove directories of dead instances under `root`. One still holding parked
/// data is kept for manual recovery and reported. Nothing else is touched.
pub(super) fn sweep(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let dir = entry.path();
        if !is_instance_name(&entry.file_name())
            || !entry.file_type().is_ok_and(|kind| kind.is_dir())
            || !instance_is_dead(&dir)
        {
            continue;
        }
        if retains_items(&dir) {
            crate::process::write_stderr_line(format_args!(
                "{}: kept filesystem recovery data from an earlier session at {}",
                crate::identity::current().executable_name(),
                dir.display()
            ));
        } else {
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

fn is_instance_name(name: &std::ffi::OsStr) -> bool {
    name.to_str()
        .and_then(|name| name.split_once('-'))
        .is_some_and(|(pid, nanos)| {
            [pid, nanos]
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
        })
}

fn instance_is_dead(dir: &Path) -> bool {
    match File::open(dir.join(LOCK_FILE)) {
        // Acquired means no live process holds it; dropping releases it again.
        Ok(lock) => lock.try_lock().is_ok(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::metadata(dir)
            .and_then(|metadata| metadata.modified())
            .is_ok_and(|modified| modified.elapsed().is_ok_and(|age| age > UNLOCKED_GRACE)),
        Err(_) => false,
    }
}

/// Unreadable directories count as retaining data: never delete what we cannot see.
fn retains_items(dir: &Path) -> bool {
    match fs::read_dir(dir) {
        Ok(areas) => areas
            .flatten()
            .any(|area| area.path().join("item").symlink_metadata().is_ok()),
        Err(_) => true,
    }
}
