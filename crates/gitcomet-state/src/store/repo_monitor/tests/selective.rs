use super::*;
mod efficiency;
mod native_lifecycle;
mod path_identities;
mod policy_lifecycle;
mod runtime_recovery;
mod setup_recovery;
mod storage_and_links;

fn repository() -> (tempfile::TempDir, PathBuf) {
    let temp = unique_temp_dir("gitcomet-selective");
    let root = normalized(&temp.path().canonicalize().unwrap());
    init_repo_for_ignore_tests(&root);
    (temp, root)
}

#[test]
fn deinitialized_submodule_keeps_parent_coverage() {
    let (_temp, root) = repository();
    let (_seed_temp, seed) = repository();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join("src/real.txt"), "before").unwrap();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    let subpath = "deps/child";
    run_git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            seed.to_str().unwrap(),
            subpath,
        ],
    );
    run_git(&root, &["commit", "-am", "Add submodule"]);
    let child = root.join(subpath);
    let retained_git_dir = normalized(&resolve_git_dir(&child).unwrap());
    run_git(&root, &["submodule", "deinit", "-f", "--", subpath]);
    assert!(!child.join(".git").exists());
    assert!(retained_git_dir.join("HEAD").is_file());

    // A fresh open used to fail its entire ignore policy while opening the
    // absent child checkout, leaving every parent source subdirectory unwatched.
    let mut rules = load_gitignore_rules(&root);
    assert!(
        !rules.failed,
        "deinitialized child broke the parent ignore policy"
    );
    assert!(rules.matcher.is_some());
    assert!(!rules.state.inputs.info.worktrees.contains(&child));
    assert!(rules.state.inputs.info.git_dirs.contains(&retained_git_dir));
    assert!(
        rules
            .state
            .inputs
            .info
            .ignore_inputs
            .contains(&child.join(".git"))
    );
    let (watcher, outcome, rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    assert!(rules.state.plan.worktree_dirs.contains(&root.join("src")));
    assert!(
        !rules
            .state
            .plan
            .worktree_dirs
            .contains(&root.join("node_modules"))
    );
    assert!(
        rules
            .state
            .policy
            .read()
            .unwrap()
            .is_cache(&retained_git_dir.join("lfs/tmp/clean"))
    );
    ready(&root, &rx);
    fs::write(root.join("src/real.txt"), "after").unwrap();
    let retained_head = retained_git_dir.join("HEAD");
    fs::write(&retained_head, fs::read(&retained_head).unwrap()).unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("src/real.txt")))
    );
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&retained_head))
    );
    drop(watcher);

    // Reinitializing restores the child matcher without losing metadata coverage.
    run_git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "update",
            "--init",
            "--",
            subpath,
        ],
    );
    rules.reload(&root);
    assert!(!rules.failed);
    assert!(rules.state.inputs.info.worktrees.contains(&child));
    assert!(
        rules
            .submodule_matchers
            .iter()
            .any(|(path, _)| path == &child)
    );
}

