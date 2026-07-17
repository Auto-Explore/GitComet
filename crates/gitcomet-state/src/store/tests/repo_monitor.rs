use super::super::repo_monitor as monitor_impl;
use super::*;

#[derive(Default)]
struct RepoActivationCallCounts {
    status: std::sync::atomic::AtomicUsize,
    log: std::sync::atomic::AtomicUsize,
    branches: std::sync::atomic::AtomicUsize,
    remote_branches: std::sync::atomic::AtomicUsize,
}

impl RepoActivationCallCounts {
    fn reset(&self) {
        self.status.store(0, std::sync::atomic::Ordering::Relaxed);
        self.log.store(0, std::sync::atomic::Ordering::Relaxed);
        self.branches.store(0, std::sync::atomic::Ordering::Relaxed);
        self.remote_branches
            .store(0, std::sync::atomic::Ordering::Relaxed);
    }

    fn refresh_call_counts(&self) -> (usize, usize, usize, usize) {
        (
            self.status.load(std::sync::atomic::Ordering::Relaxed),
            self.log.load(std::sync::atomic::Ordering::Relaxed),
            self.branches.load(std::sync::atomic::Ordering::Relaxed),
            self.remote_branches
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    fn has_activation_refresh_calls(&self) -> bool {
        let (status, log, branches, remote_branches) = self.refresh_call_counts();
        status > 0 || log > 0 || branches > 0 || remote_branches > 0
    }
}

struct RepoActivationRecordingRepo {
    spec: RepoSpec,
    calls: std::sync::Arc<RepoActivationCallCounts>,
}

impl RepoActivationRecordingRepo {
    fn new(workdir: PathBuf, calls: std::sync::Arc<RepoActivationCallCounts>) -> Self {
        Self {
            spec: RepoSpec { workdir },
            calls,
        }
    }
}

impl GitRepository for RepoActivationRecordingRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        self.calls
            .log
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        })
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unimplemented!()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Ok(Vec::new())
    }

    fn current_branch(&self) -> Result<String> {
        Ok("main".to_string())
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        self.calls
            .branches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        Ok(Vec::new())
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        self.calls
            .remote_branches
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Vec::new())
    }

    fn status(&self) -> Result<RepoStatus> {
        self.calls
            .status
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(RepoStatus {
            staged: Vec::new(),
            unstaged: Vec::new(),
        })
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        Ok(String::new())
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Ok(())
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        Ok(())
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        Ok(())
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Ok(())
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Ok(())
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        Ok(())
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Ok(())
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        Ok(Vec::new())
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Ok(())
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Ok(())
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Ok(())
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Ok(())
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Ok(())
    }

    fn fetch_all(&self) -> Result<()> {
        Ok(())
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        Ok(())
    }

    fn push(&self) -> Result<()> {
        Ok(())
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Ok(())
    }
}

fn unique_repo_monitor_test_path(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "gitcomet-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ))
}

fn active_ready_repo_state(repo_id: RepoId, workdir: PathBuf) -> AppState {
    let mut repo = RepoState::new_opening(repo_id, RepoSpec { workdir });
    repo.set_open(Loadable::Ready(()));
    AppState {
        repos: vec![repo],
        active_repo: Some(repo_id),
        ..Default::default()
    }
}

