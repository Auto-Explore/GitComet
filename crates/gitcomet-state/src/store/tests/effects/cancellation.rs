//! Cancellation: aborted loads and actions release their workers.

use super::*;

#[test]
fn signature_work_survives_repo_load_cancellation_and_does_not_use_primary_workers() {
    for cancel_repo_loads in [false, true] {
        let primary = super::super::executor::TaskExecutor::new(1);
        let signatures = super::super::executor::TaskExecutor::new(1);
        let (release_primary, wait_primary) = std::sync::mpsc::channel::<()>();
        primary.spawn(move || {
            let _ = wait_primary.recv();
        });
        let (release_signatures, wait_signatures) = std::sync::mpsc::channel::<()>();
        signatures.spawn(move || {
            let _ = wait_signatures.recv();
        });
        let executors = super::super::effects::EffectExecutors {
            executor: &primary,
            repo_load_executor: &primary,
            worktree_scan_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            metadata_executor: &primary,
            session_persist_executor: &primary,
            signature_executor: &signatures,
            history_find_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
        };
        let repo_id = RepoId(1);
        let spec = RepoSpec {
            workdir: PathBuf::from("/tmp/signature-scheduling"),
        };
        let mut state = AppState::test_default();
        state
            .repos
            .push(RepoState::new_opening(repo_id, spec.clone()));
        let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
        let repos = [(
            repo_id,
            Arc::new(DummyRepo::new(spec.workdir)) as Arc<dyn GitRepository>,
        )]
        .into_iter()
        .collect();
        let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
        let (msg_tx, msg_rx) = std::sync::mpsc::channel();
        let sender = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
        let mut tokens = FxHashMap::default();
        super::super::effects::schedule_effect(
            executors,
            &thread_state,
            &backend,
            &repos,
            &mut tokens,
            sender.clone(),
            Effect::VerifyCommitSignatures {
                repo_id,
                epoch: 0,
                batch: 1,
                cancellation: CancellationToken::new(),
                commit_ids: vec![CommitId("aaaa".into())].into(),
                formats: gitcomet_core::domain::SignatureFormats::ALL,
            },
        );
        if cancel_repo_loads {
            super::super::effects::schedule_effect(
                executors,
                &thread_state,
                &backend,
                &repos,
                &mut tokens,
                sender.clone(),
                Effect::LoadHeadBranch { repo_id },
            );
            assert!(
                tokens.contains_key(&repo_id),
                "a repo load must actually be pending"
            );
            super::super::effects::schedule_effect(
                executors,
                &thread_state,
                &backend,
                &repos,
                &mut tokens,
                sender,
                Effect::CancelRepoLoads {
                    repo_id,
                    load_epoch: 0,
                },
            );
        }
        drop(release_signatures);
        // Keep the primary worker occupied: optional verification has its own worker.
        let reply = recv_effect_message(&msg_rx, Duration::from_secs(1))
            .expect("verification reply must arrive");
        assert!(matches!(
            reply,
            Msg::Internal(crate::msg::InternalMsg::CommitSignaturesVerified {
                repo_id: RepoId(1),
                epoch: 0,
                ..
            })
        ));
        drop(release_primary);
    }
}

fn write_deterministic_blob(path: &Path, total_bytes: usize) {
    use std::io::Write as _;

    let mut file = std::fs::File::create(path).expect("blob file should be creatable");
    let mut remaining = total_bytes;
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    let mut buf = [0u8; 8192];

    while remaining > 0 {
        for byte in &mut buf {
            state ^= state << 7;
            state ^= state >> 9;
            state = state.wrapping_mul(0x2545_f491_4f6c_dd1d);
            *byte = (state >> 24) as u8;
        }

        let chunk_len = remaining.min(buf.len());
        file.write_all(&buf[..chunk_len])
            .expect("blob chunk should be writable");
        remaining -= chunk_len;
    }
}

fn local_file_url(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('\\', "/");
    if normalized.starts_with('/') {
        format!("file://{normalized}")
    } else {
        format!("file:///{normalized}")
    }
}

