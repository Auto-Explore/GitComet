use gitcomet_core::domain::{CommitId, FileEntry, FileEntryKind};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use std::path::PathBuf;
use std::sync::Arc;

use super::GixRepo;

impl GixRepo {
    pub(super) fn list_tree_files_impl(&self) -> Result<Vec<FileEntry>> {
        let repo = self._repo.to_thread_local();
        let head = repo
            .head_commit()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix head_commit: {e}"))))?;
        let tree_id = head
            .tree_id()
            .map(|id| id.detach())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree_id: {e}"))))?;
        list_tree_files_at_oid(&repo, tree_id)
    }

    pub(super) fn list_tree_files_at_commit_impl(
        &self,
        commit_id: &CommitId,
    ) -> Result<Vec<FileEntry>> {
        let repo = self._repo.to_thread_local();
        let oid = gix::ObjectId::from_hex(commit_id.0.as_bytes())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("invalid commit id: {e}"))))?;
        let commit = repo
            .find_commit(oid)
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find_commit: {e}"))))?;
        let tree_id = commit
            .tree_id()
            .map(|id| id.detach())
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree_id: {e}"))))?;
        list_tree_files_at_oid(&repo, tree_id)
    }
}

fn list_tree_files_at_oid(
    repo: &gix::Repository,
    tree_id: gix::ObjectId,
) -> Result<Vec<FileEntry>> {
    let object = repo
        .find_object(tree_id)
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find_object: {e}"))))?;
    let tree = object
        .peel_to_tree()
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel_to_tree: {e}"))))?;

    let mut entries = Vec::new();
    collect_tree_entries(repo, &tree, String::new(), &mut entries, 0)?;
    Ok(entries)
}

fn collect_tree_entries(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    parent_path: String,
    out: &mut Vec<FileEntry>,
    depth: usize,
) -> Result<()> {
    let mut child_entries: Vec<(String, gix::objs::tree::EntryMode, gix::ObjectId)> = Vec::new();

    for entry in tree.iter() {
        let entry = entry.map_err(|e| {
            Error::new(ErrorKind::Backend(format!("gix tree entry: {e}")))
        })?;
        let name = entry
            .filename()
            .to_string();
        let mode = entry.mode();
        let oid = entry.oid().to_owned();
        child_entries.push((name, mode, oid));
    }

    child_entries.sort_by(|(a_name, a_mode, _), (b_name, b_mode, _)| {
        let a_is_dir = a_mode.is_tree();
        let b_is_dir = b_mode.is_tree();
        match (a_is_dir, b_is_dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a_name.cmp(b_name),
        }
    });

    for (name, mode, oid) in child_entries {
        let path = if parent_path.is_empty() {
            name.clone()
        } else {
            format!("{parent_path}/{name}")
        };

        if mode.is_tree() {
            out.push(FileEntry {
                name,
                path: Arc::new(PathBuf::from(&path)),
                kind: FileEntryKind::Directory,
                depth,
            });

            let child_object = repo.find_object(oid).map_err(|e| {
                Error::new(ErrorKind::Backend(format!("gix find_object: {e}")))
            })?;
            let child_tree = child_object.peel_to_tree().map_err(|e| {
                Error::new(ErrorKind::Backend(format!("gix peel_to_tree: {e}")))
            })?;
            collect_tree_entries(repo, &child_tree, path, out, depth + 1)?;
        } else if mode.is_blob() || mode.is_link() {
            out.push(FileEntry {
                name,
                path: Arc::new(PathBuf::from(&path)),
                kind: FileEntryKind::File,
                depth,
            });
        }
    }

    Ok(())
}
