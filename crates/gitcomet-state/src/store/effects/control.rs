use super::{RepoTaskToken, clone, load_tokens};
use crate::model::RepoId;
use crate::msg::ControlEffect;
use crate::store::worker_channel::StoreWorkerSender;
use rustc_hash::FxHashMap;

pub(super) fn schedule(
    repo_task_tokens: &mut FxHashMap<RepoId, RepoTaskToken>,
    msg_tx: StoreWorkerSender,
    work: ControlEffect,
) {
    match work {
        ControlEffect::CancelRepoLoads {
            repo_id,
            load_epoch,
        } => load_tokens::cancel_repo_loads(repo_task_tokens, repo_id, load_epoch),
        ControlEffect::CancelGitOperation { operation_id, .. } => {
            let _ = gitcomet_core::git_operation::cancel(operation_id);
        }
        ControlEffect::AbortCloneRepo { dest } => clone::schedule_abort_clone_repo(msg_tx, dest),
    }
}
