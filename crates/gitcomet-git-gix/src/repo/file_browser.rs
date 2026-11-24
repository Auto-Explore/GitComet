use gitcomet_core::domain::{CommitId, FileEntry, FileEntryKind};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::GixRepo;

impl GixRepo {
    pub(super) fn sample_tracked_file_paths_impl(
        &self,
        limit: std::num::NonZeroUsize,
    ) -> Result<Vec<PathBuf>> {
        use gix::index::entry::Mode;
        let repo = self.repo();
        let Some(index) = repo
            .try_index()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?
        else {
            return Ok(Vec::new());
        };
        let entries = index.entries();
        let stride = entries.len().div_ceil(limit.get()).max(1);
        Ok(entries
            .iter()
            .step_by(stride)
            .filter(|entry| matches!(entry.mode, Mode::FILE | Mode::FILE_EXECUTABLE))
            .filter_map(|entry| {
                gix::path::from_bstr(entry.path(&index))
                    .ok()
                    .map(|path| path.into_owned())
            })
            .collect())
    }

    pub(super) fn list_worktree_files_impl(&self) -> Result<Vec<FileEntry>> {
        let repo = self.repo();
        let index = repo
            .index_or_empty()
            .map_err(|e| Error::new(ErrorKind::Backend(format!("gix index: {e}"))))?;
        let options = repo
            .dirwalk_options()
            .map_err(|e| {
                Error::new(ErrorKind::Backend(format!(
                    "gix file browser dirwalk options: {e}"
                )))
            })?
            // Tracked entries are hidden by default.
            .emit_tracked(true)
            // `CollapseDirectory` would fold a wholly-untracked folder into one row
            // and hide its contents.
            .emit_untracked(gix::dir::walk::EmissionMode::Matching)
            .emit_ignored(None)
            .emit_pruned(false)
            .emit_empty_directories(true)
            .recurse_repositories(false);

        let mut delegate = CollectWorktreePaths::default();
        repo.dirwalk(
            &index,
            Vec::<gix::bstr::BString>::new(),
            &AtomicBool::default(),
            options,
            &mut delegate,
        )
        .map_err(|e| Error::new(ErrorKind::Backend(format!("gix dirwalk: {e}"))))?;

        Ok(flatten_worktree_paths(delegate.paths))
    }

