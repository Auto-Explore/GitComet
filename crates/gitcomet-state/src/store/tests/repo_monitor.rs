use super::super::repo_monitor as monitor_impl;
use super::*;
use crate::store::repository_runtime::RepositoryRuntimes;

#[derive(Default)]
struct RepoActivationCallCounts {
    status: std::sync::atomic::AtomicUsize,
    log: std::sync::atomic::AtomicUsize,
    branches: std::sync::atomic::AtomicUsize,
    remote_branches: std::sync::atomic::AtomicUsize,
    worktree_lists: std::sync::atomic::AtomicUsize,
}

impl RepoActivationCallCounts {
    fn reset(&self) {
        self.status.store(0, std::sync::atomic::Ordering::Relaxed);
        self.log.store(0, std::sync::atomic::Ordering::Relaxed);
        self.branches.store(0, std::sync::atomic::Ordering::Relaxed);
        self.remote_branches
            .store(0, std::sync::atomic::Ordering::Relaxed);
        self.worktree_lists
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

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<std::sync::Arc<LogPage>> {
        cancellation.check_cancelled()?;
        let result: Result<std::sync::Arc<LogPage>> = {
            self.calls
                .log
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(std::sync::Arc::new(LogPage {
                commits: Vec::new(),
                next_cursor: None,
            }))
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unimplemented!()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Ok(Vec::new())
    }

    fn current_branch(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<String> {
        cancellation.check_cancelled()?;
        let result: Result<String> = { Ok("main".to_string()) };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Branch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Branch>> = {
            self.calls
                .branches
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Vec::new())
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remotes(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<Remote>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<Remote>> = { Ok(Vec::new()) };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_remote_branches(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<RemoteBranch>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<RemoteBranch>> = {
            self.calls
                .remote_branches
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Vec::new())
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn status(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<RepoStatus> {
        cancellation.check_cancelled()?;
        let result: Result<RepoStatus> = {
            self.calls
                .status
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(RepoStatus {
                staged: std::sync::Arc::new(Vec::new()),
                unstaged: std::sync::Arc::new(Vec::new()),
            })
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
    }

    fn list_worktrees(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<gitcomet_core::domain::Worktree>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<gitcomet_core::domain::Worktree>> = {
            self.calls
                .worktree_lists
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(vec![gitcomet_core::domain::Worktree {
                bare: false,
                path: self.spec.workdir.clone(),
                head: None,
                branch: Some("main".into()),
                detached: false,
            }])
        };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
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

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Ok(())
    }

    fn stash_list(
        &self,
        cancellation: &gitcomet_core::services::CancellationToken,
    ) -> Result<Vec<StashEntry>> {
        cancellation.check_cancelled()?;
        let result: Result<Vec<StashEntry>> = { Ok(Vec::new()) };
        let result = result?;
        cancellation.check_cancelled()?;
        Ok(result)
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

    fn fetch_all(&self, _prune: bool) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { Ok(()) };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git fetch --all"))
    }

    fn pull(
        &self,
        _mode: PullMode,
        _prune: bool,
    ) -> Result<gitcomet_core::services::CommandOutput> {
        let result: Result<()> = { Ok(()) };
        result.map(|()| gitcomet_core::services::CommandOutput::empty_success("git pull"))
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
        ..AppState::test_default()
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
        std::sync::Arc::new(FailingBackend),
    );

    wait_for_monitor_failure_count(monitor_impl::MonitorFailureKind::Start, before + 1);
    monitors.stop(RepoId(1));
}

#[test]
fn repo_monitor_thread_has_its_own_name() {
    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let workdir = tempfile::tempdir().unwrap();
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);

    monitors.start(
        RepoId(1),
        workdir.path().to_path_buf(),
        msg_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
        std::sync::Arc::new(FailingBackend),
    );

    // Unnamed, it inherits the store worker's name in /proc and profiles.
    assert_eq!(
        monitors.thread_name_for_test(RepoId(1)).as_deref(),
        Some("gitcomet-watch")
    );
    monitors.stop(RepoId(1));
}

#[test]
fn linked_worktree_monitor_threads_have_the_same_name() {
    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let workdir = tempfile::tempdir().unwrap();
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let key = (RepoId(1), 1, workdir.path().to_path_buf());

    monitors.reconcile_worktrees(
        vec![key.clone()],
        msg_tx,
        std::sync::Arc::new(std::sync::atomic::AtomicU64::new(1)),
        std::sync::Arc::new(FailingBackend),
    );

    assert_eq!(
        monitors.worktree_thread_name_for_test(&key).as_deref(),
        Some("gitcomet-watch")
    );
    monitors.stop_all();
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
        loads_in_flight.request_primary_refresh_batch(crate::model::PendingLogLoad {
            walk: gitcomet_core::domain::HistoryMode::FullReachable.into(),
            author: None,
            focus: Default::default(),
            limit: 200,
            cursor: None,
        });
        loads_in_flight.request(crate::model::LoadKind::Branches);
        loads_in_flight.request(crate::model::LoadKind::RemoteBranches);
        state
    };
    let (store, _events) = AppStore::new_test(std::sync::Arc::new(FailingBackend));
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
    let (store, _events) = AppStore::new_test(std::sync::Arc::new(FailingBackend));
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
fn repo_activation_reuses_one_worktree_listing_and_file_edits_add_none() {
    let dir = tempfile::tempdir().unwrap();
    let workdir = canonicalize_or_original(dir.path().to_path_buf());
    let repo_id = RepoId(23);
    let calls = Arc::new(RepoActivationCallCounts::default());
    let mut state = active_ready_repo_state(repo_id, workdir.clone());
    state.repos[0].set_worktrees(Loadable::Ready(Vec::new()));
    let (store, _events) = AppStore::new_test(Arc::new(FailingBackend));
    store.disable_repo_monitors_for_test();
    store.replace_snapshot_for_test(Arc::new(state));
    store.insert_repo_for_test(
        repo_id,
        Arc::new(RepoActivationRecordingRepo::new(
            workdir.clone(),
            calls.clone(),
        )),
    );
    store.dispatch(Msg::RepoActivated { repo_id });
    super::worktree_redirect::wait_until("focus to refresh the worktree list", || {
        let state = store.snapshot();
        state.repos[0]
            .worktrees
            .ready()
            .is_some_and(|list| list.len() == 1 && list[0].path == workdir)
            && !state.repos[0].loads_in_flight.any_in_flight()
    });
    for change in [
        crate::msg::RepoExternalChange::Worktree,
        crate::msg::RepoExternalChange::Index,
    ] {
        store.dispatch(Msg::RepoExternallyChanged { repo_id, change });
    }
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    super::worktree_redirect::wait_until("file edit refreshes to finish", || {
        !store.snapshot().repos[0].loads_in_flight.any_in_flight()
    });
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert_eq!(
        calls
            .worktree_lists
            .load(std::sync::atomic::Ordering::Relaxed),
        1,
        "focus must reuse the dirty scan's one list query, and edit/stage must add none"
    );
}

#[test]
fn checked_activation_skips_only_the_settled_single_checkout_listing() {
    use crate::store::repo_monitor::{ActivationContext, CheckedActivation};
    use gitcomet_core::domain::Worktree;

    for (case, expected_lists) in [
        ("unchanged", 0),
        ("linked checkout", 1),
        ("unknown list", 1),
        ("failed list", 1),
        ("unknown dirty scan", 1),
        ("failed dirty scan", 1),
        ("stale context", 1),
        ("explicit refresh", 1),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let workdir = canonicalize_or_original(dir.path().to_path_buf());
        let repo_id = RepoId(24);
        let calls = Arc::new(RepoActivationCallCounts::default());
        let mut state = active_ready_repo_state(repo_id, workdir.clone());
        let repo = &mut state.repos[0];
        repo.set_head_branch(Loadable::Ready("main".into()));
        repo.set_log(Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        })));
        repo.set_branches(Loadable::Ready(Vec::new()));
        repo.set_remote_branches(Loadable::Ready(Vec::new()));
        repo.set_tags(Loadable::Ready(Vec::new()));
        repo.upstream_divergence = Loadable::Ready(None);
        repo.sequencer_state = Loadable::Ready(Default::default());
        repo.merge_commit_message = Loadable::Ready(None);
        let own = Worktree {
            bare: false,
            path: workdir.clone(),
            head: None,
            branch: Some("main".into()),
            detached: false,
        };
        repo.set_worktrees(Loadable::Ready(vec![own.clone()]));
        repo.set_worktree_dirty(Loadable::Ready(Vec::new()));
        match case {
            "linked checkout" => repo.set_worktrees(Loadable::Ready(vec![
                own,
                Worktree {
                    bare: false,
                    path: workdir.join("linked"),
                    head: None,
                    branch: Some("linked".into()),
                    detached: false,
                },
            ])),
            "unknown list" => repo.set_worktrees(Loadable::NotLoaded),
            "failed list" => repo.set_worktrees(Loadable::Error("unavailable".into())),
            "unknown dirty scan" => repo.set_worktree_dirty(Loadable::NotLoaded),
            "failed dirty scan" => repo.set_worktree_dirty(Loadable::Error("unavailable".into())),
            _ => {}
        }
        let checked = CheckedActivation {
            repo_id,
            context: ActivationContext::for_repo(repo),
            change: RepoExternalChange {
                git_state: false,
                tags: false,
                ..RepoExternalChange::all()
            },
        };
        if case == "stale context" {
            repo.log_rev += 1;
        }
        let (store, _events) = AppStore::new_test(Arc::new(FailingBackend));
        store.disable_repo_monitors_for_test();
        store.replace_snapshot_for_test(Arc::new(state));
        store.insert_repo_for_test(
            repo_id,
            Arc::new(RepoActivationRecordingRepo::new(workdir, calls.clone())),
        );
        if case == "explicit refresh" {
            store.dispatch(Msg::LoadWorktreeDirty { repo_id });
        } else {
            store.msg_tx.send_activation_refresh_or_log(checked);
        }
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        super::worktree_redirect::wait_until(case, || {
            !store.snapshot().repos[0].loads_in_flight.any_in_flight()
        });
        store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        assert_eq!(
            calls.worktree_lists.load(Ordering::Relaxed),
            expected_lists,
            "{case}"
        );
        assert_eq!(
            calls.log.load(Ordering::Relaxed) > 0,
            case == "stale context",
            "{case}"
        );
    }
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
        ..AppState::test_default()
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
    let mut repos: RepositoryRuntimes = Default::default();

    let mut repo_monitors = monitor_impl::RepoMonitorManager::new();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    repo_monitors.insert_blocked_monitor_for_test(old_repo_id, release_rx, exited_tx);

    let started = std::time::Instant::now();
    super::super::WorkerLoopContext {
        commit_identity: &crate::store::EMPTY_TEST_COMMIT_IDENTITY,
        thread_state: &thread_state,
        active_repo_id: &active_repo_id,
        event_tx: &event_tx,
        repo_monitors: &mut repo_monitors,
        thread_msg_tx: &thread_msg_tx,
        executor: &executor,
        repo_load_executor: &repo_load_executor,
        worktree_scan_executor: &std::sync::LazyLock::new(|| TaskExecutor::new(1)),
        metadata_executor: &metadata_executor,
        signature_executor: &metadata_executor,
        history_find_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
        session_persist_executor: &session_persist_executor,
        backend: &backend,
        publication: &std::sync::atomic::AtomicU64::new(0),
        filesystem_executor: &crate::store::executor::FILESYSTEM_QUEUE,
        deferred_acknowledgements: &mut Vec::new(),
    }
    .handle_effects(&mut repos, std::iter::empty::<Effect>());
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

/// Leases count per repository, drop with it when it closes, and ignore
/// repositories that are not open.
#[test]
fn watch_leases_count_and_leave_with_their_repository() {
    let mut repos: RepositoryRuntimes = Default::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(1);
    let mut state = AppState::test_default();
    let repo_id = RepoId(1);
    repos.attach_repository_for_test(repo_id, std::sync::Arc::new(DummyRepo::new("/tmp/leased")));
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: PathBuf::from("/tmp/leased"),
        },
    ));

    let lifetime = state.repos[0].lifetime();

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcquireWatchLease { repo_id, lifetime },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcquireWatchLease { repo_id, lifetime },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcquireWatchLease {
            repo_id: RepoId(9),
            lifetime,
        },
    );
    assert_eq!(state.watch_leases.get(&repo_id), Some(&2));
    assert!(!state.watch_leases.contains_key(&RepoId(9)));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReleaseWatchLease { repo_id, lifetime },
    );
    assert_eq!(state.watch_leases.get(&repo_id), Some(&1));

    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::CloseRepo { repo_id },
    );
    assert!(
        state.watch_leases.is_empty(),
        "a closed repository's leases go"
    );
    // The outstanding lease's release is then a no-op.
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReleaseWatchLease { repo_id, lifetime },
    );
    assert!(state.watch_leases.is_empty());

    // Reusing the id starts a new lifetime. Neither a delayed acquire nor a
    // drop from the first lifetime may change the replacement's lease count.
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/tmp/reopened".into(),
        },
    ));
    let reopened = state.repos[0].lifetime();
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcquireWatchLease {
            repo_id,
            lifetime: reopened,
        },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::AcquireWatchLease { repo_id, lifetime },
    );
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReleaseWatchLease { repo_id, lifetime },
    );
    assert_eq!(state.watch_leases.get(&repo_id), Some(&1));
    reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ReleaseWatchLease {
            repo_id,
            lifetime: reopened,
        },
    );
    assert!(state.watch_leases.is_empty());
}

