use super::*;
use gitcomet_core::patch_action::{DiffActionSnapshot, PatchOperation, SourceLineSelection};
use std::collections::BTreeSet;

#[derive(Clone, Copy)]
enum QueuedPatchViewChange {
    Unchanged,
    Retarget,
    Close,
}

fn run_queued_patch_with_view_change(
    change: QueuedPatchViewChange,
) -> (Vec<String>, Result<CommandOutput>) {
    use crate::diff_session::{DiffSession, DiffSessionMsg, DiffViewId};

    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (apply_started, _apply_started_rx) = std::sync::mpsc::channel();
    let mut repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: dir.path().into(),
        },
        calls.clone(),
    );
    repo.prepared_barrier = Some(PreparedPatchBarrier {
        started: apply_started,
        release: Arc::new((Mutex::new(true), Condvar::new())),
    });
    let repo: Arc<dyn GitRepository> = Arc::new(repo);
    let repo_id = RepoId(9543);
    let view = DiffViewId(9543);
    let target = DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged);
    let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
    repo_state.open = Loadable::Ready(());
    let lifetime = repo_state.lifetime();
    let mut session = DiffSession::new(target.clone());
    session.generation = 3;
    session.diff_state.diff_file_rev = 7;
    Arc::make_mut(&mut repo_state.diff_sessions).insert(view, session);
    let mut repos = RepositoryRuntimes::default();
    repos.opened(repo_id, repo_state.lifetime_id(), repo);
    let mut state = AppState {
        repos: vec![repo_state],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    };
    let prepared = gitcomet_core::patch_action::prepare(
        DiffActionSnapshot {
            diff: gitcomet_core::domain::Diff::from_unified(
                target,
                "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new\n",
            ),
            old: "old\n".into(),
            new: "new\n".into(),
            policy: Arc::from([]),
        },
        PatchOperation::Discard,
        &SourceLineSelection {
            old: BTreeSet::from([1]),
            new: BTreeSet::from([1]),
        },
        "old\n",
        "new\n",
    )
    .unwrap();
    let id_alloc = std::sync::atomic::AtomicU64::new(9544);
    let mut effects = reduce(
        &mut repos,
        &id_alloc,
        &mut state,
        Msg::ApplyPreparedDiffPatch {
            repo_id,
            lifetime,
            view: Some(view),
            file_rev: 7,
            generation: 3,
            prepared: Arc::new(prepared),
        },
    );
    assert_eq!(effects.len(), 1);
    assert_eq!(state.repos[0].local_actions_in_flight, 1);
    let effect = effects.pop().unwrap();
    let state = Arc::new(std::sync::RwLock::new(Arc::new(state)));
    let executor = super::super::executor::TaskExecutor::new(1);
    let queue_release = Arc::new((Mutex::new(false), Condvar::new()));
    let release_guard = BlockingReleaseGuard {
        release: queue_release.clone(),
    };
    let (queued, queued_rx) = std::sync::mpsc::channel();
    executor.spawn(move || {
        queued.send(()).unwrap();
        wait_for_release_signal(&queue_release);
    });
    queued_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let filesystem_executor: std::sync::LazyLock<super::super::executor::TaskExecutor> =
        std::sync::LazyLock::new(super::super::executor::filesystem_queue);
    let scan_executor: std::sync::LazyLock<super::super::executor::TaskExecutor> =
        std::sync::LazyLock::new(|| super::super::executor::TaskExecutor::new(1));
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(tx);
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    super::super::effects::schedule_effect(
        super::super::effects::EffectExecutors {
            executor: &executor,
            repo_load_executor: &executor,
            worktree_scan_executor: &scan_executor,
            session_persist_executor: &executor,
            metadata_executor: &executor,
            signature_executor: &executor,
            history_find_executor: &scan_executor,
            filesystem_executor: &filesystem_executor,
        },
        &state,
        &backend,
        &mut repos,
        sender,
        effect,
    );
    let event = match change {
        QueuedPatchViewChange::Unchanged => None,
        QueuedPatchViewChange::Retarget => Some(DiffSessionMsg::Open {
            repo_id,
            lifetime,
            view,
            target: DiffTarget::working_tree("other.rs".into(), DiffArea::Unstaged),
        }),
        QueuedPatchViewChange::Close => Some(DiffSessionMsg::Close {
            repo_id,
            lifetime,
            view,
        }),
    };
    if let Some(event) = event {
        let mut state = state.write().unwrap();
        reduce(
            &mut repos,
            &id_alloc,
            Arc::make_mut(&mut state),
            Msg::DiffSession(event),
        );
    }
    drop(release_guard);
    executor.join();
    let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
        command: crate::msg::RepoCommandKind::ApplyWorktreePatch { reverse: true },
        result,
        ..
    }) = recv_effect_message(&rx, Duration::from_secs(2)).unwrap()
    else {
        panic!("expected the queued discard completion");
    };
    let calls = calls.lock().unwrap().clone();
    (calls, result)
}

