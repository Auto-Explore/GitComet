use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{Result, WorktreeIgnoreMatcher, WorktreePathKind};
use gix::index::entry::Mode as GitIndexMode;
use std::path::Path;

pub(crate) struct GixWorktreeIgnoreMatcher {
    repo: gix::Repository,
    index: gix::worktree::Index,
    excludes: gix::worktree::Stack,
}

impl GixWorktreeIgnoreMatcher {
    pub(crate) fn load(workdir: &Path) -> Result<Self> {
        let repo = crate::open::open_worktree_repo(workdir)
            .map_err(|error| crate::open::map_open_error(error, "gix ignore matcher open"))?;
        let worktree = repo.worktree().ok_or_else(|| {
            Error::new(ErrorKind::Backend(
                "gix ignore matcher: repository has no worktree".to_string(),
            ))
        })?;
        let index = worktree.index().map_err(|error| {
            Error::new(ErrorKind::Backend(format!(
                "gix ignore matcher index: {error}"
            )))
        })?;
        let excludes = repo
            .excludes(
                &index,
                None,
                gix::worktree::stack::state::ignore::Source::WorktreeThenIdMappingIfNotSkipped,
            )
            .map_err(|error| {
                Error::new(ErrorKind::Backend(format!(
                    "gix ignore matcher excludes: {error}"
                )))
            })?
            .detach();

        Ok(Self {
            repo,
            index,
            excludes,
        })
    }

    fn path_is_tracked(&self, relative_path: &Path, kind: WorktreePathKind) -> bool {
        let relative_path =
            gix::path::to_unix_separators_on_windows(gix::path::into_bstr(relative_path));
        if self.index.entry_by_path(relative_path.as_ref()).is_some() {
            return true;
        }

        kind == WorktreePathKind::Directory
            && self
                .index
                .entry_closest_to_directory_or_directory(relative_path.as_ref())
                .is_some()
    }
}

impl WorktreeIgnoreMatcher for GixWorktreeIgnoreMatcher {
    fn is_ignored(&mut self, relative_path: &Path, kind: WorktreePathKind) -> Result<bool> {
        if self.path_is_tracked(relative_path, kind) {
            return Ok(false);
        }

        let mode = match kind {
            WorktreePathKind::Directory => Some(GitIndexMode::DIR),
            WorktreePathKind::File => Some(GitIndexMode::FILE),
            WorktreePathKind::Unknown => None,
        };
        let platform = self
            .excludes
            .at_path(relative_path, mode, &self.repo.objects)
            .map_err(|error| {
                Error::new(ErrorKind::Backend(format!(
                    "gix ignore matcher path: {error}"
                )))
            })?;
        Ok(platform.is_excluded())
    }
}
