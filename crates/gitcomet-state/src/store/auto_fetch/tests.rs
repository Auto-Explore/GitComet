use super::*;
use gitcomet_core::test_support::UnconfiguredRepository;
use std::sync::Mutex;
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

fn group_of<'a>(state: &'a State, workdir: &Path) -> &'a Group {
    &state.groups[&state.entries[workdir].group]
}

/// Enables every subscriber without a sync, so no worker thread starts and
/// the test drives `next_job` itself.
fn enable(coordinator: &AutoFetchCoordinator) {
    set_enabled(coordinator, true);
}

fn set_enabled(coordinator: &AutoFetchCoordinator, enabled: bool) {
    let mut state = coordinator.inner.state.lock().unwrap();
    for entry in state.entries.values_mut() {
        for subscriber in entry.subscribers.values_mut() {
            subscriber.settings.auto_fetch.enabled = enabled;
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
fn busy_shared_repository_yields_and_its_worktrees_share_one_fetch() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    register(&coordinator, 2, "/linked", "/repo/.git");
    register(&coordinator, 3, "/other", "/other/.git");
    enable(&coordinator);
    let store = |common: &str| GroupKey::Store {
        common_dir: PathBuf::from(common),
        config: 7,
    };
    let fetched = |common: &str| Completion::Finished {
        common_dir: Some(PathBuf::from(common)),
        key: Some(store(common)),
        remotes: Vec::new(),
        skipped: Vec::new(),
        result: Ok(CommandOutput::empty_success("git fetch --all")),
        refs_changed: true,
    };
    let foreground = coordinator.admission().reserve(PathBuf::from("/repo/.git"));
    let mut state = coordinator.inner.state.lock().unwrap();
    assert_eq!(state.entries.len(), 3);
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    assert_eq!(job.workdir, Path::new("/other"));
    finish(&mut state, &job, fetched("/other/.git"), UNIX_EPOCH);
    assert!(next_job(&mut state, coordinator.admission(), UNIX_EPOCH).is_none());
    drop(foreground);
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    assert_eq!(job.workdir, Path::new("/repo"));
    finish(&mut state, &job, fetched("/repo/.git"), UNIX_EPOCH);
    // The linked worktree has not been opened yet; opening it finds the
    // repository's group fetched and joins it without fetching.
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    assert_eq!(job.workdir, Path::new("/linked"));
    finish(
        &mut state,
        &job,
        Completion::Deferred {
            common_dir: PathBuf::from("/repo/.git"),
            key: store("/repo/.git"),
            remotes: Vec::new(),
        },
        UNIX_EPOCH,
    );
    assert!(next_job(&mut state, coordinator.admission(), UNIX_EPOCH).is_none());
    assert_eq!(state.groups.len(), 2);
    assert_eq!(
        state.entries[Path::new("/linked")].group,
        state.entries[Path::new("/repo")].group
    );
    let later = UNIX_EPOCH + AutoFetchInterval::default().duration();
    for _ in 0..2 {
        let job = next_job(&mut state, coordinator.admission(), later).unwrap();
        let common = if job.workdir == Path::new("/other") {
            "/other/.git"
        } else {
            "/repo/.git"
        };
        finish(&mut state, &job, fetched(common), later);
    }
    assert!(
        next_job(&mut state, coordinator.admission(), later).is_none(),
        "one fetch per store each interval"
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
            key: None,
            remotes: Vec::new(),
            skipped: Vec::new(),
            result: Err(Error::new(ErrorKind::Unsupported("test offline"))),
            refs_changed: true,
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
    subscriber.settings.auto_fetch.interval = AutoFetchInterval::new(5).unwrap();
    // The failure doubles the new interval as well.
    let minutes = |minutes: u64| now + Duration::from_secs(minutes * 60);
    assert!(next_job(&mut state, coordinator.admission(), minutes(9)).is_none());
    assert!(next_job(&mut state, coordinator.admission(), minutes(10)).is_some());
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
            key: None,
            remotes: Vec::new(),
            skipped: Vec::new(),
            result: Err(Error::new(ErrorKind::Cancelled)),
            refs_changed: true,
        },
        manual + Duration::from_secs(1),
    );
    let group = group_of(&state, Path::new("/repo"));
    assert_eq!(group.snapshot.last_success_at, Some(manual));
    assert!(matches!(group.snapshot.status, AutoFetchStatus::Succeeded { at } if at == manual));
    assert!(!group.schedule.is_due(AutoFetchInterval::default(), manual));
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
            key: None,
            remotes: Vec::new(),
            skipped: Vec::new(),
            result: Ok(CommandOutput::empty_success("git fetch --all")),
            refs_changed: true,
        },
        UNIX_EPOCH,
    );
    let group = group_of(&state, Path::new("/repo"));
    assert!(group.snapshot.last_success_at.is_none());
    assert!(matches!(group.snapshot.status, AutoFetchStatus::Idle));
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

