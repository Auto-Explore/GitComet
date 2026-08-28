//! Shared test doubles for crates that consume the Git service contracts.
//!
//! Production implementations deliberately have to implement every required
//! [`GitRepository`] operation. Tests usually care about one or two operations,
//! so [`RepositoryDouble`] supplies explicit unsupported defaults and adapts to
//! the production trait in one place.

use crate::domain::{
    Branch, CommitDetails, CommitId, DiffTarget, LogCursor, LogPage, ReflogEntry, Remote,
    RemoteBranch, RepoSpec, RepoStatus, StashEntry,
};
use crate::error::{Error, ErrorKind};
use crate::services::{GitBackend, GitRepository, PullMode, Result};
use std::path::Path;
use std::sync::Arc;

fn unsupported<T>(operation: &'static str) -> Result<T> {
    Err(Error::new(ErrorKind::Unsupported(operation)))
}

/// Minimal repository contract for tests.
///
/// Implement `spec` and only the operations relevant to the test. The blanket
/// implementation below turns the value into a full [`GitRepository`].
pub trait RepositoryDouble: Send + Sync {
    fn spec(&self) -> &RepoSpec;

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        unsupported("test repository: log_head_page is not configured")
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        unsupported("test repository: commit_details is not configured")
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        unsupported("test repository: reflog_head is not configured")
    }

    fn current_branch(&self) -> Result<String> {
        unsupported("test repository: current_branch is not configured")
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        unsupported("test repository: list_branches is not configured")
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        unsupported("test repository: list_remotes is not configured")
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        unsupported("test repository: list_remote_branches is not configured")
    }

    fn status(&self) -> Result<RepoStatus> {
        unsupported("test repository: status is not configured")
    }

    fn head_path_is_gitlink(&self, _path: &Path) -> Result<bool> {
        Ok(false)
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        unsupported("test repository: diff_unified is not configured")
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        unsupported("test repository: create_branch is not configured")
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        unsupported("test repository: delete_branch is not configured")
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        unsupported("test repository: checkout_branch is not configured")
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        unsupported("test repository: checkout_commit is not configured")
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        unsupported("test repository: cherry_pick is not configured")
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        unsupported("test repository: revert is not configured")
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        unsupported("test repository: stash_create is not configured")
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        unsupported("test repository: stash_list is not configured")
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        unsupported("test repository: stash_apply is not configured")
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        unsupported("test repository: stash_drop is not configured")
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported("test repository: stage is not configured")
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        unsupported("test repository: unstage is not configured")
    }

    fn commit(&self, _message: &str) -> Result<()> {
        unsupported("test repository: commit is not configured")
    }

    fn fetch_all(&self) -> Result<()> {
        unsupported("test repository: fetch_all is not configured")
    }

    fn pull(&self, _mode: PullMode) -> Result<()> {
        unsupported("test repository: pull is not configured")
    }

    fn push(&self) -> Result<()> {
        unsupported("test repository: push is not configured")
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        unsupported("test repository: discard_worktree_changes is not configured")
    }
}