fn directory_created_during_scan_keeps_live_coverage(replacing: bool) {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("source")).unwrap();
    // Include more boundaries than macOS can exclude natively, exercising the
    // shared policy filtering alongside native coverage.
    let mut ignore = "node_modules/\n".to_string();
    for index in 0..9 {
        fs::create_dir_all(root.join(format!("ignored-{index}"))).unwrap();
        ignore.push_str(&format!("ignored-{index}/\n"));
    }
    fs::write(root.join(".gitignore"), ignore).unwrap();
    let mut rules = load_gitignore_rules(&root);
    if replacing {
        let (watcher, _, rx) = rules.start_watcher(&root);
        ready(&root, &rx);
        drop(watcher); // Production releases registrations before replacement.
        rules.reload(&root);
    }
    let created = Arc::new(AtomicBool::new(false));
    let hook_created = created.clone();
    let hook_root = root.clone();
    rules.config = MonitorConfig {
        before_registration: Some(Box::new(move || {
            // Inject before registration. The old scan-before-register setup
            // missed these children; parent-first scanning must discover them.
            if !hook_created.swap(true, Ordering::Relaxed) {
                fs::create_dir_all(hook_root.join("late/nested")).unwrap();
                fs::write(hook_root.join("late/nested/real.txt"), "before").unwrap();
                fs::create_dir_all(hook_root.join("late/node_modules/pkg")).unwrap();
            }
        })),
        ..Default::default()
    };
    let (_watcher, outcome, rx) = rules.start_watcher(&root);
    assert!(
        created.load(Ordering::Relaxed),
        "race injection did not run"
    );
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    ready(&root, &rx);
    fs::write(root.join("late/nested/real.txt"), "after").unwrap();
    fs::write(root.join("late/node_modules/pkg/ignored"), "churn").unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("late/nested/real.txt"))),
        "directory created during the scan has no live native coverage: {events:?}"
    );
    assert!(
        rules
            .state
            .plan
            .worktree_dirs
            .contains(&root.join("late/nested"))
    );
    assert!(
        !rules
            .state
            .plan
            .worktree_dirs
            .contains(&root.join("late/node_modules"))
    );
    assert!(
        events
            .iter()
            .flat_map(|event| &event.paths)
            .all(|path| !path.starts_with(root.join("late/node_modules")))
    );
}

#[test]
fn directory_created_during_startup_scan_is_watched() {
    directory_created_during_scan_keeps_live_coverage(false);
}

#[test]
fn directory_created_during_replacement_scan_is_watched() {
    directory_created_during_scan_keeps_live_coverage(true);
}

#[test]
fn root_ignore_edit_reloads_after_matching_an_ignored_directory() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("vendor")).unwrap();
    fs::create_dir_all(root.join("ignored")).unwrap();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(rules.is_ignored_rel(Path::new("ignored"), Some(true)));
    fs::write(root.join(".gitignore"), "ignored/\nvendor/\n").unwrap();
    let event =
        notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(root.join(".gitignore"));
    assert!(summarize_event(&root, Some(&root.join(".git")), &mut rules, &event).policy_dirty);
    rules.reload(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(!plan.worktree_dirs.contains(&root.join("vendor")));
}

#[test]
fn watch_plan_prunes_ignored_and_git_cache_trees() {
    let (_temp, root) = repository();
    for path in [
        "node_modules/pkg/nested",
        ".git/lfs/tmp",
        ".git/objects/ab",
        "src/nested",
    ] {
        fs::create_dir_all(root.join(path)).unwrap();
    }
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    for path in ["node_modules", ".git/lfs", ".git/objects"] {
        assert!(
            plan.dirs
                .iter()
                .all(|dir| !dir.starts_with(root.join(path))),
            "{path}"
        );
    }
    assert!(plan.dirs.contains(&root.join("src/nested")));
    assert!(plan.dirs.contains(&root.join(".git/refs/heads")));
    assert!(plan.policy.relevant(&root.join(".git/HEAD")));
}

#[cfg(unix)]
#[test]
fn disabled_config_source_does_not_watch_device_activity() {
    let (_temp, root) = repository();
    let mut rules = TestRules::default();
    rules.state.inputs.add_inputs(vec![
        PathBuf::from("/dev/null"),
        root.join("missing-config"),
    ]);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert_ne!(
        plan.policy.classify(Path::new("/dev/null")),
        PathClass::Control
    );

    assert_eq!(
        plan.policy.classify(&root.join("missing-config")),
        PathClass::Control
    );
}

#[test]
fn newly_created_ignored_directory_immediately_suppresses_descendants() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    let event = notify::Event::new(EventKind::Create(notify::event::CreateKind::Folder))
        .add_path(root.join("node_modules"));
    let effect = summarize_event(&root, Some(&root.join(".git")), &mut rules, &event);
    assert_eq!(effect.new_ignored_dirs, vec![root.join("node_modules")]);
    let mut policy = (*rules.state.snapshot()).clone();
    policy.excluded_roots.insert(root.join("node_modules"));
    assert_eq!(
        policy.classify(&root.join("node_modules/pkg/.gitignore")),
        PathClass::Excluded
    );
    // Recheck lifecycle events at the boundary, but suppress them while it
    // remains an ignored directory. Its descendants never reach the matcher.
    assert_eq!(triage(&policy, &event), Triage::Relevant);
    assert_eq!(
        summarize(&policy, &mut rules, &event),
        EventEffect::default()
    );
    let descendant = notify::Event::new(EventKind::Create(CreateKind::File))
        .add_path(root.join("node_modules/pkg/file.txt"));
    assert_eq!(triage(&policy, &descendant), Triage::Drop);
    assert!(policy.relevant(&root.join(".gitignore")));
}

