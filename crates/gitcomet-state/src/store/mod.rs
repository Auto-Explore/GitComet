use crate::model::{AppState, RepoId};
use crate::msg::{Msg, RepoExternalChange, StoreEvent};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{GitBackend, GitRepository};
use rustc_hash::FxHashMap;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock, mpsc};
use std::thread;
use std::time::Instant;

mod effects;
mod executor;
mod reducer;
mod reducer_diagnostics;
mod repo_load_trace;
mod repo_monitor;
mod repository_preferences;
mod send_diagnostics;
mod worker_channel;

use effects::RepoTaskToken;
use effects::{EffectExecutors, schedule_effect};
use executor::StoreExecutorPool;
use executor::{
    TaskExecutor, default_worker_threads, metadata_worker_threads, repo_load_worker_threads,
};
#[cfg(feature = "benchmarks")]
use reducer::fill_select_diff_inline;
use reducer::{
    fill_reorder_repo_tabs_inline, fill_set_active_repo_inline, fill_stage_path_inline,
    fill_stage_paths_inline, fill_unstage_path_inline, fill_unstage_paths_inline, reduce,
    reset_conflict_resolutions_inline, set_conflict_region_choice_inline,
};
use repo_monitor::RepoMonitorManager;
use send_diagnostics::try_send_state_changed_or_log;
use worker_channel::{StoreInstanceId, StoreWorkerCommand, StoreWorkerSender};

pub use reducer_diagnostics::StoreReducerDiagnostics;

// Executor trace labels need a static name; derive it once from the product identity.
static WORKTREE_SCAN_THREAD_NAME: std::sync::LazyLock<String> = std::sync::LazyLock::new(|| {
    format!(
        "{}-worktree-scan",
        gitcomet_core::identity::current().executable_name()
    )
});

fn canonicalize_path(path: PathBuf) -> PathBuf {
    canonicalize_or_original(path)
}

fn make_mut_state_with_diagnostics(state: &mut Arc<AppState>) -> &mut AppState {
    let shared_state_handles = Arc::strong_count(state).saturating_sub(1);
    if shared_state_handles > 0 {
        let clone_started = Instant::now();
        let state = Arc::make_mut(state);
        reducer_diagnostics::record_clone_on_write(shared_state_handles, clone_started.elapsed());
        state
    } else {
        Arc::make_mut(state)
    }
}

fn is_control_msg(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::OpenRepo(_)
            | Msg::OpenRepoFromExternalDrop(_)
            | Msg::CloseRepo { .. }
            | Msg::MoveRepoOut { .. }
            | Msg::CloseRepos { .. }
            | Msg::CancelGitOperation { .. }
            | Msg::SetActiveRepo { .. }
            | Msg::ReorderRepoTabs { .. }
    )
}

fn is_control_command(command: &StoreWorkerCommand) -> bool {
    match command {
        StoreWorkerCommand::Msg(msg) | StoreWorkerCommand::Traced(msg, _) => is_control_msg(msg),
        StoreWorkerCommand::Shutdown => true,
        StoreWorkerCommand::Repository { .. } => true,
        #[cfg(any(test, feature = "test-support"))]
        StoreWorkerCommand::InsertRepoForTest { .. } => true,
        #[cfg(any(test, feature = "test-support"))]
        StoreWorkerCommand::DisableRepoMonitorsForTest => true,
    }
}

fn can_control_command_overtake(command: &StoreWorkerCommand) -> bool {
    matches!(command.msg(), Some(Msg::Internal(_)))
}

fn first_control_command_before_order_barrier(
    deferred: &VecDeque<StoreWorkerCommand>,
) -> Option<usize> {
    for (ix, command) in deferred.iter().enumerate() {
        if is_control_command(command) {
            return Some(ix);
        }
        if !can_control_command_overtake(command) {
            return None;
        }
    }
    None
}

fn has_order_barrier_before_control(deferred: &VecDeque<StoreWorkerCommand>) -> bool {
    for command in deferred {
        if is_control_command(command) {
            return false;
        }
        if !can_control_command_overtake(command) {
            return true;
        }
    }
    false
}

fn recv_next_worker_command(
    command_rx: &mpsc::Receiver<StoreWorkerCommand>,
    deferred: &mut VecDeque<StoreWorkerCommand>,
) -> Result<StoreWorkerCommand, mpsc::RecvError> {
    if let Some(ix) = first_control_command_before_order_barrier(deferred) {
        return Ok(deferred.remove(ix).expect("deferred command exists"));
    }

    let first = match deferred.pop_front() {
        Some(command) => command,
        None => command_rx.recv()?,
    };
    if is_control_command(&first) {
        return Ok(first);
    }
    if !can_control_command_overtake(&first) {
        return Ok(first);
    }
    if has_order_barrier_before_control(deferred) {
        return Ok(first);
    }

    while let Ok(command) = command_rx.try_recv() {
        if is_control_command(&command) {
            deferred.push_front(first);
            return Ok(command);
        }
        if !can_control_command_overtake(&command) {
            deferred.push_back(command);
            break;
        }
        deferred.push_back(command);
    }

    Ok(first)
}

