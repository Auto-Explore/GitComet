use crate::model::{RepoId, RepoLifetime, RepoLoadEpoch};
use gitcomet_core::services::GitRepository;
use rustc_hash::FxHashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use super::effects::worktree_list::WorktreeListPublisher;
use super::executor::{LatestTaskSlot, SelectedDiffSlots};
use super::repo_load_trace;
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::services::CancellationToken;
use std::sync::Mutex;

/// A task's permission to reuse or populate the linked-worktree cache. Unlike a
/// load scope, it survives refreshes and tab switches. Only the runtime owner
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

enum RepositoryConnection {
    Opening,
    Opened(Arc<dyn GitRepository>),
}

/// Backend connections, ordinary load scopes, and worktree-cache ownership
/// belong to the worker. Signature verification and history find keep their
/// independent cancellation protocols.
pub(super) struct RepositoryRuntime {
    connection: RepositoryConnection,
    load_scope: Option<RepoTaskScope>,
    worktree_cache: WorktreeCacheLease,
}

impl RepositoryRuntime {
    pub(super) fn new(lifetime: RepoLifetime) -> Self {
        Self {
            connection: RepositoryConnection::Opening,
            load_scope: None,
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
        if let Some(scope) = &self.load_scope {
            scope.cancel();
        }
    }
}

#[derive(Default)]
pub(super) struct RepositoryRuntimes {
    entries: FxHashMap<RepoId, RepositoryRuntime>,
    // Reducers retire entries under the state lock; the worker disposes them
    // after releasing that lock, before it schedules the returned effects.
    retired: Vec<RepositoryRuntime>,
}

impl RepositoryRuntimes {
    fn begin_opening(&mut self, repo_id: RepoId, lifetime: RepoLifetime) -> &mut RepositoryRuntime {
        if self
            .entries
            .get(&repo_id)
            .is_some_and(|runtime| runtime.worktree_cache.lifetime != lifetime)
        {
            self.retire(&repo_id);
        }
        self.entries
            .entry(repo_id)
            .or_insert_with(|| RepositoryRuntime::new(lifetime))
    }

    pub(super) fn opened(
        &mut self,
        repo_id: RepoId,
        lifetime: RepoLifetime,
        repository: Arc<dyn GitRepository>,
    ) {
        self.begin_opening(repo_id, lifetime).connection = RepositoryConnection::Opened(repository);
    }

    pub(super) fn repository(&self, repo_id: &RepoId) -> Option<&Arc<dyn GitRepository>> {
        match &self.entries.get(repo_id)?.connection {
            RepositoryConnection::Opening => None,
            RepositoryConnection::Opened(repository) => Some(repository),
        }
    }

    pub(super) fn has_repository(&self, repo_id: &RepoId) -> bool {
        self.repository(repo_id).is_some()
    }

    pub(super) fn start_load_scope(
        &mut self,
        repo_id: RepoId,
        lifetime: RepoLifetime,
        load_epoch: RepoLoadEpoch,
    ) -> RepoTaskScope {
        let runtime = self.begin_opening(repo_id, lifetime);
        if let Some(existing) = &runtime.load_scope
            && existing.load_epoch == load_epoch
            && !existing.cancellation.is_cancelled()
        {
            repo_load_trace::trace!(
                "repo_load_token reuse repo_id={:?} load_epoch={}",
                repo_id,
                load_epoch
            );
            return existing.clone();
        }
        let scope = RepoTaskScope::new(load_epoch);
        if let Some(previous) = runtime.load_scope.replace(scope.clone()) {
            repo_load_trace::trace!(
                "repo_load_token replace_and_cancel_previous repo_id={:?} previous_epoch={} new_epoch={}",
                repo_id,
                previous.load_epoch,
                load_epoch
            );
            previous.cancel();
        } else {
            repo_load_trace::trace!(
                "repo_load_token create repo_id={:?} load_epoch={}",
                repo_id,
                load_epoch
            );
        }
        scope
    }

    pub(super) fn load_scope(&self, repo_id: &RepoId) -> Option<&RepoTaskScope> {
        self.entries.get(repo_id)?.load_scope.as_ref()
    }