/// A leased repository keeps its monitor while another is active; the
/// monitor stops once the lease is gone.
#[test]
fn a_watch_lease_keeps_a_background_repositorys_monitor_running() {
    let active = RepoId(1);
    let leased = RepoId(2);
    let dir = tempfile::tempdir().expect("tempdir");
    let mut open_repos = Vec::new();
    for repo_id in [active, leased] {
        let workdir = dir.path().join(format!("repo-{}", repo_id.0));
        std::fs::create_dir_all(&workdir).unwrap();
        let mut repo = RepoState::new_opening(repo_id, RepoSpec { workdir });
        repo.set_open(Loadable::Ready(()));
        open_repos.push(repo);
    }
    let state_with = |leases: &[(RepoId, u32)]| AppState {
        repos: open_repos.clone(),
        active_repo: Some(active),
        watch_leases: std::sync::Arc::new(leases.iter().copied().collect()),
        ..AppState::test_default()
    };
    let thread_state =
        std::sync::Arc::new(std::sync::RwLock::new(std::sync::Arc::new(state_with(&[
            (leased, 1),
        ]))));
    let active_repo_id = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(active.0));
    let (event_tx, _event_rx) = smol::channel::bounded(8);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let thread_msg_tx =
        super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let executor = TaskExecutor::new(1);
    let repo_load_executor = TaskExecutor::new(1);
    let metadata_executor = TaskExecutor::new(1);
    let session_persist_executor = TaskExecutor::new(1);
    let backend: std::sync::Arc<dyn GitBackend> = std::sync::Arc::new(FailingBackend);
    let mut repos: RepositoryRuntimes = Default::default();
    for repo in &open_repos {
        repos.attach_repository_for_test(
            repo.id,
            std::sync::Arc::new(DummyRepo::new(repo.spec.workdir.to_str().unwrap())),
        );
    }

    let mut repo_monitors = monitor_impl::RepoMonitorManager::new();

    let mut handle = |repo_monitors: &mut monitor_impl::RepoMonitorManager| {
        super::super::WorkerLoopContext {
            commit_identity: &crate::store::EMPTY_TEST_COMMIT_IDENTITY,
            thread_state: &thread_state,
            active_repo_id: &active_repo_id,
            event_tx: &event_tx,
            repo_monitors,
            thread_msg_tx: &thread_msg_tx,
            executor: &executor,
            repo_load_executor: &repo_load_executor,
            worktree_scan_executor: &std::sync::LazyLock::new(|| TaskExecutor::new(1)),
            metadata_executor: &metadata_executor,
            signature_executor: &metadata_executor,
            history_find_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            session_persist_executor: &session_persist_executor,
            backend: &backend,
            publication: &std::sync::atomic::AtomicU64::new(0),
            filesystem_executor: &crate::store::executor::FILESYSTEM_QUEUE,
            deferred_acknowledgements: &mut Vec::new(),
        }
        .handle_effects(&mut repos, std::iter::empty::<Effect>());
    };
    handle(&mut repo_monitors);
    let mut running = repo_monitors.running_repo_ids();
    running.sort_by_key(|repo_id| repo_id.0);
    assert_eq!(running, vec![active, leased]);

    *thread_state.write().unwrap() = std::sync::Arc::new(state_with(&[]));
    handle(&mut repo_monitors);
    assert_eq!(repo_monitors.running_repo_ids(), vec![active]);
    repo_monitors.stop_all();
}

