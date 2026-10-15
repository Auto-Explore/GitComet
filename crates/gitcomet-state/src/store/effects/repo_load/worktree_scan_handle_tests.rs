use super::*;
use crate::store::repository_runtime::RepositoryRuntime;

use crate::store::tests::DummyRepo;

/// Counts opens so the tests can tell a cache hit from a reopen.
struct CountingBackend {
    opens: std::sync::atomic::AtomicUsize,
}

impl GitBackend for CountingBackend {
    fn open(&self, workdir: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
        self.opens
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(Arc::new(DummyRepo::new(workdir)))
    }
}

impl CountingBackend {
    fn new() -> Self {
        Self {
            opens: std::sync::atomic::AtomicUsize::new(0),
        }
    }

    fn opens(&self) -> usize {
        self.opens.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A map of this test's own. These used to share the process-wide static, and
/// the over-the-limit test below evicts sixteen entries as it runs — under
/// cargo's parallel harness it was evicting the *other* tests' entries, and
/// they failed on an open count one higher than they asked for.
fn handles() -> Mutex<WorktreeScanHandles> {
    Mutex::new(WorktreeScanHandles::default())
}

#[test]
fn equal_repo_ids_in_different_stores_keep_independent_cache_ownership() {
    let handles = handles();
    let backend = CountingBackend::new();
    let path = PathBuf::from("/wt/shared-between-stores");
    let first = crate::model::RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/first-window"),
        },
    );
    let second = crate::model::RepoState::new_opening(
        RepoId(1),
        gitcomet_core::domain::RepoSpec {
            workdir: PathBuf::from("/second-window"),
        },
    );
    let first_runtime = RepositoryRuntime::new(first.lifetime_id());
    let second_runtime = RepositoryRuntime::new(second.lifetime_id());
    assert_eq!(first.id, second.id);
    assert_ne!(first.lifetime(), second.lifetime());
    let initial =
        worktree_scan_handle(&handles, &backend, &first_runtime.worktree_cache(), &path).unwrap();
    let shared =
        worktree_scan_handle(&handles, &backend, &second_runtime.worktree_cache(), &path).unwrap();
    assert!(Arc::ptr_eq(&initial, &shared));

    retain_worktree_scan_handles(&handles, first.lifetime_id(), &[]);

    let retained =
        worktree_scan_handle(&handles, &backend, &second_runtime.worktree_cache(), &path).unwrap();
    assert!(Arc::ptr_eq(&initial, &retained));
    assert_eq!(backend.opens(), 1);
    retain_worktree_scan_handles(&handles, second.lifetime_id(), &[]);
    assert!(lock_worktree_scan_handles(&handles).entries.is_empty());
}

#[test]
fn closed_lease_cannot_acquire_another_owners_cached_handle() {
    let handles = handles();
    let backend = CountingBackend::new();
    let path = PathBuf::from("/wt/shared-with-closed-owner");
    let live = RepositoryRuntime::new(RepoLifetime::allocate());
    let closed = RepositoryRuntime::new(RepoLifetime::allocate());
    let closed_lease = closed.worktree_cache();
    let initial = worktree_scan_handle(&handles, &backend, &live.worktree_cache(), &path).unwrap();
    drop(closed);

    assert!(worktree_scan_handle(&handles, &backend, &closed_lease, &path).is_none());
    let retained = worktree_scan_handle(&handles, &backend, &live.worktree_cache(), &path).unwrap();
    assert!(Arc::ptr_eq(&initial, &retained));
    assert_eq!(backend.opens(), 1);
    assert!(
        !lock_worktree_scan_handles(&handles).entries[&path]
            .owners
            .contains(&closed_lease.lifetime()),
        "a stale task must not restore cache ownership after its runtime closes"
    );
}

