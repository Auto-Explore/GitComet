use gitcomet_core::domain::{FileEntry, FileEntryKind};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Default)]
pub(in crate::store::effects) struct Options {
    pub(in crate::store::effects) ignored: bool,
    pub(in crate::store::effects) expanded: BTreeSet<PathBuf>,
    pub(in crate::store::effects) revealed: BTreeSet<PathBuf>,
    pub(in crate::store::effects) search: bool,
}

pub(super) fn augment(
    root: &Path,
    base: Vec<FileEntry>,
    options: Options,
    cancellation: &CancellationToken,
) -> Result<Vec<FileEntry>> {
    if !options.ignored && options.revealed.is_empty() {
        return Ok(base);
    }
    let mut entries: BTreeMap<PathBuf, FileEntryKind> = base
        .into_iter()
        .map(|e| ((*e.path).clone(), e.kind))
        .collect();
    let original: BTreeSet<_> = entries.keys().cloned().collect();
    let mut pending = vec![PathBuf::new()];
    while let Some(parent) = pending.pop() {
        cancellation.check_cancelled()?;
        if !parent.as_os_str().is_empty()
            && root.join(&parent).join(".git").symlink_metadata().is_ok()
        {
            continue;
        }
        let directory = std::fs::read_dir(root.join(&parent))
            .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
        for entry in directory {
            cancellation.check_cancelled()?;
            let entry = entry.map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            let name = entry.file_name();
            if gitcomet_core::path_utils::is_git_metadata_component(&name)
                || name.as_encoded_bytes().starts_with(b".gitcomet-operation-")
                || name.as_encoded_bytes().starts_with(b".gitcomet-save-")
            {
                continue;
            }
            let path = parent.join(name);
            let kind = entry
                .file_type()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?;
            if !(kind.is_dir() || kind.is_file() || kind.is_symlink()) {
                continue;
            }
            let revealed = options.revealed.iter().any(|p| p.starts_with(&path));
            if options.ignored || original.contains(&path) || revealed {
                entries.insert(
                    path.clone(),
                    if kind.is_dir() {
                        FileEntryKind::Directory
                    } else {
                        FileEntryKind::File
                    },
                );
            }
            if kind.is_dir()
                && (original.contains(&path)
                    || revealed
                    || (options.ignored && (options.expanded.contains(&path) || options.search)))
            {
                pending.push(path);
            }
        }
    }
    let mut children: BTreeMap<PathBuf, Vec<(PathBuf, FileEntryKind)>> = BTreeMap::new();
    for (path, kind) in entries {
        children
            .entry(path.parent().unwrap_or(Path::new("")).to_path_buf())
            .or_default()
            .push((path, kind));
    }
    for children in children.values_mut() {
        children.sort_by(|a, b| {
            (a.1 != FileEntryKind::Directory, &a.0).cmp(&(b.1 != FileEntryKind::Directory, &b.0))
        });
    }
    let mut out = vec![];
    let mut pending: Vec<_> = children
        .remove(Path::new(""))
        .unwrap_or_default()
        .into_iter()
        .rev()
        .collect();
    while let Some((path, kind)) = pending.pop() {
        if let Some(children) = children.remove(&path) {
            pending.extend(children.into_iter().rev());
        }
        out.push(FileEntry {
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            depth: path.components().count().saturating_sub(1),
            path: Arc::new(path),
            kind,
        });
    }
    Ok(out)
}
