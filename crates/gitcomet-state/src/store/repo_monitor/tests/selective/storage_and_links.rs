use super::*;

fn custom_lfs_status_stays_quiet(absolute: bool) {
    let (_temp, root) = repository();
    let cache = root.join(".git/custom-lfs");
    let storage = if absolute {
        cache.as_path()
    } else {
        Path::new("custom-lfs")
    };
    run_git(&root, &["config", "lfs.storage", storage.to_str().unwrap()]);
    lfs_payload(&root);
    run_git(&root, &["status", "--porcelain=v2"]);
    let repo = gitcomet_git_gix::GixBackend.open(&root).unwrap();
    let index_before = fs::read(root.join(".git/index")).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(root.join("asset.lfsbin"))
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(std::time::SystemTime::now() - Duration::from_secs(120)),
        )
        .unwrap();
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    // Independently verify that real clean-filter writes occurred, so an LFS
    // shortcut cannot make the quiet-monitor assertion pass vacuously.
    let (raw_tx, raw_rx) = mpsc::channel();
    let mut observer = notify::RecommendedWatcher::new(raw_tx, notify::Config::default()).unwrap();
    observer
        .watch(&cache.join("tmp"), notify::RecursiveMode::NonRecursive)
        .unwrap();
    for _ in 0..3 {
        let status = repo.status().unwrap();
        assert!(status.staged.is_empty() && status.unstaged.is_empty());
    }
    // One window covers all three: an earlier status's refresh would stay queued.
    monitor.quiet();
    assert!(raw_rx.try_iter().any(|event| event.is_ok_and(|event| {
        !should_ignore_event_kind(&event)
            && event
                .paths
                .iter()
                .any(|path| normalized(path).starts_with(&cache))
    })));
    assert_eq!(fs::read(root.join(".git/index")).unwrap(), index_before);
    fs::write(root.join("asset.lfsbin"), vec![b'y'; 1024 * 1024]).unwrap();
    monitor.refresh();
    assert!(!repo.status().unwrap().unstaged.is_empty());
    monitor.quiet();
}

#[test]
fn custom_lfs_absolute_storage_status_stays_quiet() {
    custom_lfs_status_stays_quiet(true);
}

#[test]
fn custom_lfs_relative_storage_status_stays_quiet() {
    custom_lfs_status_stays_quiet(false);
}

#[test]
fn custom_lfs_storage_covers_submodules_and_linked_worktrees() {
    let (_temp, root) = repository();
    let (_seed_temp, seed) = repository();
    run_git(&root, &["config", "lfs.storage", "custom-lfs"]);
    run_git(
        &root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            seed.to_str().unwrap(),
            "child",
        ],
    );
    run_git(&root, &["commit", "-am", "Submodule"]);
    let child = root.join("child");
    let child_cache = root.join(".git/child-storage");
    run_git(
        &child,
        &["config", "lfs.storage", child_cache.to_str().unwrap()],
    );
    let cache = root.join(".git/custom-lfs");
    for path in [&cache, &child_cache] {
        fs::create_dir_all(path.join("tmp/deep")).unwrap();
    }
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    for path in [&cache, &child_cache] {
        assert!(
            plan.policy.is_cache(&path.join("tmp/clean")),
            "{}",
            path.display()
        );
        assert!(
            !plan
                .dirs
                .iter()
                .any(|dir| dir.starts_with(path.join("tmp")))
        );
        assert!(
            classify_change(
                &root,
                Some(&root.join(".git")),
                &mut rules,
                &notify::Event::new(EventKind::Any).add_path(path.join("tmp/clean"))
            )
            .is_none()
        );
    }
    assert!(plan.dirs.contains(&root.join(".git/refs/heads")));
    assert!(
        !plan
            .policy
            .is_cache(&root.join(".git/refs/heads/custom-lfs/topic"))
    );

    let worktree = unique_temp_dir("gitcomet-custom-lfs-worktree");
    let checkout = normalized(&worktree.path().canonicalize().unwrap()).join("checkout");
    run_git(
        &root,
        &["worktree", "add", "--detach", checkout.to_str().unwrap()],
    );
    let mut rules = load_gitignore_rules(&checkout);
    let plan = TestPlan::build(&checkout, resolve_git_dir(&checkout).as_deref(), &mut rules);
    // Relative LFS storage belongs to the common Git directory, rather than
    // the linked checkout's private .git/worktrees/<name> directory.
    assert!(plan.policy.is_cache(&cache.join("tmp/clean")));
    assert!(
        !plan
            .dirs
            .iter()
            .any(|dir| dir.starts_with(cache.join("tmp")))
    );
}

