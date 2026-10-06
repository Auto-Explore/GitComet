use super::*;
use gitcomet_core::history_index::HistoryIndexHandle;
use gitcomet_core::history_perf::{Work, capture, count, with_capture_context};
use gitcomet_core::services::{
    CancellationToken, GitRepository, HistoryReadRequest, HistoryReadResult, HistoryRefFilter,
    HistorySnapshot, RepositoryOptions,
};

fn index(repo: &dyn GitRepository, mode: HistoryMode, author: Option<&str>) -> HistoryIndexHandle {
    repo.build_history_index(mode, author, &CancellationToken::new(), &mut |_| {})
        .unwrap()
        .unwrap()
}

fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    run_git(dir.path(), &["init", "-q", "-b", "master"]);
    fast_import_linear_history(dir.path(), 4096);
    dir
}

fn page_snapshot(repo: &dyn GitRepository) -> HistorySnapshot {
    match repo
        .read_history(
            HistoryMode::AllBranches,
            None,
            &HistoryReadRequest::Page {
                limit: 32,
                cursor: None,
                snapshot: None,
            },
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap()
    {
        HistoryReadResult::Page {
            snapshot: Some(snapshot),
            ..
        } => snapshot,
        result => panic!("expected a page snapshot, got {result:?}"),
    }
}

#[track_caller]
fn assert_owner_released<T: ?Sized>(weak: &std::sync::Weak<T>) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while weak.strong_count() != 0 && std::time::Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(
        weak.upgrade().is_none(),
        "owner remained alive after timeout"
    );
}

#[test]
fn pages_and_shared_indexes_intern_many_tip_snapshots_in_either_order() {
    for index_first in [false, true] {
        let dir = fixture();
        let linked = dir.path().join("linked");
        run_git(
            dir.path(),
            &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
        );
        let ids = git_stdout(dir.path(), &["rev-list", "--max-count=512", "HEAD"]);
        let mut refs = String::from("# pack-refs with: peeled sorted\n");
        for (i, id) in ids.lines().enumerate() {
            refs.push_str(&format!("{id} refs/heads/tip-{i:04}\n"));
        }
        std::fs::write(dir.path().join(".git/packed-refs"), refs).unwrap();
        let main = GixBackend.open(dir.path()).unwrap();
        let other = GixBackend.open(&linked).unwrap();
        let early_index = index_first.then(|| index(&*main, HistoryMode::AllBranches, None));
        let snapshot = page_snapshot(&*main);
        assert!(snapshot.0.len() > 10_000);
        let built = early_index.unwrap_or_else(|| index(&*main, HistoryMode::AllBranches, None));
        let linked_index = index(&*other, HistoryMode::AllBranches, None);
        assert!(Arc::ptr_eq(&built, &linked_index));
        assert!(
            Arc::weak_count(&built) > 0,
            "completed indexes are registered weakly"
        );
        for current in [
            &built.snapshot,
            &linked_index.snapshot,
            &page_snapshot(&*other),
            &page_snapshot(&*main),
        ] {
            assert!(
                Arc::ptr_eq(&snapshot.0, &current.0),
                "equal queries must publish the same snapshot allocation"
            );
        }
    }
}