#[test]
fn index_updates_refresh_tracked_exceptions_under_ignored_directories() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let tracked = root.join("node_modules/pkg/keep.txt");
    fs::write(&tracked, "tracked exception").unwrap();
    let mut rules = load_gitignore_rules(&root);
    assert!(rules.is_ignored_rel(Path::new("node_modules"), Some(true)));
    run_git(&root, &["add", "-f", "node_modules/pkg/keep.txt"]);
    let effect = summarize_event(
        &root,
        Some(&root.join(".git")),
        &mut rules,
        &notify::Event::new(EventKind::Any).add_path(root.join(".git/index")),
    );
    assert!(effect.index_dirty);
    rules.reload(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.dirs.contains(&root.join("node_modules/pkg")));
    assert!(!rules.is_ignored_rel(Path::new("node_modules/pkg/keep.txt"), Some(false)));
    assert!(rules.is_ignored_rel(Path::new("node_modules/pkg/other.txt"), Some(false)));
    run_git(
        &root,
        &["rm", "--cached", "-f", "node_modules/pkg/keep.txt"],
    );
    rules.reload(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(!plan.dirs.contains(&root.join("node_modules")));
}

#[test]
fn ignore_reload_failure_retains_last_valid_policy_and_recovers() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let config = root.join(".git/config");
    let original = fs::read(&config).unwrap();
    fs::write(&config, "[invalid\n").unwrap();
    rules.reload(&root);
    assert!(rules.failed);
    assert!(rules.is_ignored_rel(Path::new("node_modules"), Some(true)));
    fs::write(&config, original).unwrap();
    rules.reload(&root);
    assert!(!rules.failed);
}

#[test]
fn lfs_cache_events_never_keep_loads_in_flight() {
    use crate::model::RepoLoadsInFlight;
    let (_temp, root) = repository();
    let mut rules = load_gitignore_rules(&root);
    let mut loads = RepoLoadsInFlight::default();
    let mut debounce = DebouncedChange::new(Duration::from_millis(250), Duration::from_secs(2));
    let start = Instant::now();
    let flag = RepoLoadsInFlight::WORKTREE_STATUS;
    assert!(loads.request(flag));
    for index in 0..100 {
        let event =
            notify::Event::new(EventKind::Any).add_path(root.join(format!(".git/lfs/tmp/{index}")));
        if let Some(change) = classify_change(&root, Some(&root.join(".git")), &mut rules, &event) {
            debounce.push(change, start);
        }
    }
    assert!(
        debounce
            .take_if_due(start + Duration::from_secs(3))
            .is_none()
    );
    assert!(!loads.finish(flag));
    assert!(!loads.any_in_flight());
    assert!(loads.request(flag));
    let event = notify::Event::new(EventKind::Any).add_path(root.join("real.txt"));
    debounce.push(
        classify_change(&root, Some(&root.join(".git")), &mut rules, &event).unwrap(),
        start,
    );
    assert!(
        debounce
            .take_if_due(start + Duration::from_secs(3))
            .is_some()
    );
    assert!(!loads.request(flag));
    assert!(loads.finish(flag));
    assert!(!loads.finish(flag));
    assert!(!loads.any_in_flight());
}

