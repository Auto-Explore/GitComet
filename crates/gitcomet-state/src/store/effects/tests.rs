use super::*;

#[test]
fn selecting_a_new_diff_cancels_only_the_previous_diff() {
    let mut token = RepoTaskToken::new(1);
    let a = DiffTarget::working_tree("a".into(), gitcomet_core::domain::DiffArea::Unstaged);
    let b = DiffTarget::working_tree("b".into(), gitcomet_core::domain::DiffArea::Unstaged);
    let first = token.selected_diff_cancellation(&a, 1);
    let same = token.selected_diff_cancellation(&a, 1);
    // Unrelated store messages must not acquire a task's mutex. Holding
    // the shared slot here would deadlock the old per-message check.
    let slot = token.selected_diff.clone();
    let held = slot.lock().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    let same_target = a.clone();
    let worker = std::thread::spawn(move || {
        token.cancel_stale_selected_diff(Some((&same_target, 1)));
        let _ = tx.send(token);
    });
    let unlocked = rx.recv_timeout(std::time::Duration::from_secs(2));
    drop(held);
    worker.join().unwrap();
    let mut token = unlocked.expect("unchanged selection must not lock the task slot");
    assert!(!first.is_cancelled());
    token.cancel_stale_selected_diff(Some((&a, 2)));
    assert!(
        first.is_cancelled(),
        "refreshing the same path cancels the old revision"
    );
    let log = token.take_over_log();
    let next = token.selected_diff_cancellation(&b, 2);
    assert!(first.is_cancelled());
    assert!(same.is_cancelled());
    assert!(!next.is_cancelled());
    assert!(!token.cancellation.is_cancelled());
    assert!(!log.is_cancelled());
    token.cancel_stale_selected_diff(None);
    assert!(next.is_cancelled());
    let last = token.selected_diff_cancellation(&a, 3);
    token.cancel();
    assert!(last.is_cancelled());
    assert!(log.is_cancelled());
}

/// Status polls, progress events and log pages all pass through the store
/// worker. With no selected-diff load in flight there is nothing to cancel,
/// so they must not index every repo's selection.
#[test]
fn messages_without_selected_diff_work_skip_the_selection_index() {
    use super::super::{RepoMonitorManager, WorkerLoopContext, selection_index_builds_for_test};
    use crate::model::RepoState;
    use gitcomet_core::domain::{DiffArea, RepoSpec};

    struct NoBackend;
    impl GitBackend for NoBackend {
        fn open(
            &self,
            _: &std::path::Path,
        ) -> gitcomet_core::services::Result<Arc<dyn GitRepository>> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let target = DiffTarget::working_tree("a".into(), DiffArea::Unstaged);
    let ids = [RepoId(1), RepoId(2), RepoId(3)];
    let repos_state = ids
        .iter()
        .map(|&id| {
            let workdir = format!("/tmp/gitcomet-selection-index-{}", id.0).into();
            let mut repo = RepoState::new_opening(id, RepoSpec { workdir });
            repo.diff_state.diff_target = Some(target.clone());
            repo.diff_state.diff_target_rev = 1;
            repo
        })
        .collect();
    let thread_state = Arc::new(RwLock::new(Arc::new(AppState {
        repos: repos_state,
        ..AppState::test_default()
    })));
    let active_repo_id = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let (event_tx, _event_rx) = smol::channel::bounded(1);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let thread_msg_tx = StoreWorkerSender::for_test_msg_sender(msg_tx);
    let executor = TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(NoBackend);
    let mut repo_monitors = RepoMonitorManager::new();
    let mut repo_task_tokens: FxHashMap<RepoId, RepoTaskToken> =
        ids.iter().map(|&id| (id, RepoTaskToken::new(0))).collect();
    let mut repos = FxHashMap::default();
    let id_alloc = std::sync::atomic::AtomicU64::new(1);
    let mut ctx = WorkerLoopContext {
        thread_state: &thread_state,
        active_repo_id: &active_repo_id,
        event_tx: &event_tx,
        repo_monitors: &mut repo_monitors,
        repo_task_tokens: &mut repo_task_tokens,
        thread_msg_tx: &thread_msg_tx,
        executor: &executor,
        repo_load_executor: &executor,
        worktree_scan_executor: &std::sync::LazyLock::new(|| TaskExecutor::new(1)),
        metadata_executor: &executor,
        signature_executor: &executor,
        history_find_executor: &std::sync::LazyLock::new(|| TaskExecutor::new(1)),
        session_persist_executor: &executor,
        backend: &backend,
        publication: &std::sync::atomic::AtomicU64::new(0),
    };

    let builds = selection_index_builds_for_test();
    ctx.reduce_and_handle(&mut repos, &id_alloc, |_, _, _| Vec::<Effect>::new());
    assert_eq!(
        selection_index_builds_for_test(),
        builds,
        "no task token holds selected-diff work"
    );

    // Tokens holding work are still checked against the new state.
    let refreshed = ctx
        .repo_task_tokens
        .get_mut(&RepoId(2))
        .unwrap()
        .selected_diff_cancellation(&target, 1);
    let closed = ctx
        .repo_task_tokens
        .get_mut(&RepoId(3))
        .unwrap()
        .selected_diff_cancellation(&target, 1);
    ctx.reduce_and_handle(&mut repos, &id_alloc, |_, _, _| Vec::<Effect>::new());
    assert!(!refreshed.is_cancelled() && !closed.is_cancelled());
    ctx.reduce_and_handle(&mut repos, &id_alloc, |state, _, _| {
        state.repos[1].diff_state.diff_target_rev = 2;
        state.repos.remove(2);
        Vec::<Effect>::new()
    });
    assert!(refreshed.is_cancelled(), "a new revision cancels the load");
    assert!(closed.is_cancelled(), "a closed repo cancels the load");

    // Cancelling released the work, so the index is skipped again.
    let builds = selection_index_builds_for_test();
    ctx.reduce_and_handle(&mut repos, &id_alloc, |_, _, _| Vec::<Effect>::new());
    assert_eq!(selection_index_builds_for_test(), builds);
}

#[test]
fn abort_clone_repo_does_not_require_available_git() {
    assert!(!effect_requires_available_git(&Effect::AbortCloneRepo {
        dest: std::path::PathBuf::from("/tmp/example"),
    }));
}

#[test]
fn cancel_git_operation_does_not_require_available_git() {
    assert!(!effect_requires_available_git(
        &Effect::CancelGitOperation {
            repo_id: RepoId(1),
            operation_id: gitcomet_core::git_operation::GitOperationId(7),
        }
    ));
}
