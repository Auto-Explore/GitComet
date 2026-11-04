//! Whether the commit box may switch amend on: there must be a commit to amend
//! and no commit, merge or rebase in the way.

use gitcomet_state::model::{Loadable, RepoState};

fn merge_active(repo: &RepoState) -> bool {
    matches!(&repo.merge_commit_message, Loadable::Ready(Some(_)))
}

fn repo_has_head_commit(repo: &RepoState) -> bool {
    if repo.detached_head_commit.is_some() {
        return true;
    }

    match &repo.head_branch {
        Loadable::Ready(head) if head == "HEAD" => true,
        Loadable::Ready(head) => match &repo.branches {
            Loadable::Ready(branches) => branches.iter().any(|branch| branch.name == *head),
            _ => true,
        },
        _ => true,
    }
}

pub(super) fn can_amend(repo: Option<&RepoState>) -> bool {
    let Some(repo) = repo else {
        return false;
    };

    repo.commit_in_flight == 0
        && !merge_active(repo)
        && !matches!(repo.rebase_in_progress, Loadable::Ready(true))
        && repo_has_head_commit(repo)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gitcomet_core::domain::{Branch, CommitId, LogPage, RepoSpec};
    use gitcomet_state::model::RepoId;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn repo_state() -> RepoState {
        RepoState::new_opening(
            RepoId(1),
            RepoSpec {
                workdir: PathBuf::from("/tmp/repo"),
            },
        )
    }

    fn branch(name: &str, target: &str) -> Branch {
        Branch {
            name: name.to_string(),
            target: CommitId(target.into()),
            upstream: None,
            divergence: None,
        }
    }

    #[test]
    fn amend_needs_a_repository() {
        assert!(!can_amend(None));
    }

    #[test]
    fn amend_allowed_when_filtered_log_is_empty_but_head_branch_exists() {
        let mut repo = repo_state();
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "abc123")]));
        repo.log = Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }));

        assert!(can_amend(Some(&repo)));
    }

    #[test]
    fn amend_blocked_on_unborn_head_branch() {
        let mut repo = repo_state();
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(Vec::new()));
        repo.log = Loadable::Ready(Arc::new(LogPage {
            commits: Vec::new(),
            next_cursor: None,
        }));

        assert!(!can_amend(Some(&repo)));
    }

    #[test]
    fn amend_blocked_while_a_commit_is_in_flight() {
        let mut repo = repo_state();
        repo.head_branch = Loadable::Ready("main".to_string());
        repo.branches = Loadable::Ready(Arc::new(vec![branch("main", "abc123")]));
        repo.commit_in_flight = 1;

        assert!(!can_amend(Some(&repo)));
    }
}
