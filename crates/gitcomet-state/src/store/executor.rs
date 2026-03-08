use super::send_diagnostics::{SendFailureKind, send_or_log};
use std::sync::{Arc, mpsc};
use std::thread;

pub(super) fn default_worker_threads() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 8))
        .unwrap_or(2)
}

pub(super) struct TaskExecutor {
    tx: mpsc::Sender<Box<dyn FnOnce() + Send + 'static>>,
    _threads: Vec<thread::JoinHandle<()>>,
}

impl TaskExecutor {
    pub(super) fn new(threads: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Box<dyn FnOnce() + Send + 'static>>();
        let rx = Arc::new(std::sync::Mutex::new(rx));

        let mut worker_threads = Vec::with_capacity(threads);
        for _ in 0..threads {
            let rx = Arc::clone(&rx);
            worker_threads.push(thread::spawn(move || {
                loop {
                    let task = {
                        let rx = rx.lock().expect("executor lock poisoned");
                        rx.recv()
                    };
                    match task {
                        Ok(task) => task(),
                        Err(_) => break,
                    }
                }
            }));
        }

        Self {
            tx,
            _threads: worker_threads,
        }
    }

    pub(super) fn spawn(&self, task: impl FnOnce() + Send + 'static) {
        send_or_log(
            &self.tx,
            Box::new(task),
            SendFailureKind::ExecutorQueue,
            "TaskExecutor::spawn",
        );
    }
}