#[test]
fn clone_repo_effect_abort_removes_partially_created_destination() {
    struct Backend;
    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            Err(Error::new(ErrorKind::Unsupported("test backend")))
        }
    }

    const LARGE_BLOB_BYTES: usize = 64 * 1024 * 1024;

    let temp = tempfile::tempdir().expect("tempdir");
    let src = temp.path().join("src");
    let dest = temp.path().join("dest");
    std::fs::create_dir_all(&src).expect("source dir");

    run_git(&src, &["init"]);
    run_git(&src, &["config", "user.email", "you@example.com"]);
    run_git(&src, &["config", "user.name", "You"]);
    run_git(&src, &["config", "commit.gpgsign", "false"]);
    write_deterministic_blob(&src.join("payload.bin"), LARGE_BLOB_BYTES);
    run_git(&src, &["add", "payload.bin"]);
    run_git(
        &src,
        &["-c", "commit.gpgsign=false", "commit", "-m", "init"],
    );

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
        Effect::CloneRepo {
            url: local_file_url(&src),
            dest: dest.clone(),
            remote_url_policy: Default::default(),
            auth: None,
        },
    );

    let start = Instant::now();
    let mut abort_sent = false;
    let mut saw_finished_err = false;
    let mut saw_open_repo = false;

    while start.elapsed() < Duration::from_secs(30) {
        if !abort_sent && dest.exists() {
            schedule_effect_for_test(
                &executor,
                &executor,
                &backend,
                &repos,
                msg_tx.clone(),
                Effect::AbortCloneRepo { dest: dest.clone() },
            );
            abort_sent = true;
        }

        let msg = match msg_rx.recv_timeout(Duration::from_millis(10)) {
            Ok(m) => m,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => panic!("channel closed: {e:?}"),
        };

        match msg {
            Msg::Internal(crate::msg::InternalMsg::CloneRepoFinished {
                dest: finished_dest,
                result,
                ..
            }) if finished_dest == dest => {
                assert!(abort_sent, "clone finished before abort could be sent");
                let err = result.expect_err("aborted clone should not succeed");
                let err_text = err.to_string();
                assert!(
                    err_text.contains("clone aborted"),
                    "unexpected abort error: {err_text}"
                );
                saw_finished_err = true;
                break;
            }
            Msg::OpenRepo(path) if path == dest => {
                saw_open_repo = true;
            }
            _ => {}
        }
    }

    assert!(abort_sent, "did not send abort request");
    assert!(saw_finished_err, "did not observe CloneRepoFinished error");
    assert!(
        !saw_open_repo,
        "aborted clone should not open the repository"
    );
    assert!(
        !dest.exists(),
        "aborted clone should clean up the destination directory"
    );
}

#[test]
fn delete_branches_effect_stops_batch_and_preserves_cancellation() {
    let repo_id = RepoId(341);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let repo: Arc<dyn GitRepository> = Arc::new(UnsupportedRepo {
        spec: RepoSpec {
            workdir: unique_temp_path("gitcomet-delete-branches-cancelled"),
        },
        delete_branch_calls: Some(Arc::clone(&calls)),
        cancel_delete_branch: Some("cancel-here".to_string()),
    });
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(repo_id, repo);
        repos
    };
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();

    schedule_effect_for_test(
        &executor,
        &executor,
        &backend,
        &repos,
        msg_tx,
        Effect::DeleteBranches {
            repo_id,
            names: vec!["cancel-here".to_string(), "must-not-run".to_string()],
            force: false,
        },
    );

    let error = loop {
        match recv_effect_message(&msg_rx, Duration::from_secs(2))
            .expect("batch delete should finish")
        {
            Msg::Internal(crate::msg::InternalMsg::RepoActionFinished {
                repo_id: finished_repo_id,
                action: RepoActionKind::DeleteBranches,
                result,
            }) if finished_repo_id == repo_id => {
                break result.expect_err("cancellation should fail the operation");
            }
            Msg::RefreshBranches {
                repo_id: refreshed_repo_id,
            } if refreshed_repo_id == repo_id => {}
            other => panic!("unexpected batch-delete message: {other:?}"),
        }
    };

    assert!(
        matches!(error.kind(), ErrorKind::Cancelled),
        "cancellation must not be wrapped as a batch failure: {error:?}"
    );
    assert_eq!(
        *calls.lock().expect("delete branch recording mutex"),
        vec!["cancel-here".to_string()],
        "no later destructive delete may run after Stop"
    );
}

