//! Windows already owns recursive native coverage before planner discovery.
use super::super::super::plan::DiscoveryWork;
use super::*;

fn work_since(before: DiscoveryWork, after: DiscoveryWork, operation: &str) -> DiscoveryWork {
    let delta = DiscoveryWork {
        metadata_visits: after.metadata_visits - before.metadata_visits,
        metadata_reads: after.metadata_reads - before.metadata_reads,
        worktree_visits: after.worktree_visits - before.worktree_visits,
        worktree_reads: after.worktree_reads - before.worktree_reads,
    };
    println!("WINDOWS_METADATA_WORK operation={operation} {delta:?}");
    delta
}

fn setup_fixture() -> (tempfile::TempDir, PathBuf) {
    let (temp, root) = repository();
    for directory in [
        "src/nested",
        "generated/pkg",
        ".git/refs/heads/deep/old",
        ".git/logs/refs/heads/deep/old",
    ] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    fs::write(root.join(".gitignore"), "generated/\n").unwrap();
    fs::write(root.join("src/nested/.gitignore"), "*.generated\n").unwrap();
    (temp, root)
}

#[test]
fn windows_metadata_setup_keeps_ignore_discovery_without_metadata_bfs() {
    let (_temp, root) = setup_fixture();
    let mut rules = load_gitignore_rules(&root);
    let before = rules.state.plan.discovery_work;
    let (watcher, outcome, _rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    assert_eq!(watcher.watched, FxHashSet::from_iter([root.clone()]));
    assert!(
        rules
            .state
            .inputs
            .inputs
            .contains(&root.join("src/nested/.gitignore"))
    );
    assert!(
        rules
            .state
            .snapshot()
            .excluded_roots
            .contains(&root.join("generated"))
    );
    assert!(
        rules
            .state
            .plan
            .worktree_dirs
            .contains(&root.join("src/nested"))
    );
    assert!(
        !rules
            .state
            .plan
            .worktree_dirs
            .contains(&root.join("generated/pkg"))
    );
    let work = work_since(before, rules.state.plan.discovery_work, "setup");
    assert!(work.worktree_reads >= 3 && work.worktree_visits >= 3);
    assert_eq!(
        work.metadata_reads, 0,
        "recursive roots already cover metadata descendants"
    );
    assert_eq!(work.metadata_visits, 0);
}

#[test]
fn windows_metadata_incremental_keeps_new_ignore_inputs_without_metadata_bfs() {
    let (_temp, root) = setup_fixture();
    let mut rules = load_gitignore_rules(&root);
    let (mut watcher, outcome, _rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
    let before = rules.state.plan.discovery_work;
    let metadata = root.join(".git/refs/tags/created");
    let source = root.join("source-added");
    for directory in [&metadata, &source] {
        fs::create_dir_all(directory.join("nested")).unwrap();
    }
    fs::write(source.join("nested/.gitignore"), "*.generated\n").unwrap();
    let event = notify::Event::new(EventKind::Create(CreateKind::Folder))
        .add_path(metadata.clone())
        .add_path(source.clone());
    let effect = summarize(&rules.state.snapshot(), &mut rules.state.rules, &event);
    assert!(
        effect
            .change
            .as_ref()
            .is_some_and(|change| change.git_state && change.tags && change.worktree)
    );
    assert!(effect.dir_added.contains(&(metadata, false)));
    assert!(effect.dir_added.contains(&(source.clone(), true)));
    rules
        .state
        .apply_directories(&effect, &mut watcher, &rules.config);
    assert!(
        rules
            .state
            .inputs
            .inputs
            .contains(&source.join("nested/.gitignore"))
    );
    assert!(
        rules
            .state
            .plan
            .worktree_dirs
            .contains(&source.join("nested"))
    );
    assert_eq!(watcher.watched, FxHashSet::from_iter([root]));
    let work = work_since(before, rules.state.plan.discovery_work, "incremental");
    assert_eq!(work.worktree_reads, 2);
    assert_eq!(work.worktree_visits, 2);
    assert_eq!(
        work.metadata_reads, 0,
        "new metadata subtrees remain recursively watched"
    );
    assert_eq!(work.metadata_visits, 0);
}

#[test]
fn windows_metadata_overlapping_roots_preserve_legacy_discovery() {
    for equal in [false, true] {
        let temp = unique_temp_dir("gitcomet-overlap-metadata");
        let base = normalized(&temp.path().canonicalize().unwrap());
        let root = base.join("project");
        init_repo_for_ignore_tests(&root);
        fs::create_dir_all(root.join("src/nested")).unwrap();
        fs::write(root.join("src/nested/.gitignore"), "*.generated\n").unwrap();
        let overlap = if equal { root.clone() } else { base };
        let mut rules = load_gitignore_rules(&root);
        rules.state.inputs.info.git_dirs.push(overlap.clone());
        // This generic planner deliberately models the pre-optimization ordering.
        let expected = TestPlan::build(&root, None, &mut rules);
        let (mut watcher, outcome, _rx) = rules.start_watcher(&root);
        assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 0 });
        assert_eq!(rules.state.plan.dirs, expected.dirs);
        assert_eq!(rules.state.plan.worktree_dirs, expected.worktree_dirs);
        assert_eq!(
            rules.state.snapshot().excluded_roots,
            expected.policy.excluded_roots
        );
        assert_eq!(watcher.watched, FxHashSet::from_iter([overlap.clone()]));
        let setup = work_since(
            DiscoveryWork::default(),
            rules.state.plan.discovery_work,
            "overlap-setup",
        );
        assert!(setup.metadata_reads > 0 && setup.metadata_visits > 0);
        assert_eq!(
            setup.worktree_reads, 0,
            "retain the existing cross-pass deduplication"
        );
        let added = overlap.join("new-metadata/nested");
        fs::create_dir_all(&added).unwrap();
        let before = rules.state.plan.discovery_work;
        let event =
            notify::Event::new(EventKind::Create(CreateKind::Folder)).add_path(added.clone());
        let effect = summarize(&rules.state.snapshot(), &mut rules.state.rules, &event);
        assert!(effect.dir_added.contains(&(added, false)));
        rules
            .state
            .apply_directories(&effect, &mut watcher, &rules.config);
        let incremental = work_since(
            before,
            rules.state.plan.discovery_work,
            "overlap-incremental",
        );
        assert_eq!(incremental.metadata_reads, 1);
        assert_eq!(incremental.metadata_visits, 1);
    }
}

