use super::bare::{canonical_path, native_mode, refusal};
use gitcomet_core::domain::RepositoryMode;
use gitcomet_core::error::{BareConversionRefusal, Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

pub(crate) struct OpenedTopology {
    common_dir: PathBuf,
    coordination_path: PathBuf,
    intent_path: PathBuf,
    identity: DirectoryIdentity,
    mode: RepositoryMode,
    config_inputs: std::sync::Mutex<Vec<(PathBuf, crate::repo::RepoFileStamp)>>,
}

impl OpenedTopology {
    pub(crate) fn mode(&self) -> RepositoryMode {
        self.mode
    }

    pub(crate) fn read(root: &Path, repo: &gix::Repository) -> Result<Self> {
        let common_dir = canonical_path(repo.common_dir())?;
        let mut config_inputs = vec![
            common_dir.join("config"),
            repo.git_dir().join("config.worktree"),
        ];
        for section in repo.config_snapshot().plumbing().sections() {
            if let Some(path) = &section.meta().path {
                config_inputs.push(path.clone());
            }
        }
        config_inputs.sort();
        config_inputs.dedup();
        let config_inputs = config_inputs
            .into_iter()
            .map(|path| {
                let stamp = crate::repo::repo_file_stamp(&path);
                (path, stamp)
            })
            .collect();
        Ok(Self {
            coordination_path: coordination_path(&common_dir)?,
            intent_path: sibling_path(coordination_root(&common_dir), ".bare-conversion.lock")?,
            identity: DirectoryIdentity::read(&common_dir)?,
            mode: native_mode(root)?,
            common_dir,
            config_inputs: std::sync::Mutex::new(config_inputs),
        })
    }

    pub(crate) fn acquire_operation_lock(&self) -> Result<OperationLock> {
        OperationLock::acquire_at(&self.coordination_path, &self.intent_path)
    }

    pub(crate) fn check_conversion_intent(&self) -> Result<()> {
        check_conversion_intent(&self.intent_path)
    }

    pub(crate) fn validate(&self, root: &Path) -> Result<()> {
        if DirectoryIdentity::read(&self.common_dir).ok().as_ref() != Some(&self.identity) {
            return Err(Error::new(ErrorKind::RepositoryChanged));
        }
        let mut inputs = self
            .config_inputs
            .lock()
            .expect("repository topology inputs");
        if inputs
            .iter()
            .any(|(path, stamp)| crate::repo::repo_file_stamp(path) != *stamp)
        {
            if native_mode(root)? != self.mode {
                return Err(Error::new(ErrorKind::RepositoryChanged));
            }
            for (path, stamp) in &mut *inputs {
                *stamp = crate::repo::repo_file_stamp(path);
            }
        }
        Ok(())
    }
}

pub(crate) struct OperationLock {
    _file: HeldFileLock,
}

pub(super) struct ConversionLock {
    _coordination: HeldFileLock,
    _sibling: HeldFileLock,
}

struct HeldFileLock(File);

impl Drop for HeldFileLock {
    fn drop(&mut self) {
        if let Err(error) = self.0.unlock() {
            use std::io::Write;
            let _ = writeln!(
                std::io::stderr(),
                "Failed to release repository coordination lock: {error}"
            );
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LockAdmission {
    NewConversion,
    Recovery,
}

fn coordination_root(common_dir: &Path) -> &Path {
    let mut root = common_dir;
    while root.file_name().is_some_and(|name| name == ".git") {
        let Some(parent) = root.parent() else {
            break;
        };
        root = parent;
    }
    root
}

pub(super) fn coordination_path(common_dir: &Path) -> Result<PathBuf> {
    use gitcomet_core::identity::ProductIdentity;
    use gitcomet_core::platform::dirs::{Platform, ProductDirs};
    let environment = |key: &str| match key {
        "GITCOMET_PROFILE_ROOT" | "XDG_STATE_HOME" => None,
        key => std::env::var_os(key),
    };
    let identity = ProductIdentity::gitcomet();
    let directories = ProductDirs::new(&identity, Platform::current(), &environment);
    let directory = directories
        .state_dir()
        .ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "Cannot locate the user directory for repository coordination".into(),
            ))
        })?
        .join("repository-locks");
    gitcomet_core::fs_utils::ensure_private_dir(&directory)?;
    let mut hasher = gix::hash::hasher(gix::hash::Kind::Sha256);
    hasher.update(coordination_root(common_dir).as_os_str().as_encoded_bytes());
    let key = hasher
        .try_finalize()
        .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?;
    Ok(directory.join(format!("{key}.lock")))
}