#[test]
fn directory_budget_and_renames_rebuild_coverage_without_stale_counts() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("source/child")).unwrap();
    fs::create_dir_all(root.join("extra")).unwrap();
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build_with_limit(&root, Some(&root.join(".git")), &mut rules, 1);
    assert!(plan.skipped.is_some());
    assert_eq!(plan.worktree_dirs.len(), 1);
    assert!(plan.dirs.contains(&root.join(".git")));
    fs::rename(root.join("source"), root.join("moved")).unwrap();
    fs::write(root.join(".gitignore"), "extra/\n").unwrap();
    rules.reload(&root);
    for _ in 0..2 {
        let plan = TestPlan::build_with_limit(&root, Some(&root.join(".git")), &mut rules, 4);
        assert!(plan.skipped.is_none());
        assert_eq!(plan.worktree_dirs.len(), 3);
        assert!(plan.dirs.contains(&root.join("moved/child")));
        assert!(!plan.dirs.contains(&root.join("source")));
    }
    // Exactly one directory over the cap must also mark traversal incomplete,
    // even with no more pending entries to trigger another iteration.
    let plan = TestPlan::build_with_limit(&root, Some(&root.join(".git")), &mut rules, 2);
    assert_eq!(plan.skipped, Some(3));
    assert_eq!(plan.worktree_dirs.len(), 2);
    assert!(plan.worktree_dirs.contains(&root.join("moved")));
}

#[test]
fn worktree_index_events_preserve_other_checkout_git_state() {
    let (_temp, root) = repository();
    let linked_temp = unique_temp_dir("gitcomet-linked-index");
    let linked = linked_temp.path().join("checkout");
    run_git(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let linked = normalized(&linked.canonicalize().unwrap());
    let main_git = root.join(".git");
    let linked_git = normalized(&resolve_git_dir(&linked).unwrap());
    for (workdir, own_git, other_git) in [
        (&root, &main_git, &linked_git),
        (&linked, &linked_git, &main_git),
    ] {
        let mut rules = load_gitignore_rules(workdir);
        for (git, own) in [(own_git, true), (other_git, false)] {
            let effect = summarize_event(
                workdir,
                Some(own_git),
                &mut rules,
                &notify::Event::new(EventKind::Modify(ModifyKind::Any)).add_path(git.join("index")),
            );
            let change = effect.change.expect("index changes must refresh");
            assert_eq!(change.git_state, !own, "workdir={workdir:?}, index={git:?}");
            if own {
                assert!(change.index && effect.index_dirty);
            }
        }
    }
}

#[test]
fn linked_worktree_watches_own_index_and_common_refs_without_caches() {
    let (_temp, root) = repository();
    let linked_temp = unique_temp_dir("gitcomet-linked-watch");
    let linked = linked_temp.path().join("checkout");
    run_git(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let linked = normalized(&linked.canonicalize().unwrap());
    let mut rules = load_gitignore_rules(&linked);
    let git = resolve_git_dir(&linked).unwrap();
    let plan = TestPlan::build(&linked, Some(&git), &mut rules);
    assert!(plan.policy.git_roots.contains(&root.join(".git")));
    assert!(plan.policy.git_roots.contains(&normalized(&git)));
    for path in [
        git.join("index"),
        root.join(".git/refs/heads/main"),
        git.join("HEAD"),
    ] {
        assert!(
            classify_change(
                &linked,
                Some(&git),
                &mut rules,
                &notify::Event::new(EventKind::Any).add_path(path)
            )
            .is_some()
        );
    }
    for path in [
        git.join("lfs/tmp/clean"),
        root.join(".git/lfs/tmp/clean"),
        root.join(".git/objects/ab/object"),
    ] {
        assert!(
            classify_change(
                &linked,
                Some(&git),
                &mut rules,
                &notify::Event::new(EventKind::Any).add_path(path)
            )
            .is_none()
        );
    }
}

#[test]
fn linked_worktree_without_exclusions_keeps_native_coverage() {
    let (_temp, root) = repository();
    let linked_temp = unique_temp_dir("gitcomet-linked-native");
    let linked = linked_temp.path().join("checkout");
    run_git(
        &root,
        &["worktree", "add", "-b", "linked", linked.to_str().unwrap()],
    );
    let linked = normalized(&linked.canonicalize().unwrap());
    // Every cache lives in the main Git directory, and nothing is ignored, so
    // a recursive checkout stream has no native exclusions at all.
    let mut rules = load_gitignore_rules(&linked);
    let (_watcher, outcome, rx) = rules.start_watcher(&linked);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    ready(&root, &rx);
    fs::write(linked.join("source.txt"), "edit").unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&linked.join("source.txt"))),
        "{events:?}"
    );
}