#[test]
fn open_repo_effect_suppresses_result_after_cancellation() {
    use std::sync::{Condvar, Mutex};

    struct Backend {
        started_tx: std::sync::mpsc::Sender<()>,
        release: Arc<(Mutex<bool>, Condvar)>,
        repo: Arc<dyn GitRepository>,
    }

    impl GitBackend for Backend {
        fn open(&self, _path: &Path) -> std::result::Result<Arc<dyn GitRepository>, Error> {
            let _ = self.started_tx.send(());
            let (lock, condvar) = &*self.release;
            let mut released = lock.lock().expect("release mutex");
            while !*released {
                released = condvar.wait(released).expect("release condvar");
            }
            Ok(Arc::clone(&self.repo))
        }
    }

    let repo_id = RepoId(44);
    let workdir = unique_temp_path("gitcomet-open-repo-cancelled");
    let repo: Arc<dyn GitRepository> = Arc::new(UnsupportedRepo {
        spec: RepoSpec {
            workdir: workdir.clone(),
        },
        delete_branch_calls: None,
        cancel_delete_branch: None,
    });
    let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let backend: Arc<dyn GitBackend> = Arc::new(Backend {
        started_tx,
        release: Arc::clone(&release),
        repo,
    });

    let executor = super::super::executor::TaskExecutor::new(1);
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = FxHashMap::default();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        repo_id,
        RepoSpec {
            workdir: workdir.clone(),
        },
    ));
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    let mut repo_task_tokens = FxHashMap::default();
    let repo_load_executor = super::super::executor::TaskExecutor::new(1);
    let metadata_executor = super::super::executor::TaskExecutor::new(1);

    super::super::effects::schedule_effect(
        super::super::effects::EffectExecutors {
            executor: &executor,
            repo_load_executor: &repo_load_executor,
            worktree_scan_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            session_persist_executor: &executor,
            metadata_executor: &metadata_executor,
            signature_executor: &metadata_executor,
            history_find_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
        },
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx.clone(),
        Effect::OpenRepo {
            repo_id,
            path: workdir,
        },
    );
    started_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("open effect did not start");

    super::super::effects::schedule_effect(
        super::super::effects::EffectExecutors {
            executor: &executor,
            repo_load_executor: &repo_load_executor,
            worktree_scan_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
            session_persist_executor: &executor,
            metadata_executor: &metadata_executor,
            signature_executor: &metadata_executor,
            history_find_executor: &std::sync::LazyLock::new(|| {
                super::super::executor::TaskExecutor::new(1)
            }),
        },
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx,
        Effect::CancelRepoLoads {
            repo_id,
            load_epoch: 0,
        },
    );
    {
        let (lock, condvar) = &*release;
        let mut released = lock.lock().expect("release mutex");
        *released = true;
        condvar.notify_all();
    }

    assert!(
        msg_rx.recv_timeout(Duration::from_millis(200)).is_err(),
        "cancelled open effect should not emit a result"
    );
}

