//! Worktree list publications shared by the explicit loader and full scans.

use super::util::send_or_log;
use crate::model::RepoId;
use crate::msg::{InternalMsg, Msg, WorktreeListRead};
use crate::store::worker_channel::StoreWorkerSender;
use gitcomet_core::domain::Worktree;
use gitcomet_core::error::Error;
use std::sync::{Arc, Mutex};

/// One cache per repository load epoch. Cancelling/reopening a repo drops it,
/// so a previously suppressed listing cannot strand a reset view in NotLoaded.
#[derive(Clone, Default)]
pub(super) struct WorktreeListPublisher {
    latest: Arc<Mutex<Option<WorktreeListSnapshot>>>,
}

struct WorktreeListSnapshot {
    seq: u64,
    worktrees: Arc<Vec<Worktree>>,
}

impl WorktreeListPublisher {
    pub(super) fn publish(
        &self,
        msg_tx: &StoreWorkerSender,
        repo_id: RepoId,
        mut read: WorktreeListRead,
        mut result: Result<Arc<Vec<Worktree>>, Error>,
    ) {
        let mut latest = self
            .latest
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if msg_tx.is_cancelled() {
            return;
        }
        if let Some(previous) = latest.as_ref()
            && previous.seq > read.seq
        {
            if !read.completes_load {
                return;
            }
            // Even an unchanged scan advances the ordering here. An older
            // explicit reply must still finish its slot, but must not restore
            // stale rows or errors after that scan was suppressed.
            read.seq = previous.seq;
            result = Ok(previous.worktrees.clone());
        }
        if let Ok(worktrees) = &result {
            let changed = latest
                .as_ref()
                .is_none_or(|previous| previous.worktrees != *worktrees);
            *latest = Some(WorktreeListSnapshot {
                seq: read.seq,
                worktrees: worktrees.clone(),
            });
            if !read.completes_load && !changed {
                return;
            }
        }
        // Serialize sends as well as cache updates: concurrent workers cannot
        // enqueue an older value after a newer, suppressed observation.
        send_or_log(
            msg_tx,
            Msg::Internal(InternalMsg::WorktreesLoaded {
                repo_id,
                read,
                result,
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(branch: &str) -> Arc<Vec<Worktree>> {
        Arc::new(vec![Worktree {
            path: "/linked".into(),
            head: None,
            branch: Some(branch.into()),
            detached: false,
        }])
    }

    #[test]
    fn suppressed_observations_still_supersede_older_explicit_replies() {
        for fails in [false, true] {
            let publisher = WorktreeListPublisher::default();
            let (tx, rx) = std::sync::mpsc::channel();
            let tx = StoreWorkerSender::for_test_msg_sender(tx);
            let repo_id = RepoId(1);
            publisher.publish(
                &tx,
                repo_id,
                WorktreeListRead::from_dirty_scan(),
                Ok(list("new")),
            );
            rx.try_recv().unwrap();
            let explicit = WorktreeListRead::explicit();
            let observed = WorktreeListRead::from_dirty_scan();
            publisher.publish(&tx, repo_id, observed, Ok(list("new")));
            assert!(rx.try_recv().is_err());
            let result = if fails {
                Err(Error::new(gitcomet_core::error::ErrorKind::Backend(
                    "old failure".into(),
                )))
            } else {
                Ok(list("old"))
            };
            publisher.publish(&tx, repo_id, explicit, result);
            match rx.try_recv().unwrap() {
                Msg::Internal(InternalMsg::WorktreesLoaded { read, result, .. }) => {
                    assert!(read.completes_load);
                    assert_eq!(read.seq, observed.seq);
                    assert_eq!(result.unwrap(), list("new"));
                }
                other => panic!("unexpected reply: {other:?}"),
            }
        }
    }

    #[test]
    fn changed_lists_publish_and_explicit_reads_always_finish() {
        let publisher = WorktreeListPublisher::default();
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = StoreWorkerSender::for_test_msg_sender(tx);
        for (branch, explicit, expected) in [
            ("a", false, true),
            ("a", false, false),
            ("b", false, true),
            ("b", true, true),
            ("b", false, false),
            ("a", false, true),
        ] {
            let read = if explicit {
                WorktreeListRead::explicit()
            } else {
                WorktreeListRead::from_dirty_scan()
            };
            publisher.publish(&tx, RepoId(1), read, Ok(list(branch)));
            assert_eq!(rx.try_recv().is_ok(), expected);
        }
        // A replacement epoch/repository must publish even an identical list.
        WorktreeListPublisher::default().publish(
            &tx,
            RepoId(1),
            WorktreeListRead::from_dirty_scan(),
            Ok(list("a")),
        );
        assert!(rx.try_recv().is_ok());
    }
}
