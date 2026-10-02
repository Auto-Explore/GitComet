use super::io::*;
use super::storage::{self, StoragePolicy};
use super::*;
use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const JOURNAL_LIMIT: usize = 100;

pub fn global() -> &'static Mutex<Filesystem> {
    static SERVICE: OnceLock<Mutex<Filesystem>> = OnceLock::new();
    SERVICE.get_or_init(|| Mutex::new(Filesystem::default()))
}

/// Static services are not dropped at process exit. Wait for the current
/// operation, reject later work, and release ordinary journal storage.
pub fn cleanup_on_shutdown() {
    global()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .shutdown();
}

/// Keep staging and undo areas in a locked per-process directory under
/// `root` (the app state dir), so they stay out of repository worktrees.
pub fn configure_journal_storage(root: &Path) -> io::Result<PathBuf> {
    global()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .configure_journal_storage(root)
}

/// Remove areas that crashed instances left under `root`; keeps parked data.
pub fn sweep_leaked_journal_storage(root: &Path) {
    storage::sweep(root);
}

#[derive(Default)]
pub struct Filesystem {
    shutting_down: bool,
    undo: VecDeque<JournalEntry>,
    redo: Vec<JournalEntry>,
    revision: u64,
    changes: VecDeque<(u64, Vec<PathChange>)>,
    storage: Arc<StoragePolicy>,
    instance_lock: Option<File>,
}

struct Step {
    from_parent: ParentIdentity,
    to_parent: ParentIdentity,
    from: PathBuf,
    to: PathBuf,
    version: DiskVersion,
    change: Option<PathChange>,
}

#[derive(PartialEq)]
struct ParentIdentity {
    entry: (u64, u64),
    repository: Option<PathBuf>,
}
impl ParentIdentity {
    fn read(path: &Path) -> io::Result<Self> {
        let parent = path
            .parent()
            .ok_or_else(|| invalid("Missing parent directory"))?;
        Ok(Self {
            entry: entry_identity(parent)?,
            repository: parent
                .ancestors()
                .find(|p| p.join(".git").symlink_metadata().is_ok())
                .map(Path::to_path_buf),
        })
    }
    fn check(&self, path: &Path) -> io::Result<()> {
        if Self::read(path)? == *self {
            Ok(())
        } else {
            Err(invalid(
                "A parent directory or repository boundary changed; current files were preserved",
            ))
        }
    }
}
impl Step {
    fn new(
        from: PathBuf,
        to: PathBuf,
        version: DiskVersion,
        change: Option<PathChange>,
    ) -> io::Result<Self> {
        Ok(Self {
            from_parent: ParentIdentity::read(&from)?,
            to_parent: ParentIdentity::read(&to)?,
            from,
            to,
            version,
            change,
        })
    }
}

#[derive(Default)]
struct JournalEntry {
    logical_id: Option<OperationId>,
    steps: Vec<Step>,
    applied: usize,
    areas: Vec<tempfile::TempDir>,
    storage: Arc<StoragePolicy>,
}

impl JournalEntry {
    fn new(storage: &Arc<StoragePolicy>, logical_id: Option<OperationId>) -> Self {
        Self {
            logical_id,
            storage: Arc::clone(storage),
            ..Self::default()
        }
    }

    fn require_manual_recovery(&mut self, required: bool) {
        // A failed plain rename can leave only an empty journal and its log.
        // Keep receipts only when some retained entry still needs restoration.
        let required = required
            && self
                .areas
                .iter()
                .any(|area| exists(&area.path().join("item")).unwrap_or(true));
        for area in &mut self.areas {
            area.disable_cleanup(required);
        }
    }

    fn reserve(&mut self, parent: &Path) -> io::Result<PathBuf> {
        let area = self.storage.reserve(parent)?;
        let path = absolute_identity(&area.path().join("item"))?;
        self.areas.push(area);
        Ok(path)
    }

    fn record_intent(&self, from: &Path, to: &Path) -> io::Result<()> {
        // Each directory stays on the same filesystem as the data it retains.
        // Write both native paths before moving anything, for crash recovery.
        let area = self
            .areas
            .first()
            .ok_or_else(|| invalid("Missing recovery directory"))?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(area.path().join("recovery.log"))?;
        // One write rather than `writeln!`'s syscall per fragment, and no
        // fsync: `rename_exclusive` is never followed by a directory fsync
        // either, so a power loss can lose the rename while keeping this line.
        // The log can therefore only ever over-report, which is the right
        // direction for something a human inspects by hand -- and an fsync per
        // item made a large drop wait on the disk once per file.
        file.write_all(format!("move\t{}\t{}\n", encoded_path(from), encoded_path(to)).as_bytes())
    }

