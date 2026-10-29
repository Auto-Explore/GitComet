use super::*;
use crate::model::AutoFetchInterval;
use gitcomet_core::test_support::UnconfiguredRepository;
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

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
            active: false,
            needs_refresh: true,
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
        regrouped: Vec::new(),
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
            regrouped: Vec::new(),
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
            regrouped: Vec::new(),
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
            regrouped: Vec::new(),
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
            active: false,
            needs_refresh: true,
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
    // The store canonicalizes workdirs; macOS temp paths resolve through /private.
    let root = gitcomet_core::path_utils::canonicalize_or_original(temp.path().to_path_buf());
    let source = root.join("source");
    let clone = root.join("clone");
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
                // Selected, so quiet fetches keep the plain interval.
                active: true,
                needs_refresh: true,
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
        regrouped: Vec::new(),
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
    assert!(
        state.hosts.contains_key("github.com"),
        "credentials are per host"
    );
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
fn a_sign_in_failure_pauses_every_repository_on_that_host_until_a_manual_fetch() {
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
            "git@github.com: Permission denied (publickey).",
        ),
        UNIX_EPOCH,
    );
    // SSH signs in before choosing a repository: no other one would succeed.
    let much_later = UNIX_EPOCH + Duration::from_secs(30 * 24 * 60 * 60);
    assert_eq!(
        state.skipped_remotes(None, &origin("github.com"), much_later),
        ["origin"]
    );
    assert!(
        state
            .skipped_remotes(None, &origin("gitlab.com"), much_later)
            .is_empty()
    );
    drop(state);
    coordinator.manual_fetch_finished(
        Path::new("/repo"),
        much_later,
        &Ok(CommandOutput::empty_success("git fetch --all")),
    );
    let state = coordinator.inner.state.lock().unwrap();
    assert!(
        state
            .skipped_remotes(None, &origin("github.com"), much_later)
            .is_empty()
    );
}

#[test]
fn an_open_sign_in_dialog_holds_every_automatic_fetch_until_it_closes() {
    let coordinator = AutoFetchCoordinator::new();
    register(&coordinator, 1, "/repo", "/repo/.git");
    enable(&coordinator);
    coordinator.set_sign_in_open(2, true);
    coordinator.set_sign_in_open(3, true);
    let mut state = coordinator.inner.state.lock().unwrap();
    assert!(
        next_job(&mut state, coordinator.admission(), UNIX_EPOCH).is_none(),
        "a window is asking for credentials"
    );
    drop(state);
    coordinator.set_sign_in_open(2, false);
    // A closed window's dialog holds nothing.
    coordinator.unsubscribe(3);
    let mut state = coordinator.inner.state.lock().unwrap();
    assert!(next_job(&mut state, coordinator.admission(), UNIX_EPOCH).is_some());
}

