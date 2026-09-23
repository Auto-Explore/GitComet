use super::send_diagnostics::{SendFailureKind, panic_payload_to_string, send_or_log};
use gitcomet_core::mergetool_trace;
use std::any::Any;
use std::panic::{self, AssertUnwindSafe};
#[cfg(any(test, feature = "test-support"))]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc};
use std::thread;

type Task = Box<dyn FnOnce() + Send + 'static>;

/// One replaceable queue entry. Already-running work owns its cancellation
/// token; obsolete requests that haven't started release their captures here.
#[derive(Clone, Default)]
pub(super) struct LatestTaskSlot(Arc<std::sync::Mutex<Option<Task>>>);

/// One slot per selected-diff load kind. Sharing a slot would let complementary
/// results replace each other before running, so each kind gets its own field.
#[derive(Clone, Default)]
pub(super) struct SelectedDiffSlots {
    pub(super) patch: LatestTaskSlot,
    pub(super) file_text: LatestTaskSlot,
    pub(super) submodule_summary: LatestTaskSlot,
    pub(super) file_image: LatestTaskSlot,
    pub(super) preview_text: LatestTaskSlot,
}

static WORKER_TASK_PANICS: AtomicU64 = AtomicU64::new(0);

/// Mirrors [`super::send_diagnostics::send_failure_count`] so a recovered task
/// panic is observable rather than only logged. Gated like that sibling: the
/// lib target has no caller, and CI runs clippy with `-D warnings`.
#[cfg(test)]
pub(super) fn worker_task_panic_count() -> u64 {
    WORKER_TASK_PANICS.load(Ordering::Relaxed)
}

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
    Signatures,
    SessionPersist,
}

pub(super) struct TaskExecutor {
    tx: mpsc::Sender<Task>,
    _threads: Vec<thread::JoinHandle<()>>,
}

/// A panicking task must not take its worker thread with it. These pools are
/// small — [`repo_load_worker_threads`] caps at two — so a thread lost to an
/// unwind shrinks the pool for the rest of the process and can leave repo
/// loading with no worker at all. Letting the unwind escape is also how a
/// second panic in a `Drop`, or a non-unwindable frame on the way out, turns a
/// recoverable bug into a process abort.
///
/// Catching here does not hide the panic: the process panic hook has already
/// run at the original panic site and recorded the location and backtrace in
/// the crash log. Note that the hook also arms the pending startup report, so a
/// task panic still surfaces to the user on the next launch even though the
/// process survived it — unchanged by this recovery, which only decides whether
/// the worker thread lives.
fn worker_loop(rx: Arc<std::sync::Mutex<mpsc::Receiver<Task>>>) {
    // These long-lived workers run unrelated jobs across repositories. Mark
    // each worker once, before its first task, for mimalloc's locality heuristics.
    rustfs_mimalloc::set_current_thread_in_threadpool();
    loop {
        let task = {
            let rx = rx.lock().unwrap_or_else(|e| e.into_inner());
            rx.recv()
        };
        match task {
            Ok(task) => {
                if let Err(payload) = panic::catch_unwind(AssertUnwindSafe(task)) {
                    record_worker_task_panic(payload.as_ref());
                }
            }
            Err(_) => break,
        }
    }
}

fn record_worker_task_panic(payload: &(dyn Any + Send)) {
    let count = WORKER_TASK_PANICS.fetch_add(1, Ordering::Relaxed) + 1;
    let thread = thread::current();
    let thread = thread.name().unwrap_or("<unnamed>");
    let message = panic_payload_to_string(payload);
    gitcomet_core::process::write_stderr_line(format_args!(
        "gitcomet-state: executor task panicked on worker thread {thread}: {message}; total_panics={count}"
    ));
}