#[cfg(test)]
thread_local! {
    static SELECTION_INDEX_BUILDS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Selected-diff indexes built by `reduce_and_handle` on this thread.
#[cfg(test)]
fn selection_index_builds_for_test() -> usize {
    SELECTION_INDEX_BUILDS.with(std::cell::Cell::get)
}

/// Per-message scratch context for the store worker loop: the shared handles
/// for the reduce-then-handle skeleton plus the effect dispatch machinery.
struct WorkerLoopContext<'a> {
    thread_state: &'a Arc<RwLock<Arc<AppState>>>,
    active_repo_id: &'a Arc<AtomicU64>,
    event_tx: &'a smol::channel::Sender<StoreEvent>,
    repo_monitors: &'a mut RepoMonitorManager,
    repo_task_tokens: &'a mut FxHashMap<RepoId, RepoTaskToken>,
    thread_msg_tx: &'a StoreWorkerSender,
    executor: &'a TaskExecutor,
    repo_load_executor: &'a TaskExecutor,
    worktree_scan_executor: &'a std::sync::LazyLock<TaskExecutor>,
    metadata_executor: &'a TaskExecutor,
    signature_executor: &'a TaskExecutor,
    history_find_executor: &'a std::sync::LazyLock<TaskExecutor>,
    session_persist_executor: &'a TaskExecutor,
    backend: &'a Arc<dyn GitBackend>,
    publication: &'a AtomicU64,
}

