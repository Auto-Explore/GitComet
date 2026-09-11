use super::io::*;
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

#[derive(Default)]
pub struct Filesystem {
    undo: VecDeque<JournalEntry>,
    redo: Vec<JournalEntry>,
    revision: u64,
    changes: VecDeque<(u64, Vec<PathChange>)>,
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
}

impl JournalEntry {
    fn reserve(&mut self, parent: &Path) -> io::Result<PathBuf> {
        let same_volume = |candidate: &Path| {
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
        };
        let temporary = std::env::temp_dir();
        let git_directory = parent
            .ancestors()
            .map(|p| p.join(".git"))
            .find(|p| p.is_dir() && same_volume(p));
        let storage = if same_volume(&temporary) {
            temporary.as_path()
        } else {
            git_directory.as_deref().unwrap_or(parent)
        };
        let area = tempfile::Builder::new()
            .prefix(".gitcomet-operation-")
            .tempdir_in(storage)?;
        let path = area.path().join("item");
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

    fn move_entry(
        &mut self,
        from: PathBuf,
        to: PathBuf,
        change: Option<PathChange>,
    ) -> io::Result<()> {
        let version = DiskVersion::read(&from)?;
        self.record_intent(&from, &to)?;
        let step = Step::new(from, to, version, change)?;
        rename_exclusive(&step.from, &step.to)?;
        self.steps.push(step);
        self.applied = self.steps.len();
        Ok(())
    }
}

impl Filesystem {
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
        let path = absolute_identity(path)?;
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
                (Some(expected), Some(_)) => expected.matches(&path)?,
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
            .prefix(".gitcomet-save-")
            .tempfile_in(path.parent().unwrap())?;
        staged.write_all(bytes)?;
        if let Some(m) = metadata {
            staged.as_file().set_permissions(m.permissions())?;
        }
        staged.as_file().sync_all()?;
        let version = DiskVersion::read(staged.path())?;
        let mut recovery = JournalEntry::default();
        if let Some(original) = original {
            let parked = recovery.reserve(path.parent().unwrap())?;
            recovery.record_intent(&path, &parked)?;
            rename_exclusive(&path, &parked)?;
            let install = original.matches(&parked).and_then(|_| {
                if !overwrite && let Some(expected) = expected {
                    expected.matches(&parked)?;
                }
                staged
                    .persist_noclobber(&path)
                    .map(|_| ())
                    .map_err(|e| e.error)
            });
            if let Err(error) = install {
                if let Err(restore) = rename_exclusive(&parked, &path) {
                    let location = recovery.areas.remove(0).keep();
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
            new: Some(path),
        }]);
        Ok(version)
    }

