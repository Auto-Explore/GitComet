use crate::model::{RepoId, RepoLifetime};
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// A task's permission to reuse or populate the linked-worktree cache. Unlike a
/// load token, it survives refreshes and tab switches. Only the runtime owner
/// can revoke it; retaining a clone in a task cannot keep the repository open.
#[derive(Clone)]
pub(super) struct WorktreeCacheLease {
    lifetime: RepoLifetime,
    open: Arc<AtomicBool>,
}

impl WorktreeCacheLease {
    pub(super) fn lifetime(&self) -> RepoLifetime {
        self.lifetime
    }

    pub(super) fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

/// Resources belong to the worker, never to clonable AppState snapshots.
pub(super) struct RepositoryRuntime {
    worktree_cache: WorktreeCacheLease,
}

impl RepositoryRuntime {
    pub(super) fn new(lifetime: RepoLifetime) -> Self {
        Self {
            worktree_cache: WorktreeCacheLease {
                lifetime,
                open: Arc::new(AtomicBool::new(true)),
            },
        }
    }

    pub(super) fn worktree_cache(&self) -> WorktreeCacheLease {
        self.worktree_cache.clone()
    }
}

impl Drop for RepositoryRuntime {
    fn drop(&mut self) {
        // Revoke before taking the cache lock to release ownership. A task
        // opening outside that lock must check again under it before insertion.
        self.worktree_cache.open.store(false, Ordering::Release);
        super::effects::release_worktree_scan_handles(self.worktree_cache.lifetime);
    }
}

#[derive(Default)]
pub(super) struct RepositoryRuntimes {
    entries: FxHashMap<RepoId, RepositoryRuntime>,
}

impl RepositoryRuntimes {
    pub(super) fn worktree_cache(
        &mut self,
        repo_id: RepoId,
        lifetime: RepoLifetime,
    ) -> WorktreeCacheLease {
        let runtime = self
            .entries
            .entry(repo_id)
            .or_insert_with(|| RepositoryRuntime::new(lifetime));
        if runtime.worktree_cache.lifetime != lifetime {
            *runtime = RepositoryRuntime::new(lifetime);
        }
        runtime.worktree_cache()
    }

    pub(super) fn retain_open(&mut self, repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>) {
        self.entries
            .retain(|repo_id, _| repos.contains_key(repo_id));
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
    }
}