#[test]
fn custom_lfs_storage_reload_replaces_old_exclusions() {
    let (_temp, root) = repository();
    run_git(&root, &["config", "lfs.storage", "../old-cache"]);
    fs::create_dir_all(root.join("old-cache/tmp")).unwrap();
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.is_cache(&root.join("old-cache/tmp/clean")));
    assert!(!plan.worktree_dirs.contains(&root.join("old-cache/tmp")));
    *rules.state.policy.write().unwrap() = Arc::new(plan.policy);
    run_git(&root, &["config", "lfs.storage", "../new-cache"]);
    rules.reload(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.is_cache(&root.join("new-cache/tmp/clean")));
    assert!(!plan.policy.is_cache(&root.join("old-cache/tmp/clean")));
    assert!(plan.worktree_dirs.contains(&root.join("old-cache/tmp")));
    // Empty configuration means the default directory, not the Git directory.
    run_git(&root, &["config", "lfs.storage", ""]);
    rules.reload(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.is_cache(&root.join(".git/lfs/tmp/clean")));
    assert!(!plan.policy.is_cache(&root.join(".git/index")));
    // LFS uses a literal path here; unlike core.excludesFile, it does not
    // interpret '~' as the user's home directory.
    run_git(&root, &["config", "lfs.storage", "~/cache"]);
    rules.reload(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.is_cache(&root.join(".git/~/cache/tmp/clean")));
}

#[test]
fn custom_lfs_storage_sharing_repository_directories_preserves_source_and_git_state() {
    let (_temp, root) = repository();
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("source/file.txt"), "tracked source").unwrap();
    run_git(&root, &["add", "source/file.txt"]);
    for storage in [".", "..", "../source"] {
        run_git(&root, &["config", "lfs.storage", storage]);
        let mut rules = load_gitignore_rules(&root);
        let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
        let cache = normalized(&root.join(".git").join(storage));
        assert!(plan.policy.is_cache(&cache.join("tmp/clean")));
        for path in [".git/HEAD", ".git/index", "source/file.txt"] {
            assert!(
                plan.policy.relevant(&root.join(path)),
                "lfs.storage={storage} hid {path}"
            );
        }
        assert!(plan.worktree_dirs.contains(&root.join("source")));
    }
}

fn link_file(target: &Path, link: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_file(target, link).unwrap();
}

/// External inputs sit outside every native root, so Revalidate alone reloads
/// them, synchronously; a drain replaces the quiet window. Only a policy
/// reload sets `verification_context`, so late worktree residue cannot pass.
fn expect_policy_reload(monitor: &RunningMonitor) {
    monitor.revalidate();
    monitor.drain_delivered();
    let messages: Vec<_> = monitor.rx.try_iter().collect();
    assert!(
        messages
            .iter()
            .all(|message| matches!(message, Msg::RepoExternallyChanged { .. }))
            && messages.iter().any(|message| matches!(
                message,
                Msg::RepoExternallyChanged { change, .. } if change.verification_context
            )),
        "revalidation did not reload the policy: {messages:?}"
    );
}

