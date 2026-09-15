use super::*;

fn counted_monitor(root: &Path) -> (RunningMonitor, Arc<AtomicU64>, Arc<AtomicU64>) {
    let reloads = Arc::new(AtomicU64::new(0));
    let builds = Arc::new(AtomicU64::new(0));
    let count = builds.clone();
    let monitor = RunningMonitor::start_custom(
        root,
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
    );
    monitor.settle();
    monitor.quiet();
    (monitor, reloads, builds)
}

#[test]
fn stable_setup_reloads_once() {
    let (_temp, root) = repository();
    let (_monitor, reloads, builds) = counted_monitor(&root);
    assert_eq!(
        reloads.load(Ordering::Relaxed),
        1,
        "stable startup reloaded the backend twice"
    );
    assert_eq!(builds.load(Ordering::Relaxed), 1);
}

#[test]
fn external_commit_refreshes_git_state_without_rebuild() {
    let (_temp, root) = repository();
    let (monitor, _, builds) = counted_monitor(&root);
    builds.store(0, Ordering::Relaxed);
    run_git(&root, &["commit", "--allow-empty", "-m", "External commit"]);
    match monitor.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Msg::RepoExternallyChanged { change, .. } => {
            assert!(change.git_state);
            assert_ne!(change, RepoExternalChange::all());
        }
        other => panic!("unexpected message: {other:?}"),
    }
    monitor.settle();
    assert_eq!(
        builds.load(Ordering::Relaxed),
        0,
        "commit replaced native watches"
    );
}

#[test]
fn external_git_add_refreshes_index_without_rebuild() {
    let (_temp, root) = repository();
    fs::write(root.join("file.txt"), "before").unwrap();
    let (monitor, reloads, builds) = counted_monitor(&root);
    reloads.store(0, Ordering::Relaxed);
    builds.store(0, Ordering::Relaxed);
    run_git(&root, &["add", "file.txt"]);
    match monitor.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Msg::RepoExternallyChanged { change, .. } => {
            assert!(change.index);
            assert!(!change.worktree && !change.tags);
        }
        other => panic!("unexpected message: {other:?}"),
    }
    monitor.settle();
    assert_eq!(
        builds.load(Ordering::Relaxed),
        0,
        "staging replaced native watches"
    );
    assert_eq!(
        reloads.load(Ordering::Relaxed),
        1,
        "index change loaded multiple snapshots"
    );
}

#[test]
fn removing_and_recreating_ignored_dir_does_not_rebuild() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    let (monitor, _, builds) = counted_monitor(&root);
    builds.store(0, Ordering::Relaxed);
    fs::remove_dir_all(root.join("node_modules")).unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    monitor.quiet();
    assert_eq!(builds.load(Ordering::Relaxed), 0);
}

#[test]
fn new_nested_directory_is_watched_without_full_rebuild() {
    let (_temp, root) = repository();
    let (monitor, _, builds) = counted_monitor(&root);
    builds.store(0, Ordering::Relaxed);
    fs::create_dir_all(root.join("source/nested")).unwrap();
    fs::write(root.join("source/nested/file.txt"), "created").unwrap();
    monitor.refresh();
    fs::write(root.join("source/nested/file.txt"), "later edit").unwrap();
    monitor.refresh();
    assert_eq!(
        builds.load(Ordering::Relaxed),
        0,
        "new directory replaced existing coverage"
    );
}

#[test]
fn force_adding_file_under_ignored_dir_starts_watching_it() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    fs::create_dir_all(root.join("generated/nested")).unwrap();
    let source = root.join("generated/nested/source.txt");
    fs::write(&source, "initial").unwrap();
    let (monitor, reloads, builds) = counted_monitor(&root);
    reloads.store(0, Ordering::Relaxed);
    builds.store(0, Ordering::Relaxed);
    run_git(&root, &["add", "-f", "generated/nested/source.txt"]);
    monitor.refresh();
    assert_eq!(
        reloads.load(Ordering::Relaxed),
        1,
        "coverage reused the refreshed index snapshot"
    );
    assert_eq!(builds.load(Ordering::Relaxed), 1);
    fs::write(&source, "tracked edit").unwrap();
    monitor.refresh();
}

