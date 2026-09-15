use super::*;

fn lost_boundary_events_are_reconciled(error: bool) {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    let boundary = root.join("build");
    fs::create_dir(&boundary).unwrap();
    let (callback_tx, callback_rx) = mpsc::channel();
    let monitor = RunningMonitor::start_with_callback(
        &root,
        Arc::new(gitcomet_git_gix::GixBackend),
        MonitorConfig::default(),
        Some(callback_tx),
    );
    // Keep real native registrations, but deliberately discard their lifecycle
    // events to reproduce an overflow without depending on OS buffer sizes.
    fs::remove_dir(&boundary).unwrap();
    fs::write(&boundary, "unignored replacement").unwrap();
    monitor.quiet();
    while callback_rx.try_recv().is_ok() {}
    let notification = if error {
        Err(notify::Error::generic("injected native event loss"))
    } else {
        Ok(notify::Event::new(EventKind::Any).set_flag(notify::event::Flag::Rescan))
    };
    monitor.tx.send(MonitorMsg::Event(notification)).unwrap();
    monitor.refresh();
    while callback_rx.try_recv().is_ok() {}
    // A refresh alone cannot pass: the callback must admit the next real edit
    // using a reconciled policy, and the monitor must refresh for that edit.
    fs::write(&boundary, "later in-place edit").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match callback_rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(MonitorMsg::Event(Ok(event))) if event.paths.contains(&boundary) => {
                monitor.tx.send(MonitorMsg::Event(Ok(event))).unwrap();
                break;
            }
            Ok(_) => {}
            Err(error) => panic!("replacement edit is still excluded after event loss: {error}"),
        }
    }
    monitor.refresh();
}

#[test]
fn rescan_reconciles_exclusions_after_lost_boundary_events() {
    lost_boundary_events_are_reconciled(false);
}

#[test]
fn native_error_reconciles_exclusions_after_lost_boundary_events() {
    lost_boundary_events_are_reconciled(true);
}

#[test]
fn full_rebuild_discards_nested_ignore_inputs_in_newly_ignored_trees() {
    let (_temp, root) = repository();
    let directory = root.join("build/pkg");
    fs::create_dir_all(&directory).unwrap();
    let nested_ignore = directory.join(".gitignore");
    fs::write(&nested_ignore, "*.log\n").unwrap();
    let builds = Arc::new(AtomicU64::new(0));
    let count = builds.clone();
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(gitcomet_git_gix::GixBackend),
        MonitorConfig {
            idle_tick: Duration::from_millis(100),
            before_registration: Some(Box::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
            })),
            ..Default::default()
        },
    );
    monitor.settle();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    monitor.refresh();
    let before = builds.load(Ordering::Relaxed);
    fs::write(&nested_ignore, "*.log\n*.generated\n").unwrap();
    monitor.revalidate();
    monitor.quiet(); // Covers native events, activation and idle stamp checks.
    assert_eq!(builds.load(Ordering::Relaxed), before);
    fs::write(root.join(".gitignore"), "").unwrap();
    monitor.refresh();
    fs::write(&nested_ignore, "*.generated\n").unwrap();
    monitor.refresh(); // A newly visible input must be discovered again.
}

fn apply_directory_event(
    rules: &mut TestRules,
    watcher: &mut MonitorWatcher,
    event: notify::Event,
) {
    let effect = summarize(&rules.state.snapshot(), &mut rules.state.rules, &event);
    rules
        .state
        .apply_directories(&effect, watcher, &rules.config);
}

#[test]
fn newly_ignored_directory_removal_prunes_watches() {
    for rename in [false, true] {
        let (_temp, root) = repository();
        fs::write(root.join(".gitignore"), "generated/\n").unwrap();
        let directory = root.join("generated");
        let relative = "generated/nested/source.txt";
        fs::create_dir_all(directory.join("nested")).unwrap();
        fs::write(root.join(relative), "tracked exception").unwrap();
        run_git(&root, &["add", "-f", relative]);
        let mut rules = load_gitignore_rules(&root);
        let (mut watcher, _, _rx) = rules.start_watcher(&root);
        assert!(rules.state.plan.dirs.contains(&directory.join("nested")));
        run_git(&root, &["rm", "--cached", "-f", relative]);
        rules.reload(&root);
        let kind = if rename {
            fs::rename(&directory, root.join("moved")).unwrap();
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::From))
        } else {
            fs::remove_dir_all(&directory).unwrap();
            EventKind::Remove(RemoveKind::Folder)
        };
        let effect = summarize(
            &rules.state.snapshot(),
            &mut rules,
            &notify::Event::new(kind).add_path(directory.clone()),
        );
        // A rename has no directory hint once the source is gone, so it can
        // conservatively refresh. An explicitly ignored folder removal cannot.
        if !rename {
            assert_eq!(effect.change, None, "ignored removal should stay quiet");
        }
        rules
            .state
            .apply_directories(&effect, &mut watcher, &rules.config);
        assert!(
            rules
                .state
                .plan
                .dirs
                .iter()
                .all(|path| !path.starts_with(&directory)),
            "ignored removal left stale directory coverage: {effect:?}"
        );
        #[cfg(target_os = "linux")]
        assert!(
            watcher
                .watched
                .iter()
                .all(|path| !path.starts_with(&directory))
        );
    }
}

