use crate::model::{AppState, RepoId};
use crate::msg::{Effect, Msg};
use crate::store::effects::{EffectExecutors, schedule_effect};
use crate::store::executor::TaskExecutor;
use crate::store::repository_runtime::RepositoryRuntimes;
use crate::store::worker_channel::StoreWorkerSender;
use gitcomet_core::domain::DiffTarget;
use gitcomet_core::services::{CancellationToken, GitBackend};
use std::sync::{Arc, RwLock};

#[test]
fn visible_history_and_diff_content_finish_while_repository_loads_are_blocked() {
    use crate::diff_session::{
        DiffSessionContent, DiffSessionEffect, DiffSessionMsg, DiffSessionWork, DiffViewId,
    };
    use crate::indexed_history::{HistoryReadPriority, IndexedHistoryEffect, IndexedHistoryMsg};
    use crate::model::RepoState;
    use gitcomet_core::domain::{DiffArea, HistoryMode};
    use std::sync::{LazyLock, mpsc};

    let (_fixture, path, _) = crate::store::tests::worktree_redirect::repo_with_linked_worktree();
    std::fs::write(path.join("a.txt"), "modified\n").unwrap();
    let backend: Arc<dyn GitBackend> = Arc::new(gitcomet_git_gix::GixBackend);
    let repo = backend.open(&path).unwrap();
    let repo_id = RepoId(1);
    let state = RepoState::new_opening(repo_id, repo.spec().clone());
    let index = repo
        .build_history_index(
            &HistoryMode::FullReachable.into(),
            None,
            &Default::default(),
            &CancellationToken::new(),
            &mut |_| {},
        )
        .unwrap()
        .unwrap();
    let lifetime = state.lifetime();
    let state = Arc::new(RwLock::new(Arc::new(AppState {
        repos: vec![state],
        active_repo: Some(repo_id),
        ..AppState::test_default()
    })));
    let mut repos = RepositoryRuntimes::from_iter([(repo_id, repo)]);
    repos.bind_test_state(&state.read().unwrap());
    let (send, replies) = mpsc::channel();
    let sender = StoreWorkerSender::for_test_msg_sender(send);
    let occupied = TaskExecutor::new(1);
    let (release, wait) = mpsc::channel::<()>();
    occupied.spawn(move || {
        let _ = wait.recv();
    });
    let mut interactive: LazyLock<TaskExecutor> = LazyLock::new(|| TaskExecutor::new(1));
    let unused: LazyLock<TaskExecutor> = LazyLock::new(|| TaskExecutor::new(1));
    let executors = EffectExecutors {
        executor: &occupied,
        repo_load_executor: &occupied,
        interactive_executor: &interactive,
        worktree_scan_executor: &unused,
        session_persist_executor: &occupied,
        metadata_executor: &occupied,
        signature_executor: &occupied,
        history_find_executor: &unused,
        filesystem_executor: &unused,
    };
    let range = |seq, priority, cancellation| {
        Effect::IndexedHistory(IndexedHistoryEffect::Range {
            repo_id,
            seq,
            index: index.clone(),
            start: 0,
            priority,
            cancellation,
        })
    };
    let diff = DiffSessionEffect {
        repo_id,
        view: DiffViewId::next(),
        lifetime,
        generation: 1,
        cancellation: CancellationToken::new(),
        work: DiffSessionWork::Content {
            target: DiffTarget::working_tree("a.txt".into(), DiffArea::Unstaged),
            encoding: None,
            patch: false,
            file_text: true,
            image: false,
            preview_text_side: None,
        },
    };
    let obsolete = CancellationToken::new();
    for work in [
        range(1, HistoryReadPriority::Prefetch, obsolete.clone()),
        range(2, HistoryReadPriority::Prefetch, CancellationToken::new()),
        range(3, HistoryReadPriority::Visible, CancellationToken::new()),
        Effect::DiffSession(diff),
    ] {
        schedule_effect(
            executors,
            &state,
            &backend,
            &mut repos,
            sender.clone(),
            work,
        );
    }
    obsolete.cancel();
    LazyLock::force_mut(&mut interactive).join_workers();
    let before_release: Vec<_> = replies.try_iter().collect();
    drop(release);
    occupied.join();
    let ranges = |messages: &[Msg]| {
        messages
            .iter()
            .filter_map(|reply| {
                if let Msg::IndexedHistory(IndexedHistoryMsg::RangeLoaded { seq, result, .. }) =
                    reply
                {
                    Some((*seq, result.as_ref().unwrap().commits.len()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ranges(&before_release),
        [(3, 2)],
        "visible rows queued behind repository work"
    );
    assert_eq!(ranges(&replies.try_iter().collect::<Vec<_>>()), [(2, 2)]);
    assert!(
        before_release.iter().any(|msg| matches!(
            msg,
            Msg::DiffSession(DiffSessionMsg::Loaded {
                content: DiffSessionContent::FileText(Ok(Some(_))),
                ..
            })
        )),
        "diff file text queued behind repository work"
    );
}