#[test]
fn dot_git_directory_timestamp_echo_does_not_reload_or_refresh() {
    let (_temp, root) = repository();
    let mut rules = load_gitignore_rules(&root);
    let effect = summarize_event(
        &root,
        Some(&root.join(".git")),
        &mut rules,
        &notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(root.join(".git")),
    );
    assert!(!effect.policy_dirty && !effect.index_dirty);
    assert_eq!(effect.change, None);
}

#[test]
fn external_excludes_edit_applies_on_revalidate() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-activation-excludes");
    let excludes = external.path().join("ignore");
    fs::write(&excludes, "generated/\n").unwrap();
    run_git(
        &root,
        &["config", "core.excludesFile", excludes.to_str().unwrap()],
    );
    fs::create_dir_all(root.join("generated/nested")).unwrap();
    let (monitor, _, builds) = counted_monitor(&root);
    builds.store(0, Ordering::Relaxed);
    fs::write(&excludes, "").unwrap();
    monitor.quiet(); // No native registration on the external parent.
    monitor.revalidate();
    monitor.refresh();
    assert_eq!(builds.load(Ordering::Relaxed), 1);
    fs::write(root.join("generated/nested/source.txt"), "now observed").unwrap();
    monitor.refresh();
}

#[test]
fn external_excludes_edit_applies_on_existing_idle_tick() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-idle-excludes");
    let excludes = external.path().join("ignore");
    fs::write(&excludes, "").unwrap();
    run_git(
        &root,
        &["config", "core.excludesFile", excludes.to_str().unwrap()],
    );
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(gitcomet_git_gix::GixBackend),
        MonitorConfig {
            idle_tick: Duration::from_millis(100),
            ..Default::default()
        },
    );
    monitor.settle();
    fs::write(&excludes, "generated/\n").unwrap();
    monitor.refresh();
}

#[test]
fn native_error_does_not_report_degraded_coverage_when_registration_succeeds() {
    let (_temp, root) = repository();
    let monitor = RunningMonitor::start(&root);
    monitor
        .tx
        .send(MonitorMsg::Event(Err(notify::Error::generic(
            "injected overflow",
        ))))
        .unwrap();
    monitor.refresh(); // A degraded warning would fail this assertion.
}

#[test]
fn unchanged_failed_policy_waits_for_throttled_recovery() {
    let (_temp, root) = repository();
    let failing = Arc::new(AtomicBool::new(false));
    let reloads = Arc::new(AtomicU64::new(0));
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(FaultyBackend {
            load_failure: failing.clone(),
            reloads: reloads.clone(),
            ..Default::default()
        }),
        MonitorConfig {
            idle_tick: Duration::from_millis(50),
            recovery_interval: Duration::from_secs(60),
            ..Default::default()
        },
    );
    monitor.settle();
    failing.store(true, Ordering::Relaxed);
    fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    assert!(matches!(
        monitor.rx.recv_timeout(Duration::from_secs(5)),
        Ok(Msg::RepoWatchDegraded { .. })
    ));
    assert!(matches!(
        monitor.rx.recv_timeout(Duration::from_secs(5)),
        Ok(Msg::RepoExternallyChanged { .. })
    ));
    // Native directory metadata can finish the triggering burst after the
    // first flush. Continuous idle retries cannot pass this bounded settle.
    monitor.settle();
    let attempts = reloads.load(Ordering::Relaxed);
    assert!(matches!(
        monitor.rx.recv_timeout(Duration::from_millis(500)),
        Err(mpsc::RecvTimeoutError::Timeout)
    ));
    assert_eq!(
        reloads.load(Ordering::Relaxed),
        attempts,
        "unchanged failure retried on every idle tick"
    );
    failing.store(false, Ordering::Relaxed);
    fs::write(root.join(".gitignore"), "other/\n").unwrap();
    monitor.revalidate();
    monitor.refresh();
}

