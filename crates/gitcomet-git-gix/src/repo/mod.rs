use crate::util::git_workdir_cmd_for as util_git_workdir_cmd_for;
use gitcomet_core::conflict_session::ConflictSession;
use gitcomet_core::domain::{
    Branch, CommitDetails, CommitId, Diff, DiffPreviewTextSide, DiffTarget, FileDiffImage,
    FileDiffText, LogCursor, LogPage, ReflogEntry, Remote, RemoteBranch, RemoteTag, RepoSpec,
    RepoStatus, StashEntry, Submodule, Tag, UpstreamDivergence, Worktree,
};
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::git_ops_trace::{self, GitOpTraceKind};
use gitcomet_core::services::{
    BlameLine, CommandOutput, ConflictFileStages, ConflictSide, GitRepository, MergetoolResult,
    PullMode, RemoteUrlKind, ResetMode, Result,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

/// Convert a gix ObjectId to an `Arc<str>` hex string without intermediate String allocation.
/// Uses a stack buffer + `hex_to_buf` → `Arc::from(&str)` (one heap allocation instead of two).
#[inline]
pub(super) fn oid_to_arc_str(oid: &gix::oid) -> Arc<str> {
    let mut buf = gix::hash::Kind::hex_buf();
    let hex: &str = oid.hex_to_buf(&mut buf);
    Arc::from(hex)
}

/// Convert bytes to `Arc<str>`, avoiding an intermediate String allocation when the input is
/// valid UTF-8 (the common case for git commit metadata).
#[inline]
pub(super) fn bstr_to_arc_str(bytes: &[u8]) -> Arc<str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Arc::from(s),
        Err(_) => Arc::from(String::from_utf8_lossy(bytes).as_ref()),
    }
}

mod blame;
mod conflict_stages;
mod diff;
mod discard;
mod git_ops;
mod history;
mod log;
mod mergetool;
mod patch;
mod porcelain;
mod remotes;
mod status;
mod submodules;
mod tags;
mod worktrees;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct RepoFileStamp {
    exists: bool,
    len: u64,
    modified: Option<std::time::SystemTime>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GitlinkStatusCapabilityCacheEntry {
    gitmodules: RepoFileStamp,
    index: RepoFileStamp,
    may_have_gitlinks: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct BranchTrackingConfigCacheEntry {
    local_config: RepoFileStamp,
    worktree_config: RepoFileStamp,
    has_branch_sections: bool,
}

/// Caches the Tree→Index (HEAD vs index) result so that background refresh
/// cycles can skip the tree comparison when HEAD and the index file are
/// unchanged since the last status call.
#[derive(Clone, Debug)]
struct TreeIndexCacheEntry {
    head_oid: Option<gix::ObjectId>,
    index_stamp: RepoFileStamp,
    staged: Vec<gitcomet_core::domain::FileStatus>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct LogHeadPageCacheKey {
    head_oid: Option<gix::ObjectId>,
    limit: usize,
    last_seen: Option<CommitId>,
    resume_from: Option<CommitId>,
}

#[derive(Clone, Debug)]
struct LogHeadPageCacheEntry {
    key: LogHeadPageCacheKey,
    page: LogPage,
}

const LOG_HEAD_PAGE_CACHE_LIMIT: usize = 32;

pub(crate) struct GixRepo {
    spec: RepoSpec,
    _repo: gix::ThreadSafeRepository,
    gitlink_status_capability: std::sync::Mutex<Option<GitlinkStatusCapabilityCacheEntry>>,
    branch_tracking_config: std::sync::Mutex<Option<BranchTrackingConfigCacheEntry>>,
    tree_index_cache: std::sync::Mutex<Option<TreeIndexCacheEntry>>,
    log_head_page_cache: std::sync::Mutex<Vec<LogHeadPageCacheEntry>>,
}

impl GixRepo {
    pub(crate) fn new(workdir: PathBuf, repo: gix::ThreadSafeRepository) -> Self {
        Self {
            spec: RepoSpec { workdir },
            _repo: repo,
            gitlink_status_capability: std::sync::Mutex::new(None),
            branch_tracking_config: std::sync::Mutex::new(None),
            tree_index_cache: std::sync::Mutex::new(None),
            log_head_page_cache: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Returns a `Command` pre-configured with `git -C <workdir>`.
    pub(super) fn git_workdir_cmd(&self) -> Command {
        util_git_workdir_cmd_for(&self.spec.workdir)
    }

    pub(super) fn reopen_repo(&self) -> Result<gix::Repository> {
        gix::open(&self.spec.workdir).map_err(|e| match e {
            gix::open::Error::NotARepository { .. } => Error::new(ErrorKind::NotARepository),
            gix::open::Error::Io(io) => Error::new(ErrorKind::Io(io.kind())),
            e => Error::new(ErrorKind::Backend(format!("gix open fresh repo: {e}"))),
        })
    }
}

pub(crate) fn allow_test_repo_local_mergetool_command(workdir: &Path, tool_name: &str) {
    mergetool::allow_test_repo_local_mergetool_command(workdir, tool_name);
}

impl GitRepository for GixRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_head_page_impl(limit, cursor)
    }

    fn log_all_branches_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_all_branches_page_impl(limit, cursor)
    }

    fn log_file_page(
        &self,
        path: &Path,
        limit: usize,
        cursor: Option<&LogCursor>,
    ) -> Result<LogPage> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::LogWalk);
        self.log_file_page_impl(path, limit, cursor)
    }

