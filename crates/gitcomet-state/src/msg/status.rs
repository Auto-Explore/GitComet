use super::InternalMsg;
use crate::model::RepoId;
use gitcomet_core::domain::RepoStatus;

#[derive(Clone, Debug, strum::IntoStaticStr)]
pub enum StatusEffect {
    LoadWorktreeStatus {
        repo_id: RepoId,
    },
    LoadStagedStatus {
        repo_id: RepoId,
    },
    LoadUncommittedLineStats {
        repo_id: RepoId,
        generation: crate::model::LineStatsGeneration,
        status: std::sync::Arc<RepoStatus>,
        /// Also classify rows for Git LFS / git-annex in the same pass.
        large_files: bool,
    },
    LoadStatus {
        repo_id: RepoId,
    },
}

impl StatusEffect {
    pub(crate) fn repo_id(&self) -> RepoId {
        match self {
            Self::LoadWorktreeStatus { repo_id }
            | Self::LoadStagedStatus { repo_id }
            | Self::LoadUncommittedLineStats { repo_id, .. }
            | Self::LoadStatus { repo_id } => *repo_id,
        }
    }

    pub(crate) fn failed(self, error: gitcomet_core::error::Error) -> InternalMsg {
        match self {
            StatusEffect::LoadWorktreeStatus { repo_id } => InternalMsg::WorktreeStatusLoaded {
                repo_id,
                result: Err(error),
            },
            StatusEffect::LoadStagedStatus { repo_id } => InternalMsg::StagedStatusLoaded {
                repo_id,
                result: Err(error),
            },
            StatusEffect::LoadUncommittedLineStats {
                repo_id,
                generation,
                ..
            } => InternalMsg::UncommittedLineStatsLoaded {
                repo_id,
                generation,
                result: Err(error),
                large_files: None,
            },
            StatusEffect::LoadStatus { repo_id } => InternalMsg::StatusLoaded {
                repo_id,
                result: Err(error),
            },
        }
    }
}