#[cfg(windows)]
#[test]
fn watched_folders_can_be_renamed_and_recreated() {
    let (_temp, root) = repository();
    let tree = root.join("source/nested/deep");
    fs::create_dir_all(&tree).unwrap();
    fs::write(tree.join("file.txt"), "tracked").unwrap();
    run_git(&root, &["add", "."]);
    run_git(&root, &["commit", "-m", "With tree"]);
    run_git(&root, &["tag", "with-tree"]);
    run_git(&root, &["rm", "-r", "source"]);
    run_git(&root, &["commit", "-m", "Without tree"]);
    run_git(&root, &["tag", "without-tree"]);
    run_git(&root, &["checkout", "with-tree"]);
    let mut rules = load_gitignore_rules(&root);
    let (_watcher, outcome, _rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    assert!(rules.state.plan.worktree_dirs.contains(&tree));
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut raw = notify::RecommendedWatcher::new(raw_tx, notify::Config::default()).unwrap();
    raw.watch(&root, notify::RecursiveMode::NonRecursive)
        .unwrap();
    fs::rename(root.join("source"), root.join("renamed")).unwrap();
    fs::rename(root.join("renamed"), root.join("source")).unwrap();
    fs::remove_dir_all(root.join("source")).unwrap();
    fs::create_dir_all(&tree).unwrap();
    fs::write(tree.join("file.txt"), "tracked").unwrap();
    run_git(&root, &["mv", "source", "moved"]);
    run_git(&root, &["commit", "-m", "Move watched tree"]);
    run_git(&root, &["checkout", "without-tree"]);
    run_git(&root, &["checkout", "with-tree"]);
    let mut dot_git_modifies = 0;
    while let Ok(Ok(event)) = raw_rx.recv_timeout(Duration::from_millis(500)) {
        if matches!(event.kind, EventKind::Modify(_))
            && event
                .paths
                .iter()
                .any(|path| normalized(path) == root.join(".git"))
        {
            dot_git_modifies += 1;
        }
    }
    eprintln!(
        "Windows recursive folder operations passed; root .git Modify notifications: {dot_git_modifies}"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn incremental_directory_limit_warns_and_recovers() {
    let (_temp, root) = repository();
    let reloads = Arc::new(AtomicU64::new(0));
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(FaultyBackend {
            reloads: reloads.clone(),
            ..Default::default()
        }),
        MonitorConfig {
            dir_limit: 16,
            idle_tick: Duration::from_millis(100),
            recovery_interval: Duration::from_millis(800),
            ..Default::default()
        },
    );
    monitor.settle();
    let loaded = reloads.load(Ordering::Relaxed);
    let added = root.join("new-tree");
    let mut nested = added.clone();
    for _ in 0..20 {
        nested.push("nested");
    }
    fs::create_dir_all(&nested).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match monitor
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Msg::RepoWatchDegraded { .. }) => break,
            Ok(Msg::RepoExternallyChanged { .. }) => {}
            other => panic!("incremental coverage loss was not reported: {other:?}"),
        }
    }
    assert_eq!(
        reloads.load(Ordering::Relaxed),
        loaded,
        "incremental add rebuilt the whole repository"
    );
    fs::remove_dir_all(&added).unwrap();
    // The throttled retry recovers from the limit and reconciles missed state.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match monitor
            .rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        {
            Ok(Msg::RepoExternallyChanged { change, .. })
                if change == RepoExternalChange::all() =>
            {
                break;
            }
            Ok(Msg::RepoExternallyChanged { .. }) => {}
            other => panic!("partial coverage never recovered: {other:?}"),
        }
    }
    monitor.settle();
    let loaded = reloads.load(Ordering::Relaxed);
    monitor.quiet();
    assert_eq!(
        reloads.load(Ordering::Relaxed),
        loaded,
        "healthy coverage kept retrying"
    );
    fs::create_dir_all(root.join("after/nested")).unwrap();
    monitor.refresh();
    fs::write(
        root.join("after/nested/source.txt"),
        "observed after recovery",
    )
    .unwrap();
    monitor.refresh();
}
