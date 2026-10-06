use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{CancellationToken, Result};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// gix reads an atomic interrupt flag while walking clean files, when its
/// iterator emits no items at which we could otherwise check cancellation.
/// Watching the token also includes cancellation inherited from its parent.
pub(super) struct StatusCancellation {
    pub(super) interrupt: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl StatusCancellation {
    pub(super) fn new(cancellation: &CancellationToken) -> Result<Self> {
        cancellation.check_cancelled()?;
        let interrupt = Arc::new(AtomicBool::new(false));
        let stop = Arc::new(AtomicBool::new(false));
        let worker = thread::Builder::new()
            .name(format!(
                "{}-status-cancel",
                gitcomet_core::identity::current().executable_name()
            ))
            .spawn({
                let cancellation = cancellation.clone();
                let interrupt = Arc::clone(&interrupt);
                let stop = Arc::clone(&stop);
                move || {
                    while !stop.load(Ordering::Acquire) {
                        if cancellation.is_cancelled() {
                            interrupt.store(true, Ordering::Release);
                            break;
                        }
                        thread::park_timeout(Duration::from_millis(5));
                    }
                }
            })
            .map_err(|error| {
                Error::new(ErrorKind::Backend(format!(
                    "start status cancellation watcher: {error}"
                )))
            })?;
        Ok(Self {
            interrupt,
            stop,
            worker: Some(worker),
        })
    }
}

impl Drop for StatusCancellation {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            let _ = worker.join();
        }
    }
}

#[cfg(not(test))]
pub(super) fn progress(_: &Arc<AtomicBool>) -> gix::progress::Discard {
    gix::progress::Discard
}

#[cfg(test)]
pub(super) use test_progress::progress;

/// Pause inside gix's actual index walk and count visited entries. Each scan
/// captures its calling thread's hook, avoiding interference between tests.
#[cfg(test)]
pub(super) mod test_progress {
    use super::*;
    use gix::progress::{Count, Id, MessageLevel, Progress, Step, StepShared, Unit};
    use std::cell::RefCell;
    use std::sync::{Mutex, mpsc};

    thread_local! { static HOOK: RefCell<Option<Arc<ScanHook>>> = const { RefCell::new(None) }; }

    pub(in crate::repo::status) struct ScanHook {
        pub visited: StepShared,
        pub started: mpsc::Sender<Arc<AtomicBool>>,
        pub resume: Mutex<mpsc::Receiver<()>>,
    }

    pub(in crate::repo::status) fn with_hook<T>(
        hook: Arc<ScanHook>,
        scan: impl FnOnce() -> T,
    ) -> T {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                HOOK.with(|slot| slot.replace(None));
            }
        }
        HOOK.with(|slot| {
            assert!(slot.replace(Some(hook)).is_none());
        });
        let _reset = Reset;
        scan()
    }

    pub(in crate::repo::status) struct ScanProgress {
        hook: Option<Arc<ScanHook>>,
        interrupt: Arc<AtomicBool>,
        discard: gix::progress::Discard,
    }
    pub(in crate::repo::status) fn progress(interrupt: &Arc<AtomicBool>) -> ScanProgress {
        ScanProgress {
            hook: HOOK.with(|slot| slot.borrow().clone()),
            interrupt: Arc::clone(interrupt),
            discard: gix::progress::Discard,
        }
    }
    impl Count for ScanProgress {
        fn set(&self, step: Step) {
            self.counter().store(step, Ordering::Relaxed);
        }
        fn step(&self) -> Step {
            self.counter().load(Ordering::Relaxed)
        }
        fn inc_by(&self, step: Step) {
            self.counter().fetch_add(step, Ordering::Relaxed);
        }
        fn counter(&self) -> StepShared {
            self.hook
                .as_ref()
                .map_or_else(|| self.discard.counter(), |hook| Arc::clone(&hook.visited))
        }
    }
    impl Progress for ScanProgress {
        fn init(&mut self, max: Option<Step>, unit: Option<Unit>) {
            self.discard.init(max, unit);
            if let Some(hook) = &self.hook {
                hook.started.send(Arc::clone(&self.interrupt)).unwrap();
                let _ = hook
                    .resume
                    .lock()
                    .unwrap()
                    .recv_timeout(Duration::from_secs(5));
            }
        }
        fn set_name(&mut self, name: String) {
            self.discard.set_name(name);
        }
        fn name(&self) -> Option<String> {
            self.discard.name()
        }
        fn id(&self) -> Id {
            self.discard.id()
        }
        fn message(&self, level: MessageLevel, message: String) {
            self.discard.message(level, message);
        }
    }
}
