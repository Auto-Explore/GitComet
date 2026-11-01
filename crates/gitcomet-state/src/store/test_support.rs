//! Track completion per store without draining another store's shared workers.
use std::cell::RefCell;
use std::sync::{Arc, Mutex, mpsc};

#[derive(Default)]
pub(super) struct TestTasks {
    state: Mutex<(usize, Vec<mpsc::Sender<()>>)>,
}

impl TestTasks {
    pub(super) fn task(self: &Arc<Self>) -> TestTask {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).0 += 1;
        TestTask(Arc::clone(self))
    }

    pub(super) fn completion(&self) -> mpsc::Receiver<()> {
        let (send, receive) = mpsc::channel();
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if state.0 == 0 {
            let _ = send.send(());
        } else {
            state.1.push(send);
        }
        receive
    }
}

pub(super) struct TestTask(Arc<TestTasks>);

impl Drop for TestTask {
    fn drop(&mut self) {
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.0 -= 1;
        if state.0 == 0 {
            for sender in state.1.drain(..) {
                let _ = sender.send(());
            }
        }
    }
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<TestTasks>>> = const { RefCell::new(None) };
}

pub(super) struct TestTaskScope(Option<Arc<TestTasks>>);

impl TestTaskScope {
    pub(super) fn enter(tasks: Arc<TestTasks>) -> Self {
        Self(CURRENT.with(|current| current.replace(Some(tasks))))
    }
}

impl Drop for TestTaskScope {
    fn drop(&mut self) {
        CURRENT.with(|current| current.replace(self.0.take()));
    }
}

pub(super) fn track_task(task: impl FnOnce() + Send + 'static) -> impl FnOnce() + Send {
    let owner = CURRENT.with(|current| current.borrow().clone());
    let receipt = owner.as_ref().map(TestTasks::task);
    move || {
        let _scope = owner.map(TestTaskScope::enter);
        let _receipt = receipt;
        task();
    }
}