#[test]
fn fetches_that_move_no_refs_refresh_only_tabs_that_never_loaded() {
    let (_temp, source, clone, _latest) = remote_fixture();
    let coordinator = AutoFetchCoordinator::new();
    let backend = Arc::new(gitcomet_git_gix::GixBackend) as Arc<dyn GitBackend>;
    let refreshed = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let refreshed = Arc::clone(&refreshed);
        Arc::new(move |event| {
            if let Event::Refresh { lifetime, .. } = event {
                refreshed.lock().unwrap().push(lifetime);
            }
        })
    };
    let subscribe = |store_id| {
        let lifetime = RepoLifetime::allocate();
        // A sync that sees an enabled subscriber starts the real worker.
        set_enabled(&coordinator, false);
        coordinator.sync(
            store_id,
            vec![Registration {
                repo_id: RepoId(1),
                lifetime,
                workdir: clone.clone(),
                common_dir: None,
                repository: None,
            }],
            RemoteSettings::default(),
            &backend,
            Arc::clone(&sink),
        );
        enable(&coordinator);
        lifetime
    };
    let interval = AutoFetchInterval::default().duration();
    let fetch_at = |round: u32| {
        let now = UNIX_EPOCH + interval * round;
        let job = next_job(
            &mut coordinator.inner.state.lock().unwrap(),
            coordinator.admission(),
            now,
        )
        .expect("due");
        let completion = execute(&job, &coordinator.inner);
        finish(
            &mut coordinator.inner.state.lock().unwrap(),
            &job,
            completion,
            now,
        );
        std::mem::take(&mut *refreshed.lock().unwrap())
    };

    let first = subscribe(1);
    assert_eq!(fetch_at(0), vec![first], "the behind clone moved");
    assert_eq!(fetch_at(1), Vec::new(), "nothing moved");
    let second = subscribe(2);
    assert_eq!(
        fetch_at(2),
        vec![second],
        "a new window loads its first count even when nothing moved"
    );
    std::fs::write(source.join("file"), "third").unwrap();
    git(&source, &["commit", "-qam", "third"]);
    assert_eq!(fetch_at(3), vec![first, second]);
}

fn origin(host: &str) -> Vec<FetchRemote> {
    vec![FetchRemote {
        name: "origin".into(),
        host: Some(host.into()),
    }]
}

fn failed(remotes: Vec<FetchRemote>, message: &str) -> Completion {
    Completion::Finished {
        common_dir: Some(PathBuf::from("/repo/.git")),
        key: None,
        remotes,
        skipped: Vec::new(),
        result: Err(Error::new(ErrorKind::Backend(message.into()))),
        refs_changed: false,
    }
}

#[test]
fn a_sign_in_failure_pauses_the_remote_until_a_manual_fetch_succeeds() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    enable(&coordinator);
    let mut state = coordinator.inner.state.lock().unwrap();
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    finish(
        &mut state,
        &job,
        failed(
            origin("github.com"),
            "fatal: Authentication failed for 'https://github.com/o/r.git/'",
        ),
        UNIX_EPOCH,
    );
    let group = group_of(&state, Path::new("/repo"));
    assert_eq!(group.snapshot.paused_remotes, ["origin"]);
    assert!(state.hosts.is_empty(), "the host answered");
    let much_later = UNIX_EPOCH + Duration::from_secs(30 * 24 * 60 * 60);
    assert!(
        next_job(&mut state, coordinator.admission(), much_later).is_none(),
        "credentials do not fix themselves"
    );
    drop(state);
    coordinator.manual_fetch_finished(
        Path::new("/repo"),
        much_later,
        &Ok(CommandOutput::empty_success("git fetch --all")),
    );
    let mut state = coordinator.inner.state.lock().unwrap();
    assert!(
        group_of(&state, Path::new("/repo"))
            .snapshot
            .paused_remotes
            .is_empty()
    );
    let interval = AutoFetchInterval::default().duration();
    assert!(next_job(&mut state, coordinator.admission(), much_later + interval).is_some());
}

#[test]
fn a_failed_remote_blocks_only_its_own_host_and_failures_stretch_the_interval() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    enable(&coordinator);
    let mut state = coordinator.inner.state.lock().unwrap();
    let remotes = vec![
        FetchRemote {
            name: "origin".into(),
            host: Some("github.com".into()),
        },
        FetchRemote {
            name: "fork".into(),
            host: Some("fork.example".into()),
        },
    ];
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    finish(
        &mut state,
        &job,
        failed(
            remotes.clone(),
            "error: could not fetch 'fork' (exit code: 128)
             fatal: unable to access 'https://fork.example/r.git/': Could not resolve host: fork.example",
        ),
        UNIX_EPOCH,
    );
    assert_eq!(state.hosts.keys().collect::<Vec<_>>(), ["fork.example"]);
    let group = group_of(&state, Path::new("/repo"));
    assert_eq!(
        state.skipped_remotes(Some(group), &remotes, UNIX_EPOCH),
        ["fork"]
    );
    assert!(
        !state.group_waits(group, UNIX_EPOCH),
        "origin still fetches"
    );
    let interval = AutoFetchInterval::default().duration();
    assert_eq!(group.snapshot.retry_at, Some(UNIX_EPOCH + interval * 2));
    assert!(next_job(&mut state, coordinator.admission(), UNIX_EPOCH + interval).is_none());
    assert!(
        next_job(
            &mut state,
            coordinator.admission(),
            UNIX_EPOCH + interval * 2
        )
        .is_some()
    );
}

