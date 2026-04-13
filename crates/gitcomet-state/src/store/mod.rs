use crate::model::{AppState, RepoId};
use crate::msg::{Msg, StoreEvent};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{GitBackend, GitRepository};
use rustc_hash::FxHashMap as HashMap;
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
mod repo_monitor;
mod send_diagnostics;

use effects::schedule_effect;
use executor::{TaskExecutor, default_worker_threads};
use reducer::{
    fill_reorder_repo_tabs_inline, fill_select_diff_inline, fill_set_active_repo_inline,
    fill_stage_path_inline, fill_stage_paths_inline, fill_unstage_path_inline,
    fill_unstage_paths_inline, reduce, reset_conflict_resolutions_inline,
    set_conflict_region_choice_inline,
};
use repo_monitor::RepoMonitorManager;
use send_diagnostics::{SendFailureKind, send_or_log, try_send_state_changed_or_log};

pub use reducer_diagnostics::StoreReducerDiagnostics;

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

struct ReducerEffectsContext<'a> {
    thread_state: &'a Arc<RwLock<Arc<AppState>>>,
    active_repo_id: &'a Arc<AtomicU64>,
    event_tx: &'a smol::channel::Sender<StoreEvent>,
    repo_monitors: &'a mut RepoMonitorManager,
    repos: &'a HashMap<RepoId, Arc<dyn GitRepository>>,
    thread_msg_tx: &'a mpsc::Sender<Msg>,
    executor: &'a TaskExecutor,
    session_persist_executor: &'a TaskExecutor,
    backend: &'a Arc<dyn GitBackend>,
}

fn handle_reducer_effects<I>(effects: I, ctx: ReducerEffectsContext<'_>)
where
    I: IntoIterator<Item = crate::msg::Effect>,
{
    let active_value = ctx
        .thread_state
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .active_repo
        .map(|id| id.0)
        .unwrap_or(0);
    ctx.active_repo_id.store(active_value, Ordering::Relaxed);

    try_send_state_changed_or_log(ctx.event_tx, "store worker loop state notification");

    // Keep filesystem monitoring scoped to the active repository only, to minimize
    // OS watcher load in large multi-repo sessions.
    let (active_repo, active_workdir) = {
        let state = ctx.thread_state.read().unwrap_or_else(|e| e.into_inner());
        let active_repo = state.active_repo;
        let active_workdir = active_repo.and_then(|repo_id| {
            state
                .repos
                .iter()
                .find(|r| r.id == repo_id)
                .map(|r| r.spec.workdir.clone())
        });
        (active_repo, active_workdir)
    };

    for repo_id in ctx.repo_monitors.running_repo_ids() {
        if Some(repo_id) != active_repo {
            ctx.repo_monitors.stop(repo_id);
        }
    }

    if let Some(repo_id) = active_repo
        && let Some(workdir) = active_workdir
        && ctx.repos.contains_key(&repo_id)
    {
        ctx.repo_monitors.start(
            repo_id,
            workdir,
            ctx.thread_msg_tx.clone(),
            Arc::clone(ctx.active_repo_id),
        );
    }

    for effect in effects {
        schedule_effect(
            ctx.executor,
            ctx.session_persist_executor,
            ctx.thread_state,
            ctx.backend,
            ctx.repos,
            ctx.thread_msg_tx.clone(),
            effect,
        );
    }
}

pub struct AppStore {
    state: Arc<RwLock<Arc<AppState>>>,
    msg_tx: mpsc::Sender<Msg>,
}

impl Clone for AppStore {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            msg_tx: self.msg_tx.clone(),
        }
    }
}

impl AppStore {
    pub fn reducer_diagnostics() -> StoreReducerDiagnostics {
        reducer_diagnostics::snapshot()
    }

