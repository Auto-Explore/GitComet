use super::clone;
use crate::msg::ControlEffect;
use crate::store::repository_runtime::RepositoryRuntimes;
use crate::store::worker_channel::StoreWorkerSender;

pub(super) fn schedule(
    executor: &super::super::executor::TaskExecutor,
    repos: &mut RepositoryRuntimes,
    msg_tx: StoreWorkerSender,
    work: ControlEffect,
) {
    match work {
        ControlEffect::PreviewDisableBareMode { repo_id } => {
            super::bare_conversion::preview(executor, repos, msg_tx, repo_id)
        }
        ControlEffect::DisableBareMode {
            repo_id,
            common_dir,
            preview,
        } => super::bare_conversion::disable(executor, repos, msg_tx, repo_id, common_dir, preview),
        ControlEffect::RepositoryQuiesced { reply } => {
            let _ = reply.send(());
        }
        ControlEffect::RefreshRepositoryMode { repo_id } => {
            super::bare_conversion::refresh_mode(executor, repos, msg_tx, repo_id)
        }
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
