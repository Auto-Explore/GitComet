use super::message::Msg;

impl std::fmt::Debug for Msg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Msg::OpenRepo(path) => f.debug_tuple("OpenRepo").field(path).finish(),
            Msg::RestoreSession {
                open_repos,
                active_repo,
            } => f
                .debug_struct("RestoreSession")
                .field("open_repos", open_repos)
                .field("active_repo", active_repo)
                .finish(),
            Msg::CloseRepo { repo_id } => f
                .debug_struct("CloseRepo")
                .field("repo_id", repo_id)
                .finish(),
            Msg::DismissRepoError { repo_id } => f
                .debug_struct("DismissRepoError")
                .field("repo_id", repo_id)
                .finish(),
            Msg::SetActiveRepo { repo_id } => f
                .debug_struct("SetActiveRepo")
                .field("repo_id", repo_id)
                .finish(),
            Msg::ReorderRepoTabs {
                repo_id,
                insert_before,
            } => f
                .debug_struct("ReorderRepoTabs")
                .field("repo_id", repo_id)
                .field("insert_before", insert_before)
                .finish(),
            Msg::ReloadRepo { repo_id } => f
                .debug_struct("ReloadRepo")
                .field("repo_id", repo_id)
                .finish(),
            Msg::RepoExternallyChanged { repo_id, change } => f
                .debug_struct("RepoExternallyChanged")
                .field("repo_id", repo_id)
                .field("change", change)
                .finish(),
            Msg::SetHistoryScope { repo_id, scope } => f
                .debug_struct("SetHistoryScope")
                .field("repo_id", repo_id)
                .field("scope", scope)
                .finish(),
            Msg::SetFetchPruneDeletedRemoteTrackingBranches { repo_id, enabled } => f
                .debug_struct("SetFetchPruneDeletedRemoteTrackingBranches")
                .field("repo_id", repo_id)
                .field("enabled", enabled)
                .finish(),
            Msg::LoadMoreHistory { repo_id } => f
                .debug_struct("LoadMoreHistory")
                .field("repo_id", repo_id)
                .finish(),
            Msg::SelectCommit { repo_id, commit_id } => f
                .debug_struct("SelectCommit")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .finish(),
            Msg::ClearCommitSelection { repo_id } => f
                .debug_struct("ClearCommitSelection")
                .field("repo_id", repo_id)
                .finish(),
            Msg::SelectDiff { repo_id, target } => f
                .debug_struct("SelectDiff")
                .field("repo_id", repo_id)
                .field("target", target)
                .finish(),
            Msg::ClearDiffSelection { repo_id } => f
                .debug_struct("ClearDiffSelection")
                .field("repo_id", repo_id)
                .finish(),
            Msg::LoadStashes { repo_id } => f
                .debug_struct("LoadStashes")
                .field("repo_id", repo_id)
                .finish(),
            Msg::LoadConflictFile { repo_id, path } => f
                .debug_struct("LoadConflictFile")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::LoadReflog { repo_id } => f
                .debug_struct("LoadReflog")
                .field("repo_id", repo_id)
                .finish(),
            Msg::LoadFileHistory {
                repo_id,
                path,
                limit,
            } => f
                .debug_struct("LoadFileHistory")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("limit", limit)
                .finish(),
            Msg::LoadBlame { repo_id, path, rev } => f
                .debug_struct("LoadBlame")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("rev", rev)
                .finish(),
            Msg::LoadWorktrees { repo_id } => f
                .debug_struct("LoadWorktrees")
                .field("repo_id", repo_id)
                .finish(),
            Msg::LoadSubmodules { repo_id } => f
                .debug_struct("LoadSubmodules")
                .field("repo_id", repo_id)
                .finish(),
            Msg::RefreshBranches { repo_id } => f
                .debug_struct("RefreshBranches")
                .field("repo_id", repo_id)
                .finish(),
            Msg::StageHunk { repo_id, patch } => f
                .debug_struct("StageHunk")
                .field("repo_id", repo_id)
                .field("patch_len", &patch.len())
                .finish(),
            Msg::UnstageHunk { repo_id, patch } => f
                .debug_struct("UnstageHunk")
                .field("repo_id", repo_id)
                .field("patch_len", &patch.len())
                .finish(),
            Msg::ApplyWorktreePatch {
                repo_id,
                patch,
                reverse,
            } => f
                .debug_struct("ApplyWorktreePatch")
                .field("repo_id", repo_id)
                .field("reverse", reverse)
                .field("patch_len", &patch.len())
                .finish(),
            Msg::CheckoutBranch { repo_id, name } => f
                .debug_struct("CheckoutBranch")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::CheckoutRemoteBranch {
                repo_id,
                remote,
                branch,
                local_branch,
            } => f
                .debug_struct("CheckoutRemoteBranch")
                .field("repo_id", repo_id)
                .field("remote", remote)
                .field("branch", branch)
                .field("local_branch", local_branch)
                .finish(),
            Msg::CheckoutCommit { repo_id, commit_id } => f
                .debug_struct("CheckoutCommit")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .finish(),
            Msg::CherryPickCommit { repo_id, commit_id } => f
                .debug_struct("CherryPickCommit")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .finish(),
            Msg::RevertCommit { repo_id, commit_id } => f
                .debug_struct("RevertCommit")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .finish(),
            Msg::CreateBranch { repo_id, name } => f
                .debug_struct("CreateBranch")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::CreateBranchAndCheckout { repo_id, name } => f
                .debug_struct("CreateBranchAndCheckout")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::DeleteBranch { repo_id, name } => f
                .debug_struct("DeleteBranch")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::ForceDeleteBranch { repo_id, name } => f
                .debug_struct("ForceDeleteBranch")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::CloneRepo { url, dest } => f
                .debug_struct("CloneRepo")
                .field("url", url)
                .field("dest", dest)
                .finish(),
            Msg::CloneRepoProgress { dest, line } => f
                .debug_struct("CloneRepoProgress")
                .field("dest", dest)
                .field("line", line)
                .finish(),
            Msg::CloneRepoFinished { url, dest, result } => f
                .debug_struct("CloneRepoFinished")
                .field("url", url)
                .field("dest", dest)
                .field("ok", &result.is_ok())
                .finish(),
            Msg::ExportPatch {
                repo_id,
                commit_id,
                dest,
            } => f
                .debug_struct("ExportPatch")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .field("dest", dest)
                .finish(),
            Msg::ApplyPatch { repo_id, patch } => f
                .debug_struct("ApplyPatch")
                .field("repo_id", repo_id)
                .field("patch", patch)
                .finish(),
            Msg::AddWorktree {
                repo_id,
                path,
                reference,
            } => f
                .debug_struct("AddWorktree")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("reference", reference)
                .finish(),
            Msg::RemoveWorktree { repo_id, path } => f
                .debug_struct("RemoveWorktree")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::AddSubmodule { repo_id, url, path } => f
                .debug_struct("AddSubmodule")
                .field("repo_id", repo_id)
                .field("url", url)
                .field("path", path)
                .finish(),
            Msg::UpdateSubmodules { repo_id } => f
                .debug_struct("UpdateSubmodules")
                .field("repo_id", repo_id)
                .finish(),
            Msg::RemoveSubmodule { repo_id, path } => f
                .debug_struct("RemoveSubmodule")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::StagePath { repo_id, path } => f
                .debug_struct("StagePath")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::StagePaths { repo_id, paths } => f
                .debug_struct("StagePaths")
                .field("repo_id", repo_id)
                .field("paths_len", &paths.len())
                .finish(),
            Msg::UnstagePath { repo_id, path } => f
                .debug_struct("UnstagePath")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::UnstagePaths { repo_id, paths } => f
                .debug_struct("UnstagePaths")
                .field("repo_id", repo_id)
                .field("paths_len", &paths.len())
                .finish(),
            Msg::DiscardWorktreeChangesPath { repo_id, path } => f
                .debug_struct("DiscardWorktreeChangesPath")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::DiscardWorktreeChangesPaths { repo_id, paths } => f
                .debug_struct("DiscardWorktreeChangesPaths")
                .field("repo_id", repo_id)
                .field("paths_len", &paths.len())
                .finish(),
            Msg::SaveWorktreeFile {
                repo_id,
                path,
                contents,
                stage,
            } => f
                .debug_struct("SaveWorktreeFile")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("contents_len", &contents.len())
                .field("stage", stage)
                .finish(),
            Msg::Commit { repo_id, message } => f
                .debug_struct("Commit")
                .field("repo_id", repo_id)
                .field("message", message)
                .finish(),
            Msg::CommitAmend { repo_id, message } => f
                .debug_struct("CommitAmend")
                .field("repo_id", repo_id)
                .field("message", message)
                .finish(),
            Msg::FetchAll { repo_id } => f
                .debug_struct("FetchAll")
                .field("repo_id", repo_id)
                .finish(),
            Msg::Pull { repo_id, mode } => f
                .debug_struct("Pull")
                .field("repo_id", repo_id)
                .field("mode", mode)
                .finish(),
            Msg::PullBranch {
                repo_id,
                remote,
                branch,
            } => f
                .debug_struct("PullBranch")
                .field("repo_id", repo_id)
                .field("remote", remote)
                .field("branch", branch)
                .finish(),
            Msg::MergeRef { repo_id, reference } => f
                .debug_struct("MergeRef")
                .field("repo_id", repo_id)
                .field("reference", reference)
                .finish(),
            Msg::Push { repo_id } => f.debug_struct("Push").field("repo_id", repo_id).finish(),
            Msg::ForcePush { repo_id } => f
                .debug_struct("ForcePush")
                .field("repo_id", repo_id)
                .finish(),
            Msg::PushSetUpstream {
                repo_id,
                remote,
                branch,
            } => f
                .debug_struct("PushSetUpstream")
                .field("repo_id", repo_id)
                .field("remote", remote)
                .field("branch", branch)
                .finish(),
            Msg::DeleteRemoteBranch {
                repo_id,
                remote,
                branch,
            } => f
                .debug_struct("DeleteRemoteBranch")
                .field("repo_id", repo_id)
                .field("remote", remote)
                .field("branch", branch)
                .finish(),
            Msg::Reset {
                repo_id,
                target,
                mode,
            } => f
                .debug_struct("Reset")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("mode", mode)
                .finish(),
            Msg::Rebase { repo_id, onto } => f
                .debug_struct("Rebase")
                .field("repo_id", repo_id)
                .field("onto", onto)
                .finish(),
            Msg::RebaseContinue { repo_id } => f
                .debug_struct("RebaseContinue")
                .field("repo_id", repo_id)
                .finish(),
            Msg::RebaseAbort { repo_id } => f
                .debug_struct("RebaseAbort")
                .field("repo_id", repo_id)
                .finish(),
            Msg::MergeAbort { repo_id } => f
                .debug_struct("MergeAbort")
                .field("repo_id", repo_id)
                .finish(),
            Msg::CreateTag {
                repo_id,
                name,
                target,
            } => f
                .debug_struct("CreateTag")
                .field("repo_id", repo_id)
                .field("name", name)
                .field("target", target)
                .finish(),
            Msg::DeleteTag { repo_id, name } => f
                .debug_struct("DeleteTag")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::AddRemote { repo_id, name, url } => f
                .debug_struct("AddRemote")
                .field("repo_id", repo_id)
                .field("name", name)
                .field("url", url)
                .finish(),
            Msg::RemoveRemote { repo_id, name } => f
                .debug_struct("RemoveRemote")
                .field("repo_id", repo_id)
                .field("name", name)
                .finish(),
            Msg::SetRemoteUrl {
                repo_id,
                name,
                url,
                kind,
            } => f
                .debug_struct("SetRemoteUrl")
                .field("repo_id", repo_id)
                .field("name", name)
                .field("url", url)
                .field("kind", kind)
                .finish(),
            Msg::CheckoutConflictSide {
                repo_id,
                path,
                side,
            } => f
                .debug_struct("CheckoutConflictSide")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("side", side)
                .finish(),
            Msg::CheckoutConflictBase { repo_id, path } => f
                .debug_struct("CheckoutConflictBase")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::LaunchMergetool { repo_id, path } => f
                .debug_struct("LaunchMergetool")
                .field("repo_id", repo_id)
                .field("path", path)
                .finish(),
            Msg::RecordConflictAutosolveTelemetry {
                repo_id,
                path,
                mode,
                total_conflicts_before,
                total_conflicts_after,
                unresolved_before,
                unresolved_after,
                stats,
            } => f
                .debug_struct("RecordConflictAutosolveTelemetry")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("mode", mode)
                .field("total_conflicts_before", total_conflicts_before)
                .field("total_conflicts_after", total_conflicts_after)
                .field("unresolved_before", unresolved_before)
                .field("unresolved_after", unresolved_after)
                .field("stats", stats)
                .finish(),
            Msg::ConflictSetHideResolved {
                repo_id,
                path,
                hide_resolved,
            } => f
                .debug_struct("ConflictSetHideResolved")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("hide_resolved", hide_resolved)
                .finish(),
            Msg::ConflictApplyBulkChoice {
                repo_id,
                path,
                choice,
            } => f
                .debug_struct("ConflictApplyBulkChoice")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("choice", choice)
                .finish(),
            Msg::ConflictSetRegionChoice {
                repo_id,
                path,
                region_index,
                choice,
            } => f
                .debug_struct("ConflictSetRegionChoice")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("region_index", region_index)
                .field("choice", choice)
                .finish(),
            Msg::ConflictApplyAutosolve {
                repo_id,
                path,
                mode,
                whitespace_normalize,
            } => f
                .debug_struct("ConflictApplyAutosolve")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("mode", mode)
                .field("whitespace_normalize", whitespace_normalize)
                .finish(),
            Msg::Stash {
                repo_id,
                message,
                include_untracked,
            } => f
                .debug_struct("Stash")
                .field("repo_id", repo_id)
                .field("message", message)
                .field("include_untracked", include_untracked)
                .finish(),
            Msg::ApplyStash { repo_id, index } => f
                .debug_struct("ApplyStash")
                .field("repo_id", repo_id)
                .field("index", index)
                .finish(),
            Msg::DropStash { repo_id, index } => f
                .debug_struct("DropStash")
                .field("repo_id", repo_id)
                .field("index", index)
                .finish(),
            Msg::PopStash { repo_id, index } => f
                .debug_struct("PopStash")
                .field("repo_id", repo_id)
                .field("index", index)
                .finish(),
            Msg::RepoOpenedOk { repo_id, spec, .. } => f
                .debug_struct("RepoOpenedOk")
                .field("repo_id", repo_id)
                .field("spec", spec)
                .finish_non_exhaustive(),
            Msg::RepoOpenedErr {
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
            Msg::BranchesLoaded { repo_id, result } => f
                .debug_struct("BranchesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::RemotesLoaded { repo_id, result } => f
                .debug_struct("RemotesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::RemoteBranchesLoaded { repo_id, result } => f
                .debug_struct("RemoteBranchesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::StatusLoaded { repo_id, result } => f
                .debug_struct("StatusLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::HeadBranchLoaded { repo_id, result } => f
                .debug_struct("HeadBranchLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::UpstreamDivergenceLoaded { repo_id, result } => f
                .debug_struct("UpstreamDivergenceLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::LogLoaded {
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
            Msg::TagsLoaded { repo_id, result } => f
                .debug_struct("TagsLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::StashesLoaded { repo_id, result } => f
                .debug_struct("StashesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::ReflogLoaded { repo_id, result } => f
                .debug_struct("ReflogLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::RebaseStateLoaded { repo_id, result } => f
                .debug_struct("RebaseStateLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::MergeCommitMessageLoaded { repo_id, result } => f
                .debug_struct("MergeCommitMessageLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::FileHistoryLoaded {
                repo_id,
                path,
                result,
            } => f
                .debug_struct("FileHistoryLoaded")
                .field("repo_id", repo_id)
                .field("path", path)
                .field("result", result)
                .finish(),
            Msg::BlameLoaded {
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
            Msg::ConflictFileLoaded {
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
            Msg::WorktreesLoaded { repo_id, result } => f
                .debug_struct("WorktreesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::SubmodulesLoaded { repo_id, result } => f
                .debug_struct("SubmodulesLoaded")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::CommitDetailsLoaded {
                repo_id,
                commit_id,
                result,
            } => f
                .debug_struct("CommitDetailsLoaded")
                .field("repo_id", repo_id)
                .field("commit_id", commit_id)
                .field("result", result)
                .finish(),
            Msg::DiffLoaded {
                repo_id,
                target,
                result,
            } => f
                .debug_struct("DiffLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("result", result)
                .finish(),
            Msg::DiffFileLoaded {
                repo_id,
                target,
                result,
            } => f
                .debug_struct("DiffFileLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("result", result)
                .finish(),
            Msg::DiffFileImageLoaded {
                repo_id,
                target,
                result,
            } => f
                .debug_struct("DiffFileImageLoaded")
                .field("repo_id", repo_id)
                .field("target", target)
                .field("result", result)
                .finish(),
            Msg::RepoActionFinished { repo_id, result } => f
                .debug_struct("RepoActionFinished")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::CommitFinished { repo_id, result } => f
                .debug_struct("CommitFinished")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::CommitAmendFinished { repo_id, result } => f
                .debug_struct("CommitAmendFinished")
                .field("repo_id", repo_id)
                .field("result", result)
                .finish(),
            Msg::RepoCommandFinished {
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
