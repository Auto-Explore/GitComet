use super::*;
use crate::store::repository_runtime::RepositoryRuntime;
use gitcomet_core::domain::{CommitId, FileStatus, FileStatusKind};

#[test]
fn unchanged_full_scans_do_not_republish_the_list() {
    let (_dir, own, _) = crate::store::tests::repo_with_linked_worktree();
    let repo_id = RepoId(4107);
    let worker = ScanTestWorker::new();
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let repos = RepositoryRuntimes::from_iter([(repo_id, backend.open(&own).unwrap())]);
    for expected in [1, 0, 0] {
        let (tx, rx) = std::sync::mpsc::channel();
        schedule_load_worktree_dirty(
            &worker.executor,
            backend.clone(),
            &repos,
            StoreWorkerSender::for_test_msg_sender(tx),
            repo_id,
            worker.runtime.worktree_cache(),
            own.clone(),
            WorktreeDirtyScope::All,
            None,
            CancellationToken::new(),
            worker.publisher.clone(),
        );
        let mut listings = 0;
        loop {
            match rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap() {
                Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded { .. }) => listings += 1,
                Msg::Internal(crate::msg::InternalMsg::WorktreeDirtyLoaded { result, .. }) => {
                    result.unwrap();
                    break;
                }
                other => panic!("unexpected reply: {other:?}"),
            }
        }
        assert_eq!(
            listings, expected,
            "unchanged scans must not cause extra state copies"
        );
    }
}

struct ScanTestWorker {
    executor: TaskExecutor,
    runtime: RepositoryRuntime,
    publisher: WorktreeListPublisher,
}

impl ScanTestWorker {
    fn new() -> Self {
        Self {
            executor: TaskExecutor::new(1),
            runtime: RepositoryRuntime::new(RepoLifetime::allocate()),
            publisher: WorktreeListPublisher::default(),
        }
    }
}

impl Drop for ScanTestWorker {
    fn drop(&mut self) {
        // Finish workers before the runtime field releases cache ownership,
        // including when a test assertion unwinds.
        self.executor.join_workers();
    }
}

fn status(paths: &[&str]) -> RepoStatus {
    RepoStatus {
        staged: std::sync::Arc::new(Vec::new()),
        unstaged: std::sync::Arc::new(
            paths
                .iter()
                .map(|path| FileStatus {
                    path: PathBuf::from(path),
                    kind: FileStatusKind::Modified,
                    conflict: None,
                })
                .collect(),
        ),
    }
}

fn worktree() -> Worktree {
    Worktree {
        bare: false,
        path: PathBuf::from("/wt/side"),
        head: Some(CommitId("abc123".into())),
        branch: Some("side".into()),
        detached: false,
    }
}

#[test]
fn failed_scan_does_not_publish_a_worktree_listing() {
    use crate::msg::InternalMsg;
    use crate::store::tests::{DummyRepo, FailingBackend};
    let repo_id = RepoId(4106);
    let worker = ScanTestWorker::new();
    let repo: Arc<dyn GitRepository> = Arc::new(DummyRepo::new("/unavailable"));
    let repos = RepositoryRuntimes::from_iter([(repo_id, repo)]);
    for scope in [
        WorktreeDirtyScope::All,
        WorktreeDirtyScope::Paths(vec!["/linked".into()]),
    ] {
        let (tx, rx) = std::sync::mpsc::channel();
        schedule_load_worktree_dirty(
            &worker.executor,
            Arc::new(FailingBackend),
            &repos,
            StoreWorkerSender::for_test_msg_sender(tx),
            repo_id,
            worker.runtime.worktree_cache(),
            "/unavailable".into(),
            scope,
            None,
            CancellationToken::new(),
            worker.publisher.clone(),
        );
        let reply = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(
            matches!(
                reply,
                Msg::Internal(InternalMsg::WorktreeDirtyLoaded { result: Err(_), .. })
            ),
            "a failed scan must only finish its dirty-scan request: {reply:?}"
        );
        assert!(rx.recv_timeout(std::time::Duration::from_secs(5)).is_err());
    }
}