#[test]
fn external_ignore_inputs_and_missing_config_includes_are_observed() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-external-ignore");
    let excludes = external.path().join("ignore");
    let include = external.path().join("future-config");
    fs::write(&excludes, "generated/\n").unwrap();
    run_git(
        &root,
        &["config", "core.excludesFile", excludes.to_str().unwrap()],
    );
    run_git(
        &root,
        &["config", "include.path", include.to_str().unwrap()],
    );
    let mut rules = load_gitignore_rules(&root);
    assert!(rules.state.inputs.info.ignore_inputs.contains(&excludes));
    assert!(rules.state.inputs.info.ignore_inputs.contains(&include));
    assert!(rules.is_ignored_rel(Path::new("generated"), Some(true)));
    fs::write(&excludes, "other/\n").unwrap();
    let change = summarize_event(
        &root,
        Some(&root.join(".git")),
        &mut rules,
        &notify::Event::new(EventKind::Any).add_path(excludes),
    );
    assert!(change.policy_dirty);
    rules.reload(&root);
    assert!(!rules.is_ignored_rel(Path::new("generated"), Some(true)));
    assert!(rules.is_ignored_rel(Path::new("other"), Some(true)));
}

/// A "since now" FSEvents stream still receives kernel events that fseventsd
/// had not read when the stream started, so fixture writes made just before a
/// monitor starts can reach it as real changes. Once a later marker has been
/// delivered, every event queued before that marker is too old to arrive.
#[cfg(target_os = "macos")]
fn flush_native_events() {
    let temp = unique_temp_dir("gitcomet-fsevents-barrier");
    let dir = normalized(&temp.path().canonicalize().unwrap());
    let marker = dir.join("marker");
    let (tx, rx) = mpsc::channel::<notify::Result<notify::Event>>();
    let (_stream, errors) = gitcomet_fs_watch::FsEventsWatcher::new(vec![(dir, Vec::new())], tx);
    assert!(errors.is_empty(), "barrier stream failed: {errors:?}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "no FSEvents barrier event");
        fs::write(&marker, "barrier").unwrap();
        if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(100))
            && event.paths.contains(&marker)
        {
            return;
        }
    }
}
#[cfg(not(target_os = "macos"))]
fn flush_native_events() {}