    pub(super) fn list_tree_files_at_commit_impl(
        &self,
        commit_id: &CommitId,
    ) -> Result<Vec<FileEntry>> {
        let repo = self.repo();
        let oid =
            super::object_id_from_commit_id(commit_id, repo.object_hash()).ok_or_else(|| {
                Error::new(ErrorKind::Backend(format!(
                    "invalid commit id for this repository: {}",
                    commit_id.as_ref()
                )))
            })?;
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

/// `can_recurse` is deliberately left at its default: it already declines to
/// descend into ignored directories and into nested repositories, which is the
/// pruning this listing depends on.
#[derive(Default)]
struct CollectWorktreePaths {
    paths: Vec<(String, bool)>,
}

impl gix::dir::walk::Delegate for CollectWorktreePaths {
    fn emit(
        &mut self,
        entry: gix::dir::EntryRef<'_>,
        _collapsed_directory_status: Option<gix::dir::entry::Status>,
    ) -> gix::dir::walk::Action {
        use gix::dir::entry::Kind;

        let is_directory = match entry.disk_kind {
            // `Repository` is a submodule: a folder we never descend into.
            Some(Kind::Directory | Kind::Repository) => true,
            Some(Kind::File | Kind::Symlink) => false,
            // FIFOs, sockets and devices cannot be tracked or opened.
            Some(Kind::Untrackable) | None => return gix::dir::walk::Action::Continue(()),
        };

        // Staging areas the filesystem service may leave in a worktree.
        if gitcomet_core::path_utils::has_service_owned_git_component(&entry.rela_path) {
            return gix::dir::walk::Action::Continue(());
        }
        let path = entry.rela_path.to_string();
        if !path.is_empty() {
            self.paths.push((path, is_directory));
        }
        gix::dir::walk::Action::Continue(())
    }
}

#[derive(Default)]
struct WorktreeDir {
    dirs: BTreeMap<String, WorktreeDir>,
    files: BTreeSet<String>,
}

/// The walk emits leaves only, in readdir order, so intermediate directories are
/// synthesized from path components and the result re-sorted to match
/// [`collect_tree_entries`]. Both sources must agree on ordering and path form,
/// or expansion state and reveal-by-path stop matching when switching between the
/// live tree and a commit's tree.
fn flatten_worktree_paths(paths: Vec<(String, bool)>) -> Vec<FileEntry> {
    let path_count = paths.len();
    let mut root = WorktreeDir::default();

    for (path, is_directory) in paths {
        let mut node = &mut root;
        let mut components = path.split('/').filter(|c| !c.is_empty()).peekable();
        while let Some(component) = components.next() {
            if components.peek().is_some() || is_directory {
                node = node.dirs.entry(component.to_string()).or_default();
            } else {
                node.files.insert(component.to_string());
            }
        }
    }

    let mut out = Vec::with_capacity(path_count);
    flatten_worktree_dir(&root, "", 0, &mut out);
    out
}

fn flatten_worktree_dir(
    dir: &WorktreeDir,
    parent_path: &str,
    depth: usize,
    out: &mut Vec<FileEntry>,
) {
    let join = |name: &str| {
        if parent_path.is_empty() {
            name.to_string()
        } else {
            format!("{parent_path}/{name}")
        }
    };

    for (name, child) in &dir.dirs {
        // `PathBuf::from(String)` reuses the buffer: one allocation per entry
        // for the Arc rather than String + PathBuf + Arc.
        let path = join(name);
        let child_path = Arc::new(PathBuf::from(path));
        out.push(FileEntry {
            name: name.clone(),
            path: Arc::clone(&child_path),
            kind: FileEntryKind::Directory,
            depth,
            ignored: false,
        });
        flatten_worktree_dir(
            child,
            child_path.to_str().unwrap_or_default(),
            depth + 1,
            out,
        );
    }

    for name in &dir.files {
        out.push(FileEntry {
            name: name.clone(),
            path: Arc::new(PathBuf::from(join(name))),
            kind: FileEntryKind::File,
            depth,
            ignored: false,
        });
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
    collect_tree_entries(repo, &tree, "", &mut entries, 0)?;
    Ok(entries)
}

fn collect_tree_entries(
    repo: &gix::Repository,
    tree: &gix::Tree<'_>,
    parent_path: &str,
    out: &mut Vec<FileEntry>,
    depth: usize,
) -> Result<()> {
    let mut child_entries: Vec<(String, gix::objs::tree::EntryMode, gix::ObjectId)> =
        Vec::with_capacity(tree.iter().count());

    for entry in tree.iter() {
        let entry =
            entry.map_err(|e| Error::new(ErrorKind::Backend(format!("gix tree entry: {e}"))))?;
        let name = entry.filename().to_string();
        let mode = entry.mode();
        let oid = entry.oid().to_owned();
        child_entries.push((name, mode, oid));
    }

    child_entries.sort_unstable_by(|(a_name, a_mode, _), (b_name, b_mode, _)| {
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
            let child_path = Arc::new(PathBuf::from(path));
            out.push(FileEntry {
                name,
                path: Arc::clone(&child_path),
                kind: FileEntryKind::Directory,
                depth,
                ignored: false,
            });

            let child_object = repo
                .find_object(oid)
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix find_object: {e}"))))?;
            let child_tree = child_object
                .peel_to_tree()
                .map_err(|e| Error::new(ErrorKind::Backend(format!("gix peel_to_tree: {e}"))))?;
            collect_tree_entries(
                repo,
                &child_tree,
                child_path.to_str().unwrap_or_default(),
                out,
                depth + 1,
            )?;
        } else if mode.is_blob() || mode.is_link() {
            out.push(FileEntry {
                name,
                path: Arc::new(PathBuf::from(path)),
                kind: FileEntryKind::File,
                depth,
                ignored: false,
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
