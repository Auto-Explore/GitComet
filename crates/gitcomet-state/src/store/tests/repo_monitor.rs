use super::super::repo_monitor as monitor_impl;
use super::*;

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
fn repo_monitor_stop_send_failures_are_recorded() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Stop);

    monitor_impl::record_stop_send_failure(RepoId(77), "repo monitor test stop send");

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Stop);
    assert!(after >= before + 1);
}

#[test]
fn repo_monitor_join_failures_are_recorded() {
    let before = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Join);

    let join = std::thread::spawn(|| panic!("monitor panic test"));
    monitor_impl::join_monitor_or_log(join, RepoId(88), "repo monitor test join");

    let after = monitor_impl::monitor_failure_count(monitor_impl::MonitorFailureKind::Join);
    assert!(after >= before + 1);
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
