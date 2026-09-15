use super::*;

fn embedded_gitlink(root: &Path, name: &str) -> PathBuf {
    let child = root.join(name);
    init_repo_for_ignore_tests(&child);
    run_git(root, &["add", name]);
    run_git(root, &["commit", "-m", "Add embedded gitlink"]);
    assert!(!root.join(".gitmodules").exists());
    child
}

#[test]
fn embedded_gitlink_commit_refreshes_parent() {
    let (_temp, root) = repository();
    let child = embedded_gitlink(&root, "child");
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    run_git(&child, &["commit", "--allow-empty", "-m", "Advance child"]);
    // Read status without refreshing either index: the probe itself must not
    // generate the parent notification this test is looking for.
    let status = std::process::Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(&root)
        .args(["status", "--porcelain", "--ignore-submodules=none"])
        .output()
        .unwrap();
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains(" M child"));
    monitor.refresh();
}

#[test]
fn embedded_gitlinks_have_child_ignore_and_cache_policies() {
    let (_temp, root) = repository();
    let child = embedded_gitlink(&root, "child");
    let grandchild = embedded_gitlink(&child, "nested");
    fs::write(grandchild.join(".gitignore"), "node_modules/\n").unwrap();
    fs::create_dir_all(grandchild.join("node_modules/pkg")).unwrap();
    let mut rules = load_gitignore_rules(&root);
    assert!(!rules.failed);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    for checkout in [&child, &grandchild] {
        assert!(plan.policy.git_roots.contains(&checkout.join(".git")));
        assert!(rules.state.inputs.info.worktrees.contains(checkout));
        assert!(plan.policy.is_cache(&checkout.join(".git/lfs/tmp/clean")));
        assert!(
            plan.policy
                .is_cache(&checkout.join(".git/objects/ab/object"))
        );
        assert!(plan.dirs.contains(&checkout.join(".git")));
    }
    assert!(
        !plan
            .worktree_dirs
            .contains(&grandchild.join("node_modules"))
    );
}

#[test]
fn ignore_lookup_error_is_never_cached() {
    let (_temp, root) = repository();
    let unavailable = Arc::new(AtomicBool::new(true));
    let mut rules = TestRules::load(
        &root,
        Arc::new(FaultyBackend {
            lookup_failure: unavailable.clone(),
            ..Default::default()
        }),
    );
    let relative = Path::new("source/file.txt");
    assert!(rules.is_ignored_rel(relative, Some(false)));
    assert!(rules.failed);
    assert!(
        rules
            .cache_get_at(relative, Some(false), Instant::now())
            .is_none()
    );
    unavailable.store(false, Ordering::Relaxed);
    assert!(!rules.is_ignored_rel(relative, Some(false)));
}

#[test]
fn runtime_ignore_failure_warns_and_recovers_without_another_edit() {
    let (_temp, root) = repository();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("source/file.txt"), "before").unwrap();
    let unavailable = Arc::new(AtomicBool::new(false));
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(FaultyBackend {
            lookup_failure: unavailable.clone(),
            ..Default::default()
        }),
        MonitorConfig {
            idle_tick: Duration::from_millis(100),
            recovery_interval: Duration::from_millis(100),
            ..Default::default()
        },
    );
    monitor.settle();
    monitor.quiet();
    unavailable.store(true, Ordering::Relaxed);
    fs::write(root.join("source/file.txt"), "edit during lookup failure").unwrap();
    assert!(matches!(
        monitor.rx.recv_timeout(Duration::from_secs(10)),
        Ok(Msg::RepoWatchDegraded {
            reason: RepoWatchDegradedReason::IgnorePolicyFailed,
            ..
        })
    ));
    unavailable.store(false, Ordering::Relaxed);
    match monitor.rx.recv_timeout(Duration::from_secs(10)) {
        Ok(Msg::RepoExternallyChanged { change, .. }) => {
            assert_eq!(change, RepoExternalChange::all())
        }
        other => panic!("runtime failure did not reconcile missed changes: {other:?}"),
    }
    monitor.settle();
    fs::write(root.join("source/file.txt"), "edit after recovery").unwrap();
    monitor.refresh();
}