    /// Move an entry whose current version the caller already holds. A rename
    /// preserves both contents and inode, so a version read at `from` still
    /// describes the entry once it sits at `to`.
    fn move_known(
        &mut self,
        from: PathBuf,
        to: PathBuf,
        version: DiskVersion,
        change: Option<PathChange>,
    ) -> io::Result<()> {
        self.record_intent(&from, &to)?;
        let step = Step::new(from, to, version, change)?;
        rename_exclusive(&step.from, &step.to)?;
        self.steps.push(step);
        self.applied = self.steps.len();
        Ok(())
    }

    fn move_entry(
        &mut self,
        from: PathBuf,
        to: PathBuf,
        change: Option<PathChange>,
    ) -> io::Result<()> {
        let version = DiskVersion::read(&from)?;
        self.move_known(from, to, version, change)
    }
}

impl Filesystem {
    #[cfg(test)]
    fn with_storage_candidates(candidates: Vec<PathBuf>) -> Self {
        Self {
            storage: Arc::new(StoragePolicy::with_candidates(candidates)),
            ..Self::default()
        }
    }

    pub fn configure_journal_storage(&mut self, root: &Path) -> io::Result<PathBuf> {
        let (dir, lock) = storage::create_instance(root)?;
        self.release_instance();
        self.storage = Arc::new(StoragePolicy::instance(dir.clone()));
        self.instance_lock = Some(lock);
        Ok(dir)
    }

    fn release_instance(&mut self) {
        if let Some(lock) = self.instance_lock.take() {
            drop(lock);
            if let Some(dir) = self.storage.instance_dir() {
                storage::release_instance(dir);
            }
        }
    }

    fn shutdown(&mut self) {
        self.shutting_down = true;
        self.undo.clear();
        self.redo.clear();
        self.release_instance();
    }

