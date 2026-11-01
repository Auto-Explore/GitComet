use crate::model::FileBrowserState;
use gitcomet_core::domain::{FileEntry, FileEntryKind};
use gitcomet_core::error::Error;
use gitcomet_core::services::{CancellationToken, Result};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Default)]
pub(in crate::store::effects) struct Options {
    ignored: bool,
    expanded: BTreeSet<PathBuf>,
    recursive: BTreeSet<PathBuf>,
    revealed: BTreeSet<PathBuf>,
    search: bool,
}

impl From<&FileBrowserState> for Options {
    fn from(browser: &FileBrowserState) -> Self {
        Self {
            ignored: browser.show_ignored,
            expanded: browser
                .expanded_dirs
                .iter()
                .map(|p| (**p).clone())
                .collect(),
            recursive: browser
                .pending_recursive_expansions
                .iter()
                .cloned()
                .collect(),
            revealed: browser.revealed_paths.iter().cloned().collect(),
            search: !browser.search_query.trim().is_empty(),
        }
    }
}

/// Whether the backend listing already holds every revealed path. It lists each
/// listed path's ancestors, in the order `augment`'s walk would rebuild.
fn lists_all(base: &[FileEntry], revealed: &BTreeSet<PathBuf>) -> bool {
    let mut missing: Vec<&Path> = revealed.iter().map(PathBuf::as_path).collect();
    for entry in base {
        if missing.is_empty() {
            break;
        }
        // File names first: comparing whole paths walks their components.
        missing.retain(|path| {
            path.file_name() != entry.path.file_name() || *path != entry.path.as_path()
        });
    }
    missing.is_empty()
}

pub(super) fn augment(
    root: &Path,
    mut base: Vec<FileEntry>,
    options: Options,
    cancellation: &CancellationToken,
) -> Result<Vec<FileEntry>> {
    // Staging areas left in a worktree are the filesystem service's, not the user's.
    base.retain(|entry| !gitcomet_core::path_utils::has_service_owned_component(&entry.path));
    if !options.ignored && lists_all(&base, &options.revealed) {
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
        let directory_path = root.join(&parent);
        let directory = std::fs::read_dir(&directory_path).map_err(|e| {
            Error::from(e).with_context("read repository directory", Some(&directory_path))
        })?;
        for entry in directory {
            cancellation.check_cancelled()?;
            let entry = entry.map_err(|e| {
                Error::from(e)
                    .with_context("read repository directory entry", Some(&directory_path))
            })?;
            let name = entry.file_name();
            if gitcomet_core::path_utils::is_git_metadata_component(&name)
                || gitcomet_core::path_utils::is_service_owned_name(name.as_encoded_bytes())
            {
                continue;
            }
            let path = parent.join(name);
            let kind = entry.file_type().map_err(|e| {
                Error::from(e).with_context("read repository entry type", Some(&entry.path()))
            })?;
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
            // Without ignored entries only a revealed path's ancestors can hold
            // anything the backend did not list.
            if kind.is_dir()
                && (revealed
                    || (options.ignored
                        && (original.contains(&path)
                            || options.expanded.contains(&path)
                            || options.search
                            || options.recursive.iter().any(|root| path.starts_with(root)))))
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
            // The backend listing holds every non-ignored path.
            ignored: !original.contains(&path),
            path: Arc::new(path),
            kind,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