#[test]
fn twenty_worktrees_stay_warm_across_linked_tabs() {
    let handles = handles();
    let backend = CountingBackend::new();
    let first_owner = RepoLifetime::allocate();
    let first_owner_runtime = RepositoryRuntime::new(first_owner);
    let second_owner = RepoLifetime::allocate();
    let second_owner_runtime = RepositoryRuntime::new(second_owner);
    let paths: Vec<_> = (0..20)
        .map(|ix| PathBuf::from(format!("/wt/shared-warm-{ix}")))
        .collect();
    let mut first_handles = Vec::new();
    for path in &paths {
        first_handles.push(
            worktree_scan_handle(
                &handles,
                &backend,
                &first_owner_runtime.worktree_cache(),
                path,
            )
            .unwrap(),
        );
    }
    for runtime in [
        &second_owner_runtime,
        &first_owner_runtime,
        &second_owner_runtime,
    ] {
        for (path, first) in paths.iter().zip(&first_handles) {
            let reused =
                worktree_scan_handle(&handles, &backend, &runtime.worktree_cache(), path).unwrap();
            assert!(
                Arc::ptr_eq(first, &reused),
                "the checkout keeps its warmed status state"
            );
        }
    }
    assert_eq!(
        backend.opens(),
        paths.len(),
        "each physical checkout is opened just once"
    );
    assert_eq!(
        lock_worktree_scan_handles(&handles).entries.len(),
        paths.len()
    );
    retain_worktree_scan_handles(&handles, first_owner, &[]);
    assert_eq!(
        lock_worktree_scan_handles(&handles).entries.len(),
        paths.len(),
        "closing one tab retains shared handles"
    );
    retain_worktree_scan_handles(&handles, second_owner, &[]);
    assert!(
        lock_worktree_scan_handles(&handles).entries.is_empty(),
        "closing the last owner releases the cache"
    );
}

#[cfg(unix)]
#[test]
fn alternate_checkout_paths_share_a_cache_entry() {
    let dir = tempfile::tempdir().unwrap();
    let checkout = dir.path().join("checkout");
    let alias = dir.path().join("alias");
    std::fs::create_dir(&checkout).unwrap();
    std::os::unix::fs::symlink(&checkout, &alias).unwrap();
    let handles = handles();
    let backend = CountingBackend::new();
    let first_owner = RepoLifetime::allocate();
    let first_owner_runtime = RepositoryRuntime::new(first_owner);
    let second_owner = RepoLifetime::allocate();
    let second_owner_runtime = RepositoryRuntime::new(second_owner);
    let first = worktree_scan_handle(
        &handles,
        &backend,
        &first_owner_runtime.worktree_cache(),
        &checkout,
    )
    .unwrap();
    let second = worktree_scan_handle(
        &handles,
        &backend,
        &second_owner_runtime.worktree_cache(),
        &alias,
    )
    .unwrap();
    assert!(Arc::ptr_eq(&first, &second));
    assert_eq!(backend.opens(), 1);
    retain_worktree_scan_handles(&handles, first_owner, &[]);
    retain_worktree_scan_handles(&handles, second_owner, &[alias]);
    assert_eq!(lock_worktree_scan_handles(&handles).entries.len(), 1);
}

/// Opening a repository is discovery plus config parsing, and the scan runs on
/// every git-state flush. A repo with more worktrees than the cache holds must
/// still keep its hottest ones: clearing the map wholesale at the limit made
/// exactly that case reopen nearly everything every scan.
#[test]
fn a_repo_over_the_handle_limit_keeps_its_hottest_worktrees() {
    let handles = handles();
    let backend = CountingBackend::new();
    let repo_id = RepoLifetime::allocate();
    let repo_id_runtime = RepositoryRuntime::new(repo_id);
    let hot = PathBuf::from("/wt/hot-4001");
    let paths: Vec<PathBuf> = (0..WORKTREE_SCAN_HANDLE_LIMIT + 4)
        .map(|ix| PathBuf::from(format!("/wt/cold-4001-{ix}")))
        .collect();

    // The hot worktree is touched before and after every cold one. Eviction
    // takes the entry the scan has just finished with, which is always the
    // cold one -- touching `hot` again immediately afterwards makes it the
    // most recent, but by then the slot has already been taken.
    worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), &hot)
        .expect("stub backend opens");
    for path in &paths {
        worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), path)
            .expect("stub backend opens");
        worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), &hot)
            .expect("stub backend opens");
    }

    let opens_before = backend.opens();
    worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), &hot)
        .expect("stub backend opens");
    assert_eq!(
        backend.opens(),
        opens_before,
        "the repeatedly used worktree must still be cached"
    );
}