#[test]
fn linked_worktree_leases_count_and_do_not_cross_repository_lifetimes() {
    let mut repos = RepositoryRuntimes::default();
    let ids = AtomicU64::new(1);
    let repo_id = RepoId(1);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/tmp/main-worktree".into(),
        },
    ));
    let old_lifetime = state.repos[0].lifetime();
    let path = PathBuf::from("/tmp/linked-worktree");
    let event = |lifetime, watch| Msg::WatchWorktree {
        repo_id,
        lifetime,
        path: path.clone(),
        watch,
    };
    reduce(&mut repos, &ids, &mut state, event(old_lifetime, true));
    reduce(&mut repos, &ids, &mut state, event(old_lifetime, true));
    assert_eq!(
        state.worktree_watch_leases[&(repo_id, old_lifetime, path.clone())],
        2
    );
    reduce(&mut repos, &ids, &mut state, event(old_lifetime, false));
    assert_eq!(
        state.worktree_watch_leases[&(repo_id, old_lifetime, path.clone())],
        1
    );
    state.repos[0] = RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: "/tmp/main-worktree".into(),
        },
    );
    let new_lifetime = state.repos[0].lifetime();
    reduce(&mut repos, &ids, &mut state, event(new_lifetime, true));
    reduce(&mut repos, &ids, &mut state, event(old_lifetime, false));
    assert_eq!(state.worktree_watch_leases.len(), 1);
    assert_eq!(
        state.worktree_watch_leases[&(repo_id, new_lifetime, path.clone())],
        1
    );
}

