use super::send_diagnostics::{SendFailureKind, send_or_log};
use gitcomet_core::mergetool_trace;
use std::panic::{self, AssertUnwindSafe};
#[cfg(any(test, feature = "test-support"))]
use std::sync::OnceLock;
use std::sync::{Arc, mpsc};
use std::thread;

type Task = Box<dyn FnOnce() + Send + 'static>;

pub(super) fn default_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 8))
        .unwrap_or(2)
}

pub(super) fn repo_load_worker_threads() -> usize {
    default_worker_threads().saturating_sub(1).clamp(1, 2)
}

pub(super) fn metadata_worker_threads() -> usize {
    2
}

#[cfg(any(test, feature = "test-support"))]
#[derive(Clone, Copy)]
pub(super) enum StoreExecutorPool {
    Primary,
    RepoLoad,
    Metadata,
    SessionPersist,
}

pub(super) struct TaskExecutor {
    tx: mpsc::Sender<Task>,
    _threads: Vec<thread::JoinHandle<()>>,
}

fn worker_loop(rx: Arc<std::sync::Mutex<mpsc::Receiver<Task>>>, catch_panics: bool) {
    loop {
        let task = {
            let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
            rx.recv()
        };
        match task {
            Ok(task) => {
                if catch_panics {
                    let _ = panic::catch_unwind(AssertUnwindSafe(task));
                } else {
                    task();
                }
            }
            Err(_) => break,
        }
    }
}

impl TaskExecutor {
    #[cfg_attr(feature = "test-support", allow(dead_code))]
    pub(super) fn new(threads: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Task>();
        let rx = Arc::new(std::sync::Mutex::new(rx));

        let mut worker_threads = Vec::with_capacity(threads);
        for _ in 0..threads {
            let rx = Arc::clone(&rx);
            worker_threads.push(thread::spawn(move || worker_loop(rx, false)));
        }

        Self {
            tx,
            _threads: worker_threads,
        }
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(super) fn shared_for_store(pool: StoreExecutorPool, threads: usize) -> Self {
        fn sender_for(
            cell: &'static OnceLock<mpsc::Sender<Task>>,
            thread_name: &'static str,
            threads: usize,
        ) -> mpsc::Sender<Task> {
            cell.get_or_init(|| {
                let (tx, rx) = mpsc::channel::<Task>();
                let rx = Arc::new(std::sync::Mutex::new(rx));
                for ix in 0..threads.max(1) {
                    let rx = Arc::clone(&rx);
                    let _ = thread::Builder::new()
                        .name(format!("{thread_name}-{ix}"))
                        .spawn(move || worker_loop(rx, true));
                }
                tx
            })
            .clone()
        }

        static PRIMARY: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static REPO_LOAD: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static METADATA: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static SESSION_PERSIST: OnceLock<mpsc::Sender<Task>> = OnceLock::new();

        let tx = match pool {
            StoreExecutorPool::Primary => {
                sender_for(&PRIMARY, "gitcomet-test-store-primary", threads)
            }
            StoreExecutorPool::RepoLoad => {
                sender_for(&REPO_LOAD, "gitcomet-test-store-repo-load", threads)
            }
            StoreExecutorPool::Metadata => {
                sender_for(&METADATA, "gitcomet-test-store-metadata", threads)
            }
            StoreExecutorPool::SessionPersist => sender_for(
                &SESSION_PERSIST,
                "gitcomet-test-store-session-persist",
                threads,
            ),
        };

        Self {
            tx,
            _threads: Vec::new(),
        }
    }

    pub(super) fn spawn(&self, task: impl FnOnce() + Send + 'static) {
        let mergetool_trace_context = mergetool_trace::current_capture_context();
        send_or_log(
            &self.tx,
            Box::new(move || {
                let _mergetool_trace = mergetool_trace_context
                    .as_ref()
                    .map(mergetool_trace::attach_capture);
                task();
            }),
            SendFailureKind::ExecutorQueue,
            "TaskExecutor::spawn",
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_load_pool_is_capped_below_primary_pool() {
        let primary = default_worker_threads();
        let repo_load = repo_load_worker_threads();

        assert!(repo_load >= 1);
        assert!(repo_load <= 2);
        if primary > 1 {
            assert!(repo_load < primary);
        }
    }

    #[test]
    fn metadata_pool_supports_parallel_metadata_tasks() {
        assert!(metadata_worker_threads() >= 2);
    }
}