impl TaskExecutor {
    /// Coalesce queued work without occupying a worker for every obsolete request.
    /// `Some` means a queue entry already owns this slot. Taking it before running
    /// allows a concurrent replacement to enqueue its own entry; do not hold the
    /// slot lock across `task()`. Running work still needs cooperative cancellation.
    pub(super) fn spawn_latest(&self, slot: &LatestTaskSlot, task: impl FnOnce() + Send + 'static) {
        let context = mergetool_trace::current_capture_context();
        let task: Task = Box::new(move || {
            let _trace = context.as_ref().map(mergetool_trace::attach_capture);
            task();
        });
        let already_queued = slot
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .replace(task)
            .is_some();
        if !already_queued {
            let queued = slot.clone();
            let sent = self.try_spawn(move || {
                let task = queued.0.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(task) = task {
                    task();
                }
            });
            // No queue entry owns the slot, so later requests must not wait on one.
            if !sent {
                slot.0.lock().unwrap_or_else(|e| e.into_inner()).take();
            }
        }
    }
    #[cfg_attr(feature = "test-support", allow(dead_code))]
    pub(super) fn new(threads: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Task>();
        let rx = Arc::new(std::sync::Mutex::new(rx));

        let mut worker_threads = Vec::with_capacity(threads);
        for _ in 0..threads {
            let rx = Arc::clone(&rx);
            worker_threads.push(thread::spawn(move || worker_loop(rx)));
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
                        .spawn(move || worker_loop(rx));
                }
                tx
            })
            .clone()
        }

        static PRIMARY: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static REPO_LOAD: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static SIGNATURES: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static METADATA: OnceLock<mpsc::Sender<Task>> = OnceLock::new();
        static SESSION_PERSIST: OnceLock<mpsc::Sender<Task>> = OnceLock::new();

        let tx = match pool {
            StoreExecutorPool::Primary => {
                sender_for(&PRIMARY, "gitcomet-test-store-primary", threads)
            }
            StoreExecutorPool::RepoLoad => {
                sender_for(&REPO_LOAD, "gitcomet-test-store-repo-load", threads)
            }
            StoreExecutorPool::Signatures => {
                sender_for(&SIGNATURES, "gitcomet-test-store-signatures", threads)
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
        self.try_spawn(task);
    }

    /// `false` when the worker queue is disconnected (the failure is recorded).
    fn try_spawn(&self, task: impl FnOnce() + Send + 'static) -> bool {
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
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn latest_slot_replaces_pending_work_without_blocking_other_slots() {
        let executor = TaskExecutor::new(1);
        let (release_tx, release_rx) = mpsc::channel();
        executor.spawn(move || {
            release_rx.recv().unwrap();
        });
        let slot = LatestTaskSlot::default();
        let (done_tx, done_rx) = mpsc::channel();
        for value in 0..100 {
            let tx = done_tx.clone();
            executor.spawn_latest(&slot, move || {
                tx.send(value).unwrap();
            });
        }
        let other = LatestTaskSlot::default();
        executor.spawn_latest(&other, move || {
            done_tx.send(100).unwrap();
        });
        release_tx.send(()).unwrap();
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 99);
        assert_eq!(done_rx.recv_timeout(Duration::from_secs(5)).unwrap(), 100);
        assert!(done_rx.recv_timeout(Duration::from_secs(5)).is_err());
    }

    #[test]
    fn latest_slot_is_released_when_the_queue_is_disconnected() {
        // No workers: the shared receiver is dropped, so every send fails.
        let executor = TaskExecutor::new(0);
        let slot = LatestTaskSlot::default();
        executor.spawn_latest(&slot, || {});
        assert!(
            slot.0.lock().unwrap().is_none(),
            "a failed enqueue must not leave the slot claiming a queued entry"
        );
    }

    #[test]
    fn worker_keeps_serving_tasks_after_one_panics() {
        // The panic is deliberate, so the process panic hook prints it: this
        // test is noisy on stderr even when it passes.
        let before = worker_task_panic_count();
        let executor = TaskExecutor::new(1);
        let (done_tx, done_rx) = mpsc::channel();

        executor.spawn(|| panic!("deliberate task panic"));
        executor.spawn(move || {
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the sole worker should still run queued tasks after one panics");

        // Recovering must not mean swallowing: the panic is still counted.
        // Both tasks ran on the same worker, so by the time the second one has
        // reported, the first one's panic has already been recorded.
        assert!(
            worker_task_panic_count() > before,
            "expected the recovered task panic to be counted"
        );
    }

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
