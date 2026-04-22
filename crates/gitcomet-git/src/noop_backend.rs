#[cfg(test)]
use gitcomet_core::domain::*;
use gitcomet_core::error::{Error, ErrorKind};
use gitcomet_core::services::{GitBackend, GitRepository, Result};
use std::path::Path;
use std::sync::Arc;

#[derive(Default)]
pub struct NoopBackend;

impl GitBackend for NoopBackend {
    fn open(&self, _workdir: &Path) -> Result<Arc<dyn GitRepository>> {
        Err(Error::new(ErrorKind::Unsupported(
            "No Git backend enabled. Build with `--features gix`.",
        )))
    }
}

#[cfg(test)]
pub(crate) struct NoopRepo {
    spec: RepoSpec,
}

#[cfg(test)]
impl GitRepository for NoopRepo {
    fn spec(&self) -> &RepoSpec {
        &self.spec
    }

    fn log_head_page(&self, _limit: usize, _cursor: Option<&LogCursor>) -> Result<LogPage> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn commit_details(&self, _id: &CommitId) -> Result<CommitDetails> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn reflog_head(&self, _limit: usize) -> Result<Vec<ReflogEntry>> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn current_branch(&self) -> Result<String> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn list_branches(&self) -> Result<Vec<Branch>> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn list_remotes(&self) -> Result<Vec<Remote>> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn list_remote_branches(&self) -> Result<Vec<RemoteBranch>> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn status(&self) -> Result<RepoStatus> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn diff_unified(&self, _target: &DiffTarget) -> Result<String> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn create_branch(&self, _name: &str, _target: &CommitId) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn delete_branch(&self, _name: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn checkout_branch(&self, _name: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn checkout_commit(&self, _id: &CommitId) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn cherry_pick(&self, _id: &CommitId) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn revert(&self, _id: &CommitId) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn stash_create(&self, _message: &str, _include_untracked: bool) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn stash_list(&self) -> Result<Vec<StashEntry>> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn stash_apply(&self, _index: usize) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn stash_drop(&self, _index: usize) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn stage(&self, _paths: &[&Path]) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn unstage(&self, _paths: &[&Path]) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn commit(&self, _message: &str) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn fetch_all(&self) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn pull(&self, _mode: gitcomet_core::services::PullMode) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn push(&self) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }

    fn discard_worktree_changes(&self, _paths: &[&Path]) -> Result<()> {
        Err(Error::new(ErrorKind::Unsupported("No Git backend enabled")))
    }
}

#[cfg(test)]
mod tests {
    use super::{NoopBackend, NoopRepo};
    use gitcomet_core::domain::{CommitId, DiffArea, DiffTarget, LogCursor, RepoSpec};
    use gitcomet_core::error::ErrorKind;
    use gitcomet_core::services::{
        ConflictSide, GitBackend, GitRepository, PullMode, RemoteUrlKind, ResetMode, Result,
    };
    use std::path::{Path, PathBuf};

    fn assert_unsupported<T>(result: Result<T>) {
        match result {
            Ok(_) => panic!("expected unsupported error"),
            Err(err) => assert!(matches!(err.kind(), ErrorKind::Unsupported(_))),
        }
    }

    fn sample_repo() -> NoopRepo {
        NoopRepo {
            spec: RepoSpec {
                workdir: PathBuf::from("/tmp/noop-repo"),
            },
        }
    }

    #[test]
    fn noop_backend_open_returns_unsupported() {
        let backend = NoopBackend;
        let err = match backend.open(Path::new(".")) {
            Ok(_) => panic!("noop backend should not open repos"),
            Err(err) => err,
        };
        match err.kind() {
            ErrorKind::Unsupported(message) => {
                assert!(message.contains("No Git backend enabled"));
            }
            _ => panic!("expected Unsupported error"),
        }
    }

    #[test]
    fn noop_repo_required_methods_return_unsupported() {
        let repo = sample_repo();
        let commit = CommitId("abc123".into());
        let cursor = LogCursor {
            last_seen: CommitId("deadbeef".into()),
            resume_from: None,
            resume_token: None,
        };
        let diff_target = DiffTarget::WorkingTree {
            path: PathBuf::from("file.txt"),
            area: DiffArea::Unstaged,
        };
        let paths = [Path::new("file.txt")];

        assert_eq!(repo.spec().workdir, PathBuf::from("/tmp/noop-repo"));
        assert_unsupported(repo.log_head_page(20, Some(&cursor)));
        assert_unsupported(repo.commit_details(&commit));
        assert_unsupported(repo.reflog_head(5));
        assert_unsupported(repo.current_branch());
        assert_unsupported(repo.list_branches());
        assert_unsupported(repo.list_remotes());
        assert_unsupported(repo.list_remote_branches());
        assert_unsupported(repo.status());
        assert_unsupported(repo.diff_unified(&diff_target));
        assert_unsupported(repo.create_branch("feature", &commit));
        assert_unsupported(repo.delete_branch("feature"));
        assert_unsupported(repo.checkout_branch("feature"));
        assert_unsupported(repo.checkout_commit(&commit));
        assert_unsupported(repo.cherry_pick(&commit));
        assert_unsupported(repo.revert(&commit));
        assert_unsupported(repo.stash_create("savepoint", true));
        assert_unsupported(repo.stash_list());
        assert_unsupported(repo.stash_apply(0));
        assert_unsupported(repo.stash_drop(0));
        assert_unsupported(repo.stage(&paths));
        assert_unsupported(repo.unstage(&paths));
        assert_unsupported(repo.commit("message"));
        assert_unsupported(repo.fetch_all());
        assert_unsupported(repo.pull(PullMode::Default));
        assert_unsupported(repo.push());
        assert_unsupported(repo.discard_worktree_changes(&paths));
    }

