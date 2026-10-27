use super::*;
use gitcomet_core::test_support::UnconfiguredRepository;
use std::time::{Duration, UNIX_EPOCH};

mod incoming;

#[test]
fn opening_fetches_immediately_and_completion_sets_the_next_interval() {
    let interval = AutoFetchInterval::new(30).unwrap();
    let opened = UNIX_EPOCH + Duration::from_secs(10_000);
    let mut schedule = Schedule::default();
    assert!(schedule.is_due(interval, opened));
    let completed = opened + Duration::from_secs(45);
    schedule.finish(completed);
    assert!(!schedule.is_due(interval, completed + Duration::from_secs(1799)));
    assert!(schedule.is_due(interval, completed + Duration::from_secs(1800)));
    let resumed = completed + Duration::from_secs(86_400);
    assert!(schedule.is_due(interval, resumed));
    schedule.finish(resumed);
    assert!(!schedule.is_due(interval, resumed));
}

struct Backend;

impl GitBackend for Backend {
    fn open(&self, path: &Path) -> Result<Arc<dyn GitRepository>> {
        Ok(Arc::new(UnconfiguredRepository::new(path)))
    }
}

fn register(
    coordinator: &AutoFetchCoordinator,
    store_id: u64,
    path: &str,
    common: &str,
) -> RepoLifetime {
    let lifetime = RepoLifetime::allocate();
    coordinator.sync(
        store_id,
        vec![Registration {
            repo_id: RepoId(1),
            lifetime,
            workdir: PathBuf::from(path),
            common_dir: Some(PathBuf::from(common)),
            repository: None,
        }],
        RemoteSettings::default(),
        &(Arc::new(Backend) as Arc<dyn GitBackend>),
        Arc::new(|_| {}),
    );
    lifetime
}

fn enable(coordinator: &AutoFetchCoordinator) {
    let mut state = coordinator.inner.state.lock().unwrap();
    for entry in state.entries.values_mut() {
        for subscriber in entry.subscribers.values_mut() {
            subscriber.settings.auto_fetch.enabled = true;
        }
    }
}

#[test]
fn duplicate_window_close_keeps_the_fetch_until_the_last_subscriber_closes() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    register(&coordinator, 2, "/repo", "/repo/.git");
    enable(&coordinator);
    let job = {
        let mut state = coordinator.inner.state.lock().unwrap();
        assert_eq!(state.entries.len(), 1);
        let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
        assert!(next_job(&mut state, coordinator.admission(), UNIX_EPOCH).is_none());
        job
    };
    coordinator.unsubscribe(1);
    assert!(!job.cancellation.is_cancelled());
    coordinator.unsubscribe(2);
    assert!(job.cancellation.is_cancelled());
    assert!(coordinator.inner.state.lock().unwrap().entries.is_empty());
}

#[test]
fn busy_shared_repository_yields_while_worktrees_keep_distinct_schedules() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    register(&coordinator, 2, "/linked", "/repo/.git");
    register(&coordinator, 3, "/other", "/other/.git");
    enable(&coordinator);
    let foreground = coordinator.admission().reserve(PathBuf::from("/repo/.git"));
    let mut state = coordinator.inner.state.lock().unwrap();
    assert_eq!(state.entries.len(), 3);
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    assert_eq!(job.workdir, Path::new("/other"));
    finish(
        &mut state,
        &job,
        Completion::Finished {
            common_dir: Some(PathBuf::from("/other/.git")),
            result: Ok(CommandOutput::empty_success("git fetch --all")),
        },
        UNIX_EPOCH,
    );
    assert!(next_job(&mut state, coordinator.admission(), UNIX_EPOCH).is_none());
    drop(foreground);
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    assert_eq!(job.workdir, Path::new("/repo"));
    finish(
        &mut state,
        &job,
        Completion::Finished {
            common_dir: Some(PathBuf::from("/repo/.git")),
            result: Ok(CommandOutput::empty_success("git fetch --all")),
        },
        UNIX_EPOCH,
    );
    assert_eq!(
        next_job(&mut state, coordinator.admission(), UNIX_EPOCH)
            .unwrap()
            .workdir,
        Path::new("/linked")
    );
}