    pub fn prepare_outbound(
        &self,
        id: OperationId,
        paths: Vec<PathBuf>,
        cancellation: &Cancellation,
    ) -> io::Result<OutboundReceipt> {
        let paths = jobs(&Operation::Transfer {
            sources: paths,
            destination: std::env::temp_dir(),
            intent: TransferIntent::Copy,
        })?
        .into_iter()
        .map(|(path, _)| path)
        .collect::<Vec<_>>();
        let mut versions = Vec::new();
        let mut directories = Vec::new();
        for path in &paths {
            protect(path, true, cancellation)?;
            directories.push(fs::symlink_metadata(path)?.is_dir());
            versions.push(DiskVersion::read_cancellable(path, cancellation)?);
        }
        Ok(OutboundReceipt {
            id,
            paths,
            directories,
            versions,
        })
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn changes_since(&self, revision: u64) -> Vec<PathChange> {
        self.changes
            .iter()
            .filter(|(r, _)| *r > revision)
            .flat_map(|(_, changes)| changes.iter().cloned())
            .collect()
    }

    fn publish(&mut self, changes: &[PathChange]) {
        if !changes.is_empty() {
            self.revision = self.revision.wrapping_add(1);
            self.changes.push_back((self.revision, changes.to_vec()));
            // This stream includes undo/redo, unlike the bounded undo journal.
            // Retain it for the process lifetime so an idle window cannot miss
            // a rename and later autosave to an obsolete path.
        }
    }

    /// Atomic save under the same lock as transfers. `expected` is the disk
    /// version captured on load/last successful save, not on pressing Save.
    pub fn save(
        &mut self,
        path: &Path,
        bytes: &[u8],
        expected: Option<&DiskVersion>,
        overwrite: bool,
    ) -> io::Result<DiskVersion> {
        self.save_with_identity(path, bytes, expected, overwrite)
            .map(|(_, version)| version)
    }

    fn save_with_identity(
        &mut self,
        path: &Path,
        bytes: &[u8],
        expected: Option<&DiskVersion>,
        overwrite: bool,
    ) -> io::Result<(PathBuf, DiskVersion)> {
        if self.shutting_down {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "Filesystem service is shutting down",
            ));
        }
        let path = save_identity(path, overwrite)?;
        protect(&path, false, &Cancellation::default())?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        if metadata.as_ref().is_some_and(|m| !m.is_file()) {
            return Err(invalid("Only regular files can be edited"));
        }
        if !overwrite {
            match (expected, metadata.as_ref()) {
                (Some(expected), Some(_)) => expected.matches(&path, &Cancellation::default())?,
                (None, None) => {}
                _ => {
                    return Err(invalid(
                        "The file changed on disk. Reload, Save As, or explicitly replace it.",
                    ));
                }
            }
        }
        let original = metadata
            .as_ref()
            .map(|_| DiskVersion::read(&path))
            .transpose()?;
        let mut staged = tempfile::Builder::new()
            .prefix(crate::path_utils::SAVE_STAGING_PREFIX)
            .tempfile_in(path.parent().unwrap())?;
        staged.write_all(bytes)?;
        if let Some(m) = metadata {
            staged.as_file().set_permissions(m.permissions())?;
        }
        staged.as_file().sync_all()?;
        let version = DiskVersion::read(staged.path())?;
        let mut recovery = JournalEntry::new(&self.storage, None);
        if let Some(original) = original {
            let parked = recovery.reserve(path.parent().unwrap())?;
            recovery.record_intent(&path, &parked)?;
            rename_exclusive(&path, &parked)?;
            let install = original
                .matches(&parked, &Cancellation::default())
                .and_then(|_| {
                    if !overwrite && let Some(expected) = expected {
                        expected.matches(&parked, &Cancellation::default())?;
                    }
                    staged
                        .persist_noclobber(&path)
                        .map(|_| ())
                        .map_err(|e| e.error)
                });
            if let Err(error) = install {
                if let Err(restore) = rename_exclusive(&parked, &path) {
                    recovery.require_manual_recovery(true);
                    let location = recovery.areas[0].path();
                    return Err(io::Error::other(format!(
                        "Save stopped: {error}. The current file was preserved. The previous version is retained at {} (restore: {restore})",
                        location.display()
                    )));
                }
                return Err(error);
            }
        } else {
            staged.persist_noclobber(&path).map_err(|e| e.error)?;
        }
        self.publish(&[PathChange {
            old: None,
            new: Some(path.clone()),
        }]);
        Ok((path, version))
    }

    pub fn execute(
        &mut self,
        request: Request,
        mut progress: impl FnMut(Progress),
    ) -> OperationResult {
        if self.shutting_down {
            request.cancellation.cancel();
        }
        if matches!(request.operation, Operation::Undo | Operation::Redo) {
            return self.reverse(request);
        }
        if let Operation::CompleteOutbound {
            receipt,
            intent,
            source_removed,
        } = &request.operation
        {
            let mut result = OperationResult {
                moved_versions: BTreeMap::new(),
                id: request.id,
                saved_version: None,
                items: vec![],
                changes: vec![],
                undo_available: !self.undo.is_empty(),
                redo_available: !self.redo.is_empty(),
            };
            for (path, version) in receipt.paths.iter().zip(&receipt.versions) {
                let outcome = if *intent != Some(TransferIntent::Move) {
                    Ok(ItemOutcome::Skipped)
                } else if *source_removed || !exists(path).unwrap_or(true) {
                    Ok(ItemOutcome::Completed)
                } else {
                    complete_outbound_move(path, version, &request.cancellation, &self.storage)
                        .map(|_| ItemOutcome::Completed)
                };
                let outcome = outcome.unwrap_or_else(|e| ItemOutcome::Failed(e.to_string()));
                if matches!(outcome, ItemOutcome::Completed) && !exists(path).unwrap_or(true) {
                    result.changes.push(PathChange {
                        old: Some(path.clone()),
                        new: None,
                    });
                }
                result.items.push(ItemResult {
                    source: path.clone(),
                    destination: None,
                    outcome,
                });
            }
            self.publish(&result.changes);
            return result;
        }
        if let Operation::Save {
            path,
            worktree,
            contents,
            expected,
            overwrite,
        } = &request.operation
        {
            progress(Progress {
                id: request.id,
                completed_items: 0,
                total_items: 1,
                current_path: path.clone(),
            });
            let saved = check_cancel(&request.cancellation).and_then(|_| {
                let target = if let Some(root) = worktree {
                    let relative = path
                        .strip_prefix(root)
                        .map_err(|_| invalid("Save path is outside the repository worktree"))?;
                    let relative = crate::path_utils::validated_repo_relative_path(relative)?;
                    crate::path_utils::symlink_free_write_target(root, &relative)?
                } else {
                    path.clone()
                };
                self.save_with_identity(&target, contents, expected.as_ref(), *overwrite)
            });
            let (target, outcome, saved_version, changes) = match saved {
                Ok((target, version)) => (
                    target.clone(),
                    ItemOutcome::Completed,
                    Some(version),
                    vec![PathChange {
                        old: None,
                        new: Some(target),
                    }],
                ),
                Err(error) => (
                    path.clone(),
                    ItemOutcome::Failed(error.to_string()),
                    None,
                    vec![],
                ),
            };
            progress(Progress {
                id: request.id,
                completed_items: 1,
                total_items: 1,
                current_path: path.clone(),
            });
            return OperationResult {
                moved_versions: BTreeMap::new(),
                id: request.id,
                saved_version,
                changes,
                items: vec![ItemResult {
                    source: target.clone(),
                    destination: Some(target),
                    outcome,
                }],
                undo_available: !self.undo.is_empty(),
                redo_available: !self.redo.is_empty(),
            };
        }
        let mut entry = JournalEntry::new(&self.storage, Some(request.logical_id));
        let mut result = OperationResult {
            moved_versions: BTreeMap::new(),
            saved_version: None,
            id: request.id,
            items: vec![],
            changes: vec![],
            undo_available: false,
            redo_available: false,
        };
        let jobs = match jobs(&request.operation) {
            Ok(jobs) => jobs,
            Err(error) => {
                result.items.push(ItemResult {
                    source: PathBuf::new(),
                    destination: None,
                    outcome: ItemOutcome::Failed(error.to_string()),
                });
                result.undo_available = !self.undo.is_empty();
                result.redo_available = !self.redo.is_empty();
                return result;
            }
        };
        let total_items = jobs.len();
        for (index, (source, destination)) in jobs.into_iter().enumerate() {
            progress(Progress {
                id: request.id,
                completed_items: index,
                total_items,
                current_path: source.clone(),
            });
            let start = entry.steps.len();
            let outcome = if request.cancellation.is_cancelled() {
                Ok(ItemOutcome::Cancelled)
            } else {
                self.execute_item(&request, &source, destination.as_deref(), &mut entry)
            };
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => ItemOutcome::Cancelled,
                Err(error) => ItemOutcome::Failed(error.to_string()),
            };
            for step in &entry.steps[start..] {
                if let Some(change) = &step.change {
                    result.changes.push(change.clone());
                }
            }
            if matches!(outcome, ItemOutcome::Completed)
                && matches!(request.operation, Operation::DeletePermanently { .. })
            {
                result.changes.push(PathChange {
                    old: Some(source.clone()),
                    new: None,
                });
            }
            let destination = entry.steps[start..]
                .iter()
                .rev()
                .find_map(|s| s.change.as_ref().and_then(|c| c.new.clone()))
                .or(destination);
            result.items.push(ItemResult {
                source,
                destination,
                outcome,
            });
        }
        progress(Progress {
            id: request.id,
            completed_items: total_items,
            total_items,
            current_path: PathBuf::new(),
        });
        if !entry.steps.is_empty() && !request.native_source_move {
            self.redo.clear();
            if let Some(index) = self
                .undo
                .iter()
                .position(|previous| previous.logical_id == entry.logical_id)
            {
                let mut previous = self.undo.remove(index).unwrap();
                previous.steps.extend(entry.steps);
                previous.areas.extend(entry.areas);
                previous.applied = previous.steps.len();
                entry = previous;
            }
            self.undo.push_back(entry);
            if self.undo.len() > JOURNAL_LIMIT {
                self.undo.pop_front();
            }
        }
        result.moved_versions = moved_versions(&result.changes, &request.cancellation);
        self.publish(&result.changes);
        result.undo_available = !self.undo.is_empty();
        result.redo_available = !self.redo.is_empty();
        result
    }

    fn execute_item(
        &mut self,
        request: &Request,
        source: &Path,
        destination: Option<&Path>,
        journal: &mut JournalEntry,
    ) -> io::Result<ItemOutcome> {
        protect(source, true, &request.cancellation)?;
        match &request.operation {
            Operation::CreateFile { .. } | Operation::CreateDirectory { .. } => {
                if exists(source)? {
                    return Err(io::Error::new(
                        io::ErrorKind::AlreadyExists,
                        "That name already exists",
                    ));
                }
                let staged = journal.reserve(source.parent().unwrap())?;
                if matches!(request.operation, Operation::CreateDirectory { .. }) {
                    fs::create_dir(&staged)?;
                } else {
                    File::create_new(&staged)?.sync_all()?;
                }
                journal.move_entry(
                    staged,
                    source.to_path_buf(),
                    Some(PathChange {
                        old: None,
                        new: Some(source.to_path_buf()),
                    }),
                )?;
            }
            Operation::Duplicate { .. } | Operation::Transfer { .. } | Operation::Rename { .. } => {
                let intent = match request.operation {
                    Operation::Transfer { intent, .. } => intent,
                    Operation::Rename { .. } => TransferIntent::Move,
                    _ => TransferIntent::Copy,
                };
                return transfer(source, destination.unwrap(), intent, request, journal);
            }
            Operation::Trash { .. } => {
                #[cfg(any(windows, target_os = "macos"))]
                return native_trash(source, request, journal);
                #[cfg(not(any(windows, target_os = "macos")))]
                {
                    let receipt = super::trash::prepare(source)?;
                    return trash_with_receipt(source, &receipt, request, journal);
                }
            }
            Operation::DeletePermanently { confirmed, .. } => {
                if !confirmed {
                    return Err(invalid("Permanent deletion requires confirmation"));
                }
                let version = DiskVersion::read_cancellable(source, &request.cancellation)?;
                complete_outbound_move(source, &version, &request.cancellation, &self.storage)?;
                // Permanent removal is reported but never enters the journal.
                self.publish(&[PathChange {
                    old: Some(source.to_path_buf()),
                    new: None,
                }]);
            }
            Operation::Save { .. }
            | Operation::Undo
            | Operation::Redo
            | Operation::CompleteOutbound { .. } => {
                unreachable!()
            }
        }
        Ok(ItemOutcome::Completed)
    }

    fn reverse(&mut self, request: Request) -> OperationResult {
        let redo = matches!(request.operation, Operation::Redo);
        let entry = if redo {
            self.redo.pop()
        } else {
            self.undo.pop_back()
        };
        let mut result = OperationResult {
            moved_versions: BTreeMap::new(),
            saved_version: None,
            id: request.id,
            items: vec![],
            changes: vec![],
            undo_available: false,
            redo_available: false,
        };
        if let Some(mut entry) = entry {
            loop {
                let index = if redo {
                    if entry.applied == entry.steps.len() {
                        break;
                    }
                    entry.applied
                } else {
                    if entry.applied == 0 {
                        break;
                    }
                    entry.applied - 1
                };
                let step = &entry.steps[index];
                let (from, to) = if redo {
                    (&step.from, &step.to)
                } else {
                    (&step.to, &step.from)
                };
                let reverse = || -> io::Result<()> {
                    check_cancel(&request.cancellation)?;
                    step.version.matches(from, &request.cancellation)?;
                    step.from_parent.check(&step.from)?;
                    step.to_parent.check(&step.to)?;
                    if exists(to)? {
                        return Err(invalid(format!(
                            "{} already exists; it was preserved",
                            to.display()
                        )));
                    }
                    // Never create a parent here: an external move/deletion of
                    // a parent invalidates the receipt instead of recreating it.
                    if absolute_identity(from)? != *from || absolute_identity(to)? != *to {
                        return Err(invalid("A parent directory has changed"));
                    }
                    entry.record_intent(from, to)?;
                    rename_exclusive(from, to)
                };
                match reverse() {
                    Ok(()) => {
                        if let Some(change) = &step.change {
                            result.changes.push(if redo {
                                change.clone()
                            } else {
                                PathChange {
                                    old: change.new.clone(),
                                    new: change.old.clone(),
                                }
                            });
                        }
                        result.items.push(ItemResult {
                            source: from.clone(),
                            destination: Some(to.clone()),
                            outcome: ItemOutcome::Completed,
                        });
                        if redo {
                            entry.applied += 1;
                        } else {
                            entry.applied -= 1;
                        }
                    }
                    Err(error) => {
                        let needs_recovery = error.kind() != io::ErrorKind::Interrupted
                            || (entry.applied != 0 && entry.applied != entry.steps.len());
                        result.items.push(ItemResult {
                            source: from.clone(),
                            destination: Some(to.clone()),
                            outcome: ItemOutcome::Failed(error.to_string()),
                        });
                        if needs_recovery {
                            entry.require_manual_recovery(true);
                        }
                        break;
                    }
                }
            }
            if result.succeeded() {
                entry.require_manual_recovery(false);
            }
            if redo {
                if entry.applied == entry.steps.len() {
                    self.undo.push_back(entry);
                } else {
                    self.redo.push(entry);
                }
            } else if entry.applied == 0 {
                self.redo.push(entry);
            } else {
                self.undo.push_back(entry);
            }
        }
        result.moved_versions = moved_versions(&result.changes, &request.cancellation);
        self.publish(&result.changes);
        result.undo_available = !self.undo.is_empty();
        result.redo_available = !self.redo.is_empty();
        result
    }
}

