use super::InternalMsg;
use crate::model::RepoId;

#[derive(Clone, Copy, Debug, strum::IntoStaticStr)]
pub enum ReferenceEffect {
    LoadBranches { repo_id: RepoId },
    LoadRemotes { repo_id: RepoId },
    LoadRemoteBranches { repo_id: RepoId },
    LoadHeadBranch { repo_id: RepoId },
    LoadUpstreamDivergence { repo_id: RepoId },
    LoadTags { repo_id: RepoId },
    LoadRemoteTags { repo_id: RepoId },
    LoadRefMetadata { repo_id: RepoId },
}

impl ReferenceEffect {
    pub(crate) fn repo_id(self) -> RepoId {
        match self {
            Self::LoadBranches { repo_id }
            | Self::LoadRemotes { repo_id }
            | Self::LoadRemoteBranches { repo_id }
            | Self::LoadHeadBranch { repo_id }
            | Self::LoadUpstreamDivergence { repo_id }
            | Self::LoadTags { repo_id }
            | Self::LoadRemoteTags { repo_id }
            | Self::LoadRefMetadata { repo_id } => repo_id,
        }
    }

    pub(crate) fn failed(self, error: gitcomet_core::error::Error) -> InternalMsg {
        match self {
            ReferenceEffect::LoadBranches { repo_id } => InternalMsg::BranchesLoaded {
                repo_id,
                result: Err(error),
            },
            ReferenceEffect::LoadRemotes { repo_id } => InternalMsg::RemotesLoaded {
                repo_id,
                result: Err(error),
            },
            ReferenceEffect::LoadRemoteBranches { repo_id } => InternalMsg::RemoteBranchesLoaded {
                repo_id,
                result: Err(error),
            },
            ReferenceEffect::LoadHeadBranch { repo_id } => InternalMsg::HeadBranchLoaded {
                repo_id,
                result: Err(error),
            },
            ReferenceEffect::LoadUpstreamDivergence { repo_id } => {
                InternalMsg::UpstreamDivergenceLoaded {
                    repo_id,
                    result: Err(error),
                }
            }
            ReferenceEffect::LoadTags { repo_id } => InternalMsg::TagsLoaded {
                repo_id,
                result: Err(error),
            },
            ReferenceEffect::LoadRemoteTags { repo_id } => InternalMsg::RemoteTagsLoaded {
                repo_id,
                result: Err(error),
            },
            ReferenceEffect::LoadRefMetadata { repo_id } => InternalMsg::RefMetadataLoaded {
                repo_id,
                result: Err(error),
            },
        }
    }
}