    pub fn new(backend: Arc<dyn GitBackend>) -> (Self, smol::channel::Receiver<StoreEvent>) {
        let state = Arc::new(RwLock::new(Arc::new(AppState::default())));
        let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
        // Coalesced "state changed" notifications: at most one pending.
        let (event_tx, event_rx) = smol::channel::bounded::<StoreEvent>(1);

        let thread_state = Arc::clone(&state);
        let thread_msg_tx = msg_tx.clone();

        thread::spawn(move || {
            let executor = TaskExecutor::new(default_worker_threads());
            let session_persist_executor = TaskExecutor::new(1);
            let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
            let mut repo_monitors = RepoMonitorManager::new();
            let id_alloc = AtomicU64::new(1);
            let active_repo_id = Arc::new(AtomicU64::new(0));

            while let Ok(msg) = msg_rx.recv() {
                match &msg {
                    Msg::RestoreSession { .. } => repo_monitors.stop_all(),
                    Msg::CloseRepo { repo_id } => repo_monitors.stop(*repo_id),
                    _ => {}
                }

                match msg {
                    Msg::SetActiveRepo { repo_id } => {
                        let mut effects = reducer::SetActiveRepoEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_set_active_repo_inline(app_state, repo_id, &mut effects);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::ReorderRepoTabs {
                        repo_id,
                        insert_before,
                    } => {
                        let mut effects = reducer::ReorderRepoTabsEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_reorder_repo_tabs_inline(
                                app_state,
                                repo_id,
                                insert_before,
                                &mut effects,
                            );
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::SelectDiff { repo_id, target } => {
                        let mut effects = reducer::SelectDiffEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_select_diff_inline(app_state, repo_id, target, &mut effects);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::StagePath { repo_id, path } => {
                        let mut effects = reducer::SinglePathActionEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_stage_path_inline(app_state, repo_id, path, &mut effects);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::StagePaths { repo_id, paths } => {
                        let mut effects = reducer::BatchPathActionEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_stage_paths_inline(app_state, repo_id, paths, &mut effects);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::UnstagePath { repo_id, path } => {
                        let mut effects = reducer::SinglePathActionEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_unstage_path_inline(app_state, repo_id, path, &mut effects);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::UnstagePaths { repo_id, paths } => {
                        let mut effects = reducer::BatchPathActionEffects::new();
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            fill_unstage_paths_inline(app_state, repo_id, paths, &mut effects);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::ConflictSetRegionChoice {
                        repo_id,
                        path,
                        region_index,
                        choice,
                    } => {
                        {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            set_conflict_region_choice_inline(
                                app_state,
                                repo_id,
                                path,
                                region_index,
                                choice,
                            );
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                        }
                        handle_reducer_effects(
                            std::iter::empty::<crate::msg::Effect>(),
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::ConflictResetResolutions { repo_id, path } => {
                        {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            reset_conflict_resolutions_inline(app_state, repo_id, path);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                        }
                        handle_reducer_effects(
                            std::iter::empty::<crate::msg::Effect>(),
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    msg => {
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            let effects = reduce(&mut repos, &id_alloc, app_state, msg);
                            reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                            effects
                        };
                        handle_reducer_effects(
                            effects,
                            ReducerEffectsContext {
                                thread_state: &thread_state,
                                active_repo_id: &active_repo_id,
                                event_tx: &event_tx,
                                repo_monitors: &mut repo_monitors,
                                repos: &repos,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                }
            }
        });

        (Self { state, msg_tx }, event_rx)
    }

    pub fn dispatch(&self, msg: Msg) {
        send_or_log(
            &self.msg_tx,
            msg,
            SendFailureKind::StoreDispatch,
            "AppStore::dispatch",
        );
    }

    pub fn snapshot(&self) -> Arc<AppState> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        Arc::clone(&state)
    }
}

#[cfg(feature = "benchmarks")]
pub fn dispatch_sync_for_bench(state: &mut AppState, msg: Msg) -> Vec<crate::msg::Effect> {
    let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
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
    let mut effects = reducer::SetActiveRepoEffects::new();
    fill_set_active_repo_inline(state, repo_id, &mut effects);
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
    let mut effects = reducer::SelectDiffEffects::new();
    fill_select_diff_inline(state, repo_id, target, &mut effects);
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