    pub(super) fn load_scope_mut(&mut self, repo_id: &RepoId) -> Option<&mut RepoTaskScope> {
        self.entries.get_mut(repo_id)?.load_scope.as_mut()
    }

    pub(super) fn load_scopes_mut(
        &mut self,
    ) -> impl Iterator<Item = (&RepoId, &mut RepoTaskScope)> {
        self.entries.iter_mut().filter_map(|(repo_id, runtime)| {
            runtime.load_scope.as_mut().map(|scope| (repo_id, scope))
        })
    }

    pub(super) fn load_scope_count(&self) -> usize {
        self.entries
            .values()
            .filter(|runtime| runtime.load_scope.is_some())
            .count()
    }

    pub(super) fn take_load_scope(&mut self, repo_id: &RepoId) -> Option<RepoTaskScope> {
        self.entries.get_mut(repo_id)?.load_scope.take()
    }

    pub(super) fn cancel_all_loads(&mut self) {
        for runtime in self.entries.values_mut() {
            if let Some(scope) = runtime.load_scope.take() {
                scope.cancel();
            }
        }
    }

    pub(super) fn cancel_loads(&mut self, repo_id: RepoId, load_epoch: RepoLoadEpoch) {
        let matched = self
            .load_scope(&repo_id)
            .is_some_and(|scope| scope.load_epoch == load_epoch);
        repo_load_trace::trace!(
            "cancel_repo_loads_effect repo_id={:?} load_epoch={} matched_token={}",
            repo_id,
            load_epoch,
            matched
        );
        if matched && let Some(scope) = self.take_load_scope(&repo_id) {
            scope.cancel();
        }
    }

    pub(super) fn worktree_cache(
        &mut self,
        repo_id: RepoId,
        lifetime: RepoLifetime,
    ) -> WorktreeCacheLease {
        self.begin_opening(repo_id, lifetime).worktree_cache()
    }

    pub(super) fn retire(&mut self, repo_id: &RepoId) {
        if let Some(runtime) = self.entries.remove(repo_id) {
            self.retired.push(runtime);
        }
    }

    pub(super) fn release_retired(&mut self) {
        self.retired.clear();
    }

    pub(super) fn clear(&mut self) {
        self.entries.clear();
        self.release_retired();
    }

    #[cfg(test)]
    pub(super) fn attach_repository_for_test(
        &mut self,
        repo_id: RepoId,
        repository: Arc<dyn GitRepository>,
    ) {
        self.opened(repo_id, RepoLifetime::allocate(), repository);
    }

    #[cfg(test)]
    pub(super) fn bind_test_state(&mut self, state: &crate::model::AppState) {
        for repo in &state.repos {
            if let Some(runtime) = self.entries.get_mut(&repo.id)
                && runtime.worktree_cache.lifetime != repo.lifetime_id()
            {
                assert!(
                    runtime.load_scope.is_none(),
                    "bind fixtures before scheduling loads"
                );
                let repository = self.repository(&repo.id).cloned();
                self.retire(&repo.id);
                if let Some(repository) = repository {
                    self.opened(repo.id, repo.lifetime_id(), repository);
                }
            }
        }
        self.release_retired();
    }

    #[cfg(test)]
    pub(super) fn repository_specs_for_test(
        &self,
    ) -> impl Iterator<Item = (RepoId, gitcomet_core::domain::RepoSpec)> + '_ {
        self.entries
            .iter()
            .filter_map(|(repo_id, runtime)| match &runtime.connection {
                RepositoryConnection::Opening => None,
                RepositoryConnection::Opened(repo) => Some((*repo_id, repo.spec().clone())),
            })
    }
}

#[cfg(test)]
impl FromIterator<(RepoId, Arc<dyn GitRepository>)> for RepositoryRuntimes {
    fn from_iter<T: IntoIterator<Item = (RepoId, Arc<dyn GitRepository>)>>(iter: T) -> Self {
        let mut runtimes = Self::default();
        for (repo_id, repository) in iter {
            runtimes.attach_repository_for_test(repo_id, repository);
        }
        runtimes
    }
}