impl<T> GitRepository for T
where
    T: RepositoryDouble,
{
    fn spec(&self) -> &RepoSpec {
        RepositoryDouble::spec(self)
    }

    fn log_head_page(&self, limit: usize, cursor: Option<&LogCursor>) -> Result<LogPage> {
        RepositoryDouble::log_head_page(self, limit, cursor)
    }

    fn commit_details(&self, id: &CommitId) -> Result<CommitDetails> {
        RepositoryDouble::commit_details(self, id)
    }

    fn reflog_head(&self, limit: usize) -> Result<Vec<ReflogEntry>> {
        RepositoryDouble::reflog_head(self, limit)
    }

    fn current_branch(&self) -> Result<String> {
        RepositoryDouble::current_branch(self)
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        RepositoryDouble::list_branches(self)
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        RepositoryDouble::list_remotes(self)
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        RepositoryDouble::list_remote_branches(self)
    }

    fn status(&self) -> Result<RepoStatus> {
        RepositoryDouble::status(self)
    }

    fn head_path_is_gitlink(&self, path: &Path) -> Result<bool> {
        RepositoryDouble::head_path_is_gitlink(self, path)
    }

    fn diff_unified(&self, target: &DiffTarget) -> Result<String> {
        RepositoryDouble::diff_unified(self, target)
    }

    fn create_branch(&self, name: &str, target: &CommitId) -> Result<()> {
        RepositoryDouble::create_branch(self, name, target)
    }

    fn delete_branch(&self, name: &str) -> Result<()> {
        RepositoryDouble::delete_branch(self, name)
    }

    fn checkout_branch(&self, name: &str) -> Result<()> {
        RepositoryDouble::checkout_branch(self, name)
    }

    fn checkout_commit(&self, id: &CommitId) -> Result<()> {
        RepositoryDouble::checkout_commit(self, id)
    }

    fn cherry_pick(&self, id: &CommitId) -> Result<()> {
        RepositoryDouble::cherry_pick(self, id)
    }

    fn revert(&self, id: &CommitId) -> Result<()> {
        RepositoryDouble::revert(self, id)
    }

    fn stash_create(&self, message: &str, include_untracked: bool) -> Result<()> {
        RepositoryDouble::stash_create(self, message, include_untracked)
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        RepositoryDouble::stash_list(self)
    }

    fn stash_apply(&self, index: usize) -> Result<()> {
        RepositoryDouble::stash_apply(self, index)
    }

    fn stash_drop(&self, index: usize) -> Result<()> {
        RepositoryDouble::stash_drop(self, index)
    }

    fn stage(&self, paths: &[&Path]) -> Result<()> {
        RepositoryDouble::stage(self, paths)
    }

    fn unstage(&self, paths: &[&Path]) -> Result<()> {
        RepositoryDouble::unstage(self, paths)
    }

    fn commit(&self, message: &str) -> Result<()> {
        RepositoryDouble::commit(self, message)
    }

    fn fetch_all(&self) -> Result<()> {
        RepositoryDouble::fetch_all(self)
    }

    fn pull(&self, mode: PullMode) -> Result<()> {
        RepositoryDouble::pull(self, mode)
    }

    fn push(&self) -> Result<()> {
        RepositoryDouble::push(self)
    }

    fn discard_worktree_changes(&self, paths: &[&Path]) -> Result<()> {
        RepositoryDouble::discard_worktree_changes(self, paths)
    }
}

type OpenRepository = dyn Fn(&Path) -> Result<Arc<dyn GitRepository>> + Send + Sync + 'static;

/// Closure-backed backend for reducer/effect tests.
#[derive(Clone)]
pub struct TestBackend {
    open: Arc<OpenRepository>,
}

impl TestBackend {
    pub fn new(
        open: impl Fn(&Path) -> Result<Arc<dyn GitRepository>> + Send + Sync + 'static,
    ) -> Self {
        Self {
            open: Arc::new(open),
        }
    }

    pub fn returning(repository: Arc<dyn GitRepository>) -> Self {
        Self::new(move |_| Ok(Arc::clone(&repository)))
    }

    pub fn from_repository(repository: impl GitRepository + 'static) -> Self {
        Self::returning(Arc::new(repository))
    }
}

impl GitBackend for TestBackend {
    fn open(&self, workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        (self.open)(workdir)
    }
}

#[cfg(test)]
mod tests {
    use super::{RepositoryDouble, TestBackend};
    use crate::domain::{RepoSpec, RepoStatus};
    use crate::error::ErrorKind;
    use crate::services::{GitBackend, GitRepository};
    use std::path::{Path, PathBuf};

    struct StatusRepository {
        spec: RepoSpec,
    }

    impl RepositoryDouble for StatusRepository {
        fn spec(&self) -> &RepoSpec {
            &self.spec
        }

        fn status(&self) -> crate::services::Result<RepoStatus> {
            Ok(RepoStatus::default())
        }
    }

    #[test]
    fn double_delegates_overrides_and_defaults_other_operations() {
        let repository = StatusRepository {
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/status-double"),
            },
        };
        assert_eq!(
            GitRepository::status(&repository).expect("configured status"),
            RepoStatus::default()
        );
        let error = GitRepository::current_branch(&repository)
            .expect_err("unconfigured operation must be unsupported");
        assert!(matches!(error.kind(), ErrorKind::Unsupported(_)));
    }

    #[test]
    fn backend_returns_shared_repository() {
        let backend = TestBackend::from_repository(StatusRepository {
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/backend-double"),
            },
        });
        let repository = backend.open(Path::new("ignored")).expect("open double");
        assert_eq!(
            repository.spec().workdir,
            PathBuf::from("/tmp/backend-double")
        );
    }
}
