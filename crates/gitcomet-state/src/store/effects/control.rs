use super::clone;
use crate::msg::ControlEffect;
use crate::store::repository_runtime::RepositoryRuntimes;
use crate::store::worker_channel::StoreWorkerSender;

pub(super) fn schedule(
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    work: ControlEffect,
) {
    match work {
        ControlEffect::CancelRepoLoads {
            repo_id,
            load_epoch,
        } => repos.cancel_loads(repo_id, load_epoch),
        ControlEffect::CancelGitOperation { operation_id, .. } => {
            let _ = gitcomet_core::git_operation::cancel(operation_id);
        }
        ControlEffect::AbortCloneRepo { dest } => clone::schedule_abort_clone_repo(msg_tx, dest),
    }
}