#[test]
fn queued_prepared_patch_applies_when_its_view_remains_current() {
    let (calls, result) = run_queued_patch_with_view_change(QueuedPatchViewChange::Unchanged);
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(calls, ["apply reviewed"]);
}

#[test]
fn queued_prepared_patch_refuses_a_retargeted_view() {
    let (calls, result) = run_queued_patch_with_view_change(QueuedPatchViewChange::Retarget);
    assert!(
        calls.is_empty(),
        "a stale discard reached the backend: {calls:?}"
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "The comparison changed. Try again on the current diff."
    );
}

#[test]
fn queued_prepared_patch_refuses_a_closed_view() {
    let (calls, result) = run_queued_patch_with_view_change(QueuedPatchViewChange::Close);
    assert!(
        calls.is_empty(),
        "a stale discard reached the backend: {calls:?}"
    );
    assert_eq!(
        result.unwrap_err().to_string(),
        "The comparison changed. Try again on the current diff."
    );
}

#[test]
fn reviewed_apply_excludes_repository_mutations_and_editor_saves() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.rs");
    fs::write(&path, "new\n").unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let release_guard = BlockingReleaseGuard {
        release: release.clone(),
    };
    let (started, started_rx) = std::sync::mpsc::channel();
    let mut repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: dir.path().into(),
        },
        calls.clone(),
    );
    repo.prepared_barrier = Some(PreparedPatchBarrier { started, release });
    let repo: Arc<dyn GitRepository> = Arc::new(repo);
    let repo_id = RepoId(9541);
    let mut repos = RepositoryRuntimes::default();
    let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        "file.rs".into(),
        DiffArea::Unstaged,
    ));
    let lifetime = repo_state.lifetime();
    repos.opened(repo_id, repo_state.lifetime_id(), repo.clone());
    let prepared = gitcomet_core::patch_action::prepare(DiffActionSnapshot {
        diff: gitcomet_core::domain::Diff::from_unified(DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
            "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new\n"),
        old: "old\n".into(), new: "new\n".into(), policy: Arc::from([]),
    }, PatchOperation::Stage, &SourceLineSelection { old: BTreeSet::from([1]), new: BTreeSet::from([1]) }, "old\n", "new\n").unwrap();
    let executor = super::super::executor::TaskExecutor::new(3);
    let mut filesystem_executor: std::sync::LazyLock<super::super::executor::TaskExecutor> =
        std::sync::LazyLock::new(super::super::executor::filesystem_queue);
    let scan_executor: std::sync::LazyLock<super::super::executor::TaskExecutor> =
        std::sync::LazyLock::new(|| super::super::executor::TaskExecutor::new(1));
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(tx);
    let state = Arc::new(std::sync::RwLock::new(Arc::new(AppState {
        repos: vec![repo_state],
        ..AppState::test_default()
    })));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let mut schedule = |effect| {
        super::super::effects::schedule_effect(
            super::super::effects::EffectExecutors {
                executor: &executor,
                repo_load_executor: &executor,
                worktree_scan_executor: &scan_executor,
                session_persist_executor: &executor,
                metadata_executor: &executor,
                signature_executor: &executor,
                history_find_executor: &scan_executor,
                filesystem_executor: &filesystem_executor,
            },
            &state,
            &backend,
            &mut repos,
            sender.clone(),
            effect,
        )
    };
    schedule(Effect::RepoCommand(
        crate::msg::RepoCommandEffect::ApplyPreparedDiffPatch {
            repo_id,
            lifetime,
            view: None,
            file_rev: 0,
            generation: 0,
            prepared: Arc::new(prepared),
        },
    ));
    started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let gate = super::super::mutation_gate::for_repo(repo.as_ref());
    assert!(gate.try_read().is_err());
    assert!(sender.filesystem.try_lock().is_err());
    schedule(Effect::CheckoutBranch {
        repo_id,
        name: "next".into(),
    });
    schedule(Effect::Filesystem(gitcomet_core::filesystem::Request::new(
        gitcomet_core::filesystem::Operation::Save {
            path: path.clone(),
            worktree: Some(dir.path().into()),
            contents: Arc::from(b"editor\n".as_slice()),
            expected: None,
            overwrite: true,
        },
    )));
    assert!(recv_effect_message(&rx, Duration::from_millis(100)).is_err());
    assert!(calls.lock().unwrap().is_empty());
    assert_eq!(fs::read(&path).unwrap(), b"new\n");
    drop(release_guard);
    executor.join();
    std::sync::LazyLock::get_mut(&mut filesystem_executor)
        .unwrap()
        .join_workers();
    assert_eq!(*calls.lock().unwrap(), ["apply reviewed", "checkout next"]);
    assert_eq!(fs::read(&path).unwrap(), b"editor\n");
}

