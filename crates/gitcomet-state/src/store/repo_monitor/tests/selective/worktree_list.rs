use super::*;
use crate::model::{AppState, Loadable, RepoState};
use crate::store::AppStore;
use gitcomet_core::domain::Worktree;

/// Real native notifications go through the store's reducer and workers. Own
/// monitor readiness explicitly so startup loads cannot satisfy the assertions.
struct LiveWorktrees {
    monitor: RunningMonitor,
    store: AppStore,
    _events: smol::channel::Receiver<crate::msg::StoreEvent>,
}

impl LiveWorktrees {
    fn new(workdir: &Path) -> Self {
        let backend = Arc::new(gitcomet_git_gix::GixBackend);
        let repo = backend.open(workdir).unwrap();
        let mut repo_state = RepoState::new_opening(RepoId(1), repo.spec().clone());
        repo_state.set_open(Loadable::Ready(()));
        repo_state.set_worktrees(Loadable::Ready(
            repo.list_worktrees(&gitcomet_core::services::CancellationToken::new())
                .unwrap(),
        ));
        let state = AppState {
            active_repo: Some(RepoId(1)),
            repos: vec![repo_state],
            ..AppState::test_default()
        };
        let (store, events) = AppStore::new_test(backend);
        // Only the synchronized monitor below feeds native events to this store.
        store.disable_repo_monitors_for_test();
        store.replace_snapshot_for_test(Arc::new(state));
        store.insert_repo_for_test(RepoId(1), repo);
        let monitor = RunningMonitor::start(workdir);
        Self {
            monitor,
            store,
            _events: events,
        }
    }

    #[track_caller]
    fn change(&self, action: impl FnOnce(), expected: impl Fn(&[Worktree]) -> bool) {
        action();
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            match self.monitor.rx.recv_timeout(Duration::from_millis(10)) {
                Ok(message) => self.store.dispatch(message),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => panic!("monitor stopped: {error}"),
            }
            let state = self.store.snapshot();
            if state.repos[0]
                .worktrees
                .ready()
                .is_some_and(|list| expected(list))
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "worktree list did not update: {:?}",
                state.repos[0].worktrees
            );
        }
    }
}

impl Drop for LiveWorktrees {
    fn drop(&mut self) {
        let result = self
            .store
            .shutdown_for_test()
            .recv_timeout(Duration::from_secs(10));
        if !std::thread::panicking() {
            result.expect("worktree test store must stop");
        }
    }
}

fn external_worktree_list_lifecycle(observe_from_linked: bool) {
    let (_temp, main) = repository();
    let outside = unique_temp_dir("gitcomet-worktree-list");
    let outside = normalized(&outside.path().canonicalize().unwrap());
    let observer = if observe_from_linked {
        let observer = outside.join("observer");
        run_git(
            &main,
            &["worktree", "add", "--detach", observer.to_str().unwrap()],
        );
        observer
    } else {
        // In this case no worktrees directory exists when watching starts.
        assert!(!main.join(".git/worktrees").exists());
        main.clone()
    };
    let live = LiveWorktrees::new(&observer);
    let initial_count = if observe_from_linked { 2 } else { 1 };
    let linked = outside.join("external");
    live.change(
        || {
            run_git(
                &main,
                &["worktree", "add", "--detach", linked.to_str().unwrap()],
            )
        },
        |list| {
            list.len() == initial_count + 1 && list.iter().any(|w| w.path == linked && w.detached)
        },
    );
    live.change(
        || run_git(&linked, &["checkout", "-b", "external-one"]),
        |list| {
            list.iter().any(|w| {
                w.path == linked && w.branch.as_deref() == Some("external-one") && !w.detached
            })
        },
    );
    let old_head = live.store.snapshot().repos[0]
        .worktrees
        .ready()
        .unwrap()
        .iter()
        .find(|w| w.path == linked)
        .unwrap()
        .head
        .clone();
    live.change(
        || {
            run_git(
                &linked,
                &["commit", "--allow-empty", "-m", "external commit"],
            )
        },
        |list| list.iter().any(|w| w.path == linked && w.head != old_head),
    );
    live.change(
        || run_git(&linked, &["branch", "-m", "external-two"]),
        |list| {
            list.iter()
                .any(|w| w.path == linked && w.branch.as_deref() == Some("external-two"))
        },
    );
    live.change(
        || run_git(&linked, &["checkout", "--detach", "HEAD~1"]),
        |list| {
            list.iter()
                .any(|w| w.path == linked && w.detached && w.branch.is_none() && w.head == old_head)
        },
    );
    let moved = outside.join("moved");
    live.change(
        || {
            run_git(
                &main,
                &[
                    "worktree",
                    "move",
                    linked.to_str().unwrap(),
                    moved.to_str().unwrap(),
                ],
            )
        },
        |list| list.iter().any(|w| w.path == moved) && list.iter().all(|w| w.path != linked),
    );
    live.change(
        || {
            fs::rename(&moved, &linked).unwrap();
            run_git(&main, &["worktree", "repair", linked.to_str().unwrap()]);
        },
        |list| list.iter().any(|w| w.path == linked) && list.iter().all(|w| w.path != moved),
    );
    live.change(
        || run_git(&main, &["worktree", "remove", linked.to_str().unwrap()]),
        |list| list.len() == initial_count,
    );
    live.change(
        || {
            run_git(
                &main,
                &["worktree", "add", "--detach", linked.to_str().unwrap()],
            )
        },
        |list| list.len() == initial_count + 1,
    );
    live.change(
        || {
            fs::remove_dir_all(&linked).unwrap();
            run_git(&main, &["worktree", "prune", "--expire", "now"]);
        },
        |list| list.len() == initial_count,
    );
    // Listing and status reads must not sustain another notification cycle.
    live.monitor.settle();
    live.monitor.quiet();
}

#[test]
fn external_worktree_list_updates_from_main_checkout() {
    external_worktree_list_lifecycle(false);
}

#[test]
fn external_worktree_list_updates_from_linked_checkout() {
    external_worktree_list_lifecycle(true);
}