#[test]
fn a_store_reports_its_sign_in_dialog_to_automatic_fetching() {
    use super::super::AppStore;
    use crate::msg::{InternalMsg, Msg};

    let temp = tempfile::tempdir().unwrap();
    let _session = crate::session::push_test_session_file_path_override(Some(
        temp.path().join("session.json"),
    ));
    let (store, _events) = AppStore::new_test(Arc::new(Backend));
    let signing_in = |store: &AppStore| {
        store
            .barrier_for_test()
            .recv_timeout(Duration::from_secs(10))
            .unwrap();
        let state = store.msg_tx.auto_fetch.inner.state.lock().unwrap();
        !state.signing_in.is_empty()
    };
    assert!(!signing_in(&store));
    store.dispatch(Msg::Internal(InternalMsg::CloneRepoFinished {
        url: "https://example.com/private/repo.git".into(),
        dest: temp.path().join("clone"),
        result: Err(Error::new(ErrorKind::Backend(
            "fatal: could not read Username for 'https://example.com': terminal prompts disabled"
                .into(),
        ))),
    }));
    assert!(signing_in(&store), "the dialog holds automatic fetching");
    store.dispatch(Msg::CancelAuthPrompt);
    assert!(!signing_in(&store));
    store
        .shutdown_for_test()
        .recv_timeout(Duration::from_secs(10))
        .unwrap();
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
            "fatal: unable to access 'https://fork.example/r.git/': Could not resolve host: fork.example
             could not fetch 'fork' (exit code: 128)",
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

#[test]
fn a_sign_in_failure_on_one_remote_leaves_another_remotes_outage_temporary() {
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
            name: "mirror".into(),
            host: Some("mirror.example".into()),
        },
    ];
    let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
    // `git fetch --multiple` prints each remote's error before naming it.
    finish(
        &mut state,
        &job,
        failed(
            remotes,
            "fatal: Authentication failed for 'https://github.com/o/r.git/'\n\
             error: could not fetch origin\n\
             fatal: unable to access 'https://mirror.example/r.git/': Could not resolve host: mirror.example\n\
             error: could not fetch mirror",
        ),
        UNIX_EPOCH,
    );
    let group = group_of(&state, Path::new("/repo"));
    assert_eq!(group.snapshot.paused_remotes, ["origin"]);
    assert_eq!(state.hosts["github.com"].until, None);
    assert_eq!(
        state.hosts["mirror.example"].until,
        Some(UNIX_EPOCH + Duration::from_secs(5 * 60))
    );
}

#[test]
fn a_host_keeps_the_longest_block_any_of_its_remotes_earned() {
    let output = "fatal: Authentication failed for 'https://github.com/o/private.git/'\n\
                  error: could not fetch private\n\
                  fatal: unable to access 'https://github.com/o/public.git/': \
                  Could not resolve host: github.com\nerror: could not fetch public";
    for names in [["private", "public"], ["public", "private"]] {
        let coordinator = AutoFetchCoordinator::new();
        register(&coordinator, 1, "/repo", "/repo/.git");
        enable(&coordinator);
        let mut state = coordinator.inner.state.lock().unwrap();
        let remotes = names
            .map(|name| FetchRemote {
                name: name.into(),
                host: Some("github.com".into()),
            })
            .to_vec();
        let job = next_job(&mut state, coordinator.admission(), UNIX_EPOCH).unwrap();
        finish(&mut state, &job, failed(remotes, output), UNIX_EPOCH);
        let block = &state.hosts["github.com"];
        assert_eq!((block.until, block.strikes), (None, 1), "{names:?}");
    }
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
                active: false,
                needs_refresh: true,
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

#[test]
fn a_repository_fetched_recently_waits_but_loads_its_last_count_at_once() {
    let (_temp, _source, clone, _latest) = remote_fixture();
    git(&clone, &["fetch", "-q"]);
    let linked = clone.parent().unwrap().join("linked");
    git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    assert!(last_fetch_time(&clone).is_some());
    // Worktrees share one store, so each sees the other's fetch.
    assert_eq!(last_fetch_time(&linked), last_fetch_time(&clone));
    git(&linked, &["fetch", "-q"]);
    let linked_fetch = std::fs::metadata(clone.join(".git/worktrees/linked/FETCH_HEAD"))
        .and_then(|metadata| metadata.modified())
        .ok();
    assert!(linked_fetch.is_some());
    assert_eq!(last_fetch_time(&clone), linked_fetch);

    let refreshed = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let refreshed = Arc::clone(&refreshed);
        Arc::new(move |event| {
            if let Event::Refresh { lifetime, .. } = event {
                refreshed.lock().unwrap().push(lifetime);
            }
        })
    };
    let coordinator = AutoFetchCoordinator::new();
    let lifetime = RepoLifetime::allocate();
    let mut settings = RemoteSettings::default();
    settings.auto_fetch.enabled = true;
    coordinator.sync(
        1,
        vec![Registration {
            repo_id: RepoId(1),
            lifetime,
            workdir: clone.clone(),
            common_dir: None,
            repository: None,
            active: false,
            needs_refresh: true,
        }],
        settings,
        &(Arc::new(gitcomet_git_gix::GixBackend) as Arc<dyn GitBackend>),
        sink,
    );
    assert_eq!(*refreshed.lock().unwrap(), [lifetime]);
    let mut state = coordinator.inner.state.lock().unwrap();
    assert!(next_job(&mut state, coordinator.admission(), SystemTime::now()).is_none());
    let later = SystemTime::now() + AutoFetchInterval::default().duration();
    assert!(next_job(&mut state, coordinator.admission(), later).is_some());
    drop(state);
    coordinator.shutdown();
}

