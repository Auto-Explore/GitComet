//! Errors: unavailable Git, missing handles, stale worktree routing.

use super::*;

#[test]
fn unavailable_git_effect_emits_synthetic_repo_command_error() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    let state = AppState {
        git_runtime: gitcomet_core::process::GitRuntimeState {
            preference: gitcomet_core::process::GitExecutablePreference::Custom(PathBuf::new()),
            availability: gitcomet_core::process::GitExecutableAvailability::Unavailable {
                detail: "Custom Git executable is not configured. Choose an executable or switch back to System PATH.".to_string(),
            },
        },
        ..AppState::test_default()
    };

    schedule_effect_with_state_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        state,
        msg_tx,
        Effect::FetchAll {
            repo_id: RepoId(7),
            prune: true,
            auth: None,
        },
    );

    let msg = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected synthetic unavailable-git message");
    match msg {
        Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
            repo_id,
            command,
            result,
        }) => {
            assert_eq!(repo_id, RepoId(7));
            assert_eq!(command, RepoCommandKind::FetchAll);
            let err = result.expect_err("expected unavailable-git failure");
            assert!(
                err.to_string()
                    .contains("Custom Git executable is not configured"),
                "unexpected error: {err}"
            );
        }
        other => panic!("unexpected message: {other:?}"),
    }
}

#[test]
fn unavailable_git_revert_emits_synthetic_revert_command_error() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    let state = AppState {
        git_runtime: gitcomet_core::process::GitRuntimeState {
            preference: gitcomet_core::process::GitExecutablePreference::Custom(PathBuf::new()),
            availability: gitcomet_core::process::GitExecutableAvailability::Unavailable {
                detail: "git missing".to_string(),
            },
        },
        ..AppState::test_default()
    };
    let commit_id = CommitId("deadbeef".into());

    schedule_effect_with_state_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        state,
        msg_tx,
        Effect::RevertCommit {
            repo_id: RepoId(7),
            commit_id: commit_id.clone(),
            commit: false,
            mainline: Some(1),
            summary: "revert me".into(),
            auth: None,
        },
    );

    let msg = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected synthetic unavailable-git message");
    let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
        repo_id,
        command,
        result,
    }) = msg
    else {
        panic!("unexpected message: {msg:?}");
    };
    assert_eq!(repo_id, RepoId(7));
    assert_eq!(
        command,
        RepoCommandKind::Revert {
            commit_id,
            commit: false,
            mainline: Some(1),
            summary: "revert me".into(),
        }
    );
    assert!(
        result
            .expect_err("unavailable git")
            .to_string()
            .contains("git missing")
    );
}

#[test]
fn worktree_and_submodule_effects_report_missing_repo_handle() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            panic!("open should not be called in this test")
        }
    }

    let repo_id = RepoId(77);
    let executor = super::super::executor::TaskExecutor::new(1);
    let backend: Arc<dyn GitBackend> = Arc::new(Backend);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx.clone(),
        Effect::LoadWorktrees { repo_id },
    );
    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::LoadSubmodules { repo_id },
    );

    let first = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected WorktreesLoaded");
    let second = msg_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("expected SubmodulesLoaded");

    match first {
        Msg::Internal(crate::msg::InternalMsg::WorktreesLoaded {
            repo_id: got_repo_id,
            result: Err(error),
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert!(
                matches!(error.kind(), ErrorKind::Backend(message) if message.contains("Repository handle not found"))
            );
        }
        _ => panic!("expected WorktreesLoaded missing-handle error"),
    }

    match second {
        Msg::Internal(crate::msg::InternalMsg::SubmodulesLoaded {
            repo_id: got_repo_id,
            result: Err(error),
        }) => {
            assert_eq!(got_repo_id, repo_id);
            assert!(
                matches!(error.kind(), ErrorKind::Backend(message) if message.contains("Repository handle not found"))
            );
        }
        _ => panic!("expected SubmodulesLoaded missing-handle error"),
    }
}

