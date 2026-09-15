use super::*;

#[test]
fn unborn_repository_watches_subfolders_without_creating_index() {
    let temp = unique_temp_dir("gitcomet-unborn-watch");
    let root = normalized(&temp.path().canonicalize().unwrap());
    run_git(&root, &["init"]);
    fs::create_dir_all(root.join("source")).unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    fs::write(root.join("source/file.txt"), "before").unwrap();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    assert!(!root.join(".git/index").exists());
    let mut rules = load_gitignore_rules(&root);
    assert!(
        !rules.failed,
        "a missing first index is a valid empty snapshot"
    );
    let (_watcher, outcome, rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    ready(&root, &rx);
    fs::write(root.join("source/file.txt"), "after").unwrap();
    fs::write(root.join("node_modules/pkg/ignored"), "churn").unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("source/file.txt")))
    );
    assert!(
        events
            .iter()
            .flat_map(|event| &event.paths)
            .all(|path| !path.starts_with(root.join("node_modules")))
    );
    assert!(
        !root.join(".git/index").exists(),
        "monitoring wrote the index"
    );
}

#[test]
fn corrupt_index_is_not_treated_as_an_empty_repository() {
    let (_temp, root) = repository();
    let path = root.join(".git/index");
    let mut index = fs::read(&path).unwrap();
    index[..4].copy_from_slice(b"BAD!");
    fs::write(path, index).unwrap();
    assert!(load_gitignore_rules(&root).failed);
}

fn changed_snapshot_during_replacement(index: bool) {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("vendor/pkg")).unwrap();
    fs::write(root.join("vendor/pkg/real.txt"), "before").unwrap();
    fs::write(root.join(".gitignore"), "vendor/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (old, _, rx) = rules.start_watcher(&root);
    ready(&root, &rx);
    drop(old);
    rules.reload(&root);
    let hook_root = root.clone();
    let mut changed = false;
    rules.config = MonitorConfig {
        before_registration: Some(Box::new(move || {
            if !changed {
                changed = true;
                if index {
                    run_git(&hook_root, &["add", "-f", "vendor/pkg/real.txt"]);
                } else {
                    fs::write(hook_root.join(".gitignore"), "").unwrap();
                }
            }
        })),
        ..Default::default()
    };
    let (_watcher, outcome, rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    ready(&root, &rx);
    fs::write(root.join("vendor/pkg/real.txt"), "after").unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("vendor/pkg/real.txt"))),
        "replacement validated a stale ignore/index snapshot: {events:?}"
    );
}

#[test]
fn replacement_reloads_ignore_rules_changed_in_registration_gap() {
    changed_snapshot_during_replacement(false);
}

#[test]
fn replacement_reloads_tracked_exceptions_changed_in_registration_gap() {
    changed_snapshot_during_replacement(true);
}

fn replacement_reconciles_all_state(ignore_trigger: bool, recovery: bool) {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("source")).unwrap();
    fs::write(root.join("source/file.txt"), "before").unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    let unavailable = Arc::new(AtomicBool::new(recovery));
    let armed = Arc::new(AtomicBool::new(false));
    let hook_armed = armed.clone();
    let hook_root = root.clone();
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(FaultyBackend {
            load_failure: unavailable.clone(),
            ..Default::default()
        }),
        MonitorConfig {
            before_registration: Some(Box::new(move || {
                if hook_armed.swap(false, Ordering::Relaxed) {
                    fs::write(hook_root.join("source/file.txt"), "during gap").unwrap();
                    run_git(&hook_root, &["add", "source/file.txt"]);
                    let head = fs::read_to_string(hook_root.join(".git/HEAD")).unwrap();
                    let oid = fs::read(
                        hook_root
                            .join(".git")
                            .join(head.trim().strip_prefix("ref: ").unwrap()),
                    )
                    .unwrap();
                    fs::write(hook_root.join(".git/refs/heads/gap"), &oid).unwrap();
                    fs::write(hook_root.join(".git/refs/tags/gap"), oid).unwrap();
                    fs::write(hook_root.join(".git/HEAD"), "ref: refs/heads/gap\n").unwrap();
                }
            })),
            idle_tick: Duration::from_millis(100),
            recovery_interval: Duration::from_millis(100),
            ..Default::default()
        },
    );
    // Startup may report the deliberately injected degraded state.
    for msg in monitor.rx.try_iter() {
        assert!(matches!(msg, Msg::RepoWatchDegraded { .. }));
    }
    armed.store(true, Ordering::Relaxed);
    if recovery {
        unavailable.store(false, Ordering::Relaxed);
    } else if ignore_trigger {
        monitor
            .tx
            .send(MonitorMsg::Event(Ok(notify::Event::new(
                EventKind::Modify(ModifyKind::Any),
            )
            .add_path(root.join(".gitignore")))))
            .unwrap();
    } else {
        monitor
            .tx
            .send(MonitorMsg::Event(Ok(
                notify::Event::new(EventKind::Any).set_flag(notify::event::Flag::Rescan)
            )))
            .unwrap();
    }
    let result = monitor.rx.recv_timeout(Duration::from_secs(10));
    assert!(!armed.load(Ordering::Relaxed), "gap injection never ran");
    match result {
        Ok(Msg::RepoExternallyChanged { change, .. }) => {
            assert_eq!(change, RepoExternalChange::all())
        }
        other => panic!("replacement did not reconcile all state: {other:?}"),
    }
    // Native directory metadata notifications can arrive after registration.
    // The first refresh must already reconcile everything; later events must
    // settle within the same bounded window used by other native tests.
    monitor.settle();
}

#[test]
#[cfg(target_os = "linux")]
fn coverage_only_replacement_refreshes_all_state() {
    replacement_reconciles_all_state(false, false);
}

#[test]
fn ignore_replacement_refreshes_all_state() {
    replacement_reconciles_all_state(true, false);
}

#[test]
fn degraded_recovery_refreshes_all_state() {
    replacement_reconciles_all_state(false, true);
}
