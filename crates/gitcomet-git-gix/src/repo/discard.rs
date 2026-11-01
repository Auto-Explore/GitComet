use super::GixRepo;
use crate::util::run_git_simple_with_paths;
use gitcomet_core::domain::FileStatusKind;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{ConflictSide, Result};
use rustc_hash::FxHashSet;
use std::fs;
use std::path::{Path, PathBuf};

impl GixRepo {
    pub(super) fn discard_worktree_changes_impl(&self, paths: &[&Path]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let status = self.status_impl()?;
        let mut selected: FxHashSet<&Path> =
            FxHashSet::with_capacity_and_hasher(paths.len(), Default::default());
        selected.extend(paths.iter().copied());

        let mut checkout_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        let mut submodule_update_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        let mut clean_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        let mut unstaged_selected: FxHashSet<&Path> =
            FxHashSet::with_capacity_and_hasher(paths.len(), Default::default());
        // Discarding a conflict keeps our side, as "Resolve using ours" does.
        let mut conflicted_paths: Vec<&Path> = Vec::new();
        let submodule_paths: FxHashSet<std::path::PathBuf> = self
            .list_submodules_impl()?
            .into_iter()
            .map(|submodule| submodule.path)
            .collect();

        for entry in status.unstaged.iter() {
            let path = entry.path.as_path();
            if !selected.contains(path) {
                continue;
            }

            unstaged_selected.insert(path);
            match entry.kind {
                FileStatusKind::Conflicted => conflicted_paths.push(path),
                FileStatusKind::Untracked => clean_paths.push(path),
                _ if submodule_paths.contains(path) => submodule_update_paths.push(path),
                _ => checkout_paths.push(path),
            }
        }

        let mut remove_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        for entry in status.staged.iter() {
            let path = entry.path.as_path();
            if !selected.contains(path) {
                continue;
            }

            match entry.kind {
                FileStatusKind::Conflicted if !unstaged_selected.contains(path) => {
                    conflicted_paths.push(path)
                }
                FileStatusKind::Added if !unstaged_selected.contains(path) => {
                    remove_paths.push(path)
                }
                _ => {}
            }
        }

        if conflicted_paths
            .iter()
            .any(|path| submodule_paths.contains(*path))
        {
            return Err(Error::new(ErrorKind::Backend(
                "Cannot discard changes for conflicted submodules.".to_string(),
            )));
        }

        // `git checkout --` forces its way through whatever stands where a
        // path goes and deletes it, so refuse unless this discard removes all
        // of it anyway.
        let discarded: FxHashSet<&Path> =
            clean_paths.iter().chain(&remove_paths).copied().collect();
        for path in &checkout_paths {
            if let Some(blocker) = checkout_blocker(&self.spec.workdir, path, &discarded) {
                return Err(Error::new(ErrorKind::Backend(blocked_discard_message(
                    path, &blocker,
                ))));
            }
        }

        for path in conflicted_paths {
            self.checkout_conflict_side_impl(path, ConflictSide::Ours)?;
        }

        // Keep behavior deterministic for mixed selections.
        if !remove_paths.is_empty() {
            run_git_simple_with_paths(
                &self.spec.workdir,
                "git rm -f",
                &["rm", "-f"],
                &remove_paths,
            )?;
        }
        if !clean_paths.is_empty() {
            run_git_simple_with_paths(
                &self.spec.workdir,
                "git clean -fd",
                &["clean", "-fd"],
                &clean_paths,
            )?;
        }
        if !submodule_update_paths.is_empty() {
            // `git checkout -- <submodule>` does not reliably move the nested HEAD back to the
            // superproject-recorded commit. Use a path-scoped submodule update instead so discard
            // actually checks out the recorded pointer again.
            run_git_simple_with_paths(
                &self.spec.workdir,
                "git submodule update --checkout --force",
                &["submodule", "update", "--checkout", "--force"],
                &submodule_update_paths,
            )?;
        }
        if !checkout_paths.is_empty() {
            run_git_simple_with_paths(
                &self.spec.workdir,
                "git checkout --",
                &["checkout"],
                &checkout_paths,
            )?;
        }

        Ok(())
    }
}

/// What restoring `path` would delete that `discarded` does not remove first:
/// a directory now at `path` holding anything else, or a file or symlink
/// standing where one of its parent directories goes.
fn checkout_blocker(workdir: &Path, path: &Path, discarded: &FxHashSet<&Path>) -> Option<PathBuf> {
    for parent in path.ancestors().skip(1) {
        if parent.as_os_str().is_empty() || discarded.contains(parent) {
            continue;
        }
        if fs::symlink_metadata(workdir.join(parent)).is_ok_and(|metadata| !metadata.is_dir()) {
            return Some(parent.to_path_buf());
        }
    }
    let is_directory =
        fs::symlink_metadata(workdir.join(path)).is_ok_and(|metadata| metadata.is_dir());
    (is_directory && holds_undiscarded(workdir, path, discarded)).then(|| path.to_path_buf())
}

/// Whether the directory `dir` holds anything but empty directories and what
/// `discarded` names. Unreadable contents count as held.
fn holds_undiscarded(workdir: &Path, dir: &Path, discarded: &FxHashSet<&Path>) -> bool {
    let Ok(entries) = fs::read_dir(workdir.join(dir)) else {
        return true;
    };
    entries.into_iter().any(|entry| {
        let Ok(entry) = entry else {
            return true;
        };
        let path = dir.join(entry.file_name());
        if discarded.contains(path.as_path()) {
            return false;
        }
        match entry.file_type() {
            Ok(kind) if kind.is_dir() => holds_undiscarded(workdir, &path, discarded),
            _ => true,
        }
    })
}

fn blocked_discard_message(path: &Path, blocker: &Path) -> String {
    if blocker == path {
        format!(
            "Cannot discard changes to {}: a directory now stands there, and restoring the file \
             would delete it with files this discard leaves out. Move the directory, or discard \
             its files too.",
            path.display()
        )
    } else {
        format!(
            "Cannot discard changes to {}: {} is now a file, and restoring the path would delete \
             it. Move it, or discard it too.",
            path.display(),
            blocker.display()
        )
    }
}