fn open_lock(path: &Path) -> Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| {
            Error::from(error).with_context("open repository coordination lock", Some(path))
        })
}

impl OperationLock {
    pub(crate) fn acquire(common_dir: &Path) -> Result<Self> {
        Self::acquire_at(
            &coordination_path(common_dir)?,
            &sibling_path(coordination_root(common_dir), ".bare-conversion.lock")?,
        )
    }

    fn acquire_at(coordination_path: &Path, intent_path: &Path) -> Result<Self> {
        let file = open_lock(coordination_path)?;
        match file.try_lock_shared() {
            Ok(()) => {
                let file = HeldFileLock(file);
                check_conversion_intent(intent_path)?;
                Ok(Self { _file: file })
            }
            Err(std::fs::TryLockError::WouldBlock) => Err(Error::new(ErrorKind::RepositoryChanged)),
            Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
        }
    }
}

fn check_conversion_intent(path: &Path) -> Result<()> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(Error::from(error).with_context("read conversion intent", Some(path)));
        }
    };
    match file.try_lock_shared() {
        Ok(()) => {
            drop(HeldFileLock(file));
            Ok(())
        }
        Err(std::fs::TryLockError::WouldBlock) => Err(Error::new(ErrorKind::RepositoryChanged)),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

impl ConversionLock {
    pub(super) fn recover(common_dir: &Path, cancellation: &CancellationToken) -> Result<Self> {
        Self::acquire_with_admission(common_dir, cancellation, LockAdmission::Recovery)
    }

    pub(super) fn acquire(common_dir: &Path, cancellation: &CancellationToken) -> Result<Self> {
        Self::acquire_with_admission(common_dir, cancellation, LockAdmission::NewConversion)
    }

    fn acquire_with_admission(
        common_dir: &Path,
        cancellation: &CancellationToken,
        admission: LockAdmission,
    ) -> Result<Self> {
        let sibling_path = sibling_path(coordination_root(common_dir), ".bare-conversion.lock")?;
        let sibling = open_lock(&sibling_path)?;
        loop {
            cancellation.check_cancelled()?;
            match sibling.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) if admission == LockAdmission::Recovery => {
                    std::thread::sleep(crate::util::GIT_COMMAND_WAIT_POLL_MAX);
                }
                Err(std::fs::TryLockError::WouldBlock) => {
                    return Err(refusal(BareConversionRefusal::ActiveOperation {
                        path: sibling_path,
                    }));
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let sibling = HeldFileLock(sibling);
        let coordination = open_lock(&coordination_path(common_dir)?)?;
        loop {
            cancellation.check_cancelled()?;
            match coordination.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock) => {
                    std::thread::sleep(crate::util::GIT_COMMAND_WAIT_POLL_MAX)
                }
                Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let coordination = HeldFileLock(coordination);
        Ok(Self {
            _coordination: coordination,
            _sibling: sibling,
        })
    }
}

pub(super) fn sibling_path(root: &Path, suffix: &str) -> Result<PathBuf> {
    let parent = root.parent().ok_or_else(|| {
        refusal(BareConversionRefusal::UnsupportedLayout {
            path: root.to_path_buf(),
        })
    })?;
    let name = root.file_name().ok_or_else(|| {
        refusal(BareConversionRefusal::UnsupportedLayout {
            path: root.to_path_buf(),
        })
    })?;
    let mut sibling = OsString::from(".");
    sibling.push(name);
    sibling.push(suffix);
    Ok(parent.join(sibling))
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DirectoryIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial: u64,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(not(any(unix, windows)))]
    created: u128,
}

impl DirectoryIdentity {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let metadata = path.symlink_metadata()?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(refusal(BareConversionRefusal::PathCollision {
                path: path.to_path_buf(),
            }));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(windows)]
        {
            let handle = winapi_util::Handle::from_path_any(path)?;
            let information = winapi_util::file::information(&handle)?;
            Ok(Self {
                volume_serial: information.volume_serial_number(),
                file_index: information.file_index(),
            })
        }
        #[cfg(not(any(unix, windows)))]
        {
            let created = metadata
                .created()?
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|error| Error::new(ErrorKind::Backend(error.to_string())))?
                .as_nanos();
            Ok(Self { created })
        }
    }
}

#[derive(Default)]
struct State {
    generation: u64,
    active: usize,
    converting: bool,
}

#[derive(Default)]
pub(crate) struct RepositoryLifecycle {
    state: Mutex<State>,
    idle: Condvar,
}

pub(crate) struct OperationLease {
    lifecycle: Arc<RepositoryLifecycle>,
    _disk_lock: OperationLock,
}

