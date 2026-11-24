use super::bare::{canonical_path, native_mode, refusal};
use gitcomet_core::domain::RepositoryMode;
use gitcomet_core::error::{BareConversionRefusal, Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

fn config_input_stamp(path: &Path) -> crate::repo::RepoFileStamp {
    use std::io::Read as _;

    match path.metadata() {
        Ok(metadata) if metadata.is_file() => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return crate::repo::RepoFileStamp::default();
        }
        _ => return crate::repo::RepoFileStamp::uncacheable(),
    }
    let mut file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return crate::repo::RepoFileStamp::default();
        }
        Err(_) => return crate::repo::RepoFileStamp::uncacheable(),
    };
    // Metadata alone can miss same-size edits with preserved/coalesced mtime.
    // Hash small config inputs without allocating a whole-file buffer.
    let mut hash = gix::hash::hasher(gix::hash::Kind::Sha256);
    let mut buffer = [0u8; 8192];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                hash.update(&buffer[..count]);
            }
            Err(_) => return crate::repo::RepoFileStamp::uncacheable(),
        }
    }
    let Ok(content_id) = hash.try_finalize() else {
        return crate::repo::RepoFileStamp::uncacheable();
    };
    crate::repo::RepoFileStamp {
        exists: true,
        content_id: Some(content_id),
        ..Default::default()
    }
}

#[derive(Eq, PartialEq)]
enum DiscoveryEntry {
    Missing,
    Directory(DirectoryIdentity),
    File(crate::repo::RepoFileStamp),
}

fn discovery_entry(path: &Path) -> Result<DiscoveryEntry> {
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DiscoveryEntry::Missing);
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() {
        return discovery_entry(&canonical_path(path)?);
    }
    if metadata.is_dir() {
        Ok(DiscoveryEntry::Directory(DirectoryIdentity::read(path)?))
    } else {
        Ok(DiscoveryEntry::File(config_input_stamp(path)))
    }
}