#[test]
fn cancelled_selected_diff_does_not_keep_executor_busy_for_next_repo() {
    let repo_a = RepoId(520);
    let repo_b = RepoId(521);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let _release_guard = BlockingReleaseGuard {
        release: Arc::clone(&release),
    };
    let (started_tx, started_rx) = std::sync::mpsc::channel::<RepoId>();
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(
            repo_a,
            Arc::new(SelectedDiffSchedulingRepo {
                spec: RepoSpec {
                    workdir: unique_temp_path("gitcomet-selected-diff-blocking-a"),
                },
                mode: SelectedDiffRepoMode::BlockingDiff,
                started_tx: started_tx.clone(),
                started_repo_id: repo_a,
                release: Arc::clone(&release),
            }) as Arc<dyn GitRepository>,
        );
        repos.insert(
            repo_b,
            Arc::new(SelectedDiffSchedulingRepo {
                spec: RepoSpec {
                    workdir: unique_temp_path("gitcomet-selected-diff-ready-b"),
                },
                mode: SelectedDiffRepoMode::ReadyDiff,
                started_tx,
                started_repo_id: repo_b,
                release: Arc::clone(&release),
            }) as Arc<dyn GitRepository>,
        );
        repos
    };
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let executor = super::super::executor::TaskExecutor::new(1);
    let repo_load_executor = super::super::executor::TaskExecutor::new(1);
    let metadata_executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let target_a = DiffTarget::working_tree(PathBuf::from("repo-a.txt"), DiffArea::Unstaged);
    let target_b = DiffTarget::working_tree(PathBuf::from("repo-b.txt"), DiffArea::Unstaged);
    let mut state = AppState::test_default();
    let mut repo_state_a = RepoState::new_opening(
        repo_a,
        RepoSpec {
            workdir: unique_temp_path("gitcomet-selected-diff-state-a"),
        },
    );
    repo_state_a.diff_state.diff_target = Some(target_a.clone());
    let mut repo_state_b = RepoState::new_opening(
        repo_b,
        RepoSpec {
            workdir: unique_temp_path("gitcomet-selected-diff-state-b"),
        },
    );
    repo_state_b.diff_state.diff_target = Some(target_b.clone());
    state.repos.push(repo_state_a);
    state.repos.push(repo_state_b);
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    let mut repo_task_tokens = FxHashMap::default();
    let executors = super::super::effects::EffectExecutors {
        executor: &executor,
        repo_load_executor: &repo_load_executor,
        worktree_scan_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
        session_persist_executor: &executor,
        metadata_executor: &metadata_executor,
        signature_executor: &metadata_executor,
        history_find_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
    };

    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx.clone(),
        Effect::LoadSelectedDiff {
            repo_id: repo_a,
            load_patch_diff: true,
            load_file_text: false,
            preview_text_side: None,
            load_submodule_summary: false,
            load_file_image: false,
        },
    );
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("repo A diff task did not start"),
        repo_a
    );

    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx.clone(),
        Effect::CancelRepoLoads {
            repo_id: repo_a,
            load_epoch: 0,
        },
    );
    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx,
        Effect::LoadSelectedDiff {
            repo_id: repo_b,
            load_patch_diff: true,
            load_file_text: false,
            preview_text_side: None,
            load_submodule_summary: false,
            load_file_image: false,
        },
    );

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_millis(200))
            .expect("repo B diff should start once repo A load epoch is cancelled"),
        repo_b
    );
}