#[derive(Clone)]
pub(super) struct RepoTaskScope {
    pub(super) load_epoch: crate::model::RepoLoadEpoch,
    pub(super) cancellation: CancellationToken,
    /// Cancellation for the *current* log walk alone. An author-filtered walk
    /// on a large repository runs for tens of seconds and the repo-load pool
    /// has one or two threads, so a superseded walk has to be stopped for its
    /// replacement to start at all — but stopping it must not disturb the
    /// repository's other loads, which share [`Self::cancellation`].
    log_cancellation: Arc<Mutex<CancellationToken>>,
    file_browser_cancellation: Arc<Mutex<CancellationToken>>,
    // The revision distinguishes refreshes of the same path. All loads for one
    // selection share a child token, so superseding it leaves log/status work alive.
    pub(super) selected_diff: Arc<Mutex<Option<(DiffTarget, u64, CancellationToken)>>>,
    // Only the store worker changes selections. Avoid locking the shared task
    // slot on unrelated messages or when this repo has no selected-diff work.
    selected_diff_key: Option<(DiffTarget, u64)>,
    pub(super) selected_diff_slots: SelectedDiffSlots,
    pub(super) commit_details_slot: LatestTaskSlot,
    pub(super) worktree_list_publisher: WorktreeListPublisher,
}

impl RepoTaskScope {
    pub(super) fn new(load_epoch: crate::model::RepoLoadEpoch) -> Self {
        Self {
            load_epoch,
            cancellation: CancellationToken::new(),
            log_cancellation: Arc::new(Mutex::new(CancellationToken::new())),
            file_browser_cancellation: Arc::new(Mutex::new(CancellationToken::new())),
            selected_diff: Arc::new(Mutex::new(None)),
            selected_diff_key: None,
            selected_diff_slots: SelectedDiffSlots::default(),
            commit_details_slot: LatestTaskSlot::default(),
            worktree_list_publisher: WorktreeListPublisher::default(),
        }
    }

    /// Cancels the log walk in flight, if any, and hands out the token for the
    /// walk that replaces it.
    pub(super) fn take_over_log(&self) -> CancellationToken {
        let mut slot = self
            .log_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        slot.cancel();
        let next = CancellationToken::new();
        *slot = next.clone();
        next
    }

    pub(super) fn take_over_file_browser(&self) -> CancellationToken {
        let mut current = self
            .file_browser_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        current.cancel();
        *current = CancellationToken::new();
        current.clone()
    }

    pub(super) fn selected_diff_cancellation(
        &mut self,
        target: &DiffTarget,
        revision: u64,
    ) -> CancellationToken {
        self.selected_diff_key = Some((target.clone(), revision));
        let mut slot = self.selected_diff.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((previous, previous_rev, token)) = slot.as_ref() {
            if previous == target && *previous_rev == revision && !token.is_cancelled() {
                return token.clone();
            }
            token.cancel();
        }
        let token = CancellationToken::new().with_parent(self.cancellation.clone());
        *slot = Some((target.clone(), revision, token.clone()));
        token
    }

    pub(super) fn has_selected_diff_work(&self) -> bool {
        self.selected_diff_key.is_some()
    }

    pub(super) fn cancel_stale_selected_diff(&mut self, selected: Option<(&DiffTarget, u64)>) {
        if self
            .selected_diff_key
            .as_ref()
            .is_none_or(|(target, revision)| selected == Some((target, *revision)))
        {
            return;
        }
        self.selected_diff_key = None;
        let mut slot = self.selected_diff.lock().unwrap_or_else(|e| e.into_inner());
        if slot
            .as_ref()
            .is_some_and(|(target, revision, _)| selected != Some((target, *revision)))
            && let Some((_, _, token)) = slot.take()
        {
            token.cancel();
        }
    }

    /// Cancels every task running under this token, log walks included.
    pub(super) fn cancel(&self) {
        self.cancellation.cancel();
        self.file_browser_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel();
        self.log_cancellation
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .cancel();
    }
}