impl WorkerLoopContext<'_> {
    /// Runs one reducer under the write lock, records its pass, and dispatches
    /// its effects.
    ///
    /// Every reducer funnels through this so the lock/timing/effect-dispatch
    /// skeleton lives in one place. Inline reducers keep their canonical
    /// wrappers; active-repo switching specifically continues through
    /// [`fill_set_active_repo_inline`] and its navigation/finalization logic.
    fn reduce_and_handle<I>(
        &mut self,
        repos: &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
        id_alloc: &AtomicU64,
        reduce_with: impl FnOnce(
            &mut AppState,
            &mut FxHashMap<RepoId, Arc<dyn GitRepository>>,
            &AtomicU64,
        ) -> I,
    ) where
        I: IntoIterator<Item = crate::msg::Effect>,
    {
        let (effects, state, reduce_duration, publication) = {
            let mut app_state = self.thread_state.write().unwrap_or_else(|e| e.into_inner());
            let mutable = make_mut_state_with_diagnostics(&mut app_state);
            let reduce_started = Instant::now();
            let effects = reduce_with(mutable, repos, id_alloc);
            let reduce_duration = reduce_started.elapsed();
            reducer_diagnostics::record_reducer_pass(reduce_duration);
            // Bumped under the write lock, so a reader holding the read lock
            // sees the sequence number of exactly the state it cloned.
            let publication = self.publication.fetch_add(1, Ordering::Relaxed) + 1;
            (
                effects,
                Arc::clone(&app_state),
                reduce_duration,
                publication,
            )
        };
        gitcomet_core::op_trace::record_current(
            gitcomet_core::op_trace::Stage::Reduced,
            "reduce",
            gitcomet_core::op_trace::duration_ns(reduce_duration),
            publication,
        );
        // Cancel changed/cleared selections before dispatching their effects,
        // outside the state write lock. Index selections once, including absent
        // repos as cleared selections, instead of scanning all repos per token.
        // Most messages arrive with no selected-diff load in flight, so build
        // the index only for the first token that has one.
        let mut selections: Option<FxHashMap<_, _>> = None;
        for (id, token) in self.repo_task_tokens.iter_mut() {
            if !token.has_selected_diff_work() {
                continue;
            }
            let selections = selections.get_or_insert_with(|| {
                #[cfg(test)]
                SELECTION_INDEX_BUILDS.with(|builds| builds.set(builds.get() + 1));
                state
                    .repos
                    .iter()
                    .map(|repo| {
                        (
                            repo.id,
                            repo.diff_state
                                .diff_target
                                .as_ref()
                                .map(|target| (target, repo.diff_state.diff_target_rev)),
                        )
                    })
                    .collect()
            });
            token.cancel_stale_selected_diff(selections.get(id).copied().flatten());
        }
        drop(selections);
        drop(state);
        self.handle_effects(repos, effects);
    }

    fn handle_effects<I>(&mut self, repos: &FxHashMap<RepoId, Arc<dyn GitRepository>>, effects: I)
    where
        I: IntoIterator<Item = crate::msg::Effect>,
    {
        let active_value = self
            .thread_state
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .active_repo
            .map(|id| id.0)
            .unwrap_or(0);
        self.active_repo_id.store(active_value, Ordering::Relaxed);

        try_send_state_changed_or_log(
            self.event_tx,
            "store worker loop state notification",
            self.thread_msg_tx,
        );

        // Keep filesystem monitoring scoped to the active repository, plus any
        // a watch lease holds, to minimize OS watcher load in large multi-repo
        // sessions.
        let (active_repo, watched): (Option<RepoId>, Vec<(RepoId, std::path::PathBuf, bool)>) = {
            let state = self.thread_state.read().unwrap_or_else(|e| e.into_inner());
            let watched = state
                .repos
                .iter()
                .filter(|repo| {
                    state.active_repo == Some(repo.id) || state.watch_leases.contains_key(&repo.id)
                })
                .map(|repo| {
                    let leased = state.watch_leases.contains_key(&repo.id);
                    (repo.id, repo.spec.workdir.clone(), leased)
                })
                .collect();
            (state.active_repo, watched)
        };

        for repo_id in self.repo_monitors.running_repo_ids() {
            if !watched
                .iter()
                .any(|(watched_id, _, _)| *watched_id == repo_id)
            {
                self.repo_monitors.stop(repo_id);
            }
        }

        for (repo_id, workdir, leased) in watched {
            if repos.contains_key(&repo_id) {
                self.repo_monitors.start(
                    repo_id,
                    workdir,
                    self.thread_msg_tx.clone(),
                    Arc::clone(self.active_repo_id),
                    Arc::clone(self.backend),
                );
                self.repo_monitors.set_leased(repo_id, leased);
            }
        }

        let worktrees = {
            let state = self
                .thread_state
                .read()
                .unwrap_or_else(|error| error.into_inner());
            let mut wanted: Vec<_> = state.worktree_watch_leases.keys().cloned().collect();
            if let Some(repo) = state
                .repos
                .iter()
                .find(|repo| state.active_repo == Some(repo.id))
                && let Some(path) = &repo.history_state.worktree_selection
                && path != &repo.spec.workdir
            {
                let selected = (repo.id, repo.lifetime(), path.clone());
                if !wanted.contains(&selected) {
                    wanted.push(selected);
                }
            }
            wanted
        };
        self.repo_monitors.reconcile_worktrees(
            worktrees,
            self.thread_msg_tx.clone(),
            Arc::clone(self.active_repo_id),
            Arc::clone(self.backend),
        );

        for effect in effects {
            gitcomet_core::op_trace::record_current(
                gitcomet_core::op_trace::Stage::EffectQueued,
                repo_load_trace::effect_name(&effect),
                0,
                0,
            );
            if repo_load_trace::enabled() {
                let effect_repo_id = repo_load_trace::effect_repo_id(&effect);
                let (load_epoch, workdir) = effect_repo_id.map_or((None, None), |repo_id| {
                    let state = self.thread_state.read().unwrap_or_else(|e| e.into_inner());
                    state
                        .repos
                        .iter()
                        .find(|repo| repo.id == repo_id)
                        .map_or((None, None), |repo| {
                            (Some(repo.load_epoch), Some(repo.spec.workdir.clone()))
                        })
                });
                repo_load_trace::trace!(
                    "scheduling_effect effect={} repo_id={:?} load_epoch={:?} active_repo={:?} workdir={}",
                    repo_load_trace::effect_name(&effect),
                    effect_repo_id,
                    load_epoch,
                    active_repo,
                    workdir.as_ref().map_or("<unknown>", |workdir| workdir
                        .to_str()
                        .unwrap_or("<non-utf8>"))
                );
            }
            let _label =
                gitcomet_core::op_trace::label_scope(repo_load_trace::effect_name(&effect));
            schedule_effect(
                EffectExecutors {
                    executor: self.executor,
                    repo_load_executor: self.repo_load_executor,
                    worktree_scan_executor: self.worktree_scan_executor,
                    session_persist_executor: self.session_persist_executor,
                    metadata_executor: self.metadata_executor,
                    signature_executor: self.signature_executor,
                    history_find_executor: self.history_find_executor,
                },
                self.thread_state,
                self.backend,
                repos,
                self.repo_task_tokens,
                self.thread_msg_tx.clone(),
                effect,
            );
        }
    }
}

/// Keeps a repository's file watcher running while it is not the active
/// repository. Leases count: the watcher stops when the last is dropped
/// (unless the repository is active). A lease never keeps the store alive.
#[must_use = "dropping the lease releases the watch"]
pub struct WorktreeWatchLease {
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    lifetime: u64,
    path: std::path::PathBuf,
}

impl Drop for WorktreeWatchLease {
    fn drop(&mut self) {
        self.msg_tx.dispatch(Msg::WatchWorktree {
            repo_id: self.repo_id,
            lifetime: self.lifetime,
            path: self.path.clone(),
            watch: false,
        });
    }
}