fn complete_outbound_move(
    path: &Path,
    version: &DiskVersion,
    cancellation: &Cancellation,
    storage: &Arc<StoragePolicy>,
) -> io::Result<()> {
    check_cancel(cancellation)?;
    version.matches(path, cancellation)?;
    protect(path, true, cancellation)?;
    let mut recovery = JournalEntry::new(storage, None);
    let parked = recovery.reserve(path.parent().unwrap())?;
    recovery.record_intent(path, &parked)?;
    rename_exclusive(path, &parked)?;
    if let Err(error) = version
        .matches(&parked, cancellation)
        // The service may retain data beneath .git on this volume. Validate
        // the retained entry and its descendants, not that trusted ancestor.
        .and_then(|_| protect_contents(&parked, true, cancellation))
    {
        if let Err(restore) = rename_exclusive(&parked, path) {
            recovery.require_manual_recovery(true);
            return Err(io::Error::other(format!(
                "Transfer cleanup stopped: {error}. Restore failed: {restore}. Source retained at {}",
                parked.display()
            )));
        }
        return Err(error);
    }
    // No undo entry: the receiving application owns the completed transfer.
    // If cleanup is interrupted, retain the remaining bytes and receipt.
    if let Err(error) = remove_tree(&parked) {
        recovery.require_manual_recovery(true);
        return Err(io::Error::other(format!(
            "Transfer succeeded, but source cleanup failed: {error}. Remaining source data: {}",
            parked.display()
        )));
    }
    Ok(())
}