#[test]
fn disabled_coordinator_parks_and_enabled_deadlines_recompute_from_completion() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    let now = UNIX_EPOCH + Duration::from_secs(10_000);
    assert!(
        wait_duration(
            &coordinator.inner.state.lock().unwrap(),
            coordinator.admission(),
            now
        )
        .is_none()
    );
    enable(&coordinator);
    let mut state = coordinator.inner.state.lock().unwrap();
    let job = next_job(&mut state, coordinator.admission(), now).unwrap();
    finish(
        &mut state,
        &job,
        Completion::Finished {
            common_dir: None,
            result: Err(Error::new(ErrorKind::Unsupported("test offline"))),
        },
        now,
    );
    assert!(
        next_job(
            &mut state,
            coordinator.admission(),
            now + Duration::from_secs(60)
        )
        .is_none()
    );
    let subscriber = state
        .entries
        .values_mut()
        .next()
        .unwrap()
        .subscribers
        .values_mut()
        .next()
        .unwrap();
    subscriber.settings.auto_fetch.interval = AutoFetchInterval::new(1).unwrap();
    assert!(
        next_job(
            &mut state,
            coordinator.admission(),
            now + Duration::from_secs(60)
        )
        .is_some()
    );
}

#[test]
fn manual_success_clears_failure_and_an_older_automatic_result_cannot_replace_it() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    enable(&coordinator);
    let started = UNIX_EPOCH + Duration::from_secs(100);
    let job = next_job(
        &mut coordinator.inner.state.lock().unwrap(),
        coordinator.admission(),
        started,
    )
    .unwrap();
    let manual = started + Duration::from_secs(5);
    coordinator.manual_fetch_finished(
        Path::new("/repo"),
        manual,
        &Ok(CommandOutput::empty_success("git fetch --all")),
    );
    let mut state = coordinator.inner.state.lock().unwrap();
    finish(
        &mut state,
        &job,
        Completion::Finished {
            common_dir: Some(PathBuf::from("/repo/.git")),
            result: Err(Error::new(ErrorKind::Cancelled)),
        },
        manual + Duration::from_secs(1),
    );
    let entry = state.entries.get(Path::new("/repo")).unwrap();
    assert_eq!(entry.snapshot.last_success_at, Some(manual));
    assert!(matches!(entry.snapshot.status, AutoFetchStatus::Succeeded { at } if at == manual));
    assert!(!entry.schedule.is_due(AutoFetchInterval::default(), manual));
}

#[test]
fn reopening_a_closed_tab_does_not_accept_its_old_fetch_completion() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    enable(&coordinator);
    let job = next_job(
        &mut coordinator.inner.state.lock().unwrap(),
        coordinator.admission(),
        UNIX_EPOCH,
    )
    .unwrap();
    coordinator.unsubscribe(1);
    register(&coordinator, 1, "/repo", "/repo/.git");
    let mut state = coordinator.inner.state.lock().unwrap();
    finish(
        &mut state,
        &job,
        Completion::Finished {
            common_dir: Some(PathBuf::from("/repo/.git")),
            result: Ok(CommandOutput::empty_success("git fetch --all")),
        },
        UNIX_EPOCH,
    );
    let entry = state.entries.get(Path::new("/repo")).unwrap();
    assert!(entry.snapshot.last_success_at.is_none());
    assert!(matches!(entry.snapshot.status, AutoFetchStatus::Idle));
}

#[test]
fn backward_clock_jump_is_due_once_and_rebases_on_the_new_completion() {
    let mut schedule = Schedule::default();
    schedule.finish(UNIX_EPOCH + Duration::from_secs(100_000));
    let corrected = UNIX_EPOCH + Duration::from_secs(50_000);
    assert!(schedule.is_due(AutoFetchInterval::default(), corrected));
    schedule.finish(corrected);
    assert!(!schedule.is_due(AutoFetchInterval::default(), corrected));
}