#[test]
fn missing_alternates_keep_a_stable_private_generation() {
    let dir = fixture();
    let linked = dir.path().join("linked");
    run_git(
        dir.path(),
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    let missing = dir.path().join("missing-objects");
    std::fs::write(
        dir.path().join(".git/objects/info/alternates"),
        format!("{}\n", missing.display()),
    )
    .unwrap();
    let main = GixBackend.open(dir.path()).unwrap();
    let other = GixBackend.open(&linked).unwrap();
    let snapshot = page_snapshot(&*main);
    let built = index(&*main, HistoryMode::AllBranches, None);
    let linked_index = index(&*other, HistoryMode::AllBranches, None);
    let cancel = CancellationToken::new();
    assert!(
        !Arc::ptr_eq(&built, &linked_index),
        "failed identities must not enable cross-worktree sharing"
    );
    for _ in 0..4 {
        assert_eq!(snapshot, page_snapshot(&*main));
        assert_eq!(snapshot, built.snapshot);
        assert!(Arc::ptr_eq(
            &built,
            &index(&*main, HistoryMode::AllBranches, None)
        ));
        let block = main
            .read_history_range_shared(&built, 0..32, &cancel)
            .unwrap();
        assert_eq!(block.snapshot, snapshot);
        assert_eq!(block.commits.len(), 32);
        other
            .read_history_range_shared(&linked_index, 0..32, &cancel)
            .unwrap();
    }
    // A successfully identified alternate changes interpretation and permits
    // sharing again. The private generation must not mask that transition.
    std::fs::create_dir(&missing).unwrap();
    let after = index(&*main, HistoryMode::AllBranches, None);
    assert_ne!(built.snapshot, after.snapshot);
    assert!(
        main.read_history_range_shared(&built, 0..32, &cancel)
            .is_err()
    );
    assert!(Arc::ptr_eq(
        &after,
        &index(&*other, HistoryMode::AllBranches, None)
    ));
    main.read_history_range_shared(&after, 0..32, &cancel)
        .unwrap();
}

#[test]
fn four_windows_build_once_and_retain_identical_allocations() {
    let dir = fixture();
    let windows: Vec<_> = (0..4)
        .map(|_| GixBackend.open(dir.path()).unwrap())
        .collect();
    assert!(windows.iter().all(|repo| Arc::ptr_eq(repo, &windows[0])));
    let _capture = capture();
    let barrier = std::sync::Barrier::new(4);
    let results: Vec<_> = std::thread::scope(|scope| {
        let threads: Vec<_> = windows
            .iter()
            .map(|repo| {
                let barrier = &barrier;
                scope.spawn(with_capture_context(move || {
                    barrier.wait();
                    let index = index(&**repo, HistoryMode::AllBranches, None);
                    let block = repo
                        .read_history_range_shared(&index, 0..256, &CancellationToken::new())
                        .unwrap();
                    (index, block)
                }))
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect()
    });
    assert_eq!(count(Work::HistoryIndexBuild), 1);
    assert_eq!(count(Work::LogTopologyBuild), 1);
    assert_eq!(count(Work::RangeObjectRead), 256);
    for (index, block) in &results {
        assert!(Arc::ptr_eq(index, &results[0].0));
        assert!(Arc::ptr_eq(block, &results[0].1));
    }
}

#[test]
fn linked_worktrees_share_matching_queries_and_isolate_heads_filters_and_options() {
    let dir = fixture();
    let linked = dir.path().join("linked");
    run_git(
        dir.path(),
        &[
            "worktree",
            "add",
            "-qb",
            "linked",
            linked.to_str().unwrap(),
            "HEAD~100",
        ],
    );
    let main = GixBackend.open(dir.path()).unwrap();
    let other = GixBackend.open(&linked).unwrap();
    assert!(!Arc::ptr_eq(&main, &other));
    // Compare canonical paths on both sides: macOS can resolve /private/var,
    // and Windows canonicalization adds a verbatim path prefix.
    assert_eq!(
        other.spec().workdir.canonicalize().unwrap(),
        linked.canonicalize().unwrap()
    );
    let all = index(&*main, HistoryMode::AllBranches, None);
    let linked_all = index(&*other, HistoryMode::AllBranches, None);
    assert!(Arc::ptr_eq(&all, &linked_all));
    let a = main
        .read_history_range_shared(&all, 0..256, &CancellationToken::new())
        .unwrap();
    let b = other
        .read_history_range_shared(&linked_all, 0..256, &CancellationToken::new())
        .unwrap();
    assert!(Arc::ptr_eq(&a, &b));
    assert!(!Arc::ptr_eq(
        &index(&*main, HistoryMode::FullReachable, None),
        &index(&*other, HistoryMode::FullReachable, None)
    ));
    let author = index(&*main, HistoryMode::AllBranches, Some(" You "));
    assert!(Arc::ptr_eq(
        &author,
        &index(&*other, HistoryMode::AllBranches, Some("you"))
    ));
    assert!(!Arc::ptr_eq(&all, &author));
    let options = RepositoryOptions::default()
        .with_history_ref_filter(HistoryRefFilter::excluding(["refs/heads/linked"]));
    let filtered = GixBackend.open_with_options(dir.path(), &options).unwrap();
    assert!(!Arc::ptr_eq(&main, &filtered));
    assert!(!Arc::ptr_eq(
        &all,
        &index(&*filtered, HistoryMode::AllBranches, None)
    ));
    run_git(dir.path(), &["config", "extensions.worktreeConfig", "true"]);
    run_git(
        &linked,
        &["config", "--worktree", "core.multiPackIndex", "false"],
    );
    let incompatible = index(&*other, HistoryMode::AllBranches, None);
    assert!(!Arc::ptr_eq(&all, &incompatible));
    assert!(Arc::ptr_eq(
        &all,
        &index(&*main, HistoryMode::AllBranches, None)
    ));
}

#[test]
fn common_maintenance_refreshes_all_worktrees_after_repacking() {
    let dir = fixture();
    let linked = dir.path().join("linked");
    run_git(
        dir.path(),
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    let main = GixBackend.open(dir.path()).unwrap();
    let other = GixBackend.open(&linked).unwrap();
    let before = index(&*main, HistoryMode::AllBranches, None);
    let cancel = CancellationToken::new();
    let pinned = main
        .read_history_range_shared(&before, 0..256, &cancel)
        .unwrap();
    run_git(dir.path(), &["repack", "-ad"]);
    GixBackend.release_object_stores(&main.common_dir().unwrap());
    assert!(
        main.read_history_range_shared(&before, 0..256, &cancel)
            .is_err()
    );
    assert!(
        other
            .read_history_range_shared(&before, 0..256, &cancel)
            .is_err()
    );
    let after = index(&*other, HistoryMode::AllBranches, None);
    assert!(!Arc::ptr_eq(&before, &after));
    assert!(Arc::ptr_eq(
        &after,
        &index(&*main, HistoryMode::AllBranches, None)
    ));
    let refreshed = other
        .read_history_range_shared(&after, 0..256, &cancel)
        .unwrap();
    assert!(!Arc::ptr_eq(&pinned, &refreshed));
    assert_eq!(pinned.commits, refreshed.commits);
}

#[test]
fn replacement_refs_invalidate_indexes_and_decoded_blocks() {
    let dir = fixture();
    let repo = GixBackend.open(dir.path()).unwrap();
    let before = index(&*repo, HistoryMode::FullReachable, None);
    let cancel = CancellationToken::new();
    repo.read_history_range_shared(&before, 0..256, &cancel)
        .unwrap();
    run_git(dir.path(), &["replace", "HEAD", "HEAD~10"]);
    let after = index(&*repo, HistoryMode::FullReachable, None);
    assert!(!Arc::ptr_eq(&before, &after));
    assert_ne!(before.snapshot, after.snapshot);
    assert!(
        repo.read_history_range_shared(&before, 0..256, &cancel)
            .is_err()
    );
    assert!(
        repo.read_history_range_shared(&after, 0..256, &cancel)
            .is_ok()
    );
}

#[test]
fn repository_replacement_and_a_hundred_reopens_release_owners() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repo");
    std::fs::create_dir(&path).unwrap();
    run_git(&path, &["init", "-q", "-b", "master"]);
    fast_import_linear_history(&path, 10);
    let old = GixBackend.open(&path).unwrap();
    #[cfg(windows)]
    {
        // Windows cannot rename an ancestor of an open directory handle,
        // including the identities that prevent filesystem-ID reuse. Check
        // that closing the owner releases those handles before replacement.
        let weak = Arc::downgrade(&old);
        drop(old);
        assert_owner_released(&weak);
    }
    std::fs::rename(&path, dir.path().join("old")).unwrap();
    std::fs::create_dir(&path).unwrap();
    run_git(&path, &["init", "-q", "-b", "master"]);
    fast_import_linear_history(&path, 20);
    let replacement = GixBackend.open(&path).unwrap();
    #[cfg(not(windows))]
    assert!(!Arc::ptr_eq(&old, &replacement));
    assert_eq!(
        index(&*replacement, HistoryMode::FullReachable, None).len(),
        20
    );
    drop(replacement);
    for _ in 0..100 {
        let repo = GixBackend.open(&path).unwrap();
        let index = index(&*repo, HistoryMode::FullReachable, None);
        let block = repo
            .read_history_range_shared(&index, 0..20, &CancellationToken::new())
            .unwrap();
        let weak = (
            Arc::downgrade(&repo),
            Arc::downgrade(&index),
            Arc::downgrade(&block),
        );
        drop(block);
        drop(index);
        drop(repo);
        // Parallel registry readers may temporarily upgrade repository owners,
        // and the worker may still be returning after publishing its index.
        // Wait for each owner: releasing the index does not imply the repository
        // or its retained decoded blocks have finished being dropped.
        assert_owner_released(&weak.0);
        assert_owner_released(&weak.1);
        assert_owner_released(&weak.2);
    }
}

#[test]
fn linked_worktrees_keep_staging_previews_attributes_and_lfs_configuration_local() {
    use gitcomet_core::domain::{DiffArea, DiffTarget};
    let dir = fixture();
    let linked = dir.path().join("linked");
    run_git(
        dir.path(),
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    run_git(dir.path(), &["config", "extensions.worktreeConfig", "true"]);
    run_git(
        dir.path(),
        &["config", "--worktree", "filter.lfs.clean", ""],
    );
    run_git(
        dir.path(),
        &["config", "--worktree", "filter.lfs.process", ""],
    );
    let main = GixBackend.open(dir.path()).unwrap();
    let other = GixBackend.open(&linked).unwrap();
    // Resolve and retain a shared all-branches index before local mutations.
    let all = index(&*main, HistoryMode::AllBranches, None);
    assert!(Arc::ptr_eq(
        &all,
        &index(&*other, HistoryMode::AllBranches, None)
    ));
    std::fs::write(dir.path().join("file.txt"), "main edit\n").unwrap();
    std::fs::write(linked.join("file.txt"), "linked edit\n").unwrap();
    other.stage(&[Path::new("file.txt")]).unwrap();
    assert!(main.status().unwrap().staged.is_empty());
    assert_eq!(other.status().unwrap().staged.len(), 1);
    let unstaged = DiffTarget::working_tree("file.txt".into(), DiffArea::Unstaged);
    let staged = DiffTarget::working_tree("file.txt".into(), DiffArea::Staged);
    assert!(main.diff_unified(&unstaged).unwrap().contains("+main edit"));
    assert!(
        other
            .diff_unified(&staged)
            .unwrap()
            .contains("+linked edit")
    );
    main.stage(&[Path::new("file.txt")]).unwrap();
    other.unstage(&[Path::new("file.txt")]).unwrap();
    assert_eq!(main.status().unwrap().staged.len(), 1);
    assert!(other.status().unwrap().staged.is_empty());
    let cancel = CancellationToken::new();
    assert!(
        !main
            .large_file_support_cancellable(&cancel)
            .unwrap()
            .lfs
            .in_use()
    );
    let include = dir.path().join("linked-config");
    std::fs::write(
        &include,
        "[filter \"lfs\"]\nclean = git-lfs clean -- %f\n[lfs]\nstorage = linked-lfs\n",
    )
    .unwrap();
    run_git(
        &linked,
        &[
            "config",
            "--worktree",
            "includeIf.onbranch:linked.path",
            include.to_str().unwrap(),
        ],
    );
    std::fs::write(linked.join(".gitattributes"), "*.bin filter=lfs -text\n").unwrap();
    let support = other.large_file_support_cancellable(&cancel).unwrap();
    assert!(support.lfs.filter_configured);
    assert!(support.lfs.storage_dir.ends_with("linked-lfs"));
    assert_eq!(support.lfs.tracked_patterns.len(), 1);
    let main_support = main.large_file_support_cancellable(&cancel).unwrap();
    assert!(!main_support.lfs.filter_configured);
    assert!(main_support.lfs.tracked_patterns.is_empty());
    assert!(
        Arc::ptr_eq(&all, &index(&*other, HistoryMode::AllBranches, None)),
        "non-object config does not prevent sharing"
    );
}

#[test]
fn moving_and_removing_a_worktree_never_reuses_its_old_command_directory() {
    let dir = fixture();
    let linked = dir.path().join("linked");
    let moved = dir.path().join("moved");
    run_git(
        dir.path(),
        &["worktree", "add", "-qb", "linked", linked.to_str().unwrap()],
    );
    let old = GixBackend.open(&linked).unwrap();
    let all = index(&*old, HistoryMode::AllBranches, None);
    run_git(
        dir.path(),
        &[
            "worktree",
            "move",
            linked.to_str().unwrap(),
            moved.to_str().unwrap(),
        ],
    );
    assert!(GixBackend.open(&linked).is_err());
    let new = GixBackend.open(&moved).unwrap();
    assert!(!Arc::ptr_eq(&old, &new));
    assert_eq!(
        new.spec().workdir.canonicalize().unwrap(),
        moved.canonicalize().unwrap()
    );
    assert!(Arc::ptr_eq(
        &all,
        &index(&*new, HistoryMode::AllBranches, None)
    ));
    run_git(dir.path(), &["worktree", "remove", moved.to_str().unwrap()]);
    assert!(GixBackend.open(&moved).is_err());
}

#[test]
fn alternate_source_changes_invalidate_the_object_interpretation() {
    let dir = fixture();
    let donors = tempfile::tempdir().unwrap();
    let alternate = dir.path().join(".git/objects/info/alternates");
    let repo = GixBackend.open(dir.path()).unwrap();
    let mut indexes = Vec::new();
    for name in ["a", "b"] {
        let donor = donors.path().join(name);
        std::fs::create_dir(&donor).unwrap();
        run_git(&donor, &["init", "--bare", "-q"]);
        std::fs::write(&alternate, format!("{}\n", donor.join("objects").display())).unwrap();
        indexes.push(index(&*repo, HistoryMode::AllBranches, None));
    }
    assert!(!Arc::ptr_eq(&indexes[0], &indexes[1]));
    assert_eq!(indexes[0].len(), indexes[1].len());
}
