use crate::model::{AppState, RepoId};
use crate::msg::{Msg, RepoExternalChange, StoreEvent};
use gitcomet_core::path_utils::canonicalize_or_original;
use gitcomet_core::services::{GitBackend, GitRepository};
use rustc_hash::FxHashMap as HashMap;
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
mod send_diagnostics;
mod worker_channel;

use effects::RepoTaskToken;
use effects::{EffectExecutors, schedule_effect};
use executor::{TaskExecutor, default_worker_threads};
use reducer::{
    fill_reorder_repo_tabs_inline, fill_select_diff_inline, fill_set_active_repo_inline,
    fill_stage_path_inline, fill_stage_paths_inline, fill_unstage_path_inline,
    fill_unstage_paths_inline, reduce, reset_conflict_resolutions_inline,
    set_conflict_region_choice_inline,
};
use repo_monitor::RepoMonitorManager;
use send_diagnostics::try_send_state_changed_or_log;
use worker_channel::{StoreInstanceId, StoreWorkerCommand, StoreWorkerSender};

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

fn is_control_msg(msg: &Msg) -> bool {
    matches!(
        msg,
        Msg::OpenRepo(_)
            | Msg::CloseRepo { .. }
            | Msg::SetActiveRepo { .. }
            | Msg::ReorderRepoTabs { .. }
    )
}

fn is_control_command(command: &StoreWorkerCommand) -> bool {
    match command {
        StoreWorkerCommand::Msg(msg) => is_control_msg(msg),
        StoreWorkerCommand::Shutdown => true,
        #[cfg(any(test, feature = "test-support"))]
        StoreWorkerCommand::InsertRepoForTest { .. } => true,
    }
}

