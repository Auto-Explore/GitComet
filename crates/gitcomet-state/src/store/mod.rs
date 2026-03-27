use crate::model::{AppState, RepoId};
use crate::msg::{Msg, StoreEvent};
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
use reducer::reduce;
use repo_monitor::RepoMonitorManager;
use send_diagnostics::{SendFailureKind, send_or_log, try_send_state_changed_or_log};

pub use reducer_diagnostics::StoreReducerDiagnostics;

fn canonicalize_path(path: PathBuf) -> PathBuf {
    strip_windows_verbatim_prefix(std::fs::canonicalize(&path).unwrap_or(path))
}

#[cfg(windows)]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    use std::path::{Component, Prefix};

    let mut components = path.components();
    let Some(Component::Prefix(prefix)) = components.next() else {
        return path;
    };

    let mut out = match prefix.kind() {
        Prefix::VerbatimDisk(letter) => PathBuf::from(format!("{}:", char::from(letter))),
        Prefix::VerbatimUNC(server, share) => {
            let mut out = PathBuf::from(r"\\");
            out.push(server);
            out.push(share);
            out
        }
        Prefix::Verbatim(raw) => PathBuf::from(raw),
        _ => return path,
    };

    for component in components {
        out.push(component.as_os_str());
    }
    out
}

#[cfg(not(windows))]
fn strip_windows_verbatim_prefix(path: PathBuf) -> PathBuf {
    path
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

                let effects = {
                    let mut app_state = thread_state.write().unwrap_or_else(|e| e.into_inner());
                    let app_state = make_mut_state_with_diagnostics(&mut app_state);
                    let reduce_started = Instant::now();
                    let effects = reduce(&mut repos, &id_alloc, app_state, msg);
                    reducer_diagnostics::record_reducer_pass(reduce_started.elapsed());
                    effects
                };

                let active_value = thread_state
                    .read()
                    .unwrap_or_else(|e| e.into_inner())
                    .active_repo
                    .map(|id| id.0)
                    .unwrap_or(0);
                active_repo_id.store(active_value, Ordering::Relaxed);

                try_send_state_changed_or_log(&event_tx, "store worker loop state notification");

                // Keep filesystem monitoring scoped to the active repository only, to minimize
                // OS watcher load in large multi-repo sessions.
                let (active_repo, active_workdir) = {
                    let state = thread_state.read().unwrap_or_else(|e| e.into_inner());
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
                    schedule_effect(
                        &executor,
                        &session_persist_executor,
                        &backend,
                        &repos,
                        thread_msg_tx.clone(),
                        effect,
                    );
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
    reduce(&mut repos, &id_alloc, state, msg)
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