#[test]
fn repo_monitor_manager_reports_unchanged_since_inactive_once() {
    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    monitors.insert_blocked_monitor_for_test(RepoId(7), release_rx, exited_tx);

    // No mark: never left while watched.
    assert!(!monitors.unchanged_since_inactive(RepoId(7)));
    monitors.mark_inactive(RepoId(7));
    assert!(monitors.unchanged_since_inactive(RepoId(7)));
    // The mark is consumed by the switch that reads it.
    assert!(!monitors.unchanged_since_inactive(RepoId(7)));

    monitors.mark_inactive(RepoId(7));
    monitors.bump_change_generation_for_test(RepoId(7));
    assert!(!monitors.unchanged_since_inactive(RepoId(7)));

    // Degraded coverage may have missed a change.
    monitors.mark_inactive(RepoId(7));
    monitors.set_degraded_for_test(RepoId(7), true);
    assert!(!monitors.unchanged_since_inactive(RepoId(7)));
    monitors.set_degraded_for_test(RepoId(7), false);

    // A stopped monitor proves nothing.
    monitors.mark_inactive(RepoId(7));
    monitors.stop(RepoId(7));
    assert!(!monitors.unchanged_since_inactive(RepoId(7)));
    // Unknown repositories are never unchanged.
    monitors.mark_inactive(RepoId(8));
    assert!(!monitors.unchanged_since_inactive(RepoId(8)));

    release_tx
        .send(())
        .expect("test monitor release signal should send");
    exited_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("test monitor thread should exit after release");
}