#[test]
fn symlinked_ignore_input_observes_target_edits_and_link_replacement() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-ignore-link");
    let targets = unique_temp_dir("gitcomet-ignore-targets");
    let target_dir = normalized(&targets.path().canonicalize().unwrap());
    let target = target_dir.join("ignore");
    let replacement = target_dir.join("replacement");
    let link = normalized(&external.path().canonicalize().unwrap()).join("ignore-link");
    fs::write(&target, "generated/\n").unwrap();
    fs::write(&replacement, "generated/\n").unwrap();
    link_file(&target, &link);
    run_git(
        &root,
        &["config", "core.excludesFile", link.to_str().unwrap()],
    );
    fs::create_dir_all(root.join("generated/source")).unwrap();
    // Only awaited edits touch `file`; the quiet check edits its own file.
    let file = root.join("generated/source/file.txt");
    let ignored = root.join("generated/source/ignored.txt");
    fs::write(&file, "before").unwrap();
    fs::write(&ignored, "before").unwrap();
    let mut rules = load_gitignore_rules(&root);
    assert!(rules.is_ignored_rel(Path::new("generated"), Some(true)));
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    fs::write(&target, "").unwrap();
    expect_policy_reload(&monitor);
    assert!(
        monitor
            .expect_change(&file, || {
                fs::write(&file, "newly eligible").unwrap();
                monitor.revalidate();
            })
            .worktree
    );
    fs::remove_file(&link).unwrap();
    link_file(&replacement, &link);
    expect_policy_reload(&monitor);
    monitor.settle();
    fs::write(&ignored, "ignored again").unwrap();
    monitor.quiet();
    // Atomic replacement at the new target must reload the same policy too.
    let save = target_dir.join("atomic-save");
    fs::write(&save, "").unwrap();
    fs::rename(save, replacement).unwrap();
    expect_policy_reload(&monitor);
    assert!(
        monitor
            .expect_change(&file, || {
                fs::write(&file, "eligible after target save").unwrap();
                monitor.revalidate();
            })
            .worktree
    );
}

#[test]
fn symlinked_ignore_input_observes_missing_target_and_intermediate_link() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-ignore-link-chain");
    let middle = unique_temp_dir("gitcomet-ignore-link-middle");
    let targets = unique_temp_dir("gitcomet-ignore-link-target");
    let target_dir = normalized(&targets.path().canonicalize().unwrap());
    let missing = target_dir.join("missing");
    let replacement = target_dir.join("replacement");
    fs::write(&replacement, "").unwrap();
    let intermediate = normalized(&middle.path().canonicalize().unwrap()).join("intermediate");
    let link = normalized(&external.path().canonicalize().unwrap()).join("ignore-link");
    link_file(&missing, &intermediate);
    link_file(&intermediate, &link);
    run_git(
        &root,
        &["config", "core.excludesFile", link.to_str().unwrap()],
    );
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    fs::write(&missing, "generated/\n").unwrap();
    expect_policy_reload(&monitor);
    fs::remove_file(&intermediate).unwrap();
    link_file(&replacement, &intermediate);
    expect_policy_reload(&monitor);
    fs::write(&replacement, "generated/\n").unwrap();
    expect_policy_reload(&monitor);
}

#[test]
fn symlinked_ignore_directory_and_relative_target_keep_healthy_coverage() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-ignore-directory-link");
    let targets = unique_temp_dir("gitcomet-ignore-directory-target");
    let target_dir = normalized(&targets.path().canonicalize().unwrap());
    let link = normalized(&external.path().canonicalize().unwrap()).join("linked-directory");
    fs::write(target_dir.join("policy"), "generated/\n").unwrap();
    link_file(Path::new("policy"), &target_dir.join("ignore"));
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target_dir, &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target_dir, &link).unwrap();
    run_git(
        &root,
        &[
            "config",
            "core.excludesFile",
            link.join("ignore").to_str().unwrap(),
        ],
    );
    let monitor = RunningMonitor::start(&root);
    // A no-follow inotify registration of linked-directory would warn here.
    monitor.quiet();
    fs::write(target_dir.join("policy"), "").unwrap();
    monitor.revalidate();
    monitor.refresh();
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.control_files.contains(&link));
    assert!(
        plan.policy
            .control_files
            .contains(&target_dir.join("ignore"))
    );
    assert!(
        plan.policy
            .control_files
            .contains(&target_dir.join("policy"))
    );
}