    pub fn execute(
        &mut self,
        request: Request,
        mut progress: impl FnMut(Progress),
    ) -> OperationResult {
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
                    complete_outbound_move(path, version, &request.cancellation)
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
            let saved = check_cancel(&request.cancellation)
                .and_then(|_| self.save(path, contents, expected.as_ref(), *overwrite));
            let (outcome, saved_version, changes) = match saved {
                Ok(version) => (
                    ItemOutcome::Completed,
                    Some(version),
                    vec![PathChange {
                        old: None,
                        new: Some(path.clone()),
                    }],
                ),
                Err(error) => (ItemOutcome::Failed(error.to_string()), None, vec![]),
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
                    source: path.clone(),
                    destination: Some(path.clone()),
                    outcome,
                }],
                undo_available: !self.undo.is_empty(),
                redo_available: !self.redo.is_empty(),
            };
        }
        let mut entry = JournalEntry {
            logical_id: Some(request.logical_id),
            ..Default::default()
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
            if matches!(outcome, ItemOutcome::Completed) {
                match &request.operation {
                    Operation::Save { .. } => {
                        result.saved_version = DiskVersion::read(&source).ok();
                        result.changes.push(PathChange {
                            old: None,
                            new: Some(source.clone()),
                        });
                    }
                    Operation::DeletePermanently { .. } => result.changes.push(PathChange {
                        old: Some(source.clone()),
                        new: None,
                    }),
                    _ => {}
                }
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
        result.moved_versions = moved_versions(&result.changes);
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
            Operation::Save {
                contents,
                expected,
                overwrite,
                ..
            } => {
                self.save(source, contents, expected.as_ref(), *overwrite)?;
            }
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
                    let staged_info = journal.reserve(receipt.info.parent().unwrap())?;
                    // Info is already reserved atomically in the native trash. Its
                    // inverse parks it here after the original has been restored.
                    journal.steps.push(Step::new(
                        staged_info,
                        receipt.info.clone(),
                        DiskVersion::read(&receipt.info)?,
                        None,
                    )?);
                    journal.applied = journal.steps.len();
                    return transfer(
                        source,
                        &receipt.item,
                        TransferIntent::Move,
                        request,
                        journal,
                    )
                    .map(|outcome| {
                        if matches!(outcome, ItemOutcome::Completed)
                            && let Some(last) = journal.steps.last_mut()
                        {
                            last.change = Some(PathChange {
                                old: Some(source.to_path_buf()),
                                new: None,
                            });
                        }
                        outcome
                    });
                }
            }
            Operation::DeletePermanently { confirmed, .. } => {
                if !confirmed {
                    return Err(invalid("Permanent deletion requires confirmation"));
                }
                let version = DiskVersion::read_cancellable(source, &request.cancellation)?;
                complete_outbound_move(source, &version, &request.cancellation)?;
                // Permanent removal is reported but never enters the journal.
                self.publish(&[PathChange {
                    old: Some(source.to_path_buf()),
                    new: None,
                }]);
            }
            Operation::Undo | Operation::Redo | Operation::CompleteOutbound { .. } => {
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
                    step.version.matches(from)?;
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
                        result.items.push(ItemResult {
                            source: from.clone(),
                            destination: Some(to.clone()),
                            outcome: ItemOutcome::Failed(error.to_string()),
                        });
                        break;
                    }
                }
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
        result.moved_versions = moved_versions(&result.changes);
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
) -> io::Result<()> {
    check_cancel(cancellation)?;
    version.matches(path)?;
    protect(path, true, cancellation)?;
    let mut recovery = JournalEntry::default();
    let parked = recovery.reserve(path.parent().unwrap())?;
    recovery.record_intent(path, &parked)?;
    rename_exclusive(path, &parked)?;
    if let Err(error) = version
        .matches(&parked)
        .and_then(|_| protect(&parked, true, cancellation))
    {
        if let Err(restore) = rename_exclusive(&parked, path) {
            for area in recovery.areas {
                let _ = area.keep();
            }
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
        for area in recovery.areas {
            let _ = area.keep();
        }
        return Err(io::Error::other(format!(
            "Transfer succeeded, but source cleanup failed: {error}. Remaining source data: {}",
            parked.display()
        )));
    }
    Ok(())
}

fn moved_versions(changes: &[PathChange]) -> BTreeMap<PathBuf, DiskVersion> {
    let mut versions = BTreeMap::new();
    let mut pending: Vec<_> = changes
        .iter()
        .filter(|c| c.old.is_some())
        .filter_map(|c| c.new.clone())
        .collect();
    while let Some(path) = pending.pop() {
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            if let Ok(entries) = children(&path) {
                pending.extend(entries);
            }
        } else if metadata.is_file()
            && !versions.contains_key(&path)
            && let Ok(version) = DiskVersion::read(&path)
        {
            versions.insert(path, version);
        }
    }
    versions
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
    original.matches(source)?;
    if !original.same_contents(&DiskVersion::read(&backup)?) {
        return Err(invalid("File changed while preparing Trash"));
    }
    // Record the original pathname and a complete recovery copy before the
    // native call. Some platforms may return an error after moving the item.
    journal.record_intent(source, &backup)?;
    check_cancel(&request.cancellation)?;
    let result = (|| {
        let receipt = gitcomet_filesystem_native::trash_item(source)?;
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
                    for area in std::mem::take(&mut journal.areas) {
                        let _ = area.keep();
                    }
                    return Err(io::Error::other(format!(
                        "Trash failed: {error}. Recovery copy: {} (restore failed: {restore})",
                        backup.display()
                    )));
                }
            }
            Err(_) => {
                for area in std::mem::take(&mut journal.areas) {
                    let _ = area.keep();
                }
                return Err(io::Error::other(format!(
                    "Trash failed: {error}. Recovery copy: {}",
                    backup.display()
                )));
            }
            Ok(true) if original.matches(source).is_ok() => {}
            Ok(true) => {
                for area in std::mem::take(&mut journal.areas) {
                    let _ = area.keep();
                }
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
                    let destination = fs::canonicalize(destination)?;
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
                    .matches(&step.to)
                    .and_then(|_| journal.record_intent(&step.to, &step.from))
                    .and_then(|_| rename_exclusive(&step.to, &step.from));
                if let Err(restore) = rollback {
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
        let version = DiskVersion::read(&destination)?;
        // Case-insensitive volumes resolve the new spelling to the same entry.
        // Use an intermediate name so the directory entry adopts that spelling.
        if intent == TransferIntent::Move
            && source.parent() == destination.parent()
            && version == DiskVersion::read(source)?
            && !children(destination.parent().unwrap())?
                .iter()
                .any(|entry| entry.file_name() == destination.file_name())
        {
            let parked = journal.reserve(source.parent().unwrap())?;
            journal.move_entry(source.to_path_buf(), parked.clone(), None)?;
            journal.move_entry(
                parked,
                destination.clone(),
                Some(PathChange {
                    old: Some(source.to_path_buf()),
                    new: Some(destination),
                }),
            )?;
            return Ok(ItemOutcome::Completed);
        }
        let can_merge =
            fs::symlink_metadata(source)?.is_dir() && fs::symlink_metadata(&destination)?.is_dir();
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
                                    expected: DiskVersion::read(&destination)?,
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
                                        expected: DiskVersion::read(&target)?,
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
        before.matches(source)?;
        if !before.same_contents(&DiskVersion::read_cancellable(
            &staged,
            &request.cancellation,
        )?) {
            return Err(invalid("Source changed while copying"));
        }
        Some((staged, before))
    } else {
        None
    };
    if replacing {
        // Re-check the version shown in the prompt immediately before parking.
        request
            .resolutions
            .get(&destination)
            .ok_or_else(|| invalid("Destination changed while copying"))?
            .expected
            .matches(&destination)?;
        let parked = journal.reserve(destination.parent().unwrap())?;
        journal.move_entry(destination.clone(), parked, None)?;
    }
    let change = PathChange {
        old: (intent == TransferIntent::Move).then(|| source.to_path_buf()),
        new: Some(destination.clone()),
    };
    if let Some((staged, before)) = staged {
        journal.move_entry(
            staged,
            destination,
            (intent == TransferIntent::Copy).then(|| change.clone()),
        )?;
        if intent == TransferIntent::Move {
            let parked = journal.reserve(source.parent().unwrap())?;
            journal.move_entry(source.to_path_buf(), parked.clone(), Some(change))?;
            before.matches(&parked)?;
        }
    } else {
        if journal.areas.is_empty() {
            journal.reserve(source.parent().unwrap())?;
        }
        match journal.move_entry(
            source.to_path_buf(),
            destination.clone(),
            Some(change.clone()),
        ) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::CrossesDevices => {
                let before = DiskVersion::read_cancellable(source, &request.cancellation)?;
                let staged = journal.reserve(destination.parent().unwrap())?;
                copy_tree(source, &staged, &request.cancellation)?;
                before.matches(source)?;
                if !before.same_contents(&DiskVersion::read_cancellable(
                    &staged,
                    &request.cancellation,
                )?) {
                    return Err(invalid("Source changed while copying"));
                }
                journal.move_entry(staged, destination, None)?;
                let parked = journal.reserve(source.parent().unwrap())?;
                journal.move_entry(source.to_path_buf(), parked.clone(), Some(change))?;
                before.matches(&parked)?;
            }
            Err(e) => return Err(e),
        }
    }
    Ok(ItemOutcome::Completed)
}

#[cfg(test)]
mod tests;
