use super::*;

/// A symlink is a blob to Git, so a directory-only rule never hides it, whatever
/// kind the backend reported. Windows types a created entry by following the
/// link, so a moved-in directory symlink arrives as a created folder; a delayed
/// removal can find a link already at the path; edits carry no kind at all.
#[test]
fn directory_symlink_is_not_hidden_by_directory_only_ignore_for_any_event_kind() {
    let cases = [
        (
            EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::To)),
            None,
        ),
        (
            EventKind::Create(notify::event::CreateKind::Folder),
            Some(true),
        ),
        (
            EventKind::Remove(notify::event::RemoveKind::Folder),
            Some(true),
        ),
        (
            EventKind::Modify(ModifyKind::Data(notify::event::DataChange::Any)),
            None,
        ),
    ];
    for (kind, expected_hint) in cases {
        let (_temp, root) = repository();
        let target = unique_temp_dir("gitcomet-directory-symlink-target");
        fs::write(root.join(".gitignore"), "build/\n").unwrap();
        let link = root.join("build");
        #[cfg(unix)]
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target.path(), &link).unwrap();
        let mut rules = load_gitignore_rules(&root);
        let event = notify::Event::new(kind).add_path(link);
        assert_eq!(path_dir_hint(&event), expected_hint, "{kind:?}");
        let effect = summarize_event(&root, Some(&root.join(".git")), &mut rules, &event);
        assert!(
            effect.change.is_some_and(|change| change.worktree),
            "a symlink to a directory is visible to Git after {kind:?}: {effect:?}"
        );
        assert!(effect.new_ignored_dirs.is_empty(), "{kind:?}");
        assert!(effect.dir_added.is_empty(), "{kind:?}");
    }
}

/// Git never ignores a path with tracked content beneath it, so replacing a
/// tracked directory with a symlink must still refresh: its files are now
/// deleted. The bare `vendor` rule would otherwise match the link.
#[test]
fn symlink_over_tracked_directory_is_never_ignored() {
    let cases = [
        EventKind::Create(notify::event::CreateKind::Folder),
        EventKind::Create(notify::event::CreateKind::Any),
        EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::To)),
    ];
    for kind in cases {
        let (_temp, root) = repository();
        fs::create_dir(root.join("vendor")).unwrap();
        fs::write(root.join("vendor/lib.rs"), "tracked").unwrap();
        run_git(&root, &["add", "vendor/lib.rs"]);
        fs::write(root.join(".gitignore"), "vendor\n").unwrap();
        let target = unique_temp_dir("gitcomet-tracked-directory-symlink-target");
        fs::remove_dir_all(root.join("vendor")).unwrap();
        let link = root.join("vendor");
        #[cfg(unix)]
        std::os::unix::fs::symlink(target.path(), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(target.path(), &link).unwrap();
        let mut rules = load_gitignore_rules(&root);
        let event = notify::Event::new(kind).add_path(link);
        let effect = summarize_event(&root, Some(&root.join(".git")), &mut rules, &event);
        assert!(
            effect.change.is_some_and(|change| change.worktree),
            "tracked files vanished behind the link after {kind:?}: {effect:?}"
        );
    }
}

#[test]
fn moving_directory_symlink_into_worktree_refreshes_status() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-incoming-directory-symlink");
    let target = external.path().join("target");
    let incoming = external.path().join("incoming");
    fs::create_dir(&target).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &incoming).unwrap();
    #[cfg(windows)]
    std::os::windows::fs::symlink_dir(&target, &incoming).unwrap();
    fs::write(root.join(".gitignore"), "build/\n").unwrap();
    let monitor = RunningMonitor::start(&root);
    // Only the destination is watched, so an event for a visible source path
    // cannot mask a dropped rename notification for the symlink itself.
    fs::rename(&incoming, root.join("build")).unwrap();
    let status = Command::new("git")
        .arg("--no-optional-locks")
        .arg("-C")
        .arg(&root)
        .args(["status", "--porcelain", "--", "build"])
        .output()
        .unwrap();
    assert!(status.status.success());
    assert_eq!(String::from_utf8_lossy(&status.stdout).trim(), "?? build");
    monitor.refresh();
}

#[test]
fn ignored_parent_of_separate_git_dir_keeps_metadata_visible() {
    let (_temp, root) = repository();
    fs::create_dir(root.join("private")).unwrap();
    run_git(&root, &["init", "--separate-git-dir", "private/gitdir"]);
    fs::write(root.join(".gitignore"), "private/\n").unwrap();
    let git_dir = normalized(&resolve_git_dir(&root).unwrap());
    assert_eq!(git_dir, root.join("private/gitdir"));
    let mut rules = load_gitignore_rules(&root);
    let plan = TestPlan::build(&root, Some(&git_dir), &mut rules);
    assert!(plan.policy.excluded_roots.contains(&root.join("private")));
    // Exercise the macOS stream selection on every platform: a recursive
    // worktree stream subsumes this nested Git root, so it must not exclude it.
    let roots = native_watcher::minimal_roots(&plan.policy);
    assert_eq!(roots, vec![root.clone()]);
    let boundaries: Vec<_> = plan.policy.excluded_roots.iter().cloned().collect();
    let exclusions = plan::native_exclusions(&root, &plan.policy, &boundaries);
    assert!(
        exclusions.iter().all(|path| !git_dir.starts_with(path)),
        "native exclusions hide the only Git metadata stream: {exclusions:?}"
    );
    assert!(exclusions.contains(&git_dir.join("objects")));
    let monitor = RunningMonitor::start(&root);
    run_git(&root, &["commit", "--allow-empty", "-m", "External commit"]);
    monitor.refresh();
    run_git(&root, &["checkout", "-b", "other"]);
    monitor.refresh();
}

#[test]
fn inactive_unresolvable_include_keeps_source_coverage() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-conditional-include");
    let include = normalized(&external.path().canonicalize().unwrap()).join("future-config");
    run_git(
        &root,
        &[
            "config",
            "includeIf.gitdir:/gitcomet-never-matches/.path",
            "~gitcomet-nonexistent-config-user/.gitconfig",
        ],
    );
    run_git(
        &root,
        &[
            "config",
            "includeIf.gitdir:**.path",
            include.to_str().unwrap(),
        ],
    );
    fs::create_dir(root.join("source")).unwrap();
    run_git(&root, &["status", "--porcelain"]);
    assert!(
        gitcomet_git_gix::GixBackend
            .worktree_ignore_matcher(&root)
            .unwrap()
            .is_some()
    );
    let info = gitcomet_git_gix::GixBackend
        .repository_watch_info(&root)
        .unwrap()
        .unwrap();
    assert!(!info.discovery_incomplete);
    assert!(info.ignore_inputs.contains(&root.join(".git/config")));
    assert!(info.ignore_inputs.contains(&include));
    let monitor = RunningMonitor::start(&root);
    fs::write(root.join("source/file.txt"), "observed").unwrap();
    monitor.refresh();
    fs::write(&include, "[core]\n    ignoreCase = true\n").unwrap();
    monitor.revalidate();
    monitor.refresh();
}

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
