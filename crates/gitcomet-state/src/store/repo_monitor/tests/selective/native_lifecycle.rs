//! Native counterparts to VS Code's lifecycle and metadata-noise regressions.
//! These assert repository refreshes, not VS Code's per-file event API.
use super::*;

fn assert_native_quiet(monitor: &RunningMonitor) {
    let before = monitor.native_events.load(Ordering::Relaxed);
    monitor.quiet();
    assert_eq!(
        monitor.native_events.load(Ordering::Relaxed),
        before,
        "a stale native registration kept delivering callbacks while idle"
    );
}

#[test]
fn repeated_directory_recreation_stops_stale_callbacks_and_keeps_crud_visible() {
    let (_temp, root) = repository();
    let directory = root.join("recreated/nested");
    fs::create_dir_all(&directory).unwrap();
    let monitor = RunningMonitor::start(&root);
    let recreated = root.join("recreated");
    for cycle in 0..3 {
        // The edit and delete targets exist before the settle, so each awaited
        // path below is touched once afterwards and needs no quiet window
        // (3 s apiece on Windows/macOS). Between cycles only nested files
        // change, so no late event names `recreated` itself.
        let edited = directory.join(format!("edited-{cycle}.txt"));
        let removed = directory.join(format!("removed-{cycle}.txt"));
        assert!(
            monitor
                .expect_change(&recreated, || {
                    fs::remove_dir_all(&recreated).unwrap();
                    fs::create_dir_all(&directory).unwrap();
                    fs::write(&edited, "created").unwrap();
                    fs::write(&removed, "created").unwrap();
                })
                .worktree
        );
        monitor.settle();
        assert_native_quiet(&monitor);
        let created = directory.join(format!("created-{cycle}.txt"));
        assert!(
            monitor
                .expect_change(&created, || fs::write(&created, "created").unwrap())
                .worktree
        );
        assert!(
            monitor
                .expect_change(&edited, || fs::write(&edited, "modified in place").unwrap())
                .worktree
        );
        let renamed = directory.join(format!("renamed-{cycle}.txt"));
        assert!(
            monitor
                .expect_change(&renamed, || fs::rename(&created, &renamed).unwrap())
                .worktree
        );
        assert!(
            monitor
                .expect_change(&removed, || fs::remove_file(&removed).unwrap())
                .worktree
        );
    }
}

#[test]
fn replacing_repository_root_detaches_old_tree_and_watches_replacement() {
    let temp = unique_temp_dir("gitcomet-replaced-repository-root");
    let root = normalized(&temp.path().canonicalize().unwrap()).join("repository");
    init_repo_for_ignore_tests(&root);
    fs::create_dir_all(root.join("source/nested")).unwrap();
    // Prepare valid replacements before monitoring, so this probes replacement
    // of the native root rather than opening an unfinished Git repository.
    let replacements: Vec<_> = (0..3)
        .map(|cycle| temp.path().join(format!("replacement-{cycle}")))
        .collect();
    for replacement in &replacements {
        init_repo_for_ignore_tests(replacement);
        fs::create_dir_all(replacement.join("source/nested")).unwrap();
        fs::write(replacement.join("source/nested/file.txt"), "before").unwrap();
    }
    // Windows does not report moving the watched root itself. Revalidation on
    // the existing idle tick must notice its changed identity and reattach.
    let monitor = RunningMonitor::start_custom(
        &root,
        Arc::new(gitcomet_git_gix::GixBackend),
        MonitorConfig {
            idle_tick: Duration::from_millis(100),
            ..Default::default()
        },
    );
    monitor.settle();
    for (cycle, replacement) in replacements.iter().enumerate() {
        let retired = temp.path().join(format!("retired-{cycle}"));
        fs::rename(&root, &retired).unwrap();
        if cycle == 1 {
            // Force one revalidation inside the non-atomic rename gap. The
            // missing root legitimately degrades coverage; its replacement
            // must recover on input revalidation, before the 120-second retry.
            monitor.revalidate();
            assert!(matches!(
                monitor.rx.recv_timeout(Duration::from_secs(10)),
                Ok(Msg::RepoWatchDegraded { .. })
            ));
        }
        fs::rename(replacement, &root).unwrap();
        // Even back-to-back renames can straddle an idle tick on a busy runner.
        // Allow the transient warning, then require a refresh and settled,
        // working coverage of the replacement below.
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match monitor
                .rx
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            {
                Ok(Msg::RepoWatchDegraded {
                    reason:
                        RepoWatchDegradedReason::IgnorePolicyFailed
                        | RepoWatchDegradedReason::WatchLimitReached { .. },
                    ..
                }) => {}
                Ok(Msg::RepoExternallyChanged { .. }) => break,
                other => panic!("root replacement did not refresh: {other:?}"),
            }
        }
        monitor.settle();
        let before = monitor.native_events.load(Ordering::Relaxed);
        fs::write(retired.join("source/nested/stale.txt"), "old tree").unwrap();
        monitor.quiet();
        assert_eq!(
            monitor.native_events.load(Ordering::Relaxed),
            before,
            "old native root remained attached"
        );
        fs::write(root.join("source/nested/file.txt"), "new tree edit").unwrap();
        monitor.refresh();
    }
}