/// A local HTTP port nothing listens on: connecting fails at once.
fn refused_remote_url() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("http://127.0.0.1:{port}/repository.git")
}

#[test]
fn an_unreachable_host_is_contacted_once_until_its_block_expires() {
    let temp = tempfile::tempdir().unwrap();
    let url = refused_remote_url();
    let host = policy::remote_host(&url).unwrap();
    let coordinator = AutoFetchCoordinator::new();
    let backend = Arc::new(gitcomet_git_gix::GixBackend) as Arc<dyn GitBackend>;
    let mut workdirs = Vec::new();
    for name in ["first", "second"] {
        let workdir = temp.path().join(name);
        std::fs::create_dir_all(&workdir).unwrap();
        git(&workdir, &["init", "-q", "-b", "main"]);
        git(&workdir, &["remote", "add", "origin", &url]);
        coordinator.sync(
            workdirs.len() as u64 + 1,
            vec![Registration {
                repo_id: RepoId(1),
                lifetime: RepoLifetime::allocate(),
                workdir: workdir.clone(),
                common_dir: None,
                repository: None,
            }],
            RemoteSettings::default(),
            &backend,
            Arc::new(|_| {}),
        );
        workdirs.push(workdir);
    }
    enable(&coordinator);
    let strikes = || {
        coordinator
            .inner
            .state
            .lock()
            .unwrap()
            .hosts
            .get(&host)
            .map(|block| block.strikes)
    };
    let run = |now: SystemTime| {
        let job = next_job(
            &mut coordinator.inner.state.lock().unwrap(),
            coordinator.admission(),
            now,
        )?;
        let completion = execute(&job, &coordinator.inner);
        finish(
            &mut coordinator.inner.state.lock().unwrap(),
            &job,
            completion,
            now,
        );
        Some(job.workdir)
    };
    assert!(run(UNIX_EPOCH).is_some());
    assert_eq!(strikes(), Some(1));
    assert!(
        run(UNIX_EPOCH).is_some(),
        "the second repository is checked"
    );
    assert_eq!(
        strikes(),
        Some(1),
        "but it does not contact the blocked host"
    );
    assert!(run(UNIX_EPOCH).is_none());
    let expired = UNIX_EPOCH + Duration::from_secs(6 * 60);
    assert!(run(expired).is_some(), "one probe after the block");
    assert_eq!(strikes(), Some(2));
    assert!(
        run(expired).is_none(),
        "the failed probe blocks the host again"
    );
}

/// Counts remote contacts: every fetch runs the remote's upload-pack once.
#[cfg(unix)]
fn count_upload_packs(root: &Path, clone: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let log = root.join("upload-pack.log");
    let wrapper = root.join("upload-pack.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\necho contact >> '{}'\nexec git upload-pack \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    git(
        clone,
        &[
            "config",
            "remote.origin.uploadpack",
            wrapper.to_str().unwrap(),
        ],
    );
    log
}

#[cfg(unix)]
#[test]
fn linked_worktrees_of_one_repository_contact_the_remote_once() {
    let (temp, _source, clone, _latest) = remote_fixture();
    let log = count_upload_packs(temp.path(), &clone);
    let mut workdirs = vec![clone.clone()];
    for ix in 1..=2 {
        let linked = temp.path().join(format!("linked-{ix}"));
        git(
            &clone,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                &format!("linked-{ix}"),
                linked.to_str().unwrap(),
            ],
        );
        workdirs.push(linked);
    }
    let succeeded = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let succeeded = Arc::clone(&succeeded);
        Arc::new(move |event| {
            if let Event::Updated {
                lifetime, snapshot, ..
            } = event
                && matches!(snapshot.status, AutoFetchStatus::Succeeded { .. })
            {
                succeeded.lock().unwrap().push(lifetime);
            }
        })
    };
    let coordinator = AutoFetchCoordinator::new();
    let registrations: Vec<_> = workdirs
        .into_iter()
        .zip(1..)
        .map(|(workdir, id)| Registration {
            repo_id: RepoId(id),
            lifetime: RepoLifetime::allocate(),
            workdir,
            common_dir: None,
            repository: None,
        })
        .collect();
    let lifetimes: Vec<_> = registrations.iter().map(|r| r.lifetime).collect();
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    coordinator.sync(
        1,
        registrations,
        settings,
        &(Arc::new(gitcomet_git_gix::GixBackend) as Arc<dyn GitBackend>),
        sink,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !lifetimes
        .iter()
        .all(|lifetime| succeeded.lock().unwrap().contains(lifetime))
    {
        assert!(
            std::time::Instant::now() < deadline,
            "every worktree tab reports the fetch"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    coordinator.shutdown();
    assert_eq!(
        std::fs::read_to_string(&log).unwrap().lines().count(),
        1,
        "three worktree tabs, one store: one remote contact"
    );
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
