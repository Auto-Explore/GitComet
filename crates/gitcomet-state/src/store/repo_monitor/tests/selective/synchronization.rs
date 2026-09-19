use super::*;

fn injected_monitor(
    root: &Path,
    config: MonitorConfig,
) -> (RunningMonitor, mpsc::Receiver<MonitorMsg>) {
    let (tx, rx) = mpsc::channel();
    (
        RunningMonitor::start_with_callback(
            root,
            Arc::new(gitcomet_git_gix::GixBackend),
            config,
            Some(tx),
        ),
        rx,
    )
}

fn content_event(path: PathBuf) -> MonitorMsg {
    MonitorMsg::Event(Ok(notify::Event::new(EventKind::Modify(ModifyKind::Data(
        notify::event::DataChange::Content,
    )))
    .add_path(path)))
}

#[test]
fn drain_empty_requests_do_not_reload_or_rebuild() {
    let (_temp, root) = repository();
    let reloads = Arc::new(AtomicU64::new(0));
    let builds = Arc::new(AtomicU64::new(0));
    let count = builds.clone();
    let (tx, _callbacks) = mpsc::channel();
    let monitor = RunningMonitor::start_with_callback(
        &root,
        Arc::new(FaultyBackend {
            reloads: reloads.clone(),
            ..Default::default()
        }),
        MonitorConfig {
            before_registration: Some(Box::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
            })),
            ..Default::default()
        },
        Some(tx),
    );
    for _ in 0..5 {
        monitor.drain_delivered();
    }
    assert_eq!(reloads.load(Ordering::Relaxed), 1);
    assert_eq!(builds.load(Ordering::Relaxed), 1);
    assert!(monitor.rx.try_recv().is_err());
}

#[test]
fn drain_finishes_index_reload_without_replacing_watches() {
    let (_temp, root) = repository();
    fs::write(root.join("staged.txt"), "staged edit").unwrap();
    let (monitor, _callbacks) = injected_monitor(&root, MonitorConfig::default());
    let generation = monitor.drain_delivered().generation;
    run_git(&root, &["add", "staged.txt"]);
    monitor
        .tx
        .send(content_event(root.join(".git/index")))
        .unwrap();
    assert_eq!(monitor.drain_delivered().generation, generation);
    assert!(
        matches!(monitor.rx.try_recv(), Ok(Msg::RepoExternallyChanged { change, .. }) if change.index)
    );
}

#[test]
fn native_sync_degraded_coverage_is_not_a_successful_checkpoint() {
    let (_temp, root) = repository();
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(FaultyBackend {
            load_failure: Arc::new(AtomicBool::new(true)),
            ..Default::default()
        }),
        MonitorConfig::default(),
    );
    assert!(matches!(
        monitor.rx.try_recv(),
        Ok(Msg::RepoWatchDegraded { .. })
    ));
    assert!(matches!(
        monitor.checkpoint_native(Instant::now() + SYNC_TIMEOUT),
        Err(SyncError::Unavailable(_))
    ));
    // Delivered work can still drain; it does not assert healthy native coverage.
    monitor.drain_delivered();
}

#[test]
fn drain_waits_for_debounce_and_publishes_before_all_acknowledgements() {
    let (_temp, root) = repository();
    let debounce = Duration::from_millis(250);
    let (monitor, _callbacks) = injected_monitor(
        &root,
        MonitorConfig {
            debounce,
            ..Default::default()
        },
    );
    let generation = monitor.drain_delivered().generation;
    let started = Instant::now();
    monitor
        .tx
        .send(content_event(root.join("first.txt")))
        .unwrap();
    monitor
        .tx
        .send(content_event(root.join("second.txt")))
        .unwrap();
    let mut replies = Vec::new();
    for _ in 0..2 {
        let (reply, rx) = mpsc::channel();
        monitor
            .tx
            .send(MonitorMsg::Drain(DrainRequest { generation, reply }))
            .unwrap();
        replies.push(rx);
    }
    for rx in replies {
        assert_eq!(
            rx.recv_timeout(SYNC_TIMEOUT).unwrap(),
            Ok(DrainAck { generation })
        );
    }
    assert!(started.elapsed() >= debounce, "Drain forced an early flush");
    assert!(
        matches!(monitor.rx.try_recv(), Ok(Msg::RepoExternallyChanged { change, .. }) if change.worktree)
    );
    assert!(monitor.rx.try_recv().is_err());
    assert_eq!(monitor.drain_delivered().generation, generation);
    assert!(
        monitor.rx.try_recv().is_err(),
        "empty Drain manufactured a refresh"
    );
}

#[test]
fn drain_rejects_a_generation_replaced_by_policy_rebuild() {
    let (_temp, root) = repository();
    let (monitor, _callbacks) = injected_monitor(&root, MonitorConfig::default());
    let old = monitor.drain_delivered().generation;
    monitor
        .tx
        .send(content_event(root.join(".gitignore")))
        .unwrap();
    assert_eq!(
        monitor.drain_until(old, Instant::now() + SYNC_TIMEOUT),
        Err(SyncError::GenerationChanged)
    );
    let current = monitor.drain_delivered().generation;
    assert!(current.is_some() && current != old);
    assert!(matches!(
        monitor.rx.try_recv(),
        Ok(Msg::RepoExternallyChanged { .. })
    ));
}

