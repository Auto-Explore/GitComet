use super::GixRepo;
use crate::util::run_git_simple_with_paths;
use gitcomet_core::domain::FileStatusKind;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::Result;
use rustc_hash::FxHashSet as HashSet;
use std::path::Path;

impl GixRepo {
    pub(super) fn discard_worktree_changes_impl(&self, paths: &[&Path]) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        let status = self.status_impl()?;
        let mut selected: HashSet<&Path> =
            HashSet::with_capacity_and_hasher(paths.len(), Default::default());
        selected.extend(paths.iter().copied());

        let mut checkout_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        let mut submodule_update_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        let mut clean_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        let mut unstaged_selected: HashSet<&Path> =
            HashSet::with_capacity_and_hasher(paths.len(), Default::default());
        let mut has_conflicts = false;
        let submodule_paths: HashSet<std::path::PathBuf> = self
            .list_submodules_impl()?
            .into_iter()
            .map(|submodule| submodule.path)
            .collect();

        for entry in &status.unstaged {
            let path = entry.path.as_path();
            if !selected.contains(path) {
                continue;
            }

            unstaged_selected.insert(path);
            match entry.kind {
                FileStatusKind::Conflicted => has_conflicts = true,
                FileStatusKind::Untracked => clean_paths.push(path),
                _ if submodule_paths.contains(path) => submodule_update_paths.push(path),
                _ => checkout_paths.push(path),
            }
        }

        let mut remove_paths: Vec<&Path> = Vec::with_capacity(paths.len());
        for entry in &status.staged {
            let path = entry.path.as_path();
            if !selected.contains(path) {
                continue;
            }

            match entry.kind {
                FileStatusKind::Conflicted => has_conflicts = true,
                FileStatusKind::Added if !unstaged_selected.contains(path) => {
                    remove_paths.push(path)
                }
                _ => {}
            }
        }

        if has_conflicts {
            return Err(Error::new(ErrorKind::Backend(
                "Cannot discard changes for conflicted files.".to_string(),
            )));
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