fn can_control_command_overtake(command: &StoreWorkerCommand) -> bool {
    matches!(
        command,
        StoreWorkerCommand::Msg(msg) if matches!(msg.as_ref(), Msg::Internal(_))
    )
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

struct ReducerEffectsContext<'a> {
    thread_state: &'a Arc<RwLock<Arc<AppState>>>,
    active_repo_id: &'a Arc<AtomicU64>,
    event_tx: &'a smol::channel::Sender<StoreEvent>,
    repo_monitors: &'a mut RepoMonitorManager,
    repos: &'a HashMap<RepoId, Arc<dyn GitRepository>>,
    repo_task_tokens: &'a mut HashMap<RepoId, RepoTaskToken>,
    thread_msg_tx: &'a StoreWorkerSender,
    executor: &'a TaskExecutor,
    repo_load_executor: &'a TaskExecutor,
    metadata_executor: &'a TaskExecutor,
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

    try_send_state_changed_or_log(
        ctx.event_tx,
        "store worker loop state notification",
        ctx.thread_msg_tx.store_id(),
        ctx.thread_msg_tx.is_alive(),
    );

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
        if repo_load_trace::enabled() {
            let effect_repo_id = repo_load_trace::effect_repo_id(&effect);
            let (load_epoch, workdir) = effect_repo_id.map_or((None, None), |repo_id| {
                let state = ctx.thread_state.read().unwrap_or_else(|e| e.into_inner());
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
        schedule_effect(
            EffectExecutors {
                executor: ctx.executor,
                repo_load_executor: ctx.repo_load_executor,
                session_persist_executor: ctx.session_persist_executor,
                metadata_executor: ctx.metadata_executor,
            },
            ctx.thread_state,
            ctx.backend,
            ctx.repos,
            ctx.repo_task_tokens,
            ctx.thread_msg_tx.clone(),
            effect,
        );
    }
}

pub struct AppStore {
    state: Arc<RwLock<Arc<AppState>>>,
    msg_tx: StoreWorkerSender,
    public_lifetime: Arc<StorePublicLifetime>,
}

struct StorePublicLifetime {
    msg_tx: StoreWorkerSender,
}

impl StorePublicLifetime {
    fn new(msg_tx: StoreWorkerSender) -> Self {
        Self { msg_tx }
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
            msg_tx: self.msg_tx.clone(),
            public_lifetime: Arc::clone(&self.public_lifetime),
        }
    }
}

impl AppStore {
    pub fn reducer_diagnostics() -> StoreReducerDiagnostics {
        reducer_diagnostics::snapshot()
    }

    pub fn new(backend: Arc<dyn GitBackend>) -> (Self, smol::channel::Receiver<StoreEvent>) {
        let state = Arc::new(RwLock::new(Arc::new(AppState::default())));
        let (command_tx, command_rx) = mpsc::channel::<StoreWorkerCommand>();
        let store_id = StoreInstanceId::next();
        let store_alive = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let msg_tx = StoreWorkerSender::new(command_tx, Arc::clone(&store_alive), store_id);
        // Coalesced "state changed" notifications: at most one pending.
        let (event_tx, event_rx) = smol::channel::bounded::<StoreEvent>(1);

        let thread_state = Arc::clone(&state);
        let thread_msg_tx = msg_tx.clone();

        thread::spawn(move || {
            let executor = TaskExecutor::new(default_worker_threads());
            let repo_load_executor = TaskExecutor::new(default_worker_threads());
            let metadata_executor = TaskExecutor::new(1);
            let session_persist_executor = TaskExecutor::new(1);
            let mut repos: HashMap<RepoId, Arc<dyn GitRepository>> = HashMap::default();
            let mut repo_task_tokens: HashMap<RepoId, RepoTaskToken> = HashMap::default();
            let mut repo_monitors = RepoMonitorManager::new();
            let id_alloc = AtomicU64::new(1);
            let active_repo_id = Arc::new(AtomicU64::new(0));
            let mut deferred_commands = VecDeque::new();

            while let Ok(command) = recv_next_worker_command(&command_rx, &mut deferred_commands) {
                let msg = match command {
                    StoreWorkerCommand::Msg(msg) => *msg,
                    StoreWorkerCommand::Shutdown => break,
                    #[cfg(any(test, feature = "test-support"))]
                    StoreWorkerCommand::InsertRepoForTest { repo_id, repo } => {
                        repos.insert(repo_id, repo);
                        continue;
                    }
                };

                if !thread_msg_tx.is_alive() {
                    continue;
                }

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
                            token.cancellation.cancel();
                        }
                        repo_task_tokens.clear();
                    }
                    Msg::CloseRepo { repo_id } => {
                        repo_monitors.stop(*repo_id);
                        if let Some(token) = repo_task_tokens.remove(repo_id) {
                            repo_load_trace::trace!(
                                "close_repo cancelling_repo_load_token repo_id={:?} load_epoch={}",
                                repo_id,
                                token.load_epoch
                            );
                            token.cancellation.cancel();
                        }
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                    Msg::RepoActivated { repo_id } => {
                        if repo_monitors.is_running(repo_id) {
                            repo_load_trace::trace!(
                                "repo_activated_monitor_active_skip_refresh repo_id={:?} monitor_running=true",
                                repo_id
                            );
                            continue;
                        }

                        repo_load_trace::trace!(
                            "repo_activated_monitor_unavailable_fallback_refresh repo_id={:?} monitor_running=false",
                            repo_id
                        );
                        let effects = {
                            let mut app_state =
                                thread_state.write().unwrap_or_else(|e| e.into_inner());
                            let app_state = make_mut_state_with_diagnostics(&mut app_state);
                            let reduce_started = Instant::now();
                            let effects = reduce(
                                &mut repos,
                                &id_alloc,
                                app_state,
                                Msg::RepoExternallyChanged {
                                    repo_id,
                                    change: RepoExternalChange::GitState,
                                },
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
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
                                repo_task_tokens: &mut repo_task_tokens,
                                thread_msg_tx: &thread_msg_tx,
                                executor: &executor,
                                repo_load_executor: &repo_load_executor,
                                metadata_executor: &metadata_executor,
                                session_persist_executor: &session_persist_executor,
                                backend: &backend,
                            },
                        );
                    }
                }
            }

            for token in repo_task_tokens.values() {
                token.cancellation.cancel();
            }
            repo_monitors.stop_all();
        });

        (
            Self {
                state,
                msg_tx: msg_tx.clone(),
                public_lifetime: Arc::new(StorePublicLifetime::new(msg_tx)),
            },
            event_rx,
        )
    }

    pub fn dispatch(&self, msg: Msg) {
        self.msg_tx.dispatch(msg);
    }

    pub fn snapshot(&self) -> Arc<AppState> {
        let state = self.state.read().unwrap_or_else(|e| e.into_inner());
        Arc::clone(&state)
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