/// A counted watch of one open repository lifetime.
#[must_use = "dropping the lease releases the watch"]
pub struct WatchLease {
    msg_tx: StoreWorkerSender,
    repo_id: RepoId,
    lifetime: u64,
}

impl WatchLease {
    pub fn repo_id(&self) -> RepoId {
        self.repo_id
    }
}

impl Drop for WatchLease {
    fn drop(&mut self) {
        self.msg_tx.dispatch(Msg::ReleaseWatchLease {
            repo_id: self.repo_id,
            lifetime: self.lifetime,
        });
    }
}

impl std::fmt::Debug for WatchLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WatchLease")
            .field("repo_id", &self.repo_id)
            .field("lifetime", &self.lifetime)
            .finish()
    }
}

pub struct AppStore {
    state: Arc<RwLock<Arc<AppState>>>,
    /// Count of reducer passes that published state; see
    /// [`AppStore::snapshot_with_publication`].
    publication: Arc<AtomicU64>,
    msg_tx: StoreWorkerSender,
    public_lifetime: Arc<StorePublicLifetime>,
    backend: Arc<dyn GitBackend>,
}

struct StorePublicLifetime {
    msg_tx: StoreWorkerSender,
    #[cfg(any(test, feature = "test-support"))]
    history_find_dispatches: AtomicU64,
}

impl StorePublicLifetime {
    fn new(msg_tx: StoreWorkerSender) -> Self {
        Self {
            msg_tx,
            #[cfg(any(test, feature = "test-support"))]
            history_find_dispatches: AtomicU64::new(0),
        }
    }
}

impl Drop for StorePublicLifetime {
    fn drop(&mut self) {
        self.msg_tx.shutdown();
    }
}

impl Clone for AppStore {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            publication: Arc::clone(&self.publication),
            msg_tx: self.msg_tx.clone(),
            public_lifetime: Arc::clone(&self.public_lifetime),
            backend: Arc::clone(&self.backend),
        }
    }
}

impl AppStore {
    pub fn reducer_diagnostics() -> StoreReducerDiagnostics {
        reducer_diagnostics::snapshot()
    }