/// Versions of everything a move landed, so open editors can re-adopt a
/// baseline. Cancellable, and a partial map is safe: callers look each path up
/// individually and keep their existing version when one is missing.
fn moved_versions(
    changes: &[PathChange],
    cancellation: &Cancellation,
) -> BTreeMap<PathBuf, DiskVersion> {
    let mut versions = BTreeMap::new();
    let mut pending: Vec<_> = changes
        .iter()
        .filter(|c| c.old.is_some())
        .filter_map(|c| c.new.clone())
        .collect();
    while let Some(path) = pending.pop() {
        if check_cancel(cancellation).is_err() {
            break;
        }
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            if let Ok(entries) = children(&path) {
                pending.extend(entries);
            }
        } else if metadata.is_file()
            && !versions.contains_key(&path)
            && let Ok(version) = DiskVersion::read_cancellable(&path, cancellation)
        {
            versions.insert(path, version);
        }
    }
    versions
}

#[cfg(any(not(any(windows, target_os = "macos")), all(test, unix)))]
fn trash_with_receipt(
    source: &Path,
    receipt: &super::trash::Receipt,
    request: &Request,
    journal: &mut JournalEntry,
) -> io::Result<ItemOutcome> {
    let staged_info = journal.reserve(receipt.info.parent().unwrap())?;
    // Info is already reserved atomically in Trash. Undo parks it here after
    // restoring the original; Redo reinstalls the same receipt.
    journal.steps.push(Step::new(
        staged_info,
        receipt.info.clone(),
        DiskVersion::read(&receipt.info)?,
        None,
    )?);
    journal.applied = journal.steps.len();
    let outcome = transfer(
        source,
        &receipt.item,
        TransferIntent::Move,
        request,
        journal,
    )?;
    if matches!(outcome, ItemOutcome::Completed)
        && let Some(last) = journal.steps.last_mut()
    {
        last.change = Some(PathChange {
            old: Some(source.to_path_buf()),
            new: None,
        });
    }
    Ok(outcome)
}