pub(crate) struct OpenedTopology {
    common_dir: PathBuf,
    git_dir: PathBuf,
    git_dir_identity: DirectoryIdentity,
    common_link_path: PathBuf,
    common_link: crate::repo::RepoFileStamp,
    discovery_path: PathBuf,
    discovery_entry: DiscoveryEntry,
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
        let git_dir = canonical_path(repo.git_dir())?;
        let common_link_path = git_dir.join("commondir");
        let discovery_path = root.join(".git");
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
                let stamp = config_input_stamp(&path);
                (path, stamp)
            })
            .collect();
        Ok(Self {
            git_dir_identity: DirectoryIdentity::read(&git_dir)?,
            common_link: config_input_stamp(&common_link_path),
            common_link_path,
            discovery_entry: discovery_entry(&discovery_path)?,
            discovery_path,
            git_dir,
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
        // Cached mode is valid only for the discovery chain we opened. A
        // rewritten gitfile or a newly nested .git can select another repo
        // without changing the original common directory's config inputs.
        if discovery_entry(&self.discovery_path)? != self.discovery_entry
            || (self.git_dir != self.common_dir
                && DirectoryIdentity::read(&self.git_dir).ok().as_ref()
                    != Some(&self.git_dir_identity))
            || config_input_stamp(&self.common_link_path) != self.common_link
        {
            return Err(Error::new(ErrorKind::RepositoryChanged));
        }
        let mut inputs = self
            .config_inputs
            .lock()
            .expect("repository topology inputs");
        if inputs
            .iter()
            .any(|(path, stamp)| config_input_stamp(path) != *stamp)
        {
            let before: Vec<_> = inputs
                .iter()
                .map(|(path, _)| config_input_stamp(path))
                .collect();
            if before.iter().any(|stamp| stamp.uncacheable_nonce.is_some()) {
                // An unreadable input cannot be memoized. Ask Git for this
                // operation without serializing other native checks on the
                // configuration lock; later operations must revalidate too.
                drop(inputs);
                return if native_mode(root)? == self.mode {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::RepositoryChanged))
                };
            }
            if native_mode(root)? != self.mode {
                return Err(Error::new(ErrorKind::RepositoryChanged));
            }
            let after: Vec<_> = inputs
                .iter()
                .map(|(path, _)| config_input_stamp(path))
                .collect();
            if before.iter().zip(&after).any(|(before, after)| {
                before.uncacheable_nonce.is_none()
                    && after.uncacheable_nonce.is_none()
                    && before != after
            }) {
                return Err(Error::new(ErrorKind::RepositoryChanged));
            }
            for (((_, stamp), before), after) in inputs.iter_mut().zip(before).zip(after) {
                // Git remains authoritative when an input cannot be read (for
                // example an unused config.worktree directory). Never memoize
                // that uncertainty: the next operation must ask Git again.
                *stamp = if before.uncacheable_nonce.is_some() || after.uncacheable_nonce.is_some()
                {
                    crate::repo::RepoFileStamp::uncacheable()
                } else {
                    before
                };
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

/// A lock file made this long ago is removed when nothing holds it.
const STALE_AFTER: std::time::Duration = std::time::Duration::from_secs(30 * 24 * 60 * 60);
/// Times a lock file may lose its name while it is being locked.
const LOCK_ATTEMPTS: usize = 8;
/// Milliseconds an operation waits for a lock file nothing converts.
const PRUNER_WAITS: usize = 200;

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
    // Every profile of one user coordinates through one directory.
    let identity = ProductIdentity::gitcomet();
    let environment = |key: &str| {
        #[cfg(any(test, feature = "test-support"))]
        if key == "GITCOMET_PROFILE_ROOT" {
            return crate::test_support::coordination::profile_root(&identity);
        }

        match key {
            "GITCOMET_PROFILE_ROOT" | "XDG_STATE_HOME" => None,
            key => std::env::var_os(key),
        }
    };
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
    // Once per process, and off the thread that is opening a repository.
    static PRUNE: std::sync::Once = std::sync::Once::new();
    PRUNE.call_once(|| {
        let directory = directory.clone();
        let stale_after = STALE_AFTER;
        #[cfg(any(test, feature = "test-support"))]
        let stale_after = crate::test_support::coordination::stale_after(stale_after);
        let _ = std::thread::Builder::new()
            .name("repository-lock-prune".into())
            .spawn(move || prune_when_due(&directory, stale_after));
    });
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

/// Opens the lock file at `path` and locks it with `lock`, again if the file
/// was removed in between: a prune may take an unused file away after it was
/// opened here, and a lock on a file without a name excludes nobody who opens
/// the name afterwards.
fn lock_named(path: &Path, mut lock: impl FnMut(&File) -> Result<()>) -> Result<HeldFileLock> {
    for _ in 0..LOCK_ATTEMPTS {
        let file = open_lock(path)?;
        lock(&file)?;
        let held = HeldFileLock(file);
        if names(path, &held.0) {
            return Ok(held);
        }
    }
    Err(
        Error::new(ErrorKind::Backend("lock file kept being removed".into()))
            .with_context("lock repository coordination file", Some(path)),
    )
}

/// Whether `path` still names the open `file`.
#[cfg(unix)]
fn names(path: &Path, file: &File) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (path.metadata(), file.metadata()) {
        (Ok(named), Ok(held)) => named.dev() == held.dev() && named.ino() == held.ino(),
        _ => false,
    }
}

#[cfg(windows)]
fn names(path: &Path, file: &File) -> bool {
    let identity = |file: &File| {
        winapi_util::file::information(file)
            .map(|file| (file.volume_serial_number(), file.file_index()))
    };
    match (
        File::open(path).and_then(|named| identity(&named)),
        identity(file),
    ) {
        (Ok(named), Ok(held)) => named == held,
        _ => false,
    }
}

/// Nothing is pruned where a file's identity cannot be read.
#[cfg(not(any(unix, windows)))]
fn names(_path: &Path, _file: &File) -> bool {
    true
}

/// How long ago `file` was written, if it can be told.
fn age(file: &File) -> Option<std::time::Duration> {
    file.metadata().ok()?.modified().ok()?.elapsed().ok()
}

/// Its modification time is when the directory was last pruned.
const PRUNED_MARKER: &str = "pruned";

/// Prunes `directory` unless that was done lately: every process that opens
/// a repository asks, and reading the directory is the cost.
fn prune_when_due(directory: &Path, stale_after: std::time::Duration) {
    let marker = directory.join(PRUNED_MARKER);
    let pruned_lately = File::open(&marker)
        .ok()
        .and_then(|marker| age(&marker))
        .is_some_and(|since| since < stale_after / 4);
    // Written, not just created: the write is what moves its time. Without
    // the mark every process would read the directory.
    if !pruned_lately && std::fs::write(&marker, "pruned\n").is_ok() {
        prune_stale_locks(directory, stale_after);
    }
}

/// Removes the lock files in `directory` that were made `stale_after` ago or
/// earlier and that nothing holds now. One file per repository ever opened
/// would otherwise stay for good; a repository still in use gets a new one.
///
/// A file is removed under its own exclusive lock, which whoever uses the
/// repository at that moment prevents; whoever opened it just before finds
/// that it no longer has its name (`lock_named`).
#[cfg(any(unix, windows))]
fn prune_stale_locks(directory: &Path, stale_after: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let stale = |modified: std::io::Result<std::time::SystemTime>| {
        modified
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|unused| unused >= stale_after)
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "lock")
            || !stale(entry.metadata().and_then(|metadata| metadata.modified()))
        {
            continue;
        }
        let Ok(file) = OpenOptions::new().read(true).write(true).open(&path) else {
            continue;
        };
        if file.try_lock().is_err() {
            continue;
        }
        let held = HeldFileLock(file);
        // Under the lock it is still the file of that name, or a new one.
        if stale(held.0.metadata().and_then(|metadata| metadata.modified()))
            && names(&path, &held.0)
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn prune_stale_locks(_directory: &Path, _stale_after: std::time::Duration) {}

impl OperationLock {
    pub(crate) fn acquire(common_dir: &Path) -> Result<Self> {
        Self::acquire_at(
            &coordination_path(common_dir)?,
            &sibling_path(coordination_root(common_dir), ".bare-conversion.lock")?,
        )
    }

    fn acquire_at(coordination_path: &Path, intent_path: &Path) -> Result<Self> {
        let file = lock_named(coordination_path, |file| {
            // A conversion holds the file alone, and its intent, while it
            // runs; the stale-lock pruner holds the file alone for a moment.
            for _ in 0..PRUNER_WAITS {
                match file.try_lock_shared() {
                    Ok(()) => return Ok(()),
                    Err(std::fs::TryLockError::WouldBlock)
                        if check_conversion_intent(intent_path).is_ok() =>
                    {
                        std::thread::sleep(std::time::Duration::from_millis(1));
                    }
                    Err(std::fs::TryLockError::WouldBlock) => break,
                    Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
                }
            }
            Err(Error::new(ErrorKind::RepositoryChanged))
        })?;
        check_conversion_intent(intent_path)?;
        Ok(Self { _file: file })
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
        let coordination = lock_named(&coordination_path(common_dir)?, |file| {
            loop {
                cancellation.check_cancelled()?;
                match file.try_lock() {
                    Ok(()) => return Ok(()),
                    Err(std::fs::TryLockError::WouldBlock) => {
                        std::thread::sleep(crate::util::GIT_COMMAND_WAIT_POLL_MAX)
                    }
                    Err(std::fs::TryLockError::Error(error)) => return Err(error.into()),
                }
            }
        })?;
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

    /// Every repository a test opens gets a lock file. Those belong in the
    /// temporary directory, not among the user's own application state.
    #[test]
    fn a_test_binary_keeps_its_lock_files_in_the_temporary_directory() {
        let directory = tempfile::tempdir().unwrap();
        let path = coordination_path(directory.path()).unwrap();
        assert!(path.starts_with(std::env::temp_dir()), "{}", path.display());
        // A directory of the user's own: another one's is closed to them.
        if let Ok(user) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
            assert!(
                path.components().any(|part| part
                    .as_os_str()
                    .to_string_lossy()
                    .ends_with(&format!("-{user}"))),
                "{}",
                path.display()
            );
        }
    }

    const HOUR: std::time::Duration = std::time::Duration::from_secs(3_600);

    fn lock_file(directory: &Path, name: &str, age: std::time::Duration) -> PathBuf {
        let path = directory.join(name);
        let file = open_lock(&path).unwrap();
        file.set_modified(std::time::SystemTime::now() - age)
            .unwrap();
        path
    }

    fn try_shared(file: &File) -> Result<()> {
        file.try_lock_shared()
            .map_err(|_| Error::new(ErrorKind::RepositoryChanged))
    }

    /// The pruner holds a lock file alone while it removes it: an operation
    /// that meets it then waits, and only a conversion turns it away.
    #[test]
    fn an_operation_waits_out_the_pruner_but_not_a_conversion() {
        let directory = tempfile::tempdir().unwrap();
        let coordination = directory.path().join("repository.lock");
        let intent = directory.path().join(".repository.bare-conversion.lock");
        let pruner = open_lock(&coordination).unwrap();
        pruner.try_lock().unwrap();
        let (acquired_tx, acquired) = std::sync::mpsc::channel();
        let operation = {
            let (coordination, intent) = (coordination.clone(), intent.clone());
            std::thread::spawn(move || {
                let lock = OperationLock::acquire_at(&coordination, &intent);
                acquired_tx.send(()).unwrap();
                lock.map(drop)
            })
        };
        // Still waiting while the pruner holds the file.
        assert!(
            acquired
                .recv_timeout(std::time::Duration::from_millis(30))
                .is_err()
        );
        std::fs::remove_file(&coordination).unwrap();
        drop(HeldFileLock(pruner));
        operation.join().unwrap().unwrap();

        let conversion = open_lock(&coordination).unwrap();
        conversion.try_lock().unwrap();
        let held_intent = open_lock(&intent).unwrap();
        held_intent.try_lock().unwrap();
        let refused = OperationLock::acquire_at(&coordination, &intent).map(drop);
        assert!(matches!(
            refused.unwrap_err().kind(),
            ErrorKind::RepositoryChanged
        ));
        drop(HeldFileLock(held_intent));
        drop(HeldFileLock(conversion));
    }

    #[test]
    fn old_lock_files_nothing_holds_are_removed_and_the_rest_kept() {
        let directory = tempfile::tempdir().unwrap();
        let unused = lock_file(directory.path(), "unused.lock", 2 * HOUR);
        let held_path = lock_file(directory.path(), "held.lock", 2 * HOUR);
        let recent = lock_file(directory.path(), "recent.lock", HOUR / 2);
        // A conversion's journal lives beside the locks and is not one.
        let journal = lock_file(directory.path(), "journal.conversion.json", 2 * HOUR);
        let held = open_lock(&held_path).unwrap();
        held.try_lock_shared().unwrap();

        prune_stale_locks(directory.path(), HOUR);

        assert!(!unused.exists());
        assert!(held_path.exists());
        assert!(recent.exists());
        assert!(journal.exists());
        drop(HeldFileLock(held));
    }

    /// Every process that opens a repository asks for a prune, and under
    /// nextest every test is a process: the directory is read when it is due.
    #[test]
    fn a_directory_pruned_lately_is_not_read_again() {
        let directory = tempfile::tempdir().unwrap();
        let first = lock_file(directory.path(), "first.lock", 2 * HOUR);
        prune_when_due(directory.path(), HOUR);
        assert!(!first.exists());

        let second = lock_file(directory.path(), "second.lock", 2 * HOUR);
        prune_when_due(directory.path(), HOUR);
        assert!(second.exists());

        let marker = File::options()
            .write(true)
            .open(directory.path().join(PRUNED_MARKER))
            .unwrap();
        marker
            .set_modified(std::time::SystemTime::now() - HOUR / 2)
            .unwrap();
        prune_when_due(directory.path(), HOUR);
        assert!(!second.exists());
    }

    /// A prune may remove the file between its opening and its locking. A
    /// lock on a file without a name excludes nobody who opens the name.
    #[test]
    fn a_lock_file_removed_before_it_was_locked_is_opened_again() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("repository.lock");
        let mut attempts = 0;
        let held = lock_named(&path, |file| {
            attempts += 1;
            if attempts == 1 {
                std::fs::remove_file(&path).unwrap();
            }
            try_shared(file)
        })
        .unwrap();
        assert_eq!(attempts, 2);
        assert!(names(&path, &held.0));
        // What a conversion now locks is the file the lease holds.
        let conversion = open_lock(&path).unwrap();
        assert!(conversion.try_lock().is_err());
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