#[test]
fn repeated_atomic_file_replacement_keeps_later_in_place_edits_visible() {
    let (_temp, root) = repository();
    fs::create_dir(root.join("source")).unwrap();
    let file = root.join("source/file.txt");
    fs::write(&file, "initial").unwrap();
    let monitor = RunningMonitor::start(&root);
    for cycle in 0..3 {
        let replacement = root.join("source/replacement.tmp");
        fs::write(&replacement, format!("replacement {cycle}")).unwrap();
        fs::rename(&replacement, &file).unwrap();
        monitor.refresh();
        fs::write(&file, format!("in-place edit {cycle}")).unwrap();
        monitor.refresh();
    }
    assert_native_quiet(&monitor);
}

#[test]
fn case_only_directory_rename_keeps_nested_edits_visible() {
    let (_temp, root) = repository();
    fs::create_dir_all(root.join("source/nested")).unwrap();
    fs::write(root.join("source/nested/file.txt"), "before").unwrap();
    let monitor = RunningMonitor::start(&root);
    fs::rename(root.join("source"), root.join("SOURCE")).unwrap();
    monitor.refresh();
    fs::write(root.join("SOURCE/nested/file.txt"), "after").unwrap();
    monitor.refresh();
    assert_native_quiet(&monitor);
}

#[test]
fn moving_a_tree_across_an_ignore_boundary_updates_live_coverage() {
    let (_temp, root) = repository();
    fs::write(root.join(".gitignore"), "ignored/\n").unwrap();
    fs::create_dir_all(root.join("ignored/package/nested")).unwrap();
    fs::write(root.join("ignored/package/nested/file.txt"), "before").unwrap();
    let monitor = RunningMonitor::start(&root);
    fs::rename(root.join("ignored/package"), root.join("visible")).unwrap();
    monitor.refresh();
    fs::write(root.join("visible/nested/file.txt"), "now visible").unwrap();
    monitor.refresh();
    fs::rename(root.join("visible"), root.join("ignored/package")).unwrap();
    monitor.refresh();
    fs::write(
        root.join("ignored/package/nested/file.txt"),
        "ignored again",
    )
    .unwrap();
    monitor.quiet();
    fs::rename(root.join("ignored/package"), root.join("visible-again")).unwrap();
    monitor.refresh();
    fs::write(root.join("visible-again/nested/file.txt"), "visible again").unwrap();
    monitor.refresh();
}

#[test]
fn recreated_external_policy_parent_is_revalidated_without_native_watches() {
    let (_temp, root) = repository();
    let external = unique_temp_dir("gitcomet-recreated-external-policy");
    let parent = external.path().join("configuration");
    fs::create_dir(&parent).unwrap();
    let input = parent.join("ignore");
    run_git(
        &root,
        &["config", "core.excludesFile", input.to_str().unwrap()],
    );
    fs::create_dir_all(root.join("generated/nested")).unwrap();
    let file = root.join("generated/nested/source.txt");
    fs::write(&file, "before").unwrap();
    let monitor = RunningMonitor::start(&root);
    for cycle in 0..3 {
        fs::remove_dir_all(&parent).unwrap();
        fs::create_dir(&parent).unwrap();
        let ignored = cycle % 2 == 0;
        fs::write(&input, if ignored { "generated/\n" } else { "" }).unwrap();
        assert_native_quiet(&monitor);
        monitor.revalidate();
        monitor.refresh();
        fs::write(&file, format!("source edit {cycle}")).unwrap();
        if ignored {
            monitor.quiet();
        } else {
            monitor.refresh();
        }
    }
}

#[test]
fn git_metadata_noise_is_filtered_before_debouncing() {
    let (_temp, root) = repository();
    let roots = [
        root.join(".git"),
        root.join(".git/modules/child"),
        root.join(".git/worktrees/retained"),
    ];
    for git in &roots[1..] {
        run_git(&root, &["init", "--bare", git.to_str().unwrap()]);
    }
    for git in &roots {
        fs::create_dir_all(git.join("objects/12")).unwrap();
        fs::create_dir_all(git.join("lfs/tmp")).unwrap();
    }
    fs::create_dir(root.join("source")).unwrap();
    fs::write(root.join("source/file.txt"), "before").unwrap();
    let monitor = RunningMonitor::start(&root);
    let before = monitor.native_events.load(Ordering::Relaxed);
    for git in &roots {
        for suffix in [
            "objects/12/temporary",
            "lfs/tmp/temporary",
            "index.lock",
            ".watchman-cookie-host-123",
        ] {
            let path = git.join(suffix);
            fs::write(&path, "temporary metadata").unwrap();
            fs::remove_file(&path).unwrap();
        }
    }
    monitor.quiet();
    assert!(
        monitor.native_events.load(Ordering::Relaxed) > before,
        "native probe did not deliver any traffic"
    );
    let index = root.join(".git/index");
    // Each path is touched only once after startup has settled. Require its
    // actual native observation and delivered refresh, without a quiet window
    // between unrelated positive assertions.
    assert!(
        monitor
            .expect_change(&index, || {
                fs::write(&index, fs::read(&index).unwrap()).unwrap();
            })
            .index
    );
    let head = root.join(".git/HEAD");
    assert!(
        monitor
            .expect_change(&head, || {
                fs::write(&head, fs::read(&head).unwrap()).unwrap();
            })
            .git_state
    );
    let source = root.join("source/file.txt");
    assert!(
        monitor
            .expect_change(&source, || fs::write(&source, "source edit").unwrap())
            .worktree
    );
}