#[test]
fn directory_batch_deduplicates_ignored_boundaries() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (mut watcher, _, _rx) = rules.start_watcher(&root);
    let parent = root.join("source");
    let ignored = parent.join("generated");
    fs::create_dir_all(&ignored).unwrap();
    apply_directory_event(
        &mut rules,
        &mut watcher,
        notify::Event::new(EventKind::Create(CreateKind::Folder))
            .add_path(parent)
            .add_path(ignored.clone()),
    );
    assert_eq!(rules.state.plan.boundaries, vec![ignored.clone()]);
    assert!(rules.state.snapshot().excluded_roots.contains(&ignored));
    // A plain file removal must not rebuild the policy or scan all boundaries.
    let before = rules.state.snapshot();
    apply_directory_event(
        &mut rules,
        &mut watcher,
        notify::Event::new(EventKind::Remove(RemoveKind::File))
            .add_path(root.join("source/removed.txt")),
    );
    assert!(Arc::ptr_eq(&before, &rules.state.snapshot()));
}

#[test]
fn ignored_directory_churn_prunes_obsolete_boundaries() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "generated-*/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (mut watcher, outcome, _rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    for index in 0..2_000 {
        let path = root.join(format!("generated-{index}"));
        fs::create_dir(&path).unwrap();
        apply_directory_event(
            &mut rules,
            &mut watcher,
            notify::Event::new(EventKind::Create(CreateKind::Folder)).add_path(path.clone()),
        );
        fs::remove_dir(&path).unwrap();
        apply_directory_event(
            &mut rules,
            &mut watcher,
            notify::Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(path),
        );
    }
    assert_eq!(
        rules.state.plan.boundaries.len(),
        0,
        "deleted boundaries accumulated in the plan"
    );
    assert_eq!(
        rules.state.snapshot().excluded_roots.len(),
        0,
        "deleted boundaries accumulated in the callback policy"
    );
    assert!(rules.cache.len() <= GITIGNORE_CACHE_MAX_ENTRIES);
}

#[test]
fn parent_rename_and_removal_prune_nested_ignored_boundaries() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    fs::create_dir_all(root.join("source/generated")).unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (mut watcher, _, _rx) = rules.start_watcher(&root);
    fs::rename(root.join("source"), root.join("moved")).unwrap();
    apply_directory_event(
        &mut rules,
        &mut watcher,
        notify::Event::new(EventKind::Modify(ModifyKind::Name(
            notify::event::RenameMode::Both,
        )))
        .add_path(root.join("source"))
        .add_path(root.join("moved")),
    );
    assert_eq!(
        rules.state.plan.boundaries,
        vec![root.join("moved/generated")]
    );
    assert_eq!(
        rules
            .state
            .snapshot()
            .excluded_roots
            .iter()
            .collect::<Vec<_>>(),
        vec![&root.join("moved/generated")]
    );
    fs::remove_dir_all(root.join("moved")).unwrap();
    apply_directory_event(
        &mut rules,
        &mut watcher,
        notify::Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(root.join("moved")),
    );
    assert!(rules.state.plan.boundaries.is_empty());
    assert!(rules.state.snapshot().excluded_roots.is_empty());
}

#[test]
fn late_removal_event_keeps_a_recreated_ignored_boundary() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    let path = root.join("generated");
    fs::create_dir(&path).unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (mut watcher, _, _rx) = rules.start_watcher(&root);
    fs::remove_dir(&path).unwrap();
    fs::create_dir(&path).unwrap();
    apply_directory_event(
        &mut rules,
        &mut watcher,
        notify::Event::new(EventKind::Remove(RemoveKind::Folder)).add_path(path.clone()),
    );
    assert_eq!(rules.state.plan.boundaries, vec![path.clone()]);
    assert!(rules.state.snapshot().excluded_roots.contains(&path));
}

#[test]
fn delayed_file_removal_refreshes_before_excluding_its_replacement_directory() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    let path = root.join("build");
    fs::write(&path, "visible file").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (mut watcher, _, _rx) = rules.start_watcher(&root);
    let snapshot = rules.state.snapshot();
    let edit = notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(path.clone());
    assert!(
        summarize(&snapshot, &mut rules, &edit)
            .change
            .unwrap()
            .worktree
    );
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    // Windows removal notifications have no entry kind. The current directory
    // must not overwrite the cached classification of the file that vanished.
    let removed = notify::Event::new(EventKind::Remove(RemoveKind::Any)).add_path(path.clone());
    let effect = summarize(&snapshot, &mut rules, &removed);
    assert!(effect.change.is_some_and(|change| change.worktree));
    rules
        .state
        .apply_directories(&effect, &mut watcher, &rules.config);
    assert!(rules.state.snapshot().excluded_roots.contains(&path));
}

#[test]
fn index_reload_keeps_visible_nested_ignore_inputs() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("source/nested")).unwrap();
    let ignore = root.join("source/nested/.gitignore");
    fs::write(&ignore, "*.log\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (_watcher, _, _rx) = rules.start_watcher(&root);
    assert!(rules.state.inputs.inputs.contains(&ignore));
    assert!(
        rules
            .state
            .reload(&root, &gitcomet_git_gix::GixBackend, true)
    );
    assert!(rules.state.inputs.inputs.contains(&ignore));
    fs::write(&ignore, "*.log\n*.generated\n").unwrap();
    assert!(rules.state.inputs.stamps.changed());
}