    pub fn new(backend: Arc<dyn GitBackend>) -> (Self, smol::channel::Receiver<StoreEvent>) {
        Self::with_initial_state(
            backend,
            AppState::default(),
            repository_preferences::PreferenceHub::shared(),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new_test(backend: Arc<dyn GitBackend>) -> (Self, smol::channel::Receiver<StoreEvent>) {
        Self::with_initial_state(
            backend,
            AppState::test_default(),
            repository_preferences::PreferenceHub::new(
                crate::session::default_session_file_path_for_effect(),
            ),
        )
    }

    #[cfg(any(test, feature = "test-support"))]
    pub fn new_test_sharing_preferences(
        backend: Arc<dyn GitBackend>,
        other: &Self,
    ) -> (Self, smol::channel::Receiver<StoreEvent>) {
        Self::with_initial_state(
            backend,
            AppState::test_default(),
            Arc::clone(&other.msg_tx.preferences),
        )
    }

    fn with_initial_state(
        backend: Arc<dyn GitBackend>,
        initial: AppState,
        preferences: Arc<repository_preferences::PreferenceHub>,
    ) -> (Self, smol::channel::Receiver<StoreEvent>) {
        let discovery_backend = Arc::clone(&backend);
        let state = Arc::new(RwLock::new(Arc::new(initial)));
        let (command_tx, command_rx) = mpsc::channel::<StoreWorkerCommand>();
        let store_id = StoreInstanceId::next();
        let store_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let msg_tx =
            StoreWorkerSender::new(command_tx, Arc::clone(&store_alive), store_id, preferences);
        // Coalesced "state changed" notifications: at most one pending.
        let (event_tx, event_rx) = smol::channel::bounded::<StoreEvent>(1);

        let thread_state = Arc::clone(&state);
        let thread_msg_tx = msg_tx.clone();
        let publication = Arc::new(AtomicU64::new(0));
        let thread_publication = Arc::clone(&publication);

        let worker = thread::Builder::new().name("gitcomet-store".into()).spawn(move || {
            // General effects share process-wide workers. Repository loads
            // need a bounded pool per window: blocking opens and filesystem
            // scans in one window must not consume another window's capacity.
            let executor = TaskExecutor::shared_for_store(
                StoreExecutorPool::Primary,
                default_worker_threads(),
            );
            let repo_load_executor =
                TaskExecutor::named("gitcomet-repo-load", repo_load_worker_threads());
            let worktree_scan_executor: std::sync::LazyLock<TaskExecutor> =
                std::sync::LazyLock::new(|| TaskExecutor::named(&WORKTREE_SCAN_THREAD_NAME, 1));
            let metadata_executor = TaskExecutor::shared_for_store(
                StoreExecutorPool::Metadata,
                metadata_worker_threads(),
            );
            let signature_executor =
                TaskExecutor::shared_for_store(StoreExecutorPool::Signatures, 1);
            let session_persist_executor =
                TaskExecutor::shared_for_store(StoreExecutorPool::SessionPersist, 1);
            // Find scans read all of history, so each window gets its own.
            let history_find_executor: std::sync::LazyLock<TaskExecutor> =
                std::sync::LazyLock::new(|| {
                    TaskExecutor::named(crate::history_find::HISTORY_FIND_THREAD, 1)
                });
            let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
            let mut repo_task_tokens: FxHashMap<RepoId, RepoTaskToken> = FxHashMap::default();
            let mut repo_monitors = RepoMonitorManager::new();
            let id_alloc = AtomicU64::new(1);
            let active_repo_id = Arc::new(AtomicU64::new(0));
            let mut deferred_commands = VecDeque::new();

            while let Ok(command) = recv_next_worker_command(&command_rx, &mut deferred_commands) {
                let (mut msg, stamp) = match command {
                    StoreWorkerCommand::Msg(msg) => (*msg, None),
                    StoreWorkerCommand::Traced(msg, stamp) => (*msg, Some(stamp)),
                    StoreWorkerCommand::Shutdown => break,
                    StoreWorkerCommand::Repository {
                        repo_id,
                        lifetime,
                        reply,
                    } => {
                        let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
                        let repository = state
                            .repos
                            .iter()
                            .any(|repo| repo.id == repo_id && repo.lifetime() == lifetime)
                            .then(|| repos.get(&repo_id).cloned())
                            .flatten();
                        let _ = reply.send(repository);
                        continue;
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    StoreWorkerCommand::InsertRepoForTest { repo_id, repo } => {
                        repos.insert(repo_id, repo);
                        continue;
                    }
                    #[cfg(any(test, feature = "test-support"))]
                    StoreWorkerCommand::DisableRepoMonitorsForTest => {
                        repo_monitors.disable();
                        continue;
                    }
                };

                if !thread_msg_tx.is_alive() {
                    continue;
                }

                thread_msg_tx.refresh_open_preferences(&mut msg);

                // Effects scheduled while handling this message inherit its
                // operation, and so do the messages their tasks send back.
                let _op_scope = stamp.map(|stamp| {
                    gitcomet_core::op_trace::record(
                        gitcomet_core::op_trace::Stage::Received,
                        stamp.op,
                        repo_load_trace::stage_label(&msg),
                        stamp.waited_ns(),
                        0,
                    );
                    gitcomet_core::op_trace::scope(stamp.op)
                });

                if repo_load_trace::enabled() {
                    let msg_repo_id = repo_load_trace::msg_repo_id(&msg);
                    let change_flags = repo_load_trace::msg_external_change(&msg).map(|change| {
                        format!(
                            "worktree={},index={},git_state={}",
                            change.worktree, change.index, change.git_state
                        )
                    });
                    repo_load_trace::trace!(
                        "worker received msg={} repo_id={:?} change={} active_repo={:?} queued_tokens={}",
                        repo_load_trace::msg_name(&msg),
                        msg_repo_id,
                        change_flags.as_deref().unwrap_or("-"),
                        thread_state
                            .read()
                            .unwrap_or_else(|e| e.into_inner())
                            .active_repo,
                        repo_task_tokens.len()
                    );
                }

                match &msg {
                    Msg::RestoreSession { .. } => {
                        repo_load_trace::trace!(
                            "restore_session cancelling_all_repo_load_tokens count={}",
                            repo_task_tokens.len()
                        );
                        repo_monitors.stop_all();
                        for token in repo_task_tokens.values() {
                            token.cancel();
                        }
                        repo_task_tokens.clear();
                    }
                    Msg::CloseRepo { repo_id } | Msg::MoveRepoOut { repo_id } => {
                        repo_monitors.stop(*repo_id);
                        if let Some(token) = repo_task_tokens.remove(repo_id) {
                            repo_load_trace::trace!(
                                "close_repo cancelling_repo_load_token repo_id={:?} load_epoch={}",
                                repo_id,
                                token.load_epoch
                            );
                            token.cancel();
                        }
                    }
                    Msg::CloseRepos { repo_ids, .. } => {
                        for repo_id in repo_ids {
                            repo_monitors.stop(*repo_id);
                            if let Some(token) = repo_task_tokens.remove(repo_id) {
                                repo_load_trace::trace!(
                                    "close_repos cancelling_repo_load_token repo_id={:?} load_epoch={}",
                                    repo_id,
                                    token.load_epoch
                                );
                                token.cancel();
                            }
                        }
                    }
                    Msg::RepoActivated { repo_id } => {
                        repo_load_trace::trace!(
                            "repo_activated_full_refresh repo_id={:?} monitor_running={}",
                            repo_id,
                            repo_monitors.is_running(*repo_id)
                        );
                    }
                    Msg::Internal(crate::msg::InternalMsg::RepoLoadFinished {
                        repo_id,
                        load_epoch,
                        message,
                    }) if matches!(
                        message.as_ref(),
                        crate::msg::InternalMsg::RepoOpenedErr { .. }
                    ) && repo_task_tokens
                        .get(repo_id)
                        .is_some_and(|token| token.load_epoch == *load_epoch) =>
                    {
                        repo_load_trace::trace!(
                            "repo_opened_err removing_repo_load_token repo_id={:?} load_epoch={}",
                            repo_id,
                            load_epoch
                        );
                        repo_task_tokens.remove(repo_id);
                    }
                    _ => {}
                }

                let mut worker_ctx = WorkerLoopContext {
                    thread_state: &thread_state,
                    active_repo_id: &active_repo_id,
                    event_tx: &event_tx,
                    repo_monitors: &mut repo_monitors,
                    repo_task_tokens: &mut repo_task_tokens,
                    thread_msg_tx: &thread_msg_tx,
                    executor: &executor,
                    repo_load_executor: &repo_load_executor,
                    worktree_scan_executor: &worktree_scan_executor,
                    metadata_executor: &metadata_executor,
                    signature_executor: &signature_executor,
                    history_find_executor: &history_find_executor,
                    session_persist_executor: &session_persist_executor,
                    backend: &backend,
                    publication: &thread_publication,
                };

                match msg {
                    Msg::SetActiveRepo { repo_id } => {
                        worker_ctx.reduce_and_handle(
                            &mut repos,
                            &id_alloc,
                            |app_state, repos, _| {
                                let mut effects = reducer::SetActiveRepoEffects::new();
                                fill_set_active_repo_inline(
                                    repos,
                                    app_state,
                                    repo_id,
                                    &mut effects,
                                );
                                effects
                            },
                        );
                    }
                    Msg::ReorderRepoTabs {
                        repo_id,
                        insert_before,
                    } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            let mut effects = reducer::ReorderRepoTabsEffects::new();
                            fill_reorder_repo_tabs_inline(
                                app_state,
                                repo_id,
                                insert_before,
                                &mut effects,
                            );
                            effects
                        });
                    }
                    Msg::StagePath { repo_id, path } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            let mut effects = reducer::SinglePathActionEffects::new();
                            fill_stage_path_inline(app_state, repo_id, path, &mut effects);
                            effects
                        });
                    }
                    Msg::StagePaths { repo_id, paths } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            let mut effects = reducer::BatchPathActionEffects::new();
                            fill_stage_paths_inline(app_state, repo_id, paths, &mut effects);
                            effects
                        });
                    }
                    Msg::UnstagePath { repo_id, path } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            let mut effects = reducer::SinglePathActionEffects::new();
                            fill_unstage_path_inline(app_state, repo_id, path, &mut effects);
                            effects
                        });
                    }
                    Msg::UnstagePaths { repo_id, paths } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            let mut effects = reducer::BatchPathActionEffects::new();
                            fill_unstage_paths_inline(app_state, repo_id, paths, &mut effects);
                            effects
                        });
                    }
                    Msg::ConflictSetRegionChoice {
                        repo_id,
                        path,
                        region_index,
                        choice,
                    } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            set_conflict_region_choice_inline(
                                app_state,
                                repo_id,
                                path,
                                region_index,
                                choice,
                            );
                            Vec::<crate::msg::Effect>::new()
                        });
                    }
                    Msg::ConflictResetResolutions { repo_id, path } => {
                        worker_ctx.reduce_and_handle(&mut repos, &id_alloc, |app_state, _, _| {
                            reset_conflict_resolutions_inline(app_state, repo_id, path);
                            Vec::<crate::msg::Effect>::new()
                        });
                    }
                    Msg::RepoActivated { repo_id } => {
                        // Do a FULL refresh on activation (window focus). The filesystem monitor is
                        // best-effort and cannot be the sole refresh trigger: in sandboxed/Flatpak
                        // runs an external editor's or terminal's writes to the bind-mounted repo do
                        // not propagate inotify events into the sandbox, so the monitor — even when
                        // its thread is "running" — sees neither worktree edits NOR git-state changes
                        // (commits, checkouts, fetches). Refreshing only the working-changes lanes
                        // here would leave the log/branches/HEAD/divergence stale forever in exactly
                        // that case. A full refresh keeps every view correct regardless of whether,
                        // or how reliably, the watcher is delivering events; activation is throttled
                        // upstream (REPO_ACTIVATION_THROTTLE), so this does not run on every alt-tab.
                        worker_ctx.repo_monitors.revalidate(repo_id);
                        let change = RepoExternalChange::all();
                        worker_ctx.reduce_and_handle(
                            &mut repos,
                            &id_alloc,
                            |app_state, repos, id| {
                                let mut effects = reduce(
                                    repos,
                                    id,
                                    app_state,
                                    Msg::RepoExternallyChanged { repo_id, change },
                                );
                                // A day-long session gets its daily check on
                                // focus; the reducer throttles it hourly.
                                effects.extend(reducer::maintenance::request_check(
                                    app_state, repo_id,
                                ));
                                effects
                            },
                        );
                    }
                    msg => {
                        worker_ctx.reduce_and_handle(
                            &mut repos,
                            &id_alloc,
                            |app_state, repos, id| reduce(repos, id, app_state, msg),
                        );
                    }
                }
            }

            for token in repo_task_tokens.values() {
                token.cancel();
            }
            for repo in &thread_state.read().unwrap_or_else(|e| e.into_inner()).repos {
                repo.history_state.commit_signatures_cancellation.cancel();
            }
            repo_monitors.stop_all();
        });
        worker.expect("spawn store worker thread");

        (
            Self {
                state,
                publication,
                msg_tx: msg_tx.clone(),
                public_lifetime: Arc::new(StorePublicLifetime::new(msg_tx)),
                backend: discovery_backend,
            },
            event_rx,
        )
    }

    pub fn dispatch(&self, msg: Msg) {
        #[cfg(any(test, feature = "test-support"))]
        if matches!(
            &msg,
            Msg::HistoryFind(crate::history_find::HistoryFindMsg::Find { .. })
        ) {
            self.public_lifetime
                .history_find_dispatches
                .fetch_add(1, Ordering::Relaxed);
        }
        self.msg_tx.dispatch(msg);
    }

    /// Probe the closest repository boundary, including nested repos and .git
    /// files used by linked worktrees. Backend opening validates the boundary.
    pub fn discover_file_repository(
        &self,
        file: &std::path::Path,
    ) -> gitcomet_core::services::Result<Option<PathBuf>> {
        let file = gitcomet_core::filesystem::absolute_identity(file).map_err(|e| {
            gitcomet_core::error::Error::new(gitcomet_core::error::ErrorKind::Io(e.kind()))
        })?;
        for parent in file.parent().into_iter().flat_map(|p| p.ancestors()) {
            if parent.join(".git").symlink_metadata().is_ok() {
                match self.backend.open(parent) {
                    Ok(repository) => return Ok(Some(repository.spec().workdir.clone())),
                    Err(error)
                        if matches!(
                            error.kind(),
                            gitcomet_core::error::ErrorKind::NotARepository
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
        }
        Ok(None)
    }

    /// Watches this lifetime of `repo_id` until the lease is dropped. Closing
    /// and reopening the repository never transfers an outstanding lease.
    pub fn watch_worktree(
        &self,
        repo_id: RepoId,
        lifetime: u64,
        path: std::path::PathBuf,
    ) -> WorktreeWatchLease {
        self.msg_tx.dispatch(Msg::WatchWorktree {
            repo_id,
            lifetime,
            path: path.clone(),
            watch: true,
        });
        WorktreeWatchLease {
            msg_tx: self.msg_tx.clone(),
            repo_id,
            lifetime,
            path,
        }
    }

    pub fn watch_repository(&self, repo_id: RepoId, lifetime: u64) -> WatchLease {
        self.msg_tx
            .dispatch(Msg::AcquireWatchLease { repo_id, lifetime });
        WatchLease {
            msg_tx: self.msg_tx.clone(),
            repo_id,
            lifetime,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn history_find_dispatch_count_for_test(&self) -> u64 {
        self.public_lifetime
            .history_find_dispatches
            .load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> Arc<AppState> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        Arc::clone(&state)
    }

    /// Reads a backend handle for this repository lifetime on a worker thread.
    /// This waits for the store worker; do not call from the UI thread or a reducer.
    /// An old handle never resolves to a reopened repository with the same id.
    pub fn repository(&self, repo_id: RepoId, lifetime: u64) -> Option<Arc<dyn GitRepository>> {
        self.msg_tx.repository(repo_id, lifetime)
    }

    /// [`Self::snapshot`] without blocking: `None` while the reducer holds
    /// the write lock, so a UI thread can take the common uncontended case
    /// inline and fall back to a background hop only when it would wait.
    pub fn try_snapshot(&self) -> Option<Arc<AppState>> {
        match self.state.try_read() {
            Ok(state) => Some(Arc::clone(&state)),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                Some(Arc::clone(&poisoned.into_inner()))
            }
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }

    /// [`Self::try_snapshot`] plus the publication sequence number of that
    /// exact state, for correlating UI application with reducer passes.
    pub fn try_snapshot_with_publication(&self) -> Option<(Arc<AppState>, u64)> {
        let state = match self.state.try_read() {
            Ok(state) => state,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        Some((Arc::clone(&state), self.publication.load(Ordering::Relaxed)))
    }

    /// [`Self::snapshot`] plus its publication sequence number.
    pub fn snapshot_with_publication(&self) -> (Arc<AppState>, u64) {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        (Arc::clone(&state), self.publication.load(Ordering::Relaxed))
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn replace_snapshot_for_test(&self, state: Arc<AppState>) {
        let mut current = self.state.write().unwrap_or_else(|e| e.into_inner());
        *current = state;
    }

    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn insert_repo_for_test(&self, repo_id: RepoId, repo: Arc<dyn GitRepository>) {
        self.msg_tx.insert_repo_for_test(repo_id, repo);
    }

    /// Never starts a filesystem monitor for any repository. For tests over
    /// real repositories whose assertions a watcher refresh would disturb.
    #[cfg(any(test, feature = "test-support"))]
    #[doc(hidden)]
    pub fn disable_repo_monitors_for_test(&self) {
        self.msg_tx.disable_repo_monitors_for_test();
    }
}

#[cfg(feature = "benchmarks")]
pub fn dispatch_sync_for_bench(state: &mut AppState, msg: Msg) -> Vec<crate::msg::Effect> {
    let mut repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let id_alloc = AtomicU64::new(1);
    let reduce_started = Instant::now();
    let effects = reduce(&mut repos, &id_alloc, state, msg);
    reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
    effects
}

#[cfg(feature = "benchmarks")]
pub(crate) fn with_set_active_repo_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let repos = FxHashMap::default();
    let mut effects = reducer::SetActiveRepoEffects::new();
    fill_set_active_repo_inline(&repos, state, repo_id, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
pub(crate) fn with_reorder_repo_tabs_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    insert_before: Option<RepoId>,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let mut effects = reducer::ReorderRepoTabsEffects::new();
    fill_reorder_repo_tabs_inline(state, repo_id, insert_before, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
pub(crate) fn with_select_diff_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    target: gitcomet_core::domain::DiffTarget,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let repos = FxHashMap::default();
    let mut effects = reducer::SelectDiffEffects::new();
    fill_select_diff_inline(&repos, state, repo_id, target, false, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn with_stage_path_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let mut effects = reducer::SinglePathActionEffects::new();
    fill_stage_path_inline(state, repo_id, path, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn with_stage_paths_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    paths: crate::msg::RepoPathList,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let mut effects = reducer::BatchPathActionEffects::new();
    fill_stage_paths_inline(state, repo_id, paths, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn with_unstage_path_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    path: PathBuf,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let mut effects = reducer::SinglePathActionEffects::new();
    fill_unstage_path_inline(state, repo_id, path, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn with_unstage_paths_inline_for_bench<T>(
    state: &mut AppState,
    repo_id: RepoId,
    paths: crate::msg::RepoPathList,
    f: impl FnOnce(&AppState, &[crate::msg::Effect]) -> T,
) -> T {
    let mut effects = reducer::BatchPathActionEffects::new();
    fill_unstage_paths_inline(state, repo_id, paths, &mut effects);
    f(state, &effects)
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn set_conflict_region_choice_inline_for_bench(
    state: &mut AppState,
    repo_id: RepoId,
    path: crate::msg::RepoPath,
    region_index: usize,
    choice: crate::msg::ConflictRegionChoice,
) {
    set_conflict_region_choice_inline(state, repo_id, path, region_index, choice);
}

#[cfg(feature = "benchmarks")]
#[inline]
pub(crate) fn reset_conflict_resolutions_inline_for_bench(
    state: &mut AppState,
    repo_id: RepoId,
    path: crate::msg::RepoPath,
) {
    reset_conflict_resolutions_inline(state, repo_id, path);
}

#[cfg(test)]
mod path_tests {
    use super::canonicalize_path;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_temp_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "gitcomet-state-{label}-{}-{suffix}",
            std::process::id()
        ))
    }

    #[test]
    fn canonicalize_path_keeps_missing_path() {
        let missing = unique_temp_path("missing");
        let _ = fs::remove_file(&missing);
        let _ = fs::remove_dir_all(&missing);

        assert_eq!(canonicalize_path(missing.clone()), missing);
    }

    #[test]
    fn canonicalize_path_resolves_existing_path() {
        let root = unique_temp_path("existing");
        let nested = root.join("nested");
        fs::create_dir_all(&nested).expect("test directory to be created");

        let input = nested.join("..");
        let actual = canonicalize_path(input);

        #[cfg(not(windows))]
        {
            let expected = fs::canonicalize(&root).expect("canonical path for existing directory");
            assert_eq!(actual, expected);
        }

        #[cfg(windows)]
        {
            use std::path::{Component, Prefix};

            assert_eq!(actual.file_name(), root.file_name());
            let has_verbatim_prefix = matches!(
                actual.components().next(),
                Some(Component::Prefix(prefix))
                    if matches!(
                        prefix.kind(),
                        Prefix::Verbatim(_)
                            | Prefix::VerbatimDisk(_)
                            | Prefix::VerbatimUNC(_, _)
                    )
            );
            assert!(!has_verbatim_prefix);
        }

        let _ = fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod tests;