#[test]
fn panicking_worktree_scan_finishes_and_replays_pending_refresh() {
    use crate::model::{Loadable, RepoState};
    use crate::msg::{Effect, InternalMsg};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::time::Duration;

    struct PanickingOnceBackend(AtomicBool);
    impl GitBackend for PanickingOnceBackend {
        fn open(&self, path: &Path) -> Result<Arc<dyn GitRepository>, Error> {
            if self.0.swap(false, Ordering::Relaxed) {
                // Unwind after the listing was published, as when gix's
                // status walker races with removal of a linked checkout.
                panic!("injected worktree scan panic");
            }
            gitcomet_git_gix::GixBackend.open(path)
        }
    }

    let (dir, own, linked) = crate::store::tests::repo_with_linked_worktree();
    std::fs::write(linked.join("a.txt"), "linked checkout edit\n").unwrap();
    let cached_path = dir.path().join("cached-worktree");
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&own)
        .args(["worktree", "add", "--detach"])
        .arg(&cached_path)
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    std::fs::write(cached_path.join("new.txt"), "another dirty checkout\n").unwrap();
    let repo_id = RepoId(4108);
    let worker = ScanTestWorker::new();
    let cached_handle = Arc::downgrade(
        &worktree_scan_handle(
            worktree_scan_handles(),
            &gitcomet_git_gix::GixBackend,
            &worker.runtime.worktree_cache(),
            &cached_path,
        )
        .unwrap(),
    );
    let backend = Arc::new(PanickingOnceBackend(AtomicBool::new(true)));
    let repo = gitcomet_git_gix::GixBackend.open(&own).unwrap();
    let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
    repo_state.set_open(Loadable::Ready(()));
    let mut previous = worktree();
    previous.path = linked.clone();
    repo_state.set_worktree_dirty(Loadable::Ready(vec![worktree_dirty_summary(
        previous,
        status(&["a.txt"]),
        true,
        Default::default(),
    )]));
    repo_state.set_worktree_selection(Some(linked.clone()));
    let previous_dirty = repo_state.worktree_dirty.clone();
    let mut state = AppState {
        active_repo: Some(repo_id),
        repos: vec![repo_state],
        ..AppState::test_default()
    };
    let mut repos = RepositoryRuntimes::from_iter([(repo_id, repo)]);
    let id_alloc = AtomicU64::new(2);
    let reduce = |state: &mut AppState, repos: &mut RepositoryRuntimes, msg| {
        crate::store::reducer::reduce(repos, &id_alloc, state, msg)
    };
    let effects = reduce(&mut state, &mut repos, Msg::LoadWorktreeDirty { repo_id });
    assert!(matches!(
        effects.as_slice(),
        [Effect::LoadWorktreeDirty { .. }]
    ));
    assert!(
        reduce(&mut state, &mut repos, Msg::LoadWorktreeDirty { repo_id }).is_empty(),
        "a second refresh waits for the active scan to finish"
    );

    for fails in [true, false] {
        let (tx, rx) = std::sync::mpsc::channel();
        schedule_load_worktree_dirty(
            &worker.executor,
            backend.clone(),
            &repos,
            StoreWorkerSender::for_test_msg_sender(tx),
            repo_id,
            worker.runtime.worktree_cache(),
            own.clone(),
            WorktreeDirtyScope::All,
            Some(linked.clone()),
            CancellationToken::new(),
            worker.publisher.clone(),
        );
        loop {
            let msg = rx
                .recv_timeout(Duration::from_secs(5))
                .expect("even a panicking scan must send its completion");
            let finished =
                if let Msg::Internal(InternalMsg::WorktreeDirtyLoaded { result, .. }) = &msg {
                    assert_eq!(result.is_err(), fails, "{result:?}");
                    if let Err(error) = result {
                        assert!(error.to_string().contains("injected worktree scan panic"));
                    }
                    true
                } else {
                    assert!(matches!(
                        msg,
                        Msg::Internal(InternalMsg::WorktreesLoaded { .. })
                    ));
                    false
                };
            let effects = reduce(&mut state, &mut repos, msg);
            if finished {
                if fails {
                    assert!(matches!(
                        effects.as_slice(),
                        [Effect::LoadWorktreeDirty { .. }]
                    ));
                    assert_eq!(state.repos[0].worktree_dirty, previous_dirty);
                    assert_eq!(
                        state.repos[0].history_state.worktree_selection.as_ref(),
                        Some(&linked),
                        "an interrupted scan must preserve the selected worktree"
                    );
                    assert!(
                        cached_handle.upgrade().is_none(),
                        "a panicking scan must release its cached checkout handles"
                    );
                    let output = gitcomet_core::test_support::git_fixture::git_command()
                        .arg("-C")
                        .arg(&linked)
                        .args(["branch", "-m", "after-panic"])
                        .output()
                        .unwrap();
                    assert!(output.status.success(), "{output:?}");
                } else {
                    assert!(effects.is_empty(), "{effects:?}");
                    assert!(!state.repos[0].loads_in_flight.worktree_dirty_in_flight());
                }
                break;
            }
            assert!(effects.is_empty(), "{effects:?}");
        }
    }
    let dirty = state.repos[0].worktree_dirty.ready().unwrap();
    assert_eq!(dirty.len(), 2);
    let linked_dirty = dirty.iter().find(|summary| summary.path == linked).unwrap();
    assert_eq!(
        linked_dirty.unstaged.as_slice(),
        status(&["a.txt"]).unstaged.as_slice()
    );
    assert!(
        state.repos[0]
            .worktrees
            .ready()
            .unwrap()
            .iter()
            .any(|worktree| {
                worktree.path == linked && worktree.branch.as_deref() == Some("after-panic")
            })
    );
}