/// The line-stats scan reads every changed file, so a superseded one must stop
/// when its repository's loads are cancelled -- otherwise it holds the single
/// repo-load worker and every queued load behind it waits for a result nobody
/// will use.
#[test]
fn cancelled_uncommitted_line_stats_frees_the_repo_load_executor() {
    let snapshot = Arc::new(RepoStatus {
        staged: Default::default(),
        unstaged: Arc::new(vec![gitcomet_core::domain::FileStatus {
            path: PathBuf::from("snapshot-only.txt"),
            kind: gitcomet_core::domain::FileStatusKind::Modified,
            conflict: None,
        }]),
    });
    let repo_a = RepoId(530);
    let repo_b = RepoId(531);
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let _release_guard = BlockingReleaseGuard {
        release: Arc::clone(&release),
    };
    let (started_tx, started_rx) = std::sync::mpsc::channel::<RepoId>();
    let repos: FxHashMap<RepoId, Arc<dyn GitRepository>> = {
        let mut repos = FxHashMap::default();
        repos.insert(
            repo_a,
            Arc::new(SelectedDiffSchedulingRepo {
                spec: RepoSpec {
                    workdir: unique_temp_path("gitcomet-line-stats-blocking-a"),
                },
                mode: SelectedDiffRepoMode::BlockingDiff,
                started_tx: started_tx.clone(),
                started_repo_id: repo_a,
                release: Arc::clone(&release),
            }) as Arc<dyn GitRepository>,
        );
        repos.insert(
            repo_b,
            Arc::new(SelectedDiffSchedulingRepo {
                spec: RepoSpec {
                    workdir: unique_temp_path("gitcomet-line-stats-ready-b"),
                },
                mode: SelectedDiffRepoMode::ReadyDiff,
                started_tx,
                started_repo_id: repo_b,
                release: Arc::clone(&release),
            }) as Arc<dyn GitRepository>,
        );
        repos
    };
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let executor = super::super::executor::TaskExecutor::new(1);
    let repo_load_executor = super::super::executor::TaskExecutor::new(1);
    let metadata_executor = super::super::executor::TaskExecutor::new(1);
    let (msg_tx, _msg_rx) = std::sync::mpsc::channel::<Msg>();
    let msg_tx = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(msg_tx);
    let mut state = AppState::test_default();
    state.repos.push(RepoState::new_opening(
        repo_a,
        RepoSpec {
            workdir: unique_temp_path("gitcomet-line-stats-state-a"),
        },
    ));
    state.repos.push(RepoState::new_opening(
        repo_b,
        RepoSpec {
            workdir: unique_temp_path("gitcomet-line-stats-state-b"),
        },
    ));
    let thread_state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    let mut repo_task_tokens = FxHashMap::default();
    let executors = super::super::effects::EffectExecutors {
        executor: &executor,
        repo_load_executor: &repo_load_executor,
        worktree_scan_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
        session_persist_executor: &executor,
        metadata_executor: &metadata_executor,
        signature_executor: &metadata_executor,
        history_find_executor: &std::sync::LazyLock::new(|| {
            super::super::executor::TaskExecutor::new(1)
        }),
    };

    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx.clone(),
        Effect::LoadUncommittedLineStats {
            repo_id: repo_a,
            generation: 1,
            status: Arc::clone(&snapshot),
            large_files: false,
        },
    );
    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("repo A line-stats task did not start"),
        repo_a
    );

    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx.clone(),
        Effect::CancelRepoLoads {
            repo_id: repo_a,
            load_epoch: 0,
        },
    );
    super::super::effects::schedule_effect(
        executors,
        &thread_state,
        &backend,
        &repos,
        &mut repo_task_tokens,
        msg_tx,
        Effect::LoadUncommittedLineStats {
            repo_id: repo_b,
            generation: 1,
            status: snapshot,
            large_files: false,
        },
    );

    assert_eq!(
        started_rx
            .recv_timeout(Duration::from_millis(500))
            .expect("repo B line stats should start once repo A load epoch is cancelled"),
        repo_b
    );
}

/// A repo action dispatched while the worker holds no handle for the repo (the
/// tab is still opening, or the open failed) must still complete, or the
/// in-flight counter that disables the stage/unstage controls never releases.
#[test]
fn repo_action_without_open_handle_releases_in_flight_counter() {
    let backend: Arc<dyn GitBackend> = Arc::new(FailingBackend);
    let (store, _event_rx) = AppStore::new_test(backend);
    let repo_id = RepoId(1);
    let spec = RepoSpec {
        workdir: PathBuf::from("/tmp/gitcomet-missing-handle"),
    };
    let mut state = AppState {
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };
    state.repos.push(RepoState::new_opening(repo_id, spec));
    store.replace_snapshot_for_test(Arc::new(state));

    store.dispatch(Msg::StagePaths {
        repo_id,
        paths: vec![PathBuf::from("a.txt")].into(),
    });

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let repo = loop {
        let snapshot = store.snapshot();
        let repo = snapshot.repos.first().expect("the injected repo");
        // The begin bumps `ops_rev` once; the completion bumps it again.
        if repo.ops_rev >= 2 {
            break repo.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the action never completed: in flight = {}, ops_rev = {}",
            repo.local_actions_in_flight,
            repo.ops_rev
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert_eq!(repo.local_actions_in_flight, 0);
    assert!(
        repo.feedback.last_error.is_some(),
        "the missing handle must surface as an action failure"
    );
}