#[test]
fn windows_metadata_skipping_does_not_hide_registration_or_discovery_failures() {
    let (_temp, root) = repository();
    let outside = unique_temp_dir("gitcomet-missing-metadata-root");
    let missing = normalized(outside.path()).join("missing/git");
    let mut rules = load_gitignore_rules(&root);
    rules.state.inputs.info.git_dirs.push(missing.clone());
    let (watcher, outcome, _rx) = rules.start_watcher(&root);
    assert!(matches!(outcome, WatchSetupOutcome::Watching { failed_dirs } if failed_dirs > 0));
    assert!(watch_degraded_reason(outcome).is_some());
    assert!(!watcher.watched.contains(&missing));
    drop(watcher);

    let mut rules = load_gitignore_rules(&root);
    rules.state.inputs.info.discovery_incomplete = true;
    let (_watcher, outcome, _rx) = rules.start_watcher(&root);
    assert_eq!(outcome, WatchSetupOutcome::Watching { failed_dirs: 1 });
    assert!(watch_degraded_reason(outcome).is_some());
}

#[test]
fn windows_metadata_native_deep_and_new_subtrees_keep_sustained_coverage() {
    let (_temp, root) = setup_fixture();
    let head = fs::read_to_string(root.join(".git/HEAD")).unwrap();
    let oid = fs::read(
        root.join(".git")
            .join(head.trim().strip_prefix("ref: ").unwrap()),
    )
    .unwrap();
    let existing_ref = root.join(".git/refs/heads/deep/old/branch");
    let existing_log = root.join(".git/logs/refs/heads/deep/old/branch");
    fs::write(&existing_ref, &oid).unwrap();
    fs::write(&existing_log, "initial log\n").unwrap();
    let monitor = RunningMonitor::start(&root);
    let generation = monitor.drain_delivered().generation;
    assert!(
        monitor
            .expect_change(&existing_ref, || fs::write(&existing_ref, &oid).unwrap())
            .git_state
    );
    assert!(
        monitor
            .expect_change(&existing_log, || fs::write(&existing_log, "edited log\n")
                .unwrap())
            .git_state
    );

    let created = root.join(".git/refs/tags/new-tree");
    assert!(
        monitor
            .expect_change(&created, || {
                fs::create_dir_all(created.join("nested/deep")).unwrap();
                fs::write(created.join("nested/deep/tag"), &oid).unwrap();
            })
            .tags
    );
    settle_native(&monitor);
    let tag = created.join("nested/deep/tag");
    assert!(
        monitor
            .expect_change(&tag, || fs::write(&tag, &oid).unwrap())
            .tags
    );
    settle_native(&monitor);
    let moved = root.join(".git/refs/tags/moved-tree");
    assert!(
        monitor
            .expect_change(&moved, || fs::rename(&created, &moved).unwrap())
            .tags
    );
    settle_native(&monitor);
    let moved_tag = moved.join("nested/deep/tag");
    assert!(
        monitor
            .expect_change(&moved_tag, || fs::write(&moved_tag, &oid).unwrap())
            .tags
    );
    settle_native(&monitor);
    assert!(
        monitor
            .expect_change(&moved, || fs::remove_dir_all(&moved).unwrap())
            .tags
    );

    let late_log = root.join(".git/logs/refs/heads/new-log/nested");
    assert!(
        monitor
            .expect_change(&late_log, || {
                fs::create_dir_all(&late_log).unwrap();
                fs::write(late_log.join("branch"), "initial\n").unwrap();
            })
            .git_state
    );
    settle_native(&monitor);
    let branch_log = late_log.join("branch");
    assert!(
        monitor
            .expect_change(&branch_log, || fs::write(&branch_log, "later edit\n")
                .unwrap())
            .git_state
    );
    assert_eq!(
        monitor.drain_delivered().generation,
        generation,
        "ordinary metadata subtrees must not need a replacement watcher"
    );
}

fn settle_native(monitor: &RunningMonitor) {
    monitor.settle();
    // Before a later in-place write, drain the actual Windows queue rather than
    // allowing an older create/rename event for that same path to satisfy it.
    monitor
        .checkpoint_native(Instant::now() + SYNC_TIMEOUT)
        .unwrap();
    monitor.drain_delivered();
}