#[test]
fn scan_publishes_complete_worktree_list_before_opening_checkouts() {
    use crate::msg::InternalMsg;
    use crate::store::tests::repo_with_linked_worktree;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    struct BlockingBackend {
        started: mpsc::Sender<()>,
        release: Mutex<Option<mpsc::Receiver<()>>>,
        opens: AtomicUsize,
    }
    impl GitBackend for BlockingBackend {
        fn open(&self, path: &Path) -> Result<Arc<dyn GitRepository>, Error> {
            self.opens.fetch_add(1, Ordering::Relaxed);
            if let Some(release) = self.release.lock().unwrap().take() {
                self.started.send(()).unwrap();
                release.recv_timeout(Duration::from_secs(10)).unwrap();
            }
            gitcomet_git_gix::GixBackend.open(path)
        }
    }

    let (dir, own, _) = repo_with_linked_worktree();
    let missing = dir.path().join("missing");
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&own)
        .args(["worktree", "add", "--detach"])
        .arg(&missing)
        .output()
        .unwrap();
    assert!(output.status.success());
    std::fs::remove_dir_all(&missing).unwrap();
    let repo_id = RepoId(4105);
    let worker = ScanTestWorker::new();
    let repo = gitcomet_git_gix::GixBackend.open(&own).unwrap();
    let expected = repo
        .list_worktrees(&gitcomet_core::services::CancellationToken::new())
        .unwrap();
    let repos = RepositoryRuntimes::from_iter([(repo_id, repo)]);
    let (started, started_rx) = mpsc::channel();
    let (release_tx, release) = mpsc::channel();
    let backend = Arc::new(BlockingBackend {
        started,
        release: Mutex::new(Some(release)),
        opens: AtomicUsize::new(0),
    });
    let (tx, rx) = mpsc::channel();
    schedule_load_worktree_dirty(
        &worker.executor,
        backend.clone(),
        &repos,
        StoreWorkerSender::for_test_msg_sender(tx),
        repo_id,
        worker.runtime.worktree_cache(),
        own,
        WorktreeDirtyScope::All,
        None,
        CancellationToken::new(),
        worker.publisher.clone(),
    );
    started_rx.recv_timeout(Duration::from_secs(10)).unwrap();
    let first = rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let premature = rx.try_recv();
    release_tx.send(()).unwrap();
    match first {
        Msg::Internal(InternalMsg::WorktreesLoaded {
            read,
            result: Ok(list),
            ..
        }) => {
            assert!(!read.completes_load);
            assert_eq!(list.as_ref(), &expected);
            assert_eq!(
                list.len(),
                3,
                "own, clean, and missing checkouts all belong in the list"
            );
        }
        other => panic!("expected listing before status work, got {other:?}"),
    }
    assert!(matches!(premature, Err(mpsc::TryRecvError::Empty)));
    match rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Msg::Internal(InternalMsg::WorktreeDirtyLoaded {
            result: Ok(dirty), ..
        }) => assert!(dirty.is_empty()),
        other => panic!("expected clean dirty-summary result, got {other:?}"),
    }
    drop(worker);
    assert_eq!(
        backend.opens.load(Ordering::Relaxed),
        2,
        "each other checkout is opened only once"
    );
    assert!(
        rx.try_recv().is_err(),
        "reading the list must not queue more work"
    );
}

