//! Store fixtures and completion tracking for each store's own workers.
use super::{
    AppState, AppStore, FilesystemScope, RepoId, StoreEvent, auto_fetch, reducer,
    repository_preferences,
};
use gitcomet_core::services::{GitBackend, GitRepository};
use std::cell::RefCell;
use std::sync::atomic::Ordering;
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

impl AppStore {
    /// Acknowledge queued reductions and return their publication sequence.
    /// Effects scheduled by those reductions can still finish afterwards.
    pub fn barrier_for_test(&self) -> mpsc::Receiver<u64> {
        self.msg_tx.barrier_for_test()
    }

    /// Stop this store and acknowledge its worker, private pools, and effects.
    /// This stops all clones of the store; shared pools remain available.
    pub fn shutdown_for_test(&self) -> mpsc::Receiver<()> {
        let completion = self.msg_tx.test_tasks.completion();
        self.msg_tx.shutdown();
        completion
    }

    pub fn new_test(backend: Arc<dyn GitBackend>) -> (Self, smol::channel::Receiver<StoreEvent>) {
        Self::with_initial_state(
            backend,
            AppState::test_default(),
            repository_preferences::PreferenceHub::new(crate::session::default_session_file_path()),
            auto_fetch::AutoFetchCoordinator::new(),
            FilesystemScope::Store,
        )
    }

    pub fn new_test_sharing_preferences(
        backend: Arc<dyn GitBackend>,
        other: &Self,
    ) -> (Self, smol::channel::Receiver<StoreEvent>) {
        Self::with_initial_state(
            backend,
            AppState::test_default(),
            Arc::clone(&other.msg_tx.preferences),
            Arc::clone(&other.msg_tx.auto_fetch),
            FilesystemScope::Store,
        )
    }

    #[doc(hidden)]
    pub fn history_find_dispatch_count_for_test(&self) -> u64 {
        self.public_lifetime
            .history_find_dispatches
            .load(Ordering::Relaxed)
    }

    #[doc(hidden)]
    pub fn record_planned_one_sided_sides_for_test(state: &mut AppState) {
        reducer::record_planned_one_sided_sides(state);
    }

    #[doc(hidden)]
    pub fn replace_snapshot_for_test(&self, state: Arc<AppState>) {
        // Reduce what is already queued first (a new view's settings
        // bootstrap), so a slow worker cannot apply it over the replacement.
        self.barrier_for_test()
            .recv()
            .expect("store worker acknowledged the queued reductions");
        let mut current = self.state.write().unwrap_or_else(|e| e.into_inner());
        *current = state;
    }

    #[doc(hidden)]
    pub fn insert_repo_for_test(&self, repo_id: RepoId, repo: Arc<dyn GitRepository>) {
        self.msg_tx.insert_repo_for_test(repo_id, repo);
    }

    /// Never starts a filesystem monitor for any repository. For tests over
    /// real repositories whose assertions a watcher refresh would disturb.
    #[doc(hidden)]
    pub fn disable_repo_monitors_for_test(&self) {
        self.msg_tx.disable_repo_monitors_for_test();
    }
}