#[cfg(any(windows, target_os = "macos"))]
fn native_trash(
    source: &Path,
    request: &Request,
    journal: &mut JournalEntry,
) -> io::Result<ItemOutcome> {
    let backup = journal.reserve(source.parent().unwrap())?;
    let original = DiskVersion::read_cancellable(source, &request.cancellation)?;
    copy_tree(source, &backup, &request.cancellation)?;
    original.matches(source, &request.cancellation)?;
    if !original.same_contents(&DiskVersion::read(&backup)?) {
        return Err(invalid("File changed while preparing Trash"));
    }
    // Record the original pathname and a complete recovery copy before the
    // native call. Some platforms may return an error after moving the item.
    journal.record_intent(source, &backup)?;
    check_cancel(&request.cancellation)?;
    let result = (|| {
        let mut receipt = gitcomet_filesystem_native::trash_item(source)?;
        receipt.item = absolute_identity(&receipt.item)?;
        receipt.info = receipt.info.as_deref().map(absolute_identity).transpose()?;
        let version = DiskVersion::read(&receipt.item)?;
        if exists(source)? {
            return Err(invalid(
                "The native Trash operation did not move the source",
            ));
        }
        let info = receipt
            .info
            .map(|info| {
                let parked = journal.reserve(info.parent().unwrap())?;
                let version = DiskVersion::read(&info)?;
                Step::new(parked, info, version, None)
            })
            .transpose()?;
        journal.record_intent(source, &receipt.item)?;
        let step = Step::new(
            source.to_path_buf(),
            receipt.item,
            version,
            Some(PathChange {
                old: Some(source.to_path_buf()),
                new: None,
            }),
        )?;
        if let Some(info) = info {
            journal.steps.push(info);
        }
        journal.steps.push(step);
        journal.applied = journal.steps.len();
        Ok(ItemOutcome::Completed)
    })();
    if let Err(error) = result {
        match exists(source) {
            Ok(false) => {
                if let Err(restore) = rename_exclusive(&backup, source) {
                    journal.require_manual_recovery(true);
                    return Err(io::Error::other(format!(
                        "Trash failed: {error}. Recovery copy: {} (restore failed: {restore})",
                        backup.display()
                    )));
                }
            }
            Err(_) => {
                journal.require_manual_recovery(true);
                return Err(io::Error::other(format!(
                    "Trash failed: {error}. Recovery copy: {}",
                    backup.display()
                )));
            }
            Ok(true) if original.matches(source, &request.cancellation).is_ok() => {}
            Ok(true) => {
                journal.require_manual_recovery(true);
                return Err(io::Error::other(format!(
                    "Trash failed: {error}. A new item appeared at the original path and was preserved. Recovery copy: {}",
                    backup.display()
                )));
            }
        }
        return Err(error);
    }
    result
}