#[test]
fn repo_monitor_manager_never_reuses_a_dead_monitor() {
    let mut monitors = monitor_impl::RepoMonitorManager::new();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (exited_tx, exited_rx) = std::sync::mpsc::channel();
    monitors.insert_blocked_monitor_for_test(RepoId(7), release_rx, exited_tx);

    // The thread ends while the monitor stays enabled, as after a panic.
    let state = active_ready_repo_state(RepoId(7), PathBuf::from("unused-monitor-workdir"));
    let context = monitor_impl::ActivationContext::for_repo(&state.repos[0]);
    assert!(monitors.refresh_on_activation(RepoId(7), context.clone()));
    release_tx
        .send(())
        .expect("test monitor release signal should send");
    exited_rx
        .recv_timeout(std::time::Duration::from_secs(1))
        .expect("test monitor thread should exit after release");
    // The exit signal precedes the thread's end by a few instructions.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        monitors.mark_inactive(RepoId(7));
        if !monitors.unchanged_since_inactive(RepoId(7)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "a dead monitor still reports its tab unchanged"
        );
        std::thread::yield_now();
    }
    assert!(
        !monitors.refresh_on_activation(RepoId(7), context),
        "a dead monitor must leave activation to the full-refresh fallback"
    );
}

#[test]
fn warm_inactive_repos_keeps_the_most_recently_active_tabs() {
    let mut state = AppState::test_default();
    let now = SystemTime::now();
    for (id, seconds_ago, open) in [
        (1, Some(0), true),
        (2, Some(30), true),
        (3, Some(10), true),
        (4, Some(20), true),
        (5, Some(5), true),
        (6, None, true),
        (7, Some(1), false),
    ] {
        let mut repo = RepoState::new_opening(
            RepoId(id),
            RepoSpec {
                workdir: format!("/tmp/warm-{id}").into(),
            },
        );
        if open {
            repo.open = Loadable::Ready(());
        }
        repo.last_active_at = seconds_ago.map(|ago| now - Duration::from_secs(ago));
        state.repos.push(repo);
    }
    state.active_repo = Some(RepoId(1));
    // Leased tabs are watched anyway; never-active and opening tabs have
    // nothing to reuse.
    Arc::make_mut(&mut state.watch_leases).insert(RepoId(5), 1);

    assert_eq!(
        warm_inactive_repos(&state, 2),
        vec![RepoId(3), RepoId(4)],
        "the least recently active tab is evicted"
    );
    assert!(warm_inactive_repos(&state, 0).is_empty());
}