#[test]
fn drain_finishes_rescan_before_acknowledging() {
    let (_temp, root) = repository();
    let (monitor, _callbacks) = injected_monitor(&root, MonitorConfig::default());
    let old = monitor.drain_delivered().generation;
    monitor
        .tx
        .send(MonitorMsg::Event(Ok(
            notify::Event::new(EventKind::Any).set_flag(notify::event::Flag::Rescan)
        )))
        .unwrap();
    let current = monitor.drain_delivered().generation;
    assert_ne!(old, current);
    assert!(
        matches!(monitor.rx.try_recv(), Ok(Msg::RepoExternallyChanged { change, .. }) if change == RepoExternalChange::all())
    );
}

#[test]
fn drain_stop_cancels_pending_acknowledgements() {
    let (_temp, root) = repository();
    let (monitor, _callbacks) = injected_monitor(
        &root,
        MonitorConfig {
            debounce: Duration::from_secs(60),
            max_delay: Duration::from_secs(60),
            ..Default::default()
        },
    );
    monitor
        .tx
        .send(content_event(root.join("pending.txt")))
        .unwrap();
    let (reply, rx) = mpsc::channel();
    monitor
        .tx
        .send(MonitorMsg::Drain(DrainRequest {
            generation: None,
            reply,
        }))
        .unwrap();
    monitor.tx.send(MonitorMsg::Stop).unwrap();
    assert_eq!(
        rx.recv_timeout(SYNC_TIMEOUT).unwrap(),
        Err(SyncError::Stopped)
    );
    assert!(monitor.rx.try_recv().is_err());
}

#[test]
fn native_sync_positive_checks_use_operation_unique_paths() {
    let (_temp, root) = repository();
    let monitor = RunningMonitor::start(&root);
    for cycle in 0..4 {
        let path = root.join(format!("operation-{cycle}.txt"));
        let change = monitor.expect_change(&path, || fs::write(&path, "unique edit").unwrap());
        assert!(change.worktree);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn native_sync_callback_checkpoint_does_not_replace_a_quiet_window() {
    let (_temp, root) = repository();
    let monitor = RunningMonitor::start(&root);
    monitor
        .checkpoint_native(Instant::now() + SYNC_TIMEOUT)
        .unwrap();
    // Model an event still waiting upstream when the stream queues checkpointed.
    // The event is deliberately injected AFTER that checkpoint.
    monitor
        .tx
        .send(content_event(root.join("late.txt")))
        .unwrap();
    let started = Instant::now();
    monitor.settle_guarded(started);
    assert!(started.elapsed() >= QUIET_WINDOW);
    assert!(monitor.rx.try_recv().is_err());
}

#[cfg(windows)]
#[test]
fn native_sync_ntfs_cookies_cover_all_roots_without_git_or_callback_noise() {
    use std::io::Write;
    let (_temp, root) = repository();
    let linked_temp = unique_temp_dir("gitcomet-ntfs-linked");
    let linked = linked_temp.path().join("checkout");
    run_git(
        &root,
        &[
            "worktree",
            "add",
            "-b",
            "sync-linked",
            linked.to_str().unwrap(),
        ],
    );
    // On required Windows CI runners this must exercise NTFS, not silently fall back.
    assert!(gitcomet_fs_watch::is_local_ntfs(&root).unwrap());
    assert!(gitcomet_fs_watch::is_local_ntfs(&linked).unwrap());
    let monitor = RunningMonitor::start(&linked);
    let git_dir = normalized(&resolve_git_dir(&linked).unwrap());
    let index_before = fs::read(git_dir.join("index")).unwrap();
    for cycle in 0..20 {
        let source = linked.join(format!("closed-write-{cycle}.txt"));
        let metadata = git_dir.join(format!("sync-input-{cycle}"));
        let after = monitor.observations.sequence();
        for path in [&source, &metadata] {
            let mut file = fs::File::create(path).unwrap();
            file.write_all(b"completed before checkpoint").unwrap();
            file.sync_all().unwrap();
        } // Both writers have closed before creating cookies.
        monitor
            .checkpoint_native(Instant::now() + SYNC_TIMEOUT)
            .unwrap();
        // Assert per-operation identities from BOTH root domains. A count from
        // an earlier round or a worktree-only refresh cannot satisfy this test.
        assert!(
            monitor.observations.has_path_since(after, &source),
            "worktree event missed checkpoint: {:?}",
            monitor.observations
        );
        assert!(
            monitor.observations.has_path_since(after, &metadata),
            "Git-directory event missed checkpoint: {:?}",
            monitor.observations
        );
        assert!(matches!(
            monitor.rx.try_recv(),
            Ok(Msg::RepoExternallyChanged { .. })
        ));
        monitor.consume_followups();
    }
    // Preserve an actual quiet guard when establishing the raw callback baseline.
    monitor.settle();
    let callbacks = monitor.native_events.load(Ordering::Relaxed);
    monitor
        .checkpoint_native(Instant::now() + SYNC_TIMEOUT)
        .unwrap();
    monitor.quiet();
    assert_eq!(monitor.native_events.load(Ordering::Relaxed), callbacks);
    assert_eq!(fs::read(git_dir.join("index")).unwrap(), index_before);
    for directory in [&linked, &git_dir] {
        assert!(fs::read_dir(directory).unwrap().all(|entry| {
            entry
                .unwrap()
                .path()
                .extension()
                .is_none_or(|extension| extension != "gct")
        }));
    }
    let status = gitcomet_git_gix::GixBackend
        .open(&linked)
        .unwrap()
        .status()
        .unwrap();
    assert_eq!(
        status.unstaged.len(),
        20,
        "cookies polluted worktree status"
    );
}