pub(super) struct ConversionLease(Arc<RepositoryLifecycle>);

impl RepositoryLifecycle {
    pub(super) fn generation(&self) -> u64 {
        self.state.lock().expect("repository lifecycle").generation
    }

    pub(super) fn check(&self, generation: u64) -> Result<()> {
        let state = self.state.lock().expect("repository lifecycle");
        if state.generation != generation || state.converting {
            return Err(Error::new(ErrorKind::RepositoryChanged));
        }
        Ok(())
    }

    pub(super) fn begin(
        self: &Arc<Self>,
        generation: u64,
        disk_lock: OperationLock,
    ) -> Result<OperationLease> {
        let mut state = self.state.lock().expect("repository lifecycle");
        if state.generation != generation || state.converting {
            return Err(Error::new(ErrorKind::RepositoryChanged));
        }
        state.active += 1;
        Ok(OperationLease {
            lifecycle: self.clone(),
            _disk_lock: disk_lock,
        })
    }

    pub(super) fn convert(
        self: &Arc<Self>,
        generation: u64,
        cancellation: &CancellationToken,
    ) -> Result<ConversionLease> {
        {
            let mut state = self.state.lock().expect("repository lifecycle");
            if state.generation != generation || state.converting {
                return Err(Error::new(ErrorKind::RepositoryChanged));
            }
            state.converting = true;
        }
        let lease = ConversionLease(self.clone());
        let mut state = self.state.lock().expect("repository lifecycle");
        while state.active != 0 {
            cancellation.check_cancelled()?;
            state = self
                .idle
                .wait_timeout(state, crate::util::GIT_COMMAND_WAIT_POLL_MAX)
                .expect("repository lifecycle")
                .0;
        }
        drop(state);
        cancellation.check_cancelled()?;
        Ok(lease)
    }
}

impl ConversionLease {
    pub(super) fn invalidate(&self) {
        self.0
            .state
            .lock()
            .expect("repository lifecycle")
            .generation += 1;
    }
}

impl Drop for OperationLease {
    fn drop(&mut self) {
        let mut state = self.lifecycle.state.lock().expect("repository lifecycle");
        state.active -= 1;
        self.lifecycle.idle.notify_all();
    }
}

impl Drop for ConversionLease {
    fn drop(&mut self) {
        self.0
            .state
            .lock()
            .expect("repository lifecycle")
            .converting = false;
        self.0.idle.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_lease_unlocks_even_while_a_duplicated_file_description_remains() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("repository");
        std::fs::create_dir(&root).unwrap();
        let shared = OperationLock::acquire(&root).unwrap();
        let duplicate = shared._file.0.try_clone().unwrap();
        drop(shared);
        let probe = open_lock(&coordination_path(&root).unwrap()).unwrap();
        probe.try_lock().unwrap();
        drop(HeldFileLock(probe));
        let conversion = ConversionLock::acquire(&root, &CancellationToken::new()).unwrap();
        let duplicate_intent = conversion._sibling.0.try_clone().unwrap();
        drop(conversion);
        let _admitted = OperationLock::acquire(&root).unwrap();
        drop(duplicate_intent);
        drop(duplicate);
    }

    #[cfg(windows)]
    #[test]
    fn replacing_a_directory_with_preserved_creation_time_invalidates_topology() {
        use std::os::windows::fs::{FileTimesExt, OpenOptionsExt};
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("checkout");
        let replacement = directory.path().join("replacement");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(&replacement).unwrap();
        crate::test_support::init_test_repo(&root);
        crate::test_support::init_test_repo(&replacement);
        let repo = crate::open::open_worktree_repo(&root).unwrap();
        let retained = OpenedTopology::read(&root, &repo).unwrap();
        drop(repo);
        let git_dir = root.join(".git");
        let before = DirectoryIdentity::read(&git_dir).unwrap();
        let created = git_dir.metadata().unwrap().created().unwrap();
        std::fs::rename(&git_dir, directory.path().join("old-metadata")).unwrap();
        std::fs::rename(replacement.join(".git"), &git_dir).unwrap();
        let handle = OpenOptions::new()
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(&git_dir)
            .unwrap();
        handle
            .set_times(std::fs::FileTimes::new().set_created(created))
            .unwrap();
        assert_eq!(git_dir.metadata().unwrap().created().unwrap(), created);
        assert_ne!(DirectoryIdentity::read(&git_dir).unwrap(), before);
        assert!(matches!(
            retained.validate(&root).unwrap_err().kind(),
            ErrorKind::RepositoryChanged
        ));
    }
}
