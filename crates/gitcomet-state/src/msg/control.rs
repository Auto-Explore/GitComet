use crate::model::RepoId;
use gitcomet_core::git_operation::GitOperationId;
use std::path::PathBuf;

#[derive(Clone, Debug, strum::IntoStaticStr)]
pub enum ControlEffect {
    CancelRepoLoads {
        repo_id: RepoId,
        load_epoch: crate::model::RepoLoadEpoch,
    },
    CancelGitOperation {
        repo_id: RepoId,
        operation_id: GitOperationId,
    },
    AbortCloneRepo {
        dest: PathBuf,
    },
}
