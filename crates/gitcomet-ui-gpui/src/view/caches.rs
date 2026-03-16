use super::*;
use gitcomet_core::domain::LogScope;

#[derive(Clone, Debug)]
pub(super) struct HistoryCache {
    pub(super) request: HistoryCacheRequest,
    pub(super) visible_indices: Vec<usize>,
    pub(super) graph_rows: Vec<Arc<history_graph::GraphRow>>,
    pub(super) commit_row_vms: Vec<HistoryCommitRowVm>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct HistoryCacheRequest {
    pub(super) repo_id: RepoId,
    pub(super) history_scope: LogScope,
    pub(super) log_fingerprint: u64,
    pub(super) head_branch_rev: u64,
    pub(super) detached_head_commit: Option<CommitId>,
    pub(super) branches_rev: u64,
    pub(super) remote_branches_rev: u64,
    pub(super) tags_rev: u64,
    pub(super) stashes_rev: u64,
    pub(super) date_time_format: DateTimeFormat,
    pub(super) timezone: Timezone,
    pub(super) show_timezone: bool,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryCommitRowVm {
    pub(super) branches_text: SharedString,
    pub(super) tag_names: Arc<[SharedString]>,
    pub(super) author: SharedString,
    pub(super) summary: SharedString,
    pub(super) when: SharedString,
    pub(super) short_sha: SharedString,
    pub(super) is_head: bool,
    pub(super) is_stash: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct BranchSidebarFingerprint {
    head_branch_rev: u64,
    branches_rev: u64,
    remotes_rev: u64,
    remote_branches_rev: u64,
    worktrees_rev: u64,
    submodules_rev: u64,
    stashes_rev: u64,
}

impl BranchSidebarFingerprint {
    pub(super) fn from_repo(repo: &RepoState) -> Self {
        Self {
            head_branch_rev: repo.head_branch_rev,
            branches_rev: repo.branches_rev,
            remotes_rev: repo.remotes_rev,
            remote_branches_rev: repo.remote_branches_rev,
            worktrees_rev: repo.worktrees_rev,
            submodules_rev: repo.submodules_rev,
            stashes_rev: repo.stashes_rev,
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct BranchSidebarCache {
    pub(super) repo_id: RepoId,
    pub(super) fingerprint: BranchSidebarFingerprint,
    pub(super) rows: Arc<[BranchSidebarRow]>,
}

#[derive(Clone, Debug)]
pub(super) struct HistoryWorktreeSummaryCache {
    pub(super) repo_id: RepoId,
    pub(super) status: Arc<RepoStatus>,
    pub(super) show_row: bool,
    pub(super) counts: (usize, usize, usize),
}

#[derive(Clone, Debug)]
pub(super) struct HistoryStashIdsCache {
    pub(super) repo_id: RepoId,
    pub(super) stashes_rev: u64,
    pub(super) ids: Arc<HashSet<CommitId>>,
}

impl GitCometView {
    #[cfg(any(test, feature = "benchmarks"))]
    pub(super) fn branch_sidebar_rows(repo: &RepoState) -> Vec<BranchSidebarRow> {
        branch_sidebar::branch_sidebar_rows(repo)
    }
}