struct RunningMonitor {
    tx: mpsc::Sender<MonitorMsg>,
    rx: mpsc::Receiver<Msg>,
    thread: Option<std::thread::JoinHandle<()>>,
    native_events: Arc<AtomicU64>,
}
impl RunningMonitor {
    fn revalidate(&self) {
        self.tx.send(MonitorMsg::Revalidate).unwrap();
    }
    fn start(root: &Path) -> Self {
        let monitor = Self::start_custom(
            root,
            Arc::new(gitcomet_git_gix::GixBackend),
            MonitorConfig::default(),
        );
        // Recursive Windows coverage can receive deferred parent-directory
        // metadata from fixture creation. Settle that bounded startup residue
        // before asserting on edits made after the monitor is ready.
        monitor.settle();
        monitor
    }
    fn start_custom(root: &Path, backend: Arc<dyn GitBackend>, config: MonitorConfig) -> Self {
        Self::start_with_callback(root, backend, config, None)
    }
    fn start_with_callback(
        root: &Path,
        backend: Arc<dyn GitBackend>,
        mut config: MonitorConfig,
        callback_tx: Option<mpsc::Sender<MonitorMsg>>,
    ) -> Self {
        flush_native_events();
        let (tx, rx) = mpsc::channel();
        let (store_tx, store_rx) = mpsc::channel();
        let root = root.to_path_buf();
        let thread_tx = callback_tx.unwrap_or_else(|| tx.clone());
        let native_events = Arc::new(AtomicU64::new(0));
        config.native_events = Some(native_events.clone());
        let thread = std::thread::spawn(move || {
            repo_monitor_thread(
                RepoId(1),
                root,
                StoreWorkerSender::for_test_msg_sender(store_tx),
                rx,
                thread_tx,
                Arc::new(AtomicU64::new(1)),
                Arc::new(AtomicBool::new(true)),
                backend,
                config,
            )
        });
        let (ready_tx, ready_rx) = mpsc::channel();
        tx.send(MonitorMsg::Barrier(ready_tx)).unwrap();
        ready_rx.recv_timeout(Duration::from_secs(10)).unwrap();
        Self {
            tx,
            rx: store_rx,
            thread: Some(thread),
            native_events,
        }
    }
    #[track_caller]
    fn refresh(&self) {
        match self.rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Msg::RepoExternallyChanged { .. }) => {}
            other => panic!("expected repository refresh, got {other:?}"),
        }
        self.settle();
    }
    fn settle(&self) {
        // Some native backends deliver a parent-directory update after the
        // file event. Bound those follow-ups, then require a full quiet window.
        for followup in 0..=3 {
            match self.rx.recv_timeout(Duration::from_secs(3)) {
                Err(mpsc::RecvTimeoutError::Timeout) => return,
                Ok(Msg::RepoExternallyChanged { .. }) if followup < 3 => {}
                other => panic!("refreshes did not settle: {other:?}"),
            }
        }
    }
    #[track_caller]
    fn quiet(&self) {
        let result = self.rx.recv_timeout(Duration::from_secs(3));
        assert!(
            matches!(result, Err(mpsc::RecvTimeoutError::Timeout)),
            "unexpected refresh while quiet: {result:?}"
        );
    }
}
impl Drop for RunningMonitor {
    fn drop(&mut self) {
        let _ = self.tx.send(MonitorMsg::Stop);
        if let Some(thread) = self.thread.take() {
            thread.join().unwrap();
        }
    }
}

#[test]
fn native_monitor_rebuilds_after_ignore_edits_moves_and_atomic_saves() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("vendor/pkg")).unwrap();
    fs::write(root.join("root.txt"), "before").unwrap();
    let mut ignore = String::new();
    for index in 0..9 {
        fs::create_dir_all(root.join(format!("ignored-{index}"))).unwrap();
        ignore.push_str(&format!("ignored-{index}/\n"));
    }
    fs::write(root.join(".gitignore"), &ignore).unwrap();
    let monitor = RunningMonitor::start(&root);
    fs::write(root.join(".gitignore"), format!("{ignore}vendor/\n")).unwrap();
    monitor.refresh(); // Sent only once replacement watches are installed.
    monitor.quiet();
    fs::write(root.join("vendor/pkg/.gitignore"), "*").unwrap();
    fs::write(root.join("vendor/pkg/ignored.txt"), "churn").unwrap();
    monitor.quiet();
    fs::rename(root.join("vendor"), root.join("source")).unwrap();
    monitor.refresh();
    monitor.quiet();
    fs::write(root.join("source/visible.txt"), "edit").unwrap();
    monitor.refresh();
    monitor.quiet();
    // Portable atomic replacement: rename the original away then the new file in.
    fs::write(root.join("replacement.txt"), "after").unwrap();
    fs::rename(root.join("root.txt"), root.join("old.txt")).unwrap();
    fs::rename(root.join("replacement.txt"), root.join("root.txt")).unwrap();
    monitor.refresh();
    monitor.quiet();
    fs::write(root.join("root.txt"), "second edit").unwrap();
    monitor.refresh();
    monitor.quiet();
    drop(monitor); // The joined monitor releases all registrations before cleanup.
}