fn jobs(operation: &Operation) -> io::Result<Vec<(PathBuf, Option<PathBuf>)>> {
    match operation {
        Operation::Save { path, .. }
        | Operation::CreateFile { path }
        | Operation::CreateDirectory { path } => Ok(vec![(absolute_identity(path)?, None)]),
        Operation::Rename { source, name } => {
            validate_name(name)?;
            let source = absolute_identity(source)?;
            Ok(vec![(source.clone(), Some(source.with_file_name(name)))])
        }
        Operation::Undo | Operation::Redo => Ok(vec![]),
        _ => {
            let mut sources = operation
                .sources()
                .iter()
                .map(|p| absolute_identity(p))
                .collect::<io::Result<Vec<_>>>()?;
            // Parent wins over descendants, irrespective of click order.
            let all = sources.clone();
            let mut seen = std::collections::HashSet::new();
            sources.retain(|p| {
                seen.insert(p.clone())
                    && !all
                        .iter()
                        .any(|parent| parent != p && p.starts_with(parent))
            });
            let destination = match operation {
                Operation::Transfer { destination, .. } => {
                    let destination = canonical_path(destination)?;
                    if !destination.is_dir() {
                        return Err(invalid("The destination must be a folder"));
                    }
                    if destination
                        .components()
                        .any(|c| c.as_os_str().eq_ignore_ascii_case(".git"))
                    {
                        return Err(invalid("Git metadata is protected"));
                    }
                    Some(destination)
                }
                _ => None,
            };
            sources
                .into_iter()
                .map(|source| {
                    let target = if let Some(destination) = &destination {
                        Some(destination.join(source.file_name().unwrap()))
                    } else if matches!(operation, Operation::Duplicate { .. }) {
                        Some(copy_name(&source)?)
                    } else {
                        None
                    };
                    Ok((source, target))
                })
                .collect()
        }
    }
}

fn transfer(
    source: &Path,
    destination: &Path,
    intent: TransferIntent,
    request: &Request,
    journal: &mut JournalEntry,
) -> io::Result<ItemOutcome> {
    let start = journal.steps.len();
    match transfer_inner(source, destination, intent, request, journal) {
        Ok(outcome) => Ok(outcome),
        Err(error) => {
            // Restore any destination parked by this item before returning a
            // failure. A competing change blocks rollback and retains the
            // exact remaining steps in the journal instead of deleting data.
            while journal.steps.len() > start {
                let step = journal.steps.last().unwrap();
                let rollback = step
                    .version
                    // Deliberately uncancellable: this is the rollback, and abandoning
                    // it halfway strands parked data in a temporary directory.
                    .matches(&step.to, &Cancellation::default())
                    .and_then(|_| journal.record_intent(&step.to, &step.from))
                    .and_then(|_| rename_exclusive(&step.to, &step.from));
                if let Err(restore) = rollback {
                    journal.require_manual_recovery(true);
                    return Err(io::Error::other(format!(
                        "{error}. Rollback stopped: {restore}. Recovery data is retained at {}",
                        journal.areas.first().unwrap().path().display()
                    )));
                }
                journal.steps.pop();
                journal.applied = journal.steps.len();
            }
            Err(error)
        }
    }
}

