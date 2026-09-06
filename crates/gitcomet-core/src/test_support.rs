//! Shared test doubles for crates that consume the Git service contracts.
//!
//! Production implementations deliberately have to implement every required
//! [`GitRepository`] operation. Tests that only need a repository to exist —
//! to be handed to a reducer, or to stand in a backend map — use
//! [`UnconfiguredRepository`], which answers `Unsupported` to everything but
//! [`GitRepository::spec`].
//!
//! A test that needs real behavior implements [`GitRepository`] directly rather
//! than configuring a double: the trait's own provided methods already supply
//! the unsupported fallbacks, so only the required operations have to be
//! spelled out.

use crate::domain::{
    Branch, CommitDetails, CommitId, DiffTarget, LogCursor, LogPage, ReflogEntry, Remote,
    RemoteBranch, RepoSpec, RepoStatus, StashEntry,
};
use crate::error::{Error, ErrorKind};
use crate::services::{GitRepository, PullMode, Result};
use std::path::{Path, PathBuf};

fn unsupported<T>() -> Result<T> {
    Err(Error::new(ErrorKind::Unsupported(
        "test repository: this operation is not configured",
    )))
}

/// A repository that exists but does nothing.
///
/// Every required operation answers `Unsupported`; every provided operation
/// keeps the fallback [`GitRepository`] defines for it. Useful wherever a test
/// needs a `dyn GitRepository` for its identity or its [`RepoSpec`] and never
/// calls through it.
pub struct UnconfiguredRepository {
    spec: RepoSpec,
}

impl UnconfiguredRepository {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            spec: RepoSpec {
                workdir: workdir.into(),
            },
        }
    }
}

impl GitRepository for UnconfiguredRepository {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(
        &self,
        _limit: usize,
        _cursor: Option<&LogCursor>,
    ) -> Result<std::sync::Arc<LogPage>> {
        unsupported()
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported()
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported()
    }

    fn current_branch(&self) -> Result<String> {
        unsupported()
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        unsupported()
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unsupported()
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unsupported()
    }

    fn status(&self) -> Result<RepoStatus> {
        unsupported()
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported()
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported()
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported()
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported()
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported()
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported()
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        unsupported()
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported()
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unsupported()
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported()
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported()
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported()
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported()
    }

    fn commit(&self, _message: &str) -> Result<()> {
        unsupported()
    }

    fn fetch_all(&self) -> Result<()> {
        unsupported()
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        unsupported()
    }

    fn push(&self) -> Result<()> {
        unsupported()
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported()
    }
}

#[cfg(test)]
mod tests {
    use super::UnconfiguredRepository;
    use crate::error::ErrorKind;
    use crate::services::GitRepository;
    use std::path::PathBuf;

    #[test]
    fn unconfigured_repository_reports_its_spec_and_refuses_everything_else() {
        let repository = UnconfiguredRepository::new("/tmp/unconfigured");
        assert_eq!(
            repository.spec().workdir,
            PathBuf::from("/tmp/unconfigured")
        );
        let error = repository
            .current_branch()
            .expect_err("an unconfigured operation must be unsupported");
        assert!(matches!(error.kind(), ErrorKind::Unsupported(_)));
    }
}
