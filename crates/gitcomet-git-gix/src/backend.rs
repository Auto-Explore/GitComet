use crate::repo::GixRepo;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::path_utils::strip_windows_verbatim_prefix;
use gitcomet_core::services::{
    CancellationToken, GitBackend, GitRepository, Result, WorktreeIgnoreMatcher,
};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

pub struct GixBackend;

/// Weak working-tree registrations. Matching windows reuse a handle; common
/// repository maintenance also refreshes compatible and incompatible stores
/// held by its other working trees.
static OPEN_REPOS: Mutex<Vec<Weak<GixRepo>>> = Mutex::new(Vec::new());

fn reuse_or_register(repo: GixRepo) -> Arc<GixRepo> {
    let mut open = OPEN_REPOS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    open.retain(|repo| repo.strong_count() > 0);
    if let Some(existing) = open
        .iter()
        .filter_map(Weak::upgrade)
        .find(|existing| existing.identity == repo.identity && existing.options == repo.options)
    {
        return existing;
    }
    let repo = Arc::new(repo);
    open.push(Arc::downgrade(&repo));
    repo
}

/// Called under the common repository's maintenance lock. Active readers own
/// their old stores until they finish; all persistent owners switch together.
pub(crate) fn refresh_common_stores(origin: &GixRepo, common: u64) -> Result<()> {
    let open = OPEN_REPOS
        .lock()
        .expect("open repositories")
        .iter()
        .filter_map(Weak::upgrade)
        .filter(|repo| repo.common_identity() == common)
        .collect::<Vec<_>>();
    let result = origin.refresh_object_store();
    for repo in open {
        if !std::ptr::eq(origin, &*repo) {
            let _ = repo.refresh_object_store();
        }
    }
    result
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
        options: &gitcomet_core::services::RepositoryOptions,
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
        crate::refs::validate_open(&repo, cancellation)?;
        if let Some(cancellation) = cancellation {
            cancellation.check_cancelled()?;
        }

        gitcomet_core::history_perf::register_shared_memory_provider(crate::shared_history_memory);
        let repo = reuse_or_register(GixRepo::new_with_options(
            workdir,
            repo.into_sync(),
            options.clone(),
        ));
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
        self.open_impl(workdir, None, &Default::default())
    }

    fn open_with_options(
        &self,
        workdir: &Path,
        options: &gitcomet_core::services::RepositoryOptions,
    ) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, None, options)
    }

    fn open_cancellable_with_options(
        &self,
        workdir: &Path,
        options: &gitcomet_core::services::RepositoryOptions,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, Some(cancellation), options)
    }

    fn release_object_stores(&self, common_dir: &Path) {
        // Upgraded first so reopening runs without the registry lock.
        let open = OPEN_REPOS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .filter_map(Weak::upgrade)
            .collect::<Vec<_>>();
        let mut released = std::collections::HashSet::new();
        for repo in open {
            if repo.common_dir_impl() == common_dir && released.insert(repo.common_identity()) {
                let _ = repo.reopen_object_store();
            }
        }
    }

    fn open_cancellable(
        &self,
        workdir: &Path,
        cancellation: &CancellationToken,
    ) -> Result<Arc<dyn GitRepository>> {
        self.open_impl(workdir, Some(cancellation), &Default::default())
    }

    fn worktree_ignore_matcher(
        &self,
        workdir: &Path,
    ) -> Result<Option<Box<dyn WorktreeIgnoreMatcher>>> {
        crate::ignore::GixWorktreeIgnoreMatcher::load(workdir)
            .map(|matcher| Some(Box::new(matcher) as Box<dyn WorktreeIgnoreMatcher>))
    }
}