#[test]
fn metadata_noise_in_a_mixed_event_preserves_real_changes() {
    let (_temp, root) = repository();
    let mut rules = load_gitignore_rules(&root);
    let snapshot = rules.state.snapshot();
    let cookie = root.join(".git/.watchman-cookie-host-123");
    assert_eq!(snapshot.classify(&cookie), PathClass::Cache);
    // Similar names outside the administrative root are ordinary source or refs.
    assert_eq!(
        snapshot.classify(&root.join(".watchman-cookie-source")),
        PathClass::Worktree
    );
    assert_eq!(
        snapshot.classify(&root.join(".git/refs/heads/.watchman-cookie-branch")),
        PathClass::Git { tags: false }
    );
    let event = notify::Event::new(EventKind::Modify(ModifyKind::Any))
        .add_path(cookie)
        .add_path(root.join(".git/index.lock"))
        .add_path(root.join(".git"))
        .add_path(root.join(".git/index"))
        .add_path(root.join("source.txt"));
    assert_eq!(triage(&snapshot, &event), Triage::Relevant);
    let effect = summarize(&snapshot, &mut rules.state.rules, &event);
    assert!(effect.index_dirty && !effect.policy_dirty);
    assert_eq!(
        effect.change,
        Some(RepoExternalChange {
            worktree: true,
            index: true,
            git_state: false,
            tags: false,
            verification_context: false,
        })
    );
}

#[test]
fn recursive_roots_share_overlapping_coverage_without_hiding_sibling_repositories() {
    let temp = unique_temp_dir("gitcomet-overlapping-roots");
    let base = normalized(temp.path());
    let workdir = base.join("project");
    let external = base.join("project-sibling/.git");
    let mut policy = PolicySnapshot::default();
    policy.workdir = workdir.clone();
    policy.git_roots.extend([
        workdir.join(".git"),
        workdir.join(".git/modules/child"),
        external.clone(),
        external.join("worktrees/linked"),
    ]);
    let roots = native_watcher::minimal_roots(&policy);
    assert_eq!(roots.len(), 2);
    assert!(roots.contains(&workdir));
    assert!(roots.contains(&external));
}

#[test]
fn repository_root_events_do_not_query_an_empty_ignore_path() {
    let (_temp, root) = repository();
    let mut rules = load_gitignore_rules(&root);
    for hint in [None, Some(true)] {
        assert!(!rules.is_ignored_rel(Path::new(""), hint));
        assert!(
            !rules.failed,
            "the repository root is not an invalid ignore input"
        );
    }
}

#[test]
fn git_directory_lifecycle_events_survive_timestamp_echo_filtering() {
    let (_temp, root) = repository();
    let mut rules = load_gitignore_rules(&root);
    let snapshot = rules.state.snapshot();
    for kind in [
        EventKind::Create(CreateKind::Folder),
        EventKind::Remove(RemoveKind::Folder),
        EventKind::Modify(ModifyKind::Name(notify::event::RenameMode::Any)),
    ] {
        let event = notify::Event::new(kind).add_path(root.join(".git"));
        assert_eq!(triage(&snapshot, &event), Triage::Relevant);
        assert!(summarize(&snapshot, &mut rules.state.rules, &event).policy_dirty);
    }
    let rescan = notify::Event::new(EventKind::Modify(ModifyKind::Any))
        .add_path(root.join(".git"))
        .set_flag(notify::event::Flag::Rescan);
    assert_eq!(triage(&snapshot, &rescan), Triage::Rescan);
}

#[cfg(windows)]
#[test]
fn unc_event_spellings_preserve_repository_and_cache_boundaries() {
    let workdir = PathBuf::from(r"\\server\share\repository");
    let mut policy = PolicySnapshot::default();
    policy.workdir = workdir.clone();
    policy.git_roots.insert(workdir.join(".git"));
    policy.cache_roots.insert(workdir.join(".git/lfs"));
    for (native, expected) in [
        (
            r"\\?\UNC\server\share\repository\.git\lfs\tmp\clean",
            PathClass::Cache,
        ),
        (
            r"\\?\UNC\server\share\repository\source\file.txt",
            PathClass::Worktree,
        ),
        (
            r"\\?\UNC\server\other-share\repository\source\file.txt",
            PathClass::Outside,
        ),
    ] {
        assert_eq!(policy.classify(&normalized(Path::new(native))), expected);
    }
}
