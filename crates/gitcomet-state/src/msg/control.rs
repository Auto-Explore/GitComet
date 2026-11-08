use crate::model::RepoId;
use gitcomet_core::git_operation::GitOperationId;
use std::path::PathBuf;

#[derive(Clone, Debug, strum::IntoStaticStr)]
pub enum ControlEffect {
    PreviewDisableBareMode {
        repo_id: RepoId,
    },
    DisableBareMode {
        repo_id: RepoId,
        common_dir: PathBuf,
        preview: std::sync::Arc<gitcomet_core::domain::BareConversionPreview>,
    },
    RepositoryQuiesced {
        reply: std::sync::mpsc::Sender<()>,
    },
    RefreshRepositoryMode {
        repo_id: RepoId,
    },
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