fn transfer_inner(
    source: &Path,
    destination: &Path,
    intent: TransferIntent,
    request: &Request,
    journal: &mut JournalEntry,
) -> io::Result<ItemOutcome> {
    check_cancel(&request.cancellation)?;
    let mut destination = destination.to_path_buf();
    if source == destination {
        if intent == TransferIntent::Move {
            return Ok(ItemOutcome::Skipped);
        }
        destination = copy_name(source)?;
    }
    if destination.starts_with(source) {
        return Err(invalid(
            "A folder cannot be transferred into itself or a descendant",
        ));
    }
    protect(&destination, true, &request.cancellation)?;
    if exists(&destination)? {
        let version = DiskVersion::read_cancellable(&destination, &request.cancellation)?;
        // Case-insensitive volumes resolve the new spelling to the same entry.
        // Use an intermediate name so the directory entry adopts that spelling.
        if intent == TransferIntent::Move
            && source.parent() == destination.parent()
            && version == DiskVersion::read_cancellable(source, &request.cancellation)?
            && !children(destination.parent().unwrap())?
                .iter()
                .any(|entry| entry.file_name() == destination.file_name())
        {
            let parked = journal.reserve(source.parent().unwrap())?;
            journal.move_known(source.to_path_buf(), parked.clone(), version.clone(), None)?;
            journal.move_known(
                parked,
                destination.clone(),
                version,
                Some(PathChange {
                    old: Some(source.to_path_buf()),
                    new: Some(destination),
                }),
            )?;
            return Ok(ItemOutcome::Completed);
        }
        let can_merge =
            fs::symlink_metadata(source)?.is_dir() && fs::symlink_metadata(&destination)?.is_dir();
        check_cancel(&request.cancellation)?;
        let Some(resolution) = request
            .resolutions
            .get(&destination)
            .filter(|r| r.expected == version)
        else {
            return Ok(ItemOutcome::Conflict(Conflict {
                source: source.to_path_buf(),
                destination,
                version,
                can_merge,
                continuation: None,
            }));
        };
        match resolution.choice {
            ConflictChoice::Cancel => {
                request.cancellation.cancel();
                return Ok(ItemOutcome::Cancelled);
            }
            ConflictChoice::Skip => return Ok(ItemOutcome::Skipped),
            ConflictChoice::KeepBoth => destination = copy_name(&destination)?,
            ConflictChoice::Merge if can_merge => {
                let mut continuation = request.clone();
                for child in children(source)? {
                    let target = destination.join(child.file_name().unwrap());
                    let result = transfer(&child, &target, intent, &continuation, journal)?;
                    match result {
                        ItemOutcome::Conflict(mut conflict) => {
                            let next = conflict
                                .continuation
                                .get_or_insert_with(|| Box::new(continuation.clone()));
                            next.resolutions.insert(
                                destination.clone(),
                                ConflictResolution {
                                    expected: DiskVersion::read_cancellable(
                                        &destination,
                                        &request.cancellation,
                                    )?,
                                    choice: ConflictChoice::Merge,
                                },
                            );
                            return Ok(ItemOutcome::Conflict(conflict));
                        }
                        ItemOutcome::Completed | ItemOutcome::Skipped => {
                            if intent == TransferIntent::Copy {
                                continuation.resolutions.insert(
                                    target.clone(),
                                    ConflictResolution {
                                        expected: DiskVersion::read_cancellable(
                                            &target,
                                            &request.cancellation,
                                        )?,
                                        choice: ConflictChoice::Skip,
                                    },
                                );
                            }
                        }
                        other => return Ok(other),
                    }
                }
                if intent == TransferIntent::Move && children(source)?.is_empty() {
                    let parked = journal.reserve(source.parent().unwrap())?;
                    journal.move_entry(
                        source.to_path_buf(),
                        parked,
                        Some(PathChange {
                            old: Some(source.to_path_buf()),
                            new: None,
                        }),
                    )?;
                }
                return Ok(ItemOutcome::Completed);
            }
            ConflictChoice::Merge => return Err(invalid("Only two directories can be merged")),
            ConflictChoice::Replace => {}
        }
    }
    let replacing = exists(&destination)?;
    // Complete a copy before parking anything at its destination. A cancelled
    // or failed copy must not disturb the previous destination or the source.
    let staged = if intent == TransferIntent::Copy || replacing {
        let before = DiskVersion::read_cancellable(source, &request.cancellation)?;
        let staged = journal.reserve(destination.parent().unwrap())?;
        copy_tree(source, &staged, &request.cancellation)?;
        before.matches(source, &request.cancellation)?;
        let staged_version = DiskVersion::read_cancellable(&staged, &request.cancellation)?;
        if !before.same_contents(&staged_version) {
            return Err(invalid("Source changed while copying"));
        }
        Some((staged, before, staged_version))
    } else {
        None
    };
    if replacing {
        // Re-check the version shown in the prompt immediately before parking.
        let expected = &request
            .resolutions
            .get(&destination)
            .ok_or_else(|| invalid("Destination changed while copying"))?
            .expected;
        expected.matches(&destination, &request.cancellation)?;
        let parked = journal.reserve(destination.parent().unwrap())?;
        journal.move_known(destination.clone(), parked, expected.clone(), None)?;
    }
    let change = PathChange {
        old: (intent == TransferIntent::Move).then(|| source.to_path_buf()),
        new: Some(destination.clone()),
    };
    if let Some((staged, before, staged_version)) = staged {
        journal.move_known(
            staged,
            destination,
            staged_version,
            (intent == TransferIntent::Copy).then(|| change.clone()),
        )?;
        if intent == TransferIntent::Move {
            let parked = journal.reserve(source.parent().unwrap())?;
            journal.move_known(
                source.to_path_buf(),
                parked.clone(),
                before.clone(),
                Some(change),
            )?;
            before.matches(&parked, &request.cancellation)?;
        }
    } else {
        // Read once, up here: the plain rename needs it, and so does the
        // cross-device fallback, which used to read the whole tree again after
        // `move_entry` had already read and discarded it.
        let before = DiskVersion::read_cancellable(source, &request.cancellation)?;
        if journal.areas.is_empty() {
            journal.reserve(source.parent().unwrap())?;
        }
        match journal.move_known(
            source.to_path_buf(),
            destination.clone(),
            before.clone(),
            Some(change.clone()),
        ) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                let staged = journal.reserve(destination.parent().unwrap())?;
                copy_tree(source, &staged, &request.cancellation)?;
                before.matches(source, &request.cancellation)?;
                let staged_version = DiskVersion::read_cancellable(&staged, &request.cancellation)?;
                if !before.same_contents(&staged_version) {
                    return Err(invalid("Source changed while copying"));
                }
                journal.move_known(staged, destination, staged_version, None)?;
                let parked = journal.reserve(source.parent().unwrap())?;
                journal.move_known(
                    source.to_path_buf(),
                    parked.clone(),
                    before.clone(),
                    Some(change),
                )?;
                before.matches(&parked, &request.cancellation)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(ItemOutcome::Completed)
}

#[cfg(test)]
mod tests;
