//! Branch and worktree lookups behind branch rows: checked-out paths,
//! click targets and selection.

use super::*;

pub(in crate::view) fn listed_worktree_paths_by_branch(
    repo: &RepoState,
) -> FxHashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return FxHashMap::default();
    };

    let mut worktree_paths = FxHashMap::default();
    for worktree in worktrees.iter() {
        if worktree.path == repo.spec.workdir {
            continue;
        }

        let Some(branch) = worktree.branch.clone() else {
            continue;
        };

        worktree_paths
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    worktree_paths
}

pub(super) fn branch_worktree_badge_path(
    listed_worktree_path: Option<&std::path::Path>,
    active_worktree_path: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    listed_worktree_path
        .map(std::path::Path::to_path_buf)
        .or_else(|| active_worktree_path.map(std::path::Path::to_path_buf))
}

pub(in crate::view) fn active_worktree_paths_by_branch(
    repo: &RepoState,
    open_repos: &[RepoState],
) -> FxHashMap<String, std::path::PathBuf> {
    let Loadable::Ready(worktrees) = &repo.worktrees else {
        return FxHashMap::default();
    };

    // Preserve the first open repository for a path, as the former linear
    // lookup did, while avoiding a worktrees × open-repositories scan.
    let mut open_by_path = FxHashMap::default();
    for open_repo in open_repos {
        open_by_path
            .entry(&open_repo.spec.workdir)
            .or_insert(open_repo);
    }
    let mut active_worktrees = FxHashMap::default();
    for worktree in worktrees.iter() {
        let Some(open_repo) = open_by_path.get(&worktree.path) else {
            continue;
        };

        let branch = if open_repo.detached_head_commit.is_some() {
            None
        } else {
            match &open_repo.head_branch {
                Loadable::Ready(head_branch) if head_branch != "HEAD" => Some(head_branch.clone()),
                Loadable::Ready(_) => None,
                _ => worktree.branch.clone(),
            }
        };
        let Some(branch) = branch else {
            continue;
        };

        active_worktrees
            .entry(branch)
            .or_insert_with(|| worktree.path.clone());
    }

    active_worktrees
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum LocalBranchDoubleClickAction {
    CheckoutBranch { name: String },
    OpenWorktree { path: std::path::PathBuf },
}

pub(super) fn local_branch_double_click_action(
    branch: &str,
    badge_worktree_path: Option<&std::path::Path>,
) -> LocalBranchDoubleClickAction {
    match badge_worktree_path {
        Some(path) => LocalBranchDoubleClickAction::OpenWorktree {
            path: path.to_path_buf(),
        },
        None => LocalBranchDoubleClickAction::CheckoutBranch {
            name: branch.to_string(),
        },
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct BranchHistoryRevealTarget {
    pub(super) commit_id: CommitId,
    pub(super) fallback_scope: Option<LogScope>,
}

pub(in crate::view) fn branch_commit_id(
    repo: &RepoState,
    target: &BranchMenuTarget,
) -> Option<CommitId> {
    match target {
        BranchMenuTarget::Local { name } => match &repo.branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|branch| branch.name == *name)
                .map(|branch| branch.target.clone()),
            _ => None,
        },
        BranchMenuTarget::Remote { remote, branch } => match &repo.remote_branches {
            Loadable::Ready(branches) => branches
                .iter()
                .find(|candidate| candidate.remote == *remote && candidate.name == *branch)
                .map(|candidate| candidate.target.clone()),
            _ => None,
        },
    }
}

pub(super) fn branch_click_history_reveal_target(
    repo: &RepoState,
    target: &BranchMenuTarget,
    is_head: bool,
) -> Option<BranchHistoryRevealTarget> {
    let commit_id = branch_commit_id(repo, target)?;

    let fallback_scope = match target.section() {
        BranchSection::Local if is_head => Some(LogScope::FullReachable),
        BranchSection::Local | BranchSection::Remote => Some(LogScope::AllBranches),
    };

    Some(BranchHistoryRevealTarget {
        commit_id,
        fallback_scope,
    })
}

pub(super) fn branch_row_is_selected(
    selected_branch: Option<&SelectedBranch>,
    repo_id: RepoId,
    target: &BranchMenuTarget,
    selected_commit: Option<&CommitId>,
    selected_branch_commit_id: Option<&CommitId>,
) -> bool {
    selected_branch.is_some_and(|selected_branch| {
        selected_branch.repo_id == repo_id
            && selected_branch.target == *target
            && selected_branch_commit_id.is_some_and(|commit_id| selected_commit == Some(commit_id))
    })
}