fn wait_for_monitor_failure_count(kind: monitor_impl::MonitorFailureKind, expected_at_least: u64) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        let count = monitor_impl::monitor_failure_count(kind);
        if count >= expected_at_least {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for {kind:?} monitor failure count to reach {expected_at_least}; got {count}"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

fn wait_for_activation_refresh_calls(calls: &RepoActivationCallCounts) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        if calls.has_activation_refresh_calls() {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for activation fallback refresh calls"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn repo_monitor_start_failures_are_recorded_for_missing_workdir() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Start);

    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let missing_workdir = std::env::temp_dir().join(format!(
        "gitcomet-repo-monitor-missing-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let _ = std::fs::remove_file(&missing_workdir);
    let _ = std::fs::remove_dir_all(&missing_workdir);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);

    monitors.start(
        RepoId(1),
        missing_workdir,
        msg_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
    );

    wait_for_monitor_failure_count(monitor_impl::MonitorFailureKind::Start, before + 1);
    monitors.stop(RepoId(1));
}

#[test]
fn repo_monitor_manager_reports_running_enabled_monitors() {
    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    let monitor_enabled =
        monitors.insert_blocked_monitor_for_test(RepoId(7), release_rx, exited_tx);

    assert!(monitors.is_running(RepoId(7)));
    monitor_enabled.store(false, std::sync::atomic::Ordering::Relaxed);
    assert!(!monitors.is_running(RepoId(7)));
    assert!(!monitors.is_running(RepoId(8)));

    monitors.stop(RepoId(7));
    release_tx
        .send(())
        .expect("test monitor release signal should send");
    exited_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("test monitor thread should exit after release");
}

#[test]
fn repo_monitor_active_repo_activation_coalesces_with_in_flight_refresh() {
    // Activation (window focus) now does a FULL refresh — the filesystem monitor cannot be the sole
    // trigger because it does not see external edits OR git-state changes in sandboxed/Flatpak runs.
    // But when those loads are already in flight, the activation refresh must coalesce rather than
    // schedule duplicate status/log/branch loads. Seed every lane the full refresh touches as
    // in-flight (the primary batch does not include branches/remote branches).
    let repo_id = RepoId(21);
    let workdir = unique_repo_monitor_test_path("activation-coalesce");
    std::fs::create_dir_all(&workdir).expect("create activation coalesce workdir");
    let calls = std::sync::Arc::new(RepoActivationCallCounts::default());
    let state = {
        let mut state = active_ready_repo_state(repo_id, workdir.clone());
        let loads_in_flight = &mut state.repos[0].loads_in_flight;
        loads_in_flight.request_primary_refresh_batch();
        loads_in_flight.request(crate::model::RepoLoadsInFlight::BRANCHES);
        loads_in_flight.request(crate::model::RepoLoadsInFlight::REMOTE_BRANCHES);
        state
    };
    let (store, _events) = AppStore::new(std::sync::Arc::new(FailingBackend));
    store.replace_snapshot_for_test(std::sync::Arc::new(state));
    store.insert_repo_for_test(
        repo_id,
        std::sync::Arc::new(RepoActivationRecordingRepo::new(
            workdir,
            std::sync::Arc::clone(&calls),
        )),
    );

    store.dispatch(Msg::SetActiveRepo { repo_id });
    std::thread::sleep(std::time::Duration::from_millis(100));
    calls.reset();

    store.dispatch(Msg::RepoActivated { repo_id });
    std::thread::sleep(std::time::Duration::from_millis(500));

    assert_eq!(
        calls.refresh_call_counts(),
        (0, 0, 0, 0),
        "activation while a primary refresh is already in flight must coalesce, not schedule duplicate loads"
    );
}

#[test]
fn repo_monitor_unavailable_repo_activation_falls_back_to_git_state_refresh() {
    let repo_id = RepoId(22);
    let workdir = unique_repo_monitor_test_path("activation-fallback");
    std::fs::create_dir_all(&workdir).expect("create activation fallback workdir");
    let calls = std::sync::Arc::new(RepoActivationCallCounts::default());
    let state = active_ready_repo_state(repo_id, workdir.clone());
    let (store, _events) = AppStore::new(std::sync::Arc::new(FailingBackend));
    store.replace_snapshot_for_test(std::sync::Arc::new(state));
    store.insert_repo_for_test(
        repo_id,
        std::sync::Arc::new(RepoActivationRecordingRepo::new(
            workdir,
            std::sync::Arc::clone(&calls),
        )),
    );

    store.dispatch(Msg::RepoActivated { repo_id });

    wait_for_activation_refresh_calls(&calls);
    let (status, log, branches, remote_branches) = calls.refresh_call_counts();
    assert!(status > 0, "fallback should schedule status loading");
    assert!(log > 0, "fallback should schedule log loading");
    assert!(branches > 0, "fallback should schedule branch loading");
    assert!(
        remote_branches > 0,
        "fallback should schedule remote-branch loading"
    );
}

#[test]
fn repo_monitor_stop_send_failures_are_recorded() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Stop);

    monitor_impl::record_stop_send_failure(RepoId(77), "repo monitor test stop send");

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Stop);
    assert!(after > before);
}

#[test]
fn repo_monitor_join_failures_are_recorded() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Join);

    let join = std::thread::spawn(|| panic!("monitor panic test"));
    monitor_impl::join_monitor_or_log(join, RepoId(88), "repo monitor test join");

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Join);
    assert!(after > before);
}

#[test]
fn repo_monitor_stop_does_not_wait_for_monitor_thread_to_exit() {
    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    let monitor_enabled =
        monitors.insert_blocked_monitor_for_test(RepoId(7), release_rx, exited_tx);

    let started = std::time::Instant::now();
    monitors.stop(RepoId(7));
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "repo monitor stop waited for async join: {elapsed:?}"
    );
    assert!(!monitor_enabled.load(std::sync::atomic::Ordering::Relaxed));
    assert!(
        exited_rx.try_recv().is_err(),
        "monitor thread should still be blocked until the test releases it"
    );

    release_tx
        .send(())
        .expect("test monitor release signal should send");
    exited_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("test monitor thread should exit after release");
}

#[test]
fn reducer_effect_handling_does_not_wait_for_stopped_repo_monitor() {
    let old_repo_id = RepoId(10);
    let new_repo_id = RepoId(11);
    let mut old_repo = RepoState::new_opening(
        old_repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-old-monitor-repo"),
        },
    );
    old_repo.set_open(Loadable::Ready(()));
    let mut new_repo = RepoState::new_opening(
        new_repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/gitcomet-new-monitor-repo"),
        },
    );
    new_repo.set_open(Loadable::Ready(()));
    let state = AppState {
        repos: vec![old_repo, new_repo],
        active_repo: Some(new_repo_id),
        ..Default::default()
    };
    let thread_state = std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(state)));
    let active_repo_id = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(old_repo_id.0));
    let (event_tx, _event_rx) = smol::channel::bounded(1);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let thread_msg_tx =
        super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let executor = TaskExecutor::new(1);
    let repo_load_executor = TaskExecutor::new(1);
    let metadata_executor = TaskExecutor::new(1);
    let session_persist_executor = TaskExecutor::new(1);
    let backend: std::sync::Arc<dyn GitBackend> = std::sync::Arc::new(FailingBackend);
    let repos: rustc_hash::FxHashMap<RepoId, std::sync::Arc<dyn GitRepository>> =
        rustc_hash::FxHashMap::default();
    let mut repo_task_tokens: rustc_hash::FxHashMap<RepoId, RepoTaskToken> =
        rustc_hash::FxHashMap::default();
    let mut repo_monitors = monitor_impl::RepoMonitorManager::new();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    repo_monitors.insert_blocked_monitor_for_test(old_repo_id, release_rx, exited_tx);

    let started = std::time::Instant::now();
    handle_reducer_effects(
        std::iter::empty::<Effect>(),
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
    let elapsed = started.elapsed();

    assert!(
        elapsed < std::time::Duration::from_millis(100),
        "effect handling waited for monitor join: {elapsed:?}"
    );

    release_tx
        .send(())
        .expect("test monitor release signal should send");
    exited_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("test monitor thread should exit after release");
}