    fn commit_details(&self, id: &CommitId) -> Result<CommitDetails> {
        self.commit_details_impl(id)
    }

    fn reflog_head(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        self.reflog_head_impl(limit)
    }

    fn current_branch(&self) -> Result<String> {
        self.current_branch_impl()
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_branches_impl()
    }

    fn list_tags(&self) -> Result<Vec<Tag>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_tags_impl()
    }

    fn list_remote_tags(&self) -> Result<Vec<RemoteTag>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remote_tags_impl()
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remotes_impl()
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::RefEnumerate);
        self.list_remote_branches_impl()
    }

    fn status(&self) -> Result<RepoStatus> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Status);
        self.status_impl()
    }

    fn upstream_divergence(&self) -> Result<Option<UpstreamDivergence>> {
        self.upstream_divergence_impl()
    }

    fn pull_branch_with_output(&self, remote: &str, branch: &str) -> Result<CommandOutput> {
        self.pull_branch_with_output_impl(remote, branch)
    }

    fn merge_ref_with_output(&self, reference: &str) -> Result<CommandOutput> {
        self.merge_ref_with_output_impl(reference)
    }

    fn squash_ref_with_output(&self, reference: &str) -> Result<CommandOutput> {
        self.squash_ref_with_output_impl(reference)
    }

    fn diff_unified(&self, target: &DiffTarget) -> Result<String> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Diff);
        self.diff_unified_impl(target)
    }

    fn diff_parsed(&self, target: &DiffTarget) -> Result<Diff> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Diff);
        self.diff_parsed_impl(target)
    }

    fn diff_file_text(&self, target: &DiffTarget) -> Result<Option<FileDiffText>> {
        self.diff_file_text_impl(target)
    }

    fn diff_preview_text_file(
        &self,
        target: &DiffTarget,
        side: DiffPreviewTextSide,
    ) -> Result<Option<PathBuf>> {
        self.diff_preview_text_file_impl(target, side)
    }

    fn diff_file_image(&self, target: &DiffTarget) -> Result<Option<FileDiffImage>> {
        self.diff_file_image_impl(target)
    }

    fn conflict_file_stages(&self, path: &Path) -> Result<Option<ConflictFileStages>> {
        self.conflict_file_stages_impl(path)
    }

    fn conflict_session(&self, path: &Path) -> Result<Option<ConflictSession>> {
        self.conflict_session_impl(path)
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
        self.create_branch_impl(name, target)
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        self.delete_branch_impl(name)
    }

    fn delete_branch_force(&self, name: &str) -> Result<()> {
        self.delete_branch_force_impl(name)
    }

    fn checkout_branch(&self, name: &str) -> Result<()> {
        self.checkout_branch_impl(name)
    }

    fn checkout_remote_branch(&self, remote: &str, branch: &str, local_branch: &str) -> Result<()> {
        self.checkout_remote_branch_impl(remote, branch, local_branch)
    }

    fn checkout_commit(&self, id: &CommitId) -> Result<()> {
        self.checkout_commit_impl(id)
    }

    fn cherry_pick(&self, id: &CommitId) -> Result<()> {
        self.cherry_pick_impl(id)
    }

    fn revert(&self, id: &CommitId) -> Result<()> {
        self.revert_impl(id)
    }

    fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
        self.stash_create_impl(message, include_untracked)
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        self.stash_list_impl()
    }

    fn stash_apply(&self, index: usize) -> Result<()> {
        self.stash_apply_impl(index)
    }

    fn stash_drop(&self, index: usize) -> Result<()> {
        self.stash_drop_impl(index)
    }

    fn stage(&self, paths: &[&Path]) -> Result<()> {
        self.stage_impl(paths)
    }

    fn unstage(&self, paths: &[&Path]) -> Result<()> {
        self.unstage_impl(paths)
    }

    fn commit(&self, message: &str) -> Result<()> {
        self.commit_impl(message)
    }

    fn commit_amend(&self, message: &str) -> Result<()> {
        self.commit_amend_impl(message)
    }

    fn fetch_all(&self) -> Result<()> {
        self.fetch_all_impl(true)
    }

    fn fetch_all_with_output(&self) -> Result<CommandOutput> {
        self.fetch_all_with_output_impl(true)
    }

    fn fetch_all_with_output_prune(&self, prune: bool) -> Result<CommandOutput> {
        self.fetch_all_with_output_impl(prune)
    }

    fn pull(&self, mode: PullMode) -> Result<()> {
        self.pull_impl(mode)
    }

    fn pull_with_output(&self, mode: PullMode) -> Result<CommandOutput> {
        self.pull_with_output_impl(mode)
    }

    fn push(&self) -> Result<()> {
        self.push_impl()
    }

    fn push_with_output(&self) -> Result<CommandOutput> {
        self.push_with_output_impl()
    }

    fn push_force(&self) -> Result<()> {
        self.push_force_impl()
    }

    fn push_force_with_output(&self) -> Result<CommandOutput> {
        self.push_force_with_output_impl()
    }

    fn reset_with_output(&self, target: &str, mode: ResetMode) -> Result<CommandOutput> {
        self.reset_with_output_impl(target, mode)
    }

    fn rebase_with_output(&self, onto: &str) -> Result<CommandOutput> {
        self.rebase_with_output_impl(onto)
    }

    fn rebase_continue_with_output(&self) -> Result<CommandOutput> {
        self.rebase_continue_with_output_impl()
    }

    fn rebase_abort_with_output(&self) -> Result<CommandOutput> {
        self.rebase_abort_with_output_impl()
    }

    fn merge_abort_with_output(&self) -> Result<CommandOutput> {
        self.merge_abort_with_output_impl()
    }

    fn rebase_in_progress(&self) -> Result<bool> {
        self.rebase_in_progress_impl()
    }

    fn merge_commit_message(&self) -> Result<Option<String>> {
        self.merge_commit_message_impl()
    }

    fn create_tag_with_output(&self, name: &str, target: &str) -> Result<CommandOutput> {
        self.create_tag_with_output_impl(name, target)
    }

    fn delete_tag_with_output(&self, name: &str) -> Result<CommandOutput> {
        self.delete_tag_with_output_impl(name)
    }

    fn prune_merged_branches_with_output(&self) -> Result<CommandOutput> {
        self.prune_merged_branches_with_output_impl()
    }

    fn prune_local_tags_with_output(&self) -> Result<CommandOutput> {
        self.prune_local_tags_with_output_impl()
    }

    fn push_tag_with_output(&self, remote: &str, name: &str) -> Result<CommandOutput> {
        self.push_tag_with_output_impl(remote, name)
    }

    fn delete_remote_tag_with_output(&self, remote: &str, name: &str) -> Result<CommandOutput> {
        self.delete_remote_tag_with_output_impl(remote, name)
    }

    fn add_remote_with_output(&self, name: &str, url: &str) -> Result<CommandOutput> {
        self.add_remote_with_output_impl(name, url)
    }

    fn remove_remote_with_output(&self, name: &str) -> Result<CommandOutput> {
        self.remove_remote_with_output_impl(name)
    }

    fn set_remote_url_with_output(
        &self,
        name: &str,
        url: &str,
        kind: RemoteUrlKind,
    ) -> Result<CommandOutput> {
        self.set_remote_url_with_output_impl(name, url, kind)
    }

    fn push_set_upstream(&self, remote: &str, branch: &str) -> Result<()> {
        self.push_set_upstream_impl(remote, branch)
    }

    fn push_set_upstream_with_output(&self, remote: &str, branch: &str) -> Result<CommandOutput> {
        self.push_set_upstream_with_output_impl(remote, branch)
    }

    fn set_upstream_branch_with_output(
        &self,
        branch: &str,
        upstream: &str,
    ) -> Result<CommandOutput> {
        self.set_upstream_branch_with_output_impl(branch, upstream)
    }

    fn unset_upstream_branch_with_output(&self, branch: &str) -> Result<CommandOutput> {
        self.unset_upstream_branch_with_output_impl(branch)
    }

    fn delete_remote_branch_with_output(
        &self,
        remote: &str,
        branch: &str,
    ) -> Result<CommandOutput> {
        self.delete_remote_branch_with_output_impl(remote, branch)
    }

    fn blame_file(&self, path: &Path, rev: Option<&str>) -> Result<Vec<BlameLine>> {
        let _scope = git_ops_trace::scope(GitOpTraceKind::Blame);
        self.blame_file_impl(path, rev)
    }

    fn checkout_conflict_side(&self, path: &Path, side: ConflictSide) -> Result<CommandOutput> {
        self.checkout_conflict_side_impl(path, side)
    }

    fn accept_conflict_deletion(&self, path: &Path) -> Result<CommandOutput> {
        self.accept_conflict_deletion_impl(path)
    }

    fn checkout_conflict_base(&self, path: &Path) -> Result<CommandOutput> {
        self.checkout_conflict_base_impl(path)
    }

    fn launch_mergetool(&self, path: &Path) -> Result<MergetoolResult> {
        self.launch_mergetool_impl(path)
    }

    fn export_patch_with_output(&self, commit_id: &CommitId, dest: &Path) -> Result<CommandOutput> {
        self.export_patch_with_output_impl(commit_id, dest)
    }

    fn apply_patch_with_output(&self, patch: &Path) -> Result<CommandOutput> {
        self.apply_patch_with_output_impl(patch)
    }

    fn apply_unified_patch_to_index_with_output(
        &self,
        patch: &str,
        reverse: bool,
    ) -> Result<CommandOutput> {
        self.apply_unified_patch_to_index_with_output_impl(patch, reverse)
    }

    fn apply_unified_patch_to_worktree_with_output(
        &self,
        patch: &str,
        reverse: bool,
    ) -> Result<CommandOutput> {
        self.apply_unified_patch_to_worktree_with_output_impl(patch, reverse)
    }

    fn list_worktrees(&self) -> Result<Vec<Worktree>> {
        self.list_worktrees_impl()
    }

    fn add_worktree_with_output(
        &self,
        path: &Path,
        reference: Option<&str>,
    ) -> Result<CommandOutput> {
        self.add_worktree_with_output_impl(path, reference)
    }

    fn remove_worktree_with_output(&self, path: &Path) -> Result<CommandOutput> {
        self.remove_worktree_with_output_impl(path)
    }

    fn force_remove_worktree_with_output(&self, path: &Path) -> Result<CommandOutput> {
        self.force_remove_worktree_with_output_impl(path)
    }

    fn list_submodules(&self) -> Result<Vec<Submodule>> {
        self.list_submodules_impl()
    }

    fn add_submodule_with_output(&self, url: &str, path: &Path) -> Result<CommandOutput> {
        self.add_submodule_with_output_impl(url, path)
    }

    fn update_submodules_with_output(&self) -> Result<CommandOutput> {
        self.update_submodules_with_output_impl()
    }

    fn remove_submodule_with_output(&self, path: &Path) -> Result<CommandOutput> {
        self.remove_submodule_with_output_impl(path)
    }

    fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()> {
        self.discard_worktree_changes_impl(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oid_to_arc_str_round_trips_hex_object_id() {
        let expected = "0123456789abcdef0123456789abcdef01234567";
        let oid = gix::ObjectId::from_hex(expected.as_bytes()).expect("valid object id");

        assert_eq!(oid_to_arc_str(oid.as_ref()).as_ref(), expected);
    }

    #[test]
    fn bstr_to_arc_str_preserves_utf8_bytes() {
        assert_eq!(
            bstr_to_arc_str("hello git".as_bytes()).as_ref(),
            "hello git"
        );
    }

    #[test]
    fn bstr_to_arc_str_uses_lossy_conversion_for_invalid_utf8() {
        assert_eq!(bstr_to_arc_str(b"foo\x80bar").as_ref(), "foo\u{fffd}bar");
    }
}