/// Every repo command held the mutation gate's read side while it ran, so a
/// fetch, or a push waiting on credentials, refused each Difftastic partial
/// action though neither touches the index or the worktree. A pull writes
/// both, so it keeps refusing one.
#[test]
fn a_running_pull_refuses_a_reviewed_apply_but_a_fetch_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let (remote_started, remote_started_rx) = std::sync::mpsc::channel();
    let (apply_started, apply_started_rx) = std::sync::mpsc::channel();
    let remote_release = Arc::new((Mutex::new(false), Condvar::new()));
    let mut repo = RecordingCheckoutRepo::new(
        RepoSpec {
            workdir: dir.path().into(),
        },
        calls.clone(),
    );
    repo.remote_barrier = Some(PreparedPatchBarrier {
        started: remote_started,
        release: remote_release.clone(),
    });
    repo.prepared_barrier = Some(PreparedPatchBarrier {
        started: apply_started,
        release: Arc::new((Mutex::new(true), Condvar::new())),
    });
    let repo: Arc<dyn GitRepository> = Arc::new(repo);
    let repo_id = RepoId(9542);
    let mut repos = RepositoryRuntimes::default();
    let mut repo_state = RepoState::new_opening(repo_id, repo.spec().clone());
    repo_state.diff_state.diff_target = Some(DiffTarget::working_tree(
        "file.rs".into(),
        DiffArea::Unstaged,
    ));
    let lifetime = repo_state.lifetime();
    repos.opened(repo_id, repo_state.lifetime_id(), repo.clone());
    let prepared = Arc::new(gitcomet_core::patch_action::prepare(DiffActionSnapshot {
        diff: gitcomet_core::domain::Diff::from_unified(DiffTarget::working_tree("file.rs".into(), DiffArea::Unstaged),
            "diff --git a/file.rs b/file.rs\n--- a/file.rs\n+++ b/file.rs\n@@ -1 +1 @@\n-old\n+new\n"),
        old: "old\n".into(), new: "new\n".into(), policy: Arc::from([]),
    }, PatchOperation::Stage, &SourceLineSelection { old: BTreeSet::from([1]), new: BTreeSet::from([1]) }, "old\n", "new\n").unwrap());
    let executor = super::super::executor::TaskExecutor::new(3);
    let filesystem_executor: std::sync::LazyLock<super::super::executor::TaskExecutor> =
        std::sync::LazyLock::new(super::super::executor::filesystem_queue);
    let scan_executor: std::sync::LazyLock<super::super::executor::TaskExecutor> =
        std::sync::LazyLock::new(|| super::super::executor::TaskExecutor::new(1));
    let (tx, rx) = std::sync::mpsc::channel();
    let sender = super::super::worker_channel::StoreWorkerSender::for_test_msg_sender(tx);
    let state = Arc::new(std::sync::RwLock::new(Arc::new(AppState {
        repos: vec![repo_state],
        ..AppState::test_default()
    })));
    let backend: Arc<dyn GitBackend> = Arc::new(PanicOpenBackend);
    let mut schedule = |effect| {
        super::super::effects::schedule_effect(
            super::super::effects::EffectExecutors {
                executor: &executor,
                repo_load_executor: &executor,
                worktree_scan_executor: &scan_executor,
                session_persist_executor: &executor,
                metadata_executor: &executor,
                signature_executor: &executor,
                history_find_executor: &scan_executor,
                filesystem_executor: &filesystem_executor,
            },
            &state,
            &backend,
            &mut repos,
            sender.clone(),
            effect,
        )
    };
    for (remote, refused) in [
        (
            crate::msg::RepoCommandEffect::Pull {
                repo_id,
                mode: PullMode::Default,
                prune: false,
                auth: None,
            },
            true,
        ),
        (
            crate::msg::RepoCommandEffect::FetchAll {
                repo_id,
                prune: false,
                auth: None,
            },
            false,
        ),
    ] {
        let label = format!("{remote:?}");
        *remote_release.0.lock().unwrap() = false;
        schedule(Effect::RepoCommand(remote));
        remote_started_rx
            .recv_timeout(Duration::from_secs(2))
            .unwrap();
        schedule(Effect::RepoCommand(
            crate::msg::RepoCommandEffect::ApplyPreparedDiffPatch {
                repo_id,
                lifetime,
                view: None,
                file_rev: 0,
                generation: 0,
                prepared: prepared.clone(),
            },
        ));
        if refused {
            let result = loop {
                if let Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                    command: crate::msg::RepoCommandKind::StageHunk,
                    result,
                    ..
                }) = recv_effect_message(&rx, Duration::from_secs(2)).unwrap()
                {
                    break result;
                }
            };
            assert!(result.is_err(), "{label}: {result:?}");
            assert!(apply_started_rx.try_recv().is_err(), "{label}");
        } else {
            apply_started_rx
                .recv_timeout(Duration::from_secs(2))
                .unwrap_or_else(|_| panic!("{label}: the apply was refused"));
        }
        drop(BlockingReleaseGuard {
            release: remote_release.clone(),
        });
        while !matches!(
            recv_effect_message(&rx, Duration::from_secs(2)).unwrap(),
            Msg::Internal(crate::msg::InternalMsg::RepoCommandFinished {
                command: crate::msg::RepoCommandKind::Pull { .. }
                    | crate::msg::RepoCommandKind::FetchAll,
                ..
            })
        ) {}
    }
    executor.join();
    assert_eq!(*calls.lock().unwrap(), ["pull", "apply reviewed", "fetch"]);
}