/// The access pattern this cache actually sees: every scan walks
/// `list_worktrees` in the same order, so a repo with more worktrees than the
/// limit revisits them cyclically. That is LRU's worst case — it evicts
/// precisely the entry the next scan reaches first, and every handle is
/// reopened on every pass, which is the behaviour the cache exists to prevent.
#[test]
fn repeated_scans_of_an_oversized_repo_do_not_reopen_everything() {
    let handles = handles();
    let backend = CountingBackend::new();
    let repo_id = RepoLifetime::allocate();
    let repo_id_runtime = RepositoryRuntime::new(repo_id);
    let worktrees: Vec<PathBuf> = (0..WORKTREE_SCAN_HANDLE_LIMIT + 4)
        .map(|ix| PathBuf::from(format!("/wt/seq-4005-{ix}")))
        .collect();

    let scan = || {
        let before = backend.opens();
        for path in &worktrees {
            worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), path)
                .expect("stub backend opens");
        }
        assert!(lock_worktree_scan_handles(&handles).entries.len() <= WORKTREE_SCAN_HANDLE_LIMIT);
        backend.opens() - before
    };

    assert_eq!(
        scan(),
        worktrees.len(),
        "the first scan has nothing cached and opens every worktree"
    );

    let second = scan();
    let third = scan();
    // Under LRU both of these were `worktrees.len()` -- a complete miss on
    // every worktree, on every scan, forever.
    assert!(
        second < worktrees.len() && third < worktrees.len(),
        "a cyclic rescan must keep hitting the cache, got {second} then {third} \
             reopens of {} worktrees",
        worktrees.len()
    );
    assert!(
        third <= worktrees.len() - WORKTREE_SCAN_HANDLE_LIMIT + 1,
        "all but the tail past the limit should stay cached, got {third} reopens"
    );
}

/// Handles are pruned against the worktree list the scan actually walked, so a
/// removed worktree does not keep a repository open for the process lifetime.
#[test]
fn a_scan_drops_handles_for_worktrees_it_no_longer_lists() {
    let handles = handles();
    let backend = CountingBackend::new();
    let repo_id = RepoLifetime::allocate();
    let repo_id_runtime = RepositoryRuntime::new(repo_id);
    let kept = PathBuf::from("/wt/kept-4002");
    let removed = PathBuf::from("/wt/removed-4002");

    worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), &kept)
        .expect("stub backend opens");
    worktree_scan_handle(
        &handles,
        &backend,
        &repo_id_runtime.worktree_cache(),
        &removed,
    )
    .expect("stub backend opens");
    assert_eq!(backend.opens(), 2);

    retain_worktree_scan_handles(&handles, repo_id, std::slice::from_ref(&kept));

    worktree_scan_handle(&handles, &backend, &repo_id_runtime.worktree_cache(), &kept)
        .expect("stub backend opens");
    assert_eq!(
        backend.opens(),
        2,
        "the worktree still in the list keeps its handle"
    );
    worktree_scan_handle(
        &handles,
        &backend,
        &repo_id_runtime.worktree_cache(),
        &removed,
    )
    .expect("stub backend opens");
    assert_eq!(
        backend.opens(),
        3,
        "the worktree dropped from the list must have been released"
    );
}

/// One repo's scan must not evict another's handles: the map is process-wide.
#[test]
fn pruning_one_repos_handles_leaves_another_repos_alone() {
    let handles = handles();
    let backend = CountingBackend::new();
    let mine = RepoLifetime::allocate();
    let mine_runtime = RepositoryRuntime::new(mine);
    let theirs = RepoLifetime::allocate();
    let theirs_runtime = RepositoryRuntime::new(theirs);
    let path = PathBuf::from("/wt/shared-4003");

    worktree_scan_handle(&handles, &backend, &mine_runtime.worktree_cache(), &path)
        .expect("stub backend opens");
    worktree_scan_handle(&handles, &backend, &theirs_runtime.worktree_cache(), &path)
        .expect("stub backend opens");
    assert_eq!(backend.opens(), 1, "tabs share a single checkout handle");

    retain_worktree_scan_handles(&handles, mine, &[]);

    worktree_scan_handle(&handles, &backend, &theirs_runtime.worktree_cache(), &path)
        .expect("stub backend opens");
    assert_eq!(
        backend.opens(),
        1,
        "the other repo's handle must survive this repo's prune"
    );
}

