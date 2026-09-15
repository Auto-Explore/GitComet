use super::*;

#[cfg(windows)]
#[test]
fn native_windows_lfs_storage_casing_does_not_admit_cache_traffic() {
    let (_temp, root) = repository();
    let actual = root.join(".git/CustomLfsStorage");
    fs::create_dir_all(actual.join("tmp")).unwrap();
    run_git(&root, &["config", "lfs.storage", "customlfsstorage"]);
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    let (tx, rx) = mpsc::channel();
    let mut observer = notify::RecommendedWatcher::new(tx, notify::Config::default()).unwrap();
    observer
        .watch(&actual.join("tmp"), notify::RecursiveMode::NonRecursive)
        .unwrap();
    let file = actual.join("tmp/clean-filter");
    fs::write(&file, "temporary filter output").unwrap();
    fs::remove_file(&file).unwrap();
    assert!(
        rx.recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap()
            .paths
            .iter()
            .any(|path| { normalized(path).starts_with(&actual) }),
        "probe did not observe the native directory spelling"
    );
    monitor.quiet();
}

#[cfg(windows)]
fn windows_ignore_path_casing(missing: bool) {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-ignore-case");
    let parent = normalized(&external.path().canonicalize().unwrap()).join("ConfigDirectory");
    fs::create_dir(&parent).unwrap();
    let file = parent.join(if missing { "ignore" } else { "IgnoreRules" });
    if !missing {
        fs::write(&file, "generated/\n").unwrap();
    }
    let configured = file.to_str().unwrap().to_lowercase();
    run_git(&root, &["config", "core.excludesFile", &configured]);
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    fs::write(&file, "").unwrap();
    monitor.revalidate();
    monitor.refresh();
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.control_files.contains(&file));
    assert!(
        plan.policy
            .control_files
            .contains(&PathBuf::from(configured))
    );
}

#[cfg(windows)]
#[test]
fn native_windows_existing_ignore_file_uses_filesystem_casing() {
    windows_ignore_path_casing(false);
}

#[cfg(windows)]
#[test]
fn native_windows_missing_ignore_file_uses_existing_parent_casing() {
    windows_ignore_path_casing(true);
}

fn lfs_parent_after_symlink_keeps_source_coverage(absolute: bool) {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("other/child")).unwrap();
    fs::create_dir_all(root.join("other/Storage/tmp")).unwrap();
    fs::create_dir_all(root.join("Storage/tmp")).unwrap();
    let source = root.join("other/Storage/tmp/tracked.txt");
    fs::write(&source, "tracked source").unwrap();
    run_git(&root, &["add", "other/Storage/tmp/tracked.txt"]);
    let link = root.join("link");
    #[cfg(unix)]
    std::os::unix::fs::symlink(root.join("other/child"), &link).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(root.join("other/child"), &link).unwrap();
    let storage = if absolute {
        link.join("../Storage")
    } else {
        PathBuf::from("../link/../Storage")
    };
    run_git(&root, &["config", "lfs.storage", storage.to_str().unwrap()]);
    // Git LFS cleans the path before traversing links. Its actual cache is
    // Storage/tmp; other/Storage/tmp is an unrelated tracked source directory.
    let env = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["lfs", "env"])
        .output()
        .unwrap();
    assert!(
        env.status.success(),
        "{}",
        String::from_utf8_lossy(&env.stderr)
    );
    let output = String::from_utf8(env.stdout).unwrap();
    let lfs_tmp = output
        .lines()
        .find_map(|line| line.strip_prefix("TempDir="))
        .unwrap();
    assert_eq!(normalized(Path::new(lfs_tmp)), root.join("Storage/tmp"));
    let monitor = RunningMonitor::start(&root);
    monitor.quiet();
    fs::write(&source, "an edit in an unrelated source directory").unwrap();
    monitor.refresh();
    let info = gitcomet_git_gix::GixBackend
        .repository_watch_info(&root)
        .unwrap()
        .unwrap();
    assert!(info.cache_dirs.contains(&root.join("Storage/tmp")));
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&root.join(".git")), &mut rules);
    assert!(plan.policy.is_cache(&root.join("Storage/tmp/clean-filter")));
    assert!(!plan.policy.is_cache(&source));
    assert!(plan.worktree_dirs.contains(&root.join("other/Storage/tmp")));
}

#[test]
fn native_lfs_absolute_parent_after_symlink_keeps_tracked_source_visible() {
    lfs_parent_after_symlink_keeps_source_coverage(true);
}

#[test]
fn native_lfs_relative_parent_after_symlink_keeps_tracked_source_visible() {
    lfs_parent_after_symlink_keeps_source_coverage(false);
}