#[test]
fn targeted_worker_opens_only_the_requested_checkout_and_full_worker_scans_all() {
    use crate::msg::InternalMsg;
    use crate::store::tests::repo_with_linked_worktree;
    use std::time::Duration;
    struct CountingBackend {
        opened: Mutex<Vec<PathBuf>>,
    }
    impl GitBackend for CountingBackend {
        fn open(&self, path: &Path) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
            self.opened
                .lock()
                .unwrap()
                .push(canonicalize_or_original(path.to_path_buf()));
            gitcomet_git_gix::GixBackend.open(path)
        }
    }
    let (dir, own, selected) = repo_with_linked_worktree();
    let other = dir.path().join("other");
    let output = gitcomet_core::test_support::git_fixture::git_command()
        .arg("-C")
        .arg(&own)
        .args(["worktree", "add", "-q", "-b", "other"])
        .arg(&other)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let other = canonicalize_or_original(other);
    std::fs::write(selected.join("a.txt"), "selected checkout edit\n").unwrap();
    std::fs::write(other.join("a.txt"), "other checkout edit\n").unwrap();
    let repo_id = RepoId(4104);
    let repos = RepositoryRuntimes::from_iter([(
        repo_id,
        gitcomet_git_gix::GixBackend.open(&own).unwrap(),
    )]);
    let backend = Arc::new(CountingBackend {
        opened: Mutex::new(Vec::new()),
    });
    let worker = ScanTestWorker::new();
    let scan = |scope: WorktreeDirtyScope, files_for| {
        let (tx, rx) = std::sync::mpsc::channel();
        schedule_load_worktree_dirty(
            &worker.executor,
            backend.clone(),
            &repos,
            StoreWorkerSender::for_test_msg_sender(tx),
            repo_id,
            worker.runtime.worktree_cache(),
            own.clone(),
            scope.clone(),
            files_for,
            CancellationToken::new(),
            worker.publisher.clone(),
        );
        let mut listings = 0;
        loop {
            let msg = rx.recv_timeout(Duration::from_secs(5)).unwrap();
            if matches!(&msg, Msg::Internal(InternalMsg::WorktreesLoaded { .. })) {
                listings += 1;
            }
            if let Msg::Internal(InternalMsg::WorktreeDirtyLoaded {
                scope: returned,
                result,
                ..
            }) = msg
            {
                assert_eq!(returned, scope);
                assert_eq!(
                    listings,
                    usize::from(matches!(scope, WorktreeDirtyScope::All)),
                    "targeted file edits must not publish an extra state snapshot"
                );
                break result.unwrap();
            }
        }
    };
    let summaries = scan(
        WorktreeDirtyScope::Paths(vec![selected.clone()]),
        Some(selected.clone()),
    );
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].path, selected);
    assert_eq!(summaries[0].unstaged[0].path, PathBuf::from("a.txt"));
    assert_eq!(
        *backend.opened.lock().unwrap(),
        vec![selected.clone()],
        "unrelated checkouts must not be opened or scanned"
    );
    let summaries = scan(WorktreeDirtyScope::All, None);
    assert_eq!(summaries.len(), 2);
    assert!(summaries.iter().all(|summary| summary.unstaged.is_empty()));
    assert!(summaries.iter().any(|summary| summary.path == other));
    // A removed target returns a scoped empty result; it cannot erase other rows.
    let missing = dir.path().join("missing");
    assert!(scan(WorktreeDirtyScope::Paths(vec![missing]), None).is_empty());
    drop(worker);
}

/// The file lists are the expensive part of a summary -- an un-ignored build
/// directory puts tens of thousands of paths in them -- and only the selected
/// worktree's are ever rendered. The counts have to survive either way, since
/// every row shows them.
#[test]
fn only_the_selected_worktree_carries_its_files() {
    let kept = worktree_dirty_summary(
        worktree(),
        status(&["a.rs", "b.rs"]),
        true,
        Default::default(),
    );
    assert_eq!((kept.added, kept.modified, kept.deleted), (0, 2, 0));
    assert_eq!(
        kept.unstaged.len(),
        2,
        "the selected worktree keeps its files"
    );

    let counted = worktree_dirty_summary(
        worktree(),
        status(&["a.rs", "b.rs"]),
        false,
        Default::default(),
    );
    assert_eq!(
        (counted.added, counted.modified, counted.deleted),
        (0, 2, 0),
        "counts are derived before the lists are dropped"
    );
    assert!(
        counted.unstaged.is_empty() && counted.staged.is_empty(),
        "an unselected worktree carries counts alone"
    );
    assert!(
        counted.is_dirty(),
        "and still reports as dirty, or its row would disappear"
    );
}