/// The map is process-wide, and a rule that always spends the requester's own
/// budget freezes whatever split the tabs happened to open in: the repo that
/// scanned second is pinned to the leftovers forever and re-pays discovery
/// for the tail of its list on every flush.
#[test]
fn a_second_repo_takes_slots_from_the_larger_holder_rather_than_starving() {
    let handles = handles();
    let backend = CountingBackend::new();
    let first = RepoLifetime::allocate();
    let first_runtime = RepositoryRuntime::new(first);
    let second = RepoLifetime::allocate();
    let second_runtime = RepositoryRuntime::new(second);
    let paths = |repo: RepoLifetime| -> Vec<PathBuf> {
        (0..WORKTREE_SCAN_HANDLE_LIMIT)
            .map(|ix| PathBuf::from(format!("/wt/share-{}-{ix}", repo.get())))
            .collect()
    };
    let first_paths = paths(first);
    let second_paths = paths(second);

    // The first repo fills two thirds of the map, then the second arrives.
    let first_share = WORKTREE_SCAN_HANDLE_LIMIT * 2 / 3;
    for path in &first_paths[..first_share] {
        worktree_scan_handle(&handles, &backend, &first_runtime.worktree_cache(), path)
            .expect("stub backend opens");
    }

    let held = |repo_id: RepoLifetime| {
        lock_worktree_scan_handles(&handles)
            .entries
            .values()
            .filter(|entry| entry.owners.contains(&repo_id))
            .count()
    };

    // Several full scans by the second repo: under the old rule it evicted
    // the entry it had just used and never grew past the leftovers.
    for _ in 0..3 {
        for path in &second_paths {
            worktree_scan_handle(&handles, &backend, &second_runtime.worktree_cache(), path)
                .expect("stub backend opens");
        }
    }

    let second_held = held(second);
    assert!(
        second_held > WORKTREE_SCAN_HANDLE_LIMIT - first_share,
        "the second repo must grow past the leftovers, got {second_held} of \
             {WORKTREE_SCAN_HANDLE_LIMIT}"
    );
    assert!(
        held(first) > 0,
        "and must not have cleared the first repo out either"
    );
}

/// Closing a repo is the only thing that can release its handles: the per-scan
/// prune runs from that repo's own scan, and a closed repo never scans again.
/// Without this the handles sat there for the life of the process.
#[test]
fn closing_a_repo_releases_every_handle_it_held() {
    let backend = CountingBackend::new();
    let closed = RepoLifetime::allocate();
    let closed_runtime = RepositoryRuntime::new(closed);
    let other = RepoLifetime::allocate();
    let other_runtime = RepositoryRuntime::new(other);
    let paths = [
        PathBuf::from("/wt/closing-4006-a"),
        PathBuf::from("/wt/closing-4006-b"),
    ];

    // Runtime disposal reaches the shared cache. Each owner has a unique
    // lifetime, and both runtimes are dropped before this test returns.
    let shared = worktree_scan_handles();
    for path in &paths {
        worktree_scan_handle(shared, &backend, &closed_runtime.worktree_cache(), path)
            .expect("stub backend opens");
    }
    worktree_scan_handle(shared, &backend, &other_runtime.worktree_cache(), &paths[0])
        .expect("stub backend opens");

    drop(closed_runtime);

    let held = |repo_id: RepoLifetime| {
        lock_worktree_scan_handles(shared)
            .entries
            .values()
            .filter(|entry| entry.owners.contains(&repo_id))
            .count()
    };
    assert_eq!(held(closed), 0, "the closed repo must hold nothing");
    assert_eq!(held(other), 1, "and must not have taken anyone else's");

    drop(other_runtime);
}