#[test]
fn shutdown_cancels_and_waits_for_active_backend_cleanup() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;

    struct GatedBackend {
        started: mpsc::Sender<CancellationToken>,
        cleanup: Mutex<mpsc::Receiver<()>>,
        stopped: Arc<AtomicBool>,
    }

    impl GitBackend for GatedBackend {
        fn open(&self, _: &Path) -> Result<Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::Unsupported(
                "test requires cancellable open",
            )))
        }

        fn open_cancellable(
            &self,
            _: &Path,
            cancellation: &CancellationToken,
        ) -> Result<Arc<dyn GitRepository>> {
            self.started.send(cancellation.clone()).unwrap();
            self.cleanup
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
            self.stopped.store(true, Ordering::Release);
            cancellation.check_cancelled()?;
            Err(Error::new(ErrorKind::Unsupported(
                "test expected cancellation",
            )))
        }
    }

    let coordinator = AutoFetchCoordinator::new();
    let (started_tx, started_rx) = mpsc::channel();
    let (cleanup_tx, cleanup_rx) = mpsc::channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let backend: Arc<dyn GitBackend> = Arc::new(GatedBackend {
        started: started_tx,
        cleanup: Mutex::new(cleanup_rx),
        stopped: Arc::clone(&stopped),
    });
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    coordinator.sync(
        1,
        vec![Registration {
            repo_id: RepoId(1),
            lifetime: RepoLifetime::allocate(),
            workdir: PathBuf::from("/gated"),
            common_dir: None,
            repository: None,
        }],
        settings,
        &backend,
        Arc::new(|_| {}),
    );
    let cancellation = started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let (done_tx, done_rx) = mpsc::channel();
    let shutdown = {
        let coordinator = Arc::clone(&coordinator);
        std::thread::spawn(move || {
            coordinator.shutdown();
            done_tx.send(()).unwrap();
        })
    };
    let state = coordinator.inner.state.lock().unwrap();
    let (state, waited) = coordinator
        .inner
        .admission
        .changed
        .wait_timeout_while(state, Duration::from_secs(10), |state| !state.stopped)
        .unwrap();
    assert!(!waited.timed_out());
    assert!(state.stopped);
    assert!(cancellation.is_cancelled());
    assert!(matches!(done_rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    assert!(!stopped.load(Ordering::Acquire));
    drop(state);
    cleanup_tx.send(()).unwrap();
    done_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    assert!(stopped.load(Ordering::Acquire));
    shutdown.join().unwrap();
    coordinator.shutdown();
}

fn git(path: &Path, args: &[&str]) -> String {
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn remote_fixture() -> (tempfile::TempDir, PathBuf, PathBuf, String) {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let clone = temp.path().join("clone");
    std::fs::create_dir_all(&source).unwrap();
    git(&source, &["init", "-q", "-b", "main"]);
    git(&source, &["config", "user.name", "Fetch Test"]);
    git(&source, &["config", "user.email", "fetch@example.com"]);
    git(&source, &["config", "commit.gpgsign", "false"]);
    std::fs::write(source.join("file"), "first").unwrap();
    git(&source, &["add", "file"]);
    git(&source, &["commit", "-qm", "first"]);
    git(
        temp.path(),
        &[
            "clone",
            "-q",
            source.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    std::fs::write(source.join("file"), "second").unwrap();
    git(&source, &["commit", "-qam", "second"]);
    let latest = git(&source, &["rev-parse", "HEAD"]);
    (temp, source, clone, latest)
}

fn wait_for_store(
    store: &super::super::AppStore,
    events: &smol::channel::Receiver<super::super::StoreEvent>,
    ready: impl Fn(&crate::model::AppState) -> bool,
) {
    smol::block_on(smol::future::or(
        async {
            loop {
                if ready(&store.snapshot()) {
                    return;
                }
                events.recv().await.unwrap();
            }
        },
        async {
            smol::Timer::after(Duration::from_secs(10)).await;
            panic!(
                "automatic fetch store condition did not complete: {:?}",
                store.snapshot()
            );
        },
    ));
}

#[test]
fn restored_never_selected_tab_fetches_without_loading_or_activating_it() {
    use super::super::AppStore;
    use crate::model::{CommandSource, Loadable};
    use crate::msg::Msg;

    let (temp, source, clone, latest) = remote_fixture();
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let (store, events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    store.dispatch(Msg::SetRemoteSettings(settings));
    store.dispatch(Msg::RestoreSession {
        open_repos: vec![source.clone(), clone.clone()],
        active_repo: Some(source.clone()),
    });
    wait_for_store(&store, &events, |state| {
        state.repos.len() == 2
            && state
                .repos
                .iter()
                .all(|repo| matches!(repo.auto_fetch.status, AutoFetchStatus::Succeeded { .. }))
    });
    let snapshot = store.snapshot();
    let inactive = snapshot
        .repos
        .iter()
        .find(|repo| repo.spec.workdir == clone)
        .unwrap();
    assert_ne!(snapshot.active_repo, Some(inactive.id));
    assert!(matches!(inactive.open, Loadable::NotLoaded));
    assert!(matches!(inactive.history_state.log, Loadable::NotLoaded));
    assert!(matches!(inactive.status, Loadable::NotLoaded));
    assert!(inactive.feedback.diagnostics.is_empty());
    assert!(
        inactive
            .feedback
            .command_log
            .iter()
            .any(|entry| entry.source == CommandSource::Background && entry.ok)
    );
    assert_eq!(
        git(&clone, &["rev-parse", "refs/remotes/origin/main"]),
        latest
    );
    assert_ne!(git(&clone, &["rev-parse", "HEAD"]), latest);
    drop(snapshot);
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[test]
fn utility_window_participation_cannot_be_enabled_by_global_settings() {
    use super::super::AppStore;
    use crate::msg::Msg;

    let (temp, source, clone, latest) = remote_fixture();
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let (store, events) = AppStore::new_test(Arc::new(gitcomet_git_gix::GixBackend));
    store.disable_repo_monitors_for_test();
    store.set_auto_fetch_allowed(false);
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    store.dispatch(Msg::SetRemoteSettings(settings));
    store.dispatch(Msg::RestoreSession {
        open_repos: vec![source.clone(), clone.clone()],
        active_repo: Some(source),
    });
    store
        .barrier_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
    assert!(
        store
            .msg_tx
            .auto_fetch
            .inner
            .state
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(store.msg_tx.auto_fetch.worker.lock().unwrap().is_none());
    assert_ne!(
        git(&clone, &["rev-parse", "refs/remotes/origin/main"]),
        latest
    );
    store.set_auto_fetch_allowed(true);
    wait_for_store(&store, &events, |state| {
        state
            .repos
            .iter()
            .find(|repo| repo.spec.workdir == clone)
            .is_some_and(|repo| matches!(repo.auto_fetch.status, AutoFetchStatus::Succeeded { .. }))
    });
    assert_eq!(
        git(&clone, &["rev-parse", "refs/remotes/origin/main"]),
        latest
    );
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

#[cfg(unix)]
mod transport;

#[test]
fn store_messages_that_change_no_registration_skip_the_coordinator_sync() {
    use super::super::AppStore;
    use crate::msg::Msg;
    use std::sync::atomic::Ordering;

    let (store, _events) = AppStore::new_test(Arc::new(Backend));
    let settle = || {
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
    };
    let syncs = || store.msg_tx.auto_fetch.syncs.load(Ordering::Relaxed);
    store.dispatch(Msg::SetRemoteSettings(RemoteSettings::default()));
    settle();
    let before = syncs();
    for _ in 0..5 {
        store.dispatch(Msg::SetRemoteSettings(RemoteSettings::default()));
    }
    settle();
    assert_eq!(syncs(), before, "unchanged inputs must not resync");
    store.dispatch(Msg::SetRemoteSettings(RemoteSettings {
        prune_deleted_remote_branches_on_fetch: false,
        ..RemoteSettings::default()
    }));
    settle();
    assert_eq!(syncs(), before + 1, "a settings change still reaches it");
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
}

/// Cost of one coordinator sync with many tabs open. The store calls it after
/// messages that change registrations; prints rather than asserting. Run with
/// `-- --ignored --nocapture`.
#[test]
#[ignore = "timing probe"]
fn timing_auto_fetch_sync_with_many_tabs() {
    let coordinator = AutoFetchCoordinator::new();
    let backend = Arc::new(Backend) as Arc<dyn GitBackend>;
    let registrations = |count: usize| {
        (0..count)
            .map(|ix| Registration {
                repo_id: RepoId(ix as u64 + 1),
                lifetime: RepoLifetime::allocate(),
                workdir: PathBuf::from(format!("/nonexistent/workspace/repositories/repo-{ix}")),
                common_dir: Some(PathBuf::from(format!(
                    "/nonexistent/workspace/repositories/repo-{ix}/.git"
                ))),
                repository: None,
            })
            .collect::<Vec<_>>()
    };
    for count in [100, 1000] {
        let template = registrations(count);
        let fresh = || {
            template
                .iter()
                .map(|registration| Registration {
                    repo_id: registration.repo_id,
                    lifetime: registration.lifetime,
                    workdir: registration.workdir.clone(),
                    common_dir: registration.common_dir.clone(),
                    repository: None,
                })
                .collect::<Vec<_>>()
        };
        coordinator.sync(
            1,
            fresh(),
            RemoteSettings::default(),
            &backend,
            Arc::new(|_| {}),
        );
        let runs = 20;
        let started = std::time::Instant::now();
        for _ in 0..runs {
            coordinator.sync(
                1,
                fresh(),
                RemoteSettings::default(),
                &backend,
                Arc::new(|_| {}),
            );
        }
        eprintln!(
            "auto-fetch sync, {count} tabs: {:?} per sync",
            started.elapsed() / runs
        );
    }
}