#[test]
fn branch_action_in_other_worktree_fails_when_it_no_longer_holds_the_branch() {
    let repo_id = RepoId(717);
    let fixture = worktree_redirect_fixture(repo_id, "gitcomet-redirect-moved-on", "main", false);
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::CreateBranchAndCheckout {
            repo_id,
            name: "feature".to_string(),
            target: "origin/feature-one".to_string(),
            force: true,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert!(
        others.is_empty(),
        "nothing to refresh after a failed guard: {others:?}"
    );
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id: id,
            action: RepoActionKind::CreateBranchAndCheckout,
            result: Err(_),
            ..
        }) if *id == repo_id
    ));
    assert!(fixture.origin_calls.lock().unwrap().is_empty());
    assert!(
        fixture.worktree_calls.lock().unwrap().is_empty(),
        "the overwrite must not run in a worktree that moved to another branch"
    );
}

#[test]
fn branch_action_in_other_worktree_fails_when_backend_opens_own_workdir() {
    let repo_id = RepoId(718);
    let fixture = worktree_redirect_fixture(repo_id, "gitcomet-redirect-self", "feature", true);
    let msg_rx = run_effect_with_fixture(
        &fixture,
        Effect::RenameBranch {
            repo_id,
            old_name: "old".to_string(),
            new_name: "feature".to_string(),
            force: true,
        },
    );

    let (others, finished) = recv_until_action_finished(&msg_rx);
    assert!(others.is_empty(), "unexpected messages: {others:?}");
    assert!(matches!(
        &finished,
        Msg::Internal(crate::msg::InternalMsg::RepoActionFinishedInWorktree {
            repo_id: id,
            action: RepoActionKind::RenameBranch,
            result: Err(_),
            ..
        }) if *id == repo_id
    ));
    assert!(fixture.origin_calls.lock().unwrap().is_empty());
    assert!(fixture.worktree_calls.lock().unwrap().is_empty());
}

#[test]
fn worktree_effect_fixture_waits_for_worker_completion() {
    struct BlockingBackend {
        inner: Arc<dyn GitBackend>,
        started: std::sync::mpsc::Sender<()>,
        release: Arc<(Mutex<bool>, Condvar)>,
    }

    impl GitBackend for BlockingBackend {
        fn open(&self, path: &Path) -> Result<Arc<dyn GitRepository>> {
            self.started.send(()).expect("backend started");
            let (lock, condvar) = &*self.release;
            let mut released = lock.lock().expect("release mutex");
            while !*released {
                released = condvar.wait(released).expect("release wait");
            }
            self.inner.open(path)
        }
    }

    let repo_id = RepoId(719);
    let mut fixture = worktree_redirect_fixture(repo_id, "gitcomet-redirect-join", "feature", true);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let release_guard = BlockingReleaseGuard {
        release: Arc::clone(&release),
    };
    fixture.backend = Arc::new(BlockingBackend {
        inner: Arc::clone(&fixture.backend),
        started: started_tx,
        release,
    });
    let (returned_tx, returned_rx) = std::sync::mpsc::channel();
    let runner = std::thread::spawn(move || {
        let msg_rx = run_effect_with_fixture(
            &fixture,
            Effect::RenameBranch {
                repo_id,
                old_name: "old".to_string(),
                new_name: "feature".to_string(),
                force: true,
            },
        );
        returned_tx.send(msg_rx).expect("fixture returned");
    });

    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("worktree backend should start");
    let early_return = returned_rx.recv_timeout(Duration::from_millis(100));
    drop(release_guard);
    let returned_early = early_return.is_ok();
    let msg_rx = early_return.unwrap_or_else(|error| {
        assert_eq!(error, std::sync::mpsc::RecvTimeoutError::Timeout);
        returned_rx
            .recv_timeout(Duration::from_secs(5))
            .expect("fixture should return after the worker finishes")
    });
    runner.join().expect("fixture runner panicked");
    recv_until_action_finished(&msg_rx);
    assert!(
        !returned_early,
        "the fixture must join its worker before returning"
    );
}
