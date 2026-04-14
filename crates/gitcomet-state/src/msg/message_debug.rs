use super::message::InternalMsg;

impl std::fmt::Debug for InternalMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InternalMsg::SessionPersistFailed {
                repo_id,
                action,
                error,
            } => f
                .debug_struct("SessionPersistFailed")
                .field("repo_id", repo_id)
                .field("action", action)
                .field("error", error)
                .finish(),
            InternalMsg::CloneRepoProgress { dest, line } => f
                .debug_struct("CloneRepoProgress")
                .field("dest", dest)
                .field("line", line)
                .finish(),
            InternalMsg::CloneRepoFinished { url, dest, result } => f
                .debug_struct("CloneRepoFinished")
                .field("url", url)
                .field("dest", dest)
                .field("ok", &result.is_ok())
                .finish(),
            InternalMsg::RepoOpenedOk { repo_id, spec, .. } => f
                .debug_struct("RepoOpenedOk")
                .field("repo_id", repo_id)
                .field("spec", spec)
                .finish_non_exhaustive(),
            InternalMsg::RepoOpenedErr {
                repo_id,
                spec,
                error,
                ..
            } => f
                .debug_struct("RepoOpenedErr")
                .field("repo_id", repo_id)
                .field("spec", spec)
                .field("error", error)
                .finish(),
            InternalMsg::BranchesLoaded { repo_id, result } => f
                .debug_struct("BranchesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::RemotesLoaded { repo_id, result } => f
                .debug_struct("RemotesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::RemoteBranchesLoaded { repo_id, result } => f
                .debug_struct("RemoteBranchesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::WorktreeStatusLoaded { repo_id, result } => f
                .debug_struct("WorktreeStatusLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::StagedStatusLoaded { repo_id, result } => f
                .debug_struct("StagedStatusLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::StatusLoaded { repo_id, result } => f
                .debug_struct("StatusLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::HeadBranchLoaded { repo_id, result } => f
                .debug_struct("HeadBranchLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::UpstreamDivergenceLoaded { repo_id, result } => f
                .debug_struct("UpstreamDivergenceLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::LogLoaded {
                repo_id,
                scope,
                cursor,
                result,
            } => f
                .debug_struct("LogLoaded")
                .field("repo_id", repo_id)
                .field("scope", scope)
                .field("cursor", cursor)
                .field("result", result)
                .finish(),
            InternalMsg::TagsLoaded { repo_id, result } => f
                .debug_struct("TagsLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::RemoteTagsLoaded { repo_id, result } => f
                .debug_struct("RemoteTagsLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::StashesLoaded { repo_id, result } => f
                .debug_struct("StashesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::ReflogLoaded { repo_id, result } => f
                .debug_struct("ReflogLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::RebaseStateLoaded { repo_id, result } => f
                .debug_struct("RebaseStateLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::MergeCommitMessageLoaded { repo_id, result } => f
                .debug_struct("MergeCommitMessageLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::FileHistoryLoaded {
                repo_id,
                path,
                result,
            } => f
                .debug_struct("FileHistoryLoaded")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("result", result)
                .finish(),
            InternalMsg::BlameLoaded {
                repo_id,
                path,
                rev,
                result,
            } => f
                .debug_struct("BlameLoaded")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("rev", rev)
                .field("result", result)
                .finish(),
            InternalMsg::ConflictFileLoaded {
                repo_id,
                path,
                result,
                conflict_session,
            } => f
                .debug_struct("ConflictFileLoaded")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("result", result)
                .field("conflict_session", conflict_session)
                .finish(),
            InternalMsg::WorktreesLoaded { repo_id, result } => f
                .debug_struct("WorktreesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::SubmodulesLoaded { repo_id, result } => f
                .debug_struct("SubmodulesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::SubmoduleAddTrustChecked {
                repo_id,
                url,
                path,
                branch,
                name,
                force,
                result,
            } => f
                .debug_struct("SubmoduleAddTrustChecked")
                .field("repo_id", repo_id)
                .field("url", url)
                .field("path", path)
                .field("branch", branch)
                .field("name", name)
                .field("force", force)
                .field("result", result)
                .finish(),
            InternalMsg::SubmoduleUpdateTrustChecked { repo_id, result } => f
                .debug_struct("SubmoduleUpdateTrustChecked")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::CommitDetailsLoaded {
                repo_id,
                commit_id,
                result,
            } => f
                .debug_struct("CommitDetailsLoaded")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .field("result", result)
                .finish(),
            InternalMsg::DiffLoaded {
                repo_id,
                target,
                result,
            } => f
                .debug_struct("DiffLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("result", result)
                .finish(),
            InternalMsg::DiffFileLoaded {
                repo_id,
                target,
                result,
            } => f
                .debug_struct("DiffFileLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("result", result)
                .finish(),
            InternalMsg::DiffPreviewTextFileLoaded {
                repo_id,
                target,
                side,
                result,
            } => f
                .debug_struct("DiffPreviewTextFileLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("side", side)
                .field("result", result)
                .finish(),
            InternalMsg::DiffFileImageLoaded {
                repo_id,
                target,
                result,
            } => f
                .debug_struct("DiffFileImageLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("result", result)
                .finish(),
            InternalMsg::RepoActionFinished { repo_id, result } => f
                .debug_struct("RepoActionFinished")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::CommitFinished { repo_id, result } => f
                .debug_struct("CommitFinished")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::CommitAmendFinished { repo_id, result } => f
                .debug_struct("CommitAmendFinished")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            InternalMsg::RepoCommandFinished {
                repo_id,
                command,
                result,
            } => f
                .debug_struct("RepoCommandFinished")
                .field("repo_id", repo_id)
                .field("command", command)
                .field("result", result)
                .finish(),
        }
    }
}
