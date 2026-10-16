use crate::model::AppState;
use crate::msg::{InternalMsg, Msg, ReferenceEffect};
use crate::store::repository_runtime::RepositoryRuntimes;
use crate::store::{executor::TaskExecutor, worker_channel::StoreWorkerSender};
use gitcomet_core::services::{CancellationToken, GitRepository};
use std::sync::{Arc, RwLock};

use super::load_tokens::repo_load_context;
use super::util::{
    missing_repo_error, send_or_log, spawn_detached_with_repo_or_else, spawn_with_repo_or_else,
};

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
    let (executor, task_name) = match work {
        ReferenceEffect::LoadBranches { .. } => (repo_load_executor, Some("load-branches")),
        ReferenceEffect::LoadRemotes { .. } => (repo_load_executor, Some("load-remotes")),
        ReferenceEffect::LoadRemoteBranches { .. } => {
            (repo_load_executor, Some("load-remote-branches"))
        }
        ReferenceEffect::LoadHeadBranch { .. } => (repo_load_executor, Some("load-head-branch")),
        ReferenceEffect::LoadUpstreamDivergence { .. } => {
            (repo_load_executor, Some("load-upstream-divergence"))
        }
        ReferenceEffect::LoadTags { .. } | ReferenceEffect::LoadRemoteTags { .. } => {
            (metadata_executor, None)
        }
        ReferenceEffect::LoadRefMetadata { .. } => (metadata_executor, Some("load-ref-metadata")),
    };
    let run = move |repo: Arc<dyn GitRepository>, msg_tx: StoreWorkerSender| {
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
    if let Some(task_name) = task_name {
        spawn_detached_with_repo_or_else(executor, task_name, repos, repo_id, msg_tx, run, missing);
    } else {
        spawn_with_repo_or_else(executor, repos, repo_id, msg_tx, run, missing);
    }
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
