use crate::model::AppState;
use crate::msg::{InternalMsg, Msg, ReferenceEffect};
use crate::store::repository_runtime::RepositoryRuntimes;
use crate::store::{
    executor::{StoreExecutorPool, TaskExecutor},
    worker_channel::StoreWorkerSender,
};
use gitcomet_core::services::{CancellationToken, GitRepository};
use std::sync::{Arc, RwLock};

use super::load_tokens::repo_load_context;
use super::util::{missing_repo_error, send_or_log, spawn_detached_with_repo_or_else};

pub(super) fn schedule(
    repo_load_executor: &TaskExecutor,
    metadata_executor: &TaskExecutor,
    thread_state: &Arc<RwLock<Arc<AppState>>>,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    work: ReferenceEffect,
) {
    let repo_id = work.repo_id();
    let Some((msg_tx, cancellation)) = repo_load_context(thread_state, repos, msg_tx, repo_id)
    else {
        return;
    };
    // A remote can remain offline for minutes. Keep its optional tag lookup
    // away from local tags/ref metadata, including loads from other windows.
    // Create the shared pool only when remote metadata is actually requested.
    let remote_metadata_executor = matches!(work, ReferenceEffect::LoadRemoteTags { .. })
        .then(|| TaskExecutor::shared_for_store(StoreExecutorPool::RemoteMetadata, 2));
    let (executor, task_name) = match work {
        ReferenceEffect::LoadBranches { .. } => (repo_load_executor, "load-branches"),
        ReferenceEffect::LoadRemotes { .. } => (repo_load_executor, "load-remotes"),
        ReferenceEffect::LoadRemoteBranches { .. } => (repo_load_executor, "load-remote-branches"),
        ReferenceEffect::LoadHeadBranch { .. } => (repo_load_executor, "load-head-branch"),
        ReferenceEffect::LoadUpstreamDivergence { .. } => {
            (repo_load_executor, "load-upstream-divergence")
        }
        ReferenceEffect::LoadTags { .. } => (metadata_executor, "load-tags"),
        ReferenceEffect::LoadRemoteTags { .. } => (
            remote_metadata_executor
                .as_ref()
                .expect("remote metadata executor"),
            "load-remote-tags",
        ),
        ReferenceEffect::LoadRefMetadata { .. } => (metadata_executor, "load-ref-metadata"),
    };
    let run = move |repo: Arc<dyn GitRepository>, msg_tx: StoreWorkerSender| {
        if matches!(work, ReferenceEffect::LoadHeadBranch { .. }) {
            send_or_log(
                &msg_tx,
                Msg::Internal(InternalMsg::RepositoryModeLoaded {
                    repo_id,
                    result: super::bare_conversion::mode_result(repo.as_ref()),
                }),
            );
        }
        send_or_log(
            &msg_tx,
            Msg::Internal(run(work, repo.as_ref(), &cancellation)),
        );
    };
    let missing = move |msg_tx: StoreWorkerSender| {
        send_or_log(
            &msg_tx,
            Msg::Internal(work.failed(missing_repo_error(repo_id))),
        );
    };
    spawn_detached_with_repo_or_else(executor, task_name, repos, repo_id, msg_tx, run, missing);
}

fn run(
    work: ReferenceEffect,
    repo: &dyn GitRepository,
    cancellation: &CancellationToken,
) -> InternalMsg {
    match work {
        ReferenceEffect::LoadBranches { repo_id } => InternalMsg::BranchesLoaded {
            repo_id,
            result: repo.list_branches(cancellation),
        },
        ReferenceEffect::LoadRemotes { repo_id } => InternalMsg::RemotesLoaded {
            repo_id,
            result: repo.list_remotes(cancellation),
        },
        ReferenceEffect::LoadRemoteBranches { repo_id } => InternalMsg::RemoteBranchesLoaded {
            repo_id,
            result: repo.list_remote_branches(cancellation),
        },
        ReferenceEffect::LoadHeadBranch { repo_id } => InternalMsg::HeadBranchLoaded {
            repo_id,
            result: repo.current_branch(cancellation),
        },
        ReferenceEffect::LoadUpstreamDivergence { repo_id } => {
            InternalMsg::UpstreamDivergenceLoaded {
                repo_id,
                result: repo.upstream_divergence(cancellation),
            }
        }
        ReferenceEffect::LoadTags { repo_id } => InternalMsg::TagsLoaded {
            repo_id,
            result: repo.list_tags(cancellation),
        },
        ReferenceEffect::LoadRemoteTags { repo_id } => InternalMsg::RemoteTagsLoaded {
            repo_id,
            result: repo.list_remote_tags(cancellation),
        },
        ReferenceEffect::LoadRefMetadata { repo_id } => InternalMsg::RefMetadataLoaded {
            repo_id,
            result: repo.list_ref_metadata(cancellation),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{RepoId, RepoState};
    use gitcomet_core::domain::RepoSpec;
    use std::{sync::mpsc, time::Duration};

    #[test]
    fn blocked_remote_discovery_does_not_hold_up_local_tags() {
        let remote = TaskExecutor::shared_for_store(StoreExecutorPool::RemoteMetadata, 2);
        let (started_tx, started_rx) = mpsc::channel();
        let mut releases = Vec::new();
        for _ in 0..2 {
            let (release_tx, release_rx) = mpsc::channel::<()>();
            releases.push(release_tx);
            let started_tx = started_tx.clone();
            remote.spawn(move || {
                let _ = started_tx.send(());
                let _ = release_rx.recv();
            });
        }
        for _ in 0..2 {
            started_rx
                .recv_timeout(Duration::from_secs(5))
                .expect("remote worker started");
        }

        let repo_id = RepoId(1);
        let mut state = AppState::test_default();
        let spec = RepoSpec {
            workdir: "remote-metadata-test".into(),
        };
        state
            .repos
            .push(RepoState::new_opening(repo_id, spec.clone()));
        let mut repos = RepositoryRuntimes::default();
        repos.attach_repository_for_test(
            repo_id,
            Arc::new(gitcomet_core::test_support::UnconfiguredRepository::new(
                spec.workdir,
            )),
        );
        repos.bind_test_state(&state);
        let state = Arc::new(RwLock::new(Arc::new(state)));
        let (tx, rx) = mpsc::channel();
        let sender = StoreWorkerSender::for_test_msg_sender(tx);
        let local = TaskExecutor::new(1);
        for work in [
            ReferenceEffect::LoadRemoteTags { repo_id },
            ReferenceEffect::LoadTags { repo_id },
        ] {
            schedule(&local, &local, &state, &mut repos, sender.clone(), work);
        }

        let first = rx.recv_timeout(Duration::from_secs(5));
        // Always release the blocked pool before assertions (including failure).
        drop(releases);
        let first = first.expect("local tags must finish while network workers are blocked");
        fn result(message: Msg) -> InternalMsg {
            let Msg::Internal(mut message) = message else {
                panic!("expected internal result")
            };
            while let InternalMsg::RepoLoadFinished { message: inner, .. } = message {
                message = *inner;
            }
            message
        }
        assert!(matches!(result(first), InternalMsg::TagsLoaded { .. }));
        assert!(matches!(
            result(
                rx.recv_timeout(Duration::from_secs(5))
                    .expect("remote tags complete after release")
            ),
            InternalMsg::RemoteTagsLoaded { .. },
        ));
        local.join();
    }
}
