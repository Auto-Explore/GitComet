use crate::repo::GixRepo;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::strip_windows_verbatim_prefix;
use gitcomet_core::services::{
    CancellationToken, GitBackend, GitRepository, Result, WorktreeIgnoreMatcher,
};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

pub struct GixBackend;

/// Every repository this process opened: each window and worktree scan holds
/// its own store, and a maintenance run must release them all.
static OPEN_REPOS: Mutex<Vec<Weak<GixRepo>>> = Mutex::new(Vec::new());

fn register_open_repo(repo: &Arc<GixRepo>) {
    let mut open = OPEN_REPOS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    open.retain(|repo| repo.strong_count() > 0);
    open.push(Arc::downgrade(repo));
}

impl Default for GixBackend {
    fn default() -> Self {
        Self
    }
}

impl GixBackend {
    fn open_impl(
        &self,
        workdir: &Path,
        cancellation: Option<&CancellationToken>,
    ) -> Result<Arc<dyn GitRepository>> {
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        let workdir = strip_windows_verbatim_prefix(
            workdir
                .canonicalize()
                .map_err(|e| Error::new(ErrorKind::Io(e.kind())))?,
        );
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        let repo = crate::open::open_worktree_repo(&workdir)
            .map_err(|e| crate::open::map_open_error(e, "gix open"))?;
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        let repo = Arc::new(GixRepo::new(workdir, repo.into_sync()));
        register_open_repo(&repo);
        Ok(repo)
    }
}

impl GitBackend for GixBackend {
    fn repository_watch_info(
        &self,
        workdir: &Path,
    ) -> Result<Option<gitcomet_core::services::RepositoryWatchInfo>> {
        crate::ignore::repository_watch_info(workdir).map(Some)
    }

    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, None)
    }

    fn release_object_stores(&self, common_dir: &Path) {
        // Upgraded first so reopening runs without the registry lock.
        let open = OPEN_REPOS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        for repo in open {
            if repo.common_dir_impl() == common_dir {
                let _ = repo.reopen_object_store();
            }
        }
    }

    fn open_cancellable(
        &self,
        workdir: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, Some(cancellation))
    }

    fn worktree_ignore_matcher(
        &self,
        workdir: &Path,
    ) -> Result<Option<Box<dyn WorktreeIgnoreMatcher>>> {
        crate::ignore::GixWorktreeIgnoreMatcher::load(workdir)
            .map(|matcher| Some(Box::new(matcher) as Box<dyn WorktreeIgnoreMatcher>))
    }
}
