use crate::model::{AppState, RepoId};
use crate::msg::{Msg, StoreEvent};
use gitcomet_core::services::{GitBackend, GitRepository};
use rustc_hash::FxHashMap as HashMap;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, RwLock, mpsc};
use std::thread;

mod effects;
mod executor;
mod reducer;
mod repo_monitor;
mod send_diagnostics;

use effects::schedule_effect;
use executor::{TaskExecutor, default_worker_threads};
use reducer::reduce;
use repo_monitor::RepoMonitorManager;
use send_diagnostics::{SendFailureKind, send_or_log, try_send_state_changed_or_log};

pub struct AppStore {
    state: Arc<RwLock<AppState>>,
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
    pub fn new(backend: Arc<dyn GitBackend>) -> (Self, smol::channel::Receiver<StoreEvent>) {
        let state = Arc::new(RwLock::new(AppState::default()));
        let (msg_tx, msg_rx) = mpsc::channel::<Msg>();
        // Coalesced "state changed" notifications: at most one pending.
        let (event_tx, event_rx) = smol::channel::bounded::<StoreEvent>(1);

        let thread_state = Arc::clone(&state);
        let thread_msg_tx = msg_tx.clone();

        thread::spawn(move || {
            let executor = TaskExecutor::new(default_worker_threads());
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

                let effects = {
                    let mut app_state = thread_state.write().expect("state lock poisoned (write)");

                    reduce(&mut repos, &id_alloc, &mut app_state, msg)
                };

                let active_value = thread_state
                    .read()
                    .expect("state lock poisoned (read)")
                    .active_repo
                    .map(|id| id.0)
                    .unwrap_or(0);
                active_repo_id.store(active_value, Ordering::Relaxed);

                try_send_state_changed_or_log(&event_tx, "store worker loop state notification");

                // Keep filesystem monitoring scoped to the active repository only, to minimize
                // OS watcher load in large multi-repo sessions.
                let (active_repo, active_workdir) = {
                    let state = thread_state.read().expect("state lock poisoned (read)");
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

                for repo_id in repo_monitors.running_repo_ids() {
                    if Some(repo_id) != active_repo {
                        repo_monitors.stop(repo_id);
                    }
                }

                if let Some(repo_id) = active_repo
                    && let Some(workdir) = active_workdir
                    && repos.contains_key(&repo_id)
                {
                    repo_monitors.start(
                        repo_id,
                        workdir,
                        thread_msg_tx.clone(),
                        Arc::clone(&active_repo_id),
                    );
                }

                for effect in effects {
                    schedule_effect(&executor, &backend, &repos, thread_msg_tx.clone(), effect);
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

    pub fn snapshot(&self) -> AppState {
        self.state
            .read()
            .expect("state lock poisoned (read)")
            .clone()
    }
}

#[cfg(test)]
mod tests;