#[test]
fn a_store_fetched_through_a_sibling_worktree_waits_at_launch() {
    let (_temp, _source, clone, _latest) = remote_fixture();
    let linked = clone.parent().unwrap().join("linked");
    git(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    // A store's fetch runs in one worktree and rewrites only its FETCH_HEAD.
    git(&clone, &["fetch", "-q"]);
    let registrations: Vec<_> = [(clone, false), (linked, true)]
        .into_iter()
        .zip(1..)
        .map(|((workdir, active), id)| Registration {
            repo_id: RepoId(id),
            lifetime: RepoLifetime::allocate(),
            workdir,
            common_dir: None,
            repository: None,
            active,
            needs_refresh: true,
        })
        .collect();
    let coordinator = AutoFetchCoordinator::new();
    coordinator.sync(
        1,
        registrations,
        RemoteSettings::default(),
        &(Arc::new(gitcomet_git_gix::GixBackend) as Arc<dyn GitBackend>),
        Arc::new(|_| {}),
    );
    enable(&coordinator);
    let mut state = coordinator.inner.state.lock().unwrap();
    assert!(
        next_job(&mut state, coordinator.admission(), SystemTime::now()).is_none(),
        "the selected worktree's store was fetched just now"
    );
    let later = SystemTime::now() + AutoFetchInterval::default().duration();
    assert!(next_job(&mut state, coordinator.admission(), later).is_some());
    drop(state);
    coordinator.shutdown();
}

#[test]
fn repositories_restored_at_launch_wait_out_the_grace_period() {
    let coordinator = AutoFetchCoordinator::with_startup_grace(Duration::from_secs(60));
    register(&coordinator, 1, "/restored", "/restored/.git");
    enable(&coordinator);
    let now = SystemTime::now();
    let mut state = coordinator.inner.state.lock().unwrap();
    assert!(next_job(&mut state, coordinator.admission(), now).is_none());
    // The grace covers launch only: a repository opened afterwards fetches.
    state.grace_until = Some(UNIX_EPOCH);
    drop(state);
    register(&coordinator, 2, "/opened", "/opened/.git");
    enable(&coordinator);
    let mut state = coordinator.inner.state.lock().unwrap();
    let job = next_job(&mut state, coordinator.admission(), now).unwrap();
    assert_eq!(job.workdir, Path::new("/opened"));
    finish(
        &mut state,
        &job,
        Completion::Deferred {
            common_dir: PathBuf::from("/opened/.git"),
            key: GroupKey::Workdir(PathBuf::from("/opened")),
            remotes: origin("example.com"),
        },
        now,
    );
    let after_grace = now + Duration::from_secs(61);
    let job = next_job(&mut state, coordinator.admission(), after_grace).unwrap();
    assert_eq!(job.workdir, Path::new("/restored"));
}

#[test]
fn selected_tabs_fetch_first_then_the_least_recently_contacted_host() {
    let coordinator = AutoFetchCoordinator::new();
    for (store, path) in [(1, "/busy-host"), (2, "/quiet-host"), (3, "/selected")] {
        let lifetime = RepoLifetime::allocate();
        coordinator.sync(
            store,
            vec![Registration {
                repo_id: RepoId(1),
                lifetime,
                workdir: PathBuf::from(path),
                common_dir: Some(PathBuf::from(format!("{path}/.git"))),
                repository: None,
                active: path == "/selected",
                needs_refresh: true,
            }],
            RemoteSettings::default(),
            &(Arc::new(Backend) as Arc<dyn GitBackend>),
            Arc::new(|_| {}),
        );
    }
    enable(&coordinator);
    let mut state = coordinator.inner.state.lock().unwrap();
    for (path, host) in [
        ("/busy-host", "busy.example"),
        ("/quiet-host", "quiet.example"),
        ("/selected", "busy.example"),
    ] {
        let key = state.entries[Path::new(path)].group.clone();
        state.groups.get_mut(&key).unwrap().remotes = origin(host);
    }
    let earlier = Instant::now();
    state.contacts.insert("quiet.example".into(), earlier);
    state
        .contacts
        .insert("busy.example".into(), earlier + Duration::from_millis(1));
    let mut order = Vec::new();
    while let Some(job) = next_job(&mut state, coordinator.admission(), UNIX_EPOCH) {
        order.push(job.workdir.clone());
        finish(
            &mut state,
            &job,
            Completion::Finished {
                common_dir: None,
                key: None,
                remotes: Vec::new(),
                skipped: Vec::new(),
                result: Ok(CommandOutput::empty_success("git fetch --all")),
                refs_changed: false,
                regrouped: Vec::new(),
            },
            UNIX_EPOCH,
        );
    }
    assert_eq!(
        order,
        [
            PathBuf::from("/selected"),
            PathBuf::from("/quiet-host"),
            PathBuf::from("/busy-host"),
        ]
    );
}

#[test]
fn spacing_waits_end_early_when_the_fetch_is_cancelled() {
    let cancellation = CancellationToken::new();
    let started = Instant::now();
    sleep_unless_cancelled(Duration::from_millis(120), &cancellation).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(120));
    cancellation.cancel();
    let started = Instant::now();
    assert!(sleep_unless_cancelled(Duration::from_secs(10), &cancellation).is_err());
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn inactive_repositories_that_stay_quiet_wait_up_to_four_intervals() {
    let coordinator = AutoFetchCoordinator::new();
    let subscribe = |store, path: &str, active| {
        set_enabled(&coordinator, false);
        coordinator.sync(
            store,
            vec![Registration {
                repo_id: RepoId(1),
                lifetime: RepoLifetime::allocate(),
                workdir: PathBuf::from(path),
                common_dir: Some(PathBuf::from(format!("{path}/.git"))),
                repository: None,
                active,
                needs_refresh: true,
            }],
            RemoteSettings::default(),
            &(Arc::new(Backend) as Arc<dyn GitBackend>),
            Arc::new(|_| {}),
        );
        enable(&coordinator);
    };
    subscribe(1, "/quiet", false);
    subscribe(2, "/selected", true);
    let interval = AutoFetchInterval::default();
    let fetch = |path: &str, now, refs_changed| {
        let mut state = coordinator.inner.state.lock().unwrap();
        let job = next_job(&mut state, coordinator.admission(), now).unwrap();
        assert_eq!(job.workdir, Path::new(path));
        finish(
            &mut state,
            &job,
            Completion::Finished {
                common_dir: None,
                key: None,
                remotes: Vec::new(),
                skipped: Vec::new(),
                result: Ok(CommandOutput::empty_success("git fetch --all")),
                refs_changed,
                regrouped: Vec::new(),
            },
            now,
        );
        group_of(&state, Path::new(path))
            .schedule
            .wait(interval)
            .as_secs()
            / interval.duration().as_secs()
    };
    let mut now = UNIX_EPOCH;
    assert_eq!(
        fetch("/selected", now, false),
        1,
        "selected tabs never back off"
    );
    assert_eq!(fetch("/quiet", now, false), 2);
    now += interval.duration() * 2;
    assert_eq!(fetch("/selected", now, false), 1);
    assert_eq!(fetch("/quiet", now, false), 4);
    now += interval.duration() * 4;
    assert_eq!(fetch("/selected", now, false), 1);
    assert_eq!(fetch("/quiet", now, false), 4, "four intervals at most");
    now += interval.duration() * 4;
    assert_eq!(fetch("/selected", now, false), 1);
    assert_eq!(fetch("/quiet", now, true), 1, "new commits end the backoff");
    now += interval.duration();
    assert_eq!(fetch("/selected", now, false), 1);
    assert_eq!(fetch("/quiet", now, false), 2);
    subscribe(1, "/quiet", true);
    let state = coordinator.inner.state.lock().unwrap();
    assert_eq!(
        group_of(&state, Path::new("/quiet")).schedule.quiet,
        0,
        "selecting the tab ends it too"
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
            active: false,
            needs_refresh: true,
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
                active: false,
                needs_refresh: true,
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
                    active: false,
                    needs_refresh: true,
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

/// Runs the next due job to completion, as the worker would.
fn run_job(coordinator: &AutoFetchCoordinator, now: SystemTime) -> Option<PathBuf> {
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
}

/// Registers `workdirs` as one window's unopened tabs, without starting the
/// worker; tests drive jobs with [`run_job`].
fn subscribe_unopened(
    coordinator: &AutoFetchCoordinator,
    workdirs: &[&Path],
    sink: EventSink,
) -> Vec<RepoLifetime> {
    let registrations: Vec<_> = workdirs
        .iter()
        .zip(1..)
        .map(|(workdir, id)| Registration {
            repo_id: RepoId(id),
            lifetime: RepoLifetime::allocate(),
            workdir: workdir.to_path_buf(),
            common_dir: None,
            repository: None,
            active: false,
            needs_refresh: true,
        })
        .collect();
    let lifetimes = registrations.iter().map(|r| r.lifetime).collect();
    set_enabled(coordinator, false);
    coordinator.sync(
        1,
        registrations,
        RemoteSettings::default(),
        &(Arc::new(gitcomet_git_gix::GixBackend) as Arc<dyn GitBackend>),
        sink,
    );
    enable(coordinator);
    lifetimes
}

fn add_worktree(clone: &Path, name: &str) -> PathBuf {
    let linked = clone.parent().unwrap().join(name);
    git(
        clone,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            name,
            linked.to_str().unwrap(),
        ],
    );
    linked
}

fn refresh_recorder() -> (EventSink, Arc<Mutex<Vec<RepoLifetime>>>) {
    let refreshed = Arc::new(Mutex::new(Vec::new()));
    let sink: EventSink = {
        let refreshed = Arc::clone(&refreshed);
        Arc::new(move |event| {
            if let Event::Refresh { lifetime, .. } = event {
                refreshed.lock().unwrap().push(lifetime);
            }
        })
    };
    (sink, refreshed)
}

/// A local HTTP server that answers every request with 401.
fn sign_in_remote_url() -> String {
    use std::io::{Read, Write};

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.ends_with(b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => request.extend_from_slice(&buffer[..read]),
                }
            }
            let _ = stream.write_all(
                b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"test\"\r\n\
                  Content-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    format!("http://127.0.0.1:{port}/repository.git")
}

#[test]
fn a_background_fetch_that_cannot_ask_for_credentials_pauses_the_remote() {
    let temp = tempfile::tempdir().unwrap();
    let workdir = canonicalize_or_original(temp.path().to_path_buf()).join("repo");
    std::fs::create_dir_all(&workdir).unwrap();
    git(&workdir, &["init", "-q", "-b", "main"]);
    let private = sign_in_remote_url();
    git(&workdir, &["remote", "add", "private", &private]);
    let coordinator = AutoFetchCoordinator::new();
    subscribe_unopened(&coordinator, &[&workdir], Arc::new(|_| {}));
    assert!(run_job(&coordinator, UNIX_EPOCH).is_some());
    let state = coordinator.inner.state.lock().unwrap();
    let group = group_of(&state, &workdir);
    assert_eq!(
        group.snapshot.paused_remotes,
        ["private"],
        "{:?}",
        group.snapshot.status
    );
    assert_eq!(
        state.hosts[&policy::remote_host(&private).unwrap()].until,
        None
    );
}

#[test]
fn each_failed_remote_of_one_fetch_is_charged_with_its_own_failure() {
    let temp = tempfile::tempdir().unwrap();
    let workdir = canonicalize_or_original(temp.path().to_path_buf()).join("repo");
    std::fs::create_dir_all(&workdir).unwrap();
    git(&workdir, &["init", "-q", "-b", "main"]);
    let private = sign_in_remote_url();
    let offline = refused_remote_url();
    git(&workdir, &["remote", "add", "private", &private]);
    git(&workdir, &["remote", "add", "offline", &offline]);
    let coordinator = AutoFetchCoordinator::new();
    subscribe_unopened(&coordinator, &[&workdir], Arc::new(|_| {}));
    assert!(run_job(&coordinator, UNIX_EPOCH).is_some());
    let state = coordinator.inner.state.lock().unwrap();
    let group = group_of(&state, &workdir);
    assert_eq!(
        group.snapshot.paused_remotes,
        ["private"],
        "an unreachable server needs no sign-in: {:?}",
        group.snapshot.status
    );
    let block = |url: &str| state.hosts[&policy::remote_host(url).unwrap()].until;
    assert_eq!(block(&private), None, "sign-in waits for the user");
    assert_eq!(
        block(&offline),
        Some(UNIX_EPOCH + Duration::from_secs(5 * 60)),
        "an unreachable server is retried on its own"
    );
}

#[test]
fn a_worktree_whose_own_configuration_changes_leaves_the_shared_fetch() {
    let (_temp, source, clone, _latest) = remote_fixture();
    let linked = add_worktree(&clone, "linked");
    let coordinator = AutoFetchCoordinator::new();
    let statuses = Arc::new(Mutex::new(HashMap::new()));
    let sink: EventSink = {
        let statuses = Arc::clone(&statuses);
        Arc::new(move |event| {
            if let Event::Updated {
                lifetime, snapshot, ..
            } = event
            {
                statuses.lock().unwrap().insert(lifetime, snapshot.status);
            }
        })
    };
    let lifetimes = subscribe_unopened(&coordinator, &[&clone, &linked], sink);
    while run_job(&coordinator, UNIX_EPOCH).is_some() {}
    let same_group = || {
        let state = coordinator.inner.state.lock().unwrap();
        state.entries[&clone].group == state.entries[&linked].group
    };
    assert!(same_group(), "both worktrees fetch the same refs");
    // Only the linked worktree's own configuration fetches a second remote.
    git(&clone, &["config", "extensions.worktreeConfig", "true"]);
    let url = source.to_str().unwrap();
    git(&linked, &["config", "--worktree", "remote.extra.url", url]);
    git(
        &linked,
        &[
            "config",
            "--worktree",
            "remote.extra.fetch",
            "+refs/heads/*:refs/remotes/extra/*",
        ],
    );
    let later = UNIX_EPOCH + AutoFetchInterval::default().duration();
    assert_eq!(run_job(&coordinator, later).as_deref(), Some(&*clone));
    assert!(
        !same_group(),
        "the linked worktree fetches more remotes now"
    );
    assert!(
        !matches!(
            statuses.lock().unwrap()[&lifetimes[1]],
            AutoFetchStatus::Running
        ),
        "its tab stops showing the shared fetch"
    );
    assert_eq!(
        run_job(&coordinator, later).as_deref(),
        Some(&*linked),
        "it fetches at once"
    );
    assert!(run_job(&coordinator, later).is_none());
    let extra = gitcomet_core::test_support::git_fixture::git_command()
        .current_dir(&linked)
        .args(["rev-parse", "--verify", "-q", "refs/remotes/extra/main"])
        .output()
        .unwrap();
    assert!(extra.status.success(), "its extra remote was fetched");
}

#[test]
fn turning_automatic_fetch_off_and_on_clears_a_sign_in_host_block() {
    let coordinator = AutoFetchCoordinator::new();
    let lifetime = register(&coordinator, 1, "/repo", "/repo/.git");
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
    assert!(state.group_waits(group_of(&state, Path::new("/repo")), UNIX_EPOCH));
    drop(state);
    // A sync with enabled settings starts the real worker; an open sign-in
    // dialog keeps it from running jobs.
    coordinator.set_sign_in_open(99, true);
    for enabled in [false, true] {
        let mut settings = RemoteSettings::default();
        settings.auto_fetch.enabled = enabled;
        coordinator.sync(
            1,
            vec![Registration {
                repo_id: RepoId(1),
                lifetime,
                workdir: PathBuf::from("/repo"),
                common_dir: Some(PathBuf::from("/repo/.git")),
                repository: None,
                active: false,
                needs_refresh: true,
            }],
            settings,
            &(Arc::new(Backend) as Arc<dyn GitBackend>),
            Arc::new(|_| {}),
        );
    }
    let state = coordinator.inner.state.lock().unwrap();
    let group = group_of(&state, Path::new("/repo"));
    assert!(group.snapshot.paused_remotes.is_empty());
    assert!(
        !state.group_waits(group, UNIX_EPOCH),
        "turning it off and on is a documented recovery"
    );
    drop(state);
    coordinator.shutdown();
}

#[test]
fn a_worktree_that_joins_a_fetched_group_reloads_its_count() {
    let (_temp, _source, clone, _latest) = remote_fixture();
    let linked = add_worktree(&clone, "linked");
    let coordinator = AutoFetchCoordinator::new();
    let (sink, refreshed) = refresh_recorder();
    // Restored tabs never opened: no store is known yet.
    let lifetimes = subscribe_unopened(&coordinator, &[&clone, &linked], sink);
    // They loaded their last known count while the launch grace held fetches.
    for entry in coordinator.inner.state.lock().unwrap().entries.values_mut() {
        for subscriber in entry.subscribers.values_mut() {
            subscriber.needs_refresh = false;
        }
    }
    assert_eq!(run_job(&coordinator, UNIX_EPOCH).as_deref(), Some(&*clone));
    assert_eq!(*refreshed.lock().unwrap(), [lifetimes[0]]);
    assert_eq!(run_job(&coordinator, UNIX_EPOCH).as_deref(), Some(&*linked));
    {
        let state = coordinator.inner.state.lock().unwrap();
        assert_eq!(state.entries[&clone].group, state.entries[&linked].group);
    }
    assert_eq!(
        *refreshed.lock().unwrap(),
        lifetimes,
        "the joining worktree reads the refs its group just fetched"
    );
}

#[test]
fn a_successful_fetch_after_a_failure_reloads_counts_it_left_stale() {
    let (_temp, source, clone, _latest) = remote_fixture();
    let coordinator = AutoFetchCoordinator::new();
    let (sink, refreshed) = refresh_recorder();
    let lifetimes = subscribe_unopened(&coordinator, &[&clone], sink);
    let take = || std::mem::take(&mut *refreshed.lock().unwrap());
    let fetch_when_due = || {
        let due = group_of(&coordinator.inner.state.lock().unwrap(), &clone)
            .schedule
            .due_at(AutoFetchInterval::default())
            .unwrap_or(UNIX_EPOCH);
        assert!(run_job(&coordinator, due).is_some());
        due
    };
    fetch_when_due();
    assert_eq!(take(), lifetimes, "the behind clone moved");
    let set_url = |url: &str| git(&clone, &["remote", "set-url", "origin", url]);
    set_url(&refused_remote_url());
    fetch_when_due();
    assert!(matches!(
        group_of(&coordinator.inner.state.lock().unwrap(), &clone)
            .snapshot
            .status,
        AutoFetchStatus::Failed { .. }
    ));
    assert_eq!(take(), Vec::new(), "the failure moved nothing");
    set_url(source.to_str().unwrap());
    fetch_when_due();
    assert_eq!(
        take(),
        lifetimes,
        "a fetch that moved nothing still clears the failure's grey count"
    );
    let now = fetch_when_due();
    assert_eq!(take(), Vec::new(), "once fresh, unchanged fetches skip it");
    coordinator.manual_fetch_finished(
        &clone,
        now,
        &Err(Error::new(ErrorKind::Backend("remote unavailable".into()))),
    );
    fetch_when_due();
    assert_eq!(take(), lifetimes, "a failed manual fetch greys it too");
}

#[test]
fn a_remote_that_fetch_all_skips_is_never_charged_with_a_failure() {
    let temp = tempfile::tempdir().unwrap();
    let workdir = canonicalize_or_original(temp.path().to_path_buf()).join("repo");
    std::fs::create_dir_all(&workdir).unwrap();
    git(&workdir, &["init", "-q", "-b", "main"]);
    git(
        &workdir,
        &["remote", "add", "origin", "https://skipped.example/r.git"],
    );
    git(&workdir, &["config", "remote.origin.skipFetchAll", "true"]);
    let offline = refused_remote_url();
    git(&workdir, &["remote", "add", "offline", &offline]);
    let coordinator = AutoFetchCoordinator::new();
    subscribe_unopened(&coordinator, &[&workdir], Arc::new(|_| {}));
    assert!(run_job(&coordinator, UNIX_EPOCH).is_some());
    let state = coordinator.inner.state.lock().unwrap();
    assert!(matches!(
        group_of(&state, &workdir).snapshot.status,
        AutoFetchStatus::Failed { .. }
    ));
    assert_eq!(
        state.hosts.keys().collect::<Vec<_>>(),
        [&policy::remote_host(&offline).unwrap()],
        "only the remote that was contacted failed"
    );
    assert!(!state.contacts.contains_key("skipped.example"));
}

/// Git translates the `could not fetch` line that names each failed remote.
#[cfg(unix)]
#[test]
fn a_translated_git_still_shows_which_remote_failed() {
    gitcomet_core::test_support::localized_git::run(
        module_path!(),
        "a_translated_git_still_shows_which_remote_failed",
        r#"#!/bin/sh
for argument in "$@"; do
    if [ "$argument" = fetch ]; then
        if [ "${LANGUAGE:-}" = en ]; then
            printf 'fatal: unable to get password from user\nerror: could not fetch private\n' >&2
        else
            printf 'Schwerwiegend: unable to get password from user\nFehler: konnte private nicht anfordern\n' >&2
        fi
        exit 1
    fi
done
exec git "$@"
"#,
        || {
            let temp = tempfile::tempdir().unwrap();
            let workdir = canonicalize_or_original(temp.path().to_path_buf()).join("repo");
            std::fs::create_dir_all(&workdir).unwrap();
            git(&workdir, &["init", "-q", "-b", "main"]);
            git(
                &workdir,
                &["remote", "add", "origin", "https://public.example/r.git"],
            );
            git(
                &workdir,
                &["remote", "add", "private", "https://private.example/r.git"],
            );
            let coordinator = AutoFetchCoordinator::new();
            subscribe_unopened(&coordinator, &[&workdir], Arc::new(|_| {}));
            assert!(run_job(&coordinator, UNIX_EPOCH).is_some());
            let state = coordinator.inner.state.lock().unwrap();
            assert_eq!(
                group_of(&state, &workdir).snapshot.paused_remotes,
                ["private"],
                "origin fetched; only private needs a sign-in"
            );
            assert!(!state.hosts.contains_key("public.example"));
        },
    );
}
