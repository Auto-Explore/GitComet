//! A burst of history clicks needs only the latest queued detail load. Unlike
//! log/status refreshes, superseded selections have no result the UI can use.
use super::util::{RepoMap, send_or_log};
use crate::model::{AppState, RepoId};
use crate::msg::{InternalMsg, Msg};
use crate::store::executor::{LatestTaskSlot, TaskExecutor};
use crate::store::{repo_load_trace, worker_channel::StoreWorkerSender};
use gitcomet_core::domain::{CommitDetails, CommitId};
use gitcomet_core::services::Result;
use std::sync::{Arc, RwLock};

pub(super) fn schedule(
    executor: &TaskExecutor,
    slot: &LatestTaskSlot,
    repos: &RepoMap,
    state: Arc<RwLock<Arc<AppState>>>,
    tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: CommitId,
) {
    let Some(repo) = repos.get(&repo_id).cloned() else {
        return;
    };
    let id = commit_id.clone();
    queue(executor, slot, state, tx, repo_id, commit_id, move || {
        repo.commit_details(&id)
    });
}

fn queue(
    executor: &TaskExecutor,
    slot: &LatestTaskSlot,
    state: Arc<RwLock<Arc<AppState>>>,
    tx: StoreWorkerSender,
    repo_id: RepoId,
    commit_id: CommitId,
    load: impl FnOnce() -> Result<CommitDetails> + Send + 'static,
) {
    let selected_id = commit_id.clone();
    let selection = move || {
        let state = state.read().unwrap_or_else(|e| e.into_inner());
        let repo = state.repos.iter().find(|repo| repo.id == repo_id)?;
        (repo.history_state.selected_commit.as_ref() == Some(&selected_id))
            .then_some(repo.history_state.selected_commit_rev)
    };
    let Some(revision) = selection() else {
        return;
    };
    repo_load_trace::trace!("queue_repo_task repo_id={repo_id:?} task=commit_details");
    let replaced = executor.spawn_latest(slot, move || {
        if tx.is_cancelled() || selection() != Some(revision) {
            repo_load_trace::trace!(
                "skip_repo_task_stale_selection repo_id={repo_id:?} task=commit_details"
            );
            return;
        }
        repo_load_trace::trace!("start_repo_task repo_id={repo_id:?} task=commit_details");
        let result = load();
        if !tx.is_cancelled() && selection() == Some(revision) {
            send_or_log(
                &tx,
                Msg::Internal(InternalMsg::CommitDetailsLoaded {
                    repo_id,
                    commit_id,
                    result,
                }),
            );
        }
        repo_load_trace::trace!("finish_repo_task repo_id={repo_id:?} task=commit_details");
    });
    if replaced {
        repo_load_trace::trace!("replace_queued_repo_task repo_id={repo_id:?} task=commit_details");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RepoState;
    use gitcomet_core::domain::RepoSpec;
    use gitcomet_core::error::{Error, ErrorKind};
    use gitcomet_core::services::CancellationToken;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, mpsc};
    use std::time::Duration;

    fn state() -> Arc<RwLock<Arc<AppState>>> {
        let mut state = AppState::test_default();
        for id in [RepoId(1), RepoId(2)] {
            state.repos.push(RepoState::new_opening(
                id,
                RepoSpec {
                    workdir: "/tmp/commit-detail-queue".into(),
                },
            ));
        }
        Arc::new(RwLock::new(Arc::new(state)))
    }

    fn select(state: &RwLock<Arc<AppState>>, repo_id: RepoId, commit: Option<CommitId>) {
        let mut state = state.write().unwrap();
        Arc::make_mut(&mut state)
            .repos
            .iter_mut()
            .find(|r| r.id == repo_id)
            .unwrap()
            .set_selected_commit(commit);
    }

    fn result() -> Result<CommitDetails> {
        Err(Error::new(ErrorKind::Unsupported("recording detail load")))
    }

    #[test]
    fn burst_loads_only_the_latest_queued_commit_per_repository() {
        let executor = TaskExecutor::new(1);
        let (release, blocked) = mpsc::channel();
        executor.spawn(move || {
            blocked.recv().unwrap();
        });
        let state = state();
        let slots = [LatestTaskSlot::default(), LatestTaskSlot::default()];
        let calls = Arc::new(Mutex::new(Vec::new()));
        let (sender, replies) = mpsc::channel();
        let tx = StoreWorkerSender::for_test_msg_sender(sender);
        for ix in 0..100 {
            for (repo_ix, slot) in slots.iter().enumerate() {
                let id = RepoId(repo_ix as u64 + 1);
                let commit = CommitId(format!("commit-{ix}").into());
                select(&state, id, Some(commit.clone()));
                let calls = calls.clone();
                queue(
                    &executor,
                    slot,
                    state.clone(),
                    tx.clone(),
                    id,
                    commit,
                    move || {
                        calls.lock().unwrap().push((id, ix));
                        result()
                    },
                );
            }
        }
        release.send(()).unwrap();
        executor.join();
        assert_eq!(*calls.lock().unwrap(), [(RepoId(1), 99), (RepoId(2), 99)]);
        assert_eq!(replies.try_iter().count(), 2);
    }

    #[test]
    fn deselection_repo_close_and_cancellation_skip_queued_details() {
        for reason in ["deselected", "closed", "cancelled", "selected_again"] {
            let executor = TaskExecutor::new(1);
            let (release, blocked) = mpsc::channel();
            executor.spawn(move || {
                blocked.recv().unwrap();
            });
            let state = state();
            let id = RepoId(1);
            let commit = CommitId("commit".into());
            select(&state, id, Some(commit.clone()));
            let (sender, replies) = mpsc::channel();
            let cancellation = CancellationToken::new();
            let tx = StoreWorkerSender::for_test_msg_sender(sender).with_repo_load_guard(
                id,
                0,
                cancellation.clone(),
            );
            let calls = Arc::new(AtomicUsize::new(0));
            let called = calls.clone();
            queue(
                &executor,
                &LatestTaskSlot::default(),
                state.clone(),
                tx,
                id,
                commit.clone(),
                move || {
                    called.fetch_add(1, Ordering::Relaxed);
                    result()
                },
            );
            match reason {
                "closed" => Arc::make_mut(&mut state.write().unwrap()).repos.clear(),
                "cancelled" => cancellation.cancel(),
                "selected_again" => {
                    select(&state, id, None);
                    select(&state, id, Some(commit));
                }
                _ => select(&state, id, None),
            }
            release.send(()).unwrap();
            executor.join();
            assert_eq!(calls.load(Ordering::Relaxed), 0, "{reason}");
            assert_eq!(replies.try_iter().count(), 0);
        }
    }

    #[test]
    fn superseded_running_details_do_not_publish_a_stale_reply() {
        let executor = TaskExecutor::new(1);
        let state = state();
        let id = RepoId(1);
        let commit = CommitId("commit".into());
        select(&state, id, Some(commit.clone()));
        let (sender, replies) = mpsc::channel();
        let (started, start) = mpsc::channel();
        let (release, blocked) = mpsc::channel();
        queue(
            &executor,
            &LatestTaskSlot::default(),
            state.clone(),
            StoreWorkerSender::for_test_msg_sender(sender),
            id,
            commit,
            move || {
                started.send(()).unwrap();
                blocked.recv_timeout(Duration::from_secs(5)).unwrap();
                result()
            },
        );
        start.recv_timeout(Duration::from_secs(5)).unwrap();
        select(&state, id, None);
        release.send(()).unwrap();
        executor.join();
        assert_eq!(replies.try_iter().count(), 0);
    }
}