#[test]
fn native_monitor_excludes_ignored_directory_created_after_startup() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "node_modules/\n").unwrap();
    let monitor = RunningMonitor::start(&root);
    fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        fs::write(root.join("node_modules/pkg/generated"), "churn").unwrap();
        std::thread::sleep(Duration::from_millis(10));
    }
    // Creating the boundary can produce a bounded parent-directory event.
    // Descendant activity after installing the exclusions must remain silent.
    for _ in 0..3 {
        match monitor.rx.recv_timeout(Duration::from_secs(3)) {
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Ok(Msg::RepoExternallyChanged { .. }) => {}
            other => panic!("unexpected monitor result: {other:?}"),
        }
    }
    for index in 0..100 {
        fs::write(root.join(format!("node_modules/pkg/{index}")), "churn").unwrap();
    }
    fs::write(root.join("node_modules/pkg/.gitignore"), "*").unwrap();
    monitor.quiet();
    fs::write(root.join("real.txt"), "real edit").unwrap();
    monitor.refresh();
}

fn drain_monitor(rx: &mpsc::Receiver<MonitorMsg>, quiet: Duration) -> Vec<notify::Event> {
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(
            Instant::now() < deadline,
            "watcher did not become quiet: {events:?}"
        );
        match rx.recv_timeout(quiet) {
            Ok(MonitorMsg::Event(Ok(event))) => events.push(event),
            Ok(MonitorMsg::Event(Err(error))) => panic!("native watcher error: {error}"),
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => return events,
            Err(error) => panic!("watcher disconnected: {error}"),
        }
    }
}

fn ready(root: &Path, rx: &mpsc::Receiver<MonitorMsg>) {
    // A real native round trip, then a quiet interval, rather than assuming
    // that an arbitrary startup sleep makes the watcher ready.
    let head = root.join(".git/HEAD");
    fs::write(&head, fs::read(&head).unwrap()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        assert!(Instant::now() < deadline, "no native readiness event");
        if let Ok(MonitorMsg::Event(Ok(event))) = rx.recv_timeout(Duration::from_millis(100))
            && event.paths.contains(&head)
        {
            break;
        }
    }
    drain_monitor(rx, Duration::from_millis(300));
}

#[test]
fn native_ignored_tree_flood_does_not_hide_real_edits() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("node_modules/pkg/deep")).unwrap();
    fs::create_dir_all(root.join("src")).unwrap();
    // Exceed FSEvents' native exclusion limit to exercise callback filtering too.
    let mut ignore = "node_modules/\n".to_string();
    for index in 0..9 {
        let name = format!("ignored-{index}");
        fs::create_dir_all(root.join(&name)).unwrap();
        ignore.push_str(&format!("{name}/\n"));
    }
    fs::write(root.join(".gitignore"), ignore).unwrap();
    fs::write(root.join("root.txt"), "before").unwrap();
    let mut rules = load_gitignore_rules(&root);
    let (_watcher, outcome, rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    ready(&root, &rx);
    for index in 0..100 {
        fs::write(root.join(format!("node_modules/pkg/deep/{index}")), "churn").unwrap();
    }
    fs::write(root.join("node_modules/pkg/.gitignore"), "*").unwrap();
    fs::write(root.join("root.txt"), "after").unwrap();
    fs::write(root.join("src/real.txt"), "edit").unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(events.iter().all(|event| {
        event
            .paths
            .iter()
            .all(|path| !path.starts_with(root.join("node_modules")))
    }));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("root.txt"))),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("src/real.txt"))),
        "{events:?}"
    );
}

fn lfs_payload(root: &Path) {
    run_git(root, &["lfs", "install", "--local"]);
    fs::write(
        root.join(".gitattributes"),
        "*.lfsbin filter=lfs diff=lfs merge=lfs -text\n",
    )
    .unwrap();
    fs::write(root.join("asset.lfsbin"), vec![b'x'; 1024 * 1024]).unwrap();
    run_git(root, &["add", ".gitattributes", "asset.lfsbin"]);
    run_git(root, &["commit", "-m", "LFS payload"]);
}