    #[test]
    fn noop_repo_default_trait_methods_use_fallback_behavior() {
        let repo = sample_repo();
        let commit = CommitId("abc123".into());
        let cursor = LogCursor {
            last_seen: CommitId("deadbeef".into()),
            resume_from: None,
            resume_token: None,
        };
        let diff_target = DiffTarget::WorkingTree {
            path: PathBuf::from("file.txt"),
            area: DiffArea::Staged,
        };
        let path = Path::new("file.txt");

        assert_unsupported(repo.log_all_branches_page(25, Some(&cursor)));
        assert_unsupported(repo.log_file_page(path, 25, None));
        assert_unsupported(repo.list_tags());
        assert_unsupported(repo.list_remote_tags());
        assert_eq!(repo.upstream_divergence().unwrap(), None);
        assert_unsupported(repo.diff_file_text(&diff_target));
        assert_unsupported(repo.diff_file_image(&diff_target));
        assert_unsupported(repo.conflict_file_stages(path));
        assert_unsupported(repo.conflict_session(path));
        assert_unsupported(repo.delete_branch_force("feature"));
        assert_unsupported(repo.checkout_remote_branch("origin", "main", "feature"));
        assert_unsupported(repo.commit_amend("message"));
        assert_unsupported(repo.rebase_with_output("main"));
        assert_unsupported(repo.rebase_continue_with_output());
        assert_unsupported(repo.rebase_abort_with_output());
        assert_unsupported(repo.merge_abort_with_output());
        assert!(!repo.rebase_in_progress().unwrap());
        assert_eq!(repo.merge_commit_message().unwrap(), None);
        assert_unsupported(repo.create_tag_with_output("v1.0.0", "HEAD"));
        assert_unsupported(repo.delete_tag_with_output("v1.0.0"));
        assert_unsupported(repo.prune_merged_branches_with_output());
        assert_unsupported(repo.prune_local_tags_with_output());
        assert_unsupported(repo.push_tag_with_output("origin", "v1.0.0"));
        assert_unsupported(repo.delete_remote_tag_with_output("origin", "v1.0.0"));
        assert_unsupported(repo.add_remote_with_output("origin", "https://example.com/repo.git"));
        assert_unsupported(repo.remove_remote_with_output("origin"));
        assert_unsupported(repo.set_remote_url_with_output(
            "origin",
            "https://example.com/repo.git",
            RemoteUrlKind::Fetch,
        ));
        assert_unsupported(repo.fetch_all_with_output());
        assert_unsupported(repo.fetch_all_with_output_prune(true));
        assert_unsupported(repo.pull_with_output(PullMode::FastForwardOnly));
        assert_unsupported(repo.push_with_output());
        assert_unsupported(repo.push_force());
        assert_unsupported(repo.push_force_with_output());
        assert_unsupported(repo.push_set_upstream("origin", "main"));
        assert_unsupported(repo.push_set_upstream_with_output("origin", "main"));
        assert_unsupported(repo.set_upstream_branch_with_output("main", "origin/main"));
        assert_unsupported(repo.unset_upstream_branch_with_output("main"));
        assert_unsupported(repo.delete_remote_branch_with_output("origin", "main"));
        assert_unsupported(repo.commit_amend_with_output("message"));
        assert_unsupported(repo.pull_branch_with_output("origin", "main"));
        assert_unsupported(repo.merge_ref_with_output("origin/main"));
        assert_unsupported(repo.squash_ref_with_output("origin/main"));
        assert_unsupported(repo.reset_with_output("HEAD~1", ResetMode::Mixed));
        assert_unsupported(repo.blame_file(path, None));
        assert_unsupported(repo.checkout_conflict_side(path, ConflictSide::Ours));
        assert_unsupported(repo.accept_conflict_deletion(path));
        assert_unsupported(repo.checkout_conflict_base(path));
        assert_unsupported(repo.launch_mergetool(path));
        assert_unsupported(repo.export_patch_with_output(&commit, path));
        assert_unsupported(repo.apply_patch_with_output(path));
        assert_unsupported(repo.apply_unified_patch_to_index_with_output("@@ -1 +1 @@", false));
        assert_unsupported(repo.apply_unified_patch_to_worktree_with_output("@@ -1 +1 @@", true));
        assert_unsupported(repo.list_worktrees());
        assert_unsupported(repo.add_worktree_with_output(path, Some("main")));
        assert_unsupported(repo.remove_worktree_with_output(path));
        assert_unsupported(repo.force_remove_worktree_with_output(path));
        assert_unsupported(repo.list_submodules());
        assert_unsupported(repo.check_submodule_add_trust("https://example.com/repo.git", path));
        assert_unsupported(repo.check_submodule_update_trust());
        assert_unsupported(repo.add_submodule_with_output(
            "https://example.com/repo.git",
            path,
            None,
            None,
            false,
            &[],
        ));
        assert_unsupported(repo.update_submodules_with_output(&[]));
        assert_unsupported(repo.remove_submodule_with_output(path));
    }
}