#[test]
fn native_real_lfs_status_does_not_requeue_refreshes() {
    // This is a required regression, not an optional test skipped without LFS.
    let (_temp, root) = repository();
    run_git(&root, &["lfs", "version"]);
    lfs_payload(&root);
    let seed = unique_temp_dir("gitcomet-lfs-seed");
    init_repo_for_ignore_tests(seed.path());
    lfs_payload(seed.path());
    let subpath = "Assets/Standard Assets/CharacterBuilder";
    run_git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            seed.path().to_str().unwrap(),
            subpath,
        ],
    );
    run_git(&root, &["commit", "-am", "LFS submodule"]);
    let child = root.join(subpath);
    run_git(&child, &["lfs", "install", "--local"]);
    run_git(&child, &["status", "--porcelain=v2"]);
    run_git(
        &root,
        &["status", "--porcelain=v2", "--ignore-submodules=none"],
    );
    let backend = gitcomet_git_gix::GixBackend;
    let repo = backend.open(&root).unwrap();
    let child_git = resolve_git_dir(&child).unwrap();
    let indexes = [
        root.join(".git/index"),
        normalized(&child_git.join("index")),
    ];
    let before: Vec<_> = indexes.iter().map(|path| fs::read(path).unwrap()).collect();
    for directory in [&root, &child] {
        let payload = directory.join("asset.lfsbin");
        let content = fs::read(&payload).unwrap();
        let file = fs::OpenOptions::new().write(true).open(&payload).unwrap();
        file.set_times(
            fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() - Duration::from_secs(120)),
        )
        .unwrap();
        assert_eq!(content, fs::read(&payload).unwrap());
    }
    let mut rules = load_gitignore_rules(&root);
    let (_watcher, outcome, rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    ready(&root, &rx);
    // Independently observe the real temporary-file traffic so a status shortcut
    // or disabled filter cannot make this regression pass vacuously.
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut observer = notify::RecommendedWatcher::new(
        move |event: notify::Result<notify::Event>| {
            if let Ok(event) = event
                && !should_ignore_event_kind(&event)
            {
                let _ = raw_tx.send(event);
            }
        },
        notify::Config::default(),
    )
    .unwrap();
    let caches = [
        root.join(".git/lfs/tmp"),
        normalized(&child_git.join("lfs/tmp")),
    ];
    for cache in &caches {
        observer
            .watch(cache, notify::RecursiveMode::NonRecursive)
            .unwrap();
    }
    for _ in 0..3 {
        let status = repo.status().unwrap();
        assert!(
            status.staged.is_empty() && status.unstaged.is_empty(),
            "{status:?}"
        );
    }
    run_git(
        &root,
        &[
            "--no-optional-locks",
            "status",
            "--porcelain=v2",
            "--ignore-submodules=none",
        ],
    );
    let events = drain_monitor(&rx, Duration::from_secs(3));
    let raw_events: Vec<_> = raw_rx.try_iter().collect();
    for cache in &caches {
        assert!(
            raw_events
                .iter()
                .any(|event| event.paths.iter().any(|path| path.starts_with(cache))),
            "LFS did not exercise {}: {raw_events:?}",
            cache.display()
        );
    }
    let changes: Vec<_> = events
        .iter()
        .filter_map(|event| classify_change(&root, Some(&root.join(".git")), &mut rules, event))
        .collect();
    assert!(
        changes.is_empty(),
        "read-only LFS status generated refreshes: {events:?}"
    );
    for (index, expected) in indexes.iter().zip(before) {
        assert_eq!(
            fs::read(index).unwrap(),
            expected,
            "index changed: {}",
            index.display()
        );
    }
    // Real modified content must still be reported, and subsequent reads settle.
    fs::write(root.join("asset.lfsbin"), vec![b'y'; 1024 * 1024]).unwrap();
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events
            .iter()
            .any(|event| event.paths.contains(&root.join("asset.lfsbin")))
    );
    assert!(!repo.status().unwrap().unstaged.is_empty());
    let events = drain_monitor(&rx, Duration::from_secs(3));
    assert!(
        events.iter().all(|event| classify_change(
            &root,
            Some(&root.join(".git")),
            &mut rules,
            event
        )
        .is_none()),
        "{events:?}"
    );
}
